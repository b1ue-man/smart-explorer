//! The stage several writers fill over SFTP (E13, U48-4), built and switched
//! on behind the check of the folder chain (security answer S7): it assembles
//! a file from ranges at offsets, refuses unsafe folders, and looks at every
//! range handle. No server: the stage lives in memory.
use super::io_adapters::Commit;
use super::pool_writer::check_length;
use super::segmented_stage::{
    chain_is_safe, folder_is_safe, open_with, parent_of, FolderFacts, HandleCheck, StageWire,
    SFTP_SEGMENTED_WRITE,
};
use russh_sftp::protocol::FileAttributes;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A server's files in memory; creation, owner, folders and ranges behave
/// like the wire.
#[derive(Clone)]
struct Store {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    creates: Arc<AtomicUsize>,
    chain_probes: Arc<AtomicUsize>,
    verdicts: Arc<Mutex<HashMap<String, bool>>>,
    channel: bool,
    /// The owner the server shows for the stage.
    owner: Option<u32>,
    /// The folders from the stage's parent up to `/`.
    chain: Option<Vec<FolderFacts>>,
    /// Handles are checked against this account (`None`: as the owner).
    handle_owner: Arc<Mutex<Option<u32>>>,
}

const ME: u32 = 1000;
const DIR: u32 = 0o040_000;

fn folder(uid: u32, bits: u32) -> FolderFacts {
    FolderFacts {
        uid: Some(uid),
        mode: Some(DIR | bits),
    }
}

/// `/home/me/up` below `/home/me` below `/home` below `/`.
fn good_chain() -> Vec<FolderFacts> {
    vec![
        folder(ME, 0o755),
        folder(ME, 0o755),
        folder(0, 0o755),
        folder(0, 0o755),
    ]
}

impl Store {
    fn new(channel: bool) -> Self {
        Self {
            files: Arc::new(Mutex::new(HashMap::new())),
            creates: Arc::new(AtomicUsize::new(0)),
            chain_probes: Arc::new(AtomicUsize::new(0)),
            verdicts: Arc::new(Mutex::new(HashMap::new())),
            channel,
            owner: Some(ME),
            chain: Some(good_chain()),
            handle_owner: Arc::new(Mutex::new(None)),
        }
    }

    fn with_chain(chain: Option<Vec<FolderFacts>>) -> Self {
        Self {
            chain,
            ..Self::new(true)
        }
    }

    fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(name).cloned()
    }
}

struct RangeWriter {
    store: Store,
    stage: String,
    offset: u64,
    expected: u64,
    accepted: u64,
}

impl Write for RangeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.accepted + data.len() as u64 > self.expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "grown"));
        }
        let mut files = self.store.files.lock().unwrap();
        let file = files
            .get_mut(&self.stage)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let at = (self.offset + self.accepted) as usize;
        if file.len() < at + data.len() {
            file.resize(at + data.len(), 0);
        }
        file[at..at + data.len()].copy_from_slice(data);
        self.accepted += data.len() as u64;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        check_length(Some(self.expected), self.accepted)
    }
}

impl StageWire for Store {
    fn create(&self, stage: &str) -> io::Result<bool> {
        if !self.channel {
            return Ok(false);
        }
        let mut files = self.files.lock().unwrap();
        if files.contains_key(stage) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        files.insert(stage.to_string(), Vec::new());
        self.creates.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    }

    fn account_of(&self, _stage: &str) -> io::Result<Option<u32>> {
        Ok(self.owner)
    }

    fn folder_chain(&self, _parent: &str) -> io::Result<Option<Vec<FolderFacts>>> {
        self.chain_probes.fetch_add(1, Ordering::AcqRel);
        Ok(self.chain.clone())
    }

    fn verdict(&self, parent: &str) -> Option<bool> {
        self.verdicts.lock().unwrap().get(parent).copied()
    }

    fn remember(&self, parent: &str, safe: bool) {
        self.verdicts
            .lock()
            .unwrap()
            .insert(parent.to_string(), safe);
    }

    fn range_writer(
        &self,
        stage: &str,
        offset: u64,
        len: u64,
        check: HandleCheck,
    ) -> io::Result<Box<dyn Write + Send>> {
        let content = self
            .file(stage)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        // What `fstat` of the handle shows.
        let shown = FileAttributes {
            size: Some(content.len() as u64),
            uid: self.handle_owner.lock().unwrap().or(self.owner),
            user: None,
            gid: None,
            group: None,
            permissions: Some(0o100_644),
            atime: None,
            mtime: None,
        };
        check.verify(&shown)?;
        Ok(Box::new(RangeWriter {
            store: self.clone(),
            stage: stage.to_string(),
            offset,
            expected: len,
            accepted: 0,
        }))
    }

    fn size_of(&self, stage: &str) -> io::Result<u64> {
        self.file(stage)
            .map(|file| file.len() as u64)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn discard(&self, stage: &str) -> io::Result<()> {
        self.files.lock().unwrap().remove(stage);
        Ok(())
    }
}

const STAGE: &str = "/home/me/up/file.bin.se-upload-00ff";

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 253) as u8).collect()
}

#[test]
fn speed_task_sftp_segmented_stage_writes_ranges_at_offsets() {
    let store = Store::new(true);
    let content = data(30);
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    assert_eq!(stage.stage_path(), STAGE);
    assert_eq!(store.creates.load(Ordering::Acquire), 1, "created once");
    // Three writers open at once and write in an interleaved order.
    let mut last = stage.segment(20, 10).unwrap();
    let mut first = stage.segment(0, 10).unwrap();
    let mut middle = stage.segment(10, 10).unwrap();
    middle.write_all(&content[10..15]).unwrap();
    last.write_all(&content[20..30]).unwrap();
    first.write_all(&content[0..10]).unwrap();
    middle.write_all(&content[15..20]).unwrap();
    for writer in [&mut last, &mut first, &mut middle] {
        writer.flush().unwrap();
    }
    stage.finish().unwrap();
    assert_eq!(store.file(STAGE), Some(content), "the ranges made the file");
    assert_eq!(store.creates.load(Ordering::Acquire), 1, "no range creates");
}

#[test]
fn speed_task_sftp_segmented_stage_finish_checks_the_stage_length() {
    let store = Store::new(true);
    let content = data(30);
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    let mut first = stage.segment(0, 20).unwrap();
    first.write_all(&content[..20]).unwrap();
    first.flush().unwrap();
    let error = stage.finish().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "10 bytes missing");
    // A writer that got fewer bytes than its range fails at flush.
    let stage = open_with(&store, "/home/me/up/other.se-upload-01", 30, true)
        .unwrap()
        .unwrap();
    let mut short = stage.segment(0, 10).unwrap();
    short.write_all(&content[..9]).unwrap();
    assert_eq!(
        short.flush().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    // A range behind the end of the stage is refused before anything opens.
    let error = stage.segment(25, 10).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    let error = stage.segment(u64::MAX, 2).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn speed_task_sftp_segmented_stage_abort_removes_only_the_own_stage() {
    let store = Store::new(true);
    store
        .files
        .lock()
        .unwrap()
        .insert("/home/me/up/foreign".into(), b"keep".to_vec());
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    let mut writer = stage.segment(0, 10).unwrap();
    writer.write_all(&data(10)).unwrap();
    drop(writer);
    stage.abort();
    assert_eq!(store.file(STAGE), None, "the own stage is gone");
    assert_eq!(
        store.file("/home/me/up/foreign"),
        Some(b"keep".to_vec()),
        "a foreign file stays"
    );
    // A name that is taken is never adopted, written or removed.
    let error = open_with(&store, "/home/me/up/foreign", 30, true)
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(store.file("/home/me/up/foreign"), Some(b"keep".to_vec()));
}

#[test]
fn speed_task_sftp_segmented_write_needs_no_channel_and_the_switch() {
    // Switched off, or without a pool channel: streams as before.
    let store = Store::new(true);
    assert!(open_with(&store, STAGE, 30, false).unwrap().is_none());
    assert_eq!(store.creates.load(Ordering::Acquire), 0, "nothing created");
    let no_channel = Store::new(false);
    assert!(open_with(&no_channel, STAGE, 30, true).unwrap().is_none());
    // The answer to S7 turned the switch on, behind the check of the folders.
    let switch = std::hint::black_box(SFTP_SEGMENTED_WRITE);
    assert!(switch);
    assert!(!Commit::Unsynced.syncs(true));
}

#[test]
fn speed_task_sftp_segmented_write_folder_chain_rules() {
    // Every folder to `/` is ours or root's and closed to group and others.
    assert!(chain_is_safe(ME, &good_chain()));
    // A foreign owner anywhere on the way.
    let mut foreign = good_chain();
    foreign[1] = folder(2000, 0o755);
    assert!(!chain_is_safe(ME, &foreign));
    // Writable by the group, or by others.
    let mut group = good_chain();
    group[2] = folder(0, 0o775);
    assert!(!chain_is_safe(ME, &group));
    let mut others = good_chain();
    others[0] = folder(ME, 0o757);
    assert!(!chain_is_safe(ME, &others));
    // Sticky and owned by root (like /tmp): fine even when others write it.
    let mut tmp = good_chain();
    tmp[1] = folder(0, 0o1777);
    assert!(chain_is_safe(ME, &tmp));
    // Sticky but owned by someone else, even by us, does not count.
    assert!(!folder_is_safe(ME, folder(ME, 0o1777)));
    assert!(!folder_is_safe(ME, folder(2000, 0o1777)));
    // The owner or the mode missing: not safe. So is no folder at all, and
    // anything that is not a folder.
    let mut unknown_owner = good_chain();
    unknown_owner[3] = FolderFacts {
        uid: None,
        mode: Some(DIR | 0o755),
    };
    assert!(!chain_is_safe(ME, &unknown_owner));
    let mut unknown_mode = good_chain();
    unknown_mode[0] = FolderFacts {
        uid: Some(ME),
        mode: None,
    };
    assert!(!chain_is_safe(ME, &unknown_mode));
    assert!(!chain_is_safe(ME, &[]));
    assert!(!folder_is_safe(
        ME,
        FolderFacts {
            uid: Some(ME),
            mode: Some(0o100_644)
        }
    ));
    assert_eq!(parent_of("/a/b/f.se-upload-1"), "/a/b");
    assert_eq!(parent_of("/f.se-upload-1"), "/");
    assert_eq!(parent_of("f.se-upload-1"), ".");
}

#[test]
fn speed_task_sftp_segmented_write_is_refused_on_an_unsafe_chain() {
    let mut writable = good_chain();
    writable[0] = folder(ME, 0o777);
    for (chain, why) in [
        (Some(writable), "writable by others"),
        (None, "the server cannot say"),
        (
            Some(vec![FolderFacts {
                uid: None,
                mode: None,
            }]),
            "no owner, no mode",
        ),
    ] {
        let store = Store::with_chain(chain);
        let stage = open_with(&store, STAGE, 30, true).unwrap();
        assert!(stage.is_none(), "{why}: one stream, no error");
        assert_eq!(
            store.file(STAGE),
            None,
            "{why}: the stage that was made is gone"
        );
    }
    // No owner shown for the stage itself: the same.
    let mut anonymous = Store::new(true);
    anonymous.owner = None;
    assert!(open_with(&anonymous, STAGE, 30, true).unwrap().is_none());
    assert_eq!(anonymous.file(STAGE), None);
}

#[test]
fn speed_task_sftp_segmented_write_judges_each_folder_once() {
    let store = Store::new(true);
    for name in ["a", "b", "c"] {
        let stage = format!("/home/me/up/{name}.se-upload-01");
        assert!(open_with(&store, &stage, 30, true).unwrap().is_some());
    }
    assert_eq!(store.chain_probes.load(Ordering::Acquire), 1, "one look");
    // Another folder is another look; a refusal is remembered as well.
    let mut closed = Store::with_chain(None);
    closed.owner = Some(ME);
    for name in ["a", "b"] {
        let stage = format!("/other/{name}.se-upload-01");
        assert!(open_with(&closed, &stage, 30, true).unwrap().is_none());
    }
    assert_eq!(closed.chain_probes.load(Ordering::Acquire), 1);
    assert_eq!(
        closed.creates.load(Ordering::Acquire),
        1,
        "no stage for a known refusal"
    );
}

#[test]
fn speed_task_sftp_segmented_write_checks_every_range_handle() {
    let store = Store::new(true);
    let stage = open_with(&store, STAGE, 30, true).unwrap().unwrap();
    // The name now shows another account's file: nothing is written to it.
    *store.handle_owner.lock().unwrap() = Some(2000);
    let error = stage.segment(0, 10).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // The rule as a function: regular, ours, no longer than the stage.
    let attrs = |uid: Option<u32>, size: Option<u64>, mode: Option<u32>| FileAttributes {
        size,
        uid,
        user: None,
        gid: None,
        group: None,
        permissions: mode,
        atime: None,
        mtime: None,
    };
    let check = HandleCheck { uid: ME, size: 30 };
    assert!(check
        .verify(&attrs(Some(ME), Some(10), Some(0o100_600)))
        .is_ok());
    assert!(check
        .verify(&attrs(Some(2000), Some(10), Some(0o100_600)))
        .is_err());
    assert!(check
        .verify(&attrs(Some(ME), Some(31), Some(0o100_600)))
        .is_err());
    assert!(check
        .verify(&attrs(Some(ME), Some(10), Some(0o120_777)))
        .is_err());
    assert!(check
        .verify(&attrs(None, Some(10), Some(0o100_600)))
        .is_err());
    assert!(check
        .verify(&attrs(Some(ME), None, Some(0o100_600)))
        .is_err());
}
