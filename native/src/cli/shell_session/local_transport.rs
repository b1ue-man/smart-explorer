//! Test transport: the session's shell as a local child process (`sh -c` with
//! the POSIX start line, or `powershell.exe -Command -`) with piped streams.
//! Cancel kills the shell; its own children may keep the pipes open until
//! they end.
use super::transports::{Transport, TransportEnd, TransportEvent, TransportInput};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) struct LocalTransport {
    events: mpsc::Receiver<TransportEvent>,
    input: Arc<LocalInput>,
}

struct LocalInput {
    stdin: Mutex<Option<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    cancelled: AtomicBool,
}

impl TransportInput for LocalInput {
    fn stdin(&self, bytes: &[u8]) -> Result<(), String> {
        let mut slot = self.stdin.lock().map_err(|_| "stdin lock".to_string())?;
        let stdin = slot.as_mut().ok_or_else(|| "stdin is closed".to_string())?;
        stdin
            .write_all(bytes)
            .and_then(|()| stdin.flush())
            .map_err(|error| error.to_string())
    }

    fn eof(&self) -> Result<(), String> {
        if let Ok(mut slot) = self.stdin.lock() {
            slot.take();
        }
        Ok(())
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

impl LocalTransport {
    pub(super) fn spawn(program: &str, args: &[&str]) -> Self {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the local test shell");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let child = Arc::new(Mutex::new(child));
        let (sender, events) = mpsc::channel();
        let out = pump(stdout, sender.clone(), TransportEvent::Stdout);
        let err = pump(stderr, sender.clone(), TransportEvent::Stderr);
        let input = Arc::new(LocalInput {
            stdin: Mutex::new(stdin),
            child: child.clone(),
            cancelled: AtomicBool::new(false),
        });
        let waiting = input.clone();
        std::thread::spawn(move || {
            let _ = out.join();
            let _ = err.join();
            let status = loop {
                let polled = child.lock().map(|mut child| child.try_wait());
                match polled {
                    Ok(Ok(Some(status))) => break Some(status),
                    Ok(Ok(None)) => std::thread::sleep(Duration::from_millis(20)),
                    _ => break None,
                }
            };
            let cancelled = waiting.cancelled.load(Ordering::SeqCst);
            let _ = sender.send(TransportEvent::Ended(TransportEnd {
                exit_code: if cancelled {
                    None
                } else {
                    status.and_then(|status| status.code())
                },
                cancelled,
                error: None,
            }));
        });
        Self { events, input }
    }
}

fn pump(
    mut reader: impl Read + Send + 'static,
    sender: mpsc::Sender<TransportEvent>,
    event: fn(Vec<u8>) -> TransportEvent,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    if sender.send(event(buffer[..read].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
    })
}

impl Transport for LocalTransport {
    fn next_event(&mut self) -> TransportEvent {
        self.events.recv().unwrap_or_else(|_| {
            TransportEvent::Ended(TransportEnd {
                error: Some("the local test shell is gone".into()),
                ..TransportEnd::default()
            })
        })
    }

    fn input(&self) -> Arc<dyn TransportInput> {
        self.input.clone()
    }
}
