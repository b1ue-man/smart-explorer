//! Preserve this owner and its runtime until every login viewer has closed.
pub(super) fn drain() {
    let mut last_error = None;
    loop {
        match super::login_tool::shutdown() {
            Ok(_) => return,
            Err(error) => {
                if last_error.as_ref() != Some(&error) {
                    eprintln!("se mcp: login shutdown is still pending: {error}");
                    last_error = Some(error);
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}
