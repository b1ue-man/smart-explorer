//! V7 fixtures in the existing common remote suite; never run on the workstation.
use std::sync::PoisonError;
use serde_json::{json, Value};
use super::{codecs, quality_baseline, quality_provenance, tests::store_with_openh264, TEST_LOCK};
use crate::screen::core::{
    batches_quality_acceptance_task_tests::{capture, grant, request, header, record},
    capture_admission::CaptureOwner,
    capture_image::CaptureImageBuilder,
    codec_registry::CodecRegistry, decoded_frame::DecodedFrame,
    frame_budget::FrameLimits, frame_protocol::{self, Operation},
    host_backend::ScreenDecoder,
    pixel_convert::{BgraImage, FrameSize},
    quality_acceptance_metrics::{self as metrics, measure, retain},
};

const FIXTURE: &str = "batches_task_b6_quality_openh264_baseline_transport_and_still";

fn codec_profile(registry: &CodecRegistry, store: &super::OpenH264Store,
    size: FrameSize, bps: u64) -> Value {
    let mut request = request(registry, size, bps, false);
    let grant = grant(&request);
    let mut candidate = registry.open_encoder_for_stage_admitted(&request, &[], grant.clone()).unwrap();
    let mut before = quality_baseline::open(store, size, request.fps).unwrap();
    let header = header(&request);
    let mut clean = registry.open_decoder(&header).unwrap();
    let mut impaired = registry.open_decoder(&header).unwrap();
    let mut held: Option<(u64, BgraImage)> = None;
    let mut rows = Vec::new();
    let mut no_regression = true;
    let mut awaiting_keyframe=false;
    let mut withheld_frames=0;
    let mut recovered_after_fault=Vec::new();
    for id in 1..=12 {
        request.frame_id = id;
        request.keyframe = matches!(id, 1 | 7 | 10);
        let source = capture(size, id);
        let before_hash = metrics::sha256(source.data());
        let mut builder = CaptureImageBuilder::new(size, FrameLimits::LEGACY, &grant, CaptureOwner::SourcePixels).unwrap();
        builder.copy_from(&source.view()).unwrap();
        let captured = builder.freeze();
        let old = before.encoder.encode(&source, request.keyframe, bps).unwrap();
        let new = candidate.encoder.encode_captured(&captured, &request).unwrap();
        assert_eq!(old.records.len(), 1, "baseline must produce exactly one AU for every capture");
        assert_eq!(new.records.len(), 1, "candidate must produce exactly one AU for every capture");
        assert_eq!(old.keyframe, new.keyframe);
        let old_record = record(id, old.keyframe, false, old.records[0].clone());
        let new_record = record(id, new.keyframe, false, new.records[0].clone());
        let old_picture = before.decode(&old_record).unwrap().expect("baseline must output synchronously from its first capture");
        assert!(clean.decode(&new_record).unwrap());
        let identity = DecodedFrame::from_record(&new_record);
        assert_eq!(clean.decoded_frame(), Some(identity));
        assert_eq!(before_hash, metrics::sha256(source.data()));
        assert_eq!(source.data(), captured.image().data(), "both arms must receive the very same capture bytes");
        let (old_quality, new_quality) = metrics::paired(&source, identity,
            (old_picture, DecodedFrame::from_record(&old_record)), (clean.picture(), clean.decoded_frame().unwrap())).unwrap();
        no_regression &= metrics::no_codec_regression(&old_quality, &new_quality);
        let first_recovery=awaiting_keyframe && new_record.is_keyframe();
        let accepted = if id == 4 {
            // A broken reliable GOP discards the decoder; this is not a claim
            // about QUIC packet loss, latency or a real renderer callback.
            impaired = registry.open_decoder(&header).unwrap();
            awaiting_keyframe=true;
            false
        } else if awaiting_keyframe && !new_record.is_keyframe() {
            assert!(impaired.decode(&new_record).is_err());
            assert!(impaired.decoded_frame().is_none());
            withheld_frames+=1;
            false
        } else {
            assert!(impaired.decode(&new_record).unwrap());
            assert_eq!(impaired.decoded_frame(), Some(identity));
            if first_recovery {recovered_after_fault.push(id);}
            awaiting_keyframe=false;
            true
        };
        let transport = if accepted {
            let quality = measure(clean.picture(), impaired.picture()).unwrap();
            assert_eq!(quality.different_pixels, 0, "transport recovery must equal the clean decode of this exact AU");
            held = Some((id, impaired.picture().clone()));
            Some(quality)
        } else {None};
        if id == 8 {
            assert!(impaired.decode(&new_record).is_err(), "replayed record must poison the reference");
            assert!(impaired.decoded_frame().is_none());
            awaiting_keyframe=true;
        }
        let held_quality = held.as_ref().map(|(_, picture)| measure(&source, picture).unwrap());
        let label = format!("{}x{}-{bps}-{id}", size.width, size.height);
        rows.push(json!({"frame_id": id, "capture_ts_us": identity.capture_ts_us, "actual_keyframe":new_record.is_keyframe(),
            "capture": retain(&format!("{label}-capture"), &source),
            "baseline_output": retain(&format!("{label}-baseline"), old_picture),
            "candidate_output": retain(&format!("{label}-candidate"), clean.picture()),
            "codec_only": {"baseline": old_quality, "candidate": new_quality},
            "transport": {"accepted": accepted, "codec_to_accepted": transport,
                "accepted_output_frame_id": accepted.then_some(id), "held_last_accepted_frame_id": held.as_ref().map(|item| item.0),
                "capture_to_held_last_accepted": held_quality,
                "held_scope":"retained pixels for quality analysis, not an observed renderer or display",
                "post_output_replay_rejected": id == 8,
                "decoder_output_identity_after_fault": impaired.decoded_frame().map(|value| value.frame_id),
                "fault": if id==4 {"GOP aborted before record"} else if id==8 {"replay rejected after valid output"}
                    else if first_recovery {"first complete IDR after fault"}
                    else if !accepted {"dependent output withheld"} else {"none"}}}));
    }
    drop(candidate);
    assert_eq!(grant.live_bytes().unwrap(), 0);
    assert!(withheld_frames>0,"the physical stream must exercise an actual dependent frame after reference loss");
    assert_eq!(recovered_after_fault.len(),2);
    json!({"width": size.width, "height": size.height, "fps": 30, "bps": bps,
        "capture_cadence": "identical logical 33333us input cadence; no timing/throughput claim",
        "no_codec_regression": no_regression,"withheld_dependent_frames":withheld_frames,
        "first_confirmed_keyframe_after_each_fault":recovered_after_fault,"frames": rows})
}

fn hybrid_still(registry: &CodecRegistry) -> Value {
    let size = FrameSize::new(320, 192);
    let mut request = request(registry, size, 2_000_000, true);
    let grant = grant(&request);
    let mut opened = registry.open_encoder_for_stage_admitted(&request, &[], grant.clone()).unwrap();
    let header = header(&request);
    let mut clean = registry.open_decoder(&header).unwrap();
    let mut impaired = registry.open_decoder(&header).unwrap();
    let mut rows = Vec::new();
    for id in 1..=6 {
        request.frame_id = id;
        request.keyframe = matches!(id, 1 | 6);
        let source = capture(size, if id == 3 {2} else {id});
        let mut builder = CaptureImageBuilder::new(size, request.frame_contract.unwrap().limits, &grant, CaptureOwner::SourcePixels).unwrap();
        builder.copy_from(&source.view()).unwrap();
        let captured = builder.freeze();
        let encoded = opened.encoder.encode_captured(&captured, &request).unwrap();
        assert!(!encoded.records.is_empty());
        let mut video = 0;
        let mut dirty = 0;
        let mut published = 0;
        let mut impaired_published = 0;
        let mut withheld = false;
        if id == 4 { impaired = registry.open_decoder(&header).unwrap(); }
        for (index, payload) in encoded.records.iter().enumerate() {
            let more = index + 1 < encoded.records.len();
            match frame_protocol::decode(payload, size).unwrap().1 {
                Operation::Video {keyframe, ..} => {video += 1; if id == 6 {assert!(keyframe);} }
                Operation::Dirty {..} => {dirty += 1;}
                _ => {},
            }
            let record = record(id, encoded.keyframe, more, payload.clone());
            if clean.decode(&record).unwrap() {
                assert!(!more);
                assert_eq!(clean.decoded_frame(), Some(DecodedFrame::from_record(&record)));
                published += 1;
            }
            if id != 4 && !withheld {
                match impaired.decode(&record) {
                    Ok(true) => {assert!(!more); impaired_published += 1;}
                    Ok(false) => {},
                    Err(_) if id == 5 => {withheld = true; assert!(impaired.decoded_frame().is_none());}
                    Err(error) => panic!("unexpected hybrid quality decode error: {error}"),
                }
            }
        }
        assert_eq!(published, 1);
        let quality = measure(&source, clean.picture()).unwrap();
        if matches!(id, 1 | 3 | 6) {
            assert_eq!(quality.different_pixels, 0);
            assert_eq!(quality.ssim_luma, 1.0);
            assert!(quality.psnr_infinite);
        }
        if id == 3 {assert_eq!(video, 0); assert!(dirty > 0); assert!(!opened.encoder.needs_refresh());}
        if id == 5 {assert!(withheld);}
        assert_eq!(impaired_published, usize::from(!matches!(id, 4 | 5)));
        let transport_quality = if impaired_published == 1 {
            let quality = measure(clean.picture(), impaired.picture()).unwrap();
            assert_eq!(quality.different_pixels, 0);
            Some(quality)
        } else {None};
        rows.push(json!({"frame_id": id, "capture_sha256": metrics::sha256(source.data()),
            "quality": quality, "video_operations": video, "lossless_operations": dirty,
            "transport_accepted": impaired_published == 1, "codec_to_accepted": transport_quality,
            "still_same_pixels_as_frame": (id == 3).then_some(2),
            "fault": match id {4 => "GOP aborted before Begin", 5 => "missing-base dependent withheld", 6 => "first complete physical IDR and exact regions recover", _ => "none"}}));
    }
    drop(opened);
    assert_eq!(grant.live_bytes().unwrap(), 0);
    json!({"path": "productive admitted CapturedHybrid + OpenH264 + ReferenceDecoder/HybridDecoder", "frames": rows})
}

#[test]
fn batches_task_b6_quality_openh264_baseline_transport_and_still() {
    if std::env::var_os("SE_SCREEN_TASK_OPENH264").is_none() || super::super::openh264_pins::PIN.is_none() {
        eprintln!("SE_TASK_UNVERIFIED_JSON={}", json!({"fixture": FIXTURE,
            "reason": "pinned physical OpenH264 library unavailable; no SSIM/PSNR codec comparison executed"}));
        return;
    }
    let _lock = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let provenance = quality_provenance::verify();
    let (_directory, store) = store_with_openh264();
    let (encoders, decoders, _) = codecs(store.clone(), true);
    let registry = CodecRegistry::new(encoders, decoders);
    let mut profiles = Vec::new();
    for size in [FrameSize::new(320, 192), FrameSize::new(192, 320), FrameSize::new(640, 360)] {
        for bps in [150_000, 2_000_000] {profiles.push(codec_profile(&registry, &store, size, bps));}
    }
    let hybrid = hybrid_still(&registry);
    let passed = profiles.iter().all(|profile| profile["no_codec_regression"] == true);
    metrics::evidence("openh264-baseline-transport-still", json!({"provenance": provenance,
        "profiles": profiles, "hybrid_still": hybrid, "no_codec_regression": passed,
        "transport_scope": "controlled reliable-GOP abort/replay at decoder boundary; no real packet-loss, renderer, latency or rate measurement",
        "unverified_physical_codecs": ["Windows Media Foundation synchronous", "Windows Media Foundation asynchronous", "Android device-specific MediaCodec inventory and quality"],
        "result": if passed {"passed"} else {"failed"}}));
    assert!(passed, "V7 codec quality regressed against the immutable release path; preserve quality-acceptance evidence");
}
