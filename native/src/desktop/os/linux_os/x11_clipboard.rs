//! The file clipboard of an X11 surface (U34-8): copy, cut and paste of
//! files with other programs over the selection `CLIPBOARD`, as
//! `text/uri-list` and `x-special/gnome-copied-files` (plus the surface's
//! marker text). The provider has a connection of its own with an invisible
//! window (input only, 1×1) as the owner of the selection; a thread answers
//! the requests of other programs, the calls of the view send requests on the
//! same connection.
//!
//! Limit (Ä, `docs/TODO.md` `DESKTOP-X11-CLIPBOARD-INCR`): no `INCR`. A
//! property is limited by the largest request of the server, so a list that
//! does not fit is refused when copying ("Zu viele Dateien für die
//! Zwischenablage (X11 ohne INCR)"), and an offer of another program that
//! answers with `INCR` reads as "no file list".
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};
use std::time::Duration;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent,
    SelectionRequestEvent, Window, WindowClass, SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::NONE;

use super::clipboard_formats::{bytes_for, parse_file_list, FILE_TARGETS, GNOME_COPIED, URI_LIST};
use super::x11_facts::intern;
use crate::app::desktop_surface_clipboard::{install, FileClipboard};

/// How long a read waits for the other program (a program that does not
/// answer must not freeze the surface; local programs answer well within it).
const RECEIVE_WAIT: Duration = Duration::from_secs(2);
/// The most that is read from another program; far more than a list of tens
/// of thousands of files.
const RECEIVE_MAX: usize = 16 * 1024 * 1024;
/// Room for the request header next to the data of a property.
const REQUEST_SLACK: usize = 1024;
const TOO_MANY: &str = "Zu viele Dateien für die Zwischenablage (X11 ohne INCR)";

struct Atoms {
    clipboard: Atom,
    targets: Atom,
    timestamp: Atom,
    incr: Atom,
    inbox: Atom,
    stamp_probe: Atom,
    files: Vec<(Atom, &'static str)>,
}

#[derive(Clone)]
struct Own {
    files: Vec<PathBuf>,
    cut: bool,
    time: u32,
}

struct Shared {
    conn: RustConnection,
    window: Window,
    atoms: Atoms,
    own: Mutex<Option<Own>>,
    /// Where the thread passes a `SelectionNotify` (target, property).
    notify: Mutex<Option<Sender<(Atom, Atom)>>>,
    /// Where the thread passes the time of a probe `PropertyNotify`.
    stamp: Mutex<Option<Sender<u32>>>,
    /// One read at a time.
    reading: Mutex<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct X11Clipboard {
    shared: Arc<Shared>,
}

impl Shared {
    fn target_name(&self, target: Atom) -> Option<&'static str> {
        self.atoms
            .files
            .iter()
            .find(|(atom, _)| *atom == target)
            .map(|(_, name)| *name)
    }

    /// Answers a request of another program (ICCCM 2.2).
    fn answer(&self, request: &SelectionRequestEvent) {
        let property = if request.property == NONE {
            request.target
        } else {
            request.property
        };
        let done = self.fill(request, property);
        let reply = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if done { property } else { NONE },
        };
        let _ = self
            .conn
            .send_event(false, request.requestor, EventMask::NO_EVENT, reply);
        let _ = self.conn.flush();
    }

    fn fill(&self, request: &SelectionRequestEvent, property: Atom) -> bool {
        let Some(own) = lock(&self.own).clone() else {
            return false;
        };
        if request.selection != self.atoms.clipboard {
            return false;
        }
        let (conn, requestor) = (&self.conn, request.requestor);
        let atoms = &self.atoms;
        if request.target == atoms.targets {
            let mut offered = vec![atoms.targets, atoms.timestamp];
            offered.extend(atoms.files.iter().map(|(atom, _)| *atom));
            return conn
                .change_property32(
                    PropMode::REPLACE,
                    requestor,
                    property,
                    AtomEnum::ATOM,
                    &offered,
                )
                .is_ok();
        }
        if request.target == atoms.timestamp {
            return conn
                .change_property32(
                    PropMode::REPLACE,
                    requestor,
                    property,
                    AtomEnum::INTEGER,
                    &[own.time],
                )
                .is_ok();
        }
        let Some(name) = self.target_name(request.target) else {
            return false;
        };
        let Some(bytes) = bytes_for(&own.files, own.cut, name) else {
            return false;
        };
        conn.change_property8(
            PropMode::REPLACE,
            requestor,
            property,
            request.target,
            &bytes,
        )
        .is_ok()
    }

    /// The events of the connection: requests, a lost selection, the answers
    /// a read or a write waits for.
    fn run(&self) {
        while let Ok(event) = self.conn.wait_for_event() {
            match event {
                Event::SelectionRequest(request) => self.answer(&request),
                Event::SelectionClear(clear) if clear.selection == self.atoms.clipboard => {
                    *lock(&self.own) = None;
                }
                Event::SelectionNotify(notify) => {
                    if let Some(sender) = lock(&self.notify).as_ref() {
                        let _ = sender.send((notify.target, notify.property));
                    }
                }
                Event::PropertyNotify(change) if change.atom == self.atoms.stamp_probe => {
                    if let Some(sender) = lock(&self.stamp).take() {
                        let _ = sender.send(change.time);
                    }
                }
                _ => {}
            }
        }
    }

    /// A server time for `SetSelectionOwner` (an empty append to our window
    /// makes the server send a `PropertyNotify` with its time).
    fn server_time(&self) -> u32 {
        let (sender, receiver) = mpsc::channel();
        *lock(&self.stamp) = Some(sender);
        let sent = self.conn.change_property8(
            PropMode::APPEND,
            self.window,
            self.atoms.stamp_probe,
            AtomEnum::STRING,
            &[],
        );
        if sent.is_err() || self.conn.flush().is_err() {
            return x11rb::CURRENT_TIME;
        }
        receiver
            .recv_timeout(RECEIVE_WAIT)
            .unwrap_or(x11rb::CURRENT_TIME)
    }

    fn owner(&self) -> Option<Window> {
        let owner = self
            .conn
            .get_selection_owner(self.atoms.clipboard)
            .ok()?
            .reply()
            .ok()?
            .owner;
        (owner != NONE).then_some(owner)
    }

    /// The body of the other program's answer to one target; `Ok(None)`
    /// for a refusal, an `INCR` transfer or no answer in time.
    fn receive(&self, target: Atom) -> Result<Option<String>, String> {
        let (sender, receiver) = mpsc::channel();
        *lock(&self.notify) = Some(sender);
        let asked = self.conn.convert_selection(
            self.window,
            self.atoms.clipboard,
            target,
            self.atoms.inbox,
            x11rb::CURRENT_TIME,
        );
        if asked.is_err() || self.conn.flush().is_err() {
            *lock(&self.notify) = None;
            return Err("Zwischenablage: keine Verbindung zum X-Server".into());
        }
        let answer = loop {
            match receiver.recv_timeout(RECEIVE_WAIT) {
                Ok((answered, property)) if answered == target => break Some(property),
                Ok(_) => {}
                Err(_) => break None,
            }
        };
        *lock(&self.notify) = None;
        let Some(property) = answer.filter(|property| *property != NONE) else {
            return Ok(None);
        };
        let units = u32::try_from(RECEIVE_MAX / 4 + 1).unwrap_or(u32::MAX);
        let reply = self
            .conn
            .get_property(true, self.window, property, AtomEnum::ANY, 0, units)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        if reply.type_ == self.atoms.incr {
            return Ok(None);
        }
        if reply.bytes_after > 0 || reply.value.len() > RECEIVE_MAX {
            return Err("Die Zwischenablage ist zu groß (mehr als 16 MiB)".into());
        }
        Ok(Some(String::from_utf8_lossy(&reply.value).into_owned()))
    }
}

impl X11Clipboard {
    pub(crate) fn start() -> Result<X11Clipboard, String> {
        let (conn, screen) =
            RustConnection::connect(None).map_err(|error| format!("X11: {error}"))?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .map(|screen| screen.root)
            .ok_or("X11: kein Bildschirm")?;
        let need = |name: &str| intern(&conn, name).ok_or_else(|| format!("X11: Atom {name}"));
        let mut files = Vec::new();
        for name in FILE_TARGETS {
            files.push((need(name)?, name));
        }
        let atoms = Atoms {
            clipboard: need("CLIPBOARD")?,
            targets: need("TARGETS")?,
            timestamp: need("TIMESTAMP")?,
            incr: need("INCR")?,
            inbox: need("SE_CLIPBOARD_IN")?,
            stamp_probe: need("SE_CLIPBOARD_TIME")?,
            files,
        };
        let window = conn.generate_id().map_err(|error| error.to_string())?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|error| error.to_string())?;
        conn.flush().map_err(|error| error.to_string())?;
        let shared = Arc::new(Shared {
            conn,
            window,
            atoms,
            own: Mutex::new(None),
            notify: Mutex::new(None),
            stamp: Mutex::new(None),
            reading: Mutex::new(()),
        });
        let runner = shared.clone();
        std::thread::Builder::new()
            .name("x11-clipboard".into())
            .spawn(move || runner.run())
            .map_err(|error| error.to_string())?;
        Ok(X11Clipboard { shared })
    }
}

impl FileClipboard for X11Clipboard {
    fn write(&self, files: &[PathBuf], cut: bool) -> Result<(), String> {
        let shared = &self.shared;
        let limit = shared
            .conn
            .maximum_request_bytes()
            .saturating_sub(REQUEST_SLACK);
        let largest = FILE_TARGETS
            .iter()
            .filter_map(|name| bytes_for(files, cut, name))
            .map(|bytes| bytes.len())
            .max()
            .unwrap_or(0);
        if largest > limit {
            return Err(TOO_MANY.into());
        }
        let time = shared.server_time();
        *lock(&shared.own) = Some(Own {
            files: files.to_vec(),
            cut,
            time,
        });
        let claimed = shared
            .conn
            .set_selection_owner(shared.window, shared.atoms.clipboard, time)
            .map_err(|error| error.to_string())
            .and_then(|_| shared.conn.flush().map_err(|error| error.to_string()));
        if let Err(error) = claimed {
            *lock(&shared.own) = None;
            return Err(format!("Zwischenablage: {error}"));
        }
        if shared.owner() != Some(shared.window) {
            *lock(&shared.own) = None;
            return Err("Zwischenablage: der X-Server hat die Auswahl nicht übergeben".into());
        }
        Ok(())
    }

    fn read(&self) -> Result<Option<(Vec<PathBuf>, bool)>, String> {
        let shared = &self.shared;
        let _one_at_a_time = lock(&shared.reading);
        match shared.owner() {
            None => return Ok(None),
            Some(owner) if owner == shared.window => {
                return Ok(lock(&shared.own).clone().map(|own| (own.files, own.cut)));
            }
            Some(_) => {}
        }
        for (name, mime) in [("gnome", GNOME_COPIED), ("uri", URI_LIST)] {
            let target = shared
                .atoms
                .files
                .iter()
                .find(|(_, known)| *known == mime)
                .map(|(atom, _)| *atom)
                .ok_or_else(|| format!("Atom {name}"))?;
            if let Some(body) = shared.receive(target)? {
                if let Some(found) = parse_file_list(mime, &body, |path| path.exists()) {
                    return Ok(Some(found));
                }
            }
        }
        Ok(None)
    }
}

static INSTALLED: Once = Once::new();

/// Installs the provider of this process once; without a connection the
/// file clipboard stays "not available".
pub(crate) fn install_once() {
    INSTALLED.call_once(|| match X11Clipboard::start() {
        Ok(clipboard) => install(Arc::new(clipboard)),
        Err(error) => eprintln!("se Desktop: Datei-Zwischenablage: {error}"),
    });
}

#[cfg(test)]
#[path = "x11_clipboard_tests.rs"]
mod tests;
