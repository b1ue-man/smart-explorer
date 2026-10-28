//! Windows filesystem facts for transfers, the system sensors and the thread
//! priority. A value that cannot be read is `None`; the disk queue is `None`
//! until the PDH counter names of `PhysicalDisk` are confirmed on a runner
//! (suite case `speed_task_windows_pdh_physicaldisk_counters_exist`).
use super::sensor_parse::{CpuTicks, RawSample};
use std::path::{Path, PathBuf};

/// Redirecting links are walk boundaries. Data reparse points (cloud,
/// WOF and dedup) retain their ordinary-file transfer behavior.
pub(crate) fn upload_is_link_like(path: &Path, metadata: &std::fs::Metadata) -> bool {
    crate::local_access::metadata_is_link_like(path, metadata)
}

/// `\` separates path components on Windows; it is never part of a name.
pub(crate) fn backslash_is_name_char() -> bool {
    false
}

pub(crate) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    crate::vfs::replace_local_file(src, dest)
}

pub(crate) fn available_space_for_path(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or_else(|| Path::new("."))
    };
    let wide = path_to_wide(dir);
    let mut free = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
    (ok != 0).then_some(free)
}

/// Whether the drive under `path` spins (`Some(true)`: it reports a seek
/// penalty), is flash (`Some(false)`) or cannot be told (`None`: a network
/// path, a volume spanning several disks, or a refused query). A path that
/// does not exist yet is resolved through its volume root (M7).
pub(crate) fn volume_is_rotational(path: &Path) -> Option<bool> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::{
        PropertyStandardQuery, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_ID,
        STORAGE_PROPERTY_QUERY,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    /// `StorageDeviceSeekPenaltyProperty` (winioctl.h).
    const SEEK_PENALTY_PROPERTY: STORAGE_PROPERTY_ID = 7;
    /// `DEVICE_SEEK_PENALTY_DESCRIPTOR` (winioctl.h).
    #[repr(C)]
    #[derive(Default)]
    struct SeekPenalty {
        _version: u32,
        size: u32,
        incurs_seek_penalty: u8,
    }
    let wide = path_to_wide(path);
    let mut root = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let mut volume = vec![0u16; 1024];
    let named = unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), volume.as_mut_ptr(), volume.len() as u32)
    };
    if named == 0 {
        return None;
    }
    // `\\?\Volume{…}\` opens the volume only without its last `\`.
    let end = volume.iter().position(|c| *c == 0)?;
    if end > 0 && volume[end - 1] == u16::from(b'\\') {
        volume[end - 1] = 0;
    }
    let handle = unsafe {
        CreateFileW(
            volume.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut query: STORAGE_PROPERTY_QUERY = unsafe { std::mem::zeroed() };
    query.PropertyId = SEEK_PENALTY_PROPERTY;
    query.QueryType = PropertyStandardQuery;
    let mut answer = SeekPenalty::default();
    let mut returned = 0u32;
    let success = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&query as *const STORAGE_PROPERTY_QUERY).cast(),
            std::mem::size_of_val(&query) as u32,
            (&mut answer as *mut SeekPenalty).cast(),
            std::mem::size_of::<SeekPenalty>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    } != 0;
    unsafe {
        CloseHandle(handle);
    }
    // `size` covers the flag: an answer that ends before it tells nothing.
    let complete = returned >= 9 && answer.size >= 9;
    (success && complete).then_some(answer.incurs_seek_penalty != 0)
}

fn path_to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Physical memory currently available to programs, for the transfer budget.
pub(crate) fn available_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is plain data; dwLength must name its size.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is a valid, writable MEMORYSTATUSEX with dwLength set.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.ullAvailPhys)
}

/// Total physical memory. Stable from run to run, unlike the available memory,
/// so limits derived from it do not change with the momentary load (sync tree
/// limits, RV1).
pub(crate) fn physical_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is plain data; dwLength must name its size.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is a valid, writable MEMORYSTATUSEX with dwLength set.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.ullTotalPhys)
}

/// Lowers the priority of the calling thread to "below normal", never raising
/// one that is lower already (idle, lowest). Errors are ignored.
pub(crate) fn lower_thread_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    // `GetThreadPriority` answers this value on failure.
    const ERROR_RETURN: i32 = 0x7fff_ffff;
    // SAFETY: the pseudo handle of the calling thread is always valid.
    unsafe {
        let thread = GetCurrentThread();
        let current = GetThreadPriority(thread);
        if current != ERROR_RETURN && current > THREAD_PRIORITY_BELOW_NORMAL {
            let _ = SetThreadPriority(thread, THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
}

/// One reading of everything the sensors look at. Threads and disks have no
/// source here (`None`, empty).
pub(crate) fn read_raw(_watch: &[String]) -> RawSample {
    RawSample {
        cpu: system_times(),
        process_ns: process_ns(),
        cores: active_cores(),
        mem_available: available_memory(),
        threads_free: None,
        link_bps: link_bps(),
        disks: Vec::new(),
    }
}

/// No block device is read on Windows yet.
pub(crate) fn device_id_of(_path: &Path) -> Option<String> {
    None
}

pub(crate) fn device_name_of(_id: &str) -> Option<String> {
    None
}

fn filetime_ticks(time: &windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn zero_filetime() -> windows_sys::Win32::Foundation::FILETIME {
    windows_sys::Win32::Foundation::FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    }
}

/// `GetSystemTimes`: the kernel time includes the idle time.
fn system_times() -> Option<CpuTicks> {
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    let (mut idle, mut kernel, mut user) = (zero_filetime(), zero_filetime(), zero_filetime());
    // SAFETY: three valid, writable FILETIME values.
    let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
    if ok == 0 {
        return None;
    }
    let total = filetime_ticks(&kernel).saturating_add(filetime_ticks(&user));
    Some(CpuTicks {
        busy: total.saturating_sub(filetime_ticks(&idle)),
        total,
    })
}

/// `GetProcessTimes`: kernel plus user time of this process in nanoseconds
/// (the API counts 100 ns steps).
fn process_ns() -> Option<u64> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut created, mut exited) = (zero_filetime(), zero_filetime());
    let (mut kernel, mut user) = (zero_filetime(), zero_filetime());
    // SAFETY: the pseudo handle of this process and four writable FILETIMEs.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    (ok != 0).then(|| {
        filetime_ticks(&kernel)
            .saturating_add(filetime_ticks(&user))
            .saturating_mul(100)
    })
}

/// Logical processors of all processor groups.
fn active_cores() -> Option<f64> {
    use windows_sys::Win32::System::Threading::GetActiveProcessorCount;
    const ALL_PROCESSOR_GROUPS: u16 = 0xffff;
    // SAFETY: only reads a system constant.
    let count = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
    (count > 0).then_some(f64::from(count))
}

/// Transmit link speed (bytes per second) of the up adapter with a gateway
/// and the lowest IPv4 metric: the one the default route uses.
fn link_bps() -> Option<u64> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    const AF_UNSPEC: u32 = 0;
    const SKIP_ANYCAST_MULTICAST_DNS: u32 = 0x0002 | 0x0004 | 0x0008;
    const INCLUDE_GATEWAYS: u32 = 0x0080;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
    let mut size: u32 = 16 * 1024;
    for _ in 0..4 {
        let mut buffer = vec![0u8; size as usize];
        // SAFETY: the buffer is `size` bytes; the call updates `size` when it
        // needs more room and the pointer stays valid for the whole call.
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC,
                SKIP_ANYCAST_MULTICAST_DNS | INCLUDE_GATEWAYS,
                std::ptr::null(),
                buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut size,
            )
        };
        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if result != 0 {
            return None;
        }
        let mut best: Option<(u32, u64)> = None;
        let mut adapter = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        // SAFETY: on success the buffer holds a linked list of adapter
        // records written by the API within `size` bytes.
        while let Some(record) = unsafe { adapter.as_ref() } {
            let usable = record.OperStatus == IfOperStatusUp
                && record.IfType != IF_TYPE_SOFTWARE_LOOPBACK
                && !record.FirstGatewayAddress.is_null();
            let speed = record.TransmitLinkSpeed;
            let known = speed != 0 && speed != u64::MAX;
            if usable && known && best.is_none_or(|(metric, _)| record.Ipv4Metric < metric) {
                best = Some((record.Ipv4Metric, speed));
            }
            adapter = record.Next;
        }
        return best.map(|(_, bits)| bits / 8);
    }
    None
}

/// The folder `path` resolves to, links and junctions on the way followed,
/// in its plain form (`C:\…`, `\\server\share\…`) where that names the same
/// folder; otherwise in the verbatim form `canonicalize` returns.
pub(crate) fn canonical_folder(path: &Path) -> std::io::Result<PathBuf> {
    let resolved = std::fs::canonicalize(path)?;
    let plain = plain_form(&resolved.to_string_lossy());
    Ok(plain.map_or(resolved, PathBuf::from))
}

/// The plain form of a verbatim path when Win32 reads it the same: not too
/// long, no name ending in a dot or space, no device name.
fn plain_form(verbatim: &str) -> Option<String> {
    let plain = match verbatim.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => verbatim.strip_prefix(r"\\?\")?.to_string(),
    };
    let same = plain.len() < 260
        && plain
            .split('\\')
            .filter(|name| !name.is_empty())
            .skip(1)
            .all(|name| !name.ends_with(['.', ' ']) && !is_device_name(name));
    same.then_some(plain)
}

fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// Writes all of `buf` at `offset` (`seek_write` in a loop; it moves the file
/// pointer on Windows, so a later single stream seeks first), so several
/// writers can fill different ranges of one file at once.
pub(crate) fn write_all_at(
    file: &std::fs::File,
    mut buf: &[u8],
    mut offset: u64,
) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_write(buf, offset) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write whole buffer",
                ))
            }
            Ok(written) => {
                buf = &buf[written..];
                offset += written as u64;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{plain_form, write_all_at};
    use std::io::Read;

    #[test]
    fn speed_task_write_all_at_positions_data() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("ranges.bin");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create");
        file.set_len(10).expect("length");
        write_all_at(&file, b"abc", 4).expect("later range first");
        write_all_at(&file, b"xy", 0).expect("earlier range second");
        drop(file);
        let mut content = Vec::new();
        std::fs::File::open(&path)
            .expect("open")
            .read_to_end(&mut content)
            .expect("read");
        assert_eq!(content, b"xy\0\0abc\0\0\0");
    }

    #[test]
    fn transfer_engine_task_canonical_folders_drop_the_verbatim_prefix_only_when_safe() {
        assert_eq!(
            plain_form(r"\\?\C:\Users\me\Downloads").as_deref(),
            Some(r"C:\Users\me\Downloads")
        );
        assert_eq!(
            plain_form(r"\\?\UNC\server\share\dir").as_deref(),
            Some(r"\\server\share\dir")
        );
        assert_eq!(plain_form(r"\\?\C:\data\nul.txt"), None, "a device name");
        assert_eq!(
            plain_form(r"\\?\C:\data\trailing."),
            None,
            "Win32 strips it"
        );
        assert_eq!(plain_form(&format!(r"\\?\C:\{}", "a".repeat(300))), None);
        assert_eq!(plain_form(r"C:\already\plain"), None);
    }
}
