use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::filters::path_has_skipped_segment;
use super::super::model::{validate_path, FolderIndex, MAX_INDEX_FILE_BYTES, MAX_INDEX_PATHS};
use super::super::platform::replace_file;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub(super) enum SaveOutcome {
    Saved,
    Canceled,
}

impl FolderIndex {
    pub fn save(&self, path: &Path) -> io::Result<()> {
        match self.save_cancellable(path, || false)? {
            SaveOutcome::Saved => Ok(()),
            SaveOutcome::Canceled => Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "folder-index save canceled",
            )),
        }
    }

    pub(super) fn save_cancellable<F>(
        &self,
        path: &Path,
        mut canceled: F,
    ) -> io::Result<SaveOutcome>
    where
        F: FnMut() -> bool,
    {
        let (temp_path, temp_file) = create_temp_file(path)?;
        let result = write_and_replace(self, temp_file, &temp_path, path, &mut canceled);
        if matches!(&result, Ok(SaveOutcome::Saved)) {
            return result;
        }
        let cleanup = std::fs::remove_file(&temp_path);
        match (result, cleanup) {
            (result, Ok(())) => result,
            (result, Err(error)) if error.kind() == io::ErrorKind::NotFound => result,
            (Err(primary), Err(cleanup)) => Err(io::Error::new(
                primary.kind(),
                format!("{primary}; temp cleanup also failed: {cleanup}"),
            )),
            (Ok(SaveOutcome::Canceled), Err(cleanup)) => Err(io::Error::new(
                cleanup.kind(),
                format!("folder-index save canceled, but temp cleanup failed: {cleanup}"),
            )),
            (Ok(SaveOutcome::Saved), Err(_)) => Ok(SaveOutcome::Saved),
        }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "folder index is not a regular file",
            ));
        }
        if metadata.len() > MAX_INDEX_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("folder index exceeds {MAX_INDEX_FILE_BYTES} persisted bytes"),
            ));
        }

        let mut index = FolderIndex::new();
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut line_count = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            line_count = line_count.checked_add(1).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "folder-index line count overflow",
                )
            })?;
            if line_count > MAX_INDEX_PATHS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("folder index exceeds {MAX_INDEX_PATHS} lines"),
                ));
            }
            let path = line.strip_suffix('\n').unwrap_or(&line);
            let path = path.strip_suffix('\r').unwrap_or(path);
            if path.is_empty() || path_has_skipped_segment(path) {
                continue;
            }
            index.try_insert(path.to_string())?;
        }
        Ok(index)
    }
}

fn write_and_replace<F>(
    index: &FolderIndex,
    file: File,
    temp_path: &Path,
    target: &Path,
    canceled: &mut F,
) -> io::Result<SaveOutcome>
where
    F: FnMut() -> bool,
{
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    let mut path_count = 0usize;
    let mut text_bytes = 0usize;
    for path in index.iter() {
        if canceled() {
            return Ok(SaveOutcome::Canceled);
        }
        validate_path(path)?;
        path_count = path_count.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "folder-index path count overflow",
            )
        })?;
        text_bytes = text_bytes.checked_add(path.len()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "folder-index text budget overflow",
            )
        })?;
        if path_count > MAX_INDEX_PATHS
            || text_bytes > super::super::model::MAX_INDEX_PATH_TEXT_BYTES
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "folder index exceeds persistence limits",
            ));
        }
        writer.write_all(path.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    let file = writer.into_inner().map_err(|error| error.into_error())?;
    file.sync_all()?;
    drop(file);
    if canceled() {
        return Ok(SaveOutcome::Canceled);
    }
    replace_file(temp_path, target)?;
    Ok(SaveOutcome::Saved)
}

fn create_temp_file(target: &Path) -> io::Result<(PathBuf, File)> {
    let file_name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "folder-index target has no file name",
        )
    })?;
    for _ in 0..32 {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = file_name.to_os_string();
        temp_name.push(format!(".{}.{}.tmp", std::process::id(), id));
        let temp_path = target.with_file_name(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique folder-index temp file",
    ))
}
