#[cfg(debug_assertions)]
#[path = "exec_self_test.rs"]
mod exec_self_test;
#[path = "exec_supervisor.rs"]
mod exec_supervisor;
#[path = "exec_systemd.rs"]
mod exec_systemd;

use std::ffi::{CStr, OsString};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

use self::exec_systemd::{
    cgroup_populated, manager_connection, require_cgroup_v2, start_unit, stop_unit,
    unit_active_state, unit_control_group, wait_for_unit_pid,
};

use crate::share::exec_platform::StopReason;
use crate::share::exec_supervisor_protocol::{
    recv_event, send_command, SupervisorCommand, SupervisorEvent,
};
use crate::share::exec_types::{ExecProviderStatus, ExecStart};

const INTERNAL_MODE: &str = "--share-exec-supervisor";

pub(crate) struct ContainedExec {
    control: Arc<Control>,
    unit_path: OwnedObjectPath,
    control_group: PathBuf,
    socket_path: PathBuf,
    events: mpsc::Receiver<io::Result<SupervisorEvent>>,
    #[cfg(debug_assertions)]
    supervisor_pid: u32,
}

struct Control {
    connection: Connection,
    unit_name: String,
    outbound: mpsc::SyncSender<SupervisorCommand>,
    stopped: AtomicBool,
}

impl ContainedExec {
    pub(crate) fn prepare(request: &ExecStart) -> io::Result<Self> {
        require_cgroup_v2()?;
        let connection = manager_connection()?;
        let directory = runtime_directory()?;
        let socket_path = directory.join(format!("{}.sock", request.exec_id.as_str()));
        let listener = std::os::unix::net::UnixListener::bind(&socket_path)?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let unit_name = format!("smart-explorer-exec-{}.service", request.exec_id.as_str());
        let runtime_usec = request
            .effective_timeout_ms()
            .map(|timeout_ms| timeout_ms.saturating_add(30_000).saturating_mul(1_000));
        if let Err(error) = start_unit(&connection, &unit_name, &socket_path, runtime_usec) {
            let _ = std::fs::remove_file(&socket_path);
            return Err(error);
        }
        let accepted = accept_supervisor(&listener, Instant::now() + Duration::from_secs(15));
        let (stream, credentials) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                let _ = stop_unit(&connection, &unit_name);
                let _ = std::fs::remove_file(&socket_path);
                return Err(error);
            }
        };
        let setup = (|| {
            let expected_uid = unsafe { libc::geteuid() };
            if credentials.uid != expected_uid || credentials.gid != unsafe { libc::getegid() } {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "exec supervisor has the wrong Unix credentials",
                ));
            }
            let unit_path = wait_for_unit_pid(
                &connection,
                &unit_name,
                credentials.pid as u32,
                Instant::now() + Duration::from_secs(10),
            )?;
            let control_group = unit_control_group(&connection, &unit_path)?;
            Ok((unit_path, control_group))
        })();
        let (unit_path, control_group) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                let _ = stop_unit(&connection, &unit_name);
                let _ = std::fs::remove_file(&socket_path);
                return Err(error);
            }
        };
        let writer = match stream.try_clone() {
            Ok(writer) => writer,
            Err(error) => {
                let _ = stop_unit(&connection, &unit_name);
                let _ = std::fs::remove_file(&socket_path);
                return Err(error);
            }
        };
        if let Err(error) = writer.set_write_timeout(Some(Duration::from_secs(10))) {
            let _ = stop_unit(&connection, &unit_name);
            let _ = std::fs::remove_file(&socket_path);
            return Err(error);
        }
        let (event_tx, events) = mpsc::sync_channel(32);
        let (outbound, outbound_rx) = mpsc::sync_channel::<SupervisorCommand>(16);
        let writer_errors = event_tx.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("exec-systemd-input".into())
            .spawn(move || {
                let mut writer = writer;
                while let Ok(command) = outbound_rx.recv() {
                    if let Err(error) = send_command(&mut writer, &command) {
                        let _ = writer_errors.send(Err(error));
                        break;
                    }
                }
            })
        {
            let _ = stop_unit(&connection, &unit_name);
            let _ = std::fs::remove_file(&socket_path);
            return Err(error);
        }
        if let Err(error) = std::thread::Builder::new()
            .name("exec-systemd-events".into())
            .spawn(move || {
                let mut reader = stream;
                loop {
                    let event = recv_event(&mut reader);
                    let terminal = event.is_err();
                    if event_tx.send(event).is_err() || terminal {
                        break;
                    }
                }
            })
        {
            let _ = stop_unit(&connection, &unit_name);
            let _ = std::fs::remove_file(&socket_path);
            return Err(error);
        }
        let control = Arc::new(Control {
            connection,
            unit_name,
            outbound,
            stopped: AtomicBool::new(false),
        });
        Ok(Self {
            control,
            unit_path,
            control_group,
            socket_path,
            events,
            #[cfg(debug_assertions)]
            supervisor_pid: credentials.pid as u32,
        })
    }

    pub(crate) fn configure(&mut self, _request: &ExecStart) -> io::Result<()> {
        Ok(())
    }

    pub(crate) fn send(&mut self, command: &SupervisorCommand) -> io::Result<()> {
        self.control.send(command)
    }

    pub(crate) fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<SupervisorEvent> {
        match deadline {
            Some(deadline) => self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(recv_timeout_error)?,
            None => self.events.recv().map_err(|_| supervisor_closed())?,
        }
    }

    pub(crate) fn terminate_all(&mut self, _reason: StopReason) -> io::Result<()> {
        self.control.terminate()
    }

    pub(crate) fn confirm_empty(&mut self, deadline: Instant) -> io::Result<()> {
        loop {
            let active = unit_active_state(&self.control.connection, &self.unit_path)
                .unwrap_or_else(|_| "inactive".into());
            let populated = cgroup_populated(&self.control_group)?;
            if !populated && matches!(active.as_str(), "inactive" | "failed") {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{} still has a populated cgroup", self.control.unit_name),
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Control {
    fn send(&self, command: &SupervisorCommand) -> io::Result<()> {
        self.outbound
            .try_send(command.clone())
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "exec supervisor input queue is full",
                ),
                mpsc::TrySendError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "exec supervisor input closed")
                }
            })
    }

    fn terminate(&self) -> io::Result<()> {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        // StopUnit is the non-cooperative kill path. It must not wait behind a
        // full supervisor input socket when a silent command is revoked.
        stop_unit(&self.connection, &self.unit_name)
    }
}

impl Drop for ContainedExec {
    fn drop(&mut self) {
        let _ = self.terminate_all(StopReason::WorkerStopping);
        let _ = self.confirm_empty(Instant::now() + Duration::from_secs(5));
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn recv_timeout_error(error: mpsc::RecvTimeoutError) -> io::Error {
    match error {
        mpsc::RecvTimeoutError::Timeout => {
            io::Error::new(io::ErrorKind::TimedOut, "exec event deadline elapsed")
        }
        mpsc::RecvTimeoutError::Disconnected => supervisor_closed(),
    }
}

fn supervisor_closed() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "exec supervisor closed")
}

pub(crate) fn provider_status() -> ExecProviderStatus {
    let elevated = unsafe { libc::geteuid() } == 0;
    let user_label = account_name();
    match require_cgroup_v2().and_then(|_| manager_connection().map(|_| ())) {
        Ok(()) => ExecProviderStatus {
            available: true,
            provider: "systemd transient cgroup".into(),
            detail: "cgroup v2 and the systemd manager are available".into(),
            elevated,
            user_label,
        },
        Err(error) => ExecProviderStatus {
            available: false,
            provider: "systemd transient cgroup".into(),
            detail: error.to_string(),
            elevated,
            user_label,
        },
    }
}

pub(crate) fn run_supervisor_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    #[cfg(debug_assertions)]
    if let Some(result) = exec_self_test::run_helper_if_requested(arguments) {
        return Some(result);
    }
    if arguments.len() != 2 || arguments[0] != INTERNAL_MODE {
        return None;
    }
    let socket = PathBuf::from(&arguments[1]);
    let result = (|| {
        let directory = runtime_directory()?;
        if !socket.starts_with(&directory)
            || socket.extension().and_then(|v| v.to_str()) != Some("sock")
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid exec supervisor socket",
            ));
        }
        let stream = std::os::unix::net::UnixStream::connect(&socket)?;
        // The connected Unix socket no longer needs a directory entry. Unlink
        // it in the independently supervised process so a SIGKILL of the
        // worker cannot leave a stale path behind.
        std::fs::remove_file(&socket)?;
        exec_supervisor::run(stream)
    })();
    Some(result)
}

#[cfg(debug_assertions)]
pub(crate) fn run_extended_self_test() -> io::Result<()> {
    exec_self_test::run()
}

fn accept_supervisor(
    listener: &std::os::unix::net::UnixListener,
    deadline: Instant,
) -> io::Result<(std::os::unix::net::UnixStream, libc::ucred)> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let credentials = peer_credentials(&stream)?;
                return Ok((stream, credentials));
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn peer_credentials(stream: &std::os::unix::net::UnixStream) -> io::Result<libc::ucred> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    if result == 0 {
        Ok(credentials)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn runtime_directory() -> io::Result<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let base = PathBuf::from(format!("/run/user/{uid}"));
    let base = if base.is_dir() {
        base
    } else {
        PathBuf::from(format!("/tmp/smart-explorer-runtime-{uid}"))
    };
    match std::fs::create_dir(&base) {
        Ok(()) => std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_owner_directory(&base, uid, "exec runtime base")?;
    let directory = base.join("smart-explorer-exec");
    match std::fs::create_dir(&directory) {
        Ok(()) => std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_owner_directory(&directory, uid, "exec runtime directory")?;
    Ok(directory)
}

fn validate_owner_directory(path: &Path, uid: u32, label: &str) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir()
        && !metadata.file_type().is_symlink()
        && metadata.uid() == uid
        && metadata.mode() & 0o777 == 0o700
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{label} must be owner-only mode 0700"),
        ))
    }
}

fn account_name() -> String {
    unsafe {
        let uid = libc::geteuid();
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0u8; 4096];
        if libc::getpwuid_r(
            uid,
            &mut pwd,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        ) == 0
            && !result.is_null()
            && !pwd.pw_name.is_null()
        {
            return CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned();
        }
        uid.to_string()
    }
}
