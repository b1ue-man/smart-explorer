//! The mask layer of `se mcp` (plan 2026-11-28, S2 and contract 4): every
//! output to the agent is masked with every active mask list, every input of
//! the agent is restored from its placeholders before a tool runs.
//!
//! - Arguments: every string of a call's arguments is restored with
//!   `MaskSet::unmask` (a placeholder → its real value, `⟦⟦` → `⟦`); an
//!   unknown placeholder refuses the whole call, so nothing of it reaches a
//!   file or a command. Search and replace patterns in `regex` and
//!   `extended` mode get the restored value escaped, so a placeholder always
//!   stands for its literal value (`escape_of`).
//! - Results: a tool that masks its answer itself marks it
//!   (`response::masked`); every other result has each string of
//!   `structuredContent`, its text blocks and its error masked here. Field
//!   names are se's own schema and stay as they are. Content that cannot be
//!   masked (images, audio, resources) is refused while a list is active.
//! - Job output: raw command output is masked as it is appended
//!   (`mask_layer_stream.rs`); the job's own fields when it is shown
//!   (`job_view.rs`).
//!
//! Without an active list nothing is changed: arguments and results are
//! byte-identical to a server without mask lists.
use super::response;
use crate::masks::{MaskSet, MaskStore, UnmaskError, PLACEHOLDER_CLOSE, PLACEHOLDER_OPEN};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, JsonObject};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the active lists of one read are reused. Lists changed by the
/// user (desktop, `se masks`) or by another agent apply within this time;
/// `masks` changes of this session apply at once ([`MaskLayer::invalidate`]).
const REFRESH: Duration = Duration::from_millis(500);

/// Result fields that hold paths: masked with `MaskSet::mask_path`.
const PATH_KEYS: &[&str] = &[
    "path",
    "destination",
    "source",
    "root",
    "spec",
    "cwd",
    "target",
    "cleared",
    "session_directory",
    "executable",
    "kept_directories",
];

pub(super) struct MaskLayer {
    store: Mutex<Option<MaskStore>>,
    current: Mutex<Option<(Arc<MaskSet>, Instant)>>,
    /// Masked whole files of `read` (`mask_layer_read.rs`).
    pub(super) reads: super::mask_layer_read::ReadCache,
}

impl MaskLayer {
    /// The layer of a session; the store opens with the first call.
    pub(super) fn open() -> Self {
        Self {
            store: Mutex::new(None),
            current: Mutex::new(None),
            reads: Default::default(),
        }
    }

    /// A layer over an opened store (tests).
    #[cfg(test)]
    pub(super) fn with_store(store: MaskStore) -> Self {
        let layer = Self::open();
        if let Ok(mut slot) = layer.store.lock() {
            *slot = Some(store);
        }
        layer
    }

    /// Runs `work` on the store, opening it first.
    pub(super) fn store<T>(
        &self,
        work: impl FnOnce(&MaskStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut slot = self
            .store
            .lock()
            .map_err(|_| "the mask list store is poisoned".to_string())?;
        if slot.is_none() {
            let store = MaskStore::open_default()
                .map_err(|error| format!("cannot read the mask lists: {error}"))?;
            *slot = Some(store);
        }
        match slot.as_ref() {
            Some(store) => work(store),
            None => Err("cannot read the mask lists".to_string()),
        }
    }

    /// The active mask lists, read again after [`REFRESH`].
    pub(super) fn set(&self) -> Result<Arc<MaskSet>, String> {
        if let Ok(current) = self.current.lock() {
            if let Some((set, at)) = current.as_ref() {
                if at.elapsed() < REFRESH {
                    return Ok(set.clone());
                }
            }
        }
        self.reload()
    }

    /// The active mask lists read from the store now.
    pub(super) fn reload(&self) -> Result<Arc<MaskSet>, String> {
        let set = self.store(|store| {
            store
                .active()
                .map_err(|error| format!("cannot read the mask lists: {error}"))
        })?;
        if let Ok(mut current) = self.current.lock() {
            *current = Some((set.clone(), Instant::now()));
        }
        Ok(set)
    }

    /// Forgets the kept lists, so the next call reads them again.
    pub(super) fn invalidate(&self) {
        if let Ok(mut current) = self.current.lock() {
            current.take();
        }
    }

    /// The lists of a call and its arguments with their placeholders
    /// restored. A placeholder this process does not know yet (assigned by
    /// another se process meanwhile) is looked up once more in the store.
    pub(super) fn prepare(
        &self,
        tool: &str,
        arguments: Option<JsonObject>,
    ) -> Result<(Arc<MaskSet>, Option<JsonObject>), String> {
        let set = self.set()?;
        if set.is_empty() {
            return Ok((set, arguments));
        }
        match restore_arguments(&set, tool, arguments.as_ref()) {
            Ok(restored) => Ok((set, restored)),
            Err(Refused::Unknown(_)) => {
                let set = self.reload()?;
                if set.is_empty() {
                    return Ok((set, arguments));
                }
                let restored =
                    restore_arguments(&set, tool, arguments.as_ref()).map_err(Refused::text)?;
                Ok((set, restored))
            }
            Err(other) => Err(other.text()),
        }
    }
}

/// Why the arguments of a call were refused.
enum Refused {
    Unknown(String),
    Other(String),
}

impl Refused {
    fn text(self) -> String {
        match self {
            Self::Unknown(text) | Self::Other(text) => text,
        }
    }
}

/// A refusal before any tool ran. Its text names no real value (only the
/// agent's own placeholders or the store's state), so it is not masked again.
pub(super) fn refusal(message: impl Into<String>) -> CallToolResult {
    response::mark_masked(response::error(message))
}

/// `text` with every active list applied.
pub(super) fn mask_text(set: &MaskSet, text: &str) -> String {
    if set.is_empty() {
        return text.to_string();
    }
    set.mask(text).text
}

/// Masks every string of `value` in place (not the field names).
pub(super) fn mask_value(set: &MaskSet, value: &mut Value) {
    if set.is_empty() {
        return;
    }
    mask_in(set, value, false);
}

fn mask_in(set: &MaskSet, value: &mut Value, path: bool) {
    match value {
        Value::String(text) => {
            *text = if path {
                set.mask_path(text)
            } else {
                set.mask(text).text
            };
        }
        Value::Array(items) => {
            for item in items {
                mask_in(set, item, path);
            }
        }
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                mask_in(set, field, PATH_KEYS.contains(&key.as_str()));
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The result as it leaves the server: masked once with `set`, the private
/// mark of a self-masked result removed.
pub(super) fn finish(set: &MaskSet, mut result: CallToolResult) -> CallToolResult {
    let premasked = response::take_masked_mark(&mut result);
    if premasked || set.is_empty() {
        return result;
    }
    if result
        .content
        .iter()
        .any(|block| !matches!(block, ContentBlock::Text(_)))
    {
        return finish(
            set,
            response::error(
                "this answer contains a picture or other media, which cannot be masked; it is withheld while a mask list is active",
            ),
        );
    }
    let original = result.structured_content.as_ref().map(Value::to_string);
    if let Some(structured) = result.structured_content.as_mut() {
        mask_value(set, structured);
    }
    let masked = result.structured_content.as_ref().map(Value::to_string);
    for block in &mut result.content {
        if let ContentBlock::Text(text) = block {
            // The text block repeating the structured result is the masked
            // structured result again (clients compare the two).
            text.text = match (&original, &masked) {
                (Some(original), Some(masked)) if *original == text.text => masked.clone(),
                _ => set.mask(&text.text).text,
            };
        }
    }
    result
}

/// Refuses a call that would only return a picture while lists are active:
/// a picture cannot be masked (`screen` with `snapshot`).
pub(super) fn image_refusal(set: &MaskSet, request: &CallToolRequestParams) -> Option<String> {
    if set.is_empty() || request.name != "screen" {
        return None;
    }
    let arguments = request.arguments.as_ref();
    super::screen_tool::yields_image(&super::args::Args::new(arguments)).then(|| {
        "a screen snapshot is a picture, which cannot be masked; it is refused while a mask list is active (`windows`, `probe` and `screen_input` still work)".to_string()
    })
}

/// How a restored value is written into an argument.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Escape {
    /// As it is (paths, file content, commands).
    None,
    /// A Rust/Boost regular expression: every metacharacter escaped.
    Regex,
    /// Notepad++ `extended` text: backslashes doubled.
    Extended,
    /// Notepad++ regex replacement (Boost `format_all`).
    Format,
}

/// The escape of argument `key` of `tool`: the patterns of `grep` and
/// `replace` follow their `mode`, folder filters are regular expressions.
fn escape_of(tool: &str, mode: Option<&str>, key: &str) -> Escape {
    if !matches!(tool, "grep" | "replace") {
        return Escape::None;
    }
    match (key, mode) {
        ("folders" | "exclude_folders", _) => Escape::Regex,
        ("pattern", Some("regex")) => Escape::Regex,
        ("replacement", Some("regex")) => Escape::Format,
        ("pattern" | "replacement", Some("extended")) => Escape::Extended,
        _ => Escape::None,
    }
}

fn restore_arguments(
    set: &MaskSet,
    tool: &str,
    arguments: Option<&JsonObject>,
) -> Result<Option<JsonObject>, Refused> {
    let Some(arguments) = arguments else {
        return Ok(None);
    };
    let mode = arguments.get("mode").and_then(Value::as_str);
    let mut restored = JsonObject::new();
    for (key, value) in arguments {
        let mut value = value.clone();
        restore_in(set, &mut value, escape_of(tool, mode, key), key)?;
        restored.insert(key.clone(), value);
    }
    Ok(Some(restored))
}

fn restore_in(set: &MaskSet, value: &mut Value, escape: Escape, at: &str) -> Result<(), Refused> {
    match value {
        Value::String(text) => {
            *text = restore_text(set, text, escape).map_err(|error| {
                let text = format!(
                    "argument `{at}`: {error}; use placeholders exactly as Smart Explorer showed them (a literal ⟦ is written ⟦⟦)"
                );
                match error {
                    UnmaskError::Unknown(_) => Refused::Unknown(text),
                    _ => Refused::Other(text),
                }
            })?;
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                restore_in(set, item, escape, &format!("{at}[{index}]"))?;
            }
        }
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                restore_in(set, field, escape, &format!("{at}.{key}"))?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

/// `text` with its placeholders restored; with an escape, every restored
/// value (and an escaped `⟦`) is written so that it matches itself only.
fn restore_text(set: &MaskSet, text: &str, escape: Escape) -> Result<String, UnmaskError> {
    if escape == Escape::None {
        return set.unmask(text);
    }
    let mut restored = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(PLACEHOLDER_OPEN) {
        let (plain, tail) = rest.split_at(start);
        restored.push_str(&set.unmask(plain)?);
        if let Some(after) = tail.strip_prefix("⟦⟦") {
            restored.push_str(&escaped("⟦", escape));
            rest = after;
            continue;
        }
        let Some(close) = tail.find(PLACEHOLDER_CLOSE) else {
            // `unmask` names the malformed `⟦`.
            restored.push_str(&set.unmask(tail)?);
            return Ok(restored);
        };
        let token = &tail[..close + PLACEHOLDER_CLOSE.len_utf8()];
        restored.push_str(&escaped(&set.unmask(token)?, escape));
        rest = &tail[token.len()..];
    }
    restored.push_str(&set.unmask(rest)?);
    Ok(restored)
}

fn escaped(value: &str, escape: Escape) -> String {
    let special: &[char] = match escape {
        Escape::None => return value.to_string(),
        // The metacharacters `regex::escape` escapes.
        Escape::Regex => &[
            '\\', '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$', '#', '&', '-',
            '~',
        ],
        Escape::Extended => &['\\'],
        // Boost `format_all`: `$` references, `\` escapes, `(?n…:…)`
        // conditions; a backslash makes each of them literal.
        Escape::Format => &['\\', '$', '(', ')', '?', ':'],
    };
    let mut text = String::with_capacity(value.len());
    for character in value.chars() {
        if special.contains(&character) {
            text.push('\\');
        }
        text.push(character);
    }
    text
}

#[cfg(test)]
#[path = "mask_layer_tests.rs"]
mod tests;
