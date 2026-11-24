use crate::models::{NotificationSettings, NotificationTestResult};

/// Send a test notification through WeCom or the custom HTTP channel.
///
/// Accepts the UI's current (possibly unsaved) config so users can verify a
/// webhook before persisting it. The master switch is intentionally ignored.
pub async fn test_notification_channel(
    channel: String,
    settings: NotificationSettings,
) -> Result<NotificationTestResult, String> {
    log::debug!("[notifications] test channel={}", channel);
    match crate::notifications::send_test(&channel, &settings).await {
        Ok(()) => Ok(NotificationTestResult {
            ok: true,
            message: "ok".to_string(),
        }),
        Err(e) => {
            log::warn!("[notifications] test failed channel={}: {}", channel, e);
            Ok(NotificationTestResult {
                ok: false,
                message: e,
            })
        }
    }
}
