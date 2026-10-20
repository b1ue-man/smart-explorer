//! The account service (PRM-M6): `--exec-account-helper`, one root process
//! per connection of `smart-explorer-exec-helper.socket` (`Accept=yes`),
//! hardened by its unit (`exec_helper_setup.rs`). The kernel names the uid
//! of the worker at the other end (`SO_PEERCRED`); the configuration says
//! which accounts that uid may use. A uid the configuration does not list
//! gets `not_allowed` for every request. The service starts only listed
//! accounts and never root, each as an account unit in the system manager
//! with the descriptor the worker sent (`SCM_RIGHTS`) and the program
//! checked against this service's own image. When the connection ends, or
//! systemd stops the service, it stops every unit it started for the
//! connection and waits until they are empty. Its log is standard error,
//! which systemd writes to the journal; it never sees what a job runs.
use std::ffi::OsString;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;

use crate::share::exec_helper_protocol::{
    decode_payload, write_frame, HelperErrorCode, HelperReply, HelperRequest, HELPER_PROTOCOL,
};
use crate::share::exec_types::ExecId;

use super::exec_account::{self, AccountRefusal};
use super::exec_account_unit::{AccountUnit, AccountUnitStart, UNIT_PREFIX};
use super::exec_binary_copy::{self, PROGRAM_COPY};
use super::exec_helper_client::{peer_credentials, recv_frame, HELPER_SOCKET};
use super::exec_helper_config::{self, HelperConfig};
use super::exec_pam_service;
use super::exec_systemd::eio;

pub(super) const HELPER_MODE: &str = "--exec-account-helper";
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a waiting read looks whether systemd asked the service to stop.
const STOP_POLL: Duration = Duration::from_secs(1);
/// First descriptor systemd passes (`SD_LISTEN_FDS_START`).
const LISTEN_FD: RawFd = 3;
const VERSION: &str = env!("CARGO_PKG_VERSION");

const MANAGED: &str = "# Managed by Smart Explorer (se share exec accounts): the account service\n\
                       # that runs remote commands as other local accounts.";

/// Set by `SIGTERM`: systemd stops the service.
static STOPPING: AtomicBool = AtomicBool::new(false);

pub(super) fn run_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    (arguments.len() == 1 && arguments[0] == HELPER_MODE).then(run)
}

fn run() -> io::Result<()> {
    // SAFETY: `geteuid` only reads this process's id.
    if unsafe { libc::geteuid() } != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the account service runs as root from its systemd unit only",
        ));
    }
    install_stop_handler()?;
    let stream = activation_connection()?;
    stream.set_read_timeout(Some(STOP_POLL))?;
    stream.set_write_timeout(Some(CLEANUP_TIMEOUT))?;
    let peer = peer_credentials(&stream)?;
    let mut service = Service {
        stream,
        uid: peer.uid,
        pid: peer.pid,
        sessions: Vec::new(),
        manager: None,
    };
    let result = service.serve();
    service.stop_all();
    match result {
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(()),
        result => result,
    }
}

struct Service {
    stream: UnixStream,
    uid: u32,
    pid: i32,
    sessions: Vec<AccountUnit>,
    manager: Option<Connection>,
}

impl Service {
    fn serve(&mut self) -> io::Result<()> {
        // The configuration must list the worker's uid; otherwise the reply
        // to its first frame is the refusal, and the connection ends.
        let admitted = match exec_helper_config::load() {
            Ok(config)
                if config
                    .as_ref()
                    .is_some_and(|config| config.has_user(self.uid)) =>
            {
                Ok(config.unwrap_or_default())
            }
            Ok(_) => Err((
                HelperErrorCode::NotAllowed,
                format!(
                    "user {} is not registered with the account service",
                    self.uid
                ),
            )),
            Err(error) => Err((
                HelperErrorCode::Outdated,
                format!("the configuration of the account service is invalid: {error}"),
            )),
        };
        let mut greeted = false;
        loop {
            let Some((payload, descriptor)) = recv_frame(&self.stream, Some(&STOPPING))? else {
                return Ok(());
            };
            let config = match &admitted {
                Ok(config) => config,
                Err((code, detail)) => {
                    self.log(&format!("refused: {detail}"));
                    return self.refuse(*code, detail.clone());
                }
            };
            let request = match decode_payload::<HelperRequest>(&payload) {
                Ok(request) => request,
                Err(error) => return self.refuse(HelperErrorCode::BadRequest, error.to_string()),
            };
            let reply = match (greeted, request) {
                (false, HelperRequest::Hello { protocol, .. }) if protocol == HELPER_PROTOCOL => {
                    greeted = true;
                    HelperReply::HelloOk {
                        protocol: HELPER_PROTOCOL,
                        se_version: VERSION.into(),
                    }
                }
                (
                    false,
                    HelperRequest::Hello {
                        protocol,
                        se_version,
                    },
                ) => {
                    return self.refuse(
                        HelperErrorCode::Outdated,
                        format!(
                            "the worker (Smart Explorer {se_version}) speaks protocol \
                             {protocol}, this account service (Smart Explorer {VERSION}) \
                             protocol {HELPER_PROTOCOL}"
                        ),
                    );
                }
                (false, _) | (true, HelperRequest::Hello { .. }) => {
                    return self.refuse(
                        HelperErrorCode::BadRequest,
                        "hello comes first and only once".into(),
                    );
                }
                (true, HelperRequest::Probe { account }) => self.probe(config, &account),
                (true, HelperRequest::Start { account, cwd, .. }) => {
                    self.start(config, &account, cwd.as_deref(), descriptor)
                }
                (true, HelperRequest::Stop { session }) => self.stop(&session),
                (true, HelperRequest::Status { session }) => self.status(&session),
            };
            write_frame(&mut &self.stream, &reply)?;
        }
    }

    /// Answers with an error and ends the connection.
    fn refuse(&self, code: HelperErrorCode, detail: String) -> io::Result<()> {
        write_frame(&mut &self.stream, &HelperReply::Error { code, detail })
    }

    fn probe(&self, config: &HelperConfig, account: &str) -> HelperReply {
        if !config.allows(self.uid, account) {
            return error_reply(not_allowed(account, self.uid));
        }
        match exec_account::check_account(account) {
            Ok(checked) => HelperReply::Probe {
                ok: true,
                elevated: false,
                admin_group: exec_account::admin_group(&checked),
                detail: exec_account::shell_warning(&checked),
            },
            Err(refusal) => HelperReply::Probe {
                ok: false,
                elevated: refusal == AccountRefusal::Root,
                admin_group: false,
                detail: Some(refusal.detail(account)),
            },
        }
    }

    fn start(
        &mut self,
        config: &HelperConfig,
        account: &str,
        cwd: Option<&str>,
        descriptor: Option<OwnedFd>,
    ) -> HelperReply {
        match self.start_unit(config, account, cwd, descriptor) {
            Ok(unit) => {
                self.log(&format!(
                    "started {} (pid {}) as {account}",
                    unit.name(),
                    unit.main_pid()
                ));
                let session = unit.name().to_string();
                self.sessions.push(unit);
                HelperReply::Started { session }
            }
            Err((code, detail)) => {
                self.log(&format!("refused a start as {account}: {detail}"));
                HelperReply::Error { code, detail }
            }
        }
    }

    fn start_unit(
        &mut self,
        config: &HelperConfig,
        account: &str,
        cwd: Option<&str>,
        descriptor: Option<OwnedFd>,
    ) -> Result<AccountUnit, (HelperErrorCode, String)> {
        if !config.allows(self.uid, account) {
            return Err(not_allowed(account, self.uid));
        }
        let descriptor = descriptor
            .ok_or_else(|| bad_request("a start needs the supervisor's end of its connection"))?;
        stream_socket(&descriptor).map_err(|error| bad_request(&error.to_string()))?;
        exec_account::check_account(account)
            .map_err(|refusal| (refusal.helper_code(), refusal.detail(account)))?;
        exec_pam_service::verify_installed().map_err(outdated)?;
        let program = exec_binary_copy::helper_program().map_err(outdated)?;
        let connection = self.manager().map_err(start_failed)?;
        let id = ExecId::generate().map_err(start_failed)?;
        let name = format!("{UNIT_PREFIX}{}.service", id.as_str());
        let start = AccountUnitStart {
            account,
            cwd,
            program: &program,
            runtime_usec: None,
            stdio: descriptor.as_fd(),
        };
        AccountUnit::start(connection, name, &start).map_err(start_failed)
    }

    fn stop(&self, session: &str) -> HelperReply {
        match self.session(session) {
            Ok(unit) => match unit.stop() {
                Ok(()) => HelperReply::Stopped {},
                Err(error) => error_reply(start_failed(error)),
            },
            Err(reply) => reply,
        }
    }

    fn status(&self, session: &str) -> HelperReply {
        match self.session(session) {
            Ok(unit) => match unit.status() {
                Ok(status) => {
                    if status.finished() {
                        unit.collect();
                    }
                    HelperReply::Status {
                        active: status.active,
                        empty: status.empty,
                    }
                }
                Err(error) => error_reply(start_failed(error)),
            },
            Err(reply) => reply,
        }
    }

    /// A session of this connection; others do not exist for it.
    fn session(&self, session: &str) -> Result<&AccountUnit, HelperReply> {
        self.sessions
            .iter()
            .find(|unit| unit.name() == session)
            .ok_or_else(|| error_reply(bad_request("no such session on this connection")))
    }

    fn manager(&mut self) -> io::Result<Connection> {
        if let Some(connection) = &self.manager {
            return Ok(connection.clone());
        }
        let connection = Connection::system().map_err(eio)?;
        self.manager = Some(connection.clone());
        Ok(connection)
    }

    /// Stops every unit of the connection and waits until they are empty.
    fn stop_all(&mut self) {
        for unit in &self.sessions {
            if let Err(error) = unit.stop() {
                self.log(&format!("could not stop {}: {error}", unit.name()));
            }
        }
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        for unit in &self.sessions {
            loop {
                match unit.status() {
                    Ok(status) if status.finished() => {
                        unit.collect();
                        break;
                    }
                    Ok(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    other => {
                        self.log(&format!("{} is not empty: {other:?}", unit.name()));
                        break;
                    }
                }
            }
        }
        self.sessions.clear();
    }

    fn log(&self, message: &str) {
        eprintln!(
            "smart-explorer-exec-helper: uid {} pid {}: {message}",
            self.uid, self.pid
        );
    }
}

/// The socket unit: one service instance per connection (`Accept=yes`).
pub(super) fn socket_unit_text() -> String {
    format!(
        "{MANAGED}\n[Unit]\nDescription=Smart Explorer account service socket\n\n\
         [Socket]\nListenStream={HELPER_SOCKET}\nSocketMode=0666\nDirectoryMode=0755\n\
         Accept=yes\nMaxConnections=64\nRemoveOnStop=yes\n\n\
         [Install]\nWantedBy=sockets.target\n"
    )
}

/// The hardening of C3 (NR N16): no capabilities, no new privileges, a
/// read-only system, no home directories, Unix sockets only.
pub(super) fn service_unit_text() -> String {
    format!(
        "{MANAGED}\n[Unit]\nDescription=Smart Explorer account service\n\
         CollectMode=inactive-or-failed\n\n\
         [Service]\nType=exec\nExecStart={PROGRAM_COPY} {HELPER_MODE}\n\
         StandardInput=null\nStandardOutput=journal\nStandardError=journal\n\
         TimeoutStopSec=20\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\n\
         NoNewPrivileges=yes\nRestrictAddressFamilies=AF_UNIX\nCapabilityBoundingSet=\n"
    )
}

fn not_allowed(account: &str, uid: u32) -> (HelperErrorCode, String) {
    (
        HelperErrorCode::NotAllowed,
        format!("account {account} is not allowed for the remote commands of user {uid}"),
    )
}

fn error_reply((code, detail): (HelperErrorCode, String)) -> HelperReply {
    HelperReply::Error { code, detail }
}

fn bad_request(detail: &str) -> (HelperErrorCode, String) {
    (HelperErrorCode::BadRequest, detail.into())
}

fn outdated(error: io::Error) -> (HelperErrorCode, String) {
    (
        HelperErrorCode::Outdated,
        format!("{error}; update the account service"),
    )
}

fn start_failed(error: io::Error) -> (HelperErrorCode, String) {
    (HelperErrorCode::StartFailed, error.to_string())
}

/// The connection systemd accepted for this instance: `LISTEN_PID` is this
/// process, `LISTEN_FDS` is 1, the descriptor is 3 and a socket.
fn activation_connection() -> io::Result<UnixStream> {
    let pid = std::env::var("LISTEN_PID").ok();
    let count = std::env::var("LISTEN_FDS").ok();
    if pid.as_deref() != Some(std::process::id().to_string().as_str())
        || count.as_deref() != Some("1")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the account service needs exactly one connection from systemd",
        ));
    }
    // SAFETY: only asks whether descriptor 3 is open.
    if unsafe { libc::fcntl(LISTEN_FD, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: systemd passed the open descriptor 3 to this process, which
    // owns it from here on; nothing else in the process uses it.
    let descriptor = unsafe { OwnedFd::from_raw_fd(LISTEN_FD) };
    stream_socket(&descriptor)?;
    // SAFETY: sets the close-on-exec flag of a descriptor this process owns.
    if unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(UnixStream::from(descriptor))
}

/// `descriptor` is a stream socket of the Unix family.
fn stream_socket(descriptor: &OwnedFd) -> io::Result<()> {
    let option = |name| {
        let mut value: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: `getsockopt` writes one `c_int` into `value`.
        let result = unsafe {
            libc::getsockopt(
                descriptor.as_raw_fd(),
                libc::SOL_SOCKET,
                name,
                (&mut value as *mut libc::c_int).cast(),
                &mut size,
            )
        };
        if result == 0 {
            Ok(value)
        } else {
            Err(io::Error::last_os_error())
        }
    };
    if option(libc::SO_DOMAIN)? == libc::AF_UNIX && option(libc::SO_TYPE)? == libc::SOCK_STREAM {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the descriptor is not a Unix stream socket",
        ))
    }
}

extern "C" fn on_stop(_signal: libc::c_int) {
    STOPPING.store(true, Ordering::SeqCst);
}

/// `SIGTERM` sets `STOPPING`; a waiting read sees it within `STOP_POLL`.
fn install_stop_handler() -> io::Result<()> {
    // SAFETY: `sigaction` is plain data; the handler only stores an atomic.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_stop as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
