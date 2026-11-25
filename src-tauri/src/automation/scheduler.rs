//! Background cron loop.
//!
//! Runs inside the web server process (not the browser), so automations fire even when no
//! client is connected. One tick per 20s is enough: cron granularity is one minute, and
//! `last_run_at` ensures a scheduled minute is executed at most once.

use crate::automation::{cron::CronSchedule, runner};
use crate::models::RunStatus;
use crate::storage;
use crate::web_server::state::AppState;
use chrono::{DateTime, Local};
use std::time::Duration;

const TICK_INTERVAL: Duration = Duration::from_secs(20);

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // A previous process may have died mid-run; resolve any execution still marked
        // `running` before the first tick so the log doesn't get stuck.
        crate::storage::automations::reconcile_running_executions();

        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first interval tick is immediate; let the server finish booting first.
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = state.cancel_token.cancelled() => break,
                _ = ticker.tick() => {}
            }
            tick(&state).await;
        }
        log::debug!("[automation] scheduler stopped");
    });
}

async fn tick(state: &AppState) {
    let now = Local::now();
    let tasks = storage::automations::list();
    if tasks.is_empty() {
        return;
    }

    for task in tasks {
        if !task.enabled {
            continue;
        }
        let schedule = match CronSchedule::parse(&task.cron) {
            Ok(schedule) => schedule,
            Err(error) => {
                log::warn!(
                    "[automation] task '{}' ({}) has invalid cron '{}': {}",
                    task.name,
                    task.id,
                    task.cron,
                    error
                );
                continue;
            }
        };
        if !schedule.matches(&now) {
            continue;
        }
        if already_ran_this_minute(task.last_run_at.as_deref(), &now) {
            continue;
        }
        if is_run_active(task.last_run_id.as_deref()) {
            log::debug!(
                "[automation] task '{}' still has an active run; skipping overlap",
                task.name
            );
            continue;
        }

        log::info!(
            "[automation] firing '{}' ({}): {}",
            task.name,
            task.id,
            task.cron
        );
        match runner::execute(state, &task).await {
            Ok(run_id) => log::info!("[automation] '{}' started run {}", task.name, run_id),
            Err(error) => log::warn!("[automation] '{}' failed to start: {}", task.name, error),
        }
    }
}

fn already_ran_this_minute(last_run_at: Option<&str>, now: &DateTime<Local>) -> bool {
    let Some(last) = last_run_at else {
        return false;
    };
    match DateTime::parse_from_rfc3339(last) {
        Ok(last) => {
            last.with_timezone(&Local)
                .format("%Y-%m-%dT%H:%M")
                .to_string()
                == now.format("%Y-%m-%dT%H:%M").to_string()
        }
        // Unparseable timestamp: err on the side of running rather than never running.
        Err(_) => false,
    }
}

fn is_run_active(run_id: Option<&str>) -> bool {
    run_id
        .and_then(storage::runs::get_run)
        .map(|meta| meta.status == RunStatus::Running)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn detects_same_minute_in_local_time() {
        let now = Local
            .with_ymd_and_hms(2026, 3, 10, 9, 30, 20)
            .single()
            .unwrap();
        let last = now.with_timezone(&chrono::Utc).to_rfc3339();
        assert!(already_ran_this_minute(Some(&last), &now));

        let earlier = (now - chrono::Duration::minutes(1))
            .with_timezone(&chrono::Utc)
            .to_rfc3339();
        assert!(!already_ran_this_minute(Some(&earlier), &now));
        assert!(!already_ran_this_minute(None, &now));
    }
}
