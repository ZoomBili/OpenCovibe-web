use crate::agent::spawn::build_agent_command;
use crate::agent::stream::{run_codex_exec, ProcessMap};
use crate::models::{
    max_attachment_size, Attachment, AttachmentMeta, BusEvent, ConversationRef, ExecutionPath,
    RunEventType, RunStatus,
};
use crate::storage;
use crate::web_server::broadcaster::BroadcastEmitter;
use std::fs;
use std::sync::Arc;

fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    if cleaned.is_empty() {
        "attachment.bin".to_string()
    } else {
        cleaned
    }
}

fn extension_for_mime(mime: &str) -> &str {
    if mime.starts_with("image/png") {
        ".png"
    } else if mime.starts_with("image/jpeg") {
        ".jpg"
    } else if mime.starts_with("image/webp") {
        ".webp"
    } else if mime.starts_with("image/gif") {
        ".gif"
    } else if mime.starts_with("application/pdf") {
        ".pdf"
    } else if mime.starts_with("text/markdown") {
        ".md"
    } else if mime.starts_with("text/plain") {
        ".txt"
    } else if mime.contains("json") {
        ".json"
    } else {
        ""
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn send_chat_message(
    process_map: &ProcessMap,
    emitter: &Arc<BroadcastEmitter>,
    run_id: String,
    message: String,
    attachments: Option<Vec<Attachment>>,
    model: Option<String>,
    client_uuid: Option<String>,
) -> Result<(), String> {
    let run = storage::runs::get_run(&run_id).ok_or_else(|| format!("Run {} not found", run_id))?;
    if run.agent != "codex" {
        return Err("Web pipe execution is only supported for Codex".to_string());
    }
    if run.resolved_execution_path() != ExecutionPath::PipeExec {
        return Err(format!(
            "send_chat_message requires execution_path=pipe_exec for run {}",
            run_id
        ));
    }
    if run.conversation_ref.is_some()
        && storage::settings::get_agent_settings(&run.agent)
            .no_session_persistence
            .unwrap_or(false)
    {
        return Err("Cannot resume: session persistence is disabled".to_string());
    }

    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("message is required".to_string());
    }

    let mut attachment_paths = Vec::new();
    let mut attachment_metas = Vec::new();
    let attachments = attachments.unwrap_or_default();
    if !attachments.is_empty() {
        let upload_dir = std::env::temp_dir()
            .join("opencovibe-uploads")
            .join(&run_id);
        fs::create_dir_all(&upload_dir).map_err(|error| error.to_string())?;

        for attachment in attachments.iter().take(8) {
            if attachment.content_base64.is_empty() {
                continue;
            }
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&attachment.content_base64)
                .map_err(|error| error.to_string())?;
            if bytes.is_empty() || bytes.len() > max_attachment_size(&attachment.mime_type) as usize
            {
                continue;
            }
            let filename = format!(
                "{}-{}-{}{}",
                chrono::Utc::now().timestamp_millis(),
                &uuid::Uuid::new_v4().to_string()[..6],
                safe_filename(&attachment.name),
                extension_for_mime(&attachment.mime_type)
            );
            let full_path = upload_dir.join(filename);
            fs::write(&full_path, &bytes).map_err(|error| error.to_string())?;
            attachment_paths.push((
                full_path.to_string_lossy().to_string(),
                attachment.name.clone(),
                attachment.mime_type.clone(),
                attachment.size,
            ));
            attachment_metas.push(AttachmentMeta {
                name: attachment.name.clone(),
                mime_type: attachment.mime_type.clone(),
                size: attachment.size,
            });
        }
    }

    let attachment_text = if attachment_paths.is_empty() {
        String::new()
    } else {
        let files = attachment_paths
            .iter()
            .map(|(path, name, mime, size)| {
                format!("- {} ({}, {} bytes) => {}", name, mime, size, path)
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\nAttached files:\n{}\nUse these local file paths directly when needed.",
            files
        )
    };
    let full_prompt = format!("{}{}", message, attachment_text);

    let attachment_events = attachment_paths
        .iter()
        .map(|(path, name, mime, size)| {
            serde_json::json!({ "name": name, "type": mime, "size": size, "path": path })
        })
        .collect::<Vec<_>>();
    if let Err(error) = storage::events::append_event(
        &run_id,
        RunEventType::User,
        serde_json::json!({
            "text": message,
            "source": "web_chat",
            "attachments": attachment_events
        }),
    ) {
        log::warn!("[chat] failed to persist user event: {}", error);
    }
    emitter.persist_and_emit(
        &run_id,
        &BusEvent::UserMessage {
            run_id: run_id.clone(),
            text: message,
            uuid: None,
            client_uuid,
            attachments: attachment_metas,
        },
    );
    storage::runs::update_status(&run_id, RunStatus::Running, None, None)?;

    let agent_settings = storage::settings::get_agent_settings(&run.agent);
    let user_settings = storage::settings::get_user_settings();
    let adapter_settings =
        crate::agent::adapter::build_adapter_settings(&agent_settings, &user_settings, model);
    let resume_thread_id = run
        .conversation_ref
        .as_ref()
        .and_then(|reference| match reference {
            ConversationRef::CodexThread(thread_id) => Some(thread_id.as_str()),
            _ => None,
        });
    let image_paths = attachment_paths
        .iter()
        .filter(|(_, _, mime, _)| mime.starts_with("image/"))
        .map(|(path, _, _, _)| path.clone())
        .collect::<Vec<_>>();
    let (command, args) = build_agent_command(
        "codex",
        &full_prompt,
        &adapter_settings,
        true,
        resume_thread_id,
        &image_paths,
    )?;

    if let Some(codex_model) = args
        .iter()
        .position(|argument| argument == "--model")
        .and_then(|index| args.get(index + 1))
    {
        let _ = storage::runs::update_run_model(&run_id, codex_model);
    }

    let mut extra_env = std::collections::HashMap::new();
    if let Some(provider) = &adapter_settings.codex_provider {
        if let Some((key, value)) = crate::agent::spawn::codex_provider_env(provider) {
            extra_env.insert(key, value);
        }
    }

    let process_map = process_map.clone();
    let emitter = emitter.clone();
    let run_id_for_task = run_id.clone();
    tokio::spawn(async move {
        if let Err(error) = run_codex_exec(
            process_map,
            emitter.clone(),
            run_id_for_task.clone(),
            command,
            args,
            run.cwd,
            extra_env,
        )
        .await
        {
            let _ = storage::runs::update_status(
                &run_id_for_task,
                RunStatus::Failed,
                Some(1),
                Some(error.clone()),
            );
            emitter.persist_and_emit(
                &run_id_for_task,
                &BusEvent::RunState {
                    run_id: run_id_for_task.clone(),
                    state: "failed".to_string(),
                    exit_code: Some(1),
                    error: Some(error),
                },
            );
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filename_handles_unicode_without_byte_slicing() {
        let name = "文件".repeat(100);
        assert_eq!(safe_filename(&name).chars().count(), 120);
    }

    #[test]
    fn empty_filename_uses_fallback() {
        assert_eq!(safe_filename(""), "attachment.bin");
    }
}
