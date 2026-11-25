//! Persistence for cron-scheduled skill automations.
//!
//! Stored separately from `settings.json` so a corrupt scheduling entry can never take the
//! whole settings file down with it. The file is small and read on every scheduler tick
//! (once per ~20s), so a plain JSON document is enough — no index needed.

use crate::automation::cron::CronSchedule;
use crate::models::{AutomationExecution, AutomationTask, AutomationTaskInput, RunStatus};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// Serializes read-modify-write cycles inside this process. The scheduler tick and a
/// user-triggered "run now" can otherwise race and lose each other's `last_run_*` update.
static LOCK: Mutex<()> = Mutex::new(());

/// Keep the inline execution log small — the full history is always available as runs.
const MAX_EXECUTIONS: usize = 20;
/// Result previews are shown in a compact log row; the full message lives in the run.
const MAX_PREVIEW_CHARS: usize = 500;

fn automations_path() -> PathBuf {
    super::data_dir().join("automations.json")
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct AutomationStore {
    #[serde(default)]
    tasks: Vec<AutomationTask>,
}

fn load_store() -> AutomationStore {
    let path = automations_path();
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(store) => store,
            Err(error) => {
                log::warn!(
                    "[storage/automations] failed to parse {}: {}",
                    path.display(),
                    error
                );
                AutomationStore::default()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AutomationStore::default(),
        Err(error) => {
            log::warn!(
                "[storage/automations] failed to read {}: {}",
                path.display(),
                error
            );
            AutomationStore::default()
        }
    }
}

fn save_store(store: &AutomationStore) -> Result<(), String> {
    let dir = super::data_dir();
    super::ensure_dir(&dir).map_err(|error| error.to_string())?;
    let path = automations_path();
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(store).map_err(|error| error.to_string())?;
    fs::write(&tmp, body).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, &path).map_err(|error| error.to_string())
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn truncate_preview(text: Option<String>) -> Option<String> {
    text.map(|value| {
        let trimmed = value.trim();
        if trimmed.chars().count() <= MAX_PREVIEW_CHARS {
            trimmed.to_string()
        } else {
            let cut: String = trimmed.chars().take(MAX_PREVIEW_CHARS).collect();
            format!("{}…", cut)
        }
    })
    .filter(|value| !value.is_empty())
}

fn push_execution(task: &mut AutomationTask, execution: AutomationExecution) {
    task.executions.insert(0, execution);
    task.executions.truncate(MAX_EXECUTIONS);
}

/// Validate and normalize client input. Cron is parsed here so an invalid schedule can
/// never reach the scheduler loop.
fn normalize_input(input: AutomationTaskInput) -> Result<AutomationTaskInput, String> {
    let mut input = input;
    input.name = input.name.trim().to_string();
    input.skill_name = input.skill_name.trim().to_string();
    input.skill_path = input.skill_path.trim().to_string();
    input.cron = input.cron.trim().to_string();
    input.prompt = input.prompt.trim().to_string();
    input.cwd = input.cwd.trim().to_string();
    input.skill_agent = input.skill_agent.trim().to_lowercase();
    input.model = input
        .model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());

    if input.name.is_empty() {
        input.name = input.skill_name.clone();
    }
    if input.name.is_empty() {
        return Err("automation name is required".to_string());
    }
    if input.skill_name.is_empty() {
        return Err("skill name is required".to_string());
    }
    if input.skill_agent != "claude" && input.skill_agent != "codex" {
        return Err(format!(
            "invalid skill agent '{}': expected 'claude' or 'codex'",
            input.skill_agent
        ));
    }
    if input.skill_agent == "codex" && input.skill_path.is_empty() {
        return Err("Codex skills require a skill path".to_string());
    }
    if input.cron.is_empty() {
        return Err("cron expression is required".to_string());
    }
    CronSchedule::parse(&input.cron)?;
    if !input.cwd.is_empty() && !std::path::Path::new(&input.cwd).is_absolute() {
        return Err("working directory must be an absolute path".to_string());
    }
    Ok(input)
}

pub fn list() -> Vec<AutomationTask> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    load_store().tasks
}

pub fn get(id: &str) -> Option<AutomationTask> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    load_store().tasks.into_iter().find(|task| task.id == id)
}

pub fn create(input: AutomationTaskInput) -> Result<AutomationTask, String> {
    let input = normalize_input(input)?;
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_store();
    let timestamp = now_iso();
    let task = AutomationTask {
        id: uuid::Uuid::new_v4().to_string(),
        name: input.name,
        skill_name: input.skill_name,
        skill_path: input.skill_path,
        skill_agent: input.skill_agent,
        prompt: input.prompt,
        cwd: input.cwd,
        model: input.model,
        cron: input.cron,
        enabled: input.enabled,
        created_at: timestamp.clone(),
        updated_at: timestamp,
        last_run_at: None,
        last_run_id: None,
        last_status: None,
        last_error: None,
        executions: Vec::new(),
        next_run_at: None,
        next_run_local: None,
    };
    store.tasks.push(task.clone());
    save_store(&store)?;
    log::debug!("[storage/automations] created id={}", task.id);
    Ok(task)
}

pub fn update(id: &str, input: AutomationTaskInput) -> Result<AutomationTask, String> {
    let input = normalize_input(input)?;
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_store();
    let task = store
        .tasks
        .iter_mut()
        .find(|task| task.id == id)
        .ok_or_else(|| format!("Automation {} not found", id))?;
    task.name = input.name;
    task.skill_name = input.skill_name;
    task.skill_path = input.skill_path;
    task.skill_agent = input.skill_agent;
    task.prompt = input.prompt;
    task.cwd = input.cwd;
    task.model = input.model;
    task.cron = input.cron;
    task.enabled = input.enabled;
    task.updated_at = now_iso();
    // Recomputed by the list API; never persist a stale schedule preview.
    task.next_run_at = None;
    task.next_run_local = None;
    let updated = task.clone();
    save_store(&store)?;
    log::debug!("[storage/automations] updated id={}", id);
    Ok(updated)
}

pub fn delete(id: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_store();
    let before = store.tasks.len();
    store.tasks.retain(|task| task.id != id);
    if store.tasks.len() == before {
        return Err(format!("Automation {} not found", id));
    }
    save_store(&store)?;
    log::debug!("[storage/automations] deleted id={}", id);
    Ok(())
}

/// Record that a scheduled/manual run was spawned.
pub fn mark_run_started(id: &str, run_id: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_store();
    let task = store
        .tasks
        .iter_mut()
        .find(|task| task.id == id)
        .ok_or_else(|| format!("Automation {} not found", id))?;
    let timestamp = now_iso();
    task.last_run_at = Some(timestamp.clone());
    task.last_run_id = Some(run_id.to_string());
    task.last_status = Some("running".to_string());
    task.last_error = None;
    push_execution(
        task,
        AutomationExecution {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: Some(run_id.to_string()),
            started_at: timestamp,
            finished_at: None,
            status: "running".to_string(),
            error: None,
            result_preview: None,
            result_subtype: None,
        },
    );
    save_store(&store)
}

/// Record a failure that happened before a run could be created (bad cwd, spawn error).
/// `last_run_at` is still stamped so the scheduler doesn't retry every tick in the same minute.
pub fn mark_attempt_failed(id: &str, error: String) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_store();
    let Some(task) = store.tasks.iter_mut().find(|task| task.id == id) else {
        return Ok(());
    };
    let timestamp = now_iso();
    task.last_run_at = Some(timestamp.clone());
    task.last_run_id = None;
    task.last_status = Some("failed".to_string());
    task.last_error = Some(error.clone());
    push_execution(
        task,
        AutomationExecution {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: None,
            started_at: timestamp.clone(),
            finished_at: Some(timestamp),
            status: "failed".to_string(),
            error: Some(error),
            result_preview: None,
            result_subtype: None,
        },
    );
    save_store(&store)
}

/// Record the terminal state of a run. Updates the matching execution entry even when a
/// newer run has since started; only the "last run" summary is gated on `last_run_id`.
pub fn mark_run_finished(
    id: &str,
    run_id: &str,
    status: &str,
    error: Option<String>,
    result_preview: Option<String>,
    result_subtype: Option<String>,
) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_store();
    let Some(task) = store.tasks.iter_mut().find(|task| task.id == id) else {
        return Ok(());
    };

    let mut changed = false;
    if let Some(execution) = task
        .executions
        .iter_mut()
        .find(|execution| execution.run_id.as_deref() == Some(run_id))
    {
        execution.status = status.to_string();
        execution.finished_at = Some(now_iso());
        if error.is_some() {
            execution.error = error.clone();
        }
        execution.result_preview = truncate_preview(result_preview);
        execution.result_subtype = result_subtype;
        changed = true;
    }

    if task.last_run_id.as_deref() == Some(run_id) {
        task.last_status = Some(status.to_string());
        if error.is_some() {
            task.last_error = error;
        }
        changed = true;
    }

    if changed {
        save_store(&store)?;
    }
    Ok(())
}

/// After a restart the in-process monitor is gone. Any execution still marked `running`
/// gets reconciled against the persisted run state (the run reconciler runs before this).
pub fn reconcile_running_executions() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_store();
    let mut changed = false;

    for task in &mut store.tasks {
        for execution in task
            .executions
            .iter_mut()
            .filter(|execution| execution.status == "running")
        {
            let Some(run_id) = execution.run_id.as_deref() else {
                continue;
            };
            let Some(meta) = super::runs::get_run(run_id) else {
                execution.status = "failed".to_string();
                execution.finished_at = Some(now_iso());
                execution.error = Some("run record missing".to_string());
                changed = true;
                continue;
            };

            let resolved = match meta.status {
                RunStatus::Running => None,
                RunStatus::Idle | RunStatus::Completed => Some(("completed", None)),
                RunStatus::Failed => Some(("failed", meta.error_message.clone())),
                RunStatus::Stopped => Some(("stopped", None)),
                RunStatus::Pending => Some(("failed", Some("session never started".to_string()))),
            };
            let Some((status, error)) = resolved else {
                continue;
            };
            execution.status = status.to_string();
            execution.finished_at = Some(now_iso());
            if error.is_some() {
                execution.error = error;
            }
            changed = true;

            if task.last_run_id.as_deref() == Some(run_id) {
                task.last_status = Some(status.to_string());
                task.last_error = execution.error.clone();
            }
        }
    }

    if changed {
        if let Err(error) = save_store(&store) {
            log::warn!(
                "[storage/automations] failed to reconcile running executions: {}",
                error
            );
        }
    }
}
