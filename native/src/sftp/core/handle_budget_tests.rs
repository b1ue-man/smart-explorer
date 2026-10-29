//! The handle budget against a server that names handles as OpenSSH's
//! `sftp-server` does: the lowest free slot as a 4-byte big-endian number.
//! russh-sftp's server side can only send a `String`, so the fake sends
//! what russh-sftp's client makes of such a handle (`from_utf8_lossy`): from
//! slot 128 on the handle carries U+FFFD and no longer names its slot, and
//! the server answers it with FAILURE, as `sftp-server` answers a handle of
//! the wrong length.
use super::handle_budget::{BudgetSession, REPRESENTABLE_HANDLES};
use futures_util::future::join_all;
use russh_sftp::client::RawSftpSession;
use russh_sftp::protocol::{FileAttributes, Handle, OpenFlags, Status, StatusCode, Version};
use russh_sftp::server::Handler;
use std::collections::{BTreeSet, HashMap};
use std::future::{ready, Future};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Default)]
struct Slots {
    open: BTreeSet<u32>,
    peak: usize,
}

#[derive(Clone, Default)]
struct OpenSshNaming {
    slots: Arc<Mutex<Slots>>,
}

impl OpenSshNaming {
    fn slots(&self) -> MutexGuard<'_, Slots> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The handle of `slot` as russh-sftp's client decodes it.
fn named(slot: u32) -> String {
    String::from_utf8_lossy(&slot.to_be_bytes()).into_owned()
}

/// The slot a handle names, when it names one exactly.
fn slot_of(handle: &str) -> Option<u32> {
    let bytes: [u8; 4] = handle.as_bytes().try_into().ok()?;
    let slot = u32::from_be_bytes(bytes);
    (named(slot) == handle).then_some(slot)
}

fn status(id: u32, status_code: StatusCode) -> Status {
    Status {
        id,
        status_code,
        error_message: status_code.to_string(),
        language_tag: "en-US".to_string(),
    }
}

impl Handler for OpenSshNaming {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> impl Future<Output = Result<Version, Self::Error>> + Send {
        ready(Ok(Version::new()))
    }

    fn open(
        &mut self,
        id: u32,
        _filename: String,
        _pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> impl Future<Output = Result<Handle, Self::Error>> + Send {
        let mut slots = self.slots();
        let slot = (0u32..)
            .find(|slot| !slots.open.contains(slot))
            .unwrap_or(0);
        slots.open.insert(slot);
        slots.peak = slots.peak.max(slots.open.len());
        ready(Ok(Handle {
            id,
            handle: named(slot),
        }))
    }

    fn close(
        &mut self,
        id: u32,
        handle: String,
    ) -> impl Future<Output = Result<Status, Self::Error>> + Send {
        let closed = slot_of(&handle).is_some_and(|slot| self.slots().open.remove(&slot));
        ready(if closed {
            Ok(status(id, StatusCode::Ok))
        } else {
            Err(StatusCode::Failure)
        })
    }
}

fn session(server: &OpenSshNaming) -> (tokio::runtime::Runtime, RawSftpSession) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("tokio runtime");
    let (client, served) = tokio::io::duplex(1 << 20);
    let handler = server.clone();
    let session = runtime.block_on(async move {
        russh_sftp::server::run(served, handler).await;
        let session = RawSftpSession::new(client);
        session.init().await.expect("VERSION");
        session
    });
    (runtime, session)
}

/// 300 files opened and closed at once: the outcome of each.
async fn open_and_close_all<O, OF, C, CF>(open: O, close: C) -> Vec<bool>
where
    O: Fn(String) -> OF,
    OF: Future<Output = Result<Handle, russh_sftp::client::error::Error>>,
    C: Fn(String) -> CF,
    CF: Future<Output = Result<Status, russh_sftp::client::error::Error>>,
{
    let (open, close) = (&open, &close);
    join_all((0..300).map(|index| async move {
        let Ok(handle) = open(format!("/data/f{index}")).await else {
            return false;
        };
        tokio::task::yield_now().await;
        close(handle.handle).await.is_ok()
    }))
    .await
}

#[test]
fn speed_task_sftp_raw_handles_from_slot_128_fail_like_the_measurement() {
    let server = OpenSshNaming::default();
    let (runtime, raw) = session(&server);
    let outcomes = runtime.block_on(open_and_close_all(
        |path| raw.open(path, OpenFlags::READ, FileAttributes::empty()),
        |handle| raw.close(handle),
    ));
    assert!(server.slots().peak > REPRESENTABLE_HANDLES);
    assert!(
        outcomes.iter().any(|ok| !ok),
        "slots from 128 on cannot be closed through russh-sftp"
    );
}

#[test]
fn speed_task_sftp_budget_keeps_every_handle_usable() {
    let server = OpenSshNaming::default();
    let (runtime, raw) = session(&server);
    let budget = BudgetSession::new(raw, None);
    let outcomes = runtime.block_on(open_and_close_all(
        |path| budget.open(path, OpenFlags::READ, FileAttributes::empty()),
        |handle| budget.close(handle),
    ));
    assert!(outcomes.iter().all(|ok| *ok), "every file opens and closes");
    let slots = server.slots();
    assert!(slots.peak <= REPRESENTABLE_HANDLES, "peak {}", slots.peak);
    assert!(slots.peak > 1, "the files still overlap");
    assert!(slots.open.is_empty(), "every handle was closed");
}

#[test]
fn speed_task_sftp_budget_follows_a_smaller_server_limit() {
    let server = OpenSshNaming::default();
    let (runtime, raw) = session(&server);
    let budget = BudgetSession::new(raw, Some(20));
    let outcomes = runtime.block_on(open_and_close_all(
        |path| budget.open(path, OpenFlags::READ, FileAttributes::empty()),
        |handle| budget.close(handle),
    ));
    assert!(outcomes.iter().all(|ok| *ok));
    assert!(server.slots().peak <= 20);
}
