//! Hits of `grep` (and the previews of `replace`) while a mask list is active
//! (plan docs/plaene/2026-11-28-ersetzen-masken-shell, contract 4): the hit
//! lines and context lines come from the masked whole file, so a value that
//! runs over several lines (a multi-line regex entry, „Schweins-⏎braten“) is
//! found as it is in `read`. A placeholder carries the line ends of its value,
//! so line numbers stay those of the file; `ranges` are moved into the masked
//! line (`MaskedText::map_range`: a range that touches a value covers its
//! whole placeholder), `offset` is the line's offset in the masked file, and
//! the file's path is masked. A file that cannot be read again, has changed
//! since it was searched, or is not text the search shows the same way
//! (UTF-8, UTF-16 with BOM) masks each line on its own.
use crate::agent_proto::LineHit;
use crate::masks::{MaskSet, MaskedText};
use crate::vfs::BackendHandle;
use std::collections::HashMap;
use std::io::Read;
use std::ops::Range;
use std::sync::Arc;

/// One file decoded and masked.
struct MaskedFile {
    original: String,
    /// `[start, end)` of each line of `original`, without its terminator.
    lines: Vec<(usize, usize)>,
    masked: MaskedText,
    masked_lines: Vec<(usize, usize)>,
}

/// Lines of `text` as `[start, end)` without `\n` or `\r\n`.
fn line_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            let end = if index > start && text.as_bytes()[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            lines.push((start, end));
            start = index + 1;
        }
    }
    lines.push((start, text.len()));
    lines
}

/// The text of a file as the search shows it: UTF-16 by its BOM, UTF-8
/// without its BOM; `None` for anything else (each line is then masked alone).
fn decode(bytes: &[u8]) -> Option<String> {
    let utf16 = |little: bool| {
        let units = bytes[2..].chunks_exact(2).map(|pair| {
            if little {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        });
        char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .ok()
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(true),
        [0xFE, 0xFF, ..] => utf16(false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).ok(),
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
}

fn clamp(range: Range<usize>, (start, end): (usize, usize)) -> Range<usize> {
    let from = range.start.clamp(start, end);
    from..range.end.clamp(from, end)
}

impl MaskedFile {
    fn new(set: &MaskSet, original: String) -> Option<Self> {
        let masked = set.mask(&original);
        let lines = line_ranges(&original);
        let masked_lines = line_ranges(&masked.text);
        // A placeholder keeps the line count; anything else is not trusted.
        (lines.len() == masked_lines.len()).then_some(Self {
            original,
            lines,
            masked,
            masked_lines,
        })
    }

    /// `hit` from the masked file; `None` when the line differs from it.
    fn line(&self, hit: &LineHit) -> Option<LineHit> {
        let index = usize::try_from(hit.line.checked_sub(1)?).ok()?;
        let (start, end) = *self.lines.get(index)?;
        let line = self.original.get(start..end)?;
        let cut = if hit.truncated {
            line.find(hit.text.as_str())?
        } else if line == hit.text {
            0
        } else {
            return None;
        };
        let bounds = *self.masked_lines.get(index)?;
        let from = start + cut;
        let window = clamp(self.masked.map_range(from..from + hit.text.len()), bounds);
        let text = self.masked.text.get(window.clone())?.to_string();
        let mut ranges = Vec::with_capacity(hit.ranges.len());
        for &(low, high) in &hit.ranges {
            let mapped = clamp(
                self.masked
                    .map_range(from + low as usize..from + high as usize),
                (window.start, window.end),
            );
            if mapped.start < mapped.end {
                ranges.push((
                    u32::try_from(mapped.start - window.start).ok()?,
                    u32::try_from(mapped.end - window.start).ok()?,
                ));
            }
        }
        Some(LineHit {
            line: hit.line,
            offset: bounds.0 as u64,
            text,
            ranges,
            truncated: hit.truncated,
            context: hit.context,
        })
    }
}

impl MaskedFile {
    /// The lines `text` (one or more, `\n`-separated) from line `line` of
    /// the masked file; `None` when they differ from the file.
    fn block(&self, line: u64, text: &str, truncated: bool) -> Option<String> {
        let first = usize::try_from(line.checked_sub(1)?).ok()?;
        let last = first + text.matches('\n').count();
        let (start, _) = *self.lines.get(first)?;
        let (_, end) = *self.lines.get(last)?;
        let block = self.original.get(start..end)?;
        let cut = if truncated {
            block.find(text)?
        } else if block == text {
            0
        } else {
            return None;
        };
        let bounds = (
            self.masked_lines.get(first)?.0,
            self.masked_lines.get(last)?.1,
        );
        let from = start + cut;
        let window = clamp(self.masked.map_range(from..from + text.len()), bounds);
        self.masked.text.get(window).map(str::to_string)
    }
}

/// `hit` masked on its own: its text and its ranges.
fn alone(set: &MaskSet, hit: &LineHit) -> LineHit {
    let masked = set.mask(&hit.text);
    let ranges = hit
        .ranges
        .iter()
        .filter_map(|&(low, high)| {
            let mapped = masked.map_range(low as usize..high as usize);
            Some((
                u32::try_from(mapped.start).ok()?,
                u32::try_from(mapped.end).ok()?,
            ))
        })
        .filter(|(low, high)| low < high)
        .collect();
    LineHit {
        line: hit.line,
        offset: hit.offset,
        text: masked.text,
        ranges,
        truncated: hit.truncated,
        context: hit.context,
    }
}

/// Masks the hits of one search, file by file.
pub(super) struct HitMasker {
    set: Arc<MaskSet>,
    backend: BackendHandle,
    root: String,
    /// Files whose last event has not come yet (several threads search at
    /// once, so the events of files interleave).
    files: HashMap<String, Option<MaskedFile>>,
}

impl HitMasker {
    /// `None` without an active list: the hits stay as they are.
    pub(super) fn new(set: Arc<MaskSet>, backend: BackendHandle, root: &str) -> Option<Self> {
        (!set.is_empty()).then(|| Self {
            set,
            backend,
            root: root.to_string(),
            files: HashMap::new(),
        })
    }

    pub(super) fn set(&self) -> &MaskSet {
        &self.set
    }

    /// The file `rel` read again and masked whole.
    fn load(&self, rel: &str, size: u64) -> Option<MaskedFile> {
        let path = if rel.is_empty() {
            self.root.clone()
        } else {
            crate::cli::ops::join(&self.root, rel)
        };
        let mut reader = self
            .backend
            .open_read(&path)
            .or_else(|_| {
                // A search of one file names it by its name, below no folder.
                let named = self.root.rsplit(['/', '\\']).next() == Some(rel);
                if named {
                    self.backend.open_read(&self.root)
                } else {
                    Err(std::io::Error::from(std::io::ErrorKind::NotFound))
                }
            })
            .ok()?;
        let mut bytes = Vec::new();
        bytes.try_reserve(usize::try_from(size).ok()?).ok()?;
        reader.read_to_end(&mut bytes).ok()?;
        MaskedFile::new(&self.set, decode(&bytes)?)
    }

    /// `text` (lines from `line` of the file `rel`, as a replace preview
    /// shows them before the change) masked from the masked file.
    pub(super) fn block(
        &mut self,
        (rel, size): (&str, u64),
        line: u64,
        text: &str,
        truncated: bool,
    ) -> String {
        if !self.files.contains_key(rel) {
            let file = self.load(rel, size);
            self.files.insert(rel.to_string(), file);
        }
        self.files
            .get(rel)
            .and_then(Option::as_ref)
            .and_then(|file| file.block(line, text, truncated))
            .unwrap_or_else(|| self.set.mask(text).text)
    }

    /// Forgets the masked file `rel` (its last event came).
    pub(super) fn forget(&mut self, rel: &str) {
        self.files.remove(rel);
    }

    /// The masked path and hits of one `Hits` event of `rel`.
    pub(super) fn hits(
        &mut self,
        rel: &str,
        size: u64,
        lines: &[LineHit],
        last: bool,
    ) -> (String, Vec<LineHit>) {
        if !self.files.contains_key(rel) {
            let file = self.load(rel, size);
            self.files.insert(rel.to_string(), file);
        }
        let file = self.files.get(rel).and_then(Option::as_ref);
        let masked = lines
            .iter()
            .map(|hit| {
                file.and_then(|file| file.line(hit))
                    .unwrap_or_else(|| alone(&self.set, hit))
            })
            .collect();
        if last {
            self.files.remove(rel);
        }
        (self.set.mask_path(rel), masked)
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, line_ranges};

    #[test]
    fn mcp_coding_task_grep_mask_reads_lines_as_the_search_shows_them() {
        assert_eq!(line_ranges("a\r\nbc\n"), [(0, 1), (3, 5), (6, 6)]);
        assert_eq!(line_ranges("x"), [(0, 1)]);
        assert_eq!(decode(b"\xEF\xBB\xBFhi").as_deref(), Some("hi"));
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "Grüße".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode(&utf16).as_deref(), Some("Grüße"));
        assert_eq!(decode(b"Gr\xF6\xDFe"), None, "ANSI lines are masked alone");
    }
}
