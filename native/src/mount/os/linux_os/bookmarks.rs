//! File-manager places of a Linux drive (DRV-M5, decision D2-14): one entry
//! per mounted se drive in the places of the file managers.
//!
//! GLib, GVfs, Nautilus, Nemo and Thunar show a mount below `$HOME` by
//! themselves when no path part starts with a dot (strand S2 NR N8), so such
//! a drive gets no GTK bookmark; a drive anywhere else (`$XDG_RUNTIME_DIR`
//! for vaults, a folder chosen with `--mount-point`) gets one line in
//! `gtk-3.0/bookmarks`. Dolphin does not show `fuse.smartexplorer` mounts by
//! itself (assumption A8), so every drive gets an entry in
//! `user-places.xbel`. The host adds the entries after its mount and removes
//! them before it unmounts; the daemon removes them again when a host ended
//! without doing so (`mount_cleanup.rs`).
//!
//! Neither GTK nor KDE lock their file: the file is read once and replaced
//! once through `rename`, so a program that writes in between loses its
//! change (known limit, `docs/GOTCHAS.md`).
use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::{bookmarks_gtk, bookmarks_kde, mount_point};

/// One drive in the places of the file managers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BookmarkEntry {
    /// The mount id; KDE keeps it as the identity of the entry.
    pub id: String,
    /// The mount folder as configured (not canonical).
    pub folder: PathBuf,
    /// The drive label.
    pub label: String,
}

/// The two files the entries live in and the home folder the GLib rule
/// compares with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Targets {
    pub gtk: PathBuf,
    pub kde: PathBuf,
    pub home: Option<PathBuf>,
}

impl Targets {
    pub(crate) fn from_env() -> io::Result<Self> {
        Self::resolve(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("XDG_DATA_HOME"),
            crate::support_dirs::home_dir(),
        )
    }

    /// `gtk-3.0/bookmarks` below `$XDG_CONFIG_HOME` and `user-places.xbel`
    /// below `$XDG_DATA_HOME`; an empty or relative variable counts as unset
    /// and the defaults below the home folder apply.
    pub(crate) fn resolve(
        config_home: Option<OsString>,
        data_home: Option<OsString>,
        home: Option<PathBuf>,
    ) -> io::Result<Self> {
        let absolute = |value: Option<OsString>| {
            value
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && !path.as_os_str().is_empty())
        };
        let fallback = |relative: &str| {
            home.as_ref()
                .filter(|home| home.is_absolute())
                .map(|home| home.join(relative))
        };
        let config = absolute(config_home)
            .or_else(|| fallback(".config"))
            .ok_or_else(no_home)?;
        let data = absolute(data_home)
            .or_else(|| fallback(".local/share"))
            .ok_or_else(no_home)?;
        Ok(Self {
            gtk: config.join("gtk-3.0").join("bookmarks"),
            kde: data.join("user-places.xbel"),
            home,
        })
    }
}

/// Whether GLib shows a mount at `folder` by itself: it lies below
/// `<home>/` and no path part starts with a dot. Both paths are compared in
/// canonical form; when canonicalizing fails the path stays as it is.
pub(crate) fn glib_shows_mount(folder: &Path, home: &Path) -> bool {
    let folder = mount_point::canonical(folder).unwrap_or_else(|_| folder.to_path_buf());
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let folder = folder.to_string_lossy();
    let mut prefix = home.to_string_lossy().trim_end_matches('/').to_string();
    prefix.push('/');
    folder.starts_with(&prefix) && !folder.contains("/.")
}

/// Adds the entries of a mounted drive: KDE always, GTK outside the GLib
/// rule. Both are tried; the first failure is returned.
pub(crate) fn add(entry: &BookmarkEntry) -> io::Result<()> {
    add_in(&Targets::from_env()?, entry)
}

/// Removes every entry of the drive at `folder` from both files, whatever
/// the GLib rule says. Nothing there is success.
pub(crate) fn remove(folder: &Path) -> io::Result<()> {
    remove_in(&Targets::from_env()?, folder)
}

pub(crate) fn add_in(targets: &Targets, entry: &BookmarkEntry) -> io::Result<()> {
    let kde = bookmarks_kde::add_in(&targets.kde, entry);
    let by_glib = targets
        .home
        .as_deref()
        .is_some_and(|home| glib_shows_mount(&entry.folder, home));
    let gtk = if by_glib {
        Ok(())
    } else {
        bookmarks_gtk::add_in(&targets.gtk, entry)
    };
    combine(kde, gtk)
}

pub(crate) fn remove_in(targets: &Targets, folder: &Path) -> io::Result<()> {
    combine(
        bookmarks_kde::remove_in(&targets.kde, folder),
        bookmarks_gtk::remove_in(&targets.gtk, folder),
    )
}

/// `file://` and the path with every byte outside `A-Za-z0-9-._~/` as `%XX`
/// in capital hex (the form of GTK and KDE).
pub(crate) fn uri_of(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// Replaces `file` through a temporary file in the same folder: content,
/// `sync_all`, the permissions of the old file (else `0644`), `rename`.
/// A symlink at `file` is followed, so a managed dotfile stays a link.
pub(super) fn write_atomic(file: &Path, content: &[u8]) -> io::Result<()> {
    let file = fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Datei der Dateimanager-Einträge ohne Ordner",
        ));
    };
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let mode = fs::metadata(&file)
        .map(|metadata| metadata.permissions().mode() & 0o7777)
        .unwrap_or(0o644);
    let mut temporary = name.to_os_string();
    temporary.push(format!(".se-{}.tmp", std::process::id()));
    let temporary = dir.join(temporary);
    let _ = fs::remove_file(&temporary);
    let written = (|| {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        out.write_all(content)?;
        out.sync_all()?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
        fs::rename(&temporary, &file)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn combine(first: io::Result<()>, second: io::Result<()>) -> io::Result<()> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(second)) => {
            Err(io::Error::new(first.kind(), format!("{first}; {second}")))
        }
    }
}

fn no_home() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "weder XDG_CONFIG_HOME/XDG_DATA_HOME noch HOME ist gesetzt",
    )
}
