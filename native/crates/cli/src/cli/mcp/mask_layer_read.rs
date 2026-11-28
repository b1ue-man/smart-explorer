//! `read` while a mask list is active (plan 2026-11-28, S2): the whole file
//! is masked before it is cut into pages, so a value that runs over a page
//! boundary (a multi-line pattern, „Schweins-⏎braten“) is still found
//! (docs/refs/maskierung-fuzzy-ersetzen-2026-11-28.md §3.6). The masked file
//! is kept per target, file state (etag) and active lists, so the following
//! pages do not read and mask it again.
//!
//! Text and base64 pages count bytes of the masked text (`size`, `offset`,
//! `next_offset`); a placeholder carries the line ends of the value it
//! stands for, so line numbers of `line`/`lines` stay those of the file.
use super::args::Args;
use super::file_tools::etag;
use super::line_read;
use super::mask_layer::{mask_text, mask_value};
use super::response;
use super::session::Session;
use super::spool::{looks_textual, text_page};
use super::targets::Resolved;
use crate::masks::{MaskSet, UnmaskError};
use crate::vfs::VfsMeta;
use base64::Engine as _;
use rmcp::model::CallToolResult;
use serde_json::json;
use std::collections::VecDeque;
use std::io::Read;
use std::sync::{Arc, Mutex};

/// Masked files kept for the following pages: at most this many…
const KEEP_FILES: usize = 8;
/// …and at most this many bytes together (the newest file always stays).
const KEEP_BYTES: usize = 256 * 1024 * 1024;

struct Kept {
    /// Absolute target spelling and etag of the file state.
    key: String,
    set: Arc<MaskSet>,
    masked: Arc<Vec<u8>>,
}

#[derive(Default)]
pub(super) struct ReadCache {
    files: Mutex<VecDeque<Kept>>,
}

impl ReadCache {
    fn get(&self, key: &str, set: &Arc<MaskSet>) -> Option<Arc<Vec<u8>>> {
        let files = self.files.lock().ok()?;
        files
            .iter()
            .find(|kept| kept.key == key && Arc::ptr_eq(&kept.set, set))
            .map(|kept| kept.masked.clone())
    }

    fn keep(&self, key: String, set: Arc<MaskSet>, masked: Arc<Vec<u8>>) {
        let Ok(mut files) = self.files.lock() else {
            return;
        };
        files.retain(|kept| kept.key != key);
        files.push_front(Kept { key, set, masked });
        let mut total = 0usize;
        let mut keep = 0usize;
        for kept in files.iter() {
            total = total.saturating_add(kept.masked.len());
            if keep > 0 && (keep >= KEEP_FILES || total > KEEP_BYTES) {
                break;
            }
            keep += 1;
        }
        files.truncate(keep);
    }
}

/// The masked whole file of `target`.
fn masked_file(
    session: &Session,
    set: &Arc<MaskSet>,
    target: &Resolved,
    meta: &VfsMeta,
    through_link: bool,
) -> Result<Arc<Vec<u8>>, String> {
    // A link has no etag of its own: its file is read again for each page.
    let key = (!through_link).then(|| format!("{}\0{}", target.spec, etag(meta)));
    if let Some(key) = &key {
        if let Some(masked) = session.masks.reads.get(key, set) {
            return Ok(masked);
        }
    }
    let mut reader = target
        .backend
        .open_read_id(&target.path, meta.id.as_deref())
        .map_err(|error| {
            let text = session.targets.io_error(target, error);
            if through_link {
                format!(
                    "{} is a link whose target does not exist, is a directory, or cannot be read: {text}",
                    target.path
                )
            } else {
                text
            }
        })?;
    let mut bytes = Vec::new();
    if !through_link {
        let size = usize::try_from(meta.size)
            .map_err(|_| format!("{} is too large to mask in memory", target.path))?;
        bytes
            .try_reserve_exact(size)
            .map_err(|_| format!("{} is too large to mask in memory", target.path))?;
    }
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| session.targets.io_error(target, error))?;
    let masked = Arc::new(set.mask_bytes(&bytes));
    if let Some(key) = key {
        session.masks.reads.keep(key, set.clone(), masked.clone());
    }
    Ok(masked)
}

/// `read` of `target` with the active lists `set` (not empty).
pub(super) fn read(
    session: &Session,
    set: &Arc<MaskSet>,
    target: &Resolved,
    meta: &VfsMeta,
    through_link: bool,
    args: &Args<'_>,
) -> Result<CallToolResult, String> {
    let lines = args.value("line").is_some() || args.value("lines").is_some();
    // Arguments are checked before the file is read.
    let request = if lines {
        Some(line_read::request(args)?)
    } else {
        None
    };
    let base64 = match args.opt_string("encoding")?.as_deref() {
        None | Some("text") => false,
        Some("base64") => true,
        Some(other) => return Err(format!("unknown encoding `{other}`; use text or base64")),
    };
    let masked = masked_file(session, set, target, meta, through_link)?;
    let tag = (!through_link).then(|| etag(meta));
    page(
        set,
        &masked,
        &target.path,
        tag,
        through_link,
        request,
        base64,
        args,
    )
}

/// One page of the masked file `masked` of `path` (`tag`: its etag).
#[allow(clippy::too_many_arguments)]
pub(super) fn page(
    set: &MaskSet,
    masked: &[u8],
    path: &str,
    tag: Option<String>,
    through_link: bool,
    request: Option<line_read::LineRequest>,
    base64: bool,
    args: &Args<'_>,
) -> Result<CallToolResult, String> {
    let known_size = (!through_link).then_some(masked.len() as u64);
    let binary = || {
        let mut details = json!({ "size": known_size, "etag": tag });
        mask_value(set, &mut details);
        response::masked_error_with(mask_text(set, &line_read::binary_text(path)), details)
    };
    if let Some(request) = request {
        let mut page = line_read::collect(
            std::io::Cursor::new(masked),
            request.first,
            request.wanted,
            request.budget,
        )
        .map_err(|error| error.to_string())?;
        if page.binary {
            return Ok(binary());
        }
        // Every field but the content (masked already) is masked here.
        let content = std::mem::take(&mut page.content);
        let mut value = line_read::page_value(
            path,
            known_size,
            tag.clone(),
            through_link,
            request.first,
            page,
        );
        value["masked"] = json!(true);
        mask_value(set, &mut value);
        value["content"] = json!(content);
        return Ok(response::masked(value));
    }
    let budget = response::page_budget(args.opt_u64("max_bytes")?);
    // base64 grows the payload by a third.
    let budget = if base64 { budget / 4 * 3 } else { budget };
    let length = args
        .opt_u64("length")?
        .map(|length| usize::try_from(length).unwrap_or(usize::MAX))
        .unwrap_or(budget)
        .clamp(1, budget.max(1));
    let total = masked.len();
    let offset = args.opt_u64("offset")?.unwrap_or(0);
    let start = usize::try_from(offset).unwrap_or(usize::MAX).min(total);
    let end = start.saturating_add(length).min(total);
    let bytes = &masked[start..end];
    let at_end = end >= total;
    let (content, next_offset, lossy) = if base64 {
        let content = base64::engine::general_purpose::STANDARD.encode(bytes);
        (content, end as u64, false)
    } else {
        if !looks_textual(bytes) {
            return Ok(binary());
        }
        let page = text_page(bytes, offset, at_end);
        (page.text, page.next_offset, page.lossy)
    };
    let mut value = json!({
        "path": path,
        "size": known_size,
        "etag": tag,
        "through_link": through_link,
        "offset": offset,
        "encoding": if base64 { "base64" } else { "text" },
        "next_offset": (next_offset < total as u64).then_some(next_offset),
        "lossy": lossy,
        "masked": true,
    });
    mask_value(set, &mut value);
    value["content"] = json!(content);
    Ok(response::masked(value))
}

/// Bytes the agent wrote as base64 with their placeholders restored, the
/// counterpart of the masked base64 pages of `read`.
pub(super) fn restore_bytes(set: &MaskSet, bytes: Vec<u8>) -> Result<Vec<u8>, UnmaskError> {
    if set.is_empty() {
        return Ok(bytes);
    }
    set.unmask_bytes(&bytes)
}
