//! Programs the speed cases start (iperf3, fio, cp, rclone, curl, sftp,
//! robocopy …): time limit, captured output, a protocol file next to the
//! report, and "nicht ausgeführt" for a program that is not installed.
#![allow(dead_code)]

use crate::support::{one_line, var};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What a finished program printed and how long it ran.
pub struct Exec {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub wall: Duration,
    pub timed_out: bool,
}

impl Exec {
    pub fn ok(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }

    pub fn describe(&self) -> String {
        let how = if self.timed_out {
            "Zeitlimit überschritten".to_string()
        } else {
            format!("Exitcode {:?}", self.code)
        };
        format!(
            "{how}: {}",
            one_line(&self.stderr.chars().take(600).collect::<String>())
        )
    }
}

fn drain(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stream.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Runs a program with a time limit. `Err` is a spawn failure (the program is
/// missing); the output also goes to `<report>.tools.log`.
pub fn exec(
    program: &str,
    args: &[String],
    envs: &[(String, String)],
    timeout: Duration,
) -> std::io::Result<Exec> {
    let started = Instant::now();
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn()?;
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if started.elapsed() > timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let result = Exec {
        code: status.and_then(|status| status.code()),
        stdout: out
            .map(|handle| handle.join().unwrap_or_default())
            .unwrap_or_default(),
        stderr: err
            .map(|handle| handle.join().unwrap_or_default())
            .unwrap_or_default(),
        wall: started.elapsed(),
        timed_out,
    };
    log_tool(program, args, &result);
    Ok(result)
}

/// Hides the credentials of a URL argument (`scheme://user:secret@host`).
fn mask(arg: &str) -> String {
    if let (Some(scheme), Some(at)) = (arg.find("://"), arg.rfind('@')) {
        if at > scheme {
            return format!("{}***{}", &arg[..scheme + 3], &arg[at..]);
        }
    }
    arg.to_string()
}

fn log_tool(program: &str, args: &[String], result: &Exec) {
    let Some(report) = var("SE_SPEED_REPORT") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{report}.tools.log"))
    {
        let shown: Vec<String> = args.iter().map(|arg| mask(arg)).collect();
        let _ = writeln!(
            file,
            "$ {program} {} -> {:?} after {:.1}s\n{}{}",
            shown.join(" "),
            result.code,
            result.wall.as_secs_f64(),
            result.stdout.chars().take(4000).collect::<String>(),
            result.stderr.chars().take(2000).collect::<String>()
        );
    }
}

/// `exec` for a tool the case needs: a missing program is "nicht ausgeführt".
pub fn run_tool(
    program: &str,
    args: &[String],
    envs: &[(String, String)],
    timeout: Duration,
) -> Result<Exec, String> {
    exec(program, args, envs, timeout).map_err(|error| format!("{program} nicht startbar: {error}"))
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// The first output of `program args` (a version query), or why the program
/// is missing.
pub fn exec_missing(program: &str, args: &[String]) -> Result<String, String> {
    let done =
        exec(program, args, &[], Duration::from_secs(60)).map_err(|error| error.to_string())?;
    let text = if done.stdout.trim().is_empty() {
        done.stderr
    } else {
        done.stdout
    };
    Ok(text)
}

/// `sync; echo 3 > /proc/sys/vm/drop_caches` (M-4: before every local run).
pub fn drop_caches() -> Result<(), String> {
    let done = run_tool(
        "sudo",
        &strings(&["-n", "sh", "-c", "sync; echo 3 > /proc/sys/vm/drop_caches"]),
        &[],
        Duration::from_secs(120),
    )?;
    if done.ok() {
        Ok(())
    } else {
        Err(format!("Seitencache nicht leerbar: {}", done.describe()))
    }
}

/// `syncfs` on the file system of `path` (M-4: the time of a local copy ends
/// after it; Smart Explorer itself does not sync new copies).
#[cfg(target_os = "linux")]
pub fn syncfs(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    // SAFETY: the descriptor belongs to the open `file` for the whole call.
    let result = unsafe { libc::syncfs(file.as_raw_fd()) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!("syncfs: {}", std::io::Error::last_os_error()))
    }
}
