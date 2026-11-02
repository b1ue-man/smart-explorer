//! Private real Iroh relay, production control framing and encoder-free baseline.
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, RelayMap, RelayMode};
use serde_json::Value;
use tokio::sync::mpsc;

use super::metrics::{
    Control, Delivery, Measurement, Observed, ProbeSent, Profile, FRAME_US, STATIONARY_US,
    WARMUP_US,
};
use crate::screen::core::gop_stream::{now_us, path_of};
use crate::screen::core::wire_control::{self, FrameAck, HostFrame, Ping, Pong, ViewerFrame};

pub(super) use super::tasks::Tasks;

pub(super) type Shared = Arc<Mutex<Observed>>;
const ALPN: &[u8] = b"smart-explorer-p27-private-measurement/1";

pub(super) fn network(mode: &str, args: &[String]) -> Value {
    let helper = std::env::var_os("SE_P27_NETWORK_HELPER")
        .expect("the single remote suite must invoke the owned namespace helper");
    let output = std::process::Command::new("bash")
        .arg(helper)
        .arg(mode)
        .args(args)
        .output()
        .expect("owned namespace helper starts");
    assert!(
        output.status.success(),
        "network profile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("network profile JSON");
    let namespace = std::fs::read_link("/proc/self/ns/net").unwrap();
    assert_eq!(value["namespace"].as_str(), namespace.to_str());
    assert_ne!(value["namespace"], value["parent_namespace"]);
    assert_eq!(value["links"].as_array().unwrap().len(), 1);
    assert_eq!(value["links"][0]["ifname"], "lo");
    value
}

pub(super) fn assert_shaped(snapshot: &Value, before: &Value) {
    for handle in ["10:", "20:"] {
        let queue = snapshot["qdiscs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|queue| queue["handle"] == handle)
            .expect("both relay legs remain shaped");
        let previous = before["qdiscs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|queue| queue["handle"] == handle)
            .expect("same profile before measured IO");
        assert!(
            queue["packets"].as_u64().unwrap_or(0) > previous["packets"].as_u64().unwrap_or(0),
            "actual relay traffic must traverse netem {handle}: {queue}"
        );
    }
}

pub(super) struct Pair {
    host_endpoint: Endpoint,
    viewer_endpoint: Endpoint,
    pub connections: Vec<(Connection, Connection)>,
}

impl Pair {
    pub async fn new(map: &RelayMap, windows: usize) -> Self {
        async fn endpoint(map: &RelayMap) -> Endpoint {
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .alpns(vec![ALPN.to_vec()])
                .clear_address_lookup()
                .clear_ip_transports()
                .relay_mode(RelayMode::Custom(map.clone()))
                // The private test relay uses a new self-signed certificate.
                // This builder exists only in this cfg(test, linux) fixture.
                .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
                .bind()
                .await
                .expect("private relay-only endpoint")
        }
        let host_endpoint = endpoint(map).await;
        let viewer_endpoint = endpoint(map).await;
        tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(host_endpoint.online(), viewer_endpoint.online());
        })
        .await
        .expect("both endpoints register at the owned private relay");
        let mut connections = Vec::new();
        for _ in 0..windows {
            let (host, viewer) = tokio::time::timeout(Duration::from_secs(30), async {
                tokio::join!(host_endpoint.connect(viewer_endpoint.addr(), ALPN), async {
                    viewer_endpoint
                        .accept()
                        .await
                        .expect("private accept remains open")
                        .await
                })
            })
            .await
            .expect("private relay TLS connection deadline");
            let (host, viewer) = (host.unwrap(), viewer.unwrap());
            for connection in [&host, &viewer] {
                assert!(connection
                    .paths()
                    .iter()
                    .any(|path| path.is_selected() && path.is_relay()));
                assert!(!connection.paths().iter().any(|path| path.is_ip()));
                assert_eq!(path_of(connection).0, super::super::PathKind::Relay);
            }
            assert_eq!(host.remote_id(), viewer_endpoint.id());
            assert!(
                connections
                    .iter()
                    .all(|(prior, _): &(Connection, Connection)| prior.stable_id()
                        != host.stable_id()),
                "windows need distinct screen connections"
            );
            connections.push((host, viewer));
        }
        Self {
            host_endpoint,
            viewer_endpoint,
            connections,
        }
    }

    pub async fn close(self) {
        for (host, viewer) in &self.connections {
            host.close(0u32.into(), b"P27 measurement complete");
            viewer.close(0u32.into(), b"P27 measurement complete");
        }
        tokio::join!(self.host_endpoint.close(), self.viewer_endpoint.close());
    }
}

pub(super) struct ControlLink {
    pub acknowledgements: mpsc::Receiver<FrameAck>,
    pub viewer_frames: mpsc::Sender<ViewerFrame>,
}

pub(super) async fn control_link(
    host: &Connection,
    viewer: &Connection,
    flow: usize,
    observations: Shared,
    tasks: &mut Tasks,
) -> ControlLink {
    let peer = host.remote_id().to_string();
    let (mut host_send, mut host_recv) = host.open_bi().await.unwrap();
    crate::screen::core::control_sender::send(
        peer.clone(),
        &mut host_send,
        &HostFrame::Pong(Pong {
            sequence: 0,
            ping_sent_us: 0,
            received_us: 0,
            sent_us: now_us(),
        }),
    )
    .await
    .unwrap();
    let (mut viewer_send, mut viewer_recv) = viewer.accept_bi().await.unwrap();
    assert!(matches!(
        wire_control::recv_host_frame(&mut viewer_recv)
            .await
            .unwrap(),
        HostFrame::Pong(_)
    ));
    let (ack_tx, acknowledgements) = mpsc::channel(64);
    let (viewer_frames, mut outbound) = mpsc::channel(64);
    tasks.spawn(async move {
        loop {
            match wire_control::recv_viewer_frame(&mut host_recv).await? {
                ViewerFrame::FrameAck(ack) | ViewerFrame::PreviewAck(ack) => {
                    ack_tx
                        .send(ack)
                        .await
                        .map_err(|_| io::Error::other("pipeline ACK receiver closed"))?;
                }
                ViewerFrame::Ping(ping) => {
                    let received = now_us();
                    crate::screen::core::control_sender::send(
                        peer.clone(),
                        &mut host_send,
                        &HostFrame::Pong(Pong::answer(&ping, received, now_us())),
                    )
                    .await?;
                }
                _ => {
                    return Err(io::Error::other(
                        "unexpected measurement viewer control frame",
                    ))
                }
            }
        }
    });
    let sent = observations.clone();
    tasks.spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_micros(FRAME_US));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sequence = 0;
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    sequence += 1;
                    let at_us = now_us();
                    sent.lock().unwrap().probes_sent.push(ProbeSent {flow, sequence, at_us});
                    wire_control::send_viewer_frame(&mut viewer_send,
                        &ViewerFrame::Ping(Ping {sequence, sent_us: at_us})).await?;
                }
                frame = outbound.recv() => {
                    let frame = frame.ok_or_else(|| io::Error::other("picture ACK source closed"))?;
                    wire_control::send_viewer_frame(&mut viewer_send, &frame).await?;
                }
            }
        }
    });
    tasks.spawn(async move {
        loop {
            match wire_control::recv_host_frame(&mut viewer_recv).await? {
                HostFrame::Pong(pong) => observations.lock().unwrap().controls.push(Control {
                    flow,
                    sequence: pong.sequence,
                    sent_us: pong.ping_sent_us,
                    received_us: now_us(),
                }),
                _ => {
                    return Err(io::Error::other(
                        "unexpected measurement host control frame",
                    ))
                }
            }
        }
    });
    ControlLink {
        acknowledgements,
        viewer_frames,
    }
}

pub(super) async fn calibrate(observed: &Shared) -> u64 {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let samples = observed.lock().unwrap();
    assert!(
        samples.controls.len() >= 15,
        "unloaded actual control RTT needs completed probes"
    );
    samples
        .controls
        .iter()
        .map(|sample| {
            sample
                .received_us
                .checked_sub(sample.sent_us)
                .expect("measurement clock remains monotone")
        })
        .min()
        .unwrap()
}

pub(super) fn span(idle_rtt_us: u64) -> Measurement {
    let start_us = now_us();
    Measurement {
        start_us,
        stationary_start_us: start_us + WARMUP_US,
        end_us: start_us + WARMUP_US + STATIONARY_US,
        idle_rtt_us,
        observed: Observed::default(),
    }
}

pub(super) async fn until(deadline_us: u64) {
    let remaining = deadline_us
        .checked_sub(now_us())
        .expect("measurement missed its fixed deadline");
    tokio::time::sleep(Duration::from_micros(remaining)).await;
}

pub(super) async fn reference(map: &RelayMap, profile: Profile) -> Measurement {
    let pair = Pair::new(map, profile.windows).await;
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mut tasks = Tasks::default();
    let mut links = Vec::new();
    for (flow, (host, viewer)) in pair.connections.iter().enumerate() {
        links.push(control_link(host, viewer, flow, observed.clone(), &mut tasks).await);
    }
    let mut measured = span(calibrate(&observed).await);
    for (flow, (host, viewer)) in pair.connections.iter().enumerate() {
        let host = host.clone();
        let viewer = viewer.clone();
        tasks.spawn(async move {
            let mut stream = host.open_uni().await.map_err(io::Error::other)?;
            let payload: Vec<_> = (0..16 * 1024).map(|index| index as u8).collect();
            loop {
                stream.write_all(&payload).await.map_err(io::Error::other)?;
            }
        });
        let received = observed.clone();
        tasks.spawn(async move {
            let mut stream = viewer.accept_uni().await.map_err(io::Error::other)?;
            let mut bytes = [0u8; 16 * 1024];
            let mut offset = 0usize;
            loop {
                let count = stream
                    .read(&mut bytes)
                    .await
                    .map_err(io::Error::other)?
                    .ok_or_else(|| {
                        io::Error::other("reference ended before its measurement window")
                    })?;
                for (index, byte) in bytes[..count].iter().enumerate() {
                    if *byte != (offset + index) as u8 {
                        return Err(io::Error::other("reference payload differs"));
                    }
                }
                offset = (offset + count) % (16 * 1024);
                received.lock().unwrap().deliveries.push(Delivery {
                    flow,
                    at_us: now_us(),
                    bytes: count as u64,
                    captured_us: None,
                });
            }
        });
    }
    until(measured.end_us + 2_000_000).await;
    tasks
        .finish()
        .await
        .expect("reference measurement tasks finish");
    measured.observed = observed.lock().unwrap().clone();
    drop(links);
    pair.close().await;
    measured
}
