//! Unix (Linux, Android) filesystem facts for transfers.
use std::path::{Path, PathBuf};

/// Uploads never follow links: a symlink source is refused, not resolved.
pub(crate) fn upload_is_link_like(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// `\` is an ordinary character in Unix file names.
pub(crate) fn backslash_is_name_char() -> bool {
    true
}

pub(crate) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(src, dest)
}

/// The folder `path` resolves to, links on the way followed.
pub(crate) fn canonical_folder(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// No free-space probe here; the download preflight is then skipped.
pub(crate) fn available_space_for_path(_path: &Path) -> Option<u64> {
    None
}

/// Memory currently available to programs (`MemAvailable` on Linux and
/// Android), for the transfer budget.
pub(crate) fn available_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Total physical memory (`MemTotal` on Linux and Android). Stable from run to
/// run, unlike the available memory, so limits derived from it do not change
/// with the momentary load (sync tree limits, RV1).
pub(crate) fn physical_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Writes all of `buf` at `offset` without moving the file pointer, so
/// several writers can fill different ranges of one file at once.
pub(crate) fn write_all_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, buf, offset)
}

#[cfg(test)]
mod tests {
    use super::write_all_at;
    use std::io::Read;

    #[test]
    fn speed_task_write_all_at_positions_data() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("ranges.bin");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create");
        file.set_len(10).expect("length");
        write_all_at(&file, b"abc", 4).expect("later range first");
        write_all_at(&file, b"xy", 0).expect("earlier range second");
        drop(file);
        let mut content = Vec::new();
        std::fs::File::open(&path)
            .expect("open")
            .read_to_end(&mut content)
            .expect("read");
        assert_eq!(content, b"xy\0\0abc\0\0\0");
    }
}
