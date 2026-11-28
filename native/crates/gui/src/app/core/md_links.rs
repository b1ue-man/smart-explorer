//! Link and image targets of a rendered Markdown file: web links (http,
//! https, mailto), anchors inside the document and paths on the place of the
//! document, relative to its folder or absolute (`/…`, `C:/…`, `file://`).
//! Free of `egui`.

/// Where a link points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum LinkTarget {
    /// http, https or mailto: the browser.
    Web(String),
    /// A file on the place of the document (resolved, `/`-separated).
    File(String),
    /// `#heading` inside the document.
    Anchor(String),
    /// Any other scheme (never followed).
    Unsupported,
}

fn is_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn has_scheme(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return false;
    };
    scheme.len() > 1
        && scheme.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn without_fragment(target: &str) -> &str {
    target.split(['#', '?']).next().unwrap_or(target)
}

/// `.` and `..` resolved; `..` never climbs above the root (`/`, `C:/`, `//`).
pub(in crate::app) fn normalize_path(path: &str) -> String {
    let (prefix, rest) = if let Some(rest) = path.strip_prefix("//") {
        ("//".to_string(), rest)
    } else if is_drive(path) {
        (
            format!("{}/", &path[..2]),
            path[2..].trim_start_matches('/'),
        )
    } else if let Some(rest) = path.strip_prefix('/') {
        ("/".to_string(), rest)
    } else {
        (String::new(), path)
    };
    let mut parts: Vec<&str> = Vec::new();
    for segment in rest.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            segment => parts.push(segment),
        }
    }
    format!("{prefix}{}", parts.join("/"))
}

/// Resolves a link or image target of a Markdown file in `base_dir`.
pub(in crate::app) fn resolve_link(base_dir: &str, target: &str) -> LinkTarget {
    let target = target.trim();
    if target.is_empty() {
        return LinkTarget::Unsupported;
    }
    if let Some(anchor) = target.strip_prefix('#') {
        return LinkTarget::Anchor(anchor.to_string());
    }
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return LinkTarget::Web(target.to_string());
    }
    if lower.starts_with("file://") {
        let rest = percent_decode(without_fragment(&target[7..]));
        // file:///C:/x → C:/x, file:///home/x → /home/x
        let rest = match rest.strip_prefix('/') {
            Some(drive) if is_drive(drive) => drive.to_string(),
            _ => rest,
        };
        return LinkTarget::File(normalize_path(&rest));
    }
    if has_scheme(target) && !is_drive(target) {
        return LinkTarget::Unsupported;
    }
    let mut path = percent_decode(without_fragment(target));
    if is_drive(base_dir) || is_drive(&path) {
        path = path.replace('\\', "/");
    }
    if path.starts_with('/') || is_drive(&path) {
        return LinkTarget::File(normalize_path(&path));
    }
    let joined = if base_dir.is_empty() {
        path
    } else {
        format!("{}/{}", base_dir.trim_end_matches('/'), path)
    };
    LinkTarget::File(normalize_path(&joined))
}

/// The folder of a file path (`/`-separated).
pub(in crate::app) fn parent_dir(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

/// GitHub's anchor of a heading: lowercase, spaces as `-`, only letters,
/// digits, `-` and `_` kept.
pub(in crate::app) fn heading_slug(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|ch| match ch {
            ' ' => Some('-'),
            ch if ch.is_alphanumeric() || ch == '-' || ch == '_' => Some(ch),
            _ => None,
        })
        .collect()
}
