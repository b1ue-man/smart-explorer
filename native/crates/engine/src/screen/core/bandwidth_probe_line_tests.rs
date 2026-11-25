//! The probe and the „Übertragung“ over a real loopback connection.
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;

use super::*;
use crate::screen::core::gop_stream::{
    accept_picture_streams_with_contract, GopWriter, PictureEvent,
};
use crate::screen::core::host_backend::{ScreenSource, SourceFrame};
use crate::screen::core::host_pipeline::{Pipeline, PipelineActor};
use crate::screen::core::negotiate::{
    negotiate_for_contract, CodecKind, CodecOffer, CodecPreference,
};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::screen_metadata_runtime_fixtures as fixtures;
use crate::screen::core::wire_binary::{GopHeader, StreamKind};
use crate::screen::core::wire_control::ViewerFrame;
use crate::screen::core::wire_hello::tests::client_hello;

const DEADLINE: Duration = Duration::from_secs(20);

/// A source that never delivers: the session's pictures play no part.
struct Still;

impl ScreenSource for Still {
    fn size(&self) -> FrameSize {
        FrameSize::new(64, 64)
    }
    fn capture(&mut self, _frame_id: u64, _keyframe: bool) -> std::io::Result<SourceFrame> {
        Err(std::io::Error::other("no picture in this test"))
    }
}

fn pipeline(connection: &Connection) -> Pipeline {
    let tiles = [CodecOffer::tiles()];
    let choice = negotiate_for_contract(&tiles, &tiles, CodecPreference::Auto, None)[0];
    Pipeline::new(
        Arc::new(crate::screen::os::synthetic_backend::SyntheticBackend),
        Box::new(Still),
        choice,
        connection,
        30,
        None,
        Arc::new(AtomicU32::new(0)),
    )
}

/// The viewer's acceptor tells the probe stream from a picture stream: the
/// picture group beside the probe arrives and is acknowledged, the probe is
/// counted (pictures included) and answered with its id and a rate.
#[test]
fn probe_task_loopback_probe_is_answered_beside_pictures() {
    fixtures::run(true, async {
        tokio::time::timeout(DEADLINE, async {
            let link = fixtures::link().await;
            let (events, mut pictures) = tokio::sync::mpsc::channel(16);
            let (acks, mut answers) = tokio::sync::mpsc::channel(64);
            let reader = tokio::spawn(accept_picture_streams_with_contract(
                link.viewer.clone(),
                events,
                acks,
                None,
            ));
            let head = ProbeHead {
                probe_id: 5,
                duration_ms: 300,
            };
            let probe = tokio::spawn(send_probe(link.host.clone(), head));
            let header = GopHeader {
                gop_id: 1,
                frame_contract: None,
                family_layout_revision: None,
                kind: StreamKind::Video,
                codec: CodecKind::Tiles,
                profile: None,
                size: FrameSize::new(64, 64),
            };
            let mut writer = GopWriter::open(&link.host, header).await.unwrap();
            writer
                .write_frame(1, 0, true, vec![vec![7; 1000]])
                .await
                .unwrap();
            match pictures.recv().await.expect("a picture event") {
                PictureEvent::Record { header, record, .. } => {
                    assert_eq!((header.gop_id, record.frame_id), (1, 1));
                }
                PictureEvent::Broken { gop_id } => panic!("picture group {gop_id} broke"),
            }
            assert!(probe.await.unwrap().unwrap() > 0, "the host wrote filler");
            let (mut answer, mut acked) = (None, false);
            while answer.is_none() || !acked {
                match answers.recv().await.expect("the viewer answers") {
                    ViewerFrame::FrameAck(ack) => {
                        assert_eq!((ack.gop_id, ack.frame_id), (1, 1));
                        acked = true;
                    }
                    ViewerFrame::ProbeResult(result) => answer = Some(result),
                    other => panic!("unexpected viewer frame {other:?}"),
                }
            }
            let answer = answer.unwrap();
            assert_eq!(answer.probe_id, 5);
            assert!(answer.bps > 0 && answer.bytes > 0, "{answer:?}");
            assert!(answer.duration_us >= WINDOW_US, "{answer:?}");
            assert!(answer.duration_us < 1_000_000, "{answer:?}");
            writer.finish();
            reader.abort();
            link.close().await;
        })
        .await
        .expect("loopback probe deadline");
    });
}

/// The host applies the „Übertragung“ of the hello (absent: „Echtzeit“)
/// and of `SET_DELIVERY`; the opening probe starts at the session's first
/// second; only the running probe's answer reaches the encoder and `STATS`
/// (`measured_kbps`, `measured_age_ms`, `delivery`, `rate_limit`
/// `measured`); „Neu messen“ starts the next probe at once.
#[test]
fn probe_task_host_applies_delivery_and_measurement() {
    fixtures::run(true, async {
        tokio::time::timeout(DEADLINE, async {
            let link = fixtures::link().await;
            let mut hello = client_hello();
            hello.delivery = Some(Delivery::Reliable);
            hello.bandwidth_probe = Some(BANDWIDTH_PROBE_V1);
            // As `host_server::serve` does.
            let mut pipeline = pipeline(&link.host);
            pipeline.set_delivery(hello.delivery.unwrap_or_default());
            let mut line = LineLink::for_hello(&hello);
            let mut actor = PipelineActor::new(pipeline, link.host.clone());
            assert_eq!(line.delivery(), Delivery::Reliable);
            assert_eq!(actor.delivery(), Some(Delivery::Reliable));
            // A viewer before the field: the default, and no probe.
            let mut old = LineLink::for_hello(&client_hello());
            assert_eq!(old.delivery(), Delivery::Realtime);
            let mut stats = actor.second(&link.host);
            old.second(&actor, &link.host, &mut stats);
            assert_eq!(old.running(), None);

            let set = ViewerFrame::SetDelivery(Delivery::Realtime);
            line.on_frame(set, &mut actor, &link.host).unwrap();
            assert_eq!(actor.delivery(), Some(Delivery::Realtime));
            let mut stats = actor.second(&link.host);
            line.second(&actor, &link.host, &mut stats);
            let id = line.running().expect("the opening probe runs");
            let chain = stats.chain.clone().unwrap();
            assert_eq!(chain.delivery, Some(Delivery::Realtime));
            assert_eq!((chain.measured_kbps, chain.measured_age_ms), (None, None));

            let answer = |probe_id, bps| {
                ViewerFrame::ProbeResult(ProbeResult {
                    probe_id,
                    bps,
                    bytes: 2_625_000,
                    duration_us: 500_000,
                })
            };
            line.on_frame(answer(id + 1, 9_000_000), &mut actor, &link.host)
                .unwrap();
            assert_eq!((actor.measured_bps(), line.running()), (None, Some(id)));
            line.on_frame(answer(id, 42_000_000), &mut actor, &link.host)
                .unwrap();
            assert_eq!(actor.measured_bps(), Some(42_000_000));
            assert_eq!(line.measured_bps(), Some(42_000_000));
            let mut stats = actor.second(&link.host);
            line.report(&mut stats);
            let chain = stats.chain.unwrap();
            assert_eq!(chain.measured_kbps, Some(42_000));
            assert!(chain.measured_age_ms.is_some());
            assert_eq!(chain.rate_limit.as_deref(), Some("measured"));
            // A second answer to the same probe changes nothing.
            line.on_frame(answer(id, 1_000_000), &mut actor, &link.host)
                .unwrap();
            assert_eq!(actor.measured_bps(), Some(42_000_000));

            line.on_frame(ViewerFrame::Remeasure, &mut actor, &link.host)
                .unwrap();
            assert_eq!(line.running(), Some(id + 1));
            assert!(line
                .on_frame(ViewerFrame::ListWindows, &mut actor, &link.host)
                .is_err());
            drop((line, old, actor));
            link.close().await;
        })
        .await
        .expect("host line deadline");
    });
}
