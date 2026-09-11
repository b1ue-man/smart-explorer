//! Windows task-runner acceptance for the actual GUI clipboard routing.
//! The task entrypoint isolates app data and serializes native clipboard tests.

use super::clipboard_lifecycle::PreparedTempClipboard;
use super::clipboard_state::PreparationResult;
use super::drag_drop::same_drop_namespace;
use super::prelude::*;
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
        ext: Arc::from(Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("")),
        size,
        mtime_ms: 1,
        btime_ms: 1,
        is_dir,
        is_symlink: false,
        hidden: false,
        system: false,
        depth: 0,
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

fn finish_upload(app: &mut App, files: u64, transferred_bytes: u64) {
    let rx = app.upload_rx.take().unwrap_or_else(|| {
        panic!("GUI did not start an upload: {:?}; {:?}", app.error_msg, app.notice)
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let message = rx.recv_timeout(left).expect("upload must report terminal completion");
        if let TransferMsg::Done { progress, errors, canceled } = message {
            assert!(!canceled, "unexpected transfer cancellation");
            assert!(errors.is_empty(), "transfer errors: {errors:?}");
            assert_eq!(progress.errors, 0);
            assert_eq!(progress.files_done, files);
            assert_eq!(progress.bytes_done, transferred_bytes);
            break;
        }
    }
    app.transfer_worker.take().unwrap().join().expect("upload worker panicked");
    app.transfer_cancel = None;
    app.transfer_progress = None;
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
}

fn finish_preparation(app: &mut App, remote_download: bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.clip_prepare_rx.is_some() || app.clip_download_rx.is_some() {
        if remote_download {
            app.drain_clip_download();
        } else {
            app.drain_clip_prepare();
        }
        assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
        assert!(Instant::now() < deadline, "clipboard preparation did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.clipboard_preparation.pending().is_none());
}

fn assert_owned_result_cleanup(owned: &mut OwnedFiles) {
    let queued = owned.file("discarded-queued.txt", b"queued");
    let (tx, rx) = unbounded();
    assert!(tx.send(PreparedTempClipboard::new(vec![path_text(&queued)])).is_ok());
    drop(rx);
    assert!(!queued.exists(), "dropping queued receiver must release its result");

    let late = owned.file("discarded-late.txt", b"late");
    let (tx, rx) = unbounded();
    drop(rx);
    let failed_send = tx.send(PreparedTempClipboard::new(vec![path_text(&late)]));
    assert!(failed_send.is_err());
    drop(failed_send);
    assert!(!late.exists(), "a worker completing after receiver disposal must clean up");
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
    assert!(app.update_rx.is_none(), "task construction must not launch an update check");
    assert_owned_result_cleanup(&mut owned);

    // Real local Ctrl+C -> real Windows CF_HDROP -> GUI remote Ctrl+V -> Share.
    let plain_bytes = b"local clipboard -> Share, exact bytes\0\xff";
    let plain = owned.file("local-über.txt", plain_bytes);
    app.root_path = path_text(plain.parent().unwrap());
    select(&mut app, entry(&path_text(&plain), false, plain_bytes.len() as u64));
    app.clipboard_copy_files(false);
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
    let (copied, cut) = read_clipboard_files().unwrap().unwrap();
    assert!(!cut);
    assert_eq!(copied.len(), 1);
    assert_eq!(PathBuf::from(&copied[0]), plain);
    app.remote = Some(remote(&peer.backend));
    app.root_path = "/A".into();
    app.clipboard_paste_files();
    finish_upload(&mut app, 1, plain_bytes.len() as u64);
    assert_eq!(std::fs::read(peer.root_a.join("local-über.txt")).unwrap(), plain_bytes);
    assert_eq!(std::fs::read(&plain).unwrap(), plain_bytes);

    // Pending paste cannot upload the earlier CF_HDROP. A newer local copy
    // replaces both preparations even if the old worker completes afterwards.
    let stamp = app.begin_clipboard_preparation().unwrap();
    let (old_tx, old_rx) = unbounded();
    app.clip_download_rx = Some(old_rx);
    app.clipboard_paste_files();
    assert!(app.upload_rx.is_none());
    assert_eq!(app.clipboard_preparation.pending(), Some(stamp));
    assert!(app.notice.as_ref().unwrap().0.contains("vorbereitet"));
    app.remote = None;
    app.clipboard_copy_files(false);
    assert!(app.clip_download_rx.is_none());
    assert!(app.clipboard_preparation.pending().is_none());
    let late = owned.file("superseded-worker.txt", b"old worker");
    let old_result = old_tx.send(PreparationResult {
        stamp,
        result: Ok(PreparedTempClipboard::new(vec![path_text(&late)])),
    });
    assert!(old_result.is_err());
    drop(old_result);
    assert!(!late.exists());

    // An external sequence change discards a queued downloaded result without
    // changing the newer Windows clipboard.
    let stale = owned.file("externally-superseded.txt", b"stale");
    let stamp = app.begin_clipboard_preparation().unwrap();
    let (tx, rx) = unbounded();
    app.clip_download_rx = Some(rx);
    assert!(tx.send(PreparationResult {
        stamp,
        result: Ok(PreparedTempClipboard::new(vec![path_text(&stale)])),
    }).is_ok());
    write_clipboard_files(&[path_text(&plain)], ClipboardEffect::Copy).unwrap();
    let external_sequence = virtual_clipboard_sequence();
    app.drain_clip_download();
    assert!(!stale.exists());
    assert!(app.clipboard_preparation.pending().is_none());
    assert_eq!(virtual_clipboard_sequence(), external_sequence);

    // Real filtered preparation publishes OLE descriptors, then own-payload
    // remote paste keeps the selected folder and nested relative hierarchy.
    let vault = open_temp_path("vault").unwrap();
    owned.0.push(vault.clone());
    std::fs::create_dir_all(vault.join("docs")).unwrap();
    std::fs::write(vault.join("keep-root.md"), b"root note").unwrap();
    std::fs::write(vault.join("docs/keep-note.md"), b"nested note").unwrap();
    std::fs::write(vault.join("ignored.txt"), b"must not upload").unwrap();
    app.root_path = path_text(vault.parent().unwrap());
    app.filter = FilterDef::new();
    app.filter.text = "keep".into();
    select(&mut app, entry(&path_text(&vault), true, 0));
    app.clipboard_copy_files(false);
    assert!(app.clip_prepare_rx.is_some());
    finish_preparation(&mut app, false);
    assert_eq!(app.virtual_clip.as_ref().unwrap().1.len(), 2);
    app.remote = Some(remote(&peer.backend));
    app.root_path = "/B".into();
    app.clipboard_paste_files();
    finish_upload(&mut app, 2, (b"root note".len() + b"nested note".len()) as u64);
    assert_eq!(std::fs::read(peer.root_b.join("vault/keep-root.md")).unwrap(), b"root note");
    assert_eq!(std::fs::read(peer.root_b.join("vault/docs/keep-note.md")).unwrap(), b"nested note");
    assert!(!peer.root_b.join("vault/ignored.txt").exists());
    assert!(!peer.root_b.join("keep-note.md").exists(), "hierarchy must not flatten");

    // Real remote copy/download publication keeps its owned temp file alive.
    app.filter = FilterDef::new();
    app.root_path = "/A".into();
    select(&mut app, entry("/A/local-über.txt", false, plain_bytes.len() as u64));
    app.clipboard_copy_files(false);
    assert!(app.clip_download_rx.is_some());
    finish_preparation(&mut app, true);
    let (downloaded, cut) = read_clipboard_files().unwrap().unwrap();
    assert!(!cut);
    assert_eq!(downloaded.len(), 1);
    let retained = PathBuf::from(&downloaded[0]);
    owned.0.push(retained.clone());
    assert_ne!(retained, plain);
    assert_eq!(std::fs::read(&retained).unwrap(), plain_bytes);

    // Neither a remote cut nor a local cut pasted to remote may silently copy.
    let sequence = virtual_clipboard_sequence();
    app.clipboard_copy_files(true);
    assert!(app.error_msg.as_deref().unwrap().contains("nicht unterstützt"));
    assert!(app.clip_download_rx.is_none());
    assert_eq!(virtual_clipboard_sequence(), sequence);
    assert_eq!(std::fs::read(peer.root_a.join("local-über.txt")).unwrap(), plain_bytes);
    assert!(retained.exists());
    app.error_msg = None;
    write_clipboard_files(&[path_text(&plain)], ClipboardEffect::Move).unwrap();
    app.root_path = "/B".into();
    app.clipboard_paste_files();
    assert!(app.upload_rx.is_none());
    assert!(app.error_msg.as_deref().unwrap().contains("nicht unterstützt"));
    assert_eq!(std::fs::read(&plain).unwrap(), plain_bytes);
    assert!(!peer.root_b.join("local-über.txt").exists());

    // Same textual /A parent on different actual peer handles must transfer.
    assert!(same_drop_namespace(None, None));
    assert!(same_drop_namespace(Some(&peer.backend), Some(&peer.backend.clone())));
    assert!(!same_drop_namespace(None, Some(&peer.backend)));
    assert!(!same_drop_namespace(Some(&peer.backend), Some(&other.backend)));
    app.error_msg = None;
    app.remote = Some(remote(&other.backend));
    app.root_path = "/A".into();
    app.drag_src = Some(peer.backend.clone());
    app.drag_files = vec!["/A/local-über.txt".into()];
    app.drop_files_into_tab(app.active_tab, false);
    // Cross-peer accounting includes the download and upload transfer legs.
    finish_upload(&mut app, 1, plain_bytes.len() as u64 * 2);
    assert_eq!(std::fs::read(other.root_a.join("local-über.txt")).unwrap(), plain_bytes);
    assert_eq!(std::fs::read(peer.root_a.join("local-über.txt")).unwrap(), plain_bytes);

    // Same handle + same parent remains a no-op; a requested remote move is
    // rejected before starting a worker or mutating either tree.
    app.drag_src = Some(other.backend.clone());
    app.drag_files = vec!["/A/local-über.txt".into()];
    app.drop_files_into_tab(app.active_tab, false);
    assert!(app.upload_rx.is_none());
    app.drag_src = Some(peer.backend.clone());
    app.drag_files = vec!["/A/local-über.txt".into()];
    app.root_path = "/B".into();
    app.drop_files_into_tab(app.active_tab, true);
    assert!(app.upload_rx.is_none());
    assert!(app.error_msg.as_deref().unwrap().contains("nicht unterstützt"));
    assert!(!other.root_b.join("local-über.txt").exists());
    assert_eq!(std::fs::read(peer.root_a.join("local-über.txt")).unwrap(), plain_bytes);
}
