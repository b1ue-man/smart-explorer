use super::args::ApplyArgs;
#[cfg(any(windows, test))]
use super::hash::verify_sha256;
use std::path::Path;

pub(crate) fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<()> {
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

#[cfg(windows)]
pub(crate) fn relaunch_elevated(args: &ApplyArgs) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    validate_helper_for_elevation(&exe, args.helper_sha256.as_deref())?;
    let argv = elevated_argv(args);
    spawn_elevated_detached(&exe, &argv)
}

#[cfg(any(windows, test))]
pub(crate) fn elevated_argv(args: &ApplyArgs) -> Vec<String> {
    let mut argv = vec![
        "--apply".to_string(),
        "--target".to_string(),
        args.target.to_string_lossy().into_owned(),
        "--staged".to_string(),
        args.staged.to_string_lossy().into_owned(),
        "--parent-pid".to_string(),
        args.parent_pid.to_string(),
        "--version".to_string(),
        args.version.clone(),
        "--last-applied".to_string(),
        args.last_applied.to_string_lossy().into_owned(),
        "--error-file".to_string(),
        args.error_file.to_string_lossy().into_owned(),
        "--elevated".to_string(),
    ];
    if let Some(hash) = &args.staged_sha256 {
        argv.push("--staged-sha256".to_string());
        argv.push(hash.clone());
    }
    if let Some(hash) = &args.helper_sha256 {
        argv.push("--helper-sha256".to_string());
        argv.push(hash.clone());
    }
    argv
}

#[cfg(any(windows, test))]
pub(crate) fn validate_helper_for_elevation(
    exe: &Path,
    expected_sha256: Option<&str>,
) -> std::io::Result<()> {
    if let Some(expected) = expected_sha256 {
        verify_sha256(exe, expected).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Updater-Helfer vor UAC revalidieren: {e}"),
            )
        })?;
    }
    Ok(())
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
            staged: "staged.exe".into(),
            staged_sha256: Some(hash.to_string()),
            helper_sha256: Some(hash.to_string()),
            parent_pid: 42,
            version: "1.2.3".into(),
            last_applied: "last.txt".into(),
            error_file: "error.txt".into(),
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

        assert!(validate_helper_for_elevation(&path, Some(&expected)).is_err());

        let _ = std::fs::remove_file(path);
    }
}
