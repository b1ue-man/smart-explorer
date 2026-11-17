//! The child window of the Direct3D 11 picture (plan 2026-11-17 P8): a
//! `WS_CHILD` window of the viewer window over the picture area, made and
//! moved on the window's thread. It answers `WM_NCHITTEST` with
//! `HTTRANSPARENT`, so every mouse message goes to the viewer window
//! beneath it (same thread) and egui's input mapping, focus and pointer
//! stay exactly as without it; it never erases or paints itself (the swap
//! chain owns its pixels). Overlays egui draws over the picture are cut out
//! of its region (`SetWindowRgn`), so egui's drawing shows there.
use std::io;
use std::sync::Arc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, ValidateRect, RGN_DIFF,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SetWindowPos, HTTRANSPARENT,
    HWND_TOP, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
    SWP_NOZORDER, SWP_SHOWWINDOW, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT, WNDCLASSEXW, WS_CHILD,
    WS_CLIPSIBLINGS, WS_EX_NOPARENTNOTIFY,
};

use super::super::layout::{DirectGeometry, PixelRect};
use super::super::DirectPresenter;
use super::Presenter;

/// `ERROR_CLASS_ALREADY_EXISTS`: another viewer window of this process
/// registered the class.
const CLASS_EXISTS: i32 = 1410;

fn class_name() -> Vec<u16> {
    "SmartExplorerScreenDirect3D11\0".encode_utf16().collect()
}

unsafe extern "system" fn child_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            // SAFETY: the window is live while it gets messages.
            unsafe { ValidateRect(hwnd, std::ptr::null()) };
            0
        }
        // SAFETY: the default handling of the window's own message.
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// A new, hidden child window of `parent`.
fn create(parent: usize) -> io::Result<usize> {
    let class = class_name();
    // SAFETY: a zeroed class description with the fields Windows reads set;
    // the class name lives through both calls.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut description: WNDCLASSEXW = std::mem::zeroed();
        description.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        description.lpfnWndProc = Some(child_proc);
        description.hInstance = instance;
        description.lpszClassName = class.as_ptr();
        if RegisterClassExW(&description) == 0
            && io::Error::last_os_error().raw_os_error() != Some(CLASS_EXISTS)
        {
            return Err(io::Error::last_os_error());
        }
        let child = CreateWindowExW(
            WS_EX_NOPARENTNOTIFY,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            parent as HWND,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if child.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(child as usize)
    }
}

/// The child's region: all of it but `cutouts` (none: no region).
fn set_region(child: usize, width: i32, height: i32, cutouts: &[PixelRect]) -> io::Result<()> {
    let hwnd = child as HWND;
    // SAFETY: GDI regions made here; after a successful `SetWindowRgn` the
    // system owns the region, every other one is deleted.
    unsafe {
        if cutouts.is_empty() {
            SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
            return Ok(());
        }
        let region = CreateRectRgn(0, 0, width, height);
        if region.is_null() {
            return Err(io::Error::other("CreateRectRgn failed"));
        }
        for cut in cutouts {
            let hole = CreateRectRgn(cut.left, cut.top, cut.right, cut.bottom);
            if !hole.is_null() {
                CombineRgn(region, region, hole, RGN_DIFF);
                DeleteObject(hole);
            }
        }
        if SetWindowRgn(hwnd, region, 1) == 0 {
            DeleteObject(region);
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// The child window and the presenter that draws into it.
pub struct Surface {
    presenter: Arc<Presenter>,
    parent: usize,
    /// The child's `HWND` (0: none yet).
    child: usize,
    placed: Option<PixelRect>,
    shown: bool,
    region: Option<(PixelRect, Vec<PixelRect>)>,
}

impl Surface {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            presenter: Presenter::new()?,
            parent: 0,
            child: 0,
            placed: None,
            shown: false,
            region: None,
        })
    }

    pub fn presenter(&self) -> Arc<dyn DirectPresenter> {
        self.presenter.clone()
    }

    fn hide(&mut self) {
        if self.child != 0 && self.shown {
            let flags = SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
            // SAFETY: the live child of this thread.
            unsafe { SetWindowPos(self.child as HWND, std::ptr::null_mut(), 0, 0, 0, 0, flags) };
        }
        self.shown = false;
    }

    fn destroy(&mut self) {
        self.presenter.close();
        if self.child != 0 {
            // SAFETY: the child made by this thread, destroyed once.
            unsafe { DestroyWindow(self.child as HWND) };
        }
        self.child = 0;
        self.placed = None;
        self.shown = false;
        self.region = None;
    }

    /// On the window's thread, each frame: places the child under `parent`
    /// at `placement` (the geometry and whether the picture shows now) or
    /// hides it (`None`), and sizes the swap chain.
    pub fn update(
        &mut self,
        parent: usize,
        placement: Option<(&DirectGeometry, bool)>,
    ) -> io::Result<()> {
        if parent != self.parent && self.child != 0 {
            // A new viewer window: the old child and swap chain go.
            self.destroy();
            self.presenter = Presenter::new()?;
        }
        self.parent = parent;
        let Some((geometry, show)) = placement.filter(|(g, _)| !g.child.is_empty() && parent != 0)
        else {
            self.hide();
            return Ok(());
        };
        if self.child == 0 {
            self.child = create(parent)?;
        }
        let child = geometry.child;
        let wanted_region = (child, geometry.cutouts.clone());
        if self.region.as_ref() != Some(&wanted_region) {
            set_region(self.child, child.width(), child.height(), &geometry.cutouts)?;
            self.region = Some(wanted_region);
        }
        if self.placed != Some(child) || self.shown != show {
            let visibility = if show { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
            // SAFETY: the live child of this thread.
            let placed = unsafe {
                SetWindowPos(
                    self.child as HWND,
                    HWND_TOP,
                    child.left,
                    child.top,
                    child.width(),
                    child.height(),
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER | visibility,
                )
            };
            if placed == 0 {
                return Err(io::Error::last_os_error());
            }
            self.placed = Some(child);
            self.shown = show;
        }
        let hwnd = windows::Win32::Foundation::HWND(self.child as *mut core::ffi::c_void);
        self.presenter.configure(
            hwnd,
            child.width().max(1) as u32,
            child.height().max(1) as u32,
            geometry.dest,
        )
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        self.destroy();
    }
}
