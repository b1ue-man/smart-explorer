//! The host pipeline's answer to `SET_DECODERS` (plan 2026-11-14
//! „Bildkette“ P1): a changed pairing ends the running picture group and
//! its encoder, so the next frame is a key frame of the new codec in a new
//! group; an unchanged pairing changes nothing. The continuation of B-36
//! follows the decoder of the running pairing.
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use super::*;
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::negotiate::{negotiate, CodecPreference, H264Profile};

const ALPN: &[u8] = b"bildkette-proto-fixture/1";

struct Source;

impl ScreenSource for Source {
    fn size(&self) -> FrameSize {
        FrameSize::new(640, 360)
    }
    fn capture(&mut self, _id: u64, _key: bool) -> io::Result<SourceFrame> {
        Err(io::Error::other("the pairing fixture never captures"))
    }
}

async fn connections() -> (iroh::Endpoint, iroh::Endpoint, Connection, Connection) {
    async fn endpoint() -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup()
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap()
    }
    let left = endpoint().await;
    let right = endpoint().await;
    let address = iroh::EndpointAddr::from_parts(
        right.id(),
        right
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    );
    let (outgoing, incoming) = tokio::join!(left.connect(address, ALPN), async {
        right.accept().await.unwrap().await
    });
    (left, right, outgoing.unwrap(), incoming.unwrap())
}

fn h264() -> CodecOffer {
    CodecOffer::h264(H264Profile::High, false)
}

fn hevc() -> CodecOffer {
    CodecOffer::video(CodecKind::Hevc, false, false)
}

fn host_encoders() -> Vec<CodecOffer> {
    vec![h264(), hevc(), CodecOffer::tiles()]
}

fn set(decoders: Vec<CodecOffer>, continuing: Vec<CodecOffer>) -> SetDecoders {
    SetDecoders {
        decoders,
        gop_continue: false,
        gop_continue_decoders: continuing,
        codec_choice: None,
    }
}

fn run(test: impl FnOnce(&mut Pipeline)) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            let (left, right, host, viewer) = connections().await;
            let choice = negotiate(
                &host_encoders(),
                &[h264(), CodecOffer::tiles()],
                CodecPreference::Auto,
            )[0];
            assert_eq!(choice.codec, CodecKind::H264);
            let mut pipeline = Pipeline::new(
                Arc::new(crate::screen::os::synthetic_backend::SyntheticBackend),
                Box::new(Source),
                choice,
                &host,
                30,
                None,
                Arc::new(AtomicU32::new(0)),
            );
            pipeline.set_pairing(host_encoders(), None, ContinueOffers::default());
            test(&mut pipeline);
            drop(pipeline);
            host.close(0u32.into(), b"done");
            viewer.close(0u32.into(), b"done");
            left.close().await;
            right.close().await;
        })
        .await
        .expect("pairing fixture deadline");
    });
}

/// A new pairing restarts the encoder and asks for a key frame in a new
/// group of the new codec; the same offers again change nothing.
#[test]
fn bildkette_task_proto_switch_changes_the_pairing_and_starts_a_group() {
    run(|pipeline| {
        let epoch = pipeline.encoder_epoch;
        assert!(pipeline.set_decoders(&set(vec![hevc(), CodecOffer::tiles()], Vec::new())));
        assert_eq!(pipeline.choice.codec, CodecKind::Hevc);
        assert_eq!(pipeline.pairing.current.codec, CodecKind::Hevc);
        assert!(pipeline.encoder_epoch > epoch, "the encoder of H.264 went");
        assert!(pipeline.running.is_none() && pipeline.gop.is_none());
        assert!(
            pipeline.pacer.keyframe_due(),
            "the new group starts with a key frame"
        );
        // Back to H.264: again a new group.
        let epoch = pipeline.encoder_epoch;
        assert!(pipeline.set_decoders(&set(vec![h264(), CodecOffer::tiles()], Vec::new())));
        assert_eq!(pipeline.choice.codec, CodecKind::H264);
        assert!(pipeline.encoder_epoch > epoch);
    });
}

/// The same pairing again: no new group, no new encoder, nothing else.
#[test]
fn bildkette_task_proto_unchanged_pairing_does_nothing() {
    run(|pipeline| {
        let (epoch, choice) = (pipeline.encoder_epoch, pipeline.choice);
        let gop = pipeline.next_gop_id;
        assert!(!pipeline.set_decoders(&set(vec![h264(), CodecOffer::tiles()], Vec::new())));
        assert_eq!(pipeline.encoder_epoch, epoch);
        assert_eq!(pipeline.choice, choice);
        assert_eq!(pipeline.next_gop_id, gop);
        // A different order of the same offers negotiates the same pairing.
        assert!(!pipeline.set_decoders(&set(vec![CodecOffer::tiles(), h264()], Vec::new())));
        assert_eq!(pipeline.encoder_epoch, epoch);
        // Under „Verlustfrei“ the tiles run: a new pairing restarts nothing
        // and applies when the switch goes off.
        let mode = pipeline
            .quality_policy
            .mode
            .update(StreamKind::Video, None, None, false, true);
        pipeline.set_quality_mode(mode);
        assert_eq!(pipeline.choice.codec, CodecKind::Tiles);
        let epoch = pipeline.encoder_epoch;
        assert!(pipeline.set_decoders(&set(vec![hevc(), CodecOffer::tiles()], Vec::new())));
        assert_eq!(pipeline.choice.codec, CodecKind::Tiles);
        assert_eq!(pipeline.encoder_epoch, epoch);
        let mode = pipeline
            .quality_policy
            .mode
            .update(StreamKind::Video, None, None, false, false);
        pipeline.set_quality_mode(mode);
        assert_eq!(pipeline.choice.codec, CodecKind::Hevc);
    });
}

/// B-36 per codec after a switch: the frame's announcement applies, and a
/// continuation is offered only while the running pairing's decoder is one
/// of its continuing decoders.
#[test]
fn bildkette_task_proto_continuation_follows_the_running_pairing() {
    run(|pipeline| {
        assert!(!pipeline.continuation_offered());
        let decoders = vec![hevc(), CodecOffer::tiles()];
        assert!(pipeline.set_decoders(&set(decoders.clone(), vec![hevc()])));
        assert!(pipeline.continuation_offered());
        // The same pairing with another announcement: nothing restarts,
        // the announcement applies.
        let epoch = pipeline.encoder_epoch;
        assert!(!pipeline.set_decoders(&set(decoders.clone(), Vec::new())));
        assert_eq!(pipeline.encoder_epoch, epoch);
        assert!(!pipeline.continuation_offered());
        // A pending continuation whose decoder no longer continues resets
        // to a key frame instead of sending an undecodable continuation.
        assert!(!pipeline.set_decoders(&set(decoders.clone(), vec![hevc()])));
        pipeline.continue_from = Some(5);
        assert!(!pipeline.set_decoders(&set(decoders, Vec::new())));
        assert_eq!(pipeline.continue_from, None);
        // Fix 2026-11-18 („Diashow“): the reset keeps the encoder; its key
        // frame starts the new group (`Pipeline::reset_stream`).
        assert_eq!(pipeline.encoder_epoch, epoch);
        assert!(pipeline.pacer.keyframe_due());
        // H.264 is not among the continuing decoders; HEVC stays the W2 fallback.
        // (The tiles in the list keep „Automatisch“, plan 2026-11-18.)
        let automatic = vec![h264(), hevc(), CodecOffer::tiles()];
        assert!(pipeline.set_decoders(&set(automatic, vec![hevc()])));
        assert_eq!(pipeline.choice.codec, CodecKind::H264);
        assert_eq!(pipeline.quality_policy.fallbacks.len(), 1);
        assert_eq!(pipeline.quality_policy.fallbacks[0].codec, CodecKind::Hevc);
        assert!(!pipeline.continuation_offered());
    });
}

/// Review 2026-11-19 (Codex 10/6): a change from the full view to the
/// preview at the same picture size keeps the running encoder and starts a
/// group of the new kind with its key frame; a new size replaces it.
#[test]
fn review1119_task_preview_switch_keeps_a_fitting_encoder() {
    run(|pipeline| {
        // The preview's picture size of this source, then the full view at it.
        pipeline.set_mode(StreamKind::Preview, Some(FrameSize::new(320, 180)), None);
        let size = pipeline.target_size();
        pipeline.set_mode(StreamKind::Video, Some(size), None);
        assert_eq!(pipeline.target_size(), size, "the full view at that size");
        let encoder = crate::screen::core::host_stage::StageEncoder {
            name: "fixture".into(),
            hardware: false,
        };
        pipeline.running = Some((encoder, size));
        let epoch = pipeline.encoder_epoch;
        pipeline.set_mode(StreamKind::Preview, Some(size), None);
        assert_eq!(pipeline.encoder_epoch, epoch, "no invalidate_reference");
        assert!(pipeline.running.is_some(), "the encoder stays");
        assert!(
            pipeline.kept_encoder,
            "its next picture must be a key frame"
        );
        assert!(pipeline.pacer.keyframe_due(), "a group of the new kind");
        // Another size: `resize` replaces the encoder.
        pipeline.set_mode(StreamKind::Preview, Some(FrameSize::new(160, 90)), None);
        assert!(
            pipeline.encoder_epoch > epoch,
            "a new encoder for the new size"
        );
        assert!(pipeline.running.is_none());
    });
}
