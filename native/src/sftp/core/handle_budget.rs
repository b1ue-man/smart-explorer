//! Open handles of one SFTP request stream (`sftp-server` process), bounded
//! so every handle stays usable.
//!
//! OpenSSH's `sftp-server` names a handle by its slot index as a 4-byte
//! big-endian number and hands out the lowest free slots: the highest index
//! in use is the peak of handles held open at once minus one. russh-sftp
//! 2.3 decodes every handle as UTF-8 with `from_utf8_lossy` (buf.rs), so a
//! handle with a byte of 0x80 or more (slot 128 and up) comes back with
//! U+FFFD in place of that byte and is sent back five bytes long: the server
//! answers each READ, WRITE and CLOSE on it with a plain FAILURE ("Failure:
//! Failure" per file in the 2026-10-28 measurement, where several packets of
//! 64 files shared one channel) and the slot stays occupied for good.
//!
//! Hence at most `REPRESENTABLE_HANDLES` handles (slots 0 to 127, one ASCII
//! byte each) are held on one stream at once, fewer when the server's own
//! `limits@openssh.com` says so. A handle counts from before its OPEN is
//! sent until its CLOSE was answered (the server works through the requests
//! of one stream in order, so it never holds more at once). A handle whose
//! fate is unknown (OPEN or CLOSE without an answer) keeps its slot, as it
//! does on the server. A handle that nevertheless arrives damaged (a server
//! with another naming) retires the stream instead of failing every file
//! that would follow on it.
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::RawSftpSession;
use russh_sftp::protocol::{FileAttributes, Handle, OpenFlags, Status};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Slots 0 to 127: their 4-byte handles are plain ASCII and survive
/// russh-sftp's UTF-8 decoding unchanged.
pub(super) const REPRESENTABLE_HANDLES: usize = 128;

/// Handles one stream may hold at once: the server's open-handle limit
/// (`None` or 0: it names none), never more than `REPRESENTABLE_HANDLES`.
pub(super) fn handle_budget(server_limit: Option<u64>) -> usize {
    server_limit
        .filter(|limit| *limit >= 1)
        .map_or(REPRESENTABLE_HANDLES, |limit| {
            usize::try_from(limit)
                .unwrap_or(usize::MAX)
                .min(REPRESENTABLE_HANDLES)
        })
}

/// Whether a handle came through russh-sftp's decoding byte for byte: a
/// lossy decode leaves U+FFFD where the invalid bytes were.
pub(super) fn handle_intact(handle: &str) -> bool {
    !handle.contains('\u{FFFD}')
}

/// Free slots reserved before an OPEN; an unused one returns on drop.
pub(super) struct HandleSlots(Option<OwnedSemaphorePermit>);

impl HandleSlots {
    fn take_one(&mut self) -> Option<OwnedSemaphorePermit> {
        let permit = self.0.as_mut()?;
        if permit.num_permits() > 1 {
            permit.split(1)
        } else {
            self.0.take()
        }
    }
}

/// A raw SFTP session whose OPEN and CLOSE keep the handle budget; every
/// other request goes straight to the session (`Deref`).
pub(super) struct BudgetSession {
    raw: RawSftpSession,
    slots: Arc<Semaphore>,
    /// The slot of every open handle, by handle.
    held: Mutex<HashMap<String, OwnedSemaphorePermit>>,
    /// A damaged handle arrived: no further OPEN on this stream.
    retired: AtomicBool,
}

impl Deref for BudgetSession {
    type Target = RawSftpSession;

    fn deref(&self) -> &RawSftpSession {
        &self.raw
    }
}

fn retired_error() -> SftpError {
    SftpError::Limited("SFTP-Kanal nimmt keine weiteren Dateien an".to_string())
}

/// The server certainly processed the request (or it never left).
fn answered(error: &SftpError) -> bool {
    matches!(error, SftpError::Status(_) | SftpError::Limited(_))
}

impl BudgetSession {
    pub(super) fn new(raw: RawSftpSession, server_limit: Option<u64>) -> Self {
        Self {
            raw,
            slots: Arc::new(Semaphore::new(handle_budget(server_limit))),
            held: Mutex::new(HashMap::new()),
            retired: AtomicBool::new(false),
        }
    }

    /// No further OPEN: waiting and later ones fail as congestion (nothing
    /// was sent, the file goes again elsewhere).
    pub(super) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        self.slots.close();
    }

    pub(super) fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }

    /// Reserves `count` slots at once, for a step that holds that many
    /// handles together (a server copy: source and stage): reserving them
    /// one by one could leave every waiter holding one and waiting forever.
    pub(super) async fn reserve(&self, count: u32) -> Result<HandleSlots, SftpError> {
        if self.is_retired() {
            return Err(retired_error());
        }
        self.slots
            .clone()
            .acquire_many_owned(count.max(1))
            .await
            .map(|permit| HandleSlots(Some(permit)))
            .map_err(|_| retired_error())
    }

    /// OPEN within the budget: waits for a free slot first.
    pub(super) async fn open<T: Into<String>>(
        &self,
        filename: T,
        flags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, SftpError> {
        let mut slots = self.reserve(1).await?;
        self.open_reserved(&mut slots, filename, flags, attrs).await
    }

    /// OPEN on one slot of `slots`.
    pub(super) async fn open_reserved<T: Into<String>>(
        &self,
        slots: &mut HandleSlots,
        filename: T,
        flags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, SftpError> {
        let filename: String = filename.into();
        let Some(slot) = slots.take_one() else {
            return Err(retired_error());
        };
        if self.is_retired() {
            return Err(retired_error());
        }
        let handle = match self.raw.open(filename.clone(), flags, attrs).await {
            Ok(handle) => handle,
            Err(error) => {
                if !answered(&error) {
                    // The server may have opened it after all.
                    slot.forget();
                }
                return Err(error);
            }
        };
        if !handle_intact(&handle.handle) {
            // The handle cannot be named back to the server: it stays open
            // there until the stream closes. A file this OPEN created anew
            // (exclusive create) is removed by name, so no empty stage stays.
            slot.forget();
            self.retire();
            if flags.contains(OpenFlags::CREATE) && flags.contains(OpenFlags::EXCLUDE) {
                let _ = self.raw.remove(filename).await;
            }
            return Err(SftpError::Limited(
                "SFTP-Server lieferte ein nicht darstellbares Datei-Handle".to_string(),
            ));
        }
        lock(&self.held).insert(handle.handle.clone(), slot);
        Ok(handle)
    }

    /// CLOSE; the slot frees once the server answered it.
    pub(super) async fn close<H: Into<String>>(&self, handle: H) -> Result<Status, SftpError> {
        let handle: String = handle.into();
        let result = self.raw.close(handle.clone()).await;
        let slot = lock(&self.held).remove(&handle);
        if let Some(slot) = slot {
            match &result {
                Ok(_) | Err(SftpError::Status(_)) => drop(slot),
                // No answer: the server may still hold the handle.
                Err(_) => slot.forget(),
            }
        }
        result
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_sftp_handle_budget_stays_representable() {
        assert_eq!(handle_budget(None), REPRESENTABLE_HANDLES);
        assert_eq!(handle_budget(Some(0)), REPRESENTABLE_HANDLES);
        assert_eq!(handle_budget(Some(1019)), REPRESENTABLE_HANDLES);
        assert_eq!(handle_budget(Some(20)), 20);
        // Slot 127 is the last whose big-endian handle is plain ASCII.
        let last = String::from_utf8_lossy(&127u32.to_be_bytes()).into_owned();
        assert!(handle_intact(&last));
        assert_eq!(last.len(), 4);
        let first_bad = String::from_utf8_lossy(&128u32.to_be_bytes()).into_owned();
        assert!(!handle_intact(&first_bad));
        assert_ne!(first_bad.len(), 4);
    }
}
