//! Signals `desktop_task_linux_launch_decision_table`,
//! `desktop_task_linux_desktop_entry_trust_rules` (files with a mode, an
//! owner and attributes for each rule) and
//! `desktop_task_linux_desktop_dir_home_disables_files`.
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::desktop_entry_trust::*;
use super::desktop_folder::{folders_of, resolve, Resolved};

/// Facts per path, set by the check.
#[derive(Default)]
struct Fake {
    files: HashMap<PathBuf, (u32, u32)>,
    executable: Vec<PathBuf>,
    attributes: HashMap<(PathBuf, String), String>,
    content: HashMap<PathBuf, Vec<u8>>,
    no_gio: bool,
}

impl TrustProbe for Fake {
    fn mode_and_owner(&self, path: &Path) -> Option<(u32, u32)> {
        self.files.get(path).copied()
    }
    fn executable(&self, path: &Path) -> bool {
        self.executable.iter().any(|known| known == path)
    }
    fn gio_attribute(&self, path: &Path, attribute: &str) -> Option<String> {
        self.attributes
            .get(&(path.to_path_buf(), attribute.to_string()))
            .cloned()
    }
    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        self.content.get(path).cloned()
    }
    fn gio_available(&self) -> bool {
        !self.no_gio
    }
}

const FILE: &str = "/home/u/Desktop/a.desktop";
const FOLDER: &str = "/home/u/Desktop";

/// A starter and its folder with the given modes and owner.
fn starter(file_mode: u32, folder_mode: u32, owner: u32, run: bool, trusted: &str) -> Fake {
    let mut fake = Fake::default();
    fake.files.insert(FILE.into(), (file_mode, owner));
    fake.files.insert(FOLDER.into(), (folder_mode, 1000));
    if run {
        fake.executable.push(FILE.into());
    }
    fake.attributes.insert(
        (FILE.into(), "metadata::trusted".into()),
        trusted.to_string(),
    );
    fake
}

#[test]
fn desktop_task_linux_launch_decision_table() {
    let file = Path::new(FILE);
    let decide = |kind: DesktopKind, fake: &Fake| launch_decision(file, &kind, fake);

    // GNOME and Ubuntu: executable, trusted, nobody else may write.
    let good = starter(0o755, 0o755, 1000, true, "true");
    assert_eq!(decide(DesktopKind::Gnome, &good), Launch::Direct);
    assert_eq!(
        decide(
            DesktopKind::Gnome,
            &starter(0o755, 0o755, 1000, false, "true")
        ),
        Launch::Portal,
        "not executable"
    );
    assert_eq!(
        decide(
            DesktopKind::Gnome,
            &starter(0o755, 0o755, 1000, true, "false")
        ),
        Launch::Portal,
        "not trusted"
    );
    assert_eq!(
        decide(
            DesktopKind::Gnome,
            &starter(0o775, 0o755, 1000, true, "true")
        ),
        Launch::Portal,
        "group may write the file"
    );
    assert_eq!(
        decide(
            DesktopKind::Gnome,
            &starter(0o757, 0o755, 1000, true, "true")
        ),
        Launch::Portal,
        "others may write the file"
    );
    assert_eq!(
        decide(
            DesktopKind::Gnome,
            &starter(0o755, 0o777, 1000, true, "true")
        ),
        Launch::Portal,
        "others may write the folder"
    );
    let mut unknown = starter(0o755, 0o755, 1000, true, "true");
    unknown.attributes.clear();
    assert_eq!(decide(DesktopKind::Gnome, &unknown), Launch::Portal);

    // KDE: executable, or owned by root.
    assert_eq!(
        decide(DesktopKind::Kde, &starter(0o755, 0o755, 1000, true, "")),
        Launch::Direct
    );
    assert_eq!(
        decide(DesktopKind::Kde, &starter(0o644, 0o755, 0, false, "")),
        Launch::Direct,
        "owned by root"
    );
    assert_eq!(
        decide(DesktopKind::Kde, &starter(0o644, 0o755, 1000, false, "")),
        Launch::Portal
    );

    // Xfce: the checksum of the content.
    let mut xfce = starter(0o644, 0o755, 1000, false, "");
    xfce.content
        .insert(FILE.into(), b"[Desktop Entry]\n".to_vec());
    let sum = hex_sha256(b"[Desktop Entry]\n");
    assert_eq!(
        hex_sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    xfce.attributes.insert(
        (FILE.into(), "metadata::xfce-exe-checksum".into()),
        sum.clone(),
    );
    assert_eq!(decide(DesktopKind::Xfce, &xfce), Launch::Direct);
    xfce.attributes.insert(
        (FILE.into(), "metadata::xfce-exe-checksum".into()),
        sum.to_uppercase(),
    );
    assert_eq!(decide(DesktopKind::Xfce, &xfce), Launch::Direct, "case");
    xfce.content.insert(FILE.into(), b"changed".to_vec());
    assert_eq!(decide(DesktopKind::Xfce, &xfce), Launch::Portal);
    xfce.attributes.clear();
    assert_eq!(decide(DesktopKind::Xfce, &xfce), Launch::Portal);

    // Any other desktop never starts a starter itself.
    assert_eq!(decide(DesktopKind::Other, &good), Launch::Portal);
    // Without `gio` every rule gives the portal.
    for kind in [DesktopKind::Gnome, DesktopKind::Kde, DesktopKind::Xfce] {
        let mut no_gio = starter(0o755, 0o755, 0, true, "true");
        no_gio.no_gio = true;
        assert_eq!(decide(kind, &no_gio), Launch::Portal, "{kind:?}");
    }

    // The desktop names.
    assert_eq!(desktop_kind("ubuntu:GNOME"), DesktopKind::Gnome);
    assert_eq!(desktop_kind("GNOME"), DesktopKind::Gnome);
    assert_eq!(desktop_kind("KDE"), DesktopKind::Kde);
    assert_eq!(desktop_kind("XFCE"), DesktopKind::Xfce);
    for other in ["X-Cinnamon", "MATE", "sway", "COSMIC", "LXQt", ""] {
        assert_eq!(desktop_kind(other), DesktopKind::Other, "{other}");
    }
    assert_eq!(
        parse_gio_attribute(
            "uri: file:///a\nattributes:\n  metadata::trusted: true\n",
            "metadata::trusted"
        ),
        Some("true".to_string())
    );
    assert_eq!(
        parse_gio_attribute("uri: file:///a\n", "metadata::trusted"),
        None
    );
}

/// Real files for the mode, the owner and `access`; the attributes of `gio`
/// come from the check.
struct OnDisk {
    gio: HashMap<(PathBuf, String), String>,
}

impl TrustProbe for OnDisk {
    fn mode_and_owner(&self, path: &Path) -> Option<(u32, u32)> {
        SystemTrust.mode_and_owner(path)
    }
    fn executable(&self, path: &Path) -> bool {
        SystemTrust.executable(path)
    }
    fn gio_attribute(&self, path: &Path, attribute: &str) -> Option<String> {
        self.gio
            .get(&(path.to_path_buf(), attribute.to_string()))
            .cloned()
    }
    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        SystemTrust.read(path)
    }
    fn gio_available(&self) -> bool {
        true
    }
}

fn chmod(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn desktop_task_linux_desktop_entry_trust_rules() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Desktop");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("app.desktop");
    let text = b"[Desktop Entry]\nType=Application\nName=T\nExec=true\n";
    std::fs::write(&file, text).unwrap();
    let mut probe = OnDisk {
        gio: HashMap::new(),
    };
    probe
        .gio
        .insert((file.clone(), "metadata::trusted".into()), "true".into());

    chmod(&folder, 0o755);
    chmod(&file, 0o755);
    assert_eq!(
        launch_decision(&file, &DesktopKind::Gnome, &probe),
        Launch::Direct
    );
    chmod(&file, 0o775);
    assert_eq!(
        launch_decision(&file, &DesktopKind::Gnome, &probe),
        Launch::Portal,
        "writable for the group"
    );
    chmod(&file, 0o755);
    chmod(&folder, 0o757);
    assert_eq!(
        launch_decision(&file, &DesktopKind::Gnome, &probe),
        Launch::Portal,
        "folder writable for others"
    );
    chmod(&folder, 0o755);
    chmod(&file, 0o644);
    assert_eq!(
        launch_decision(&file, &DesktopKind::Gnome, &probe),
        Launch::Portal,
        "not executable"
    );
    assert_eq!(
        launch_decision(&file, &DesktopKind::Kde, &probe),
        if SystemTrust.mode_and_owner(&file).map(|(_, owner)| owner) == Some(0) {
            Launch::Direct
        } else {
            Launch::Portal
        },
        "KDE: a file of a user is no starter without the executable bit"
    );
    chmod(&file, 0o700);
    assert_eq!(
        launch_decision(&file, &DesktopKind::Kde, &probe),
        Launch::Direct
    );
    probe.gio.insert(
        (file.clone(), "metadata::xfce-exe-checksum".into()),
        hex_sha256(text),
    );
    assert_eq!(
        launch_decision(&file, &DesktopKind::Xfce, &probe),
        Launch::Direct
    );
    std::fs::write(&file, b"changed after the checksum").unwrap();
    assert_eq!(
        launch_decision(&file, &DesktopKind::Xfce, &probe),
        Launch::Portal
    );
    // se never sets the attribute itself: nothing in the folder changed but
    // the content written above.
    let entries = std::fs::read_dir(&folder).unwrap().count();
    assert_eq!(entries, 1);
}

#[test]
fn desktop_task_linux_desktop_dir_home_disables_files() {
    let home = Path::new("/home/u");
    let folder = |text: Option<&str>| resolve(text, home);
    assert_eq!(
        folder(None),
        Resolved::Folder("/home/u/Desktop".into()),
        "no user-dirs.dirs: ~/Desktop"
    );
    assert_eq!(
        folder(Some("XDG_DOCUMENTS_DIR=\"$HOME/Docs\"\n")),
        Resolved::Folder("/home/u/Desktop".into())
    );
    assert_eq!(
        folder(Some("# comment\nXDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n")),
        Resolved::Folder("/home/u/Schreibtisch".into())
    );
    assert_eq!(
        folder(Some("XDG_DESKTOP_DIR='${HOME}/D'\n")),
        Resolved::Folder("/home/u/D".into())
    );
    assert_eq!(
        folder(Some("XDG_DESKTOP_DIR=\"/data/desk\"\n")),
        Resolved::Folder("/data/desk".into())
    );
    // The home folder itself: switched off.
    for off in [
        "XDG_DESKTOP_DIR=\"$HOME\"\n",
        "XDG_DESKTOP_DIR=\"$HOME/\"\n",
        "XDG_DESKTOP_DIR=\"/home/u\"\n",
        "  XDG_DESKTOP_DIR=\"${HOME}\"  \n",
    ] {
        assert_eq!(folder(Some(off)), Resolved::Off, "{off:?}");
    }
    // A relative value is no folder: the default.
    assert_eq!(
        folder(Some("XDG_DESKTOP_DIR=\"Desktop\"\n")),
        Resolved::Folder("/home/u/Desktop".into())
    );
    // Off gives no folder to scan; a folder is a launcher folder.
    assert!(folders_of(Some(Resolved::Off)).is_empty());
    assert!(folders_of(None).is_empty());
    let folders = folders_of(Some(Resolved::Folder("/home/u/Desktop".into())));
    assert_eq!(folders.len(), 1);
    assert!(folders[0].launchers && !folders[0].public);
    assert_eq!(folders[0].path, Path::new("/home/u/Desktop"));
}
