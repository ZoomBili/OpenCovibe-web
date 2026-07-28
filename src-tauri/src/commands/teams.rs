use crate::models::{TeamConfig, TeamInboxMessage, TeamSummary, TeamTask};
use crate::storage::teams;

pub fn list_teams() -> Result<Vec<TeamSummary>, String> {
    log::debug!("[teams] list_teams");
    Ok(teams::list_teams())
}

pub fn get_team_config(name: String) -> Result<TeamConfig, String> {
    log::debug!("[teams] get_team_config: {}", name);
    teams::get_team_config(&name).ok_or_else(|| format!("Team '{}' not found", name))
}

pub fn list_team_tasks(team_name: String) -> Result<Vec<TeamTask>, String> {
    log::debug!("[teams] list_team_tasks: {}", team_name);
    Ok(teams::list_team_tasks(&team_name))
}

pub fn get_team_task(team_name: String, task_id: String) -> Result<TeamTask, String> {
    log::debug!("[teams] get_team_task: {} #{}", team_name, task_id);
    teams::get_team_task(&team_name, &task_id)
        .ok_or_else(|| format!("Task '{}' not found in team '{}'", task_id, team_name))
}

pub fn get_team_inbox(
    team_name: String,
    agent_name: String,
) -> Result<Vec<TeamInboxMessage>, String> {
    log::debug!("[teams] get_team_inbox: {} / {}", team_name, agent_name);
    Ok(teams::get_team_inbox(&team_name, &agent_name))
}

pub fn get_all_team_inboxes(name: String) -> Result<Vec<TeamInboxMessage>, String> {
    log::debug!("[teams] get_all_team_inboxes: {}", name);
    Ok(teams::get_all_team_inboxes(&name))
}

pub fn delete_team(name: String) -> Result<(), String> {
    log::debug!("[teams] delete_team: {}", name);
    teams::delete_team(&name)
}
