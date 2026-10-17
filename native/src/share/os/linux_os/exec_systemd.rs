use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Str, Value};

const DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IFACE: &str = "org.freedesktop.systemd1.Manager";
const INTERNAL_MODE: &str = "--share-exec-supervisor";

/// One entry of `ListUnitsByPatterns`: name, description, load, active and
/// sub state, follower, unit path, job id, job type, job path.
type UnitListing = (
    Text,
    Text,
    Text,
    Text,
    Text,
    Text,
    OwnedObjectPath,
    u32,
    Text,
    OwnedObjectPath,
);
type Text = String;

pub(super) fn start_unit(
    connection: &Connection,
    name: &str,
    socket: &Path,
    runtime_usec: Option<u64>,
) -> io::Result<()> {
    start_unit_with_collect_mode(connection, name, socket, runtime_usec, "inactive-or-failed")
}

#[cfg(debug_assertions)]
pub(super) fn start_runtime_test_unit(
    connection: &Connection,
    name: &str,
    socket: &Path,
    runtime_usec: u64,
) -> io::Result<()> {
    // Retain the expected failed unit until the self-test has verified its
    // exact Result. Production units continue to collect both terminal states.
    start_unit_with_collect_mode(connection, name, socket, Some(runtime_usec), "inactive")
}

fn start_unit_with_collect_mode(
    connection: &Connection,
    name: &str,
    socket: &Path,
    runtime_usec: Option<u64>,
    collect_mode: &str,
) -> io::Result<()> {
    let executable = format!("/proc/{}/exe", std::process::id());
    let socket = socket.to_string_lossy().into_owned();
    let properties = unit_properties(executable, socket, runtime_usec, collect_mode)?;
    start_transient_unit(connection, name, properties)
}

/// Starts the transient service `name` with `properties` (job mode `fail`).
pub(super) fn start_transient_unit(
    connection: &Connection,
    name: &str,
    properties: Vec<(&'static str, OwnedValue)>,
) -> io::Result<()> {
    let auxiliary: Vec<(&str, Vec<(&str, OwnedValue)>)> = Vec::new();
    let _: OwnedObjectPath = manager_proxy(connection)?
        .call("StartTransientUnit", &(name, "fail", properties, auxiliary))
        .map_err(eio)?;
    Ok(())
}

/// Properties of the transient unit of one job of the worker's own account.
pub(super) fn unit_properties(
    executable: String,
    socket: String,
    runtime_usec: Option<u64>,
    collect_mode: &str,
) -> io::Result<Vec<(&'static str, OwnedValue)>> {
    let exec = vec![(
        executable.clone(),
        vec![executable, INTERNAL_MODE.into(), socket],
        false,
    )];
    job_properties(
        "Smart Explorer remote execution",
        exec,
        runtime_usec,
        collect_mode,
    )
}

/// Properties of the unit of every job, also of the account units
/// (`exec_account_unit.rs`). Every one of them exists in systemd 249 (Ubuntu
/// 22.04), which refuses a unit with any property it does not know.
/// `ExitType` is not sent: it exists only from systemd 250 on, where its
/// default `main` is what the unit needs.
pub(super) fn job_properties(
    description: &str,
    exec: Vec<(String, Vec<String>, bool)>,
    runtime_usec: Option<u64>,
    collect_mode: &str,
) -> io::Result<Vec<(&'static str, OwnedValue)>> {
    let mut properties = vec![
        ("Description", string_value(description)),
        ("Type", string_value("exec")),
        ("ExecStart", complex_value(exec)?),
        ("KillMode", string_value("control-group")),
        ("KillSignal", OwnedValue::from(libc::SIGKILL)),
        ("SendSIGKILL", OwnedValue::from(true)),
        ("Restart", string_value("no")),
        ("OOMPolicy", string_value("stop")),
        ("TimeoutStartUSec", OwnedValue::from(15_000_000u64)),
        ("TimeoutStopUSec", OwnedValue::from(2_000_000u64)),
        ("CollectMode", string_value(collect_mode)),
    ];
    if let Some(runtime_usec) = runtime_usec {
        properties.push(("RuntimeMaxUSec", OwnedValue::from(runtime_usec)));
    }
    Ok(properties)
}

pub(super) fn stop_unit(connection: &Connection, name: &str) -> io::Result<()> {
    let result: zbus::Result<OwnedObjectPath> =
        manager_proxy(connection)?.call("StopUnit", &(name, "replace"));
    match result {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("not loaded") => Ok(()),
        Err(error) => Err(eio(error)),
    }
}

pub(super) fn manager_connection() -> io::Result<Connection> {
    if unsafe { libc::geteuid() } == 0 {
        Connection::system().map_err(eio)
    } else {
        let address = format!("unix:path=/run/user/{}/bus", unsafe { libc::geteuid() });
        zbus::blocking::connection::Builder::address(address.as_str())
            .map_err(eio)?
            .build()
            .map_err(eio)
    }
}

pub(super) fn wait_for_unit_pid(
    connection: &Connection,
    name: &str,
    expected: u32,
    deadline: Instant,
) -> io::Result<OwnedObjectPath> {
    loop {
        if let Ok(path) =
            manager_proxy(connection)?.call::<_, _, OwnedObjectPath>("GetUnit", &(name,))
        {
            let matches = Proxy::new(
                connection,
                DESTINATION,
                path.as_str(),
                "org.freedesktop.systemd1.Service",
            )
            .ok()
            .and_then(|service| service.get_property::<u32>("MainPID").ok())
                == Some(expected);
            if matches {
                return Ok(path);
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "systemd did not confirm the exec supervisor MainPID",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn unit_control_group(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> io::Result<PathBuf> {
    let value: String = service_proxy(connection, path)?
        .get_property("ControlGroup")
        .map_err(eio)?;
    checked_cgroup_path(&value)
}

pub(super) fn unit_active_state(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> io::Result<String> {
    unit_proxy(connection, path)?
        .get_property("ActiveState")
        .map_err(eio)
}

/// The object path of a loaded unit; `None` when systemd has not loaded it
/// (any more).
pub(super) fn loaded_unit(
    connection: &Connection,
    name: &str,
) -> io::Result<Option<OwnedObjectPath>> {
    let result: zbus::Result<OwnedObjectPath> =
        manager_proxy(connection)?.call("GetUnit", &(name,));
    match result {
        Ok(path) => Ok(Some(path)),
        Err(error) if error.to_string().contains("not loaded") => Ok(None),
        Err(error) => Err(eio(error)),
    }
}

/// `MainPID` of a service unit; 0 while none runs.
pub(super) fn unit_main_pid(connection: &Connection, path: &OwnedObjectPath) -> io::Result<u32> {
    service_proxy(connection, path)?
        .get_property("MainPID")
        .map_err(eio)
}

/// Why a service unit ended: its `Result` and the exit status of its main
/// process (`ExecMainStatus`, for example 224 when PAM refused the account).
pub(super) fn unit_exit_status(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> io::Result<(String, i32)> {
    let service = service_proxy(connection, path)?;
    let result: String = service.get_property("Result").map_err(eio)?;
    let status: i32 = service.get_property("ExecMainStatus").map_err(eio)?;
    Ok((result, status))
}

/// Whether the unit has left its first inactive state, so an inactive unit
/// has ended instead of waiting for its start job.
pub(super) fn unit_was_started(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> io::Result<bool> {
    let left: u64 = unit_proxy(connection, path)?
        .get_property("InactiveExitTimestampMonotonic")
        .map_err(eio)?;
    Ok(left > 0)
}

/// `User=` of a service unit; empty for root.
pub(super) fn unit_user(connection: &Connection, path: &OwnedObjectPath) -> io::Result<String> {
    service_proxy(connection, path)?
        .get_property("User")
        .map_err(eio)
}

/// Names of the loaded units that match `pattern` (`ListUnitsByPatterns`).
pub(super) fn unit_names(connection: &Connection, pattern: &str) -> io::Result<Vec<String>> {
    let units: Vec<UnitListing> = manager_proxy(connection)?
        .call("ListUnitsByPatterns", &(Vec::<&str>::new(), vec![pattern]))
        .map_err(eio)?;
    Ok(units.into_iter().map(|unit| unit.0).collect())
}

/// Reads the unit files again (`systemctl daemon-reload`).
pub(super) fn reload_manager(connection: &Connection) -> io::Result<()> {
    manager_proxy(connection)?
        .call::<_, _, ()>("Reload", &())
        .map_err(eio)
}

/// Enables the unit file `name` (its `[Install]` section).
pub(super) fn enable_unit_file(connection: &Connection, name: &str) -> io::Result<()> {
    let _: (bool, Vec<(String, String, String)>) = manager_proxy(connection)?
        .call("EnableUnitFiles", &(vec![name], false, true))
        .map_err(eio)?;
    Ok(())
}

/// Disables the unit file `name`; a missing file is no error.
pub(super) fn disable_unit_file(connection: &Connection, name: &str) -> io::Result<()> {
    let result: zbus::Result<Vec<(String, String, String)>> =
        manager_proxy(connection)?.call("DisableUnitFiles", &(vec![name], false));
    match result {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("does not exist") => Ok(()),
        Err(error) => Err(eio(error)),
    }
}

/// Starts the unit `name` anew (stops it first if it runs).
pub(super) fn restart_unit(connection: &Connection, name: &str) -> io::Result<()> {
    let _: OwnedObjectPath = manager_proxy(connection)?
        .call("RestartUnit", &(name, "replace"))
        .map_err(eio)?;
    Ok(())
}

#[cfg(debug_assertions)]
pub(super) fn unit_runtime_max_usec(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> io::Result<u64> {
    service_proxy(connection, path)?
        .get_property("RuntimeMaxUSec")
        .map_err(eio)
}

#[cfg(debug_assertions)]
pub(super) fn unit_result(connection: &Connection, path: &OwnedObjectPath) -> io::Result<String> {
    service_proxy(connection, path)?
        .get_property("Result")
        .map_err(eio)
}

/// Unloads a failed unit; one that is not loaded is no error.
pub(super) fn reset_failed_unit(connection: &Connection, name: &str) -> io::Result<()> {
    let result: zbus::Result<()> = manager_proxy(connection)?.call("ResetFailedUnit", &(name,));
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.to_string().contains("not loaded") => Ok(()),
        Err(error) => Err(eio(error)),
    }
}

pub(super) fn cgroup_populated(path: &Path) -> io::Result<bool> {
    let events = match std::fs::read_to_string(path.join("cgroup.events")) {
        Ok(events) => events,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(!events.lines().any(|line| line.trim() == "populated 0"))
}

pub(super) fn require_cgroup_v2() -> io::Result<()> {
    if Path::new("/sys/fs/cgroup/cgroup.controllers").is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cgroup v2 is unavailable",
        ))
    }
}

fn manager_proxy(connection: &Connection) -> io::Result<Proxy<'_>> {
    Proxy::new(connection, DESTINATION, MANAGER_PATH, MANAGER_IFACE).map_err(eio)
}

fn unit_proxy<'a>(connection: &'a Connection, path: &'a OwnedObjectPath) -> io::Result<Proxy<'a>> {
    Proxy::new(
        connection,
        DESTINATION,
        path.as_str(),
        "org.freedesktop.systemd1.Unit",
    )
    .map_err(eio)
}

fn service_proxy<'a>(
    connection: &'a Connection,
    path: &'a OwnedObjectPath,
) -> io::Result<Proxy<'a>> {
    Proxy::new(
        connection,
        DESTINATION,
        path.as_str(),
        "org.freedesktop.systemd1.Service",
    )
    .map_err(eio)
}

fn checked_cgroup_path(value: &str) -> io::Result<PathBuf> {
    let relative = Path::new(value.trim_start_matches('/'));
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "systemd returned an invalid cgroup path",
        ));
    }
    Ok(Path::new("/sys/fs/cgroup").join(relative))
}

pub(super) fn string_value(value: &str) -> OwnedValue {
    OwnedValue::from(Str::from(value.to_owned()))
}

/// A descriptor property (`h`); the value owns a duplicate of `fd`, which
/// D-Bus hands to systemd with the call.
pub(super) fn fd_value(fd: BorrowedFd<'_>) -> io::Result<OwnedValue> {
    OwnedValue::try_from(Value::from(Fd::from(fd))).map_err(eio)
}

fn complex_value<T>(value: T) -> io::Result<OwnedValue>
where
    T: zbus::zvariant::Type + serde::Serialize,
    Value<'static>: From<T>,
{
    OwnedValue::try_from(Value::from(value)).map_err(eio)
}

pub(super) fn eio(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd;
    use std::path::Path;

    use zbus::zvariant::OwnedValue;

    use super::super::exec_account_unit::{
        account_properties, account_unit_properties, working_directory,
    };
    use super::unit_properties;

    /// Every unit property systemd 249 (Ubuntu 22.04) knows and this host may
    /// send, including those of the account units (PRM-M5). systemd 249
    /// refuses a whole unit for one unknown property.
    const SYSTEMD_249_PROPERTIES: [&str; 17] = [
        "Description",
        "Type",
        "ExecStart",
        "KillMode",
        "KillSignal",
        "SendSIGKILL",
        "Restart",
        "OOMPolicy",
        "TimeoutStartUSec",
        "TimeoutStopUSec",
        "CollectMode",
        "RuntimeMaxUSec",
        "User",
        "PAMName",
        "WorkingDirectory",
        "StandardInputFileDescriptor",
        "StandardOutputFileDescriptor",
    ];

    #[test]
    fn perm_task_systemd_unit_props_v249_compatible() {
        let (_worker, supervisor) = std::os::unix::net::UnixStream::pair().unwrap();
        for runtime in [None, Some(5_000_000)] {
            for collect_mode in ["inactive-or-failed", "inactive"] {
                let properties = unit_properties(
                    "/proc/1/exe".into(),
                    "/run/user/1000/se-exec.sock".into(),
                    runtime,
                    collect_mode,
                )
                .unwrap();
                assert_v249_compatible(&properties, runtime.is_some());
            }
            // The account units (PRM-M5) send their own properties as well.
            for cwd in [None, Some("/srv")] {
                let account =
                    account_properties("se-remote", &working_directory(cwd), supervisor.as_fd())
                        .unwrap();
                let properties = account_unit_properties(
                    Path::new("/usr/local/sbin/smart-explorer-exec"),
                    account,
                    runtime,
                )
                .unwrap();
                assert_v249_compatible(&properties, runtime.is_some());
            }
        }
    }

    fn assert_v249_compatible(properties: &[(&'static str, OwnedValue)], runtime: bool) {
        let names: Vec<&str> = properties.iter().map(|(name, _)| *name).collect();
        for name in &names {
            assert!(
                SYSTEMD_249_PROPERTIES.contains(name),
                "systemd 249 does not know the unit property {name}"
            );
        }
        assert!(!names.contains(&"ExitType"));
        assert_eq!(names.contains(&"RuntimeMaxUSec"), runtime);
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "a property is sent twice");
    }
}
