//! Acceptance signals of the FTP upload round trips (E13, P3): publishing a
//! stage on a server without MLST (vsftpd) probes no other file of the folder,
//! and a new pooled connection does not ask FEAT again.
use super::data_overlap_server::{lock, Server};
use super::io_adapters::FtpConnection;
use super::listing_detail::{inherited_features, Detail};
use super::metadata::Features;
use super::staging::publish_with;
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use suppaftp::FtpError;

fn plain(server: &Server) -> Arc<FtpConnection> {
    server.connection(|address| TcpStream::connect(address).map_err(FtpError::ConnectionError))
}

fn events_starting(server: &Server, prefix: &str) -> usize {
    lock(&server.state.events)
        .iter()
        .filter(|line| line.starts_with(prefix))
        .count()
}

#[test]
fn speed_task_ftp_kinds_listing_skips_the_per_file_probes() {
    assert!(!Detail::Kinds.refines_list_rows());
    assert!(!Detail::Kinds.records_precision());
}

#[test]
fn speed_task_ftp_exact_listing_keeps_the_per_file_probes() {
    assert!(Detail::Exact.refines_list_rows());
    assert!(Detail::Exact.records_precision());
}

#[test]
fn speed_task_ftp_new_connection_takes_over_the_feat_answer() {
    let known = Features {
        mlst: false,
        mlsd: false,
        mfmt: true,
    };
    let own = Features {
        mlst: true,
        mlsd: true,
        mfmt: false,
    };
    let taken = inherited_features(None, Some(known)).expect("the other answer");
    assert!(taken.mfmt && !taken.mlst);
    let kept = inherited_features(Some(own), Some(known)).expect("its own answer");
    assert!(kept.mlst && !kept.mfmt);
    assert!(inherited_features(None, None).is_none());
}

#[test]
fn speed_task_ftp_inherited_feat_answer_saves_the_request() {
    let server = Server::start();
    server.state.no_mlst.store(true, Ordering::SeqCst);
    let first = plain(&server);
    let second = plain(&server);
    assert!(!first.features().expect("FEAT").mlst);
    assert_eq!(events_starting(&server, "FEAT"), 1);
    second.inherit_server_facts(&first);
    assert!(!second.features().expect("inherited").mlst);
    assert_eq!(events_starting(&server, "FEAT"), 1, "no second FEAT");
}

#[test]
fn speed_task_ftp_publish_without_mlst_probes_no_other_file() {
    let server = Server::start();
    server.state.no_mlst.store(true, Ordering::SeqCst);
    {
        let mut files = lock(&server.state.files);
        for name in ["/up/stage", "/up/other-1", "/up/other-2", "/up/other-3"] {
            files.insert(name.to_string(), b"x".to_vec());
        }
    }
    let connection = plain(&server);
    publish_with(true, &connection, "/up/stage", "/up/final", false).expect("published");
    assert!(lock(&server.state.files).contains_key("/up/final"));
    assert!(!lock(&server.state.files).contains_key("/up/stage"));
    assert_eq!(events_starting(&server, "LIST"), 1, "one listing");
    assert_eq!(events_starting(&server, "SIZE"), 0, "no SIZE of any file");
    assert_eq!(events_starting(&server, "MDTM"), 0, "no MDTM of any file");
}

#[test]
fn speed_task_ftp_publish_without_mlst_still_refuses_an_existing_destination() {
    let server = Server::start();
    server.state.no_mlst.store(true, Ordering::SeqCst);
    {
        let mut files = lock(&server.state.files);
        files.insert("/up/stage".to_string(), b"new".to_vec());
        files.insert("/up/final".to_string(), b"old".to_vec());
    }
    let connection = plain(&server);
    let error = publish_with(true, &connection, "/up/stage", "/up/final", false)
        .expect_err("an existing destination is not replaced");
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(lock(&server.state.files)["/up/final"], b"old");
    assert_eq!(events_starting(&server, "RNFR"), 0, "nothing was renamed");
}
