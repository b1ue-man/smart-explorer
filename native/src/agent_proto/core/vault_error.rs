//! Typed vault errors through the string error frame of the agent protocol
//! (C3.2), on the pattern of `SE_TRANSPORT_V1`: the worker that serves a
//! vault to another process (`IpcRequest::Vault { req: Open }`) writes
//! `SE_VAULT_V1 <code> <site-id> <text>`, and the client rebuilds the typed
//! error (`agent/core/agent_error.rs`: `VaultLocked`). Older clients show
//! the text as before (an error with a message); an unknown code or a
//! malformed site keeps that legacy meaning. Dependency-free: this module is
//! also part of the standalone `se-agent`.

const PREFIX: &str = "SE_VAULT_V1 ";
/// The code of a locked vault (`vault_locked` on the other channels).
pub(crate) const VAULT_LOCKED_WIRE: &str = "locked";
/// Hex digits of a site ID.
const SITE_ID_LEN: usize = 32;

/// A vault error read from an error frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VaultWireError<'a> {
    pub(crate) code: &'a str,
    pub(crate) site: &'a str,
    pub(crate) text: &'a str,
}

/// `SE_VAULT_V1 <code> <site> <text>`.
pub(crate) fn vault_error_message(code: &str, site: &str, text: &str) -> String {
    format!("{PREFIX}{code} {site} {text}")
}

/// The vault error of `message`; `None` for every other text, also for a
/// code this version does not know.
pub(crate) fn parse_vault_error(message: &str) -> Option<VaultWireError<'_>> {
    let (code, rest) = message.strip_prefix(PREFIX)?.split_once(' ')?;
    let (site, text) = rest.split_once(' ').unwrap_or((rest, ""));
    let known = code == VAULT_LOCKED_WIRE;
    let site_id = site.len() == SITE_ID_LEN
        && site
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    (known && site_id).then_some(VaultWireError { code, site, text })
}
