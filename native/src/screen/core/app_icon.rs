//! P3/V6: a list contains references only; content travels in a separate bounded reply.
//! An icon hash authenticates bytes, never a filesystem path or an app permission.
use std::io::{self, Cursor};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::window_list::{AppEntry, WindowEntry, WindowList};

pub(crate) const ICON_VERSION: u8 = 1;
/// Product resource budget: one RGBA icon is at most 256 KiB before encoding.
pub(crate) const MAX_ICON_SIDE: u32 = 256;
pub(crate) const MAX_ICON_PIXELS: u32 = MAX_ICON_SIDE * MAX_ICON_SIDE;
pub(crate) const MAX_ICON_BYTES: usize = 256 * 1024;
pub(crate) const MAX_ICON_BASE64_BYTES: usize = MAX_ICON_BYTES.div_ceil(3) * 4;
pub(crate) const MAX_ICON_REQUEST_BYTES: usize = 4096;
pub(crate) const MAX_ICON_REPLY_BYTES: usize =
    MAX_ICON_BASE64_BYTES + MAX_ICON_REQUEST_BYTES * 2 + 1024;
pub(crate) const MAX_APP_ID_BYTES: usize = 1024;

pub(crate) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_APP_ID_BYTES && !id.chars().any(char::is_control)
}

pub(crate) fn checked_pixels(width: u32, height: u32) -> io::Result<usize> {
    if width == 0 || height == 0 || width > MAX_ICON_SIDE || height > MAX_ICON_SIDE {
        return Err(invalid("icon dimensions exceed the icon budget"));
    }
    width
        .checked_mul(height)
        .filter(|pixels| *pixels <= MAX_ICON_PIXELS)
        .and_then(|pixels| usize::try_from(pixels).ok())
        .ok_or_else(|| invalid("icon pixel count exceeds the icon budget"))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct IconRef {
    pub version: u8,
    pub app_id: String,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}

impl IconRef {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.version != ICON_VERSION
            || !valid_app_id(&self.app_id)
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("invalid icon reference"));
        }
        checked_pixels(self.width, self.height)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum IconTarget {
    Window { id: String },
    App { app_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct IconRequest {
    pub target: IconTarget,
    pub icon: IconRef,
}

/// Created only by comparing the request to the caller's freshly authorized list.
/// OS adapters accept this value instead of a remotely supplied path/hash.
pub(crate) enum AuthorizedIconEntry<'a> {
    Window(&'a WindowEntry),
    App(&'a AppEntry),
}

impl IconRequest {
    pub(crate) fn validate(&self) -> io::Result<()> {
        self.icon.validate()?;
        let valid = match &self.target {
            IconTarget::Window { id } => {
                !id.is_empty()
                    && id.len() <= super::window_list::MAX_ID_BYTES
                    && id.bytes().all(|byte| byte.is_ascii_graphic())
            }
            IconTarget::App { app_id } => valid_app_id(app_id) && *app_id == self.icon.app_id,
        };
        if !valid {
            return Err(invalid("invalid icon target"));
        }
        Ok(())
    }

    /// The caller must enforce current screen-list permission before obtaining `list`.
    /// A missing/changed entry, app or hash fails closed without touching an OS source.
    pub(crate) fn authorize<'a>(
        &self,
        list: &'a WindowList,
    ) -> io::Result<AuthorizedIconEntry<'a>> {
        self.validate()?;
        match &self.target {
            IconTarget::Window { id } => {
                let mut entries = list.windows.iter().filter(|entry| &entry.id == id);
                if let Some(entry) = entries.next().filter(|entry| {
                    entry.icon.as_ref() == Some(&self.icon)
                        && entry.app_id.as_deref() == Some(self.icon.app_id.as_str())
                }) {
                    if entries.next().is_none() {
                        return Ok(AuthorizedIconEntry::Window(entry));
                    }
                }
            }
            IconTarget::App { app_id } => {
                let mut entries = list.apps.iter().filter(|entry| &entry.app_id == app_id);
                if let Some(entry) = entries
                    .next()
                    .filter(|entry| entry.icon.as_ref() == Some(&self.icon))
                {
                    if entries.next().is_none() {
                        return Ok(AuthorizedIconEntry::App(entry));
                    }
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "icon entry is absent, ambiguous or changed",
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IconReply {
    pub icon: IconRef,
    pub png_base64: String,
}

/// Always correlates a response, including a vanished/changed icon, to its exact request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IconResponse {
    pub request: IconRequest,
    pub content: Option<IconReply>,
}

impl IconResponse {
    pub(crate) fn validate(&self) -> io::Result<()> {
        self.request.validate()?;
        if let Some(content) = &self.content {
            content.bytes_for(&self.request.icon)?;
        }
        Ok(())
    }
}

impl IconReply {
    pub(crate) fn new(app_id: &str, png: &[u8]) -> io::Result<Self> {
        if !valid_app_id(app_id) {
            return Err(invalid("invalid icon app identity"));
        }
        let (width, height) = png_dimensions(png)?;
        Ok(Self {
            icon: IconRef {
                version: ICON_VERSION,
                app_id: app_id.into(),
                sha256: format!("{:x}", Sha256::digest(png)),
                width,
                height,
            },
            png_base64: STANDARD.encode(png),
        })
    }

    pub(crate) fn bytes_for(&self, expected: &IconRef) -> io::Result<Vec<u8>> {
        expected.validate()?;
        if &self.icon != expected || self.png_base64.len() > MAX_ICON_BASE64_BYTES {
            return Err(invalid(
                "icon reply does not match its reference or byte budget",
            ));
        }
        let png = STANDARD
            .decode(&self.png_base64)
            .map_err(|_| invalid("invalid icon base64"))?;
        let (width, height) = png_dimensions(&png)?;
        if (width, height) != (expected.width, expected.height)
            || format!("{:x}", Sha256::digest(&png)) != expected.sha256
        {
            return Err(invalid("icon hash or dimensions changed"));
        }
        Ok(png)
    }
}

/// Check the PNG IHDR before the decoder allocates its image/scanline buffers.
pub(crate) fn png_dimensions(png: &[u8]) -> io::Result<(u32, u32)> {
    if png.len() > MAX_ICON_BYTES
        || png.len() < 33
        || &png[..8] != b"\x89PNG\r\n\x1a\n"
        || png[8..12] != [0, 0, 0, 13]
        || &png[12..16] != b"IHDR"
    {
        return Err(invalid("invalid or oversized icon PNG"));
    }
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    checked_pixels(width, height)?;
    Ok((width, height))
}

/// Strict PNG decoder, shared by all viewers; no GIF/JPEG/SVG decoder on the wire.
pub(crate) fn decode_png(png: &[u8]) -> io::Result<(u32, u32, Vec<u8>)> {
    let (width, height) = png_dimensions(png)?;
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_limits(png::Limits {
        bytes: MAX_ICON_BYTES * 8,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|_| invalid("invalid icon PNG header"))?;
    let bytes = reader
        .output_buffer_size()
        .filter(|bytes| *bytes <= MAX_ICON_BYTES)
        .ok_or_else(|| invalid("decoded icon exceeds its pixel budget"))?;
    let mut decoded = vec![0; bytes];
    let output = reader
        .next_frame(&mut decoded)
        .map_err(|_| invalid("invalid icon PNG pixels"))?;
    if output.width != width || output.height != height || reader.info().animation_control.is_some()
    {
        return Err(invalid("animated or inconsistent icon PNG"));
    }
    let mut rgba = Vec::with_capacity(checked_pixels(width, height)? * 4);
    let decoded = &decoded[..output.buffer_size()];
    match output.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(decoded),
        png::ColorType::Rgb => {
            for p in decoded.chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in decoded.chunks_exact(2) {
                rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        png::ColorType::Grayscale => {
            for p in decoded {
                rgba.extend_from_slice(&[*p, *p, *p, 255]);
            }
        }
        png::ColorType::Indexed => return Err(invalid("unexpanded icon palette")),
    }
    if rgba.len() != checked_pixels(width, height)? * 4 {
        return Err(invalid("invalid icon pixel length"));
    }
    Ok((width, height, rgba))
}

#[cfg(test)]
#[path = "batches_icon_task_tests.rs"]
mod batches_task_tests;
