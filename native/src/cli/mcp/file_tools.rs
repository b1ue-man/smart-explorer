//! Reading tools (`list`, `stat`, `read`) and the simple changes `mkdir` and
//! `remove`, over the same backends and guards as the terminal commands.
use super::args::Args;
use super::response;
use super::session::Session;
use super::spool::{looks_textual, text_page};
use crate::cli::entry_kind::EntryKind;
use crate::vfs::VfsMeta;
use base64::Engine as _;
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use std::io::Read;

/// Short fingerprint of a file state (size, time, provider id, md5). `write`
/// and `edit` refuse to replace a file whose fingerprint differs from the
/// caller's `if_match`.
pub(super) fn etag(meta: &VfsMeta) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!(
        "{}\0{}\0{}\0{}",
        meta.size,
        meta.mtime_ms,
        meta.id.as_deref().unwrap_or_default(),
        meta.content_md5.as_deref().unwrap_or_default()
    ));
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn kind(meta: &VfsMeta) -> &'static str {
    EntryKind::of(meta).label()
}

pub(super) fn meta_value(path: &str, meta: &VfsMeta) -> Value {
    json!({
        "path": path,
        "name": meta.name,
        "type": kind(meta),
        "size": meta.size,
        "mtime_ms": meta.mtime_ms,
        "id": meta.id,
        "md5": meta.content_md5,
        "etag": etag(meta),
    })
}

pub(super) fn list(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let target = session.targets.resolve(&args.string("target")?)?;
    let mut entries = target
        .backend
        .list_dir(&target.path)
        .map_err(|error| error.to_string())?;
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let total = entries.len();
    let offset = usize::try_from(args.opt_u64("offset")?.unwrap_or(0))
        .unwrap_or(usize::MAX)
        .min(total);
    let budget = response::page_budget(args.opt_u64("max_bytes")?);
    let mut used = 0usize;
    let mut page = Vec::new();
    for entry in &entries[offset..] {
        let value = json!({
            "name": entry.name,
            "type": kind(entry),
            "size": entry.size,
            "mtime_ms": entry.mtime_ms,
        });
        let cost = value.to_string().len() + 1;
        if !page.is_empty() && used + cost > budget {
            break;
        }
        used += cost;
        page.push(value);
    }
    let next = offset + page.len();
    Ok(response::ok(json!({
        "path": target.path,
        "total": total,
        "offset": offset,
        "entries": page,
        "next_offset": (next < total).then_some(next),
    })))
}

pub(super) fn stat(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let target = session.targets.resolve(&args.string("target")?)?;
    let meta = target
        .backend
        .stat(&target.path)
        .map_err(|error| error.to_string())?;
    Ok(response::ok(meta_value(&target.path, &meta)))
}

/// Reads a page of a file. Through a link the content is the file it points
/// to; its size is unknown (`size` and `etag` are null) and one byte more
/// than the page shows whether the file continues.
pub(super) fn read(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let target = session.targets.resolve(&args.string("target")?)?;
    let meta = target
        .backend
        .stat(&target.path)
        .map_err(|error| error.to_string())?;
    let through_link = match EntryKind::of(&meta) {
        EntryKind::Dir => return Err(format!("{} is a directory; use `list`", target.path)),
        EntryKind::Special => return Err(format!("{} is not a regular file", target.path)),
        EntryKind::Link => true,
        EntryKind::File => false,
    };
    let base64 = match args.opt_string("encoding")?.as_deref() {
        None | Some("text") => false,
        Some("base64") => true,
        Some(other) => return Err(format!("unknown encoding `{other}`; use text or base64")),
    };
    let budget = response::page_budget(args.opt_u64("max_bytes")?);
    // base64 grows the payload by a third.
    let budget = if base64 { budget / 4 * 3 } else { budget };
    let length = args
        .opt_u64("length")?
        .map(|length| usize::try_from(length).unwrap_or(usize::MAX))
        .unwrap_or(budget)
        .clamp(1, budget.max(1));
    let offset = args.opt_u64("offset")?.unwrap_or(0);
    let known_size = (!through_link).then_some(meta.size);
    let tag = (!through_link).then(|| etag(&meta));
    let mut bytes = if known_size.is_some_and(|size| offset >= size) {
        Vec::new()
    } else if through_link {
        read_range(&target, &meta, offset, length.saturating_add(1)).map_err(|error| {
            format!(
                "{} is a link whose target does not exist, is a directory, or cannot be read: {error}",
                target.path
            )
        })?
    } else {
        read_range(&target, &meta, offset, length)?
    };
    let more = through_link && bytes.len() > length;
    bytes.truncate(length);
    let end = offset.saturating_add(bytes.len() as u64);
    let at_end = match known_size {
        Some(size) => end >= size,
        None => !more,
    };
    let (content, next_offset, lossy) = if base64 {
        let content = base64::engine::general_purpose::STANDARD.encode(&bytes);
        (content, end, false)
    } else {
        if !looks_textual(&bytes) {
            return Ok(response::error_with(
                format!(
                    "{} looks binary; read it with encoding \"base64\" or copy it to a local path",
                    target.path
                ),
                json!({ "size": known_size, "etag": tag }),
            ));
        }
        let page = text_page(&bytes, offset, at_end);
        (page.text, page.next_offset, page.lossy)
    };
    let continues = match known_size {
        Some(size) => next_offset < size,
        None => more || next_offset < end,
    };
    Ok(response::ok(json!({
        "path": target.path,
        "size": known_size,
        "etag": tag,
        "through_link": through_link,
        "offset": offset,
        "encoding": if base64 { "base64" } else { "text" },
        "content": content,
        "next_offset": continues.then_some(next_offset),
        "lossy": lossy,
    })))
}

/// Up to `length` bytes from `offset`; backends without ranged reads are read
/// from the start and the skipped bytes discarded.
pub(super) fn read_range(
    target: &crate::cli::target::Target,
    meta: &VfsMeta,
    offset: u64,
    length: usize,
) -> Result<Vec<u8>, String> {
    let id = meta.id.as_deref();
    let reader = match target
        .backend
        .open_read_at(&target.path, id, offset)
        .map_err(|error| error.to_string())?
    {
        Some(reader) => reader,
        None => {
            let mut reader = target
                .backend
                .open_read_id(&target.path, id)
                .map_err(|error| error.to_string())?;
            let skipped = std::io::copy(&mut (&mut reader).take(offset), &mut std::io::sink())
                .map_err(|error| error.to_string())?;
            if skipped < offset {
                return Ok(Vec::new());
            }
            reader
        }
    };
    let mut bytes = Vec::new();
    reader
        .take(length as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

pub(super) fn mkdir(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let target = session.targets.resolve(&args.string("target")?)?;
    let existed = target
        .backend
        .try_exists(&target.path)
        .map_err(|error| error.to_string())?;
    target
        .backend
        .mkdir_all(&target.path)
        .map_err(|error| error.to_string())?;
    Ok(response::ok(json!({
        "path": target.path,
        "created": !existed,
    })))
}

pub(super) fn remove(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let target = session.targets.resolve(&args.string("target")?)?;
    // The tool call is the explicit request `se rm` needs `--force` for.
    crate::cli::ops::remove(
        &target,
        args.flag("recursive")?,
        true,
        args.flag("allow_root")?,
    )?;
    Ok(response::ok(
        json!({ "path": target.path, "removed": true }),
    ))
}

#[cfg(test)]
mod tests {
    use super::{etag, kind};
    use crate::vfs::VfsMeta;

    #[test]
    fn mcp_task_etag_changes_with_any_file_fact() {
        let meta = VfsMeta {
            name: "a.txt".into(),
            size: 3,
            mtime_ms: 10,
            ..Default::default()
        };
        let mut changed = meta.clone();
        changed.mtime_ms = 11;
        assert_eq!(etag(&meta).len(), 16);
        assert_eq!(etag(&meta), etag(&meta.clone()));
        assert_ne!(etag(&meta), etag(&changed));
        assert_eq!(kind(&meta), "file");
    }
}
