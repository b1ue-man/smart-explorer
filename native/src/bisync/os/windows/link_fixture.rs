use std::path::Path;

pub(crate) fn directory(target: &Path, link: &Path) {
    assert!(std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
        || std::env::var("SMART_EXPLORER_REMOTE_RUNNER").as_deref() == Ok("1"));
    // The working directory carries spaces and '&' without shell interpolation.
    let created = std::process::Command::new("cmd.exe")
        .current_dir(link.parent().unwrap())
        .args(["/d", "/c", "mklink", "/J"])
        .arg(link.file_name().unwrap())
        .arg(target)
        .output().unwrap();
    assert!(created.status.success(), "junction creation: {} {}",
        String::from_utf8_lossy(&created.stdout), String::from_utf8_lossy(&created.stderr));
}

pub(crate) fn remove_directory(link: &Path) { std::fs::remove_dir(link).unwrap(); }
