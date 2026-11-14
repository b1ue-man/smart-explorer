//! The registered launcher keeps the agent host's stdio connection for the
//! whole agent session. It starts the installation's confirmed launcher as
//! its child (that child admits and supervises one runtime, `launch`) and
//! relays the newline-delimited MCP messages. When the runtime ends while the
//! host stays connected (a Smart Explorer update stops it, or it exits), the
//! relay answers the requests that runtime left open with an error, waits
//! until the installation is not being updated, starts the then confirmed
//! launcher, replays the host's `initialize` handshake and announces a changed
//! tool list to a host that was told it may change. A request the host
//! cancels while it waits for a runtime reaches none. The end of the runtime
//! is the end of its output or, when a process it started still holds that
//! pipe, of its process. Interactive Claude Code does not restart a stdio server that
//! exited (docs/AI_AGENT_PLUGIN.md, "Updates"); without this relay every
//! update would leave the agent without its Smart Explorer tools.
use crate::mcp_launcher::core::relay::{tools_list_changed, RelayBook, Route};
use crate::mcp_launcher::{os, GenerationChild, Installation};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Set on the relay's child launcher: it starts its runtime directly and
/// sets it for that runtime, which words its update message for a relayed
/// session. Each of the two reads it once and removes it (`relayed`).
pub const RELAY_CHILD_ENV: &str = "SE_MCP_RELAY_CHILD";
/// A runtime that ends sooner without having answered anything failed to start.
const QUICK_EXIT: Duration = Duration::from_secs(10);
/// Quick failures in a row after which the relay ends the session.
const MAX_QUICK_FAILURES: u32 = 5;
/// The longest wait for an installation that is being updated or recovered.
const UPDATE_WAIT: Duration = Duration::from_secs(15 * 60);
const POLL: Duration = Duration::from_millis(500);
/// How long the output of a runtime whose process has ended is still read.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);
const ENDED: &str = "Smart Explorer replaced its MCP runtime (update or runtime exit) before this request was answered; its outcome is unknown. Check the current state before repeating a change.";
const GAVE_UP: &str = "Smart Explorer could not start its MCP runtime again; the server's standard error names the reason.";

enum Event {
    Host(Option<Vec<u8>>),
    Runtime(u64, Option<Vec<u8>>),
}

struct Running {
    generation: u64,
    child: GenerationChild,
    input: Option<Box<dyn Write + Send>>,
    started: Instant,
    answered: bool,
}

struct Relay {
    book: RelayBook,
    queue: VecDeque<Vec<u8>>,
    host_open: bool,
    events: Sender<Event>,
    inbox: Receiver<Event>,
    generation: u64,
}

pub fn run(root: &Path) -> Result<i32, String> {
    let (events, inbox) = channel();
    let host = events.clone();
    std::thread::Builder::new()
        .name("se-relay-host".into())
        .spawn(move || pump(std::io::stdin().lock(), |line| host.send(Event::Host(line))))
        .map_err(|e| e.to_string())?;
    let mut relay = Relay {
        book: RelayBook::default(),
        queue: VecDeque::new(),
        host_open: true,
        events,
        inbox,
        generation: 0,
    };
    let mut failures = 0u32;
    loop {
        let Some(mut running) = relay.start(root, failures)? else {
            return Ok(0);
        };
        relay.serve(&mut running);
        let code = running
            .child
            .wait()
            .map(|status| status.code().unwrap_or(1))
            .map_err(|e| format!("relay_child_wait_failed: {e}"))?;
        if !relay.host_open {
            return Ok(code);
        }
        for answer in relay.book.runtime_ended(ENDED) {
            relay.to_host(&answer);
        }
        let quick = !running.answered && running.started.elapsed() < QUICK_EXIT;
        failures = if quick { failures + 1 } else { 0 };
        // Before the host has a session, a failed start ends the connection
        // at once, as a directly started runtime would.
        if quick && (!relay.book.session_started() || failures >= MAX_QUICK_FAILURES) {
            // The lines the host sent while the runtime ended are refused too.
            let _ = relay.wait(Duration::ZERO);
            relay.refuse_queue(GAVE_UP);
            return Ok(if code == 0 { 1 } else { code });
        }
        eprintln!("se-mcp-launcher: runtime ended (exit {code}); starting the confirmed generation on this connection");
    }
}

/// Reads newline-terminated lines until end of input; `None` marks the end.
fn pump<R: Read, E>(input: R, mut send: impl FnMut(Option<Vec<u8>>) -> Result<(), E>) {
    let mut reader = BufReader::new(input);
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => {
                let _ = send(None);
                return;
            }
            Ok(_) => {
                if send(Some(line)).is_err() {
                    return;
                }
            }
        }
    }
}

impl Relay {
    /// Starts the installation's confirmed launcher once the installation is
    /// not being updated. `None`: the host ended the session meanwhile.
    fn start(&mut self, root: &Path, failures: u32) -> Result<Option<Running>, String> {
        let backoff = match failures {
            0 => Duration::ZERO,
            n => Duration::from_secs(1 << (n - 1).min(4)),
        };
        if !self.wait(backoff) {
            return Ok(None);
        }
        let deadline = Instant::now() + UPDATE_WAIT;
        let mut child = loop {
            match spawn_confirmed(root) {
                Ok(child) => break child,
                Err(error) if waiting_for_update(&error) && Instant::now() < deadline => {
                    if !self.wait(POLL) {
                        return Ok(None);
                    }
                }
                Err(error) => {
                    self.refuse_queue(GAVE_UP);
                    return Err(error);
                }
            }
        };
        self.generation += 1;
        let generation = self.generation;
        if let Some(output) = child.stdout.take() {
            let events = self.events.clone();
            std::thread::Builder::new()
                .name("se-relay-runtime".into())
                .spawn(move || pump(output, |line| events.send(Event::Runtime(generation, line))))
                .map_err(|e| e.to_string())?;
        }
        if let Some(mut errors) = child.stderr.take() {
            std::thread::Builder::new()
                .name("se-relay-errors".into())
                .spawn(move || {
                    let _ = std::io::copy(&mut errors, &mut std::io::stderr());
                })
                .map_err(|e| e.to_string())?;
        }
        let mut running = Running {
            generation,
            input: child.stdin.take(),
            child,
            started: Instant::now(),
            answered: false,
        };
        if generation > 1 {
            for line in self.book.replay(generation) {
                write_line(&mut running.input, &line);
            }
            if self.book.announces_tool_changes() {
                self.to_host(&tools_list_changed());
            }
        }
        while let Some(line) = self.queue.pop_front() {
            self.deliver(&mut running, line);
        }
        if !self.host_open {
            running.input = None;
        }
        Ok(Some(running))
    }

    /// Relays until the runtime's output ends. A process the runtime started
    /// may keep that pipe open after the runtime's own process has ended:
    /// then the lines already written are read for `OUTPUT_GRACE`, and the
    /// runtime counts as ended, so its open requests are answered.
    fn serve(&mut self, running: &mut Running) {
        let mut checked = Instant::now();
        let mut exited: Option<Instant> = None;
        loop {
            if exited.is_none() && checked.elapsed() >= POLL {
                checked = Instant::now();
                if matches!(running.child.try_wait(), Ok(Some(_))) {
                    exited = Some(Instant::now());
                }
            }
            if exited.is_some_and(|since| since.elapsed() >= OUTPUT_GRACE) {
                return;
            }
            let event = match self.inbox.recv_timeout(POLL) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            match event {
                Event::Host(Some(line)) => self.deliver(running, line),
                Event::Host(None) => {
                    self.host_open = false;
                    running.input = None;
                }
                Event::Runtime(generation, Some(line)) if generation == running.generation => {
                    running.answered = true;
                    if self.book.runtime_to_host(&line) == Route::Forward {
                        self.to_host(&line);
                    }
                    if !self.host_open {
                        running.input = None;
                    }
                }
                Event::Runtime(generation, None) if generation == running.generation => return,
                Event::Runtime(..) => {}
            }
        }
    }

    fn deliver(&mut self, running: &mut Running, line: Vec<u8>) {
        if self.book.host_to_runtime(&line) == Route::Forward {
            write_line(&mut running.input, &line);
        }
    }

    /// Waits `duration` while no runtime runs: host lines queue up, `ping`
    /// is answered here. `false`: the host ended the session.
    fn wait(&mut self, duration: Duration) -> bool {
        let until = Instant::now() + duration;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let event = if left.is_zero() {
                match self.inbox.try_recv() {
                    Ok(event) => event,
                    Err(_) => return self.host_open,
                }
            } else {
                match self.inbox.recv_timeout(left) {
                    Ok(event) => event,
                    Err(RecvTimeoutError::Timeout) => return self.host_open,
                    Err(RecvTimeoutError::Disconnected) => return false,
                }
            };
            match event {
                Event::Host(Some(line)) => match RelayBook::local_ping_answer(&line) {
                    Some(answer) => self.to_host(&answer),
                    None => self.enqueue(line),
                },
                Event::Host(None) => {
                    self.host_open = false;
                    return false;
                }
                Event::Runtime(..) => {}
            }
        }
    }

    /// Keeps a host line for the next runtime. A cancellation of a request
    /// that still waits here withdraws it: neither line reaches a runtime,
    /// and the host expects no answer to a request it cancelled.
    fn enqueue(&mut self, line: Vec<u8>) {
        if let Some(cancelled) = RelayBook::cancelled_request(&line) {
            let waiting = self
                .queue
                .iter()
                .position(|queued| RelayBook::is_request_with_key(queued, &cancelled));
            if let Some(index) = waiting {
                self.queue.remove(index);
                return;
            }
        }
        self.queue.push_back(line);
    }

    fn refuse_queue(&mut self, reason: &str) {
        while let Some(line) = self.queue.pop_front() {
            for answer in RelayBook::refusal(&line, reason) {
                self.to_host(&answer);
            }
        }
    }

    fn to_host(&mut self, line: &[u8]) {
        let mut output = std::io::stdout().lock();
        if output
            .write_all(line)
            .and_then(|()| output.flush())
            .is_err()
        {
            self.host_open = false;
        }
    }
}

fn write_line(input: &mut Option<Box<dyn Write + Send>>, line: &[u8]) {
    let failed = input
        .as_mut()
        .is_some_and(|pipe| pipe.write_all(line).and_then(|()| pipe.flush()).is_err());
    if failed {
        // The runtime is ending; its open requests are answered when it has.
        *input = None;
    }
}

fn waiting_for_update(error: &str) -> bool {
    [
        "update_in_progress",
        "update_recovery_required",
        "lock_busy:",
    ]
    .iter()
    .any(|state| error.contains(state))
}

/// The confirmed launcher of the installation, admitted by the same checks a
/// launcher applies before it starts a runtime.
fn spawn_confirmed(root: &Path) -> Result<GenerationChild, String> {
    let installation = Installation::load(root)?;
    {
        let _control = installation.lock("control.lock", false)?;
        crate::mcp_launcher::admission::check_state(&installation)?;
    }
    let path = installation.launcher_path();
    let image = os::open_verified(
        &installation.root,
        &path,
        &installation.manifest.launcher_sha256,
    )?;
    let mut command = Command::new(&path);
    command.arg("mcp").env(RELAY_CHILD_ENV, "1");
    os::spawn_relayed(command, image)
}

/// Whether this process is the relay's child launcher or the runtime that
/// one started. The mark is read once and then removed from the environment:
/// a process this one starts (the background worker, a job, an agent host
/// started inside one of them) must not take itself for a relay's child, or
/// its registered launcher would start a runtime itself and end with it.
pub fn relayed() -> bool {
    static RELAYED: OnceLock<bool> = OnceLock::new();
    *RELAYED.get_or_init(|| {
        let marked =
            std::env::var_os(RELAY_CHILD_ENV).as_deref() == Some(std::ffi::OsStr::new("1"));
        std::env::remove_var(RELAY_CHILD_ENV);
        marked
    })
}

#[cfg(test)]
mod plugin_task_relay_process_tests {
    use super::{pump, waiting_for_update};

    #[test]
    fn plugin_task_relay_waits_only_while_the_installation_is_updated() {
        assert!(waiting_for_update("update_in_progress"));
        assert!(waiting_for_update("update_recovery_required"));
        assert!(waiting_for_update("lock_busy: control.lock"));
        assert!(!waiting_for_update("runtime_sha256_mismatch"));
        assert!(!waiting_for_update("installation_uninstall_in_progress"));
    }

    #[test]
    fn plugin_task_relay_reads_whole_lines_and_marks_the_end() {
        let mut lines = Vec::new();
        pump(&b"{\"a\":1}\n{\"b\":2}\nlast"[..], |line| {
            lines.push(line);
            Ok::<(), ()>(())
        });
        assert_eq!(
            lines,
            vec![
                Some(b"{\"a\":1}\n".to_vec()),
                Some(b"{\"b\":2}\n".to_vec()),
                Some(b"last".to_vec()),
                None,
            ]
        );
    }
}
