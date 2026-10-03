//! The watch thread of Linux and Android (RV1, V4): it owns the one inotify
//! instance of the process (`inotify_tree`), takes commands through a channel
//! woken by an `eventfd`, and sleeps in `poll` until events, commands or the
//! next re-arm time arrive.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};

use super::inotify_tree::Tree;
use super::service::{deliver_owed, overflow_owed, RootSpec};
use super::types::WatchId;

/// While an `Overflow` is owed to a full channel, it is offered again this
/// often.
const OWED_RETRY: Duration = Duration::from_secs(1);

enum Command {
    Add(RootSpec),
    Remove(WatchId),
}

pub(crate) struct Backend {
    commands: Sender<Command>,
    waker: Arc<OwnedFd>,
}

impl Backend {
    pub(crate) fn start() -> io::Result<Self> {
        // SAFETY: plain system call; the result is checked below.
        let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a fresh descriptor owned by nobody else.
        let waker = Arc::new(unsafe { OwnedFd::from_raw_fd(raw) });
        let (commands, receiver) = crossbeam_channel::unbounded();
        let thread_waker = waker.clone();
        std::thread::Builder::new()
            .name("watch-inotify".into())
            .spawn(move || run(receiver, thread_waker))?;
        Ok(Self { commands, waker })
    }

    pub(crate) fn add(&self, spec: RootSpec) {
        if self.commands.send(Command::Add(spec)).is_ok() {
            self.wake();
        }
    }

    pub(crate) fn remove(&self, id: WatchId) {
        if self.commands.send(Command::Remove(id)).is_ok() {
            self.wake();
        }
    }

    fn wake(&self) {
        let one: u64 = 1;
        // SAFETY: writes the 8-byte counter increment the eventfd expects.
        unsafe {
            libc::write(
                self.waker.as_raw_fd(),
                (&one as *const u64).cast(),
                std::mem::size_of::<u64>(),
            )
        };
    }
}

fn run(commands: Receiver<Command>, waker: Arc<OwnedFd>) {
    let mut tree = Tree::new();
    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Add(spec) => tree.add(spec),
                Command::Remove(id) => tree.remove(id),
            }
        }
        let now = Instant::now();
        tree.rearm_due(now);
        let mut timeout = tree
            .next_retry()
            .map(|at| at.saturating_duration_since(now));
        if overflow_owed() {
            timeout = Some(timeout.map_or(OWED_RETRY, |wait| wait.min(OWED_RETRY)));
        }
        let millis = timeout.map_or(-1, |wait| {
            libc::c_int::try_from(wait.as_millis().max(1)).unwrap_or(libc::c_int::MAX)
        });
        let mut descriptors = [
            libc::pollfd {
                fd: waker.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: tree.raw_fd().unwrap_or(-1),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: two valid pollfd entries; a negative fd is ignored.
        let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, millis) };
        if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            // Nothing sensible is left to wait on; avoid a busy loop.
            std::thread::sleep(OWED_RETRY);
        }
        if descriptors[0].revents & libc::POLLIN != 0 {
            let mut counter = 0u64;
            // SAFETY: reads the 8-byte counter into a local.
            unsafe {
                libc::read(
                    waker.as_raw_fd(),
                    (&mut counter as *mut u64).cast(),
                    std::mem::size_of::<u64>(),
                )
            };
        }
        if descriptors[1].revents & libc::POLLIN != 0 {
            tree.read_events();
        }
        deliver_owed();
    }
}
