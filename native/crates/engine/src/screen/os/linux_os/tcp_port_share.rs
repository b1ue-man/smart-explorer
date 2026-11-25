//! Port sharing of the direct TCP picture link (`shared/video_tcp_link.rs`)
//! on Linux and Android (the same kernel): the listener, every dial socket
//! and the reflector question bind one port. Linux admits a further socket
//! on a port with a listener only when all of them set `SO_REUSEPORT` (and
//! belong to the same user); `SO_REUSEADDR` alone is refused there.
use std::io;

use tokio::net::TcpSocket;

pub(crate) fn share_port(socket: &TcpSocket) -> io::Result<()> {
    socket.set_reuseaddr(true)?;
    socket.set_reuseport(true)
}
