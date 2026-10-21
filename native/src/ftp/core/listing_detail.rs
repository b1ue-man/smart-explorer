//! Two pure decisions that keep round trips off the FTP upload path on servers
//! without MLST/MLSD (vsftpd): how much a folder listing must find out about
//! every regular file, and which server facts a new pooled connection takes
//! over instead of asking again.
//!
//! Evidence (suite run 2, `speed_task_ftp_large_up_wan`, 50 ms round trip):
//! publishing a stage ran FEAT, PWD, CWD, LIST (PASV, LIST), CWD, then SIZE and
//! MDTM for every regular file of the folder, then RNFR and RNTO, about ten
//! round trips and two more for each further file in the folder.
use super::metadata::Features;

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
