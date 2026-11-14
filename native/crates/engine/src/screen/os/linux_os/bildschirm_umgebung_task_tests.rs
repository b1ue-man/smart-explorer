//! Task tests of E47 (L4): a worker that follows a session takes its window
//! list, icons, encoder offer and portal tokens from the session's account
//! (simulated session environment), and reads no file of that account that
//! another owner holds or that lies behind a link.
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use super::FollowedAccount;
use crate::screen::core::negotiate::{CodecOffer, H264Profile};
use crate::screen::core::window_list::{AppEntry, ListSource, WindowList};
use crate::screen::core::wire::{from_json, to_json};
use crate::screen::os::host_backend::followed::windows_from;
use crate::screen::os::host_backend::{LIMIT_NO_WINDOW_LIST, LIMIT_WINDOW_PICK_ON_HOST};
use crate::screen::os::host_worker_client::session_queries::{
    command_for, offers_of, serve, SessionAnswer, SessionQuery, QUERY_ENV,
};
use crate::screen::os::portal_tokens::TokenKind;
use crate::screen::os::session_detect::active_session::{Account, ActiveSession};
use crate::screen::os::session_detect::SessionKind;
use crate::screen::os::synthetic_backend::SyntheticBackend;

const TOKENS: &str = r#"{"version":1,"screencast":{"token":"granted","saved_at":1}}"#;

fn own_uid() -> u32 {
    // SAFETY: geteuid takes no arguments and cannot fail.
    unsafe { libc::geteuid() }
}

fn account_list() -> WindowList {
    WindowList {
        windows: Vec::new(),
        apps: vec![AppEntry {
            app_id: "org.example.Editor".into(),
            name: "Editor".into(),
            icon: None,
            pid: None,
        }],
        list_source: ListSource::SystemdUnits,
        limits: vec![LIMIT_WINDOW_PICK_ON_HOST.into()],
    }
}

/// A home with the token file at the place of the program's data.
fn home_with_tokens() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(super::APP_DATA);
    fs::create_dir_all(&data).unwrap();
    fs::write(
        data.join(crate::screen::os::portal_tokens::FILE_NAME),
        TOKENS,
    )
    .unwrap();
    let path = home.path().to_path_buf();
    (home, path)
}

fn has_screencast(account: &FollowedAccount) -> bool {
    account.tokens().get(TokenKind::ScreenCast).is_some()
}

#[test]
fn bildschirm_sitzung_task_fensterliste_kommt_aus_dem_sitzungskonto() {
    let asked = std::cell::Cell::new(0);
    let list = windows_from(SessionKind::Wayland, || {
        asked.set(asked.get() + 1);
        Ok(SessionAnswer::Windows(account_list()))
    });
    assert_eq!(asked.get(), 1);
    assert_eq!(
        list,
        account_list(),
        "the account's list, limits not doubled"
    );
    let failed = windows_from(SessionKind::Wayland, || {
        Ok(SessionAnswer::Failed("no compositor".into()))
    });
    assert_eq!(failed.list_source, ListSource::Unknown);
    assert_eq!(
        failed.limits,
        vec![
            LIMIT_WINDOW_PICK_ON_HOST.to_string(),
            LIMIT_NO_WINDOW_LIST.to_string()
        ]
    );
    let none = windows_from(SessionKind::None, || {
        asked.set(asked.get() + 1);
        Ok(SessionAnswer::Windows(account_list()))
    });
    assert_eq!(asked.get(), 1, "no session: no query process");
    assert_eq!(none.limits, vec![LIMIT_NO_WINDOW_LIST.to_string()]);
}

#[test]
fn bildschirm_sitzung_task_abfrage_startet_mit_der_umgebung_des_sitzungskontos() {
    let account = Account {
        uid: 1000,
        gid: 1000,
        name: "person".into(),
        home: "/home/person".into(),
        shell: "/bin/sh".into(),
        groups: vec![1000],
    };
    let env = vec![
        ("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string()),
        ("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string()),
        (
            "DBUS_SESSION_BUS_ADDRESS".to_string(),
            "unix:path=/run/user/1000/bus".to_string(),
        ),
    ];
    let session = ActiveSession::simulated(SessionKind::Wayland, account, env);
    let command = command_for(Path::new("/opt/se/smart_explorer"), &session);
    let envs: Vec<(String, Option<String>)> = command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    let value = |name: &str| {
        envs.iter()
            .rev()
            .find(|(known, _)| known == name)
            .and_then(|(_, value)| value.clone())
    };
    assert_eq!(value("WAYLAND_DISPLAY").as_deref(), Some("wayland-0"));
    assert_eq!(
        value("DBUS_SESSION_BUS_ADDRESS").as_deref(),
        Some("unix:path=/run/user/1000/bus")
    );
    assert_eq!(value("XDG_RUNTIME_DIR").as_deref(), Some("/run/user/1000"));
    assert_eq!(value("HOME").as_deref(), Some("/home/person"));
    assert_eq!(value(QUERY_ENV).as_deref(), Some("1"));
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args, ["--screen-host-worker"]);
}

#[test]
fn bildschirm_sitzung_task_kodiererangebot_kommt_aus_dem_sitzungskonto() {
    let h264 = CodecOffer::h264(H264Profile::ConstrainedBaseline, false);
    assert_eq!(
        offers_of(Ok(SessionAnswer::Encoders(vec![h264]))),
        vec![h264, CodecOffer::tiles()]
    );
    assert_eq!(
        offers_of(Err(std::io::Error::other("no answer"))),
        vec![CodecOffer::tiles()]
    );
    // The query process answers with its own (the account's) inventory.
    let mut out = Vec::new();
    let code = serve(
        to_json(&SessionQuery::Encoders).unwrap().as_slice(),
        &mut out,
        &SyntheticBackend,
        || vec![h264],
    );
    assert_eq!(code, 0);
    let answer: SessionAnswer = from_json(&out).unwrap();
    assert_eq!(answer, SessionAnswer::Encoders(vec![h264]));
    let mut out = Vec::new();
    serve(
        to_json(&SessionQuery::Windows).unwrap().as_slice(),
        &mut out,
        &SyntheticBackend,
        Vec::new,
    );
    let answer: SessionAnswer = from_json(&out).unwrap();
    let expected =
        crate::screen::core::host_backend::ScreenHostBackend::windows(&SyntheticBackend).unwrap();
    assert_eq!(answer, SessionAnswer::Windows(expected));
}

#[test]
fn bildschirm_sitzung_task_fremde_oder_verlinkte_tokendatei_wird_nicht_gelesen() {
    let (_keep, home) = home_with_tokens();
    let own = FollowedAccount::new(own_uid(), &home).unwrap();
    assert!(has_screencast(&own), "the account's own file is read");
    let foreign = FollowedAccount::new(own_uid().wrapping_add(1), &home).unwrap();
    assert!(
        !has_screencast(&foreign),
        "a file of another owner is not read"
    );
    // The token file is a link to a file elsewhere.
    let (_other, elsewhere) = home_with_tokens();
    let (_keep2, linked_file) = home_with_tokens();
    let file = linked_file
        .join(super::APP_DATA)
        .join(crate::screen::os::portal_tokens::FILE_NAME);
    fs::remove_file(&file).unwrap();
    symlink(
        elsewhere
            .join(super::APP_DATA)
            .join(crate::screen::os::portal_tokens::FILE_NAME),
        &file,
    )
    .unwrap();
    let account = FollowedAccount::new(own_uid(), &linked_file).unwrap();
    assert!(!has_screencast(&account), "a linked token file is not read");
    // A directory on the way is a link out of the home.
    let (_keep3, linked_dir) = home_with_tokens();
    let share = linked_dir.join(".local/share");
    fs::remove_dir_all(&share).unwrap();
    symlink(elsewhere.join(".local/share"), &share).unwrap();
    let account = FollowedAccount::new(own_uid(), &linked_dir).unwrap();
    assert!(
        !has_screencast(&account),
        "a linked directory is not followed"
    );
    assert!(FollowedAccount::new(own_uid(), "relative/home").is_none());
    assert!(FollowedAccount::new(own_uid(), "/home/../root").is_none());
}

#[test]
fn bildschirm_sitzung_task_fremdes_oder_verlinktes_symbolverzeichnis_wird_nicht_genutzt() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".local/share");
    fs::create_dir_all(data.join("icons")).unwrap();
    let system = PathBuf::from("/usr/share");
    let dirs = vec![data.clone(), data.join("icons"), system.clone()];
    let own = FollowedAccount::new(own_uid(), home.path()).unwrap();
    assert_eq!(own.trusted_dirs(dirs.clone()), dirs);
    let foreign = FollowedAccount::new(own_uid().wrapping_add(1), home.path()).unwrap();
    assert_eq!(foreign.trusted_dirs(dirs.clone()), vec![system.clone()]);
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(outside.path().join("icons")).unwrap();
    fs::remove_dir_all(&data).unwrap();
    symlink(outside.path(), &data).unwrap();
    assert_eq!(own.trusted_dirs(dirs), vec![system]);
}
