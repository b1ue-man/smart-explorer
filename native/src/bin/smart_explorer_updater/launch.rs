use super::args::ApplyArgs;
use super::hash::verify_sha256;
use std::path::Path;

pub(crate) fn spawn_verified_detached(
    exe: &Path,
    expected_sha256: &str,
    args: &[&str],
) -> std::io::Result<()> {
    validate_before_spawn(exe, expected_sha256)?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
        if cmd.spawn().is_ok() {
            return Ok(());
        }
        validate_before_spawn(exe, expected_sha256)?;
        let mut retry = std::process::Command::new(exe);
        retry
            .args(args)
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
        retry.spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        cmd.spawn().map(|_| ())
    }
}

fn validate_before_spawn(exe: &Path, expected_sha256: &str) -> std::io::Result<()> {
    verify_sha256(exe, expected_sha256).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Programmdatei vor Start revalidieren: {error}"),
        )
    })
}

#[cfg(windows)]
pub(crate) fn relaunch_elevated(args: &ApplyArgs) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    validate_helper_for_elevation(&exe, &args.helper_sha256)?;
    let argv = elevated_argv(args);
    spawn_elevated_detached(&exe, &argv)
}

#[cfg(any(windows, test))]
pub(crate) fn elevated_argv(args: &ApplyArgs) -> Vec<String> {
    let argv = vec![
        "--apply".to_string(),
        "--target".to_string(),
        args.target.to_string_lossy().into_owned(),
        "--target-sha256".to_string(),
        args.target_sha256.clone(),
        "--staged".to_string(),
        args.staged.to_string_lossy().into_owned(),
        "--staged-sha256".to_string(),
        args.staged_sha256.clone(),
        "--helper-target".to_string(),
        args.helper_target.to_string_lossy().into_owned(),
        "--helper-sha256".to_string(),
        args.helper_sha256.clone(),
        "--cli-staged".to_string(),
        args.cli_staged.to_string_lossy().into_owned(),
        "--cli-target".to_string(),
        args.cli_target.to_string_lossy().into_owned(),
        "--cli-sha256".to_string(),
        args.cli_sha256.clone(),
        "--archive".to_string(),
        args.archive.to_string_lossy().into_owned(),
        "--parent-pid".to_string(),
        args.parent_pid.to_string(),
        "--version".to_string(),
        args.version.clone(),
        "--last-applied".to_string(),
        args.last_applied.to_string_lossy().into_owned(),
        "--error-file".to_string(),
        args.error_file.to_string_lossy().into_owned(),
        "--manifest".to_string(),
        args.manifest.to_string_lossy().into_owned(),
        "--pin-file".to_string(),
        args.pin_file.to_string_lossy().into_owned(),
        "--elevated".to_string(),
    ];
    argv
}

#[cfg(any(windows, test))]
pub(crate) fn validate_helper_for_elevation(
    exe: &Path,
    expected_sha256: &str,
) -> std::io::Result<()> {
    verify_sha256(exe, expected_sha256).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Updater-Helfer vor UAC revalidieren: {e}"),
        )
    })
}

#[cfg(not(windows))]
pub(crate) fn relaunch_elevated(_args: &ApplyArgs) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "elevation is only supported on Windows",
    ))
}

#[cfg(windows)]
fn spawn_elevated_detached(exe: &Path, args: &[String]) -> std::io::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide_os(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }
    fn wide_str(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let verb = wide_str("runas");
    let file = wide_os(exe.as_os_str());
    let params = wide_str(&join_windows_args(args));
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if rc > 32 {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("Administratorfreigabe abgebrochen oder verweigert (ShellExecuteW={rc})"),
        ))
    }
}

#[cfg(windows)]
fn join_windows_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty()
        && !arg
            .chars()
            .any(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '"'))
    {
        return arg.to_string();
    }

    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "smart-explorer-updater-launch-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn args_with_hashes(hash: &str) -> ApplyArgs {
        ApplyArgs {
            target: "target.exe".into(),
            target_sha256: hash.to_string(),
            staged: "staged.exe".into(),
            staged_sha256: hash.to_string(),
            helper_target: "helper-target.exe".into(),
            helper_sha256: hash.to_string(),
            cli_staged: "cli-staged.exe".into(),
            cli_target: "cli-target.exe".into(),
            cli_sha256: hash.to_string(),
            archive: "archive.exe".into(),
            parent_pid: 42,
            version: "1.2.3".into(),
            last_applied: "last.txt".into(),
            error_file: "error.txt".into(),
            manifest: "manifest.json".into(),
            pin_file: "pin.txt".into(),
            elevated: false,
        }
    }

    #[test]
    fn elevated_argv_carries_staged_and_helper_hashes() {
        let hash = "b".repeat(64);
        let argv = elevated_argv(&args_with_hashes(&hash));

        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--staged-sha256" && pair[1] == hash));
        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--helper-sha256" && pair[1] == hash));
    }

    #[test]
    fn helper_hash_guard_rejects_same_size_tamper_before_elevation() {
        let path = unique_temp_file("helper");
        std::fs::write(&path, b"good").unwrap();
        let expected = super::super::hash::sha256_file(&path).unwrap();
        std::fs::write(&path, b"evil").unwrap();

        assert!(validate_helper_for_elevation(&path, &expected).is_err());

        let _ = std::fs::remove_file(path);
    }
}
