pub mod broker;
pub mod capture_admission;
pub mod capture_lifetime;
pub mod child;
pub mod child_reader;
pub mod client;
pub mod helper;
pub mod helper_capture;
pub mod installation;
use std::io;
pub fn denied(reason: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason.to_string())
}
pub fn now() -> io::Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
}
pub fn nonce() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub mod gateway;
pub mod gateway_client;
pub mod host_status;
pub mod provision;
pub mod self_channel;
pub mod self_install;
pub mod self_setup;
pub mod self_watch;
pub mod status;
pub mod target;
pub mod viewer;
pub mod viewer_images;
