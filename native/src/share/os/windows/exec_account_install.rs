//! What the setup of the Windows account service does as administrator
//! (PRM-M7, D.5, D.2.8): the program copy in `%ProgramFiles%` (checked
//! against SHA-256 after copying, and against the hash the user confirmed
//! with UAC), the configuration and the protected data folder, the service
//! with the access list of the registered users, the account `se-remote`
//! (a standard user in the group Users with a random password nobody
//! knows), and their removal. `se-remote` is deleted, with its profile,
//! only on explicit request. An account that is no local user of this PC,
//! a system account and every member of the Administrators group are never
//! allowed.
use std::io;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::ERROR_MEMBER_IN_ALIAS;
use windows_sys::Win32::NetworkManagement::NetManagement::{
    NERR_Success, NERR_UserExists, NERR_UserNotFound, NetApiBufferFree, NetLocalGroupAddMembers,
    NetUserAdd, NetUserDel, NetUserGetLocalGroups, LG_INCLUDE_INDIRECT, LOCALGROUP_MEMBERS_INFO_0,
    LOCALGROUP_USERS_INFO_0, MAX_PREFERRED_LENGTH, UF_DONT_EXPIRE_PASSWD, UF_PASSWD_CANT_CHANGE,
    UF_SCRIPT, USER_INFO_1, USER_PRIV_USER,
};
use windows_sys::Win32::Security::{
    LookupAccountSidW, WinBuiltinAdministratorsSid, WinBuiltinUsersSid, SID_NAME_USE,
};
use windows_sys::Win32::UI::Shell::DeleteProfileW;

use crate::share::access_grant::REMOTE_ACCOUNT_NAME;

use super::exec_account_config::{
    config_path, data_dir, load_config, log_path, program_copy, program_dir, ServiceConfig,
};
use super::exec_account_files::{
    ensure_data_dir, remove_file, sha256_file, sha256_reader, write_trusted, CONFIG_SDDL,
};
use super::exec_account_s4u::check_account;
use super::exec_account_scm::{
    delete_service, install_service, service_state, set_service_access, stop_service,
};
use super::exec_account_security::{from_wide_ptr, lookup_account, process_is_elevated, wide, Sid};
use super::exec_account_setup::ExecAccountsAction;

/// Runs `action` for the Windows user `for_sid` (administrator only).
/// `source` is the program to install; `expected` is the SHA-256 the user
/// confirmed with UAC.
pub(super) fn apply(
    action: &ExecAccountsAction,
    for_sid: &str,
    source: &Path,
    expected: Option<&str>,
) -> io::Result<Vec<String>> {
    if !process_is_elevated() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the account service is set up by an administrator only",
        ));
    }
    match action {
        ExecAccountsAction::Setup {
            remote_account,
            allow,
        } => setup(for_sid, *remote_account, allow, source, expected),
        ExecAccountsAction::Allow { account } => change(for_sid, account, true),
        ExecAccountsAction::Disallow { account } => change(for_sid, account, false),
        ExecAccountsAction::Update => update(source, expected),
        ExecAccountsAction::Remove {
            delete_remote_account,
        } => remove(*delete_remote_account),
    }
}

fn setup(
    for_sid: &str,
    remote_account: bool,
    allow: &[String],
    source: &Path,
    expected: Option<&str>,
) -> io::Result<Vec<String>> {
    for account in allow {
        allowable(account)?;
    }
    ensure_data_dir()?;
    let mut done = Vec::new();
    if remote_account {
        done.push(if ensure_remote_account()? {
            "created the remote access account se-remote".to_string()
        } else {
            "the remote access account se-remote exists".to_string()
        });
    }
    let mut config = load_config()?.unwrap_or_else(ServiceConfig::new);
    config.add_user(for_sid).map_err(invalid)?;
    let remote = remote_account.then_some(REMOTE_ACCOUNT_NAME);
    for account in remote.into_iter().chain(allow.iter().map(String::as_str)) {
        config.allow(for_sid, account).map_err(invalid)?;
    }
    write_trusted(&config_path()?, &config.render(), CONFIG_SDDL)?;
    done.push(format!(
        "user {for_sid} may run remote commands as: {}",
        config.accounts_for(for_sid).join(", ")
    ));
    done.extend(install(&config, source, expected)?);
    Ok(done)
}

fn change(for_sid: &str, account: &str, allow: bool) -> io::Result<Vec<String>> {
    let mut config = load_config()?.ok_or_else(not_set_up)?;
    let changed = if allow {
        allowable(account)?;
        config.allow(for_sid, account).map_err(invalid)?
    } else {
        config.disallow(for_sid, account)
    };
    write_trusted(&config_path()?, &config.render(), CONFIG_SDDL)?;
    set_service_access(&config.user_sids())?;
    Ok(vec![match (allow, changed) {
        (true, true) => format!("user {for_sid} may now run remote commands as {account}"),
        (true, false) => format!("user {for_sid} could run remote commands as {account} already"),
        (false, true) => format!("user {for_sid} may no longer run remote commands as {account}"),
        (false, false) => format!("{account} was not allowed for user {for_sid}"),
    }])
}

fn update(source: &Path, expected: Option<&str>) -> io::Result<Vec<String>> {
    if service_state()?.is_none() {
        return Err(not_set_up());
    }
    ensure_data_dir()?;
    let config = load_config()?.unwrap_or_else(ServiceConfig::new);
    // Writing it again repairs its access list.
    write_trusted(&config_path()?, &config.render(), CONFIG_SDDL)?;
    install(&config, source, expected)
}

/// The program copy and the service with the access list of `config`.
fn install(
    config: &ServiceConfig,
    source: &Path,
    expected: Option<&str>,
) -> io::Result<Vec<String>> {
    let copy = program_copy()?;
    let (hash, replaced) = install_copy(source, &copy, expected)?;
    let mut done = vec![if replaced {
        format!("installed {} (SHA-256 {hash})", copy.display())
    } else {
        format!("{} is current (SHA-256 {hash})", copy.display())
    }];
    let created = install_service(&copy, &config.user_sids())?;
    done.push(if created {
        "registered the service SmartExplorerExecAccounts (LocalSystem, started on demand)".into()
    } else {
        "updated the service SmartExplorerExecAccounts".into()
    });
    Ok(done)
}

/// Copies `source` to `copy` where it differs; returns its SHA-256 and
/// whether the copy was replaced. A running service stops first (its
/// sessions end); a copy a relay still runs is moved aside, not
/// overwritten.
fn install_copy(source: &Path, copy: &Path, expected: Option<&str>) -> io::Result<(String, bool)> {
    let bytes = std::fs::read(source)?;
    let hash = sha256_reader(&mut bytes.as_slice())?;
    if expected.is_some_and(|expected| !expected.eq_ignore_ascii_case(&hash)) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the program changed after it was confirmed; nothing was installed",
        ));
    }
    if sha256_file(copy).is_ok_and(|current| current == hash) {
        return Ok((hash, false));
    }
    let directory = program_dir()?;
    std::fs::create_dir_all(&directory)?;
    stop_service()?;
    remove_leftovers(&directory);
    let temporary = copy.with_extension("exe.new");
    std::fs::write(&temporary, &bytes)?;
    if sha256_file(&temporary)? != hash {
        let _ = std::fs::remove_file(&temporary);
        return Err(io::Error::other(
            "the new program copy differs from its source",
        ));
    }
    if copy.exists() {
        let aside = copy.with_extension(format!("old-{}", std::process::id()));
        std::fs::rename(copy, &aside)?;
        let _ = std::fs::remove_file(&aside);
    }
    std::fs::rename(&temporary, copy)?;
    if sha256_file(copy)? != hash {
        return Err(io::Error::other(format!(
            "{} differs from the running program after copying",
            copy.display()
        )));
    }
    Ok((hash, true))
}

/// Copies moved aside earlier, which no relay runs any more.
fn remove_leftovers(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with("se.old-") || name == "se.exe.new" {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn remove(delete_remote_account: bool) -> io::Result<Vec<String>> {
    let mut done = Vec::new();
    if delete_service()? {
        done.push("stopped and removed the service SmartExplorerExecAccounts".to_string());
    }
    let directory = program_dir()?;
    let copy = program_copy()?;
    if remove_file(&copy)? {
        done.push(format!("removed {}", copy.display()));
    }
    remove_leftovers(&directory);
    let _ = std::fs::remove_dir(&directory);
    if let Some(parent) = directory.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    for path in [config_path()?, log_path()?] {
        if remove_file(&path)? {
            done.push(format!("removed {}", path.display()));
        }
    }
    remove_reports();
    let _ = std::fs::remove_dir(data_dir()?);
    if !delete_remote_account {
        done.push("kept the account se-remote (--delete-remote-account deletes it)".into());
        return Ok(done);
    }
    if delete_remote_account_and_profile()? {
        done.push("deleted the account se-remote and its profile".into());
    }
    Ok(done)
}

fn remove_reports() {
    let Ok(entries) = data_dir().and_then(std::fs::read_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with("exec-accounts-report-") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// An account an administrator may allow: a local user account of this
/// PC, no system account and no member of the Administrators group.
fn allowable(account: &str) -> io::Result<()> {
    check_account(account).map_err(|refused| invalid(refused.reply().1))?;
    if administrators_member(account)? {
        return Err(invalid(format!(
            "{account} is a member of the Administrators group; the account service never \
             runs commands as an administrator"
        )));
    }
    Ok(())
}

/// `account` is a member of the Administrators group, also indirectly.
fn administrators_member(account: &str) -> io::Result<bool> {
    let administrators = account_name_of(&Sid::well_known(WinBuiltinAdministratorsSid)?)?;
    let name = wide(account);
    let mut buffer: *mut u8 = null_mut();
    let (mut read, mut total) = (0u32, 0u32);
    let status = unsafe {
        NetUserGetLocalGroups(
            null(),
            name.as_ptr(),
            0,
            LG_INCLUDE_INDIRECT,
            &mut buffer,
            MAX_PREFERRED_LENGTH,
            &mut read,
            &mut total,
        )
    };
    if status != NERR_Success {
        if !buffer.is_null() {
            unsafe { NetApiBufferFree(buffer as *const core::ffi::c_void) };
        }
        return Err(net_error("NetUserGetLocalGroups", status));
    }
    let groups = buffer.cast::<LOCALGROUP_USERS_INFO_0>();
    let member = (0..read as usize).any(|index| {
        // SAFETY: the buffer holds `read` entries with NUL-terminated names.
        let group = unsafe { from_wide_ptr((*groups.add(index)).lgrui0_name) };
        group.eq_ignore_ascii_case(&administrators)
    });
    unsafe { NetApiBufferFree(buffer as *const core::ffi::c_void) };
    Ok(member)
}

/// The name of the account or group `sid` (localized for built-in groups).
pub(super) fn account_name_of(sid: &Sid) -> io::Result<String> {
    let (mut name_size, mut domain_size, mut kind) = (0u32, 0u32, 0 as SID_NAME_USE);
    unsafe {
        LookupAccountSidW(
            null(),
            sid.as_psid(),
            null_mut(),
            &mut name_size,
            null_mut(),
            &mut domain_size,
            &mut kind,
        )
    };
    if name_size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut name = vec![0u16; name_size as usize];
    let mut domain = vec![0u16; domain_size.max(1) as usize];
    let found = unsafe {
        LookupAccountSidW(
            null(),
            sid.as_psid(),
            name.as_mut_ptr(),
            &mut name_size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut kind,
        )
    };
    if found == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf16_lossy(
        &name[..(name_size as usize).min(name.len())],
    ))
}

/// Creates the standard account `se-remote` in the group Users with a
/// random password nobody knows (C2 W-f); `Ok(false)` when it exists.
fn ensure_remote_account() -> io::Result<bool> {
    if lookup_account(REMOTE_ACCOUNT_NAME).is_ok() {
        allowable(REMOTE_ACCOUNT_NAME)?;
        return Ok(false);
    }
    let mut name = wide(REMOTE_ACCOUNT_NAME);
    let mut password = wide(random_password()?);
    let mut comment = wide("Smart Explorer remote access");
    let info = USER_INFO_1 {
        usri1_name: name.as_mut_ptr(),
        usri1_password: password.as_mut_ptr(),
        usri1_password_age: 0,
        usri1_priv: USER_PRIV_USER,
        usri1_home_dir: null_mut(),
        usri1_comment: comment.as_mut_ptr(),
        usri1_flags: UF_SCRIPT | UF_DONT_EXPIRE_PASSWD | UF_PASSWD_CANT_CHANGE,
        usri1_script_path: null_mut(),
    };
    let mut parameter = 0u32;
    let status = unsafe {
        NetUserAdd(
            null(),
            1,
            (&info as *const USER_INFO_1).cast(),
            &mut parameter,
        )
    };
    password.iter_mut().for_each(|unit| *unit = 0);
    match status {
        NERR_Success => {}
        NERR_UserExists => return Ok(false),
        status => return Err(net_error("NetUserAdd (se-remote)", status)),
    }
    add_to_users(REMOTE_ACCOUNT_NAME)?;
    Ok(true)
}

fn add_to_users(account: &str) -> io::Result<()> {
    let users = wide(account_name_of(&Sid::well_known(WinBuiltinUsersSid)?)?);
    let member = lookup_account(account)?.sid;
    let entry = LOCALGROUP_MEMBERS_INFO_0 {
        lgrmi0_sid: member.as_psid(),
    };
    let status = unsafe {
        NetLocalGroupAddMembers(
            null(),
            users.as_ptr(),
            0,
            (&entry as *const LOCALGROUP_MEMBERS_INFO_0).cast(),
            1,
        )
    };
    if status != NERR_Success && status != ERROR_MEMBER_IN_ALIAS {
        return Err(net_error("NetLocalGroupAddMembers (Users)", status));
    }
    Ok(())
}

/// Deletes `se-remote`, then its profile folder, which `NetUserDel` keeps;
/// `Ok(false)` when there was no such account. The service is gone before,
/// so none of its processes runs.
fn delete_remote_account_and_profile() -> io::Result<bool> {
    let Ok(found) = lookup_account(REMOTE_ACCOUNT_NAME) else {
        return Ok(false);
    };
    let sid = wide(found.sid.text()?);
    let name = wide(REMOTE_ACCOUNT_NAME);
    match unsafe { NetUserDel(null(), name.as_ptr()) } {
        NERR_Success => {}
        NERR_UserNotFound => return Ok(false),
        status => return Err(net_error("NetUserDel (se-remote)", status)),
    }
    // A profile that was never created is no error.
    unsafe { DeleteProfileW(sid.as_ptr(), null(), null()) };
    Ok(true)
}

/// 32 characters with upper and lower case letters, digits and symbols, so
/// every password policy of Windows accepts them.
fn random_password() -> io::Result<String> {
    const CLASSES: [&[u8]; 4] = [
        b"ABCDEFGHJKLMNPQRSTUVWXYZ",
        b"abcdefghijkmnopqrstuvwxyz",
        b"23456789",
        b"!#%+-.:=?@_",
    ];
    let mut random = [0u8; 32];
    getrandom::getrandom(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(random
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let class = CLASSES[index % CLASSES.len()];
            char::from(class[usize::from(*byte) % class.len()])
        })
        .collect())
}

fn net_error(what: &str, status: u32) -> io::Error {
    // Codes below 2100 are Windows errors; NERR codes start at 2100.
    if status < 2100 {
        io::Error::new(
            io::Error::from_raw_os_error(status as i32).kind(),
            format!("{what}: {}", io::Error::from_raw_os_error(status as i32)),
        )
    } else {
        io::Error::other(format!("{what}: NET_API_STATUS {status}"))
    }
}

fn not_set_up() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "the account service is not set up; run `se share exec accounts setup` first",
    )
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
