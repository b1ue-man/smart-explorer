//! Controlled window facts and host/viewer helpers for popup regression cases.
use super::super::*;
use crate::remote_window::{HostSession, WindowAuthority};
use crate::screen::core::family_protocol::{FamilyInput, FamilyOperation, FamilyOperationResult};
use crate::screen::core::family_viewer::FamilyViewer;
use crate::screen::core::input_model::{InputEvent, PointerButton};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ViewerFrame;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub(super) const H: Window = 10;
pub(super) const M: Window = 20;
pub(super) const S: Window = 30;
pub(super) const ROOT: Window = 1000;

pub(super) struct Rights {
    pub(super) control: AtomicBool,
    pub(super) view: AtomicBool,
}
impl Rights {
    fn new() -> Self {
        Self {
            control: AtomicBool::new(true),
            view: AtomicBool::new(true),
        }
    }
}
impl WindowAuthority for Rights {
    fn may_view(&self, session: &str) -> bool {
        session == "popup-fixture" && self.view.load(Ordering::SeqCst)
    }
    fn may_control(&self, session: &str) -> bool {
        session == "popup-fixture" && self.control.load(Ordering::SeqCst)
    }
    fn in_list_scope(&self, app: Option<&str>) -> bool {
        app == Some("org.example.Editor")
    }
}

pub(super) struct Facts {
    pub(super) windows: Vec<ObservedWindow>,
    pub(super) override_redirect: HashSet<Window>,
    pub(super) unbound: HashSet<Window>,
    pub(super) invalid: HashSet<Window>,
    pub(super) parents: HashMap<Window, Window>,
    pub(super) active: Window,
    pub(super) input: Window,
    pub(super) effects: Vec<(NativeWindowId, WindowOperation)>,
    pub(super) disconnected: bool,
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
        if self.unbound.contains(&window) {
            return Ok(None);
        }
        let Some(w) = self
            .windows
            .iter()
            .find(|w| w.binding.native.0 == u64::from(window))
        else {
            return Ok(None);
        };
        self.validate(&w.binding)?;
        Ok(Some(focus::Node {
            binding: w.binding.clone(),
            process: w.process.as_bytes().to_vec(),
            owner: w.owner.map(|id| xid(id).unwrap()),
            owner_focus: focus::owner_focus(w.kind, self.override_redirect.contains(&window)),
            eligible: w.visible && !w.minimized && w.kind != WindowKind::Tooltip,
        }))
    }
    pub(super) fn confirmed(&self, binding: &WindowBinding) -> io::Result<bool> {
        focus::confirmed(
            binding,
            self.active,
            self.input,
            ROOT,
            |window| self.node(window),
            |window| Ok(*self.parents.get(&window).unwrap_or(&ROOT)),
        )
    }
}

#[derive(Clone)]
pub(super) struct Adapter(Arc<Mutex<Facts>>);
impl WindowAdapter for Adapter {
    fn bind(&mut self, id: NativeWindowId) -> io::Result<WindowBinding> {
        self.0
            .lock()
            .unwrap()
            .windows
            .iter()
            .find(|w| w.binding.native == id)
            .map(|w| w.binding.clone())
            .ok_or_else(gone)
    }
    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        self.0.lock().unwrap().validate(binding)
    }
    fn observe(&mut self, _: &WindowBinding) -> io::Result<Observation> {
        let facts = self.0.lock().unwrap();
        let windows = facts
            .windows
            .iter()
            .cloned()
            .map(|mut w| {
                w.focused = w.binding.native.0 == u64::from(facts.active);
                w.operations.focus = focus::can_activate(
                    w.kind,
                    facts
                        .override_redirect
                        .contains(&xid(w.binding.native).unwrap()),
                );
                w
            })
            .collect();
        Ok(Observation {
            windows,
            limitations: Vec::new(),
        })
    }
    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        self.validate(binding)?;
        // An activation request never manufactures an OS fact or success.
        self.0
            .lock()
            .unwrap()
            .effects
            .push((binding.native, WindowOperation::Focus));
        Ok(())
    }
    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool> {
        self.0.lock().unwrap().confirmed(binding)
    }
    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        self.validate(binding)?;
        self.0
            .lock()
            .unwrap()
            .effects
            .push((binding.native, operation));
        Ok(())
    }
    fn disconnect(&mut self) {
        self.0.lock().unwrap().disconnected = true;
    }
}

pub(super) fn window(id: Window, owner: Option<Window>, kind: WindowKind) -> ObservedWindow {
    ObservedWindow {
        binding: WindowBinding {
            native: NativeWindowId(u64::from(id)),
            birth: vec![id as u8; 16],
        },
        owner: owner.map(|id| NativeWindowId(u64::from(id))),
        process: "x11:uid:session:pid:start".into(),
        app_id: Some("org.example.Editor".into()),
        app: "Editor".into(),
        title: format!("window-{id}"),
        kind,
        geometry: if id == H {
            CaptureGeometry {
                x: 0,
                y: 0,
                width: 200,
                height: 150,
            }
        } else {
            CaptureGeometry {
                x: 40 + (id as i32 - 20) * 4,
                y: 40,
                width: 80,
                height: 50,
            }
        },
        z_order: id,
        focused: false,
        visible: true,
        minimized: false,
        operations: WindowOperations {
            input: kind != WindowKind::Tooltip,
            focus: true,
            resize: id == H,
            close: id == H,
        },
    }
}
pub(super) fn fixture(
    windows: Vec<ObservedWindow>,
    overrides: &[Window],
    input: Window,
) -> (
    HostSession<Adapter>,
    Arc<Mutex<Facts>>,
    Rights,
    FamilyViewer,
) {
    let rights = Rights::new();
    let binding = windows[0].binding.clone();
    let facts = Arc::new(Mutex::new(Facts {
        windows,
        override_redirect: overrides.iter().copied().collect(),
        unbound: HashSet::new(),
        invalid: HashSet::new(),
        parents: HashMap::new(),
        active: H,
        input,
        effects: Vec::new(),
        disconnected: false,
    }));
    let mut host = HostSession::open_bound_as(
        "popup-fixture".into(),
        binding,
        Adapter(facts.clone()),
        &rights,
        "ab".repeat(16),
    )
    .unwrap();
    let snapshot = host.snapshot(&rights).unwrap();
    let mut viewer =
        FamilyViewer::for_source(&SourceId::Window(format!("@family.{}", snapshot.session)))
            .unwrap();
    viewer.receive(snapshot).unwrap();
    (host, facts, rights, viewer)
}
pub(super) fn focus_request(frames: Vec<ViewerFrame>) -> FamilyOperation {
    let mut frames = frames.into_iter();
    let Some(ViewerFrame::FamilyOperation(request)) = frames.next() else {
        panic!("focus operation required");
    };
    assert!(
        frames.next().is_none(),
        "input must wait for the correlated focus reply"
    );
    request
}
pub(super) fn acknowledge(
    host: &mut HostSession<Adapter>,
    rights: &Rights,
    viewer: &mut FamilyViewer,
    request: FamilyOperation,
) -> Vec<ViewerFrame> {
    let status = host
        .operate(&request.identity, request.operation, rights)
        .unwrap();
    let snapshot = host.snapshot(rights).unwrap();
    viewer
        .result(FamilyOperationResult {
            request_id: request.request_id,
            status: Some(status),
            reason: None,
            snapshot: Some(snapshot),
        })
        .unwrap()
        .1
}
pub(super) fn input_frame(frames: Vec<ViewerFrame>) -> FamilyInput {
    let mut frames = frames.into_iter();
    let Some(ViewerFrame::FamilyInput(input)) = frames.next() else {
        panic!("family input required");
    };
    assert!(frames.next().is_none());
    input
}
pub(super) fn native(snapshot: &FamilySnapshot, id: Window) -> WindowIdentity {
    snapshot
        .members
        .iter()
        .find(|w| w.title == format!("window-{id}"))
        .unwrap()
        .identity
        .clone()
}
pub(super) fn inject(
    host: &mut HostSession<Adapter>,
    rights: &Rights,
    input: &FamilyInput,
) -> io::Result<NativeWindowId> {
    host.with_input(
        &input.target.identity,
        input.target.snapshot_revision,
        rights,
        |member| Ok(member.binding.native),
    )
}
pub(super) fn click() -> Vec<InputEvent> {
    vec![
        InputEvent::PointerAbs { x: 32768, y: 32768 },
        InputEvent::Button {
            button: PointerButton::Left,
            down: true,
        },
        InputEvent::Button {
            button: PointerButton::Left,
            down: false,
        },
    ]
}
