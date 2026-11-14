//! Linux adapter of the login self-setup (E21): the broker's grant socket
//! (`SO_PEERCRED` names the account), root replacement of the policy,
//! ledger and network records, pkexec, and the embedded installer.
use super::service::{gateway_account, peer_uid};
use crate::login::{
    core::{gateway_types, self_grants::ConsoleAbilities},
    os::shared::{
        denied, nonce,
        self_channel::{self, Caller, Record},
        self_install::{Elevation, ScriptCall},
    },
};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// The app binary that handles the login service switches (assumption A2).
pub const APP_FILE: &str = "smart_explorer";
pub const SCRIPT_NAME: &str = "install-login-service.sh";
/// The checked-in installer, fixed in this release's binary.
pub const SCRIPT: &str = include_str!("../../../../../../install-login-service.sh");
pub const STAGE_IMAGE: &str = "smart-explorer-login-image";
const GRANTS_DIR: &str = "/run/smart-explorer-login-grants";
const GRANTS_SOCKET: &str = "/run/smart-explorer-login-grants/grants.sock";
const PKEXEC: &str = "/usr/bin/pkexec";
/// Any account can sign in at the greeter and unlock its own session;
/// Linux has no UAC desktop and no secure attention sequence here. A new
/// device needs root or a logind session of the account on seat0.
fn abilities(uid: u32) -> ConsoleAbilities {
    ConsoleAbilities {
        before_login: true,
        locked: true,
        uac: false,
        sas: false,
        console: uid == 0 || on_seat0(uid),
    }
}

/// logind `ListSessions`: (id, uid, user, seat, path). Unknown = no.
fn on_seat0(uid: u32) -> bool {
    type Session = (String, u32, String, String, zbus::zvariant::OwnedObjectPath);
    let sessions = (|| -> zbus::Result<Vec<Session>> {
        let connection = zbus::blocking::Connection::system()?;
        let manager = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )?;
        manager.call("ListSessions", &())
    })();
    sessions.is_ok_and(|sessions| {
        sessions
            .iter()
            .any(|(_, owner, _, seat, _)| *owner == uid && seat == "seat0")
    })
}

pub fn privileged() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Root runs the step itself; any other account asks polkit through
/// pkexec for Smart Explorer's own `se` next to this program.
pub fn elevation() -> Result<Elevation, String> {
    if privileged() {
        return Ok(Elevation::InProcess);
    }
    let se = std::env::current_exe()
        .map_err(|error| format!("se_missing: {error}"))?
        .with_file_name("se");
    if !se.is_file() {
        return Err("se_missing".into());
    }
    if !Path::new(PKEXEC).is_file() {
        return Err("no_prompt".into());
    }
    Ok(Elevation::External(se))
}

/// `pkexec <se> rdp host-setup --system <step>…`: every step of the one
/// bundled call (E44) under one polkit dialog. No time limit: the dialog
/// waits for the person (RDP rule r7).
pub fn run_elevated(se: &Path, steps: &[String]) -> Result<i32, String> {
    let status = Command::new(PKEXEC)
        .arg(se)
        .args(crate::rdp::host_setup::elevated::system_args(steps))
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("pkexec: {error}"))?;
    status
        .code()
        .ok_or_else(|| "pkexec ended without exit code".into())
}

pub fn stage_root() -> io::Result<PathBuf> {
    Ok(PathBuf::from("/run"))
}

pub fn open_image(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

pub fn create_stage_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

pub fn run_install_script(stage: &Path, call: &ScriptCall<'_>) -> io::Result<i32> {
    let bash = ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bash is missing"))?;
    let mut command = Command::new(bash);
    command.arg(stage.join(SCRIPT_NAME));
    match call {
        ScriptCall::SelfInstall {
            image,
            sha256,
            version,
            policy,
            key,
            network,
        } => {
            command
                .arg("self-install")
                .arg(image)
                .arg(sha256)
                .arg(version)
                .arg(policy)
                .arg(key)
                .arg(network);
        }
        ScriptCall::Uninstall => {
            command.arg("uninstall");
        }
    }
    let status = command
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .current_dir("/")
        .stdin(Stdio::null())
        .status()?;
    Ok(status.code().unwrap_or(-1))
}

fn record_path(record: Record) -> io::Result<PathBuf> {
    let config = super::paths()?.config;
    let state = super::gateway_state_directory()?;
    Ok(match record {
        Record::Policy => config.join("policy.json"),
        Record::Ledger => config.join(crate::login::core::self_grants::LEDGER_FILE),
        Record::Network => state.join(gateway_types::NETWORK_FILE),
        Record::MachineKey => state.join(gateway_types::MACHINE_FILE),
        Record::Address => state.join(gateway_types::ADDRESS_FILE),
    })
}

fn trusted_owner(uid: u32) -> bool {
    uid == 0 || gateway_account().is_ok_and(|(gateway, _)| gateway == uid)
}

/// Root reads a protected record; a missing one is `None`.
pub fn read_self_record(record: Record, max: usize) -> io::Result<Option<Vec<u8>>> {
    if !privileged() {
        return Err(denied("login records are read only by root"));
    }
    let file = match open_image(&record_path(record)?) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.nlink() != 1 || !trusted_owner(meta.uid()) || meta.len() > max as u64
    {
        return Err(denied("login record has an unsafe owner, kind or size"));
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(denied("login record exceeds its limit"));
    }
    Ok(Some(bytes))
}

/// Atomic replacement inside the existing protected directory, with the
/// owner and mode the installer gives the record.
pub fn write_self_record(record: Record, bytes: &[u8]) -> io::Result<()> {
    if !privileged() {
        return Err(denied("login records are replaced only by root"));
    }
    let (uid, gid) = gateway_account()?;
    let (owner, group, mode) = match record {
        Record::Policy => (0, gid, 0o640),
        Record::Ledger => (0, 0, 0o600),
        Record::Network => (uid, gid, 0o600),
        Record::MachineKey | Record::Address => {
            return Err(denied("this login record is never replaced by the broker"))
        }
    };
    let path = record_path(record)?;
    let dir = path
        .parent()
        .ok_or_else(|| denied("login record has no directory"))?;
    let meta = fs::symlink_metadata(dir)?;
    if !meta.is_dir() || !trusted_owner(meta.uid()) || meta.mode() & 0o022 != 0 {
        return Err(denied("login record directory is not protected"));
    }
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| denied("invalid login record name"))?;
    let temporary = dir.join(format!(".{name}-{}", nonce()?));
    let mut file = create_stage_file(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        if unsafe { libc::fchown(file.as_raw_fd(), owner, group) } != 0 {
            return Err(io::Error::last_os_error());
        }
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        File::open(dir)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Worker side: the socket must be served by root.
pub fn connect_grants() -> io::Result<UnixStream> {
    let stream = UnixStream::connect(GRANTS_SOCKET)?;
    if peer_uid(&stream)? != 0 {
        return Err(denied("login grant socket is not served by root"));
    }
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(stream)
}

/// Broker side: every local account may connect; the kernel names it.
/// One client at a time with five-second deadlines, beside the gateway
/// socket, so a slow client never delays a login operation.
pub(super) fn start_grant_listener(stop: &'static AtomicBool) -> io::Result<()> {
    let directory = fs::symlink_metadata(GRANTS_DIR)?;
    if !directory.is_dir() || directory.uid() != 0 {
        return Err(denied("login grant runtime directory must be root-owned"));
    }
    fs::set_permissions(GRANTS_DIR, fs::Permissions::from_mode(0o755))?;
    if let Ok(meta) = fs::symlink_metadata(GRANTS_SOCKET) {
        if !meta.file_type().is_socket() || meta.uid() != 0 {
            return Err(denied("untrusted existing login grant socket"));
        }
        fs::remove_file(GRANTS_SOCKET)?;
    }
    let listener = UnixListener::bind(GRANTS_SOCKET)?;
    fs::set_permissions(GRANTS_SOCKET, fs::Permissions::from_mode(0o666))?;
    listener.set_nonblocking(true)?;
    let gateway = gateway_account()?.0;
    std::thread::Builder::new()
        .name("login-grants".into())
        .spawn(move || serve_grants(listener, gateway, stop, Path::new(GRANTS_SOCKET)))?;
    Ok(())
}

fn serve_grants(listener: UnixListener, gateway: u32, stop: &'static AtomicBool, socket: &Path) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let prepared = stream
                    .set_nonblocking(false)
                    .and_then(|()| stream.set_read_timeout(Some(Duration::from_secs(5))))
                    .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(5))));
                // The gateway's own account never sets grants.
                match (prepared, peer_uid(&stream)) {
                    (Ok(()), Ok(uid)) if uid != gateway => {
                        let caller = Caller {
                            account: uid.to_string(),
                            abilities: abilities(uid),
                        };
                        if let Err(error) = self_channel::serve(&mut stream, &caller) {
                            eprintln!("login grant channel: {error}");
                        }
                    }
                    _ => {}
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(error) => {
                eprintln!("login grant channel: {error}");
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
    let _ = fs::remove_file(socket);
}

#[cfg(test)]
#[path = "self_platform_task_tests.rs"]
mod self_platform_task_tests;
