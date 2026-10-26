//! Confirmed window operations and member-bound input on the same host state.
use super::*;

impl<A: WindowAdapter> HostSession<A> {
    pub(crate) fn operate(
        &mut self,
        identity: &WindowIdentity,
        operation: WindowOperation,
        authority: &dyn WindowAuthority,
    ) -> io::Result<OperationStatus> {
        self.admitted(authority, true)?;
        self.refresh(authority)?;
        let (binding, operations) = self.target(identity)?;
        let supported = match operation {
            WindowOperation::Focus => operations.focus,
            WindowOperation::Close => operations.close,
            WindowOperation::Resize { width, height } => {
                operations.resize
                    && width > 0
                    && height > 0
                    && width <= i32::MAX as u32
                    && height <= i32::MAX as u32
            }
        };
        if !supported {
            return Err(refused(
                io::ErrorKind::Unsupported,
                "window_operation_unavailable",
            ));
        }
        self.adapter.validate(&binding)?;
        self.admitted(authority, true)?;
        if operation == WindowOperation::Focus {
            self.input_armed = false;
            self.adapter.activate(&binding)?;
            self.refresh(authority)?;
            self.target(identity)?;
            if !self.adapter.focused(&binding)? {
                return Err(refused(
                    io::ErrorKind::WouldBlock,
                    "window_activation_unconfirmed",
                ));
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
    pub(crate) fn with_input<T>(
        &mut self,
        identity: &WindowIdentity,
        expected_revision: u64,
        authority: &dyn WindowAuthority,
        inject: impl FnOnce(&CaptureMember) -> io::Result<T>,
    ) -> io::Result<T> {
        if let Err(error) = self.admitted(authority, true) {
            self.input_armed = false;
            return Err(error);
        }
        self.refresh(authority)?;
        if !self.input_armed {
            return Err(refused(
                io::ErrorKind::WouldBlock,
                "window_focus_confirmation_required",
            ));
        }
        if expected_revision != self.snapshot.revision {
            return Err(refused(
                io::ErrorKind::WouldBlock,
                "window_input_snapshot_changed",
            ));
        }
        let (binding, operations) = self.target(identity)?;
        if !operations.input {
            return Err(refused(
                io::ErrorKind::Unsupported,
                "window_input_unavailable",
            ));
        }
        self.adapter.validate(&binding)?;
        if !self.adapter.focused(&binding)? {
            self.input_armed = false;
            return Err(refused(
                io::ErrorKind::WouldBlock,
                "window_activation_unconfirmed",
            ));
        }
        self.admitted(authority, true)?;
        let member = self
            .snapshot
            .members
            .iter()
            .find(|m| &m.identity == identity)
            .ok_or_else(|| refused(io::ErrorKind::NotFound, "stale_window_generation"))?;
        let result = inject(&CaptureMember {
            identity: identity.clone(),
            binding,
            geometry: member.geometry,
            z_order: member.z_order,
            input: true,
        });
        if result.is_err() {
            self.input_armed = false;
        }
        result
    }
}
