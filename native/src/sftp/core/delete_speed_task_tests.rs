//! Delete-speed task tests for SFTP folder deletion without a server: the
//! pipelined REMOVE runner keeps one result per leaf in leaf order, stays
//! within its depth, starts nothing after a failure, and READDIR types keep
//! links out of the walk. The wire run is covered by the remote suite.
use super::metadata::to_vfs;
use super::tree_remove::REMOVE_DEPTH;
use crate::vfs::{remove_leaves_pipelined as remove_pipelined, DeleteTarget};
use russh_sftp::protocol::FileAttributes;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

fn leaves(count: usize) -> Vec<DeleteTarget> {
    (0..count)
        .map(|index| DeleteTarget {
            path: format!("/tree/leaf-{index}"),
            id: None,
            is_dir: false,
            is_symlink: false,
        })
        .collect()
}

fn index_of(leaf: &DeleteTarget) -> u64 {
    leaf.path.rsplit('-').next().unwrap().parse().unwrap()
}

#[test]
fn delete_speed_task_sftp_pipelined_removal_keeps_leaf_order_within_depth() {
    // One SSH window of full 256-KiB READs, the depth of a download.
    assert_eq!(REMOVE_DEPTH, 64);
    let leaves = leaves(10);
    let in_flight = AtomicUsize::new(0);
    let most = AtomicUsize::new(0);
    let answered = Mutex::new(Vec::new());
    let results = runtime().block_on(remove_pipelined(&leaves, 3, |leaf| {
        let (in_flight, most, answered) = (&in_flight, &most, &answered);
        async move {
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            let index = index_of(leaf);
            // Later leaves answer first inside each window.
            tokio::time::sleep(Duration::from_millis(30 - 3 * index)).await;
            in_flight.fetch_sub(1, Ordering::SeqCst);
            answered.lock().unwrap().push(index);
            if index == 9 {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "leaf 9"))
            } else {
                Ok(())
            }
        }
    }));
    assert_eq!(results.len(), 10);
    assert!(results[..9].iter().all(Result::is_ok));
    assert_eq!(results[9].as_ref().unwrap_err().to_string(), "leaf 9");
    assert_eq!(most.load(Ordering::SeqCst), 3);
    let answered = answered.into_inner().unwrap();
    assert_eq!(answered.len(), 10);
    assert_ne!(answered, (0..10).collect::<Vec<u64>>(), "answers overlap");
}

#[test]
fn delete_speed_task_sftp_pipelined_removal_starts_nothing_after_a_failure() {
    let leaves = leaves(8);
    let started = Mutex::new(Vec::new());
    let results = runtime().block_on(remove_pipelined(&leaves, 2, |leaf| {
        let started = &started;
        async move {
            let index = index_of(leaf);
            started.lock().unwrap().push(index);
            if index == 1 {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(())
        }
    }));
    let started = started.into_inner().unwrap();
    assert!(started.contains(&1));
    assert!(started.iter().all(|index| *index < 2), "{started:?}");
    assert_eq!(results.len(), 8);
    assert_eq!(results[0].is_ok(), started.contains(&0));
    for result in &results[1..] {
        let error = result.as_ref().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "denied");
    }
}

#[test]
fn delete_speed_task_sftp_listing_types_keep_links_out_of_the_walk() {
    let entry = |mode: Option<u32>| {
        let mut attributes = FileAttributes::empty();
        attributes.permissions = mode;
        to_vfs("entry".into(), &attributes)
    };
    let link = entry(Some(0o120_777));
    assert!(link.is_symlink && !link.is_dir, "a link is a leaf");
    let folder = entry(Some(0o040_755));
    assert!(folder.is_dir && !folder.is_symlink, "a folder is entered");
    let file = entry(Some(0o100_644));
    assert!(!file.is_dir && !file.is_symlink);
    // Without a mode an entry is a leaf: REMOVE refuses it if it is a folder.
    let unknown = entry(None);
    assert!(!unknown.is_dir && !unknown.is_symlink);
}
