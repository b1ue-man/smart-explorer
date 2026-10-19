//! The clipboard of the layer backend (DSK-M7, U37-12): copy, cut and paste
//! of files between programs over `wl_data_device` (`text/uri-list`,
//! `x-special/gnome-copied-files`), and the text of egui's `Copy` and
//! `Paste`. Dropping files on the surface, dragging them out and input
//! methods are not part of it (`desktop-launcher.md`).
//!
//! The provider is installed for the actions of the view (`FileClipboard`),
//! which call it from the UI thread in the middle of a frame; so every
//! method here sends requests only and never needs the loop's state. The
//! offer of another program is read through a pipe with the waits of
//! [`RECEIVE_WAIT`] and [`RECEIVE_MAX`].
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use smithay_client_toolkit::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use smithay_client_toolkit::data_device_manager::data_offer::{
    DataOfferHandler, DragOffer, SelectionOffer,
};
use smithay_client_toolkit::data_device_manager::data_source::{
    CopyPasteSource, DataSourceHandler,
};
use smithay_client_toolkit::data_device_manager::{DataDeviceManagerState, WritePipe};
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_source::WlDataSource;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use super::clipboard_uri::{
    parse_file_list, Content, MIME_GNOME, MIME_TEXT, MIME_TEXT_UTF8, MIME_URI_LIST,
    MIME_UTF8_STRING,
};
use super::state::LayerState;
use crate::app::desktop_surface_clipboard::FileClipboard;

/// How long the clipboard waits for another program's answer: a program that
/// does not answer must not freeze the surface; local programs answer well
/// within it.
pub(super) const RECEIVE_WAIT: Duration = Duration::from_secs(2);
/// The most that is read from another program: protection against a source
/// that never ends; far more than a list of tens of thousands of files.
pub(super) const RECEIVE_MAX: usize = 16 * 1024 * 1024;

struct ClipDevice {
    seat: WlSeat,
    device: DataDevice,
}

/// A selection of ours: its source and what it holds.
struct Own {
    source: CopyPasteSource,
    content: Content,
}

#[derive(Default)]
struct ClipState {
    devices: Vec<ClipDevice>,
    own: Option<Own>,
    /// The seat and serial of the last key or button event on a surface; a
    /// selection is only accepted with such a serial.
    last: Option<(WlSeat, u32)>,
}

pub(super) struct LayerClipboard {
    state: Mutex<ClipState>,
    connection: Connection,
    qh: QueueHandle<LayerState>,
    manager: DataDeviceManagerState,
}

impl LayerClipboard {
    pub(super) fn new(
        connection: Connection,
        qh: QueueHandle<LayerState>,
        manager: DataDeviceManagerState,
    ) -> Arc<LayerClipboard> {
        Arc::new(LayerClipboard {
            state: Mutex::new(ClipState::default()),
            connection,
            qh,
            manager,
        })
    }

    fn lock(&self) -> MutexGuard<'_, ClipState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The data device of a seat; once per seat.
    pub(super) fn add_seat(&self, seat: &WlSeat) {
        let mut state = self.lock();
        if state.devices.iter().any(|device| &device.seat == seat) {
            return;
        }
        let device = self.manager.get_data_device(&self.qh, seat);
        state.devices.push(ClipDevice {
            seat: seat.clone(),
            device,
        });
    }

    pub(super) fn remove_seat(&self, seat: &WlSeat) {
        let mut state = self.lock();
        state.devices.retain(|device| &device.seat != seat);
        if state.last.as_ref().is_some_and(|(last, _)| last == seat) {
            state.last = None;
        }
    }

    /// Remembers the serial of an input event on a surface of ours.
    pub(super) fn note_serial(&self, seat: &WlSeat, serial: u32) {
        self.lock().last = Some((seat.clone(), serial));
    }

    fn set_selection(&self, content: Content) -> Result<(), String> {
        let mut state = self.lock();
        let (seat, serial) = state
            .last
            .clone()
            .ok_or("noch keine Eingabe auf der Fläche (Serie fehlt)")?;
        let device = state
            .devices
            .iter()
            .find(|device| device.seat == seat)
            .ok_or("kein Datengerät für die Eingabe")?;
        let source = self
            .manager
            .create_copy_paste_source(&self.qh, content.mimes().iter().copied());
        source.set_selection(&device.device, serial);
        state.own = Some(Own { source, content });
        Ok(())
    }

    /// egui's copied text becomes the selection.
    pub(super) fn write_text(&self, text: &str) {
        if let Err(error) = self.set_selection(Content::Text(text.to_string())) {
            eprintln!("se Desktop: Zwischenablage: {error}");
        }
    }

    /// The bytes a program asked for from our source; `None` when the source
    /// is not ours any more or the type is not offered.
    pub(super) fn bytes_for(&self, source: &WlDataSource, mime: &str) -> Option<Vec<u8>> {
        let state = self.lock();
        let own = state
            .own
            .as_ref()
            .filter(|own| own.source.inner() == source)?;
        own.content.bytes_for(mime)
    }

    /// Another program took the selection: our copy is gone.
    pub(super) fn cancelled(&self, source: &WlDataSource) {
        let mut state = self.lock();
        if state
            .own
            .as_ref()
            .is_some_and(|own| own.source.inner() == source)
        {
            state.own = None;
        }
    }

    /// The offer another program currently holds, on the seat of the last
    /// input or else the first one. Fetched fresh: the toolkit destroys an
    /// offer when the next one arrives.
    fn foreign_offer(state: &ClipState) -> Option<SelectionOffer> {
        let preferred = state
            .last
            .as_ref()
            .and_then(|(seat, _)| state.devices.iter().find(|device| &device.seat == seat));
        preferred
            .or_else(|| state.devices.first())
            .and_then(|device| device.device.data().selection_offer())
    }

    /// Reads one type of an offer through a pipe.
    fn receive(&self, offer: &SelectionOffer, mime: &str) -> Result<Vec<u8>, String> {
        let mut pipe = offer
            .receive(mime.to_string())
            .map_err(|error| format!("{error}"))?;
        self.connection
            .flush()
            .map_err(|error| format!("Verbindung: {error}"))?;
        let deadline = Instant::now() + RECEIVE_WAIT;
        let mut data = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            if !wait_readable(pipe.as_raw_fd(), deadline)? {
                return Err("die Quelle antwortet nicht".into());
            }
            match pipe.read(&mut buffer) {
                Ok(0) => return Ok(data),
                Ok(read) => {
                    data.extend_from_slice(&buffer[..read]);
                    if data.len() > RECEIVE_MAX {
                        return Err("die Quelle liefert zu viel".into());
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    /// The text for egui's `Paste`: our own text, or the text of another
    /// program's selection; empty when there is none or the selection is a
    /// file list (the view reads files itself, `FileClipboard::read`).
    pub(super) fn paste_text(&self) -> String {
        let offer = {
            let state = self.lock();
            match &state.own {
                Some(Own {
                    content: Content::Text(text),
                    ..
                }) => return text.clone(),
                Some(_) => return String::new(),
                None => Self::foreign_offer(&state),
            }
        };
        let Some(offer) = offer else {
            return String::new();
        };
        let mimes = offer.with_mime_types(<[String]>::to_vec);
        if mimes
            .iter()
            .any(|mime| mime == MIME_URI_LIST || mime == MIME_GNOME)
        {
            return String::new();
        }
        let wanted = [MIME_TEXT_UTF8, MIME_TEXT, MIME_UTF8_STRING]
            .into_iter()
            .find(|wanted| mimes.iter().any(|mime| mime == wanted));
        let Some(wanted) = wanted else {
            return String::new();
        };
        match self.receive(&offer, wanted) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).replace("\r\n", "\n"),
            Err(error) => {
                eprintln!("se Desktop: Zwischenablage: {error}");
                String::new()
            }
        }
    }
}

/// Waits until `fd` is readable (or hung up): `Ok(false)` at `deadline`.
fn wait_readable(fd: i32, deadline: Instant) -> Result<bool, String> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = i32::try_from(left.as_millis().max(1)).unwrap_or(i32::MAX);
        // SAFETY: one valid `pollfd` that lives through the call.
        let ready = unsafe { libc::poll(&mut poll, 1, millis) };
        if ready >= 0 {
            return Ok(ready > 0);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error.to_string());
        }
    }
}

impl FileClipboard for LayerClipboard {
    fn write(&self, files: &[PathBuf], cut: bool) -> Result<(), String> {
        self.set_selection(Content::Files {
            files: files.to_vec(),
            cut,
        })
    }

    fn read(&self) -> Result<Option<(Vec<PathBuf>, bool)>, String> {
        let offer = {
            let state = self.lock();
            match &state.own {
                Some(Own {
                    content: Content::Files { files, cut },
                    ..
                }) => return Ok(Some((files.clone(), *cut))),
                Some(_) => return Ok(None),
                None => Self::foreign_offer(&state),
            }
        };
        let Some(offer) = offer else {
            return Ok(None);
        };
        let mimes = offer.with_mime_types(<[String]>::to_vec);
        let wanted = [MIME_GNOME, MIME_URI_LIST]
            .into_iter()
            .find(|wanted| mimes.iter().any(|mime| mime == wanted));
        let Some(wanted) = wanted else {
            return Ok(None);
        };
        let body = self.receive(&offer, wanted)?;
        Ok(parse_file_list(
            wanted,
            &String::from_utf8_lossy(&body),
            |path| path.exists(),
        ))
    }
}

impl DataSourceHandler for LayerState {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        fd: WritePipe,
    ) {
        let Some(bytes) = self
            .clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.bytes_for(source, &mime))
        else {
            // The pipe closes unwritten: the reader sees an empty answer.
            return;
        };
        let fd = OwnedFd::from(fd);
        let _ = std::thread::Builder::new()
            .name("se-desktop-clipboard".into())
            .spawn(move || {
                let _ = File::from(fd).write_all(&bytes);
            });
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if let Some(clipboard) = &self.clipboard {
            clipboard.cancelled(source);
        }
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

/// Dropping files on the surface and dragging them out are not supported
/// (`DESKTOP-WAYLAND-DND`): drag offers are never accepted.
impl DataDeviceHandler for LayerState {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &WlSurface,
    ) {
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for LayerState {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}
