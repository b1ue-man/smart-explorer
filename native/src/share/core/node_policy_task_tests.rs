use super::*;
use crate::share::relation_rights::{PrincipalKey, RelationScope, RestrictionReason, RightsRestriction};

fn restriction(relation: RelationScope, key: &str) -> RestrictionSet {
    let mut restrictions = RestrictionSet::default();
    restrictions.push(RightsRestriction { relation, principal: Some(PrincipalKey {
        public_key: key.into(), node_id: String::new(),
    }), reason: RestrictionReason::WriteRevoked });
    restrictions
}

#[test]
fn review_task_host_restrictions_hit_key_and_relation_only() {
    let policy = SessionPolicy::default();
    let direct = policy.snapshot(PeerPrincipal::new("direct", "lookup", "a", "key-a", "node-a")).unwrap();
    let alias = policy.snapshot(PeerPrincipal::new("direct", "other-lookup", "alias", "key-a", "node-a")).unwrap();
    let room = policy.snapshot(PeerPrincipal::new("room", "room-a", "a", "key-a", "node-a")).unwrap();
    let other = policy.snapshot(PeerPrincipal::new("direct", "lookup", "b", "key-b", "node-b")).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let preserved = Arc::new(AtomicBool::new(false));
    policy.register_cancel(&direct, &cancelled).unwrap();
    policy.register_cancel(&room, &preserved).unwrap();
    policy.invalidate(&restriction(RelationScope::Direct, "key-a"), 9).unwrap();
    assert!(direct.check().is_err());
    assert!(alias.check().is_err());
    assert!(cancelled.load(Ordering::Acquire));
    assert!(!preserved.load(Ordering::Acquire));
    room.check().unwrap();
    other.check().unwrap();
    let renewed = policy.snapshot(direct.principal().clone()).unwrap();
    renewed.check().unwrap();
    assert_eq!(renewed.revision(), 9);
    // Regranting cannot restore a retained result's old generation.
    assert!(direct.check().is_err());
}

#[test]
fn review_task_host_room_restrictions_preserve_other_room_and_extensions() {
    let policy = SessionPolicy::default();
    let a = policy.snapshot(PeerPrincipal::new("room", "a", "device", "key", "node")).unwrap();
    let b = policy.snapshot(PeerPrincipal::new("room", "b", "device", "key", "node")).unwrap();
    policy.invalidate(&RestrictionSet::default(), 7).unwrap();
    a.check().unwrap();
    b.check().unwrap();
    policy.invalidate(&restriction(RelationScope::Room { room_id: "a".into() }, "key"), 8).unwrap();
    assert!(a.check().is_err());
    b.check().unwrap();
    policy.invalidate(&RestrictionSet::everything(RestrictionReason::Unattributed), 10).unwrap();
    assert!(b.check().is_err());
}

#[test]
fn review_task_host_cancel_registration_after_restriction_fails_closed() {
    let policy = SessionPolicy::default();
    let generation = policy.snapshot(PeerPrincipal::new("direct", "", "", "key", "node")).unwrap();
    policy.invalidate(&restriction(RelationScope::Direct, "key"), 1).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    assert!(policy.register_cancel(&generation, &cancel).is_err());
    assert!(cancel.load(Ordering::Acquire));
}
