//! What a drive's engine asks of the device by itself is background work and
//! sets no metadata presence; what a program waits for does (plan
//! 2026-11-14, E4c/E4d). The fixture counts the calls it receives inside and
//! outside a `crate::vfs::BackgroundScope`; each request carries that mark
//! over the IPC to the daemon (`daemon/os/shared/request_origin_task_tests.rs`),
//! which answers it as `Job` (`metadata_presence_tests.rs`). Remote suite
//! only (AGENTS.md).
use super::{
    engine::MountEngine,
    optimization_fixture::OptimizationBackend,
    types::{MountId, MountMode, MountRuntimeConfig},
    waiting_presence::RENEW_AFTER,
};
use crate::vfs::BackendHandle;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn engine_over(
    name: &str,
) -> io::Result<(Arc<OptimizationBackend>, MountEngine, tempfile::TempDir)> {
    let spool = tempfile::tempdir()?;
    let backend = OptimizationBackend::new();
    backend.put("/a/file.txt", b"x");
    let handle: BackendHandle = backend.clone();
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse(name)?, MountMode::ReadOnly),
        handle,
        spool.path(),
    )?;
    Ok((backend, engine, spool))
}

#[test]
fn verbindungen_task_metadata_presence_a_drive_refresh_and_start_run_as_background(
) -> io::Result<()> {
    let (backend, engine, _spool) = engine_over("presence-refresh")?;
    // A program lists the root: the engine asks the device for it.
    assert_eq!(engine.cached_remote_directory("/", 0)?.len(), 1);
    assert!(
        backend.foreground_calls() > 0,
        "a listing a program waits for"
    );
    assert_eq!(backend.background_calls(), 0);
    let foreground = backend.foreground_calls();
    // The refresh the host runs every few seconds.
    engine.refresh_metadata()?;
    assert!(
        backend.background_calls() > 0,
        "the refresh asked the device"
    );
    assert_eq!(
        backend.foreground_calls(),
        foreground,
        "a refresh is background work, no presence"
    );
    // The checks of the mount start.
    let background = backend.background_calls();
    engine.prepare_host_remote()?;
    assert!(backend.background_calls() > background, "the root check");
    assert_eq!(backend.foreground_calls(), foreground);
    Ok(())
}

/// Polls `done` for at most ten seconds.
fn wait_for(done: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done(), "{what}");
}

#[test]
fn verbindungen_task_metadata_presence_a_request_that_waits_for_a_refresh_sets_presence(
) -> io::Result<()> {
    let (backend, engine, _spool) = engine_over("presence-waiting")?;
    backend.as_share_device();
    let slot = engine.metadata_cache.load_slot("/")?;
    // A refresh of the root is in flight.
    let refresh = slot.lock_background()?;
    assert!(slot.held_by_background());
    let before = backend.foreground_calls();
    std::thread::scope(|scope| -> io::Result<()> {
        let waiting = scope.spawn(|| engine.cached_remote_directory("/", 0));
        // At once: a small request of the drive's own origin sets the
        // presence beside the wait.
        wait_for(
            || backend.foreground_calls() > before,
            "the wait for a background refresh sets presence",
        );
        // Before its hold time ends, the next one: the presence lasts as
        // long as the wait.
        wait_for(
            || backend.foreground_calls() > before + 1,
            "the presence is renewed while the wait lasts",
        );
        drop(refresh);
        let listed = waiting
            .join()
            .map_err(|_| io::Error::other("the waiting request panicked"))??;
        assert_eq!(listed.len(), 1);
        Ok(())
    })?;
    // The wait is over: the helper asks no more.
    std::thread::sleep(Duration::from_millis(300));
    let ended = backend.foreground_calls();
    std::thread::sleep(RENEW_AFTER + Duration::from_secs(1));
    assert_eq!(
        backend.foreground_calls(),
        ended,
        "the presence ends with the wait"
    );
    Ok(())
}

#[test]
fn verbindungen_task_metadata_presence_waiting_for_a_program_adds_no_request() -> io::Result<()> {
    let (backend, engine, _spool) = engine_over("presence-plain")?;
    backend.as_share_device();
    let slot = engine.metadata_cache.load_slot("/")?;
    // A listing in flight that a program waits for (no background mark):
    // its holder already sets the presence, so a second one waits quietly.
    let gate = slot.lock()?;
    assert!(!slot.held_by_background());
    let before = backend.foreground_calls();
    std::thread::scope(|scope| -> io::Result<()> {
        let waiting = scope.spawn(|| engine.cached_remote_directory("/", 0));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(backend.foreground_calls(), before, "no extra request");
        drop(gate);
        waiting
            .join()
            .map_err(|_| io::Error::other("the waiting request panicked"))??;
        Ok(())
    })
}

/// Review I (E4c): a source without a Share device knows no presence, so a
/// wait for its refresh asks nothing; and background work that waits for a
/// background fetch is no person's wait either.
#[test]
fn verbindungen_task_metadata_presence_no_extra_request_without_a_device_or_a_person(
) -> io::Result<()> {
    for (case, name, share_device, background) in [
        (
            "a source without a Share device",
            "presence-sftp",
            false,
            false,
        ),
        (
            "background work that waits",
            "presence-own-work",
            true,
            true,
        ),
    ] {
        let (backend, engine, _spool) = engine_over(name)?;
        if share_device {
            backend.as_share_device();
        }
        let slot = engine.metadata_cache.load_slot("/")?;
        let refresh = slot.lock_background()?;
        let before = (backend.foreground_calls(), backend.background_calls());
        std::thread::scope(|scope| -> io::Result<()> {
            let waiting = scope.spawn(|| {
                let _work = background.then(crate::vfs::BackgroundScope::enter);
                engine.cached_remote_directory("/", 0)
            });
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(
                (backend.foreground_calls(), backend.background_calls()),
                before,
                "{case}: no extra request while it waits"
            );
            drop(refresh);
            waiting
                .join()
                .map_err(|_| io::Error::other("the waiting request panicked"))??;
            Ok(())
        })?;
    }
    Ok(())
}
