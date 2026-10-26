//! fexecve binds execution to the descriptor that was actually hashed.
use crate::mcp_launcher::GenerationChild;
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
pub(in crate::mcp_launcher) type NativeChild = Child;
pub(in crate::mcp_launcher) fn request_stop(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, libc::SIGINT) == 0 }
}
use std::time::Duration;

pub(in crate::mcp_launcher) fn spawn_verified(
    command: Command,
    image: File,
    lifecycle: bool,
    capture: bool,
    input: bool,
) -> Result<GenerationChild, String> {
    spawn_impl(command, image, lifecycle, capture, input, true)
}
fn spawn_impl(
    mut command: Command,
    image: File,
    lifecycle: bool,
    capture: bool,
    input: bool,
    wait_ack: bool,
) -> Result<GenerationChild, String> {
    if capture {
        command
            .stdin(if input { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }
    let (mut parent, child_channel) = UnixStream::pair().map_err(|e| e.to_string())?;
    let channel_fd = child_channel.as_raw_fd();
    let image_fd = image.as_raw_fd();
    if wait_ack {
        command.env("SE_MCP_ADMISSION_CHANNEL", channel_fd.to_string());
        command.env("SE_MCP_LIFECYCLE", if lifecycle { "1" } else { "0" });
    } else {
        command
            .env_remove("SE_MCP_ADMISSION_CHANNEL")
            .env_remove("SE_MCP_LIFECYCLE")
            .env_remove("SE_MCP_LIFECYCLE_HANDLE")
            .env_remove("SE_MCP_MAINTENANCE");
    }
    let mut argv =
        vec![CString::new(command.get_program().as_encoded_bytes()).map_err(|e| e.to_string())?];
    for argument in command.get_args() {
        argv.push(CString::new(argument.as_encoded_bytes()).map_err(|e| e.to_string())?);
    }
    let mut environment = std::env::vars_os().collect::<std::collections::BTreeMap<_, _>>();
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            environment.insert(key.into(), value.into());
        } else {
            environment.remove(key);
        }
    }
    let environment = environment
        .into_iter()
        .map(|(key, value)| {
            let mut item = key.as_encoded_bytes().to_vec();
            item.push(b'=');
            item.extend_from_slice(value.as_encoded_bytes());
            CString::new(item).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let exec = FrozenExec::new(image_fd, argv, environment);
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(channel_fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            exec.execute();
            Err(std::io::Error::last_os_error())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("runtime_fexecve_failed: {e}"))?;
    drop(image);
    drop(child_channel);
    if !wait_ack {
        return Ok(wrap(child, None));
    }
    parent
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    let mut ready = [0; 1];
    if let Err(error) = parent.read_exact(&mut ready) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("runtime_admission_not_acknowledged: {error}"));
    }
    if ready != [1] {
        let _ = child.kill();
        let _ = child.wait();
        return Err("runtime_admission_rejected".into());
    }
    if command
        .get_envs()
        .any(|(key, value)| key == "SE_MCP_MAINTENANCE" && value == Some(std::ffi::OsStr::new("1")))
    {
        parent.write_all(&[2]).map_err(|e| e.to_string())?;
    }
    parent.set_read_timeout(None).map_err(|e| e.to_string())?;
    let channel = if lifecycle {
        Some(unsafe { File::from_raw_fd(parent.into_raw_fd()) })
    } else {
        None
    };
    Ok(wrap(child, channel))
}

pub(in crate::mcp_launcher) fn take_admission_channel(
) -> Result<Option<(File, bool, Option<File>)>, String> {
    let Some(raw) = std::env::var_os("SE_MCP_ADMISSION_CHANNEL") else {
        return Ok(None);
    };
    let fd = raw
        .to_str()
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|fd| *fd > 2)
        .ok_or("runtime_admission_channel_invalid")?;
    std::env::remove_var("SE_MCP_ADMISSION_CHANNEL");
    let lifecycle =
        std::env::var_os("SE_MCP_LIFECYCLE").as_deref() == Some(std::ffi::OsStr::new("1"));
    std::env::remove_var("SE_MCP_LIFECYCLE");
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(format!(
            "runtime_channel_cloexec: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(Some((unsafe { File::from_raw_fd(fd) }, lifecycle, None)))
}

pub(in crate::mcp_launcher) fn acknowledge(channel: &mut File) -> Result<(), String> {
    channel
        .write_all(&[1])
        .map_err(|e| format!("runtime_admission_ack: {e}"))
}

fn wrap(mut process: Child, lifecycle: Option<File>) -> GenerationChild {
    GenerationChild {
        stdin: process
            .stdin
            .take()
            .map(|s| Box::new(s) as Box<dyn Write + Send>),
        stdout: process
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        stderr: process
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        process,
        _lifecycle: lifecycle,
    }
}
pub(in crate::mcp_launcher) fn spawn_legacy(
    mut command: Command,
    input: bool,
) -> Result<GenerationChild, String> {
    command
        .stdin(if input { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
        .spawn()
        .map(|child| wrap(child, None))
        .map_err(|e| e.to_string())
}

pub(in crate::mcp_launcher) fn run_stable_launcher(
    command: Command,
    image: File,
) -> Result<i32, String> {
    let mut child = spawn_impl(command, image, false, false, false, false)?;
    child
        .wait()
        .map(|s| s.code().unwrap_or(1))
        .map_err(|e| e.to_string())
}

// CString storage and pointer arrays are frozen before fork. The child path
// performs only fcntl/fexecve; no allocator, lock or environment access.
struct FrozenExec {
    fd: i32,
    _argv: Vec<CString>,
    _env: Vec<CString>,
    argv: Vec<*const libc::c_char>,
    env: Vec<*const libc::c_char>,
}
unsafe impl Send for FrozenExec {}
unsafe impl Sync for FrozenExec {}
impl FrozenExec {
    fn new(fd: i32, argv: Vec<CString>, env: Vec<CString>) -> Self {
        let args = argv
            .iter()
            .map(|v| v.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        let vars = env
            .iter()
            .map(|v| v.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        Self {
            fd,
            _argv: argv,
            _env: env,
            argv: args,
            env: vars,
        }
    }
    unsafe fn execute(&self) {
        libc::fexecve(self.fd, self.argv.as_ptr(), self.env.as_ptr());
    }
}
pub(in crate::mcp_launcher) fn spawn_current_daemon(
    command: Command,
    image: File,
) -> Result<(), String> {
    let _child = spawn_impl(command, image, false, true, false, false)?;
    Ok(())
}

pub(in crate::mcp_launcher) fn spawn_installation_probe(
    command: Command,
    image: File,
) -> Result<GenerationChild, String> {
    spawn_impl(command, image, false, true, false, false)
}
