//! Channel between the shell (`--desktop-shell`) and its surfaces (C5.5, Ä14).
//!
//! A frame is a 4-byte length (u32, little endian) followed by UTF-8 JSON; the
//! only bound is the u32 range, and a reader allocates only for bytes that
//! actually arrive. Transport: stdin/stdout of the surface process, or under
//! GNOME the Unix socket of `desktop-gnome.json` (first frame with token).
//! The end of the stream at a frame boundary is the end of the channel: the
//! surface then exits with code 76.
use std::fmt;
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::config::DesktopConfig;
use super::items::{DesktopFile, SystemItem};
use super::layout::{LayoutOp, MonitorLayout};
use super::tiles::TileModel;

pub(crate) const PROTOCOL_VERSION: u32 = 1;
/// Exit code of a surface whose channel ended (C5.5); never a crash.
pub(crate) const EXIT_CHANNEL_END: i32 = 76;

/// Why the shell stops its surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopReason {
    Disable,
    Update,
    Restart,
}

/// What one surface shows (DSK-M4 draws it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SurfaceModel {
    pub(crate) tiles: TileModel,
    pub(crate) files: Vec<DesktopFile>,
    pub(crate) system_items: Vec<SystemItem>,
    /// The tile "Alle Bildschirme".
    pub(crate) screens: bool,
    /// The arrangement of this surface's monitor.
    pub(crate) layout: MonitorLayout,
}

/// Shell → surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ShellMessage {
    Hello { protocol: u32, monitor: String },
    Model { rev: u64, model: Box<SurfaceModel> },
    Config { config: DesktopConfig },
    Stop { reason: StopReason },
}

/// Surface → shell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SurfaceMessage {
    Hello {
        protocol: u32,
        monitor: String,
        pid: u32,
        /// 128-bit token of `desktop.token`, only on the GNOME socket.
        token: Option<String>,
    },
    /// At most once per second after a drawn frame (heartbeat).
    Alive,
    Layout {
        op: LayoutOp,
    },
    MonitorsChanged,
    WallpaperChanged,
    /// The button "Verbinden" (starts the background service, C10).
    Connect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolError {
    Io(String),
    /// The stream ended inside a frame.
    Truncated {
        expected: u64,
        received: u64,
    },
    TooLarge(usize),
    Json(String),
    Hello(String),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::Io(error) => write!(formatter, "Kanal: {error}"),
            ProtocolError::Truncated { expected, received } => write!(
                formatter,
                "Kanal: Rahmen abgeschnitten ({received} von {expected} Byte)"
            ),
            ProtocolError::TooLarge(length) => {
                write!(formatter, "Kanal: Rahmen mit {length} Byte zu groß")
            }
            ProtocolError::Json(error) => write!(formatter, "Kanal: ungültige Nachricht: {error}"),
            ProtocolError::Hello(error) => write!(formatter, "Kanal: Begrüßung abgelehnt: {error}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// One frame: length prefix and JSON body.
pub(crate) fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, ProtocolError> {
    let body =
        serde_json::to_vec(message).map_err(|error| ProtocolError::Json(error.to_string()))?;
    let length = u32::try_from(body.len()).map_err(|_| ProtocolError::TooLarge(body.len()))?;
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub(crate) fn write_frame<W: Write, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<(), ProtocolError> {
    let frame = encode_frame(message)?;
    writer
        .write_all(&frame)
        .and_then(|()| writer.flush())
        .map_err(|error| ProtocolError::Io(error.to_string()))
}

/// The next message; `Ok(None)` when the stream ends at a frame boundary.
pub(crate) fn read_frame<R: Read, T: DeserializeOwned>(
    reader: &mut R,
) -> Result<Option<T>, ProtocolError> {
    let mut header = [0u8; 4];
    let mut filled = 0;
    while filled < header.len() {
        match reader.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => {
                return Err(ProtocolError::Truncated {
                    expected: 4,
                    received: filled as u64,
                })
            }
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ProtocolError::Io(error.to_string())),
        }
    }
    let expected = u64::from(u32::from_le_bytes(header));
    let mut body = Vec::new();
    reader
        .by_ref()
        .take(expected)
        .read_to_end(&mut body)
        .map_err(|error| ProtocolError::Io(error.to_string()))?;
    let received = body.len() as u64;
    if received != expected {
        return Err(ProtocolError::Truncated { expected, received });
    }
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| ProtocolError::Json(error.to_string()))
}

/// Every complete frame in `bytes`, and the length of a trailing partial one.
pub(crate) fn decode_frames<T: DeserializeOwned>(
    bytes: &[u8],
) -> Result<(Vec<T>, usize), ProtocolError> {
    let mut messages = Vec::new();
    let mut rest = bytes;
    while rest.len() >= 4 {
        let length = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        let end = length.saturating_add(4);
        let Some(body) = rest.get(4..end) else {
            break;
        };
        let message: T =
            serde_json::from_slice(body).map_err(|error| ProtocolError::Json(error.to_string()))?;
        messages.push(message);
        rest = &rest[end..];
    }
    Ok((messages, rest.len()))
}

/// The shell's check of a surface greeting: protocol 1, the monitor it
/// started the surface for, and on the GNOME socket the token of
/// `desktop.token` (compared in constant time).
pub(crate) fn check_hello(
    message: &SurfaceMessage,
    expected_monitor: Option<&str>,
    expected_token: Option<&str>,
) -> Result<(), ProtocolError> {
    let SurfaceMessage::Hello {
        protocol,
        monitor,
        token,
        ..
    } = message
    else {
        return Err(ProtocolError::Hello(
            "erste Nachricht ist keine Begrüßung".into(),
        ));
    };
    if *protocol != PROTOCOL_VERSION {
        return Err(ProtocolError::Hello(format!("Protokoll {protocol}")));
    }
    if expected_monitor.is_some_and(|expected| expected != monitor.as_str()) {
        return Err(ProtocolError::Hello(format!("fremder Monitor {monitor:?}")));
    }
    if let Some(expected) = expected_token {
        let given = token.as_deref().unwrap_or_default();
        if !same_secret(expected.as_bytes(), given.as_bytes()) {
            return Err(ProtocolError::Hello("Token falsch".into()));
        }
    }
    Ok(())
}

fn same_secret(expected: &[u8], given: &[u8]) -> bool {
    if expected.len() != given.len() {
        return false;
    }
    expected
        .iter()
        .zip(given)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}
