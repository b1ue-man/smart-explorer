//! B8: real HostSession with mutable OS facts; no desktop or window server needed.
use super::{host::{HostSession, WindowAdapter, WindowAuthority}, model::*};
use std::{io, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};

struct Rights { view: Arc<AtomicBool>, control: AtomicBool }
impl Rights {
    fn new() -> Self { Self { view: Arc::new(AtomicBool::new(true)), control: AtomicBool::new(true) } }
}
impl WindowAuthority for Rights {
    fn may_view(&self, session: &str) -> bool { session == "screen-session" && self.view.load(Ordering::SeqCst) }
    fn may_control(&self, session: &str) -> bool { session == "screen-session" && self.control.load(Ordering::SeqCst) }
    fn in_list_scope(&self, app_id: Option<&str>) -> bool { app_id == Some("org.example.Editor") }
}
#[derive(Default)]
struct OsFacts {
    windows: Vec<ObservedWindow>, effects: Vec<(WindowBinding, WindowOperation)>,
    disconnected: bool, revoke_during_observe: Option<Arc<AtomicBool>>,
}
#[derive(Clone)]
struct Adapter(Arc<Mutex<OsFacts>>);
impl WindowAdapter for Adapter {
    fn bind(&mut self, native: NativeWindowId) -> io::Result<WindowBinding> {
        self.0.lock().unwrap().windows.iter().find(|window| window.binding.native == native)
            .map(|window| window.binding.clone()).ok_or_else(|| io::Error::other("window absent"))
    }
    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        if self.0.lock().unwrap().windows.iter().any(|window| window.binding == *binding) { Ok(()) }
        else { Err(io::Error::new(io::ErrorKind::NotFound, "OS lifetime marker changed")) }
    }
    fn observe(&mut self, _: &WindowBinding) -> io::Result<Observation> {
        let mut state = self.0.lock().unwrap();
        if let Some(view) = state.revoke_during_observe.take() { view.store(false, Ordering::SeqCst); }
        Ok(Observation { windows: state.windows.clone(), limitations: Vec::new() })
    }
    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        self.validate(binding)?;
        let mut state = self.0.lock().unwrap();
        for window in &mut state.windows { window.focused = window.binding == *binding; }
        state.effects.push((binding.clone(), WindowOperation::Focus));
        Ok(())
    }
    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool> {
        Ok(self.0.lock().unwrap().windows.iter().any(|window| window.binding == *binding && window.focused))
    }
    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        self.validate(binding)?;
        self.0.lock().unwrap().effects.push((binding.clone(), operation));
        Ok(())
    }
    fn disconnect(&mut self) { self.0.lock().unwrap().disconnected = true; }
}
fn window(id: u64, owner: Option<u64>) -> ObservedWindow {
    ObservedWindow { binding: WindowBinding { native: NativeWindowId(id), birth: vec![id as u8, 1] },
        owner: owner.map(NativeWindowId), process: "pid-41:logon-1:birth-7".into(),
        app_id: Some("org.example.Editor".into()), app: "Editor".into(), title: format!("window-{id}"),
        kind: if owner.is_some() { WindowKind::Dialog } else { WindowKind::Main },
        geometry: CaptureGeometry { x: (id as i32 - 1) * 200, y: 0, width: 100, height: 100 },
        z_order: id as u32, focused: false, visible: true, minimized: false,
        operations: WindowOperations { resize: true, focus: true, close: true, input: true } }
}
fn open(windows: Vec<ObservedWindow>, rights: &Rights) -> (HostSession<Adapter>, Arc<Mutex<OsFacts>>) {
    let root = windows[0].binding.clone();
    let facts = Arc::new(Mutex::new(OsFacts { windows, ..Default::default() }));
    let host = HostSession::open_bound_as("screen-session".into(), root, Adapter(facts.clone()), rights,
        "ab".repeat(16)).unwrap();
    (host, facts)
}

#[test]
fn batches_family_includes_owner_descendants_excludes_foreign_process_and_preserves_close_dialog() {
    let rights = Rights::new();
    let mut foreign = window(4, Some(1));
    foreign.process = "pid-99:logon-1:birth-2".into();
    let mut outside_scope = window(5, Some(1));
    outside_scope.app_id = Some("org.example.Terminal".into());
    let (mut host, facts) = open(vec![window(1, None), window(2, Some(1)), window(3, Some(2)),
        foreign, outside_scope, window(6, None)], &rights);
    let snapshot = host.snapshot(&rights).unwrap();
    assert_eq!(snapshot.members.iter().map(|member| member.title.as_str()).collect::<Vec<_>>(),
        vec!["window-1", "window-2", "window-3"]);
    assert_eq!(snapshot.members[2].owner, Some(snapshot.members[1].identity.clone()));
    assert_eq!(host.operate(&snapshot.root, WindowOperation::Close, &rights).unwrap(), OperationStatus::Requested);
    facts.lock().unwrap().windows.push(window(7, Some(1)));
    let after_close = host.snapshot(&rights).unwrap();
    assert!(after_close.members.iter().any(|member| member.identity == snapshot.root));
    assert!(after_close.members.iter().any(|member| member.title == "window-7"));
    let before_disconnect = facts.lock().unwrap().effects.clone();
    host.disconnect();
    let state = facts.lock().unwrap();
    assert!(state.disconnected);
    assert_eq!(state.effects, before_disconnect, "disconnect must not close the application");
}

#[test]
fn batches_family_recycled_child_handle_gets_new_generation_and_old_root_cannot_rebind() {
    let rights = Rights::new();
    let (mut host, facts) = open(vec![window(1, None), window(2, Some(1))], &rights);
    let first = host.snapshot(&rights).unwrap();
    let old_child = first.members[1].identity.clone();
    facts.lock().unwrap().windows[1].binding.birth = vec![2, 2];
    let update = host.refresh(&rights).unwrap();
    assert!(update.removed.contains(&old_child));
    assert_ne!(update.snapshot.members[1].identity, old_child);
    assert!(host.operate(&old_child, WindowOperation::Close, &rights).is_err());
    assert!(facts.lock().unwrap().effects.is_empty());
    facts.lock().unwrap().windows[0].binding.birth = vec![1, 2];
    assert!(host.snapshot(&rights).is_err());
    assert!(host.operate(&first.root, WindowOperation::Focus, &rights).is_err());
    assert!(facts.lock().unwrap().effects.is_empty());
}

#[test]
fn batches_family_input_requires_confirmed_focus_current_revision_and_new_confirmation_after_focus_cycle() {
    let rights = Rights::new();
    let (mut host, facts) = open(vec![window(1, None), window(2, Some(1))], &rights);
    let initial = host.snapshot(&rights).unwrap();
    let root = initial.root;
    let mut injected = 0;
    assert!(host.with_input(&root, initial.revision, &rights, |_| { injected += 1; Ok(()) }).is_err());
    assert_eq!(host.operate(&root, WindowOperation::Focus, &rights).unwrap(), OperationStatus::FocusConfirmed);
    let focused = host.snapshot(&rights).unwrap();
    assert!(host.with_input(&root, initial.revision, &rights, |_| { injected += 1; Ok(()) }).is_err());
    host.with_input(&root, focused.revision, &rights, |_| { injected += 1; Ok(()) }).unwrap();
    let epoch = host.input_epoch();
    {
        let mut state = facts.lock().unwrap();
        state.windows[0].focused = false;
        state.windows[1].focused = true;
    }
    host.refresh(&rights).unwrap();
    {
        let mut state = facts.lock().unwrap();
        state.windows[0].focused = true;
        state.windows[1].focused = false;
    }
    let returned = host.snapshot(&rights).unwrap();
    assert!(host.input_epoch() > epoch);
    assert!(host.with_input(&root, returned.revision, &rights, |_| { injected += 1; Ok(()) }).is_err());
    host.operate(&root, WindowOperation::Focus, &rights).unwrap();
    let confirmed = host.snapshot(&rights).unwrap();
    host.with_input(&root, confirmed.revision, &rights, |_| { injected += 1; Ok(()) }).unwrap();
    assert_eq!(injected, 2);
}

#[test]
fn batches_family_capture_boundary_tracks_geometry_but_not_title_and_blank_canvas_has_no_input() {
    let rights = Rights::new();
    let (mut host, facts) = open(vec![window(1, None), window(2, Some(1))], &rights);
    let root = host.snapshot(&rights).unwrap().root;
    let boundary = host.boundary(&root, &rights).unwrap();
    assert!(boundary.member_at(150, 50).is_none());
    assert!(boundary.map_pointer(32768, 32768).is_none());
    assert!(boundary.map_pointer(0, 0).is_some());
    facts.lock().unwrap().windows[0].title = "renamed".into();
    assert!(host.boundary_current(&boundary, &rights).unwrap());
    facts.lock().unwrap().windows[1].geometry.x += 20;
    assert!(!host.boundary_current(&boundary, &rights).unwrap());
}

#[test]
fn batches_family_revocation_during_observation_blocks_publication_and_control() {
    let rights = Rights::new();
    let (mut host, facts) = open(vec![window(1, None)], &rights);
    let root = host.snapshot(&rights).unwrap().root;
    rights.control.store(false, Ordering::SeqCst);
    let read_only = host.snapshot(&rights).unwrap();
    assert_eq!(read_only.members[0].operations, WindowOperations::default());
    assert!(host.operate(&root, WindowOperation::Close, &rights).is_err());
    assert!(facts.lock().unwrap().effects.is_empty());
    rights.control.store(true, Ordering::SeqCst);
    facts.lock().unwrap().revoke_during_observe = Some(rights.view.clone());
    assert!(host.snapshot(&rights).is_err());
    assert!(host.boundary(&root, &rights).is_err());
    assert!(host.operate(&root, WindowOperation::Focus, &rights).is_err());
    assert!(facts.lock().unwrap().effects.is_empty());
}

#[test]
fn batches_family_portal_and_android_selection_cannot_inherit_reused_source_identity() {
    use super::selection::{AndroidSelection, PortalInputPath, PortalSelection};
    let mut portal = PortalSelection { session: "portal-1".into(), node_id: 7, pipewire_serial: Some(11),
        logical_geometry: CaptureGeometry { x: 200, y: 100, width: 640, height: 480 },
        pointer_granted: true, keyboard_granted: false, touch_granted: false, input_path: PortalInputPath::Eis };
    assert!(portal.validate_stream("portal-1", 7, Some(11), false).is_ok());
    assert!(portal.validate_stream("portal-1", 7, Some(12), true).is_err());
    assert!(portal.validate_stream("portal-2", 7, Some(11), true).is_err());
    assert!(portal.admit_input(PortalInputPath::Eis, true, false, false).is_ok());
    assert!(portal.admit_input(PortalInputPath::Eis, false, true, false).is_err());
    assert!(portal.admit_input(PortalInputPath::Notify, true, false, false).is_err());
    assert_eq!(portal.logical_pointer(u16::MAX, u16::MAX).unwrap(), (639.0, 479.0));
    portal.pipewire_serial = None;
    assert!(portal.validate_stream("portal-1", 7, None, true).is_ok());
    assert!(portal.validate_stream("portal-1", 7, None, false).is_err());
    let android = AndroidSelection { projection: "projection-1".into(),
        system_chosen_package: Some("org.example.Editor".into()), single_app: true };
    assert!(android.validate("projection-1", Some("org.example.Editor")).is_ok());
    assert!(android.validate("projection-2", Some("org.example.Editor")).is_err());
    assert!(android.validate("projection-1", Some("org.example.Terminal")).is_err());
}
