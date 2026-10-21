//! Ranges over SMB (E13, U48-7): the READs of a range stay within it, and the
//! writer of ranges is off until its security question is answered. The
//! reader itself needs a server (smb2's `FileReader` cannot be built here),
//! so the bounds are checked where they are decided.
use super::reader::{bounded_size, next_read};
use super::segmented_stage::SMB_SEGMENTED_WRITE;

#[test]
fn speed_task_smb_range_reader_stops_at_end() {
    // The range bounds the size the READs are cut by.
    assert_eq!(bounded_size(1000, Some(400)), 400);
    assert_eq!(bounded_size(1000, None), 1000);
    // A range past the end of the file ends with the file (it shrank).
    assert_eq!(bounded_size(300, Some(400)), 300);
    assert_eq!(
        bounded_size(1000, Some(0)),
        0,
        "an empty range reads nothing"
    );
    // The last READ is shortened; nothing is asked for beyond.
    assert_eq!(next_read(900, 1000, 512), Some((900, 100)));
    assert_eq!(next_read(1000, 1000, 512), None);
    assert_eq!(next_read(1200, 1000, 512), None);
}

#[test]
fn speed_task_smb_range_reader_pipeline_stays_within_range() {
    // The READs issued for one range, as `top_up` asks for them.
    let (start, end, file, chunk) = (1_000u64, 5_000u64, 10_000u64, 512u64);
    let size = bounded_size(file, Some(end));
    let mut next = start;
    let mut reads = Vec::new();
    while let Some((offset, len)) = next_read(next, size, chunk) {
        reads.push((offset, len));
        next = offset + len;
    }
    assert_eq!(reads.first(), Some(&(start, chunk)));
    assert!(reads.iter().all(|(offset, len)| offset + len <= end));
    assert_eq!(
        reads.iter().map(|(_, len)| len).sum::<u64>(),
        end - start,
        "exactly the range, no byte twice, none skipped"
    );
    assert_eq!(reads.last().map(|(offset, len)| offset + len), Some(end));
    // Two neighbouring ranges ask for disjoint bytes.
    let second_size = bounded_size(file, Some(9_000));
    assert_eq!(next_read(end, second_size, chunk), Some((end, chunk)));
}

#[test]
fn speed_task_smb_segmented_write_is_off_until_answered() {
    let switch = std::hint::black_box(SMB_SEGMENTED_WRITE);
    assert!(
        !switch,
        "reopening a stage by path is not yet judged for SMB"
    );
}
