//! Native Z order and positively established opaque rectangles. DWM glass,
//! per-pixel alpha, color keys and absent/failed shape/opacity proofs remain
//! conservatively visible. Ordinary DWM clients have no public opacity proof.
use eframe::egui::{self, Pos2, Rect};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{ClientToScreen, CreateRectRgn, DeleteObject, GetRgnBox, GetWindowRgn, SIMPLEREGION};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_PAUSE};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClientRect, GetForegroundWindow, GetWindowRect, GetLayeredWindowAttributes,
    LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA,
    GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE, WS_EX_LAYERED,
    WS_EX_TRANSPARENT, WS_EX_NOREDIRECTIONBITMAP, WS_EX_COMPOSITED,
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP};
use super::super::screen_visibility::WindowVisibility;

static TARGET: AtomicIsize = AtomicIsize::new(0);
static EXIT: AtomicBool = AtomicBool::new(false);
static RESERVED: AtomicBool = AtomicBool::new(false);
struct HookGuard(HHOOK);
impl Drop for HookGuard {
    fn drop(&mut self) { unsafe { let _ = UnhookWindowsHookEx(self.0); } }
}
thread_local! { static HOOK: RefCell<Option<HookGuard>> = const { RefCell::new(None) }; }
unsafe extern "system" fn keyboard(code: i32, event: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: WH_KEYBOARD_LL supplies this structure for the callback duration.
        let key = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
        if key.vkCode == u32::from(VK_PAUSE.0) {
            let message = event.0 as u32;
            let up = message == WM_KEYUP || message == WM_SYSKEYUP;
            if up && RESERVED.swap(false,Ordering::Relaxed) { return LRESULT(1); }
            let foreground = unsafe { GetForegroundWindow() };
            let mut owner = 0u32;
            unsafe { GetWindowThreadProcessId(foreground,Some(&mut owner)); }
            let active = foreground.0 as isize == TARGET.load(Ordering::Relaxed) && owner == std::process::id();
            let modifiers = unsafe { GetAsyncKeyState(i32::from(VK_CONTROL.0)) < 0 && GetAsyncKeyState(i32::from(VK_MENU.0)) < 0 };
            if active && modifiers && (message == WM_KEYDOWN || message == WM_SYSKEYDOWN) {
                RESERVED.store(true,Ordering::Relaxed);
                EXIT.store(true,Ordering::Relaxed);
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(HHOOK::default(),code,event,data) }
}
fn install_keyboard(hwnd: HWND) {
    TARGET.store(hwnd.0 as isize,Ordering::Relaxed);
    HOOK.with(|slot| {
        if slot.borrow().is_some() { return; }
        // SAFETY: this executable's callback remains alive until the guard unhooks it.
        let module = unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) };
        if module.is_null() { return; }
        if let Ok(hook) = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL,Some(keyboard),HINSTANCE(module),0) } {
            *slot.borrow_mut() = Some(HookGuard(hook));
        }
    });
}

unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: caller keeps the Vec alive throughout EnumWindows.
    unsafe { &mut *(data.0 as *mut Vec<HWND>) }.push(hwnd);
    BOOL(1)
}
fn order() -> Option<Vec<HWND>> {
    let mut windows = Vec::new();
    // SAFETY: callback receives a pointer to the live local vector.
    unsafe { EnumWindows(Some(collect),LPARAM(&mut windows as *mut Vec<HWND> as isize)).ok()?; }
    Some(windows)
}
fn client(hwnd: HWND) -> Option<Rect> {
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: valid output storage; failed/stale handles return no geometry.
    unsafe {
        GetClientRect(hwnd,&mut rect).ok()?;
        if !ClientToScreen(hwnd,&mut origin).as_bool() { return None; }
    }
    Some(Rect::from_min_size(Pos2::new(origin.x as f32,origin.y as f32),
        egui::vec2((rect.right as f32-rect.left as f32),(rect.bottom as f32-rect.top as f32))))
}
fn opaque_rect(hwnd: HWND) -> Option<Rect> {
    // SAFETY: plain queries. Only a successful whole-window opacity promise
    // (SetLayeredWindowAttributes alpha=255, no color key) removes pixels.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() { return None; }
        let style = GetWindowLongW(hwnd,GWL_EXSTYLE) as u32;
        if style & WS_EX_LAYERED.0 == 0 || style & (WS_EX_TRANSPARENT.0 | WS_EX_NOREDIRECTIONBITMAP.0 | WS_EX_COMPOSITED.0) != 0 { return None; }
        let mut alpha = 0u8;
        let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS::default();
        GetLayeredWindowAttributes(hwnd,None,Some(&mut alpha),Some(&mut flags)).ok()?;
        if alpha != 255 || flags != LWA_ALPHA { return None; }
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(hwnd,DWMWA_CLOAKED,&mut cloaked as *mut u32 as _,4).ok()?;
        if cloaked != 0 { return None; }
        let region = CreateRectRgn(0,0,0,0);
        if region.0.is_null() { return None; }
        let mut shape = RECT::default();
        let known = GetWindowRgn(hwnd,region) == SIMPLEREGION && GetRgnBox(region,&mut shape) == SIMPLEREGION;
        let _ = DeleteObject(region);
        // ERROR is ambiguous between an unset region and failure; never infer
        // a rectangular opaque cover from it.
        if !known { return None; }
        let mut window = RECT::default();
        GetWindowRect(hwnd,&mut window).ok()?;
        let shape = Rect::from_min_max(egui::pos2((window.left as f32+shape.left as f32),(window.top as f32+shape.top as f32)),
            egui::pos2((window.left as f32+shape.right as f32),(window.top as f32+shape.bottom as f32)));
        Some(shape.intersect(client(hwnd)?))
    }
}
pub(crate) fn sample(frame: &eframe::Frame, ctx: &egui::Context) -> WindowVisibility {
    let fallback = || WindowVisibility { minimized:ctx.input(|i| i.viewport().minimized == Some(true)), ..Default::default() };
    let Ok(handle) = frame.window_handle() else { return fallback(); };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else { return fallback(); };
    let hwnd = HWND(raw.hwnd.get() as *mut _);
    install_keyboard(hwnd);
    // SAFETY: read-only window/key-state queries; no input injection.
    let minimized = unsafe { IsIconic(hwnd).as_bool() };
    let mut shown = unsafe { IsWindowVisible(hwnd).as_bool() };
    let mut own_cloaked = 0u32;
    unsafe { if DwmGetWindowAttribute(hwnd,DWMWA_CLOAKED,&mut own_cloaked as *mut u32 as _,4).is_ok() && own_cloaked != 0 { shown = false; } }
    let exit_chord = EXIT.swap(false,Ordering::Relaxed) || unsafe { GetForegroundWindow() == hwnd &&
        [VK_CONTROL,VK_MENU,VK_PAUSE].iter().all(|key| GetAsyncKeyState(i32::from(key.0)) < 0) };
    let mut result = WindowVisibility { minimized, shown:Some(shown), rects:Vec::new(), exit_chord };
    if minimized || !shown { return result; }
    let Some(area) = client(hwnd) else { result.shown = None; return result; };
    let Some(windows) = order() else { result.shown = None; return result; };
    let Some(index) = windows.iter().position(|w| *w == hwnd) else { result.shown=None; return result; };
    let mut visible = vec![area];
    let mut covers = Vec::new();
    for &cover in &windows[..index] {
        if let Some(rect) = opaque_rect(cover) { super::subtract(&mut visible,rect); covers.push((cover,rect)); }
    }
    if order().as_ref() != Some(&windows) || client(hwnd) != Some(area)
        || covers.iter().any(|(cover,rect)| opaque_rect(*cover) != Some(*rect)) {
        result.shown = None;
    } else {
        result.rects = visible.into_iter().map(|rect| rect.translate(-area.min.to_vec2())).collect();
        result.shown = Some(!result.rects.is_empty());
    }
    result
}
