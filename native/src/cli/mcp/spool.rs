//! Append-only output files of one MCP session's jobs (command output, search
//! hits). Output is never capped in memory: a job appends everything, and
//! callers read pages from any offset.
//!
//! The files hold only ciphertext. Output can be the contents of an unlocked
//! vault (`se cat vault://…`, place K49 of the vault plaintext boundary), and
//! the session folder stays on disk until the session ends, or after a crash.
//! Every append is one AES-256-GCM record under a key that exists only in
//! memory for this one spool; an index in memory maps output offsets to the
//! records.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

/// One sealed append: output bytes `start..start + len`, stored at file
/// offset `at` with the tag after them.
#[derive(Clone, Copy)]
struct Record {
    start: u64,
    len: u64,
    at: u64,
}

struct Writer {
    file: File,
    records: Vec<Record>,
    /// Bytes of the file that belong to records (ciphertext and tags).
    written: u64,
}

pub(super) struct Spool {
    path: PathBuf,
    key: LessSafeKey,
    writer: Mutex<Writer>,
    len: AtomicU64,
}

/// The key of one spool: random and only in memory.
fn new_key() -> io::Result<LessSafeKey> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| io::Error::other("no random key for the job output"))?;
    let key = UnboundKey::new(&AES_256_GCM, &bytes);
    bytes.fill(0);
    key.map(LessSafeKey::new)
        .map_err(|_| io::Error::other("the job output key was refused"))
}

/// Record `number` as the nonce (unique: one key per spool) and as the
/// associated data, so no record can stand in for another.
fn nonce(number: u64) -> Nonce {
    let mut bytes = [0u8; NONCE_LEN];
    bytes[..8].copy_from_slice(&number.to_le_bytes());
    Nonce::assume_unique_for_key(bytes)
}

impl Spool {
    pub(super) fn create(path: PathBuf) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            path,
            key: new_key()?,
            writer: Mutex::new(Writer {
                file,
                records: Vec::new(),
                written: 0,
            }),
            len: AtomicU64::new(0),
        })
    }

    fn lock(&self) -> io::Result<MutexGuard<'_, Writer>> {
        self.writer
            .lock()
            .map_err(|_| io::Error::other("job output writer is poisoned"))
    }

    pub(super) fn append(&self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut writer = self.lock()?;
        let number = writer.records.len() as u64;
        let mut sealed = bytes.to_vec();
        self.key
            .seal_in_place_append_tag(nonce(number), Aad::from(number.to_le_bytes()), &mut sealed)
            .map_err(|_| io::Error::other("the job output could not be sealed"))?;
        // After a failed write the next record starts where this one did.
        let at = writer.written;
        writer.file.seek(SeekFrom::Start(at))?;
        writer.file.write_all(&sealed)?;
        writer.file.flush()?;
        let start = self.len();
        writer.records.push(Record {
            start,
            len: bytes.len() as u64,
            at,
        });
        writer.written = at + sealed.len() as u64;
        // Published under the lock: a reader that sees the new length finds
        // the record.
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
        let mut bytes = Vec::with_capacity(wanted);
        if wanted == 0 {
            return Ok(bytes);
        }
        let end = offset + wanted as u64;
        let records: Vec<(u64, Record)> = {
            let writer = self.lock()?;
            let first = writer
                .records
                .partition_point(|record| record.start + record.len <= offset);
            writer.records[first..]
                .iter()
                .take_while(|record| record.start < end)
                .enumerate()
                .map(|(index, record)| ((first + index) as u64, *record))
                .collect()
        };
        let mut file = File::open(&self.path)?;
        for (number, record) in records {
            let len = usize::try_from(record.len)
                .map_err(|_| io::Error::other("job output record too large"))?;
            let mut sealed = vec![0u8; len + AES_256_GCM.tag_len()];
            file.seek(SeekFrom::Start(record.at))?;
            file.read_exact(&mut sealed)?;
            let plain = self
                .key
                .open_in_place(nonce(number), Aad::from(number.to_le_bytes()), &mut sealed)
                .map_err(|_| io::Error::other("a job output record did not authenticate"))?;
            let from = offset.saturating_sub(record.start);
            let to = end.min(record.start + record.len) - record.start;
            bytes.extend_from_slice(&plain[from as usize..to as usize]);
        }
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

    #[test]
    fn mcp_task_spool_file_holds_no_plaintext() {
        // Output of an unlocked vault (`se cat vault://…`) must not reach the
        // session folder in the clear (place K49): the file holds sealed
        // records only, and pages across records still read back.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out");
        let spool = Spool::create(path.clone()).unwrap();
        let marker = b"tresor-klartext-markierung";
        spool.append(b"vor ").unwrap();
        spool.append(marker).unwrap();
        spool.append(b" nach").unwrap();
        let stored = std::fs::read(&path).unwrap();
        assert!(!stored.windows(marker.len()).any(|window| window == marker));
        assert!(!stored.windows(4).any(|window| window == b"vor "));
        let total = 4 + marker.len() + 5;
        assert_eq!(spool.len(), total as u64);
        let mut whole = b"vor ".to_vec();
        whole.extend_from_slice(marker);
        whole.extend_from_slice(b" nach");
        assert_eq!(spool.read(0, total).unwrap(), whole);
        assert_eq!(spool.read(2, 6).unwrap(), &whole[2..8]);
        assert_eq!(spool.read(total as u64 - 3, 10).unwrap(), b"ach");
        // A second spool of the same session has its own key: the same
        // output gives other bytes.
        let other_path = dir.path().join("other");
        let other = Spool::create(other_path.clone()).unwrap();
        other.append(b"vor ").unwrap();
        assert_ne!(std::fs::read(&other_path).unwrap(), stored[..20].to_vec());
    }
}
