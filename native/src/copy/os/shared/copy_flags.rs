//! The flags of the Windows kernel copy (`CopyFile2`) and the rules that
//! choose them. Pure: no Windows imports, so the rules are checked on every
//! host; only `windows.rs` hands the result to the system.
// Only `windows.rs` uses it outside the tests; other hosts check the logic.
#![cfg_attr(not(windows), allow(dead_code))]

/// Never replace the stage name (it is created exclusively).
pub(crate) const COPY_FILE_FAIL_IF_EXISTS: u32 = 0x0000_0001;
/// A source swapped for a link is copied as a link, which the reparse check
/// of the stage then refuses.
pub(crate) const COPY_FILE_COPY_SYMLINK: u32 = 0x0000_0800;
/// Bypasses the system cache. Microsoft recommends it for very large copies.
pub(crate) const COPY_FILE_NO_BUFFERING: u32 = 0x0000_1000;
/// Leaves the destination size unreserved before the copy (Windows 10 build
/// 19041 and later; an older system refuses it as an invalid argument, which
/// `windows_kernel_copy.rs` answers by asking again without it). Value and
/// version: Microsoft Learn `COPYFILE2_EXTENDED_PARAMETERS`; the same value
/// as windows-sys 0.59 `COPY_FILE_DISABLE_PRE_ALLOCATION`.
pub(crate) const COPY_FILE_DISABLE_PRE_ALLOCATION: u32 = 0x0400_0000;
/// Asks SMB to compress the transfer (Windows ignores it where it cannot).
pub(crate) const COPY_FILE_REQUEST_COMPRESSED_TRAFFIC: u32 = 0x1000_0000;
/// Stays `false`: `CopyFile2` does not tell whether the SMB connection is
/// encrypted, and compressing before encrypting is the case security answer
/// E13-S11 rules out. No setting in any surface switches it.
pub(crate) const WINDOWS_REQUEST_COMPRESSED_TRAFFIC: bool = false;

/// A file at least this part of the available memory bypasses the cache:
/// such a file would push everything else out of it. Reason of the divisor
/// 4: the budget of the transfer engine is also a quarter of the available
/// memory, so one cached file never claims more than one engine budget.
/// (The engine's `local_plan::UNBUFFERED_DIVISOR` repeats it for the Linux
/// copy; the Windows copy has one more, fixed, threshold below.)
const NO_BUFFERING_DIVISOR: u64 = 4;
/// A file of at least this size bypasses the cache whatever the memory:
/// Microsoft recommends `COPY_FILE_NO_BUFFERING` "for very large file
/// copies" (Learn `COPYFILE2_EXTENDED_PARAMETERS`, `CopyFileEx`), `robocopy
/// /J` is meant for large files, and BetterCopy (`engine.rs:84-102`) uses
/// this threshold. Either rule is enough.
const NO_BUFFERING_FROM: u64 = 256 * 1024 * 1024;
/// A file of at most this size is copied without reserving its size first:
/// one call less per file where the reservation buys no contiguity. The
/// limit of the local packets' small files (`local_packet::SMALL_FILE`).
const DISABLE_PRE_ALLOCATION_UP_TO: u64 = 1024 * 1024;

/// Whether `path` names a UNC share: it starts with two separators, but the
/// verbatim local prefix (`\\?\C:\x`, `\\.\C:\x`) is no share; only
/// `\\?\UNC\server\share` is.
pub(crate) fn is_unc(path: &str) -> bool {
    let text = path.replace('/', "\\");
    let Some(rest) = text.strip_prefix("\\\\") else {
        return false;
    };
    match rest
        .strip_prefix("?\\")
        .or_else(|| rest.strip_prefix(".\\"))
    {
        Some(after) => after
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("UNC\\")),
        None => !rest.is_empty(),
    }
}

/// `dwCopyFlags` of one copy: always `FAIL_IF_EXISTS | COPY_SYMLINK`; plus
/// `NO_BUFFERING` when the file is at least 256 MiB or at least a quarter of
/// the available memory (`None` = unknown: that rule never applies); plus
/// `DISABLE_PRE_ALLOCATION` for a file of at most 1 MiB; plus
/// `REQUEST_COMPRESSED_TRAFFIC` only when a share is involved and the switch
/// is on.
pub(crate) fn windows_copy_flags(
    size: u64,
    available_memory: Option<u64>,
    unc: bool,
    compressed_traffic: bool,
) -> u32 {
    let mut flags = COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_COPY_SYMLINK;
    if size >= NO_BUFFERING_FROM
        || available_memory.is_some_and(|memory| size >= memory / NO_BUFFERING_DIVISOR)
    {
        flags |= COPY_FILE_NO_BUFFERING;
    }
    if size <= DISABLE_PRE_ALLOCATION_UP_TO {
        flags |= COPY_FILE_DISABLE_PRE_ALLOCATION;
    }
    if unc && compressed_traffic {
        flags |= COPY_FILE_REQUEST_COMPRESSED_TRAFFIC;
    }
    flags
}

#[cfg(test)]
#[path = "copy_flags_tests.rs"]
mod tests;
