use super::config::SftpConfig;
use super::session::{connect_async, Client};
use russh::client;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;

const SFTP_CONNECT_DEADLINE: Duration = Duration::from_secs(30);

pub(super) struct SftpTransport {
    session: client::Handle<Client>,
    sftp: Arc<SftpSession>,
}

pub(super) struct Generation<T> {
    value: T,
    stale: AtomicBool,
}

impl<T> Generation<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            stale: AtomicBool::new(false),
        }
    }

    fn mark_stale(&self) {
        self.stale.store(true, Ordering::Release);
    }

    fn is_stale(&self) -> bool {
        self.stale.load(Ordering::Acquire)
    }
}

struct GenerationSlot<T> {
    current: Arc<Generation<T>>,
}

impl<T> GenerationSlot<T> {
    fn new(value: T) -> Self {
        Self {
            current: Arc::new(Generation::new(value)),
        }
    }

    fn current(&self) -> Arc<Generation<T>> {
        self.current.clone()
    }

    fn replace(&mut self, value: T) -> Arc<Generation<T>> {
        let generation = Arc::new(Generation::new(value));
        self.current = generation.clone();
        generation
    }
}

pub(super) type SftpGeneration = Generation<SftpTransport>;

impl SftpGeneration {
    pub(super) fn session(&self) -> &client::Handle<Client> {
        &self.value.session
    }

    pub(super) fn sftp(&self) -> &SftpSession {
        &self.value.sftp
    }
}

pub(super) struct SftpConnection {
    runtime: Arc<Runtime>,
    // Authentication material is retained only so a dead generation can be
    // replaced without asking the UI for credentials again.
    config: Arc<SftpConfig>,
    state: Mutex<GenerationSlot<SftpTransport>>,
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
            state: Mutex::new(GenerationSlot::new(transport)),
        }))
    }

    pub(super) fn runtime(&self) -> Arc<Runtime> {
        self.runtime.clone()
    }

    /// Return a usable generation. Reconnection happens before an operation is
    /// dispatched and is serialized so concurrent observers install one
    /// replacement rather than independent SSH sessions.
    pub(super) fn current(&self) -> io::Result<Arc<SftpGeneration>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = state.current();
        if !current.is_stale() && !current.session().is_closed() {
            return Ok(current);
        }
        current.mark_stale();
        let replacement = connect_transport(&self.runtime, &self.config)?;
        Ok(state.replace(replacement))
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
    fn scripted_stale_generation_is_replaced_once() {
        let mut slot = GenerationSlot::new("first");
        let first = slot.current();
        first.mark_stale();

        let replacement = if slot.current().is_stale() {
            slot.replace("second")
        } else {
            panic!("scripted first generation must be stale");
        };
        assert_eq!(replacement.value, "second");

        // A late failure from generation one cannot poison generation two.
        first.mark_stale();
        assert!(!slot.current().is_stale());
        assert!(Arc::ptr_eq(&replacement, &slot.current()));
    }

    #[test]
    fn only_transport_failures_mark_a_generation_dead() {
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
    fn timeout_and_protocol_desync_retire_without_immediate_replay() {
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
    fn connect_deadline_bounds_a_blackholed_setup() {
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
}
