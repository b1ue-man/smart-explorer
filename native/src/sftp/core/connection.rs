use super::config::SftpConfig;
use super::session::{connect_async, Client};
#[path = "reconnect_gate.rs"]
mod reconnect_gate;
use self::reconnect_gate::{AbsoluteDeadline, Generation, ReconnectAccess, ReconnectGate};
use russh::client;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

const SFTP_CONNECT_DEADLINE: Duration = Duration::from_secs(30);
const SFTP_METADATA_DEADLINE: Duration = Duration::from_secs(20);

pub(super) type SftpMetadataFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, SftpError>> + 'a>>;

pub(super) struct SftpTransport {
    session: client::Handle<Client>,
    sftp: Arc<SftpSession>,
}

pub(super) type SftpGeneration = Generation<SftpTransport>;

impl SftpGeneration {
    pub(super) fn session(&self) -> &client::Handle<Client> {
        &self.value().session
    }

    pub(super) fn sftp(&self) -> &SftpSession {
        &self.value().sftp
    }
}

pub(super) struct SftpConnection {
    runtime: Arc<Runtime>,
    // Authentication material is retained only so a dead generation can be
    // replaced without asking the UI for credentials again.
    config: Arc<SftpConfig>,
    reconnect: ReconnectGate<SftpTransport>,
}

impl SftpConnection {
    pub(super) fn connect(config: SftpConfig) -> io::Result<Arc<Self>> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
        );
        let config = Arc::new(config);
        let transport = connect_transport(&runtime, &config)?;
        Ok(Arc::new(Self {
            runtime,
            config,
            reconnect: ReconnectGate::new(transport),
        }))
    }

    pub(super) fn runtime(&self) -> Arc<Runtime> {
        self.runtime.clone()
    }

    /// Run an idempotent metadata request under a fixed deadline. A proven
    /// dead transport may be reconnected and replayed once; a deadline only
    /// makes the generation suspect, so it is retired without replay.
    pub(super) fn safe_metadata<T>(
        &self,
        operation: impl for<'a> Fn(&'a SftpGeneration) -> SftpMetadataFuture<'a, T>,
    ) -> io::Result<T> {
        let deadline = AbsoluteDeadline::after(SFTP_METADATA_DEADLINE);
        let mut generation = self.current_until(deadline)?;
        for attempt in 0..2 {
            match block_on_sftp_operation(&self.runtime, deadline, operation(&generation)) {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(error)) => {
                    let retry_safe = self.note_sftp_error(&generation, &error);
                    if attempt == 0 && retry_safe {
                        generation = self.current_until(deadline)?;
                        continue;
                    }
                    return Err(super::io_err(error));
                }
                Err(error) => {
                    self.note_io_error(&generation, &error);
                    return Err(error);
                }
            }
        }
        unreachable!("bounded SFTP metadata attempts")
    }

    /// Return a usable generation. Reconnection happens before an operation is
    /// dispatched and is serialized so concurrent observers install one
    /// replacement rather than independent SSH sessions.
    pub(super) fn current(&self) -> io::Result<Arc<SftpGeneration>> {
        self.current_with_deadline(None)
    }

    fn current_until(&self, deadline: AbsoluteDeadline) -> io::Result<Arc<SftpGeneration>> {
        self.current_with_deadline(Some(deadline))
    }

    fn current_with_deadline(
        &self,
        deadline: Option<AbsoluteDeadline>,
    ) -> io::Result<Arc<SftpGeneration>> {
        match self.reconnect.acquire(deadline, |generation| {
            !generation.is_stale() && !generation.value().session.is_closed()
        })? {
            ReconnectAccess::Current(generation) => Ok(generation),
            ReconnectAccess::Reconnect(reconnect) => {
                let replacement = match deadline {
                    Some(deadline) => {
                        connect_transport_until(&self.runtime, &self.config, deadline)
                    }
                    None => connect_transport(&self.runtime, &self.config),
                };
                reconnect.finish_until(deadline, replacement)
            }
        }
    }

    pub(super) fn mark_stale(&self, generation: &SftpGeneration) {
        generation.mark_stale();
    }

    pub(super) fn note_sftp_error(&self, generation: &SftpGeneration, error: &SftpError) -> bool {
        let disposition = if generation.session().is_closed() {
            FailureDisposition::dead()
        } else {
            classify_sftp_error(error)
        };
        if disposition.retire {
            generation.mark_stale();
        }
        disposition.retry_safe
    }

    pub(super) fn note_ssh_error(&self, generation: &SftpGeneration, error: &russh::Error) -> bool {
        let dead = generation.session().is_closed() || ssh_error_proves_dead(error);
        if dead {
            generation.mark_stale();
        }
        dead
    }

    pub(super) fn note_io_error(&self, generation: &SftpGeneration, error: &io::Error) -> bool {
        let disposition = if generation.session().is_closed() {
            FailureDisposition::dead()
        } else {
            classify_io_error(error)
        };
        if disposition.retire {
            generation.mark_stale();
        }
        disposition.retry_safe
    }
}

fn connect_transport(runtime: &Runtime, config: &SftpConfig) -> io::Result<SftpTransport> {
    let (session, sftp) = block_on_connect(runtime, SFTP_CONNECT_DEADLINE, connect_async(config))?;
    Ok(SftpTransport {
        session,
        sftp: Arc::new(sftp),
    })
}

fn connect_transport_until(
    runtime: &Runtime,
    config: &SftpConfig,
    deadline: AbsoluteDeadline,
) -> io::Result<SftpTransport> {
    let (session, sftp) = block_on_absolute(
        runtime,
        deadline,
        "transport reconnect",
        connect_async(config),
    )??;
    Ok(SftpTransport {
        session,
        sftp: Arc::new(sftp),
    })
}

fn block_on_connect<F, T>(runtime: &Runtime, deadline: Duration, future: F) -> io::Result<T>
where
    F: Future<Output = io::Result<T>>,
{
    runtime.block_on(async {
        tokio::time::timeout(deadline, future).await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "SFTP connect/auth/subsystem setup timed out",
            )
        })?
    })
}

fn block_on_sftp_operation<F, T>(
    runtime: &Runtime,
    deadline: AbsoluteDeadline,
    future: F,
) -> io::Result<Result<T, SftpError>>
where
    F: Future<Output = Result<T, SftpError>>,
{
    block_on_absolute(runtime, deadline, "metadata operation", future)
}

fn block_on_absolute<F, T>(
    runtime: &Runtime,
    deadline: AbsoluteDeadline,
    stage: &str,
    future: F,
) -> io::Result<T>
where
    F: Future<Output = T>,
{
    deadline.remaining(stage)?;
    let expires = tokio::time::Instant::from_std(deadline.expires());
    let value = runtime.block_on(async {
        tokio::time::timeout_at(expires, future)
            .await
            .map_err(|_| deadline.timeout(stage))
    })?;
    // Tokio intentionally lets an immediately-ready future win even when its
    // deadline has just elapsed. Recheck here so this synchronous API never
    // reports a response after the shared metadata budget expired.
    deadline.remaining(stage)?;
    Ok(value)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FailureDisposition {
    retire: bool,
    retry_safe: bool,
}

impl FailureDisposition {
    const fn dead() -> Self {
        Self {
            retire: true,
            retry_safe: true,
        }
    }

    const fn suspect() -> Self {
        Self {
            retire: true,
            retry_safe: false,
        }
    }

    const fn healthy() -> Self {
        Self {
            retire: false,
            retry_safe: false,
        }
    }
}

fn classify_sftp_error(error: &SftpError) -> FailureDisposition {
    match error {
        SftpError::IO(_) => FailureDisposition::dead(),
        SftpError::UnexpectedBehavior(message) if transport_message(message) => {
            FailureDisposition::dead()
        }
        // These make the current SFTP request stream unsafe for follow-up
        // operations, but do not prove that replaying now is race-free.
        SftpError::Timeout | SftpError::UnexpectedPacket | SftpError::UnexpectedBehavior(_) => {
            FailureDisposition::suspect()
        }
        SftpError::Status(_) | SftpError::Limited(_) => FailureDisposition::healthy(),
    }
}

fn ssh_error_proves_dead(error: &russh::Error) -> bool {
    matches!(
        error,
        russh::Error::Disconnect
            | russh::Error::HUP
            | russh::Error::ConnectionTimeout
            | russh::Error::KeepaliveTimeout
            | russh::Error::InactivityTimeout
            | russh::Error::SendError
            | russh::Error::RecvError
    ) || matches!(error, russh::Error::IO(error) if classify_io_error(error).retry_safe)
}

fn classify_io_error(error: &io::Error) -> FailureDisposition {
    if matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof
    ) || (error.kind() == io::ErrorKind::Other && transport_message(&error.to_string()))
    {
        FailureDisposition::dead()
    } else if error.kind() == io::ErrorKind::TimedOut {
        FailureDisposition::suspect()
    } else {
        FailureDisposition::healthy()
    }
}

fn transport_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "session closed",
        "sender dropped",
        "senderror",
        "recverror",
        "broken pipe",
        "connection reset",
        "unexpected eof",
        "write channel closed",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_sftp_stale_generation_is_replaced_once() {
        let gate = ReconnectGate::new("first");
        let first = match gate.acquire(None, |_| true).unwrap() {
            ReconnectAccess::Current(generation) => generation,
            ReconnectAccess::Reconnect(_) => panic!("fresh generation must be current"),
        };
        first.mark_stale();

        let reconnect = match gate
            .acquire(None, |generation| !generation.is_stale())
            .unwrap()
        {
            ReconnectAccess::Reconnect(reconnect) => reconnect,
            ReconnectAccess::Current(_) => panic!("stale generation must be replaced"),
        };
        let replacement = reconnect.finish_until(None, Ok("second")).unwrap();
        assert_eq!(*replacement.value(), "second");

        // A late failure from generation one cannot poison generation two.
        first.mark_stale();
        let current = match gate
            .acquire(None, |generation| !generation.is_stale())
            .unwrap()
        {
            ReconnectAccess::Current(generation) => generation,
            ReconnectAccess::Reconnect(_) => panic!("replacement must remain current"),
        };
        assert!(!current.is_stale());
        assert!(Arc::ptr_eq(&replacement, &current));
    }

    #[test]
    fn remote_drive_task_sftp_only_transport_failures_retire_for_safe_replay() {
        assert_eq!(
            classify_io_error(&io::Error::new(io::ErrorKind::ConnectionReset, "reset")),
            FailureDisposition::dead()
        );
        assert_eq!(
            classify_sftp_error(&SftpError::UnexpectedBehavior("sender dropped".into())),
            FailureDisposition::dead()
        );
        assert_eq!(
            classify_io_error(&io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
            FailureDisposition::healthy()
        );
    }

    #[test]
    fn remote_drive_task_sftp_timeout_retires_without_immediate_replay() {
        assert_eq!(
            classify_sftp_error(&SftpError::Timeout),
            FailureDisposition::suspect()
        );
        assert_eq!(
            classify_sftp_error(&SftpError::UnexpectedPacket),
            FailureDisposition::suspect()
        );
        assert_eq!(
            classify_io_error(&io::Error::new(io::ErrorKind::TimedOut, "timed out")),
            FailureDisposition::suspect()
        );
    }

    #[test]
    fn remote_drive_task_sftp_connect_deadline_bounds_blackholed_setup() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let error = block_on_connect(
            &runtime,
            Duration::from_millis(20),
            std::future::pending::<io::Result<()>>(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn remote_drive_task_sftp_metadata_future_has_absolute_deadline() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let error = block_on_sftp_operation(
            &runtime,
            AbsoluteDeadline::after(Duration::from_millis(20)),
            std::future::pending::<Result<(), SftpError>>(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(classify_io_error(&error), FailureDisposition::suspect());
    }
}
