//! Existing member-geometry checks for the bound Linux family input adapter.
use super::*;
use crate::remote_window::{
    CaptureGeometry, CaptureMember, NativeWindowId, WindowBinding, WindowIdentity,
};

fn member() -> CaptureMember {
    CaptureMember {
        identity: WindowIdentity {
            session: "ab".repeat(16),
            generation: 2,
        },
        binding: WindowBinding {
            native: NativeWindowId(42),
            birth: vec![7; 16],
        },
        geometry: CaptureGeometry {
            x: -50,
            y: 20,
            width: 321,
            height: 123,
        },
        z_order: 1,
        input: true,
    }
}

#[test]
fn batches_family_x11_input_retains_admitted_member_geometry_until_inject_bound() {
    let member = member();
    let events = vec![
        InputEvent::PointerAbs { x: u16::MAX, y: 0 },
        InputEvent::Touch {
            id: 0,
            phase: crate::screen::core::input_model::TouchPhase::Start,
            x: 0,
            y: u16::MAX,
        },
    ];
    let mut called = false;
    let result = inject_x11_family(&member, &events, |source, area, forwarded| {
        called = true;
        assert_eq!(source, &SourceId::Window("42".into()));
        assert_eq!(
            area,
            super::super::x11_input::Area {
                x: -50,
                y: 20,
                width: 321,
                height: 123
            }
        );
        assert_eq!(forwarded, events.as_slice());
        // A final native geometry refusal must reach WorkerFamily so it
        // releases held input; it must not fall back to unbound injection.
        Err(ControlBlock::ControlUnavailable)
    });
    assert!(called);
    assert_eq!(result, Err(ControlBlock::ControlUnavailable));
}

#[test]
fn batches_family_x11_invalid_member_geometry_never_reaches_the_injector() {
    let mut cases = vec![member(); 4];
    cases[0].input = false;
    cases[1].geometry.width = 0;
    cases[2].geometry.x = -1;
    cases[2].geometry.width = i32::MAX as u32 + 1;
    cases[3].binding.native = NativeWindowId(u64::from(u32::MAX) + 1);
    for member in cases {
        assert_eq!(
            inject_x11_family(&member, &[], |_, _, _| panic!(
                "invalid member reached XTest"
            )),
            Err(ControlBlock::ControlUnavailable)
        );
    }
}
