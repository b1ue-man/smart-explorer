//! A text log whose size is bounded by rotation (plan 2026-11-17 P9, the
//! screen host log): once the next line would take the current file over
//! `max_bytes`, it becomes `<name>.1.<ext>`, the older files move one number
//! up and the oldest beyond `keep` goes. No line is ever dropped: a line
//! longer than `max_bytes` gets a file of its own.
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub struct RotatingLog {
    path: PathBuf,
    max_bytes: u64,
    keep: usize,
    file: Option<File>,
    size: u64,
}

/// `screen-host.log` with `index` 2 is `screen-host.2.log`; index 0 is the
/// current file.
pub fn rotated_path(path: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match path.extension() {
        Some(extension) => format!("{stem}.{index}.{}", extension.to_string_lossy()),
        None => format!("{stem}.{index}"),
    };
    path.with_file_name(name)
}

impl RotatingLog {
    /// A log at `path` of at most `max_bytes` per file and `keep` older files.
    pub fn new(path: PathBuf, max_bytes: u64, keep: usize) -> Self {
        Self {
            path,
            max_bytes: max_bytes.max(1),
            keep,
            file: None,
            size: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open(&mut self) -> io::Result<&mut File> {
        if self.file.is_none() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            self.size = file.metadata()?.len();
            self.file = Some(file);
        }
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file not open"))
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        if self.keep == 0 {
            std::fs::remove_file(&self.path).or_else(absent)?;
        } else {
            std::fs::remove_file(rotated_path(&self.path, self.keep)).or_else(absent)?;
            for index in (1..self.keep).rev() {
                let from = rotated_path(&self.path, index);
                if from.exists() {
                    std::fs::rename(&from, rotated_path(&self.path, index + 1))?;
                }
            }
            std::fs::rename(&self.path, rotated_path(&self.path, 1)).or_else(absent)?;
        }
        self.size = 0;
        Ok(())
    }

    /// Appends `line` and a line end.
    pub fn append(&mut self, line: &str) -> io::Result<()> {
        let bytes = line.len() as u64 + 1;
        self.open()?;
        if self.size > 0 && self.size.saturating_add(bytes) > self.max_bytes {
            self.rotate()?;
        }
        let file = self.open()?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        self.size = self.size.saturating_add(bytes);
        Ok(())
    }
}

fn absent(error: io::Error) -> io::Result<()> {
    if error.kind() == io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn p9_task_rotating_log_bounds_its_files_and_keeps_every_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("screen-host.log");
        let mut log = RotatingLog::new(path.clone(), 30, 2);
        for index in 0..10 {
            log.append(&format!("line {index:02} of the host")).unwrap();
        }
        // 20 bytes per line: one line per file, the newest three kept.
        assert_eq!(read(&path), "line 09 of the host\n");
        assert_eq!(read(&rotated_path(&path, 1)), "line 08 of the host\n");
        assert_eq!(read(&rotated_path(&path, 2)), "line 07 of the host\n");
        assert!(!rotated_path(&path, 3).exists());
        for index in 0..=2 {
            let size = std::fs::metadata(rotated_path(&path, index)).unwrap().len();
            assert!(size <= 30, "file {index} has {size} bytes");
        }
        // A line longer than a file gets a file of its own, never dropped.
        let long = "x".repeat(100);
        log.append(&long).unwrap();
        assert_eq!(read(&path), format!("{long}\n"));
        log.append("after").unwrap();
        assert_eq!(read(&path), "after\n");
        assert_eq!(read(&rotated_path(&path, 1)), format!("{long}\n"));
        // A new log on the same path continues the current file.
        let mut again = RotatingLog::new(path.clone(), 30, 2);
        again.append("next").unwrap();
        assert_eq!(read(&path), "after\nnext\n");
    }

    #[test]
    fn p9_task_rotating_log_names_its_older_files() {
        let path = Path::new("/data/logs/screen-host.log");
        assert_eq!(rotated_path(path, 0), path);
        assert_eq!(
            rotated_path(path, 3),
            Path::new("/data/logs/screen-host.3.log")
        );
        assert_eq!(
            rotated_path(Path::new("/data/plain"), 1),
            Path::new("/data/plain.1")
        );
    }
}
