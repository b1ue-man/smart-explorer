use crate::filter::CompiledFilter;
use crate::types::{FileEntry, FilterDef};
use std::sync::Arc;

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
    pub(super) errors: &'a mut Vec<String>,
}

impl RemoteEntryCollector<'_> {
    pub(super) fn collect(&mut self, src: &str, rel: String, selected_root: bool) {
        let meta = match self.be.stat(src) {
            Ok(m) => m,
            Err(e) => {
                self.errors.push(format!("{}: {}", src, e));
                return;
            }
        };
        if meta.is_dir {
            if self.filter.is_none() {
                self.dirs.push(rel.clone());
            }
            if let Some(ctx) = self.filter {
                let parent = remote_parent(src);
                let entry = remote_file_entry(src, &parent, &meta, ctx.depth_for(src));
                if !selected_root && !ctx.allows_dir_descendants(&entry) {
                    return;
                }
            }
            let entries = match self.be.list_dir(src) {
                Ok(entries) => entries,
                Err(e) => {
                    self.errors.push(format!("{}: {}", src, e));
                    return;
                }
            };
            for entry in entries {
                let child_src = format!("{}/{}", src.trim_end_matches('/'), entry.name);
                let child_rel = if rel.is_empty() {
                    entry.name
                } else {
                    format!("{}/{}", rel, entry.name)
                };
                self.collect(&child_src, child_rel, false);
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
    }
}
