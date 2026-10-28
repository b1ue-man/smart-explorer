//! Windows TCP socket facts (design 4.1, 2.7): `GetExtendedTcpTable` with
//! owner PIDs, process identity from `GetProcessTimes`, the account from the
//! process token. Same facade as `share/os/linux_os/proc_net_tcp.rs`.
use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, FILETIME, HANDLE};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
    TCP_TABLE_OWNER_PID_CONNECTIONS, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

pub(crate) use crate::share::loopback_services::{ListenerRow, OwnerClass};

const AF_INET: u32 = 2;
const AF_INET6: u32 = 23;
const NO_ERROR: u32 = 0;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
/// A table above this size is reported as an error, never truncated.
const MAX_TABLE_BYTES: u32 = 64 * 1024 * 1024;

/// Identity of one process: creation time as FILETIME ticks, so a reused
/// PID never matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessIdentity {
    pub pid: u32,
    pub started: u64,
    pub image: Option<PathBuf>,
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned open by the OS and is closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// One complete table of `family`/`class`, in a u64-aligned buffer.
fn raw_table(family: u32, class: i32) -> io::Result<Vec<u64>> {
    let mut size = 0u32;
    // SAFETY: a null buffer with size 0 asks only for the required size.
    let first =
        unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, family, class, 0) };
    if first != ERROR_INSUFFICIENT_BUFFER && first != NO_ERROR {
        return Err(io::Error::from_raw_os_error(first as i32));
    }
    for _ in 0..8 {
        if size > MAX_TABLE_BYTES {
            return Err(io::Error::other("TCP table exceeds its bound"));
        }
        let mut buffer = vec![0u64; (size as usize).div_ceil(8).max(1)];
        let mut length = (buffer.len() * 8) as u32;
        // SAFETY: the buffer is writable for `length` bytes.
        let result = unsafe {
            GetExtendedTcpTable(buffer.as_mut_ptr().cast(), &mut length, 0, family, class, 0)
        };
        match result {
            NO_ERROR => return Ok(buffer),
            // The table grew between both calls: the OS reported the new size.
            ERROR_INSUFFICIENT_BUFFER => size = length,
            error => return Err(io::Error::from_raw_os_error(error as i32)),
        }
    }
    Err(io::Error::other(
        "TCP table kept growing while it was read; read it again",
    ))
}

/// Rows of the table in `buffer`: a `u32` entry count, then the rows.
fn rows<T: Copy>(buffer: &[u64]) -> Vec<T> {
    let bytes = buffer.len() * 8;
    // SAFETY: the buffer starts with dwNumEntries; rows follow at the
    // alignment of T (u32 for both row types) and are bounds-checked here.
    unsafe {
        let base = buffer.as_ptr().cast::<u8>();
        let count = *base.cast::<u32>() as usize;
        let offset = std::mem::size_of::<u32>();
        let fit = bytes.saturating_sub(offset) / std::mem::size_of::<T>();
        let first = base.add(offset).cast::<T>();
        (0..count.min(fit)).map(|i| *first.add(i)).collect()
    }
}

fn port(raw: u32) -> u16 {
    u16::from_be((raw & 0xFFFF) as u16)
}

fn v4_rows(class: i32) -> io::Result<Vec<(SocketAddr, SocketAddr, u32)>> {
    let buffer = raw_table(AF_INET, class)?;
    Ok(rows::<MIB_TCPROW_OWNER_PID>(&buffer)
        .into_iter()
        .map(|row| {
            let local = Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes());
            let remote = Ipv4Addr::from(row.dwRemoteAddr.to_ne_bytes());
            (
                SocketAddr::new(IpAddr::V4(local), port(row.dwLocalPort)),
                SocketAddr::new(IpAddr::V4(remote), port(row.dwRemotePort)),
                row.dwOwningPid,
            )
        })
        .collect())
}

fn v6_rows(class: i32) -> io::Result<Vec<(SocketAddr, SocketAddr, u32)>> {
    let buffer = raw_table(AF_INET6, class)?;
    Ok(rows::<MIB_TCP6ROW_OWNER_PID>(&buffer)
        .into_iter()
        .map(|row| {
            (
                SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr)),
                    port(row.dwLocalPort),
                ),
                SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::from(row.ucRemoteAddr)),
                    port(row.dwRemotePort),
                ),
                row.dwOwningPid,
            )
        })
        .collect())
}

fn open_process(pid: u32) -> io::Result<Handle> {
    // SAFETY: plain query-only open; the handle is owned by `Handle`.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(Handle(handle))
}

/// The string SID of the process token's user.
pub(crate) fn process_sid(pid: u32) -> io::Result<String> {
    let process = open_process(pid)?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: query-only token open of an owned process handle.
    if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(raw);
    let mut size = 0;
    // SAFETY: size query with a null buffer.
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut size);
    }
    if size == 0 || size > 65536 {
        return Err(io::Error::last_os_error());
    }
    let mut bytes = vec![0u64; (size as usize).div_ceil(8)];
    let buffer = bytes.as_mut_ptr().cast();
    // SAFETY: the u64 buffer is aligned and `size` bytes long.
    let filled = unsafe { GetTokenInformation(token.0, TokenUser, buffer, size, &mut size) };
    if filled == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetTokenInformation(TokenUser) filled a TOKEN_USER.
    let user = unsafe { &*bytes.as_ptr().cast::<TOKEN_USER>() };
    let mut text = std::ptr::null_mut();
    // SAFETY: the SID points into `bytes`, which outlives this call.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut length = 0;
    // SAFETY: ConvertSidToStringSidW returns a NUL-terminated string.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: `length` UTF-16 units precede the terminator.
    let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
    // SAFETY: the string was allocated by the OS with LocalAlloc.
    unsafe {
        LocalFree(text.cast());
    }
    Ok(sid)
}

/// Design 4.2: a user SID (`S-1-5-21-…` local/domain, `S-1-12-1-…` Entra ID)
/// is a person's account; SYSTEM, LOCAL/NETWORK SERVICE, `NT SERVICE\*`
/// (`S-1-5-80-…`) and every other well-known SID count as system.
pub(crate) fn sid_class(sid: &str) -> OwnerClass {
    if sid.starts_with("S-1-5-21-") || sid.starts_with("S-1-12-1-") {
        OwnerClass::NormalUser {
            uid_or_sid: sid.to_string(),
        }
    } else {
        OwnerClass::System
    }
}

/// Every TCP listener with its owner PID and owner class. A token that this
/// account cannot open counts as `Unknown` (treated as a system account).
pub(crate) fn listeners() -> io::Result<Vec<ListenerRow>> {
    let mut rows = v4_rows(TCP_TABLE_OWNER_PID_LISTENER)?;
    rows.extend(v6_rows(TCP_TABLE_OWNER_PID_LISTENER)?);
    let mut classes: BTreeMap<u32, OwnerClass> = BTreeMap::new();
    Ok(rows
        .into_iter()
        .map(|(local, _, pid)| {
            let owner = classes
                .entry(pid)
                .or_insert_with(|| match pid {
                    // The System Idle Process and the System process.
                    0 | 4 => OwnerClass::System,
                    _ => process_sid(pid).map_or(OwnerClass::Unknown, |sid| sid_class(&sid)),
                })
                .clone();
            ListenerRow {
                local,
                pid: Some(pid),
                owner,
            }
        })
        .collect())
}

/// The PID owning the connection socket `local → remote` (design 2.7.1).
#[allow(dead_code)]
pub(crate) fn connection_owner(local: SocketAddr, remote: SocketAddr) -> io::Result<Option<u32>> {
    let rows = match local {
        SocketAddr::V4(_) => v4_rows(TCP_TABLE_OWNER_PID_CONNECTIONS)?,
        SocketAddr::V6(_) => v6_rows(TCP_TABLE_OWNER_PID_CONNECTIONS)?,
    };
    Ok(rows
        .into_iter()
        .find(|(row_local, row_remote, _)| *row_local == local && *row_remote == remote)
        .map(|(_, _, pid)| pid))
}

/// The string SID owning the connection socket `local → remote`.
// Consumer: the viewer's listener admission (work package WP2, design 4.4).
#[allow(dead_code)]
pub(crate) fn connection_account(
    local: SocketAddr,
    remote: SocketAddr,
) -> io::Result<Option<String>> {
    match connection_owner(local, remote)? {
        Some(pid) => process_sid(pid).map(Some),
        None => Ok(None),
    }
}

/// The account of this process, in the form of `connection_account`.
#[allow(dead_code)]
pub(crate) fn own_account() -> io::Result<String> {
    process_sid(std::process::id())
}

pub(crate) fn process_identity(pid: u32) -> io::Result<ProcessIdentity> {
    let process = open_process(pid)?;
    // SAFETY: FILETIME is plain data; zeroes are valid output buffers.
    let mut times: [FILETIME; 4] = unsafe { std::mem::zeroed() };
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: owned handle, writable outputs.
    if unsafe { GetProcessTimes(process.0, created, exited, kernel, user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let created = times[0];
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    let pointer = path.as_mut_ptr();
    // SAFETY: the buffer holds `length` UTF-16 units.
    let named = unsafe { QueryFullProcessImageNameW(process.0, 0, pointer, &mut length) } != 0;
    let image = named.then(|| PathBuf::from(String::from_utf16_lossy(&path[..length as usize])));
    Ok(ProcessIdentity {
        pid,
        started: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        image,
    })
}

pub(crate) fn process_name(pid: u32) -> Option<String> {
    let image = process_identity(pid).ok()?.image?;
    image
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.chars().take(256).collect())
}

/// Whether every LISTEN socket on `port` belongs to `pid` (design 2.7 point 4).
pub(crate) fn listener_owned_by(port: u16, pid: u32) -> io::Result<bool> {
    let mut rows = v4_rows(TCP_TABLE_OWNER_PID_LISTENER)?;
    rows.extend(v6_rows(TCP_TABLE_OWNER_PID_LISTENER)?);
    let listening: Vec<_> = rows
        .into_iter()
        .filter(|(local, _, _)| local.port() == port)
        .collect();
    Ok(!listening.is_empty() && listening.iter().all(|(_, _, owner)| *owner == pid))
}
