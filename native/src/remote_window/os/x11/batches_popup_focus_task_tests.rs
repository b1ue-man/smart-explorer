//! Controlled X11 facts drive the production focus predicate, HostSession and
//! shared desktop/Android viewer. Successful replies come from HostSession;
//! no X server or real XRes/XTest run is claimed by these fixtures.
use super::*;
use crate::remote_window::{HostSession, WindowAuthority};
use crate::screen::core::family_protocol::{FamilyInput, FamilyOperation, FamilyOperationResult};
use crate::screen::core::family_viewer::FamilyViewer;
use crate::screen::core::input_model::{InputEvent, PointerButton, TouchPhase};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ViewerFrame;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

const H: Window = 10;
const M: Window = 20;
const S: Window = 30;
const ROOT: Window = 1000;

struct Rights { control: AtomicBool, view: AtomicBool }
impl Rights {
    fn new() -> Self { Self { control: AtomicBool::new(true), view: AtomicBool::new(true) } }
}
impl WindowAuthority for Rights {
    fn may_view(&self, session: &str) -> bool { session == "popup-fixture" && self.view.load(Ordering::SeqCst) }
    fn may_control(&self, session: &str) -> bool { session == "popup-fixture" && self.control.load(Ordering::SeqCst) }
    fn in_list_scope(&self, app: Option<&str>) -> bool { app == Some("org.example.Editor") }
}

struct Facts {
    windows: Vec<ObservedWindow>,
    override_redirect: HashSet<Window>,
    unbound: HashSet<Window>,
    invalid: HashSet<Window>,
    parents: HashMap<Window, Window>,
    active: Window,
    input: Window,
    effects: Vec<(NativeWindowId, WindowOperation)>,
    disconnected: bool,
}
impl Facts {
    fn validate(&self, binding: &WindowBinding) -> io::Result<()> {
        let window = xid(binding.native)?;
        if self.invalid.contains(&window) || !self.windows.iter().any(|w| w.binding == *binding) {
            return Err(gone());
        }
        Ok(())
    }
    fn node(&self, window: Window) -> io::Result<Option<focus::Node>> {
        if self.unbound.contains(&window) { return Ok(None); }
        let Some(w) = self.windows.iter().find(|w| w.binding.native.0 == u64::from(window)) else { return Ok(None); };
        self.validate(&w.binding)?;
        Ok(Some(focus::Node { binding: w.binding.clone(), process: w.process.as_bytes().to_vec(),
            owner: w.owner.map(|id| xid(id).unwrap()),
            owner_focus: focus::owner_focus(w.kind, self.override_redirect.contains(&window)),
            eligible: w.visible && !w.minimized && w.kind != WindowKind::Tooltip }))
    }
    fn confirmed(&self, binding: &WindowBinding) -> io::Result<bool> {
        focus::confirmed(binding, self.active, self.input, ROOT, |window| self.node(window),
            |window| Ok(*self.parents.get(&window).unwrap_or(&ROOT)))
    }
}

#[derive(Clone)]
struct Adapter(Arc<Mutex<Facts>>);
impl WindowAdapter for Adapter {
    fn bind(&mut self, id: NativeWindowId) -> io::Result<WindowBinding> {
        self.0.lock().unwrap().windows.iter().find(|w| w.binding.native == id)
            .map(|w| w.binding.clone()).ok_or_else(gone)
    }
    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()> { self.0.lock().unwrap().validate(binding) }
    fn observe(&mut self, _: &WindowBinding) -> io::Result<Observation> {
        let facts = self.0.lock().unwrap();
        let windows = facts.windows.iter().cloned().map(|mut w| {
            w.focused = w.binding.native.0 == u64::from(facts.active);
            w.operations.focus = focus::can_activate(w.kind,
                facts.override_redirect.contains(&xid(w.binding.native).unwrap()));
            w
        }).collect();
        Ok(Observation { windows, limitations: Vec::new() })
    }
    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        self.validate(binding)?;
        // An activation request never manufactures an OS fact or success.
        self.0.lock().unwrap().effects.push((binding.native, WindowOperation::Focus));
        Ok(())
    }
    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool> { self.0.lock().unwrap().confirmed(binding) }
    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        self.validate(binding)?;
        self.0.lock().unwrap().effects.push((binding.native, operation));
        Ok(())
    }
    fn disconnect(&mut self) { self.0.lock().unwrap().disconnected = true; }
}

fn window(id: Window, owner: Option<Window>, kind: WindowKind) -> ObservedWindow {
    ObservedWindow { binding: WindowBinding { native: NativeWindowId(u64::from(id)), birth: vec![id as u8; 16] },
        owner: owner.map(|id| NativeWindowId(u64::from(id))), process: "x11:uid:session:pid:start".into(),
        app_id: Some("org.example.Editor".into()), app: "Editor".into(), title: format!("window-{id}"),
        kind, geometry: if id == H { CaptureGeometry { x: 0, y: 0, width: 200, height: 150 } }
            else { CaptureGeometry { x: 40 + (id as i32 - 20) * 4, y: 40, width: 80, height: 50 } },
        z_order: id, focused: false, visible: true, minimized: false,
        operations: WindowOperations { input: kind != WindowKind::Tooltip,
            focus: true, resize: id == H, close: id == H } }
}
fn fixture(windows: Vec<ObservedWindow>, overrides: &[Window], input: Window)
    -> (HostSession<Adapter>, Arc<Mutex<Facts>>, Rights, FamilyViewer)
{
    let rights = Rights::new();
    let binding = windows[0].binding.clone();
    let facts = Arc::new(Mutex::new(Facts { windows,
        override_redirect: overrides.iter().copied().collect(), unbound: HashSet::new(), invalid: HashSet::new(),
        parents: HashMap::new(), active: H, input, effects: Vec::new(), disconnected: false }));
    let mut host = HostSession::open_bound_as("popup-fixture".into(), binding, Adapter(facts.clone()),
        &rights, "ab".repeat(16)).unwrap();
    let snapshot = host.snapshot(&rights).unwrap();
    let mut viewer = FamilyViewer::for_source(&SourceId::Window(format!("@family.{}", snapshot.session))).unwrap();
    viewer.receive(snapshot).unwrap();
    (host, facts, rights, viewer)
}
fn focus_request(frames: Vec<ViewerFrame>) -> FamilyOperation {
    let mut frames = frames.into_iter();
    let Some(ViewerFrame::FamilyOperation(request)) = frames.next() else { panic!("focus operation required"); };
    assert!(frames.next().is_none(), "input must wait for the correlated focus reply");
    request
}
fn acknowledge(host: &mut HostSession<Adapter>, rights: &Rights, viewer: &mut FamilyViewer,
    request: FamilyOperation) -> Vec<ViewerFrame>
{
    let status = host.operate(&request.identity, request.operation, rights).unwrap();
    let snapshot = host.snapshot(rights).unwrap();
    viewer.result(FamilyOperationResult { request_id: request.request_id,
        status: Some(status), reason: None, snapshot: Some(snapshot) }).unwrap().1
}
fn input_frame(frames: Vec<ViewerFrame>) -> FamilyInput {
    let mut frames = frames.into_iter();
    let Some(ViewerFrame::FamilyInput(input)) = frames.next() else { panic!("family input required"); };
    assert!(frames.next().is_none());
    input
}
fn native(snapshot: &FamilySnapshot, id: Window) -> WindowIdentity {
    snapshot.members.iter().find(|w| w.title == format!("window-{id}")).unwrap().identity.clone()
}
fn inject(host: &mut HostSession<Adapter>, rights: &Rights, input: &FamilyInput) -> io::Result<NativeWindowId> {
    host.with_input(&input.target.identity, input.target.snapshot_revision, rights, |member| Ok(member.binding.native))
}
fn click() -> Vec<InputEvent> {
    vec![InputEvent::PointerAbs { x: 32768, y: 32768 },
        InputEvent::Button { button: PointerButton::Left, down: true },
        InputEvent::Button { button: PointerButton::Left, down: false }]
}

#[test]
fn batches_family_x11_combo_popup_keeps_identity_after_owner_focus_ack() {
    for input_focus in [H, M, 77] {
        for events in [click(), vec![InputEvent::Touch { id: 0, phase: TouchPhase::Start, x: 32768, y: 32768 },
            InputEvent::Touch { id: 0, phase: TouchPhase::End, x: 32768, y: 32768 }]] {
            let (mut host, facts, rights, mut viewer) = fixture(vec![window(H, None, WindowKind::Main),
                window(M, Some(H), WindowKind::Popup)], &[M], input_focus);
            facts.lock().unwrap().parents.insert(77, M);
            let before = host.snapshot(&rights).unwrap();
            let popup = native(&before, M);
            assert!(!before.members.iter().find(|w| w.identity == popup).unwrap().operations.focus);
            let request = focus_request(viewer.input(events.clone()).unwrap());
            assert_eq!(request.identity, before.root);
            let queued = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
            assert_eq!(queued.target.identity, popup);
            assert_eq!(queued.events.len(), events.len());
            let (_, x, y) = before.pointer_target(32768, 32768).unwrap();
            let expected: Vec<_> = events.iter().cloned().map(|event| match event {
                InputEvent::PointerAbs { .. } => InputEvent::PointerAbs { x, y },
                InputEvent::Touch { id, phase, .. } => InputEvent::Touch { id, phase, x, y },
                event => event,
            }).collect();
            assert_eq!(queued.events, expected, "owner activation must not change member coordinates or button/touch phases");
            assert_eq!(inject(&mut host, &rights, &queued).unwrap(), NativeWindowId(u64::from(M)));
            assert_eq!(facts.lock().unwrap().effects, vec![(NativeWindowId(u64::from(H)), WindowOperation::Focus)]);
        }
    }
}

#[test]
fn batches_family_x11_nested_menus_use_recursive_owner_focus() {
    for actual in [H, M, S, 77] {
        let (mut host, facts, rights, mut viewer) = fixture(vec![window(H, None, WindowKind::Main),
            window(M, Some(H), WindowKind::Menu), window(S, Some(M), WindowKind::Menu)], &[M, S], actual);
        facts.lock().unwrap().parents.insert(77, S);
        let snapshot = host.snapshot(&rights).unwrap();
        let request = focus_request(viewer.input(click()).unwrap());
        assert_eq!(request.identity, snapshot.root);
        let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
        assert_eq!(input.target.identity, native(&snapshot, S));
        assert_eq!(inject(&mut host, &rights, &input).unwrap(), NativeWindowId(u64::from(S)));
    }
}

#[test]
fn batches_family_x11_modal_dialog_is_the_nearest_activation_boundary() {
    let (mut host, facts, rights, mut viewer) = fixture(vec![window(H, None, WindowKind::Main),
        window(M, Some(H), WindowKind::Dialog), window(S, Some(M), WindowKind::Popup)], &[S], H);
    let snapshot = host.snapshot(&rights).unwrap();
    let request = focus_request(viewer.input(click()).unwrap());
    assert_eq!(request.identity, native(&snapshot, M));
    assert!(host.operate(&request.identity, request.operation, &rights).is_err(), "H cannot acknowledge D");
    {
        let mut os = facts.lock().unwrap(); os.active = M; os.input = S;
    }
    let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
    assert_eq!(inject(&mut host, &rights, &input).unwrap(), NativeWindowId(u64::from(S)));
    let mut disabled = host.snapshot(&rights).unwrap();
    disabled.revision += 1;
    disabled.members.iter_mut().find(|w| w.identity == native(&snapshot, M)).unwrap().operations.focus = false;
    viewer.receive(disabled).unwrap(); viewer.disarm();
    assert!(viewer.input(click()).is_err(), "disabled dialog must not be skipped for H");
}

#[test]
fn batches_family_x11_focus_rejects_foreign_unbound_recycled_owners_and_cycles() {
    let (_, facts, _, _) = fixture(vec![window(H, None, WindowKind::Main),
        window(M, Some(H), WindowKind::Menu), window(S, Some(M), WindowKind::Popup)], &[M, S], H);
    let mut os = facts.lock().unwrap();
    let original = os.windows.clone();
    let binding = os.windows[2].binding.clone();
    assert!(os.confirmed(&binding).unwrap());
    os.windows[1].process = "foreign-start-or-user-or-session".into();
    assert!(!os.confirmed(&binding).unwrap()); os.windows = original.clone();
    os.unbound.insert(M); assert!(!os.confirmed(&binding).unwrap()); os.unbound.clear();
    os.invalid.insert(M); assert!(os.confirmed(&binding).is_err()); os.invalid.clear();
    os.windows[2].binding.birth.push(1); assert!(!os.confirmed(&binding).unwrap()); os.windows = original.clone();
    os.windows[1].owner = Some(NativeWindowId(u64::from(S)));
    assert!(!os.confirmed(&binding).unwrap()); os.windows = original.clone();
    os.windows[1].owner = None; assert!(!os.confirmed(&binding).unwrap()); os.windows = original.clone();
    os.windows[1].visible = false; assert!(!os.confirmed(&binding).unwrap()); os.windows = original.clone();
    os.windows[1].minimized = true; assert!(!os.confirmed(&binding).unwrap()); os.windows = original;
    os.windows.push(window(40, None, WindowKind::Popup)); os.override_redirect.insert(40);
    os.input = 40;
    assert!(!os.confirmed(&binding).unwrap(), "same process does not authorize an unassociated popup");
    os.windows.push(window(50, Some(H), WindowKind::Dialog)); os.input = 50;
    assert!(!os.confirmed(&binding).unwrap(), "another managed dialog has its own activation boundary");
    for input in [0, 1, ROOT, 999] { os.input = input; assert!(!os.confirmed(&binding).unwrap()); }
    os.input = H; os.active = 999; assert!(!os.confirmed(&binding).unwrap());
}

#[test]
fn batches_family_x11_host_preserves_disarming_permissions_and_lifetimes() {
    let (mut host, facts, rights, mut viewer) = fixture(vec![window(H, None, WindowKind::Main),
        window(M, Some(H), WindowKind::Popup)], &[M], H);
    let request = focus_request(viewer.input(click()).unwrap());
    let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
    assert!(inject(&mut host, &rights, &input).is_ok());
    facts.lock().unwrap().input = 999;
    assert!(inject(&mut host, &rights, &input).is_err());
    facts.lock().unwrap().input = H;
    assert!(inject(&mut host, &rights, &input).is_err(), "returning focus alone must not rearm");
    let root = host.snapshot(&rights).unwrap().root;
    host.operate(&root, WindowOperation::Focus, &rights).unwrap();
    assert!(inject(&mut host, &rights, &input).is_ok());
    facts.lock().unwrap().active = 999; host.snapshot(&rights).unwrap();
    facts.lock().unwrap().active = H; let returned = host.snapshot(&rights).unwrap();
    assert!(host.with_input(&input.target.identity, returned.revision, &rights, |_| Ok(())).is_err());
    host.operate(&root, WindowOperation::Focus, &rights).unwrap();
    rights.control.store(false, Ordering::SeqCst);
    assert!(inject(&mut host, &rights, &input).is_err());
    rights.control.store(true, Ordering::SeqCst);
    assert!(inject(&mut host, &rights, &input).is_err(), "grant restoration needs a fresh confirmation");
    host.operate(&root, WindowOperation::Focus, &rights).unwrap();
    let mut current = input.clone();
    current.target.snapshot_revision = host.snapshot(&rights).unwrap().revision;
    facts.lock().unwrap().invalid.insert(M);
    assert!(inject(&mut host, &rights, &current).is_err(), "current revision still requires the popup lifetime");
    facts.lock().unwrap().invalid.clear(); facts.lock().unwrap().invalid.insert(H);
    assert!(host.snapshot(&rights).is_err());
    rights.view.store(false, Ordering::SeqCst);
    assert!(host.operate(&root, WindowOperation::Focus, &rights).is_err());
}

#[test]
fn batches_family_x11_viewer_discards_uncorrelated_stale_layout_and_disarmed_input() {
    let (mut host, _, rights, mut viewer) = fixture(vec![window(H, None, WindowKind::Main),
        window(M, Some(H), WindowKind::Popup)], &[M], H);
    let request = focus_request(viewer.input(click()).unwrap());
    let mut snapshot = host.snapshot(&rights).unwrap();
    let reply = |id: String, snapshot: FamilySnapshot| FamilyOperationResult { request_id: id,
        status: Some(OperationStatus::FocusConfirmed), reason: None, snapshot: Some(snapshot) };
    assert!(viewer.result(reply("unrelated".into(), snapshot.clone())).unwrap().1.is_empty());
    snapshot.revision += 1; snapshot.layout_revision += 1;
    snapshot.members[1].geometry.x += 1;
    viewer.receive(snapshot.clone()).unwrap();
    assert!(viewer.result(reply(request.request_id, snapshot.clone())).unwrap().1.is_empty());
    let request = focus_request(viewer.input(click()).unwrap()); viewer.disarm();
    assert!(viewer.result(reply(request.request_id, snapshot)).unwrap().1.is_empty());
}

#[test]
fn batches_family_x11_regular_windows_menus_tooltips_close_resize_and_disconnect_survive() {
    for kind in [WindowKind::Dialog, WindowKind::Popup] {
        let (mut host, facts, rights, mut viewer) = fixture(vec![window(H, None, WindowKind::Main),
            window(M, Some(H), kind)], &[], M);
        facts.lock().unwrap().active = M;
        let snapshot = host.snapshot(&rights).unwrap(); viewer.receive(snapshot.clone()).unwrap();
        let request = focus_request(viewer.input(click()).unwrap());
        assert_eq!(request.identity, native(&snapshot, M), "focusable popup/dialog keeps its own identity");
        let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
        assert_eq!(inject(&mut host, &rights, &input).unwrap(), NativeWindowId(u64::from(M)));
    }
    for kind in [WindowKind::Menu, WindowKind::Tooltip] {
        let (mut host, _, rights, mut viewer) = fixture(vec![window(H, None, WindowKind::Main),
            window(M, Some(H), kind)], &[M], H);
        let request = focus_request(viewer.input(click()).unwrap());
        let input = input_frame(acknowledge(&mut host, &rights, &mut viewer, request));
        assert_eq!(inject(&mut host, &rights, &input).unwrap().0,
            u64::from(if kind == WindowKind::Menu { M } else { H }));
    }
    let (mut host, facts, rights, _) = fixture(vec![window(H, None, WindowKind::Main)], &[], H);
    let root = host.snapshot(&rights).unwrap().root;
    assert_eq!(host.operate(&root, WindowOperation::Close, &rights).unwrap(), OperationStatus::Requested);
    facts.lock().unwrap().windows.push(window(M, Some(H), WindowKind::Dialog));
    assert_eq!(host.snapshot(&rights).unwrap().members.len(), 2, "save dialog must remain visible after Close");
    assert_eq!(host.operate(&root, WindowOperation::Resize { width: 640, height: 480 }, &rights).unwrap(), OperationStatus::Requested);
    let effects = facts.lock().unwrap().effects.clone();
    host.disconnect();
    assert!(facts.lock().unwrap().disconnected); assert_eq!(facts.lock().unwrap().effects, effects);
}

#[test]
fn batches_family_x11_combo_atom_and_activation_capabilities_are_explicit() {
    let atoms = Atoms { supported: 1, client_list: 2, client_stacking: 3, active: 4, moveresize: 5,
        name: 6, utf8: 7, pid: 8, protocols: 9, delete_window: 10, state: 11, hidden: 12,
        allowed_actions: 13, action_resize: 14, window_type: 15, dialog: 16, menu: 17,
        dropdown: 18, popup: 19, tooltip: 20, normal: 21, combo: 22 };
    for types in [vec![atoms.combo], vec![atoms.combo, atoms.normal]] {
        assert_eq!(properties::kind_from_types(&atoms, &types), WindowKind::Popup);
    }
    assert_eq!(properties::kind_from_types(&atoms, &[atoms.tooltip, atoms.combo]), WindowKind::Tooltip);
    assert_eq!(properties::kind_from_types(&atoms, &[atoms.dialog]), WindowKind::Dialog);
    assert_eq!(properties::kind_from_types(&atoms, &[]), WindowKind::Main);
    for kind in [WindowKind::Main, WindowKind::Dialog, WindowKind::Popup] {
        assert!(focus::can_activate(kind, false)); assert!(!focus::can_activate(kind, true));
    }
    assert!(!focus::can_activate(WindowKind::Menu, false));
    assert!(!focus::can_activate(WindowKind::Tooltip, false));
}
