use super::*;
use crate::app::app_models::TransferMsg;
use crate::types::FilterDef;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!(
        "se_clip_test_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn txt_filter() -> FilterDef {
    let mut filter = FilterDef::new();
    filter.extensions = vec!["txt".to_string()];
    filter
}

#[test]
fn remote_clipboard_downloads_folder_tree() {
    let remote = temp_dir("remote");
    std::fs::create_dir_all(remote.join("Gate/sub")).unwrap();
    std::fs::write(remote.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(remote.join("Gate/sub/b.txt"), b"beta").unwrap();
    let be = crate::vfs::LocalBackend::new(&fwd(&remote));
    let item = (format!("{}/Gate", fwd(&remote)), "Gate".to_string(), true);

    let local = download_remote_clipboard_items(&be, &[item], None);

    assert_eq!(local.len(), 1);
    let local_dir = PathBuf::from(&local[0]);
    assert!(local_dir.is_dir());
    assert_eq!(std::fs::read(local_dir.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(local_dir.join("sub/b.txt")).unwrap(), b"beta");

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(local_dir);
}

#[test]
fn remote_clipboard_filters_folder_tree() {
    let remote = temp_dir("remote_filter_clip");
    std::fs::create_dir_all(remote.join("Gate/sub")).unwrap();
    std::fs::write(remote.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(remote.join("Gate/drop.bin"), b"drop").unwrap();
    std::fs::write(remote.join("Gate/sub/b.txt"), b"beta").unwrap();
    std::fs::write(remote.join("Gate/sub/drop.md"), b"drop").unwrap();
    let root = fwd(&remote);
    let be = crate::vfs::LocalBackend::new(&root);
    let item = (format!("{root}/Gate"), "Gate".to_string(), true);

    let local = download_remote_clipboard_items(&be, &[item], Some((txt_filter(), root)));

    assert_eq!(local.len(), 1);
    let local_dir = PathBuf::from(&local[0]);
    assert_eq!(std::fs::read(local_dir.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(local_dir.join("sub/b.txt")).unwrap(), b"beta");
    assert!(!local_dir.join("drop.bin").exists());
    assert!(!local_dir.join("sub/drop.md").exists());

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(local_dir);
}

#[test]
fn remote_download_filters_selected_folder() {
    let remote = temp_dir("remote_filter_download");
    let dest = temp_dir("remote_filter_dest");
    std::fs::create_dir_all(remote.join("Gate/sub")).unwrap();
    std::fs::write(remote.join("Gate/a.txt"), b"alpha").unwrap();
    std::fs::write(remote.join("Gate/drop.bin"), b"drop").unwrap();
    std::fs::write(remote.join("Gate/sub/b.txt"), b"beta").unwrap();
    std::fs::write(remote.join("Gate/sub/drop.md"), b"drop").unwrap();
    let root = fwd(&remote);
    let be = crate::vfs::LocalBackend::new(&root);
    let src = format!("{root}/Gate");
    let (tx, rx) = crossbeam_channel::unbounded();

    download_paths_progress(&be, &[src], &fwd(&dest), Some((txt_filter(), root)), &tx);

    let mut done = None;
    while let Ok(msg) = rx.try_recv() {
        if let TransferMsg::Done { progress, errors } = msg {
            done = Some((progress, errors));
        }
    }
    let (progress, errors) = done.expect("download should send Done");
    assert_eq!(progress.files_total, 2);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(std::fs::read(dest.join("Gate/a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(dest.join("Gate/sub/b.txt")).unwrap(), b"beta");
    assert!(!dest.join("Gate/drop.bin").exists());
    assert!(!dest.join("Gate/sub/drop.md").exists());

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(&dest);
}
