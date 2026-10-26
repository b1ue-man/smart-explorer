//! A host session owns the original bindings until disconnect, never the app.
use std::collections::HashMap;
use std::io;
use super::model::*;

fn refused(kind: io::ErrorKind, text: &str) -> io::Error { io::Error::new(kind, text) }

/// Implemented at the existing authorization boundary. Every method is evaluated
/// freshly, including after a blocking OS operation. Control remains a session grant.
pub(crate) trait WindowAuthority {
    fn may_view(&self, session: &str) -> bool;
    fn may_control(&self, session: &str) -> bool;
    fn in_list_scope(&self, app_id: Option<&str>) -> bool;
}

pub(crate) trait WindowAdapter: Send {
    /// Host-only candidates. A candidate is not a binding or an authorization.
    fn candidate_windows(&mut self) -> io::Result<Vec<(NativeWindowId,String)>> {
        Err(io::Error::new(io::ErrorKind::Unsupported,"window_candidates_unavailable"))
    }
    /// Fresh OS identity, including the actual user/logon where the platform
    /// exposes it. Display labels and client-supplied PID properties do not count.
    fn process_user_matches(&mut self,_binding:&WindowBinding,_user_id:&str)->io::Result<bool> {Ok(false)}
    fn bind(&mut self, native: NativeWindowId) -> io::Result<WindowBinding>;
    fn inspect(&mut self, binding: &WindowBinding) -> io::Result<ObservedWindow> {
        self.observe(binding)?.windows.into_iter().find(|window| window.binding == *binding)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound,"original_window_gone"))
    }
    /// Verify root's lifetime marker first. Return only verified owner descendants.
    fn observe(&mut self, root: &WindowBinding) -> io::Result<Observation>;
    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()>;
    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()>;
    fn activate_if(&mut self,binding:&WindowBinding,admitted:&dyn Fn()->bool)->io::Result<()> {
        if !admitted() {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"app_activation_admission_ended"));}
        self.activate(binding)
    }
    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool>;
    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()>;
    fn export_pin(&mut self, _binding: &WindowBinding) -> io::Result<WindowPin> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "window_pin_transfer_unavailable"))
    }
    /// Relinquish cleanup only after the receiving worker confirmed adoption.
    fn transfer_pin(&mut self, _binding: &WindowBinding) {}
    /// Remove only this adapter's markers. Never close/kill the application.
    fn disconnect(&mut self) {}
}

impl<T: WindowAdapter + ?Sized> WindowAdapter for Box<T> {
    fn candidate_windows(&mut self)->io::Result<Vec<(NativeWindowId,String)>> {(**self).candidate_windows()}
    fn process_user_matches(&mut self,binding:&WindowBinding,user_id:&str)->io::Result<bool> {(**self).process_user_matches(binding,user_id)}
    fn bind(&mut self, native: NativeWindowId) -> io::Result<WindowBinding> { (**self).bind(native) }
    fn inspect(&mut self, binding: &WindowBinding) -> io::Result<ObservedWindow> { (**self).inspect(binding) }
    fn observe(&mut self, root: &WindowBinding) -> io::Result<Observation> { (**self).observe(root) }
    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()> { (**self).validate(binding) }
    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()> { (**self).activate(binding) }
    fn activate_if(&mut self,binding:&WindowBinding,admitted:&dyn Fn()->bool)->io::Result<()> {(**self).activate_if(binding,admitted)}
    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool> { (**self).focused(binding) }
    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        (**self).request(binding, operation)
    }
    fn disconnect(&mut self) { (**self).disconnect(); }
    fn export_pin(&mut self, binding: &WindowBinding) -> io::Result<WindowPin> { (**self).export_pin(binding) }
    fn transfer_pin(&mut self, binding: &WindowBinding) { (**self).transfer_pin(binding); }
}

pub(crate) struct HostSession<A: WindowAdapter> {
    adapter: A,
    root: WindowBinding,
    session: String,
    next_generation: u64,
    bindings: HashMap<NativeWindowId, (WindowBinding, WindowIdentity)>,
    snapshot: FamilySnapshot,
    connected: bool,
    input_armed: bool,
    /// Observed focus transitions are independent of capture layout changes.
    input_epoch: u64,
}

impl<A: WindowAdapter> HostSession<A> {
    /// `session` is the existing authorized screen session. Each source lifetime
    /// receives an additional host-generated nonce, including reconnects.
    pub(crate) fn open(session: String, native: NativeWindowId, mut adapter: A,
        authority: &dyn WindowAuthority) -> io::Result<Self> {
        if session.is_empty() || !authority.may_view(&session) {
            return Err(refused(io::ErrorKind::PermissionDenied, "window_view_not_authorized"));
        }
        let root = adapter.bind(native)?;
        Self::open_bound(session, root, adapter, authority)
    }

    /// Selection from a generation-bearing list must use its existing host
    /// binding. A stale HWND/XID fails here; it is never silently bound anew.
    /// The adapter must be the owner of that marker's namespace/lifetime.
    pub(crate) fn open_bound(session: String, root: WindowBinding, adapter: A,
        authority: &dyn WindowAuthority) -> io::Result<Self> {
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let lifetime: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        Self::open_bound_as(session, root, adapter, authority, lifetime)
    }

    pub(crate) fn open_bound_as(session: String, root: WindowBinding, mut adapter: A,
        authority: &dyn WindowAuthority, lifetime: String) -> io::Result<Self> {
        if session.is_empty() || !authority.may_view(&session) {
            return Err(refused(io::ErrorKind::PermissionDenied, "window_view_not_authorized"));
        }
        adapter.validate(&root)?;
        if lifetime.len() != 32 || !lifetime.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(refused(io::ErrorKind::InvalidData, "invalid_window_family_lifetime"));
        }
        let identity = WindowIdentity { session: lifetime.clone(), generation: 1 };
        let snapshot = FamilySnapshot { version: FAMILY_VERSION, session: lifetime, revision: 0,layout_revision:0,
            root: identity.clone(), members: Vec::new(), limitations: Vec::new() };
        let mut host = Self { adapter, root: root.clone(), session, next_generation: 2,
            bindings: HashMap::from([(root.native, (root, identity))]), snapshot, connected: true, input_armed: false,input_epoch:0 };
        host.refresh(authority)?;
        Ok(host)
    }

    fn admitted(&self, authority: &dyn WindowAuthority, control: bool) -> io::Result<()> {
        if !self.connected { return Err(refused(io::ErrorKind::NotConnected, "window_disconnected")); }
        if !authority.may_view(&self.session) || (control && !authority.may_control(&self.session)) {
            return Err(refused(io::ErrorKind::PermissionDenied, "window_permission_revoked"));
        }
        Ok(())
    }

    pub(crate) fn refresh(&mut self, authority: &dyn WindowAuthority) -> io::Result<FamilyUpdate> {
        self.admitted(authority, false)?;
        self.adapter.validate(&self.root)?;
        let observed = self.adapter.observe(&self.root)?;
        let root = observed.windows.iter().find(|w| w.binding == self.root)
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "original_window_gone"))?;
        if !authority.in_list_scope(root.app_id.as_deref()) {
            return Err(refused(io::ErrorKind::PermissionDenied, "window_outside_list_scope"));
        }
        let process = root.process.clone();
        let mut accepted = vec![self.root.native];
        loop {
            let previous = accepted.len();
            for window in &observed.windows {
                if window.process == process && !accepted.contains(&window.binding.native)
                    && window.owner.is_some_and(|owner| accepted.contains(&owner))
                    && authority.in_list_scope(window.app_id.as_deref()) {
                    accepted.push(window.binding.native);
                }
            }
            if accepted.len() == previous { break; }
        }
        let mut bindings = HashMap::new();
        for window in &observed.windows {
            if !accepted.contains(&window.binding.native) || !window.geometry.valid() { continue; }
            let identity = match self.bindings.get(&window.binding.native) {
                Some((binding, identity)) if binding == &window.binding => identity.clone(),
                _ => {
                    let generation = self.next_generation;
                    self.next_generation = generation.checked_add(1)
                        .ok_or_else(|| refused(io::ErrorKind::InvalidData, "window_generation_exhausted"))?;
                    WindowIdentity { session: self.snapshot.session.clone(), generation }
                }
            };
            bindings.insert(window.binding.native, (window.binding.clone(), identity));
        }
        let mut members = Vec::new();
        for window in observed.windows {
            let Some((_, identity)) = bindings.get(&window.binding.native) else { continue; };
            let control = authority.may_control(&self.session);
            let operations = if control { window.operations } else { WindowOperations::default() };
            let previous = self.snapshot.members.iter().find(|m| &m.identity == identity);
            let app_id = window.app_id;
            let old_icon = previous.filter(|m| m.app_id == app_id).and_then(|m| m.icon.clone());
            members.push(FamilyMember { identity: identity.clone(),
                owner: if window.binding == self.root { None } else {
                    window.owner.and_then(|id| bindings.get(&id).map(|(_, i)| i.clone())) },
                app_id, app: window.app, title: window.title, icon: old_icon,
                kind: window.kind, geometry: window.geometry, z_order: window.z_order,
                focused: window.focused, visible: window.visible, minimized: window.minimized, operations });
        }
        if !members.iter().any(|m| m.identity == self.snapshot.root) {
            return Err(refused(io::ErrorKind::NotFound, "original_window_unavailable"));
        }
        if members.iter().any(|member| !authority.in_list_scope(member.app_id.as_deref())) {
            return Err(refused(io::ErrorKind::PermissionDenied, "window_family_scope_changed"));
        }
        if !authority.may_control(&self.session) {
            for member in &mut members { member.operations = WindowOperations::default(); }
            self.input_armed = false;
        }
        members.sort_by_key(|m| m.identity.generation);
        let focused=|members:&[FamilyMember]|members.iter().filter(|m|m.focused)
            .map(|m|m.identity.clone()).collect::<Vec<_>>();
        if focused(&members)!=focused(&self.snapshot.members) {
            self.input_armed=false;
            self.input_epoch=self.input_epoch.checked_add(1)
                .ok_or_else(||refused(io::ErrorKind::InvalidData,"window_input_epoch_exhausted"))?;
        }
        self.admitted(authority, false)?;
        let previous_revision = self.snapshot.revision;
        let layout_changed=members.len()!=self.snapshot.members.len() || members.iter().zip(&self.snapshot.members)
            .any(|(new,old)|new.identity!=old.identity || new.owner!=old.owner || new.geometry!=old.geometry
                || new.z_order!=old.z_order || new.visible!=old.visible || new.minimized!=old.minimized);
        let removed = self.snapshot.members.iter().filter(|old|
            !members.iter().any(|new| new.identity == old.identity)).map(|m| m.identity.clone()).collect();
        if members != self.snapshot.members || observed.limitations != self.snapshot.limitations {
            self.snapshot.revision = previous_revision.checked_add(1)
                .ok_or_else(|| refused(io::ErrorKind::InvalidData, "window_revision_exhausted"))?;
            self.snapshot.members = members;
            self.snapshot.limitations = observed.limitations;
        }
        if layout_changed {
            self.snapshot.layout_revision=self.snapshot.layout_revision.checked_add(1)
                .ok_or_else(||refused(io::ErrorKind::InvalidData,"window_layout_revision_exhausted"))?;
        }
        self.bindings = bindings;
        self.snapshot.validate()?;
        Ok(FamilyUpdate { previous_revision, snapshot: self.snapshot.clone(), removed })
    }

    /// A full re-fetch always refreshes platform facts and current permissions.
    pub(crate) fn snapshot(&mut self, authority: &dyn WindowAuthority) -> io::Result<FamilySnapshot> {
        Ok(self.refresh(authority)?.snapshot)
    }

    /// Lets the original input worker release held keys after focus changed,
    /// including an observed leave-and-return between published snapshots.
    pub(crate) fn input_epoch(&self)->u64 {self.input_epoch}

    fn target(&self, identity: &WindowIdentity) -> io::Result<(WindowBinding, WindowOperations)> {
        let member = self.snapshot.members.iter().find(|m| &m.identity == identity)
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "stale_window_generation"))?;
        let binding = self.bindings.values().find(|(_, id)| id == identity)
            .map(|(binding, _)| binding.clone())
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "stale_window_binding"))?;
        Ok((binding, member.operations))
    }

    pub(crate) fn boundary(&mut self, identity: &WindowIdentity, authority: &dyn WindowAuthority)
        -> io::Result<CaptureBoundary> {
        self.refresh(authority)?;
        self.target(identity)?;
        let mut members = Vec::new();
        for member in &self.snapshot.members {
            if member.minimized || !member.visible { continue; }
            let (binding, _) = self.target(&member.identity)?;
            self.adapter.validate(&binding)?;
            members.push(CaptureMember { identity: member.identity.clone(), binding,
                geometry: member.geometry, z_order: member.z_order, input: member.operations.input });
        }
        Ok(CaptureBoundary { session: self.snapshot.session.clone(), revision: self.snapshot.layout_revision, members })
    }

    /// Call after acquiring member surfaces, before publishing pixels. On false,
    /// discard the frame and invalidate the encoder reference before recapture.
    pub(crate) fn boundary_current(&mut self, boundary: &CaptureBoundary,
        authority: &dyn WindowAuthority) -> io::Result<bool> {
        let identity = self.snapshot.root.clone();
        Ok(self.boundary(&identity, authority)? == *boundary)
    }

    pub(crate) fn operate(&mut self, identity: &WindowIdentity, operation: WindowOperation,
        authority: &dyn WindowAuthority) -> io::Result<OperationStatus> {
        self.admitted(authority, true)?;
        self.refresh(authority)?;
        let (binding, operations) = self.target(identity)?;
        let supported = match operation { WindowOperation::Focus => operations.focus,
            WindowOperation::Close => operations.close,
            WindowOperation::Resize { width, height } => operations.resize && width > 0
                && height > 0 && width <= i32::MAX as u32 && height <= i32::MAX as u32 };
        if !supported { return Err(refused(io::ErrorKind::Unsupported, "window_operation_unavailable")); }
        self.adapter.validate(&binding)?;
        self.admitted(authority, true)?;
        if operation == WindowOperation::Focus {
            self.input_armed = false;
            self.adapter.activate(&binding)?;
            self.refresh(authority)?;
            self.target(identity)?;
            if !self.adapter.focused(&binding)? {
                return Err(refused(io::ErrorKind::WouldBlock, "window_activation_unconfirmed"));
            }
            self.adapter.validate(&binding)?;
            self.admitted(authority, true)?;
            self.input_armed = true;
            return Ok(OperationStatus::FocusConfirmed);
        }
        self.adapter.request(&binding, operation)?;
        Ok(OperationStatus::Requested)
    }

    /// The closure must retain the normal SendInput/XTest security checks and
    /// release held inputs on any error. A focus check is not an isolation fence.
    pub(crate) fn with_input<T>(&mut self, identity: &WindowIdentity, expected_revision: u64,
        authority: &dyn WindowAuthority, inject: impl FnOnce(&CaptureMember) -> io::Result<T>) -> io::Result<T> {
        if let Err(error) = self.admitted(authority, true) { self.input_armed = false; return Err(error); }
        self.refresh(authority)?;
        if !self.input_armed {
            return Err(refused(io::ErrorKind::WouldBlock, "window_focus_confirmation_required"));
        }
        if expected_revision != self.snapshot.revision {
            return Err(refused(io::ErrorKind::WouldBlock, "window_input_snapshot_changed"));
        }
        let (binding, operations) = self.target(identity)?;
        if !operations.input { return Err(refused(io::ErrorKind::Unsupported, "window_input_unavailable")); }
        self.adapter.validate(&binding)?;
        if !self.adapter.focused(&binding)? {
            self.input_armed = false;
            return Err(refused(io::ErrorKind::WouldBlock, "window_activation_unconfirmed"));
        }
        self.admitted(authority, true)?;
        let member = self.snapshot.members.iter().find(|m| &m.identity == identity)
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "stale_window_generation"))?;
        let result = inject(&CaptureMember { identity: identity.clone(), binding,
            geometry: member.geometry, z_order: member.z_order, input: true });
        if result.is_err() { self.input_armed = false; }
        result
    }

    /// The icon adapter supplies only references from its freshly authorized
    /// platform list. A missing icon is a normal neutral-fallback condition.
    pub(crate) fn set_icon(&mut self, identity: &WindowIdentity,
        icon: Option<crate::screen::core::app_icon::IconRef>, authority: &dyn WindowAuthority) -> io::Result<()> {
        self.refresh(authority)?;
        let (binding, _) = self.target(identity)?;
        self.adapter.validate(&binding)?;
        if let Some(icon) = &icon { icon.validate()?; }
        let member = self.snapshot.members.iter_mut().find(|m| &m.identity == identity)
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "stale_window_generation"))?;
        if let Some(icon) = &icon {
            if member.app_id.as_deref() != Some(icon.app_id.as_str()) {
                return Err(refused(io::ErrorKind::InvalidData, "window_icon_app_mismatch"));
            }
        }
        if member.icon != icon {
            member.icon = icon;
            self.snapshot.revision = self.snapshot.revision.checked_add(1)
                .ok_or_else(|| refused(io::ErrorKind::InvalidData, "window_revision_exhausted"))?;
        }
        Ok(())
    }

    pub(crate) fn disconnect(&mut self) {
        self.connected = false;
        self.input_armed = false;
        self.bindings.clear();
        self.snapshot.members.clear();
        self.adapter.disconnect();
    }
}

impl<A: WindowAdapter> Drop for HostSession<A> {
    fn drop(&mut self) { self.disconnect(); }
}
