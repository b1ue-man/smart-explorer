//! The content search through the storage interface: a backend without its
//! own search is read file by file (links never opened, a vanished file
//! reported, Drive ids passed on), a backend with its own search answers
//! itself, and the listing cache passes both through.
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::agent_proto::{ContentEvent, ContentQuery, ContentSummary, SkipReason, TextMode};
use crate::vfs::{content_search, Backend, CachingBackend, Scheme, VfsMeta, VfsResult};

/// A folder tree in memory: `path → content`, folders implied; names ending
/// in `.link` are links, `gone.txt` is listed but cannot be opened.
struct MemoryBackend {
    files: BTreeMap<String, Vec<u8>>,
    opened: Mutex<Vec<(String, Option<String>)>>,
    own_search: bool,
    own_calls: AtomicUsize,
}

impl MemoryBackend {
    fn new(own_search: bool) -> Self {
        let mut files = BTreeMap::new();
        files.insert("/data/a.txt".into(), b"alpha\nneedle one\n".to_vec());
        files.insert("/data/sub/b.md".into(), b"# needle\n".to_vec());
        files.insert("/data/sub/c.link".into(), b"needle in link\n".to_vec());
        files.insert("/data/gone.txt".into(), Vec::new());
        files.insert("/data/.cache/d.txt".into(), b"needle hidden\n".to_vec());
        Self {
            files,
            opened: Mutex::new(Vec::new()),
            own_search,
            own_calls: AtomicUsize::new(0),
        }
    }

    fn meta(name: &str, is_dir: bool, size: u64) -> VfsMeta {
        VfsMeta {
            name: name.to_string(),
            is_dir,
            is_symlink: name.ends_with(".link"),
            size,
            mtime_ms: 1_000,
            id: (!is_dir).then(|| format!("id-{name}")),
            ..VfsMeta::default()
        }
    }
}

impl Backend for MemoryBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Webdav
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let prefix = format!("{}/", path.trim_end_matches('/'));
        let mut entries: BTreeMap<String, VfsMeta> = BTreeMap::new();
        for (file, content) in &self.files {
            let Some(rest) = file.strip_prefix(&prefix) else {
                continue;
            };
            let (name, is_dir) = match rest.split_once('/') {
                Some((folder, _)) => (folder, true),
                None => (rest, false),
            };
            entries.insert(
                name.to_string(),
                Self::meta(name, is_dir, content.len() as u64),
            );
        }
        if entries.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, path.to_string()));
        }
        Ok(entries.into_values().collect())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let name = path.rsplit('/').next().unwrap_or_default();
        match self.files.get(path) {
            Some(content) => Ok(Self::meta(name, false, content.len() as u64)),
            None => Ok(Self::meta(name, true, 0)),
        }
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.open_read_id(path, None)
    }

    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.opened
            .lock()
            .unwrap()
            .push((path.to_string(), id.map(str::to_string)));
        if path.ends_with("gone.txt") {
            return Err(io::Error::new(io::ErrorKind::NotFound, "verschwunden"));
        }
        let content = self.files.get(path).cloned().unwrap_or_default();
        Ok(Box::new(io::Cursor::new(content)))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn supports_content_search(&self) -> bool {
        self.own_search
    }

    fn content_search(
        &self,
        _root: &str,
        _query: &ContentQuery,
        tx: crossbeam_channel::Sender<ContentEvent>,
        _cancel: &AtomicBool,
    ) -> VfsResult<Option<ContentSummary>> {
        if !self.own_search {
            return Ok(None);
        }
        self.own_calls.fetch_add(1, Ordering::Relaxed);
        let _ = tx.send(ContentEvent::Progress {
            files: 7,
            bytes: 0,
            hits: 0,
        });
        Ok(Some(ContentSummary {
            files_searched: 7,
            ..ContentSummary::default()
        }))
    }
}

fn run(
    backend: &dyn Backend,
    root: &str,
    query: &ContentQuery,
) -> (Vec<ContentEvent>, ContentSummary) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let summary =
        content_search(backend, root, query, tx, &AtomicBool::new(false)).expect("search");
    (rx.try_iter().collect(), summary)
}

fn hit_files(events: &[ContentEvent]) -> Vec<String> {
    let mut files: Vec<String> = events
        .iter()
        .filter_map(|event| match event {
            ContentEvent::Hits { rel, .. } => Some(rel.clone()),
            _ => None,
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

#[test]
fn content_search_task_reads_files_through_a_backend() {
    let backend = MemoryBackend::new(false);
    let (events, summary) = run(&backend, "/data", &ContentQuery::text("NEEDLE"));
    assert_eq!(hit_files(&events), ["a.txt", "sub/b.md"]);
    assert!(events.iter().any(|event| matches!(
        event,
        ContentEvent::Skipped { rel, reason: SkipReason::Unreadable(text) }
            if rel == "gone.txt" && text.contains("verschwunden")
    )));
    assert_eq!(summary.files_matched, 2);
    assert_eq!(summary.hits, 2);
    assert_eq!(summary.skipped, 1);
    let opened = backend.opened.lock().unwrap().clone();
    assert!(opened.iter().all(|(path, _)| !path.ends_with(".link")));
    assert!(!opened.iter().any(|(path, _)| path.contains(".cache")));
    assert!(opened.contains(&("/data/a.txt".to_string(), Some("id-a.txt".to_string()))));

    // Hidden folders with "In hidden folders"; a filter and a folder expression.
    let mut query = ContentQuery::text("needle");
    query.hidden = true;
    query.filters = "*.txt".into();
    let (events, _) = run(&backend, "/data/", &query);
    assert_eq!(hit_files(&events), [".cache/d.txt", "a.txt"]);
    query.folder_exclude = Some(r"^\.cache$".into());
    let (events, _) = run(&backend, "/data", &query);
    assert_eq!(hit_files(&events), ["a.txt"]);

    // One file as the root.
    let mut regex = ContentQuery::text("^needle");
    regex.mode = TextMode::Regex;
    let (events, summary) = run(&backend, "/data/a.txt", &regex);
    assert_eq!(hit_files(&events), ["a.txt"]);
    assert_eq!(summary.files_searched, 1);

    // An invalid pattern fails before anything is read.
    backend.opened.lock().unwrap().clear();
    regex.pattern = "[".into();
    let (tx, _rx) = crossbeam_channel::unbounded();
    let error = content_search(&backend, "/data", &regex, tx, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(backend.opened.lock().unwrap().is_empty());
}

#[test]
fn content_search_task_backend_search_and_cache_pass_through() {
    let own = Arc::new(MemoryBackend::new(true));
    let (events, summary) = run(&*own, "/data", &ContentQuery::text("needle"));
    assert_eq!(summary.files_searched, 7);
    assert_eq!(events.len(), 1);
    assert!(own.opened.lock().unwrap().is_empty());

    let cached = CachingBackend::new(own.clone());
    assert!(cached.supports_content_search());
    let (_, summary) = run(&cached, "/data", &ContentQuery::text("needle"));
    assert_eq!(summary.files_searched, 7);
    assert_eq!(own.own_calls.load(Ordering::Relaxed), 2);

    let plain = CachingBackend::new(Arc::new(MemoryBackend::new(false)));
    assert!(!plain.supports_content_search());
    let (events, _) = run(&plain, "/data", &ContentQuery::text("needle"));
    assert_eq!(hit_files(&events), ["a.txt", "sub/b.md"]);
}
