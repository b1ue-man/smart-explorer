//! Remove one entry from the held parent, never via an old root spelling.
use std::{ffi::{CString, OsStr}, io, os::unix::{ffi::OsStrExt, fs::MetadataExt, io::AsRawFd}};
use super::{validate_name, DirectoryHandle};

impl DirectoryHandle {
    pub(crate) fn open_child_for_delete(&self, name: &OsStr) -> io::Result<Self> {
        self.open_child(name)
    }

    /// Non-directory entries, including links, are unlinked without following
    /// their target. A concurrent directory replacement is never recursed into.
    pub(crate) fn remove_child(&self, name: &OsStr) -> io::Result<()> {
        validate_name(name)?;
        if self.metadata_at(name)?.is_dir() {
            return Err(changed());
        }
        unlink(self, name, 0)
    }

    /// The opened object and the final parent entry must still agree. Linux
    /// unlinkat removes only this final entry, even if an ancestor was renamed.
    pub(crate) fn remove_empty_child(&self, name: &OsStr, expected: Self) -> io::Result<()> {
        validate_name(name)?;
        let observed = self.metadata_at(name)?;
        let held = expected.metadata()?;
        if !observed.is_dir() || observed.dev() != held.dev() || observed.ino() != held.ino() {
            return Err(changed());
        }
        unlink(self, name, libc::AT_REMOVEDIR)
    }
}

fn unlink(parent: &DirectoryHandle, name: &OsStr, flags: i32) -> io::Result<()> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry name contains NUL"))?;
    // SAFETY: parent and the single-component C string remain live. unlinkat
    // does not traverse the final entry, including when it is a link.
    if unsafe { libc::unlinkat(parent.file.as_raw_fd(), name.as_ptr(), flags) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
fn changed() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "opened delete entry changed")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_host_delete_root_swap_never_uses_old_path_spelling() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("root");
        let outside = fixture.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(root.join("victim"), b"selected").unwrap();
        std::fs::write(outside.join("victim"), b"outside").unwrap();
        let held = DirectoryHandle::open_root(&root).unwrap();
        std::fs::rename(&root, fixture.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(&outside, &root).unwrap();
        held.remove_child(OsStr::new("victim")).unwrap();
        assert_eq!(std::fs::read(outside.join("victim")).unwrap(), b"outside");
        assert!(!fixture.path().join("moved/victim").exists());
    }
    #[test]
    fn review_task_host_delete_child_symlink_is_a_nonrecursive_leaf() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::create_dir(fixture.path().join("outside")).unwrap();
        std::fs::write(fixture.path().join("outside/kept"), b"kept").unwrap();
        std::os::unix::fs::symlink(fixture.path().join("outside"), fixture.path().join("link")).unwrap();
        let parent = DirectoryHandle::open_root(fixture.path()).unwrap();
        assert!(parent.open_child_for_delete(OsStr::new("link")).is_err());
        parent.remove_child(OsStr::new("link")).unwrap();
        assert_eq!(std::fs::read(fixture.path().join("outside/kept")).unwrap(), b"kept");
    }
}
