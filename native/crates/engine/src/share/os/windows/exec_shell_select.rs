//! The shell of a shell command on a Windows host (`share/core/exec_shell.rs`):
//! PowerShell 7 (`pwsh.exe`) when the job's `PATH` or
//! `%ProgramFiles%\PowerShell\7` has it, else Windows PowerShell 5.1, which
//! every supported Windows has; `cmd.exe` on request.
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::share::exec_shell::{self, PWSH_LABEL, WINDOWS_POWERSHELL_LABEL};
use crate::share::exec_types::ExecStart;

/// Decides the shell of `request` (job environment `environment`) and
/// returns its result name; a PowerShell command becomes the argv start of
/// the PowerShell found.
pub fn resolve_shell(
    request: &mut ExecStart,
    environment: &BTreeMap<String, String>,
) -> io::Result<Option<String>> {
    exec_shell::windows(request, || powershell_program(environment))
}

/// Path and result name of the PowerShell a job runs in. The job's `PATH`
/// comes first (an installation from the Microsoft Store is an app execution
/// alias there), then the machine-wide installation folder of PowerShell 7,
/// which a `PATH` captured before its installation lacks.
fn powershell_program(environment: &BTreeMap<String, String>) -> (String, &'static str) {
    let search_path = environment
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.as_str())
        .unwrap_or("");
    let mut candidates: Vec<PathBuf> = std::env::split_paths(search_path)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("pwsh.exe"))
        .collect();
    for variable in ["ProgramFiles", "ProgramW6432"] {
        if let Some(root) = std::env::var_os(variable).filter(|root| !root.is_empty()) {
            candidates.push(PathBuf::from(root).join(r"PowerShell\7\pwsh.exe"));
        }
    }
    let found = candidates
        .iter()
        .filter(|candidate| present(candidate))
        .find_map(|candidate| candidate.to_str());
    if let Some(pwsh) = found {
        return (pwsh.to_string(), PWSH_LABEL);
    }
    let root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let windows_powershell = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    (
        windows_powershell.to_string_lossy().into_owned(),
        WINDOWS_POWERSHELL_LABEL,
    )
}

/// A file, or an app execution alias (a reparse point that `metadata`
/// cannot follow, but `CreateProcessW` starts).
fn present(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_dir())
}
