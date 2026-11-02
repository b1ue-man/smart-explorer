//! The error of a locked vault (C3.2). Backends, the agent and the vault
//! itself return and recognise it, so it lives in the storage layer (plan
//! docs/plaene/2026-11-01-idealzustand, D13); `vault` re-exports it.
use std::{fmt, io};

/// The structured code of a locked vault across process boundaries (IPC
/// `Err.code`, Bridge `ApiError` kind, agent envelope; C3.2).
pub const VAULT_LOCKED_CODE: &str = "vault_locked";

/// The payload of every error of a locked vault (C3.2). It travels inside an
/// `io::Error` of kind `PermissionDenied`, so `try_exists` (only `NotFound`
/// means absent) never reads a locked vault as empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultLocked {
    /// Site ID of the locked vault (never a path or a name inside it).
    pub site: String,
}

impl fmt::Display for VaultLocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tresor {} ist gesperrt", self.site)
    }
}

impl std::error::Error for VaultLocked {}

/// The error every operation on a locked vault returns.
pub fn vault_locked(site: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        VaultLocked {
            site: site.to_string(),
        },
    )
}

/// The site of a `VaultLocked` error; `None` for every other error.
pub fn locked_site(error: &io::Error) -> Option<&str> {
    error
        .get_ref()?
        .downcast_ref::<VaultLocked>()
        .map(|locked| locked.site.as_str())
}

/// The error frame text of the worker's vault connections (`Vault::Open`)
/// for a locked vault: `SE_VAULT_V1 locked <site> <text>` (C3.2); `None`
/// for every other error, which keeps its text.
pub fn wire_error_text(error: &std::io::Error) -> Option<String> {
    let site = locked_site(error)?;
    Some(crate::agent_proto::vault_error_message(
        crate::agent_proto::VAULT_LOCKED_WIRE,
        site,
        &error.to_string(),
    ))
}
