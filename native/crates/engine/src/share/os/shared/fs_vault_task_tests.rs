//! Tests of the vault boundary of the Share host (E35) and of the names that
//! peers see for saved connections (R-FS-3). Bound from `fs_host_policy.rs`.
use std::io;
use std::path::{Path, PathBuf};

use super::{ensure_handle_allowed, in_vault_drive};
use crate::local_access::DirectoryHandle;
use crate::share::wire::FsResponse;

/// A vault drive published for one test; it is withdrawn when the test ends.
struct PublishedMount {
    site: &'static str,
    root: PathBuf,
}

impl PublishedMount {
    fn publish(site: &'static str, root: &Path) -> Self {
        crate::vault::publish_vault_mount(site, root);
        Self {
            site,
            root: root.to_path_buf(),
        }
    }
}

impl Drop for PublishedMount {
    fn drop(&mut self) {
        crate::vault::withdraw_vault_mount(self.site, &self.root);
    }
}

/// The Share host's answer for `path`: a handle on the object the path names,
/// checked as every handle of a Share operation is checked.
fn share_refusal(path: &Path) -> io::Result<()> {
    ensure_handle_allowed(&DirectoryHandle::open_root(path)?)
}

/// A new canonical folder `name` below a fresh temporary directory; the guard
/// keeps that directory alive until the test ends.
fn canonical_folder(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temporary directory");
    let folder = std::fs::canonicalize(dir.path())
        .expect("canonical temporary directory")
        .join(name);
    std::fs::create_dir_all(&folder).expect("test folder");
    (dir, folder)
}

/// A symbolic link to `target` inside `dir` (Unix only; file symbolic links
/// on Windows need a privilege, so the junction test covers that spelling).
#[cfg(unix)]
fn symlink_spelling(dir: &Path, target: &Path) -> Option<PathBuf> {
    let link = dir.join("verknuepfung");
    std::os::unix::fs::symlink(target, &link).expect("symbolic link");
    Some(link)
}

#[cfg(not(unix))]
fn symlink_spelling(_dir: &Path, _target: &Path) -> Option<PathBuf> {
    None
}

#[test]
fn fs_tresor_task_spellings_of_a_mounted_drive_are_private() {
    let (dir, drive) = canonical_folder("tresor-schreibweisen");
    let secret = drive.join("Geheim");
    std::fs::create_dir_all(&secret).expect("vault folder");
    let _mount = PublishedMount::publish("fs-tresor-task-mount-2", &drive);
    let mut spellings = vec![
        secret.clone(),
        PathBuf::from(format!("{}/", secret.display())),
        PathBuf::from(format!("{}/./", secret.display())),
        drive.join("Geheim").join("..").join("Geheim"),
    ];
    spellings.extend(symlink_spelling(dir.path(), &secret));
    for spelling in &spellings {
        let text = spelling.to_string_lossy().into_owned();
        assert!(in_vault_drive(&text), "private spelling: {text}");
        let error = share_refusal(spelling).expect_err("refused spelling");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{text}");
    }
}

#[test]
fn fs_tresor_task_sibling_with_shared_name_prefix_stays_shareable() {
    let (_dir, drive) = canonical_folder("tresor-nachbar");
    let sibling = drive.with_file_name("tresor-nachbar-offen");
    std::fs::create_dir_all(&sibling).expect("sibling folder");
    let _mount = PublishedMount::publish("fs-tresor-task-mount-3", &drive);
    let text = sibling.to_string_lossy().into_owned();
    assert!(
        !in_vault_drive(&text),
        "a name that only starts like the drive is no vault"
    );
    assert!(share_refusal(&sibling).is_ok());
}

/// A junction `link` to `target`, made by `mklink /J`, which needs no
/// administrator right. Dropping it removes the link and keeps the target.
#[cfg(windows)]
struct Junction(PathBuf);

#[cfg(windows)]
impl Junction {
    fn create(link: &Path, target: &Path) -> Self {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("cmd")
            .raw_arg(format!(
                r#"/C mklink /J "{}" "{}""#,
                link.display(),
                target.display()
            ))
            .status()
            .expect("command interpreter");
        assert!(status.success(), "junction {} created", link.display());
        Self(link.to_path_buf())
    }
}

#[cfg(windows)]
impl Drop for Junction {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// The 8.3 short name of `path` as the command interpreter's `%~s` gives it.
#[cfg(windows)]
fn short_path(path: &Path) -> String {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new("cmd")
        .raw_arg(format!(
            r#"/C for %I in ("{}") do @echo %~sI"#,
            path.display()
        ))
        .output()
        .expect("command interpreter");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[cfg(windows)]
#[test]
fn fs_tresor_task_windows_verbatim_and_case_spellings_are_private() {
    let (dir, drive) = canonical_folder("Tresor-Schreibweise");
    let secret = drive.join("Geheim");
    std::fs::create_dir_all(&secret).expect("vault folder");
    let _mount = PublishedMount::publish("fs-tresor-task-mount-w1", &drive);
    let plain = dir.path().join("Tresor-Schreibweise").join("Geheim");
    let plain_text = plain.to_string_lossy().into_owned();
    let spellings = [
        secret.to_string_lossy().into_owned(),
        plain_text.clone(),
        plain_text.to_uppercase(),
        plain_text.to_lowercase(),
        format!(r"\\?\{plain_text}"),
    ];
    for text in &spellings {
        assert!(in_vault_drive(text), "private spelling: {text}");
        let error = share_refusal(Path::new(text)).expect_err("refused spelling");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{text}");
    }
}

#[cfg(windows)]
#[test]
#[ignore = "NTFS-Verbindungspunkt; Suite crates-task"]
fn fs_tresor_task_windows_junction_into_vault_is_private() {
    let (dir, drive) = canonical_folder("tresor-verbindungspunkt");
    let secret = drive.join("Geheim-Ordner");
    std::fs::create_dir_all(&secret).expect("vault folder");
    let link = dir.path().join("verbindungspunkt");
    let _junction = Junction::create(&link, &drive);
    let _mount = PublishedMount::publish("fs-tresor-task-mount-w2", &drive);
    let through = link.join("Geheim-Ordner");
    let text = through.to_string_lossy().into_owned();
    assert!(
        in_vault_drive(&text),
        "private through the junction: {text}"
    );
    let error = share_refusal(&through).expect_err("refused through the junction");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[cfg(windows)]
#[test]
#[ignore = "8.3-Kurzname auf dem Volume nötig; Suite crates-task"]
fn fs_tresor_task_windows_short_name_is_private() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let drive = dir.path().join("tresor-kurzname-laufwerk");
    let secret = drive.join("geheim-ordner-lang");
    std::fs::create_dir_all(&secret).expect("vault folder");
    let short = short_path(&secret);
    assert_ne!(
        short,
        secret.display().to_string(),
        "the volume has 8.3 names"
    );
    let canonical_drive = std::fs::canonicalize(&drive).expect("canonical drive");
    let _mount = PublishedMount::publish("fs-tresor-task-mount-w3", &canonical_drive);
    assert!(
        in_vault_drive(&short),
        "private under the 8.3 name: {short}"
    );
    let error = share_refusal(Path::new(&short)).expect_err("refused under the 8.3 name");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

/// E35: the identity branch alone, without the spelling check, finds the
/// objects of a mounted drive by their nearest existing folder, leaves a
/// sibling out and leaves a path without any existing folder to the spelling
/// check.
#[cfg(windows)]
#[test]
fn fs_tresor_task_windows_identity_alone_decides() {
    let (_dir, drive) = canonical_folder("tresor-identitaet");
    let secret = drive.join("Geheim");
    std::fs::create_dir_all(&secret).expect("vault folder");
    let sibling = drive.with_file_name("tresor-identitaet-offen");
    std::fs::create_dir_all(&sibling).expect("sibling folder");
    let _mount = PublishedMount::publish("fs-tresor-task-mount-w4", &drive);
    assert!(super::identity_in_vault(&drive), "the drive root");
    assert!(
        super::identity_in_vault(&secret),
        "a folder below the drive"
    );
    assert!(
        super::identity_in_vault(&secret.join("noch-nicht-da.txt")),
        "a new file below the drive, by its folder"
    );
    assert!(
        !super::identity_in_vault(&sibling),
        "a sibling is no vault object"
    );
    let nowhere = Path::new("fs-tresor-task-ohne-ordner-5c1e").join("datei.txt");
    assert!(
        !super::identity_in_vault(&nowhere),
        "no existing folder names no vault object"
    );
}

/// L19 (E11, E35, E53): a listing of an export does not show an unlocked
/// vault drive when the export names its root in another spelling; the
/// identity of the entry decides, and an ordinary folder stays listed.
#[cfg(windows)]
#[test]
fn vault_task_windows_listing_hides_unlocked_drive_in_other_spelling() {
    use crate::share::{
        export_config::ExportAccess,
        fs::{ShareExportConfig, SharedRoot},
        fs_access::FsAccess,
        fs_host_policy_task_tests::text,
    };
    let (_dir, folder) = canonical_folder("Docs");
    let drive = folder.join("tresor");
    std::fs::create_dir_all(&drive).expect("vault drive folder");
    std::fs::create_dir_all(folder.join("offen")).expect("ordinary folder");
    let _mount = PublishedMount::publish("vault-task-listing-mount", &drive);
    // The export names its root in upper case and without the verbatim prefix.
    let spelled: String = folder
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_uppercase();
    let access = FsAccess::dynamic(ShareExportConfig {
        roots: vec![
            SharedRoot::new("Docs", text(Path::new(&spelled))).with_access(ExportAccess::ReadWrite)
        ],
        ..Default::default()
    });
    let (entries, _omitted) =
        crate::share::host_list::listing("/Docs", &access).expect("the export is listed");
    assert!(
        !entries.iter().any(|entry| entry.name == "tresor"),
        "the unlocked vault drive is listed"
    );
    assert!(
        entries.iter().any(|entry| entry.name == "offen"),
        "the ordinary folder is not listed"
    );
}

#[test]
fn fs_fehler_task_connection_root_is_named_by_virtual_path() {
    // The root of a saved connection reaches the peer as its virtual path,
    // with forward slashes and with backslashes alike (R-FS-3).
    let forward = "sftp://fehler-task-host.invalid/exporte/probe";
    crate::share::fs::note_connection_root(forward, "/Verbindungen/Fehlerprobe");
    let error = io::Error::other(format!("Verbindung {forward}/bericht.txt abgelehnt"));
    let FsResponse::Err { msg, .. } = crate::share::fs_error::response(&error) else {
        panic!("error response expected");
    };
    assert_eq!(
        msg,
        "Verbindung /Verbindungen/Fehlerprobe/bericht.txt abgelehnt"
    );

    let share = r"\\fehler-task-server\exporte";
    crate::share::fs::note_connection_root(share, "/Verbindungen/Fehlerprobe-Unc");
    let error = io::Error::other(format!(r"Freigabe {share}\bericht.txt nicht erreichbar"));
    let FsResponse::Err { msg, .. } = crate::share::fs_error::response(&error) else {
        panic!("error response expected");
    };
    assert_eq!(
        msg,
        r"Freigabe /Verbindungen/Fehlerprobe-Unc\bericht.txt nicht erreichbar"
    );
}
