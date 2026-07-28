pub fn notify_if_background(_app: &(), title: &str, body: &str) {
    log::info!("[notification] {}: {}", title, body);
}
