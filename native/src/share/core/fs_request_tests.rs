//! Wire compatibility of the reserved-directory rename request.
use super::*;
#[test]
fn batches_task_transfer_compat_reserved_share_rename_is_a_single_write_with_distinct_wire_operation(
) {
    let request = FsRequest::RenameReservedDirectory {
        src: "/owned".into(),
        dst: "/final".into(),
    };
    assert!(request.mutates_filesystem());
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(wire["op"], "rename_reserved_directory");
    assert_eq!(wire["src"], "/owned");
    assert_eq!(wire["dst"], "/final");
    let decoded: FsRequest = serde_json::from_value(wire).unwrap();
    assert!(decoded.mutates_filesystem());
    let ordinary: FsRequest =
        serde_json::from_str(r#"{"op":"rename_no_replace","src":"/old","dst":"/new"}"#).unwrap();
    assert_eq!(
        serde_json::to_value(ordinary).unwrap()["op"],
        "rename_no_replace"
    );
}
