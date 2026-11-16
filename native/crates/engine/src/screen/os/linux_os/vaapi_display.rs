//! One VA display on one DRM render node (BK4, B-24): opened as the
//! account of the process (the host process runs as the session account;
//! the render node's group `render` or the seat's ACL give that account
//! access, no administrator right), with libva's messages silenced, and
//! terminated when dropped. Functions and their behaviour: libva 2.20.0
//! `va.h` (`vaInitialize`, `vaTerminate`, `vaSetErrorCallback`,
//! `vaSetInfoCallback`, `vaErrorStr`) and `va_drm.h` (`vaGetDisplayDRM`).
use std::ffi::{c_char, c_void, CStr};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::vaapi_ffi::{VaApi, VaHandle, VaId, VaStatus, VA_STATUS_SUCCESS};

/// The DRM render nodes of this system (`/dev/dri/renderD*`) in number order.
pub(super) fn render_nodes(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<(u32, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let number = name.strip_prefix("renderD")?.parse().ok()?;
            Some((number, entry.path()))
        })
        .collect();
    nodes.sort();
    nodes.into_iter().map(|(_, path)| path).collect()
}

/// libva's messages stay out of the host process's error output: the
/// codec list (`se screen codec`) is the only place that reports devices.
unsafe extern "C" fn silent(_context: *mut c_void, _message: *const c_char) {}

/// One initialized VA display on one render node.
pub(super) struct VaDisplay {
    pub api: Arc<VaApi>,
    pub handle: VaHandle,
    pub node: PathBuf,
    // Dropped after `Drop::drop` terminated the display.
    _file: File,
}

// SAFETY: a `VaDisplay` is owned by exactly one encoder, decoder or probe
// and used from one thread at a time (`&mut self` or by value); va.h names
// no thread affinity for a display.
unsafe impl Send for VaDisplay {}

impl VaDisplay {
    pub(super) fn open(api: Arc<VaApi>, node: &Path) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(node)?;
        // SAFETY: the descriptor is open and outlives the display (`_file`).
        let handle = unsafe { (api.get_display_drm)(file.as_raw_fd()) };
        if handle.is_null() {
            return Err(io::Error::other(format!(
                "{}: vaGetDisplayDRM gave no display",
                node.display()
            )));
        }
        // SAFETY: valid display; `silent` ignores its arguments.
        unsafe {
            (api.set_error_callback)(handle, Some(silent), std::ptr::null_mut());
            (api.set_info_callback)(handle, Some(silent), std::ptr::null_mut());
        }
        let (mut major, mut minor) = (0, 0);
        // SAFETY: valid display and writable version slots.
        let status = unsafe { (api.initialize)(handle, &mut major, &mut minor) };
        if status != VA_STATUS_SUCCESS {
            let message = error_text(&api, status);
            // SAFETY: a display that failed to initialize is still released.
            unsafe { (api.terminate)(handle) };
            return Err(io::Error::other(format!(
                "{}: vaInitialize: {message}",
                node.display()
            )));
        }
        Ok(Self {
            api,
            handle,
            node: node.to_path_buf(),
            _file: file,
        })
    }

    /// Waits for `surface` with a bounded 2 s timeout where `vaSyncSurface2`
    /// is implemented (`VA_STATUS_ERROR_UNIMPLEMENTED` = 0x14 falls back).
    pub(super) fn sync(&self, surface: VaId, what: &str) -> io::Result<()> {
        const UNIMPLEMENTED: VaStatus = 0x14;
        const TIMEOUT_NS: u64 = 2_000_000_000;
        let status = match self.api.sync_surface2 {
            Some(sync2) => {
                // SAFETY: the surface belongs to this display.
                let status = unsafe { sync2(self.handle, surface, TIMEOUT_NS) };
                if status == UNIMPLEMENTED {
                    unsafe { (self.api.sync_surface)(self.handle, surface) }
                } else {
                    status
                }
            }
            None => unsafe { (self.api.sync_surface)(self.handle, surface) },
        };
        self.check(status, what)
    }

    /// `Err` with the node and libva's text unless `status` is success.
    pub(super) fn check(&self, status: VaStatus, what: &str) -> io::Result<()> {
        if status == VA_STATUS_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "{}: {what}: {}",
                self.node.display(),
                error_text(&self.api, status)
            )))
        }
    }
}

impl Drop for VaDisplay {
    fn drop(&mut self) {
        // SAFETY: the display was initialized; every config, context,
        // surface and buffer owner borrowed it and is gone.
        unsafe { (self.api.terminate)(self.handle) };
    }
}

fn error_text(api: &VaApi, status: VaStatus) -> String {
    // SAFETY: `vaErrorStr` returns a static string for every status.
    let text = unsafe { (api.error_str)(status) };
    if text.is_null() {
        return format!("VA status {status}");
    }
    // SAFETY: static NUL-terminated string of libva.
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    format!("{text} ({status})")
}
