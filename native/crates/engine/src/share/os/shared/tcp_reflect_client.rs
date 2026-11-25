//! Client of the Share server's TCP address reflector (`tcp_reflect_v1`,
//! `share/core/address_services.rs`): connects from a socket the caller
//! bound to the port whose public mapping it wants and reads the one line
//! the reflector writes, `se-reflect 1 <ip>:<port>\n`. The stream stays
//! open and goes back to the caller: while it is held the NAT keeps the
//! mapping of that port, which is the one a peer must hit; the server ends
//! it after its idle bound.
use std::io;
use std::net::{IpAddr, SocketAddr};

use tokio::net::{TcpSocket, TcpStream};
use tokio::time::{timeout_at, Instant};

use super::address_services::{parse_reflect_line, MAX_REFLECT_LINE_BYTES};

fn timed_out(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("TCP-Adressspiegel des Share-Servers: Zeitüberschreitung beim {what}"),
    )
}

/// The addresses of `host` (a DNS name or an IP, IPv6 with or without
/// brackets) with `port`. Names go through Hickory's async resolver within
/// `deadline`, as the signaling connection resolves them.
pub async fn resolve(host: &str, port: u16, deadline: Instant) -> io::Result<Vec<SocketAddr>> {
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let resolver = hickory_resolver::Resolver::builder_tokio()
        .map_err(io::Error::other)?
        .build()
        .map_err(io::Error::other)?;
    let lookup = timeout_at(deadline, resolver.lookup_ip(host))
        .await
        .map_err(|_| timed_out("Auflösen des Namens"))?
        .map_err(io::Error::other)?;
    let addresses: Vec<SocketAddr> = lookup.iter().map(|ip| SocketAddr::new(ip, port)).collect();
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "TCP-Adressspiegel des Share-Servers: der Name hat keine Adresse",
        ));
    }
    Ok(addresses)
}

/// Connects `socket` to the reflector at `server` and returns the address
/// the reflector saw with the still open stream.
pub async fn reflect(
    socket: TcpSocket,
    server: SocketAddr,
    deadline: Instant,
) -> io::Result<(SocketAddr, TcpStream)> {
    let stream = timeout_at(deadline, socket.connect(server))
        .await
        .map_err(|_| timed_out("Verbinden"))??;
    let mut line = [0u8; MAX_REFLECT_LINE_BYTES];
    let mut filled = 0;
    loop {
        timeout_at(deadline, stream.readable())
            .await
            .map_err(|_| timed_out("Lesen der Antwort"))??;
        match stream.try_read(&mut line[filled..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "TCP-Adressspiegel des Share-Servers schloss vor seiner Antwort",
                ))
            }
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            Err(error) => return Err(error),
        }
        if let Some(end) = line[..filled].iter().position(|byte| *byte == b'\n') {
            let observed = parse_reflect_line(&line[..=end]).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TCP-Adressspiegel des Share-Servers antwortete unlesbar",
                )
            })?;
            return Ok((observed, stream));
        }
        if filled == line.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TCP-Adressspiegel des Share-Servers antwortete zu lang",
            ));
        }
    }
}
