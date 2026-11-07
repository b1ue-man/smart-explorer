use super::*;

fn object(id: &str) -> DriveObject {
    DriveObject {
        id: id.into(),
        mime_type: "application/octet-stream".into(),
        size: Some(3),
        md5: Some("900150983cd24fb0d6963f7d28e17f72".into()),
        mtime_ms: None,
    }
}

#[test]
fn duplicate_name_is_never_selected_arbitrarily() {
    // Case "duplicate name": two objects under one name are never picked arbitrarily.
    let error = require_one(vec![object("a"), object("b")], "destination").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "duplicate name");

    // Case "ordinary rename": an occupied destination is refused, a free one accepted.
    assert!(
        require_absent(&[], "destination").is_ok(),
        "ordinary rename"
    );
    let error = require_absent(&[object("other")], "destination").unwrap_err();
    assert_eq!(
        error.kind(),
        io::ErrorKind::AlreadyExists,
        "ordinary rename"
    );
}

#[test]
fn post_rename_verification_requires_only_the_expected_id() {
    assert!(verify_unique_id(&[object("stage")], "stage").is_ok());
    assert!(verify_unique_id(&[object("other")], "stage").is_err());
    assert!(verify_unique_id(&[object("stage"), object("other")], "stage").is_err());
}

#[test]
fn staged_bytes_must_match_remote_metadata() {
    let staged = object("stage");
    assert!(validate_staged_content(&staged, 3, "900150983CD24FB0D6963F7D28E17F72").is_ok());
    assert!(validate_staged_content(&staged, 4, staged.md5.as_deref().unwrap()).is_err());
}
