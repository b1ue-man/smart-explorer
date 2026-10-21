//! Signal `drive_task_cli_error_texts_unchanged` (DRV-M10, "Bestandsverträge"
//! of strand S2): the error texts of the `se drive` commands that scripts see
//! stay word for word what they are today (the list of the "Kern-Lesung",
//! section 5, and the refusals of the platform options).
//!
//! A text the commands build before they ask the background worker comes
//! from the real function. A text that exists only after a request to the
//! worker cannot be produced here: such a request starts the worker, and a
//! unit test must not start one. Those texts are held as they are written in
//! the sources of the commands (`texts_after_a_worker_request`).
use std::path::Path;

use super::super::{parse_drive_selection, vault_source};
use super::{absolute_folder, select_longest_root, source_from_spec};
use crate::creds::{AuthKind, Protocol, SavedConnection};
use crate::mount::{DriveLetter, DriveSelection};

/// The error of a call that must fail.
fn failure<T>(result: Result<T, String>) -> String {
    match result {
        Ok(_) => panic!("the call was expected to fail"),
        Err(text) => text,
    }
}

fn connection(label: &str, user: &str, host: &str) -> SavedConnection {
    SavedConnection {
        protocol: Protocol::Sftp,
        host: host.into(),
        port: 22,
        user: user.into(),
        auth: AuthKind::Password,
        root: "/srv/data".into(),
        label: label.into(),
        use_agent: false,
    }
}

/// Two saved remotes that authorize the same path equally well.
fn equally_good() -> Vec<SavedConnection> {
    vec![
        connection("one", "alice", "one.example.test"),
        connection("two", "bob", "two.example.test"),
    ]
}

#[test]
fn drive_task_cli_error_texts_unchanged() {
    drive_letter_texts();
    source_texts();
    folder_and_vault_texts();
    platform_texts();
    texts_after_a_worker_request();
}

/// `--letter`: `auto` or one ASCII letter, with or without the colon.
fn drive_letter_texts() {
    assert_eq!(failure(parse_drive_selection("")), "drive letter is empty");
    assert_eq!(
        failure(parse_drive_selection(" : ")),
        "drive letter is empty"
    );
    assert_eq!(
        failure(parse_drive_selection("MM")),
        "drive letter must be `auto` or one ASCII letter"
    );
    assert_eq!(
        failure(parse_drive_selection("1")),
        "drive letter must be an ASCII letter"
    );
    let m = DriveSelection::Letter(DriveLetter::parse('M').unwrap());
    assert_eq!(
        parse_drive_selection(" AUTO "),
        Ok(DriveSelection::Automatic)
    );
    assert_eq!(parse_drive_selection("m:"), Ok(m));
    assert_eq!(parse_drive_selection("M\\"), Ok(m));
}

/// `<TARGET>` of `se drive mount`.
fn source_texts() {
    assert_eq!(failure(source_from_spec("  ")), "drive source is empty");
    assert_eq!(
        failure(source_from_spec("just-a-folder")),
        "only saved remotes, Google Drive, and Smart Explorer Share peers can be mounted"
    );
    assert_eq!(
        failure(source_from_spec("gdrive://work/../etc")),
        "backend root may not contain '.' or '..'"
    );
    // The two texts about saved connections read the store of saved
    // connections, never write it. The host of the probe is reserved for
    // tests (RFC 6761): no real connection names it.
    assert!(
        crate::creds::load_connections_checked().is_ok(),
        "the store of saved connections cannot be read (fixture home of the stage?)"
    );
    assert_eq!(
        failure(source_from_spec(
            "sftp://nobody@no-such-host.invalid:22/data"
        )),
        "no saved connection authorizes that remote path; save the remote first"
    );
    assert_eq!(
        failure(source_from_spec("@no-such-label:/data")),
        "unknown saved connection shorthand: @no-such-label:/data"
    );
    // Equal roots of two accounts are ambiguous; both are named in the order
    // they were found.
    let accounts = equally_good()
        .iter()
        .map(SavedConnection::account)
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        failure(select_longest_root(equally_good().into_iter(), false)),
        format!("ambiguous saved remote URL: {accounts}")
    );
    assert_eq!(
        failure(select_longest_root(equally_good().into_iter(), true)),
        format!("ambiguous saved network share: {accounts}")
    );
}

/// `--mount-point` and the vault as a drive source.
fn folder_and_vault_texts() {
    assert_eq!(
        failure(absolute_folder(Path::new("/srv/../etc"))),
        "invalid --mount-point: drive location may not contain '.' or '..'"
    );
    assert_eq!(
        failure(absolute_folder(Path::new("/"))),
        "invalid --mount-point: drive location may not be a filesystem root"
    );
    assert_eq!(
        failure(vault_source(
            "vault-store://0123456789abcdef0123456789abcdef"
        )),
        "vault-store:// lässt sich nicht einbinden; nutze vault://<SITE>"
    );
    assert_eq!(
        failure(vault_source(
            "vault://0123456789abcdef0123456789abcdef/Ordner"
        )),
        "Ein Tresor-Laufwerk zeigt immer den ganzen Tresor: vault://<SITE>"
    );
}

/// The refusal of a Windows-only option under Linux (exit code 2).
#[cfg(target_os = "linux")]
fn platform_texts() {
    use super::super::drive_output_linux::{refuse_windows_options, LETTERS_ONLY_ON_WINDOWS};

    assert_eq!(
        LETTERS_ONLY_ON_WINDOWS,
        "Laufwerksbuchstaben gibt es nur unter Windows; nutze --mount-point"
    );
    assert_eq!(
        refuse_windows_options("M", false),
        Some(LETTERS_ONLY_ON_WINDOWS)
    );
    assert_eq!(
        refuse_windows_options("auto", true),
        Some(LETTERS_ONLY_ON_WINDOWS)
    );
    assert_eq!(refuse_windows_options("auto", false), None);
}

/// Windows has no refusal function to call; the texts are held in the
/// sources (`texts_after_a_worker_request`).
#[cfg(not(target_os = "linux"))]
fn platform_texts() {}

/// Texts that exist only after a request to the worker, or that the code
/// prints itself, as they are written in the sources.
fn texts_after_a_worker_request() {
    const DRIVE: &str = include_str!("drive.rs");
    const OPTIONS: &str = include_str!("drive_options.rs");
    const OPEN: &str = include_str!("drive_open.rs");
    const DEVICES: &str = include_str!("drive_devices.rs");
    let held = [
        (
            "drive.rs",
            DRIVE,
            r#""no managed drive is mounted at {wanted}""#,
        ),
        (
            "drive.rs",
            DRIVE,
            r#""drive disappeared while it was mounting""#,
        ),
        (
            "drive.rs",
            DRIVE,
            r#""drive {} did not finish mounting within {} seconds""#,
        ),
        (
            "drive.rs",
            DRIVE,
            r#""se: Einhängeorte gibt es nur unter Linux; nutze --letter""#,
        ),
        (
            "drive_options.rs",
            OPTIONS,
            r#""no saved network-share connection authorizes that UNC path; save it first""#,
        ),
        ("drive_open.rs", OPEN, r#""unknown drive {id}""#),
        (
            "drive_open.rs",
            OPEN,
            r#""Das Laufwerk ist nicht eingehängt""#,
        ),
        (
            "drive_devices.rs",
            DEVICES,
            r#""Einhängeorte gibt es nur unter Linux; nutze --letter""#,
        ),
        (
            "drive_devices.rs",
            DEVICES,
            r#""Laufwerksbuchstaben gibt es nur unter Windows; nutze --mount-point""#,
        ),
    ];
    for (file, source, text) in held {
        assert!(source.contains(text), "{file} no longer holds {text}");
    }
}
