//! Runtime selection/security cases owned by the single remote optimization
//! suite. No drive is created here; the real-volume fixture proves batching.

use std::{
    fs::{self, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{symlink_dir, symlink_file, MetadataExt},
    },
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

use crate::mount::{MountId, MountRuntimePreference};

use super::{
    cache_lease::CacheLease,
    private_payload::{
        PrivatePayload, BUNDLED_DOKANY_BYTES, BUNDLED_DOKANY_SHA256, BUNDLED_DOKANY_SOURCE,
        BUNDLED_DOKANY_SOURCE_ARCHIVE, BUNDLED_DOKANY_SOURCE_SHA256,
    },
    runtime::DokanyRuntime,
    runtime_attempt::RuntimeAttempt,
    runtime_notices,
    runtime_selection::RuntimeSelection,
};

const REPARSE_POINT: u32 = 0x400;
const CANARY: &[u8] = b"owned runtime fixture data must remain unchanged";

#[test]
#[ignore = "remote optimization suite; pinned DLL/driver and symlink privilege required"]
fn mount_optimization_task_private_runtime_selection_and_rejections() {
    assert_fixture_provenance();
    let official = DokanyRuntime::preflight().expect("suite must install/start the official runtime");
    assert!(!official.is_private());
    let official_path = official.loaded_path().expect("official loaded pathname");
    assert_eq!(
        official_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase(),
        "dokan2.dll"
    );
    drop(official);

    healthy_private_and_completion();
    abandoned_attempt_uses_official(&official_path);
    explicit_system_ignores_private(&official_path);
    for input in [
        RejectedInput::Tampered,
        RejectedInput::Truncated,
        RejectedInput::Directory,
        RejectedInput::FileSymlink,
        RejectedInput::HardLink,
        RejectedInput::DirectorySymlink,
    ] {
        rejected_input_uses_official(input, &official_path);
    }
    eprintln!("optimization-runtime: private selection, payload ownership and fallback cases passed");
}

fn assert_fixture_provenance() {
    assert_eq!(std::mem::size_of::<usize>(), 8, "fixture must be Windows x64");
    let expected = std::env::var("SMART_EXPLORER_DOKANY_DLL_SHA256")
        .expect("suite must supply the trusted prepared DLL SHA-256 to this test binary");
    assert_eq!(expected.len(), 64);
    assert!(expected.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(expected.to_ascii_lowercase(), BUNDLED_DOKANY_SHA256);
    assert!(
        BUNDLED_DOKANY_BYTES.len() > 512,
        "official-only binaries cannot pass this fixture"
    );
    assert_eq!(digest(BUNDLED_DOKANY_BYTES), BUNDLED_DOKANY_SHA256);
    assert_eq!(
        BUNDLED_DOKANY_SOURCE,
        "f1d5de68ff459af94e309cfdd171e4b8ca2af4dd"
    );
    assert!(!BUNDLED_DOKANY_SOURCE_ARCHIVE.is_empty());
    assert_eq!(
        digest(BUNDLED_DOKANY_SOURCE_ARCHIVE),
        BUNDLED_DOKANY_SOURCE_SHA256
    );
}

fn healthy_private_and_completion() {
    let case = RuntimeCase::new("healthy");
    assert!(
        CacheLease::acquire(case.root(), &case.id).is_err(),
        "lease must exclude a second owner"
    );
    let payload = PrivatePayload::stage(case.root())
        .expect("fresh private staging")
        .expect("private payload required");
    assert_same_path(&payload.path, &case.payload_path());
    // Check the file ownership before an image mapping could independently
    // deny writes. This specifically exercises PrivatePayload's locked handle.
    assert_write_denied(&payload.path);
    assert_not_loaded(&payload.path);
    let directory = payload.path.parent().unwrap();
    assert_eq!(
        fs::read(directory.join("corresponding-source.zip")).unwrap(),
        BUNDLED_DOKANY_SOURCE_ARCHIVE
    );
    for name in ["NOTICE.txt", "LICENSE-GPL-3.0.txt", "LICENSE-LGPL-3.0.txt"] {
        assert!(
            !fs::read(directory.join(name)).unwrap().is_empty(),
            "missing {name}"
        );
        assert_write_denied(&directory.join(name));
    }
    drop(payload);

    // Exercise the loader's slash normalization and Windows path case handling,
    // not merely the comparison helper below. Lease still owns the same objects.
    let alternate = PathBuf::from(
        case.root()
            .to_str()
            .unwrap()
            .replace('\\', "/")
            .to_ascii_uppercase(),
    );
    let selection = RuntimeSelection::select(&alternate, &case.id, MountRuntimePreference::Auto)
        .expect("valid private selection must not require compatibility mode");
    assert!(
        selection.runtime.is_private(),
        "healthy private input silently fell back"
    );
    let loaded = selection.runtime.loaded_path().unwrap();
    assert_same_path(&loaded, &case.payload_path());
    assert_eq!(digest(&fs::read(&loaded).unwrap()), BUNDLED_DOKANY_SHA256);
    assert_write_denied(&loaded);
    assert_eq!(selection.runtime.info().library_api, 231);
    assert_eq!(selection.runtime.info().driver_protocol, 400);
    selection.complete();
    assert_missing(&case.marker());
    assert_not_loaded(&case.payload_path());
    // A fresh arm proves completion removed the owned object, not just a name
    // that still aliases an exclusively held old marker.
    RuntimeAttempt::arm(case.root(), &case.id)
        .unwrap()
        .complete()
        .unwrap();
    assert_missing(&case.marker());
    case.finish();
}

fn abandoned_attempt_uses_official(official_path: &Path) {
    let case = RuntimeCase::new("unfinished");
    let selection = RuntimeSelection::select(case.root(), &case.id, MountRuntimePreference::Auto)
        .expect("initial private selection");
    assert!(selection.runtime.is_private());
    drop(selection); // Deliberately do not mark this attempt completed.
    assert_marker(&case);
    let retry = RuntimeSelection::select(case.root(), &case.id, MountRuntimePreference::Auto)
        .expect("unfinished private attempt must permit official retry");
    assert_official(&retry, official_path);
    retry.complete();
    assert_marker(&case); // Compatibility completion must not clear an old attempt.
    assert_not_loaded(&case.payload_path());
    case.finish();
}

fn explicit_system_ignores_private(official_path: &Path) {
    let case = RuntimeCase::new("system");
    let invalid_root = case.root().join("private-runtime");
    fs::write(&invalid_root, CANARY).unwrap();
    let selection = RuntimeSelection::select(case.root(), &case.id, MountRuntimePreference::System)
        .expect("explicit System preference must not inspect the invalid private root");
    assert_official(&selection, official_path);
    selection.complete();
    assert_missing(&case.marker());
    assert_eq!(fs::read(invalid_root).unwrap(), CANARY);
    case.finish();
}

#[derive(Clone, Copy, Debug)]
enum RejectedInput {
    Tampered,
    Truncated,
    Directory,
    FileSymlink,
    HardLink,
    DirectorySymlink,
}

fn rejected_input_uses_official(input: RejectedInput, official_path: &Path) {
    let case = RuntimeCase::new("rejected");
    let candidate = case.payload_path();
    install_rejected_input(&case, input);
    assert_not_loaded(&candidate);
    // This staging API does not load libraries. An error here establishes the
    // rejection at the pre-load boundary before exercising selection fallback.
    let error = match PrivatePayload::stage(case.root()) {
        Err(error) => error,
        Ok(_) => panic!("{input:?} passed pre-load private-payload validation"),
    };
    eprintln!(
        "optimization-runtime: {input:?} rejected before load ({:?})",
        error.kind()
    );
    assert_not_loaded(&candidate);
    let selection = RuntimeSelection::select(case.root(), &case.id, MountRuntimePreference::Auto)
        .unwrap_or_else(|error| panic!("{input:?} prevented official fallback: {error}"));
    assert_official(&selection, official_path);
    selection.complete();
    assert_marker(&case);
    assert_not_loaded(&candidate);
    assert_eq!(fs::read(case.root().join("sentinel.txt")).unwrap(), CANARY);
    if matches!(input, RejectedInput::FileSymlink | RejectedInput::HardLink) {
        assert_eq!(
            fs::read(case.root().join("link-target.dll")).unwrap(),
            BUNDLED_DOKANY_BYTES
        );
    }
    if matches!(input, RejectedInput::DirectorySymlink) {
        assert_eq!(
            fs::read(case.root().join("redirected/sentinel.txt")).unwrap(),
            CANARY
        );
        assert_eq!(
            fs::read_dir(case.root().join("redirected")).unwrap().count(),
            1
        );
    }
    case.finish();
}

fn install_rejected_input(case: &RuntimeCase, input: RejectedInput) {
    let candidate = case.payload_path();
    if matches!(input, RejectedInput::DirectorySymlink) {
        let redirected = case.root().join("redirected");
        fs::create_dir(&redirected).unwrap();
        fs::write(redirected.join("sentinel.txt"), CANARY).unwrap();
        symlink_dir(&redirected, case.root().join("private-runtime"))
            .expect("directory-reparse case needs runner symlink privilege; no skip permitted");
        assert_reparse(&case.root().join("private-runtime"));
        return;
    }
    fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    match input {
        RejectedInput::Tampered => {
            let mut bytes = BUNDLED_DOKANY_BYTES.to_vec();
            bytes[0] ^= 1;
            fs::write(candidate, bytes).unwrap();
        }
        RejectedInput::Truncated => {
            fs::write(
                candidate,
                &BUNDLED_DOKANY_BYTES[..BUNDLED_DOKANY_BYTES.len() / 2],
            )
            .unwrap();
        }
        RejectedInput::Directory => fs::create_dir(candidate).unwrap(),
        RejectedInput::FileSymlink | RejectedInput::HardLink => {
            let target = case.root().join("link-target.dll");
            fs::write(&target, BUNDLED_DOKANY_BYTES).unwrap();
            if matches!(input, RejectedInput::FileSymlink) {
                symlink_file(&target, &candidate)
                    .expect("file-reparse case needs runner symlink privilege; no skip permitted");
                assert_reparse(&candidate);
            } else {
                fs::hard_link(target, candidate)
                    .expect("single-link payload validation requires a hardlink fixture");
            }
        }
        RejectedInput::DirectorySymlink => unreachable!(),
    }
}

struct RuntimeCase {
    // Declaration order also keeps panic cleanup from dropping TempDir while
    // CacheLease still denies deletion of its directories.
    lease: CacheLease,
    id: MountId,
    directory: TempDir,
}

impl RuntimeCase {
    fn new(label: &str) -> Self {
        let directory = tempfile::Builder::new()
            .prefix(&format!("se-Opt-Runtime-{label}-"))
            .tempdir()
            .expect("create private local fixture directory");
        let id = MountId::new_random().unwrap();
        let lease = CacheLease::acquire(directory.path(), &id).unwrap();
        fs::write(directory.path().join("sentinel.txt"), CANARY).unwrap();
        Self {
            lease,
            id,
            directory,
        }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }

    fn payload_path(&self) -> PathBuf {
        self.root()
            .join("private-runtime")
            .join(format!(
                "{}-{}",
                BUNDLED_DOKANY_SHA256,
                runtime_notices::identity()
            ))
            .join("smart-explorer-dokan2.dll")
    }

    fn marker(&self) -> PathBuf {
        self.root()
            .join(self.id.as_str())
            .join(format!("private-{BUNDLED_DOKANY_SHA256}.attempt"))
    }

    fn finish(self) {
        drop(self.lease);
        self.directory
            .close()
            .expect("owned fixture cleanup must release every runtime/lease handle");
    }
}

fn assert_official(selection: &RuntimeSelection, official_path: &Path) {
    assert!(
        !selection.runtime.is_private(),
        "expected official non-batched selection"
    );
    assert_same_path(&selection.runtime.loaded_path().unwrap(), official_path);
}

fn assert_marker(case: &RuntimeCase) {
    assert_eq!(
        fs::read(case.marker()).unwrap(),
        BUNDLED_DOKANY_SHA256.as_bytes()
    );
}

fn assert_missing(path: &Path) {
    assert_eq!(
        fs::symlink_metadata(path).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

fn assert_reparse(path: &Path) {
    assert_ne!(
        fs::symlink_metadata(path).unwrap().file_attributes() & REPARSE_POINT,
        0
    );
}

fn assert_write_denied(path: &Path) {
    let error = OpenOptions::new().write(true).open(path).unwrap_err();
    assert!(
        matches!(error.raw_os_error(), Some(5) | Some(32)),
        "unexpected write-open failure: {error}"
    );
}

fn assert_not_loaded(path: &Path) {
    let wide = path
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == b'/' as u16 {
                b'\\' as u16
            } else {
                unit
            }
        })
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // GetModuleHandle does not increment ownership; never FreeLibrary this value.
    assert!(
        unsafe { GetModuleHandleW(wide.as_ptr()) }.is_null(),
        "candidate unexpectedly remains loaded"
    );
}

fn assert_same_path(actual: &Path, expected: &Path) {
    fn normalized(path: &Path) -> String {
        let value = path.to_str().unwrap().replace('/', "\\");
        value
            .strip_prefix("\\\\?\\")
            .unwrap_or(&value)
            .to_ascii_lowercase()
    }
    assert_eq!(normalized(actual), normalized(expected));
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
