//! Linux: root through pkexec (the polkit agent of the session asks the user)
//! or, without an agent, `sudo -n`; the helper `se elevated exec` talks over
//! its standard input and output. Process trees are process groups.
//!
//! pkexec runs with `--disable-internal-agent`, so it never asks on a
//! terminal the agent host owns; its minimal environment and its working
//! directory (root's home) do not matter, because the helper sets both from
//! its arguments. The caller may signal pkexec until it started the helper
//! (pkexec keeps the caller's real uid until then); afterwards only the
//! cancel frame or the end of the helper's input stops the command.
use std::io::{self, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::child::ElevatedChild;
use super::framed::{self, Launched};
use super::helper_args::{self, HelperCall};
use super::outcome::{self, Launcher, Verdict};
use super::types::{ElevateError, ElevatedCommand, ElevatedSpec, Elevation};

const PKEXEC: &str = "/usr/bin/pkexec";
const SUDO_CANDIDATES: [&str; 2] = ["/usr/bin/sudo", "/bin/sudo"];
/// Time between TERM and KILL when a tree is cancelled.
const KILL_AFTER: Duration = Duration::from_secs(10);
/// Error output of a launcher kept for its message.
const MAX_LAUNCHER_ERROR: usize = 16 * 1024;

pub fn is_elevated() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// The se program that runs the helper: this program when it is `se`, else
/// the `se` beside it (the desktop app and the updater have no `se elevated`).
fn se_program() -> Result<PathBuf, ElevateError> {
    let current = std::env::current_exe()
        .map_err(|error| ElevateError::Failed(format!("cannot find this program: {error}")))?;
    let se = if current.file_name().is_some_and(|name| name == "se") {
        current
    } else {
        current.with_file_name("se")
    };
    if !se.is_file() {
        return Err(ElevateError::Failed(format!(
            "{} is missing; the elevated helper is part of the se program",
            se.display()
        )));
    }
    Ok(se)
}

fn image_hash(path: &Path) -> Result<String, ElevateError> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|error| {
        ElevateError::Failed(format!("cannot read {}: {error}", path.display()))
    })?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| ElevateError::Failed(error.to_string()))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn spawn_elevated(spec: ElevatedSpec) -> Result<ElevatedChild, ElevateError> {
    if matches!(spec.command, ElevatedCommand::CmdLine(_)) {
        return Err(ElevateError::Failed(
            "cmd.exe lines run only on Windows".into(),
        ));
    }
    let se = se_program()?;
    let call = HelperCall {
        spec,
        pipe: None,
        image_hash: Some(image_hash(&se)?),
    };
    let args = helper_args::build("exec", &call);
    let mut why_not_polkit = format!("{PKEXEC} is not installed");
    if Path::new(PKEXEC).is_file() {
        let mut command = Command::new(PKEXEC);
        command
            .arg("--disable-internal-agent")
            .arg(&se)
            .arg("elevated")
            .args(&args);
        match launch(command, Launcher::Pkexec, Elevation::Polkit)? {
            Ok(child) => return Ok(child),
            Err(why) => why_not_polkit = why,
        }
    }
    let Some(sudo) = SUDO_CANDIDATES
        .iter()
        .find(|path| Path::new(path).is_file())
    else {
        return Err(ElevateError::NoAgent(format!(
            "{why_not_polkit}, and sudo is not installed"
        )));
    };
    let mut command = Command::new(sudo);
    command
        .args(["-n", "--"])
        .arg(&se)
        .arg("elevated")
        .args(&args);
    match launch(
        command,
        Launcher::SudoNonInteractive,
        Elevation::SudoNoPassword,
    )? {
        Ok(child) => Ok(child),
        Err(why) => Err(ElevateError::NoAgent(format!("{why_not_polkit}; {why}"))),
    }
}

/// Starts a launcher; `Ok(Err(why))` when the next way may still work.
fn launch(
    mut command: Command,
    launcher: Launcher,
    via: Elevation,
) -> Result<Result<ElevatedChild, String>, ElevateError> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| ElevateError::Failed(format!("cannot start the launcher: {error}")))?;
    let (Some(stdin), Some(mut stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ElevateError::Failed("the launcher has no pipes".into()));
    };
    let errors = collect(stderr);
    if !framed::await_ready(&mut stdout) {
        let status = child.wait().ok().and_then(|status| status.code());
        let text = errors.text();
        return match outcome::classify(launcher, status, &text) {
            Verdict::TryNext(why) => Ok(Err(why)),
            Verdict::Stop(error) => Err(error),
        };
    }
    let pid = child.id();
    let child = Arc::new(Mutex::new(Some(child)));
    let reaped = child.clone();
    Ok(Ok(framed::start(
        Launched {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            abort: Box::new(move || {
                // Allowed only while the launcher still has our real uid;
                // refused (EPERM) once the helper runs as root.
                // SAFETY: kill with a positive pid signals that one process.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
            }),
            reap: Box::new(move || {
                if let Ok(mut slot) = reaped.lock() {
                    if let Some(mut child) = slot.take() {
                        let _ = child.wait();
                    }
                }
            }),
        },
        via,
    )))
}

/// Error output of a launcher, read on its own thread.
struct Collected(Arc<Mutex<Vec<u8>>>, Option<std::thread::JoinHandle<()>>);

impl Collected {
    fn text(mut self) -> String {
        if let Some(reader) = self.1.take() {
            let _ = reader.join();
        }
        let bytes = self.0.lock().map(|bytes| bytes.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

fn collect(mut stderr: impl Read + Send + 'static) -> Collected {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = bytes.clone();
    let reader = std::thread::Builder::new()
        .name("se-elevate-launcher-err".into())
        .spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stderr.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                if let Ok(mut kept) = sink.lock() {
                    let room = MAX_LAUNCHER_ERROR.saturating_sub(kept.len());
                    kept.extend_from_slice(&buffer[..read.min(room)]);
                }
            }
        })
        .ok();
    Collected(bytes, reader)
}

pub fn confirm_user_presence(reason: &str) -> Result<(), ElevateError> {
    if !Path::new(PKEXEC).is_file() {
        return Err(ElevateError::NoAgent(format!(
            "{PKEXEC} is not installed; the confirmation needs the polkit dialog"
        )));
    }
    let se = se_program()?;
    let output = Command::new(PKEXEC)
        .arg("--disable-internal-agent")
        .arg(&se)
        .args(["elevated", "confirm", "--reason", reason])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| ElevateError::Failed(format!("pkexec: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    match outcome::classify(Launcher::Pkexec, output.status.code(), &text) {
        Verdict::TryNext(why) => Err(ElevateError::NoAgent(why)),
        Verdict::Stop(error) => Err(error),
    }
}

pub fn ask_password(
    _caption: &str,
    _message: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, ElevateError> {
    Err(ElevateError::Failed(
        "the credential dialog of this module is Windows-only; se asks in its own window".into(),
    ))
}

/// Linux helper channel: standard input and output (pkexec and sudo hand
/// them on). The program's hash is compared with the requested one.
#[allow(clippy::type_complexity)]
pub fn helper_transport(
    call: &HelperCall,
) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>), String> {
    if call.pipe.is_some() {
        return Err("se elevated: named pipes are Windows-only".into());
    }
    if let Some(expected) = &call.image_hash {
        let own = std::env::current_exe().map_err(|error| error.to_string())?;
        let actual = image_hash(&own).map_err(|error| error.to_string())?;
        if &actual != expected {
            return Err("se elevated: this program is not the one the caller asked for".into());
        }
    }
    Ok((Box::new(io::stdin()), Box::new(io::stdout())))
}

pub fn direct_command(command: &ElevatedCommand) -> io::Result<Command> {
    match command {
        ElevatedCommand::Argv { program, args } => {
            let mut command = Command::new(program);
            command.args(args).process_group(0);
            Ok(command)
        }
        ElevatedCommand::CmdLine(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cmd.exe lines run only on Windows",
        )),
    }
}

pub fn exit_signal(status: &ExitStatus) -> Option<i32> {
    status.signal()
}

/// The process group of a started command (`process_group(0)`: its id is
/// the first process's pid).
pub struct Tree {
    group: libc::pid_t,
}

impl Tree {
    pub fn adopt(child: &Child) -> io::Result<Self> {
        let group = libc::pid_t::try_from(child.id())
            .map_err(|_| io::Error::other("process id out of range"))?;
        Ok(Self { group })
    }

    /// TERM to the group, KILL after `KILL_AFTER`.
    pub fn terminate(&self) {
        signal_group(self.group, libc::SIGTERM);
        let group = self.group;
        let _ = std::thread::Builder::new()
            .name("se-elevate-kill".into())
            .spawn(move || {
                std::thread::sleep(KILL_AFTER);
                signal_group(group, libc::SIGKILL);
            });
    }

    pub fn kill(&self) {
        signal_group(self.group, libc::SIGKILL);
    }
}

fn signal_group(group: libc::pid_t, signal: libc::c_int) {
    if group > 0 {
        // SAFETY: a negative pid addresses the process group `group`; an
        // ended group answers ESRCH, which is ignored.
        unsafe {
            libc::kill(-group, signal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{spawn_direct, ElevatedCommand, ElevatedEvent, ElevatedSpec, Elevation};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    fn sh(script: &str) -> ElevatedSpec {
        let mut env = BTreeMap::new();
        env.insert("SE_ELEVATE_TEST".to_string(), "value".to_string());
        ElevatedSpec {
            command: ElevatedCommand::Argv {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
            },
            cwd: Some("/".into()),
            env,
        }
    }

    fn drain(child: &mut super::ElevatedChild) -> (Vec<u8>, Vec<u8>, super::super::ElevatedExit) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        while let Some(event) = child.next_event() {
            match event {
                ElevatedEvent::Stdout(bytes) => out.extend(bytes),
                ElevatedEvent::Stderr(bytes) => err.extend(bytes),
                ElevatedEvent::Exited(exit) => return (out, err, exit),
            }
        }
        panic!("no end");
    }

    #[test]
    fn shell_elevate_task_direct_start_passes_streams_cwd_env_and_status() {
        let mut child = spawn_direct(sh(
            "read line; echo \"$line $SE_ELEVATE_TEST $(pwd)\"; echo e >&2; exit 3",
        ))
        .unwrap();
        assert_eq!(child.via(), Elevation::Direct);
        child.input().stdin(b"hello\n").unwrap();
        child.input().eof().unwrap();
        let (out, err, exit) = drain(&mut child);
        assert_eq!(String::from_utf8_lossy(&out), "hello value /\n");
        assert_eq!(String::from_utf8_lossy(&err), "e\n");
        assert_eq!(exit.code, Some(3));
        assert!(!exit.cancelled);
    }

    #[test]
    fn shell_elevate_task_cancel_and_root_end_stop_the_whole_group() {
        let started = Instant::now();
        let mut child = spawn_direct(sh("sleep 300 & echo started; wait")).unwrap();
        assert_eq!(
            child.next_event(),
            Some(ElevatedEvent::Stdout(b"started\n".to_vec()))
        );
        child.input().cancel();
        let (_, _, exit) = drain(&mut child);
        assert!(exit.cancelled);
        assert_eq!(exit.signal, Some(libc::SIGTERM));
        // A background process does not hold the command after its shell ended.
        let mut child = spawn_direct(sh("sleep 300 & echo done")).unwrap();
        let (out, _, exit) = drain(&mut child);
        assert_eq!(String::from_utf8_lossy(&out), "done\n");
        assert_eq!(exit.code, Some(0));
        assert!(started.elapsed() < Duration::from_secs(20));
    }
}
