//! A command started as a child of this process with piped standard streams,
//! in its own process group (Linux) or job object (Windows,
//! `platform::Tree`). Both the direct start of an already elevated process
//! and the elevated helper use it. When the first process ends, its output
//! gets a short grace, then the rest of the tree is ended, so a background
//! process cannot hold the command open (the Share host ends the tree the
//! same way, `share/core/exec_job.rs`).
use std::io::{self, Read, Write};
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use super::child::{ElevatedChild, InputPort};
use super::platform::{self, Tree};
use super::types::{ElevateError, ElevatedEvent, ElevatedExit, ElevatedSpec, Elevation};

/// Output that still arrives after the first process ended.
const OUTPUT_GRACE: Duration = Duration::from_millis(250);
/// Wait for the pipes after the rest of the tree was ended.
const TREE_STOP_WAIT: Duration = Duration::from_secs(2);

struct DirectInput {
    stdin: Mutex<Option<ChildStdin>>,
    tree: Arc<Tree>,
    cancelled: Arc<AtomicBool>,
}

impl InputPort for DirectInput {
    fn stdin(&self, bytes: &[u8]) -> io::Result<()> {
        let mut slot = self
            .stdin
            .lock()
            .map_err(|_| io::Error::other("stdin lock poisoned"))?;
        let stdin = slot
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdin is closed"))?;
        stdin.write_all(bytes)?;
        stdin.flush()
    }

    fn eof(&self) -> io::Result<()> {
        if let Ok(mut slot) = self.stdin.lock() {
            slot.take();
        }
        Ok(())
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.stdin.lock() {
            slot.take();
        }
        self.tree.terminate();
    }
}

pub(super) fn spawn(spec: &ElevatedSpec, via: Elevation) -> Result<ElevatedChild, ElevateError> {
    let mut command = platform::direct_command(&spec.command)
        .map_err(|error| ElevateError::Failed(error.to_string()))?;
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    command
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        ElevateError::Failed(format!("cannot start {}: {error}", spec.display()))
    })?;
    let tree = match Tree::adopt(&child) {
        Ok(tree) => Arc::new(tree),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ElevateError::Failed(format!(
                "cannot contain the started command: {error}"
            )));
        }
    };
    let (sender, events) = mpsc::channel();
    let open = Arc::new(AtomicUsize::new(0));
    let pumps = [
        child
            .stdout
            .take()
            .map(|out| pump(out, sender.clone(), ElevatedEvent::Stdout, &open)),
        child
            .stderr
            .take()
            .map(|err| pump(err, sender.clone(), ElevatedEvent::Stderr, &open)),
    ];
    let cancelled = Arc::new(AtomicBool::new(false));
    let input = Arc::new(DirectInput {
        stdin: Mutex::new(child.stdin.take()),
        tree: tree.clone(),
        cancelled: cancelled.clone(),
    });
    let waiter = std::thread::Builder::new()
        .name("se-elevate-wait".into())
        .spawn(move || {
            let status = child.wait();
            settle(&open, OUTPUT_GRACE);
            // The first process ended: the rest of its tree goes too.
            tree.kill();
            settle(&open, TREE_STOP_WAIT);
            for pump in pumps.into_iter().flatten() {
                if open.load(Ordering::SeqCst) == 0 {
                    let _ = pump.join();
                }
            }
            let exit = match status {
                Ok(status) => ElevatedExit {
                    code: status.code(),
                    signal: platform::exit_signal(&status),
                    cancelled: cancelled.load(Ordering::SeqCst),
                    error: None,
                },
                Err(error) => ElevatedExit {
                    cancelled: cancelled.load(Ordering::SeqCst),
                    error: Some(format!("cannot wait for the command: {error}")),
                    ..ElevatedExit::default()
                },
            };
            let _ = sender.send(ElevatedEvent::Exited(exit));
        });
    if let Err(error) = waiter {
        input.cancel();
        return Err(ElevateError::Failed(format!(
            "cannot start the waiting thread: {error}"
        )));
    }
    Ok(ElevatedChild::new(events, input, via))
}

/// Waits until every pump ended or `limit` passed.
fn settle(open: &AtomicUsize, limit: Duration) {
    let end = Instant::now() + limit;
    while open.load(Ordering::SeqCst) > 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn pump(
    mut reader: impl Read + Send + 'static,
    sender: mpsc::Sender<ElevatedEvent>,
    event: fn(Vec<u8>) -> ElevatedEvent,
    open: &Arc<AtomicUsize>,
) -> std::thread::JoinHandle<()> {
    open.fetch_add(1, Ordering::SeqCst);
    let open = open.clone();
    std::thread::spawn(move || {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if sender.send(event(buffer[..read].to_vec())).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        open.fetch_sub(1, Ordering::SeqCst);
    })
}
