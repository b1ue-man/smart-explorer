//! The long P27 fixture is selected once by the common remote runner, in netns.
#[path = "batches_rate_reference_metrics.rs"]
mod metrics;
#[path = "batches_rate_reference_tasks.rs"]
mod tasks;
#[path = "batches_rate_reference_transport.rs"]
mod transport;

use std::io;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SessionAdmission, SourceKey,
};
use crate::screen::core::capture_plan::CapturePhases;
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::frame_budget::{FrameCapabilities, FrameContract};
use crate::screen::core::gop_stream::{self, now_us, PictureEvent};
use crate::screen::core::host_backend::{ScreenSource, SourceFrame};
use crate::screen::core::host_pipeline::{Pipeline, PipelineActor, Produced, WorkCompletion};
use crate::screen::core::host_stage::{
    HostStage, StageEncoder, StageError, StageFrame, StageRequest,
};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_hello::ScreenErrorCode;
use metrics::{Delivery, Measurement, Observed, Offer, Profile, FPS};
use transport::{Pair, Shared, Tasks};

const SIZE: FrameSize = FrameSize::new(512, 512);
const OUTPUT_BYTES: u64 = super::MAX_BITRATE_BPS / FPS as u64 / 8;

struct SaturatingSource {
    flow: usize,
    grant: CaptureGrant,
    observed: Shared,
    connection: iroh::endpoint::Connection,
}

impl ScreenSource for SaturatingSource {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        Some(self.grant.clone())
    }
    fn supports_frame_regions(&self) -> bool {
        false
    }
    fn capture(&mut self, _: u64, _: bool) -> io::Result<SourceFrame> {
        Err(io::Error::other(
            "P27 synthetic records use the admitted host-stage boundary",
        ))
    }
    fn host_stage(&mut self) -> Option<&mut dyn HostStage> {
        Some(self)
    }
}

impl HostStage for SaturatingSource {
    fn produce(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        let capture_ts_us = now_us();
        let started = Instant::now();
        let length = (request.target_bps / u64::from(request.fps) / 8).max(8);
        let records = CapturedRecords::from_allocator(&self.grant, length, 1, || {
            let mut payload = vec![0u8; length as usize];
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte = (index as u64)
                    .wrapping_mul(33)
                    .wrapping_add(request.frame_id) as u8;
            }
            payload[..8].copy_from_slice(&request.frame_id.to_be_bytes());
            Ok(vec![payload])
        })
        .map_err(|error| StageError::new(ScreenErrorCode::EncoderFailed, error))?;
        // Actual preparation time is measured; one microsecond is the timer's
        // quantisation floor, not an invented encoder capacity or arrival time.
        let prepare_us = started.elapsed().as_micros().max(1) as u64;
        self.observed.lock().unwrap().offers.push(Offer {
            flow: self.flow,
            at_us: capture_ts_us,
            target_bps: request.target_bps,
            bytes: length,
            prepare_us,
            interval_us: 1_000_000 / u64::from(request.fps),
            rtt_us: gop_stream::path_of(&self.connection).1,
        });
        Ok(StageFrame::Encoded {
            records,
            keyframe: request.keyframe,
            capture_ts_us,
            encoder: StageEncoder {
                name: "p27-saturating-admitted-records".to_owned(),
                hardware: false,
            },
            source_size: SIZE,
        })
    }
}

fn source(
    flow: usize,
    observed: Shared,
    contract: FrameContract,
    connection: &iroh::endpoint::Connection,
) -> (Box<dyn ScreenSource>, Arc<SessionAdmission>) {
    let admission = crate::screen::core::capture_test_support::session(Some(contract));
    let key = SourceKey::new(admission.session(), "p27-synthetic-records").unwrap();
    let mut plan = CapturePlan::new(key, CaptureGeneration::new(1));
    plan.bind_contract(Some(contract)).unwrap();
    plan.add_bytes(
        CaptureOwner::CodecOutput,
        OUTPUT_BYTES,
        1,
        CapturePhases::ALL,
    )
    .unwrap();
    plan.add_bytes(
        CaptureOwner::Metadata,
        std::mem::size_of::<Vec<u8>>() as u64,
        1,
        CapturePhases::ALL,
    )
    .unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    (
        Box::new(SaturatingSource {
            flow,
            grant,
            observed,
            connection: connection.clone(),
        }),
        admission,
    )
}

fn contract() -> FrameContract {
    let mut caps = FrameCapabilities::admitted(SIZE, 64 * 1024 * 1024).unwrap();
    caps.hybrid_regions = false;
    FrameContract::negotiate(Some(caps), Some(caps))
        .unwrap()
        .unwrap()
}

fn choice() -> CodecChoice {
    CodecChoice {
        codec: CodecKind::H264,
        profile: Some(H264Profile::ConstrainedBaseline),
        encoder_hardware: false,
        decoder_hardware: false,
        max_size: SIZE,
        max_pixels: SIZE.pixels(),
        max_macroblocks_per_second: None,
    }
}

fn consume_picture(
    flow: usize,
    mut incoming: tokio::sync::mpsc::Receiver<PictureEvent>,
    observed: Shared,
    tasks: &mut Tasks,
) {
    tasks.spawn(async move {
        loop {
            match incoming
                .recv()
                .await
                .ok_or_else(|| io::Error::other("picture reader closed"))?
            {
                PictureEvent::Record {
                    record,
                    received_us,
                    ..
                } => {
                    if record.has_more()
                        || record.payload.len() < 8
                        || record.payload[..8] != record.frame_id.to_be_bytes()
                        || record.capture_ts_us > received_us
                    {
                        return Err(io::Error::other(
                            "synthetic received frame identity or clock differs",
                        ));
                    }
                    for (index, byte) in record.payload.iter().enumerate().skip(8) {
                        if *byte
                            != (index as u64)
                                .wrapping_mul(33)
                                .wrapping_add(record.frame_id) as u8
                        {
                            return Err(io::Error::other("synthetic frame payload differs"));
                        }
                    }
                    observed.lock().unwrap().deliveries.push(Delivery {
                        flow,
                        at_us: received_us,
                        bytes: record.payload.len() as u64,
                        captured_us: Some(record.capture_ts_us),
                    });
                }
                // Normal productive stream resets remain possible. Lost bytes
                // never enter useful throughput or become invented ACK samples.
                PictureEvent::Broken { gop_id } => {
                    observed
                        .lock()
                        .unwrap()
                        .broken_streams
                        .push((flow, gop_id, now_us()));
                }
            }
        }
    });
}

async fn screen(map: &iroh::RelayMap, profile: Profile) -> Measurement {
    let pair = Pair::new(map, profile.windows).await;
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mut tasks = Tasks::default();
    let mut links = Vec::new();
    for (flow, (host, viewer)) in pair.connections.iter().enumerate() {
        links.push(transport::control_link(host, viewer, flow, observed.clone(), &mut tasks).await);
    }
    let mut measured = transport::span(transport::calibrate(&observed).await);
    let contract = contract();
    for (flow, ((host, viewer), mut link)) in pair.connections.iter().zip(links).enumerate() {
        let (events, incoming) = gop_stream::picture_channel();
        let viewer = viewer.clone();
        let outgoing = link.viewer_frames.clone();
        tasks.spawn(async move {
            gop_stream::accept_picture_streams_with_contract(
                viewer,
                events,
                outgoing,
                Some(contract),
            )
            .await;
            Err(io::Error::other(
                "picture stream acceptor stopped during measurement",
            ))
        });
        consume_picture(flow, incoming, observed.clone(), &mut tasks);
        let (source, admission) = source(flow, observed.clone(), contract, host);
        let mut pipeline = Pipeline::new(
            Arc::new(crate::screen::os::synthetic_backend::SyntheticBackend),
            source,
            choice(),
            host,
            FPS,
            Some(SIZE),
            Arc::new(AtomicU32::new(0)),
        );
        pipeline.hold_admission(admission);
        pipeline.set_frame_contract(Some(contract)).unwrap();
        let host = host.clone();
        let mut actor = PipelineActor::new(pipeline, host.clone());
        tasks.spawn(async move {
            let mut poll = tokio::time::interval(Duration::from_millis(2));
            poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut second = tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(1),
                Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = poll.tick() => {
                        actor.poll();
                        if let Some(completion) = actor.take_completion() {
                            match completion {
                                WorkCompletion::Tick(Ok(Produced::Idle | Produced::Dropped | Produced::Sent | Produced::Reset)) => {},
                                WorkCompletion::Tick(Err(failure)) => return Err(failure.error),
                                WorkCompletion::Tick(Ok(outcome)) => return Err(io::Error::other(
                                    format!("productive screen pipeline cannot continue: {outcome:?}"))),
                                WorkCompletion::Snapshot(_) => return Err(io::Error::other("unexpected snapshot in P27 live stream")),
                            }
                        }
                        if let Some(reason) = actor.take_fallback_notice() {return Err(io::Error::other(reason));}
                        actor.begin_tick().map_err(|failure| failure.error)?;
                    }
                    ack = link.acknowledgements.recv() => {
                        let ack = ack.ok_or_else(|| io::Error::other("wire ACK stream closed"))?;
                        actor.on_ack(ack);
                    }
                    _ = second.tick() => {actor.second(&host);}
                }
            }
        });
    }
    transport::until(measured.end_us + 2_000_000).await;
    tasks
        .finish()
        .await
        .expect("screen measurement tasks finish");
    measured.observed = observed.lock().unwrap().clone();
    pair.close().await;
    measured
}

/// No broad `batches_task_` marker: the common runner must select this exact
/// ignored fixture once, through review-task-network.sh, on Linux only.
#[test]
#[ignore = "requires the single remote suite's owned Linux network namespace"]
fn p27_network_reference_acceptance() {
    assert!(
        std::env::var_os("SE_P27_NAMESPACE").is_some(),
        "P27 must run through its namespace helper"
    );
    let candidate = std::env::var("CANDIDATE_SHA").expect("candidate-bound remote suite");
    assert!(candidate.len() == 40 && candidate.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let logs = std::path::PathBuf::from(
        std::env::var_os("SMART_EXPLORER_TASK_LOG_ROOT")
            .expect("common remote-suite artifact directory"),
    )
    .join("p27-network");
    std::fs::create_dir_all(&logs).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(600), async {
            let mut reports = Vec::new();
            for profile in [Profile {label: "10mbit-one-window", bps: 10_000_000, windows: 1},
                Profile {label: "1mbit-one-window", bps: 1_000_000, windows: 1},
                Profile {label: "10mbit-four-windows", bps: 10_000_000, windows: 4}] {
                let (map, _, relay) = iroh::test_utils::run_relay_server_with(false).await.unwrap();
                let port = relay.https_addr().unwrap().port();
                let configured = transport::network("profile", &[port.to_string(), profile.bps.to_string()]);
                assert_eq!(configured["rate_bps"].as_u64(), Some(profile.bps));
                assert_eq!(configured["delay_per_leg_us"], 5000);
                let reference = transport::reference(&map, profile).await;
                let reference_profile = transport::network("snapshot", &[port.to_string(), profile.bps.to_string()]);
                transport::assert_shaped(&reference_profile, &configured);
                std::fs::write(logs.join(format!("{}-reference.json", profile.label)),
                    serde_json::to_vec(&reference).unwrap()).unwrap();
                let measured = screen(&map, profile).await;
                let screen_profile = transport::network("snapshot", &[port.to_string(), profile.bps.to_string()]);
                transport::assert_shaped(&screen_profile, &reference_profile);
                std::fs::write(logs.join(format!("{}-screen.json", profile.label)),
                    serde_json::to_vec(&measured).unwrap()).unwrap();
                reports.push(serde_json::json!({"acceptance": metrics::evaluate(profile, &reference, &measured),
                    "configured_profile": configured, "reference_profile": reference_profile,
                    "screen_profile": screen_profile}));
                drop(relay);
            }
            let passed = reports.iter().all(|report| report["acceptance"]["result"] == "passed");
            let report = serde_json::json!({"schema": 1, "candidate": candidate,
                "fixture": "p27_network_reference_acceptance", "result": if passed {"passed"} else {"failed"},
                "profiles": reports,
                "scope": "Private pinned Iroh relay; actual productive screen pipeline with admitted synthetic records; no physical encoder, display, input injection or public relay."});
            std::fs::write(logs.join("acceptance.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
            println!("SE_P27_ACCEPTANCE_JSON={report}");
            assert!(passed, "V7 P27 acceptance failed; retain p27-network raw observations and acceptance.json");
        }).await.expect("bounded P27 transport measurement; no duplicate test invocation");
    });
}
