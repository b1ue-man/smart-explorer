//! X11 0.13: private lifetime properties survive neither DestroyWindow nor a
//! server/session reconnect. Family membership requires transient+process proof.
use std::collections::{HashMap, HashSet};
use std::io;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _,
    EventMask, MapState, PropMode, Window};
use x11rb::protocol::res::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use crate::remote_window::core::host::WindowAdapter;
use crate::remote_window::core::model::*;

mod properties;
use properties::{Atoms, ProcessFacts};

pub(crate) struct X11Adapter {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
    property: Atom,
    property_name: String,
    markers: HashMap<NativeWindowId, WindowBinding>,
    app_ids: HashMap<NativeWindowId, (Vec<u8>, Option<String>)>,
}

fn failed(error: impl std::fmt::Display) -> io::Error { io::Error::other(format!("x11: {error}")) }
fn gone() -> io::Error { io::Error::new(io::ErrorKind::NotFound, "original_x11_window_gone") }
fn xid(id: NativeWindowId) -> io::Result<Window> { u32::try_from(id.0).ok().filter(|v| *v != 0).ok_or_else(gone) }

impl X11Adapter {
    pub(crate) fn adopt(pin: &WindowPin) -> io::Result<Self> {
        if pin.platform != PinPlatform::X11 || !pin.namespace.starts_with("_SMART_EXPLORER_WINDOW_")
            || pin.namespace.len() > 128 || !pin.namespace.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid_x11_pin_namespace"));
        }
        let mut adapter = Self::connect()?;
        adapter.property = adapter.conn.intern_atom(true, pin.namespace.as_bytes()).map_err(failed)?
            .reply().map_err(failed)?.atom;
        if adapter.property == 0 { return Err(gone()); }
        adapter.property_name = pin.namespace.clone();
        adapter.validate_inner(&pin.binding)?;
        adapter.markers.insert(pin.binding.native, pin.binding.clone());
        Ok(adapter)
    }
    /// Native X11 only. A Wayland desktop must keep the portal path even when
    /// DISPLAY names an XWayland server; it cannot acquire broader app rights here.
    pub(crate) fn connect() -> io::Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland") {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "wayland_requires_portal_selection"));
        }
        let (conn, screen) = RustConnection::connect(None).map_err(failed)?;
        let root = conn.setup().roots[screen].root;
        let version = conn.res_query_version(1, 2)
            .map_err(|error| io::Error::new(io::ErrorKind::Unsupported,
                format!("x11_client_identity_requires_xres_1_2: {error}")))?
            .reply().map_err(|error| io::Error::new(io::ErrorKind::Unsupported,
                format!("x11_client_identity_requires_xres_1_2: {error}")))?;
        if (version.server_major, version.server_minor) < (1, 2) {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "x11_client_identity_requires_xres_1_2"));
        }
        let atoms = Atoms::new(&conn)?;
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let property_name = format!("_SMART_EXPLORER_WINDOW_{suffix}");
        let property = conn.intern_atom(false, property_name.as_bytes())
            .map_err(failed)?.reply().map_err(failed)?.atom;
        Ok(Self { conn, root, atoms, property, property_name, markers: HashMap::new(), app_ids: HashMap::new() })
    }

    fn property_bytes(&self, window: Window, atom: Atom, kind: Atom) -> io::Result<Vec<u8>> {
        properties::bytes(&self.conn, window, atom, kind)
    }
    fn words(&self, window: Window, atom: Atom, kind: Atom) -> io::Result<Vec<u32>> {
        properties::words(&self.conn, window, atom, kind)
    }
    fn supports(&self, atom: Atom) -> io::Result<bool> {
        Ok(self.words(self.root, self.atoms.supported, AtomEnum::ATOM.into())?.contains(&atom))
    }
    fn process(&self, window: Window) -> io::Result<ProcessFacts> {
        properties::process(&self.conn, &self.atoms, window)
    }

    fn app_identity(&mut self, binding: &WindowBinding) -> io::Result<Option<String>> {
        self.validate_inner(binding)?;
        if let Some((birth, app_id)) = self.app_ids.get(&binding.native) {
            if birth == &binding.birth { return Ok(app_id.clone()); }
        }
        // The desktop-file identity is resolved once per source lifetime; P3
        // continues to refresh the icon bytes/hash independently of this key.
        let app_id = crate::screen::os::app_icons::window_identity(binding.native.0)?;
        self.validate_inner(binding)?;
        self.app_ids.insert(binding.native, (binding.birth.clone(), app_id.clone()));
        Ok(app_id)
    }

    fn candidates(&self) -> io::Result<Vec<Window>> {
        let mut windows = self.words(self.root, self.atoms.client_stacking, AtomEnum::WINDOW.into())?;
        if windows.is_empty() {
            windows = self.words(self.root, self.atoms.client_list, AtomEnum::WINDOW.into())?;
        }
        // Override-redirect menus/tooltips are omitted by _NET_CLIENT_LIST.
        let tree = self.conn.query_tree(self.root).map_err(failed)?.reply().map_err(failed)?;
        for child in &tree.children { if !windows.contains(child) { windows.push(*child); } }
        let mut ranked = Vec::with_capacity(windows.len());
        for (fallback, window) in windows.into_iter().enumerate() {
            let mut ancestor = window;
            let mut visited = HashSet::new();
            let mut rank = None;
            while visited.insert(ancestor) {
                if let Some(index) = tree.children.iter().position(|child| *child == ancestor) {
                    rank = Some(index); break;
                }
                let Ok(parent) = self.conn.query_tree(ancestor).map_err(failed)
                    .and_then(|cookie| cookie.reply().map_err(failed)) else { break; };
                if parent.parent == self.root || parent.parent == 0 { break; }
                ancestor = parent.parent;
            }
            ranked.push((rank.unwrap_or(fallback), fallback, window));
        }
        ranked.sort_by_key(|(rank, fallback, _)| (*rank, *fallback));
        Ok(ranked.into_iter().map(|(_, _, window)| window).collect())
    }

    fn validate_inner(&self, binding: &WindowBinding) -> io::Result<()> {
        let window = xid(binding.native)?;
        let process = self.process(window)?;
        let marker = self.property_bytes(window, self.property, AtomEnum::STRING.into())?;
        if marker.len() != 16 || binding.birth.get(..16) != Some(marker.as_slice())
            || binding.birth.get(16..) != Some(process.signature().as_slice()) {
            return Err(gone());
        }
        Ok(())
    }

    fn root_message(&self, window: Window, atom: Atom, words: [u32; 5]) -> io::Result<()> {
        let event = ClientMessageEvent::new(32, window, atom, words);
        self.conn.send_event(false, self.root, EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event).map_err(failed)?.check().map_err(failed)?;
        self.conn.flush().map_err(failed)
    }

    fn focused_inner(&self, binding: &WindowBinding) -> io::Result<bool> {
        self.validate_inner(binding)?;
        let target = xid(binding.native)?;
        let active = self.words(self.root, self.atoms.active, AtomEnum::WINDOW.into())?
            .first().copied().unwrap_or(0);
        let mut focus = self.conn.get_input_focus().map_err(failed)?.reply().map_err(failed)?.focus;
        let kind = properties::kind(&self.conn, &self.atoms, target)?;
        let mut active_target = target;
        if kind == WindowKind::Menu {
            let owner = self.words(target, AtomEnum::WM_TRANSIENT_FOR.into(), AtomEnum::WINDOW.into())?
                .first().copied().ok_or_else(gone)?;
            let Some(owner_binding) = self.markers.get(&NativeWindowId(u64::from(owner))) else {
                return Ok(false);
            };
            self.validate_inner(owner_binding)?;
            active_target = owner;
        }
        if active != active_target { return Ok(false); }
        let mut visited = HashSet::new();
        while focus != 0 && focus != self.root && visited.insert(focus) {
            if focus == target || (kind == WindowKind::Menu && focus == active_target) { return Ok(true); }
            focus = self.conn.query_tree(focus).map_err(failed)?.reply().map_err(failed)?.parent;
        }
        Ok(false)
    }
}

impl WindowAdapter for X11Adapter {
    fn candidate_windows(&mut self)->io::Result<Vec<(NativeWindowId,String)>> {
        let mut roots=Vec::new();
        for window in self.candidates()? {
            let attributes=match self.conn.get_window_attributes(window).map_err(failed).and_then(|c|c.reply().map_err(failed)) {Ok(value)=>value,Err(_)=>continue};
            if attributes.map_state!=MapState::VIEWABLE {continue;}
            if self.words(window,AtomEnum::WM_TRANSIENT_FOR.into(),AtomEnum::WINDOW.into())?.first().is_some_and(|owner|*owner!=0&&*owner!=self.root) {continue;}
            if matches!(properties::kind(&self.conn,&self.atoms,window)?,WindowKind::Menu|WindowKind::Tooltip) {continue;}
            if let Ok(process)=self.process(window) {roots.push((NativeWindowId(u64::from(window)),process.key()));}
        }Ok(roots)
    }
    fn process_user_matches(&mut self,binding:&WindowBinding,user_id:&str)->io::Result<bool> {
        self.validate_inner(binding)?;
        let process=self.process(xid(binding.native)?)?;
        let matches=process.key().split(':').nth(1).is_some_and(|uid|user_id==format!("linux-uid:{uid}"));
        self.validate_inner(binding)?;Ok(matches)
    }
    fn inspect(&mut self,binding:&WindowBinding)->io::Result<ObservedWindow> {
        self.validate_inner(binding)?;
        let window=xid(binding.native)?;
        let process=self.process(window)?;
        let attributes=self.conn.get_window_attributes(window).map_err(failed)?.reply().map_err(failed)?;
        let active=self.words(self.root,self.atoms.active,AtomEnum::WINDOW.into())?.first().copied().unwrap_or(0);
        let states=self.words(window,self.atoms.state,AtomEnum::ATOM.into())?;
        let observed=ObservedWindow {binding:binding.clone(),owner:None,process:process.key(),app_id:None,
            app:properties::app(&self.conn,window)?,title:properties::title(&self.conn,&self.atoms,window)?,
            kind:WindowKind::Main,geometry:properties::geometry(&self.conn,self.root,window)?,z_order:0,
            focused:active==window,visible:attributes.map_state==MapState::VIEWABLE,
            minimized:states.contains(&self.atoms.hidden),operations:WindowOperations::default()};
        self.validate_inner(binding)?;Ok(observed)
    }
    fn export_pin(&mut self, binding: &WindowBinding) -> io::Result<WindowPin> {
        self.validate_inner(binding)?;
        Ok(WindowPin { platform: PinPlatform::X11, namespace: self.property_name.clone(), binding: binding.clone() })
    }
    fn transfer_pin(&mut self, binding: &WindowBinding) {
        if self.markers.get(&binding.native) == Some(binding) { self.markers.remove(&binding.native); }
    }
    fn bind(&mut self, native: NativeWindowId) -> io::Result<WindowBinding> {
        if let Some(binding) = self.markers.get(&native).cloned() {
            if self.validate_inner(&binding).is_ok() { return Ok(binding); }
        }
        let window = xid(native)?;
        // Pin and validate in a short server transaction; no client may recycle
        // the XID between its lifetime-property check and property installation.
        self.conn.grab_server().map_err(failed)?.check().map_err(failed)?;
        let result = (|| {
            let process = self.process(window)?;
            let mut birth = vec![0u8; 16];
            getrandom::getrandom(&mut birth).map_err(|error| io::Error::other(error.to_string()))?;
            self.conn.change_property8(PropMode::REPLACE, window, self.property, AtomEnum::STRING,
                &birth).map_err(failed)?.check().map_err(failed)?;
            birth.extend_from_slice(&process.signature());
            let binding = WindowBinding { native, birth };
            self.validate_inner(&binding)?;
            Ok(binding)
        })();
        let ungrab = self.conn.ungrab_server().map_err(failed).and_then(|cookie| cookie.check().map_err(failed));
        let binding = result?;
        ungrab?;
        self.markers.insert(native, binding.clone());
        Ok(binding)
    }

    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()> { self.validate_inner(binding) }

    fn observe(&mut self, root: &WindowBinding) -> io::Result<Observation> {
        self.validate_inner(root)?;
        let root_window = xid(root.native)?;
        let root_process = self.process(root_window)?;
        let root_app_id = self.app_identity(root)?;
        let mut candidates = self.candidates()?;
        if !candidates.contains(&root_window) { candidates.push(root_window); }
        let mut facts = Vec::new();
        let mut limitations = vec!["session_wide_input_no_app_isolation".into()];
        for (index, window) in candidates.into_iter().enumerate() {
            let Ok(process) = self.process(window) else { continue; };
            if process != root_process { continue; }
            let Ok(attributes) = self.conn.get_window_attributes(window).map_err(failed)
                .and_then(|cookie| cookie.reply().map_err(failed)) else { continue; };
            if window != root_window && attributes.map_state != MapState::VIEWABLE { continue; }
            let owner = self.words(window, AtomEnum::WM_TRANSIENT_FOR.into(), AtomEnum::WINDOW.into())?
                .first().copied().filter(|value| *value != self.root && *value != window);
            let kind = properties::kind(&self.conn, &self.atoms, window)?;
            let kind = if owner.is_some() && kind == WindowKind::Main { WindowKind::Popup } else { kind };
            let geometry = properties::geometry(&self.conn, self.root, window)?;
            facts.push((window, owner, kind, geometry, index as u32, attributes.override_redirect,
                attributes.map_state == MapState::VIEWABLE));
        }
        let mut family = HashSet::from([root_window]);
        loop {
            let previous = family.len();
            for (window, owner, _, _, _, _, _) in &facts {
                if owner.is_some_and(|owner| family.contains(&owner)) { family.insert(*window); }
            }
            if family.len() == previous { break; }
        }
        let active = self.words(self.root, self.atoms.active, AtomEnum::WINDOW.into())?
            .first().copied().unwrap_or(0);
        let can_focus = self.supports(self.atoms.active)?;
        let can_resize = self.supports(self.atoms.moveresize)?;
        if !can_focus { limitations.push("x11_activation_not_supported".into()); }
        let mut windows = Vec::new();
        for (window, owner, kind, geometry, z_order, override_redirect, visible) in facts {
            if !family.contains(&window) {
                if kind != WindowKind::Main || override_redirect {
                    limitations.push(format!("unassociated_surface:{window}:requires_local_dialog_or_explicit_desktop"));
                }
                continue;
            }
            let binding = if window == root_window { root.clone() }
                else { self.bind(NativeWindowId(u64::from(window)))? };
            let protocols = self.words(window, self.atoms.protocols, AtomEnum::ATOM.into())?;
            let allowed = self.words(window, self.atoms.allowed_actions, AtomEnum::ATOM.into())?;
            let states = self.words(window, self.atoms.state, AtomEnum::ATOM.into())?;
            let interactive = !matches!(kind, WindowKind::Tooltip | WindowKind::Menu);
            windows.push(ObservedWindow { binding, owner: owner.map(|id| NativeWindowId(u64::from(id))),
                process: root_process.key(), app_id: root_app_id.clone(),
                app: properties::app(&self.conn, window)?, title: properties::title(&self.conn, &self.atoms, window)?,
                kind: if window == root_window { WindowKind::Main } else { kind }, geometry, z_order,
                focused: active == window, visible, minimized: states.contains(&self.atoms.hidden),
                operations: WindowOperations { resize: can_resize && !override_redirect
                    && allowed.contains(&self.atoms.action_resize), focus: can_focus && interactive,
                    // EWMH close permits WM fallbacks that can kill a client.
                    // Send only the application's advertised graceful protocol.
                    close: protocols.contains(&self.atoms.delete_window),
                    input: can_focus && kind != WindowKind::Tooltip } });
        }
        limitations.sort(); limitations.dedup();
        self.validate_inner(root)?;
        Ok(Observation { windows, limitations })
    }

    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        self.activate_if(binding,&||true)
    }
    fn activate_if(&mut self,binding:&WindowBinding,admitted:&dyn Fn()->bool)->io::Result<()> {
        self.validate_inner(binding)?;
        if !self.supports(self.atoms.active)? {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "x11_activation_not_supported"));
        }
        let active = self.words(self.root, self.atoms.active, AtomEnum::WINDOW.into())?
            .first().copied().unwrap_or(0);
        if !admitted() {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"app_activation_admission_ended"));}
        self.root_message(xid(binding.native)?, self.atoms.active, [2, x11rb::CURRENT_TIME, active, 0, 0])
    }

    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool> { self.focused_inner(binding) }

    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        self.conn.grab_server().map_err(failed)?.check().map_err(failed)?;
        let result = (|| {
            self.validate_inner(binding)?;
            let window = xid(binding.native)?;
            match operation {
                WindowOperation::Close => {
                    if !self.words(window, self.atoms.protocols, AtomEnum::ATOM.into())?
                        .contains(&self.atoms.delete_window) {
                        return Err(io::Error::new(io::ErrorKind::Unsupported, "x11_graceful_close_unavailable"));
                    }
                    let event = ClientMessageEvent::new(32, window, self.atoms.protocols,
                        [self.atoms.delete_window, x11rb::CURRENT_TIME, 0, 0, 0]);
                    self.conn.send_event(false, window, EventMask::NO_EVENT, event)
                        .map_err(failed)?.check().map_err(failed)?;
                }
                WindowOperation::Resize { width, height } => {
                    // EWMH 1.5: bits 10/11 select width/height; source 2 = pager.
                    if width == 0 || height == 0 || width > u16::MAX as u32 || height > u16::MAX as u32
                        || !self.supports(self.atoms.moveresize)? {
                        return Err(io::Error::new(io::ErrorKind::Unsupported, "x11_resize_unavailable"));
                    }
                    if !self.words(window, self.atoms.allowed_actions, AtomEnum::ATOM.into())?
                        .contains(&self.atoms.action_resize) {
                        return Err(io::Error::new(io::ErrorKind::Unsupported, "x11_resize_not_allowed"));
                    }
                    self.root_message(window, self.atoms.moveresize,
                        [(1 << 10) | (1 << 11) | (2 << 12), 0, 0, width, height])?;
                }
                WindowOperation::Focus => return Err(io::Error::new(io::ErrorKind::InvalidInput,
                    "use_confirmed_activation")),
            }
            self.conn.flush().map_err(failed)
        })();
        let ungrab = self.conn.ungrab_server().map_err(failed).and_then(|cookie| cookie.check().map_err(failed));
        result?; ungrab
    }

    fn disconnect(&mut self) {
        for binding in self.markers.values() {
            if self.validate_inner(binding).is_ok() {
                if let Ok(window) = xid(binding.native) {
                    if let Ok(cookie) = self.conn.delete_property(window, self.property) { let _ = cookie.check(); }
                }
            }
        }
        let _ = self.conn.flush();
        self.markers.clear();
        self.app_ids.clear();
    }
}

impl Drop for X11Adapter { fn drop(&mut self) { self.disconnect(); } }
