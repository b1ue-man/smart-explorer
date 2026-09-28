//! One consented read-access request per transfer for protected local folders
//! (on Windows the scoped backup-read helper). Listings and file opens share
//! the answer, so the user is asked at most once, and a refusal ends the
//! transfer instead of prompting again for every file.
use std::io;
use std::sync::Mutex;

/// What a refused local read may do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessAnswer {
    /// Read access was granted: retry the refused operation once.
    Granted,
    /// The user declined: the transfer stops without asking again.
    Refused,
    /// No elevated read path exists here (other platforms, network drives,
    /// an earlier grant, backup privilege already held) or the request itself
    /// failed (its reason): the refusal stands for this entry only.
    Unavailable(Option<String>),
}

pub struct AccessGate {
    root: String,
    answer: Mutex<Option<AccessAnswer>>,
}

impl AccessGate {
    /// `root` is the selected local folder the grant would cover.
    pub fn new(root: &str) -> Self {
        Self {
            root: root.to_string(),
            answer: Mutex::new(None),
        }
    }

    /// Asks once and blocks until the answer is known; later and concurrent
    /// callers get the same answer without a new prompt.
    pub fn request(&self) -> AccessAnswer {
        let mut answer = self
            .answer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(known) = answer.as_ref() {
            return known.clone();
        }
        let native =
            std::path::PathBuf::from(self.root.replace('/', std::path::MAIN_SEPARATOR_STR));
        let requested = crate::local_access::display_path(&native);
        let asked = if crate::local_access::can_request_access(&requested) {
            match crate::local_access::request_access(&requested) {
                Ok(true) => AccessAnswer::Granted,
                Ok(false) => AccessAnswer::Refused,
                Err(detail) => AccessAnswer::Unavailable(Some(detail)),
            }
        } else {
            AccessAnswer::Unavailable(None)
        };
        *answer = Some(asked.clone());
        asked
    }

    /// True once the user declined read access.
    pub fn refused(&self) -> bool {
        matches!(
            *self
                .answer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Some(AccessAnswer::Refused)
        )
    }
}

/// The refusal of `error` extended by the reason the request failed.
pub(crate) fn with_access_detail(error: io::Error, detail: Option<&str>) -> io::Error {
    match detail {
        Some(detail) => io::Error::new(error.kind(), format!("{error} ({detail})")),
        None => error,
    }
}
