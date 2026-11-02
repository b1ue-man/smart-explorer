//! A persistent current-thread owner window and RAII clipboard sessions.
use std::{cell::RefCell, marker::PhantomData, rc::Rc};
use windows::{core::{w, Result}, Win32::{
    Foundation::{HINSTANCE, HWND},
    System::DataExchange::{CloseClipboard, OpenClipboard},
    UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HMENU, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE},
}};

struct OwnerWindow(HWND);

impl Drop for OwnerWindow {
    fn drop(&mut self) {
        // TLS destruction occurs on the creating thread. All data is rendered
        // eagerly; no WM_RENDERFORMAT handler or borrowed foreground HWND exists.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

thread_local! {
    static OWNER: RefCell<Option<OwnerWindow>> = const { RefCell::new(None) };
}

fn owner_window() -> Result<HWND> {
    OWNER.try_with(|slot| {
        let mut slot = slot.try_borrow_mut()
            .map_err(|_| super::invalid("Clipboard owner initialization was reentered"))?;
        if let Some(window) = slot.as_ref() { return Ok(window.0); }
        // STATIC is a predefined system class; HWND_MESSAGE creates no visible
        // window and neither reads nor changes the foreground application.
        let window = unsafe {
            CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!("SmartExplorer clipboard"),
                WINDOW_STYLE(0), 0, 0, 0, 0, HWND_MESSAGE, HMENU::default(), HINSTANCE::default(), None)?
        };
        *slot = Some(OwnerWindow(window));
        Ok(window)
    }).map_err(|_| super::invalid("Clipboard owner thread is shutting down"))?
}

pub(super) struct Clipboard {
    open: bool,
    // OpenClipboard/CloseClipboard and the owner HWND must stay on this thread.
    _thread: PhantomData<Rc<()>>,
}

impl Clipboard {
    pub(super) fn open() -> Result<Self> {
        unsafe { OpenClipboard(owner_window()?)?; }
        Ok(Self { open: true, _thread: PhantomData })
    }

    pub(super) fn close(mut self) -> Result<()> {
        unsafe { CloseClipboard()?; }
        self.open = false;
        Ok(())
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        if self.open { let _ = unsafe { CloseClipboard() }; }
    }
}
