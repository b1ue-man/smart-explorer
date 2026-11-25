use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use super::*;
use crate::screen::core::video_tcp_wire::{
    CandidateKind, TcpCandidate, TcpFailureReason, TcpPathKind, TcpToken, VideoTcpAnswer,
    MAX_LINK_RECORD_BYTES, TOKEN_BYTES,
};

fn run<F: Future>(work: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(work)
}

fn soon(millis: u64) -> Instant {
    Instant::now() + Duration::from_millis(millis)
}

/// Both sides on 127.0.0.1, each with its own listener port; the host's
/// answer optionally with another token or key pin.
async fn race_pair(
    token: Option<[u8; TOKEN_BYTES]>,
    pin: Option<[u8; 32]>,
    deadline: Instant,
) -> (
    Result<TcpPictureLink, TcpLinkFailure>,
    Result<TcpPictureLink, TcpLinkFailure>,
) {
    let host = HostTcpLocal::loopback().unwrap();
    let viewer = ViewerTcpLocal::loopback().unwrap();
    let mut answer = host.answer();
    if let Some(token) = token {
        answer.token = TcpToken(token);
    }
    if let Some(pin) = pin {
        answer.cert_sha256 = pin;
    }
    let offer = viewer.offer();
    let host = tokio::spawn(async move { host.establish(&offer, deadline).await });
    let viewer = tokio::spawn(async move { viewer.establish(&answer, deadline).await });
    (host.await.unwrap(), viewer.await.unwrap())
}

fn reason(result: Result<TcpPictureLink, TcpLinkFailure>) -> TcpFailureReason {
    match result {
        Ok(_) => panic!("the link came up"),
        Err(failure) => failure.reason,
    }
}

/// User 2026-11-25 („TCP und UDP, DIREKT“): two sides on one machine, each
/// listening on its port and dialing the other's from it (TCP simultaneous
/// open or the listener, whichever wins), bring up one link; records of
/// any size up to the bound travel both ways, a clean end reads as `None`,
/// an oversized record is refused before its memory is requested.
#[test]
fn video_tcp_task_links_establish_by_simultaneous_open_over_loopback() {
    run(async {
        let (host, viewer) = race_pair(None, None, soon(5_000)).await;
        let (mut host, mut viewer) = (host.unwrap(), viewer.unwrap());
        assert_eq!(host.info.remote, viewer.info.local);
        assert_eq!(viewer.info.remote, host.info.local);
        assert_eq!(host.info.path, TcpPathKind::Lan);
        assert_eq!(viewer.info.path, TcpPathKind::Lan);
        assert!(host.info.remote.ip().is_loopback());
        assert!(matches!(
            host.result(),
            VideoTcpResult::Connected {
                path: TcpPathKind::Lan,
                ..
            }
        ));

        let big: Vec<u8> = (0..(1 << 20) + 7).map(|n| (n % 251) as u8).collect();
        let expected = [b"head".as_slice(), big.as_slice()].concat();
        let mut writer = host.writer;
        let sender = tokio::spawn(async move {
            writer.send(&[b"head", &big]).await.unwrap();
            writer.send(&[b"small"]).await.unwrap();
            writer
        });
        assert_eq!(viewer.reader.recv().await.unwrap().unwrap(), expected);
        assert_eq!(viewer.reader.recv().await.unwrap().unwrap(), b"small");
        let host_writer = sender.await.unwrap();
        assert_eq!(
            host_writer.bytes_sent(),
            (4 + expected.len() + 4 + 5) as u64 + 4
        );
        assert_eq!(viewer.reader.bytes_received(), host_writer.bytes_sent());

        viewer.writer.send(&[b"ba", b"ck"]).await.unwrap();
        assert_eq!(host.reader.recv().await.unwrap().unwrap(), b"back");

        // A clean end between records.
        host_writer.finish().await.unwrap();
        assert_eq!(viewer.reader.recv().await.unwrap(), None);

        // Bounds: the reader's, and the writer's before anything is sent.
        host.reader.set_max_record(16);
        viewer.writer.send(&[&[0u8; 17]]).await.unwrap();
        let error = host.reader.recv().await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let chunk = vec![0u8; 1 << 20];
        let pieces = MAX_LINK_RECORD_BYTES / chunk.len() + 1;
        let parts: Vec<&[u8]> = (0..pieces).map(|_| chunk.as_slice()).collect();
        let error = viewer.writer.send(&parts).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    });
}

/// The viewer pins the SHA-256 of the host's key from the answer: the
/// right one passes (previous test), another one fails TLS on both sides.
#[test]
fn video_tcp_task_tls_pin_refuses_another_host_key() {
    run(async {
        let (host, viewer) = race_pair(None, Some([0x5a; 32]), soon(2_500)).await;
        assert_eq!(reason(viewer), TcpFailureReason::Tls);
        assert_eq!(reason(host), TcpFailureReason::Tls);
    });
}

/// The host compares the viewer's token in constant time and drops a
/// connection with another one; the viewer then sees the refusal.
#[test]
fn video_tcp_task_wrong_token_is_dropped_and_reported() {
    let token = TcpToken([9; TOKEN_BYTES]);
    assert!(tls::token_matches(&token, &[9; TOKEN_BYTES]));
    let mut other = [9; TOKEN_BYTES];
    other[TOKEN_BYTES - 1] = 8;
    assert!(!tls::token_matches(&token, &other));
    run(async {
        let (host, viewer) = race_pair(Some([1; TOKEN_BYTES]), None, soon(2_500)).await;
        let host = host.err().unwrap();
        assert_eq!(host.reason, TcpFailureReason::WrongToken);
        assert!(host.status_line().contains("falscher Sitzungsschlüssel"));
        assert_eq!(reason(viewer), TcpFailureReason::Refused);
    });
}

/// Without a reachable candidate the reason is „nicht erreichbar“; a
/// connection that never speaks TLS ends in the time-out; every reason has
/// its German status line.
#[test]
fn video_tcp_task_failures_name_their_german_reason() {
    run(async {
        // A bound port without a listener refuses every SYN.
        let closed = tokio::net::TcpSocket::new_v4().unwrap();
        closed.bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
        let refusing = closed.local_addr().unwrap();
        let viewer = ViewerTcpLocal::loopback().unwrap();
        let answer = answer_with(refusing);
        let failure = viewer.establish(&answer, soon(1_200)).await.err().unwrap();
        assert_eq!(failure.reason, TcpFailureReason::Unreachable);
        assert!(failure
            .status_line()
            .starts_with("TCP direkt nicht möglich: keine TCP-Adresse der Gegenstelle erreichbar"));
        assert_eq!(
            failure.result(),
            VideoTcpResult::Failed {
                reason: TcpFailureReason::Unreachable
            }
        );

        // A listener that accepts and stays silent.
        let silent = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let silent_addr = silent.local_addr().unwrap();
        let holder = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = silent.accept().await {
                held.push(stream);
            }
        });
        let viewer = ViewerTcpLocal::loopback().unwrap();
        let failure = viewer
            .establish(&answer_with(silent_addr), soon(1_500))
            .await
            .err()
            .unwrap();
        assert_eq!(failure.reason, TcpFailureReason::Timeout);
        assert_eq!(
            failure.status_line(),
            "TCP direkt nicht möglich: Zeitüberschreitung beim Aufbau der TCP-Verbindung"
        );
        holder.abort();
    });
    let local = TcpLinkFailure::new(TcpFailureReason::LocalSocket, Some("EACCES".into()));
    assert_eq!(
        local.to_string(),
        "TCP direkt nicht möglich: TCP-Port konnte auf diesem Gerät nicht geöffnet werden (EACCES)"
    );
}

fn answer_with(addr: SocketAddr) -> VideoTcpAnswer {
    VideoTcpAnswer {
        candidates: vec![TcpCandidate {
            addr,
            kind: CandidateKind::Local,
        }],
        token: TcpToken([3; TOKEN_BYTES]),
        cert_sha256: [4; 32],
    }
}
