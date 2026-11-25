use crate::automation::cron::CronSchedule;
use crate::automation::runner;
use crate::models::{AutomationTask, AutomationTaskInput};
use crate::storage;
use crate::web_server::state::AppState;
use chrono::Local;
use serde_json::{json, Value};

fn with_next_run(mut task: AutomationTask) -> AutomationTask {
    // Project-scoped skills only load when the CLI starts inside that project. Surface the
    // inferred project root so the UI shows the directory the run will actually use (and
    // saving/toggling the task makes it explicit).
    if task.cwd.trim().is_empty() {
        if let Some(project_root) = runner::skill_project_root(&task.skill_path) {
            task.cwd = project_root;
        }
    }

    // Disabled tasks intentionally have no "next run" — avoids implying they will fire.
    if task.enabled {
        if let Ok(schedule) = CronSchedule::parse(&task.cron) {
            if let Some(next) = schedule.next_runs(Local::now(), 1).into_iter().next() {
                task.next_run_at = Some(next.to_rfc3339());
                task.next_run_local = Some(next.format("%Y-%m-%d %H:%M").to_string());
            }
        }
    } else {
        task.next_run_at = None;
        task.next_run_local = None;
    }
    task
}

pub fn list_automations() -> Result<Vec<AutomationTask>, String> {
    Ok(storage::automations::list()
        .into_iter()
        .map(with_next_run)
        .collect())
}

pub fn create_automation(input: AutomationTaskInput) -> Result<AutomationTask, String> {
    log::debug!(
        "[automations] create: name={}, skill={} ({}), cron={}",
        input.name,
        input.skill_name,
        input.skill_agent,
        input.cron
    );
    storage::automations::create(input).map(with_next_run)
}

pub fn update_automation(id: String, input: AutomationTaskInput) -> Result<AutomationTask, String> {
    log::debug!("[automations] update: id={}", id);
    storage::automations::update(&id, input).map(with_next_run)
}

pub fn delete_automation(id: String) -> Result<bool, String> {
    log::debug!("[automations] delete: id={}", id);
    storage::automations::delete(&id)?;
    Ok(true)
}

/// Validate a cron expression and preview its next three fire times (server local time).
pub fn validate_cron(expression: String) -> Value {
    match CronSchedule::parse(&expression) {
        Ok(schedule) => {
            let runs = schedule.next_runs(Local::now(), 3);
            // RFC3339 for machine use, plus server-local wall-clock strings for display —
            // the browser may be in a different time zone than the NAS.
            let next_runs: Vec<String> = runs.iter().map(|time| time.to_rfc3339()).collect();
            let next_runs_local: Vec<String> = runs
                .iter()
                .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
                .collect();
            json!({
                "valid": true,
                "next_runs": next_runs,
                "next_runs_local": next_runs_local,
                "timezone": Local::now().offset().to_string(),
            })
        }
        Err(error) => json!({
            "valid": false,
            "error": error,
        }),
    }
}

/// Trigger an automation immediately, regardless of its schedule.
pub async fn run_automation_now(state: &AppState, id: String) -> Result<Value, String> {
    let task =
        storage::automations::get(&id).ok_or_else(|| format!("Automation {} not found", id))?;

    if let Some(run_id) = task.last_run_id.as_deref() {
        let active = storage::runs::get_run(run_id)
            .map(|meta| meta.status == crate::models::RunStatus::Running)
            .unwrap_or(false);
        if active {
            return Err("This automation already has a running session".to_string());
        }
    }

    let run_id = runner::execute(state, &task).await?;
    Ok(json!({ "run_id": run_id }))
}
