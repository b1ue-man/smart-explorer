//! Win32 0.59 bindings. Owner chains plus process creation/session facts define
//! the family. A unique window property pins a lifetime across HWND recycling.
use crate::remote_window::core::host::WindowAdapter;
use crate::remote_window::core::model::*;
use std::collections::{HashMap, HashSet};
use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HANDLE, HWND, LPARAM, RECT};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

mod facts;
use facts::{ProcessFacts, WindowFacts};

pub(crate) struct WindowsAdapter {
    property: Vec<u16>,
    markers: HashMap<NativeWindowId, WindowBinding>,
    session: u32,
}

fn gone() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "original_window_gone")
}
fn control_ready() -> io::Result<()> {
    if let Some(block) = crate::screen::os::window_control_block() {
        let reason = match block {
            crate::screen::core::wire_control::ControlBlock::SecureDesktop => "secure_desktop",
            _ => "control_unavailable",
        };
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, reason));
    }
    Ok(())
}
fn hwnd(id: NativeWindowId) -> io::Result<HWND> {
    let value = usize::try_from(id.0).map_err(|_| gone())?;
    if value == 0 {
        return Err(gone());
    }
    Ok(value as HWND)
}
fn marker(binding: &WindowBinding) -> usize {
    let mut bytes = [0u8; std::mem::size_of::<usize>()];
    if let Some(value) = binding.birth.get(..bytes.len()) {
        bytes.copy_from_slice(value);
    }
    usize::from_ne_bytes(bytes)
}

impl WindowsAdapter {
    pub(crate) fn adopt(pin: &WindowPin) -> io::Result<Self> {
        if pin.platform != PinPlatform::Windows
            || !pin.namespace.starts_with("SmartExplorer.RemoteWindow.")
            || pin.namespace.len() > 128
            || !pin.namespace.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid_windows_pin_namespace",
            ));
        }
        let mut adapter = Self::new()?;
        adapter.property = pin.namespace.encode_utf16().chain(Some(0)).collect();
        adapter.validate(&pin.binding)?;
        adapter
            .markers
            .insert(pin.binding.native, pin.binding.clone());
        Ok(adapter)
    }
    pub(crate) fn new() -> io::Result<Self> {
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let property = format!("SmartExplorer.RemoteWindow.{suffix}\0")
            .encode_utf16()
            .collect();
        let mut session = 0;
        // SAFETY: local output and current process ID.
        if unsafe {
            ProcessIdToSessionId(
                windows_sys::Win32::System::Threading::GetCurrentProcessId(),
                &mut session,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            property,
            markers: HashMap::new(),
            session,
        })
    }

    fn process(&self, id: NativeWindowId) -> io::Result<ProcessFacts> {
        let process = facts::process(hwnd(id)?)?;
        if process.session != self.session {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "window_session_mismatch",
            ));
        }
        Ok(process)
    }
}

impl WindowAdapter for WindowsAdapter {
    fn candidate_windows(&mut self) -> io::Result<Vec<(NativeWindowId, String)>> {
        Ok(facts::enumerate()?
            .into_iter()
            .filter_map(|handle| {
                let facts = WindowFacts::read(handle).ok()?;
                if facts.owner.is_some() || !facts.visible || !facts.geometry.valid() {
                    return None;
                }
                let native = NativeWindowId(handle as usize as u64);
                Some((native, self.process(native).ok()?.key()))
            })
            .collect())
    }
    fn process_user_matches(&mut self, binding: &WindowBinding, user_id: &str) -> io::Result<bool> {
        self.validate(binding)?;
        let matches = facts::user_matches(hwnd(binding.native)?, user_id)?;
        self.validate(binding)?;
        Ok(matches)
    }
    fn inspect(&mut self, binding: &WindowBinding) -> io::Result<ObservedWindow> {
        self.validate(binding)?;
        let process = self.process(binding.native)?;
        let facts = WindowFacts::read(hwnd(binding.native)?)?;
        let available = control_ready().is_ok();
        let window = ObservedWindow {
            binding: binding.clone(),
            owner: None,
            process: process.key(),
            app_id: crate::screen::os::app_icons::window_identity(binding.native.0)?,
            app: process.app(),
            title: facts.title,
            kind: WindowKind::Main,
            geometry: facts.geometry,
            z_order: 0,
            focused: available && facts.focused,
            visible: facts.visible,
            minimized: facts.minimized,
            operations: if available {
                facts.operations
            } else {
                WindowOperations::default()
            },
        };
        self.validate(binding)?;
        Ok(window)
    }
    fn export_pin(&mut self, binding: &WindowBinding) -> io::Result<WindowPin> {
        self.validate(binding)?;
        let namespace = String::from_utf16(&self.property[..self.property.len().saturating_sub(1)])
            .map_err(|_| io::Error::other("invalid_windows_pin_namespace"))?;
        Ok(WindowPin {
            platform: PinPlatform::Windows,
            namespace,
            binding: binding.clone(),
        })
    }
    fn transfer_pin(&mut self, binding: &WindowBinding) {
        if self.markers.get(&binding.native) == Some(binding) {
            self.markers.remove(&binding.native);
        }
    }
    fn bind(&mut self, native: NativeWindowId) -> io::Result<WindowBinding> {
        if let Some(binding) = self.markers.get(&native).cloned() {
            if self.validate(&binding).is_ok() {
                return Ok(binding);
            }
        }
        let handle = hwnd(native)?;
        let process = self.process(native)?;
        let mut birth = vec![0u8; std::mem::size_of::<usize>()];
        getrandom::getrandom(&mut birth).map_err(|error| io::Error::other(error.to_string()))?;
        if birth.iter().all(|b| *b == 0) {
            birth[0] = 1;
        }
        birth.extend_from_slice(&process.signature());
        let binding = WindowBinding { native, birth };
        // SAFETY: owned NUL-terminated name; marker is an opaque value, no pointer dereference.
        if unsafe { SetPropW(handle, self.property.as_ptr(), marker(&binding) as HANDLE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // Verify after tagging: a concurrent destroy/recreate cannot continue the old binding.
        self.markers.insert(native, binding.clone());
        self.validate(&binding)?;
        Ok(binding)
    }

    fn validate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        let handle = hwnd(binding.native)?;
        let process = self.process(binding.native)?;
        let offset = std::mem::size_of::<usize>();
        // SAFETY: queries only; property belongs to this adapter's unique namespace.
        if unsafe { IsWindow(handle) } == 0
            || unsafe { GetPropW(handle, self.property.as_ptr()) } as usize != marker(binding)
            || binding.birth.get(offset..) != Some(process.signature().as_slice())
        {
            return Err(gone());
        }
        Ok(())
    }

    fn observe(&mut self, root: &WindowBinding) -> io::Result<Observation> {
        self.validate(root)?;
        let root_process = self.process(root.native)?;
        let app_id = crate::screen::os::app_icons::window_identity(root.native.0)?;
        let handles = facts::enumerate()?;
        let mut candidates = Vec::new();
        let mut limitations = vec!["session_wide_input_no_app_isolation".into()];
        let control_available = control_ready().is_ok();
        if !control_available {
            limitations.push("window_control_blocked_by_session_or_desktop".into());
        }
        for (index, handle) in handles.iter().copied().enumerate() {
            let native = NativeWindowId(handle as usize as u64);
            let mut pid = 0;
            // SAFETY: reject unrelated processes before opening/querying one.
            unsafe {
                GetWindowThreadProcessId(handle, &mut pid);
            }
            if pid != root_process.pid {
                continue;
            }
            let Ok(process) = self.process(native) else {
                continue;
            };
            if process.pid != root_process.pid || process.created != root_process.created {
                continue;
            }
            let Ok(window) = WindowFacts::read(handle) else {
                continue;
            };
            if native != root.native && !window.visible {
                continue;
            }
            candidates.push((native, process, window, (handles.len() - index) as u32));
        }
        let mut family = HashSet::from([root.native]);
        loop {
            let before = family.len();
            for (id, _, window, _) in &candidates {
                if window.owner.is_some_and(|owner| family.contains(&owner)) {
                    family.insert(*id);
                }
            }
            if family.len() == before {
                break;
            }
        }
        // System menu windows may omit GW_OWNER. GetGUIThreadInfo supplies the
        // real menu owner, so thread/PID coincidence alone never admits a menu.
        for (id, process, window, _) in &mut candidates {
            if !family.contains(id) && window.class == "#32768" {
                if let Some(owner) = facts::menu_owner(process.thread) {
                    if family.contains(&owner) {
                        window.owner = Some(owner);
                        family.insert(*id);
                    }
                }
            }
        }
        let mut windows = Vec::new();
        for (native, process, window, z_order) in candidates {
            if !family.contains(&native) {
                if matches!(
                    window.kind,
                    WindowKind::Tooltip | WindowKind::Menu | WindowKind::Dialog
                ) || window.popup
                {
                    limitations.push(format!(
                        "unassociated_surface:{}:requires_local_dialog_or_explicit_desktop",
                        native.0
                    ));
                }
                continue;
            }
            let binding = if native == root.native {
                root.clone()
            } else {
                match self.bind(native) {
                    Ok(binding) => binding,
                    Err(_) => {
                        limitations
                            .push(format!("family_member_identity_unavailable:{}", native.0));
                        continue;
                    }
                }
            };
            let mut operations = window.operations;
            // Non-activating menus/tooltips keep the owner's active HWND. They
            // remain capture surfaces and pointer targets; input is gated on owner.
            if matches!(window.kind, WindowKind::Menu | WindowKind::Tooltip) {
                operations.focus = false;
                operations.input = window.kind == WindowKind::Menu;
            }
            if !control_available {
                operations = WindowOperations::default();
            }
            windows.push(ObservedWindow {
                binding,
                owner: window.owner,
                process: root_process.key(),
                app_id: app_id.clone(),
                app: process.app(),
                title: window.title,
                kind: if native == root.native {
                    WindowKind::Main
                } else {
                    window.kind
                },
                geometry: window.geometry,
                z_order,
                focused: control_available && window.focused,
                visible: window.visible,
                minimized: window.minimized,
                operations,
            });
        }
        limitations.sort();
        limitations.dedup();
        self.validate(root)?;
        Ok(Observation {
            windows,
            limitations,
        })
    }

    fn activate(&mut self, binding: &WindowBinding) -> io::Result<()> {
        self.activate_if(binding, &|| true)
    }
    fn activate_if(
        &mut self,
        binding: &WindowBinding,
        admitted: &dyn Fn() -> bool,
    ) -> io::Result<()> {
        control_ready()?;
        self.validate(binding)?;
        let handle = hwnd(binding.native)?;
        let check = || {
            if admitted() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "app_activation_admission_ended",
                ))
            }
        };
        // No AttachThreadInput, synthetic Alt press or foreground-lock bypass.
        unsafe {
            if IsIconic(handle) != 0 {
                check()?;
                ShowWindowAsync(handle, SW_RESTORE);
            }
            if GetForegroundWindow() != handle {
                check()?;
                if SetForegroundWindow(handle) == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "window_activation_denied",
                    ));
                }
            }
        }
        self.validate(binding)
    }

    fn focused(&mut self, binding: &WindowBinding) -> io::Result<bool> {
        control_ready()?;
        self.validate(binding)?;
        let handle = hwnd(binding.native)?;
        let facts = WindowFacts::read(handle)?;
        if facts.kind == WindowKind::Menu {
            let process = self.process(binding.native)?;
            let owner = facts.owner.or_else(|| facts::menu_owner(process.thread));
            let Some(owner) = owner else {
                return Ok(false);
            };
            let Some(owner_binding) = self.markers.get(&owner).cloned() else {
                return Ok(false);
            };
            self.validate(&owner_binding)?;
            // SAFETY: the independently validated menu owner must own foreground.
            return Ok(unsafe { GetForegroundWindow() == hwnd(owner)? });
        }
        // SAFETY: plain foreground query; IsWindowEnabled blocks disabled modal owners.
        Ok(unsafe {
            GetForegroundWindow() == handle && IsWindowEnabled(handle) != 0 && IsIconic(handle) == 0
        })
    }

    fn request(&mut self, binding: &WindowBinding, operation: WindowOperation) -> io::Result<()> {
        control_ready()?;
        self.validate(binding)?;
        let handle = hwnd(binding.native)?;
        // SAFETY: validated handle and bounded dimensions. Requests remain asynchronous.
        let success = unsafe {
            match operation {
                WindowOperation::Close => PostMessageW(handle, WM_CLOSE, 0, 0),
                WindowOperation::Resize { width, height } => SetWindowPos(
                    handle,
                    std::ptr::null_mut(),
                    0,
                    0,
                    i32::try_from(width).map_err(|_| gone())?,
                    i32::try_from(height).map_err(|_| gone())?,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                ),
                WindowOperation::Focus => return self.activate(binding),
            }
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn disconnect(&mut self) {
        for binding in self.markers.values() {
            if let Ok(handle) = hwnd(binding.native) {
                // SAFETY: remove only our matching marker; never act on a reused HWND.
                unsafe {
                    if GetPropW(handle, self.property.as_ptr()) as usize == marker(binding) {
                        RemovePropW(handle, self.property.as_ptr());
                    }
                }
            }
        }
        self.markers.clear();
    }
}

impl Drop for WindowsAdapter {
    fn drop(&mut self) {
        self.disconnect();
    }
}
