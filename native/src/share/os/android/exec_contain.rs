//! Per-job subreaper of the Android exec host. The supervisor spawns an
//! intermediate process instead of the job's root: between `fork` and `exec`
//! (std's `pre_exec`) it becomes a child subreaper and clones the root with a
//! raw `clone(SIGCHLD)`; the root returns and becomes the shell, the
//! intermediate never returns. It reports the root's wait status through a
//! status pipe, reaps everything below it until `ECHILD` and exits. Orphans
//! of the job (double forks, `setsid` children) are reparented to it, so the
//! job's tree stays below it until the end: `terminate` kills that tree from
//! `/proc` (never the intermediate, never other children of the app such as
//! sync hooks) and the job is empty once the intermediate was reaped.
//!
//! After an app crash the intermediates live on under init. Each job keeps a
//! record (pid and start time) in app-private storage; the first provider
//! use of the next app process ends trees whose record still matches.

use std::fs::{DirBuilder, File};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Once, PoisonError};
use std::time::{Duration, Instant};

use super::exec_supervisor::{ArmedIntermediate, Intermediate};
use crate::share::exec_proc::{
    is_recorded_root, kill_plan, parse_record, parse_stat, record_text, ProcStat,
};

/// Directory of the crash records below the worker's data directory.
const RECORDS: &str = "exec-roots";
/// How long `terminate` keeps killing before `confirm_empty` takes over.
const TERMINATE_BUDGET: Duration = Duration::from_secs(2);
/// How long the crash recovery works on one leftover tree.
const LEFTOVER_BUDGET: Duration = Duration::from_secs(3);
const KILL_PAUSE: Duration = Duration::from_millis(10);
const WAIT_POLL: Duration = Duration::from_millis(20);
const MAX_REAP_PAUSE: Duration = Duration::from_millis(250);
/// Upper bound of the descriptor sweep in the intermediate.
const MAX_FD_SWEEP: libc::rlim_t = 1 << 20;

/// State one job shares between its `ContainedExec` and the supervisor thread.
pub(super) struct JobRoot {
    record: PathBuf,
    state: Mutex<RootState>,
    changed: Condvar,
    terminate: AtomicBool,
}

#[derive(Default)]
struct RootState {
    /// The running intermediate; `None` before the spawn and once reaped. A
    /// pid is only signalled or scanned under this lock while it is set, so
    /// a reaped (and possibly reused) pid is never touched.
    intermediate: Option<libc::pid_t>,
    /// The supervisor thread has ended: no process of the job is left.
    done: bool,
}

impl JobRoot {
    pub(super) fn new(exec_id: &str) -> Self {
        Self {
            record: records_dir().join(exec_id),
            state: Mutex::new(RootState::default()),
            changed: Condvar::new(),
            terminate: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, RootState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Kills the job's tree until a pass finds nothing alive or the budget
    /// is used up; `wait_empty` and the supervisor keep going afterwards.
    pub(super) fn terminate(&self) {
        self.terminate.store(true, Ordering::Release);
        let deadline = Instant::now() + TERMINATE_BUDGET;
        loop {
            let alive = match self.lock().intermediate {
                Some(pid) => kill_tree_once(pid),
                None => 0,
            };
            if alive == 0 || Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(KILL_PAUSE);
        }
    }

    /// Waits until nothing of the job runs any more (the supervisor thread
    /// ended after reaping the intermediate), killing on while terminating.
    pub(super) fn wait_empty(&self, deadline: Instant) -> io::Result<()> {
        let mut state = self.lock();
        loop {
            if state.done {
                return Ok(());
            }
            if self.terminate.load(Ordering::Acquire) {
                if let Some(pid) = state.intermediate {
                    kill_tree_once(pid);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Prozesse des Befehls laufen noch",
                ));
            }
            state = self
                .changed
                .wait_timeout(state, (deadline - now).min(WAIT_POLL))
                .map(|(state, _)| state)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
    }

    /// The supervisor thread ends (after `finish`, or without a spawn).
    pub(super) fn supervisor_done(&self) {
        self.lock().done = true;
        self.changed.notify_all();
    }

    fn spawned(&self, pid: libc::pid_t) {
        self.lock().intermediate = Some(pid);
        // Best effort: without a record the job still runs, it only escapes
        // the cleanup after an app crash.
        if let Some(stat) = stat_of(pid) {
            let _ = write_record(&self.record, &record_text(pid, stat.start_time));
        }
    }

    /// Kills on until the intermediate has ended, then reaps it.
    fn finish(&self, child: &mut Child) {
        self.terminate.store(true, Ordering::Release);
        let mut pause = KILL_PAUSE;
        loop {
            {
                let mut state = self.lock();
                if let Some(pid) = state.intermediate {
                    kill_tree_once(pid);
                }
                if !matches!(child.try_wait(), Ok(None)) {
                    state.intermediate = None;
                    let _ = std::fs::remove_file(&self.record);
                    self.changed.notify_all();
                    return;
                }
            }
            std::thread::sleep(pause);
            pause = (pause * 2).min(MAX_REAP_PAUSE);
        }
    }
}

/// The intermediate of the supervisor's spawn policy on Android.
pub(super) struct Subreaper {
    root: Arc<JobRoot>,
}

impl Subreaper {
    pub(super) fn new(root: Arc<JobRoot>) -> Self {
        Self { root }
    }
}

impl Intermediate for Subreaper {
    fn arm(
        &self,
        command: &mut Command,
        own_process_group: bool,
    ) -> io::Result<Box<dyn ArmedIntermediate>> {
        let (status, write) = status_pipe()?;
        let status_fd = write.as_raw_fd();
        // SAFETY: the hook runs in the forked child and only makes
        // async-signal-safe calls without allocating (`intermediate_hook`).
        unsafe {
            command.pre_exec(move || intermediate_hook(status_fd, own_process_group));
        }
        Ok(Box::new(Armed {
            root: self.root.clone(),
            status,
            write: Some(write),
            received: [0; 4],
            filled: 0,
        }))
    }
}

struct Armed {
    root: Arc<JobRoot>,
    /// Non-blocking read end of the status pipe.
    status: File,
    /// This process' copy of the write end, closed once the child has its own
    /// so that the intermediate's exit shows as end of file.
    write: Option<OwnedFd>,
    received: [u8; 4],
    filled: usize,
}

impl ArmedIntermediate for Armed {
    fn spawned(&mut self, pid: u32) {
        self.write = None;
        // Linux pids always fit; `finish` would otherwise still wait for the
        // intermediate, which ends once its tree has.
        if let Ok(pid) = libc::pid_t::try_from(pid) {
            self.root.spawned(pid);
        }
    }

    fn root_status(&mut self) -> io::Result<Option<ExitStatus>> {
        while self.filled < self.received.len() {
            match self.status.read(&mut self.received[self.filled..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Zwischenprozess des Befehls endete ohne Status",
                    ))
                }
                Ok(read) => self.filled += read,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(Some(ExitStatus::from_raw(i32::from_ne_bytes(
            self.received,
        ))))
    }

    fn finish(&mut self, child: &mut Child) {
        self.root.finish(child);
    }
}

/// `O_CLOEXEC` pipe: the root closes its copy of the write end on `exec`, the
/// intermediate (which never execs) keeps it. The write end never lands on
/// a standard descriptor, which the spawn replaces before the hook runs.
fn status_pipe() -> io::Result<(File, OwnedFd)> {
    let mut fds: [libc::c_int; 2] = [0; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes; after its
    // success both are new and owned here, and the fcntl calls only change
    // flags of or duplicate those owned descriptors.
    unsafe {
        if libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) != 0 {
            return Err(io::Error::last_os_error());
        }
        let read = OwnedFd::from_raw_fd(fds[0]);
        let mut write = OwnedFd::from_raw_fd(fds[1]);
        let flags = libc::fcntl(read.as_raw_fd(), libc::F_GETFL);
        if flags < 0 || libc::fcntl(read.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
        if write.as_raw_fd() <= libc::STDERR_FILENO {
            let copy = libc::fcntl(write.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3);
            if copy < 0 {
                return Err(io::Error::last_os_error());
            }
            write = OwnedFd::from_raw_fd(copy);
        }
        Ok((File::from(read), write))
    }
}

/// Runs in the forked child between `fork` and `exec`: only async-signal-safe
/// calls, no allocation, no locks. A raw `clone` instead of `fork()` skips
/// the atfork handlers, which could wait for locks another thread of the app
/// held when it forked. The root returns (and execs the shell); the
/// intermediate never returns.
fn intermediate_hook(status_fd: RawFd, own_process_group: bool) -> io::Result<()> {
    // SAFETY: signal-mask and disposition changes, prctl, clone and setpgid
    // are async-signal-safe system calls on memory owned by this frame.
    unsafe {
        // What the root inherits: SIGCHLD reported (not auto-reaped) and
        // SIGPIPE deliverable (the supervisor threads block it).
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
        let mut pipe_signal: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut pipe_signal);
        libc::sigaddset(&mut pipe_signal, libc::SIGPIPE);
        libc::sigprocmask(libc::SIG_UNBLOCK, &pipe_signal, std::ptr::null_mut());
        let (on, off): (libc::c_ulong, libc::c_ulong) = (1, 0);
        if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, on, off, off, off) != 0 {
            return Err(io::Error::last_os_error());
        }
        // clone(flags, stack, parent_tid, child_tid/tls, tls/child_tid): only
        // the exit signal is set, so the order of the null arguments (it
        // differs between architectures) does not matter.
        let flags = libc::c_long::from(libc::SIGCHLD);
        let none: libc::c_long = 0;
        let pid = libc::syscall(libc::SYS_clone, flags, none, none, none, none);
        if pid < 0 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            // The root: its own process group lets one group signal reach
            // the shell and its plain children without the intermediate. A
            // failed exec after this return still reaches the spawner
            // through std's error pipe, which the root inherited.
            if own_process_group && libc::setpgid(0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            return Ok(());
        }
        supervise_root(pid as libc::pid_t, status_fd)
    }
}

/// The intermediate's life: drop every descriptor but the status pipe (the
/// output pipes and std's error pipe must see end of file without it),
/// report the root's wait status, reap everything below until `ECHILD`.
fn supervise_root(root: libc::pid_t, status_fd: RawFd) -> ! {
    // SAFETY: async-signal-safe system calls on memory owned by this frame.
    unsafe {
        // Only the root inherited the defaults; the intermediate must not be
        // ended by a closed pipe or a stray termination signal, since its
        // death would hand the job's orphans to init.
        for signal in [
            libc::SIGPIPE,
            libc::SIGHUP,
            libc::SIGINT,
            libc::SIGTERM,
            libc::SIGQUIT,
        ] {
            libc::signal(signal, libc::SIG_IGN);
        }
        let mut limit: libc::rlimit = std::mem::zeroed();
        let sweep = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0
            && limit.rlim_cur != libc::RLIM_INFINITY
        {
            limit.rlim_cur.min(MAX_FD_SWEEP)
        } else {
            MAX_FD_SWEEP
        };
        let mut fd: RawFd = 0;
        while (fd as libc::rlim_t) < sweep {
            if fd != status_fd {
                libc::close(fd);
            }
            fd += 1;
        }
        let mut status_fd = Some(status_fd);
        loop {
            let mut status: libc::c_int = 0;
            let pid = libc::waitpid(-1, &mut status, 0);
            if pid == root {
                if let Some(fd) = status_fd.take() {
                    write_all_raw(fd, &status.to_ne_bytes());
                    libc::close(fd);
                }
            } else if pid < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
        }
        libc::_exit(0)
    }
}

fn write_all_raw(fd: RawFd, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        // SAFETY: writes from a live slice; no allocation.
        let written = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if written > 0 {
            bytes = &bytes[written as usize..];
        } else if written == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return;
        }
    }
}

/// One kill pass over the tree below `intermediate`; returns how many live
/// descendants it found (and signalled).
fn kill_tree_once(intermediate: libc::pid_t) -> usize {
    let plan = kill_plan(&proc_table(), intermediate, std::process::id() as i32);
    // SAFETY: plain signals to pids and groups of this job (see `kill_plan`).
    unsafe {
        for group in &plan.groups {
            libc::kill(-group, libc::SIGKILL);
        }
        for pid in &plan.pids {
            libc::kill(*pid, libc::SIGKILL);
        }
    }
    plan.pids.len()
}

/// Snapshot of every process this app may see (`hidepid` hides others).
fn proc_table() -> Vec<ProcStat> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: i32 = entry.file_name().to_str()?.parse().ok()?;
            stat_of(pid)
        })
        .collect()
}

fn stat_of(pid: libc::pid_t) -> Option<ProcStat> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat(&text).filter(|stat| stat.pid == pid)
}

fn records_dir() -> PathBuf {
    crate::support_dirs::sync_data_dir().join(RECORDS)
}

fn write_record(path: &std::path::Path, text: &str) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    std::fs::write(path, text)
}

/// Ends the trees of intermediates a crashed app process left behind (same
/// pid and start time as recorded), then forgets every record. Runs once per
/// app process, before its first job starts.
pub(super) fn recover_once() {
    static RECOVERED: Once = Once::new();
    RECOVERED.call_once(|| {
        let Ok(entries) = std::fs::read_dir(records_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let record = std::fs::read_to_string(&path).ok();
            if let Some((pid, start_time)) = record.as_deref().and_then(parse_record) {
                end_leftover(pid, start_time);
            }
            let _ = std::fs::remove_file(&path);
        }
    });
}

fn end_leftover(pid: libc::pid_t, start_time: u64) {
    if !is_recorded_root(stat_of(pid).as_ref(), pid, start_time) {
        return;
    }
    // The tree first: the intermediate keeps it chained below itself.
    let deadline = Instant::now() + LEFTOVER_BUDGET;
    while kill_tree_once(pid) > 0 && Instant::now() < deadline {
        std::thread::sleep(KILL_PAUSE);
    }
    // It normally exits on its own now; the start time is checked again right
    // before the last signal because its pid may be free by then.
    if is_recorded_root(stat_of(pid).as_ref(), pid, start_time) {
        // SAFETY: a plain signal to the verified leftover intermediate.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}
