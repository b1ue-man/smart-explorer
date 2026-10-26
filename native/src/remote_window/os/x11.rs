//! X11 0.13: private lifetime properties survive neither DestroyWindow nor a
//! server/session reconnect. Family membership requires transient+process proof.
use crate::remote_window::core::host::WindowAdapter;
use crate::remote_window::core::model::*;
use std::collections::{HashMap, HashSet};
use std::io;
use x11rb::connection::Connection;
use x11rb::protocol::res::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, MapState, PropMode, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

mod family;
mod focus;
mod input;
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

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("x11: {error}"))
}
fn gone() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "original_x11_window_gone")
}
fn xid(id: NativeWindowId) -> io::Result<Window> {
    u32::try_from(id.0)
        .ok()
        .filter(|v| *v != 0)
        .ok_or_else(gone)
}

impl X11Adapter {
    pub(crate) fn adopt(pin: &WindowPin) -> io::Result<Self> {
        if pin.platform != PinPlatform::X11
            || !pin.namespace.starts_with("_SMART_EXPLORER_WINDOW_")
            || pin.namespace.len() > 128
            || !pin.namespace.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid_x11_pin_namespace",
            ));
        }
        let mut adapter = Self::connect()?;
        adapter.property = adapter
            .conn
            .intern_atom(true, pin.namespace.as_bytes())
            .map_err(failed)?
            .reply()
            .map_err(failed)?
            .atom;
        if adapter.property == 0 {
            return Err(gone());
        }
        adapter.property_name = pin.namespace.clone();
        adapter.validate_inner(&pin.binding)?;
        adapter
            .markers
            .insert(pin.binding.native, pin.binding.clone());
        Ok(adapter)
    }
    /// Native X11 only. A Wayland desktop must keep the portal path even when
    /// DISPLAY names an XWayland server; it cannot acquire broader app rights here.
    pub(crate) fn connect() -> io::Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "wayland_requires_portal_selection",
            ));
        }
        let (conn, screen) = RustConnection::connect(None).map_err(failed)?;
        let root = conn.setup().roots[screen].root;
        let version = conn
            .res_query_version(1, 2)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("x11_client_identity_requires_xres_1_2: {error}"),
                )
            })?
            .reply()
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("x11_client_identity_requires_xres_1_2: {error}"),
                )
            })?;
        if (version.server_major, version.server_minor) < (1, 2) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "x11_client_identity_requires_xres_1_2",
            ));
        }
        let atoms = Atoms::new(&conn)?;
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let property_name = format!("_SMART_EXPLORER_WINDOW_{suffix}");
        let property = conn
            .intern_atom(false, property_name.as_bytes())
            .map_err(failed)?
            .reply()
            .map_err(failed)?
            .atom;
        Ok(Self {
            conn,
            root,
            atoms,
            property,
            property_name,
            markers: HashMap::new(),
            app_ids: HashMap::new(),
        })
    }

    fn property_bytes(&self, window: Window, atom: Atom, kind: Atom) -> io::Result<Vec<u8>> {
        properties::bytes(&self.conn, window, atom, kind)
    }
    fn words(&self, window: Window, atom: Atom, kind: Atom) -> io::Result<Vec<u32>> {
        properties::words(&self.conn, window, atom, kind)
    }
    fn supports(&self, atom: Atom) -> io::Result<bool> {
        Ok(self
            .words(self.root, self.atoms.supported, AtomEnum::ATOM.into())?
            .contains(&atom))
    }
    fn process(&self, window: Window) -> io::Result<ProcessFacts> {
        properties::process(&self.conn, &self.atoms, window)
    }

    fn validate_inner(&self, binding: &WindowBinding) -> io::Result<()> {
        self.validated_process(binding).map(|_| ())
    }

    fn validated_process(&self, binding: &WindowBinding) -> io::Result<ProcessFacts> {
        let window = xid(binding.native)?;
        let process = self.process(window)?;
        let marker = self.property_bytes(window, self.property, AtomEnum::STRING.into())?;
        if marker.len() != 16
            || binding.birth.get(..16) != Some(marker.as_slice())
            || binding.birth.get(16..) != Some(process.signature().as_slice())
        {
            return Err(gone());
        }
        Ok(process)
    }

}

impl WindowAdapter for X11Adapter {
    fn candidate_windows(&mut self) -> io::Result<Vec<(NativeWindowId, String)>> {
        self.candidate_windows_inner()
    }
    fn process_user_matches(&mut self, binding: &WindowBinding, user_id: &str) -> io::Result<bool> {
        self.validate_inner(binding)?;
        let process = self.process(xid(binding.native)?)?;
        let matches = process
            .key()
            .split(':')
            .nth(1)
            .is_some_and(|uid| user_id == format!("linux-uid:{uid}"));
        self.validate_inner(binding)?;
        Ok(matches)
    }
    fn inspect(&mut self, binding: &WindowBinding) -> io::Result<ObservedWindow> {
        self.validate_inner(binding)?;
        let window = xid(binding.native)?;
        let process = self.process(window)?;
        let attributes = self
            .conn
            .get_window_attributes(window)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        let active = self
            .words(self.root, self.atoms.active, AtomEnum::WINDOW.into())?
            .first()
            .copied()
            .unwrap_or(0);
        let states = self.words(window, self.atoms.state, AtomEnum::ATOM.into())?;
        let observed = ObservedWindow {
            binding: binding.clone(),
            owner: None,
            process: process.key(),
            app_id: None,
            app: properties::app(&self.conn, window)?,
            title: properties::title(&self.conn, &self.atoms, window)?,
            kind: WindowKind::Main,
            geometry: properties::geometry(&self.conn, self.root, window)?,
            z_order: 0,
            focused: active == window,
            visible: attributes.map_state == MapState::VIEWABLE,
            minimized: states.contains(&self.atoms.hidden),
            operations: WindowOperations::default(),
        };
        self.validate_inner(binding)?;
        Ok(observed)
    }
    fn export_pin(&mut self, binding: &WindowBinding) -> io::Result<WindowPin> {
        self.validate_inner(binding)?;
        Ok(WindowPin {
            platform: PinPlatform::X11,
            namespace: self.property_name.clone(),
            binding: binding.clone(),
        })
    }
    fn transfer_pin(&mut self, binding: &WindowBinding) {
        if self.markers.get(&binding.native) == Some(binding) {
            self.markers.remove(&binding.native);
        }
    }
    fn bind(&mut self, native: NativeWindowId) -> io::Result<WindowBinding> {
        if let Some(binding) = self.markers.get(&native).cloned() {
            if self.validate_inner(&binding).is_ok() {
                return Ok(binding);
            }
        }
        let window = xid(native)?;
        // Pin and validate in a short server transaction; no client may recycle
        // the XID between its lifetime-property check and property installation.
        self.conn
            .grab_server()
            .map_err(failed)?
            .check()
            .map_err(failed)?;
        let result = (|| {
            let process = self.process(window)?;
            let mut birth = vec![0u8; 16];
            getrandom::getrandom(&mut birth)
                .map_err(|error| io::Error::other(error.to_string()))?;
            self.conn
                .change_property8(
                    PropMode::REPLACE,
                    window,
                    self.property,
                    AtomEnum::STRING,
                    &birth,
                )
                .map_err(failed)?
                .check()
                .map_err(failed)?;
            birth.extend_from_slice(&process.signature());
            let binding = WindowBinding { native, birth };
            self.validate_inner(&binding)?;
            Ok(binding)
        })();
        let ungrab = self
            .conn
            .ungrab_server()
            .map_err(failed)
            .and_then(|cookie| cookie.check().map_err(failed));
        let binding = result?;
        ungrab?;
        self.markers.insert(native, binding.clone());
        Ok(binding)
    }

    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        self.validate_inner(binding)
    }

    fn observe(&mut self, root: &WindowBinding) -> io::Result<Observation> {
        self.observe_inner(root)
    }

    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        self.activate_if(binding, &|| true)
    }
    fn activate_if(
        &mut self,
        binding: &WindowBinding,
        admitted: &dyn Fn() -> bool,
    ) -> io::Result<()> {
        self.activate_if_inner(binding, admitted)
    }

    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool> {
        self.focused_inner(binding)
    }

    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        self.request_inner(binding, operation)
    }

    fn disconnect(&mut self) {
        for binding in self.markers.values() {
            if self.validate_inner(binding).is_ok() {
                if let Ok(window) = xid(binding.native) {
                    if let Ok(cookie) = self.conn.delete_property(window, self.property) {
                        let _ = cookie.check();
                    }
                }
            }
        }
        let _ = self.conn.flush();
        self.markers.clear();
        self.app_ids.clear();
    }
}

impl Drop for X11Adapter {
    fn drop(&mut self) {
        self.disconnect();
    }
}

#[cfg(test)]
mod batches_popup_focus_task_tests;
