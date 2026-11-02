//! systemd attributes this datagram to the existing main process (NotifyAccess=main).
use std::{
    io,
    os::{
        linux::net::SocketAddrExt,
        unix::{
            ffi::OsStrExt,
            net::{SocketAddr, UnixDatagram},
        },
    },
};
pub(super) fn notify() -> io::Result<()> {
    let Some(socket) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    let install = crate::login::os::shared::installation::running()?;
    let bytes = socket.as_bytes();
    let address = if bytes.first() == Some(&b'@') {
        SocketAddr::from_abstract_name(&bytes[1..])?
    } else {
        SocketAddr::from_pathname(std::path::Path::new(&socket))?
    };
    let message = format!(
        "READY=1\nSTATUS=Smart Explorer login ready version={} generation={} sha256={}",
        install.manifest.version, install.manifest.generation, install.manifest.sha256
    );
    let datagram = UnixDatagram::unbound()?;
    if datagram.send_to_addr(message.as_bytes(), &address)? != message.len() {
        return Err(io::Error::other(
            "incomplete systemd login readiness notification",
        ));
    }
    Ok(())
}
