//! Linux I/O of the xrdp sign-in with a one-time se token (plan 2026-11-17
//! P4, decision 6; the portable part is `core::xrdp_login`). The worker
//! writes the SHA-256 of each token it issues to `TOKEN_DIR/<its uid>/
//! tokens.json` (owner its uid, mode 0700/0600). The root step `xrdp-login`
//! of the existing elevated setup creates that directory for the invoking
//! worker and inserts the PAM line; `xrdp-login-remove` takes the line out
//! again. `se --rdp-xrdp-login`, which `pam_exec` starts as root inside
//! xrdp-sesman, accepts a login only for a listed token of an allowed
//! account while a se tunnel of that worker connects to xrdp.
use crate::rdp::core::xrdp_login::{
    is_se_line, new_token, pam_line, pam_with_line, pam_without_line, token_accepted, token_digest,
    tunnel_row, user_allowed, TokenRecord, MAX_TOKENS, PAM_FILE, TOKEN_DIR, TOKEN_FILE,
    TOKEN_LIFETIME_SECS, TOKEN_PREFIX,
};
use crate::rdp::core::SetupState;
use crate::rdp::os::shared::host_login::Viewer;
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const MAX_LIST_BYTES: u64 = 256 * 1024;
const MAX_INPUT_BYTES: u64 = 4096;

fn euid() -> u32 {
    // SAFETY: geteuid takes no pointer and has no side effects.
    unsafe { libc::geteuid() }
}

/// Whether this worker runs as root (it may change PAM without polkit).
pub(super) fn running_as_root() -> bool {
    euid() == 0
}

/// The name of `uid` in the user database.
pub fn user_name(uid: u32) -> Option<String> {
    // SAFETY: `passwd` is plain data; zero is a valid empty value.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16 * 1024];
    // SAFETY: `entry` and `buffer` outlive the call; the name is copied
    // before they go.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() || entry.pw_name.is_null() {
        return None;
    }
    // SAFETY: a filled entry's name is a NUL-terminated string in `buffer`.
    let name = unsafe { CStr::from_ptr(entry.pw_name) };
    name.to_str().ok().map(str::to_string)
}

/// The uid of the account `name`.
pub fn user_uid(name: &str) -> Option<u32> {
    let name = CString::new(name).ok()?;
    // SAFETY: `passwd` is plain data; zero is a valid empty value.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16 * 1024];
    // SAFETY: as in `user_name`; only the numeric uid is read.
    let status = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    (status == 0 && !found.is_null()).then_some(entry.pw_uid)
}

/// The account this worker runs as.
pub fn own_user() -> Option<String> {
    user_name(euid())
}

/// Whether `/etc/pam.d/xrdp-sesman` carries the se line.
pub fn pam_installed() -> bool {
    std::fs::read_to_string(PAM_FILE).is_ok_and(|text| text.lines().any(is_se_line))
}

/// A directory of `uid` only: no link, owner `uid`, no rights for others.
fn private_dir(path: &Path, uid: u32) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.file_type().is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0
    })
}

/// The PAM line and this worker's token directory exist (root step done).
pub fn ready() -> Result<PathBuf, String> {
    if !pam_installed() {
        return Err(format!("{PAM_FILE} hat die se-Zeile nicht"));
    }
    let dir = Path::new(TOKEN_DIR).join(euid().to_string());
    if !private_dir(&dir, euid()) {
        return Err(format!(
            "{} fehlt oder gehört nicht diesem Dienst",
            dir.display()
        ));
    }
    Ok(dir)
}

static LIST: Mutex<()> = Mutex::new(());

/// The records of a list `owner` wrote: a plain file of `owner`, writable
/// only by it, at most 256 KiB; anything else counts as empty.
fn read_list(path: &Path, owner: u32) -> Vec<TokenRecord> {
    let trusted = std::fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.file_type().is_file() && meta.uid() == owner && meta.mode() & 0o022 == 0
    });
    let mut text = String::new();
    let read = trusted
        && std::fs::File::open(path)
            .and_then(|file| file.take(MAX_LIST_BYTES + 1).read_to_string(&mut text))
            .is_ok_and(|length| length as u64 <= MAX_LIST_BYTES);
    if !read {
        return Vec::new();
    }
    serde_json::from_str(&text).unwrap_or_default()
}

/// Same directory, mode 0600, atomic rename.
fn write_list(dir: &Path, records: &[TokenRecord]) -> Result<(), String> {
    let bytes = serde_json::to_vec(records).map_err(|error| error.to_string())?;
    let staged = dir.join(format!("{TOKEN_FILE}.tmp"));
    let _ = std::fs::remove_file(&staged);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)
        .and_then(|mut file| file.write_all(&bytes).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&staged, dir.join(TOKEN_FILE)));
    written.map_err(|error| format!("xrdp-Token nicht speicherbar: {error}"))
}

/// A new token for `user` on the xrdp listener `port`, bound to `viewer`;
/// it replaces the viewer's earlier token for that account.
pub fn issue(user: &str, port: u16, viewer: &Viewer) -> Result<String, String> {
    let dir = ready()?;
    let now = crate::share::core_now_secs();
    let token = new_token()?;
    let _list = LIST.lock().unwrap_or_else(PoisonError::into_inner);
    let mut records = read_list(&dir.join(TOKEN_FILE), euid());
    records.retain(|record| {
        record.expires_at > now
            && !(record.user == user
                && record.device_id == viewer.device_id
                && record.public_key == viewer.public_key)
    });
    if records.len() >= MAX_TOKENS {
        // The oldest go first.
        records = records.split_off(records.len() + 1 - MAX_TOKENS);
    }
    records.push(TokenRecord {
        user: user.to_string(),
        digest: token_digest(&token),
        port,
        device_id: viewer.device_id.clone(),
        public_key: viewer.public_key.clone(),
        expires_at: now + TOKEN_LIFETIME_SECS,
    });
    write_list(&dir, &records)?;
    Ok(token)
}

/// Removes the records `remove` names from this worker's list.
pub fn revoke(remove: impl Fn(&TokenRecord) -> bool) {
    let dir = Path::new(TOKEN_DIR).join(euid().to_string());
    if !private_dir(&dir, euid()) {
        return;
    }
    let _list = LIST.lock().unwrap_or_else(PoisonError::into_inner);
    let mut records = read_list(&dir.join(TOKEN_FILE), euid());
    let before = records.len();
    records.retain(|record| !remove(record));
    if records.len() != before {
        if let Err(error) = write_list(&dir, &records) {
            eprintln!("RDP-Anmeldung: {error}");
        }
    }
}

/// The root steps of this module (`host_setup::system_step`); `None` for
/// the other steps.
pub(super) fn system_step(step: super::host_setup::SystemStep) -> Option<SetupState> {
    use super::host_setup::SystemStep;
    let result = match step {
        SystemStep::XrdpLogin => install(crate::share::exec_accounts_invoking_uid().unwrap_or(0)),
        SystemStep::XrdpLoginRemove => remove(),
        _ => return None,
    };
    Some(match result {
        Ok(()) => SetupState::Done,
        Err(reason) => SetupState::Failed(reason),
    })
}

/// After se configured xrdp (same root step): the sign-in too, best effort;
/// a failure leaves the token sign-in to its own step at the next start.
pub(super) fn install_with_xrdp() {
    if let Err(error) = install(crate::share::exec_accounts_invoking_uid().unwrap_or(0)) {
        eprintln!("xrdp-Anmeldung mit Einmal-Token nicht eingerichtet: {error}");
    }
}

/// Root: the token directory of `owner` and the PAM line.
fn install(owner: u32) -> Result<(), String> {
    if euid() != 0 {
        return Err("xrdp_login_needs_root".into());
    }
    let base = Path::new(TOKEN_DIR);
    std::fs::create_dir_all(base).map_err(|e| format!("xrdp_login_dir: {e}"))?;
    std::fs::set_permissions(base, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("xrdp_login_dir: {e}"))?;
    root_owned(base)?;
    let dir = base.join(owner.to_string());
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) if !meta.file_type().is_dir() => {
            return Err(format!("xrdp_login_dir_not_directory: {}", dir.display()))
        }
        Ok(_) => {}
        Err(_) => std::fs::create_dir(&dir).map_err(|e| format!("xrdp_login_dir: {e}"))?,
    }
    std::os::unix::fs::chown(&dir, Some(owner), None)
        .and_then(|()| std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)))
        .map_err(|e| format!("xrdp_login_dir: {e}"))?;
    let helper = crate::updater::installed_cli_path().map_err(|e| format!("se_path: {e}"))?;
    root_owned(&helper)?;
    let helper = helper.to_str().ok_or("se_path_not_utf8")?;
    let (text, mode) = pam_text()?;
    let (changed_text, changed) = pam_with_line(&text, &pam_line(helper)?)?;
    if changed {
        replace_root_file(Path::new(PAM_FILE), changed_text.as_bytes(), mode)?;
    }
    Ok(())
}

/// Root: the PAM line out again (the worker deletes its own records).
fn remove() -> Result<(), String> {
    if euid() != 0 {
        return Err("xrdp_login_needs_root".into());
    }
    let (text, mode) = match pam_text() {
        Ok(found) => found,
        Err(_) if !Path::new(PAM_FILE).exists() => return Ok(()),
        Err(error) => return Err(error),
    };
    let (kept, changed) = pam_without_line(&text);
    if changed {
        replace_root_file(Path::new(PAM_FILE), kept.as_bytes(), mode)?;
    }
    Ok(())
}

/// The PAM file of xrdp and its mode: a plain file of root, no link.
fn pam_text() -> Result<(String, u32), String> {
    let meta = std::fs::symlink_metadata(PAM_FILE).map_err(|e| format!("xrdp_pam: {e}"))?;
    if !meta.file_type().is_file() || meta.uid() != 0 {
        return Err(format!("xrdp_pam_not_root_file: {PAM_FILE}"));
    }
    let text = std::fs::read_to_string(PAM_FILE).map_err(|e| format!("xrdp_pam: {e}"))?;
    Ok((text, meta.mode() & 0o7777))
}

/// Same directory, same mode, synced, atomic rename.
fn replace_root_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let staged = path.with_extension("se-staged");
    let _ = std::fs::remove_file(&staged);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&staged)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(mode)))
        .and_then(|()| std::fs::rename(&staged, path))
        .map_err(|error| format!("xrdp_pam_write: {error}"))
}

/// `path` and every directory above it belong to root and nobody else may
/// write them.
fn root_owned(path: &Path) -> Result<(), String> {
    let path = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    for ancestor in path.ancestors() {
        let meta = std::fs::metadata(ancestor).map_err(|e| e.to_string())?;
        if meta.uid() != 0 || meta.mode() & 0o022 != 0 {
            return Err(format!("not_root_owned: {}", ancestor.display()));
        }
    }
    Ok(())
}

/// A connection of a se tunnel of `uid` to the xrdp listener on `port`.
fn tunnel_live(port: u16, uid: u32) -> bool {
    ["/proc/net/tcp", "/proc/net/tcp6"].iter().any(|table| {
        std::fs::read_to_string(table)
            .is_ok_and(|text| text.lines().skip(1).any(|row| tunnel_row(row, port, uid)))
    })
}

/// `se --rdp-xrdp-login` from `pam_exec` (root, `PAM_USER`, the typed
/// password on standard input): 0 accepts the login, 1 hands it on to the
/// usual password check.
pub fn pam_check() -> i32 {
    if euid() != 0 || std::env::var("PAM_TYPE").is_ok_and(|kind| kind != "auth") {
        return 1;
    }
    let Ok(user) = std::env::var("PAM_USER") else {
        return 1;
    };
    let mut input = zeroize::Zeroizing::new(Vec::new());
    if std::io::stdin()
        .lock()
        .take(MAX_INPUT_BYTES)
        .read_to_end(&mut input)
        .is_err()
    {
        return 1;
    }
    let end = input
        .iter()
        .position(|byte| *byte == 0 || *byte == b'\n')
        .unwrap_or(input.len());
    let Ok(token) = std::str::from_utf8(&input[..end]) else {
        return 1;
    };
    if !token.starts_with(TOKEN_PREFIX) || root_owned(Path::new(TOKEN_DIR)).is_err() {
        return 1;
    }
    let Some(user_uid) = user_uid(&user) else {
        return 1;
    };
    let Ok(entries) = std::fs::read_dir(TOKEN_DIR) else {
        return 1;
    };
    let now = crate::share::core_now_secs();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(owner) = name.to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let dir = entry.path();
        if !private_dir(&dir, owner) {
            continue;
        }
        let listed = owner != 0
            && user_uid != owner
            && crate::share::exec_account_helper_allows(owner, &user);
        if !user_allowed(owner, Some(user_uid), listed) {
            continue;
        }
        let records = read_list(&dir.join(TOKEN_FILE), owner);
        if token_accepted(&records, &user, token, now, |port| tunnel_live(port, owner)) {
            return 0;
        }
    }
    1
}
