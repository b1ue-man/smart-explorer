//! The error an application sees for an engine error on a FUSE drive
//! (DRV-M4, table of strand S2 C4). The mapping goes only by `ErrorKind`:
//! errors that crossed the daemon's mount proxy carry no OS code, and a raw
//! code of a local helper must not leak a meaning the operation never had.
//! A conflict with a remote change is `WouldBlock` in the engine and reads
//! as `EBUSY`; every kind the table does not name reads as `EIO`.
use std::ffi::OsStr;
use std::io;

use fuser::Errno;

/// `errno` of an engine error for a FUSE reply.
pub(crate) fn errno(error: &io::Error) -> Errno {
    Errno::from_i32(errno_for(error))
}

/// A name the kernel passed: Unicode only (every backend stores text).
pub(crate) fn name_text(name: &OsStr) -> Result<&str, Errno> {
    name.to_str().ok_or(Errno::EINVAL)
}

pub(crate) fn errno_for(error: &io::Error) -> i32 {
    errno_for_kind(error.kind())
}

pub(crate) fn errno_for_kind(kind: io::ErrorKind) -> i32 {
    use io::ErrorKind as Kind;
    match kind {
        Kind::NotFound => libc::ENOENT,
        Kind::NotADirectory => libc::ENOTDIR,
        Kind::IsADirectory => libc::EISDIR,
        Kind::DirectoryNotEmpty => libc::ENOTEMPTY,
        Kind::PermissionDenied => libc::EACCES,
        Kind::ReadOnlyFilesystem => libc::EROFS,
        Kind::AlreadyExists => libc::EEXIST,
        Kind::InvalidInput | Kind::InvalidData => libc::EINVAL,
        Kind::TimedOut => libc::ETIMEDOUT,
        Kind::StorageFull | Kind::WriteZero => libc::ENOSPC,
        Kind::Unsupported => libc::ENOTSUP,
        Kind::OutOfMemory => libc::ENOMEM,
        Kind::WouldBlock => libc::EBUSY,
        Kind::Interrupted => libc::EINTR,
        Kind::CrossesDevices => libc::EXDEV,
        Kind::ConnectionRefused => libc::ECONNREFUSED,
        Kind::ConnectionReset => libc::ECONNRESET,
        Kind::NotConnected => libc::ENOTCONN,
        Kind::NetworkDown => libc::ENETDOWN,
        Kind::NetworkUnreachable => libc::ENETUNREACH,
        Kind::HostUnreachable => libc::EHOSTUNREACH,
        _ => libc::EIO,
    }
}

#[cfg(test)]
mod tests {
    use super::errno_for_kind;
    use std::io::ErrorKind as Kind;

    #[test]
    fn fuse_errno_table_of_strand_c4() {
        let table = [
            (Kind::NotFound, libc::ENOENT),
            (Kind::NotADirectory, libc::ENOTDIR),
            (Kind::IsADirectory, libc::EISDIR),
            (Kind::DirectoryNotEmpty, libc::ENOTEMPTY),
            (Kind::PermissionDenied, libc::EACCES),
            (Kind::ReadOnlyFilesystem, libc::EROFS),
            (Kind::AlreadyExists, libc::EEXIST),
            (Kind::InvalidInput, libc::EINVAL),
            (Kind::InvalidData, libc::EINVAL),
            (Kind::TimedOut, libc::ETIMEDOUT),
            (Kind::StorageFull, libc::ENOSPC),
            (Kind::WriteZero, libc::ENOSPC),
            (Kind::Unsupported, libc::ENOTSUP),
            (Kind::OutOfMemory, libc::ENOMEM),
            (Kind::WouldBlock, libc::EBUSY),
            (Kind::Interrupted, libc::EINTR),
            (Kind::CrossesDevices, libc::EXDEV),
            (Kind::ConnectionRefused, libc::ECONNREFUSED),
            (Kind::ConnectionReset, libc::ECONNRESET),
            (Kind::NotConnected, libc::ENOTCONN),
            (Kind::NetworkDown, libc::ENETDOWN),
            (Kind::NetworkUnreachable, libc::ENETUNREACH),
            (Kind::HostUnreachable, libc::EHOSTUNREACH),
            (Kind::Other, libc::EIO),
            (Kind::BrokenPipe, libc::EIO),
            (Kind::UnexpectedEof, libc::EIO),
        ];
        for (kind, errno) in table {
            assert_eq!(errno_for_kind(kind), errno, "{kind:?}");
        }
    }
}
