use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: Arc<str>,
    pub parent: Arc<str>,
    pub name: Arc<str>,
    pub ext: Arc<str>,
    pub size: u64,
    pub mtime_ms: i64,
    pub btime_ms: i64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub hidden: bool,
    pub system: bool,
    pub depth: u32,
    /// Backend-unique id (Google Drive file-id) when the name isn't unique;
    /// None for local/SFTP/FTP/WebDAV where the path identifies the item.
    pub id: Option<Arc<str>>,
}

impl FileEntry {
    pub fn full_path(&self) -> PathBuf {
        PathBuf::from(self.path.as_ref())
    }

    /// A selection key that uniquely identifies this row. Equals `path` for
    /// normal entries; for backends that allow duplicate names in one folder
    /// (Google Drive), it appends the backend id so each duplicate selects
    /// independently. Use `sel_key_path` to recover the path from a key.
    pub fn key(&self) -> Arc<str> {
        match &self.id {
            Some(id) => Arc::from(format!("{}\u{1f}{}", self.path, id)),
            None => self.path.clone(),
        }
    }
}

/// The filesystem path encoded in a selection key (strips any `\u{1f}<id>`).
pub fn sel_key_path(key: &str) -> &str {
    key.split('\u{1f}').next().unwrap_or(key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Path,
    Size,
    Mtime,
    Btime,
    Ext,
    Depth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMode {
    Substring,
    Regex,
    Glob,
}

#[derive(Clone, Debug)]
pub struct Range<T> {
    pub min: Option<T>,
    pub max: Option<T>,
}

impl<T> Default for Range<T> {
    fn default() -> Self {
        Self { min: None, max: None }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FilterDef {
    pub text: String,
    pub text_mode: TextMode,
    pub extensions: Vec<String>,
    pub size: Range<u64>,
    pub mtime: Range<i64>,
    pub btime: Range<i64>,
    pub depth: Range<u32>,
    pub include_files: bool,
    pub include_dirs: bool,
    pub include_hidden: bool,
    pub include_system: bool,
}

impl FilterDef {
    pub fn new() -> Self {
        // Default = pass everything. Hidden and system files are shown by
        // default — the user can uncheck them if they want a cleaner view.
        // This makes "no filter" actually mean "no filter".
        Self {
            include_files: true,
            include_dirs: true,
            include_hidden: true,
            include_system: true,
            ..Default::default()
        }
    }
}

impl Default for TextMode {
    fn default() -> Self {
        TextMode::Substring
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyMode {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    Skip,
    Overwrite,
    Rename,
}

#[derive(Clone, Debug)]
pub struct CopyOptions {
    pub root: PathBuf,
    pub dest: PathBuf,
    pub preserve_structure: bool,
    pub conflict: Conflict,
    pub mode: CopyMode,
}

#[derive(Clone, Debug)]
pub struct ScanProgress {
    pub scanned: u64,
    pub bytes: u64,
    pub errors: u64,
    pub elapsed_ms: u64,
    pub current_path: String,
    pub done: bool,
}

#[derive(Clone, Debug)]
pub struct CopyProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub elapsed_ms: u64,
    pub current_path: String,
    pub errors: u64,
    pub done: bool,
}
