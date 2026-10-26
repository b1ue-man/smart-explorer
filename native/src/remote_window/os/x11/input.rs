//! X11 activation confirmation and graceful window-operation requests.
use super::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

impl X11Adapter {
    fn root_message(&self, window: Window, atom: Atom, words: [u32; 5]) -> io::Result<()> {
        let event = ClientMessageEvent::new(32, window, atom, words);
        self.conn
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                event,
            )
            .map_err(failed)?
            .check()
            .map_err(failed)?;
        self.conn.flush().map_err(failed)
    }

    pub(super) fn focused_inner(&self, binding: &WindowBinding) -> io::Result<bool> {
        // Read owners, lifetime markers, WM activation and actual input focus
        // in one short server transaction. XTest remains session-wide input.
        self.conn
            .grab_server()
            .map_err(failed)?
            .check()
            .map_err(failed)?;
        let result = (|| {
            self.validate_inner(binding)?;
            let active = self
                .words(self.root, self.atoms.active, AtomEnum::WINDOW.into())?
                .first()
                .copied()
                .unwrap_or(0);
            let input = self
                .conn
                .get_input_focus()
                .map_err(failed)?
                .reply()
                .map_err(failed)?
                .focus;
            focus::confirmed(
                binding,
                active,
                input,
                self.root,
                |window| self.focus_node(window),
                |window| {
                    Ok(self
                        .conn
                        .query_tree(window)
                        .map_err(failed)?
                        .reply()
                        .map_err(failed)?
                        .parent)
                },
            )
        })();
        let ungrab = self
            .conn
            .ungrab_server()
            .map_err(failed)
            .and_then(|cookie| cookie.check().map_err(failed));
        let confirmed = result?;
        ungrab?;
        Ok(confirmed)
    }

    fn focus_node(&self, window: Window) -> io::Result<Option<focus::Node>> {
        // Focus traversal must never bind a new XID or trust a PID/type alone.
        let Some(binding) = self.markers.get(&NativeWindowId(u64::from(window))) else {
            return Ok(None);
        };
        let process = self.validated_process(binding)?;
        let attributes = self
            .conn
            .get_window_attributes(window)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        let kind = properties::kind(&self.conn, &self.atoms, window)?;
        let states = self.words(window, self.atoms.state, AtomEnum::ATOM.into())?;
        let owner = self
            .words(
                window,
                AtomEnum::WM_TRANSIENT_FOR.into(),
                AtomEnum::WINDOW.into(),
            )?
            .first()
            .copied()
            .filter(|owner| *owner > 1 && *owner != self.root);
        Ok(Some(focus::Node {
            binding: binding.clone(),
            process: process.signature(),
            owner,
            owner_focus: focus::owner_focus(kind, attributes.override_redirect),
            eligible: kind != WindowKind::Tooltip
                && attributes.map_state == MapState::VIEWABLE
                && !states.contains(&self.atoms.hidden),
        }))
    }

    pub(super) fn activate_if_inner(
        &mut self,
        binding: &WindowBinding,
        admitted: &dyn Fn() -> bool,
    ) -> io::Result<()> {
        self.validate_inner(binding)?;
        if !self.supports(self.atoms.active)? {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "x11_activation_not_supported",
            ));
        }
        let active = self
            .words(self.root, self.atoms.active, AtomEnum::WINDOW.into())?
            .first()
            .copied()
            .unwrap_or(0);
        // An explicit confirmation may retain a live popup's input focus. Do
        // not ask the WM to reactivate an already confirmed regular owner.
        let already_focused = active == xid(binding.native)? && self.focused_inner(binding)?;
        if !admitted() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "app_activation_admission_ended",
            ));
        }
        if already_focused {
            return Ok(());
        }
        self.root_message(
            xid(binding.native)?,
            self.atoms.active,
            [2, x11rb::CURRENT_TIME, active, 0, 0],
        )
    }

    pub(super) fn request_inner(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        self.conn
            .grab_server()
            .map_err(failed)?
            .check()
            .map_err(failed)?;
        let result = (|| {
            self.validate_inner(binding)?;
            let window = xid(binding.native)?;
            match operation {
                WindowOperation::Close => {
                    if !self
                        .words(window, self.atoms.protocols, AtomEnum::ATOM.into())?
                        .contains(&self.atoms.delete_window)
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "x11_graceful_close_unavailable",
                        ));
                    }
                    let event = ClientMessageEvent::new(
                        32,
                        window,
                        self.atoms.protocols,
                        [self.atoms.delete_window, x11rb::CURRENT_TIME, 0, 0, 0],
                    );
                    self.conn
                        .send_event(false, window, EventMask::NO_EVENT, event)
                        .map_err(failed)?
                        .check()
                        .map_err(failed)?;
                }
                WindowOperation::Resize { width, height } => {
                    // EWMH 1.5: bits 10/11 select width/height; source 2 = pager.
                    if width == 0
                        || height == 0
                        || width > u16::MAX as u32
                        || height > u16::MAX as u32
                        || !self.supports(self.atoms.moveresize)?
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "x11_resize_unavailable",
                        ));
                    }
                    if !self
                        .words(window, self.atoms.allowed_actions, AtomEnum::ATOM.into())?
                        .contains(&self.atoms.action_resize)
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "x11_resize_not_allowed",
                        ));
                    }
                    self.root_message(
                        window,
                        self.atoms.moveresize,
                        [(1 << 10) | (1 << 11) | (2 << 12), 0, 0, width, height],
                    )?;
                }
                WindowOperation::Focus => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "use_confirmed_activation",
                    ))
                }
            }
            self.conn.flush().map_err(failed)
        })();
        let ungrab = self
            .conn
            .ungrab_server()
            .map_err(failed)
            .and_then(|cookie| cookie.check().map_err(failed));
        result?;
        ungrab
    }
}
