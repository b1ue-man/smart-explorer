//! Explicit public-machine-target import. No discovery/profile/key-store lookup.
use crate::login::{core::gateway_types::MAX_NETWORK, GatewayTarget};
use std::{
    io::{self, Read},
    path::Path,
};
pub(crate) fn read_target_file(path: &Path) -> io::Result<GatewayTarget> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > MAX_NETWORK as u64 {
        return Err(io::Error::other("public login target exceeds 32 KiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_NETWORK as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_NETWORK {
        return Err(io::Error::other("public login target exceeds 32 KiB"));
    }
    let target: GatewayTarget = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    target.validate()?;
    Ok(target)
}
