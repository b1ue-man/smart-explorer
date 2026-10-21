//! Acceptance signals of the SMB packet paths (plan U46-4) against the
//! scripted wire of batch_fake.rs.
use super::batch::{limits, stage_name, BATCH_MAX_BYTES, BATCH_MAX_FILES};
use super::batch_fake::{entries, failure, get, items, published, put, Fake, RTT};
use super::batch_wire::{packet_create, PACKET_OPTIONS};
use super::wire::{FILE_NON_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT};
use crate::vfs::{unpublished_of, BatchPutOutcome};
use smb2::msg::create::CreateDisposition;

#[test]
fn speed_task_smb_batch_put_two_round_trips_for_small_files() {
    let fake = Fake::new(1 << 20, 1 << 20);
    let (list, mut data) = entries(64, 4096);
    let outcomes = put(&fake, &list, &mut data);
    for (outcome, entry) in outcomes.iter().zip(&list) {
        assert_eq!(published(outcome), entry.path);
    }
    // CREATE+WRITE+CLOSE, then the rename: two rounds for 64 files at once.
    assert!(fake.clock.now.get() <= 3 * RTT, "{}", fake.clock.now.get());
    let files = fake.files.borrow();
    assert_eq!(files.len(), 64, "no stage is left behind");
    assert_eq!(files["/share/dir/file7.bin"], vec![7u8; 4096]);
    assert_eq!(fake.logged("write"), 0, "one block needs no further WRITE");
}

#[test]
fn speed_task_smb_batch_put_blocks_above_max_write_follow_at_once() {
    let fake = Fake::new(1024, 1024);
    let (list, mut data) = entries(2, 5000);
    let outcomes = put(&fake, &list, &mut data);
    assert!(outcomes
        .iter()
        .all(|o| matches!(o, BatchPutOutcome::Published(_))));
    assert!(fake.log.borrow().contains(&"write 4096".to_string()));
    assert_eq!(fake.logged("write"), 8);
    // CREATE+first block, the other four blocks together, CLOSE, rename.
    assert_eq!(fake.clock.now.get(), 4 * RTT);
    assert_eq!(fake.files.borrow()["/share/dir/file1.bin"], vec![1u8; 5000]);
}

#[test]
fn speed_task_smb_batch_put_name_collision_numbers() {
    let fake = Fake::new(1 << 20, 1 << 20);
    for taken in ["/share/dir/file0.bin", "/share/dir/file0 (2).bin"] {
        fake.files
            .borrow_mut()
            .insert(taken.to_string(), b"mine".to_vec());
    }
    let (list, mut data) = entries(2, 8);
    let outcomes = put(&fake, &list, &mut data);
    assert_eq!(published(&outcomes[0]), "/share/dir/file0 (3).bin");
    assert_eq!(published(&outcomes[1]), "/share/dir/file1.bin");
    let files = fake.files.borrow();
    assert_eq!(files["/share/dir/file0.bin"], b"mine", "never replaced");
    assert_eq!(files["/share/dir/file0 (2).bin"], b"mine");
}

#[test]
fn speed_task_smb_batch_put_error_before_rename_is_unpublished() {
    let fake = Fake::new(1024, 1024);
    fake.faults.borrow_mut().fail_write_more = true;
    let (list, mut data) = entries(2, 3000);
    for outcome in put(&fake, &list, &mut data) {
        assert!(unpublished_of(&failure(outcome)));
    }
    assert!(fake.files.borrow().is_empty(), "the stages are removed");
    // A refused CREATE has nothing of ours to remove, a foreign file stays.
    let fake = Fake::new(1 << 20, 1 << 20);
    fake.faults.borrow_mut().refuse_create = true;
    fake.files
        .borrow_mut()
        .insert("/share/dir/x".into(), b"x".to_vec());
    let (list, mut data) = entries(2, 8);
    for outcome in put(&fake, &list, &mut data) {
        assert!(unpublished_of(&failure(outcome)));
    }
    assert_eq!(fake.logged("remove"), 0);
    assert_eq!(fake.files.borrow().len(), 1);
}

#[test]
fn speed_task_smb_batch_put_lost_reply_is_not_unpublished() {
    let fake = Fake::new(1 << 20, 1 << 20);
    fake.faults.borrow_mut().lose_rename = true;
    let (list, mut data) = entries(2, 8);
    for outcome in put(&fake, &list, &mut data) {
        let error = failure(outcome);
        assert!(!unpublished_of(&error));
        assert!(error.to_string().contains(".se-upload-"), "{error}");
    }
    // The result is unknown: one rename each, no repeat, the stage stays.
    assert_eq!(fake.logged("rename"), 2);
    assert_eq!(fake.logged("remove"), 0);
    assert_eq!(fake.files.borrow().len(), 2);
}

#[test]
fn speed_task_smb_batch_put_never_follows_reparse_points() {
    for new_stage in [true, false] {
        let request = packet_create("dir\\name".to_string(), new_stage);
        assert_ne!(request.create_options & FILE_OPEN_REPARSE_POINT, 0);
        assert_ne!(request.create_options & FILE_NON_DIRECTORY_FILE, 0);
        assert_eq!(request.create_options, PACKET_OPTIONS);
    }
    // A stage is always new: the server refuses a taken name atomically.
    let stage = packet_create("dir\\name".to_string(), true);
    assert!(matches!(
        stage.create_disposition,
        CreateDisposition::FileCreate
    ));
    assert_eq!(stage.share_access.0, 0);
    // Stage names sit next to the destination and never equal it.
    let name = stage_name("/share/dir/a.txt");
    assert!(name.starts_with("/share/dir/a.txt.se-upload-") && name != "/share/dir/a.txt");
}

#[test]
fn speed_task_smb_batch_get_detects_size_change_from_create_response() {
    let fake = Fake::new(1024, 1024);
    let mut list = items(&fake, 2, 100);
    list[0].size = 50;
    let events = get(&fake, &list);
    assert_eq!(&events[..2], ["begin 0 50", "end 0 InvalidData"]);
    assert_eq!(
        fake.logged("read_more"),
        0,
        "no further READ after the mismatch"
    );
    // A larger file whose CREATE shows another size is closed unread.
    let fake = Fake::new(64, 64);
    let mut list = items(&fake, 2, 300);
    list[1].size = 200;
    let events = get(&fake, &list);
    assert!(
        events.contains(&"end 1 InvalidData".to_string()),
        "{events:?}"
    );
    assert_eq!(fake.logged("read_more"), 4, "only the first file's blocks");
}

#[test]
fn speed_task_smb_batch_get_delivers_in_request_order() {
    let fake = Fake::new(1 << 20, 1 << 20);
    let list = items(&fake, 3, 10);
    fake.faults
        .borrow_mut()
        .slow_read
        .insert(list[0].path.clone(), 5 * RTT);
    let events = get(&fake, &list);
    let begins: Vec<&String> = events.iter().filter(|e| e.starts_with("begin")).collect();
    assert_eq!(begins, ["begin 0 10", "begin 1 10", "begin 2 10"]);
    assert_eq!(
        fake.clock.now.get(),
        6 * RTT,
        "the slow member alone sets the time"
    );
    // A large file: the blocks after the first arrive together.
    let fake = Fake::new(1024, 64);
    let list = items(&fake, 2, 300);
    let events = get(&fake, &list);
    assert!(events.contains(&"end 1 ok".to_string()));
    assert_eq!(
        fake.clock.now.get(),
        3 * RTT,
        "open, the blocks together, close"
    );
}

#[test]
fn speed_task_smb_batch_get_reparse_entries_use_the_followed_read() {
    let fake = Fake::new(1 << 20, 1 << 20);
    let list = items(&fake, 3, 10);
    fake.faults
        .borrow_mut()
        .reparse
        .insert(list[0].path.clone());
    fake.faults
        .borrow_mut()
        .reparse
        .insert(list[1].path.clone());
    fake.faults.borrow_mut().links.insert(list[1].path.clone());
    let events = get(&fake, &list);
    assert!(
        events.contains(&"end 0 ok".to_string()),
        "dedup file is read"
    );
    assert!(
        events.contains(&"failed 1 InvalidInput".to_string()),
        "{events:?}"
    );
    assert_eq!(fake.logged("followed"), 2);
}

#[test]
fn speed_task_smb_batch_limits_none_without_live_generation() {
    assert!(limits(false).is_none());
    let found = limits(true).expect("a live connection carries packets");
    assert_eq!(found.max_files, BATCH_MAX_FILES);
    assert_eq!(found.max_bytes, BATCH_MAX_BYTES);
    assert_eq!((found.max_files, found.max_bytes), (256, 16 * 1024 * 1024));
}
