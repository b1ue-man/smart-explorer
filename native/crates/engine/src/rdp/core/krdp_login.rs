//! KRdp sign-in with the grant's account (RDP-ZIELKONTO, second step), the
//! portable part. KRdp (KDE/krdp `server/main.cpp`, `src/RdpConnection.cpp`,
//! read 2026-11-18) admits the names of `krdpserverrc` `[General] Users`
//! with the password QtKeychain keeps under service `KRDP`, key = the name
//! (KWallet folder `KRDP`, application `KRDP`), read once when the server
//! starts; with `SystemUserEnabled` it first tries PAM service `login` for
//! the session's own login name. se adds one KRdp user of its own,
//! `KRDP_USER`, with a random password in that wallet entry and hands it
//! to the viewer like the xrdp token: KRdp always shows the Plasma session of
//! the account that runs it, so only that account's grant may use it.

/// The KRdp user se adds; not a system account (KRdp's own list).
pub const KRDP_USER: &str = "se-krdp";
/// QtKeychain service, KWallet folder and application id KRdp uses.
pub const KEYCHAIN_SERVICE: &str = "KRDP";

/// Whether the grant's account may sign in through KRdp: KRdp shows the
/// session of `session_owner` (the account the worker and KRdp run as);
/// a root worker reaches no user's wallet.
pub fn account_allowed(session_owner: &str, grant_account: &str) -> Result<(), String> {
    if session_owner == "root" {
        return Err("KRdp läuft in der Plasma-Sitzung eines Kontos; der Hintergrunddienst als root erreicht dessen KWallet nicht, die automatische Anmeldung braucht den Hintergrunddienst dieses Kontos".into());
    }
    if session_owner != grant_account {
        return Err(format!(
            "KRdp zeigt die Plasma-Sitzung von {session_owner}; die Freigabe nennt {grant_account}"
        ));
    }
    Ok(())
}

/// The `Users` value with `user` added; `None` when it is already listed.
/// Every other entry keeps its text (escaped commas included).
pub fn users_with(raw: &str, user: &str) -> Option<String> {
    if raw.split(',').any(|entry| entry.trim() == user) {
        return None;
    }
    Some(if raw.trim().is_empty() {
        user.to_string()
    } else {
        format!("{raw},{user}")
    })
}

/// The `Users` value without `user`; `None` when it was not listed.
pub fn users_without(raw: &str, user: &str) -> Option<String> {
    let entries: Vec<&str> = raw.split(',').collect();
    let kept: Vec<&str> = entries
        .iter()
        .copied()
        .filter(|entry| entry.trim() != user)
        .collect();
    (kept.len() != entries.len()).then(|| kept.join(","))
}

/// What a steering end does to se's KRdp user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageChange {
    /// Every device that received the password still steers.
    Keep,
    /// A device that received it lost steering, others still steer: a new
    /// password nobody received yet.
    Rotate,
    /// Nobody steers any more: the user and its password go.
    Remove,
}

/// `exists`: se's KRdp user or its password is there; `recipient_ended`: a
/// device that received the current password no longer steers.
pub fn coverage_change(
    exists: bool,
    recipient_ended: bool,
    steering_remains: bool,
) -> CoverageChange {
    match (exists, steering_remains, recipient_ended) {
        (false, ..) => CoverageChange::Keep,
        (true, false, _) => CoverageChange::Remove,
        (true, true, true) => CoverageChange::Rotate,
        (true, true, false) => CoverageChange::Keep,
    }
}

/// A new KRdp password: 32 random bytes, base64url (KRdp compares Latin-1
/// text; these characters are ASCII).
pub fn new_password() -> Result<String, String> {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("random: {error}"))?;
    let password = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    bytes.fill(0);
    Ok(password)
}

#[cfg(test)]
#[path = "krdp_login_task_tests.rs"]
mod krdp_login_task_tests;
