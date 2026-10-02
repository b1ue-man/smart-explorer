//! Opening a file only when it is a regular file (`open_regular`).
use std::{fmt, io};

/// What an open does with a link at the last path component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FinalLink {
    /// Open the link's target (Explorer semantics).
    #[allow(dead_code)] // for the Explorer read path; walks refuse links
    Follow,
    /// Refuse the link: in walks a link is a boundary, never content.
    Refuse,
}

/// Why `open_regular` refused a path. It travels inside the returned
/// `io::Error` (kind `InvalidInput`); `NotRegular::of` reads it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotRegular {
    Link,
    Directory,
    Special,
}

impl NotRegular {
    pub(crate) fn error(self) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, self)
    }

    /// The refusal carried by `error`, if `open_regular` produced it.
    pub(crate) fn of(error: &io::Error) -> Option<Self> {
        error.get_ref()?.downcast_ref::<Self>().copied()
    }
}

impl fmt::Display for NotRegular {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Link => "Verknüpfung wird nicht als Datei gelesen",
            Self::Directory => "Ordner statt Datei",
            Self::Special => "Spezialdatei (Pipe, Socket oder Gerät) wird nicht gelesen",
        })
    }
}

impl std::error::Error for NotRegular {}
