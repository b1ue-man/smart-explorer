//! Acceptance signals of DRV-M1 (plan 2026-10-17 „Rechte, se Desktop,
//! Tresore“, `strang-laufwerke.md` DRV-M1): the Windows rules stay exactly
//! as before, and the POSIX dialect accepts every name Linux and Android can
//! represent. The serde signal of the `types.rs` split lives in
//! `types_split_tests.rs`.
use super::optimization_fixture::FixtureDirectory;
use super::path::validate_windows_component;
use super::path_dialect::POSIX_NAME_MAX_BYTES;
use super::{
    BackendRoot, MountEngine, MountId, MountMode, MountRuntimeConfig, OpenDisposition,
    OpenFileOptions, PathDialect, PathProjector,
};
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Cursor, Read, Write};
use std::sync::Arc;

/// Names that Windows rejects and Linux or Android store without trouble.
const WINDOWS_INVALID_POSIX_VALID: &[&str] = &[
    "a:b",
    "CON",
    "con.txt",
    "COM1",
    "LPT9.log",
    "NUL",
    "CLOCK$",
    "trailing.",
    "trailing ",
    "what?",
    "x*y",
    "<tag>",
    "pipe|",
    "quote\"",
    "tab\tname",
];

#[test]
fn drive_task_dialect_windows_table_unchanged() {
    let long_ascii = "a".repeat(255);
    let too_long_ascii = "a".repeat(256);
    let long_umlaut = "ä".repeat(255);
    let too_long_emoji = "😀".repeat(128);
    // (name, valid under the Windows rules of v0.5.182)
    let table: &[(&str, bool)] = &[
        ("report.txt", true),
        ("Grüße.md", true),
        ("COM0", true),
        ("COMX", true),
        (".hidden", true),
        ("a:b", false),
        ("CON", false),
        ("con.txt", false),
        ("nul.tar.gz", false),
        ("COM1", false),
        ("LPT9.log", false),
        ("COM¹", false),
        ("CLOCK$", false),
        ("trailing.", false),
        ("trailing ", false),
        ("what?", false),
        ("x*y", false),
        ("<tag>", false),
        ("pipe|", false),
        ("quote\"", false),
        ("back\\slash", false),
        ("slash/", false),
        ("tab\tname", false),
        ("nul\0byte", false),
        (".", false),
        ("..", false),
        ("", false),
        (long_ascii.as_str(), true),
        (too_long_ascii.as_str(), false),
        (long_umlaut.as_str(), true),
        (too_long_emoji.as_str(), false),
    ];
    for (name, valid) in table {
        assert_eq!(
            PathDialect::Windows.validate_component(name).is_ok(),
            *valid,
            "Windows rule changed for {name:?}"
        );
        assert_eq!(validate_windows_component(name).is_ok(), *valid, "{name:?}");
    }
    let root = BackendRoot::parse("/srv").unwrap();
    let legacy = PathProjector::new(root.clone());
    let explicit = PathProjector::with_dialect(root, PathDialect::Windows);
    // (callback path, backend path or None for a rejected path)
    let projections: &[(&str, Option<&str>)] = &[
        ("", Some("/srv")),
        ("\\", Some("/srv")),
        ("/", Some("/srv")),
        ("\\a\\b.txt", Some("/srv/a/b.txt")),
        ("/a/b.txt", Some("/srv/a/b.txt")),
        ("\\a/b", Some("/srv/a/b")),
        ("a", None),
        ("\\\\server\\share", None),
        ("\\a\\", None),
        ("\\a\\\\b", None),
        ("\\a:b", None),
        ("\\CON", None),
        ("\\a\0b", None),
    ];
    for (callback, expected) in projections {
        for projector in [&legacy, &explicit] {
            let projected = projector.project(callback);
            assert_eq!(
                projected.as_ref().ok().map(|path| path.backend()),
                *expected,
                "Windows projection changed for {callback:?}"
            );
        }
    }
    assert_eq!(legacy.dialect(), PathDialect::Windows);
    assert_eq!(PathDialect::default(), PathDialect::Windows);
    assert_eq!(
        serde_json::to_string(&PathDialect::Windows).unwrap(),
        "\"windows\""
    );
    assert_eq!(
        serde_json::to_string(&PathDialect::Posix).unwrap(),
        "\"posix\""
    );
}

#[test]
fn drive_task_dialect_posix_accepts_colon_con_trailing_dot() {
    for name in WINDOWS_INVALID_POSIX_VALID
        .iter()
        .chain(&["...", ".hidden", "Grüße"])
    {
        assert!(
            PathDialect::Posix.validate_component(name).is_ok(),
            "POSIX rejected {name:?}"
        );
    }
    let projector =
        PathProjector::with_dialect(BackendRoot::parse("/srv").unwrap(), PathDialect::Posix);
    for (callback, backend) in [
        ("/dir/a:b", "/srv/dir/a:b"),
        ("/CON", "/srv/CON"),
        ("/trailing.", "/srv/trailing."),
        ("/what?/x*y", "/srv/what?/x*y"),
    ] {
        let projected = projector.project(callback).unwrap();
        assert_eq!(projected.backend(), backend);
    }
    for root in ["", "/"] {
        let projected = projector.project(root).unwrap();
        assert_eq!(projected.backend(), "/srv");
        assert!(projected.relative().is_empty());
    }
    assert_eq!(PathDialect::Posix.callback_path("dir/a:b"), "/dir/a:b");
    assert_eq!(
        PathDialect::Windows.callback_path("dir/name"),
        "\\dir\\name"
    );
}

#[test]
fn drive_task_dialect_posix_rejects_nul_slash_backslash_dotdot() {
    for name in ["", ".", "..", "a\0b", "a/b", "/", "a\\b", "\\"] {
        let error = PathDialect::Posix.validate_component(name).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
    }
    let projector =
        PathProjector::with_dialect(BackendRoot::parse("/srv").unwrap(), PathDialect::Posix);
    for callback in [
        "a", "\\dir", "/dir\\x", "//x", "/x/", "/x//y", "/./x", "/../x", "/x/..", "/a\0b",
    ] {
        assert!(
            projector.project(callback).is_err(),
            "POSIX accepted {callback:?}"
        );
    }
}

#[test]
fn drive_task_dialect_posix_name_max_bytes() {
    assert_eq!(POSIX_NAME_MAX_BYTES, 255);
    // (name, POSIX valid, Windows valid): bytes against UTF-16 units.
    let table = [
        ("a".repeat(255), true, true),
        ("a".repeat(256), false, false),
        ("ä".repeat(127), true, true),
        ("ä".repeat(128), false, true),
        ("€".repeat(85), true, true),
        ("€".repeat(86), false, true),
        ("😀".repeat(63), true, true),
        ("😀".repeat(64), false, true),
        ("😀".repeat(128), false, false),
    ];
    for (name, posix, windows) in table {
        assert_eq!(
            PathDialect::Posix.validate_component(&name).is_ok(),
            posix,
            "POSIX limit for {} bytes",
            name.len()
        );
        assert_eq!(
            PathDialect::Windows.validate_component(&name).is_ok(),
            windows,
            "Windows limit for {} UTF-16 units",
            name.encode_utf16().count()
        );
    }
}

#[test]
fn drive_task_posix_listing_shows_windows_invalid_names() {
    let mut names = WINDOWS_INVALID_POSIX_VALID.to_vec();
    names.push("plain.txt");
    let backend = NamesBackend::new(&names);
    let directory = FixtureDirectory::new();
    let posix = engine(&directory, &backend, PathDialect::Posix, "posix-listing");
    assert_eq!(listed(&posix, "/"), sorted(&names));
    // The data path works for such a name as well.
    let handle = posix
        .open_file(
            "/CON",
            OpenFileOptions {
                writable: false,
                disposition: OpenDisposition::OpenExisting,
            },
        )
        .unwrap();
    let mut bytes = vec![0; 16];
    let read = posix.read(handle, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..read], b"CON");
    posix.close(handle).unwrap();
    assert!(posix.stat("/a:b").is_ok());
    // The same backend under the unchanged Windows rules.
    let windows_directory = FixtureDirectory::new();
    let windows = engine(
        &windows_directory,
        &backend,
        PathDialect::Windows,
        "windows-listing",
    );
    assert_eq!(listed(&windows, "\\"), ["plain.txt"]);
}

#[test]
fn drive_task_posix_listing_hides_backslash_names() {
    let names = ["a\\b", "dir\\sub\\file.txt", "\\leading", "kept.txt"];
    let backend = NamesBackend::new(&names);
    for dialect in [PathDialect::Posix, PathDialect::Windows] {
        let directory = FixtureDirectory::new();
        let engine = engine(&directory, &backend, dialect, "backslash-listing");
        let root = dialect.callback_path("");
        assert_eq!(listed(&engine, &root), ["kept.txt"], "{dialect:?}");
    }
}

fn engine(
    directory: &FixtureDirectory,
    backend: &Arc<NamesBackend>,
    dialect: PathDialect,
    id: &str,
) -> MountEngine {
    let handle: crate::vfs::BackendHandle = backend.clone();
    MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse(id).unwrap(), MountMode::ReadOnly)
            .with_path_dialect(dialect),
        handle,
        directory.path(),
    )
    .unwrap()
}

fn listed(engine: &MountEngine, callback_path: &str) -> Vec<String> {
    let mut names = engine
        .list_dir(callback_path)
        .unwrap()
        .into_iter()
        .map(|metadata| metadata.name)
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut names = names
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// One directory of regular files whose content is the file name; names
/// are passed through exactly as a POSIX server would report them.
struct NamesBackend {
    names: Vec<String>,
}

impl NamesBackend {
    fn new(names: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            names: names.iter().map(|name| name.to_string()).collect(),
        })
    }

    fn file(&self, name: &str) -> VfsMeta {
        VfsMeta {
            name: name.to_string(),
            size: name.len() as u64,
            mtime_ms: 1_700_000_000_000,
            ..VfsMeta::default()
        }
    }

    fn child(&self, path: &str) -> Option<&String> {
        let name = path.strip_prefix('/')?;
        self.names
            .iter()
            .find(|candidate| candidate.as_str() == name)
    }
}

impl Backend for NamesBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn case_sensitive_paths(&self, _root: &str) -> bool {
        true
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if path != "/" {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "fixture directory missing",
            ));
        }
        Ok(self.names.iter().map(|name| self.file(name)).collect())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path == "/" {
            return Ok(VfsMeta {
                name: String::new(),
                is_dir: true,
                ..VfsMeta::default()
            });
        }
        self.child(path)
            .map(|name| self.file(name))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fixture object missing"))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let name = self
            .child(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fixture file missing"))?;
        Ok(Box::new(Cursor::new(name.clone().into_bytes())))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(read_only())
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Err(read_only())
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }
}

fn read_only() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "fixture is read-only")
}
