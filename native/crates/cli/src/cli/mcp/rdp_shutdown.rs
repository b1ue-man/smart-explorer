//! Keep the process owner alive until its RDP clients and leases have closed.
pub(super) fn drain() {
    let mut last_error = None;
    loop {
        match crate::rdp::shutdown_owned_sessions() {
            Ok(()) => return,
            Err(error) => {
                if last_error.as_ref() != Some(&error) {
                    eprintln!("se mcp: RDP shutdown is still pending: {error}");
                    last_error = Some(error);
                }
                // The runtime remains alive: pending setup and owner monitors
                // must finish before the MCP process can release its resources.
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}
