use std::fs::File;
use std::io;
use std::os::fd::FromRawFd;

pub(super) fn protect_stdout() -> io::Result<File> {
    // SAFETY: duplicating and redirecting the process's own standard
    // descriptors; the duplicate is owned by the returned File only.
    unsafe {
        let protocol = libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD_CLOEXEC, 3);
        if protocol < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) < 0 {
            let error = io::Error::last_os_error();
            libc::close(protocol);
            return Err(error);
        }
        Ok(File::from_raw_fd(protocol))
    }
}

pub(super) fn configure_child(_command: &mut std::process::Command) {}

pub(super) fn request_child_stop(child: &std::process::Child) -> bool {
    let Ok(pid) = libc::pid_t::try_from(child.id()) else {
        return false;
    };
    // SAFETY: signals only the child this server started and still owns.
    unsafe { libc::kill(pid, libc::SIGINT) == 0 }
}
