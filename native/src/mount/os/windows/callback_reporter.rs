use std::sync::Arc;

use crate::{daemon::MountHostSession, mount::MountStatus};

/// Keep status delivery separate from the real filesystem callback machinery.
pub(super) enum CallbackReporter {
    Host(Arc<MountHostSession>),
    #[cfg(test)]
    Capture(std::sync::mpsc::Sender<MountStatus>),
}

impl From<Arc<MountHostSession>> for CallbackReporter {
    fn from(session: Arc<MountHostSession>) -> Self {
        Self::Host(session)
    }
}

impl CallbackReporter {
    pub(super) fn report(&self, status: MountStatus) -> Result<(), String> {
        match self {
            Self::Host(session) => session.report_status(status),
            #[cfg(test)]
            Self::Capture(sender) => sender
                .send(status)
                .map_err(|_| "mounted-volume status observer closed".to_string()),
        }
    }
}
