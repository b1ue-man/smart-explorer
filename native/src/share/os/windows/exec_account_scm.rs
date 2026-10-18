//! The service `SmartExplorerExecAccounts` in the service control manager
//! (C2, D.5): LocalSystem, started on demand, its program the protected
//! copy. Registered users may start it and query its state (service DACL,
//! NR N5); everything else needs an administrator.
use std::io;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_EXISTS,
    ERROR_SERVICE_NOT_ACTIVE,
};
use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
use windows_sys::Win32::System::Services::{
    ChangeServiceConfig2W, ChangeServiceConfigW, CloseServiceHandle, ControlService,
    CreateServiceW, DeleteService, OpenSCManagerW, OpenServiceW, QueryServiceStatus,
    SetServiceObjectSecurity, StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT,
    SC_MANAGER_CREATE_SERVICE, SERVICE_ALL_ACCESS, SERVICE_CONFIG_DESCRIPTION,
    SERVICE_CONTROL_STOP, SERVICE_DEMAND_START, SERVICE_DESCRIPTIONW, SERVICE_ERROR_NORMAL,
    SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STATUS, SERVICE_STOPPED,
    SERVICE_WIN32_OWN_PROCESS,
};

use super::exec_account_config::{SERVICE_DISPLAY_NAME, SERVICE_NAME};
use super::exec_account_security::{wide, SecurityDescriptor};

/// The program's mode as a service (`exec_account_service.rs`).
pub(super) const SERVICE_MODE: &str = "--exec-account-service";
const DESCRIPTION: &str = "Runs remote commands of Smart Explorer as the local accounts an \
                           administrator allowed for them (se share exec accounts).";
/// Default rights of a service (SYSTEM, Administrators, interactive and
/// service users may query it).
const BASE_DACL: &str = "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)\
                         (A;;CCLCSWLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)";
const STOP_WAIT: Duration = Duration::from_secs(30);

struct ScHandle(SC_HANDLE);

impl ScHandle {
    fn new(handle: SC_HANDLE) -> io::Result<Self> {
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for ScHandle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0) };
    }
}

fn manager(access: u32) -> io::Result<ScHandle> {
    ScHandle::new(unsafe { OpenSCManagerW(null(), null(), access) })
}

/// The service with `access`; `Ok(None)` when it is not installed.
fn service(access: u32) -> io::Result<Option<ScHandle>> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let name = wide(SERVICE_NAME);
    match ScHandle::new(unsafe { OpenServiceW(manager.0, name.as_ptr(), access) }) {
        Ok(service) => Ok(Some(service)),
        Err(error) if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST as i32) => Ok(None),
        Err(error) => Err(error),
    }
}

fn status(service: &ScHandle) -> io::Result<SERVICE_STATUS> {
    let mut status: SERVICE_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { QueryServiceStatus(service.0, &mut status) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(status)
}

/// The service's state (`SERVICE_RUNNING`, …); `Ok(None)` when it is not
/// installed.
pub(super) fn service_state() -> io::Result<Option<u32>> {
    match service(SERVICE_QUERY_STATUS)? {
        Some(service) => Ok(Some(status(&service)?.dwCurrentState)),
        None => Ok(None),
    }
}

/// Starts the service (a registered user may); a running service is fine.
pub(super) fn start_service() -> io::Result<()> {
    let service = service(SERVICE_START)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the account service is not installed",
        )
    })?;
    if unsafe { StartServiceW(service.0, 0, null()) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_SERVICE_ALREADY_RUNNING as i32) {
            return Err(error);
        }
    }
    Ok(())
}

/// Registers the service with `program` (administrators only) or points an
/// existing one at it; `Ok(true)` when it was new. `users` may start it.
pub(super) fn install_service(program: &Path, users: &[&str]) -> io::Result<bool> {
    let manager = manager(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
    let name = wide(SERVICE_NAME);
    let display = wide(SERVICE_DISPLAY_NAME);
    let local_system = wide("LocalSystem");
    // LocalSystem has no password: an empty one, as the SCM asks for it.
    let no_password = wide("");
    // Quoted: the path has spaces ("Program Files").
    let command = wide(format!("\"{}\" {SERVICE_MODE}", program.display()));
    let created = unsafe {
        CreateServiceW(
            manager.0,
            name.as_ptr(),
            display.as_ptr(),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_DEMAND_START,
            SERVICE_ERROR_NORMAL,
            command.as_ptr(),
            null(),
            null_mut(),
            null(),
            null(),
            null(),
        )
    };
    let (service, new) = match ScHandle::new(created) {
        Ok(service) => (service, true),
        Err(error) if error.raw_os_error() == Some(ERROR_SERVICE_EXISTS as i32) => {
            let service = ScHandle::new(unsafe {
                OpenServiceW(manager.0, name.as_ptr(), SERVICE_ALL_ACCESS)
            })?;
            let changed = unsafe {
                ChangeServiceConfigW(
                    service.0,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_DEMAND_START,
                    SERVICE_ERROR_NORMAL,
                    command.as_ptr(),
                    null(),
                    null_mut(),
                    null(),
                    local_system.as_ptr(),
                    no_password.as_ptr(),
                    display.as_ptr(),
                )
            };
            if changed == 0 {
                return Err(io::Error::last_os_error());
            }
            (service, false)
        }
        Err(error) => return Err(error),
    };
    let mut text = wide(DESCRIPTION);
    let description = SERVICE_DESCRIPTIONW {
        lpDescription: text.as_mut_ptr(),
    };
    let described = unsafe {
        ChangeServiceConfig2W(
            service.0,
            SERVICE_CONFIG_DESCRIPTION,
            (&description as *const SERVICE_DESCRIPTIONW).cast(),
        )
    };
    if described == 0 {
        return Err(io::Error::last_os_error());
    }
    set_access(&service, users)?;
    Ok(new)
}

/// Gives `users` the right to start the service and query its state.
pub(super) fn set_service_access(users: &[&str]) -> io::Result<()> {
    match service(SERVICE_ALL_ACCESS)? {
        Some(service) => set_access(&service, users),
        None => Ok(()),
    }
}

/// The service DACL of D.5 for `users` (SIDs).
pub(super) fn service_sddl(users: &[&str]) -> String {
    let mut sddl = BASE_DACL.to_string();
    for sid in users {
        sddl.push_str(&format!("(A;;LCRP;;;{sid})"));
    }
    sddl
}

fn set_access(service: &ScHandle, users: &[&str]) -> io::Result<()> {
    let descriptor = SecurityDescriptor::from_sddl(&service_sddl(users))?;
    let set =
        unsafe { SetServiceObjectSecurity(service.0, DACL_SECURITY_INFORMATION, descriptor.raw()) };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Stops the service and waits until it stopped; its sessions end with it
/// (kill-on-close). `Ok(false)` when it was not running or not installed.
pub(super) fn stop_service() -> io::Result<bool> {
    let Some(service) = service(SERVICE_ALL_ACCESS)? else {
        return Ok(false);
    };
    let mut current: SERVICE_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut current) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_SERVICE_NOT_ACTIVE as i32) {
            return Ok(false);
        }
        return Err(error);
    }
    let deadline = Instant::now() + STOP_WAIT;
    while status(&service)?.dwCurrentState != SERVICE_STOPPED {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the account service did not stop within 30 seconds",
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(true)
}

/// Stops and deletes the service; `Ok(false)` when it was not installed.
pub(super) fn delete_service() -> io::Result<bool> {
    stop_service()?;
    let Some(service) = service(SERVICE_ALL_ACCESS)? else {
        return Ok(false);
    };
    if unsafe { DeleteService(service.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(true)
}
