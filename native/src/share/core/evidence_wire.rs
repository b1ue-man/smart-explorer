//! Transfer evidence and bulk folders of the Share protocol (`evidence_v1`,
//! `create_dirs_v1`, both advertised in `FsTransferCapabilities`). A client
//! asks for evidence per request (`evidence: true`) only from a host that
//! advertises it; the host then answers with what it already read (size,
//! time, provider ID, bound file identity) instead of the client asking again
//! with a Stat and a FileIdentity per file. Older peers never see these
//! fields or replies.
use std::io;

use serde::{Deserialize, Serialize};

use super::FsErrorKind;

/// Folders of one `CreateDirs` request: one reply carries one outcome each.
pub(crate) const CREATE_DIRS_MAX: usize = 256;

/// What a host read about one regular file: `Stat` (size, time, ID) and the
/// `FileIdentity` reply for the same path, from the same access check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsFileEvidence {
    pub(crate) size: u64,
    pub(crate) mtime_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    /// The host-bound identity exactly as `FileIdentity` answers it; `None`
    /// where the provider has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) identity: Option<String>,
}

/// `create_dirs_v1`: folders created in this order (parents first), each
/// like `CreateDir` without `exclusive`, reporting whether it was created.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsCreateDirs {
    pub(crate) paths: Vec<String>,
}

/// Outcome of one folder of `CreateDirs`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "o", rename_all = "snake_case")]
pub(crate) enum FsDirOutcome {
    /// This request created the folder.
    Created,
    /// A plain folder was there already; it is merged into.
    Existed,
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<FsErrorKind>,
        msg: String,
    },
}

impl FsDirOutcome {
    pub(crate) fn of(result: io::Result<bool>) -> Self {
        match result {
            Ok(true) => Self::Created,
            Ok(false) => Self::Existed,
            Err(error) => Self::Failed {
                kind: crate::share::fs_error::kind_of(&error),
                msg: error.to_string(),
            },
        }
    }
}

/// Host-side bounds of a `CreateDirs` before any folder is touched.
pub(crate) fn validate_create_dirs(request: &FsCreateDirs) -> io::Result<()> {
    if request.paths.is_empty() || request.paths.len() > CREATE_DIRS_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Eine Ordneranfrage enthält 1 bis {CREATE_DIRS_MAX} Ordner"),
        ));
    }
    Ok(())
}

/// What a client stage write with `evidence` reports when it is committed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsWritten {
    pub(crate) evidence: FsFileEvidence,
    /// Whether the request's `probe` path existed after the stage was
    /// closed; `None` without a probe or when the host could not tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) existed: Option<bool>,
}

#[cfg(test)]
#[path = "evidence_wire_tests.rs"]
mod tests;
