//! The STA thread that owns one remote data object: it puts the object on
//! the clipboard or marshals it for a drag, pumps messages while Explorer
//! works with it, and ends once the last object is released. It never calls
//! `OleFlushClipboard`, which would render every file's contents at once.
use super::data_object::RemoteDataObject;
use super::handoff::Handoff;
use super::signal::{Signal, WAIT_SLICE};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use windows::core::{Error, Interface, Result};
use windows::Win32::Foundation::{E_OUTOFMEMORY, E_UNEXPECTED, HGLOBAL, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::Marshal::CoMarshalInterface;
use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
use windows::Win32::System::Com::{
    CoDisconnectObject, IDataObject, IStream, MSHCTX_LOCAL, MSHLFLAGS_NORMAL, STREAM_SEEK_SET,
};
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::System::Ole::{OleInitialize, OleSetClipboard, OleUninitialize};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, PostThreadMessageW, SetTimer,
    TranslateMessage, MSG, PM_NOREMOVE, WM_APP, WM_USER,
};

/// Back-stop tick of the worker's loop: a modal loop may swallow the
/// wake-up message, so the thread notices its end within a second anyway.
const TICK_MS: u32 = 1_000;

pub(super) const RELEASE_PENDING: u8 = 0;
pub(super) const RELEASE_DONE: u8 = 1;
pub(super) const RELEASE_FAILED: u8 = 2;

/// Coordination between the worker thread and its objects and caller.
#[derive(Default)]
pub(super) struct Control {
    thread: AtomicU32,
    ended: AtomicBool,
    release: AtomicU8,
    exited: AtomicBool,
}

impl Control {
    fn wake(&self) {
        let thread = self.thread.load(Ordering::Acquire);
        if thread != 0 && !self.exited.load(Ordering::Acquire) {
            // A lost wake-up only delays the end until the next tick.
            let _ = unsafe { PostThreadMessageW(thread, WM_APP, WPARAM(0), LPARAM(0)) };
        }
    }

    /// The drag's GUI side is done with its proxy (or never got one).
    pub(super) fn release(&self, state: u8) {
        self.release.store(state, Ordering::Release);
        self.wake();
    }

    /// The worker thread has ended (tests wait for it).
    #[cfg(test)]
    pub(super) fn exited(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }
}

/// Held by every COM object of one hand-off; dropping the last one closes
/// the hand-off and ends the worker.
pub(super) struct LifeToken {
    handoff: Arc<Handoff>,
    control: Arc<Control>,
}

impl LifeToken {
    /// For objects without a worker thread (in-process tests).
    #[cfg(test)]
    pub(super) fn detached(handoff: Arc<Handoff>) -> Arc<Self> {
        Arc::new(Self {
            handoff,
            control: Arc::new(Control::default()),
        })
    }
}

impl Drop for LifeToken {
    fn drop(&mut self) {
        self.handoff.close();
        self.control.ended.store(true, Ordering::Release);
        self.control.wake();
    }
}

pub(super) enum Mode {
    Clipboard,
    Drag,
}

pub(super) enum Started {
    Clipboard(u32),
    Drag(Marshaled),
}

/// The drag's data object, marshaled for the GUI thread.
pub(super) struct Marshaled(IStream);

// SAFETY: an HGLOBAL stream is free-threaded (the documented hand-over
// medium of CoMarshalInterThreadInterfaceInStream); after the send only the
// receiving thread uses it.
unsafe impl Send for Marshaled {}

impl Marshaled {
    pub(super) fn into_stream(self) -> IStream {
        self.0
    }
}

/// Starts the worker for `handoff` and waits (COM-safely: sent messages such
/// as the old clipboard owner's WM_DESTROYCLIPBOARD keep flowing) until the
/// object is on the clipboard or marshaled.
pub(super) fn start(handoff: Arc<Handoff>, mode: Mode) -> Result<(Started, Arc<Control>)> {
    let control = Arc::new(Control::default());
    let ready = Arc::new(Signal::new(true)?);
    let (reply, answer) = mpsc::sync_channel(1);
    let (worker_control, worker_ready) = (control.clone(), ready.clone());
    std::thread::Builder::new()
        .name("remote-virtual-files".into())
        .spawn(move || run(handoff, &worker_control, mode, &reply, &worker_ready))
        .map_err(|error| {
            Error::new(
                E_OUTOFMEMORY,
                format!("Zwischenablage-Thread konnte nicht starten: {error}"),
            )
        })?;
    let started = await_reply(&answer, &ready)?;
    Ok((started, control))
}

fn await_reply(answer: &Receiver<Result<Started>>, ready: &Signal) -> Result<Started> {
    loop {
        match answer.try_recv() {
            Ok(started) => return started,
            Err(TryRecvError::Empty) => {
                ready.wait(WAIT_SLICE);
            }
            Err(TryRecvError::Disconnected) => {
                return Err(Error::new(
                    E_UNEXPECTED,
                    "Der Zwischenablage-Thread wurde unerwartet beendet",
                ))
            }
        }
    }
}

fn run(
    handoff: Arc<Handoff>,
    control: &Arc<Control>,
    mode: Mode,
    reply: &SyncSender<Result<Started>>,
    ready: &Signal,
) {
    let answer = |result: Result<Started>| {
        let _ = reply.send(result);
        ready.notify();
    };
    let mut message = MSG::default();
    // Create this thread's message queue before its id is published.
    let _ = unsafe { PeekMessageW(&mut message, HWND::default(), WM_USER, WM_USER, PM_NOREMOVE) };
    control
        .thread
        .store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    if let Err(error) = unsafe { OleInitialize(None) } {
        handoff.close();
        answer(Err(error));
        control.exited.store(true, Ordering::Release);
        return;
    }
    let life = Arc::new(LifeToken {
        handoff: handoff.clone(),
        control: control.clone(),
    });
    let object: IDataObject = RemoteDataObject::new(handoff, life).into();
    // This thread keeps no reference of its own on the clipboard (OLE holds
    // one while the object is current); a drag keeps one until the GUI
    // thread has its proxy. Every path moves `object`, so nothing here
    // outlives Explorer's use of it.
    let held = match mode {
        Mode::Clipboard => {
            match unsafe { OleSetClipboard(&object) } {
                Ok(()) => answer(Ok(Started::Clipboard(unsafe {
                    GetClipboardSequenceNumber()
                }))),
                Err(error) => answer(Err(error)),
            }
            drop(object);
            None
        }
        Mode::Drag => match marshal(&object) {
            Ok(stream) => {
                answer(Ok(Started::Drag(Marshaled(stream))));
                Some(object)
            }
            Err(error) => {
                answer(Err(error));
                drop(object);
                None
            }
        },
    };
    pump(control, held);
    unsafe { OleUninitialize() };
    control.exited.store(true, Ordering::Release);
}

/// A cross-process marshaling context puts the stub in this apartment, so
/// every call through the GUI thread's proxy runs here, never on the GUI
/// thread; in-process marshaling would hand the agile object itself over.
fn marshal(object: &IDataObject) -> Result<IStream> {
    unsafe {
        let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
        CoMarshalInterface(
            &stream,
            &IDataObject::IID,
            object,
            MSHCTX_LOCAL.0 as u32,
            None,
            MSHLFLAGS_NORMAL.0 as u32,
        )?;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        Ok(stream)
    }
}

fn pump(control: &Control, mut held: Option<IDataObject>) {
    let timer = unsafe { SetTimer(HWND::default(), 0, TICK_MS, None) };
    let mut message = MSG::default();
    loop {
        let release = control.release.load(Ordering::Acquire);
        if release != RELEASE_PENDING {
            if let (RELEASE_FAILED, Some(object)) = (release, held.as_ref()) {
                // The marshaled reference was never unmarshaled: drop it.
                let _ = unsafe { CoDisconnectObject(object, 0) };
            }
            held = None;
        }
        if held.is_none() && control.ended.load(Ordering::Acquire) {
            break;
        }
        let result = unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) };
        if result.0 == 0 || result.0 == -1 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    if timer != 0 {
        let _ = unsafe { KillTimer(HWND::default(), timer) };
    }
}
