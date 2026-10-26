//! Verified family snapshots, capture boundaries and icon metadata.
use super::*;

impl<A: WindowAdapter> HostSession<A> {
    pub(crate) fn refresh(&mut self, authority: &dyn WindowAuthority) -> io::Result<FamilyUpdate> {
        self.admitted(authority, false)?;
        self.adapter.validate(&self.root)?;
        let observed = self.adapter.observe(&self.root)?;
        let root = observed
            .windows
            .iter()
            .find(|w| w.binding == self.root)
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "original_window_gone"))?;
        if !authority.in_list_scope(root.app_id.as_deref()) {
            return Err(refused(
                io::ErrorKind::PermissionDenied,
                "window_outside_list_scope",
            ));
        }
        let process = root.process.clone();
        let mut accepted = vec![self.root.native];
        loop {
            let previous = accepted.len();
            for window in &observed.windows {
                if window.process == process
                    && !accepted.contains(&window.binding.native)
                    && window.owner.is_some_and(|owner| accepted.contains(&owner))
                    && authority.in_list_scope(window.app_id.as_deref())
                {
                    accepted.push(window.binding.native);
                }
            }
            if accepted.len() == previous {
                break;
            }
        }
        let mut bindings = HashMap::new();
        for window in &observed.windows {
            if !accepted.contains(&window.binding.native) || !window.geometry.valid() {
                continue;
            }
            let identity = match self.bindings.get(&window.binding.native) {
                Some((binding, identity)) if binding == &window.binding => identity.clone(),
                _ => {
                    let generation = self.next_generation;
                    self.next_generation = generation.checked_add(1).ok_or_else(|| {
                        refused(io::ErrorKind::InvalidData, "window_generation_exhausted")
                    })?;
                    WindowIdentity {
                        session: self.snapshot.session.clone(),
                        generation,
                    }
                }
            };
            bindings.insert(window.binding.native, (window.binding.clone(), identity));
        }
        let mut members = Vec::new();
        for window in observed.windows {
            let Some((_, identity)) = bindings.get(&window.binding.native) else {
                continue;
            };
            let control = authority.may_control(&self.session);
            let operations = if control {
                window.operations
            } else {
                WindowOperations::default()
            };
            let previous = self
                .snapshot
                .members
                .iter()
                .find(|m| &m.identity == identity);
            let app_id = window.app_id;
            let old_icon = previous
                .filter(|m| m.app_id == app_id)
                .and_then(|m| m.icon.clone());
            members.push(FamilyMember {
                identity: identity.clone(),
                owner: if window.binding == self.root {
                    None
                } else {
                    window
                        .owner
                        .and_then(|id| bindings.get(&id).map(|(_, i)| i.clone()))
                },
                app_id,
                app: window.app,
                title: window.title,
                icon: old_icon,
                kind: window.kind,
                geometry: window.geometry,
                z_order: window.z_order,
                focused: window.focused,
                visible: window.visible,
                minimized: window.minimized,
                operations,
            });
        }
        if !members.iter().any(|m| m.identity == self.snapshot.root) {
            return Err(refused(
                io::ErrorKind::NotFound,
                "original_window_unavailable",
            ));
        }
        if members
            .iter()
            .any(|member| !authority.in_list_scope(member.app_id.as_deref()))
        {
            return Err(refused(
                io::ErrorKind::PermissionDenied,
                "window_family_scope_changed",
            ));
        }
        if !authority.may_control(&self.session) {
            for member in &mut members {
                member.operations = WindowOperations::default();
            }
            self.input_armed = false;
        }
        members.sort_by_key(|m| m.identity.generation);
        let focused = |members: &[FamilyMember]| {
            members
                .iter()
                .filter(|m| m.focused)
                .map(|m| m.identity.clone())
                .collect::<Vec<_>>()
        };
        if focused(&members) != focused(&self.snapshot.members) {
            self.input_armed = false;
            self.input_epoch = self.input_epoch.checked_add(1).ok_or_else(|| {
                refused(io::ErrorKind::InvalidData, "window_input_epoch_exhausted")
            })?;
        }
        self.admitted(authority, false)?;
        let previous_revision = self.snapshot.revision;
        let layout_changed = members.len() != self.snapshot.members.len()
            || members
                .iter()
                .zip(&self.snapshot.members)
                .any(|(new, old)| {
                    new.identity != old.identity
                        || new.owner != old.owner
                        || new.geometry != old.geometry
                        || new.z_order != old.z_order
                        || new.visible != old.visible
                        || new.minimized != old.minimized
                });
        let removed = self
            .snapshot
            .members
            .iter()
            .filter(|old| !members.iter().any(|new| new.identity == old.identity))
            .map(|m| m.identity.clone())
            .collect();
        if members != self.snapshot.members || observed.limitations != self.snapshot.limitations {
            self.snapshot.revision = previous_revision
                .checked_add(1)
                .ok_or_else(|| refused(io::ErrorKind::InvalidData, "window_revision_exhausted"))?;
            self.snapshot.members = members;
            self.snapshot.limitations = observed.limitations;
        }
        if layout_changed {
            self.snapshot.layout_revision = self
                .snapshot
                .layout_revision
                .checked_add(1)
                .ok_or_else(|| {
                    refused(
                        io::ErrorKind::InvalidData,
                        "window_layout_revision_exhausted",
                    )
                })?;
        }
        self.bindings = bindings;
        self.snapshot.validate()?;
        Ok(FamilyUpdate {
            previous_revision,
            snapshot: self.snapshot.clone(),
            removed,
        })
    }

    pub(crate) fn boundary(
        &mut self,
        identity: &WindowIdentity,
        authority: &dyn WindowAuthority,
    ) -> io::Result<CaptureBoundary> {
        self.refresh(authority)?;
        self.target(identity)?;
        let mut members = Vec::new();
        for member in &self.snapshot.members {
            if member.minimized || !member.visible {
                continue;
            }
            let (binding, _) = self.target(&member.identity)?;
            self.adapter.validate(&binding)?;
            members.push(CaptureMember {
                identity: member.identity.clone(),
                binding,
                geometry: member.geometry,
                z_order: member.z_order,
                input: member.operations.input,
            });
        }
        Ok(CaptureBoundary {
            session: self.snapshot.session.clone(),
            revision: self.snapshot.layout_revision,
            members,
        })
    }

    /// Call after acquiring member surfaces, before publishing pixels. On false,
    /// discard the frame and invalidate the encoder reference before recapture.
    pub(crate) fn boundary_current(
        &mut self,
        boundary: &CaptureBoundary,
        authority: &dyn WindowAuthority,
    ) -> io::Result<bool> {
        let identity = self.snapshot.root.clone();
        Ok(self.boundary(&identity, authority)? == *boundary)
    }

    /// The icon adapter supplies only references from its freshly authorized
    /// platform list. A missing icon is a normal neutral-fallback condition.
    pub(crate) fn set_icon(
        &mut self,
        identity: &WindowIdentity,
        icon: Option<crate::screen::core::app_icon::IconRef>,
        authority: &dyn WindowAuthority,
    ) -> io::Result<()> {
        self.refresh(authority)?;
        let (binding, _) = self.target(identity)?;
        self.adapter.validate(&binding)?;
        if let Some(icon) = &icon {
            icon.validate()?;
        }
        let member = self
            .snapshot
            .members
            .iter_mut()
            .find(|m| &m.identity == identity)
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "stale_window_generation"))?;
        if let Some(icon) = &icon {
            if member.app_id.as_deref() != Some(icon.app_id.as_str()) {
                return Err(refused(
                    io::ErrorKind::InvalidData,
                    "window_icon_app_mismatch",
                ));
            }
        }
        if member.icon != icon {
            member.icon = icon;
            self.snapshot.revision =
                self.snapshot.revision.checked_add(1).ok_or_else(|| {
                    refused(io::ErrorKind::InvalidData, "window_revision_exhausted")
                })?;
        }
        Ok(())
    }
}
