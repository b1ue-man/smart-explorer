use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};

const DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IFACE: &str = "org.freedesktop.systemd1.Manager";
const INTERNAL_MODE: &str = "--share-exec-supervisor";

pub(super) fn start_unit(
    connection: &Connection,
    name: &str,
    socket: &Path,
    runtime_usec: Option<u64>,
) -> io::Result<()> {
    let executable = format!("/proc/{}/exe", std::process::id());
    let socket = socket.to_string_lossy().into_owned();
    let exec = vec![(
        executable.clone(),
        vec![executable, INTERNAL_MODE.into(), socket],
        false,
    )];
    let mut properties = vec![
        (
            "Description",
            string_value("Smart Explorer remote execution"),
        ),
        ("Type", string_value("exec")),
        ("ExitType", string_value("main")),
        ("ExecStart", complex_value(exec)?),
        ("KillMode", string_value("control-group")),
        ("KillSignal", OwnedValue::from(libc::SIGKILL)),
        ("SendSIGKILL", OwnedValue::from(true)),
        ("Restart", string_value("no")),
        ("OOMPolicy", string_value("stop")),
        ("TimeoutStartUSec", OwnedValue::from(15_000_000u64)),
        ("TimeoutStopUSec", OwnedValue::from(2_000_000u64)),
        ("CollectMode", string_value("inactive-or-failed")),
    ];
    if let Some(runtime_usec) = runtime_usec {
        properties.push(("RuntimeMaxUSec", OwnedValue::from(runtime_usec)));
    }
    let auxiliary: Vec<(&str, Vec<(&str, OwnedValue)>)> = Vec::new();
    let _: OwnedObjectPath = manager_proxy(connection)?
        .call("StartTransientUnit", &(name, "fail", properties, auxiliary))
        .map_err(eio)?;
    Ok(())
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

fn string_value(value: &str) -> OwnedValue {
    OwnedValue::from(Str::from(value.to_owned()))
}

fn complex_value<T>(value: T) -> io::Result<OwnedValue>
where
    T: zbus::zvariant::Type + serde::Serialize,
    Value<'static>: From<T>,
{
    OwnedValue::try_from(Value::from(value)).map_err(eio)
}

fn eio(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
