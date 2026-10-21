//! The stages that ranged uploads fill (E13-S2): one table per connection,
//! used by the Share host and the SSH agent alike. A stage is created
//! exclusively by its host, which hands the table the handle it got then;
//! every range is written through that handle, never through the path. A
//! range is entered as taken when it is accepted (`pending`), so two
//! overlapping requests can never both be accepted, and it becomes `covered`
//! only after its last byte came. The table adds up the space all open
//! stages have still to fill, per volume, so sparse files cannot overbook a
//! disk together.
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use super::local_platform::{self, FileIdentity};
use super::range_journal::Journal;
use super::range_space::{lock, merge_in, overlaps, release_global, reserve_global};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

fn unknown_stage() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "Unbekannte Stufe (nicht von dieser Verbindung angelegt)",
    )
}

struct Stage {
    file: Arc<File>,
    path: PathBuf,
    identity: FileIdentity,
    size: u64,
    covered: Vec<(u64, u64)>,
    pending: Vec<(u64, u64)>,
    /// Bytes still to come: what this stage keeps reserved on its volume.
    unwritten: u64,
    key: String,
    /// Milliseconds since the table's start at the last byte or request.
    progress: Arc<AtomicU64>,
}

struct Inner {
    stages: HashMap<String, Stage>,
    /// Stages whose file is being created (they count against the limit).
    opening: usize,
    closed: bool,
}

/// Every table of the process (for the sweep of vanished clients).
fn tables() -> &'static Mutex<Vec<Weak<RangeStages>>> {
    static TABLES: OnceLock<Mutex<Vec<Weak<RangeStages>>>> = OnceLock::new();
    TABLES.get_or_init(|| Mutex::new(Vec::new()))
}

/// The stages of one connection.
pub(crate) struct RangeStages {
    inner: Mutex<Inner>,
    limit: usize,
    stall: Duration,
    epoch: Instant,
    /// Opens the process's list of unfinished stages on first use.
    provider: Option<JournalProvider>,
    journal: OnceLock<Option<Arc<Journal>>>,
}

/// Where a host keeps its list of unfinished stages (`Journal::open`).
pub(crate) type JournalProvider = fn() -> Option<Arc<Journal>>;

impl RangeStages {
    /// A table for one connection: at most `limit` open stages, and a stage
    /// that makes no progress for `stall` is removed.
    pub(crate) fn new(
        limit: usize,
        stall: Duration,
        provider: Option<JournalProvider>,
    ) -> Arc<Self> {
        let table = Arc::new(Self {
            inner: Mutex::new(Inner {
                stages: HashMap::new(),
                opening: 0,
                closed: false,
            }),
            limit,
            stall,
            epoch: Instant::now(),
            provider,
            journal: OnceLock::new(),
        });
        {
            let mut tables = lock(tables());
            tables.retain(|table| table.strong_count() > 0);
            tables.push(Arc::downgrade(&table));
        }
        // The process's list opens with its first table: that is the start
        // that removes the stages a process before it left behind.
        let _ = table.journal();
        table
    }

    fn journal(&self) -> Option<&Arc<Journal>> {
        self.journal
            .get_or_init(|| self.provider.and_then(|open| open()))
            .as_ref()
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Removes the stages of every connection that made no progress for
    /// their stall time (time-out of a client that vanished).
    pub(crate) fn sweep_all() {
        let alive: Vec<Arc<RangeStages>> =
            lock(tables()).iter().filter_map(Weak::upgrade).collect();
        for table in alive {
            table.sweep();
        }
    }

    /// Removes this connection's stages without progress for the stall time.
    pub(crate) fn sweep(&self) {
        let now = self.now_ms();
        let stall = u64::try_from(self.stall.as_millis()).unwrap_or(u64::MAX);
        let expired: Vec<Stage> = {
            let mut inner = lock(&self.inner);
            let names: Vec<String> = inner
                .stages
                .iter()
                .filter(|(_, stage)| {
                    stage.pending.is_empty()
                        && now.saturating_sub(stage.progress.load(Ordering::Relaxed)) >= stall
                })
                .map(|(name, _)| name.clone())
                .collect();
            names
                .iter()
                .filter_map(|name| inner.stages.remove(name))
                .collect()
        };
        for stage in expired {
            self.dispose(stage, true);
        }
    }

    /// Starts opening a stage of `size` bytes in `dir`: checks the limit and
    /// the space (everything the open stages still reserve is counted) and
    /// reserves both. The caller then creates the file exclusively and hands
    /// it to `Pending::commit`.
    pub(crate) fn begin(self: &Arc<Self>, dir: &Path, size: u64) -> io::Result<Pending> {
        Self::sweep_all();
        {
            let mut inner = lock(&self.inner);
            if inner.closed {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Die Verbindung ist beendet",
                ));
            }
            if inner.stages.len() + inner.opening >= self.limit {
                return Err(io::Error::new(
                    io::ErrorKind::ResourceBusy,
                    "Zu viele offene Stufen auf dieser Verbindung",
                ));
            }
            inner.opening += 1;
        }
        let reserved = local_platform::space_key(dir)
            .and_then(|key| reserve_global(&key, dir, size).map(|()| key));
        match reserved {
            Ok(key) => Ok(Pending {
                table: self.clone(),
                key,
                size,
                settled: false,
            }),
            Err(error) => {
                lock(&self.inner).opening -= 1;
                Err(error)
            }
        }
    }

    /// Accepts the range `[offset, offset + len)` of `name`: it must lie
    /// inside the stage and touch neither a finished nor an accepted range.
    pub(crate) fn claim(self: &Arc<Self>, name: &str, offset: u64, len: u64) -> io::Result<Claim> {
        let mut inner = lock(&self.inner);
        if inner.closed {
            return Err(unknown_stage());
        }
        let stage = inner.stages.get_mut(name).ok_or_else(unknown_stage)?;
        let end = offset
            .checked_add(len)
            .filter(|end| len > 0 && *end <= stage.size)
            .ok_or_else(|| invalid("Bereich liegt außerhalb der Stufe"))?;
        if overlaps(&stage.covered, offset, end) || overlaps(&stage.pending, offset, end) {
            return Err(invalid(
                "Bereich überlappt einen fertigen oder belegten Bereich",
            ));
        }
        stage.pending.push((offset, end));
        stage.progress.store(self.now_ms(), Ordering::Relaxed);
        Ok(Claim {
            table: self.clone(),
            name: name.to_string(),
            file: stage.file.clone(),
            progress: stage.progress.clone(),
            offset,
            len,
            written: 0,
            settled: false,
        })
    }

    /// Closes a complete stage: every range arrived, none is running and the
    /// file has the announced length. The stage stays on disk for publication.
    pub(crate) fn finish(&self, name: &str) -> io::Result<()> {
        let stage = {
            let mut inner = lock(&self.inner);
            let stage = inner.stages.get(name).ok_or_else(unknown_stage)?;
            if !stage.pending.is_empty() {
                return Err(invalid("Es laufen noch Bereiche"));
            }
            let complete = stage.size == 0 || stage.covered == [(0, stage.size)];
            if !complete || stage.unwritten != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Nicht alle Bereiche der Stufe sind angekommen",
                ));
            }
            if stage.file.metadata()?.len() != stage.size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Die Stufe hat nicht die angekündigte Länge",
                ));
            }
            inner.stages.remove(name).ok_or_else(unknown_stage)?
        };
        self.dispose(stage, false);
        Ok(())
    }

    /// Removes an unfinished stage of this connection (file and entry).
    /// False when the table does not hold it.
    pub(crate) fn discard(&self, name: &str) -> bool {
        let stage = lock(&self.inner).stages.remove(name);
        match stage {
            Some(stage) => {
                self.dispose(stage, true);
                true
            }
            None => false,
        }
    }

    /// Whether `name` is an unfinished stage of this connection.
    pub(crate) fn holds(&self, name: &str) -> bool {
        lock(&self.inner).stages.contains_key(name)
    }

    /// The connection ended: every unfinished stage goes, and no new one
    /// is accepted.
    pub(crate) fn close(&self) {
        let stages: Vec<Stage> = {
            let mut inner = lock(&self.inner);
            inner.closed = true;
            inner.stages.drain().map(|(_, stage)| stage).collect()
        };
        for stage in stages {
            self.dispose(stage, true);
        }
    }

    /// Stages currently open on this connection.
    pub(crate) fn open_count(&self) -> usize {
        lock(&self.inner).stages.len()
    }

    fn settle(&self, name: &str, offset: u64, len: u64) -> io::Result<()> {
        let key = {
            let mut inner = lock(&self.inner);
            let stage = inner.stages.get_mut(name).ok_or_else(unknown_stage)?;
            let end = offset + len;
            let position = stage
                .pending
                .iter()
                .position(|&range| range == (offset, end))
                .ok_or_else(|| invalid("Bereich ist nicht angenommen"))?;
            stage.pending.swap_remove(position);
            merge_in(&mut stage.covered, offset, end);
            stage.unwritten = stage.unwritten.saturating_sub(len);
            stage.key.clone()
        };
        release_global(&key, len);
        Ok(())
    }

    fn unclaim(&self, name: &str, offset: u64, len: u64) {
        let mut inner = lock(&self.inner);
        if let Some(stage) = inner.stages.get_mut(name) {
            let range = (offset, offset + len);
            if let Some(position) = stage.pending.iter().position(|&pending| pending == range) {
                stage.pending.swap_remove(position);
            }
        }
    }

    /// Gives a stage up: releases its reservation and its list entry and,
    /// when `delete`, removes the file, but only while the path still names
    /// the file this table created.
    fn dispose(&self, stage: Stage, delete: bool) {
        let Stage {
            file,
            path,
            identity,
            key,
            unwritten,
            ..
        } = stage;
        drop(file);
        if delete && local_platform::path_matches_identity(&path, identity).unwrap_or(false) {
            let _ = std::fs::remove_file(&path);
        }
        release_global(&key, unwritten);
        if let Some(journal) = self.journal() {
            journal.done(&path);
        }
    }
}

/// A stage that is being created; dropping it undoes the reservation.
pub(crate) struct Pending {
    table: Arc<RangeStages>,
    key: String,
    size: u64,
    settled: bool,
}

impl Pending {
    /// The stage `name` (the name its client uses) was created exclusively as
    /// `path`, with the length already set; `file` is the handle that
    /// created it.
    pub(crate) fn commit(mut self, name: String, path: PathBuf, file: File) -> io::Result<()> {
        let identity = local_platform::file_identity(&file)?;
        self.settled = true;
        let stage = Stage {
            file: Arc::new(file),
            path: path.clone(),
            identity,
            size: self.size,
            covered: Vec::new(),
            pending: Vec::new(),
            unwritten: self.size,
            key: std::mem::take(&mut self.key),
            progress: Arc::new(AtomicU64::new(self.table.now_ms())),
        };
        let rejected = {
            let mut inner = lock(&self.table.inner);
            inner.opening = inner.opening.saturating_sub(1);
            if inner.closed {
                Some(stage)
            } else {
                inner.stages.insert(name, stage);
                None
            }
        };
        if let Some(stage) = rejected {
            self.table.dispose(stage, true);
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Die Verbindung ist beendet",
            ));
        }
        if let Some(journal) = self.table.journal() {
            journal.add(&path);
        }
        Ok(())
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.settled {
            let mut inner = lock(&self.table.inner);
            inner.opening = inner.opening.saturating_sub(1);
            drop(inner);
            release_global(&self.key, self.size);
        }
    }
}

/// One accepted range. Dropping it before `complete` frees the range for a
/// new request; what it already wrote is overwritten then.
pub(crate) struct Claim {
    table: Arc<RangeStages>,
    name: String,
    file: Arc<File>,
    progress: Arc<AtomicU64>,
    offset: u64,
    len: u64,
    written: u64,
    settled: bool,
}

impl Claim {
    /// Writes the next bytes of the range through the stage's own handle.
    /// More than the announced length is refused.
    pub(crate) fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let end = self
            .written
            .checked_add(data.len() as u64)
            .filter(|end| *end <= self.len)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Mehr Bytes als der Bereich angekündigt hat",
                )
            })?;
        local_platform::write_all_at(&self.file, data, self.offset + self.written)?;
        self.written = end;
        self.progress.store(self.table.now_ms(), Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    pub(crate) fn written(&self) -> u64 {
        self.written
    }

    /// The range is complete: exactly its length came.
    pub(crate) fn complete(mut self) -> io::Result<()> {
        if self.written != self.len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Weniger Bytes als der Bereich angekündigt hat",
            ));
        }
        self.table.settle(&self.name, self.offset, self.len)?;
        self.settled = true;
        Ok(())
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if !self.settled {
            self.table.unclaim(&self.name, self.offset, self.len);
        }
    }
}

/// `<destination>.se-upload-<16 hex>` with 64 random bits of the system
/// source and no fall-back: only the host picks the name, and a stage name
/// nobody can know before it exists needs real randomness.
pub(crate) fn random_stage_name(destination: &str) -> io::Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|error| {
        io::Error::other(format!("Zufallsquelle für den Stufennamen fehlt: {error}"))
    })?;
    let tail = format!(".se-upload-{:016x}", u64::from_le_bytes(bytes));
    let split = destination.rfind(['/', '\\']).map_or(0, |index| index + 1);
    let (parent, name) = destination.split_at(split);
    Ok(format!(
        "{parent}{}",
        crate::vfs::fit_stage_name("", name, &tail)
    ))
}

#[cfg(test)]
#[path = "range_stage_tests.rs"]
mod tests;
