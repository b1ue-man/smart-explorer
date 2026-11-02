//! POLLRDHUP observes a received FIN even when ordinary input remains queued.
use std::{io, net::TcpStream, os::fd::AsRawFd};

pub(super) fn probe(stream: &TcpStream) -> io::Result<(bool, bool)> {
    let mut descriptor = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN | libc::POLLRDHUP,
        revents: 0,
    };
    // SAFETY: a live socket and one writable pollfd; zero timeout never blocks.
    let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
    if result < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok((false, false))
        } else {
            Err(error)
        };
    }
    Ok((
        descriptor.revents & (libc::POLLRDHUP | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)
            != 0,
        descriptor.revents & libc::POLLIN != 0,
    ))
}
