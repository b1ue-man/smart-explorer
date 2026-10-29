//! What keeps round trips off FTP listings and uploads on servers without
//! MLST/MLSD (vsftpd): how much a folder listing must find out about every
//! regular file, which server facts a new pooled connection takes over
//! instead of asking again, and the SIZE/MDTM probes of one listing spread
//! over several connections of the pool.
//!
//! Evidence (suite run 2, `speed_task_ftp_large_up_wan`, 50 ms round trip):
//! publishing a stage ran FEAT, PWD, CWD, LIST (PASV, LIST), CWD, then SIZE and
//! MDTM for every regular file of the folder, then RNFR and RNTO, about ten
//! round trips and two more for each further file in the folder.
use super::io_adapters::FtpConnection;
use super::metadata::Features;
use super::pool::FtpLease;
use crate::vfs::VfsMeta;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

/// How much a listing must know about each regular file of a LIST answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Detail {
    /// Exact size and modification time: a LIST row is refined by SIZE and
    /// MDTM (browsing, sync, stat). A LIST row has no year or seconds.
    Exact,
    /// Names and kinds only (promotion of a stage): the row is taken as it
    /// is, no SIZE or MDTM follows.
    Kinds,
}

impl Detail {
    /// Whether every regular file of a LIST answer is probed with SIZE and
    /// MDTM.
    pub(super) fn refines_list_rows(self) -> bool {
        matches!(self, Detail::Exact)
    }

    /// Whether the listing says anything about the time precision of the
    /// server: only a listing that refined its rows knows it.
    pub(super) fn records_precision(self) -> bool {
        matches!(self, Detail::Exact)
    }
}

/// The FEAT answer a new connection starts with: its own when it has one,
/// else the one of the connection it was opened beside (same server, same
/// account, same TLS state). `None`: it has to ask.
pub(super) fn inherited_features(
    own: Option<Features>,
    other: Option<Features>,
) -> Option<Features> {
    own.or(other)
}

/// More control connections of the same account for the probes of one
/// listing (the pool's free ones, pool.rs).
pub(super) trait ProbeHelpers {
    /// Starts logins in the background so that `count` connections can
    /// help soon; never waits for them.
    fn prepare(&self, count: usize);
    /// A logged-in free connection now, or `None`; never waits and never
    /// logs in. The lease goes back to the pool when dropped.
    fn take(&self) -> Option<FtpLease>;
}

pub(super) type Helpers<'a> = &'a dyn ProbeHelpers;

/// No connection besides the listing's own.
pub(super) struct NoHelpers;

impl ProbeHelpers for NoHelpers {
    fn prepare(&self, _count: usize) {}

    fn take(&self) -> Option<FtpLease> {
        None
    }
}

/// Connections that probe `count` rows at once besides the listing's own:
/// ⌈√count⌉ − 1. A probe costs one round trip per command and a login a few
/// (TCP, greeting, USER, PASS, TYPE), so with `k` connections the rows take
/// about `count / k` round trips plus one login; that sum is smallest near
/// `k = √count`. One row stays on one connection, a folder of 100 files
/// gets 9 helpers, one of 5000 files 70 (the server's connection limit, once
/// it refused a login, caps them like every transfer connection).
pub(super) fn helper_count(count: usize) -> usize {
    ((count as f64).sqrt().ceil() as usize).saturating_sub(1)
}

/// The exact facts (SIZE, MDTM) of the regular files `probes` names as
/// `(path, name)`, answer `i` for probe `i`. The listing's own `connection`
/// probes one row after the other; between two rows it hands every
/// connection `helpers` has free by then (up to `helper_count`) a thread of
/// its own that probes the next unanswered rows as well. Nothing waits for a
/// login: one that is late only helps less, one that hangs helps not at all.
/// Every row is still probed exactly as on one connection (suite run 2: 5000
/// rows one after another were 10000 round trips, 400 s at 40 ms).
pub(super) fn refine(
    connection: &FtpConnection,
    probes: &[(String, String)],
    helpers: Helpers<'_>,
) -> Vec<io::Result<Option<VfsMeta>>> {
    let answers: Mutex<Vec<Option<io::Result<Option<VfsMeta>>>>> =
        Mutex::new((0..probes.len()).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    // One row on `on`; false once every row was taken.
    let probe_one = |on: &FtpConnection| {
        let index = next.fetch_add(1, Ordering::Relaxed);
        let Some((path, name)) = probes.get(index) else {
            return false;
        };
        let answer =
            on.with_stream_read(|stream| super::metadata_probe::regular_file(stream, path, name));
        if let Some(slot) = answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(index)
        {
            *slot = Some(answer);
        }
        true
    };
    let wanted = helper_count(probes.len());
    helpers.prepare(wanted);
    std::thread::scope(|scope| {
        let mut started = 0;
        loop {
            while started < wanted && next.load(Ordering::Relaxed) < probes.len() {
                let Some(lease) = helpers.take() else {
                    break;
                };
                let probe_one = &probe_one;
                let helper = std::thread::Builder::new()
                    .name("ftp-probe".to_string())
                    .spawn_scoped(scope, move || while probe_one(lease.connection()) {});
                match helper {
                    Ok(_) => started += 1,
                    // No thread: this connection goes back, the rows stay.
                    Err(_) => started = wanted,
                }
            }
            if !probe_one(connection) {
                break;
            }
        }
    });
    answers
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .into_iter()
        .map(|answer| answer.unwrap_or_else(|| Err(io::Error::other("FTP-Probe fehlt"))))
        .collect()
}
