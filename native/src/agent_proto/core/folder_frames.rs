//! Additive `folder-reservation-v1` and `file-identity-v1` object frames.
//! Separate labels gate their requests;
//! existing stage/capacity tags and legacy request layouts stay unchanged.
use super::super::super::types::Frame;
use super::super::{bad, Reader};
use super::{put_bool, put_str, string_len};
use std::io;

const MAX_PATH: usize = 32_768;
const MAX_IDENTITY: usize = 65_536;

pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    match frame {
        Frame::CreateDirExclusive(path) | Frame::DirectoryIdentity(path)
        | Frame::DirectoryReservationSupported(path) | Frame::FileIdentity(path) => Some(
            bounded_len(path, MAX_PATH)),
        Frame::RenameDirectoryNoReplace { src, dst } => Some(
            bounded_len(src, MAX_PATH).and_then(|left|
                bounded_len(dst, MAX_PATH).map(|right| left + right))),
        Frame::DirectoryIdentityValue(identity) | Frame::FileIdentityValue(identity) => Some(match identity {
            Some(value) => bounded_len(value, MAX_IDENTITY).map(|length| length + 1),
            None => Ok(1),
        }),
        _ => None,
    }
}
fn bounded_len(value: &str, maximum: usize) -> io::Result<usize> {
    if value.len() > maximum { return Err(bad("folder metadata exceeds protocol limit")); }
    string_len(value)
}
pub(in super::super) fn encode(frame: &Frame, bytes: &mut Vec<u8>) -> bool {
    match frame {
        Frame::CreateDirExclusive(path) => { bytes.push(72); put_str(bytes, path); }
        Frame::DirectoryIdentity(path) => { bytes.push(73); put_str(bytes, path); }
        Frame::DirectoryReservationSupported(path) => { bytes.push(76); put_str(bytes, path); }
        Frame::FileIdentity(path) => { bytes.push(77); put_str(bytes, path); }
        Frame::RenameDirectoryNoReplace { src, dst } => {
            bytes.push(75); put_str(bytes, src); put_str(bytes, dst);
        }
        Frame::DirectoryIdentityValue(identity) => {
            bytes.push(74);
            put_bool(bytes, identity.is_some());
            if let Some(value) = identity { put_str(bytes, value); }
        }
        Frame::FileIdentityValue(identity) => {
            bytes.push(78);
            put_bool(bytes, identity.is_some());
            if let Some(value) = identity { put_str(bytes, value); }
        }
        _ => return false,
    }
    true
}
pub(in super::super) fn decode(tag: u8, reader: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(match tag {
        72 => Some(Frame::CreateDirExclusive(reader.tree_string(MAX_PATH)?)),
        73 => Some(Frame::DirectoryIdentity(reader.tree_string(MAX_PATH)?)),
        76 => Some(Frame::DirectoryReservationSupported(reader.tree_string(MAX_PATH)?)),
        77 => Some(Frame::FileIdentity(reader.tree_string(MAX_PATH)?)),
        75 => Some(Frame::RenameDirectoryNoReplace {
            src: reader.tree_string(MAX_PATH)?, dst: reader.tree_string(MAX_PATH)?,
        }),
        74 => Some(Frame::DirectoryIdentityValue(if reader.bool()? {
            Some(reader.tree_string(MAX_IDENTITY)?)
        } else { None })),
        78 => Some(Frame::FileIdentityValue(if reader.bool()? {
            Some(reader.tree_string(MAX_IDENTITY)?)
        } else { None })),
        _ => None,
    })
}

impl Frame {
    /// Preserve exact failure classes of the new contract through legacy Err
    /// frames; only negotiated object requests emit this bounded vocabulary.
    pub(crate) fn folder_reply(result: io::Result<Frame>) -> Frame {
        match result {
            Ok(frame) => frame,
            Err(error) => {
                use io::ErrorKind::*;
                let code = match error.kind() {
                    AlreadyExists => "exists", NotFound => "missing", PermissionDenied => "denied",
                    ReadOnlyFilesystem => "readonly", Unsupported => "unsupported",
                    InvalidData => "data", InvalidInput | NotADirectory => "input",
                    Interrupted => "interrupted", TimedOut => "timeout", BrokenPipe => "pipe",
                    ConnectionReset => "reset", ConnectionAborted => "aborted",
                    NotConnected => "disconnected", StorageFull => "full", QuotaExceeded => "quota",
                    _ => "other",
                };
                Frame::Err(format!("SE_FOLDER_V1 {code} {error}"))
            }
        }
    }
    pub(crate) fn folder_error(message: &str) -> Option<io::Error> {
        let (code, message) = message.strip_prefix("SE_FOLDER_V1 ")?.split_once(' ')?;
        use io::ErrorKind::*;
        let kind = match code {
            "exists" => AlreadyExists, "missing" => NotFound, "denied" => PermissionDenied,
            "readonly" => ReadOnlyFilesystem, "unsupported" => Unsupported,
            "data" => InvalidData, "input" => InvalidInput, "interrupted" => Interrupted,
            "timeout" => TimedOut, "pipe" => BrokenPipe, "reset" => ConnectionReset,
            "aborted" => ConnectionAborted, "disconnected" => NotConnected,
            "full" => StorageFull, "quota" => QuotaExceeded, "other" => Other,
            _ => return None,
        };
        Some(io::Error::new(kind, message.to_string()))
    }
}
