//! Acceptance signals of the FTP upload round trips (E13, P3): publishing a
//! stage on a server without MLST (vsftpd) probes no other file of the folder,
//! and a new pooled connection does not ask FEAT again. A stage named after
//! its destination is looked at with one filtered LIST (pair_listing.rs).
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

#[test]
fn speed_task_ftp_publish_without_mlst_lists_only_the_stage_and_its_name() {
    let server = Server::start();
    server.state.no_mlst.store(true, Ordering::SeqCst);
    let stage = "/up/final.se-upload-0123456789abcdef";
    {
        let mut files = lock(&server.state.files);
        files.insert(stage.to_string(), b"new".to_vec());
        files.insert("/up/other-1".to_string(), b"x".to_vec());
    }
    let connection = plain(&server);
    publish_with(true, &connection, stage, "/up/final", false).expect("published");
    assert_eq!(lock(&server.state.files)["/up/final"], b"new");
    assert!(!lock(&server.state.files).contains_key(stage));
    assert_eq!(events_starting(&server, "LIST -a /up/final*"), 1);
    assert_eq!(
        events_starting(&server, "LIST"),
        1,
        "no listing of the folder"
    );
    assert_eq!(events_starting(&server, "CWD"), 0, "no folder change");
    assert_eq!(events_starting(&server, "SIZE"), 0);
}

#[test]
fn speed_task_ftp_publish_by_pattern_still_refuses_an_existing_destination() {
    let server = Server::start();
    server.state.no_mlst.store(true, Ordering::SeqCst);
    let stage = "/up/final.se-upload-0123456789abcdef";
    {
        let mut files = lock(&server.state.files);
        files.insert(stage.to_string(), b"new".to_vec());
        files.insert("/up/final".to_string(), b"old".to_vec());
    }
    let connection = plain(&server);
    let error = publish_with(true, &connection, stage, "/up/final", false)
        .expect_err("an existing destination is not replaced");
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(lock(&server.state.files)["/up/final"], b"old");
    assert_eq!(events_starting(&server, "RNFR"), 0, "nothing was renamed");
}

#[test]
fn speed_task_ftp_probe_helpers_grow_with_the_square_root() {
    use super::listing_detail::helper_count;
    assert_eq!(helper_count(0), 0);
    assert_eq!(helper_count(1), 0);
    assert_eq!(helper_count(100), 9);
    assert_eq!(helper_count(5000), 70);
}

#[test]
fn speed_task_ftp_listing_probes_every_row_once_on_pool_connections() {
    use super::connection::connect_stream;
    use super::io_adapters::FtpReconnect;
    use super::pool::FtpPool;
    let server = Server::start();
    server.state.no_mlst.store(true, Ordering::SeqCst);
    let names: Vec<String> = (0..16).map(|index| format!("f{index:02}")).collect();
    {
        let mut files = lock(&server.state.files);
        for (index, name) in names.iter().enumerate() {
            files.insert(format!("/d/{name}"), vec![b'x'; index + 1]);
        }
    }
    let url = server.url();
    let reconnect: FtpReconnect = Arc::new(move || connect_stream(&url));
    let primary = FtpConnection::new(reconnect().expect("log in"), reconnect.clone()).unwrap();
    let pool = FtpPool::new(primary, reconnect);
    let listing = super::metadata::list_with(pool.primary(), "/d", &pool).expect("listed");
    assert!(listing.omitted.is_empty(), "{:?}", listing.omitted);
    assert_eq!(listing.entries.len(), names.len());
    for entry in &listing.entries {
        let index: usize = entry.name[1..].parse().unwrap();
        assert_eq!(entry.size, index as u64 + 1, "{}", entry.name);
        // MDTM answers 20260101000000, the LIST row says 2018.
        assert_eq!(entry.mtime_ms, 1_767_225_600_000, "{}", entry.name);
    }
    for name in &names {
        let size = format!("SIZE /d/{name}");
        let mdtm = format!("MDTM /d/{name}");
        assert_eq!(events_starting(&server, &size), 1, "{size}");
        assert_eq!(events_starting(&server, &mdtm), 1, "{mdtm}");
    }
}
