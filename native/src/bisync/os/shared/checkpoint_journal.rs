//! Fsynced append-only checkpoint frames. A torn final frame is ignored and
//! truncated before further appends; fully written corrupt frames are errors.
//! Compaction writes baseline and folder history before retiring the journal.
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::baseline_records::RecordBook;
use super::keys::KeyPolicy;
use super::plan_types::Record;
use super::replica_state::baseline_file;
use super::run_types::StateKey;
use super::snapshot_types::DirSet;
use super::state_metadata::{load_dirs, save_dirs};

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Frame {
    #[serde(default)]
    pub fold_case: bool,
    pub records: Vec<Record>,
    pub forget: Vec<String>,
    pub dirs_add: Vec<String>,
    pub dirs_remove: Vec<String>,
}

pub(super) struct Journal {
    path: PathBuf,
    valid_bytes: u64,
    keys: KeyPolicy,
}

impl Journal {
    pub fn load(key: &StateKey, keys: KeyPolicy) -> io::Result<(Self, RecordBook, Option<DirSet>)> {
        let baseline = super::load_baseline(&baseline_file(key)?)?;
        let mut records = RecordBook::new(baseline, keys);
        let mut dirs = load_dirs(key)?;
        let path = baseline_file(key)?.with_extension("journal");
        let mut journal = Self {
            path,
            valid_bytes: 0,
            keys,
        };
        let backend = crate::vfs::LocalBackend::new("/");
        let path = journal
            .path
            .to_str()
            .ok_or_else(|| io::Error::other("checkpoint path is not Unicode"))?;
        let metadata = match crate::vfs::Backend::stat(&backend, path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Frame::default().ensure_fits(
                    &records,
                    dirs.as_ref().unwrap_or(&DirSet::new()),
                    dirs.as_ref().map_or(0, dir_bytes),
                    super::SyncLimits::for_memory(crate::transfer::physical_memory()),
                )?;
                return Ok((journal, records, dirs));
            }
            Err(error) => return Err(error),
        };
        let limit = journal_limit();
        if metadata.is_dir || metadata.is_symlink || metadata.special || metadata.size > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "checkpoint journal exceeds its budget",
            ));
        }
        let mut file = crate::vfs::open_read_regular(&backend, path, None)?;
        loop {
            let mut length = [0u8; 8];
            if !read_complete(&mut file, &mut length)? {
                break;
            }
            let length = u64::from_be_bytes(length);
            if length > limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "checkpoint frame exceeds its budget",
                ));
            }
            if length
                > metadata
                    .size
                    .saturating_sub(journal.valid_bytes)
                    .saturating_sub(40)
            {
                break;
            }
            let mut digest = [0u8; 32];
            if !read_complete(&mut file, &mut digest)? {
                break;
            }
            let count = usize::try_from(length)
                .map_err(|_| io::Error::other("checkpoint frame is too large"))?;
            let mut bytes = vec![0u8; count];
            if !read_complete(&mut file, &mut bytes)? {
                break;
            }
            if Sha256::digest(&bytes).as_slice() != digest {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "checkpoint frame checksum mismatch",
                ));
            }
            let frame: Frame = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            frame.apply(&mut records, dirs.get_or_insert_with(DirSet::new))?;
            journal.valid_bytes = journal.valid_bytes.saturating_add(40 + length);
        }
        records.set_keys(keys);
        Frame::default().ensure_fits(
            &records,
            dirs.as_ref().unwrap_or(&DirSet::new()),
            dirs.as_ref().map_or(0, dir_bytes),
            super::SyncLimits::for_memory(crate::transfer::physical_memory()),
        )?;
        Ok((journal, records, dirs))
    }

    pub fn append(&mut self, frame: &Frame) -> io::Result<()> {
        frame.validate()?;
        if frame.fold_case != self.keys.fold_case {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "checkpoint uses another pair key policy",
            ));
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(frame).map_err(io::Error::other)?;
        if self
            .valid_bytes
            .saturating_add(40)
            .saturating_add(bytes.len() as u64)
            > journal_limit()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "checkpoint journal exceeds its budget",
            ));
        }
        let mut file = match std::fs::symlink_metadata(&self.path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => OpenOptions::new()
                .write(true)
                .append(true)
                .open(&self.path)?,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "checkpoint journal is not a regular file",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => OpenOptions::new()
                .write(true)
                .append(true)
                .create_new(true)
                .open(&self.path)?,
            Err(error) => return Err(error),
        };
        file.set_len(self.valid_bytes)?;
        file.write_all(&(bytes.len() as u64).to_be_bytes())?;
        file.write_all(&Sha256::digest(&bytes))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        self.valid_bytes = self.valid_bytes.saturating_add(40 + bytes.len() as u64);
        let backend = crate::vfs::LocalBackend::new("/");
        let path = self
            .path
            .to_str()
            .ok_or_else(|| io::Error::other("checkpoint path is not Unicode"))?;
        crate::vfs::sync_filesystem(&backend, path).map(|_| ())
    }

    pub fn compact(
        &mut self,
        key: &StateKey,
        records: &RecordBook,
        dirs: &DirSet,
    ) -> io::Result<()> {
        super::save_baseline(&baseline_file(key)?, &records.baseline)?;
        save_dirs(key, dirs)?;
        match std::fs::remove_file(&self.path) {
            Ok(()) => {
                self.valid_bytes = 0;
                let backend = crate::vfs::LocalBackend::new("/");
                let parent = self
                    .path
                    .parent()
                    .and_then(|path| path.to_str())
                    .ok_or_else(|| io::Error::other("checkpoint parent is not Unicode"))?;
                crate::vfs::sync_filesystem(&backend, parent)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.valid_bytes = 0,
            Err(error) => return Err(error),
        }
        Ok(())
    }
}

impl Frame {
    pub fn ensure_fits(
        &self,
        records: &RecordBook,
        dirs: &DirSet,
        bytes: u64,
        limits: super::SyncLimits,
    ) -> io::Result<u64> {
        self.validate()?;
        let (files, text) = records.projected(&self.records, &self.forget);
        let mut count = dirs.len() as u64;
        let mut text_dirs = bytes;
        let removed: std::collections::BTreeSet<_> = self.dirs_remove.iter().collect();
        let added: std::collections::BTreeSet<_> = self.dirs_add.iter().collect();
        for rel in &removed {
            if dirs.contains(*rel) {
                count = count.saturating_sub(1);
                text_dirs = text_dirs.saturating_sub(rel.len() as u64);
            }
        }
        for rel in &added {
            if !dirs.contains(*rel) || removed.contains(rel) {
                count = count.saturating_add(1);
                text_dirs = text_dirs.saturating_add(rel.len() as u64);
            }
        }
        if files.saturating_add(count) > limits.state_entries
            || text.saturating_add(text_dirs) > limits.state_text_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "checkpoint state exceeds its memory budget",
            ));
        }
        Ok(text_dirs)
    }

    fn validate(&self) -> io::Result<()> {
        for rel in self
            .forget
            .iter()
            .chain(self.records.iter().map(|(rel, _)| rel))
            .chain(&self.dirs_add)
            .chain(&self.dirs_remove)
        {
            if crate::agent_proto::ValidatedRelativePath::parse(rel)?.as_str() != rel {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "checkpoint path is not canonical",
                ));
            }
        }
        Ok(())
    }

    pub fn apply(&self, records: &mut RecordBook, dirs: &mut DirSet) -> io::Result<()> {
        self.validate()?;
        records.set_keys(KeyPolicy {
            fold_case: self.fold_case,
        });
        for rel in &self.forget {
            crate::agent_proto::ValidatedRelativePath::parse(rel)?;
            records.forget(rel);
        }
        for (rel, entry) in &self.records {
            crate::agent_proto::ValidatedRelativePath::parse(rel)?;
            records.record(rel, *entry);
        }
        for rel in &self.dirs_remove {
            crate::agent_proto::ValidatedRelativePath::parse(rel)?;
            dirs.remove(rel);
        }
        for rel in &self.dirs_add {
            crate::agent_proto::ValidatedRelativePath::parse(rel)?;
            dirs.insert(rel.clone());
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
            && self.forget.is_empty()
            && self.dirs_add.is_empty()
            && self.dirs_remove.is_empty()
    }
}

pub(super) fn dir_bytes(dirs: &DirSet) -> u64 {
    dirs.iter()
        .fold(0u64, |bytes, rel| bytes.saturating_add(rel.len() as u64))
}

fn journal_limit() -> u64 {
    // JSON signatures and framing need more space than the compact baseline.
    super::SyncLimits::for_memory(crate::transfer::physical_memory())
        .state_file_bytes()
        .saturating_mul(4)
}

fn read_complete(file: &mut impl Read, bytes: &mut [u8]) -> io::Result<bool> {
    match file.read_exact(bytes) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}
