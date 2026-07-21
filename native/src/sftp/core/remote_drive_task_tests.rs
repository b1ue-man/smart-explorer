use super::{
    backend::{append_bounded, CapturedExec},
    errors::io_err,
    session::interleave_addresses,
};
use russh_sftp::client::error::Error as SftpError;
use std::{io, net::SocketAddr};

#[test]
fn remote_drive_task_sftp_interleaves_and_deduplicates_address_families() {
    let v4a: SocketAddr = "192.0.2.10:22".parse().unwrap();
    let v4b: SocketAddr = "192.0.2.11:22".parse().unwrap();
    let v6a: SocketAddr = "[2001:db8::10]:22".parse().unwrap();
    let v6b: SocketAddr = "[2001:db8::11]:22".parse().unwrap();

    assert_eq!(
        interleave_addresses(vec![v4a, v4a, v4b, v6a, v6b]),
        vec![v4a, v6a, v4b, v6b]
    );
    assert_eq!(
        interleave_addresses(vec![v6a, v6a, v6b, v4a, v4b]),
        vec![v6a, v4a, v6b, v4b]
    );
}

#[test]
fn remote_drive_task_sftp_preserves_typed_transport_errors() {
    let raw = io_err(russh::Error::IO(io::Error::from_raw_os_error(10060)));
    assert_eq!(raw.raw_os_error(), Some(10060));

    let timeout = io_err(SftpError::Timeout);
    assert_eq!(timeout.kind(), io::ErrorKind::TimedOut);

    let denied = io_err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
    assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn remote_drive_task_sftp_exec_capture_is_bounded_and_requires_success() {
    let mut bytes = Vec::new();
    assert!(!append_bounded(&mut bytes, b"1234", 6));
    assert!(append_bounded(&mut bytes, b"5678", 6));
    assert_eq!(bytes, b"123456");

    let output = CapturedExec {
        stdout: b" Linux x86_64\n".to_vec(),
        exit_status: Some(0),
        ..CapturedExec::default()
    }
    .finish()
    .unwrap();
    assert_eq!(output, "Linux x86_64");

    let missing = CapturedExec::default().finish().unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::InvalidData);
    assert!(CapturedExec {
        exit_status: Some(7),
        stderr: b"denied".to_vec(),
        ..CapturedExec::default()
    }
    .finish()
    .unwrap_err()
    .to_string()
    .contains("status 7: denied"));
    assert!(CapturedExec {
        exit_status: Some(0),
        exit_signal: Some("TERM".into()),
        ..CapturedExec::default()
    }
    .finish()
    .is_err());
    assert_eq!(
        CapturedExec {
            exit_status: Some(0),
            stdout_truncated: true,
            ..CapturedExec::default()
        }
        .finish()
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidData
    );
}
