//! Pure classification of literal Share names and path text.

pub(super) fn private_name(name: &str) -> bool {
    let name = name.split(':').next().unwrap_or(name).trim_end_matches(['.', ' ']).to_ascii_lowercase();
    name == ".se-versions" || name.strip_prefix(".held.se-recycle-")
        .is_some_and(|suffix| suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub(super) fn private_path(path: &str) -> bool {
    path.split(['/', '\\']).any(private_name)
}

pub(super) fn within(path: &str, root: &str) -> bool {
    path == root || path.strip_prefix(root.trim_end_matches('/')).is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn normalized(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix("//?/").unwrap_or(&path);
    let path = path.strip_prefix("UNC/").map_or_else(|| path.to_owned(), |tail| format!("//{tail}"));
    path.trim_end_matches('/').to_ascii_lowercase()
}

pub(super) fn system_write(path: &str) -> bool {
    let path = normalized(path);
    let parts: Vec<_> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| matches!(*p, ".ssh" | ".gnupg" | ".aws" | ".kube")) { return true; }
    if parts.last().is_some_and(|p| matches!(*p, ".profile" | ".bashrc" | ".bash_profile" |
        ".bash_login" | ".bash_logout" | ".zshrc" | ".zshenv" | ".zprofile" | ".zlogin" | ".zlogout"
        | ".login" | ".cshrc" | ".pam_environment")) { return true; }
    if ["/etc", "/usr", "/bin", "/sbin", "/boot", "/lib", "/lib64", "/var/lib/systemd"]
        .iter().any(|root| within(&path, root)) { return true; }
    ["/.config/autostart", "/.config/systemd", "/.local/share/systemd", "/.config/fish", "/.config/powershell",
        "/microsoft/windows/start menu/programs/startup", "/microsoft/crypto", "/microsoft/credentials",
        "/documents/windowspowershell", "/documents/powershell"]
        .iter().any(|marker| path.ends_with(marker) || path.contains(&format!("{marker}/")))
        || parts.get(1).is_some_and(|p| parts[0].ends_with(':') && matches!(*p, "windows" | "program files" | "program files (x86)"))
}

#[cfg(test)]
#[path = "fs_policy_task_tests.rs"]
mod task_tests;
