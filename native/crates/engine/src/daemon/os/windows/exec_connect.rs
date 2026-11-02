//! Socket-local TCP_INFO detects FIN/CLOSE_WAIT behind unread input without recv.
use std::{io, net::TcpStream, os::windows::io::AsRawSocket};
use windows_sys::Win32::Networking::WinSock::{
    TCP_INFO_v0, WSAGetLastError, WSAIoctl, WSAPoll, POLLERR, POLLHUP, POLLNVAL, POLLRDNORM,
    SIO_TCP_INFO, TCPSTATE_ESTABLISHED, WSAPOLLFD,
};

pub(super) fn probe(stream: &TcpStream) -> io::Result<(bool, bool)> {
    let socket = stream.as_raw_socket() as usize;
    let version = 0u32;
    let mut returned = 0u32;
    // SAFETY: TCP_INFO_v0 is POD; WSAIoctl initializes it through the sized pointer.
    let mut info: TCP_INFO_v0 = unsafe { std::mem::zeroed() };
    let result = unsafe {
        WSAIoctl(
            socket,
            SIO_TCP_INFO,
            &version as *const _ as *const _,
            std::mem::size_of_val(&version) as u32,
            &mut info as *mut _ as *mut _,
            std::mem::size_of_val(&info) as u32,
            &mut returned,
            std::ptr::null_mut(),
            None,
        )
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    if returned < std::mem::size_of_val(&info) as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unvollstaendige TCP_INFO_v0-Antwort",
        ));
    }
    if info.State != TCPSTATE_ESTABLISHED {
        return Ok((true, false));
    }
    let mut descriptor = WSAPOLLFD {
        fd: socket,
        events: POLLRDNORM,
        revents: 0,
    };
    // SAFETY: a live socket and one writable descriptor; the zero timeout is nonblocking.
    if unsafe { WSAPoll(&mut descriptor, 1, 0) } < 0 {
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    Ok((
        descriptor.revents & (POLLERR | POLLHUP | POLLNVAL) != 0,
        descriptor.revents & POLLRDNORM != 0,
    ))
}
