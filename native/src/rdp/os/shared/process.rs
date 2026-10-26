//! Bounded read-only tool discovery/probes. No shell, stdin or credential output.
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: u64 = 128 * 1024;

pub(crate) fn capture(path: &Path, args: &[&str]) -> Result<String, String> {
    capture_with_env(path, args, &[])
}
pub(crate) fn capture_with_env(
    path: &Path,
    args: &[&str],
    environment: &[(&str, &str)],
) -> Result<String, String> {
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("LC_ALL", "C")
        .env("LANG", "C");
    command.envs(environment.iter().copied());
    super::clients::hide_probe_console(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot execute {}: {e}", path.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Probe stdout is unavailable".to_string())?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("RDP inventory command exceeded ten seconds".into());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("RDP inventory wait: {error}"));
            }
        }
    }?;
    let bytes = reader
        .join()
        .map_err(|_| "RDP inventory output reader failed".to_string())?
        .map_err(|e| format!("RDP inventory output: {e}"))?;
    if !status.success() || bytes.len() as u64 > OUTPUT_LIMIT {
        return Err(format!(
            "RDP inventory command failed or exceeded its output limit: {}",
            path.display()
        ));
    }
    String::from_utf8(bytes).map_err(|_| "RDP inventory output is not UTF-8".into())
}

pub(crate) fn find_tool(names: &[&str]) -> Option<PathBuf> {
    // Explicit OS installation directories; never search the working directory
    // or an untrusted per-project PATH for a program to launch.
    names
        .iter()
        .flat_map(|name| {
            ["/usr/bin", "/usr/local/bin", "/bin", "/usr/sbin"]
                .map(|directory| Path::new(directory).join(name))
        })
        .find(|path| path.is_file())
}
pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > OUTPUT_LIMIT {
        return Err(format!("{} exceeds inventory limit", path.display()));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8", path.display()))
}
