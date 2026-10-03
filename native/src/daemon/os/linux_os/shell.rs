//! A hook shell and its process group; cancellation also ends its children.
use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};

pub(crate) fn shell_command(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    command
}

pub(crate) struct ShellChild {
    child: Child,
    finished: bool,
}

pub(crate) fn spawn_shell(mut command: Command) -> io::Result<ShellChild> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setpgid is async-signal-safe; no allocation in the fork hook.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    command.spawn().map(|child| ShellChild {
        child,
        finished: false,
    })
}

impl ShellChild {
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.finished {
            return self.child.try_wait();
        }
        // Keep an exited leader unreaped until Drop kills its group; its
        // numeric PID cannot be reused while descendants are being cleaned.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        let status = unsafe { info.si_status() };
        let raw = if info.si_code == libc::CLD_EXITED {
            status << 8
        } else {
            status
                | if info.si_code == libc::CLD_DUMPED {
                    0x80
                } else {
                    0
                }
        };
        Ok(Some(ExitStatus::from_raw(raw)))
    }
    pub(crate) fn stop(&mut self) {
        if self.finished {
            return;
        }
        if let Ok(pid) = i32::try_from(self.child.id()) {
            // SAFETY: the child is leader of its own group. Negative PID
            // addresses that group only; descendants cannot outlive a hook.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.finished = true;
    }
}

impl Drop for ShellChild {
    fn drop(&mut self) {
        self.stop();
    }
}
