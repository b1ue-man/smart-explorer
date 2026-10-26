//! Keep the daemon runtime alive until all login handlers and tasks are gone.
pub(super) fn drain() {
    let mut last_error = None;
    loop {
        match crate::daemon::shutdown_login_viewers() {
            Ok(_) => return,
            Err(error) => {
                let message = format!("daemon stop: login viewers still closing: {error}");
                if last_error.as_ref() != Some(&message) {
                    crate::daemon::log_worker(&message);
                    last_error = Some(message);
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}
