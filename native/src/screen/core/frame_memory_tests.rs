use super::*;
const MIB:u64=1024*1024;
fn facts(bytes:u64)->MemoryFacts {
    MemoryFacts {available_physical:bytes,available_commit:bytes,available_address_space:bytes}
}
#[test]
fn batches_task_b6_memory_promises_share_capacity_and_survive_own_allocations() {
    let pool=Arc::new(MemoryPool::default());
    let first=pool.reserve(Some(facts(4096*MIB)),864*MIB).unwrap();
    let second=pool.reserve(Some(facts(4096*MIB)),864*MIB).unwrap();
    assert_eq!(first.bytes(),864*MIB);
    assert_eq!(second.bytes(),160*MIB);
    assert!(pool.reserve(Some(facts(4096*MIB)),MIB).is_err());
    assert_eq!(first.bytes(),864*MIB);
    drop(second);
    assert!(pool.reserve(Some(facts(2000*MIB)),864*MIB).is_err());
    assert_eq!(first.bytes(),864*MIB);
    let replacement=pool.reserve(Some(facts(4096*MIB)),864*MIB).unwrap();
    assert_eq!(replacement.bytes(),160*MIB);
    drop(first);drop(replacement);
    assert_eq!(pool.reserve(Some(facts(128*MIB)),864*MIB).unwrap().bytes(),32*MIB);
}
#[test]
fn batches_task_b6_unknown_commit_address_or_small_heap_cannot_enlarge() {
    let pool=Arc::new(MemoryPool::default());
    let unknown=pool.reserve(None,864*MIB).unwrap();
    assert_eq!(unknown.bytes(),512*MIB);drop(unknown);
    for limited in [MemoryFacts {available_commit:128*MIB,..facts(8192*MIB)},
        MemoryFacts {available_address_space:128*MIB,..facts(8192*MIB)},facts(128*MIB)] {
        let lease=pool.reserve(Some(limited),864*MIB).unwrap();assert_eq!(lease.bytes(),32*MIB);
    }
    let android=pool.reserve(None,24*MIB).unwrap();assert_eq!(android.bytes(),24*MIB);
}

#[test]
fn batches_task_b6_memory_lease_outlives_session_until_last_buffer_owner() {
    let pool=Arc::new(MemoryPool::default());
    let session=Arc::new(pool.reserve(Some(facts(3072*MIB)),768*MIB).unwrap());
    let capture_or_renderer=session.clone();
    drop(session);
    assert!(pool.reserve(Some(facts(3072*MIB)),MIB).is_err());
    drop(capture_or_renderer);
    assert_eq!(pool.reserve(Some(facts(3072*MIB)),768*MIB).unwrap().bytes(),768*MIB);
}

#[test]
fn batches_task_b6_small_android_offer_is_shared_instead_of_multiplied() {
    let pool=Arc::new(MemoryPool::default());
    let first=pool.reserve(None,32*MIB).unwrap();
    assert_eq!(first.bytes(),32*MIB);
    assert!(pool.reserve(None,32*MIB).is_err());
    drop(first);
    assert_eq!(pool.reserve(None,32*MIB).unwrap().bytes(),32*MIB);
}
