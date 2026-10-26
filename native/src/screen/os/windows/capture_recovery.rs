//! Requested capture activity survives a temporarily refused native rebuild.
use std::io;

#[derive(Clone, Copy)]
pub(in super::super) struct CaptureRecovery { active: bool }
impl CaptureRecovery {
    pub(in super::super) fn active() -> Self { Self { active: true } }
    pub(in super::super) fn request(&mut self, active: bool) { self.active = active; }

    /// An absent producer is pending only while capture remains requested.
    /// A failed restart leaves that intent intact for the next capture call.
    pub(in super::super) fn ensure_live(self, live: bool, restart: impl FnOnce() -> io::Result<()>)
        -> io::Result<bool> {
        if !self.active { return Ok(false); }
        if !live { restart()?; }
        Ok(true)
    }

    /// A resume acknowledgement records intent, not a promise that old external
    /// images have already retired. Capture will retry after their real drop.
    pub(in super::super) fn activate(self, live: bool, restart: impl FnOnce() -> io::Result<()>)
        -> io::Result<()> {
        match self.ensure_live(live, restart) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            result => result.map(|_| ()),
        }
    }
}
