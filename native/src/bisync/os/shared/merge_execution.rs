//! Prepare both sides, preserve originals and publish individual durable merge results.
use std::io;
use std::sync::atomic::AtomicBool;
use super::apply_guard::{capture, drift, revalidate, CapturedFile, ExpectedFile};
use super::incremental::SyncEndpoints;
use super::merge_precheck::{checked_original, Compared};
use super::merge_recorded::{MergeChoice, MergeReport, OriginalContent};
use super::merge_recovery::{self, Recovery};
use super::resolve_conflict::ResolvePhase;
use super::run_types::StateKey;
use super::types::{BisyncOptions, Conflict, PairSide};
use super::versions::{RunVersions, VersionReason, VersionSide};

pub(super) struct MergeWork<'a> {
    pub endpoints: SyncEndpoints<'a>, pub key: &'a StateKey, pub conflict: &'a Conflict,
    pub opts: BisyncOptions, pub original_a: OriginalContent<'a>, pub original_b: OriginalContent<'a>,
    pub choice: MergeChoice<'a>, pub winner: &'a [u8],
    pub state_a: &'a CapturedFile, pub state_b: &'a CapturedFile,
    pub path_a: &'a str, pub path_b: &'a str, pub rel_a: &'a str, pub rel_b: &'a str,
    pub recovering: bool, pub versions: &'a RunVersions,
}
pub(super) fn execute(work: MergeWork<'_>, recovery: &mut Recovery, report: &mut MergeReport,
    cancel: &AtomicBool, progress: &mut impl FnMut(ResolvePhase), lock: &super::PairLock,
) -> io::Result<()> {
    let MergeWork { endpoints, key, conflict, opts, original_a, original_b, choice, winner,
        state_a, state_b, path_a, path_b, rel_a, rel_b, recovering, versions } = work;
    let SyncEndpoints { a, root_a, b, root_b } = endpoints;
    let keys = super::KeyPolicy::for_pair(a.case_sensitive_paths(root_a), b.case_sensitive_paths(root_b));

    if let MergeChoice::KeepBoth { keep_a } = choice {
        let loser = if keep_a { original_b.bytes } else { original_a.bytes }
            .ok_or_else(|| drift("keep-both loser is absent"))?;
        super::merge_keep_both::preserve_both(endpoints, &conflict.rel, [rel_a, rel_b], loser,
            opts.cross_mounts, key, recovery, report, cancel)?;
    }
    let sides = [VersionSide { side: PairSide::A, backend: a, root: root_a },
        VersionSide { side: PairSide::B, backend: b, root: root_b }];
    let states = [state_a, state_b];
    let paths = [path_a, path_b];
    let rels = [rel_a, rel_b];
    // Stage every outstanding side before the first original moves.
    let mut stages = Vec::new();
    for index in 0..2 {
        let done = if index == 0 { recovery.done_a } else { recovery.done_b };
        if done { stages.push(None); continue; }
        super::apply_boundary::target(sides[index].backend, sides[index].root, rels[index], Some(winner.len() as u64))?;
        stages.push(Some(super::apply_stage::stage_bytes(sides[index].backend, paths[index],
            states[index], winner, super::versions::now_ms(), cancel)?));
    }
    // The staging and keep-both copies may take time. Compare both
    // exact original texts again under this lock before any backup.
    let (_, checked_a, done_a) = checked_original(a, path_a, original_a, recovery, PairSide::A, winner, recovering, cancel)?;
    let (_, checked_b, done_b) = checked_original(b, path_b, original_b, recovery, PairSide::B, winner, recovering, cancel)?;
    if done_a { recovery.a = checked_a; report.a = checked_a; }
    if done_b { recovery.b = checked_b; report.b = checked_b; }
    progress(ResolvePhase::BackingUp);
    let (sig_a, sig_b) = (report.a, report.b);
    let mut backups: Vec<Option<super::version_save::Preserved>> = Vec::new();
    for index in 0..2 {
        if stages[index].is_none() || states[index].metadata.is_none() { backups.push(None); continue; }
        let expected = if index == 0 { sig_a } else { sig_b };
        let backup = (|| {
            super::transfer_stream::check(cancel)?;
            super::apply_boundary::guard(sides[index].backend, sides[index].root, rels[index], opts.cross_mounts)?;
            super::version_save::save(versions, &sides[index], paths[index], rels[index], states[index],
                expected.map_or(ExpectedFile::Missing, ExpectedFile::Present), VersionReason::Resolved, cancel)
        })();
        match backup {
            Ok(backup) => backups.push(Some(backup)),
            Err(error) => {
                for (prior, backup) in backups.iter().enumerate() {
                    if let Some(backup) = backup { super::version_save::rollback(&sides[prior], paths[prior], backup)?; }
                }
                return Err(error);
            }
        }
    }
    for index in 0..2 {
        let Some(mut stage) = stages[index].take() else { continue; };
        progress(ResolvePhase::Copying);
        let result = (|| {
            super::transfer_stream::check(cancel)?;
            super::apply_boundary::guard(sides[index].backend, sides[index].root, rels[index], opts.cross_mounts)?;
            let current = if backups[index].as_ref().is_some_and(|backup| backup.moved) {
                CapturedFile { metadata: None }
            } else { (*states[index]).clone() };
            if current.metadata.is_some() && !sides[index].backend.has_duplicate_file_names()
                && !sides[index].backend.mount_path_capabilities(paths[index])?.staged_write.namespace_replace {
                stage.bind(versions, &sides[index], rels[index], false)?;
                if let Some(backup) = &backups[index] { stage.require_backup(backup.signature)?; }
            }
            let outcome = stage.publish(paths[index], &current, true, cancel)?;
            super::apply_stage::require_durable(outcome.durable)?;
            if index == 0 { recovery.a = Some(outcome.destination); recovery.done_a = true;
                report.a = recovery.a; report.confirmed_a = true; }
            else { recovery.b = Some(outcome.destination); recovery.done_b = true;
                report.b = recovery.b; report.confirmed_b = true; }
            merge_recovery::save(key, recovery)
        })();
        if let Err(error) = result {
            for pending in index..2 {
                if let Some(backup) = backups[pending].as_ref() {
                    super::version_save::rollback(&sides[pending], paths[pending], backup)?;
                }
            }
            return Err(error);
        }
    }
    progress(ResolvePhase::ReadingSignatures);
    let finish = AtomicBool::new(false);
    for (backend, path, signature) in [(a, path_a, report.a), (b, path_b, report.b)] {
        let current = capture(backend, path, signature.map_or(ExpectedFile::Missing, ExpectedFile::Present), "merge result")?;
        let meta = current.regular("merge result")?;
        let mut reader = crate::vfs::open_read_regular(backend, path, meta.id.as_deref())?;
        let mut compared = Compared::new(winner, winner);
        super::transfer_stream::stream(&mut *reader, &mut compared, &finish, None, 0, |_| {})?;
        if !compared.first_matches() { return Err(drift("merge result changed before recording")); }
        revalidate(backend, path, &current, "merge result")?;
    }
    if !recovery.done_a || !recovery.done_b { return Err(drift("merge lacks confirmed writes")); }
    let mut records = vec![(conflict.rel.clone(), (report.a, report.b))];
    records.extend(report.preserved.iter().filter(|file| file.a.is_some() && file.b.is_some())
        .map(|file| (file.rel.clone(), (file.a, file.b))));
    super::replica_state::merge_with_keys(lock, key, &records, keys)?;
    let mut names = super::state_spellings::load(key, keys)?;
    names.files_a.insert(keys.key(&conflict.rel).into_owned(), rel_a.to_string());
    names.files_b.insert(keys.key(&conflict.rel).into_owned(), rel_b.to_string());
    if let Some(sibling) = &recovery.sibling {
        names.files_a.insert(keys.key(sibling).into_owned(), super::merge_keep_both::sibling_spelling(sibling, rel_a));
        names.files_b.insert(keys.key(sibling).into_owned(), super::merge_keep_both::sibling_spelling(sibling, rel_b));
    }
    super::state_spellings::save(key, &names)?;
    let (_, records, _) = super::checkpoint_journal::Journal::load(key, keys)?;
    report.baseline = records.baseline;
    merge_recovery::remove(key, &conflict.rel)?;
    // Both durable results are already recorded. A failed cleanup
    // cannot recreate a pending conflict or revoke that baseline.
    let _ = super::merge_inputs::remove(key, &conflict.rel);
    Ok(())
}
