//! Signals of DRV-M5 for the places of the file managers: GTK bookmarks only
//! outside the GLib rule, exact removal, foreign entries untouched, cleanup
//! after a host crash. Everything runs on temporary files; no environment
//! variable is changed.
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::bookmarks::{add_in, glib_shows_mount, remove_in, uri_of, BookmarkEntry, Targets};

struct Fixture {
    root: tempfile::TempDir,
    home: PathBuf,
    targets: Targets,
}

fn fixture() -> Fixture {
    // `tempfile::tempdir()` names its folder `.tmpXXXX`; GLib's rule rejects
    // any `/.` in the whole mount path, so the home below it must not start
    // with a dot.
    let root = tempfile::Builder::new()
        .prefix("se-bookmarks-")
        .tempdir()
        .expect("temporary folder");
    let home = root.path().join("home");
    fs::create_dir_all(home.join("SmartExplorer")).expect("home");
    let targets = Targets::resolve(
        Some(root.path().join("config").into_os_string()),
        Some(root.path().join("data").into_os_string()),
        Some(home.clone()),
    )
    .expect("targets");
    Fixture {
        root,
        home,
        targets,
    }
}

fn entry(id: &str, folder: &Path, label: &str) -> BookmarkEntry {
    BookmarkEntry {
        id: id.into(),
        folder: folder.to_path_buf(),
        label: label.into(),
    }
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

const XBEL_HEAD: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<!DOCTYPE xbel>\n",
    "<xbel xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" ",
    "xmlns:kdepriv=\"http://www.kde.org/kdepriv\" ",
    "xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n"
);

/// A KDE file with foreign content: a comment, an entity, irregular blanks
/// and a foreign element with the `href` of the drive used in the tests.
fn foreign_xbel() -> String {
    format!(
        "{XBEL_HEAD}{}",
        concat!(
            " <!-- Orte &amp; mehr -->\n",
            " <bookmark href=\"file:///home/u\">\n",
            "  <title>Home &amp; Co</title>\n",
            "  <info>\n",
            "   <metadata owner=\"http://www.kde.org\">\n",
            "    <ID>1234</ID>\n",
            "   </metadata>\n",
            "  </info>\n",
            " </bookmark>\n",
            "\t<bookmark href=\"file:///srv/a\">\n",
            "  <title>Fremd</title>\n",
            " </bookmark>\n",
            "</xbel>\n"
        )
    )
}

#[test]
fn drive_task_gtk_bookmark_only_outside_glib_rule() {
    let fixture = fixture();
    let inside = fixture.home.join("SmartExplorer").join("Geräte");
    let hidden = fixture.home.join(".local").join("x");
    let outside = fixture.root.path().join("run/smart-explorer/Tresore/A");
    for folder in [&inside, &hidden, &outside] {
        fs::create_dir_all(folder.parent().expect("parent")).expect("parent folder");
    }
    // A drive below the home folder without a dot: GLib shows it itself.
    add_in(&fixture.targets, &entry("one", &inside, "Geräte")).expect("add inside");
    assert!(
        !fixture.targets.gtk.exists(),
        "no GTK bookmark for a drive GLib shows by itself"
    );
    assert!(text(&fixture.targets.kde).contains(&uri_of(&inside)));
    // Outside the rule: hidden folder and runtime folder.
    add_in(&fixture.targets, &entry("two", &hidden, "Versteckt")).expect("add hidden");
    add_in(&fixture.targets, &entry("three", &outside, "A")).expect("add outside");
    let expected = format!("{} Versteckt\n{} A\n", uri_of(&hidden), uri_of(&outside));
    assert_eq!(text(&fixture.targets.gtk), expected);
    // Adding again changes nothing.
    add_in(&fixture.targets, &entry("three", &outside, "A")).expect("add twice");
    assert_eq!(text(&fixture.targets.gtk), expected);
    assert_eq!(
        text(&fixture.targets.kde).matches("<bookmark href").count(),
        3,
        "KDE lists every drive once"
    );
    // The file manager does not see a temporary file left behind.
    let leftovers = fs::read_dir(fixture.targets.gtk.parent().expect("gtk folder"))
        .expect("gtk folder")
        .filter_map(Result::ok)
        .filter(|item| item.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn drive_task_gtk_bookmark_removed_exactly() {
    let fixture = fixture();
    let folder = Path::new("/srv/a");
    let before = concat!(
        "file:///home/u/Dokumente Dokumente\n",
        "\n",
        "file:///mnt/x\n",
        "file:///srv/a Vom Nutzer umbenannt\n",
        "file:///srv/ab Nicht meins\n"
    );
    let after = concat!(
        "file:///home/u/Dokumente Dokumente\n",
        "\n",
        "file:///mnt/x\n",
        "file:///srv/ab Nicht meins\n"
    );
    fs::create_dir_all(fixture.targets.gtk.parent().expect("gtk folder")).expect("folder");
    fs::write(&fixture.targets.gtk, before).expect("gtk file");
    remove_in(&fixture.targets, folder).expect("remove");
    assert_eq!(
        text(&fixture.targets.gtk),
        after,
        "the line with the URI goes whatever its label, every other line stays"
    );
    // Without a line of its own the file is not written at all.
    let inode = fs::metadata(&fixture.targets.gtk).expect("metadata").ino();
    remove_in(&fixture.targets, folder).expect("remove again");
    assert_eq!(text(&fixture.targets.gtk), after);
    assert_eq!(
        fs::metadata(&fixture.targets.gtk).expect("metadata").ino(),
        inode,
        "an untouched file is not replaced"
    );
}

#[test]
fn drive_task_kde_places_preserves_foreign_entries() {
    let fixture = fixture();
    let original = foreign_xbel();
    fs::create_dir_all(fixture.targets.kde.parent().expect("data folder")).expect("folder");
    fs::write(&fixture.targets.kde, &original).expect("kde file");
    // The foreign element has the same href as the drive of se.
    let folder = Path::new("/srv/a");
    add_in(&fixture.targets, &entry("abc123", folder, "Server & Co")).expect("add");
    let added = text(&fixture.targets.kde);
    assert!(added.contains("<title>Server &amp; Co</title>"));
    assert!(added.contains("<ID>se-abc123</ID>"));
    assert!(added.contains("<metadata owner=\"urn:smart-explorer:places\">"));
    assert!(added.contains("<id>abc123</id>"));
    assert!(added.contains("<bookmark:icon name=\"folder-remote\"/>"));
    assert!(added.starts_with(&original[..original.len() - "</xbel>\n".len()]));
    // A second add updates the element instead of adding another.
    add_in(&fixture.targets, &entry("abc123", folder, "Neu")).expect("update");
    let updated = text(&fixture.targets.kde);
    assert_eq!(updated.matches("urn:smart-explorer:places").count(), 1);
    assert!(updated.contains("<title>Neu</title>") && !updated.contains("Server &amp; Co"));
    // Removing it restores the original bytes, the foreign element included.
    remove_in(&fixture.targets, folder).expect("remove");
    assert_eq!(text(&fixture.targets.kde), original);
    // A file that is no XML is reported and left alone.
    fs::write(&fixture.targets.kde, "<xbel><bookmark>").expect("broken file");
    assert!(add_in(&fixture.targets, &entry("abc123", folder, "X")).is_err());
    assert_eq!(text(&fixture.targets.kde), "<xbel><bookmark>");
}

#[test]
fn drive_task_kde_places_removed_exactly() {
    let fixture = fixture();
    fs::create_dir_all(fixture.targets.kde.parent().expect("data folder")).expect("folder");
    fs::write(&fixture.targets.kde, foreign_xbel()).expect("kde file");
    let first = Path::new("/srv/one");
    let second = Path::new("/srv/two");
    add_in(&fixture.targets, &entry("one", first, "Eins")).expect("add one");
    add_in(&fixture.targets, &entry("two", second, "Zwei")).expect("add two");
    let both = text(&fixture.targets.kde);
    remove_in(&fixture.targets, first).expect("remove one");
    let remaining = text(&fixture.targets.kde);
    assert!(!remaining.contains("file:///srv/one") && !remaining.contains("Eins"));
    assert!(remaining.contains("file:///srv/two") && remaining.contains("<id>two</id>"));
    assert!(remaining.contains("file:///srv/a") && remaining.contains("Home &amp; Co"));
    assert!(both.len() > remaining.len());
    // A folder nobody registered changes nothing.
    let inode = fs::metadata(&fixture.targets.kde).expect("metadata").ino();
    remove_in(&fixture.targets, Path::new("/srv/a")).expect("foreign href");
    assert_eq!(text(&fixture.targets.kde), remaining);
    assert_eq!(
        fs::metadata(&fixture.targets.kde).expect("metadata").ino(),
        inode
    );
}

#[test]
fn drive_task_bookmarks_cleaned_after_host_crash() {
    let fixture = fixture();
    let gtk_before = "file:///home/u/Dokumente Dokumente\n";
    fs::create_dir_all(fixture.targets.gtk.parent().expect("gtk folder")).expect("folder");
    fs::write(&fixture.targets.gtk, gtk_before).expect("gtk file");
    let kde_before = foreign_xbel();
    fs::create_dir_all(fixture.targets.kde.parent().expect("data folder")).expect("folder");
    fs::write(&fixture.targets.kde, &kde_before).expect("kde file");
    let folder = fixture.root.path().join("run/smart-explorer/Tresore/A");
    fs::create_dir_all(&folder).expect("drive folder");
    // The host adds the entries and then dies: it never removes them.
    add_in(&fixture.targets, &entry("crash", &folder, "A")).expect("add");
    assert!(text(&fixture.targets.gtk).contains(&uri_of(&folder)));
    assert!(text(&fixture.targets.kde).contains(&uri_of(&folder)));
    // The daemon removes them without asking anyone, as often as needed.
    remove_in(&fixture.targets, &folder).expect("cleanup");
    remove_in(&fixture.targets, &folder).expect("cleanup is idempotent");
    assert_eq!(text(&fixture.targets.gtk), gtk_before);
    assert_eq!(text(&fixture.targets.kde), kde_before);
    // Without any file there is nothing to clean.
    let empty = self::fixture();
    remove_in(&empty.targets, &folder).expect("no files");
}

#[test]
fn drive_task_glib_display_rule_path() {
    let fixture = fixture();
    let home = &fixture.home;
    let run = fixture.root.path().join("run/smart-explorer/Tresore");
    fs::create_dir_all(&run).expect("runtime folder");
    fs::create_dir_all(home.join(".local")).expect("hidden folder");
    fs::create_dir_all(home.join("SmartExplorer/.hidden")).expect("hidden below");
    fs::create_dir_all(fixture.root.path().join("home2")).expect("sibling");
    let link = fixture.root.path().join("link");
    std::os::unix::fs::symlink(home, &link).expect("home link");
    let table: Vec<(PathBuf, PathBuf, bool)> = vec![
        (home.join("SmartExplorer/Geräte"), home.clone(), true),
        (home.join(".local/x"), home.clone(), false),
        (home.join("SmartExplorer/.hidden/x"), home.clone(), false),
        (run.join("A"), home.clone(), false),
        (PathBuf::from("/mnt/x"), home.clone(), false),
        (fixture.root.path().join("home2/x"), home.clone(), false),
        (home.clone(), home.clone(), false),
        // Home as a symlink, on either side.
        (link.join("SmartExplorer/x"), home.clone(), true),
        (home.join("SmartExplorer/x"), link.clone(), true),
        (link.join(".local/x"), home.clone(), false),
    ];
    for (folder, base, expected) in table {
        assert_eq!(
            glib_shows_mount(&folder, &base),
            expected,
            "{} below {}",
            folder.display(),
            base.display()
        );
    }
}

#[test]
fn bookmark_uri_and_targets_follow_the_rules() {
    assert_eq!(
        uri_of(Path::new("/home/ü/a b/~x_y-z.1")),
        "file:///home/%C3%BC/a%20b/~x_y-z.1"
    );
    let home = PathBuf::from("/home/u");
    let by_default =
        Targets::resolve(None, Some("relative".into()), Some(home.clone())).expect("defaults");
    assert_eq!(
        by_default.gtk,
        Path::new("/home/u/.config/gtk-3.0/bookmarks")
    );
    assert_eq!(
        by_default.kde,
        Path::new("/home/u/.local/share/user-places.xbel")
    );
    let set =
        Targets::resolve(Some("/c".into()), Some("/d".into()), Some(home.clone())).expect("set");
    assert_eq!(set.gtk, Path::new("/c/gtk-3.0/bookmarks"));
    assert_eq!(set.kde, Path::new("/d/user-places.xbel"));
    assert!(Targets::resolve(None, None, None).is_err());
}
