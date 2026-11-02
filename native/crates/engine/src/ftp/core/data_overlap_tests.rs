//! Acceptance signals of plan U46-5/U46-6 against the scripted server of
//! data_overlap_server.rs. suppaftp 10.0.2 sends STOR/RETR right after the
//! answer to PASV and connects the data socket afterwards (`data_command`),
//! so the overlap of command and data connection needs no code of its own:
//! these tests keep it, and the rule "every reply before a dependent
//! command", from regressing.
use super::data_overlap_server::{lock, Server};
use super::staging::publish_with;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::Ordering;
use suppaftp::FtpError;

#[test]
fn speed_task_ftp_stor_is_sent_while_the_data_connection_connects() {
    let server = Server::start();
    let connection = server.connection_after_command();
    let mut writer = connection
        .open_store("/stage", 5)
        .expect("STOR before connect");
    writer.write_all(b"hello").unwrap();
    writer.flush().unwrap();
    assert_eq!(lock(&server.state.files)["/stage"], b"hello");
    let (stor, accepted) = (
        server.state.position("STOR /stage"),
        server.state.position("data accepted"),
    );
    assert!(
        stor.is_some() && stor < accepted,
        "STOR is read before the data accept"
    );
}

#[test]
fn speed_task_ftp_retr_is_sent_while_the_data_connection_connects() {
    let server = Server::start();
    lock(&server.state.files).insert("/file".to_string(), b"payload".to_vec());
    let connection = server.connection_after_command();
    let mut reader = connection
        .open_reader_at("/file", 0)
        .unwrap()
        .expect("RETR");
    let mut body = Vec::new();
    reader.read_to_end(&mut body).unwrap();
    assert_eq!(body, b"payload");
    let (retr, accepted) = (
        server.state.position("RETR /file"),
        server.state.position("data accepted"),
    );
    assert!(retr.is_some() && retr < accepted);
}

#[test]
fn speed_task_ftp_overlap_failure_before_150_reports_possible_stage() {
    let server = Server::start();
    let refused = |_address: SocketAddr| {
        let error = io::Error::new(io::ErrorKind::ConnectionRefused, "data socket refused");
        Err(FtpError::ConnectionError(error))
    };
    let connection = server.connection(refused);
    let error = connection
        .open_store("/stage", 5)
        .err()
        .expect("no data connection");
    assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
    assert!(error.to_string().contains("/stage"), "{error}");
    assert!(
        error.to_string().contains("möglicherweise angelegt"),
        "{error}"
    );
    // The control connection that sent STOR is not reused out of step.
    let pwd = connection.with_stream_read(|stream| stream.pwd().map_err(io::Error::other));
    assert_eq!(pwd.unwrap(), "/");
}

#[test]
fn speed_task_ftp_still_waits_for_every_reply_before_dependent_commands() {
    let server = Server::start();
    let connection = server.connection_after_command();
    let mut writer = connection.open_store("/a", 3).unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    drop(writer);
    connection
        .with_stream_mutation(|stream| stream.rename("/a", "/b").map_err(io::Error::other))
        .unwrap();
    let pwd = connection.with_stream_read(|stream| stream.pwd().map_err(io::Error::other));
    assert_eq!(pwd.unwrap(), "/");
    assert!(
        !server.state.pipelined.load(Ordering::SeqCst),
        "a command arrived ahead of a reply"
    );
    assert!(lock(&server.state.files).contains_key("/b"));
}

#[test]
fn speed_task_ftp_publish_without_stage_probe_maps_rnfr_550_to_not_found() {
    let server = Server::start();
    let connection =
        server.connection(|address| TcpStream::connect(address).map_err(FtpError::ConnectionError));
    for verify in [true, false] {
        let error = publish_with(verify, &connection, "/missing", "/final", false)
            .expect_err("a missing stage is not published");
        assert_eq!(
            error.kind(),
            io::ErrorKind::NotFound,
            "verify={verify}: {error}"
        );
    }
    let events = lock(&server.state.events).clone();
    let rnfr = events
        .iter()
        .filter(|line| *line == "RNFR /missing")
        .count();
    assert_eq!(
        rnfr, 1,
        "only the unverified branch reaches RNFR: {events:?}"
    );
    // The stage exists: the unverified branch renames it like the verified one.
    lock(&server.state.files).insert("/stage".to_string(), b"x".to_vec());
    publish_with(false, &connection, "/stage", "/final", false).unwrap();
    assert!(lock(&server.state.files).contains_key("/final"));
    assert!(!lock(&server.state.files).contains_key("/stage"));
}
