//! C08 executes old saved jobs across production loader, resolver and runner.
pub(crate) use super::sync_reliability_task_old_jobs_fixture::{
    resolve_fixture, EndpointFixtures, SavedJob,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub(crate) fn run_saved_job(job: &crate::syncjobs::SyncJob, cancel: &AtomicBool) {
    super::job::run_one(job, cancel);
}

pub(super) fn local_pair() -> (tempfile::TempDir, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("source");
    let b = temp.path().join("target");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    (
        temp,
        a.to_str().unwrap().replace('\\', "/"),
        b.to_str().unwrap().replace('\\', "/"),
    )
}

pub(super) fn reload(saved: &SavedJob) -> crate::syncjobs::SyncJob {
    super::run_loop::sync_reliability_task_reload_after_restart()
        .into_iter()
        .find(|job| job.id == saved.id)
        .expect("same saved job loaded after restart")
}

pub(super) fn assert_noop(state: &crate::syncjobs::JobState) {
    let result = state.last_result.as_ref().unwrap();
    assert_eq!(
        (
            result.a_to_b,
            result.b_to_a,
            result.deleted,
            result.conflicts,
            result.errors
        ),
        (0, 0, 0, 0, 0)
    );
}

fn crossremote_checkpoint(provider: &str, phase: &str) {
    use std::io::Write;
    // Direct writes survive libtest's per-test capture if a later call aborts.
    let mut stderr = std::io::stderr().lock();
    writeln!(stderr, "C08 crossremote provider={provider} phase={phase}").unwrap();
    stderr.flush().unwrap();
}

const DIRECT_STAGE_PROOF_FILE: &str = "ipc-owner.txt";
const DIRECT_STAGE_PROOF_BYTES: &[u8] = b"real Direct creator bytes";

fn confirm_direct_stage_owner(
    owner: &dyn crate::vfs::Backend,
    reopened: &dyn crate::vfs::Backend,
    root: &str,
) {
    use crate::connect::sync_reliability_task_provider_fixture as provider_fixture;
    use std::io::Write;
    let destination = crate::vfs::sync_child_path(owner, root, DIRECT_STAGE_PROOF_FILE).unwrap();
    assert!(!owner.try_exists(&destination).unwrap());
    let stage = crate::vfs::unique_staging_path(owner, &destination, "daemon").unwrap();
    let stage_name = stage.rsplit('/').next().unwrap();
    crossremote_checkpoint("direct", "opening-exclusive-daemon-stage");
    let mut writer = owner.open_write_new(&stage).unwrap();
    writer.write_all(DIRECT_STAGE_PROOF_BYTES).unwrap();
    writer.flush().unwrap();
    drop(writer);
    crossremote_checkpoint("direct", "exclusive-daemon-stage-acknowledged");
    let error = reopened.promote_staged(&stage, &destination).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Stage wurde nicht von diesem Backend angelegt"),
        "unexpected publication refusal: {error}"
    );
    assert_eq!(
        provider_fixture::read(owner, root, stage_name),
        DIRECT_STAGE_PROOF_BYTES
    );
    assert!(!owner.try_exists(&destination).unwrap());
    crossremote_checkpoint("direct", "reopened-backend-cannot-claim-creator-stage");
    owner.promote_staged(&stage, &destination).unwrap();
    assert!(!owner.try_exists(&stage).unwrap());
    assert_eq!(
        provider_fixture::read(reopened, root, DIRECT_STAGE_PROOF_FILE),
        DIRECT_STAGE_PROOF_BYTES
    );
    crossremote_checkpoint(
        "direct",
        "creator-stage-published-and-reopened-bytes-confirmed",
    );
}

#[test]
fn sync_reliability_task_old_jobs_resolver_overrides_are_exact_and_thread_scoped() {
    let (_temp, a, _) = local_pair();
    let backend: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&a));
    let endpoint = "gdrive:///fixture-exact";
    {
        let guard = EndpointFixtures::new(vec![(endpoint.into(), backend.clone(), a.clone())]);
        assert_eq!(crate::connect::resolve_endpoint(endpoint).unwrap().1, a);
        assert!(resolve_fixture("gdrive:///fixture-exact/").is_none());
        assert!(
            std::thread::spawn(move || resolve_fixture(endpoint).is_none())
                .join()
                .unwrap()
        );
        let nested = EndpointFixtures::new(vec![(endpoint.into(), backend, "nested".into())]);
        assert_eq!(resolve_fixture(endpoint).unwrap().unwrap().1, "nested");
        drop(nested);
        assert_eq!(resolve_fixture(endpoint).unwrap().unwrap().1, a);
        guard.insert_failure(endpoint, "Authentication failed");
        assert_eq!(
            crate::connect::resolve_endpoint(endpoint).err().unwrap(),
            "Authentication failed"
        );
    }
    assert!(resolve_fixture(endpoint).is_none());
}

#[test]
fn sync_reliability_task_old_jobs_drive_restart_keeps_options_owner_and_converges() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().to_str().unwrap().replace('\\', "/");
    let source = "gdrive:///Notebook";
    let drive = crate::gdrive::sync_reliability_task_fixture::DriveFixture::new("/Notebook");
    let file_id = drive.write_file("old.txt", b"published old Drive bytes");
    // Drive metadata has no hidden flag: a dot-prefixed title is literal.
    drive.write_file(".hidden", b"literal Drive dot-name bytes");
    drive.write_file("ignored.skip", b"ignored must stay omitted");
    let (backend, root) = drive.endpoint();
    let endpoints = EndpointFixtures::new(vec![(source.into(), backend.clone(), root.clone())]);
    let saved = SavedJob::old(source, &target, "interval");
    let first = saved.run();
    assert_eq!(
        std::fs::read(temp.path().join("old.txt")).unwrap(),
        b"published old Drive bytes"
    );
    assert_eq!(
        std::fs::read(temp.path().join(".hidden")).unwrap(),
        b"literal Drive dot-name bytes"
    );
    assert!(!temp.path().join("ignored.skip").exists());
    let local = crate::vfs::LocalBackend::new(&target);
    let key = saved.key(&*backend, &root, &local, &target);
    let baseline = crate::bisync::baseline_file(&key).unwrap();
    assert!(crate::bisync::load_baseline(&baseline)
        .unwrap()
        .contains_key("old.txt"));
    saved.assert_options(source, &target);

    let restarted = drive.restart();
    assert_eq!(restarted.state_identity(), backend.state_identity());
    endpoints.insert(source, restarted.clone(), &root);
    assert_eq!(
        drive.write_file("old.txt", b"Drive changed after restart"),
        file_id
    );
    std::fs::write(
        temp.path().join("from-target.txt"),
        b"bidirectional local bytes",
    )
    .unwrap();
    let changed = saved.run();
    assert!(changed.last_success >= first.last_success);
    assert_eq!(
        std::fs::read(temp.path().join("old.txt")).unwrap(),
        b"Drive changed after restart"
    );
    assert_eq!(
        drive.read_file("from-target.txt"),
        b"bidirectional local bytes"
    );
    assert_eq!(drive.read_file(".hidden"), b"literal Drive dot-name bytes");
    assert_eq!(
        std::fs::read(temp.path().join(".hidden")).unwrap(),
        b"literal Drive dot-name bytes"
    );
    assert_eq!(saved.key(&*restarted, &root, &local, &target), key);
    let before = std::fs::read(&baseline).unwrap();
    assert_noop(&saved.run());
    assert_eq!(std::fs::read(baseline).unwrap(), before);
    assert_eq!(drive.read_file(".hidden"), b"literal Drive dot-name bytes");
    saved.assert_options(source, &target);
}

#[test]
fn sync_reliability_task_old_jobs_crossremote_runs_normal_saved_connection_and_drive() {
    use crate::connect::sync_reliability_task_provider_fixture as provider_fixture;
    let required = if cfg!(windows) { "direct" } else { "sftp" };
    let provider = provider_fixture::providers()
        .into_iter()
        .find(|provider| provider.name == required)
        .unwrap_or_else(|| {
            panic!("C08 requires actual {required} provider in discovered manifest")
        });
    let source = "gdrive:///Crossremote";
    let drive = crate::gdrive::sync_reliability_task_fixture::DriveFixture::new("/Crossremote");
    let (a, root_a) = drive.endpoint();
    let endpoints = EndpointFixtures::new(vec![(source.into(), a.clone(), root_a.clone())]);
    let mut nonce = [0u8; 8];
    getrandom::getrandom(&mut nonce).unwrap();
    let child = format!("c08-{:016x}", u64::from_be_bytes(nonce));
    crossremote_checkpoint(required, "opening-normal-saved-child");
    let (b, root_b) = provider.open(&child);
    crossremote_checkpoint(required, "normal-saved-child-opened");
    let target = format!(
        "{}/{child}",
        provider.endpoint.trim_end_matches(['/', '\\'])
    );
    assert!(
        resolve_fixture(&target).is_none(),
        "remote target must use production connection resolution"
    );
    let (reopened, reopened_root) = crate::connect::resolve_endpoint(&target).unwrap();
    assert_eq!(reopened_root, root_b);
    assert_eq!(reopened.state_identity(), b.state_identity());
    assert_ne!(a.namespace_identity(), b.namespace_identity());
    crossremote_checkpoint(required, "saved-target-reopened");
    if required == "direct" {
        confirm_direct_stage_owner(&*b, &*reopened, &root_b);
    }
    drive.write_file("drive.txt", b"old Drive into actual remote");
    crossremote_checkpoint(required, "writing-normal-provider-file");
    provider_fixture::write(&*b, &root_b, "remote.txt", b"actual remote into old Drive");
    crossremote_checkpoint(required, "normal-provider-write-published");
    let saved = SavedJob::old(source, &target, "realtime");
    crossremote_checkpoint(required, "running-original-saved-job");
    saved.run();
    assert_eq!(
        provider_fixture::read(&*b, &root_b, "drive.txt"),
        b"old Drive into actual remote"
    );
    assert_eq!(
        drive.read_file("remote.txt"),
        b"actual remote into old Drive"
    );
    let key = saved.key(&*a, &root_a, &*b, &root_b);
    let baseline = crate::bisync::baseline_file(&key).unwrap();
    let initial = crate::bisync::load_baseline(&baseline).unwrap();
    assert!(initial.contains_key("drive.txt") && initial.contains_key("remote.txt"));
    if required == "direct" {
        assert!(initial.contains_key(DIRECT_STAGE_PROOF_FILE));
        assert_eq!(
            drive.read_file(DIRECT_STAGE_PROOF_FILE),
            DIRECT_STAGE_PROOF_BYTES
        );
    }
    crossremote_checkpoint(required, "original-bytes-and-owned-baseline-confirmed");
    let restarted = drive.restart();
    endpoints.insert(source, restarted.clone(), &root_a);
    let restarted_job = reload(&saved);
    assert_eq!(restarted_job.id, saved.id);
    assert_eq!(
        (&*restarted_job.source, &*restarted_job.target),
        (source, &*target)
    );
    let (restarted_remote, restarted_root) = crate::connect::resolve_endpoint(&target).unwrap();
    assert_eq!(restarted_root, root_b);
    assert_eq!(restarted_remote.state_identity(), b.state_identity());
    crossremote_checkpoint(required, "job-reloaded-and-both-endpoints-recreated");
    drive.write_file("drive.txt", b"changed after worker recreation");
    provider_fixture::write(
        &*b,
        &root_b,
        "remote.txt",
        b"remote change after recreation",
    );
    crossremote_checkpoint(required, "running-recreated-saved-job");
    saved.run();
    assert_eq!(
        provider_fixture::read(&*b, &root_b, "drive.txt"),
        b"changed after worker recreation"
    );
    assert_eq!(
        drive.read_file("remote.txt"),
        b"remote change after recreation"
    );
    assert_eq!(
        saved.key(&*restarted, &root_a, &*restarted_remote, &root_b),
        key
    );
    assert_eq!(crate::bisync::baseline_file(&key).unwrap(), baseline);
    if required == "direct" {
        assert_eq!(
            provider_fixture::read(&*restarted_remote, &root_b, DIRECT_STAGE_PROOF_FILE),
            DIRECT_STAGE_PROOF_BYTES
        );
        assert_eq!(
            drive.read_file(DIRECT_STAGE_PROOF_FILE),
            DIRECT_STAGE_PROOF_BYTES
        );
    }
    crossremote_checkpoint(required, "recreated-bytes-and-state-key-confirmed");
    let before = std::fs::read(&baseline).unwrap();
    assert_noop(&saved.run());
    assert_eq!(std::fs::read(baseline).unwrap(), before);
    saved.assert_options(source, &target);
    crossremote_checkpoint(required, "saved-options-baseline-and-noop-confirmed");
}
