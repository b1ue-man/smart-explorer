//! Append-only output files of one MCP session's jobs (command output, search
//! hits). Output is never capped in memory: a job appends everything, and
//! callers read pages from any offset.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub(super) struct Spool {
    path: PathBuf,
    writer: Mutex<File>,
    len: AtomicU64,
}

impl Spool {
    pub(super) fn create(path: PathBuf) -> io::Result<Self> {
        let writer = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            path,
            writer: Mutex::new(writer),
            len: AtomicU64::new(0),
        })
    }

    pub(super) fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("job output writer is poisoned"))?;
        writer.write_all(bytes)?;
        writer.flush()?;
        self.len.fetch_add(bytes.len() as u64, Ordering::SeqCst);
        Ok(())
    }

    pub(super) fn len(&self) -> u64 {
        self.len.load(Ordering::SeqCst)
    }

    /// Up to `max` bytes from `offset`; fewer at the current end.
    pub(super) fn read(&self, offset: u64, max: usize) -> io::Result<Vec<u8>> {
        let available = self.len().saturating_sub(offset);
        let wanted = usize::try_from(available).unwrap_or(usize::MAX).min(max);
        let mut bytes = vec![0u8; wanted];
        if wanted == 0 {
            return Ok(bytes);
        }
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

/// One decoded page of a byte stream.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TextPage {
    pub(super) text: String,
    /// Offset of the first byte not included.
    pub(super) next_offset: u64,
    /// Invalid UTF-8 was replaced with U+FFFD.
    pub(super) lossy: bool,
}

/// Decodes `bytes` read at `offset`. An incomplete UTF-8 sequence at the end
/// is left for the next page unless `stream_complete`; invalid bytes are
/// replaced.
pub(super) fn text_page(bytes: &[u8], offset: u64, stream_complete: bool) -> TextPage {
    match std::str::from_utf8(bytes) {
        Ok(text) => TextPage {
            text: text.to_string(),
            next_offset: offset + bytes.len() as u64,
            lossy: false,
        },
        Err(error) if error.error_len().is_none() && !stream_complete => {
            let valid = &bytes[..error.valid_up_to()];
            TextPage {
                // `valid_up_to` marks the longest valid prefix.
                text: String::from_utf8_lossy(valid).into_owned(),
                next_offset: offset + valid.len() as u64,
                lossy: false,
            }
        }
        Err(_) => TextPage {
            text: String::from_utf8_lossy(bytes).into_owned(),
            next_offset: offset + bytes.len() as u64,
            lossy: true,
        },
    }
}

/// Whether `bytes` look like text: valid UTF-8 (an incomplete sequence at the
/// cut is allowed) without NUL bytes.
pub(super) fn looks_textual(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(error) => error.error_len().is_none() && bytes.len() - error.valid_up_to() < 4,
    }
}

#[cfg(test)]
mod tests {
    use super::{looks_textual, text_page, Spool};

    #[test]
    fn mcp_task_pages_never_split_a_utf8_character() {
        let text = "aé€😀b".as_bytes();
        // Cut inside the 4-byte emoji: the page stops before it.
        let page = text_page(&text[..8], 0, false);
        assert_eq!(page.text, "aé€");
        assert_eq!(page.next_offset, 6);
        assert!(!page.lossy);
        let rest = text_page(&text[6..], 6, true);
        assert_eq!(rest.text, "😀b");
        let broken = text_page(&[b'a', 0xff, b'b'], 3, false);
        assert!(broken.lossy);
        assert_eq!(broken.next_offset, 6);
        assert!(looks_textual(&text[..8]));
        assert!(!looks_textual(b"bin\0ary"));
    }

    #[test]
    fn mcp_task_spool_reads_pages_from_any_offset() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::create(dir.path().join("out")).unwrap();
        spool.append(b"hello ").unwrap();
        spool.append(b"world").unwrap();
        assert_eq!(spool.len(), 11);
        assert_eq!(spool.read(6, 100).unwrap(), b"world");
        assert_eq!(spool.read(0, 5).unwrap(), b"hello");
        assert!(spool.read(11, 10).unwrap().is_empty());
    }
}
