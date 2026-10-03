//! Match listed mirror files and choose copy, skip or protected omission.
use super::sync_pass::Pass;
use super::sync_scan::FileTask;
use crate::bisync::{KeyPolicy, OmissionKind};
use crate::bisync::sync_flows::PairSide;
use crate::bisync::sync_overload::under_permits;
use crate::vfs::VfsMeta;
use std::collections::HashMap;
use std::io;

pub(super) enum Decision { Copy(Option<VfsMeta>), Skip, Omit, Fail(String) }

pub(super) fn decide(meta: &VfsMeta, counterpart: io::Result<Option<VfsMeta>>) -> Decision {
    match counterpart {
        Err(error) => Decision::Fail(format!("inspect destination: {error}")),
        Ok(None) => Decision::Copy(None),
        Ok(Some(found)) if found.is_symlink || found.special => Decision::Omit,
        Ok(Some(found)) if found.is_dir => Decision::Fail("destination is a directory".into()),
        Ok(Some(found)) if found.size != meta.size || meta.mtime_ms > found.mtime_ms => Decision::Copy(Some(found)),
        Ok(Some(_)) => Decision::Skip,
    }
}

#[derive(Default)]
pub(super) struct Destination { pub(super) entries: HashMap<String, Option<VfsMeta>> }
impl Destination {
    pub(super) fn of(listing: Vec<VfsMeta>, keys: KeyPolicy) -> Self {
        let mut entries = HashMap::new();
        for meta in listing {
            let key = keys.key(&meta.name).into_owned();
            if entries.contains_key(&key) { entries.insert(key, None); }
            else { entries.insert(key, Some(meta)); }
        }
        Self { entries }
    }
}


pub(super) fn file(
    pass: &Pass, source_path: String, meta: VfsMeta, rel: String, target_rel: String,
    destination_path: String, mut counterpart: Option<VfsMeta>,
) -> bool {
    if counterpart.is_some() && pass.confirm_listing {
        if !pass.dry_run && !pass.dst.is_local() {
            return pass.queue_file(FileTask { source: source_path, destination: destination_path,
                rel, target_rel, meta, expected: counterpart, confirm: true });
        }
        let result = under_permits(pass.cancel, &pass.progress,
            || pass.listing_permit(PairSide::B), |_| match pass.dst.stat(&destination_path) {
                Ok(meta) => Ok(Some(meta)), Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            });
        counterpart = match result {
            Some(Ok(found)) => found,
            Some(Err(error)) => { pass.io_error(&destination_path, error); return true; }
            None => return false,
        };
    }
    let expected = match decide(&meta, Ok(counterpart)) {
        Decision::Copy(expected) => expected,
        Decision::Skip => { pass.skipped(); return true; }
        Decision::Omit => { pass.omit_kind(&rel, OmissionKind::Link); return true; }
        Decision::Fail(message) => { pass.error(&destination_path, message); return true; }
    };
    if pass.dry_run { pass.would_copy(&destination_path); return true; }
    pass.queue_file(FileTask { source: source_path, destination: destination_path,
        rel, target_rel, meta, expected, confirm: false })
}
