//! Task suite „Bildkette“ BK3b (plan 2026-11-14 B-27): the daemon grants a
//! self-paced host process its next credit when credit frees (an answered
//! picture, an acknowledgement after a refused one), not per tick of the
//! session loop, and its frame clock no longer rounds the host process's
//! cadence. Not registered yet (`docs/tests/register.tsv`): the manager adds
//! the rows.
use super::Chain;

fn chain(self_paced: bool, ready: bool, freed: bool) -> Chain {
    Chain {
        self_paced,
        ready,
        freed,
    }
}

#[test]
fn bildkette_task_chain_credit_follows_the_answer_not_the_tick() {
    // The stage answered its credit: the next one goes at once.
    assert!(chain(true, true, false).starts());
    // Pacer or sender budget refused it: nothing until an acknowledgement
    // frees the queue (no spinning on the refusal) ...
    assert!(!chain(true, false, false).starts());
    assert!(!chain(true, false, false).freed_idle(false));
    // ... and then at once, unless an operation runs (its end decides).
    assert!(chain(true, false, true).starts());
    assert!(chain(true, false, true).freed_idle(false));
    assert!(!chain(true, false, true).freed_idle(true));
    // A stage that captures on request keeps the tick of the session loop.
    assert!(!chain(false, true, true).starts());
    assert!(!chain(false, true, true).freed_idle(false));
}

/// Source text of the daemon's pipeline and session loop.
fn source(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/screen/core")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn bildkette_task_chain_session_tick_no_longer_rounds_a_self_paced_rate() {
    let produce = source("host_pipeline_produce.rs");
    // The frame clock gates only a stage that captures on request.
    assert!(produce.contains("if !self.self_paced && !self.clock.take(now)"));
    // The cadence still goes to the host process with every credit (B-02).
    assert!(produce.contains("cadence_fps: (1_000_000 / self.clock.interval_us().max(1))"));
    // Pacer and sender budget still decide before every credit.
    assert!(produce.contains("self.pacer.decide(now, target, path, rtt_us)"));
    // Unthrottled (2026-11-25) the budget gets a rate that holds nothing back.
    assert!(produce.contains("self.sender.admit(admit_bps, path, rtt_us, now, keyframe)"));
    let session = source("host_session.rs");
    // The end of a picture operation, not the next tick, starts the next.
    assert!(session.contains("if missed_tick || self.pipeline.chained()"));
    assert!(session.contains("self.pipeline.credit_freed()"));
    assert!(session.contains("self.pipeline.begin_chained()"));
}
