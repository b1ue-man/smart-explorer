//! Plan 2026-11-17 „Windows-Bild“ P3 (a): the encoding stage asks for the
//! number of rank stages only after a failure (asking built the whole codec
//! register, with every Media Foundation enumeration, for every picture),
//! and a process keeps its register.
use std::borrow::Cow;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::encode::{EncoderOpener, Encoding};
use crate::screen::core::{
    capture_admission::{
        CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SessionAdmission, SourceKey,
    },
    capture_image::{AdmittedBytes, CaptureImageBuilder, CapturedImage},
    capture_picture::Picture,
    capture_plan::CapturePhases,
    capture_records::CapturedRecords,
    codec_registry::OpenedEncoder,
    frame_budget::FrameLimits,
    host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder},
    host_stage::StageRequest,
    negotiate::{CodecChoice, CodecKind, H264Profile},
    pixel_convert::{BgraImage, FrameSize},
};

const SIZE: FrameSize = FrameSize::new(64, 64);

/// An encoder that succeeds `ok` times, then fails.
struct Counted {
    ok: usize,
}

impl ScreenEncoder for Counted {
    fn encode_captured(
        &mut self,
        frame: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        if self.ok == 0 {
            return Err(io::Error::other("driver hung"));
        }
        self.ok -= 1;
        let mut bytes = AdmittedBytes::buffer(&frame.grant(), CaptureOwner::CodecOutput, 8)?;
        bytes.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88])?;
        Ok(CapturedEncodedFrame {
            records: CapturedRecords::one(bytes)?,
            keyframe: request.keyframe,
        })
    }
    fn name(&self) -> String {
        "counted".into()
    }
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn encode(&mut self, _: &BgraImage, _: bool, _: u64) -> io::Result<EncodedFrame> {
        Err(io::Error::other("admitted path only"))
    }
}

/// Opens `first` (working `ok` pictures), then a working one; counts the
/// openings and the questions for the stage count.
#[derive(Default)]
struct Opener {
    ok: usize,
    opens: AtomicUsize,
    stage_counts: AtomicUsize,
}

impl EncoderOpener for Opener {
    fn open(
        &self,
        _: &StageRequest,
        refused: &[String],
        _: CaptureGrant,
    ) -> io::Result<OpenedEncoder> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let first = !refused.iter().any(|name| name == "first");
        Ok(OpenedEncoder {
            encoder: Box::new(Counted {
                ok: if first { self.ok } else { usize::MAX },
            }),
            factory: if first { "first" } else { "second" }.into(),
            hardware: first,
        })
    }

    fn stage_count(&self, _: &StageRequest) -> usize {
        self.stage_counts.fetch_add(1, Ordering::SeqCst);
        2
    }
}

fn request(frame_id: u64) -> StageRequest {
    StageRequest {
        frame_id,
        frame_contract: None,
        visible_rects: None,
        keyframe: frame_id == 1,
        target_bps: 4_000_000,
        size: SIZE,
        choice: CodecChoice {
            codec: CodecKind::H264,
            profile: Some(H264Profile::High),
            encoder_hardware: true,
            decoder_hardware: true,
            max_size: FrameSize::MAX,
            max_pixels: FrameSize::MAX.pixels(),
            max_macroblocks_per_second: None,
        },
        encoder_epoch: 1,
        fps: 60,
    }
}

fn grant() -> (Arc<SessionAdmission>, CaptureGrant) {
    let admission = crate::screen::core::capture_test_support::session(None);
    let key = SourceKey::new(admission.session(), "p3-stage-count").unwrap();
    let mut plan = CapturePlan::new(key, CaptureGeneration::new(1));
    plan.bind_contract(None).unwrap();
    for owner in [
        CaptureOwner::SourcePixels,
        CaptureOwner::ScaledPixels,
        CaptureOwner::CodecInput,
    ] {
        plan.add_bgra(owner, SIZE, 2, CapturePhases::ALL).unwrap();
    }
    for owner in [
        CaptureOwner::CodecOutput,
        CaptureOwner::FilterScratch,
        CaptureOwner::Metadata,
    ] {
        plan.add_bytes(owner, 65_536, 2, CapturePhases::ALL)
            .unwrap();
    }
    let grant = admission.authority().approve(plan).unwrap();
    (admission, grant)
}

#[test]
fn bildkette_task_verify_encode_does_not_rebuild_registry_per_picture() {
    let (_admission, grant) = grant();
    let image = Picture::Cpu(
        CaptureImageBuilder::new(
            SIZE,
            FrameLimits::LEGACY,
            &grant,
            CaptureOwner::SourcePixels,
        )
        .unwrap()
        .freeze(),
    );
    // Thirty pictures of a working encoder: one opening, no stage count.
    let opener = Arc::new(Opener {
        ok: usize::MAX,
        ..Opener::default()
    });
    let mut encoding = Encoding::new(opener.clone());
    for frame_id in 1..=30 {
        let req = request(frame_id);
        let (_, name, _) = encoding
            .encode_with_fallback(&image, Cow::Borrowed(&req), &grant)
            .unwrap();
        assert_eq!(name, "counted");
    }
    assert_eq!(opener.opens.load(Ordering::SeqCst), 1);
    assert_eq!(opener.stage_counts.load(Ordering::SeqCst), 0);
    // The first failure of a working instance gets one fresh instance of the
    // same factory with a key frame (review 2026-11-19 B1); its failure asks
    // once for the stage count and the next stage takes over with a key frame.
    let opener = Arc::new(Opener {
        ok: 3,
        ..Opener::default()
    });
    let mut encoding = Encoding::new(opener.clone());
    for frame_id in 1..=10 {
        let req = request(frame_id);
        let (frame, _, hardware) = encoding
            .encode_with_fallback(&image, Cow::Borrowed(&req), &grant)
            .unwrap();
        if frame_id == 4 {
            assert!(frame.keyframe && hardware);
            assert!(encoding.refused.is_empty());
        }
        if frame_id == 7 {
            assert!(frame.keyframe && !hardware);
        }
    }
    assert_eq!(encoding.refused, ["first"]);
    assert_eq!(opener.opens.load(Ordering::SeqCst), 3);
    assert_eq!(opener.stage_counts.load(Ordering::SeqCst), 1);
    // A process keeps its register for one store and switch.
    let store = tempfile::tempdir().unwrap();
    let first = crate::screen::os::codec_inventory::kept(Some(store.path()));
    let again = crate::screen::os::codec_inventory::kept(Some(store.path()));
    assert!(first.same_factories(&again));
}
