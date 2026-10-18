//! The worker's side of the Windows account service (PRM-M7, D.2.7). One
//! connection serves one job session; the service ends the session's
//! processes when the connection closes, also when the worker dies. A
//! stopped service is started on demand (registered users may start it).
//! At admission (`resolve_account`) the configuration must list the account
//! for this worker's user and the service's probe must accept it, so a
//! refusal carries its code (`account_not_allowed`,
//! `account_helper_unavailable`, `account_unavailable`) before any process
//! exists.
use std::fs::File;
use std::io;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;

use crate::share::exec_client_messages::{ACCOUNT_HELPER_UNAVAILABLE, ACCOUNT_NOT_ALLOWED};
use crate::share::exec_helper_protocol::{
    account_start_error, read_frame, write_frame, HelperErrorCode, HelperReply, HelperRequest,
    ACCOUNT_UNAVAILABLE, HELPER_PROTOCOL,
};
use crate::share::exec_supervisor_protocol::RunAsAccount;
use crate::share::exec_types::ExecStart;

use super::exec_account_config::load_config;
use super::exec_account_pipes::{ControlPipe, JobPipes};
use super::exec_account_scm::{service_state, start_service};
use super::exec_account_security::{current_user_sid, lookup_account};

/// How long the worker waits for a service it started to open its pipe.
const START_WAIT: Duration = Duration::from_secs(20);

/// Why no connection came about.
pub(super) enum OpenFailure {
    NotInstalled,
    /// The service did not start or does not answer.
    NotRunning(io::Error),
    /// The service refused this user or speaks another protocol.
    Refused(io::Error),
}

impl OpenFailure {
    pub(super) fn into_error(self) -> io::Error {
        match self {
            Self::NotInstalled => account_start_error(
                ACCOUNT_HELPER_UNAVAILABLE,
                "the account service is not set up on this PC",
            ),
            Self::NotRunning(error) => account_start_error(
                ACCOUNT_HELPER_UNAVAILABLE,
                format!("the account service does not answer ({error})"),
            ),
            Self::Refused(error) => error,
        }
    }
}

/// What the service's probe said about an account.
pub(super) enum ProbeOutcome {
    Measured {
        ok: bool,
        elevated: bool,
        admin_group: bool,
        detail: Option<String>,
    },
    Refused(HelperErrorCode, String),
}

/// A connection to the account service after `hello`.
pub(super) struct ServiceConnection {
    pipe: ControlPipe,
    version: String,
}

impl ServiceConnection {
    pub(super) fn open() -> Result<Self, OpenFailure> {
        let pipe = match ControlPipe::open() {
            Ok(Some(pipe)) => pipe,
            Ok(None) => start_and_wait()?,
            Err(error) => return Err(open_error(error)),
        };
        let mut connection = Self {
            pipe,
            version: String::new(),
        };
        match connection.request(&HelperRequest::hello()) {
            Ok(HelperReply::HelloOk {
                protocol,
                se_version,
            }) if protocol == HELPER_PROTOCOL => {
                connection.version = se_version;
                Ok(connection)
            }
            Ok(HelperReply::HelloOk {
                protocol,
                se_version,
            }) => Err(OpenFailure::Refused(account_start_error(
                ACCOUNT_HELPER_UNAVAILABLE,
                format!(
                    "the account service of Smart Explorer {se_version} speaks protocol \
                     {protocol}, this worker protocol {HELPER_PROTOCOL}"
                ),
            ))),
            Ok(other) => Err(OpenFailure::Refused(unexpected(&other))),
            Err(error) => Err(OpenFailure::Refused(error)),
        }
    }

    /// Smart Explorer version of the service.
    pub(super) fn version(&self) -> &str {
        &self.version
    }

    /// One request and its reply as the service sent it.
    fn exchange(&mut self, request: &HelperRequest) -> io::Result<HelperReply> {
        write_frame(&mut self.pipe, request)?;
        read_frame::<HelperReply>(&mut self.pipe)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the account service closed the connection",
            )
        })
    }

    /// One request; an error reply becomes an error with the code the
    /// admission sends.
    fn request(&mut self, request: &HelperRequest) -> io::Result<HelperReply> {
        match self.exchange(request)? {
            HelperReply::Error { code, detail } => {
                Err(account_start_error(code.wire_code(), detail))
            }
            reply => Ok(reply),
        }
    }

    pub(super) fn probe(&mut self, account: &str) -> io::Result<ProbeOutcome> {
        let request = HelperRequest::Probe {
            account: account.into(),
        };
        match self.exchange(&request)? {
            HelperReply::Probe {
                ok,
                elevated,
                admin_group,
                detail,
            } => Ok(ProbeOutcome::Measured {
                ok,
                elevated,
                admin_group,
                detail,
            }),
            HelperReply::Error { code, detail } => Ok(ProbeOutcome::Refused(code, detail)),
            other => Err(unexpected(&other)),
        }
    }
}

/// Starts the installed service and waits for its pipe.
fn start_and_wait() -> Result<ControlPipe, OpenFailure> {
    match service_state() {
        Ok(None) => return Err(OpenFailure::NotInstalled),
        Ok(Some(_)) => {}
        Err(error) => return Err(OpenFailure::NotRunning(error)),
    }
    start_service().map_err(OpenFailure::NotRunning)?;
    let deadline = Instant::now() + START_WAIT;
    loop {
        match ControlPipe::open() {
            Ok(Some(pipe)) => return Ok(pipe),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                return Err(OpenFailure::NotRunning(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the started account service opened no pipe within 20 seconds",
                )))
            }
            Err(error) => return Err(open_error(error)),
        }
    }
}

/// Only SYSTEM and registered users may open the service's pipe.
fn open_error(error: io::Error) -> OpenFailure {
    if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
        OpenFailure::Refused(account_start_error(
            ACCOUNT_NOT_ALLOWED,
            "this Windows user is not registered with the account service",
        ))
    } else {
        OpenFailure::NotRunning(error)
    }
}

/// The session of one job: its connection, whose relay runs the job's
/// supervisor as another account.
pub(super) struct AccountSession {
    connection: ServiceConnection,
    session: String,
}

impl AccountSession {
    /// Asks the service to run the supervisor of `request` as `account`;
    /// returns the session with the writer of the relay's input and the
    /// reader of its output.
    pub(super) fn start(
        request: &ExecStart,
        account: &RunAsAccount,
    ) -> io::Result<(Self, File, File)> {
        let pipes = JobPipes::create()?;
        let (pipe_in, pipe_out) = pipes.names();
        let mut connection = ServiceConnection::open().map_err(OpenFailure::into_error)?;
        let start = HelperRequest::Start {
            account: account.name.clone(),
            cwd: request.cwd.clone(),
            pipe_in: Some(pipe_in),
            pipe_out: Some(pipe_out),
        };
        let session = match connection.request(&start)? {
            HelperReply::Started { session } => session,
            other => return Err(unexpected(&other)),
        };
        let (writer, reader) = pipes.connect()?;
        Ok((
            Self {
                connection,
                session,
            },
            writer,
            reader,
        ))
    }

    pub(super) fn stop(&mut self) -> io::Result<()> {
        let request = HelperRequest::Stop {
            session: self.session.clone(),
        };
        match self.connection.request(&request)? {
            HelperReply::Stopped {} => Ok(()),
            other => Err(unexpected(&other)),
        }
    }

    /// The relay has ended and the session's Job has no process left.
    pub(super) fn finished(&mut self) -> io::Result<bool> {
        let request = HelperRequest::Status {
            session: self.session.clone(),
        };
        match self.connection.request(&request)? {
            HelperReply::Status { active, empty } => Ok(!active && empty),
            other => Err(unexpected(&other)),
        }
    }
}

/// This worker's user as the string form of its SID.
pub(super) fn own_sid() -> io::Result<String> {
    current_user_sid()?.text()
}

/// Whether a grant may name another account here: the service is installed
/// and its configuration lists this worker's user. The same facts as the
/// account choices, without asking the service.
pub(crate) fn run_as_supported() -> bool {
    let Ok(sid) = own_sid() else {
        return false;
    };
    matches!(service_state(), Ok(Some(_)))
        && load_config()
            .ok()
            .flatten()
            .is_some_and(|config| config.has_user(&sid))
}

/// The grant's run-as account at admission: allowed for this worker's
/// user, and accepted by the service's logon probe.
pub(crate) fn resolve_account(name: &str) -> io::Result<RunAsAccount> {
    let sid = own_sid()?;
    let config = load_config().map_err(|error| {
        account_start_error(
            ACCOUNT_HELPER_UNAVAILABLE,
            format!("the configuration of the account service cannot be read: {error}"),
        )
    })?;
    let Some(config) = config else {
        return Err(account_start_error(
            ACCOUNT_HELPER_UNAVAILABLE,
            "the account service is not set up on this PC",
        ));
    };
    if !config.allows(&sid, name) {
        return Err(account_start_error(
            ACCOUNT_NOT_ALLOWED,
            format!("account {name} is not allowed for the remote commands of user {sid}"),
        ));
    }
    let mut connection = ServiceConnection::open().map_err(OpenFailure::into_error)?;
    match connection.probe(name)? {
        ProbeOutcome::Measured { ok: true, .. } => {}
        ProbeOutcome::Measured { detail, .. } => {
            let detail = detail.unwrap_or_else(|| format!("Windows refuses account {name}"));
            return Err(account_start_error(ACCOUNT_UNAVAILABLE, detail));
        }
        ProbeOutcome::Refused(code, detail) => {
            return Err(account_start_error(code.wire_code(), detail));
        }
    }
    let resolved = lookup_account(name).map_err(|error| {
        account_start_error(ACCOUNT_UNAVAILABLE, format!("account {name}: {error}"))
    })?;
    // `uid` is the account's relative id; group, home and shell are the
    // relay's own on Windows (it runs as the account).
    Ok(RunAsAccount {
        name: name.to_string(),
        uid: resolved.sid.rid(),
        gid: 0,
        groups: Vec::new(),
        home: String::new(),
        shell: String::new(),
    })
}

fn unexpected(reply: &HelperReply) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unexpected reply of the account service: {reply:?}"),
    )
}
