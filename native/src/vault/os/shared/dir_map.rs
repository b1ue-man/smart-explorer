//! The cleartext map of one vault storage (vault plan C6.1, U26-4): from
//! the root, breadth first, every content directory with its folder, every
//! entry with its cleartext name and shape, and the findings the walk meets
//! on the way (the CryptoFS health check, C6.2). The map lives only in memory
//! and only while the vault is unlocked; it is built with the key of the
//! session, one key use per name, so a lock ends the walk with `VaultLocked`.
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::cryptomator::{
    content_dir, decrypt_name, parse_dir_file, read_bounded, shortened_name, CIPHER_SUFFIX,
    CONTENTS_FILE, DATA_DIR, DIR_FILE, DIR_FILE_READ_LIMIT, DIR_ID_BACKUP_FILE, INFLATED_NAME_FILE,
    INFLATED_NAME_READ_LIMIT, ROOT_DIR_ID, SYMLINK_FILE,
};
use crate::vault::core::health_model::{FindingKind, HealthFinding};
use crate::vault::core::listing::{
    classify_entry, conflict_candidates, node_shape, EntryClass, NodeShape,
};
use crate::vault::core::path_map::{child, storage_path};
use crate::vault::core::VaultSession;
use crate::vfs::{self, Backend, VfsMeta};

/// One content directory.
#[derive(Clone, Debug)]
pub(super) struct DirInfo {
    pub(super) id: String,
    /// The folder's cleartext path: `/` for the root, else `/A/B`.
    pub(super) clear: String,
}

/// One entry of a content directory.
#[derive(Clone, Debug)]
pub(super) struct EntryInfo {
    /// The entry's full cleartext path (`/Ordner/Datei.txt`).
    pub(super) clear: String,
    pub(super) shape: NodeShape,
    pub(super) shortened: bool,
    /// The full encrypted name (`….c9r`) the entry decrypts with.
    pub(super) encrypted: String,
    /// The directory ID a folder node points to.
    pub(super) child: Option<String>,
    /// A conflict copy of a foreign program (`<canonical> (1).c9r` and the
    /// like): the stored name is not the canonical one.
    pub(super) foreign: bool,
}

/// The map of one side.
#[derive(Default)]
pub(super) struct DirMap {
    /// Content directory (`d/XX/YYYY…`) → ID and folder path.
    pub(super) dirs: HashMap<String, DirInfo>,
    /// Stored entry (`d/XX/YYYY…/<name>`) → what it is.
    pub(super) names: HashMap<String, EntryInfo>,
    /// Folder path → its content directory.
    pub(super) by_clear: HashMap<String, String>,
    /// What the walk found wrong.
    pub(super) findings: Vec<HealthFinding>,
}

struct Walk<'a> {
    inner: &'a dyn Backend,
    root: &'a str,
    session: &'a VaultSession,
}

struct Work {
    content: String,
    id: String,
    clear: String,
    /// The node file (`dir.c9r`) that points here.
    via: Option<String>,
}

/// What `name.c9s` of a shortened entry holds.
enum Inflated {
    Name(String),
    Trailing(String),
    Missing,
    Obese,
    NotDecodable,
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Vergleich abgebrochen")
}

fn nfc(text: &str) -> String {
    icu_normalizer::ComposingNormalizerBorrowed::new_nfc()
        .normalize(text)
        .to_string()
}

fn clear_path(folder: &str, name: &str) -> String {
    if folder == "/" {
        format!("/{name}")
    } else {
        format!("{folder}/{name}")
    }
}

impl Walk<'_> {
    fn path(&self, rel: &str) -> String {
        storage_path(self.root, rel)
    }

    fn list(&self, rel: &str) -> io::Result<Option<Vec<VfsMeta>>> {
        match self.inner.list_dir(&self.path(rel)) {
            Ok(entries) => Ok(Some(entries)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// At most `limit` bytes of a file; `None` when it holds more.
    fn small(&self, rel: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
        read_bounded(self.inner.open_read(&self.path(rel))?, limit)
    }

    fn decrypt(&self, encrypted: &str, parent_id: &str) -> io::Result<Option<String>> {
        let attempt = self
            .session
            .with_key(|key| Ok(decrypt_name(key, encrypted, parent_id)))?;
        Ok(attempt.ok())
    }

    /// The cleartext of a stored name; for a `….c9r` entry also through the
    /// encrypted name hidden in a foreign conflict copy. Returns the clear
    /// name, the name that decrypts and whether it came from a conflict copy.
    fn name_of(
        &self,
        encrypted: &str,
        parent_id: &str,
        conflicts: bool,
    ) -> io::Result<Option<(String, String, bool)>> {
        if let Some(clear) = self.decrypt(encrypted, parent_id)? {
            return Ok(Some((clear, encrypted.to_string(), false)));
        }
        if !conflicts || encrypted.starts_with('.') {
            return Ok(None);
        }
        let Some(base) = encrypted.strip_suffix(CIPHER_SUFFIX) else {
            return Ok(None);
        };
        for candidate in conflict_candidates(base) {
            let inner = format!("{candidate}{CIPHER_SUFFIX}");
            if let Some(clear) = self.decrypt(&inner, parent_id)? {
                return Ok(Some((clear, inner, true)));
            }
        }
        Ok(None)
    }

    fn inflated(&self, node: &str, stored: &str) -> io::Result<Inflated> {
        let rel = child(node, INFLATED_NAME_FILE);
        let reader = match self.inner.open_read(&self.path(&rel)) {
            Ok(reader) => reader,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Inflated::Missing),
            Err(error) => return Err(error),
        };
        let Some(bytes) = read_bounded(reader, INFLATED_NAME_READ_LIMIT)? else {
            return Ok(Inflated::Obese);
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return Ok(Inflated::NotDecodable);
        };
        if text.len() > CIPHER_SUFFIX.len() && text.ends_with(CIPHER_SUFFIX) {
            return Ok(Inflated::Name(text));
        }
        // A valid name followed by stray bytes (CryptoFS TrailingBytes…).
        if let Some(end) = text.find(CIPHER_SUFFIX).map(|at| at + CIPHER_SUFFIX.len()) {
            if shortened_name(&text[..end]) == stored {
                return Ok(Inflated::Trailing(text[..end].to_string()));
            }
        }
        Ok(Inflated::NotDecodable)
    }
}

impl DirMap {
    /// Walks the storage `root` on `inner` with the key of `session`.
    pub(super) fn build(
        inner: &dyn Backend,
        root: &str,
        session: &VaultSession,
        cancel: &AtomicBool,
    ) -> io::Result<Self> {
        let walk = Walk {
            inner,
            root,
            session,
        };
        let mut map = Self::default();
        let root_content = session.with_key(|key| content_dir(key, ROOT_DIR_ID))?;
        let mut queue = VecDeque::from([Work {
            content: root_content,
            id: ROOT_DIR_ID.to_string(),
            clear: "/".to_string(),
            via: None,
        }]);
        let mut owners: HashMap<String, String> = HashMap::new();
        while let Some(work) = queue.pop_front() {
            if cancel.load(Ordering::Acquire) {
                return Err(interrupted());
            }
            map.scan_dir(&walk, work, &mut queue, &mut owners)?;
        }
        map.find_orphans(&walk, cancel)?;
        Ok(map)
    }

    fn find(&mut self, kind: FindingKind, rel: &str, detail: Option<String>) {
        self.findings.push(HealthFinding::new(kind, rel, detail));
    }

    /// The entry stored as `content`/`stored`.
    pub(super) fn entry(&self, content: &str, stored: &str) -> Option<&EntryInfo> {
        self.names.get(&child(content, stored))
    }

    fn scan_dir(
        &mut self,
        walk: &Walk<'_>,
        work: Work,
        queue: &mut VecDeque<Work>,
        owners: &mut HashMap<String, String>,
    ) -> io::Result<()> {
        let Work {
            content,
            id,
            clear,
            via,
        } = work;
        self.dirs.insert(
            content.clone(),
            DirInfo {
                id: id.clone(),
                clear: clear.clone(),
            },
        );
        self.by_clear.insert(clear.clone(), content.clone());
        let Some(entries) = walk.list(&content)? else {
            if let Some(node) = via {
                self.find(FindingKind::MissingContentDir, &node, Some(content));
            }
            return Ok(());
        };
        let mut backup = false;
        for meta in entries {
            let rel = child(&content, &meta.name);
            if meta.name == DIR_ID_BACKUP_FILE {
                backup = true;
            } else if vfs::is_staging_name(&meta.name) {
                self.find(FindingKind::StageLeftover, &rel, None);
            } else if meta.name == DIR_FILE && !meta.is_dir {
                self.find(FindingKind::LooseDirFile, &rel, None);
            } else if !meta.is_symlink && !meta.special {
                let shortened = match classify_entry(&meta.name) {
                    EntryClass::Ignored => continue,
                    EntryClass::Short => false,
                    EntryClass::Shortened => true,
                };
                let place = Place {
                    id: &id,
                    clear: &clear,
                    rel: &rel,
                };
                self.scan_entry(walk, &place, &meta, shortened, queue, owners)?;
            }
        }
        if !backup {
            let rel = child(&content, DIR_ID_BACKUP_FILE);
            self.find(FindingKind::MissingDirIdBackup, &rel, None);
        }
        Ok(())
    }

    fn scan_entry(
        &mut self,
        walk: &Walk<'_>,
        place: &Place<'_>,
        meta: &VfsMeta,
        shortened: bool,
        queue: &mut VecDeque<Work>,
        owners: &mut HashMap<String, String>,
    ) -> io::Result<()> {
        let inside: Vec<String> = if meta.is_dir {
            match walk.list(place.rel)? {
                Some(list) => list.into_iter().map(|entry| entry.name).collect(),
                None => return Ok(()),
            }
        } else {
            Vec::new()
        };
        let encrypted = if shortened {
            if !meta.is_dir {
                self.find(FindingKind::MissingLongName, place.rel, None);
                return Ok(());
            }
            match walk.inflated(place.rel, &meta.name)? {
                Inflated::Name(name) => {
                    if shortened_name(&name) != meta.name {
                        let right = shortened_name(&name);
                        self.find(FindingKind::LongShortNamesMismatch, place.rel, Some(right));
                    }
                    name
                }
                Inflated::Trailing(name) => {
                    self.find(FindingKind::TrailingBytesInNameFile, place.rel, None);
                    name
                }
                Inflated::Missing => {
                    self.find(FindingKind::MissingLongName, place.rel, None);
                    return Ok(());
                }
                Inflated::Obese => {
                    self.find(FindingKind::ObeseNameFile, place.rel, None);
                    return Ok(());
                }
                Inflated::NotDecodable => {
                    self.find(FindingKind::NotDecodableLongName, place.rel, None);
                    return Ok(());
                }
            }
        } else {
            meta.name.clone()
        };
        let Some((clear, decrypting, foreign)) = walk.name_of(&encrypted, place.id, !shortened)?
        else {
            self.find(FindingKind::UndecryptableName, place.rel, None);
            return Ok(());
        };
        if foreign {
            self.find(FindingKind::ForeignConflictCopy, place.rel, None);
        }
        let path = clear_path(place.clear, &nfc(&clear));
        let has = |name: &str| inside.iter().any(|known| known == name);
        let shape = if meta.is_dir {
            node_shape(
                shortened,
                has(DIR_FILE),
                has(SYMLINK_FILE),
                has(CONTENTS_FILE),
            )
        } else {
            NodeShape::File
        };
        let mut info = EntryInfo {
            clear: path.clone(),
            shape,
            shortened,
            encrypted: decrypting,
            child: None,
            foreign,
        };
        match shape {
            NodeShape::Untyped => self.find(FindingKind::UnknownType, place.rel, None),
            NodeShape::Ambiguous => self.find(FindingKind::AmbiguousType, place.rel, None),
            NodeShape::Dir => {
                info.child = self.scan_node_file(walk, place, &path, queue, owners)?;
            }
            NodeShape::File | NodeShape::Symlink => {}
        }
        self.names.insert(place.rel.to_string(), info);
        Ok(())
    }

    /// Reads `dir.c9r` of a folder node and queues the folder it names.
    fn scan_node_file(
        &mut self,
        walk: &Walk<'_>,
        place: &Place<'_>,
        path: &str,
        queue: &mut VecDeque<Work>,
        owners: &mut HashMap<String, String>,
    ) -> io::Result<Option<String>> {
        let node = child(place.rel, DIR_FILE);
        let bytes = match walk.small(&node, DIR_FILE_READ_LIMIT) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                self.find(FindingKind::ObeseDirFile, &node, None);
                return Ok(None);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let Ok(id) = parse_dir_file(&bytes) else {
            self.find(FindingKind::EmptyDirFile, &node, None);
            return Ok(None);
        };
        if let Some(first) = owners.get(&id) {
            self.find(FindingKind::DirIdCollision, &node, Some(first.clone()));
            return Ok(Some(id));
        }
        owners.insert(id.clone(), node.clone());
        let content = walk.session.with_key(|key| content_dir(key, &id))?;
        queue.push_back(Work {
            content,
            id: id.clone(),
            clear: path.to_string(),
            via: Some(node),
        });
        Ok(Some(id))
    }

    /// Content directories below `d/` no node points to, with a `dirid.c9r`.
    fn find_orphans(&mut self, walk: &Walk<'_>, cancel: &AtomicBool) -> io::Result<()> {
        let Some(buckets) = walk.list(DATA_DIR)? else {
            return Ok(());
        };
        for bucket in buckets.iter().filter(|meta| meta.is_dir) {
            if cancel.load(Ordering::Acquire) {
                return Err(interrupted());
            }
            let bucket_rel = child(DATA_DIR, &bucket.name);
            for meta in walk.list(&bucket_rel)?.unwrap_or_default() {
                let rel = child(&bucket_rel, &meta.name);
                if !meta.is_dir || self.dirs.contains_key(&rel) {
                    continue;
                }
                let backup = child(&rel, DIR_ID_BACKUP_FILE);
                if walk.inner.try_exists(&walk.path(&backup))? {
                    self.find(FindingKind::OrphanContentDir, &rel, None);
                }
            }
        }
        Ok(())
    }
}

/// Where the entry being scanned lies.
struct Place<'a> {
    id: &'a str,
    clear: &'a str,
    rel: &'a str,
}
