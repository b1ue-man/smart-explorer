//! Explicit GL-read events exercise the real ledger, not a graphics driver.
use super::*;

#[test]
fn batches_task_b6_glow_scope_keeps_internally_consumed_multiple_errors() {
    let mut scope=ScopeGuard::enter(1).unwrap();
    let mut errors=[glow::INVALID_ENUM,glow::OUT_OF_MEMORY,glow::NO_ERROR].into_iter();
    let mut reads=0;
    drain_with(1,|| {reads+=1;errors.next().unwrap()},|_|{});
    assert_eq!(reads,3);
    // The final glGetError observes NO_ERROR; the operation still has evidence.
    drain_with(1,||glow::NO_ERROR,|_|{});
    let error=scope.complete(Ok(())).unwrap_err();
    assert!(error.contains_gl_error(glow::INVALID_ENUM));
    assert!(error.contains_gl_error(glow::OUT_OF_MEMORY));
    assert!(error.state_unreliable());assert!(!active_key(1));
}
#[test]
fn batches_task_b6_glow_scope_release_check_is_active_and_nested_failure_reaches_caller() {
    let mut outer=ScopeGuard::enter(1).unwrap();
    assert!(active_key(1),"the release macro uses this opt-in predicate");
    let mut inner=ScopeGuard::enter(1).unwrap();
    record(1,GlOperationError::from_gl_code(glow::INVALID_VALUE));
    assert!(inner.complete(Ok(())).is_err());assert!(active_key(1));
    assert!(outer.complete(Ok(())).is_err());assert!(!active_key(1));
    let mut next=ScopeGuard::enter(1).unwrap();assert!(next.complete(Ok(())).is_ok());
}
#[test]
fn batches_task_b6_glow_scope_reports_creation_failure_and_does_not_mix_contexts() {
    let mut scope=ScopeGuard::enter(1).unwrap();
    assert!(scope.complete::<()>(Err(GlOperationError::texture_creation_failed())).is_err());
    let mut scope=ScopeGuard::enter(1).unwrap();
    assert!(ScopeGuard::enter(2).is_err());
    assert!(scope.complete(Ok(())).unwrap_err().state_unreliable());
    assert!(!active_key(1));assert!(!active_key(2));
}
#[test]
fn batches_task_b6_glow_scope_retains_lost_and_repeated_error_states_without_a_timeout() {
    for error in [glow::CONTEXT_LOST,glow::INVALID_OPERATION] {
        let mut scope=ScopeGuard::enter(1).unwrap();let mut reads=0;
        drain_with(1,|| {reads+=1;error},|_|{});
        assert!(scope.complete(Ok(())).unwrap_err().state_unreliable());
        assert_eq!(reads,if error==glow::CONTEXT_LOST {1} else {2});
    }
}
#[test]
fn batches_task_b6_glow_scope_drop_and_success_leave_no_stale_ledger() {
    let scope=ScopeGuard::enter(1).unwrap();
    record(1,GlOperationError::from_gl_code(glow::INVALID_OPERATION));drop(scope);
    let mut scope=ScopeGuard::enter(1).unwrap();
    drain_with(1,||glow::NO_ERROR,|_|{});
    assert_eq!(scope.complete(Ok(7)),Ok(7));
    assert!(!GlOperationError::from_gl_code(glow::INVALID_OPERATION).state_unreliable());
}
