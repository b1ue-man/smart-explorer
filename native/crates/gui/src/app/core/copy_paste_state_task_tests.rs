use super::ClipboardPreparation;

fn task_runner_only() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(),
        Ok("1"),
        "run through the isolated copy/paste task entrypoint"
    );
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_pending_admission_requires_current_generation_and_sequence() {
    task_runner_only();
    let mut state = ClipboardPreparation::default();
    assert!(state.pending().is_none());
    let first = state.begin(17).unwrap();
    assert_eq!(state.pending(), Some(first));
    assert!(state.accepts(first, Some(17)));
    assert!(!state.accepts(first, Some(18)));
    assert!(!state.accepts(first, None));

    // Two preparations may start before either changes the OS clipboard.
    let newer = state.begin(17).unwrap();
    assert_ne!(first, newer);
    assert_eq!(state.pending(), Some(newer));
    assert!(!state.accepts(first, Some(17)));
    assert!(state.accepts(newer, Some(17)));
    assert!(!state.accepts(newer, Some(18)));
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_cancel_and_restart_never_readmit_an_old_result() {
    task_runner_only();
    let mut state = ClipboardPreparation::default();
    let first = state.begin(25).unwrap();
    state.clear();
    assert!(state.pending().is_none());
    assert!(!state.accepts(first, Some(25)));
    let restarted = state.begin(25).unwrap();
    assert_ne!(first, restarted);
    assert!(!state.accepts(first, Some(25)));
    assert!(state.accepts(restarted, Some(25)));
    state.clear();
    state.clear();
    assert!(!state.accepts(restarted, Some(25)));
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_external_sequence_supersedes_a_pending_result() {
    task_runner_only();
    let mut state = ClipboardPreparation::default();
    let old = state.begin(31).unwrap();
    assert!(!state.accepts(old, Some(32)));
    state.clear();
    let current = state.begin(32).unwrap();
    assert!(!state.accepts(old, Some(31)));
    assert!(!state.accepts(old, Some(32)));
    assert!(state.accepts(current, Some(32)));
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_generation_exhaustion_cannot_reuse_an_old_stamp() {
    task_runner_only();
    let mut state = ClipboardPreparation::default();
    state.generation = u64::MAX - 1;
    let last = state.begin(40).unwrap();
    state.clear();
    assert!(state.begin(40).is_err());
    assert!(state.pending().is_none());
    assert!(!state.accepts(last, Some(40)));
}
