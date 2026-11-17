//! Windows task tests of plan 2026-11-17 „Windows-Bild“ P3 without a GPU:
//! the input route of the encoders (b) and the key-frame rule D8 of the
//! Media Foundation output, both pure functions.
use crate::screen::core::negotiate::CodecKind;
use crate::screen::os::mf_capture_output::checked_unit;
use crate::screen::os::mf_encoder::d3d_input::{
    encoder_route, input_route, EncoderFacts, InputRoute, SystemReason,
};

#[test]
fn bildkette_task_verify_vendor_route_without_capture_uses_own_device() {
    assert_eq!(input_route(Ok(()), true), InputRoute::Capture);
    // No capture device yet (a check, `bench --local`), the capture on a
    // VM's display adapter or the iGPU: a device of its own.
    for reason in [
        SystemReason::NoCapture,
        SystemReason::OtherAdapter,
        SystemReason::OtherVendor,
    ] {
        assert_eq!(input_route(Err(reason), true), InputRoute::OwnDevice);
        assert_eq!(input_route(Err(reason), false), InputRoute::SystemMemory);
    }
    // As before: a GPU path switched off after a failure, a device without
    // video support, an MFT without D3D11, a refused device manager.
    for reason in [
        SystemReason::Disabled,
        SystemReason::NoVideo,
        SystemReason::NotD3D11Aware,
        SystemReason::ManagerRefused,
    ] {
        assert_eq!(input_route(Err(reason), true), InputRoute::SystemMemory);
    }
    // The measured VM: capture on the virtual adapter, the encoder on the
    // passed-through GPU.
    let vm = EncoderFacts {
        disabled: false,
        capture: true,
        video: true,
        aware: true,
        capture_adapter: Some(1),
        encoder_adapter: Some(2),
    };
    let route = encoder_route(vm);
    assert_eq!(route, Err(SystemReason::OtherAdapter));
    assert_eq!(input_route(route, true), InputRoute::OwnDevice);
    let none = EncoderFacts {
        capture: false,
        ..vm
    };
    assert_eq!(
        input_route(encoder_route(none), true),
        InputRoute::OwnDevice
    );
}

fn nal(kind: u8, body: &[u8]) -> Vec<u8> {
    [vec![0, 0, 0, 1, 0x60 | kind], body.to_vec()].concat()
}

#[test]
fn bildkette_task_verify_mf_keyframe_needs_idr_unit() {
    let h264 = CodecKind::H264;
    let sets = [nal(7, &[0x42, 0x00, 0x1f]), nal(8, &[0xce])].concat();
    let idr = nal(5, &[0x88, 0x84]);
    let slice = nal(1, &[0x9a, 0x02]);
    let aud = nal(9, &[0xf0]);
    let mut kept = None;
    // CleanPoint and the IDR agree: a key frame; its parameter sets kept.
    let first = [sets.clone(), idr.clone()].concat();
    let unit = checked_unit(h264, first, true, true, &mut kept, None).unwrap();
    assert!(unit.keyframe);
    assert_eq!(kept.as_deref(), Some(sets.as_slice()));
    // CleanPoint on a picture without IDR (the viewer drops such a key
    // frame: black) and an IDR without CleanPoint: encoder failures.
    assert!(checked_unit(h264, slice.clone(), true, false, &mut kept, None).is_err());
    assert!(checked_unit(h264, idr.clone(), false, false, &mut kept, None).is_err());
    // Neither: a picture of the group, unchanged.
    let delta = checked_unit(h264, slice.clone(), false, false, &mut kept, None).unwrap();
    assert!(!delta.keyframe);
    assert_eq!(delta.records, vec![slice.clone()]);
    // A key frame on request that is none.
    assert!(checked_unit(h264, slice, false, true, &mut kept, None).is_err());
    // A later IDR without parameter sets gets the kept ones, behind its
    // access unit delimiter.
    let bare = [aud.clone(), idr.clone()].concat();
    let unit = checked_unit(h264, bare, true, true, &mut kept, None).unwrap();
    assert_eq!(
        unit.records,
        vec![[aud, sets.clone(), idr.clone()].concat()]
    );
    // The first IDR without any: those of the output type, else a failure.
    let mut none = None;
    assert!(checked_unit(h264, idr.clone(), true, true, &mut none, None).is_err());
    let unit = checked_unit(
        h264,
        idr.clone(),
        true,
        true,
        &mut none,
        Some(sets.as_slice()),
    )
    .unwrap();
    assert_eq!(unit.records, vec![[sets.clone(), idr].concat()]);
    assert_eq!(none.as_deref(), Some(sets.as_slice()));
    // HEVC: a CRA (type 21) marked as a key frame is no IDR.
    let cra = vec![0, 0, 0, 1, 21 << 1, 1, 0x80];
    let mut kept = None;
    assert!(checked_unit(CodecKind::Hevc, cra, true, false, &mut kept, None).is_err());
}
