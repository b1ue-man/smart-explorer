use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::fs::ShareExportConfig;
use super::types::{ExecRequest, ExecResult};

pub(crate) fn run(req: ExecRequest, cfg: &ShareExportConfig) -> io::Result<ExecResult> {
    if !cfg.allow_exec {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "remote execution is not enabled for this peer",
        ));
    }
    if req.shell && !cfg.allow_shell_exec {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "remote shell execution is not enabled for this peer",
        ));
    }
    if req.argv.is_empty() || req.argv[0].trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty remote command",
        ));
    }

    let mut cmd = command_for(&req);
    if let Some(cwd) = req.cwd.as_deref().filter(|s| !s.trim().is_empty()) {
        cmd.current_dir(cwd);
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout pipe unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr pipe unavailable"))?;
    let max = req.max_output_bytes.max(1) as usize;
    let out_handle = thread::spawn(move || read_limited(stdout, max));
    let err_handle = thread::spawn(move || read_limited(stderr, max));

    let deadline = Instant::now() + Duration::from_millis(req.timeout_ms.max(1));
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break child.wait().ok();
        }
        thread::sleep(Duration::from_millis(25));
    };

    let (stdout, stdout_truncated) = out_handle
        .join()
        .unwrap_or_else(|_| Ok((Vec::new(), true)))?;
    let (stderr, stderr_truncated) = err_handle
        .join()
        .unwrap_or_else(|_| Ok((Vec::new(), true)))?;

    Ok(ExecResult {
        stdout,
        stderr,
        exit_code: status.and_then(|s| s.code()),
        timed_out,
        stdout_truncated,
        stderr_truncated,
    })
}

fn command_for(req: &ExecRequest) -> Command {
    if req.shell {
        shell_command(&req.argv[0])
    } else {
        let mut cmd = Command::new(&req.argv[0]);
        cmd.args(&req.argv[1..]);
        cmd
    }
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", command]);
    cmd
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.args(["-lc", command]);
    cmd
}

fn read_limited(mut reader: impl Read, max: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let remaining = max.saturating_sub(out.len());
        if remaining > 0 {
            out.extend_from_slice(&buf[..n.min(remaining)]);
        }
        if n > remaining {
            truncated = true;
        }
    }
    Ok((out, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ExecRequest {
        ExecRequest {
            argv: vec!["rustc".into(), "--version".into()],
            cwd: None,
            timeout_ms: 10_000,
            max_output_bytes: 16,
            shell: false,
        }
    }

    #[test]
    fn exec_is_denied_by_default() {
        let err = run(req(), &ShareExportConfig::default()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn shell_requires_separate_permission() {
        let mut cfg = ShareExportConfig {
            allow_exec: true,
            ..Default::default()
        };
        let mut r = req();
        r.shell = true;
        r.argv = vec!["echo hi".into()];
        let err = run(r.clone(), &cfg).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        cfg.allow_shell_exec = true;
        let _ = run(r, &cfg);
    }

    #[test]
    fn argv_execution_returns_status_and_stdout() {
        let cfg = ShareExportConfig {
            allow_exec: true,
            ..Default::default()
        };
        let mut r = req();
        r.max_output_bytes = 1024;
        let result = run(r, &cfg).unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(String::from_utf8_lossy(&result.stdout).contains("rustc"));
        assert!(!result.timed_out);
    }

    #[test]
    fn output_is_truncated_but_command_completes() {
        let cfg = ShareExportConfig {
            allow_exec: true,
            ..Default::default()
        };
        let mut r = req();
        r.max_output_bytes = 4;
        let result = run(r, &cfg).unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout.len(), 4);
        assert!(result.stdout_truncated);
    }

    #[test]
    fn timeout_kills_command_and_marks_result() {
        let cfg = ShareExportConfig {
            allow_exec: true,
            ..Default::default()
        };
        let mut r = sleep_req();
        r.timeout_ms = 50;
        let result = run(r, &cfg).unwrap();
        assert!(result.timed_out);
    }

    #[cfg(windows)]
    fn sleep_req() -> ExecRequest {
        ExecRequest {
            argv: vec![
                "powershell".into(),
                "-NoProfile".into(),
                "-Command".into(),
                "Start-Sleep -Milliseconds 500".into(),
            ],
            cwd: None,
            timeout_ms: 10_000,
            max_output_bytes: 16,
            shell: false,
        }
    }

    #[cfg(not(windows))]
    fn sleep_req() -> ExecRequest {
        ExecRequest {
            argv: vec!["sh".into(), "-c".into(), "sleep 1".into()],
            cwd: None,
            timeout_ms: 10_000,
            max_output_bytes: 16,
            shell: false,
        }
    }
}
