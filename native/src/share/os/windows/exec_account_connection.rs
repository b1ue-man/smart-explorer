//! One connection of the Windows account service (C2, D.2.7). The service
//! learns the calling user only from the kernel: after the first frame it
//! impersonates the client (`ImpersonateNamedPipeClient`), reads the user of
//! the thread token and reverts at once. A process id is never used to
//! decide anything. A user the configuration does not list gets
//! `not_allowed` for its first frame, and so does every account the
//! configuration does not list for that user; the configuration is read
//! once per connection. Sessions belong to their connection: when the
//! worker closes it or dies, the service ends their processes.
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::io::FromRawHandle;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{RevertToSelf, TOKEN_QUERY};
use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
use windows_sys::Win32::System::Pipes::{DisconnectNamedPipe, ImpersonateNamedPipeClient};
use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

use crate::share::exec_helper_protocol::{
    read_frame, write_frame, HelperErrorCode, HelperReply, HelperRequest, HELPER_PROTOCOL,
};

use super::exec_account_config::{load_config, ServiceConfig};
use super::exec_account_files::append_log;
use super::exec_account_s4u::{check_account, probe};
use super::exec_account_security::{token_user, Sid};
use super::exec_account_service::{lock, Shared};
use super::exec_account_spawn::Session;
use super::exec_handles::OwnedHandle;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Serves the connected pipe until the worker closes it, then ends every
/// session of it.
pub(super) fn serve_connection(pipe: OwnedHandle, shared: &Shared) {
    let raw = pipe.raw();
    // SAFETY: the handle is open and owned by nothing else from here on.
    let mut stream = unsafe { File::from_raw_handle(pipe.into_raw().cast()) };
    let mut connection = Connection {
        caller: String::from("unknown"),
        sessions: Vec::new(),
    };
    if let Err(error) = connection.serve(&mut stream, raw, shared) {
        connection.log(&format!("connection failed: {error}"));
    }
    for session in connection.sessions.drain(..) {
        let account = session.account().to_string();
        if !session.close() {
            append_log(&format!(
                "user {}: a session as {account} did not end within 10 seconds",
                connection.caller
            ));
        }
    }
    // `DisconnectNamedPipe` discards what the client has not read yet, and
    // `File::flush` does nothing: without `FlushFileBuffers` (it returns once
    // the client has read everything, or at once when the client is gone) a
    // refusal written just before would reach the worker as error 233
    // ("no process on the other end of the pipe") instead of as its reply.
    unsafe {
        FlushFileBuffers(raw);
        DisconnectNamedPipe(raw);
    }
}

struct Connection {
    /// The caller's SID as text, for the log.
    caller: String,
    sessions: Vec<Session>,
}

impl Connection {
    fn serve(&mut self, stream: &mut File, pipe: HANDLE, shared: &Shared) -> io::Result<()> {
        // The first frame must be read before the client can be identified.
        let first = match read_frame::<HelperRequest>(stream) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(()),
            Err(error) => return refuse(stream, HelperErrorCode::BadRequest, error.to_string()),
        };
        let caller = match client_user(pipe) {
            Ok(caller) => caller,
            Err(error) => {
                append_log(&format!("refused a client it could not identify: {error}"));
                let detail = "the account service could not identify the caller".to_string();
                return refuse(stream, HelperErrorCode::NotAllowed, detail);
            }
        };
        self.caller = caller.text()?;
        let config = match load_config() {
            Ok(Some(config)) if config.has_user(&self.caller) => config,
            Ok(_) => {
                self.log("refused: not registered with the account service");
                let detail = format!(
                    "user {} is not registered with the account service",
                    self.caller
                );
                return refuse(stream, HelperErrorCode::NotAllowed, detail);
            }
            Err(error) => {
                self.log(&format!("refused: the configuration is invalid: {error}"));
                let detail =
                    format!("the configuration of the account service is invalid: {error}");
                return refuse(stream, HelperErrorCode::Outdated, detail);
            }
        };
        match first {
            HelperRequest::Hello { protocol, .. } if protocol == HELPER_PROTOCOL => {
                let hello = HelperReply::HelloOk {
                    protocol: HELPER_PROTOCOL,
                    se_version: VERSION.into(),
                };
                write_frame(stream, &hello)?;
            }
            HelperRequest::Hello {
                protocol,
                se_version,
            } => {
                let detail = format!(
                    "the worker (Smart Explorer {se_version}) speaks protocol {protocol}, this \
                     account service (Smart Explorer {VERSION}) protocol {HELPER_PROTOCOL}"
                );
                return refuse(stream, HelperErrorCode::Outdated, detail);
            }
            _ => return refuse(stream, HelperErrorCode::BadRequest, hello_first()),
        }
        loop {
            let request = match read_frame::<HelperRequest>(stream) {
                Ok(Some(request)) => request,
                Ok(None) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    return refuse(stream, HelperErrorCode::BadRequest, error.to_string())
                }
                Err(error) => return Err(error),
            };
            shared.touch();
            let reply = match request {
                HelperRequest::Hello { .. } => {
                    return refuse(stream, HelperErrorCode::BadRequest, hello_first())
                }
                HelperRequest::Probe { account } => {
                    if config.allows(&self.caller, &account) {
                        probe(&lock(&shared.lsa), &account)
                    } else {
                        error_reply(not_allowed(&account, &self.caller))
                    }
                }
                HelperRequest::Start {
                    account,
                    pipe_in,
                    pipe_out,
                    ..
                } => self.start(
                    &config,
                    &caller,
                    &account,
                    (pipe_in.as_deref(), pipe_out.as_deref()),
                    shared,
                ),
                HelperRequest::Stop { session } => self.stop(&session),
                HelperRequest::Status { session } => self.status(&session),
            };
            write_frame(stream, &reply)?;
        }
    }

    fn start(
        &mut self,
        config: &ServiceConfig,
        caller: &Sid,
        account: &str,
        (pipe_in, pipe_out): (Option<&str>, Option<&str>),
        shared: &Shared,
    ) -> HelperReply {
        if !config.allows(&self.caller, account) {
            return self.refused(account, not_allowed(account, &self.caller));
        }
        let checked = match check_account(account) {
            Ok(checked) => checked,
            Err(refusal) => return self.refused(account, refusal.reply()),
        };
        match Session::start(&shared.lsa, caller, &checked, pipe_in, pipe_out) {
            Ok(session) => {
                self.log(&format!("started session {} as {account}", session.id()));
                let id = session.id().to_string();
                self.sessions.push(session);
                HelperReply::Started { session: id }
            }
            Err(refusal) => self.refused(account, refusal),
        }
    }

    fn refused(&self, account: &str, (code, detail): (HelperErrorCode, String)) -> HelperReply {
        self.log(&format!("refused a start as {account}: {detail}"));
        HelperReply::Error { code, detail }
    }

    fn stop(&self, id: &str) -> HelperReply {
        match self.sessions.iter().find(|session| session.id() == id) {
            Some(session) => match session.stop() {
                Ok(()) => HelperReply::Stopped {},
                Err(error) => error_reply((HelperErrorCode::StartFailed, error.to_string())),
            },
            None => error_reply(no_session()),
        }
    }

    fn status(&mut self, id: &str) -> HelperReply {
        let Some(session) = self.sessions.iter_mut().find(|session| session.id() == id) else {
            return error_reply(no_session());
        };
        match session.status() {
            Ok((active, empty)) => {
                if !active && empty {
                    session.release_profile();
                }
                HelperReply::Status { active, empty }
            }
            Err(error) => error_reply((HelperErrorCode::StartFailed, error.to_string())),
        }
    }

    fn log(&self, message: &str) {
        append_log(&format!("user {}: {message}", self.caller));
    }
}

/// The user at the other end of `pipe` as the kernel recorded it. The
/// client allows identification only; the thread reverts before it
/// returns, and a thread that cannot revert ends the process.
fn client_user(pipe: HANDLE) -> io::Result<Sid> {
    if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _revert = Revert;
    let mut token = null_mut();
    // `OpenAsSelf`: the service's own rights open the token, which an
    // identification-level impersonation could not.
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle::new(token)?;
    token_user(token.raw())
}

struct Revert;

impl Drop for Revert {
    fn drop(&mut self) {
        if unsafe { RevertToSelf() } == 0 {
            // Never go on serving with the client's identity.
            std::process::abort();
        }
    }
}

/// Answers with an error and ends the connection.
fn refuse(stream: &mut File, code: HelperErrorCode, detail: String) -> io::Result<()> {
    write_frame(stream, &HelperReply::Error { code, detail })?;
    stream.flush()
}

fn error_reply((code, detail): (HelperErrorCode, String)) -> HelperReply {
    HelperReply::Error { code, detail }
}

fn not_allowed(account: &str, caller: &str) -> (HelperErrorCode, String) {
    (
        HelperErrorCode::NotAllowed,
        format!("account {account} is not allowed for the remote commands of user {caller}"),
    )
}

fn no_session() -> (HelperErrorCode, String) {
    (
        HelperErrorCode::BadRequest,
        "no such session on this connection".into(),
    )
}

fn hello_first() -> String {
    "hello comes first and only once".into()
}
