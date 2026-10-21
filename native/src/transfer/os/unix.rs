//! Unix (Linux, Android) filesystem facts for transfers, the system sensors
//! (`/proc`, cgroup v2, `/sys`) and the thread priority. A source that is
//! missing or forbidden (Android refuses apps `/proc/stat`) reads as `None`.
use super::sensor_parse::{
    find_disk, parse_cpu_max, parse_default_route, parse_link_speed, parse_meminfo_available,
    parse_pids, parse_proc_stat, parse_process_ticks, RawSample,
};
use std::path::{Path, PathBuf};

/// Niceness of worker threads: they yield the cores to the foreground at
/// once (Android: its `THREAD_PRIORITY_BACKGROUND`).
#[cfg(target_os = "android")]
const WORKER_NICE: libc::c_int = 10;
#[cfg(not(target_os = "android"))]
const WORKER_NICE: libc::c_int = 5;

/// Uploads never follow links: a symlink source is refused, not resolved.
pub(crate) fn upload_is_link_like(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// `\` is an ordinary character in Unix file names.
pub(crate) fn backslash_is_name_char() -> bool {
    true
}

pub(crate) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(src, dest)
}

/// The folder `path` resolves to, links on the way followed.
pub(crate) fn canonical_folder(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// No free-space probe here; the download preflight is then skipped.
pub(crate) fn available_space_for_path(_path: &Path) -> Option<u64> {
    None
}

/// Memory currently available to programs (`MemAvailable` on Linux and
/// Android), for the transfer budget.
pub(crate) fn available_memory() -> Option<u64> {
    parse_meminfo_available(&read_text("/proc/meminfo")?)
}

fn read_text(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Total physical memory (`MemTotal` on Linux and Android). Stable from run to
/// run, unlike the available memory, so limits derived from it do not change
/// with the momentary load (sync tree limits, RV1).
pub(crate) fn physical_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Writes all of `buf` at `offset` without moving the file pointer, so
/// several writers can fill different ranges of one file at once.
pub(crate) fn write_all_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, buf, offset)
}

/// Lowers the niceness of the calling thread to `WORKER_NICE`, never below
/// what it has (a process started with a higher niceness keeps it). Errors
/// are ignored: the priority is a courtesy, not a requirement.
pub(crate) fn lower_thread_priority() {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // SAFETY: plain system calls about the calling thread's own id.
        unsafe {
            let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
            if libc::getpriority(libc::PRIO_PROCESS, tid) < WORKER_NICE {
                let _ = libc::setpriority(libc::PRIO_PROCESS, tid, WORKER_NICE);
            }
        }
    }
}

/// The niceness of the calling thread (tests).
#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
fn thread_niceness() -> libc::c_int {
    // SAFETY: plain system calls about the calling thread's own id.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::getpriority(libc::PRIO_PROCESS, tid)
    }
}

/// One reading of everything the sensors look at; `watch` names the block
/// devices (`major:minor`) whose queue is wanted.
pub(crate) fn read_raw(watch: &[String]) -> RawSample {
    let cores = std::thread::available_parallelism().ok().map(|n| {
        let cores = n.get();
        match cgroup_file("cpu.max") {
            Some(text) => parse_cpu_max(&text, cores),
            None => cores as f64,
        }
    });
    RawSample {
        cpu: read_text("/proc/stat").and_then(|text| parse_proc_stat(&text)),
        process_ns: process_ns(),
        cores,
        mem_available: available_memory(),
        threads_free: threads_free(),
        link_bps: link_bps(),
        disks: disks(watch),
    }
}

/// CPU time of this process in nanoseconds (`utime + stime` of
/// `/proc/self/stat` in clock ticks).
fn process_ns() -> Option<u64> {
    let ticks = parse_process_ticks(&read_text("/proc/self/stat")?)?;
    // SAFETY: sysconf only reads a system constant.
    let per_second = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) })
        .ok()
        .filter(|rate| *rate > 0)?;
    Some(ticks.saturating_mul(1_000_000_000 / per_second))
}

/// A file of the cgroup v2 this process runs in.
fn cgroup_file(name: &str) -> Option<String> {
    let membership = read_text("/proc/self/cgroup")?;
    let relative = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    read_text(&format!("/sys/fs/cgroup{}/{name}", relative.trim_end()))
}

fn threads_free() -> Option<u64> {
    parse_pids(&cgroup_file("pids.max")?, &cgroup_file("pids.current")?)
}

fn link_bps() -> Option<u64> {
    let interface = parse_default_route(&read_text("/proc/net/route")?)?;
    // An interface name never holds a path separator; refuse one that does.
    if interface.contains('/') {
        return None;
    }
    parse_link_speed(&read_text(&format!("/sys/class/net/{interface}/speed"))?)
}

fn disks(watch: &[String]) -> Vec<(String, super::sensor_parse::DiskTicks)> {
    if watch.is_empty() {
        return Vec::new();
    }
    let Some(text) = read_text("/proc/diskstats") else {
        return Vec::new();
    };
    watch
        .iter()
        .filter_map(|id| find_disk(&text, id).map(|(_, ticks)| (id.clone(), ticks)))
        .collect()
}

/// The block device (`major:minor`) holding `path`; `None` where the kernel
/// has no device behind it (network and virtual file systems).
pub(crate) fn device_id_of(path: &Path) -> Option<String> {
    let dev = std::os::unix::fs::MetadataExt::dev(&std::fs::metadata(path).ok()?);
    let (major, minor) = major_minor(dev);
    (major != 0).then(|| format!("{major}:{minor}"))
}

/// Major and minor number of a `st_dev` (the glibc encoding).
fn major_minor(dev: u64) -> (u64, u64) {
    (
        ((dev >> 8) & 0xfff) | ((dev >> 32) & 0xffff_f000),
        (dev & 0xff) | ((dev >> 12) & 0xffff_ff00),
    )
}

/// The `st_dev` of the block device under `path` (a file that does not exist
/// yet is looked up through its folder); `None` where no block device is
/// behind it, as on network, FUSE and other virtual file systems.
fn block_device_number(path: &Path) -> Option<u64> {
    let dev_of = |path: &Path| {
        let dev = std::os::unix::fs::MetadataExt::dev(&std::fs::metadata(path).ok()?);
        (major_minor(dev).0 != 0).then_some(dev)
    };
    dev_of(path).or_else(|| dev_of(path.parent()?))
}

/// Which pair of devices a local copy from `source` to `target` runs on:
/// the `st_dev` of each side (the same number twice on one device). `None`
/// when either side has no block device behind it; a copy measured for a pair
/// (`engine::local`) is only ever one between real devices, never a network or
/// virtual file system whose writes at scattered offsets may not be allowed or
/// may cost more than they gain.
pub(crate) fn copy_device_pair(source: &Path, target: &Path) -> Option<(u64, u64)> {
    Some((block_device_number(source)?, block_device_number(target)?))
}

/// Whether the drive under `path` spins (`Some(true)`), is flash
/// (`Some(false)`) or cannot be told (`None`: no block device behind it,
/// such as a network or virtual file system, or `/sys` is not readable).
/// A file that does not exist yet is looked up through its folder.
pub(crate) fn volume_is_rotational(path: &Path) -> Option<bool> {
    let id = device_id_of(path).or_else(|| device_id_of(path.parent()?))?;
    rotational_in(Path::new("/sys/dev/block"), &id)
}

/// `queue/rotational` of the block device `id` (`major:minor`) below the
/// device directory `root`; a partition has none of its own, so the one of
/// its parent device counts.
pub(crate) fn rotational_in(root: &Path, id: &str) -> Option<bool> {
    let device = root.join(id);
    let queue = if device.join("partition").exists() {
        device.join("../queue/rotational")
    } else {
        device.join("queue/rotational")
    };
    match std::fs::read_to_string(queue).ok()?.trim() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// The kernel's name of a device (`sda1`), for the texts.
pub(crate) fn device_name_of(id: &str) -> Option<String> {
    find_disk(&read_text("/proc/diskstats")?, id).map(|(name, _)| name)
}

#[cfg(test)]
mod tests {
    use super::write_all_at;
    use std::io::Read;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn speed_task_priority_only_lowers() {
        // a fresh thread starts at the process's niceness
        let (start, lowered, again) = std::thread::spawn(|| {
            let start = super::thread_niceness();
            super::lower_thread_priority();
            let lowered = super::thread_niceness();
            super::lower_thread_priority();
            (start, lowered, super::thread_niceness())
        })
        .join()
        .expect("priority thread");
        assert!(lowered >= start, "never raised above the start value");
        assert!(lowered >= start.min(super::WORKER_NICE));
        assert_eq!(lowered, start.max(super::WORKER_NICE));
        assert_eq!(again, lowered, "a second call changes nothing");
    }

    #[test]
    fn speed_task_sensor_unix_reads_what_it_can_and_nothing_else() {
        let raw = super::read_raw(&["0:0".to_string()]);
        // whatever the kernel hides stays None; what is there is plausible
        if let Some(cores) = raw.cores {
            assert!(cores > 0.0);
        }
        if let Some(mem) = raw.mem_available {
            assert!(mem > 0);
        }
        assert!(super::device_id_of(std::path::Path::new("/nonexistent/x")).is_none());
        assert!(super::device_name_of("999999:999999").is_none());
    }

    #[test]
    fn speed_task_local_rotational_unknown_is_not_parallel() {
        // What the kernel does not tell is `None`, which the engine treats
        // as "not flash": no parallel ranges (`local::parallel_allowed`).
        let root = tempfile::tempdir().expect("temp dir");
        assert_eq!(super::rotational_in(root.path(), "8:0"), None, "no device");
        let disk = root.path().join("8:0/queue");
        std::fs::create_dir_all(&disk).expect("device folder");
        std::fs::write(disk.join("rotational"), "garbage\n").expect("write");
        assert_eq!(super::rotational_in(root.path(), "8:0"), None, "bad value");
        std::fs::write(disk.join("rotational"), "0\n").expect("write");
        assert_eq!(super::rotational_in(root.path(), "8:0"), Some(false));
        std::fs::write(disk.join("rotational"), "1\n").expect("write");
        assert_eq!(super::rotational_in(root.path(), "8:0"), Some(true));
        // A partition reads its parent device's file (`<id>/../queue`).
        let parent = root.path().join("parent");
        std::fs::create_dir_all(parent.join("queue")).expect("parent");
        std::fs::write(parent.join("queue/rotational"), "0\n").expect("write");
        std::fs::create_dir_all(parent.join("sda1")).expect("partition");
        std::fs::write(parent.join("sda1/partition"), "1\n").expect("write");
        let link = root.path().join("8:1");
        std::os::unix::fs::symlink(parent.join("sda1"), &link).expect("symlink");
        assert_eq!(super::rotational_in(root.path(), "8:1"), Some(false));
        // No block device behind the path (or a missing one): unknown.
        assert_eq!(
            super::volume_is_rotational(std::path::Path::new("/nonexistent/dir/x")),
            None
        );
    }

    #[test]
    fn speed_task_local_device_pair_is_a_pair_of_block_devices_or_nothing() {
        // The glibc encoding: major 8, minor 1 is 0x801; a major above 4095
        // (bit 44 up) and a minor above 255 (bit 20 up) spill into the upper bits.
        assert_eq!(super::major_minor(0x801), (8, 1));
        assert_eq!(super::major_minor((259 << 8) | 3), (259, 3));
        assert_eq!(
            super::major_minor((1 << 44) | (0x12 << 8)),
            (4096 + 0x12, 0)
        );
        assert_eq!(super::major_minor((1 << 20) | 3), (0, 0x103));
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("a.bin");
        std::fs::write(&file, b"x").expect("file");
        // Whatever holds the temp folder, both sides of one folder agree, a
        // file that does not exist yet is looked up through its folder, and a
        // missing folder has no device.
        let pair = super::copy_device_pair(&file, &dir.path().join("new.bin"));
        if let Some((one, other)) = pair {
            assert_eq!(one, other, "one folder is one device");
        }
        assert_eq!(
            super::copy_device_pair(&file, &dir.path().join("new.bin")),
            super::copy_device_pair(&file, &file)
        );
        assert_eq!(
            super::copy_device_pair(&file, std::path::Path::new("/nonexistent/dir/x")),
            None
        );
    }

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
}
