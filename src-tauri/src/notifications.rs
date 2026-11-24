//! Outbound notifications — WeCom (企业微信) group robots and custom HTTP endpoints.
//!
//! Triggered from [`crate::web_server::broadcaster::BroadcastEmitter::persist_and_emit`]
//! for the A-class bus events that users care about:
//!
//! - `RunState { state: "idle" | "completed" }` → turn/run finished
//! - `RunState { state: "failed" }` → run failed
//! - `PermissionPrompt` → the agent is waiting for approval
//!
//! Delivery is best-effort and runs on a detached tokio task, so a slow or
//! unreachable endpoint never blocks the agent loop. All configuration lives in
//! `~/.opencovibe/settings.json` under `user.notifications`.

use crate::models::{BusEvent, HttpNotification, NotificationSettings, WecomNotification};
use serde_json::{json, Value};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_URL_LEN: usize = 2048;
const MAX_HEADER_COUNT: usize = 20;
const MAX_BODY_BYTES: usize = 64 * 1024;
/// WeCom text messages are capped at 2048 **bytes**, so truncate by bytes
/// (UTF-8 boundary aware) rather than characters — 1800 Chinese chars = 5.4 KB.
const MAX_WECOM_TEXT_BYTES: usize = 1800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotifyKind {
    Complete,
    Error,
    Permission,
}

impl NotifyKind {
    fn event_name(self) -> &'static str {
        match self {
            Self::Complete => "run_completed",
            Self::Error => "run_failed",
            Self::Permission => "permission_required",
        }
    }

    fn status(self) -> &'static str {
        match self {
            Self::Complete => "completed",
            Self::Error => "failed",
            Self::Permission => "waiting_for_approval",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Complete => "✅",
            Self::Error => "❌",
            Self::Permission => "⏳",
        }
    }

    fn status_label(self, zh: bool) -> &'static str {
        match (self, zh) {
            (Self::Complete, true) => "任务完成",
            (Self::Complete, false) => "Task completed",
            (Self::Error, true) => "任务失败",
            (Self::Error, false) => "Task failed",
            (Self::Permission, true) => "等待审批",
            (Self::Permission, false) => "Approval required",
        }
    }
}

/// Data exposed to notification templates.
///
/// Placeholders: `{event} {title} {status} {run_id} {run_name} {cwd} {agent}
/// {message} {run_url} {time} {project} {session} {turn} {turn_preview}
/// {status_label} {duration} {tokens_m} {text} {text_body}`.
#[derive(Clone, Debug)]
struct NotificationPayload {
    event: &'static str,
    title: String,
    status: &'static str,
    run_id: String,
    run_name: String,
    cwd: String,
    agent: String,
    message: String,
    run_url: String,
    time: String,
    zh: bool,
    // ── Rendered notification fields (Scheme D) ──
    icon: &'static str,
    project: String,
    session: String,
    turn_label: String,
    turn_preview: String,
    status_label: String,
    duration: String,
    tokens_m: String,
    /// Full multi-line text, including the `🤖 AI Coding` title line.
    text: String,
    /// Multi-line body without the title line.
    text_body: String,
}

impl NotificationPayload {
    fn field(&self, key: &str) -> Option<&str> {
        Some(match key {
            "event" => self.event,
            "title" => self.title.as_str(),
            "status" => self.status,
            "run_id" => self.run_id.as_str(),
            "run_name" => self.run_name.as_str(),
            "cwd" => self.cwd.as_str(),
            "agent" => self.agent.as_str(),
            "message" => self.message.as_str(),
            "run_url" => self.run_url.as_str(),
            "time" => self.time.as_str(),
            "project" => self.project.as_str(),
            "session" => self.session.as_str(),
            "turn" => self.turn_label.as_str(),
            "turn_preview" => self.turn_preview.as_str(),
            "status_label" => self.status_label.as_str(),
            "duration" => self.duration.as_str(),
            "tokens_m" => self.tokens_m.as_str(),
            "text" => self.text.as_str(),
            "text_body" => self.text_body.as_str(),
            _ => return None,
        })
    }
}

/// Inspect a live bus event and dispatch notifications when configured.
///
/// Called from `persist_and_emit` only (not from replay/import), so imported CLI
/// history never triggers outbound messages.
pub fn on_bus_event(run_id: &str, event: &BusEvent) {
    let (kind, error, tool_name) = match event {
        BusEvent::RunState { state, error, .. } => {
            let kind = match state.as_str() {
                "idle" | "completed" => NotifyKind::Complete,
                "failed" => NotifyKind::Error,
                _ => return,
            };
            (kind, error.clone(), None)
        }
        BusEvent::PermissionPrompt { tool_name, .. } => {
            (NotifyKind::Permission, None, Some(tool_name.clone()))
        }
        _ => return,
    };

    // Read settings only for the few event kinds that can notify.
    let settings = crate::storage::settings::get_user_settings().notifications;
    if !settings.enabled || !allows(&settings, kind) {
        return;
    }

    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        log::debug!(
            "[notify] no tokio runtime; skipping event={}",
            kind.event_name()
        );
        return;
    };

    let run_id = run_id.to_string();
    handle.spawn(async move {
        let payload = build_payload(&run_id, kind, error, tool_name, &settings);
        deliver_all(&payload, &settings).await;
    });
}

fn allows(settings: &NotificationSettings, kind: NotifyKind) -> bool {
    match kind {
        NotifyKind::Complete => settings.on_complete,
        NotifyKind::Error => settings.on_error,
        NotifyKind::Permission => settings.on_permission,
    }
}

fn build_payload(
    run_id: &str,
    kind: NotifyKind,
    error: Option<String>,
    tool_name: Option<String>,
    settings: &NotificationSettings,
) -> NotificationPayload {
    let zh = settings.language != "en";
    let meta = crate::storage::runs::get_run(run_id);

    let (title, fallback_message) = match (kind, zh) {
        (NotifyKind::Complete, true) => (
            "OpenCovibe · 任务完成",
            "本轮对话已完成，等待你的下一步指令。",
        ),
        (NotifyKind::Complete, false) => (
            "OpenCovibe · Task completed",
            "The agent finished this turn.",
        ),
        (NotifyKind::Error, true) => ("OpenCovibe · 任务失败", "运行失败，请打开会话查看详情。"),
        (NotifyKind::Error, false) => (
            "OpenCovibe · Task failed",
            "The run failed. Open the session for details.",
        ),
        (NotifyKind::Permission, true) => {
            ("OpenCovibe · 等待审批", "Agent 正在等待你批准工具调用。")
        }
        (NotifyKind::Permission, false) => (
            "OpenCovibe · Approval required",
            "The agent is waiting for tool approval.",
        ),
    };

    let message = match kind {
        NotifyKind::Complete => fallback_message.to_string(),
        NotifyKind::Error => error
            .filter(|e| !e.trim().is_empty())
            .or_else(|| meta.as_ref().and_then(|m| m.error_message.clone()))
            .unwrap_or_else(|| fallback_message.to_string()),
        NotifyKind::Permission => match tool_name.filter(|t| !t.trim().is_empty()) {
            Some(tool) if zh => format!("工具调用「{}」等待审批。", tool),
            Some(tool) => format!("Tool call \"{}\" is waiting for approval.", tool),
            None => fallback_message.to_string(),
        },
    };

    let raw_session = meta
        .as_ref()
        .and_then(|m| m.name.clone())
        .filter(|n| !n.trim().is_empty())
        .or_else(|| {
            meta.as_ref()
                .map(|m| first_line(&m.prompt, 80))
                .filter(|p| !p.is_empty())
        })
        .unwrap_or_default();
    let session = strip_leading_date(&raw_session);

    let cwd = meta.as_ref().map(|m| m.cwd.clone()).unwrap_or_default();
    let project = display_project(&cwd, &settings.project_path_mode);
    let agent = meta.as_ref().map(|m| m.agent.clone()).unwrap_or_default();

    let (turn_index, turn_text) =
        crate::storage::events::last_user_turn(run_id).unwrap_or((1, String::new()));
    let turn_label = if zh {
        format!("第 {turn_index} 轮")
    } else {
        format!("Turn {turn_index}")
    };
    let turn_preview = first_line_normalized(&turn_text, 40);

    let usage = crate::storage::events::extract_run_usage(run_id);
    let duration_ms = usage
        .as_ref()
        .map(|u| u.duration_ms)
        .filter(|ms| *ms > 0)
        .unwrap_or_else(|| elapsed_ms(meta.as_ref()));
    let total_tokens = usage
        .as_ref()
        .map(|u| u.input_tokens.saturating_add(u.output_tokens))
        .unwrap_or(0);

    let run_url = settings
        .link_base_url
        .as_deref()
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .map(|base| {
            let base = base.trim_end_matches('/');
            format!("{}/chat?run={}", base, run_id)
        })
        .unwrap_or_default();

    finalize_text(NotificationPayload {
        event: kind.event_name(),
        title: title.to_string(),
        status: kind.status(),
        run_id: run_id.to_string(),
        run_name: raw_session,
        cwd,
        agent,
        message,
        run_url,
        time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        zh,
        icon: kind.icon(),
        project,
        session,
        turn_label,
        turn_preview,
        status_label: kind.status_label(zh).to_string(),
        duration: format_duration(duration_ms, zh),
        tokens_m: format_tokens(total_tokens),
        text: String::new(),
        text_body: String::new(),
    })
}

fn first_line(text: &str, max_chars: usize) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    truncate_chars(first, max_chars)
}

/// First line with runs of whitespace collapsed, then truncated.
fn first_line_normalized(text: &str, max_chars: usize) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let normalized = first.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&normalized, max_chars)
}

/// Project line: full cwd or just its last path component.
fn display_project(cwd: &str, mode: &str) -> String {
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return String::new();
    }
    if mode == "full" {
        return cwd.to_string();
    }
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if base.is_empty() {
        cwd.to_string()
    } else {
        base.to_string()
    }
}

/// Remove a leading `YYYY-MM-DD` / `YYYY/MM/DD` / `YYYY.MM.DD` /
/// `YYYY年MM月DD日` (optionally bracketed) from a session name.
fn strip_leading_date(name: &str) -> String {
    let s = name.trim();
    let s = s.strip_prefix('[').unwrap_or(s);
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 8 || !chars[..4].iter().all(|c| c.is_ascii_digit()) {
        return s.to_string();
    }

    let mut i = 4;
    if chars.get(i) == Some(&'年') {
        i += 1;
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || chars.get(i) != Some(&'月') {
            return s.to_string();
        }
        i += 1;
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || chars.get(i) != Some(&'日') {
            return s.to_string();
        }
        i += 1;
    } else {
        let sep = match chars.get(i) {
            Some(&'-') => '-',
            Some(&'/') => '/',
            Some(&'.') => '.',
            _ => return s.to_string(),
        };
        i += 1;
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || chars.get(i) != Some(&sep) {
            return s.to_string();
        }
        i += 1;
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return s.to_string();
        }
    }

    while i < chars.len()
        && (chars[i].is_whitespace()
            || matches!(
                chars[i],
                '-' | '–' | '—' | '_' | ':' | '：' | '|' | '·' | ']'
            ))
    {
        i += 1;
    }
    let rest: String = chars[i..].iter().collect();
    let rest = rest.trim();
    if rest.is_empty() {
        s.to_string()
    } else {
        rest.to_string()
    }
}

fn format_duration(ms: u64, zh: bool) -> String {
    if ms < 60_000 {
        return if zh {
            "<1 分钟".to_string()
        } else {
            "<1 min".to_string()
        };
    }
    let minutes = ((ms as f64) / 60_000.0).round() as u64;
    let hours = minutes / 60;
    let mins = minutes % 60;
    if hours > 0 {
        if mins > 0 {
            if zh {
                format!("{hours} 小时 {mins} 分钟")
            } else {
                format!("{hours} h {mins} min")
            }
        } else if zh {
            format!("{hours} 小时")
        } else {
            format!("{hours} h")
        }
    } else if zh {
        format!("{minutes} 分钟")
    } else {
        format!("{minutes} min")
    }
}

/// Input + output tokens, formatted in millions (`1.28M`).
fn format_tokens(total: u64) -> String {
    if total == 0 {
        return "0M".to_string();
    }
    let millions = total as f64 / 1_000_000.0;
    if millions < 0.01 {
        "<0.01M".to_string()
    } else if millions >= 10.0 {
        format!("{millions:.1}M")
    } else {
        format!("{millions:.2}M")
    }
}

fn elapsed_ms(meta: Option<&crate::models::RunMeta>) -> u64 {
    let Some(meta) = meta else {
        return 0;
    };
    let Ok(started) = chrono::DateTime::parse_from_rfc3339(&meta.started_at) else {
        return 0;
    };
    let ended = meta
        .ended_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);
    let started = started.with_timezone(&chrono::Utc);
    (ended - started).num_milliseconds().max(0) as u64
}

/// Scheme D body (without the `🤖 AI Coding` title line):
///
/// ```text
/// 📁 工程：OpenCovibe-web-main
/// 💬 会话：通知推送优化
/// 🔄 轮次：第 3 轮 · 修复登录接口报错
/// ✅ 任务完成 · ⏱️ 3 分钟
/// 🔢 1.28M
/// ```
fn render_text_body(payload: &NotificationPayload) -> String {
    let sep = if payload.zh { "：" } else { ": " };
    let project_label = if payload.zh { "工程" } else { "Project" };
    let session_label = if payload.zh { "会话" } else { "Session" };
    let turn_label = if payload.zh { "轮次" } else { "Turn" };

    let mut lines = Vec::with_capacity(5);
    if !payload.project.is_empty() {
        lines.push(format!("📁 {project_label}{sep}{}", payload.project));
    }
    if !payload.session.is_empty() {
        lines.push(format!("💬 {session_label}{sep}{}", payload.session));
    }
    let turn_line = if payload.turn_preview.is_empty() {
        format!("🔄 {turn_label}{sep}{}", payload.turn_label)
    } else {
        format!(
            "🔄 {turn_label}{sep}{} · {}",
            payload.turn_label, payload.turn_preview
        )
    };
    lines.push(turn_line);
    lines.push(format!(
        "{} {} · ⏱️ {}",
        payload.icon, payload.status_label, payload.duration
    ));
    lines.push(format!("🔢 {}", payload.tokens_m));
    lines.join("\n")
}

fn finalize_text(mut payload: NotificationPayload) -> NotificationPayload {
    payload.text_body = render_text_body(&payload);
    payload.text = format!("🤖 AI Coding\n{}", payload.text_body);
    payload
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let end = text
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    format!("{}…", &text[..end])
}

async fn deliver_all(payload: &NotificationPayload, settings: &NotificationSettings) {
    if settings.wecom.enabled {
        match send_wecom(&settings.wecom, payload).await {
            Ok(()) => log::info!("[notify] wecom sent event={}", payload.event),
            Err(e) => log::warn!("[notify] wecom failed event={}: {}", payload.event, e),
        }
    }
    if settings.http.enabled {
        match send_http(&settings.http, payload).await {
            Ok(()) => log::info!("[notify] http sent event={}", payload.event),
            Err(e) => log::warn!("[notify] http failed event={}: {}", payload.event, e),
        }
    }
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

fn validate_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL is empty".to_string());
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err(format!("URL is too long (max {MAX_URL_LEN})"));
    }
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported URL scheme: {other}")),
    }
    if parsed.host_str().unwrap_or("").is_empty() {
        return Err("URL has no host".to_string());
    }
    Ok(trimmed.to_string())
}

fn wecom_text(payload: &NotificationPayload) -> String {
    truncate_utf8_bytes(&payload.text, MAX_WECOM_TEXT_BYTES)
}

fn truncate_utf8_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

async fn send_wecom(cfg: &WecomNotification, payload: &NotificationPayload) -> Result<(), String> {
    let url = validate_url(&cfg.webhook_url)?;

    let mut text = json!({ "content": wecom_text(payload) });
    let mentions: Vec<&str> = cfg
        .mentioned_mobiles
        .iter()
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .collect();
    if !mentions.is_empty() {
        text["mentioned_mobile_list"] = json!(mentions);
    }
    let body = json!({ "msgtype": "text", "text": text });

    let resp = http_client()?
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "WeCom HTTP {}: {}",
            status,
            truncate_chars(&body_text, 300)
        ));
    }
    if let Ok(v) = serde_json::from_str::<Value>(&body_text) {
        if let Some(code) = v.get("errcode").and_then(|c| c.as_i64()) {
            if code != 0 {
                let msg = v.get("errmsg").and_then(|m| m.as_str()).unwrap_or("");
                return Err(format!("WeCom errcode {}: {}", code, msg));
            }
        }
    }
    Ok(())
}

async fn send_http(cfg: &HttpNotification, payload: &NotificationPayload) -> Result<(), String> {
    let url = validate_url(&render_template(&cfg.url, payload, false))?;

    let method_name = cfg.method.trim().to_ascii_uppercase();
    let method = match method_name.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        other => return Err(format!("unsupported HTTP method: {other}")),
    };

    let client = http_client()?;
    let mut req = client.request(method.clone(), &url);
    let mut has_content_type = false;

    for header in cfg.headers.iter().take(MAX_HEADER_COUNT) {
        let key = header.key.trim();
        if key.is_empty() {
            continue;
        }
        let Ok(name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) else {
            log::debug!("[notify] skipping invalid header name: {}", key);
            continue;
        };
        let Ok(value) = reqwest::header::HeaderValue::from_str(header.value.as_str()) else {
            log::debug!("[notify] skipping invalid header value for: {}", key);
            continue;
        };
        if name == reqwest::header::CONTENT_TYPE {
            has_content_type = true;
        }
        req = req.header(name, value);
    }

    if method != reqwest::Method::GET && method != reqwest::Method::HEAD {
        let content_type = cfg
            .headers
            .iter()
            .find(|h| h.key.eq_ignore_ascii_case("content-type"))
            .map(|h| h.value.to_ascii_lowercase());
        let is_json = content_type
            .as_deref()
            .map(|c| c.contains("json"))
            .unwrap_or(true);

        let body = if cfg.body_template.trim().is_empty() {
            default_body(payload)
        } else {
            render_template(&cfg.body_template, payload, is_json)
        };
        // Fail locally with an actionable message instead of letting the remote
        // endpoint return an opaque 500. The most common mistake is an unquoted
        // placeholder in a JSON template: `"title": {title}`.
        if is_json && !body.trim().is_empty() {
            if let Err(e) = serde_json::from_str::<Value>(&body) {
                return Err(format!(
                    "request body is not valid JSON: {e}. In JSON templates, placeholders must be inside quotes, e.g. \"title\": \"{{title}}\"."
                ));
            }
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(format!(
                "notification body is too large ({} bytes, max {MAX_BODY_BYTES})",
                body.len()
            ));
        }
        if !has_content_type {
            req = req.header(
                reqwest::header::CONTENT_TYPE,
                if is_json {
                    "application/json"
                } else {
                    "text/plain; charset=utf-8"
                },
            );
        }
        req = req.body(body);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "HTTP {}: {}",
            status,
            truncate_chars(&body_text, 300)
        ));
    }
    Ok(())
}

fn default_body(payload: &NotificationPayload) -> String {
    serde_json::to_string(&json!({
        "event": payload.event,
        "title": payload.title,
        "status": payload.status,
        "run_id": payload.run_id,
        "run_name": payload.run_name,
        "cwd": payload.cwd,
        "agent": payload.agent,
        "message": payload.message,
        "run_url": payload.run_url,
        "timestamp": payload.time,
        "project": payload.project,
        "session": payload.session,
        "turn": payload.turn_label,
        "turn_preview": payload.turn_preview,
        "status_label": payload.status_label,
        "duration": payload.duration,
        "tokens_m": payload.tokens_m,
        "text": payload.text,
        "text_body": payload.text_body,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

/// Replace `{placeholder}` tokens in one pass.
///
/// When `json_escape` is true, values are escaped for insertion inside a JSON
/// string literal (the surrounding quotes are expected to come from the template).
fn render_template(template: &str, payload: &NotificationPayload, json_escape: bool) -> String {
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            let key = &after[..end];
            if let Some(value) = payload.field(key) {
                if json_escape {
                    out.push_str(&json_escape_inner(value));
                } else {
                    out.push_str(value);
                }
                rest = &after[end + 1..];
                continue;
            }
        }
        // Unknown/incomplete placeholder: keep the `{` and continue scanning.
        out.push('{');
        rest = after;
    }

    out.push_str(rest);
    out
}

fn json_escape_inner(value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());
    quoted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or("")
        .to_string()
}

fn test_payload(settings: &NotificationSettings) -> NotificationPayload {
    let zh = settings.language != "en";
    finalize_text(NotificationPayload {
        event: "test",
        title: if zh {
            "OpenCovibe · 测试通知".to_string()
        } else {
            "OpenCovibe · Test notification".to_string()
        },
        status: "test",
        run_id: String::new(),
        run_name: if zh {
            "通知推送优化".to_string()
        } else {
            "Notification push".to_string()
        },
        cwd: "/vol1/1000/aicoding/OpenCovibe-web-main".to_string(),
        agent: String::new(),
        message: if zh {
            "如果你收到这条消息，说明通知配置可用。".to_string()
        } else {
            "If you received this, your notification channel works.".to_string()
        },
        run_url: settings
            .link_base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("")
            .to_string(),
        time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        zh,
        icon: "✅",
        project: display_project(
            "/vol1/1000/aicoding/OpenCovibe-web-main",
            &settings.project_path_mode,
        ),
        session: if zh {
            "通知推送优化".to_string()
        } else {
            "Notification push".to_string()
        },
        turn_label: if zh {
            "第 3 轮".to_string()
        } else {
            "Turn 3".to_string()
        },
        turn_preview: if zh {
            "修复登录接口报错".to_string()
        } else {
            "Fix login API error".to_string()
        },
        status_label: if zh {
            "任务完成".to_string()
        } else {
            "Task completed".to_string()
        },
        duration: if zh {
            "3 分钟".to_string()
        } else {
            "3 min".to_string()
        },
        tokens_m: "1.28M".to_string(),
        text: String::new(),
        text_body: String::new(),
    })
}

/// Send a test message through one channel, bypassing the master switch.
///
/// The UI passes its current draft config so users can test before saving.
pub async fn send_test(channel: &str, settings: &NotificationSettings) -> Result<(), String> {
    let payload = test_payload(settings);
    match channel {
        "wecom" => send_wecom(&settings.wecom, &payload).await,
        "http" => send_http(&settings.http, &payload).await,
        other => Err(format!("unknown notification channel: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> NotificationPayload {
        finalize_text(NotificationPayload {
            event: "run_completed",
            title: "Title".to_string(),
            status: "completed",
            run_id: "run-1".to_string(),
            run_name: "Fix login".to_string(),
            cwd: "/work/app".to_string(),
            agent: "claude".to_string(),
            message: "Done".to_string(),
            run_url: "https://example.com/chat?run=run-1".to_string(),
            time: "2026-01-02 03:04:05".to_string(),
            zh: false,
            icon: "✅",
            project: "OpenCovibe-web-main".to_string(),
            session: "Fix login".to_string(),
            turn_label: "Turn 3".to_string(),
            turn_preview: "Fix login API error".to_string(),
            status_label: "Task completed".to_string(),
            duration: "3 min".to_string(),
            tokens_m: "1.28M".to_string(),
            text: String::new(),
            text_body: String::new(),
        })
    }

    #[test]
    fn renders_known_placeholders_in_one_pass() {
        let p = payload();
        assert_eq!(
            render_template("{run_name} @ {cwd} #{run_id}", &p, false),
            "Fix login @ /work/app #run-1"
        );
    }

    #[test]
    fn keeps_unknown_placeholders_verbatim() {
        let p = payload();
        assert_eq!(
            render_template("{unknown} {run_name}", &p, false),
            "{unknown} Fix login"
        );
    }

    #[test]
    fn json_escapes_values_when_requested() {
        let mut p = payload();
        p.run_name = "a\"b\nc".to_string();
        assert_eq!(
            render_template("{\"name\":\"{run_name}\"}", &p, true),
            "{\"name\":\"a\\\"b\\nc\"}"
        );
    }

    #[test]
    fn default_body_is_valid_json() {
        let raw = default_body(&payload());
        let v: Value = serde_json::from_str(&raw).expect("valid json");
        assert_eq!(v["event"], "run_completed");
        assert_eq!(v["run_name"], "Fix login");
    }

    #[test]
    fn rejects_non_http_urls() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("not a url").is_err());
        assert!(validate_url("https://example.com/hook").is_ok());
    }

    #[test]
    fn notification_settings_default_to_disabled() {
        let s: NotificationSettings = serde_json::from_str("{}").expect("deserialize");
        assert!(!s.enabled);
        assert!(s.on_complete && s.on_error && s.on_permission);
        assert!(!s.wecom.enabled && !s.http.enabled);
        assert_eq!(s.http.method, "POST");
        assert_eq!(s.project_path_mode, "basename");
    }

    #[test]
    fn truncate_chars_respects_unicode_boundaries() {
        assert_eq!(truncate_chars("你好世界", 2), "你好…");
        assert_eq!(truncate_chars("abc", 5), "abc");
    }

    #[test]
    fn truncate_utf8_bytes_splits_on_char_boundary() {
        let text = "你".repeat(1000); // 3000 bytes
        let truncated = truncate_utf8_bytes(&text, 1800);
        assert_eq!(truncated.chars().count(), 601); // 600 chars + ellipsis
        assert_eq!(truncated.len(), 1803); // 1800 bytes + 3-byte ellipsis
    }

    #[test]
    fn wecom_text_contains_run_details() {
        let text = wecom_text(&payload());
        assert!(text.starts_with("🤖 AI Coding\n"), "{text}");
        assert!(text.contains("📁 Project: OpenCovibe-web-main"), "{text}");
        assert!(text.contains("💬 Session: Fix login"), "{text}");
        assert!(
            text.contains("🔄 Turn: Turn 3 · Fix login API error"),
            "{text}"
        );
        assert!(text.contains("✅ Task completed · ⏱️ 3 min"), "{text}");
        assert!(text.contains("🔢 1.28M"), "{text}");
    }

    #[test]
    fn scheme_d_text_layout() {
        let mut p = payload();
        p.zh = true;
        p.project = "OpenCovibe-web-main".to_string();
        p.session = "通知推送优化".to_string();
        p.turn_label = "第 3 轮".to_string();
        p.turn_preview = "修复登录接口报错".to_string();
        p.status_label = "任务完成".to_string();
        p.duration = "3 分钟".to_string();
        p.tokens_m = "1.28M".to_string();
        let p = finalize_text(p);

        assert_eq!(
            p.text_body,
            "📁 工程：OpenCovibe-web-main\n\
             💬 会话：通知推送优化\n\
             🔄 轮次：第 3 轮 · 修复登录接口报错\n\
             ✅ 任务完成 · ⏱️ 3 分钟\n\
             🔢 1.28M"
        );
        assert_eq!(p.text, format!("🤖 AI Coding\n{}", p.text_body));
    }

    #[test]
    fn strips_leading_date_prefixes() {
        assert_eq!(
            strip_leading_date("2026-11-24 通知推送优化"),
            "通知推送优化"
        );
        assert_eq!(
            strip_leading_date("2026/11/24 通知推送优化"),
            "通知推送优化"
        );
        assert_eq!(strip_leading_date("2026.11.24 通知"), "通知");
        assert_eq!(strip_leading_date("2026年11月24日 通知"), "通知");
        assert_eq!(strip_leading_date("[2026-11-24] 通知"), "通知");
        assert_eq!(strip_leading_date("通知推送优化"), "通知推送优化");
        // Date-only names are kept rather than becoming empty.
        assert_eq!(strip_leading_date("2026-11-24"), "2026-11-24");
    }

    #[test]
    fn project_display_mode() {
        let cwd = "/vol1/1000/aicoding/OpenCovibe-web-main";
        assert_eq!(display_project(cwd, "basename"), "OpenCovibe-web-main");
        assert_eq!(display_project(cwd, "full"), cwd);
        assert_eq!(display_project("/work/app/", "basename"), "app");
        assert_eq!(display_project("", "basename"), "");
    }

    #[test]
    fn formats_duration_and_tokens() {
        assert_eq!(format_duration(30_000, true), "<1 分钟");
        assert_eq!(format_duration(180_000, true), "3 分钟");
        assert_eq!(format_duration(3_900_000, true), "1 小时 5 分钟");
        assert_eq!(format_duration(180_000, false), "3 min");

        assert_eq!(format_tokens(0), "0M");
        assert_eq!(format_tokens(12_000), "0.01M");
        assert_eq!(format_tokens(1_280_000), "1.28M");
        assert_eq!(format_tokens(12_800_000), "12.8M");
    }

    /// Minimal one-request HTTP server used by the delivery tests.
    async fn spawn_capture_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            let mut header_end: Option<usize> = None;
            let mut content_length = 0usize;
            loop {
                let n = socket.read(&mut buf).await.expect("read");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);

                if header_end.is_none() {
                    if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = Some(pos + 4);
                        let headers = String::from_utf8_lossy(&request[..pos]);
                        for line in headers.lines() {
                            if let Some(value) =
                                line.to_ascii_lowercase().strip_prefix("content-length:")
                            {
                                content_length = value.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                }
                if let Some(start) = header_end {
                    if request.len() >= start + content_length {
                        break;
                    }
                }
            }
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
            String::from_utf8_lossy(&request).to_string()
        });
        (addr, handle)
    }

    #[tokio::test]
    async fn http_channel_posts_json_body() {
        let (addr, handle) = spawn_capture_server().await;
        let mut settings = NotificationSettings::default();
        settings.http.url = format!("http://{addr}/hook");

        send_test("http", &settings).await.expect("send");
        let request = handle.await.expect("server");

        assert!(request.starts_with("POST /hook"), "{request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{request}"
        );
        assert!(request.contains("\"event\":\"test\""), "{request}");
    }

    #[tokio::test]
    async fn wecom_channel_posts_text_message() {
        let (addr, handle) = spawn_capture_server().await;
        let mut settings = NotificationSettings::default();
        settings.wecom.webhook_url = format!("http://{addr}/wecom");

        send_test("wecom", &settings).await.expect("send");
        let request = handle.await.expect("server");

        assert!(request.starts_with("POST /wecom"), "{request}");
        assert!(request.contains("\"msgtype\":\"text\""), "{request}");
        assert!(request.contains("\"content\""), "{request}");
    }

    #[tokio::test]
    async fn rejects_unquoted_placeholder_before_sending() {
        let mut settings = NotificationSettings::default();
        // Port 1 is never reachable; validation must fail before any network I/O.
        settings.http.url = "http://127.0.0.1:1/hook".to_string();
        settings.http.body_template = "{\"title\": {title}}".to_string();

        let err = send_test("http", &settings).await.expect_err("must reject");
        assert!(err.contains("not valid JSON"), "{err}");
        assert!(err.contains("inside quotes"), "{err}");
    }

    #[tokio::test]
    async fn quoted_placeholders_produce_valid_json_body() {
        let (addr, handle) = spawn_capture_server().await;
        let mut settings = NotificationSettings::default();
        settings.http.url = format!("http://{addr}/hook");
        settings.http.body_template =
            "{\"title\": \"{title}\", \"content\": \"{status}\", \"type\": \"text\"}".to_string();

        send_test("http", &settings).await.expect("send");
        let request = handle.await.expect("server");
        let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
        let parsed: Value = serde_json::from_str(body).expect("valid json");
        assert_eq!(parsed["content"], "test");
        assert_eq!(parsed["type"], "text");
        assert!(parsed["title"].as_str().is_some());
    }
}
