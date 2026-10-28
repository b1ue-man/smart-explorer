//! Linux (and Android) TCP socket facts from `/proc/net/tcp{,6}` (design
//! 4.1, 2.7): listeners, the owner of one connection, process identity.
//! Android apps cannot read `/proc/net` since Android 10 (assumption A7);
//! there every query reports that explicitly instead of returning nothing.
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

pub(crate) use crate::share::loopback_services::{ListenerRow, OwnerClass};

/// Upper bound of one `/proc/net/tcp*` read; a host with more sockets is
/// reported as an error instead of a silently partial list.
const MAX_TABLE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FD_ENTRIES: usize = 1 << 20;
const TCP_LISTEN: u8 = 0x0A;

/// Identity of one process: start time in clock ticks since boot (field 22
/// of `/proc/<pid>/stat`), so a reused PID never matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessIdentity {
    pub pid: u32,
    pub started: u64,
    pub image: Option<PathBuf>,
}

/// One parsed row of `/proc/net/tcp{,6}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcRow {
    pub local: SocketAddr,
    pub remote: SocketAddr,
    pub state: u8,
    pub uid: u32,
    pub inode: u64,
}

fn unsupported_on_android() -> io::Result<()> {
    if cfg!(target_os = "android") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Android erlaubt Apps seit Version 10 kein Lesen von /proc/net; lauschende Dienste sind hier nicht ermittelbar",
        ));
    }
    Ok(())
}

fn hex_address(text: &str) -> Option<IpAddr> {
    match text.len() {
        8 => {
            let word = u32::from_str_radix(text, 16).ok()?;
            Some(IpAddr::V4(Ipv4Addr::from(word.to_ne_bytes())))
        }
        32 => {
            let mut bytes = [0u8; 16];
            for (index, chunk) in bytes.chunks_mut(4).enumerate() {
                let word = u32::from_str_radix(&text[index * 8..index * 8 + 8], 16).ok()?;
                chunk.copy_from_slice(&word.to_ne_bytes());
            }
            Some(IpAddr::V6(Ipv6Addr::from(bytes)))
        }
        _ => None,
    }
}

fn hex_endpoint(text: &str) -> Option<SocketAddr> {
    let (address, port) = text.rsplit_once(':')?;
    Some(SocketAddr::new(
        hex_address(address)?,
        u16::from_str_radix(port, 16).ok()?,
    ))
}

/// Parses the text of one `/proc/net/tcp` or `/proc/net/tcp6` file. Rows
/// that do not have the documented columns are skipped.
pub(crate) fn parse_table(text: &str) -> Vec<ProcRow> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 10 {
                return None;
            }
            Some(ProcRow {
                local: hex_endpoint(fields[1])?,
                remote: hex_endpoint(fields[2])?,
                state: u8::from_str_radix(fields[3], 16).ok()?,
                uid: fields[7].parse().ok()?,
                inode: fields[9].parse().ok()?,
            })
        })
        .collect()
}

fn read_bounded(path: &Path) -> io::Result<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_TABLE_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_TABLE_BYTES {
        return Err(io::Error::other(format!(
            "{} exceeds the socket table bound",
            path.display()
        )));
    }
    Ok(text)
}

/// All rows of both tables of the caller's network namespace.
pub(crate) fn table() -> io::Result<Vec<ProcRow>> {
    unsupported_on_android()?;
    let mut rows = parse_table(&read_bounded(Path::new("/proc/net/tcp"))?);
    match read_bounded(Path::new("/proc/net/tcp6")) {
        Ok(text) => rows.extend(parse_table(&text)),
        // A kernel without IPv6 has no tcp6 table; that is not an error.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(rows)
}

/// `UID_MIN` of `/etc/login.defs` (shadow-utils), default 1000.
pub(crate) fn uid_min_from(login_defs: &str) -> u32 {
    login_defs
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next() == Some("UID_MIN"))
                .then(|| parts.next()?.parse().ok())
                .flatten()
        })
        .unwrap_or(1000)
}

fn uid_min() -> u32 {
    std::fs::read_to_string("/etc/login.defs")
        .map(|text| uid_min_from(&text))
        .unwrap_or(1000)
}

/// Design 4.2: a UID from `UID_MIN` up is a person's account; `nobody`
/// (65534) and the overflow UID are not.
pub(crate) fn owner_class(uid: u32, uid_min: u32) -> OwnerClass {
    if uid >= uid_min && uid != 65534 && uid != u32::MAX {
        OwnerClass::NormalUser {
            uid_or_sid: uid.to_string(),
        }
    } else {
        OwnerClass::System
    }
}

/// Socket inodes of `pid`'s open descriptors. Reading another account's
/// descriptors needs root; that failure is returned, not hidden.
pub(crate) fn socket_inodes(pid: u32) -> io::Result<BTreeSet<u64>> {
    let mut inodes = BTreeSet::new();
    for entry in std::fs::read_dir(format!("/proc/{pid}/fd"))?.take(MAX_FD_ENTRIES) {
        let Ok(target) = entry.and_then(|entry| std::fs::read_link(entry.path())) else {
            continue;
        };
        let target = target.to_string_lossy();
        if let Some(inode) = target
            .strip_prefix("socket:[")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|inode| inode.parse().ok())
        {
            inodes.insert(inode);
        }
    }
    Ok(inodes)
}

/// Inode → PID for every process whose descriptors this account may read.
fn inode_owners(wanted: &BTreeSet<u64>) -> BTreeMap<u64, u32> {
    let mut owners = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return owners;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        if let Ok(inodes) = socket_inodes(pid) {
            for inode in inodes.intersection(wanted) {
                owners.entry(*inode).or_insert(pid);
            }
        }
        if owners.len() == wanted.len() {
            break;
        }
    }
    owners
}

/// Every TCP listener of this network namespace with its owner class and,
/// where this account may see it, its PID.
pub(crate) fn listeners() -> io::Result<Vec<ListenerRow>> {
    let rows: Vec<_> = table()?
        .into_iter()
        .filter(|row| row.state == TCP_LISTEN)
        .collect();
    let wanted = rows.iter().map(|row| row.inode).collect();
    let owners = inode_owners(&wanted);
    let minimum = uid_min();
    Ok(rows
        .into_iter()
        .map(|row| ListenerRow {
            local: row.local,
            pid: owners.get(&row.inode).copied(),
            owner: owner_class(row.uid, minimum),
        })
        .collect())
}

fn connection_row(local: SocketAddr, remote: SocketAddr) -> io::Result<Option<ProcRow>> {
    Ok(table()?
        .into_iter()
        .find(|row| same_endpoint(row.local, local) && same_endpoint(row.remote, remote)))
}

/// IPv4 sockets on an IPv6 table appear as `::ffff:a.b.c.d`.
fn same_endpoint(row: SocketAddr, wanted: SocketAddr) -> bool {
    let canonical = |address: IpAddr| match address {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        other => other,
    };
    row.port() == wanted.port() && canonical(row.ip()) == canonical(wanted.ip())
}

/// The PID of the process that owns the connection socket `local → remote`;
/// `None` when this account cannot see that process's descriptors.
// Consumer: the viewer's RDP listener admission (work package WP2, 2.7.1).
#[allow(dead_code)]
pub(crate) fn connection_owner(local: SocketAddr, remote: SocketAddr) -> io::Result<Option<u32>> {
    let Some(row) = connection_row(local, remote)? else {
        return Ok(None);
    };
    let owners = inode_owners(&BTreeSet::from([row.inode]));
    Ok(owners.get(&row.inode).copied())
}

/// The account (decimal UID) that owns the connection socket.
// Consumer: the viewer's listener admission (work package WP2, design 4.4).
#[allow(dead_code)]
pub(crate) fn connection_account(
    local: SocketAddr,
    remote: SocketAddr,
) -> io::Result<Option<String>> {
    Ok(connection_row(local, remote)?.map(|row| row.uid.to_string()))
}

/// The account of this process, in the form of `connection_account`.
#[allow(dead_code)]
pub(crate) fn own_account() -> io::Result<String> {
    // SAFETY: geteuid takes no pointer and has no side effects.
    Ok(unsafe { libc::geteuid() }.to_string())
}

/// Field 22 (`starttime`) of `/proc/<pid>/stat`; the command name in
/// field 2 may contain spaces and parentheses, so fields count from the
/// last `)`.
pub(crate) fn start_time_from_stat(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

pub(crate) fn process_identity(pid: u32) -> io::Result<ProcessIdentity> {
    unsupported_on_android()?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let started = start_time_from_stat(&stat)
        .ok_or_else(|| io::Error::other(format!("/proc/{pid}/stat has no start time")))?;
    Ok(ProcessIdentity {
        pid,
        started,
        image: std::fs::read_link(format!("/proc/{pid}/exe")).ok(),
    })
}

pub(crate) fn process_name(pid: u32) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.chars().take(256).collect())
}

/// Whether the LISTEN socket on `port` belongs to `pid` (design 2.7 point
/// 4). Without access to that process's descriptors the socket UID must be
/// the process owner's UID (design 3.3, „Portbeleg ohne fd-Zugriff“).
pub(crate) fn listener_owned_by(port: u16, pid: u32) -> io::Result<bool> {
    let listening: Vec<_> = table()?
        .into_iter()
        .filter(|row| row.state == TCP_LISTEN && row.local.port() == port)
        .collect();
    if listening.is_empty() {
        return Ok(false);
    }
    match socket_inodes(pid) {
        Ok(inodes) => Ok(listening.iter().all(|row| inodes.contains(&row.inode))),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            use std::os::unix::fs::MetadataExt;
            let owner = std::fs::metadata(format!("/proc/{pid}"))?.uid();
            Ok(listening.iter().all(|row| row.uid == owner))
        }
        Err(error) => Err(error),
    }
}

/// Ports of LISTEN sockets owned by `uid` on loopback or wildcard addresses:
/// the RDP inventory's proof when the service's descriptors are unreadable.
#[cfg_attr(target_os = "android", allow(dead_code))]
pub(crate) fn loopback_listen_ports_of_uid(uid: u32) -> io::Result<Vec<u16>> {
    let mut ports: Vec<_> = table()?
        .into_iter()
        .filter(|row| {
            row.state == TCP_LISTEN
                && row.uid == uid
                && crate::share::loopback_services::scope_of(row.local.ip()).is_some()
        })
        .map(|row| row.local.port())
        .collect();
    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}
