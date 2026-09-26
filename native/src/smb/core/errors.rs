//! `smb2` errors as `io::Error` with German messages. The connect stages
//! keep their own wording: the mobile facade classifies a failed connect by
//! its text (`Anmeldung` → `auth`, a missing share → `not_found`, everything
//! else → `network`), and the kinds drive pool eviction and retries.
use smb2::types::status::NtStatus;
use smb2::{Error, ErrorKind};
use std::io;

const MISSING_SHARE_PREFIX: &str = "SMB-Freigabe „";
const MISSING_SHARE_SUFFIX: &str = "“ nicht gefunden";

/// The `io::ErrorKind` for an smb2 classification.
pub(super) fn io_kind(kind: ErrorKind) -> io::ErrorKind {
    use io::ErrorKind as K;
    match kind {
        ErrorKind::NotFound => K::NotFound,
        ErrorKind::AlreadyExists => K::AlreadyExists,
        ErrorKind::AccessDenied | ErrorKind::AuthRequired | ErrorKind::SigningRequired => {
            K::PermissionDenied
        }
        ErrorKind::ConnectionLost | ErrorKind::SessionExpired => K::ConnectionAborted,
        ErrorKind::TimedOut => K::TimedOut,
        ErrorKind::Cancelled => K::Interrupted,
        ErrorKind::InvalidName | ErrorKind::IsADirectory | ErrorKind::NotADirectory => {
            K::InvalidInput
        }
        ErrorKind::InvalidData | ErrorKind::TooLarge => K::InvalidData,
        ErrorKind::Unsupported | ErrorKind::DfsReferral => K::Unsupported,
        ErrorKind::DiskFull => K::StorageFull,
        ErrorKind::SharingViolation => K::ResourceBusy,
        _ => K::Other,
    }
}

fn kind_of(error: &Error) -> io::ErrorKind {
    match error {
        Error::Io(inner) => inner.kind(),
        _ if error.status() == Some(NtStatus::DIRECTORY_NOT_EMPTY) => {
            io::ErrorKind::DirectoryNotEmpty
        }
        _ => io_kind(error.kind()),
    }
}

fn label(kind: io::ErrorKind) -> &'static str {
    use io::ErrorKind as K;
    match kind {
        K::NotFound => "Nicht gefunden",
        K::AlreadyExists => "Existiert bereits",
        K::PermissionDenied => "Keine Berechtigung",
        K::ConnectionAborted | K::ConnectionReset | K::BrokenPipe | K::UnexpectedEof => {
            "Verbindung zum SMB-Server unterbrochen"
        }
        K::TimedOut => "Zeitüberschreitung",
        K::InvalidInput => "Ungültiger Name oder falscher Eintragstyp",
        K::InvalidData => "Ungültige Antwort des SMB-Servers",
        K::Unsupported => "Vom SMB-Server nicht unterstützt",
        K::StorageFull => "Kein Speicherplatz mehr auf dem SMB-Server",
        K::ResourceBusy => "Wird gerade von einem anderen Programm verwendet",
        K::DirectoryNotEmpty => "Ordner ist nicht leer",
        K::Interrupted => "Abgebrochen",
        _ => "SMB-Fehler",
    }
}

/// An operation error on `path`.
pub(super) fn map(error: Error, action: &str, path: &str) -> io::Error {
    let kind = kind_of(&error);
    io::Error::new(kind, format!("{}: {path} ({action}, {error})", label(kind)))
}

/// The TCP connect failed (no address answered, refused, timed out).
pub(super) fn connect_failed(host: &str, port: u16, error: Error) -> io::Error {
    let kind = match error.kind() {
        ErrorKind::TimedOut => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::ConnectionRefused,
    };
    io::Error::new(
        kind,
        format!("SMB-Server {host}:{port} nicht erreichbar ({error})"),
    )
}

/// NEGOTIATE failed: the server refused or did not understand SMB 2/3.
pub(super) fn negotiate_failed(error: Error) -> io::Error {
    let kind = match error.kind() {
        ErrorKind::TimedOut => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Unsupported,
    };
    io::Error::new(
        kind,
        format!("Server spricht nur SMB1 oder kein unterstütztes SMB2/3 ({error})"),
    )
}

/// SESSION_SETUP failed: refused credentials read as a sign-in failure,
/// everything else as a connection problem.
pub(super) fn session_failed(error: Error) -> io::Error {
    let refused = matches!(error, Error::Auth { .. })
        || matches!(
            error.kind(),
            ErrorKind::AuthRequired | ErrorKind::AccessDenied | ErrorKind::SigningRequired
        );
    if refused {
        return io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("SMB-Anmeldung abgelehnt: Benutzer, Passwort oder Domäne falsch ({error})"),
        );
    }
    let kind = match kind_of(&error) {
        io::ErrorKind::Other => io::ErrorKind::ConnectionAborted,
        kind => kind,
    };
    io::Error::new(
        kind,
        format!("SMB-Sitzung konnte nicht aufgebaut werden ({error})"),
    )
}

/// TREE_CONNECT to `share` failed.
pub(super) fn share_failed(share: &str, error: Error) -> io::Error {
    if matches!(error, Error::ShareRedirected { .. }) {
        return io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "SMB-Freigabe „{share}“ liegt auf einem anderen Clusterknoten (nicht unterstützt)"
            ),
        );
    }
    match error.kind() {
        ErrorKind::NotFound => io::Error::new(
            io::ErrorKind::NotFound,
            format!("{MISSING_SHARE_PREFIX}{share}{MISSING_SHARE_SUFFIX}"),
        ),
        ErrorKind::AccessDenied => io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Kein Zugriff auf die SMB-Freigabe „{share}“"),
        ),
        _ => map(error, "Freigabe verbinden", share),
    }
}

/// Whether a connect failure names a missing SMB share (facade kind
/// `not_found` instead of a network error).
pub fn names_missing_share(message: &str) -> bool {
    message.starts_with(MISSING_SHARE_PREFIX) && message.contains(MISSING_SHARE_SUFFIX)
}

/// An `io::ErrorKind` that means the connection itself is gone.
pub(super) fn is_connection_loss(kind: io::ErrorKind) -> bool {
    use io::ErrorKind as K;
    matches!(
        kind,
        K::ConnectionReset
            | K::ConnectionAborted
            | K::NotConnected
            | K::BrokenPipe
            | K::UnexpectedEof
    )
}

/// The connection is proven gone: replace it; idempotent reads may replay.
pub(super) fn is_dead(error: &Error) -> bool {
    match error {
        Error::Io(inner) => is_connection_loss(inner.kind()),
        _ => matches!(
            error.kind(),
            ErrorKind::ConnectionLost | ErrorKind::SessionExpired
        ),
    }
}

/// A timeout does not prove the request never arrived: the connection is
/// retired, but nothing is replayed.
pub(super) fn is_suspect(error: &Error) -> bool {
    error.kind() == ErrorKind::TimedOut
}
