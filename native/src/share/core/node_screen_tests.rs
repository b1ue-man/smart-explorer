//! Screen sessions between two real Iroh nodes on loopback (SCR-M2): the
//! measurement with the test picture, recovery after picture streams the
//! host gave up, the indicator rule, an older device without the screen
//! ALPN, and FS and Exec on the same node. The S1 rights, confirmations,
//! ends by a narrowed or ended right and the access log are in `rights`.
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::VarInt;
use serde_json::Value;

use super::*;
use crate::screen::core::bench::{self, BenchOptions, BenchReport};
use crate::screen::core::host_backend::{SessionIndicator, SourceRefusal};
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::negotiate::{CodecChoice, CodecOffer, CodecPreference};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::viewer_client::{handshake, ViewerError, ViewerRequest, ViewerSession};
use crate::screen::core::window_list::{MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::{ControlBlock, HostFrame, StateReport};
use crate::screen::core::wire_hello::{ApprovalKind, HostPlatform, SessionMode};
use crate::share::access_approval::ApprovalDecision;
use crate::share::access_grant::CapabilityDecision::{Allow, Ask, Off};
use crate::share::relation_rights::authorization_restrictions;

#[path = "node_screen_task_fixture.rs"]
mod fixture;
use fixture::{screen_grant, ScreenPair};

#[path = "node_screen_rights_tests.rs"]
mod rights;

#[path = "node_screen_codec_tests.rs"]
mod codec;

#[path = "node_screen_still_tests.rs"]
mod still;

static AUDIT: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn capture(entry: crate::share::access_audit::AccessEntry<'_>) {
    let line = crate::share::access_audit::access_line(&entry);
    AUDIT.lock().unwrap().push(line);
}

fn audit_of(device_id: &str) -> Vec<Value> {
    let lines = AUDIT.lock().unwrap();
    lines
        .iter()
        .filter(|line| line["device_id"] == device_id)
        .cloned()
        .collect()
}

fn request(mode: SessionMode, source: SourceId, size: FrameSize, fps: u16) -> ViewerRequest {
    ViewerRequest {
        frame_capabilities: None,
        mode,
        source: Some(source),
        decoders: vec![CodecOffer::tiles()],
        display_size: Some(size),
        max_fps: fps,
    }
}

fn test_picture(mode: SessionMode) -> ViewerRequest {
    request(mode, SourceId::Synthetic, FrameSize::new(640, 360), 30)
}

fn options(seconds: u32) -> BenchOptions {
    BenchOptions {
        codec: CodecPreference::Tiles,
        size: FrameSize::new(640, 360),
        fps: 30,
        seconds,
    }
}

fn refused(pair: &ScreenPair, request: &ViewerRequest) -> String {
    match pair.open(request, |_| {}) {
        Err(ViewerError::Refused(code)) => code,
        Err(other) => panic!("expected a refusal, got {other:?}"),
        Ok(_) => panic!("expected a refusal, the session started"),
    }
}

fn bench_on(pair: &ScreenPair, seconds: u32) -> BenchReport {
    let session = pair.open(&test_picture(SessionMode::View), |_| {}).unwrap();
    pair.viewer
        .0
        .block_on(bench::run(
            session,
            options(seconds),
            &crate::screen::core::codec_registry::CodecRegistry::builtin(),
        ))
        .unwrap()
}

/// Waits for the host's `STATE` of `state`, reading pictures meanwhile.
fn wait_state(
    pair: &ScreenPair,
    session: &mut ViewerSession,
    state: SessionState,
    within: Duration,
) -> Option<StateReport> {
    pair.viewer.0.block_on(async {
        let wait = async {
            loop {
                tokio::select! {
                    frame = session.host.recv() => match frame {
                        Some(Ok(HostFrame::State(report))) if report.state == state => {
                            return Some(report);
                        }
                        Some(Ok(_)) => {}
                        _ => return None,
                    },
                    _ = session.pictures.recv() => {}
                }
            }
        };
        tokio::time::timeout(within, wait).await.ok().flatten()
    })
}

#[test]
fn screen_task_pair_bench_tiles() {
    let pair = ScreenPair::new(Allow, Off);
    let report = bench_on(&pair, 3);
    assert_eq!(report.codec, "tiles");
    assert_eq!(report.encoder, "tiles");
    assert_eq!(report.path, "direct");
    assert_eq!(report.corrupt_frames, 0, "{report:?}");
    assert!(report.frames_shown >= 30, "{report:?}");
    assert!(report.keyframes >= 1, "{report:?}");
    assert_eq!(report.static_psnr_db, None, "lossless: infinite PSNR");
    assert!(report.latency_ms.p95 < 1000.0, "{report:?}");
    // The session ended with the measurement and left the registry.
    let registry = pair.host.0.screen_registry().clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !registry.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(registry.is_empty());
}

#[test]
fn screen_task_recovery_after_injected_reset() {
    let pair = ScreenPair::new(Allow, Off);
    let resets = pair.host.0.screen.inject_resets.clone();
    let injector = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1200));
        resets.store(1, Ordering::Release);
    });
    let report = bench_on(&pair, 4);
    injector.join().unwrap();
    assert_eq!(report.corrupt_frames, 0, "{report:?}");
    assert!(!report.recoveries.is_empty(), "{report:?}");
    assert!(
        report.recoveries.iter().all(|ms| *ms <= 1000.0),
        "{report:?}"
    );
    assert!(
        report.keyframes >= 2,
        "a key frame after the reset: {report:?}"
    );
}

#[test]
fn screen_task_reset_streams_free_slots() {
    // Six resets in a row: with four uni-stream slots the viewer must stop
    // each broken stream, or no picture stream after the fourth could open.
    let pair = ScreenPair::new(Allow, Off);
    let resets = pair.host.0.screen.inject_resets.clone();
    let injector = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        resets.store(6, Ordering::Release);
    });
    let report = bench_on(&pair, 5);
    injector.join().unwrap();
    assert_eq!(report.corrupt_frames, 0, "{report:?}");
    assert!(
        report.keyframes >= 2,
        "key frames after the resets: {report:?}"
    );
    // About 20 frames arrive before the resets; most of the 4 seconds after
    // them must still deliver pictures.
    assert!(report.frames_shown >= 60, "{report:?}");
}

/// The test picture with a graphical session, for the indicator rule.
struct Desktop;

impl ScreenHostBackend for Desktop {
    fn prepare_session_source(&self, owner:&str, session:&str, source:&SourceId,
        wanted:Option<FrameSize>, control:bool,
        admission:Arc<crate::screen::core::capture_admission::SessionAdmission>)
        ->Result<Box<dyn crate::screen::core::host_backend::PreparedSource>,SourceRefusal>
    {
        SyntheticBackend.prepare_session_source(owner,session,source,wanted,control,admission)
    }
    fn open_encoder_for_stage_admitted(&self, request:&crate::screen::core::host_stage::StageRequest,
        grant:crate::screen::core::capture_admission::CaptureGrant)
        ->io::Result<Box<dyn crate::screen::core::host_backend::ScreenEncoder>>
    {
        SyntheticBackend.open_encoder_for_stage_admitted(request,grant)
    }

    fn platform(&self) -> HostPlatform {
        HostPlatform::Linux
    }
    fn graphical_session(&self) -> bool {
        true
    }
    fn monitors(&self) -> Vec<MonitorInfo> {
        vec![MonitorInfo {
            id: "1".into(),
            name: "Test".into(),
            x: 0,
            y: 0,
            width: 640,
            height: 360,
            primary: true,
        }]
    }
    fn encoders(&self) -> Vec<CodecOffer> {
        SyntheticBackend.encoders()
    }
    fn input(&self) -> InputCapabilities {
        InputCapabilities::default()
    }
    fn limits(&self) -> Vec<String> {
        Vec::new()
    }
    fn windows(&self) -> io::Result<WindowList> {
        SyntheticBackend.windows()
    }
    fn open_source(
        &self,
        source: &SourceId,
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn crate::screen::core::host_backend::ScreenSource>, SourceRefusal> {
        SyntheticBackend.open_source(source, wanted)
    }
    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn crate::screen::core::host_backend::ScreenEncoder>> {
        SyntheticBackend.open_encoder(choice, size)
    }
    fn inject(&self, _events: &[InputEvent]) -> Result<(), ControlBlock> {
        Err(ControlBlock::UnsupportedOnHost)
    }
}

struct Indicator {
    available: bool,
}

impl SessionIndicator for Indicator {
    fn available(&self) -> bool {
        self.available
    }
    fn show(&self) -> Result<(), String> {
        if self.available {
            Ok(())
        } else {
            Err("no desktop app".into())
        }
    }
}

#[test]
fn screen_task_indicator_unavailable_refuses() {
    let pair = ScreenPair::new(Allow, Off);
    *pair.host.0.screen.backend.write().unwrap() = Arc::new(Desktop);
    *pair.host.0.screen.indicator.write().unwrap() = Arc::new(Indicator { available: false });
    let monitor = request(
        SessionMode::View,
        SourceId::Monitor("1".into()),
        FrameSize::new(640, 360),
        30,
    );
    assert_eq!(refused(&pair, &monitor), "indicator_unavailable");
    let mut primary = monitor.clone();
    primary.source = None;
    assert_eq!(refused(&pair, &primary), "indicator_unavailable");
    // The test picture shows no screen content and needs no indicator.
    let synthetic = pair.open(&test_picture(SessionMode::View), |_| {}).unwrap();
    synthetic.close();
    // With an indicator the monitor passes the indicator rule; this host has
    // no capture yet (`no_display`).
    *pair.host.0.screen.indicator.write().unwrap() = Arc::new(Indicator { available: true });
    assert_eq!(refused(&pair, &monitor), "no_display");
}

#[test]
fn screen_task_old_peer_unsupported() {
    let pair = ScreenPair::new(Allow, Off);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    // An older device: the FS, Exec and LAN-link ALPNs, no screen.
    let secret = iroh::SecretKey::from_bytes(&[42u8; 32]);
    let old = runtime
        .block_on(async {
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .secret_key(secret.clone())
                .alpns(vec![
                    super::super::node::ALPN.to_vec(),
                    super::super::exec_protocol::EXEC_ALPN.to_vec(),
                    super::super::lan_link_wire::LAN_LINK_ALPN.to_vec(),
                ])
                .relay_mode(iroh::RelayMode::Disabled)
                .bind()
                .await
        })
        .unwrap();
    // Like the node's accept loop: the handshake (and with it the ALPN
    // refusal) only runs for a connection that is accepted and awaited. A
    // connection that completes (the viewer's file-session probe, when the
    // refusal of the screen ALPN is lost) stays open until the viewer closes
    // it.
    let acceptor = old.clone();
    runtime.spawn(async move {
        while let Some(incoming) = acceptor.accept().await {
            tokio::spawn(async move {
                if let Ok(connection) = incoming.await {
                    connection.closed().await;
                }
            });
        }
    });
    let port = old
        .bound_sockets()
        .into_iter()
        .find(|socket| socket.is_ipv4())
        .map(|socket| socket.port())
        .unwrap();
    let mut endpoint = pair.endpoint.clone();
    let node_id = secret.public().to_string();
    endpoint.presence.node_id = node_id.clone();
    endpoint.presence.public_key = node_id.clone();
    endpoint.presence.candidates = vec![format!("127.0.0.1:{port}")];
    endpoint.expected_node_id = Some(node_id);
    let connected = pair
        .viewer
        .0
        .block_on(pair.viewer.0.connect_screen(&endpoint));
    assert!(
        matches!(connected, Err(ViewerError::Unsupported)),
        "{:?}",
        connected.as_ref().err()
    );
    assert_eq!(ViewerError::Unsupported.code(), "screen_unsupported");
    runtime.block_on(old.close());
}

#[test]
fn screen_task_fs_exec_unchanged() {
    let pair = ScreenPair::new(Allow, Off);
    // Files: the export lists as before.
    let peer = super::super::backend::PeerBackend::new(
        pair.endpoint.clone(),
        pair.viewer_identity.clone(),
        pair.viewer.0.clone(),
    );
    let backend: crate::vfs::BackendHandle = Arc::new(peer);
    let entries = backend.list_dir("/").unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry.name == "Docs" && entry.is_dir));
    // Exec: the host still opens the Exec handshake with its hello.
    let connection = pair.viewer.0.connect_exec(&pair.endpoint).unwrap();
    let hello = pair.viewer.0.block_on(async {
        let (_send, mut recv) = connection.accept_bi().await.map_err(io::Error::other)?;
        super::super::exec_protocol::recv_server_hello(&mut recv).await
    });
    let hello = hello.unwrap();
    assert_eq!(hello.server_node_id, pair.endpoint.presence.node_id);
    connection.close(VarInt::from_u32(0), b"done");
    // Screen on the same node and the same connection pool.
    let probe = pair
        .open(&test_picture(SessionMode::Probe), |_| {})
        .unwrap();
    assert_eq!(probe.hello.encoders, vec![CodecOffer::tiles()]);
    assert_eq!(probe.hello.limits, ["test_picture_only"]);
    probe.close();
    let entries = backend.list_dir("/Docs").unwrap();
    assert!(entries.iter().any(|entry| entry.name == "note.txt"));
}

/// Over the share server's relay only (`SE_SHARE_RELAY_ONLY=1`, suite
/// fixture `share`).
#[test]
#[ignore = "needs the share-server fixture and SE_SHARE_RELAY_ONLY=1"]
fn screen_task_relay_only_bench() {
    let signal = std::env::var("SE_TASK_SHARE_SIGNAL").expect("SE_TASK_SHARE_SIGNAL");
    assert_eq!(std::env::var("SE_SHARE_RELAY_ONLY").as_deref(), Ok("1"));
    let server = format!("tcp://{signal}");
    let pair = ScreenPair::with_server(&server, Allow, Off, None).unwrap();
    let report = bench_on(&pair, 5);
    assert_eq!(report.path, "relay", "{report:?}");
    assert_eq!(report.corrupt_frames, 0, "{report:?}");
    assert!(report.frames_shown >= 30, "{report:?}");
}

/// With `tc netem` on `lo` (2 % loss, 12.5 ms each way), set up by the suite.
#[test]
#[ignore = "needs tc netem on lo; the suite sets SE_SCREEN_TASK_NETEM=1"]
fn screen_task_netem_recovery() {
    assert_eq!(
        std::env::var("SE_SCREEN_TASK_NETEM").as_deref(),
        Ok("1"),
        "run by the suite after `tc qdisc add dev lo root netem loss 2% delay 12.5ms`"
    );
    let pair = ScreenPair::new(Allow, Off);
    let report = bench_on(&pair, 10);
    assert_eq!(report.corrupt_frames, 0, "{report:?}");
    assert!(
        report.recoveries.iter().all(|ms| *ms <= 1000.0),
        "recovery within one second: {report:?}"
    );
    assert!(report.frames_shown >= 150, "{report:?}");
}
