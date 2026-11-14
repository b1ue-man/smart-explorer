//! E29 (M11): the connections that count as a session of the GNOME desktop
//! sharing, read from the kernel's TCP tables. The se tunnel of a session is an
//! accepted loopback connection on the RDP port; a LAN peer is not counted (S7).

/// Peer addresses of loopback connections in `/proc/net/tcp` (IPv4, IPv6, mapped).
const LOOPBACK_PEERS: [&str; 3] = [
    "0100007F",
    "00000000000000000000000001000000",
    "0000000000000000FFFF00000100007F",
];

/// Established connections from loopback peers to one of `ports`, counted on
/// the listening side: one row per accepted connection (E29, S7).
pub(super) fn loopback_connections(ports: &[u16]) -> usize {
    ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .map(|table| {
            std::fs::read_to_string(*table).map_or(0, |text| {
                text.lines()
                    .skip(1)
                    .filter(|row| accepted_from_loopback(row, ports))
                    .count()
            })
        })
        .sum()
}

/// One row of a TCP table: the local port is one of `ports`, the peer address is
/// loopback, and the connection is established (state `01`).
pub(super) fn accepted_from_loopback(row: &str, ports: &[u16]) -> bool {
    let fields: Vec<&str> = row.split_whitespace().collect();
    let (Some(local), Some(remote), Some(state)) = (fields.get(1), fields.get(2), fields.get(3))
    else {
        return false;
    };
    let (Some((_, port)), Some((peer, _))) = (local.rsplit_once(':'), remote.rsplit_once(':'))
    else {
        return false;
    };
    *state == "01"
        && LOOPBACK_PEERS.contains(&peer)
        && u16::from_str_radix(port, 16).is_ok_and(|port| ports.contains(&port))
}
