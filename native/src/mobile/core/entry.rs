//! The `Entry` JSON type (api.md §2): name, location, type class and the
//! warning for names Windows cannot address.
use super::location::{join, Loc};
use crate::types::{win32_name_issue, FileEntry};
use crate::vfs::VfsMeta;
use serde_json::{json, Value};
use std::sync::Arc;

/// Lower-case extension of a file name (empty for folders and dot files).
pub(crate) fn extension(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(index) if index > 0 && index + 1 < name.len() => name[index + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// `dir|image|video|audio|text|archive|document|apk|other`.
pub(crate) fn kind_of(ext: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "dir";
    }
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "bmp" | "svg" | "tif"
        | "tiff" | "avif" | "ico" | "dng" => "image",
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "3gp" | "wmv" | "flv" | "mpg" | "mpeg"
        | "ts" => "video",
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "wma" | "mid" | "midi"
        | "amr" => "audio",
        "txt" | "md" | "log" | "csv" | "json" | "xml" | "yml" | "yaml" | "ini" | "conf" | "cfg"
        | "toml" | "rs" | "kt" | "java" | "py" | "js" | "html" | "htm" | "css" | "sh" | "c"
        | "h" | "cpp" | "tsv" | "srt" => "text",
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" => "archive",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
        | "rtf" | "epub" => "document",
        "apk" => "apk",
        _ => "other",
    }
}

/// MIME type for opening a file with another app.
pub(crate) fn mime_of(name: &str) -> &'static str {
    match extension(name, false).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "3gp" => "video/3gpp",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "m4a" | "aac" => "audio/mp4",
        "mid" | "midi" => "audio/midi",
        "txt" | "log" | "ini" | "conf" | "cfg" | "toml" | "rs" | "kt" | "java" | "py" | "sh"
        | "c" | "h" | "cpp" | "srt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "json" => "application/json",
        "xml" => "text/xml",
        "yml" | "yaml" => "application/yaml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "text/javascript",
        "zip" => "application/zip",
        "rar" => "application/vnd.rar",
        "7z" => "application/x-7z-compressed",
        "tar" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "rtf" => "application/rtf",
        "epub" => "application/epub+zip",
        "apk" => "application/vnd.android.package-archive",
        _ => "application/octet-stream",
    }
}

/// The warning text for a name Windows cannot address, if any.
pub(crate) fn problem_of(name: &str) -> Option<&'static str> {
    win32_name_issue(name).map(|issue| issue.label_de())
}

/// A listing row as the desktop `FileEntry` (for filters and sorting).
pub(crate) fn file_entry(parent: &str, meta: &VfsMeta, depth: u32) -> FileEntry {
    FileEntry {
        path: Arc::from(join(parent, &meta.name).as_str()),
        parent: Arc::from(parent),
        name: Arc::from(meta.name.as_str()),
        ext: Arc::from(extension(&meta.name, meta.is_dir).as_str()),
        size: if meta.is_dir { 0 } else { meta.size },
        mtime_ms: meta.mtime_ms,
        btime_ms: meta.btime_ms,
        is_dir: meta.is_dir,
        is_symlink: meta.is_symlink,
        hidden: meta.hidden || meta.name.starts_with('.'),
        system: meta.system,
        depth,
        id: meta.id.as_deref().map(Arc::from),
    }
}

/// Tree placement of a row in scan views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TreeInfo {
    pub depth: u32,
    pub has_children: bool,
    pub expanded: bool,
}

/// The `Entry` JSON for a row of `base`'s connection.
pub(crate) fn entry_json(base: &Loc, entry: &FileEntry, tree: TreeInfo) -> Value {
    let ext = extension(&entry.name, entry.is_dir);
    json!({
        "name": entry.name.as_ref(),
        "location": base.at(&entry.path),
        "isDir": entry.is_dir,
        "isLink": entry.is_symlink,
        "size": entry.size,
        "mtimeMs": entry.mtime_ms,
        "hidden": entry.hidden,
        "problem": problem_of(&entry.name),
        "kind": kind_of(&ext, entry.is_dir),
        "ext": ext,
        "depth": tree.depth,
        "hasChildren": tree.has_children,
        "expanded": tree.expanded,
    })
}
