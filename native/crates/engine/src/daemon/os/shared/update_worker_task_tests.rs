//! Tests of the verdict behind `worker_answers` (M30, E55): a worker that
//! answers after the proof counts as running, whatever its version; a process
//! without the proof, an unreachable address and a missing worker do not.

use super::{answers, ProbeOutcome};

#[test]
fn update_worker_task_answering_worker_counts_as_running() {
    let generation = "0".repeat(32);
    let answering = [
        ProbeOutcome::Ready {
            generation: generation.clone(),
        },
        ProbeOutcome::Starting { generation },
        ProbeOutcome::Retiring,
        ProbeOutcome::OtherPong,
    ];
    for outcome in &answering {
        assert!(answers(outcome), "{outcome:?} must count as running");
    }
}

#[test]
fn update_worker_task_without_proof_or_pong_does_not_count_as_running() {
    let silent = [
        ProbeOutcome::Unproven,
        ProbeOutcome::Unreachable,
        ProbeOutcome::Missing,
    ];
    for outcome in &silent {
        assert!(!answers(outcome), "{outcome:?} must not count as running");
    }
}
