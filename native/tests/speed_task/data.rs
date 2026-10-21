//! Test data of the speed cases (M-4): incompressible files, small-file trees,
//! the mixed tree and word-salad text. Everything is deterministic where the
//! plan names a seed, and incompressible where it says random bytes.
#![allow(dead_code)]

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const KIB: u64 = 1024;
pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
/// Seed of the mixed tree (M-4).
pub const MIXED_SEED: u64 = 20_261_021;
/// Files and size range of the mixed tree (M-4).
pub const MIXED_FILES: usize = 300;
pub const MIXED_MIN: f64 = 1024.0;
pub const MIXED_MAX: f64 = 16.0 * 1024.0 * 1024.0;

/// A fresh folder below `parent` that is removed when the case ends, also
/// when it fails: the large files of many cases must not fill the runner.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(parent: &Path, name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let path = parent.join(format!("{name}-{nanos}"));
        std::fs::create_dir_all(&path).expect("create the scratch folder");
        Scratch(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// A fresh subfolder.
    pub fn sub(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(&path).expect("create a scratch subfolder");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// xorshift64*: fast, deterministic, and good enough to defeat LZ4.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    /// A seed that differs per call, for files whose bytes need not repeat.
    pub fn fresh() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(1);
        Rng::new(nanos ^ 0x9e37_79b9_7f4a_7c15)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn fill(&mut self, buffer: &mut [u8]) {
        for chunk in buffer.chunks_mut(8) {
            let word = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
    }
}

/// A file of `bytes` random bytes, written in 1 MiB steps.
pub fn random_file(path: &Path, bytes: u64) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create the folder of the random file");
    }
    let mut rng = Rng::fresh();
    let mut writer = BufWriter::with_capacity(
        MIB as usize,
        File::create(path).expect("create the random file"),
    );
    let mut block = vec![0u8; MIB as usize];
    let mut left = bytes;
    while left > 0 {
        let step = left.min(MIB) as usize;
        rng.fill(&mut block[..step]);
        writer
            .write_all(&block[..step])
            .expect("write random bytes");
        left -= step as u64;
    }
    writer.flush().expect("flush the random file");
}

/// The fixed words of the text files (M-4: a ratio of about 0.3 is expected
/// from LZ4 on this vocabulary).
const WORDS: [&str; 24] = [
    "transfer", "engine", "range", "stage", "folder", "window", "server", "client", "packet",
    "stream", "credit", "flow", "block", "queue", "share", "agent", "drive", "remote", "local",
    "rename", "publish", "commit", "verify", "cache",
];

/// A text file of `bytes` bytes made of fixed words in a seeded order.
pub fn text_file(path: &Path, bytes: u64) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create the folder of the text file");
    }
    let mut rng = Rng::new(MIXED_SEED);
    let mut writer = BufWriter::with_capacity(
        MIB as usize,
        File::create(path).expect("create the text file"),
    );
    let mut block = Vec::with_capacity(MIB as usize + 64);
    let mut left = bytes;
    while left > 0 {
        block.clear();
        while block.len() < MIB as usize {
            let word = WORDS[(rng.next_u64() % WORDS.len() as u64) as usize];
            block.extend_from_slice(word.as_bytes());
            block.push(if rng.next_u64() % 12 == 0 {
                b'\n'
            } else {
                b' '
            });
        }
        let step = left.min(block.len() as u64) as usize;
        writer.write_all(&block[..step]).expect("write text");
        left -= step as u64;
    }
    writer.flush().expect("flush the text file");
}

/// The LZ4 ratio of the first 256 KiB of `path` (compressed ÷ raw), for the
/// report; 1.0 when the file is shorter than one byte.
pub fn lz4_ratio(path: &Path) -> f64 {
    use std::io::Read;
    let mut sample = vec![0u8; 256 * 1024];
    let read = File::open(path)
        .and_then(|mut file| file.read(&mut sample))
        .unwrap_or(0);
    if read == 0 {
        return 1.0;
    }
    let packed = lz4_flex::block::compress(&sample[..read]);
    packed.len() as f64 / read as f64
}

/// `files` files of `bytes` bytes in `folders` folders (`d000`…, `f00000.bin`…),
/// each with its own pseudo-random content. Returns the root.
pub fn small_tree(root: &Path, folders: usize, files: usize, bytes: usize) -> PathBuf {
    let mut content = vec![0u8; bytes];
    for index in 0..files {
        let folder = root.join(format!("d{:03}", index % folders.max(1)));
        if index < folders.max(1) {
            std::fs::create_dir_all(&folder).expect("create a small-tree folder");
        }
        Rng::new(index as u64 + 1).fill(&mut content);
        std::fs::write(folder.join(format!("f{index:05}.bin")), &content)
            .expect("write a small file");
    }
    root.to_path_buf()
}

/// The mixed tree (M-4): 300 files, sizes uniform in the logarithm between
/// 1 KiB and 16 MiB, random content, deterministic for `seed`. Returns the
/// total number of bytes.
pub fn mixed_tree(root: &Path, seed: u64) -> u64 {
    std::fs::create_dir_all(root).expect("create the mixed-tree folder");
    let mut rng = Rng::new(seed);
    let mut total = 0u64;
    let ratio = MIXED_MAX / MIXED_MIN;
    for index in 0..MIXED_FILES {
        let size = (MIXED_MIN * ratio.powf(rng.unit())) as u64;
        let path = root.join(format!("m{index:03}.bin"));
        let mut content = vec![0u8; size as usize];
        rng.fill(&mut content);
        std::fs::write(&path, &content).expect("write a mixed-tree file");
        total += size;
    }
    total
}

/// Files and bytes below `root`, for the report of a tree case.
pub fn tree_totals(root: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if let Ok(meta) = entry.metadata() {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    (files, bytes)
}
