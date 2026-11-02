//! Remote task suite only: real private IPv4, pinned TLS, no NAT/ICS mutation.
use super::super::lan_link_wire::{self, LinkFrame, MAX_LINK_FRAME};
use super::*;
use iroh::endpoint::{RecvStream, SendStream};
use tokio::io::AsyncWriteExt;

#[path = "lan_link_transport_task_fixture.rs"]
mod fixture;
use fixture::{accepted_contact, assert_status_only, fixture_identity, Fixture};

#[path = "lan_link_transport_task_lifecycle_tests.rs"]
mod lifecycle;

const OBSERVE: Duration = Duration::from_secs(4);

async fn wait_until(label: &str, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(OBSERVE, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("bounded observation failed: {label}"));
}
async fn send_status_frame(send: &mut SendStream, frame: &LinkFrame) {
    tokio::time::timeout(ROUND_DEADLINE, async {
        let bytes = lan_link_wire::encode(frame).unwrap();
        send.write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .unwrap();
        send.write_all(&bytes).await.unwrap();
        send.flush().await.unwrap();
    })
    .await
    .expect("bounded status frame send");
}
async fn receive_status_frame(recv: &mut RecvStream) -> LinkFrame {
    let mut length = [0; 4];
    recv.read_exact(&mut length).await.unwrap();
    let length = u32::from_be_bytes(length) as usize;
    assert!(length > 0 && length <= MAX_LINK_FRAME);
    let mut bytes = vec![0; length];
    recv.read_exact(&mut bytes).await.unwrap();
    lan_link_wire::decode(&bytes).unwrap()
}
async fn raw_challenge(
    connection: &Connection,
    identity: DirectPeerIdentity,
    nonce: [u8; 32],
) -> (SendStream, [u8; 32]) {
    tokio::time::timeout(ROUND_DEADLINE, async {
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send_status_frame(&mut send, &LinkFrame::Challenge { nonce, identity }).await;
        match receive_status_frame(&mut recv).await {
            LinkFrame::Answer {
                echo,
                nonce: remote_nonce,
                ..
            } => {
                lan_link_wire::check_echo(&nonce, &echo).unwrap();
                (send, remote_nonce)
            }
            _ => panic!("expected status Answer"),
        }
    })
    .await
    .expect("bounded raw status round")
}

#[test]
fn review_task_s09_transport_real_private_tls_round_has_only_status_rights() {
    let mut fixture = Fixture::start(10);
    let rt = fixture.a.rt.clone();
    rt.block_on(async {
        let connection = fixture.connect().await;
        fixture.start_rounds(&connection, fixture.expected_pin());
        let (a, b) = fixture.wait_status().await;
        assert_eq!(a.pin, fixture.expected_pin());
        assert_eq!(
            b.pin,
            lan_link_facts::grant_pin(&fixture.b.auth.lock().unwrap().direct_grants[0]).unwrap()
        );
        assert!(!a.peer_uplink && b.peer_uplink);
        assert_ne!(
            a.challenge, b.challenge,
            "both peers need their own fresh random challenge"
        );
        for fact in [&a, &b] {
            assert!(fact.fresh(now_secs()));
            assert_eq!(fact.interface.local_ip, fixture.lan.ip);
            assert_eq!(fact.interface.adapter_id, fixture.lan.interface.adapter_id);
            assert_eq!(fact.expires_at - fact.confirmed_at, MAX_FACT_LIFETIME_SECS);
        }
        assert_status_only(&fixture.a, &connection, fixture.baseline[0]);
        assert_status_only(
            &fixture.b,
            &fixture.server_connection(),
            fixture.baseline[1],
        );
    });
}

#[test]
fn review_task_s09_transport_rejects_wrong_pin_and_replayed_confirm() {
    let mut fixture = Fixture::start(20);
    let rt = fixture.a.rt.clone();
    rt.block_on(async {
        let connection = fixture.connect().await;
        fixture.a.auth.lock().unwrap().direct_contacts[0] = accepted_contact(&fixture_identity(22));
        fixture.start_rounds(&connection, fixture.expected_pin());
        let result = tokio::time::timeout(OBSERVE, fixture.rounds.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.is_err(),
            "actual TLS peer must match the saved full pin"
        );
        assert!(fixture.a.lan_link_snapshot().is_empty());
        assert!(fixture.b.lan_link_snapshot().is_empty());
        assert_status_only(&fixture.a, &connection, fixture.baseline[0]);
        assert_status_only(
            &fixture.b,
            &fixture.server_connection(),
            fixture.baseline[1],
        );
        close(&connection);
    });
    drop(fixture);
    drop(rt);
    let mut fixture = Fixture::start(24);
    let rt = fixture.a.rt.clone();
    rt.block_on(async {
        let (connection, _) = fixture.positive().await;
        let identity = local_identity(&fixture.a).unwrap();
        let (mut send, first_nonce) = raw_challenge(&connection, identity.clone(), [1; 32]).await;
        send_status_frame(
            &mut send,
            &LinkFrame::Confirm {
                echo: first_nonce,
                uplink: OwnUplink::Present,
            },
        )
        .await;
        send.finish().unwrap();
        wait_until("first real Confirm", || {
            fixture
                .b
                .lan_link_snapshot()
                .iter()
                .any(|fact| fact.challenge == first_nonce)
        })
        .await;
        let server = fixture.server_connection();
        let (mut send, next_nonce) = raw_challenge(&connection, identity, [2; 32]).await;
        assert_ne!(first_nonce, next_nonce);
        send_status_frame(
            &mut send,
            &LinkFrame::Confirm {
                echo: first_nonce,
                uplink: OwnUplink::Absent,
            },
        )
        .await;
        send.finish().unwrap();
        wait_until("replayed Confirm closes channel", || {
            connection.close_reason().is_some()
        })
        .await;
        assert!(fixture.b.lan_link_snapshot().is_empty());
        assert_status_only(&fixture.b, &server, fixture.baseline[1]);
    });
    drop(fixture);
    drop(rt);
    let fixture = Fixture::start(26);
    let rt = fixture.a.rt.clone();
    rt.block_on(async {
        let connection = fixture.connect().await;
        wait_until("server admitted pinned TLS", || {
            !fixture
                .b
                .lan_links
                .state
                .lock()
                .unwrap()
                .channels
                .is_empty()
        })
        .await;
        let server = fixture.server_connection();
        let mut identity = local_identity(&fixture.a).unwrap();
        identity.device_id = "unaccepted-device-alias".into();
        tokio::time::timeout(ROUND_DEADLINE, async {
            let (mut send, _recv) = connection.open_bi().await.unwrap();
            send_status_frame(
                &mut send,
                &LinkFrame::Challenge {
                    nonce: [3; 32],
                    identity,
                },
            )
            .await;
            wait_until(
                "wrong device alias rejected despite correct TLS key",
                || connection.close_reason().is_some(),
            )
            .await;
        })
        .await
        .expect("bounded wrong identity round");
        assert!(fixture.b.lan_link_snapshot().is_empty());
        assert_status_only(&fixture.b, &server, fixture.baseline[1]);
    });
}

#[test]
fn review_task_s09_transport_malformed_and_stalled_frames_are_bounded() {
    for mode in 0..4 {
        let fixture = Fixture::start(80 + mode * 2);
        let rt = fixture.a.rt.clone();
        rt.block_on(async {
            let connection = fixture.connect().await;
            wait_until("server admitted status ALPN", || {
                !fixture
                    .b
                    .lan_links
                    .state
                    .lock()
                    .unwrap()
                    .channels
                    .is_empty()
            })
            .await;
            let server = fixture.server_connection();
            let payload: &[u8] = if mode == 0 {
                br#"{"type":"Exec","argv":["never-run"]}"#
            } else {
                br#"{"type":"Write","path":"/never-created","data":"not-written"}"#
            };
            let (_send, _recv) = tokio::time::timeout(ROUND_DEADLINE, async {
                let (mut send, recv) = connection.open_bi().await.unwrap();
                match mode {
                    0 | 1 => {
                        send.write_all(&(payload.len() as u32).to_be_bytes())
                            .await
                            .unwrap();
                        send.write_all(payload).await.unwrap();
                    }
                    2 => send
                        .write_all(&((MAX_LINK_FRAME + 1) as u32).to_be_bytes())
                        .await
                        .unwrap(),
                    _ => send.write_all(&[0, 0]).await.unwrap(), // retain unfinished length until absolute timeout
                }
                send.flush().await.unwrap();
                (send, recv)
            })
            .await
            .expect("bounded malformed/stalled send");
            wait_until("malformed/stalled round ended", || {
                connection.close_reason().is_some()
            })
            .await;
            assert!(fixture.b.lan_link_snapshot().is_empty());
            assert_status_only(&fixture.a, &connection, fixture.baseline[0]);
            assert_status_only(&fixture.b, &server, fixture.baseline[1]);
        });
    }
}
