//! NTFS-like resolution of the `MAXIMUM_ALLOWED` desired-access flag.
//!
//! Windows callers (Explorer prominently) open files with `MAXIMUM_ALLOWED`
//! and expect the filesystem to grant the largest access that the volume and
//! the current sharing state permit — a read-only grant on write-protected
//! media rather than a failure, and a degraded grant instead of a sharing
//! violation when another handle blocks write or delete sharing. Keeping the
//! raw flag in a handle record would instead make every such open demand
//! read+write+delete, which both denies the open on read-only mounts and
//! poisons later share-compatibility checks against the stored access mask.
//!
//! The helpers here are pure so the policy is testable on every platform; the
//! Windows handle table applies them while it holds its state lock.

pub(crate) const FILE_READ_DATA: u32 = 0x0000_0001;
pub(crate) const FILE_WRITE_DATA: u32 = 0x0000_0002;
pub(crate) const FILE_APPEND_DATA: u32 = 0x0000_0004;
pub(crate) const FILE_READ_EA: u32 = 0x0000_0008;
pub(crate) const FILE_WRITE_EA: u32 = 0x0000_0010;
pub(crate) const FILE_EXECUTE: u32 = 0x0000_0020;
pub(crate) const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
pub(crate) const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
pub(crate) const DELETE: u32 = 0x0001_0000;
pub(crate) const READ_CONTROL: u32 = 0x0002_0000;
pub(crate) const SYNCHRONIZE: u32 = 0x0010_0000;
pub(crate) const MAXIMUM_ALLOWED: u32 = 0x0200_0000;

pub(crate) const fn requests_maximum_allowed(desired_access: u32) -> bool {
    desired_access & MAXIMUM_ALLOWED != 0
}

/// The read-only grant every mount can offer for `MAXIMUM_ALLOWED`: the flag
/// is replaced by the concrete read/execute/attribute rights.
pub(crate) const fn maximum_allowed_read_grant(desired_access: u32) -> u32 {
    (desired_access & !MAXIMUM_ALLOWED)
        | FILE_READ_DATA
        | FILE_READ_EA
        | FILE_READ_ATTRIBUTES
        | FILE_EXECUTE
        | READ_CONTROL
        | SYNCHRONIZE
}

/// The full grant a writable mount offers for `MAXIMUM_ALLOWED` when sharing
/// state permits it: the read grant plus concrete write and delete rights.
pub(crate) const fn maximum_allowed_full_grant(desired_access: u32) -> u32 {
    maximum_allowed_read_grant(desired_access)
        | FILE_WRITE_DATA
        | FILE_APPEND_DATA
        | FILE_WRITE_EA
        | FILE_WRITE_ATTRIBUTES
        | DELETE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_maximum_allowed_read_grant_carries_no_write_or_delete() {
        let granted = maximum_allowed_read_grant(MAXIMUM_ALLOWED);
        assert_eq!(granted & MAXIMUM_ALLOWED, 0);
        assert_eq!(
            granted & (FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES | DELETE),
            0
        );
        assert_ne!(granted & FILE_READ_DATA, 0);
        assert_ne!(granted & FILE_READ_ATTRIBUTES, 0);
        assert_ne!(granted & SYNCHRONIZE, 0);
    }

    #[test]
    fn remote_drive_task_maximum_allowed_full_grant_adds_write_and_delete() {
        let granted = maximum_allowed_full_grant(MAXIMUM_ALLOWED | SYNCHRONIZE);
        assert_eq!(granted & MAXIMUM_ALLOWED, 0);
        assert_ne!(granted & FILE_READ_DATA, 0);
        assert_ne!(granted & FILE_WRITE_DATA, 0);
        assert_ne!(granted & FILE_APPEND_DATA, 0);
        assert_ne!(granted & DELETE, 0);
    }

    #[test]
    fn remote_drive_task_explicit_access_is_never_rewritten() {
        for desired in [
            0,
            FILE_READ_DATA | SYNCHRONIZE,
            FILE_WRITE_DATA | FILE_READ_ATTRIBUTES,
            DELETE,
        ] {
            assert!(!requests_maximum_allowed(desired));
        }
        assert!(requests_maximum_allowed(MAXIMUM_ALLOWED | FILE_READ_DATA));
        // Explicit rights that accompany MAXIMUM_ALLOWED survive resolution.
        let granted = maximum_allowed_read_grant(MAXIMUM_ALLOWED | FILE_WRITE_DATA);
        assert_ne!(granted & FILE_WRITE_DATA, 0);
    }
}
