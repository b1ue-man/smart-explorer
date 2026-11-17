//! xrdp sign-in with a one-time se token (plan 2026-11-17 P4, decision 6),
//! the portable part: the PAM line, its place in `/etc/pam.d/xrdp-sesman`,
//! the token records and the decision of the PAM helper. xrdp checks every
//! login through its PAM service `xrdp-sesman`; se inserts one line before
//! the shared password stack:
//!
//! `auth [success=done default=ignore] pam_exec.so quiet expose_authtok type=auth <se> --rdp-xrdp-login`
//!
//! `pam_exec` hands the typed password to `se --rdp-xrdp-login` on standard
//! input (`expose_authtok`). A token se issued for this account, through a
//! live se tunnel, ends the stack with success (`success=done`); everything
//! else, a typed password too, goes on to the usual password check
//! (`default=ignore`). The worker keeps only the SHA-256 of a token, in a
//! directory only its own uid reads (`TOKEN_DIR/<uid>/tokens.json`).
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The PAM service of xrdp (xrdp `sesman/verify_user_pam.c`, package file).
pub const PAM_FILE: &str = "/etc/pam.d/xrdp-sesman";
/// The only argument of the se helper the PAM line starts.
pub const PAM_SWITCH: &str = "--rdp-xrdp-login";
/// The comment above the line, so a person reading the file knows it.
pub const PAM_COMMENT: &str = "# Smart Explorer: xrdp-Anmeldung mit einem Einmal-Token von se (entfernt mit dem letzten Bildschirm steuern)";
/// Every token starts so; the helper ignores any other input at once.
pub const TOKEN_PREFIX: &str = "se-xrdp1.";
/// Token lists, one directory per worker uid (owner that uid, mode 0700).
pub const TOKEN_DIR: &str = "/var/lib/smart-explorer/xrdp-login";
pub const TOKEN_FILE: &str = "tokens.json";
/// A token is accepted at most this long after it was issued; a new start
/// issues a new one, the end of „Bildschirm steuern“ revokes it earlier.
pub const TOKEN_LIFETIME_SECS: i64 = 12 * 60 * 60;
/// Most records one worker keeps (one per viewer and account).
pub const MAX_TOKENS: usize = 256;

/// The line se inserts; `helper` is the absolute path of the installed `se`.
pub fn pam_line(helper: &str) -> Result<String, String> {
    let unsafe_char = |c: char| c.is_whitespace() || c.is_control() || "[]#\\".contains(c);
    if !helper.starts_with('/') || helper.len() > 1024 || helper.chars().any(unsafe_char) {
        return Err(format!("xrdp_pam_helper_path_unusable: {helper:?}"));
    }
    Ok(format!(
        "auth [success=done default=ignore] pam_exec.so quiet expose_authtok type=auth {helper} {PAM_SWITCH}"
    ))
}

/// A line se inserted (any helper path), or its comment.
pub fn is_se_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed == PAM_COMMENT {
        return true;
    }
    !trimmed.starts_with('#')
        && trimmed.split_whitespace().any(|word| word == "pam_exec.so")
        && trimmed.split_whitespace().any(|word| word == PAM_SWITCH)
}

/// The shared password stack the line must precede: Debian/Ubuntu
/// `@include common-auth`, Fedora/RHEL `auth include|substack
/// password-auth|system-auth`.
fn shared_password_stack(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["@include", "common-auth", ..] => true,
        ["auth", "include" | "substack", stack, ..] => {
            matches!(*stack, "common-auth" | "password-auth" | "system-auth")
        }
        _ => false,
    }
}

/// `text` without the lines se inserted, and whether it changed.
pub fn pam_without_line(text: &str) -> (String, bool) {
    let kept: Vec<&str> = text.lines().filter(|line| !is_se_line(line)).collect();
    if kept.len() == text.lines().count() {
        return (text.to_string(), false);
    }
    (kept.join("\n") + "\n", true)
}

/// `text` with the comment and `line` exactly once, right before the shared
/// password stack (earlier se lines elsewhere move there), and whether it
/// changed. A file without a known stack stays untouched (an error).
pub fn pam_with_line(text: &str, line: &str) -> Result<(String, bool), String> {
    let mut lines: Vec<&str> = text.lines().filter(|line| !is_se_line(line)).collect();
    let at = lines
        .iter()
        .position(|line| shared_password_stack(line))
        .ok_or("xrdp_pam_layout_unknown: no @include common-auth or auth include password-auth")?;
    lines.insert(at, line);
    lines.insert(at, PAM_COMMENT);
    let changed = lines != text.lines().collect::<Vec<_>>();
    Ok((lines.join("\n") + "\n", changed))
}

/// One issued token, without the token itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRecord {
    /// The account the token signs in (`PAM_USER`).
    pub user: String,
    /// SHA-256 of the token, lowercase hex.
    pub digest: String,
    /// Loopback port of the xrdp listener the se tunnel connects to.
    pub port: u16,
    /// The viewer that received it (revocation when its steering ends).
    pub device_id: String,
    pub public_key: String,
    /// Unix seconds after which it is refused.
    pub expires_at: i64,
}

/// A new token: the prefix and 32 random bytes, base64url.
pub fn new_token() -> Result<String, String> {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("random: {error}"))?;
    let token = format!(
        "{TOKEN_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    );
    bytes.fill(0);
    Ok(token)
}

/// SHA-256 of `token`, lowercase hex.
pub fn token_digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// The decision of the PAM helper: a record names `user`, carries the
/// digest of `token`, has not expired, and a se tunnel to its port lives
/// now (`tunnel_live`). Anything else is no se token for this login.
pub fn token_accepted(
    records: &[TokenRecord],
    user: &str,
    token: &str,
    now: i64,
    tunnel_live: impl Fn(u16) -> bool,
) -> bool {
    if !token.starts_with(TOKEN_PREFIX) || token.len() > 256 || user.is_empty() {
        return false;
    }
    let digest = token_digest(token);
    records.iter().any(|record| {
        record.user == user
            && same_digest(&record.digest, &digest)
            && now < record.expires_at
            && tunnel_live(record.port)
    })
}

/// Compares two digests without stopping at the first difference.
fn same_digest(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// Whether the worker of `owner_uid` may sign `user` (uid `user_uid`) in: a
/// root worker every account, another worker its own account and the ones
/// the account service configuration lists for it (`listed`, as Exec), never
/// root.
pub fn user_allowed(owner_uid: u32, user_uid: Option<u32>, listed: bool) -> bool {
    match user_uid {
        None => false,
        Some(_) if owner_uid == 0 => true,
        Some(0) => false,
        Some(uid) => uid == owner_uid || listed,
    }
}

/// One row of `/proc/net/tcp` or `/proc/net/tcp6` that is a live se tunnel
/// connection to the xrdp listener: established (`01`), the peer is a
/// loopback address on `port`, and the socket belongs to `uid` (the worker
/// whose host side of the tunnel connects to xrdp).
pub fn tunnel_row(row: &str, port: u16, uid: u32) -> bool {
    let fields: Vec<&str> = row.split_whitespace().collect();
    let (Some(remote), Some(state), Some(owner)) = (fields.get(2), fields.get(3), fields.get(7))
    else {
        return false;
    };
    let Some((address, remote_port)) = remote.rsplit_once(':') else {
        return false;
    };
    *state == "01"
        && loopback_hex(address)
        && u16::from_str_radix(remote_port, 16).is_ok_and(|found| found == port)
        && owner.parse::<u32>().is_ok_and(|found| found == uid)
}

/// A loopback address as the kernel's TCP tables print it: IPv4 127/8 (the
/// last byte of the host-order word), `::1`, or IPv4-mapped 127/8.
fn loopback_hex(address: &str) -> bool {
    match address.len() {
        8 => address.ends_with("7F"),
        32 => {
            address == "00000000000000000000000001000000"
                || (address.starts_with("0000000000000000FFFF0000") && address.ends_with("7F"))
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "xrdp_login_task_tests.rs"]
mod xrdp_login_task_tests;
