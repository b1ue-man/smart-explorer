//! Acceptance signal `drive_task_ipc_mount_types_serde_unchanged` (plan
//! 2026-10-17 „Rechte, se Desktop, Tresore“, package R1): the drive-host
//! types moved out of `ipc_protocol.rs` keep the serde form of v0.5.182.
//!
//! Every file in `ipc_protocol_mount_fixtures/` holds the compact JSON that
//! the type definitions of commit `da2646d2` produce: `MountBackendScheme`,
//! two `MountHostConfig` values and the `MountHostReady` answer that carries
//! both; `mount_host_config_without_defaults.json` is the older input without
//! the defaulted policy fields. The only addition since that commit is
//! `MountHostConfig::path_dialect`: input without it stays readable and means
//! the Windows rules, and output appends it after `label` while every earlier
//! byte stays as it was.
use super::super::{IpcResponse, MountDeleteDisposition};
use super::{MountBackendScheme, MountHostConfig};
use crate::mount::{
    DriveLetter, DriveSelection, MountCachePolicy, MountMetadataPolicy, MountMode,
    MountRuntimePreference, PathDialect,
};

const SCHEMES: &str = include_str!("ipc_protocol_mount_fixtures/mount_backend_schemes.json");
const CONFIG_AUTOMATIC: &str =
    include_str!("ipc_protocol_mount_fixtures/mount_host_config_automatic.json");
const CONFIG_LETTER: &str =
    include_str!("ipc_protocol_mount_fixtures/mount_host_config_letter.json");
const CONFIG_WITHOUT_DEFAULTS: &str =
    include_str!("ipc_protocol_mount_fixtures/mount_host_config_without_defaults.json");
const HOST_READY: &str = include_str!("ipc_protocol_mount_fixtures/mount_host_ready.json");

#[test]
fn drive_task_ipc_mount_types_serde_unchanged() {
    backend_scheme_names_and_mapping();
    host_config_round_trip();
    host_ready_round_trip();
}

/// Fixture text without the line break at the end of the file (a checkout
/// with CRLF line endings ends in `\r\n`).
fn fixture(text: &str) -> &str {
    text.trim_end()
}

fn encode<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

/// The v0.5.182 bytes of one `MountHostConfig` plus the field added after
/// that release, appended after `label`.
fn with_dialect(config: &str) -> String {
    let body = config
        .strip_suffix('}')
        .expect("a MountHostConfig fixture is one JSON object");
    let dialect = encode(&PathDialect::default());
    format!("{body},\"path_dialect\":{dialect}}}")
}

/// Wire names in declaration order, and the total mapping to and from
/// `vfs::Scheme`.
fn backend_scheme_names_and_mapping() {
    let table = [
        (crate::vfs::Scheme::Local, MountBackendScheme::Local),
        (crate::vfs::Scheme::Sftp, MountBackendScheme::Sftp),
        (crate::vfs::Scheme::Ftp, MountBackendScheme::Ftp),
        (crate::vfs::Scheme::Webdav, MountBackendScheme::Webdav),
        (crate::vfs::Scheme::Smb, MountBackendScheme::Smb),
        (crate::vfs::Scheme::GDrive, MountBackendScheme::GoogleDrive),
        (crate::vfs::Scheme::Peer, MountBackendScheme::Peer),
    ];
    let wire: Vec<MountBackendScheme> = table.iter().map(|(_, wire)| *wire).collect();
    assert_eq!(encode(&wire), fixture(SCHEMES));
    let decoded: Vec<MountBackendScheme> = serde_json::from_str(fixture(SCHEMES)).unwrap();
    assert_eq!(encode(&decoded), fixture(SCHEMES));
    for (scheme, wire) in table {
        assert_eq!(crate::vfs::Scheme::from(wire), scheme, "{wire:?}");
        let mapped = MountBackendScheme::from(scheme);
        assert_eq!(encode(&mapped), encode(&wire), "{scheme:?}");
    }
}

fn host_config_round_trip() {
    // Automatic drive, writable, every policy at its default; the label needs
    // JSON escapes and holds a non-ASCII letter.
    let automatic: MountHostConfig = serde_json::from_str(fixture(CONFIG_AUTOMATIC)).unwrap();
    assert_eq!(automatic.id.as_str(), "r1-fixture-drive");
    assert_eq!(automatic.drive, DriveSelection::Automatic);
    assert_eq!(automatic.mode, MountMode::ReadWrite);
    assert_eq!(automatic.metadata, MountMetadataPolicy::default());
    assert_eq!(automatic.cache, MountCachePolicy::default());
    assert_eq!(automatic.runtime_preference, MountRuntimePreference::Auto);
    assert_eq!(automatic.label, "Projekt \"Ä\" \\ Backup");
    assert_eq!(automatic.path_dialect, PathDialect::Windows);
    assert_eq!(encode(&automatic), with_dialect(fixture(CONFIG_AUTOMATIC)));

    // Drive letter, read-only, every policy away from its default.
    let letter: MountHostConfig = serde_json::from_str(fixture(CONFIG_LETTER)).unwrap();
    assert_eq!(letter.id.as_str(), "0123456789abcdef0123456789abcdef");
    let z = DriveLetter::parse('Z').unwrap();
    assert_eq!(letter.drive, DriveSelection::Letter(z));
    assert_eq!(letter.mode, MountMode::ReadOnly);
    assert_eq!(letter.metadata, MountMetadataPolicy::new(0).unwrap());
    assert_eq!(letter.cache, MountCachePolicy::new(65_536).unwrap());
    assert_eq!(letter.runtime_preference, MountRuntimePreference::System);
    assert_eq!(letter.label, "Fotos");
    assert_eq!(letter.path_dialect, PathDialect::Windows);
    assert_eq!(encode(&letter), with_dialect(fixture(CONFIG_LETTER)));

    // An older daemon sent no policy fields: the defaults fill them, and the
    // output is the full form.
    let older: MountHostConfig = serde_json::from_str(fixture(CONFIG_WITHOUT_DEFAULTS)).unwrap();
    assert_eq!(older, automatic);
    assert_eq!(encode(&older), with_dialect(fixture(CONFIG_AUTOMATIC)));

    // The new field keeps a value other than the default as well.
    let mut posix = letter;
    posix.path_dialect = PathDialect::Posix;
    let decoded: MountHostConfig = serde_json::from_str(&encode(&posix)).unwrap();
    assert_eq!(decoded, posix);
}

/// The drive-host handshake answer still carries both moved types at the
/// old place and in the old bytes.
fn host_ready_round_trip() {
    let ready = fixture(HOST_READY);
    let config = fixture(CONFIG_LETTER);
    assert!(
        ready.contains(config),
        "the answer embeds the letter fixture"
    );
    let decoded: IpcResponse = serde_json::from_str(ready).unwrap();
    let IpcResponse::MountHostReady {
        config: decoded_config,
        scheme,
        capabilities,
        session_token,
        backend_token,
    } = &decoded
    else {
        panic!("fixture is not MountHostReady: {decoded:?}");
    };
    let expected: MountHostConfig = serde_json::from_str(config).unwrap();
    assert_eq!(decoded_config, &expected);
    assert_eq!(encode(scheme), "\"google_drive\"");
    assert!(!capabilities.rename_overwrites);
    assert!(capabilities.staged_write_create);
    assert!(capabilities.staged_write_replace);
    assert!(!capabilities.staged_namespace_replace);
    assert!(capabilities.case_sensitive_paths);
    assert!(matches!(
        capabilities.delete_disposition,
        MountDeleteDisposition::Recycle
    ));
    assert_eq!(capabilities.parallelism, 8);
    assert_eq!(session_token, "session-fixture-token");
    assert_eq!(backend_token, "backend-fixture-token");
    assert_eq!(
        encode(&decoded),
        ready.replacen(config, &with_dialect(config), 1)
    );
}
