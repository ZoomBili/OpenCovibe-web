//! Executes a single automation: create a run, spawn a session, send the skill invocation.
//!
//! Automations are intentionally headless — nobody is watching the run, so once the turn
//! finishes (or fails) we stop the actor to release the CLI child process. The run itself
//! stays in history, and can still be resumed later from the UI.

use crate::agent::session_protocol::CodexSkillRef;
use crate::models::{AutomationTask, RunStatus};
use crate::web_server::state::AppState;
use std::time::Duration;

/// Spawn a run for this automation. Returns the new run id on success.
pub async fn execute(state: &AppState, task: &AutomationTask) -> Result<String, String> {
    let result = execute_inner(state, task).await;
    if let Err(error) = &result {
        let _ = crate::storage::automations::mark_attempt_failed(&task.id, error.clone());
    }
    result
}

async fn execute_inner(state: &AppState, task: &AutomationTask) -> Result<String, String> {
    let agent = match task.skill_agent.as_str() {
        "codex" => "codex",
        _ => "claude",
    };
    let cwd = resolve_cwd(task)?;
    let prompt = task.prompt.trim();

    // Claude triggers skills through the textual slash command; Codex only triggers through
    // structured `{type:"skill"}` input items (plain `/name` text is not enough).
    let (initial_message, display_prompt) = if agent == "claude" {
        let text = if prompt.is_empty() {
            format!("/{}", task.skill_name)
        } else {
            format!("/{} {}", task.skill_name, prompt)
        };
        (text.clone(), text)
    } else {
        let text = if prompt.is_empty() {
            format!("Run the \"{}\" skill.", task.skill_name)
        } else {
            prompt.to_string()
        };
        let display = if prompt.is_empty() {
            format!("/{}", task.skill_name)
        } else {
            format!("/{} {}", task.skill_name, prompt)
        };
        (text, display)
    };

    let skills = if agent == "codex" && !task.skill_path.trim().is_empty() {
        vec![CodexSkillRef {
            name: task.skill_name.clone(),
            path: task.skill_path.clone(),
        }]
    } else {
        Vec::new()
    };

    log::info!(
        "[automation] running '{}' ({}): cron='{}', agent={}, cwd={}",
        task.name,
        task.id,
        task.cron,
        agent,
        cwd
    );

    // Force the session_actor path: Codex skill refs and Claude skill events both need the
    // long-lived protocol. `exec` transports cannot carry the structured skill item.
    let run = crate::commands::runs::start_run(
        display_prompt,
        cwd,
        agent.to_string(),
        task.model.clone(),
        None,
        None,
        Some("session_actor".to_string()),
    )?;
    let run_id = run.id.clone();
    if let Err(error) = crate::storage::automations::mark_run_started(&task.id, &run_id) {
        // The task was deleted between the scheduler snapshot and this spawn. Don't leave
        // an invisible Pending run behind.
        let _ = crate::storage::runs::update_status(
            &run_id,
            RunStatus::Failed,
            None,
            Some(error.clone()),
        );
        return Err(error);
    }

    let start_result = crate::commands::session::start_session_impl(
        &state.emitter,
        &state.sessions,
        &state.spawn_locks,
        &state.cancel_token,
        run_id.clone(),
        None,
        None,
        Some(initial_message),
        None,
        None,
        None,
        if skills.is_empty() {
            None
        } else {
            Some(skills)
        },
    )
    .await;

    if let Err(error) = start_result {
        let _ = crate::storage::runs::update_status(
            &run_id,
            RunStatus::Failed,
            None,
            Some(error.clone()),
        );
        let _ = crate::storage::automations::mark_run_finished(
            &task.id,
            &run_id,
            "failed",
            Some(error.clone()),
            None,
            None,
        );
        return Err(error);
    }

    spawn_completion_monitor(state.clone(), task.id.clone(), run_id.clone());
    Ok(run_id)
}

/// Project root for a project-scoped skill path.
///
/// Claude/Codex discover project skills relative to the session cwd:
///   `<project>/.claude/skills/...`, `<project>/.agents/skills/...`, `<project>/.codex/skills/...`
/// User-level skills live directly under `$HOME`; those return `None` so the configured
/// working directory still wins.
pub fn skill_project_root(skill_path: &str) -> Option<String> {
    let path = std::path::Path::new(skill_path);
    if !path.is_absolute() {
        return None;
    }
    let home = crate::storage::home_dir().map(std::path::PathBuf::from);
    let components: Vec<_> = path.components().collect();
    for (index, component) in components.iter().enumerate() {
        let name = component.as_os_str();
        if name == std::ffi::OsStr::new(".claude")
            || name == std::ffi::OsStr::new(".agents")
            || name == std::ffi::OsStr::new(".codex")
        {
            if index == 0 {
                return None;
            }
            let mut root = std::path::PathBuf::new();
            for part in &components[..index] {
                root.push(part.as_os_str());
            }
            if home.as_deref() == Some(root.as_path()) {
                return None;
            }
            if root.is_dir() {
                return Some(root.to_string_lossy().to_string());
            }
            return None;
        }
    }
    None
}

fn resolve_cwd(task: &AutomationTask) -> Result<String, String> {
    let configured = task.cwd.trim();
    let cwd = if !configured.is_empty() {
        configured.to_string()
    } else if let Some(project_root) = skill_project_root(&task.skill_path) {
        // Project-scoped skill: it only resolves when the CLI starts in that project.
        project_root
    } else {
        crate::storage::settings::get_user_settings()
            .working_directory
            .unwrap_or_default()
    };
    let cwd = if cwd.is_empty() {
        crate::storage::home_dir().ok_or_else(|| "could not resolve home directory".to_string())?
    } else {
        cwd
    };
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(format!("working directory does not exist: {}", cwd));
    }
    Ok(cwd)
}

/// Read the final assistant preview / result subtype for the execution log.
/// Best-effort: the log entry must still be written even if run data is already gone.
fn collect_run_result(run_id: &str) -> (Option<String>, Option<String>, Option<String>) {
    match crate::commands::runs::get_run(run_id.to_string()) {
        Ok(run) => (
            run.last_message_preview,
            run.result_subtype,
            run.error_message,
        ),
        Err(error) => {
            log::debug!(
                "[automation] could not read result for run {}: {}",
                run_id,
                error
            );
            (None, None, None)
        }
    }
}

/// Poll the run until it reaches a terminal/idle state, then stop the actor.
/// This keeps cron jobs from accumulating one idle CLI process per fire.
fn spawn_completion_monitor(state: AppState, automation_id: String, run_id: String) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6 * 60 * 60);
        loop {
            tokio::select! {
                _ = state.cancel_token.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(10)) => {}
            }

            let Some(meta) = crate::storage::runs::get_run(&run_id) else {
                let _ = crate::storage::automations::mark_run_finished(
                    &automation_id,
                    &run_id,
                    "failed",
                    Some("run record disappeared".to_string()),
                    None,
                    None,
                );
                return;
            };

            let terminal = matches!(
                &meta.status,
                RunStatus::Idle | RunStatus::Completed | RunStatus::Failed | RunStatus::Stopped
            );
            if terminal || tokio::time::Instant::now() >= deadline {
                let status = if terminal {
                    // A finished turn lands on `Idle` (session kept alive). For a headless
                    // automation that means "done", not "waiting for the user".
                    if meta.status == RunStatus::Idle {
                        "completed".to_string()
                    } else {
                        meta.status.to_string()
                    }
                } else {
                    "timeout".to_string()
                };
                let (result_preview, result_subtype, run_error) = collect_run_result(&run_id);
                let timeout_error = "automation run exceeded 6h; session stopped".to_string();
                let error = if terminal {
                    meta.error_message.clone().or(run_error)
                } else {
                    // Persist a terminal status too — otherwise the scheduler's "active run"
                    // guard keeps seeing `Running` and never fires this task again.
                    let _ = crate::storage::runs::update_status(
                        &run_id,
                        RunStatus::Failed,
                        None,
                        Some(timeout_error.clone()),
                    );
                    Some(timeout_error)
                };
                let _ = crate::storage::automations::mark_run_finished(
                    &automation_id,
                    &run_id,
                    &status,
                    error,
                    result_preview,
                    result_subtype,
                );
                // Stop the actor without rewriting the run status — the run finished
                // naturally; only the idle CLI process should go away.
                let _ = crate::commands::session::stop_actor(&state.sessions, &run_id).await;
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_project_root_for_project_scoped_skills() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("my-project");
        std::fs::create_dir_all(project.join(".claude").join("skills").join("demo")).unwrap();
        let skill = project
            .join(".claude")
            .join("skills")
            .join("demo")
            .join("SKILL.md");
        assert_eq!(
            skill_project_root(skill.to_str().unwrap()),
            Some(project.to_string_lossy().to_string())
        );
    }

    #[test]
    fn ignores_user_level_skill_paths() {
        let home = crate::storage::home_dir().expect("home");
        for marker in [".claude", ".agents", ".codex"] {
            let path = format!("{}/{}/skills/demo/SKILL.md", home, marker);
            assert_eq!(skill_project_root(&path), None, "path={path}");
        }
    }
}
