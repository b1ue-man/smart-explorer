//! Finding and running the agent hosts' own command-line tools (`claude`,
//! `codex`). Installation goes through their documented commands instead of
//! editing their configuration files.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The first `name` on PATH; on Windows also `name.exe`, `.cmd` and `.bat`
/// (npm installs `.cmd` shims, native installers `.exe`).
pub(super) fn find(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let candidates: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|extension| format!("{name}.{extension}"))
            .collect()
    } else {
        vec![name.to_string()]
    };
    std::env::split_paths(&path)
        .flat_map(|directory| candidates.iter().map(move |file| directory.join(file)))
        .find(|candidate| candidate.is_file())
}

pub(super) struct Output {
    pub(super) success: bool,
    pub(super) code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl Output {
    /// One readable line for reports.
    pub(super) fn detail(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            self.stdout.trim()
        } else {
            self.stderr.trim()
        };
        let text: String = text.chars().take(600).collect();
        match self.code {
            Some(code) => format!("exit {code}: {text}"),
            None => format!("terminated: {text}"),
        }
    }
}

/// Runs a host command without a terminal (standard input closed).
pub(super) fn run(program: &Path, args: &[&str]) -> Result<Output, String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run {}: {error}", program.display()))?;
    Ok(Output {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}
