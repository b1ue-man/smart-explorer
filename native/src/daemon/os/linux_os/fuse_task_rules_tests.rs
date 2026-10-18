//! Signals of DRV-M4 (strang-laufwerke.md L 264), part 2: the rules of a
//! mounted Linux drive (decision C2): owners, conflicts at `close(2)`, range
//! reads of a large read-only file, POSIX names Windows cannot hold, and
//! links and special files, which a drive never shows, creates or follows.
//! Every test mounts through `fusermount3` and is `#[ignore]`: stage
//! `linux-root: drive-fuse` (`SMART_EXPLORER_FUSE_TASK=1`, fixture `fuse`).
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::fuse_task_backends::{
    local, memory, rooted, Gated, GatedBackend, NamesBackend, HIDDEN_NAMES, LISTED_NAMES,
};
use super::fuse_task_harness::{
    close_checked, eventually, last, names_in, require_task_stage, unmount_cleanly, FuseTaskHost,
    HostSetup,
};
use crate::mount::range_read_task_tests::RangeBackend;
use crate::mount::{MountMode, MountRecovery, MountStatus};

const RW: MountMode = MountMode::ReadWrite;
const RO: MountMode = MountMode::ReadOnly;

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_chown_other_uid_eperm() -> io::Result<()> {
    require_task_stage();
    let (fixture, drive) = memory(RW)?;
    fixture.put("/owned.txt", b"owned");
    fixture.mkdir("/folder");
    let host = FuseTaskHost::start(HostSetup::new(drive, RW))?;
    // SAFETY: both calls only read ids of this process.
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    for name in ["owned.txt", "folder"] {
        let path = host.path(name);
        let meta = fs::metadata(&path)?;
        assert_eq!((meta.uid(), meta.gid()), (uid, gid), "{name}");
        // The user who mounted and his group: success without effect.
        std::os::unix::fs::chown(&path, Some(uid), Some(gid))?;
        std::os::unix::fs::lchown(&path, None, Some(gid))?;
        for (other_uid, other_gid) in [
            (Some(uid.wrapping_add(1)), None),
            (None, Some(gid.wrapping_add(1))),
            (Some(uid.wrapping_add(1)), Some(gid)),
        ] {
            let error = std::os::unix::fs::chown(&path, other_uid, other_gid)
                .expect_err("another owner is refused");
            assert_eq!(
                error.raw_os_error(),
                Some(libc::EPERM),
                "{name}: {other_uid:?} {other_gid:?}"
            );
        }
        let meta = fs::metadata(&path)?;
        assert_eq!((meta.uid(), meta.gid()), (uid, gid), "{name}");
    }
    assert_eq!(fixture.bytes("/owned.txt"), b"owned");
    unmount_cleanly(host)
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_conflict_returns_ebusy_and_copy() -> io::Result<()> {
    require_task_stage();
    let gated = GatedBackend::new();
    gated.inner.put("/doc.txt", b"original");
    let host = FuseTaskHost::start(HostSetup::new(gated.drive(RW)?, RW))?;
    let mut file = OpenOptions::new().write(true).open(host.path("doc.txt"))?;
    file.write_all(b"local edit")?;
    // The save at close(2) waits before its upload; the remote file
    // changes meanwhile.
    let gate = gated.gate(Gated::Upload);
    let closing = std::thread::spawn(move || close_checked(file));
    assert!(
        gate.entered_within(Duration::from_secs(30)),
        "the save never started its upload"
    );
    gated.inner.put("/doc.txt", b"remote change");
    gate.release(true);
    let closed = closing
        .join()
        .map_err(|_| io::Error::other("the closing thread panicked"))?;
    let error = closed.expect_err("close(2) reports the conflict");
    assert_eq!(error.raw_os_error(), Some(libc::EBUSY), "{error}");
    // The remote change stays; the save lies beside it as a conflict copy.
    assert_eq!(gated.inner.bytes("/doc.txt"), b"remote change");
    let copies: Vec<String> = crate::vfs::Backend::list_dir(&*gated.inner, "/")?
        .into_iter()
        .map(|entry| entry.name)
        .filter(|name| name.starts_with("doc (Konflikt ") && name.ends_with(".txt"))
        .collect();
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(gated.inner.bytes(&format!("/{}", copies[0])), b"local edit");
    assert!(
        eventually(Duration::from_secs(30), || host
            .names()
            .is_ok_and(|names| names.contains(&copies[0]))),
        "the conflict copy does not appear in the drive"
    );
    let (result, reports) = host.finish(false)?;
    // The journal keeps the conflict until the user resolves it.
    let detail = result.expect_err("a conflict keeps the cache");
    assert!(detail.contains("1 conflict"), "{detail}");
    assert!(
        matches!(
            last(&reports),
            Some((MountStatus::Failed { .. }, Some(MountRecovery::Required)))
        ),
        "{reports:?}"
    );
    Ok(())
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_large_readonly_range_read() -> io::Result<()> {
    require_task_stage();
    let raw = RangeBackend::new(64 * 1024 * 1024);
    let setup = HostSetup {
        cache_mib: Some(0),
        ..HostSetup::new(rooted(raw.clone(), "/root", RO)?, RO)
    };
    let host = FuseTaskHost::start(setup)?;
    assert_eq!(
        host.names()?,
        ["large"],
        "the link of the fixture is hidden"
    );
    let file = File::open(host.path("large"))?;
    assert_eq!(file.metadata()?.len(), raw.size);
    for offset in [0, 7 * 4096 + 13, raw.size - 4096] {
        let mut buffer = vec![0u8; 4096];
        let read = file.read_at(&mut buffer, offset)?;
        assert_eq!(read, buffer.len(), "offset {offset}");
        for (index, byte) in buffer.iter().enumerate() {
            assert_eq!(
                *byte,
                ((offset + index as u64) % 251) as u8,
                "offset {offset} + {index}"
            );
        }
    }
    drop(file);
    assert_eq!(
        raw.full_reads.load(Ordering::SeqCst),
        0,
        "the drive downloaded the whole file"
    );
    assert!(raw.range_reads.load(Ordering::SeqCst) >= 1);
    let streamed = raw.bytes.load(Ordering::SeqCst) as u64;
    assert!(streamed < raw.size / 2, "{streamed} bytes streamed");
    unmount_cleanly(host)
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_names_colon_con_listed() -> io::Result<()> {
    require_task_stage();
    let host = FuseTaskHost::start(HostSetup::new(NamesBackend::drive()?, RO))?;
    let mut expected: Vec<&str> = LISTED_NAMES.to_vec();
    expected.sort_unstable();
    assert_eq!(host.names()?, expected);
    for name in LISTED_NAMES {
        assert_eq!(fs::read(host.path(name))?, name.as_bytes(), "{name}");
    }
    // A read-only drive refuses every change.
    let refused = fs::write(host.path("new.txt"), b"x").expect_err("read-only");
    assert_eq!(refused.raw_os_error(), Some(libc::EROFS));
    unmount_cleanly(host)
}

fn make_fifo(path: &Path) -> io::Result<()> {
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in the path"))?;
    // SAFETY: `path` is a NUL-terminated string that lives across the call.
    if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_links_hidden() -> io::Result<()> {
    require_task_stage();
    // The fixture's links and FIFO are hidden.
    let names = FuseTaskHost::start(HostSetup::new(NamesBackend::drive()?, RO))?;
    for hidden in HIDDEN_NAMES {
        assert!(
            !names.names()?.iter().any(|name| name == hidden),
            "{hidden}"
        );
        let error = fs::symlink_metadata(names.path(hidden)).expect_err("hidden");
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT), "{hidden}");
    }
    unmount_cleanly(names)?;

    // A real folder: a link out of the root, a link to a file, a FIFO.
    let fixture = tempfile::tempdir()?;
    let export = fixture.path().join("export");
    let outside = fixture.path().join("outside");
    fs::create_dir_all(export.join("dir"))?;
    fs::create_dir_all(&outside)?;
    fs::write(
        outside.join("victim.txt"),
        b"never deleted through the drive",
    )?;
    fs::write(export.join("dir/plain.txt"), b"plain")?;
    std::os::unix::fs::symlink(&outside, export.join("dir/out"))?;
    std::os::unix::fs::symlink(outside.join("victim.txt"), export.join("file-link"))?;
    make_fifo(&export.join("fifo"))?;
    let host = FuseTaskHost::start(HostSetup::new(local(&export, RW)?, RW))?;
    assert_eq!(host.names()?, ["dir"]);
    assert_eq!(names_in(&host.path("dir"))?, ["plain.txt"]);
    for hidden in ["file-link", "fifo", "dir/out"] {
        let error = fs::symlink_metadata(host.path(hidden)).expect_err("hidden");
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT), "{hidden}");
    }
    // No link or special file is ever created on a drive.
    assert!(std::os::unix::fs::symlink("/etc", host.path("new-link")).is_err());
    assert!(make_fifo(&host.path("new-fifo")).is_err());
    assert!(fs::symlink_metadata(export.join("new-link")).is_err());
    // A recursive delete never follows a link out of the root: the hidden
    // link keeps its folder, and what it points to stays.
    let removed = fs::remove_dir_all(host.path("dir"));
    assert!(removed.is_err(), "the hidden link must keep dir");
    assert_eq!(
        fs::read(outside.join("victim.txt"))?,
        b"never deleted through the drive"
    );
    assert!(fs::symlink_metadata(export.join("dir/out"))?
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(export.join("file-link"))?
        .file_type()
        .is_symlink());
    unmount_cleanly(host)
}
