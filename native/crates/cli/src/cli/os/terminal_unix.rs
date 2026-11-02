//! The local terminal of a `se shell` terminal session on Unix: raw mode with
//! the saved settings restored on every way out, the typed bytes as they come,
//! the window size and its changes (SIGWINCH), and the requests to end the
//! process (SIGTERM, SIGHUP, SIGQUIT, SIGINT). In raw mode the keyboard makes
//! no SIGINT, Ctrl+C is the byte 3 for the remote program; those signals can
//! only come from outside.
//!
//! The signal handler does nothing but `write` one byte (the signal number)
//! into a non-blocking self-pipe, which is async-signal-safe; a reader thread
//! turns the bytes into events. The handlers replace the process's earlier ones
//! (the Ctrl+C handler of the line mode) and the earlier ones come back when
//! the terminal is released.
use super::LocalEvent;
use crossbeam_channel::Sender;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

/// Size of one read of the typed bytes.
const INPUT_BUFFER: usize = 64 * 1024;
/// How long a reader waits before it looks at the stop flag.
const STOP_CHECK_MS: libc::c_int = 200;
/// How long a blocked write to a non-blocking terminal waits per attempt.
const WRITE_WAIT_MS: libc::c_int = 100;
/// The signals of a session: resize first, then the requests to end.
const SIGNALS: [libc::c_int; 5] = [
    libc::SIGWINCH,
    libc::SIGTERM,
    libc::SIGHUP,
    libc::SIGQUIT,
    libc::SIGINT,
];

/// Write end of the signal self-pipe while handlers are installed, else -1.
static SIGNAL_PIPE: AtomicI32 = AtomicI32::new(-1);

/// A terminal in raw mode that reports what happens to it. Dropping it
/// restores the terminal settings and the earlier signal handlers.
pub(in crate::cli) struct LocalTerminal {
    saved: Option<libc::termios>,
    handlers: Vec<(libc::c_int, libc::sigaction)>,
    /// Keeps the write end open as long as the handlers can use it.
    _pipe_write: OwnedFd,
    stop: Arc<AtomicBool>,
}

impl LocalTerminal {
    /// Installs the signal handlers, switches the terminal on standard input to
    /// raw mode and starts the threads that send `events`.
    pub(in crate::cli) fn enter(events: Sender<LocalEvent>) -> Result<Self, String> {
        let (pipe_read, pipe_write) = signal_pipe()?;
        SIGNAL_PIPE.store(pipe_write.as_raw_fd(), Ordering::SeqCst);
        let mut terminal = Self {
            saved: None,
            handlers: Vec::new(),
            _pipe_write: pipe_write,
            stop: Arc::new(AtomicBool::new(false)),
        };
        terminal.install_handlers()?;
        terminal.saved = Some(enter_raw()?);
        let stop = terminal.stop.clone();
        let sender = events.clone();
        std::thread::Builder::new()
            .name("se-term-signals".into())
            .spawn(move || read_signals(pipe_read, sender, stop))
            .map_err(|error| format!("signal reader: {error}"))?;
        let stop = terminal.stop.clone();
        std::thread::Builder::new()
            .name("se-term-input".into())
            .spawn(move || read_input(events, stop))
            .map_err(|error| format!("input reader: {error}"))?;
        Ok(terminal)
    }

    fn install_handlers(&mut self) -> Result<(), String> {
        for signal in SIGNALS {
            // SAFETY: an all-zero sigaction is a valid value for this C struct
            // (no flags, empty mask, no handler); the used fields are set below.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            // SAFETY: as above; `sigaction` fills it in.
            let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            // SAFETY: `sa_mask` is a writable signal set.
            unsafe { libc::sigemptyset(&mut action.sa_mask) };
            action.sa_flags = libc::SA_RESTART;
            // SAFETY: both pointers refer to live sigaction values; the handler
            // only calls `write`, which is async-signal-safe.
            if unsafe { libc::sigaction(signal, &action, &mut previous) } != 0 {
                return Err(format!("signal handler: {}", io::Error::last_os_error()));
            }
            self.handlers.push((signal, previous));
        }
        Ok(())
    }
}

impl Drop for LocalTerminal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The terminal first: a signal that arrives while this runs is still
        // caught and does not kill the process with the terminal in raw mode.
        if let Some(saved) = self.saved.take() {
            restore(&saved);
        }
        for (signal, previous) in self.handlers.drain(..) {
            // SAFETY: puts back the action `sigaction` returned for this signal.
            unsafe { libc::sigaction(signal, &previous, std::ptr::null_mut()) };
        }
        SIGNAL_PIPE.store(-1, Ordering::SeqCst);
    }
}

/// Whether the terminal on standard input accepts raw mode; it is restored
/// at once.
pub(super) fn check_raw_mode() -> Result<(), String> {
    let saved = enter_raw()?;
    restore(&saved);
    Ok(())
}

/// Columns and rows of the terminal on standard output (else standard input).
pub(super) fn window_size() -> Option<(u16, u16)> {
    [libc::STDOUT_FILENO, libc::STDIN_FILENO]
        .into_iter()
        .find_map(|fd| {
            // SAFETY: an all-zero winsize is valid; TIOCGWINSZ fills it in.
            let mut size: libc::winsize = unsafe { std::mem::zeroed() };
            // SAFETY: `size` is a writable winsize that outlives the call.
            let ok = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) } == 0;
            (ok && size.ws_col > 0 && size.ws_row > 0).then_some((size.ws_col, size.ws_row))
        })
}

/// Writes the remote terminal's bytes to the local terminal unchanged.
#[derive(Default)]
pub(in crate::cli) struct TerminalOutput;

impl TerminalOutput {
    /// Writes all of `bytes` to standard output, retrying after interruptions
    /// and while a non-blocking terminal is full.
    pub(in crate::cli) fn write(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            // SAFETY: the pointer and length describe the live slice `bytes`.
            let written =
                unsafe { libc::write(libc::STDOUT_FILENO, bytes.as_ptr().cast(), bytes.len()) };
            if written < 0 {
                let error = io::Error::last_os_error();
                match error.kind() {
                    io::ErrorKind::Interrupted => {}
                    io::ErrorKind::WouldBlock => wait_writable(),
                    _ => return Err(error),
                }
                continue;
            }
            let count = usize::try_from(written).unwrap_or_default();
            if count == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }
}

fn wait_writable() {
    let mut wait = libc::pollfd {
        fd: libc::STDOUT_FILENO,
        events: libc::POLLOUT,
        revents: 0,
    };
    // SAFETY: `wait` is a live pollfd and the count is 1.
    unsafe { libc::poll(&mut wait, 1, WRITE_WAIT_MS) };
}

/// Raw mode on standard input; returns the settings to restore.
fn enter_raw() -> Result<libc::termios, String> {
    let mut saved = MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `saved` is a writable termios buffer that outlives the call.
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, saved.as_mut_ptr()) } != 0 {
        return Err(format!("terminal settings: {}", io::Error::last_os_error()));
    }
    // SAFETY: tcgetattr returned 0, so it initialized `saved`.
    let saved = unsafe { saved.assume_init() };
    let mut raw = saved;
    // SAFETY: `raw` is a live termios value.
    unsafe { libc::cfmakeraw(&mut raw) };
    // Reads return as soon as one byte is there (glibc's cfmakeraw leaves both).
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    // SAFETY: `raw` is the saved setting with only the raw-mode changes.
    if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &raw) } != 0 {
        return Err(format!("raw mode: {}", io::Error::last_os_error()));
    }
    Ok(saved)
}

fn restore(saved: &libc::termios) {
    // SAFETY: puts back the settings read from the same descriptor. A terminal
    // that is gone makes this fail, and then there is nothing left to restore.
    unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, saved) };
}

/// Non-blocking, close-on-exec self-pipe: (read end, write end).
fn signal_pipe() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors `pipe2` writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return Err(format!("signal pipe: {}", io::Error::last_os_error()));
    }
    // SAFETY: `pipe2` returned two new descriptors that nothing else owns.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// The handler: one byte with the signal number into the self-pipe. Only
/// `write` and the thread's `errno` are touched.
extern "C" fn on_signal(signal: libc::c_int) {
    let fd = SIGNAL_PIPE.load(Ordering::SeqCst);
    if fd < 0 {
        return;
    }
    let byte = signal as u8;
    let errno = errno_slot();
    // SAFETY: the errno slot is the calling thread's own.
    let saved = errno.map(|slot| unsafe { *slot });
    // SAFETY: `byte` lives on the stack; a full pipe or an error loses the
    // signal, which is acceptable for a repeated request.
    unsafe { libc::write(fd, std::ptr::addr_of!(byte).cast(), 1) };
    if let (Some(slot), Some(saved)) = (errno, saved) {
        // SAFETY: puts back the value read above into the same slot.
        unsafe { *slot = saved };
    }
}

#[cfg(target_os = "linux")]
fn errno_slot() -> Option<*mut libc::c_int> {
    // SAFETY: returns the address of the calling thread's errno.
    Some(unsafe { libc::__errno_location() })
}

/// Other Unix systems: errno stays as the handler leaves it.
#[cfg(not(target_os = "linux"))]
fn errno_slot() -> Option<*mut libc::c_int> {
    None
}

/// Turns the signal bytes into events until the terminal is released.
fn read_signals(pipe: OwnedFd, events: Sender<LocalEvent>, stop: Arc<AtomicBool>) {
    let fd = pipe.as_raw_fd();
    let mut buffer = [0u8; 64];
    while !stop.load(Ordering::SeqCst) {
        match wait_readable(fd) {
            Wait::Timeout => continue,
            Wait::Failed => return,
            Wait::Ready => {}
        }
        // SAFETY: the pointer and length describe the live array `buffer`.
        let count = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if count == 0 {
            return;
        }
        let Ok(count) = usize::try_from(count) else {
            match io::Error::last_os_error().kind() {
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock => continue,
                _ => return,
            }
        };
        for &number in &buffer[..count] {
            let event = if libc::c_int::from(number) == libc::SIGWINCH {
                match window_size() {
                    Some((cols, rows)) => LocalEvent::Resized(cols, rows),
                    None => continue,
                }
            } else {
                LocalEvent::Terminate(128 + i32::from(number))
            };
            if events.send(event).is_err() {
                return;
            }
        }
    }
}

/// Sends the typed bytes as they come until the terminal is released; the end
/// of the input is the loss of the terminal (Ctrl+D is only a byte in raw mode).
fn read_input(events: Sender<LocalEvent>, stop: Arc<AtomicBool>) {
    let mut buffer = vec![0u8; INPUT_BUFFER];
    while !stop.load(Ordering::SeqCst) {
        match wait_readable(libc::STDIN_FILENO) {
            Wait::Timeout => continue,
            Wait::Failed => {
                let _ = events.send(LocalEvent::InputClosed);
                return;
            }
            Wait::Ready => {}
        }
        // SAFETY: the pointer and length describe the live buffer.
        let count =
            unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
        let count = match usize::try_from(count) {
            Ok(0) => {
                let _ = events.send(LocalEvent::InputClosed);
                return;
            }
            Ok(count) => count,
            Err(_) => match io::Error::last_os_error().kind() {
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock => continue,
                _ => {
                    let _ = events.send(LocalEvent::InputClosed);
                    return;
                }
            },
        };
        if events
            .send(LocalEvent::Input(buffer[..count].to_vec()))
            .is_err()
        {
            return;
        }
    }
}

enum Wait {
    Ready,
    Timeout,
    Failed,
}

/// Waits up to `STOP_CHECK_MS` for `fd` to become readable (or to report a
/// hang-up or error, which the next read turns into an end).
fn wait_readable(fd: libc::c_int) -> Wait {
    let mut wait = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `wait` is a live pollfd and the count is 1.
    let ready = unsafe { libc::poll(&mut wait, 1, STOP_CHECK_MS) };
    match ready {
        0 => Wait::Timeout,
        n if n > 0 => Wait::Ready,
        _ if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => Wait::Timeout,
        _ => Wait::Failed,
    }
}
