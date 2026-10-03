//! Bounded listing and retention by original file, side and replica.
use super::pair_lock::PairLock;
use super::paths::{join, parent_of, validate_child_name};
use super::transfer_stream::check;
use super::types::Versioning;
use super::version_listing::{children, legacy, managed, Managed};
use super::version_manifest::{self as record, Manifest};
use super::version_retention::{keep_version, selected};
use super::versions::{VersionEntry, VersionSide, VersionStore};
use crate::vfs::{Backend, LocalBackend};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) fn list(
    pair: &str,
    sides: &[VersionSide<'_>],
    cancel: &AtomicBool,
) -> io::Result<Vec<VersionEntry>> {
    let mut entries = Vec::new();
    let mut paths = BTreeSet::new();
    for member in super::backend_identity_state::family(pair)? {
        for entry in super::version_listing::list(&member, sides, cancel)? {
            if paths.insert((entry.side, entry.stored_path.clone())) {
                entries.push(entry);
            }
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.preserved_ms));
    Ok(entries)
}

fn erase(
    backend: &dyn Backend,
    item: &Managed,
    lock: &PairLock,
    cancel: &AtomicBool,
) -> io::Result<()> {
    check(cancel)?;
    if item.manifest.lock.is_empty()
        || !super::backend_identity_state::lock_matches(
            &item.manifest.pair,
            &item.manifest.lock,
            lock.id(),
        )?
    {
        return Err(record::invalid("version belongs to another pair lock"));
    }
    // Observe both immutable records before deleting any bytes. A record
    // replacement must not authorize removal of foreign data.
    let mut records = BTreeMap::new();
    for name in ["entry.json", "intent.json"] {
        let path = join(&item.dir, name);
        let observed = super::apply_guard::capture(
            backend,
            &path,
            super::apply_guard::ExpectedFile::Unknown,
            "version record removal",
        )?;
        match record::read(backend, &path, cancel) {
            Ok(current)
                if current.pair == item.manifest.pair
                    && current.lock == item.manifest.lock
                    && current.rel == item.manifest.rel
                    && current.data == item.manifest.data
                    && current.run == item.manifest.run => {}
            Ok(_) => return Err(record::invalid("version record changed before removal")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        super::apply_guard::revalidate(backend, &path, &observed, "version record removal")?;
        records.insert(name, observed);
    }
    let expected = super::apply_guard::ExpectedFile::Present(item.manifest.data_sig);
    let data =
        super::apply_guard::capture(backend, &item.manifest.data, expected, "version removal")?;
    let meta = data.regular("version removal")?;
    if item.manifest.data_id.is_some() && meta.id != item.manifest.data_id {
        return Err(record::invalid("version data identity changed"));
    }
    if meta
        .content_md5
        .as_ref()
        .zip(item.manifest.digest.as_ref())
        .is_some_and(|(actual, expected)| !actual.eq_ignore_ascii_case(expected))
    {
        return Err(record::invalid("version data content changed"));
    }
    if backend.is_local()
        || (meta.content_md5.is_none()
            && (item.manifest.digest.is_some() || item.manifest.data_sig.hash != 0))
    {
        let mut reader =
            crate::vfs::open_read_regular(backend, &item.manifest.data, meta.id.as_deref())?;
        let checked = super::transfer_stream::stream(
            &mut *reader,
            &mut io::sink(),
            cancel,
            None,
            item.manifest.data_sig.hash,
            |_| {},
        )?;
        drop(reader);
        if checked.bytes != item.manifest.size
            || item
                .manifest
                .digest
                .as_ref()
                .is_some_and(|digest| !digest.eq_ignore_ascii_case(&checked.hex()))
        {
            return Err(record::invalid("version data content changed"));
        }
    }
    super::apply_guard::revalidate(backend, &item.manifest.data, &data, "version removal")?;
    // Remove exactly the known regular data and records, never recursive
    // unknown content or a link. Unknown children leave the directory intact.
    for name in ["data", "entry.json", "intent.json"] {
        let path = join(&item.dir, name);
        let observed = if name == "data" {
            &data
        } else {
            records
                .get(name)
                .ok_or_else(|| record::invalid("unobserved version record"))?
        };
        let Some(meta) = observed.metadata.as_ref() else {
            continue;
        };
        observed.regular("version removal")?;
        super::apply_guard::revalidate(backend, &path, observed, "version removal")?;
        check(cancel)?;
        backend.remove_file_id(&path, meta.id.as_deref())?;
    }
    if children(backend, &item.dir, cancel)?.is_empty() {
        backend.remove_dir(&item.dir)?;
    }
    super::apply_stage::require_durable(super::apply_stage::namespace(backend, &item.dir)?)
}

pub(super) fn prune(
    lock: &PairLock,
    pair: &str,
    sides: &[VersionSide<'_>],
    versioning: &Versioning,
    cancel: &AtomicBool,
) -> io::Result<()> {
    maintain(lock, pair, sides, Some(versioning), cancel)
}
pub(super) fn remove(
    lock: &PairLock,
    pair: &str,
    sides: &[VersionSide<'_>],
    cancel: &AtomicBool,
) -> io::Result<()> {
    maintain(lock, pair, sides, None, cancel)
}
fn maintain(
    lock: &PairLock,
    pair: &str,
    sides: &[VersionSide<'_>],
    retention: Option<&Versioning>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    for member in super::backend_identity_state::family(pair)? {
        maintain_member(lock, &member, sides, retention, cancel)?;
    }
    Ok(())
}
fn maintain_member(
    lock: &PairLock,
    pair: &str,
    sides: &[VersionSide<'_>],
    retention: Option<&Versioning>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    record::validate_pair(pair)?;
    for side in sides {
        let items: Vec<_> = managed(
            side.backend,
            &join(side.root, ".se-versions"),
            pair,
            VersionStore::SyncRoot,
            cancel,
        )?
        .into_iter()
        .filter(|item| item.manifest.belongs(pair, side))
        .collect();
        let delete = retention
            .map(|rule| selected(&items, rule))
            .unwrap_or_else(|| (0..items.len()).collect());
        for index in delete {
            erase(side.backend, &items[index], lock, cancel)?;
        }
    }
    let app = super::persistence::versions_dir(pair);
    let root = app
        .to_str()
        .ok_or_else(|| record::invalid("versions path is not Unicode"))?;
    let backend = LocalBackend::new(root);
    let items = managed(&backend, root, pair, VersionStore::AppData, cancel)?;
    let delete = retention
        .map(|rule| selected(&items, rule))
        .unwrap_or_else(|| (0..items.len()).collect());
    for index in delete {
        erase(&backend, &items[index], lock, cancel)?;
    }
    // Legacy files have no replica metadata: retention still groups per rel,
    // rather than pruning entire run folders by their timestamp.
    let legacy = legacy(&backend, root, cancel)?;
    let mut groups: BTreeMap<&str, Vec<&VersionEntry>> = BTreeMap::new();
    for entry in &legacy {
        groups.entry(&entry.rel).or_default().push(entry);
    }
    for mut entries in groups.into_values() {
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.preserved_ms));
        let mut buckets = BTreeSet::new();
        for (position, entry) in entries.into_iter().enumerate() {
            let keep = retention.is_some_and(|rule| {
                keep_version(
                    rule,
                    position,
                    entry.preserved_ms.max(0) as u64 / 1000,
                    super::versions::now_ms().max(0) as u64 / 1000,
                    &mut buckets,
                )
            });
            if !keep {
                check(cancel)?;
                let meta = backend.stat(&entry.stored_path)?;
                if meta.is_dir
                    || meta.is_symlink
                    || meta.special
                    || meta.size != entry.size
                    || meta.mtime_ms != entry.mtime_ms
                {
                    return Err(record::invalid("legacy version changed before removal"));
                }
                backend.remove_file_id(&entry.stored_path, meta.id.as_deref())?;
                super::apply_stage::require_durable(super::apply_stage::namespace(
                    &backend,
                    &entry.stored_path,
                )?)?;
            }
        }
    }
    Ok(())
}

pub(super) fn find(
    pair: &str,
    entry: &VersionEntry,
    side: &VersionSide<'_>,
    cancel: &AtomicBool,
) -> io::Result<Manifest> {
    record::validate_pair(pair)?;
    let dir =
        parent_of(&entry.stored_path).ok_or_else(|| record::invalid("version has no directory"))?;
    let manifest = if entry.store == VersionStore::SyncRoot {
        let archive = join(side.root, ".se-versions");
        let relative = dir
            .strip_prefix(&format!("{}/", archive.trim_end_matches('/')))
            .ok_or_else(|| record::invalid("version is outside its sync archive"))?;
        if relative.split('/').count() != 3 {
            return Err(record::invalid("version has an invalid archive depth"));
        }
        let mut parent = archive;
        for name in relative.split('/') {
            validate_child_name(name)?;
            children(side.backend, &parent, cancel)?;
            parent = join(&parent, name);
        }
        children(side.backend, &dir, cancel)?;
        record::read(side.backend, &join(&dir, "entry.json"), cancel).or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                record::read(side.backend, &join(&dir, "intent.json"), cancel)
            } else {
                Err(error)
            }
        })?
    } else {
        let root = app_root(pair, &entry.stored_path)?;
        let root = root
            .to_str()
            .ok_or_else(|| record::invalid("version path is not Unicode"))?;
        if !std::path::Path::new(&entry.stored_path).starts_with(root) {
            return Err(record::invalid("version is outside pair app data"));
        }
        let backend = LocalBackend::new(root);
        record::read(&backend, &join(&dir, "entry.json"), cancel)?
    };
    if !manifest.belongs(pair, side) || manifest.entry(entry.store)? != *entry {
        return Err(record::invalid("version identity has changed"));
    }
    Ok(manifest)
}

pub(super) fn app_root(pair: &str, stored_path: &str) -> io::Result<std::path::PathBuf> {
    for member in super::backend_identity_state::family(pair)? {
        let root = super::persistence::versions_dir(&member);
        if std::path::Path::new(stored_path).starts_with(&root) {
            return Ok(root);
        }
    }
    Err(record::invalid("version is outside verified pair app data"))
}
