//! Secure, delay-loaded access to the Dokany 2.3.1 user-mode runtime.

use std::{
    ffi::c_void,
    fmt,
    ptr::{null_mut, NonNull},
    sync::Arc,
};

use windows_sys::Win32::{
    Foundation::{
        FreeLibrary, GetLastError, ERROR_BAD_EXE_FORMAT, ERROR_MOD_NOT_FOUND, HMODULE, WAIT_FAILED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
};

use crate::mount::{validate_dokany_version_domains, DokanyVersionCompatibilityError};

use super::dokany_abi::{
    DokanFileInfo, DokanHandle, DokanOperations, DokanOptions, NtStatus, DOKANY_DLL_NAME,
    DOKANY_DRIVER_PROTOCOL_VERSION, DOKANY_LIBRARY_API_VERSION, DRIVER_INSTALL_ERROR,
    DRIVE_LETTER_ERROR, ERROR, MOUNT_ERROR, MOUNT_POINT_ERROR, START_ERROR, SUCCESS, VERSION_ERROR,
};

const DOKANY_DLL_WIDE: &[u16] = &[
    b'd' as u16,
    b'o' as u16,
    b'k' as u16,
    b'a' as u16,
    b'n' as u16,
    b'2' as u16,
    b'.' as u16,
    b'd' as u16,
    b'l' as u16,
    b'l' as u16,
    0,
];

type InitFn = unsafe extern "system" fn();
type ShutdownFn = unsafe extern "system" fn();
type VersionFn = unsafe extern "system" fn() -> u32;
type CreateFileSystemFn =
    unsafe extern "system" fn(*mut DokanOptions, *mut DokanOperations, *mut DokanHandle) -> i32;
type IsFileSystemRunningFn = unsafe extern "system" fn(DokanHandle) -> i32;
type WaitForFileSystemClosedFn = unsafe extern "system" fn(DokanHandle, u32) -> u32;
type CloseHandleFn = unsafe extern "system" fn(DokanHandle);
type ResetTimeoutFn = unsafe extern "system" fn(u32, *mut DokanFileInfo) -> i32;
type IsNameInExpressionFn = unsafe extern "system" fn(*const u16, *const u16, i32) -> i32;
type NtStatusFromWin32Fn = unsafe extern "system" fn(u32) -> NtStatus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DokanyRuntimeInfo {
    pub(crate) required_library_api: u32,
    pub(crate) library_api: u32,
    pub(crate) required_driver_protocol: u32,
    pub(crate) driver_protocol: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DokanyPreflightError {
    RuntimeNotInstalled,
    RuntimeArchitectureMismatch,
    RuntimeLoadFailed { win32_error: u32 },
    RuntimeSymbolMissing { symbol: &'static str },
    LibraryApiMismatch { expected: u32, found: u32 },
    DriverUnavailable,
    DriverProtocolMismatch { expected: u32, found: u32 },
}

impl fmt::Display for DokanyPreflightError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeNotInstalled => write!(
                formatter,
                "Dokany 2.3.1 is not installed ({DOKANY_DLL_NAME} is absent from System32)"
            ),
            Self::RuntimeArchitectureMismatch => write!(
                formatter,
                "the installed {DOKANY_DLL_NAME} has the wrong processor architecture"
            ),
            Self::RuntimeLoadFailed { win32_error } => write!(
                formatter,
                "Windows could not load {DOKANY_DLL_NAME} from System32 (error {win32_error})"
            ),
            Self::RuntimeSymbolMissing { symbol } => write!(
                formatter,
                "the installed {DOKANY_DLL_NAME} is missing required API symbol {symbol}"
            ),
            Self::LibraryApiMismatch { expected, found } => write!(
                formatter,
                "Dokany library API mismatch: required {expected}, found {found}"
            ),
            Self::DriverUnavailable => write!(
                formatter,
                "the Dokany driver is not installed, not running, or not reachable"
            ),
            Self::DriverProtocolMismatch { expected, found } => write!(
                formatter,
                "Dokany driver protocol mismatch: required {expected}, found {found}"
            ),
        }
    }
}

impl std::error::Error for DokanyPreflightError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DokanyCreateError {
    General,
    InvalidDriveLetter,
    DriverInstall,
    DriverStart,
    Mount,
    InvalidMountPoint,
    Version,
    NullHandle,
    Unknown(i32),
}

impl fmt::Display for DokanyCreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::General => "Dokany could not create the file system",
            Self::InvalidDriveLetter => "the requested drive letter is invalid or unavailable",
            Self::DriverInstall => "the Dokany driver could not be installed",
            Self::DriverStart => "the Dokany driver could not start the file system",
            Self::Mount => "Windows could not mount the Dokany file system",
            Self::InvalidMountPoint => "the requested Dokany mount point is invalid",
            Self::Version => "the Dokany runtime and driver versions are incompatible",
            Self::NullHandle => "Dokany reported success without returning a file-system handle",
            Self::Unknown(code) => {
                return write!(formatter, "Dokany mount failed with code {code}")
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for DokanyCreateError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DokanyWaitOutcome {
    Closed,
    Timeout,
    Failed { win32_error: u32 },
    Unexpected(u32),
}

#[derive(Clone)]
pub(crate) struct DokanyRuntime {
    inner: Arc<RuntimeInner>,
}

impl DokanyRuntime {
    /// Loads only the System32 copy, validates library API 231 and driver
    /// protocol `0x190`, then initializes Dokany. No DLL access occurs before
    /// this explicit call.
    pub(crate) fn preflight() -> Result<Self, DokanyPreflightError> {
        let module = LoadedModule::system32()?;
        let api = Api::resolve(&module)?;
        let library_api = unsafe { (api.version)() };
        let driver_protocol = unsafe { (api.driver_protocol_version)() };
        validate_dokany_version_domains(library_api, driver_protocol).map_err(
            |error| match error {
                DokanyVersionCompatibilityError::LibraryApiMismatch { expected, found } => {
                    DokanyPreflightError::LibraryApiMismatch { expected, found }
                }
                DokanyVersionCompatibilityError::DriverUnavailable => {
                    DokanyPreflightError::DriverUnavailable
                }
                DokanyVersionCompatibilityError::DriverProtocolMismatch { expected, found } => {
                    DokanyPreflightError::DriverProtocolMismatch { expected, found }
                }
            },
        )?;

        unsafe { (api.init)() };
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                _module: module,
                api,
                info: DokanyRuntimeInfo {
                    required_library_api: DOKANY_LIBRARY_API_VERSION,
                    library_api,
                    required_driver_protocol: DOKANY_DRIVER_PROTOCOL_VERSION,
                    driver_protocol,
                },
            }),
        })
    }

    pub(crate) fn info(&self) -> DokanyRuntimeInfo {
        self.inner.info
    }

    /// `options`, `operations`, their referenced strings, and `global_context`
    /// must remain valid until the returned filesystem has been closed.
    pub(crate) unsafe fn create_file_system_raw(
        &self,
        options: *mut DokanOptions,
        operations: *mut DokanOperations,
    ) -> Result<DokanyFileSystem, DokanyCreateError> {
        let mut handle: DokanHandle = null_mut();
        let status =
            unsafe { (self.inner.api.create_file_system)(options, operations, &mut handle) };
        if status != SUCCESS {
            return Err(DokanyCreateError::from_code(status));
        }
        let handle = NonNull::new(handle).ok_or(DokanyCreateError::NullHandle)?;
        Ok(DokanyFileSystem {
            runtime: Arc::clone(&self.inner),
            handle: Some(handle),
        })
    }

    pub(crate) unsafe fn reset_timeout(
        &self,
        timeout_ms: u32,
        file_info: *mut DokanFileInfo,
    ) -> bool {
        unsafe { (self.inner.api.reset_timeout)(timeout_ms, file_info) != 0 }
    }

    pub(crate) unsafe fn is_name_in_expression(
        &self,
        expression: *const u16,
        name: *const u16,
        ignore_case: bool,
    ) -> bool {
        unsafe {
            (self.inner.api.is_name_in_expression)(expression, name, i32::from(ignore_case)) != 0
        }
    }

    pub(crate) fn nt_status_from_win32(&self, error: u32) -> NtStatus {
        unsafe { (self.inner.api.nt_status_from_win32)(error) }
    }
}

pub(crate) struct DokanyFileSystem {
    runtime: Arc<RuntimeInner>,
    handle: Option<NonNull<c_void>>,
}

impl DokanyFileSystem {
    pub(crate) fn is_running(&self) -> bool {
        self.handle
            .map(|handle| unsafe {
                (self.runtime.api.is_file_system_running)(handle.as_ptr()) != 0
            })
            .unwrap_or(false)
    }

    pub(crate) fn wait(&self, timeout_ms: u32) -> DokanyWaitOutcome {
        let Some(handle) = self.handle else {
            return DokanyWaitOutcome::Closed;
        };
        match unsafe { (self.runtime.api.wait_for_file_system_closed)(handle.as_ptr(), timeout_ms) }
        {
            WAIT_OBJECT_0 => DokanyWaitOutcome::Closed,
            WAIT_TIMEOUT => DokanyWaitOutcome::Timeout,
            WAIT_FAILED => DokanyWaitOutcome::Failed {
                win32_error: unsafe { GetLastError() },
            },
            result => DokanyWaitOutcome::Unexpected(result),
        }
    }

    pub(crate) fn close(mut self) {
        self.close_inner();
    }

    fn close_inner(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe { (self.runtime.api.close_handle)(handle.as_ptr()) };
        }
    }
}

impl Drop for DokanyFileSystem {
    fn drop(&mut self) {
        self.close_inner();
    }
}

// Dokany instance calls are thread-safe; the opaque handle is owned and closed
// exactly once by this type.
unsafe impl Send for DokanyFileSystem {}
unsafe impl Sync for DokanyFileSystem {}

impl DokanyCreateError {
    fn from_code(code: i32) -> Self {
        match code {
            ERROR => Self::General,
            DRIVE_LETTER_ERROR => Self::InvalidDriveLetter,
            DRIVER_INSTALL_ERROR => Self::DriverInstall,
            START_ERROR => Self::DriverStart,
            MOUNT_ERROR => Self::Mount,
            MOUNT_POINT_ERROR => Self::InvalidMountPoint,
            VERSION_ERROR => Self::Version,
            other => Self::Unknown(other),
        }
    }
}

struct RuntimeInner {
    _module: LoadedModule,
    api: Api,
    info: DokanyRuntimeInfo,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        unsafe { (self.api.shutdown)() };
    }
}

// Function pointers and the loaded module are process-global Windows objects;
// Dokany documents the API and callbacks as multi-threaded.
unsafe impl Send for RuntimeInner {}
unsafe impl Sync for RuntimeInner {}

struct Api {
    init: InitFn,
    shutdown: ShutdownFn,
    version: VersionFn,
    driver_protocol_version: VersionFn,
    create_file_system: CreateFileSystemFn,
    is_file_system_running: IsFileSystemRunningFn,
    wait_for_file_system_closed: WaitForFileSystemClosedFn,
    close_handle: CloseHandleFn,
    reset_timeout: ResetTimeoutFn,
    is_name_in_expression: IsNameInExpressionFn,
    nt_status_from_win32: NtStatusFromWin32Fn,
}

impl Api {
    fn resolve(module: &LoadedModule) -> Result<Self, DokanyPreflightError> {
        macro_rules! symbol {
            ($name:literal, $kind:ty) => {{
                let raw = module.symbol(concat!($name, "\0").as_bytes(), $name)?;
                unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $kind>(raw) }
            }};
        }

        Ok(Self {
            init: symbol!("DokanInit", InitFn),
            shutdown: symbol!("DokanShutdown", ShutdownFn),
            version: symbol!("DokanVersion", VersionFn),
            driver_protocol_version: symbol!("DokanDriverVersion", VersionFn),
            create_file_system: symbol!("DokanCreateFileSystem", CreateFileSystemFn),
            is_file_system_running: symbol!("DokanIsFileSystemRunning", IsFileSystemRunningFn),
            wait_for_file_system_closed: symbol!(
                "DokanWaitForFileSystemClosed",
                WaitForFileSystemClosedFn
            ),
            close_handle: symbol!("DokanCloseHandle", CloseHandleFn),
            reset_timeout: symbol!("DokanResetTimeout", ResetTimeoutFn),
            is_name_in_expression: symbol!("DokanIsNameInExpression", IsNameInExpressionFn),
            nt_status_from_win32: symbol!("DokanNtStatusFromWin32", NtStatusFromWin32Fn),
        })
    }
}

struct LoadedModule(HMODULE);

impl LoadedModule {
    fn system32() -> Result<Self, DokanyPreflightError> {
        let handle = unsafe {
            LoadLibraryExW(
                DOKANY_DLL_WIDE.as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if handle.is_null() {
            let win32_error = unsafe { GetLastError() };
            return Err(match win32_error {
                ERROR_MOD_NOT_FOUND => DokanyPreflightError::RuntimeNotInstalled,
                ERROR_BAD_EXE_FORMAT => DokanyPreflightError::RuntimeArchitectureMismatch,
                _ => DokanyPreflightError::RuntimeLoadFailed { win32_error },
            });
        }
        Ok(Self(handle))
    }

    fn symbol(
        &self,
        nul_terminated_name: &'static [u8],
        display_name: &'static str,
    ) -> Result<unsafe extern "system" fn() -> isize, DokanyPreflightError> {
        unsafe { GetProcAddress(self.0, nul_terminated_name.as_ptr()) }.ok_or(
            DokanyPreflightError::RuntimeSymbolMissing {
                symbol: display_name,
            },
        )
    }
}

impl Drop for LoadedModule {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}
