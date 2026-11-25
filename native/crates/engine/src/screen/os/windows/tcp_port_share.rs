//! Port sharing of the direct TCP picture link (`shared/video_tcp_link.rs`)
//! on Windows: the listener, every dial socket and the reflector question
//! bind one port. `SO_REUSEADDR` on the later sockets admits that while no
//! socket on the port set `SO_EXCLUSIVEADDRUSE`; Windows has no
//! `SO_REUSEPORT`. Another process could bind the port the same way; it
//! gets nothing it can use, because TLS pins the host key and the host
//! checks the session token.
use std::io;

use tokio::net::TcpSocket;

pub(crate) fn share_port(socket: &TcpSocket) -> io::Result<()> {
    socket.set_reuseaddr(true)
}
