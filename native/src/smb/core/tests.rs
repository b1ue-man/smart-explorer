//! Pure parts of the SMB backend: endpoint and path parsing, the directory
//! listing parser with its reparse attribute, the rename buffer, the error
//! mapping and the protocol/endpoint integration. Live SMB runs in the
//! Android device suite against Samba.
use super::errors::{
    connect_failed, is_dead, is_suspect, map, names_missing_share, negotiate_failed,
    session_failed, share_failed,
};
use super::listing::{
    filetime_ms, parse_directory_info, server_level, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT,
};
use super::replace::rename_info;
use super::url::{
    entry_path, host_of, parse_smb_url, rename_paths, server_addr, split_domain_user, split_path,
    SmbPath,
};
use super::{backend_from_url, root_has_share, root_with_share};
use crate::creds::Protocol;
use smb2::types::status::NtStatus;
use smb2::types::Command;
use smb2::Error;
use std::io;

/// 2020-01-01T00:00:00Z as a Windows FILETIME.
const JAN_2020: u64 = 132_223_104_000_000_000;

fn protocol_error(status: NtStatus, command: Command) -> Error {
    Error::Protocol { status, command }
}

/// One FileBothDirectoryInformation entry, padded to 8 bytes unless last.
fn entry(name: &str, attributes: u32, size: u64, last: bool) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let length = 94 + units.len() * 2;
    let padded = (length + 7) & !7;
    let mut bytes = vec![0u8; if last { length } else { padded }];
    let next = if last { 0 } else { padded as u32 };
    bytes[0..4].copy_from_slice(&next.to_le_bytes());
    bytes[8..16].copy_from_slice(&JAN_2020.to_le_bytes());
    bytes[24..32].copy_from_slice(&JAN_2020.to_le_bytes());
    bytes[40..48].copy_from_slice(&size.to_le_bytes());
    bytes[56..60].copy_from_slice(&attributes.to_le_bytes());
    bytes[60..64].copy_from_slice(&((units.len() * 2) as u32).to_le_bytes());
    for (index, unit) in units.iter().enumerate() {
        bytes[94 + index * 2..96 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    bytes
}

#[test]
fn android_task_smb_url_parses_user_host_port_share_and_path() {
    let full = parse_smb_url("smb://FIRMA\\anna:ge:heim@nas.local:1445/daten/projekte/").unwrap();
    assert_eq!(full.user, "FIRMA\\anna");
    assert_eq!(full.password.as_deref(), Some("ge:heim"));
    assert_eq!((full.host.as_str(), full.port), ("nas.local", 1445));
    assert_eq!(full.root, "/daten/projekte");

    let bare = parse_smb_url("SMB://nas").unwrap();
    assert_eq!((bare.user.as_str(), bare.password), ("", None));
    assert_eq!((bare.port, bare.root.as_str()), (445, "/"));

    let v6 = parse_smb_url("smb://u@[fe80::1]:446/s").unwrap();
    assert_eq!((v6.host.as_str(), v6.port), ("fe80::1", 446));
    assert_eq!(parse_smb_url("smb://u@fe80::1/s").unwrap().host, "fe80::1");

    for bad in [
        "sftp://u@h/s",
        "smb://u@h:abc/s",
        "smb://u@:445/s",
        "smb://u@h:0/s",
    ] {
        assert!(parse_smb_url(bad).is_err(), "{bad}");
    }
}

#[test]
fn android_task_smb_paths_split_into_share_and_share_relative_path() {
    assert_eq!(split_path("/").unwrap(), None);
    assert_eq!(split_path("").unwrap(), None);
    let share = |share: &str, rel: &str| {
        Some(SmbPath {
            share: share.into(),
            rel: rel.into(),
        })
    };
    assert_eq!(split_path("/daten").unwrap(), share("daten", ""));
    assert_eq!(split_path("/daten/a/b/").unwrap(), share("daten", "a/b"));
    assert_eq!(split_path("/daten//a/./b").unwrap(), share("daten", "a/b"));
    // Literal names stay literal (trailing blanks, reserved characters).
    assert_eq!(
        split_path("/d/Bericht /a?").unwrap(),
        share("d", "Bericht /a?")
    );
    assert!(split_path("/daten/../x").is_err());
    assert!(split_path("/da\\ten/x").is_err());

    assert_eq!(entry_path("/d/x").unwrap().rel, "x");
    assert_eq!(
        entry_path("/d").unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        entry_path("/").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );

    // Renames stay within one share (share names are case-insensitive).
    let (from, to) = rename_paths("/daten/a.txt", "/DATEN/b.txt").unwrap();
    assert_eq!((from.rel.as_str(), to.rel.as_str()), ("a.txt", "b.txt"));
    let across = rename_paths("/daten/a.txt", "/andere/a.txt").unwrap_err();
    assert_eq!(across.kind(), io::ErrorKind::Unsupported);
}

#[test]
fn android_task_smb_domain_user_address_and_saved_root() {
    assert_eq!(
        split_domain_user("FIRMA\\anna"),
        ("FIRMA".to_string(), "anna".to_string())
    );
    assert_eq!(
        split_domain_user("anna"),
        (String::new(), "anna".to_string())
    );
    assert_eq!(
        split_domain_user("anna@firma.example"),
        (String::new(), "anna@firma.example".to_string())
    );
    assert_eq!(server_addr("nas", 445), "nas:445");
    assert_eq!(server_addr("fe80::1", 445), "[fe80::1]:445");
    assert_eq!(server_addr("[::1]", 1445), "[::1]:1445");
    assert_eq!(host_of("[fe80::1]:445"), "fe80::1");
    assert_eq!(host_of("10.0.2.2:1445"), "10.0.2.2");
    assert_eq!(host_of("nas"), "nas");

    assert!(root_has_share("/daten/x"));
    assert!(!root_has_share("/"));
    // The server level has no session of its own: open at the saved share.
    assert_eq!(root_with_share("/", "/daten/start/"), "/daten/start");
    assert_eq!(root_with_share("/andere/y", "/daten"), "/andere/y");
}

#[test]
fn android_task_smb_directory_listing_keeps_reparse_points_as_links() {
    let mut buffer = Vec::new();
    buffer.extend(entry(".", FILE_ATTRIBUTE_DIRECTORY, 0, false));
    buffer.extend(entry("..", FILE_ATTRIBUTE_DIRECTORY, 0, false));
    buffer.extend(entry("bericht.txt", 0x20, 12, false));
    buffer.extend(entry("ordner", FILE_ATTRIBUTE_DIRECTORY, 4096, false));
    buffer.extend(entry(
        "verknüpfung",
        FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
        0,
        false,
    ));
    buffer.extend(entry("versteckt", FILE_ATTRIBUTE_HIDDEN, 1, false));
    // smb2 maps `?` to U+F025 on the wire; the listing shows the real name.
    buffer.extend(entry("frage\u{F025}", 0x20, 3, true));

    let entries = parse_directory_info(&buffer).unwrap();
    let names: Vec<&str> = entries.iter().map(|meta| meta.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "bericht.txt",
            "ordner",
            "verknüpfung",
            "versteckt",
            "frage?"
        ]
    );
    let file = &entries[0];
    assert!(!file.is_dir && !file.is_symlink && !file.hidden);
    assert_eq!((file.size, file.mtime_ms), (12, 1_577_836_800_000));
    assert_eq!(file.btime_ms, 1_577_836_800_000);
    let folder = &entries[1];
    assert!(folder.is_dir && !folder.is_symlink);
    assert_eq!(folder.size, 0);
    // A junction/symlink is a link that recursive delete and sync never enter.
    let link = &entries[2];
    assert!(link.is_dir && link.is_symlink);
    assert!(entries[3].hidden);

    assert!(parse_directory_info(&[]).unwrap().is_empty());
    assert_eq!(filetime_ms(0), 0);
}

#[test]
fn android_task_smb_directory_listing_rejects_malformed_entries() {
    let whole = entry("name", 0, 1, true);
    assert!(parse_directory_info(&whole[..60]).is_err());
    assert!(parse_directory_info(&whole[..whole.len() - 1]).is_err());
    let mut odd = whole.clone();
    odd[60..64].copy_from_slice(&7u32.to_le_bytes());
    assert!(parse_directory_info(&odd).is_err());
    // A next offset pointing into the current name or past the buffer.
    let mut overlap = entry("name", 0, 1, false);
    overlap[0..4].copy_from_slice(&8u32.to_le_bytes());
    assert!(parse_directory_info(&overlap).is_err());
    let beyond = entry("name", 0, 1, false);
    assert!(parse_directory_info(&beyond).is_err());
    let mut surrogate = entry("ab", 0, 1, true);
    surrogate[94..96].copy_from_slice(&0xD800u16.to_le_bytes());
    assert!(parse_directory_info(&surrogate).is_err());
}

#[test]
fn android_task_smb_server_level_lists_only_the_configured_share() {
    let root = server_level("seshare");
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].name, "seshare");
    assert!(root[0].is_dir && !root[0].is_symlink);
}

#[test]
fn android_task_smb_rename_information_buffer_layout() {
    let target = "ordner\\neu.txt";
    let replace = rename_info(target, true);
    assert_eq!(replace.len(), 20 + target.len() * 2);
    assert_eq!(replace[0], 1, "ReplaceIfExists");
    assert_eq!(&replace[1..16], &[0u8; 15], "reserved + RootDirectory");
    assert_eq!(&replace[16..20], &28u32.to_le_bytes(), "FileNameLength");
    assert_eq!(&replace[20..24], &[b'o', 0, b'r', 0]);
    assert_eq!(&replace[32..34], &[b'\\', 0]);
    let keep = rename_info(target, false);
    assert_eq!(keep[0], 0);
    assert_eq!(&keep[1..], &replace[1..]);
    // Non-ASCII names are UTF-16LE code units.
    assert_eq!(&rename_info("ä", true)[16..22], &[2, 0, 0, 0, 0xE4, 0]);
}

#[test]
fn android_task_smb_connect_errors_keep_their_meaning() {
    let refused = session_failed(protocol_error(
        NtStatus::LOGON_FAILURE,
        Command::SessionSetup,
    ));
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert!(refused.to_string().contains("Anmeldung"));
    let guest = session_failed(Error::Auth {
        message: "guest session for a named account".into(),
    });
    assert_eq!(guest.kind(), io::ErrorKind::PermissionDenied);
    let denied = session_failed(protocol_error(
        NtStatus::ACCESS_DENIED,
        Command::SessionSetup,
    ));
    assert!(denied.to_string().contains("Anmeldung"));
    let dropped = session_failed(Error::Disconnected);
    assert_eq!(dropped.kind(), io::ErrorKind::ConnectionAborted);
    assert!(!dropped.to_string().to_lowercase().contains("anmeld"));

    let missing = share_failed(
        "gibt-es-nicht",
        protocol_error(NtStatus::BAD_NETWORK_NAME, Command::TreeConnect),
    );
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    assert!(missing.to_string().contains("gibt-es-nicht"));
    assert!(names_missing_share(&missing.to_string()));
    assert!(!names_missing_share(&refused.to_string()));
    let forbidden = share_failed(
        "s",
        protocol_error(NtStatus::ACCESS_DENIED, Command::TreeConnect),
    );
    assert_eq!(forbidden.kind(), io::ErrorKind::PermissionDenied);

    let smb1 = negotiate_failed(Error::Disconnected);
    assert_eq!(smb1.kind(), io::ErrorKind::Unsupported);
    assert!(smb1.to_string().contains("SMB1"));
    assert_eq!(
        negotiate_failed(Error::Timeout).kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(
        connect_failed("nas", 445, Error::Timeout).kind(),
        io::ErrorKind::TimedOut
    );
    let unreachable = connect_failed("nas", 445, Error::Disconnected);
    assert_eq!(unreachable.kind(), io::ErrorKind::ConnectionRefused);
    assert!(unreachable.to_string().contains("nas:445"));
}

#[test]
fn android_task_smb_operation_errors_map_to_io_kinds() {
    let cases = [
        (NtStatus::OBJECT_NAME_NOT_FOUND, io::ErrorKind::NotFound),
        (NtStatus::OBJECT_PATH_NOT_FOUND, io::ErrorKind::NotFound),
        (
            NtStatus::OBJECT_NAME_COLLISION,
            io::ErrorKind::AlreadyExists,
        ),
        (NtStatus::ACCESS_DENIED, io::ErrorKind::PermissionDenied),
        (
            NtStatus::DIRECTORY_NOT_EMPTY,
            io::ErrorKind::DirectoryNotEmpty,
        ),
        (NtStatus::NOT_SUPPORTED, io::ErrorKind::Unsupported),
        (NtStatus::FILE_IS_A_DIRECTORY, io::ErrorKind::InvalidInput),
        (NtStatus::SHARING_VIOLATION, io::ErrorKind::ResourceBusy),
        (NtStatus::DISK_FULL, io::ErrorKind::StorageFull),
    ];
    for (status, kind) in cases {
        let error = map(protocol_error(status, Command::Create), "Test", "/s/x");
        assert_eq!(error.kind(), kind, "{status:?}");
        assert!(error.to_string().contains("/s/x"));
    }
    assert_eq!(
        map(Error::Disconnected, "Test", "/s").kind(),
        io::ErrorKind::ConnectionAborted
    );
    let reset = Error::Io(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
    assert_eq!(
        map(reset, "Test", "/s").kind(),
        io::ErrorKind::ConnectionReset
    );

    // A lost connection is replaced (reads replay); a timeout only retires it.
    assert!(is_dead(&Error::Disconnected));
    assert!(is_dead(&Error::Io(io::ErrorKind::BrokenPipe.into())));
    assert!(is_dead(&protocol_error(
        NtStatus::NETWORK_NAME_DELETED,
        Command::Create
    )));
    assert!(!is_dead(&Error::Timeout) && is_suspect(&Error::Timeout));
    assert!(!is_dead(&protocol_error(
        NtStatus::OBJECT_NAME_NOT_FOUND,
        Command::Create
    )));
}

#[test]
fn android_task_smb_urls_without_credentials_or_share_never_connect() {
    let anonymous = backend_from_url("smb://anna@nas.invalid/daten")
        .err()
        .unwrap();
    assert_eq!(anonymous.kind(), io::ErrorKind::PermissionDenied);
    // A root without a share is refused before any network access.
    let no_share = backend_from_url("smb://anna:pw@127.0.0.1:9/")
        .err()
        .unwrap();
    assert_eq!(no_share.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn android_task_smb_protocol_and_endpoints_are_recognized() {
    assert_eq!(Protocol::parse("smb"), Some(Protocol::Smb));
    assert_eq!(Protocol::Smb.as_str(), "smb");
    assert_eq!(Protocol::Smb.default_port(), 445);
    assert!(Protocol::Smb.is_url());

    let endpoint = "smb://FIRMA\\anna@nas:1445/daten/x";
    assert!(crate::connect::is_remote_url(endpoint));
    let (protocol, user, host, port, path) = crate::connect::parse_remote_url(endpoint).unwrap();
    assert_eq!(protocol, Protocol::Smb);
    assert_eq!(
        (user.as_str(), host.as_str(), port),
        ("FIRMA\\anna", "nas", 1445)
    );
    assert_eq!(path, "/daten/x");
    crate::connect::validate_sync_endpoints(endpoint, "/storage/emulated/0/x").unwrap();
    assert!(
        crate::connect::validate_sync_endpoints(endpoint, "smb://FIRMA\\anna@nas:1445/daten")
            .is_err()
    );
}
