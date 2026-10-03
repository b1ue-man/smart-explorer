//! freedesktop Notifications on the user's session bus (no shell command).
use crate::syncjobs::ProblemNotice;
pub(super) fn notify(notice: &ProblemNotice) -> Result<(), String> {
    let connection = zbus::blocking::Connection::session().map_err(|error| error.to_string())?;
    let hints: std::collections::HashMap<&str, zbus::zvariant::Value<'_>> = std::collections::HashMap::new();
    let body = notice.text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let reply = connection.call_method(Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications", Some("org.freedesktop.Notifications"), "Notify",
        &("Smart Explorer", 0u32, "dialog-warning", notice.title.as_str(), body.as_str(), Vec::<String>::new(), hints, -1i32))
        .map_err(|error| error.to_string())?;
    reply.body().deserialize::<u32>().map(|_| ()).map_err(|error| error.to_string())
}
