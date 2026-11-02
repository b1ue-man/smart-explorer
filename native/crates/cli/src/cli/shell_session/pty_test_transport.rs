//! Test transport of terminal sessions: a local shell as the session leader
//! of a pseudo-terminal, as an SSH server or a Share host gives it to a login
//! shell. The master side is the transport; resizing sets the window size,
//! cancelling kills the shell's process group.
use super::transports::{Transport, TransportEnd, TransportEvent, TransportInput};
use super::types::TerminalSize;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub(super) struct PtyTransport {
    events: mpsc::Receiver<TransportEvent>,
    input: Arc<PtyInput>,
}

struct PtyInput {
    master: Mutex<File>,
    pid: i32,
    cancelled: AtomicBool,
}

impl TransportInput for PtyInput {
    fn stdin(&self, bytes: &[u8]) -> Result<(), String> {
        let mut master = self.master.lock().map_err(|_| "master lock".to_string())?;
        master
            .write_all(bytes)
            .and_then(|()| master.flush())
            .map_err(|error| error.to_string())
    }

    /// The end of the user's input in a terminal is a ^D byte, not a closed
    /// stream: nothing to do.
    fn eof(&self) -> Result<(), String> {
        Ok(())
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        // SAFETY: kill(2) with the shell's pid and its process group.
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
            libc::kill(self.pid, libc::SIGKILL);
        }
    }

    fn resize(&self, size: TerminalSize) -> Result<(), String> {
        let master = self.master.lock().map_err(|_| "master lock".to_string())?;
        set_window(master.as_raw_fd(), size)
    }
}

impl PtyTransport {
    /// Runs `program` with `args` on a new pseudo-terminal of `size`; the
    /// program is the session leader and the terminal its controlling one.
    pub(super) fn spawn(program: &str, args: &[&str], size: TerminalSize) -> Result<Self, String> {
        let (master, slave) = open_pty(size)?;
        let mut command = Command::new(program);
        command
            .args(args)
            .env("TERM", "xterm")
            .env_remove("ENV")
            .stdin(Stdio::from(clone_file(&slave)?))
            .stdout(Stdio::from(clone_file(&slave)?))
            .stderr(Stdio::from(slave));
        // SAFETY: only setsid(2) and ioctl(2) run between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("start the test shell: {error}"))?;
        // The parent's copies of the slave must go, or the master never ends.
        drop(command);
        let pid = i32::try_from(child.id()).map_err(|error| error.to_string())?;
        let mut reading = clone_file(&master)?;
        let input = Arc::new(PtyInput {
            master: Mutex::new(master),
            pid,
            cancelled: AtomicBool::new(false),
        });
        let (sender, events) = mpsc::channel();
        let waiting = input.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reading.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        let bytes = buffer[..read].to_vec();
                        if sender.send(TransportEvent::Stdout(bytes)).is_err() {
                            return;
                        }
                    }
                }
            }
            let status = child.wait().ok();
            let cancelled = waiting.cancelled.load(Ordering::SeqCst);
            let exit_code = status.and_then(|status| {
                status
                    .code()
                    .or_else(|| status.signal().map(|signal| 128 + signal))
            });
            let _ = sender.send(TransportEvent::Ended(TransportEnd {
                exit_code: if cancelled { None } else { exit_code },
                cancelled,
                error: None,
            }));
        });
        Ok(Self { events, input })
    }
}

impl Transport for PtyTransport {
    fn next_event(&mut self) -> TransportEvent {
        self.events.recv().unwrap_or_else(|_| {
            TransportEvent::Ended(TransportEnd {
                error: Some("the test terminal is gone".into()),
                ..TransportEnd::default()
            })
        })
    }

    fn input(&self) -> Arc<dyn TransportInput> {
        self.input.clone()
    }
}

fn open_pty(size: TerminalSize) -> Result<(File, File), String> {
    // SAFETY: plain libc calls on descriptors this function owns; every raw
    // descriptor is wrapped at once, so no error path leaks one.
    unsafe {
        let raw = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        if raw < 0 {
            return Err(os_error("posix_openpt"));
        }
        let master = OwnedFd::from_raw_fd(raw);
        if libc::grantpt(master.as_raw_fd()) != 0 || libc::unlockpt(master.as_raw_fd()) != 0 {
            return Err(os_error("grantpt or unlockpt"));
        }
        let mut name = [0 as libc::c_char; 128];
        if libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()) != 0 {
            return Err(os_error("ptsname_r"));
        }
        let raw = libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
        if raw < 0 {
            return Err(os_error("open the pseudo-terminal slave"));
        }
        let slave = OwnedFd::from_raw_fd(raw);
        set_window(master.as_raw_fd(), size)?;
        Ok((File::from(master), File::from(slave)))
    }
}

fn set_window(fd: RawFd, size: TerminalSize) -> Result<(), String> {
    let window = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ reads one `winsize` through the pointer.
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &window) } < 0 {
        return Err(os_error("TIOCSWINSZ"));
    }
    Ok(())
}

fn clone_file(file: &File) -> Result<File, String> {
    file.try_clone().map_err(|error| error.to_string())
}

fn os_error(what: &str) -> String {
    format!("{what}: {}", io::Error::last_os_error())
}
