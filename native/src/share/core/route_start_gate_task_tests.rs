//! The deterministic form of the switch the route monitor performs (Z-R2):
//! the relay is admitted first, one stream is open across the change, then
//! the direct grant replaces the relay grant. Linux only (private relay).
use std::io;
use std::time::Duration;

use iroh::endpoint::{ApplicationPathToken, ApplicationPathUpdate, Connection};

use super::task_tests::{content, relay_tx, runtime, send_file, sha, Pair, FILE, WAIT};

fn token(connection: &Connection, relay: bool) -> Option<ApplicationPathToken> {
    let paths = connection.paths();
    connection
        .application_path_policy()
        .paths
        .into_iter()
        .filter(|observed| observed.validated)
        .find(|observed| {
            paths
                .iter()
                .find(|path| path.id() == observed.token.path_id())
                .is_some_and(|path| path.is_relay() == relay)
        })
        .map(|observed| observed.token)
}

async fn admit(connection: &Connection, relay: bool) {
    let kind = if relay { "relay" } else { "direct" };
    tokio::time::timeout(WAIT, async {
        loop {
            if let Some(token) = token(connection, relay) {
                if connection
                    .update_application_path_policy(ApplicationPathUpdate::Allow(token))
                    .is_ok()
                {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("validated {kind} path"));
}

/// The deterministic switch the monitor performs: relay admitted first, a
/// stream in progress, then the direct grant replaces it.
#[test]
fn route_start_task_gate_switch_mid_transfer_keeps_every_file_once() {
    runtime().block_on(async {
        let relay = iroh::test_utils::run_relay_server_with(false).await;
        let (map, url, _relay) = relay.expect("private relay");
        let pair = Pair::new(&map, &url, true, false).await;
        admit(&pair.client, true).await;
        admit(&pair.host, true).await;
        for id in 0..4 {
            let sent = send_file(&pair.client, id, FILE).await;
            sent.expect("file over relay");
        }
        assert!(relay_tx(&pair.client) >= 4 * FILE as u64);
        // One stream spans the switch: its first half goes out over the relay,
        // its second half only after the direct grant replaced the relay grant.
        let big = 32 * FILE;
        let before = relay_tx(&pair.client);
        let (half_sent, half) = tokio::sync::oneshot::channel();
        let (switched, go) = tokio::sync::oneshot::channel::<()>();
        let client = pair.client.clone();
        let in_flight = tokio::spawn(async move {
            let data = content(4, big);
            let (mut send, mut recv) = client.open_bi().await.map_err(io::Error::other)?;
            let mut header = [0u8; 16];
            header[..8].copy_from_slice(&4u64.to_be_bytes());
            header[8..].copy_from_slice(&(big as u64).to_be_bytes());
            send.write_all(&header).await.map_err(io::Error::other)?;
            let (first, second) = data.split_at(big / 2);
            send.write_all(first).await.map_err(io::Error::other)?;
            let _ = half_sent.send(());
            go.await.map_err(io::Error::other)?;
            send.write_all(second).await.map_err(io::Error::other)?;
            send.finish().map_err(io::Error::other)?;
            let mut ack = [0u8; 40];
            recv.read_exact(&mut ack).await.map_err(io::Error::other)?;
            if ack[8..] != sha(&data) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "acknowledgement",
                ));
            }
            Ok::<(), io::Error>(())
        });
        half.await.expect("first half written");
        tokio::time::timeout(WAIT, async {
            while relay_tx(&pair.client) < before + FILE as u64 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("part of the open stream crossed the relay");
        admit(&pair.client, false).await;
        admit(&pair.host, false).await;
        let relay_at_switch = relay_tx(&pair.client);
        switched.send(()).expect("stream task waits");
        for id in 5..21 {
            let sent = send_file(&pair.client, id, FILE).await;
            sent.expect("file over direct");
        }
        let spanning = in_flight.await.expect("stream task");
        spanning.expect("the stream across the switch completes once");
        let relay_end = relay_tx(&pair.client);
        assert!(relay_end <= relay_at_switch, "new payload over relay");
        {
            let files = pair.ledger.files.lock().unwrap();
            assert_eq!(files.len(), 21);
            assert!(files.values().all(|(_, count)| *count == 1));
            assert_eq!(files[&4].0, sha(&content(4, big)));
            for id in (0..21).filter(|id| *id != 4) {
                assert_eq!(files[&id].0, sha(&content(id, FILE)), "file {id}");
            }
        }
        pair.close().await;
    });
}
