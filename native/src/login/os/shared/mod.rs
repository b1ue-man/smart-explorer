pub(crate) mod broker;
pub(crate) mod capture_admission;
pub(crate) mod capture_lifetime;
pub(crate) mod child;
pub(crate) mod child_reader;
pub(crate) mod client;
pub(crate) mod helper;
pub(crate) mod helper_capture;
pub(crate) mod installation;
use std::io;
pub(crate) fn denied(reason: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason.to_string())
}
pub(crate) fn now() -> io::Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
}
pub(crate) fn nonce() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) mod gateway;
pub(crate) mod gateway_client;
pub(crate) mod host_status;
pub(crate) mod provision;
pub(crate) mod status;
pub(crate) mod target;
pub(crate) mod viewer;
pub(crate) mod viewer_images;
