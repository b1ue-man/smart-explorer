//! The account helper frames in the exact JSON form of contract D.2.7, the
//! frame limits and the codes a refused account start hands on.
use std::io;

use serde_json::{json, Value};

use super::*;

fn round_trip<T>(message: &T, expected: Value)
where
    T: Serialize + DeserializeOwned + PartialEq + fmt::Debug,
{
    let frame = encode_frame(message).unwrap();
    let length = frame_length(frame[..4].try_into().unwrap()).unwrap();
    assert_eq!(length, frame.len() - 4);
    let object: Value = serde_json::from_slice(&frame[4..]).unwrap();
    assert_eq!(object, expected);
    let read: T = read_frame(&mut &frame[..]).unwrap().unwrap();
    assert_eq!(&read, message);
}

#[test]
fn helper_protocol_frames_follow_contract_d27() {
    round_trip(
        &HelperRequest::Hello {
            protocol: 1,
            se_version: "0.5.183".into(),
        },
        json!({"hello": {"protocol": 1, "se_version": "0.5.183"}}),
    );
    round_trip(
        &HelperRequest::Probe {
            account: "se-remote".into(),
        },
        json!({"probe": {"account": "se-remote"}}),
    );
    round_trip(
        &HelperRequest::Start {
            account: "se-remote".into(),
            cwd: None,
            pipe_in: None,
            pipe_out: None,
        },
        json!({"start": {"account": "se-remote", "cwd": null}}),
    );
    // Windows names the worker's two pipes instead of passing a descriptor.
    round_trip(
        &HelperRequest::Start {
            account: "se-remote".into(),
            cwd: None,
            pipe_in: Some(r"\\.\pipe\SmartExplorerExec-0a-in".into()),
            pipe_out: Some(r"\\.\pipe\SmartExplorerExec-0a-out".into()),
        },
        json!({"start": {
            "account": "se-remote",
            "cwd": null,
            "pipe_in": r"\\.\pipe\SmartExplorerExec-0a-in",
            "pipe_out": r"\\.\pipe\SmartExplorerExec-0a-out",
        }}),
    );
    round_trip(
        &HelperRequest::Stop {
            session: "smart-explorer-exec-0a.service".into(),
        },
        json!({"stop": {"session": "smart-explorer-exec-0a.service"}}),
    );
    round_trip(
        &HelperReply::Probe {
            ok: true,
            elevated: false,
            admin_group: false,
            detail: None,
        },
        json!({"probe": {"ok": true, "elevated": false, "admin_group": false, "detail": null}}),
    );
    round_trip(&HelperReply::Stopped {}, json!({"stopped": {}}));
    round_trip(
        &HelperReply::Status {
            active: true,
            empty: false,
        },
        json!({"status": {"active": true, "empty": false}}),
    );
    round_trip(
        &HelperReply::Error {
            code: HelperErrorCode::PasswordChangeRequired,
            detail: "x".into(),
        },
        json!({"error": {"code": "password_change_required", "detail": "x"}}),
    );
    // A code of a newer helper still reads, as an account the host cannot use.
    let newer: HelperReply =
        decode_payload(br#"{"error":{"code":"quota_exceeded","detail":"y"}}"#).unwrap();
    let HelperReply::Error { code, .. } = newer else {
        panic!("not an error reply: {newer:?}");
    };
    assert_eq!(code, HelperErrorCode::Other);
    assert_eq!(code.wire_code(), ACCOUNT_UNAVAILABLE);
    // The reason of a failed S4U probe (Windows) has its own code.
    assert_eq!(
        serde_json::to_value(AccountReason::LogonRefused).unwrap(),
        json!("logon_refused")
    );
}

#[test]
fn helper_protocol_limits_and_refusal_codes() {
    assert!(frame_length([0, 0, 0, 0]).is_err());
    assert!(frame_length(((MAX_HELPER_FRAME_BYTES + 1) as u32).to_be_bytes()).is_err());
    assert_eq!(
        frame_length((MAX_HELPER_FRAME_BYTES as u32).to_be_bytes()).unwrap(),
        MAX_HELPER_FRAME_BYTES
    );
    let oversized = HelperRequest::Probe {
        account: "a".repeat(MAX_HELPER_FRAME_BYTES),
    };
    assert!(encode_frame(&oversized).is_err());
    // A connection that ends between frames is no error; one inside a frame is.
    assert!(read_frame::<HelperReply>(&mut &b""[..]).unwrap().is_none());
    let error = read_frame::<HelperReply>(&mut &[0u8, 0][..]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

    assert_eq!(HelperErrorCode::NotAllowed.wire_code(), ACCOUNT_NOT_ALLOWED);
    assert_eq!(
        HelperErrorCode::Outdated.wire_code(),
        ACCOUNT_HELPER_UNAVAILABLE
    );
    let refused = account_start_error(ACCOUNT_NOT_ALLOWED, "account deploy is not allowed");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(account_error_code(&refused), Some(ACCOUNT_NOT_ALLOWED));
    assert_eq!(refused.to_string(), "account deploy is not allowed");
    assert_eq!(account_error_code(&io::Error::other("plain")), None);
}
