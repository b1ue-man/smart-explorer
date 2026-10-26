use super::*;

pub(super) fn user_matches(window: HWND, expected: &str) -> io::Result<bool> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenStatistics, TokenUser,
            TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn stats(token: HANDLE) -> io::Result<TOKEN_STATISTICS> {
        let mut value = std::mem::MaybeUninit::<TOKEN_STATISTICS>::uninit();
        let mut size = 0;
        if unsafe {
            GetTokenInformation(
                token,
                TokenStatistics,
                value.as_mut_ptr().cast(),
                std::mem::size_of::<TOKEN_STATISTICS>() as u32,
                &mut size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { value.assume_init() })
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut raw = std::ptr::null_mut();
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) };
    unsafe {
        CloseHandle(process);
    }
    if opened == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Token(raw);
    let mut own = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut own) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let own = Token(own);
    let (actual, current) = (
        stats(token.0)?.AuthenticationId,
        stats(own.0)?.AuthenticationId,
    );
    if actual.HighPart != current.HighPart || actual.LowPart != current.LowPart {
        return Ok(false);
    }
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut size);
    }
    if size == 0 || size > 65536 {
        return Err(io::Error::last_os_error());
    }
    let mut bytes = vec![0u64; (size as usize).div_ceil(8)];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            bytes.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*bytes.as_ptr().cast::<TOKEN_USER>() };
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut length = 0;
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
    unsafe {
        LocalFree(text.cast());
    }
    Ok(expected == format!("windows-sid:{sid}"))
}

pub(super) struct ProcessFacts {
    pub(super) pid: u32,
    pub(super) thread: u32,
    pub(super) session: u32,
    pub(super) created: u64,
    pub(super) image: Option<String>,
}

impl ProcessFacts {
    pub(super) fn signature(&self) -> Vec<u8> {
        [
            self.pid.to_le_bytes().as_slice(),
            self.thread.to_le_bytes().as_slice(),
            self.session.to_le_bytes().as_slice(),
            self.created.to_le_bytes().as_slice(),
        ]
        .concat()
    }
    pub(super) fn key(&self) -> String {
        format!("{}:{}:{}", self.session, self.pid, self.created)
    }
    pub(super) fn app(&self) -> String {
        self.image
            .as_deref()
            .and_then(|p| p.rsplit(['\\', '/']).next())
            .unwrap_or("")
            .into()
    }
}

pub(super) fn process(handle: HWND) -> io::Result<ProcessFacts> {
    let mut pid = 0;
    // SAFETY: plain window/process queries into owned buffers.
    unsafe {
        let thread = GetWindowThreadProcessId(handle, &mut pid);
        if pid == 0 || thread == 0 {
            return Err(gone());
        }
        let mut session = 0;
        if ProcessIdToSessionId(pid, &mut session) == 0 {
            return Err(io::Error::last_os_error());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        if ok == 0 {
            let error = io::Error::last_os_error();
            CloseHandle(process);
            return Err(error);
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let image = (QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) != 0)
            .then(|| String::from_utf16_lossy(&path[..length as usize]));
        CloseHandle(process);
        Ok(ProcessFacts {
            pid,
            thread,
            session,
            created: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
            image,
        })
    }
}

unsafe extern "system" fn collect(handle: HWND, data: LPARAM) -> BOOL {
    // SAFETY: enumeration's vector lives until EnumWindows returns.
    unsafe {
        (&mut *(data as *mut Vec<HWND>)).push(handle);
    }
    1
}

pub(super) fn enumerate() -> io::Result<Vec<HWND>> {
    let mut handles = Vec::new();
    // SAFETY: synchronous callback has an exclusive vector pointer.
    if unsafe { EnumWindows(Some(collect), &mut handles as *mut Vec<HWND> as LPARAM) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(handles)
}

pub(super) struct WindowFacts {
    pub(super) owner: Option<NativeWindowId>,
    pub(super) title: String,
    pub(super) class: String,
    pub(super) kind: WindowKind,
    pub(super) geometry: CaptureGeometry,
    pub(super) visible: bool,
    pub(super) popup: bool,
    pub(super) minimized: bool,
    pub(super) focused: bool,
    pub(super) operations: WindowOperations,
}

impl WindowFacts {
    pub(super) fn read(handle: HWND) -> io::Result<Self> {
        // SAFETY: queries only into owned memory; disappearing windows fail closed.
        unsafe {
            // WGC's window surface excludes invisible resize borders. DWM's
            // visible frame bounds keep capture placement in the same pixels.
            let mut visible = windows::Win32::Foundation::RECT::default();
            if windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                windows::Win32::Foundation::HWND(handle),
                windows::Win32::Graphics::Dwm::DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut visible as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of_val(&visible) as u32,
            )
            .is_err()
            {
                return Err(gone());
            }
            // Never substitute DPI-virtualized GetWindowRect coordinates for
            // this family's physical capture-surface identity.
            let rect = visible;
            let geometry = CaptureGeometry {
                x: rect.left,
                y: rect.top,
                width: u32::try_from(i64::from(rect.right) - i64::from(rect.left))
                    .map_err(|_| gone())?,
                height: u32::try_from(i64::from(rect.bottom) - i64::from(rect.top))
                    .map_err(|_| gone())?,
            };
            if !geometry.valid() {
                return Err(gone());
            }
            let mut title = vec![0u16; GetWindowTextLengthW(handle).max(0) as usize + 1];
            let length =
                GetWindowTextW(handle, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
            let title = String::from_utf16_lossy(&title[..length]);
            let mut class = [0u16; 256];
            let length =
                GetClassNameW(handle, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
            let class = String::from_utf16_lossy(&class[..length]);
            let style = GetWindowLongW(handle, GWL_STYLE) as u32;
            let extended = GetWindowLongW(handle, GWL_EXSTYLE) as u32;
            let owner = GetWindow(handle, GW_OWNER);
            let owner = (!owner.is_null()).then_some(NativeWindowId(owner as usize as u64));
            let kind = match class.as_str() {
                "#32768" => WindowKind::Menu,
                "#32770" => WindowKind::Dialog,
                "tooltips_class32" => WindowKind::Tooltip,
                _ => WindowKind::Popup,
            };
            let enabled = IsWindowEnabled(handle) != 0;
            let activating = extended & WS_EX_NOACTIVATE == 0;
            let menu = GetSystemMenu(handle, 0);
            let close_enabled = !menu.is_null()
                && GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) & (MF_DISABLED | MF_GRAYED) == 0;
            Ok(Self {
                owner,
                title,
                class,
                kind,
                geometry,
                visible: IsWindowVisible(handle) != 0,
                popup: style & WS_POPUP != 0,
                minimized: IsIconic(handle) != 0,
                focused: GetForegroundWindow() == handle,
                operations: WindowOperations {
                    resize: style & WS_THICKFRAME != 0 && enabled,
                    focus: enabled && activating,
                    close: enabled && close_enabled,
                    input: enabled && activating,
                },
            })
        }
    }
}

pub(super) fn menu_owner(thread: u32) -> Option<NativeWindowId> {
    // SAFETY: correctly sized output; no cross-thread state modification.
    unsafe {
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(thread, &mut info) != 0
            && info.flags & GUI_INMENUMODE != 0
            && !info.hwndMenuOwner.is_null()
        {
            Some(NativeWindowId(info.hwndMenuOwner as usize as u64))
        } else {
            None
        }
    }
}
