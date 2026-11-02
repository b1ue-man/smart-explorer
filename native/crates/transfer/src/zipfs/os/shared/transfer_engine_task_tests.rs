//! Transfer-engine task tests for ZIP reading (plan K7): a large entry
//! streams in bounded chunks instead of being decompressed into memory as a
//! whole, and every entry size reads back exactly through the backend.
use super::entry::{self, ArchiveFile, STREAM_CHUNK};
use super::ZipBackend;
use crate::vfs::Backend;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index * 7 % 251) as u8).collect()
}

/// Writes a deflated archive; returns its folder and its forward-slash path.
fn temp_zip(tag: &str, entries: &[(&str, Vec<u8>)]) -> (PathBuf, String) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "se_zip_transfer_{tag}_{}_{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&base).unwrap();
    let path = base.join("arc.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
    let forward = path.to_string_lossy().replace('\\', "/");
    (base, forward)
}

#[test]
fn transfer_engine_task_zip_large_entry_streams_in_bounded_chunks() {
    let big = pattern(STREAM_CHUNK * 12 + 17);
    let (base, zip_path) = temp_zip("stream", &[("big.bin", big.clone())]);

    let file = std::fs::File::open(base.join("arc.zip")).unwrap();
    let archive = zip::ZipArchive::new(ArchiveFile::opened(base.join("arc.zip"), file)).unwrap();
    let mut streamed = entry::stream(archive, 0).unwrap();
    let mut first = vec![0u8; 1024];
    streamed.read_exact(&mut first).unwrap();
    assert_eq!(first, big[..1024]);
    // However long the reader pauses, the decompressing thread is at most
    // one chunk ahead: one handed over, one queued, one waiting to be sent.
    std::thread::sleep(Duration::from_millis(200));
    let produced = streamed.produced.load(Ordering::SeqCst);
    assert!(
        produced <= 3,
        "the thread decompressed {produced} chunks ahead"
    );
    let mut rest = Vec::new();
    streamed.read_to_end(&mut rest).unwrap();
    assert_eq!([first, rest].concat(), big);

    let backend = ZipBackend::open(&zip_path).unwrap();
    let mut all = Vec::new();
    backend
        .open_read("/big.bin")
        .unwrap()
        .read_to_end(&mut all)
        .unwrap();
    assert_eq!(all, big);
    drop(backend);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn transfer_engine_task_zip_every_entry_size_reads_back_exactly() {
    let entries = [
        ("a.txt", b"hello".to_vec()),
        ("empty", Vec::new()),
        ("edge.bin", pattern(STREAM_CHUNK)),
        ("over.bin", pattern(STREAM_CHUNK + 1)),
    ];
    let (base, zip_path) = temp_zip("sizes", &entries);
    let backend = ZipBackend::open(&zip_path).unwrap();
    for (name, expected) in &entries {
        let mut out = Vec::new();
        backend
            .open_read(&format!("/{name}"))
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(&out, expected, "{name}");
    }
    // Decompression is CPU work: at most one reader per core.
    assert!(backend
        .transfer_ceiling("/a.txt")
        .is_some_and(|cores| cores >= 1));
    drop(backend);
    let _ = std::fs::remove_dir_all(base);
}
