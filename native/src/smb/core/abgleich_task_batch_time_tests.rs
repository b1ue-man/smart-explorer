//! Packets give every file the time of its source (review W4): SMB puts a
//! SET_INFO of LastWriteTime into the compound of the file, so the packet
//! costs no round trip more; a file without a time keeps the time of its
//! upload.
use super::batch_fake::{entries, published, put, Fake, RTT};
use super::stage_finish::filetime;

#[test]
fn abgleich_task_smb_packet_files_carry_the_time_of_their_source() {
    let fake = Fake::new(1 << 20, 1 << 20);
    let (mut list, mut data) = entries(3, 4096);
    list[0].mtime_ms = Some(1_700_000_000_000);
    list[2].mtime_ms = Some(1_600_000_000_500);
    let outcomes = put(&fake, &list, &mut data);
    let paths: Vec<&str> = outcomes.iter().map(published).collect();
    let times = fake.times.borrow();
    assert_eq!(times.get(paths[0]).copied(), filetime(1_700_000_000_000));
    assert_eq!(
        times.get(paths[1]),
        None,
        "no time: the upload's time stays"
    );
    assert_eq!(times.get(paths[2]).copied(), filetime(1_600_000_000_500));
    // CREATE+WRITE+SET_INFO+CLOSE in one compound, then the rename.
    assert!(fake.clock.now.get() <= 3 * RTT, "{}", fake.clock.now.get());
}

#[test]
fn abgleich_task_smb_packet_time_of_a_large_file_goes_with_its_close() {
    let timed = Fake::new(1024, 1024);
    let (mut list, mut data) = entries(2, 5000);
    list[1].mtime_ms = Some(1_700_000_000_000);
    let outcomes = put(&timed, &list, &mut data);
    let paths: Vec<&str> = outcomes.iter().map(published).collect();
    let times = timed.times.borrow();
    assert_eq!(times.get(paths[0]), None);
    assert_eq!(times.get(paths[1]).copied(), filetime(1_700_000_000_000));
    // CREATE+first block, the other blocks together, SET_INFO+CLOSE, rename:
    // the same four rounds as without a time.
    assert_eq!(timed.clock.now.get(), 4 * RTT);
}
