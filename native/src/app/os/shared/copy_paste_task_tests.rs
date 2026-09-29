//! Windows task-runner acceptance for the actual GUI clipboard routing: every
//! copy only remembers the selection (and offers it to Explorer), every paste
//! or drop starts one engine job at once. The task entrypoint isolates app
//! data and serializes native clipboard tests.

use super::prelude::*;
use super::transfer_route::TransferPlace;
use super::*;
use crate::app::shared_platform_helpers::ClipboardEffect;
use std::time::Duration;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{OleInitialize, OleSetClipboard, OleUninitialize};

struct OleSession;

impl OleSession {
    fn new() -> Self {
        unsafe { OleInitialize(None) }.expect("initialize this test thread as an OLE STA");
        Self
    }
}

impl Drop for OleSession {
    fn drop(&mut self) {
        unsafe {
            let _ = OleSetClipboard(None::<&IDataObject>);
            OleUninitialize();
        }
    }
}

#[derive(Default)]
struct OwnedFiles(Vec<PathBuf>);

impl OwnedFiles {
    fn file(&mut self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = open_temp_path(name).unwrap();
        self.0.push(path.clone());
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for OwnedFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            cleanup_temp_copy(path);
        }
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn entry(path: &str, is_dir: bool, size: u64) -> FileEntry {
    let (parent, name) = path.rsplit_once('/').expect("absolute task entry path");
    FileEntry {
        path: Arc::from(path),
        parent: Arc::from(parent),
        name: Arc::from(name),
        ext: Arc::from(
            Path::new(name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or(""),
        ),
        size,
        mtime_ms: 1,
        btime_ms: 1,
        is_dir,
        is_symlink: false,
        hidden: false,
        system: false,
        depth: 1,
        id: None,
    }
}

fn select(app: &mut App, selected: FileEntry) {
    app.selection.clear();
    app.selection.insert(selected.key());
    app.entries = vec![selected];
    app.error_msg = None;
    app.notice = None;
}

fn remote(backend: &crate::vfs::BackendHandle) -> crate::connect::RemoteState {
    crate::connect::RemoteState {
        backend: backend.clone(),
        label: "isolated copy/paste task peer".into(),
        agent_version: None,
        zip_return: None,
        sftp: None,
        account: None,
        endpoint_prefix: None,
    }
}

/// Exactly one transfer was started; wait until the list shows it finished
/// with `files` files and `bytes` bytes and without any issue.
fn finish_transfer(app: &mut App, files: u64, bytes: u64) {
    assert_eq!(
        app.transfer_center.lane.active.len(),
        1,
        "exactly one transfer must run: {:?}; {:?}",
        app.error_msg,
        app.notice
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    while !app.transfer_center.is_idle() {
        app.drain_transfers();
        assert!(Instant::now() < deadline, "transfer never completed");
        std::thread::sleep(Duration::from_millis(5));
    }
    let finished = app
        .transfer_center
        .finished
        .front()
        .expect("the transfer is listed as finished");
    assert!(!finished.canceled, "unexpected transfer cancellation");
    assert!(finished.failure.is_none(), "{:?}", finished.failure);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.errors, 0, "{:?}", finished.errors);
    assert_eq!(finished.progress.files_done, files);
    assert_eq!(finished.progress.bytes_done, bytes);
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
}

fn finish_preparation(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.clip_prepare_rx.is_some() || app.clip_download_rx.is_some() {
        app.drain_clip_prepare();
        app.drain_clip_download();
        assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
        assert!(
            Instant::now() < deadline,
            "clipboard preparation did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.clipboard_preparation.pending().is_none());
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_gui_clipboard_lifecycle_and_real_share_routing() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(),
        Ok("1"),
        "run through the isolated copy/paste task entrypoint"
    );
    let appdata = PathBuf::from(std::env::var_os("APPDATA").expect("isolated APPDATA"));
    assert!(appdata.is_dir());
    assert!(std::env::var_os("LOCALAPPDATA").is_some());
    assert!(crate::support_dirs::app_data_dir().starts_with(&appdata));
    // Drop OLE ownership before deleting any paths still advertised to Windows.
    let mut owned = OwnedFiles::default();
    let _ole = OleSession::new();
    let peer = crate::share::CopyPastePeerFixture::new().unwrap();
    let other = crate::share::CopyPastePeerFixture::new().unwrap();
    let mut app = App::new_for_copy_task();
    assert!(
        app.update_rx.is_none(),
        "task construction must not launch an update check"
    );

    // Local Ctrl+C: real CF_HDROP for Explorer, our entry for the app; the
    // paste into the Share starts at once (no preparation to wait for).
    let plain_bytes = b"local clipboard -> Share, exact bytes\0\xff";
    let plain = owned.file("local-über.txt", plain_bytes);
    app.root_path = path_text(plain.parent().unwrap());
    select(
        &mut app,
        entry(&path_text(&plain), false, plain_bytes.len() as u64),
    );
    app.clipboard_copy_files(false);
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
    let (copied, cut) = read_clipboard_files().unwrap().unwrap();
    assert!(!cut);
    assert_eq!(copied.len(), 1);
    assert_eq!(PathBuf::from(&copied[0]), plain);
    assert!(app
        .clip
        .as_ref()
        .is_some_and(|clip| clip.is_current(virtual_clipboard_sequence(), None)));
    app.remote = Some(remote(&peer.backend));
    app.root_path = "/A".into();
    app.clipboard_paste_files();
    finish_transfer(&mut app, 1, plain_bytes.len() as u64);
    assert_eq!(
        std::fs::read(peer.root_a.join("local-über.txt")).unwrap(),
        plain_bytes
    );
    assert_eq!(std::fs::read(&plain).unwrap(), plain_bytes);

    // Another program copies: our entry is no longer current and the paste
    // takes the OS file clipboard instead.
    let external_bytes = b"copied in Explorer";
    let external = owned.file("explorer-copy.txt", external_bytes);
    write_clipboard_files(&[path_text(&external)], ClipboardEffect::Copy).unwrap();
    assert!(app
        .clip
        .as_ref()
        .is_some_and(|clip| !clip.is_current(virtual_clipboard_sequence(), None)));
    app.root_path = "/B".into();
    app.clipboard_paste_files();
    finish_transfer(&mut app, 1, external_bytes.len() as u64);
    assert_eq!(
        std::fs::read(peer.root_b.join("explorer-copy.txt")).unwrap(),
        external_bytes
    );
    assert!(app.clip.is_none(), "the stale entry is dropped");

    // A filtered local folder: pasting in the app does not wait for the
    // virtual files Explorer gets; only matching files keep their folders.
    let vault = open_temp_path("vault").unwrap();
    owned.0.push(vault.clone());
    std::fs::create_dir_all(vault.join("docs")).unwrap();
    std::fs::write(vault.join("keep-root.md"), b"root note").unwrap();
    std::fs::write(vault.join("docs/keep-note.md"), b"nested note").unwrap();
    std::fs::write(vault.join("ignored.txt"), b"must not upload").unwrap();
    app.remote = None;
    app.root_path = path_text(vault.parent().unwrap());
    app.filter = FilterDef::new();
    app.filter.text = "keep".into();
    select(&mut app, entry(&path_text(&vault), true, 0));
    app.clipboard_copy_files(false);
    assert!(app.clip_prepare_rx.is_some(), "virtual files are prepared");
    app.remote = Some(remote(&peer.backend));
    app.root_path = "/B".into();
    app.clipboard_paste_files();
    finish_transfer(
        &mut app,
        2,
        (b"root note".len() + b"nested note".len()) as u64,
    );
    finish_preparation(&mut app);
    assert!(app
        .clip
        .as_ref()
        .is_some_and(|clip| clip.is_current(virtual_clipboard_sequence(), None)));
    assert_eq!(
        std::fs::read(peer.root_b.join("vault/keep-root.md")).unwrap(),
        b"root note"
    );
    assert_eq!(
        std::fs::read(peer.root_b.join("vault/docs/keep-note.md")).unwrap(),
        b"nested note"
    );
    assert!(!peer.root_b.join("vault/ignored.txt").exists());
    assert!(
        !peer.root_b.join("keep-note.md").exists(),
        "hierarchy must not flatten"
    );

    // Remote Ctrl+C downloads nothing; the paste into a local folder is one
    // engine download.
    app.filter = FilterDef::new();
    app.root_path = "/A".into();
    select(
        &mut app,
        entry("/A/local-über.txt", false, plain_bytes.len() as u64),
    );
    app.clipboard_copy_files(false);
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
    assert!(app.transfer_center.is_idle(), "copying starts no transfer");
    let download_dir = open_temp_path("download-target").unwrap();
    owned.0.push(download_dir.clone());
    std::fs::create_dir_all(&download_dir).unwrap();
    app.remote = None;
    app.root_path = path_text(&download_dir);
    app.entries.clear();
    app.clipboard_paste_files();
    finish_transfer(&mut app, 1, plain_bytes.len() as u64);
    assert_eq!(
        std::fs::read(download_dir.join("local-über.txt")).unwrap(),
        plain_bytes
    );

    // Neither a remote cut nor a local cut pasted to remote may silently copy.
    app.remote = Some(remote(&peer.backend));
    app.root_path = "/A".into();
    select(
        &mut app,
        entry("/A/local-über.txt", false, plain_bytes.len() as u64),
    );
    let sequence = virtual_clipboard_sequence();
    app.clipboard_copy_files(true);
    assert!(app
        .error_msg
        .as_deref()
        .unwrap()
        .contains("nicht unterstützt"));
    assert_eq!(virtual_clipboard_sequence(), sequence);
    app.error_msg = None;
    write_clipboard_files(&[path_text(&plain)], ClipboardEffect::Move).unwrap();
    app.root_path = "/B".into();
    app.clipboard_paste_files();
    assert!(app.transfer_center.is_idle());
    assert!(app
        .error_msg
        .as_deref()
        .unwrap()
        .contains("nicht unterstützt"));
    assert_eq!(std::fs::read(&plain).unwrap(), plain_bytes);
    assert!(!peer.root_b.join("local-über.txt").exists());

    // Equal textual /A paths on two different peers are two places.
    assert!(!TransferPlace::remote(peer.backend.clone(), "eins")
        .same_place(&TransferPlace::remote(other.backend.clone(), "zwei")));
    app.error_msg = None;
    app.remote = Some(remote(&other.backend));
    app.root_path = "/A".into();
    app.drag_src = Some(peer.backend.clone());
    app.drag_files = vec!["/A/local-über.txt".into()];
    app.drop_files_into_tab(app.active_tab, false);
    finish_transfer(&mut app, 1, plain_bytes.len() as u64);
    assert_eq!(
        std::fs::read(other.root_a.join("local-über.txt")).unwrap(),
        plain_bytes
    );
    assert_eq!(
        std::fs::read(peer.root_a.join("local-über.txt")).unwrap(),
        plain_bytes
    );

    // Dropping onto the same folder of the same peer changes nothing; a
    // requested remote move is refused before any transfer starts.
    app.drag_src = Some(other.backend.clone());
    app.drag_files = vec!["/A/local-über.txt".into()];
    app.drop_files_into_tab(app.active_tab, false);
    assert!(app.transfer_center.is_idle());
    app.drag_src = Some(peer.backend.clone());
    app.drag_files = vec!["/A/local-über.txt".into()];
    app.root_path = "/B".into();
    app.drop_files_into_tab(app.active_tab, true);
    assert!(app.transfer_center.is_idle());
    assert!(app
        .error_msg
        .as_deref()
        .unwrap()
        .contains("nicht unterstützt"));
    assert!(!other.root_b.join("local-über.txt").exists());
    assert_eq!(
        std::fs::read(peer.root_a.join("local-über.txt")).unwrap(),
        plain_bytes
    );
}
