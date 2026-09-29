//! Remote selections as virtual files for Explorer: Ctrl+C in a remote tab
//! puts a lazily listed FILEGROUPDESCRIPTORW/FILECONTENTS object on the
//! clipboard, and dragging out hands the same kind of object to DoDragDrop.
//! Nothing is listed or downloaded before Explorer asks. Recorded API
//! contract: `docs/refs/windows-virtual-files.md`.
#[path = "remote/catalog.rs"]
mod catalog;
#[path = "remote/data_object.rs"]
mod data_object;
#[path = "remote/fetch.rs"]
mod fetch;
#[path = "remote/handoff.rs"]
mod handoff;
#[path = "remote/names.rs"]
mod names;
#[path = "remote/prefetch.rs"]
mod prefetch;
#[path = "remote/producer.rs"]
mod producer;
#[path = "remote/session.rs"]
mod session;
#[path = "remote/signal.rs"]
mod signal;
#[path = "remote/stream.rs"]
mod stream;
#[path = "remote/worker.rs"]
mod worker;

#[cfg(test)]
#[path = "remote/test_fakes.rs"]
mod test_fakes;
#[cfg(test)]
#[path = "remote/test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "remote/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "remote/tests_resilience.rs"]
mod tests_resilience;

use crate::transfer::{flow_for, Flow, ListedEntry, SelectionListing, SelectionSource};
use handoff::{skip_to, Config, Handoff, RemoteContent};
use std::io::{self, Read};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_INVALIDARG, E_UNEXPECTED};
use windows::Win32::System::Com::Marshal::CoUnmarshalInterface;
use windows::Win32::System::Com::{IDataObject, IStream};
use worker::{Control, Mode, Started, RELEASE_DONE, RELEASE_FAILED};

impl RemoteContent for SelectionSource {
    fn display_label(&self) -> String {
        self.label.clone()
    }

    fn list_entries(
        &self,
        cancel: &AtomicBool,
        on_found: &(dyn Fn(u64) + Sync),
    ) -> SelectionListing {
        self.list_all(cancel, on_found)
    }

    fn open_entry(&self, entry: &ListedEntry) -> io::Result<Box<dyn Read + Send>> {
        self.open(entry)
    }

    fn open_entry_at(&self, entry: &ListedEntry, offset: u64) -> io::Result<Box<dyn Read + Send>> {
        if offset == 0 {
            return self.open(entry);
        }
        // Resume where the backend can start mid-file, otherwise read past.
        let resumed = self
            .backend
            .open_read_at(&entry.path, entry.id.as_deref(), offset)?;
        match resumed {
            Some(reader) => Ok(reader),
            None => skip_to(self.open(entry)?, offset),
        }
    }

    fn connection_flow(&self) -> Arc<Flow> {
        let first = self.paths.first().map(String::as_str).unwrap_or("/");
        flow_for(&*self.backend, first)
    }
}

fn require_selection(source: &SelectionSource) -> Result<()> {
    if source.paths.is_empty() {
        return Err(Error::new(E_INVALIDARG, "Keine Remote-Auswahl übergeben"));
    }
    Ok(())
}

/// Puts `source` on the clipboard as virtual files; Explorer lists and reads
/// it only when pasting. Returns the clipboard sequence number.
pub fn set_remote_clipboard(source: SelectionSource) -> Result<u32> {
    require_selection(&source)?;
    set_clipboard_with(Arc::new(source), Config::default()).map(|(sequence, _)| sequence)
}

fn set_clipboard_with(
    source: Arc<dyn RemoteContent>,
    config: Config,
) -> Result<(u32, Arc<Control>)> {
    let handoff = Handoff::new(source, config)?;
    match worker::start(handoff, Mode::Clipboard)? {
        (Started::Clipboard(sequence), control) => Ok((sequence, control)),
        (Started::Drag(_), _) => Err(Error::from(E_UNEXPECTED)),
    }
}

/// Prepares dragging `source` out as virtual files: the data object lives
/// on its own thread; `RemoteDrag::data_object` yields the proxy for
/// `DoDragDrop` on the calling (GUI) thread.
pub(crate) fn start_remote_drag(source: SelectionSource) -> Result<RemoteDrag> {
    require_selection(&source)?;
    start_drag_with(Arc::new(source), Config::default())
}

fn start_drag_with(source: Arc<dyn RemoteContent>, config: Config) -> Result<RemoteDrag> {
    let handoff = Handoff::new(source, config)?;
    match worker::start(handoff, Mode::Drag)? {
        (Started::Drag(marshaled), control) => Ok(RemoteDrag {
            marshaled: Some(marshaled.into_stream()),
            control,
            unmarshaled: false,
        }),
        (Started::Clipboard(_), control) => {
            control.release(RELEASE_FAILED);
            Err(Error::from(E_UNEXPECTED))
        }
    }
}

/// The GUI side of one drag. Dropping it tells the worker that the GUI
/// thread no longer needs the object; Explorer may keep using it (async
/// copy) until it releases it.
pub(crate) struct RemoteDrag {
    marshaled: Option<IStream>,
    control: Arc<Control>,
    unmarshaled: bool,
}

impl RemoteDrag {
    /// A proxy whose calls all run on the object's thread.
    pub(crate) fn data_object(&mut self) -> Result<IDataObject> {
        let stream = self
            .marshaled
            .take()
            .ok_or_else(|| Error::new(E_UNEXPECTED, "Das Datenobjekt wurde bereits übergeben"))?;
        let object = unsafe { CoUnmarshalInterface::<_, IDataObject>(&stream) }?;
        self.unmarshaled = true;
        Ok(object)
    }

    #[cfg(test)]
    fn control(&self) -> Arc<Control> {
        self.control.clone()
    }
}

impl Drop for RemoteDrag {
    fn drop(&mut self) {
        self.control.release(if self.unmarshaled {
            RELEASE_DONE
        } else {
            RELEASE_FAILED
        });
    }
}
