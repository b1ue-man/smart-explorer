//! Encoding detection, line index and window reads of `TextDocument`.
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::textview::{
    detect_encoding, LineEnding, TextDocument, TextEncoding, TextSource, TextViewError, LINE_STRIDE,
};

fn indexed(bytes: Vec<u8>) -> TextDocument {
    let doc = TextDocument::from_bytes(bytes).expect("text document");
    doc.build_index(&AtomicBool::new(false), &mut |_| {})
        .expect("index");
    doc
}

fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
    let mut bytes = if big_endian {
        vec![0xFE, 0xFF]
    } else {
        vec![0xFF, 0xFE]
    };
    for unit in text.encode_utf16() {
        let pair = if big_endian {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        };
        bytes.extend_from_slice(&pair);
    }
    bytes
}

#[test]
fn textview_task_encoding_detection() {
    assert_eq!(
        detect_encoding("Grüße".as_bytes(), true).unwrap(),
        TextEncoding::Utf8
    );
    assert_eq!(
        detect_encoding(b"\xEF\xBB\xBFabc", true).unwrap(),
        TextEncoding::Utf8Bom
    );
    assert_eq!(
        detect_encoding(&utf16("Hä", false), true).unwrap(),
        TextEncoding::Utf16Le
    );
    assert_eq!(
        detect_encoding(&utf16("Hä", true), true).unwrap(),
        TextEncoding::Utf16Be
    );
    // "Grüße" in Windows-1252 is not UTF-8.
    let ansi = b"Gr\xFC\xDFe".to_vec();
    assert_eq!(
        detect_encoding(&ansi, true).unwrap(),
        TextEncoding::Windows1252
    );
    assert_eq!(indexed(ansi).lines(0, 1).unwrap(), vec!["Grüße"]);
    // A UTF-8 sequence cut at the end of the sniffed prefix stays UTF-8,
    // but not at the end of the whole file.
    let cut = &"aä".as_bytes()[..2];
    assert_eq!(detect_encoding(cut, false).unwrap(), TextEncoding::Utf8);
    assert_eq!(
        detect_encoding(cut, true).unwrap(),
        TextEncoding::Windows1252
    );
    // NUL bytes in the first 8 KiB: no text file.
    assert!(matches!(
        TextDocument::from_bytes(b"ab\0cd".to_vec()),
        Err(TextViewError::NotText)
    ));
    let mut nul_unit = utf16("ab", false);
    nul_unit.extend_from_slice(&[0, 0]);
    assert!(matches!(
        TextDocument::from_bytes(nul_unit),
        Err(TextViewError::NotText)
    ));
    // A NUL after the first 8 KiB does not make the file binary.
    let mut late = vec![b'a'; 9000];
    late.push(0);
    assert!(TextDocument::from_bytes(late).is_ok());
    // UTF-16 text decodes line by line, the BOM is not part of line 0.
    let doc = indexed(utf16("eins\r\nzwei\ndrei", false));
    assert_eq!(doc.lines(0, 5).unwrap(), vec!["eins", "zwei", "drei"]);
    let doc = indexed(utf16("Ä\rÖ", true));
    assert_eq!(doc.lines(0, 5).unwrap(), vec!["Ä", "Ö"]);
    let doc = indexed(b"\xEF\xBB\xBFerste\nzweite".to_vec());
    assert_eq!(doc.lines(0, 2).unwrap(), vec!["erste", "zweite"]);
}

#[test]
fn textview_task_utf8_switches_to_ansi_after_sniff() {
    let mut bytes = vec![b'a'; 9000];
    bytes.extend_from_slice(b"\n\xE4rger");
    let doc = TextDocument::from_bytes(bytes).unwrap();
    assert_eq!(doc.encoding(), TextEncoding::Utf8);
    let status = doc
        .build_index(&AtomicBool::new(false), &mut |_| {})
        .unwrap();
    assert_eq!(status.encoding, TextEncoding::Windows1252);
    assert_eq!(doc.lines(1, 1).unwrap(), vec!["ärger"]);
}

#[test]
fn textview_task_line_endings_and_window() {
    let doc = indexed(b"a\r\nb\nc\rd".to_vec());
    assert_eq!(doc.line_count(), Some(4));
    let status = doc.status();
    assert_eq!(
        (status.endings.crlf, status.endings.lf, status.endings.cr),
        (1, 1, 1)
    );
    assert!(status.endings.is_mixed());
    assert_eq!(doc.lines(1, 2).unwrap(), vec!["b", "c"]);
    let lines = doc.read_lines(0, 10).unwrap();
    let endings: Vec<_> = lines.iter().map(|l| l.ending).collect();
    assert_eq!(
        endings,
        vec![
            Some(LineEnding::CrLf),
            Some(LineEnding::Lf),
            Some(LineEnding::Cr),
            None
        ]
    );
    assert_eq!(lines[3].number, 3);
    // A final line ending opens one more (empty) line; an empty file has one.
    let doc = indexed(b"x\n".to_vec());
    assert_eq!(doc.lines(0, 5).unwrap(), vec!["x", ""]);
    assert_eq!(doc.status().endings.dominant(), Some(LineEnding::Lf));
    let doc = indexed(Vec::new());
    assert_eq!(doc.line_count(), Some(1));
    assert_eq!(doc.lines(0, 5).unwrap(), vec![""]);
    // Beyond the end: nothing.
    assert!(doc.lines(7, 3).unwrap().is_empty());
    // CR LF split by the 64 KiB window chunk and the 1 MiB index chunk.
    for split in [64 * 1024, 1024 * 1024] {
        let mut bytes = vec![b'a'; split - 1];
        bytes.extend_from_slice(b"\r\nb\rc");
        let doc = indexed(bytes);
        assert_eq!(doc.line_count(), Some(3), "split {split}");
        assert_eq!(doc.status().endings.crlf, 1);
        let lines = doc.read_lines(0, 3).unwrap();
        assert_eq!(lines[0].text.len(), split - 1);
        assert_eq!(lines[0].ending, Some(LineEnding::CrLf));
        assert_eq!(lines[1].text, "b");
        assert_eq!(lines[2].text, "c");
    }
}

/// `count` lines `Zeile 00000000\n` (15 bytes each), generated on demand.
struct Synthetic {
    count: u64,
    largest_read: AtomicUsize,
    lowest_offset: AtomicU64,
}

const SYNTHETIC_LINE: u64 = 15;

impl TextSource for Synthetic {
    fn len(&self) -> u64 {
        self.count * SYNTHETIC_LINE
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.largest_read.fetch_max(buf.len(), Ordering::Relaxed);
        self.lowest_offset.fetch_min(offset, Ordering::Relaxed);
        let len = self.len();
        let mut n = 0;
        while n < buf.len() && offset + (n as u64) < len {
            let pos = offset + n as u64;
            let (line, column) = (pos / SYNTHETIC_LINE, (pos % SYNTHETIC_LINE) as usize);
            let text = format!("Zeile {line:08}\n");
            let take = (text.len() - column).min(buf.len() - n);
            buf[n..n + take].copy_from_slice(&text.as_bytes()[column..column + take]);
            n += take;
        }
        Ok(n)
    }
}

#[test]
fn textview_task_large_source_without_full_memory() {
    let source = Arc::new(Synthetic {
        count: 300_000,
        largest_read: AtomicUsize::new(0),
        lowest_offset: AtomicU64::new(u64::MAX),
    });
    let doc = TextDocument::open(source.clone()).unwrap();
    // Usable before the index exists: walks from the start.
    assert_eq!(doc.lines(2, 1).unwrap(), vec!["Zeile 00000002"]);
    let mut reports = 0;
    let status = doc
        .build_index(&AtomicBool::new(false), &mut |_| reports += 1)
        .unwrap();
    assert!(status.complete);
    assert!(reports > 1, "progress after every chunk");
    // The final LF opens one more, empty line.
    assert_eq!(doc.line_count(), Some(300_001));
    assert!(source.largest_read.load(Ordering::Relaxed) <= 1024 * 1024);
    // A window read starts at the stored offset of its stride, not at 0.
    source.lowest_offset.store(u64::MAX, Ordering::Relaxed);
    let first = 250_000;
    let lines = doc.lines(first, 3).unwrap();
    assert_eq!(
        lines,
        vec!["Zeile 00250000", "Zeile 00250001", "Zeile 00250002"]
    );
    let stride_start = first / LINE_STRIDE * LINE_STRIDE * SYNTHETIC_LINE;
    assert!(source.lowest_offset.load(Ordering::Relaxed) >= stride_start);
    assert_eq!(doc.lines(300_000, 5).unwrap(), vec![""]);
}

#[test]
fn textview_task_index_cancel_and_restart() {
    let doc = TextDocument::from_bytes(vec![b'x'; 10]).unwrap();
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        doc.build_index(&cancel, &mut |_| {}),
        Err(TextViewError::Cancelled)
    ));
    assert_eq!(doc.line_count(), None);
    cancel.store(false, Ordering::Relaxed);
    doc.build_index(&cancel, &mut |_| {}).unwrap();
    assert_eq!(doc.line_count(), Some(1));
}
