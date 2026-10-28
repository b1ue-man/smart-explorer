//! Signals of DRV-M4 (strang-laufwerke.md L 264), part 1: what programs do
//! with a mounted Linux drive (read, write, folders, editor saves, GLib's
//! replace, `cp -a`, `rsync -a`) over a local folder and the SFTP container
//! of the stage. Every test mounts through `fusermount3` and is `#[ignore]`:
//! stage `linux-root: drive-fuse` (`SMART_EXPLORER_FUSE_TASK=1`, fixtures
//! `fuse` and, for SFTP, `sftp` and `home`).
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, UNIX_EPOCH};

use super::fuse_task_backends::{local, sftp};
use super::fuse_task_harness::{
    eventually, names_in, program, remote_bytes, remove_remote_tree, require_task_stage, tool,
    unmount_cleanly, FuseTaskHost, HostSetup,
};
use crate::mount::MountMode;

const STAGE: &str = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)";
const RW: MountMode = MountMode::ReadWrite;

/// A local export folder with one seeded file, mounted read-write.
fn local_drive(files: &[(&str, &[u8])]) -> io::Result<(tempfile::TempDir, FuseTaskHost)> {
    let fixture = tempfile::tempdir()?;
    let export = fixture.path().join("export");
    fs::create_dir_all(&export)?;
    for (name, bytes) in files {
        let path = export.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    let host = FuseTaskHost::start(HostSetup::new(local(&export, RW)?, RW))?;
    Ok((fixture, host))
}

fn export(fixture: &tempfile::TempDir) -> std::path::PathBuf {
    fixture.path().join("export")
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_mount_local_fixture_rw() -> io::Result<()> {
    require_task_stage();
    let (fixture, host) = local_drive(&[("docs/seed.txt", b"seed")])?;
    let root = export(&fixture);
    assert_eq!(host.names()?, ["docs"], "{STAGE}");
    assert_eq!(fs::read(host.path("docs/seed.txt"))?, b"seed");

    fs::write(host.path("new.txt"), b"written through FUSE")?;
    assert!(settled(&root.join("new.txt"), b"written through FUSE"));
    let mut appended = OpenOptions::new().append(true).open(host.path("new.txt"))?;
    appended.write_all(b" + appended")?;
    drop(appended);
    assert!(settled(
        &root.join("new.txt"),
        b"written through FUSE + appended"
    ));
    // ftruncate(2) through a descriptor, uploaded after close(2).
    OpenOptions::new()
        .write(true)
        .open(host.path("docs/seed.txt"))?
        .set_len(2)?;
    assert!(settled(&root.join("docs/seed.txt"), b"se"));

    fs::create_dir(host.path("made"))?;
    assert!(root.join("made").is_dir());
    fs::rename(host.path("new.txt"), host.path("made/moved.txt"))?;
    assert!(!root.join("new.txt").exists());
    assert_eq!(names_in(&host.path("made"))?, ["moved.txt"]);
    assert_eq!(
        fs::read(root.join("made/moved.txt"))?,
        b"written through FUSE + appended"
    );
    fs::remove_file(host.path("made/moved.txt"))?;
    fs::remove_dir(host.path("made"))?;
    assert!(!root.join("made").exists());
    assert_eq!(host.names()?, ["docs"]);
    unmount_cleanly(host)
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse with the sftp fixture (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_sftp_container_rw() -> io::Result<()> {
    require_task_stage();
    let (raw, drive, server) = sftp(RW)?;
    let host = FuseTaskHost::start(HostSetup::new(drive, RW))?;
    let remote = |name: &str| format!("{server}/{name}");

    fs::write(host.path("hello.txt"), b"hello over SFTP")?;
    assert!(
        eventually(Duration::from_secs(30), || {
            remote_bytes(&raw, &remote("hello.txt")).is_ok_and(|bytes| bytes == b"hello over SFTP")
        }),
        "the upload after close(2) reaches the server"
    );
    assert_eq!(fs::read(host.path("hello.txt"))?, b"hello over SFTP");
    fs::create_dir(host.path("folder"))?;
    fs::write(host.path("folder/inner.txt"), b"inner")?;
    assert!(raw.stat(&remote("folder"))?.is_dir);
    fs::rename(host.path("hello.txt"), host.path("folder/renamed.txt"))?;
    assert!(raw.stat(&remote("hello.txt")).is_err());
    assert_eq!(
        remote_bytes(&raw, &remote("folder/renamed.txt"))?,
        b"hello over SFTP"
    );
    fs::remove_file(host.path("folder/inner.txt"))?;
    assert_eq!(names_in(&host.path("folder"))?, ["renamed.txt"]);
    fs::remove_file(host.path("folder/renamed.txt"))?;
    fs::remove_dir(host.path("folder"))?;
    assert!(raw.list_dir(&server)?.is_empty());
    unmount_cleanly(host)?;
    raw.remove_dir(&server)
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_editor_save_rename_over() -> io::Result<()> {
    require_task_stage();
    let (fixture, host) = local_drive(&[("note.txt", b"v1")])?;
    let root = export(&fixture);
    let before = fs::metadata(host.path("note.txt"))?.ino();

    // gedit, Kate, VS Code: a temporary file, fsync(2), rename(2) over the
    // original.
    let mut temporary = File::create(host.path(".note.txt.save"))?;
    temporary.write_all(b"v2")?;
    temporary.sync_all()?;
    drop(temporary);
    fs::rename(host.path(".note.txt.save"), host.path("note.txt"))?;
    assert_eq!(fs::read(host.path("note.txt"))?, b"v2");
    assert_eq!(fs::read(root.join("note.txt"))?, b"v2");
    assert_ne!(fs::metadata(host.path("note.txt"))?.ino(), before);
    assert_eq!(names_in(&root)?, ["note.txt"]);

    // Vim's writebackup: rename the original away, write anew, delete the
    // backup.
    fs::rename(host.path("note.txt"), host.path("note.txt~"))?;
    fs::write(host.path("note.txt"), b"v3")?;
    fs::remove_file(host.path("note.txt~"))?;
    assert!(settled(&root.join("note.txt"), b"v3"));
    assert_eq!(names_in(&root)?, ["note.txt"]);
    assert_eq!(host.names()?, ["note.txt"]);
    unmount_cleanly(host)
}

/// The system calls of GLib's `g_file_replace` on an existing file
/// (`glocalfileoutputstream.c`, `handle_overwrite_open`) where `gio` is
/// missing: a failing `fchown` or `fchmod` with the original's owner and
/// mode would make GLib fall back to rewriting the file in place.
fn glib_replace(target: &Path, contents: &[u8]) -> io::Result<()> {
    let original = OpenOptions::new()
        .write(true)
        .create(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(target)?;
    let stat = original.metadata()?;
    let temporary_path = target.with_file_name(".goutputstream-FUSE01");
    let mut temporary = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o666)
        .open(&temporary_path)?;
    std::os::unix::fs::fchown(&temporary, Some(stat.uid()), Some(stat.gid()))?;
    temporary.set_permissions(fs::Permissions::from_mode(stat.mode() & 0o7777))?;
    drop(original);
    temporary.write_all(contents)?;
    temporary.sync_all()?;
    fs::rename(&temporary_path, target)?;
    drop(temporary);
    Ok(())
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_gio_replace_stays_atomic() -> io::Result<()> {
    require_task_stage();
    let (fixture, host) = local_drive(&[("doc.txt", b"first")])?;
    let root = export(&fixture);
    let target = host.path("doc.txt");
    let before = fs::metadata(&target)?;
    match tool("gio") {
        Some(gio) => {
            let mut child = Command::new(gio)
                .arg("save")
                .arg(&target)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("gio stdin"))?
                .write_all(b"second")?;
            let output = child.wait_with_output()?;
            assert!(
                output.status.success(),
                "gio save: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        None => {
            eprintln!("[drive task] gio fehlt; GLibs Ersetzen wird mit seinen Systemaufrufen nachgestellt");
            glib_replace(&target, b"second")?;
        }
    }
    let after = fs::metadata(&target)?;
    assert_eq!(fs::read(root.join("doc.txt"))?, b"second");
    // Same mode and owner, so GLib kept its atomic path: a new inode took
    // the name, nothing was rewritten in place.
    assert_eq!(after.mode() & 0o7777, before.mode() & 0o7777);
    assert_ne!(
        after.ino(),
        before.ino(),
        "GLib fell back to rewriting in place"
    );
    assert_eq!(names_in(&root)?, ["doc.txt"]);
    unmount_cleanly(host)
}

/// A source tree with fixed times and modes; returns (name, bytes, mode).
fn source_tree(root: &Path) -> io::Result<Vec<(&'static str, &'static [u8], u32)>> {
    let files: Vec<(&'static str, &'static [u8], u32)> = vec![
        ("a.txt", b"alpha", 0o640),
        ("sub/b.bin", &[0, 1, 2, 3, 255], 0o600),
        ("sub/empty", b"", 0o644),
    ];
    let stamp = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    for (name, bytes, mode) in &files {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(*mode))?;
        File::options()
            .write(true)
            .open(&path)?
            .set_modified(stamp)?;
    }
    Ok(files)
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse with the sftp fixture (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_cp_a_exit_zero_and_times_applied_on_sftp() -> io::Result<()> {
    require_task_stage();
    let (raw, drive, server) = sftp(RW)?;
    let host = FuseTaskHost::start(HostSetup::new(drive, RW))?;
    let source = tempfile::tempdir()?;
    let tree = source.path().join("tree");
    let files = source_tree(&tree)?;
    let (code, output) = program(Command::new("cp").arg("-a").arg(&tree).arg(&host.folder))?;
    assert_eq!(code, 0, "cp -a: {output}");
    for (name, bytes, mode) in &files {
        let remote = format!("{server}/tree/{name}");
        assert_eq!(remote_bytes(&raw, &remote)?, *bytes, "{name}");
        // Times and modes cp -a sets on the open descriptor reach the
        // publication (decision C2); SFTP keeps seconds.
        assert_eq!(raw.stat(&remote)?.mtime_ms, 1_600_000_000_000, "{name}");
        assert_eq!(
            crate::vfs::unix_mode(&*raw, &remote)?,
            Some(*mode),
            "{name}"
        );
        let shown = fs::metadata(host.path(&format!("tree/{name}")))?;
        assert_eq!(shown.mtime(), 1_600_000_000, "{name}");
    }
    unmount_cleanly(host)?;
    remove_remote_tree(&raw, &server)
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_rsync_a_exit_zero() -> io::Result<()> {
    require_task_stage();
    let rsync = tool("rsync").expect("rsync fehlt auf dem Runner der Stufe linux-root: drive-fuse");
    let (fixture, host) = local_drive(&[])?;
    let root = export(&fixture);
    let source = fixture.path().join("source");
    let files = source_tree(&source)?;
    let run = || {
        program(
            Command::new(&rsync)
                .arg("-a")
                .arg(format!("{}/", source.display()))
                .arg(host.path("copy")),
        )
    };
    let (code, output) = run()?;
    assert_eq!(code, 0, "rsync -a: {output}");
    for (name, bytes, _) in &files {
        assert_eq!(fs::read(root.join("copy").join(name))?, *bytes, "{name}");
    }
    // No temporary `.<name>.XXXXXX` of rsync stays behind.
    assert_eq!(names_in(&root.join("copy"))?, ["a.txt", "sub"]);
    assert_eq!(names_in(&root.join("copy/sub"))?, ["b.bin", "empty"]);
    // A second run replaces what it sends again (times rsync sets by path
    // after closing are not kept, GOTCHAS) and also ends with 0.
    let (code, output) = run()?;
    assert_eq!(code, 0, "second rsync -a: {output}");
    assert_eq!(names_in(&root.join("copy"))?, ["a.txt", "sub"]);
    unmount_cleanly(host)
}

/// G4 ML1: `close(2)` returns before the upload; the exported file shows
/// `expected` once the background upload is done.
fn settled(path: &Path, expected: &[u8]) -> bool {
    eventually(Duration::from_secs(30), || {
        fs::read(path).is_ok_and(|bytes| bytes == expected)
    })
}
