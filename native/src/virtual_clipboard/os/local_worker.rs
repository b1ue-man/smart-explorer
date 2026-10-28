//! The STA thread that owns a local virtual-files data object (M17). It puts
//! the object on the clipboard and pumps messages while OLE and Explorer
//! hold it, so Explorer's `GetData` and every `IStream::Read` run here and
//! never wait for the GUI thread's next frame. The same rules as the remote
//! worker apply (`remote/worker.rs`): the object stays alive as long as the
//! clipboard or a paste holds it, this thread keeps no reference of its own,
//! and it never calls `OleFlushClipboard`, which would render every file's
//! contents at once.
use crate::virtual_clipboard::remote::signal::{Signal, WAIT_SLICE};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_OUTOFMEMORY, E_UNEXPECTED, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::System::Ole::{OleInitialize, OleSetClipboard, OleUninitialize};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, PostThreadMessageW, SetTimer,
    TranslateMessage, MSG, PM_NOREMOVE, WM_APP, WM_USER,
};

#[cfg(test)]
#[path = "local_worker_tests.rs"]
mod tests;

/// Tick of the message loop: a modal loop may swallow the wake-up message,
/// so the thread notices its end within a second anyway.
const TICK_MS: u32 = 1_000;

/// Coordination between the worker thread, its data object and the caller.
#[derive(Default)]
struct Control {
    thread: AtomicU32,
    ended: AtomicBool,
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
}

/// Held by the data object: dropping it (OLE and every paste released the
/// object) ends the worker thread.
pub(super) struct LifeGuard(Arc<Control>);

impl Drop for LifeGuard {
    fn drop(&mut self) {
        self.0.ended.store(true, Ordering::Release);
        self.0.wake();
    }
}

/// The data object is on the clipboard, owned by its worker thread.
pub(super) struct Placed {
    /// Clipboard sequence number right after `OleSetClipboard`.
    pub(super) sequence: u32,
    /// Read by the tests; holding it costs nothing.
    #[cfg_attr(not(test), allow(dead_code))]
    control: Arc<Control>,
}

impl Placed {
    /// The worker thread has ended (tests wait for it).
    #[cfg(test)]
    pub(super) fn exited(&self) -> bool {
        self.control.exited.load(Ordering::Acquire)
    }

    /// The id of the thread that owns the object (tests compare it).
    #[cfg(test)]
    pub(super) fn thread(&self) -> u32 {
        self.control.thread.load(Ordering::Acquire)
    }
}

type MakeObject = Box<dyn FnOnce(LifeGuard) -> IDataObject + Send>;

/// Creates the data object with `make` on a new STA thread and puts it on
/// the clipboard there. The caller waits COM-safely, so sent messages such as
/// the old clipboard owner's WM_DESTROYCLIPBOARD keep flowing meanwhile.
pub(super) fn place(make: MakeObject) -> Result<Placed> {
    let control = Arc::new(Control::default());
    let ready = Arc::new(Signal::new(true)?);
    let (reply, answer) = mpsc::sync_channel(1);
    let (worker_control, worker_ready) = (control.clone(), ready.clone());
    std::thread::Builder::new()
        .name("local-virtual-files".into())
        .spawn(move || run(make, &worker_control, &reply, &worker_ready))
        .map_err(|error| {
            Error::new(
                E_OUTOFMEMORY,
                format!("Zwischenablage-Thread konnte nicht starten: {error}"),
            )
        })?;
    let sequence = await_reply(&answer, &ready)?;
    Ok(Placed { sequence, control })
}

fn await_reply(answer: &Receiver<Result<u32>>, ready: &Signal) -> Result<u32> {
    loop {
        match answer.try_recv() {
            Ok(sequence) => return sequence,
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

fn run(make: MakeObject, control: &Arc<Control>, reply: &SyncSender<Result<u32>>, ready: &Signal) {
    let answer = |result: Result<u32>| {
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
        answer(Err(error));
        control.exited.store(true, Ordering::Release);
        return;
    }
    let object = make(LifeGuard(control.clone()));
    // OLE holds its own reference while the object is current; dropping ours
    // leaves the object's lifetime to the clipboard and to running pastes.
    match unsafe { OleSetClipboard(&object) } {
        Ok(()) => answer(Ok(unsafe { GetClipboardSequenceNumber() })),
        Err(error) => answer(Err(error)),
    }
    drop(object);
    pump(control);
    unsafe { OleUninitialize() };
    control.exited.store(true, Ordering::Release);
}

/// Dispatches COM calls and clipboard messages until the object is gone.
fn pump(control: &Control) {
    let timer = unsafe { SetTimer(HWND::default(), 0, TICK_MS, None) };
    let mut message = MSG::default();
    while !control.ended.load(Ordering::Acquire) {
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
