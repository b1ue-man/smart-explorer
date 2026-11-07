//! The MD5 cases of the FTP content-digest tests in
//! `abgleich_task_hash_tests.rs`: each is a case of the SHA-256 test with the
//! same setup and runs on the loopback server there.
use super::*;

pub(super) fn abgleich_task_ftp_md5_by_hash_command_without_a_data_connection() {
    let server = Server::start(Script::with_feat(&["SIZE", "HASH SHA-256;MD5*"]));
    let backend = server.backend();
    assert_eq!(md5(&backend).unwrap().as_deref(), Some(MD5));
    let commands = server.commands();
    assert_no_data(&commands);
    let opts = commands
        .iter()
        .position(|c| c == "OPTS HASH MD5")
        .expect("the algorithm was selected");
    let hash = commands.iter().position(|c| c == "HASH /f.bin").unwrap();
    assert!(opts < hash, "{commands:?}");
    // The SHA-256 question after it selects its own algorithm again.
    assert_eq!(digest(&backend).unwrap().expect("hashed").sha256, HEX);
}

pub(super) fn abgleich_task_ftp_md5_by_xmd5_command_takes_251_250_and_213() {
    for reply in [
        format!("251 {MD5}"),
        format!("250 {}", MD5.to_ascii_uppercase()),
        format!("213 {MD5} f.bin"),
    ] {
        let mut script = Script::with_feat(&["SIZE", "XCRC", "XMD5"]);
        script.xmd5 = reply.clone();
        let server = Server::start(script);
        assert_eq!(
            md5(&server.backend()).unwrap().as_deref(),
            Some(MD5),
            "{reply}"
        );
        let commands = server.commands();
        assert_no_data(&commands);
        assert!(commands.iter().any(|c| c == "XMD5 /f.bin"));
        assert!(!commands.iter().any(|c| c.starts_with("HASH")));
    }
}

pub(super) fn abgleich_task_ftp_md5_without_an_md5_command_leaves_the_content_to_the_caller() {
    let server = Server::start(Script::with_feat(&[
        "SIZE",
        "MDTM",
        "XSHA256",
        "HASH SHA-256;SHA-1*",
    ]));
    assert_eq!(md5(&server.backend()).unwrap(), None);
    let commands = server.commands();
    assert!(
        !commands.iter().any(|c| c.starts_with("HASH")
            || c.starts_with("XMD5")
            || c.starts_with("XSHA256")
            || c.starts_with("OPTS")),
        "{commands:?}"
    );
}

pub(super) fn abgleich_task_ftp_md5_of_another_algorithm_or_range_is_not_taken() {
    for (features, field, reply) in [
        (
            ["SIZE", "HASH MD5"],
            0,
            format!("213 SHA-1 0-4 {MD5} /f.bin"),
        ),
        (["SIZE", "HASH MD5"], 0, format!("213 MD5 0-2 {MD5} /f.bin")),
        (["SIZE", "HASH MD5"], 0, format!("213 MD5 0-4 {HEX} /f.bin")),
        (
            ["SIZE", "HASH MD5"],
            0,
            "213 MD5 0-4 nothex /f.bin".to_string(),
        ),
        (
            ["SIZE", "HASH MD5"],
            0,
            "556 refused for policy reasons".to_string(),
        ),
        (["SIZE", "XMD5"], 1, format!("251 {HEX}")),
        (["SIZE", "XMD5"], 1, format!("251 {MD5} {MD5}")),
        (["SIZE", "XMD5"], 1, "550 no such file".to_string()),
        (["SIZE", "XMD5"], 1, format!("500 {MD5}")),
    ] {
        let mut script = Script::with_feat(&features);
        if field == 0 {
            script.md5_hash = reply.clone();
        } else {
            script.xmd5 = reply.clone();
        }
        let server = Server::start(script);
        assert_eq!(md5(&server.backend()).unwrap(), None, "{reply}");
        assert_no_data(&server.commands());
    }
}

pub(super) fn abgleich_task_ftp_md5_of_a_file_written_while_hashed_is_an_error() {
    let mut script = Script::with_feat(&["SIZE", "HASH MD5"]);
    script.mdtm_after_hash = "213 20260101000100";
    let server = Server::start(script);
    let error = md5(&server.backend()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

pub(super) fn abgleich_task_ftp_md5_replies_parse_strictly() {
    use super::super::content_digest::{
        parse_bare_reply_for, parse_hash_reply_for, Algorithm::Md5,
    };
    assert_eq!(
        parse_hash_reply_for(&format!("213 md5 0-9 {MD5} x"), 10, Md5).as_deref(),
        Some(MD5)
    );
    assert_eq!(
        parse_hash_reply_for(&format!("213 SHA-256 0-9 {MD5} x"), 10, Md5),
        None
    );
    assert_eq!(
        parse_bare_reply_for(&format!("251 {MD5}"), Md5).as_deref(),
        Some(MD5)
    );
    assert_eq!(parse_bare_reply_for(&format!("250 {HEX}"), Md5), None);
    assert_eq!(parse_bare_reply_for(&format!("500 {MD5}"), Md5), None);
    assert_eq!(
        parse_bare_reply_for(&format!("251 {MD5} {MD5}"), Md5),
        None,
        "two digests are none"
    );
}
