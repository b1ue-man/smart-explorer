use crate::filter::CompiledFilter;
use crate::types::{FileEntry, FilterDef};
use std::collections::HashSet;
use std::sync::{atomic::AtomicBool, Arc};

const MAX_TRANSFER_NODES: u64 = 1_000_000;
const MAX_TRANSFER_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TRANSFER_DEPTH: usize = 512;
const MAX_DISPLAYED_ERRORS: usize = 100;
const MAX_ERROR_TEXT_BYTES: usize = 4 * 1024;

#[derive(Default)]
pub(super) struct TransferCollectionBudget {
    nodes: u64,
    text_bytes: u64,
}

impl TransferCollectionBudget {
    fn next_text_total(&self, text: &[&str]) -> Result<u64, String> {
        let added = text
            .iter()
            .try_fold(0u64, |total, value| total.checked_add(value.len() as u64));
        added
            .and_then(|added| self.text_bytes.checked_add(added))
            .filter(|total| *total <= MAX_TRANSFER_TEXT_BYTES)
            .ok_or_else(|| {
                format!(
                    "Maximale Pfad-/Namenmenge von {} MiB überschritten",
                    MAX_TRANSFER_TEXT_BYTES / (1024 * 1024)
                )
            })
    }

    pub(super) fn ensure_text_fits(&self, text: &[&str]) -> Result<(), String> {
        self.next_text_total(text).map(|_| ())
    }

    pub(super) fn record_text(&mut self, text: &[&str]) -> Result<(), String> {
        self.text_bytes = self.next_text_total(text)?;
        Ok(())
    }

    pub(super) fn record_node(&mut self, depth: usize, text: &[&str]) -> Result<(), String> {
        if depth > MAX_TRANSFER_DEPTH {
            return Err(format!(
                "Maximale Verzeichnistiefe von {MAX_TRANSFER_DEPTH} überschritten"
            ));
        }
        if self.nodes >= MAX_TRANSFER_NODES {
            return Err(format!(
                "Maximale Anzahl von {MAX_TRANSFER_NODES} Einträgen überschritten"
            ));
        }
        self.record_text(text)?;
        self.nodes += 1;
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct TransferErrorLog {
    displayed: Vec<String>,
    total: u64,
}

impl TransferErrorLog {
    pub(super) fn push(&mut self, message: impl Into<String>) {
        self.total = self.total.saturating_add(1);
        if self.displayed.len() < MAX_DISPLAYED_ERRORS {
            self.displayed.push(bounded_error_text(message.into()));
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub(super) fn total(&self) -> u64 {
        self.total
    }

    pub(super) fn into_displayed(mut self) -> Vec<String> {
        if self.total > self.displayed.len() as u64 {
            let suppressed = self.total - self.displayed.len() as u64;
            let suffix = format!(" [… {suppressed} weitere Fehler unterdrückt]");
            if let Some(first) = self.displayed.first_mut() {
                let max_prefix = MAX_ERROR_TEXT_BYTES.saturating_sub(suffix.len());
                let mut end = first.len().min(max_prefix);
                while !first.is_char_boundary(end) {
                    end -= 1;
                }
                first.truncate(end);
                first.push_str(&suffix);
            }
        }
        self.displayed
    }
}

fn bounded_error_text(mut message: String) -> String {
    if message.len() <= MAX_ERROR_TEXT_BYTES {
        return message;
    }
    let mut end = MAX_ERROR_TEXT_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message.push('…');
    message
}

pub(super) fn validate_transfer_name(name: &str, context: &str) -> Result<(), String> {
    crate::vfs::validate_child_name(name).map_err(|error| format!("{context}: {error}"))
}

pub(super) struct RemoteFileEntry {
    pub(super) src: String,
    pub(super) rel: String,
    pub(super) size: u64,
}

pub(super) struct RemoteFilterCtx {
    cf: CompiledFilter,
    filter: FilterDef,
    root_prefix: String,
}

impl RemoteFilterCtx {
    fn new(filter: FilterDef, root_prefix: String) -> Self {
        Self {
            cf: CompiledFilter::compile(&filter),
            filter,
            root_prefix: root_prefix.trim_end_matches('/').to_string(),
        }
    }

    fn depth_for(&self, path: &str) -> u32 {
        let path = path.trim_end_matches('/');
        let root = self.root_prefix.as_str();
        if path == root {
            return 0;
        }
        let rel = if root.is_empty() {
            path.trim_start_matches('/')
        } else {
            path.strip_prefix(root)
                .unwrap_or(path)
                .trim_start_matches('/')
        };
        rel.split('/').filter(|s| !s.is_empty()).count() as u32
    }

    fn matches(&self, e: &FileEntry) -> bool {
        self.cf.matches(e, &self.root_prefix)
    }

    fn allows_dir_descendants(&self, e: &FileEntry) -> bool {
        self.filter.include_dirs
            && (!e.hidden || self.filter.include_hidden)
            && (!e.system || self.filter.include_system)
    }
}

pub(super) fn compile_remote_filter(
    filter: Option<(FilterDef, String)>,
) -> Option<RemoteFilterCtx> {
    filter.map(|(filter, root_prefix)| RemoteFilterCtx::new(filter, root_prefix))
}

fn remote_ext_of(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() && i > 0 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

fn remote_parent(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn remote_file_entry(
    path: &str,
    parent: &str,
    meta: &crate::vfs::VfsMeta,
    depth: u32,
) -> FileEntry {
    FileEntry {
        path: Arc::from(path),
        parent: Arc::from(parent),
        name: Arc::from(meta.name.as_str()),
        ext: Arc::from(remote_ext_of(&meta.name, meta.is_dir).as_str()),
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        btime_ms: meta.btime_ms,
        is_dir: meta.is_dir,
        is_symlink: meta.is_symlink,
        hidden: meta.hidden,
        system: meta.system,
        depth,
        id: meta.id.as_deref().map(Arc::from),
    }
}

pub(super) struct RemoteEntryCollector<'a> {
    pub(super) be: &'a dyn crate::vfs::Backend,
    pub(super) filter: Option<&'a RemoteFilterCtx>,
    pub(super) files: &'a mut Vec<RemoteFileEntry>,
    pub(super) dirs: &'a mut Vec<String>,
    pub(super) budget: &'a mut TransferCollectionBudget,
    pub(super) cancel: Option<&'a AtomicBool>,
}

impl RemoteEntryCollector<'_> {
    pub(super) fn collect(
        &mut self,
        src: &str,
        rel: String,
        selected_root: bool,
    ) -> Result<(), String> {
        super::cancel::check_optional(self.cancel)?;
        let meta = self
            .be
            .stat(src)
            .map_err(|error| format!("{src}: {error}"))?;
        super::cancel::check_optional(self.cancel)?;
        self.collect_with_meta(src, rel, selected_root, meta, 0)
    }

    pub(super) fn collect_with_meta(
        &mut self,
        src: &str,
        rel: String,
        selected_root: bool,
        meta: crate::vfs::VfsMeta,
        depth: usize,
    ) -> Result<(), String> {
        super::cancel::check_optional(self.cancel)?;
        self.budget
            .record_node(depth, &[src, &rel, &meta.name])
            .map_err(|error| format!("{src}: {error}"))?;
        if !meta.name.is_empty() {
            validate_transfer_name(&meta.name, src)?;
        } else if !selected_root {
            return Err(format!("{src}: Backend lieferte einen leeren Namen"));
        }
        if meta.is_symlink {
            return Err(format!(
                "{src}: Links und Reparse-Punkte werden nicht übertragen"
            ));
        }
        if meta.is_dir {
            if self.filter.is_none() {
                self.dirs.push(rel.clone());
            }
            if let Some(ctx) = self.filter {
                let parent = remote_parent(src);
                let entry = remote_file_entry(src, &parent, &meta, ctx.depth_for(src));
                if !selected_root && !ctx.allows_dir_descendants(&entry) {
                    return Ok(());
                }
            }
            super::cancel::check_optional(self.cancel)?;
            let entries = self
                .be
                .list_dir(src)
                .map_err(|error| format!("{src}: {error}"))?;
            super::cancel::check_optional(self.cancel)?;
            let mut child_names = HashSet::with_capacity(entries.len().min(4096));
            for entry in entries {
                super::cancel::check_optional(self.cancel)?;
                validate_transfer_name(&entry.name, src)?;
                self.budget
                    .ensure_text_fits(&[&entry.name])
                    .map_err(|error| format!("{src}: {error}"))?;
                if !child_names.insert(entry.name.clone()) {
                    return Err(format!(
                        "{src}: Backend lieferte den Namen {:?} mehrfach",
                        entry.name
                    ));
                }
                if entry.is_symlink {
                    return Err(format!(
                        "{src}/{}: Links und Reparse-Punkte werden nicht übertragen",
                        entry.name
                    ));
                }
                let child_src = format!("{}/{}", src.trim_end_matches('/'), entry.name);
                let child_rel = if rel.is_empty() {
                    entry.name
                } else {
                    format!("{}/{}", rel, entry.name)
                };
                let child_meta = self
                    .be
                    .stat(&child_src)
                    .map_err(|error| format!("{child_src}: {error}"))?;
                super::cancel::check_optional(self.cancel)?;
                self.collect_with_meta(&child_src, child_rel, false, child_meta, depth + 1)?;
            }
        } else if selected_root
            || self
                .filter
                .map(|ctx| {
                    let parent = remote_parent(src);
                    let entry = remote_file_entry(src, &parent, &meta, ctx.depth_for(src));
                    ctx.matches(&entry)
                })
                .unwrap_or(true)
        {
            self.files.push(RemoteFileEntry {
                src: src.to_string(),
                rel,
                size: meta.size,
            });
        }
        Ok(())
    }
}
