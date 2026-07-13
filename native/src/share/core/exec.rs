use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use super::exec_policy::ExecGrant;
use super::types::{ExecRequest, ExecResult};

const MAX_TIMEOUT_MS: u64 = 15 * 60 * 1_000;
const MAX_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ARG_COUNT: usize = 256;
const MAX_ARG_BYTES: usize = 64 * 1024;
const MAX_SINGLE_ARG_BYTES: usize = 16 * 1024;
const MAX_CWD_BYTES: usize = 4 * 1024;
const MAX_GLOBAL_EXEC: usize = 2;
const MAX_PEER_EXEC: usize = 1;

static ACTIVE_EXEC: AtomicUsize = AtomicUsize::new(0);
static PEER_EXEC: OnceLock<Mutex<HashMap<String, Weak<AtomicUsize>>>> = OnceLock::new();

#[derive(Debug)]
pub(super) struct PreparedExec {
    _request: ValidatedExec,
    _permit: ExecPermit,
}

#[derive(Debug)]
struct ValidatedExec {
    _argv: Vec<String>,
    _cwd: Option<String>,
    _timeout: Duration,
    _max_output_bytes: usize,
    _shell: bool,
}

#[derive(Debug)]
struct ExecPermit {
    peer: Arc<AtomicUsize>,
}

impl Drop for ExecPermit {
    fn drop(&mut self) {
        self.peer.fetch_sub(1, Ordering::AcqRel);
        ACTIVE_EXEC.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) fn peer_slots(peer_key: &str) -> Arc<AtomicUsize> {
    let registry = PEER_EXEC.get_or_init(|| Mutex::new(HashMap::new()));
    let mut peers = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    peers.retain(|_, slots| slots.strong_count() > 0);
    if let Some(slots) = peers.get(peer_key).and_then(Weak::upgrade) {
        return slots;
    }
    let slots = Arc::new(AtomicUsize::new(0));
    peers.insert(peer_key.to_string(), Arc::downgrade(&slots));
    slots
}

/// Validates and reserves all execution resources before the caller creates a
/// blocking task or process. Shell and argv execution intentionally share the
/// same full-code-execution permission boundary.
pub(super) fn prepare(
    req: ExecRequest,
    grant: &ExecGrant,
    peer: Arc<AtomicUsize>,
) -> io::Result<PreparedExec> {
    if !grant.enabled {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "remote full-code execution is not enabled for this account",
        ));
    }
    let request = validate(req)?;
    acquire(&ACTIVE_EXEC, MAX_GLOBAL_EXEC, "global")?;
    if let Err(error) = acquire(&peer, MAX_PEER_EXEC, "per-peer") {
        ACTIVE_EXEC.fetch_sub(1, Ordering::AcqRel);
        return Err(error);
    }
    Ok(PreparedExec {
        _request: request,
        _permit: ExecPermit { peer },
    })
}

impl PreparedExec {
    pub(super) fn run(self) -> io::Result<ExecResult> {
        // Arbitrary code can detach from a Unix process group. Until both
        // supported platforms have a typed adapter that guarantees descendant
        // containment and teardown on timeout/disconnect, fail closed instead
        // of offering a partially safe remote-exec implementation.
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "remote execution is disabled: full process-tree containment is unavailable",
        ))
    }
}

fn validate(req: ExecRequest) -> io::Result<ValidatedExec> {
    if req.argv.is_empty() || req.argv[0].trim().is_empty() {
        return Err(invalid("empty remote command"));
    }
    if req.argv.len() > MAX_ARG_COUNT {
        return Err(invalid("remote command has too many arguments"));
    }
    if req.shell && req.argv.len() != 1 {
        return Err(invalid(
            "shell execution accepts exactly one command string",
        ));
    }
    let mut arg_bytes = 0usize;
    for arg in &req.argv {
        if arg.as_bytes().contains(&0) {
            return Err(invalid("remote command contains a NUL byte"));
        }
        if arg.len() > MAX_SINGLE_ARG_BYTES {
            return Err(invalid("remote command argument is too large"));
        }
        arg_bytes = arg_bytes
            .checked_add(arg.len())
            .ok_or_else(|| invalid("remote command size overflow"))?;
        if arg_bytes > MAX_ARG_BYTES {
            return Err(invalid("remote command is too large"));
        }
    }

    let cwd = req.cwd.filter(|cwd| !cwd.trim().is_empty());
    if let Some(cwd) = &cwd {
        if cwd.len() > MAX_CWD_BYTES {
            return Err(invalid("remote working directory is too large"));
        }
        if cwd.as_bytes().contains(&0) {
            return Err(invalid("remote working directory contains a NUL byte"));
        }
    }

    let max_output = req.max_output_bytes.clamp(1, MAX_OUTPUT_BYTES);
    let max_output_bytes = usize::try_from(max_output)
        .map_err(|_| invalid("remote output limit does not fit this platform"))?;
    Ok(ValidatedExec {
        _argv: req.argv,
        _cwd: cwd,
        _timeout: Duration::from_millis(req.timeout_ms.clamp(1, MAX_TIMEOUT_MS)),
        _max_output_bytes: max_output_bytes,
        _shell: req.shell,
    })
}

fn acquire(counter: &AtomicUsize, limit: usize, scope: &str) -> io::Result<()> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            active.checked_add(1).filter(|next| *next <= limit)
        })
        .map(|_| ())
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("remote execution {scope} concurrency limit reached"),
            )
        })
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ExecRequest {
        ExecRequest {
            argv: vec!["program".into(), "--version".into()],
            cwd: None,
            timeout_ms: 30_000,
            max_output_bytes: 1024,
            shell: false,
        }
    }

    fn allowed() -> ExecGrant {
        ExecGrant {
            enabled: true,
            ..ExecGrant::default()
        }
    }

    #[test]
    fn exec_is_denied_by_default() {
        let error = prepare(req(), &ExecGrant::default(), Arc::default()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn shell_uses_the_same_permission_but_runtime_fails_closed() {
        let mut request = req();
        request.shell = true;
        request.argv = vec!["echo hi".into()];
        let error = prepare(request, &allowed(), Arc::default())
            .unwrap()
            .run()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn request_limits_are_clamped_before_execution() {
        let mut request = req();
        request.timeout_ms = u64::MAX;
        request.max_output_bytes = u64::MAX;
        let validated = validate(request).unwrap();
        assert_eq!(validated._timeout, Duration::from_millis(MAX_TIMEOUT_MS));
        assert_eq!(validated._max_output_bytes, MAX_OUTPUT_BYTES as usize);
    }

    #[test]
    fn oversized_strings_and_ambiguous_shell_argv_are_rejected() {
        let mut request = req();
        request.cwd = Some("x".repeat(MAX_CWD_BYTES + 1));
        assert_eq!(
            validate(request).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let mut request = req();
        request.argv[0] = "x".repeat(MAX_SINGLE_ARG_BYTES + 1);
        assert_eq!(
            validate(request).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let mut request = req();
        request.shell = true;
        assert_eq!(
            validate(request).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn per_peer_slot_is_reserved_and_released() {
        let peer = Arc::new(AtomicUsize::new(0));
        let first = prepare(req(), &allowed(), peer.clone()).unwrap();
        let error = prepare(req(), &allowed(), peer.clone()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        drop(first);
        assert!(prepare(req(), &allowed(), peer).is_ok());
    }
}
