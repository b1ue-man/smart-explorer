//! Names of the private stages, spools and quarantines this app writes next
//! to user files.

const UNIQUE_MARKER: &str = ".se-";
const DOTTED_MARKER: &str = ".smart-explorer";
/// Longest name component every supported filesystem stores: 255 bytes
/// (ext4, f2fs, Android storage) and 255 UTF-16 units (NTFS, exFAT, FAT).
const MAX_COMPONENT: usize = 255;

/// `lead + name + tail` as one storable name component: `name` is shortened
/// on a character boundary until the whole fits 255 bytes and 255 UTF-16
/// units; at least one character of it stays, and names that fit are kept.
pub(crate) fn fit_stage_name(lead: &str, name: &str, tail: &str) -> String {
    let mut bytes = lead.len() + tail.len();
    let mut units = lead.encode_utf16().count() + tail.encode_utf16().count();
    let mut end = 0;
    for (index, character) in name.char_indices() {
        bytes += character.len_utf8();
        units += character.len_utf16();
        if end > 0 && (bytes > MAX_COMPONENT || units > MAX_COMPONENT) {
            break;
        }
        end = index + character.len_utf8();
    }
    format!("{lead}{}{tail}", &name[..end])
}

/// Whether `name` is a stage, spool or quarantine name this app creates next
/// to user files: `unique_staging_path` (`<file>.se-<purpose>-<16 hex>`),
/// copy and transfer stages (`.<file>.smart-explorer[-…].part`), move
/// quarantines (`….move`), agent batches and spools, upload parts. A walk
/// reports such an entry as the app's own file instead of syncing it, so a
/// stage left by a crash never becomes a user file.
pub fn is_staging_name(name: &str) -> bool {
    is_unique_stage(name)
        || is_dotted_stage(name)
        || is_part_stage(name)
        || is_agent_spool(name)
        || is_private_stage(name)
        || is_replacement_stage(name)
}

/// Recovery siblings are independent of the user's filename length.
fn is_replacement_stage(name: &str) -> bool {
    name.strip_prefix(".se-replace-")
        .is_some_and(|nonce| nonce.len() == 16 && lower_hex(nonce))
}

/// Exclusive owner-private support-dir stages, never generic user `.tmp`.
fn is_private_stage(name: &str) -> bool {
    name.strip_prefix(".se-private-")
        .and_then(|rest| rest.strip_suffix(".tmp"))
        .is_some_and(|nonce| nonce.len() == 32 && lower_hex(nonce))
}

fn lower_hex(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `<file>.se-<purpose>-<16 hex>`; `<file>` may be shortened, never empty.
fn is_unique_stage(name: &str) -> bool {
    let Some((head, suffix)) = name.rsplit_once('-') else {
        return false;
    };
    let Some(marker) = head.rfind(UNIQUE_MARKER) else {
        return false;
    };
    let purpose = &head[marker + UNIQUE_MARKER.len()..];
    marker > 0
        && suffix.len() == 16
        && lower_hex(suffix)
        && !purpose.is_empty()
        && purpose
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `.<file>.smart-explorer.part` and `.<file>.smart-explorer-<hex>[-<hex>…]`
/// with `.part` or `.move`.
fn is_dotted_stage(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some(body) = rest
        .strip_suffix(".part")
        .or_else(|| rest.strip_suffix(".move"))
    else {
        return false;
    };
    let Some(marker) = body.rfind(DOTTED_MARKER) else {
        return false;
    };
    let ids = &body[marker + DOTTED_MARKER.len()..];
    marker > 0
        && (ids.is_empty()
            || ids
                .strip_prefix('-')
                .is_some_and(|ids| ids.split('-').all(lower_hex)))
}

/// `<file>.se-agent-batch-<hex>-<hex>.part` and `<file>.se-upload-<pid>-<hex>.part`.
fn is_part_stage(name: &str) -> bool {
    let Some(body) = name.strip_suffix(".part") else {
        return false;
    };
    let Some(marker) = body.rfind(UNIQUE_MARKER) else {
        return false;
    };
    let tail = &body[marker + UNIQUE_MARKER.len()..];
    let ids = tail
        .strip_prefix("agent-batch-")
        .or_else(|| tail.strip_prefix("upload-"));
    marker > 0
        && ids.is_some_and(|ids| {
            let mut parts = ids.split('-');
            matches!(
                (parts.next(), parts.next(), parts.next()),
                (Some(first), Some(second), None) if lower_hex(first) && lower_hex(second)
            )
        })
}

/// `.se-agent-<purpose>-<hex>-<hex>-<hex>.spool` (agent tree spools).
fn is_agent_spool(name: &str) -> bool {
    let Some(body) = name
        .strip_prefix(".se-agent-")
        .and_then(|rest| rest.strip_suffix(".spool"))
    else {
        return false;
    };
    let mut parts = body.rsplitn(4, '-');
    let ids_ok = parts
        .by_ref()
        .take(3)
        .filter(|part| lower_hex(part))
        .count()
        == 3;
    ids_ok && parts.next().is_some_and(|purpose| !purpose.is_empty())
}
