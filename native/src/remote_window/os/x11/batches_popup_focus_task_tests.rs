//! Existing popup focus/input cases use shared controlled facts.
use super::*;
use crate::screen::core::family_protocol::FamilyOperationResult;
use crate::screen::core::input_model::{InputEvent, TouchPhase};
use std::sync::atomic::Ordering;

#[path = "batches_popup_focus_fixtures.rs"]
mod fixtures;
use fixtures::*;

#[test]
fn batches_family_x11_combo_popup_keeps_identity_after_owner_focus_ack() {
    for input_focus in [H, M, 77] {
        for events in [
            click(),
            vec![
                InputEvent::Touch {
                    id: 0,
                    phase: TouchPhase::Start,
                    x: 32768,
                    y: 32768,
                },
                InputEvent::Touch {
                    id: 0,
                    phase: TouchPhase::End,
                    x: 32768,
                    y: 32768,
                },
            ],
        ] {
            let (mut host, facts, rights, mut viewer) = fixture(
                vec![
                    window(H, None, WindowKind::Main),
                    window(M, Some(H), WindowKind::Popup),
                ],
                &[M],
                input_focus,
            );
            facts.lock().unwrap().parents.insert(77, M);
            let before = host.snapshot(&rights).unwrap();
            let popup = native(&before, M);
            assert!(
                !before
                    .members
                    .iter()
                    .find(|w| w.identity == popup)
                    .unwrap()
                    .operations
                    .focus
            );
            let request = focus_request(viewer.input(events.clone()).unwrap());
            assert_eq!(request.identity, before.root);
            let queued = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
            assert_eq!(queued.target.identity, popup);
            assert_eq!(queued.events.len(), events.len());
            let (_, x, y) = before.pointer_target(32768, 32768).unwrap();
            let expected: Vec<_> = events
                .iter()
                .cloned()
                .map(|event| match event {
                    InputEvent::PointerAbs { .. } => InputEvent::PointerAbs { x, y },
                    InputEvent::Touch { id, phase, .. } => InputEvent::Touch { id, phase, x, y },
                    event => event,
                })
                .collect();
            assert_eq!(
                queued.events, expected,
                "owner activation must not change member coordinates or button/touch phases"
            );
            assert_eq!(
                inject(&mut host, &rights, &queued).unwrap(),
                NativeWindowId(u64::from(M))
            );
            assert_eq!(
                facts.lock().unwrap().effects,
                vec![(NativeWindowId(u64::from(H)), WindowOperation::Focus)]
            );
        }
    }
}

#[test]
fn batches_family_x11_nested_menus_use_recursive_owner_focus() {
    for actual in [H, M, S, 77] {
        let (mut host, facts, rights, mut viewer) = fixture(
            vec![
                window(H, None, WindowKind::Main),
                window(M, Some(H), WindowKind::Menu),
                window(S, Some(M), WindowKind::Menu),
            ],
            &[M, S],
            actual,
        );
        facts.lock().unwrap().parents.insert(77, S);
        let snapshot = host.snapshot(&rights).unwrap();
        let request = focus_request(viewer.input(click()).unwrap());
        assert_eq!(request.identity, snapshot.root);
        let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
        assert_eq!(input.target.identity, native(&snapshot, S));
        assert_eq!(
            inject(&mut host, &rights, &input).unwrap(),
            NativeWindowId(u64::from(S))
        );
    }
}

#[test]
fn batches_family_x11_modal_dialog_is_the_nearest_activation_boundary() {
    let (mut host, facts, rights, mut viewer) = fixture(
        vec![
            window(H, None, WindowKind::Main),
            window(M, Some(H), WindowKind::Dialog),
            window(S, Some(M), WindowKind::Popup),
        ],
        &[S],
        H,
    );
    let snapshot = host.snapshot(&rights).unwrap();
    let request = focus_request(viewer.input(click()).unwrap());
    assert_eq!(request.identity, native(&snapshot, M));
    assert!(
        host.operate(&request.identity, request.operation, &rights)
            .is_err(),
        "H cannot acknowledge D"
    );
    {
        let mut os = facts.lock().unwrap();
        os.active = M;
        os.input = S;
    }
    let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
    assert_eq!(
        inject(&mut host, &rights, &input).unwrap(),
        NativeWindowId(u64::from(S))
    );
    let mut disabled = host.snapshot(&rights).unwrap();
    disabled.revision += 1;
    disabled
        .members
        .iter_mut()
        .find(|w| w.identity == native(&snapshot, M))
        .unwrap()
        .operations
        .focus = false;
    viewer.receive(disabled).unwrap();
    viewer.disarm();
    assert!(
        viewer.input(click()).is_err(),
        "disabled dialog must not be skipped for H"
    );
}

#[test]
fn batches_family_x11_focus_rejects_foreign_unbound_recycled_owners_and_cycles() {
    let (_, facts, _, _) = fixture(
        vec![
            window(H, None, WindowKind::Main),
            window(M, Some(H), WindowKind::Menu),
            window(S, Some(M), WindowKind::Popup),
        ],
        &[M, S],
        H,
    );
    let mut os = facts.lock().unwrap();
    let original = os.windows.clone();
    let binding = os.windows[2].binding.clone();
    assert!(os.confirmed(&binding).unwrap());
    os.windows[1].process = "foreign-start-or-user-or-session".into();
    assert!(!os.confirmed(&binding).unwrap());
    os.windows = original.clone();
    os.unbound.insert(M);
    assert!(!os.confirmed(&binding).unwrap());
    os.unbound.clear();
    os.invalid.insert(M);
    assert!(os.confirmed(&binding).is_err());
    os.invalid.clear();
    os.windows[2].binding.birth.push(1);
    assert!(!os.confirmed(&binding).unwrap());
    os.windows = original.clone();
    os.windows[1].owner = Some(NativeWindowId(u64::from(S)));
    assert!(!os.confirmed(&binding).unwrap());
    os.windows = original.clone();
    os.windows[1].owner = None;
    assert!(!os.confirmed(&binding).unwrap());
    os.windows = original.clone();
    os.windows[1].visible = false;
    assert!(!os.confirmed(&binding).unwrap());
    os.windows = original.clone();
    os.windows[1].minimized = true;
    assert!(!os.confirmed(&binding).unwrap());
    os.windows = original;
    os.windows.push(window(40, None, WindowKind::Popup));
    os.override_redirect.insert(40);
    os.input = 40;
    assert!(
        !os.confirmed(&binding).unwrap(),
        "same process does not authorize an unassociated popup"
    );
    os.windows.push(window(50, Some(H), WindowKind::Dialog));
    os.input = 50;
    assert!(
        !os.confirmed(&binding).unwrap(),
        "another managed dialog has its own activation boundary"
    );
    for input in [0, 1, ROOT, 999] {
        os.input = input;
        assert!(!os.confirmed(&binding).unwrap());
    }
    os.input = H;
    os.active = 999;
    assert!(!os.confirmed(&binding).unwrap());
}

#[test]
fn batches_family_x11_host_preserves_disarming_permissions_and_lifetimes() {
    let (mut host, facts, rights, mut viewer) = fixture(
        vec![
            window(H, None, WindowKind::Main),
            window(M, Some(H), WindowKind::Popup),
        ],
        &[M],
        H,
    );
    let request = focus_request(viewer.input(click()).unwrap());
    let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
    assert!(inject(&mut host, &rights, &input).is_ok());
    facts.lock().unwrap().input = 999;
    assert!(inject(&mut host, &rights, &input).is_err());
    facts.lock().unwrap().input = H;
    assert!(
        inject(&mut host, &rights, &input).is_err(),
        "returning focus alone must not rearm"
    );
    let root = host.snapshot(&rights).unwrap().root;
    host.operate(&root, WindowOperation::Focus, &rights)
        .unwrap();
    assert!(inject(&mut host, &rights, &input).is_ok());
    facts.lock().unwrap().active = 999;
    host.snapshot(&rights).unwrap();
    facts.lock().unwrap().active = H;
    let returned = host.snapshot(&rights).unwrap();
    assert!(host
        .with_input(&input.target.identity, returned.revision, &rights, |_| Ok(
            ()
        ))
        .is_err());
    host.operate(&root, WindowOperation::Focus, &rights)
        .unwrap();
    rights.control.store(false, Ordering::SeqCst);
    assert!(inject(&mut host, &rights, &input).is_err());
    rights.control.store(true, Ordering::SeqCst);
    assert!(
        inject(&mut host, &rights, &input).is_err(),
        "grant restoration needs a fresh confirmation"
    );
    host.operate(&root, WindowOperation::Focus, &rights)
        .unwrap();
    let mut current = input.clone();
    current.target.snapshot_revision = host.snapshot(&rights).unwrap().revision;
    facts.lock().unwrap().invalid.insert(M);
    assert!(
        inject(&mut host, &rights, &current).is_err(),
        "current revision still requires the popup lifetime"
    );
    facts.lock().unwrap().invalid.clear();
    facts.lock().unwrap().invalid.insert(H);
    assert!(host.snapshot(&rights).is_err());
    rights.view.store(false, Ordering::SeqCst);
    assert!(host
        .operate(&root, WindowOperation::Focus, &rights)
        .is_err());
}

#[test]
fn batches_family_x11_viewer_discards_uncorrelated_stale_layout_and_disarmed_input() {
    let (mut host, _, rights, mut viewer) = fixture(
        vec![
            window(H, None, WindowKind::Main),
            window(M, Some(H), WindowKind::Popup),
        ],
        &[M],
        H,
    );
    let request = focus_request(viewer.input(click()).unwrap());
    let mut snapshot = host.snapshot(&rights).unwrap();
    let reply = |id: String, snapshot: FamilySnapshot| FamilyOperationResult {
        request_id: id,
        status: Some(OperationStatus::FocusConfirmed),
        reason: None,
        snapshot: Some(snapshot),
    };
    assert!(viewer
        .result(reply("unrelated".into(), snapshot.clone()))
        .unwrap()
        .1
        .is_empty());
    snapshot.revision += 1;
    snapshot.layout_revision += 1;
    snapshot.members[1].geometry.x += 1;
    viewer.receive(snapshot.clone()).unwrap();
    assert!(viewer
        .result(reply(request.request_id, snapshot.clone()))
        .unwrap()
        .1
        .is_empty());
    let request = focus_request(viewer.input(click()).unwrap());
    viewer.disarm();
    assert!(viewer
        .result(reply(request.request_id, snapshot))
        .unwrap()
        .1
        .is_empty());
}

#[test]
fn batches_family_x11_regular_windows_menus_tooltips_close_resize_and_disconnect_survive() {
    for kind in [WindowKind::Dialog, WindowKind::Popup] {
        let (mut host, facts, rights, mut viewer) = fixture(
            vec![window(H, None, WindowKind::Main), window(M, Some(H), kind)],
            &[],
            M,
        );
        facts.lock().unwrap().active = M;
        let snapshot = host.snapshot(&rights).unwrap();
        viewer.receive(snapshot.clone()).unwrap();
        let request = focus_request(viewer.input(click()).unwrap());
        assert_eq!(
            request.identity,
            native(&snapshot, M),
            "focusable popup/dialog keeps its own identity"
        );
        let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
        assert_eq!(
            inject(&mut host, &rights, &input).unwrap(),
            NativeWindowId(u64::from(M))
        );
    }
    for kind in [WindowKind::Menu, WindowKind::Tooltip] {
        let (mut host, _, rights, mut viewer) = fixture(
            vec![window(H, None, WindowKind::Main), window(M, Some(H), kind)],
            &[M],
            H,
        );
        let request = focus_request(viewer.input(click()).unwrap());
        let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
        assert_eq!(
            inject(&mut host, &rights, &input).unwrap().0,
            u64::from(if kind == WindowKind::Menu { M } else { H })
        );
    }
    let (mut host, facts, rights, _) = fixture(vec![window(H, None, WindowKind::Main)], &[], H);
    let root = host.snapshot(&rights).unwrap().root;
    assert_eq!(
        host.operate(&root, WindowOperation::Close, &rights)
            .unwrap(),
        OperationStatus::Requested
    );
    facts
        .lock()
        .unwrap()
        .windows
        .push(window(M, Some(H), WindowKind::Dialog));
    assert_eq!(
        host.snapshot(&rights).unwrap().members.len(),
        2,
        "save dialog must remain visible after Close"
    );
    assert_eq!(
        host.operate(
            &root,
            WindowOperation::Resize {
                width: 640,
                height: 480
            },
            &rights
        )
        .unwrap(),
        OperationStatus::Requested
    );
    let effects = facts.lock().unwrap().effects.clone();
    host.disconnect();
    assert!(facts.lock().unwrap().disconnected);
    assert_eq!(facts.lock().unwrap().effects, effects);
}

#[test]
fn batches_family_x11_combo_atom_and_activation_capabilities_are_explicit() {
    let atoms = Atoms {
        supported: 1,
        client_list: 2,
        client_stacking: 3,
        active: 4,
        moveresize: 5,
        name: 6,
        utf8: 7,
        pid: 8,
        protocols: 9,
        delete_window: 10,
        state: 11,
        hidden: 12,
        allowed_actions: 13,
        action_resize: 14,
        window_type: 15,
        dialog: 16,
        menu: 17,
        dropdown: 18,
        popup: 19,
        tooltip: 20,
        normal: 21,
        combo: 22,
    };
    for types in [vec![atoms.combo], vec![atoms.combo, atoms.normal]] {
        assert_eq!(
            properties::kind_from_types(&atoms, &types),
            WindowKind::Popup
        );
    }
    assert_eq!(
        properties::kind_from_types(&atoms, &[atoms.tooltip, atoms.combo]),
        WindowKind::Tooltip
    );
    assert_eq!(
        properties::kind_from_types(&atoms, &[atoms.dialog]),
        WindowKind::Dialog
    );
    assert_eq!(properties::kind_from_types(&atoms, &[]), WindowKind::Main);
    for kind in [WindowKind::Main, WindowKind::Dialog, WindowKind::Popup] {
        assert!(focus::can_activate(kind, false));
        assert!(!focus::can_activate(kind, true));
    }
    assert!(!focus::can_activate(WindowKind::Menu, false));
    assert!(!focus::can_activate(WindowKind::Tooltip, false));
}
