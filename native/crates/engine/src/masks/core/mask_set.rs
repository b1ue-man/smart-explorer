//! The masking set of the enabled lists: real value → placeholder for every
//! output to an agent, placeholder → real value for every input of it.
//! A placeholder needed for the first time is assigned through the store
//! (`Allocate`) and saved before it is shown, so another process resolves
//! it and never hands out the same number for another value.
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use super::doc::StoreDoc;
use super::matcher::{Matcher, Span};
use super::model::{EntryKind, MaskError, UnmaskError};
use super::placeholder::{self, OPEN};
use super::scope::Scope;
use super::stream::StreamMasker;
use super::tables::{allocate, Assignment, Need, Tables};

/// Assigns and rereads placeholders; the store implements it.
pub(crate) trait Allocate: Send + Sync {
    /// Saves assignments for `needs` and returns the tables after the save.
    fn allocate(&self, needs: &[Need]) -> Result<Tables, String>;
    /// The tables as saved now (assignments of other processes).
    fn reload(&self) -> Result<Tables, String>;
}

/// Assignments only in memory: for sets built without a store.
pub(crate) struct MemoryAllocator {
    doc: Mutex<StoreDoc>,
}

impl MemoryAllocator {
    pub(crate) fn new(doc: StoreDoc) -> Self {
        Self {
            doc: Mutex::new(doc),
        }
    }
}

impl Allocate for MemoryAllocator {
    fn allocate(&self, needs: &[Need]) -> Result<Tables, String> {
        let mut doc = self.doc.lock().unwrap_or_else(PoisonError::into_inner);
        allocate(&mut doc, needs);
        Ok(Tables::from_doc(&doc))
    }

    fn reload(&self) -> Result<Tables, String> {
        let doc = self.doc.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(Tables::from_doc(&doc))
    }
}

/// One replaced range: where it was in the original, where it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Edit {
    pub orig: Range<usize>,
    pub masked: Range<usize>,
}

/// A masked text with the map from original to masked byte offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskedText {
    pub text: String,
    edits: Vec<Edit>,
    complete: bool,
}

impl MaskedText {
    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn into_string(self) -> String {
        self.text
    }

    /// Whether anything was replaced or escaped.
    pub fn is_changed(&self) -> bool {
        !self.edits.is_empty()
    }

    /// `false` when a placeholder could not be saved (store not writable):
    /// those spans show `⟦Label-0⟧`, which nothing resolves. Nothing of the
    /// value is shown either way.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Maps a byte range of the original onto the masked text. A range that
    /// touches a masked value covers its whole placeholder.
    pub fn map_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.map_offset(range.start, false);
        let end = self.map_offset(range.end.max(range.start), true);
        start..end.max(start)
    }

    fn map_offset(&self, pos: usize, is_end: bool) -> usize {
        let before = self.edits.partition_point(|edit| edit.orig.end <= pos);
        if let Some(edit) = self.edits.get(before) {
            if edit.orig.start < pos {
                return if is_end {
                    edit.masked.end
                } else {
                    edit.masked.start
                };
            }
        }
        let mapped = match before.checked_sub(1).map(|index| &self.edits[index]) {
            Some(edit) => edit.masked.end + (pos - edit.orig.end),
            None => pos,
        };
        mapped.min(self.text.len())
    }
}

/// The masking set; `MaskStore::active` hands it out.
pub struct MaskSet {
    matcher: Matcher,
    tables: RwLock<Tables>,
    allocator: Option<Arc<dyn Allocate>>,
    lockdown: bool,
    scopes: Vec<Scope>,
    config_generation: u64,
}

impl std::fmt::Debug for MaskSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaskSet")
            .field("empty", &self.is_empty())
            .field("lockdown", &self.lockdown)
            .field("config_generation", &self.config_generation)
            .finish_non_exhaustive()
    }
}

impl MaskSet {
    /// A set without lists: every text stays as it is.
    pub fn empty() -> Self {
        Self {
            matcher: Matcher::empty(),
            tables: RwLock::new(Tables::default()),
            allocator: None,
            lockdown: false,
            scopes: Vec::new(),
            config_generation: 0,
        }
    }

    /// The set of the enabled lists of `doc`.
    pub(crate) fn build(
        doc: &StoreDoc,
        allocator: Option<Arc<dyn Allocate>>,
    ) -> Result<Self, MaskError> {
        let enabled = doc.lists.iter().filter(|list| list.enabled);
        let matcher = Matcher::compile(
            enabled
                .clone()
                .flat_map(|list| list.entries.iter().map(move |entry| (list.id, entry))),
        )?;
        Ok(Self {
            matcher,
            tables: RwLock::new(Tables::from_doc(doc)),
            allocator,
            lockdown: doc.lockdown(),
            scopes: enabled
                .flat_map(|list| list.scopes.iter().cloned())
                .collect(),
            config_generation: doc.config_generation,
        })
    }

    /// No enabled entry and no assigned placeholder: masking and unmasking
    /// leave every text unchanged.
    pub fn is_empty(&self) -> bool {
        self.matcher.is_empty() && self.read_tables().is_empty()
    }

    /// An enabled list has a protected path: agents work only through se.
    pub fn lockdown(&self) -> bool {
        self.lockdown
    }

    /// The protected local paths of the enabled lists.
    pub fn local_scopes(&self) -> Vec<PathBuf> {
        self.scopes.iter().filter_map(Scope::local_path).collect()
    }

    /// Every protected path of the enabled lists (local and remote).
    pub fn protected_scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Generation of the lists this set was built from.
    pub fn config_generation(&self) -> u64 {
        self.config_generation
    }

    /// Masks a text for the agent.
    pub fn mask(&self, text: &str) -> MaskedText {
        if self.is_empty() {
            return MaskedText {
                text: text.to_string(),
                edits: Vec::new(),
                complete: true,
            };
        }
        let spans = char_aligned(text, self.matcher.find(text.as_bytes()));
        let (bytes, edits, complete) = self.render(text.as_bytes(), &spans);
        let text = String::from_utf8(bytes)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
        MaskedText {
            text,
            edits,
            complete,
        }
    }

    /// Masks a path or a name (the whole string: values may contain `/`).
    pub fn mask_path(&self, path: &str) -> String {
        self.mask(path).into_string()
    }

    /// Masks bytes (content sent as Base64): literal and regex entries on
    /// the bytes, fuzzy entries on the valid UTF-8 sections.
    pub fn mask_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        if self.is_empty() {
            return bytes.to_vec();
        }
        let spans = self.matcher.find(bytes);
        self.render(bytes, &spans).0
    }

    /// Restores the real values of every placeholder in an input. `⟦⟦` is a
    /// real `⟦`; an unknown placeholder or a lone `⟦` refuses the input.
    pub fn unmask(&self, text: &str) -> Result<String, UnmaskError> {
        let bytes = self.unmask_inner(text.as_bytes(), true)?;
        String::from_utf8(bytes).map_err(|_| UnmaskError::NotText(String::new()))
    }

    /// [`MaskSet::unmask`] for bytes (content written as Base64).
    pub fn unmask_bytes(&self, bytes: &[u8]) -> Result<Vec<u8>, UnmaskError> {
        self.unmask_inner(bytes, false)
    }

    /// A stream masker over this set (command output).
    pub fn stream(self: &Arc<Self>) -> StreamMasker {
        StreamMasker::new(Arc::clone(self))
    }

    pub(crate) fn hold(&self) -> usize {
        self.matcher.hold()
    }

    pub(crate) fn find_spans(&self, bytes: &[u8]) -> Vec<Span> {
        self.matcher.find(bytes)
    }

    /// Writes `bytes` with every span replaced by its placeholder and every
    /// other `⟦` doubled.
    pub(crate) fn render(&self, bytes: &[u8], spans: &[Span]) -> (Vec<u8>, Vec<Edit>, bool) {
        let needs: Vec<Need> = spans.iter().map(|span| self.need(bytes, span)).collect();
        let assignments = self.assign(&needs);
        let mut out = Vec::with_capacity(bytes.len() + spans.len() * 16);
        let mut edits = Vec::new();
        let mut complete = true;
        let mut pos = 0;
        for (span, assignment) in spans.iter().zip(assignments) {
            escape_into(&bytes[pos..span.start], pos, &mut out, &mut edits);
            let start = out.len();
            let (number, variant) = assignment.unwrap_or_else(|| {
                complete = false;
                (0, None)
            });
            let label = &self.matcher.entry(span.owner).label;
            let endings = placeholder::line_endings(&bytes[span.start..span.end]);
            placeholder::write(label, number, variant, &endings, &mut out);
            edits.push(Edit {
                orig: span.start..span.end,
                masked: start..out.len(),
            });
            pos = span.end;
        }
        escape_into(&bytes[pos..], pos, &mut out, &mut edits);
        (out, edits, complete)
    }

    fn need(&self, bytes: &[u8], span: &Span) -> Need {
        let entry = self.matcher.entry(span.owner);
        let found = bytes[span.start..span.end].to_vec();
        let (canonical, variant) = match entry.kind {
            EntryKind::Regex => (found, None),
            EntryKind::Literal | EntryKind::Fuzzy if found == entry.value => (found, None),
            EntryKind::Literal | EntryKind::Fuzzy => (entry.value.clone(), Some(found)),
        };
        Need {
            list: entry.list,
            entry: entry.entry,
            label: entry.label.clone(),
            canonical,
            variant,
        }
    }

    fn assign(&self, needs: &[Need]) -> Vec<Option<Assignment>> {
        let mut result: Vec<Option<Assignment>> = {
            let tables = self.read_tables();
            needs.iter().map(|need| tables.resolve(need)).collect()
        };
        let mut missing: Vec<Need> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (need, assigned) in needs.iter().zip(&result) {
            if assigned.is_none() && seen.insert(need) {
                missing.push(need.clone());
            }
        }
        if missing.is_empty() {
            return result;
        }
        let Some(allocator) = &self.allocator else {
            return result;
        };
        if let Ok(fresh) = allocator.allocate(&missing) {
            for (need, assigned) in needs.iter().zip(result.iter_mut()) {
                if assigned.is_none() {
                    *assigned = fresh.resolve(need);
                }
            }
            *self.tables.write().unwrap_or_else(PoisonError::into_inner) = fresh;
        }
        result
    }

    fn unmask_inner(&self, bytes: &[u8], text_only: bool) -> Result<Vec<u8>, UnmaskError> {
        if self.is_empty() {
            return Ok(bytes.to_vec());
        }
        let mut out = Vec::with_capacity(bytes.len());
        let mut pos = 0;
        let mut reloaded = false;
        while let Some(found) = memchr::memmem::find(&bytes[pos..], OPEN) {
            let at = pos + found;
            out.extend_from_slice(&bytes[pos..at]);
            if bytes[at + OPEN.len()..].starts_with(OPEN) {
                out.extend_from_slice(OPEN);
                pos = at + 2 * OPEN.len();
                continue;
            }
            let parsed =
                placeholder::parse_at(bytes, at).ok_or(UnmaskError::Malformed { offset: at })?;
            let shown = String::from_utf8_lossy(&bytes[at..at + parsed.len]).into_owned();
            let value = self.value_of(&parsed, &shown, &mut reloaded)?;
            if text_only && std::str::from_utf8(&value).is_err() {
                return Err(UnmaskError::NotText(shown));
            }
            out.extend_from_slice(&value);
            pos = at + parsed.len;
        }
        out.extend_from_slice(&bytes[pos..]);
        Ok(out)
    }

    fn value_of(
        &self,
        parsed: &placeholder::Parsed,
        shown: &str,
        reloaded: &mut bool,
    ) -> Result<Vec<u8>, UnmaskError> {
        let lookup = |tables: &Tables| {
            tables
                .value_of(&parsed.label, parsed.number, parsed.variant)
                .map(<[u8]>::to_vec)
        };
        if let Some(value) = lookup(&*self.read_tables()) {
            return Ok(value);
        }
        if !*reloaded {
            *reloaded = true;
            if let Some(allocator) = &self.allocator {
                let fresh = allocator.reload().map_err(UnmaskError::Store)?;
                let value = lookup(&fresh);
                *self.tables.write().unwrap_or_else(PoisonError::into_inner) = fresh;
                if let Some(value) = value {
                    return Ok(value);
                }
            }
        }
        Err(UnmaskError::Unknown(shown.to_string()))
    }

    fn read_tables(&self) -> std::sync::RwLockReadGuard<'_, Tables> {
        self.tables.read().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Doubles every `⟦` of an unmasked segment that starts at `offset`.
fn escape_into(segment: &[u8], offset: usize, out: &mut Vec<u8>, edits: &mut Vec<Edit>) {
    let mut last = 0;
    for found in memchr::memmem::find_iter(segment, OPEN) {
        out.extend_from_slice(&segment[last..found]);
        let start = out.len();
        out.extend_from_slice(OPEN);
        out.extend_from_slice(OPEN);
        edits.push(Edit {
            orig: offset + found..offset + found + OPEN.len(),
            masked: start..out.len(),
        });
        last = found + OPEN.len();
    }
    out.extend_from_slice(&segment[last..]);
}

/// Spans widened to character boundaries of `text`, merged again.
fn char_aligned(text: &str, spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for mut span in spans {
        while span.start > 0 && !text.is_char_boundary(span.start) {
            span.start -= 1;
        }
        while span.end < text.len() && !text.is_char_boundary(span.end) {
            span.end += 1;
        }
        match out.last_mut() {
            Some(last) if span.start < last.end => last.end = last.end.max(span.end),
            _ => out.push(span),
        }
    }
    out
}
