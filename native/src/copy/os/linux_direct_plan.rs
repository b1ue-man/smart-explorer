//! The pure decisions of the unbuffered local copy (`linux_direct`): which
//! file goes around the page cache, how it is cut into the part that is read
//! and written with `O_DIRECT` and the short rest that is not, and which
//! errors mean "this file system does not take direct I/O".
use std::io;

pub(super) const MIB: u64 = 1024 * 1024;

/// Bytes per direct request. The kernel splits one request into the
/// device's own request size (typically 0.5 to 1.25 MiB) and issues the
/// pieces at once, so one 4 MiB read or write keeps several requests of the
/// drive busy by itself.
pub(super) const CHUNK: usize = 4 * 1024 * 1024;
/// Threads of an unbuffered copy, the one that holds the operation's permit
/// included. Every thread has one request of `CHUNK` in flight (it reads, then
/// writes), so 4 threads keep 16 MiB in flight: the depth of the `fio
/// --iodepth=16 --bs=1M` run that measured the drives' capacity (suite run 4:
/// read 425, write 271, mixed 407 MB/s). More threads would only queue
/// behind the same drive.
pub(super) const THREADS: usize = 4;
/// Address alignment of the buffers: the largest memory alignment that
/// `STATX_DIOALIGN` or the block layer asks of a user buffer in practice.
pub(super) const BUFFER_ALIGN: usize = 4096;
/// The logical block size without a report of the kernel (`statx` knows
/// `STATX_DIOALIGN` since Linux 6.1; Android and old kernels do not).
pub(super) const DEFAULT_UNIT: u64 = 4096;
/// Smallest unit ever used (the smallest logical block of a drive).
const MIN_UNIT: u64 = 512;
/// A drive that wants larger units than this is not copied directly: the
/// request borders (whole mebibytes) must be multiples of the unit.
const MAX_UNIT: u64 = 64 * 1024;
/// A file at least this part of the available memory goes around the page
/// cache, the rule of Windows' `COPY_FILE_NO_BUFFERING`
/// (`copy_flags::NO_BUFFERING_DIVISOR`; the engine's `local_plan` repeats it):
/// such a file pushes everything else out of the cache and runs into the
/// dirty limits, which hold the buffered copy at one drive's readahead.
pub(super) const MEMORY_DIVISOR: u64 = 4;

/// Whether a file of `size` bytes is copied around the page cache:
/// `size >= available / 4`; unknown memory (`/proc/meminfo` unreadable): never.
pub(super) fn wanted(size: u64, available_memory: Option<u64>) -> bool {
    size > 0 && available_memory.is_some_and(|memory| size >= memory / MEMORY_DIVISOR)
}

/// The major number of an `st_dev` (the glibc encoding).
fn major(dev: u64) -> u64 {
    ((dev >> 8) & 0xfff) | ((dev >> 32) & 0xffff_f000)
}

/// Whether the file system with this `st_dev` sits on a block device: network,
/// FUSE, tmpfs and the anonymous devices of btrfs have major number 0.
pub(super) fn block_backed(dev: u64) -> bool {
    major(dev) != 0
}

/// What `STATX_DIOALIGN` says of one file: (buffer address alignment, offset
/// and length alignment). `None` = the kernel does not say.
pub(super) type DioReport = Option<(u32, u32)>;

/// The unit of the direct part from the reports of the source and the
/// stage: every request offset and length is a multiple of it, the buffers
/// are aligned to `BUFFER_ALIGN`. `None` = direct I/O is not possible: a file
/// reports 0 (it takes none), a larger buffer alignment than the buffers
/// have, or a unit beyond `MAX_UNIT`. Without a report the unit is 4096, which
/// every logical block size in use divides.
pub(super) fn unit_of(reports: [DioReport; 2]) -> Option<u64> {
    let mut unit = MIN_UNIT;
    for report in reports {
        let Some((memory, offset)) = report else {
            unit = unit.max(DEFAULT_UNIT);
            continue;
        };
        if memory == 0 || offset == 0 || u64::from(memory) > BUFFER_ALIGN as u64 {
            return None;
        }
        let offset = u64::from(offset);
        if !offset.is_power_of_two() || offset > MAX_UNIT {
            return None;
        }
        unit = unit.max(offset);
    }
    Some(unit)
}

/// The file cut into the part copied directly (a multiple of `unit`) and the
/// rest (shorter than `unit`) that goes through the normal handles.
pub(super) fn split_main(size: u64, unit: u64) -> (u64, u64) {
    let rest = size % unit.max(1);
    (size - rest, rest)
}

/// Errors of the first request that say "this file system or this kernel
/// takes no direct I/O here" (tmpfs before Linux 6.6, ecryptfs, an encrypted
/// Android file without inline encryption, a seccomp filter): the file is then
/// copied the older way. Anything else (a real read or write error) is the
/// copy's own error.
pub(super) fn is_unsupported(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(
            libc::EINVAL
                | libc::EOPNOTSUPP
                | libc::ENOSYS
                | libc::EBADF
                | libc::ENOTTY
                | libc::EPERM
        )
    )
}
