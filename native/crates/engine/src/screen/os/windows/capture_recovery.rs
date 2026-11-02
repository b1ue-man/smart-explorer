//! Requested capture activity survives a temporarily refused native rebuild.
use std::io;

use windows::core::HRESULT;
use windows::Win32::Foundation::E_ACCESSDENIED;
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_NOT_CURRENTLY_AVAILABLE, DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_UNSUPPORTED,
};

use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::session_state::SessionState;

/// The state a failed `IDXGIOutput1::DuplicateOutput` waits in, for the
/// results Microsoft documents as passing: `E_ACCESSDENIED` ("only an
/// application that runs at LOCAL_SYSTEM can access the secure desktop"),
/// `DXGI_ERROR_UNSUPPORTED` ("wait for system notification of desktop
/// switches and mode changes and then call DuplicateOutput again"),
/// `DXGI_ERROR_SESSION_DISCONNECTED` and `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE`
/// (four concurrent duplication applications). Every other result is an error.
fn duplication_pause(code: HRESULT) -> Option<SessionState> {
    if code == E_ACCESSDENIED {
        Some(SessionState::SecureDesktop)
    } else if code == DXGI_ERROR_SESSION_DISCONNECTED {
        Some(SessionState::SessionInactive)
    } else if code == DXGI_ERROR_UNSUPPORTED || code == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE {
        Some(SessionState::Starting)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct CaptureRecovery {
    active: bool,
}
impl CaptureRecovery {
    pub(in super::super) fn active() -> Self {
        Self { active: true }
    }
    pub(in super::super) fn request(&mut self, active: bool) {
        self.active = active;
    }

    /// An absent producer is pending only while capture remains requested.
    /// A failed restart leaves that intent intact for the next capture call.
    pub(in super::super) fn ensure_live(
        self,
        live: bool,
        restart: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<bool> {
        if !self.active {
            return Ok(false);
        }
        if !live {
            restart()?;
        }
        Ok(true)
    }

    /// A resume acknowledgement records intent, not a promise that old external
    /// images have already retired. Capture will retry after their real drop.
    pub(in super::super) fn activate(
        self,
        live: bool,
        restart: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        match self.ensure_live(live, restart) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            result => result.map(|_| ()),
        }
    }

    /// A rebuild refused with a passing duplication result pauses; the
    /// producer stays absent and the next capture rebuilds it again.
    pub(in super::super) fn paused_by_duplication(error: io::Error) -> io::Result<SourceFrame> {
        let code = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<windows::core::Error>())
            .map(windows::core::Error::code);
        match code.and_then(duplication_pause) {
            Some(state) => Ok(SourceFrame::Paused(state)),
            None => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paused(code: HRESULT) -> Option<SessionState> {
        let error = io::Error::other(windows::core::Error::from_hresult(code));
        match CaptureRecovery::paused_by_duplication(error) {
            Ok(SourceFrame::Paused(state)) => Some(state),
            Ok(_) => panic!("a refused rebuild never yields a picture"),
            Err(_) => None,
        }
    }

    #[test]
    fn screen_metadata_runtime_task_dxgi_rebuild_waits_on_documented_passing_results() {
        assert_eq!(paused(E_ACCESSDENIED), Some(SessionState::SecureDesktop));
        assert_eq!(
            paused(DXGI_ERROR_SESSION_DISCONNECTED),
            Some(SessionState::SessionInactive)
        );
        assert_eq!(paused(DXGI_ERROR_UNSUPPORTED), Some(SessionState::Starting));
        assert_eq!(
            paused(DXGI_ERROR_NOT_CURRENTLY_AVAILABLE),
            Some(SessionState::Starting)
        );
        // A lost device or any other failure still ends the capture.
        assert_eq!(paused(windows::Win32::Foundation::E_INVALIDARG), None);
        assert!(CaptureRecovery::paused_by_duplication(io::Error::other("monitor gone")).is_err());
    }
}
