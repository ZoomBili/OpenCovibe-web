//! One-shot Codex exec process management for Web fallback sessions.

use crate::agent::claude_stream::augmented_path;
use crate::agent::codex_parser::extract_codex_delta;
use crate::agent::pipe_parser::{CodexStdoutParser, PipeStdoutParser};
use crate::models::{BusEvent, ConversationRef, RunEventType, RunStatus};
use crate::process_ext::HideConsole;
use crate::storage;
use crate::web_server::broadcaster::BroadcastEmitter;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::Mutex;

const CODEX_IDLE_TIMEOUT: Duration = Duration::from_secs(1800);

pub type ProcessMap = Arc<Mutex<HashMap<String, Child>>>;

pub fn new_process_map() -> ProcessMap {
    Arc::new(Mutex::new(HashMap::new()))
}

fn is_benign_codex_stderr(line: &str) -> bool {
    let line = line.trim();
    line.is_empty() || line == "Reading additional input from stdin..."
}

fn summarize_stderr(stderr: &str) -> Option<String> {
    let summary = stderr
        .lines()
        .filter(|line| !is_benign_codex_stderr(line))
        .collect::<Vec<_>>()
        .join("\n");
    if summary.is_empty() {
        None
    } else {
        Some(summary.chars().take(1000).collect())
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_codex_exec(
    process_map: ProcessMap,
    emitter: Arc<BroadcastEmitter>,
    run_id: String,
    command: String,
    args: Vec<String>,
    cwd: String,
    extra_env: HashMap<String, String>,
) -> Result<(), String> {
    if process_map.lock().await.contains_key(&run_id) {
        return Err("A Codex turn is already running".to_string());
    }
    let process_seq = storage::runs::next_codex_process_seq(&run_id)?;
    let resolved = if std::path::Path::new(&command).is_absolute() {
        command
    } else {
        crate::agent::claude_stream::resolve_codex_path()
    };
    let mut command = Command::new(&resolved);
    command
        .args(&args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PATH", augmented_path())
        .env("OPENCOVIBE_TASK_ID", &run_id)
        .env("OPENCOVIBE_RUN_ID", &run_id)
        .env_remove("CLAUDECODE")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .hide_console()
        .kill_on_drop(true);
    for (key, value) in extra_env {
        command.env(key, value);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start Codex CLI at {}: {}", resolved, error))?;
    let stdout = child.stdout.take().ok_or("Codex stdout is unavailable")?;
    let stderr = child.stderr.take().ok_or("Codex stderr is unavailable")?;
    process_map.lock().await.insert(run_id.clone(), child);

    let stdout_run_id = run_id.clone();
    let stdout_emitter = emitter.clone();
    let stdout_process_map = process_map.clone();
    let stdout_handle = tokio::spawn(async move {
        let mut parser = CodexStdoutParser::new(process_seq);
        let mut lines = BufReader::new(stdout).lines();
        let mut assistant_text = String::new();
        let mut fallback_text = String::new();
        let mut timed_out = false;

        loop {
            let line = match tokio::time::timeout(CODEX_IDLE_TIMEOUT, lines.next_line()).await {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => break,
                Ok(Err(error)) => {
                    log::warn!("[stream] Codex stdout read failed: {}", error);
                    break;
                }
                Err(_) => {
                    timed_out = true;
                    if let Some(mut child) = stdout_process_map.lock().await.remove(&stdout_run_id)
                    {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                    }
                    break;
                }
            };
            let _ = storage::events::append_event(
                &stdout_run_id,
                RunEventType::Stdout,
                serde_json::json!({ "text": line, "source": "web_chat" }),
            );
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(payload) => {
                    let event_type = payload
                        .get("type")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    if event_type == "thread.started" {
                        if let Some(thread_id) =
                            payload.get("thread_id").and_then(|value| value.as_str())
                        {
                            let thread_id = thread_id.to_string();
                            let _ = storage::runs::with_meta(&stdout_run_id, |meta| {
                                meta.conversation_ref =
                                    Some(ConversationRef::CodexThread(thread_id));
                                Ok(())
                            });
                        }
                    }
                    let events = parser.parse_line(&stdout_run_id, &payload);
                    for event in &events {
                        if let BusEvent::MessageComplete { text, .. } = event {
                            assistant_text.push_str(text);
                        }
                        stdout_emitter.persist_and_emit(&stdout_run_id, event);
                    }
                    if events.is_empty() {
                        if let Some(text) = extract_codex_delta(&payload) {
                            fallback_text.push_str(&text);
                            stdout_emitter.persist_and_emit(
                                &stdout_run_id,
                                &BusEvent::MessageDelta {
                                    run_id: stdout_run_id.clone(),
                                    text,
                                    parent_tool_use_id: None,
                                },
                            );
                        }
                    }
                }
                Err(_) => {
                    let text = format!("{}\n", trimmed);
                    fallback_text.push_str(&text);
                    stdout_emitter.persist_and_emit(
                        &stdout_run_id,
                        &BusEvent::MessageDelta {
                            run_id: stdout_run_id.clone(),
                            text,
                            parent_tool_use_id: None,
                        },
                    );
                }
            }
        }

        if !fallback_text.is_empty() {
            assistant_text.push_str(&fallback_text);
            stdout_emitter.persist_and_emit(
                &stdout_run_id,
                &BusEvent::MessageComplete {
                    run_id: stdout_run_id.clone(),
                    message_id: format!("codex-{}-fallback", process_seq),
                    text: fallback_text,
                    parent_tool_use_id: None,
                    model: None,
                    stop_reason: None,
                    message_usage: None,
                },
            );
        }
        (assistant_text, timed_out)
    });

    let stderr_run_id = run_id.clone();
    let stderr_emitter = emitter.clone();
    let stderr_handle = tokio::spawn(async move {
        let mut stderr_text = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            stderr_text.push_str(&line);
            stderr_text.push('\n');
            let _ = storage::events::append_event(
                &stderr_run_id,
                RunEventType::Stderr,
                serde_json::json!({ "text": line, "source": "web_chat" }),
            );
            if !is_benign_codex_stderr(&line) {
                stderr_emitter.persist_and_emit(
                    &stderr_run_id,
                    &BusEvent::CommandOutput {
                        run_id: stderr_run_id.clone(),
                        content: format!("[stderr] {}", line),
                    },
                );
            }
        }
        stderr_text
    });

    let (assistant_text, timed_out) = stdout_handle
        .await
        .map_err(|error| format!("Codex stdout task failed: {}", error))?;
    let stderr_text = stderr_handle
        .await
        .map_err(|error| format!("Codex stderr task failed: {}", error))?;
    let child = process_map.lock().await.remove(&run_id);
    let exit_code = if let Some(mut child) = child {
        child
            .wait()
            .await
            .map_err(|error| format!("Failed waiting for Codex: {}", error))?
            .code()
            .unwrap_or(1)
    } else if timed_out {
        -2
    } else {
        -1
    };

    if !assistant_text.trim().is_empty() {
        let _ = storage::events::append_event(
            &run_id,
            RunEventType::Assistant,
            serde_json::json!({ "text": assistant_text.trim(), "source": "web_chat" }),
        );
    }

    let has_thread = storage::runs::get_run(&run_id)
        .map(|run| run.conversation_ref.is_some())
        .unwrap_or(false);
    let error = match exit_code {
        0 | -1 => None,
        -2 => Some(format!(
            "Codex timed out after {} seconds without output",
            CODEX_IDLE_TIMEOUT.as_secs()
        )),
        _ => summarize_stderr(&stderr_text)
            .or_else(|| Some(format!("Codex exited with code {}", exit_code))),
    };
    let (status, state) = match exit_code {
        0 if has_thread => (RunStatus::Idle, "idle"),
        0 => (RunStatus::Completed, "completed"),
        -1 => (RunStatus::Stopped, "stopped"),
        _ => (RunStatus::Failed, "failed"),
    };
    storage::runs::update_status(
        &run_id,
        status,
        (exit_code >= 0).then_some(exit_code),
        error.clone(),
    )?;
    emitter.persist_and_emit(
        &run_id,
        &BusEvent::RunState {
            run_id: run_id.clone(),
            state: state.to_string(),
            exit_code: Some(exit_code),
            error,
        },
    );
    Ok(())
}

pub async fn stop_process(process_map: &ProcessMap, run_id: &str) -> bool {
    log::debug!("[stream] stop_process: run_id={}", run_id);
    // Short lock: remove child, then kill+wait outside the lock.
    let removed = {
        let mut map = process_map.lock().await;
        map.remove(run_id)
    };
    if let Some(mut child) = removed {
        let _ = child.kill().await;
        let _ = child.wait().await;
        log::debug!("[stream] stop_process: killed run_id={}", run_id);
        true
    } else {
        log::debug!("[stream] stop_process: no process for run_id={}", run_id);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_summary_ignores_codex_banner() {
        assert_eq!(
            summarize_stderr("Reading additional input from stdin...\nreal error\n"),
            Some("real error".to_string())
        );
    }
}
