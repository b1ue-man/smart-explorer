//! Errors of the text view and its language packs. The texts are German, as
//! every surface (CLI, desktop, Android) shows them to the user unchanged.
use std::fmt;
use std::io;

/// Why a file cannot be shown or a language pack cannot be managed.
#[derive(Debug)]
pub enum TextViewError {
    /// NUL bytes (UTF-16: a NUL code unit) in the first 8 KiB: binary data.
    NotText,
    /// The source or the language folder failed.
    Io(io::Error),
    /// The caller set the cancel flag.
    Cancelled,
    /// A language pack (or one file of it) is not a valid Sublime syntax.
    InvalidPack(String),
    /// A pack name that cannot be a file name in the language folder.
    InvalidPackName(String),
    /// `add` with a name another added pack already has.
    PackExists(String),
    /// `remove` of a name no added pack has.
    PackNotFound(String),
    /// `remove` of a built-in language.
    BuiltinPack(String),
}

impl fmt::Display for TextViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotText => f.write_str(
                "keine Textdatei: die ersten 8 KiB enthalten NUL-Bytes (Binärdaten)",
            ),
            Self::Io(error) => write!(f, "Lesen fehlgeschlagen: {error}"),
            Self::Cancelled => f.write_str("abgebrochen"),
            Self::InvalidPack(reason) => write!(f, "ungültiges Sprachpaket: {reason}"),
            Self::InvalidPackName(name) => write!(
                f,
                "ungültiger Paketname „{name}“: erlaubt sind 1 bis 100 Zeichen ohne / \\ : * ? \" < > | und ohne führenden Punkt"
            ),
            Self::PackExists(name) => write!(
                f,
                "ein Sprachpaket „{name}“ ist schon hinzugefügt; entferne es zuerst (se languages remove {name})"
            ),
            Self::PackNotFound(name) => {
                write!(f, "kein hinzugefügtes Sprachpaket „{name}“ gefunden")
            }
            Self::BuiltinPack(name) => write!(
                f,
                "„{name}“ ist eine eingebaute Sprache und lässt sich nicht entfernen; ein hinzugefügtes Paket mit gleichem Namen oder gleicher Endung geht ihr vor"
            ),
        }
    }
}

impl std::error::Error for TextViewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for TextViewError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Result of the text view functions.
pub type TextViewResult<T> = Result<T, TextViewError>;
