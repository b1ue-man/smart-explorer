//! A host session owns the original bindings until disconnect, never the app.
use super::model::*;
use std::collections::HashMap;
use std::io;

#[path = "host_family.rs"]
mod family;
#[path = "host_input.rs"]
mod input;

fn refused(kind: io::ErrorKind, text: &str) -> io::Error {
    io::Error::new(kind, text)
}

/// Implemented at the existing authorization boundary. Every method is evaluated
/// freshly, including after a blocking OS operation. Control remains a session grant.
pub(crate) trait WindowAuthority {
    fn may_view(&self, session: &str) -> bool;
    fn may_control(&self, session: &str) -> bool;
    fn in_list_scope(&self, app_id: Option<&str>) -> bool;
}

pub(crate) trait WindowAdapter: Send {
    /// Host-only candidates. A candidate is not a binding or an authorization.
    fn candidate_windows(&mut self) -> io::Result<Vec<(NativeWindowId, String)>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "window_candidates_unavailable",
        ))
    }
    /// Fresh OS identity, including the actual user/logon where the platform
    /// exposes it. Display labels and client-supplied PID properties do not count.
    fn process_user_matches(
        &mut self,
        _binding: &WindowBinding,
        _user_id: &str,
    ) -> io::Result<bool> {
        Ok(false)
    }
    fn bind(&mut self, native: NativeWindowId) -> io::Result<WindowBinding>;
    fn inspect(&mut self, binding: &WindowBinding) -> io::Result<ObservedWindow> {
        self.observe(binding)?
            .windows
            .into_iter()
            .find(|window| window.binding == *binding)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "original_window_gone"))
    }
    /// Verify root's lifetime marker first. Return only verified owner descendants.
    fn observe(&mut self, root: &WindowBinding) -> io::Result<Observation>;
    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()>;
    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()>;
    fn activate_if(
        &mut self,
        binding: &WindowBinding,
        admitted: &dyn Fn() -> bool,
    ) -> io::Result<()> {
        if !admitted() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "app_activation_admission_ended",
            ));
        }
        self.activate(binding)
    }
    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool>;
    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()>;
    fn export_pin(&mut self, _binding: &WindowBinding) -> io::Result<WindowPin> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "window_pin_transfer_unavailable",
        ))
    }
    /// Relinquish cleanup only after the receiving worker confirmed adoption.
    fn transfer_pin(&mut self, _binding: &WindowBinding) {}
    /// Remove only this adapter's markers. Never close/kill the application.
    fn disconnect(&mut self) {}
}

impl<T: WindowAdapter + ?Sized> WindowAdapter for Box<T> {
    fn candidate_windows(&mut self) -> io::Result<Vec<(NativeWindowId, String)>> {
        (**self).candidate_windows()
    }
    fn process_user_matches(&mut self, binding: &WindowBinding, user_id: &str) -> io::Result<bool> {
        (**self).process_user_matches(binding, user_id)
    }
    fn bind(&mut self, native: NativeWindowId) -> io::Result<WindowBinding> {
        (**self).bind(native)
    }
    fn inspect(&mut self, binding: &WindowBinding) -> io::Result<ObservedWindow> {
        (**self).inspect(binding)
    }
    fn observe(&mut self, root: &WindowBinding) -> io::Result<Observation> {
        (**self).observe(root)
    }
    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        (**self).validate(binding)
    }
    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        (**self).activate(binding)
    }
    fn activate_if(
        &mut self,
        binding: &WindowBinding,
        admitted: &dyn Fn() -> bool,
    ) -> io::Result<()> {
        (**self).activate_if(binding, admitted)
    }
    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool> {
        (**self).focused(binding)
    }
    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        (**self).request(binding, operation)
    }
    fn disconnect(&mut self) {
        (**self).disconnect();
    }
    fn export_pin(&mut self, binding: &WindowBinding) -> io::Result<WindowPin> {
        (**self).export_pin(binding)
    }
    fn transfer_pin(&mut self, binding: &WindowBinding) {
        (**self).transfer_pin(binding);
    }
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
    pub(crate) fn open(
        session: String,
        native: NativeWindowId,
        mut adapter: A,
        authority: &dyn WindowAuthority,
    ) -> io::Result<Self> {
        if session.is_empty() || !authority.may_view(&session) {
            return Err(refused(
                io::ErrorKind::PermissionDenied,
                "window_view_not_authorized",
            ));
        }
        let root = adapter.bind(native)?;
        Self::open_bound(session, root, adapter, authority)
    }

    /// Selection from a generation-bearing list must use its existing host
    /// binding. A stale HWND/XID fails here; it is never silently bound anew.
    /// The adapter must be the owner of that marker's namespace/lifetime.
    pub(crate) fn open_bound(
        session: String,
        root: WindowBinding,
        adapter: A,
        authority: &dyn WindowAuthority,
    ) -> io::Result<Self> {
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let lifetime: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        Self::open_bound_as(session, root, adapter, authority, lifetime)
    }

    pub(crate) fn open_bound_as(
        session: String,
        root: WindowBinding,
        mut adapter: A,
        authority: &dyn WindowAuthority,
        lifetime: String,
    ) -> io::Result<Self> {
        if session.is_empty() || !authority.may_view(&session) {
            return Err(refused(
                io::ErrorKind::PermissionDenied,
                "window_view_not_authorized",
            ));
        }
        adapter.validate(&root)?;
        if lifetime.len() != 32
            || !lifetime
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(refused(
                io::ErrorKind::InvalidData,
                "invalid_window_family_lifetime",
            ));
        }
        let identity = WindowIdentity {
            session: lifetime.clone(),
            generation: 1,
        };
        let snapshot = FamilySnapshot {
            version: FAMILY_VERSION,
            session: lifetime,
            revision: 0,
            layout_revision: 0,
            root: identity.clone(),
            members: Vec::new(),
            limitations: Vec::new(),
        };
        let mut host = Self {
            adapter,
            root: root.clone(),
            session,
            next_generation: 2,
            bindings: HashMap::from([(root.native, (root, identity))]),
            snapshot,
            connected: true,
            input_armed: false,
            input_epoch: 0,
        };
        host.refresh(authority)?;
        Ok(host)
    }

    fn admitted(&self, authority: &dyn WindowAuthority, control: bool) -> io::Result<()> {
        if !self.connected {
            return Err(refused(io::ErrorKind::NotConnected, "window_disconnected"));
        }
        if !authority.may_view(&self.session) || (control && !authority.may_control(&self.session))
        {
            return Err(refused(
                io::ErrorKind::PermissionDenied,
                "window_permission_revoked",
            ));
        }
        Ok(())
    }

    /// A full re-fetch always refreshes platform facts and current permissions.
    pub(crate) fn snapshot(
        &mut self,
        authority: &dyn WindowAuthority,
    ) -> io::Result<FamilySnapshot> {
        Ok(self.refresh(authority)?.snapshot)
    }

    /// Lets the original input worker release held keys after focus changed,
    /// including an observed leave-and-return between published snapshots.
    pub(crate) fn input_epoch(&self) -> u64 {
        self.input_epoch
    }

    fn target(&self, identity: &WindowIdentity) -> io::Result<(WindowBinding, WindowOperations)> {
        let member = self
            .snapshot
            .members
            .iter()
            .find(|m| &m.identity == identity)
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "stale_window_generation"))?;
        let binding = self
            .bindings
            .values()
            .find(|(_, id)| id == identity)
            .map(|(binding, _)| binding.clone())
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "stale_window_binding"))?;
        Ok((binding, member.operations))
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
    fn drop(&mut self) {
        self.disconnect();
    }
}
