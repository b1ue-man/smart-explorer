//! Acceptance signals of the protections the engine path keeps (U49): no
//! request for read access, whole-listing name checks, the switch.
use super::{facts, target, Scratch};
use crate::cli::engine_copy::{
    eligible, try_copy, try_copy_with, with_legacy_path, CLI_ENGINE_PATH,
};
use crate::cli::tree_observer::TreeObserver;
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::io;
use std::sync::Arc;

/// A source folder `/source` that lists the given names (all small files).
struct Listing(Vec<&'static str>);

impl Backend for Listing {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }
    fn root_display(&self) -> String {
        "/".to_string()
    }
    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        Ok(self.0.iter().map(|name| file_meta(name)).collect())
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match path {
            "/source" => Ok(VfsMeta {
                name: "source".to_string(),
                is_dir: true,
                id: Some("source-root".to_string()),
                ..VfsMeta::default()
            }),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn std::io::Read + Send>> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn std::io::Write + Send>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn file_meta(name: &str) -> VfsMeta {
    VfsMeta {
        name: name.to_string(),
        size: 4,
        mtime_ms: 7,
        id: Some(format!("id-{name}")),
        ..VfsMeta::default()
    }
}

#[test]
fn speed_task_cli_engine_never_lets_the_engine_ask_for_read_access() {
    let mut asks = facts(crate::cli::entry_kind::EntryKind::File);
    asks.access_prompt_possible = true;
    assert!(!eligible(&asks), "the older path reports a refused read");
    asks.access_prompt_possible = false;
    assert!(eligible(&asks));
    // Where no request is possible (here: every non-Windows host) the local
    // source runs on the engine.
    let scratch = Scratch::new("access");
    let src = scratch.file("source.txt", b"readable");
    let dst = scratch.path("copy.txt");
    let (source, destination) = (target(&src), target(&dst));
    let meta = source.backend.stat(&source.path).expect("stat");
    let probed = try_copy_with(
        true,
        &source,
        &meta,
        &meta,
        &destination,
        &dst,
        false,
        &TreeObserver::default(),
    )
    .expect("copy");
    #[cfg(not(windows))]
    assert!(probed.is_some());
    #[cfg(windows)]
    let _ = probed;
}

#[test]
fn speed_task_cli_engine_unsafe_or_duplicate_listed_names_stay_on_the_older_path() {
    for names in [vec!["good.txt", "../escape"], vec!["a.txt", "a.txt"]] {
        let scratch = Scratch::new("names");
        let handle: BackendHandle = Arc::new(Listing(names));
        let source = crate::cli::target::Target::with_backend_key(
            handle,
            "/source".to_string(),
            "synthetic-source",
        );
        let dst = scratch.path("destination");
        let destination = target(&dst);
        let meta = source.backend.stat(&source.path).expect("stat");
        let outcome = try_copy_with(
            true,
            &source,
            &meta,
            &meta,
            &destination,
            &dst,
            true,
            &TreeObserver::default(),
        )
        .expect("the older path reports the error, not this one");
        assert!(outcome.is_none());
        assert!(!scratch.os("destination").exists(), "nothing was created");
    }
}

#[test]
fn speed_task_cli_engine_switch_is_on_and_the_older_path_can_be_forced() {
    assert!(CLI_ENGINE_PATH);
    let scratch = Scratch::new("switch");
    let src = scratch.file("source.txt", b"data");
    let dst = scratch.path("copy.txt");
    let (source, destination) = (target(&src), target(&dst));
    let meta = source.backend.stat(&source.path).expect("stat");
    let observer = TreeObserver::default();
    let forced =
        with_legacy_path(|| try_copy(&source, &meta, &meta, &destination, &dst, false, &observer))
            .expect("no error");
    assert!(forced.is_none());
    assert!(!scratch.os("copy.txt").exists());
    assert!(!observer.progress().engine);
}
