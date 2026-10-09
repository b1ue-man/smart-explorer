//! Media classification by file extension, shared by the Android facade
//! (`Entry.kind`) and the desktop open path, which hands image, video and audio
//! files to their viewer together with the neighboring files of their folder.
//! Pure string logic; no file is read.

/// Kind of a media file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    /// The `Entry.kind` word of the Android contract (api.md §2).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

/// Media kind of a lower- or mixed-case extension without the dot.
pub fn media_kind_of_ext(ext: &str) -> Option<MediaKind> {
    let ext = ext.to_ascii_lowercase();
    Some(match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "bmp" | "svg" | "tif"
        | "tiff" | "avif" | "ico" | "dng" => MediaKind::Image,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "3gp" | "wmv" | "flv" | "mpg" | "mpeg"
        | "ts" => MediaKind::Video,
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "wma" | "mid" | "midi"
        | "amr" => MediaKind::Audio,
        _ => return None,
    })
}

/// Media kind of a file name or path (the text after its last dot; a leading
/// dot alone, as in `.jpg`, is a hidden name without extension).
pub fn media_kind_of_name(name: &str) -> Option<MediaKind> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match file.rfind('.') {
        Some(index) if index > 0 && index + 1 < file.len() => media_kind_of_ext(&file[index + 1..]),
        _ => None,
    }
}
