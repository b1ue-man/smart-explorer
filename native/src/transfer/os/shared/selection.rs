//! A remote selection handed to another program (on Windows: Explorer paste
//! and drag-out as virtual files). It is listed only when that program asks
//! for the file list, in parallel under the connection's flow, and read file
//! by file on demand. Nothing is downloaded in advance.
use super::entries::compile_remote_filter;
use super::flow::flow_for;
use super::walk::{walk, WalkEvent, WalkOptions, WalkRoot};
use super::walk_listers::BackendLister;
use crate::types::FilterDef;
use crate::vfs::BackendHandle;
use std::io::{self, Read};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

/// What was copied: whole selected entries on one connection, optionally
/// limited by the view's filter below selected folders.
#[derive(Clone)]
pub struct SelectionSource {
    pub backend: BackendHandle,
    pub paths: Vec<String>,
    pub filter: Option<(FilterDef, String)>,
    pub label: String,
}

/// One entry of the listing; folders precede their contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedEntry {
    /// Path below the paste target, forward slashes.
    pub rel: String,
    /// Source path on the backend.
    pub path: String,
    pub id: Option<String>,
    /// Length of the bytes `open` yields; 0 when unknown.
    pub size: u64,
    /// False for provider exports whose length is only known after reading.
    pub size_known: bool,
    pub mtime_ms: i64,
    pub is_dir: bool,
}

#[derive(Debug, Default)]
pub struct SelectionListing {
    pub entries: Vec<ListedEntry>,
    /// Entries that cannot be handed over, with the reason.
    pub problems: Vec<(String, String)>,
    /// Protected omissions (the active app trash).
    pub omitted: u64,
    /// False when the listing was canceled before it finished.
    pub complete: bool,
}

impl SelectionSource {
    /// Lists everything below the selection; `on_found` receives the running
    /// entry count for progress displays.
    pub fn list_all(
        &self,
        cancel: &AtomicBool,
        on_found: &(dyn Fn(u64) + Sync),
    ) -> SelectionListing {
        let roots: Vec<WalkRoot> = self
            .paths
            .iter()
            .map(|path| WalkRoot {
                path: path.clone(),
                rel: self.download_name(path, root_name(path)),
            })
            .collect();
        let options = WalkOptions {
            filter: compile_remote_filter(self.filter.clone()),
            folders: self.filter.is_none(),
            ..WalkOptions::default()
        };
        let first = self.paths.first().map(String::as_str).unwrap_or("/");
        let flow = flow_for(&*self.backend, first);
        let listing = Mutex::new(SelectionListing::default());
        let complete = walk(
            &BackendLister(&*self.backend),
            &roots,
            &options,
            &flow,
            cancel,
            &|event| {
                let mut listing = listing
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match event {
                    WalkEvent::Dir { path, rel } => listing.entries.push(ListedEntry {
                        rel,
                        path,
                        id: None,
                        size: 0,
                        size_known: true,
                        mtime_ms: 0,
                        is_dir: true,
                    }),
                    WalkEvent::File {
                        path,
                        rel,
                        size,
                        mtime_ms,
                        id,
                        ..
                    } => {
                        let known = self.backend.read_size(&path, size).ok().flatten();
                        let rel = match rel.rsplit_once('/') {
                            Some((parent, name)) => {
                                format!("{parent}/{}", self.download_name(&path, name))
                            }
                            None => rel,
                        };
                        listing.entries.push(ListedEntry {
                            rel,
                            path,
                            id,
                            size: known.unwrap_or(0),
                            size_known: known.is_some(),
                            mtime_ms,
                            is_dir: false,
                        });
                    }
                    WalkEvent::Omitted { .. } => listing.omitted += 1,
                    WalkEvent::Problem { path, message } => listing.problems.push((path, message)),
                    // Remote selections have no access gate; kept for completeness.
                    WalkEvent::AccessRefused { path } => listing
                        .problems
                        .push((path, "Lesezugriff wurde abgelehnt".to_string())),
                }
                let found = listing.entries.len() as u64;
                drop(listing);
                on_found(found);
                true
            },
        );
        let mut listing = listing
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        listing.complete = complete;
        listing
    }

    /// Opens one listed file for reading (exact object for ID providers).
    pub fn open(&self, entry: &ListedEntry) -> io::Result<Box<dyn Read + Send>> {
        self.backend.open_read_id(&entry.path, entry.id.as_deref())
    }

    /// The local name a file is saved as (provider exports add an extension).
    fn download_name(&self, path: &str, name: &str) -> String {
        self.backend.download_name(path, name)
    }
}

fn root_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Auswahl")
}
