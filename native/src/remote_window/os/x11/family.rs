//! Verified X11 family discovery, app metadata and owner observations.
use super::*;
use x11rb::protocol::xproto::ConnectionExt as _;

impl X11Adapter {
    fn app_identity(&mut self, binding: &WindowBinding) -> io::Result<Option<String>> {
        self.validate_inner(binding)?;
        if let Some((birth, app_id)) = self.app_ids.get(&binding.native) {
            if birth == &binding.birth {
                return Ok(app_id.clone());
            }
        }
        // The desktop-file identity is resolved once per source lifetime; P3
        // continues to refresh the icon bytes/hash independently of this key.
        let app_id = crate::screen::os::app_icons::window_identity(binding.native.0)?;
        self.validate_inner(binding)?;
        self.app_ids
            .insert(binding.native, (binding.birth.clone(), app_id.clone()));
        Ok(app_id)
    }

    fn candidates(&self) -> io::Result<Vec<Window>> {
        let mut windows = self.words(
            self.root,
            self.atoms.client_stacking,
            AtomEnum::WINDOW.into(),
        )?;
        if windows.is_empty() {
            windows = self.words(self.root, self.atoms.client_list, AtomEnum::WINDOW.into())?;
        }
        // Override-redirect menus/tooltips are omitted by _NET_CLIENT_LIST.
        let tree = self
            .conn
            .query_tree(self.root)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        for child in &tree.children {
            if !windows.contains(child) {
                windows.push(*child);
            }
        }
        let mut ranked = Vec::with_capacity(windows.len());
        for (fallback, window) in windows.into_iter().enumerate() {
            let mut ancestor = window;
            let mut visited = HashSet::new();
            let mut rank = None;
            while visited.insert(ancestor) {
                if let Some(index) = tree.children.iter().position(|child| *child == ancestor) {
                    rank = Some(index);
                    break;
                }
                let Ok(parent) = self
                    .conn
                    .query_tree(ancestor)
                    .map_err(failed)
                    .and_then(|cookie| cookie.reply().map_err(failed))
                else {
                    break;
                };
                if parent.parent == self.root || parent.parent == 0 {
                    break;
                }
                ancestor = parent.parent;
            }
            ranked.push((rank.unwrap_or(fallback), fallback, window));
        }
        ranked.sort_by_key(|(rank, fallback, _)| (*rank, *fallback));
        Ok(ranked.into_iter().map(|(_, _, window)| window).collect())
    }

    pub(super) fn candidate_windows_inner(&mut self) -> io::Result<Vec<(NativeWindowId, String)>> {
        let mut roots = Vec::new();
        for window in self.candidates()? {
            let attributes = match self
                .conn
                .get_window_attributes(window)
                .map_err(failed)
                .and_then(|c| c.reply().map_err(failed))
            {
                Ok(value) => value,
                Err(_) => continue,
            };
            if attributes.map_state != MapState::VIEWABLE {
                continue;
            }
            if self
                .words(
                    window,
                    AtomEnum::WM_TRANSIENT_FOR.into(),
                    AtomEnum::WINDOW.into(),
                )?
                .first()
                .is_some_and(|owner| *owner != 0 && *owner != self.root)
            {
                continue;
            }
            if matches!(
                properties::kind(&self.conn, &self.atoms, window)?,
                WindowKind::Menu | WindowKind::Tooltip
            ) {
                continue;
            }
            if let Ok(process) = self.process(window) {
                roots.push((NativeWindowId(u64::from(window)), process.key()));
            }
        }
        Ok(roots)
    }

    pub(super) fn observe_inner(&mut self, root: &WindowBinding) -> io::Result<Observation> {
        self.validate_inner(root)?;
        let root_window = xid(root.native)?;
        let root_process = self.process(root_window)?;
        let root_app_id = self.app_identity(root)?;
        let mut candidates = self.candidates()?;
        if !candidates.contains(&root_window) {
            candidates.push(root_window);
        }
        let mut facts = Vec::new();
        let mut limitations = vec!["session_wide_input_no_app_isolation".into()];
        for (index, window) in candidates.into_iter().enumerate() {
            let Ok(process) = self.process(window) else {
                continue;
            };
            if process != root_process {
                continue;
            }
            let Ok(attributes) = self
                .conn
                .get_window_attributes(window)
                .map_err(failed)
                .and_then(|cookie| cookie.reply().map_err(failed))
            else {
                continue;
            };
            if window != root_window && attributes.map_state != MapState::VIEWABLE {
                continue;
            }
            let owner = self
                .words(
                    window,
                    AtomEnum::WM_TRANSIENT_FOR.into(),
                    AtomEnum::WINDOW.into(),
                )?
                .first()
                .copied()
                .filter(|value| *value != self.root && *value != window);
            let kind = properties::kind(&self.conn, &self.atoms, window)?;
            let kind = if owner.is_some() && kind == WindowKind::Main {
                WindowKind::Popup
            } else {
                kind
            };
            let geometry = properties::geometry(&self.conn, self.root, window)?;
            facts.push((
                window,
                owner,
                kind,
                geometry,
                index as u32,
                attributes.override_redirect,
                attributes.map_state == MapState::VIEWABLE,
            ));
        }
        let mut family = HashSet::from([root_window]);
        loop {
            let previous = family.len();
            for (window, owner, _, _, _, _, _) in &facts {
                if owner.is_some_and(|owner| family.contains(&owner)) {
                    family.insert(*window);
                }
            }
            if family.len() == previous {
                break;
            }
        }
        let active = self
            .words(self.root, self.atoms.active, AtomEnum::WINDOW.into())?
            .first()
            .copied()
            .unwrap_or(0);
        let can_focus = self.supports(self.atoms.active)?;
        let can_resize = self.supports(self.atoms.moveresize)?;
        if !can_focus {
            limitations.push("x11_activation_not_supported".into());
        }
        let mut windows = Vec::new();
        for (window, owner, kind, geometry, z_order, override_redirect, visible) in facts {
            if !family.contains(&window) {
                if kind != WindowKind::Main || override_redirect {
                    limitations.push(format!(
                        "unassociated_surface:{window}:requires_local_dialog_or_explicit_desktop"
                    ));
                }
                continue;
            }
            let binding = if window == root_window {
                root.clone()
            } else {
                self.bind(NativeWindowId(u64::from(window)))?
            };
            let protocols = self.words(window, self.atoms.protocols, AtomEnum::ATOM.into())?;
            let allowed = self.words(window, self.atoms.allowed_actions, AtomEnum::ATOM.into())?;
            let states = self.words(window, self.atoms.state, AtomEnum::ATOM.into())?;
            let interactive = focus::can_activate(kind, override_redirect);
            windows.push(ObservedWindow {
                binding,
                owner: owner.map(|id| NativeWindowId(u64::from(id))),
                process: root_process.key(),
                app_id: root_app_id.clone(),
                app: properties::app(&self.conn, window)?,
                title: properties::title(&self.conn, &self.atoms, window)?,
                kind: if window == root_window {
                    WindowKind::Main
                } else {
                    kind
                },
                geometry,
                z_order,
                focused: active == window,
                visible,
                minimized: states.contains(&self.atoms.hidden),
                operations: WindowOperations {
                    resize: can_resize
                        && !override_redirect
                        && allowed.contains(&self.atoms.action_resize),
                    focus: can_focus && interactive,
                    // EWMH close permits WM fallbacks that can kill a client.
                    // Send only the application's advertised graceful protocol.
                    close: protocols.contains(&self.atoms.delete_window),
                    input: can_focus && kind != WindowKind::Tooltip,
                },
            });
        }
        limitations.sort();
        limitations.dedup();
        self.validate_inner(root)?;
        Ok(Observation {
            windows,
            limitations,
        })
    }
}
