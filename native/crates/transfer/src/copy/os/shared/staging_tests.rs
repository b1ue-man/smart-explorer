use super::*;

#[test]
fn speed_task_local_staging_side_follows_the_tag_before_the_kind() {
    // Plain errors are sorted by their kind: only what a destination can
    // cause belongs to the target.
    let plain = |kind| side_of(io::Error::new(kind, "x"));
    assert!(matches!(
        plain(io::ErrorKind::StorageFull),
        LocalFailure::Target(_)
    ));
    assert!(matches!(
        plain(io::ErrorKind::Other),
        LocalFailure::Source(_)
    ));
    // A platform copy that knows the side says so, whatever the kind: a
    // write error with an ordinary kind is the target's, a read error that
    // looks like a full disk is the source's, and the error inside stays
    // intact.
    let write = io::Error::other(LocalFailure::Target(io::Error::from_raw_os_error(5)));
    match side_of(write) {
        LocalFailure::Target(error) => assert_eq!(error.raw_os_error(), Some(5)),
        other => panic!("{other:?}"),
    }
    let read = io::Error::other(LocalFailure::Source(io::Error::from(
        io::ErrorKind::StorageFull,
    )));
    match side_of(read) {
        LocalFailure::Source(error) => assert_eq!(error.kind(), io::ErrorKind::StorageFull),
        other => panic!("{other:?}"),
    }
    // A foreign payload is no tag.
    let foreign = io::Error::other("something else");
    assert!(matches!(side_of(foreign), LocalFailure::Source(_)));
}
