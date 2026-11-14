//! A worker without session variables follows the active session of
//! `seat0` (E23, S3), shown with simulated logind answers, process tables
//! and account databases: X11 and Wayland sessions, a greeter, a session
//! switch, a foreign `XAUTHORITY` and no active session.
use std::collections::HashMap;
use std::io;
use std::process::Command;

use super::spawn::{environment, has_own_session};
use super::{
    is_current, resolve, Account, Accounts, Logind, LogindSession, ProcessTable, Unresolved,
};
use crate::screen::os::session_detect::{detect, SessionKind};

struct FakeLogind(Option<LogindSession>);
impl Logind for FakeLogind {
    fn active_session(&self) -> io::Result<Option<LogindSession>> {
        Ok(self.0.clone())
    }
}
struct DownLogind;
impl Logind for DownLogind {
    fn active_session(&self) -> io::Result<Option<LogindSession>> {
        Err(io::Error::other("no system bus"))
    }
}

#[derive(Default)]
struct FakeProcess {
    uids: [u32; 4],
    cgroup: String,
    environ: Vec<u8>,
}
#[derive(Default)]
struct FakeTable {
    processes: HashMap<u32, FakeProcess>,
    files: HashMap<String, u32>,
    sockets: HashMap<String, u32>,
    dirs: HashMap<String, u32>,
}
impl FakeTable {
    fn process(mut self, pid: u32, uid: u32, scope: &str, env: &[(&str, &str)]) -> Self {
        let mut environ = Vec::new();
        for (name, value) in env {
            environ.extend_from_slice(format!("{name}={value}").as_bytes());
            environ.push(0);
        }
        let cgroup = format!("0::/user.slice/user-{uid}.slice/{scope}\n");
        self.processes.insert(
            pid,
            FakeProcess {
                uids: [uid; 4],
                cgroup,
                environ,
            },
        );
        self
    }
}
impl ProcessTable for FakeTable {
    fn pids(&self) -> Vec<u32> {
        let mut pids: Vec<u32> = self.processes.keys().copied().collect();
        pids.sort_unstable();
        pids
    }
    fn uids(&self, pid: u32) -> Option<[u32; 4]> {
        self.processes.get(&pid).map(|process| process.uids)
    }
    fn cgroup(&self, pid: u32) -> Option<String> {
        self.processes
            .get(&pid)
            .map(|process| process.cgroup.clone())
    }
    fn environ(&self, pid: u32) -> Option<Vec<u8>> {
        self.processes
            .get(&pid)
            .map(|process| process.environ.clone())
    }
    fn file_owner(&self, path: &str) -> Option<u32> {
        self.files.get(path).copied()
    }
    fn socket_owner(&self, path: &str) -> Option<u32> {
        self.sockets.get(path).copied()
    }
    fn dir_owner(&self, path: &str) -> Option<u32> {
        self.dirs.get(path).copied()
    }
}

struct FakeAccounts;
impl Accounts for FakeAccounts {
    fn account(&self, uid: u32) -> Option<Account> {
        (uid != 4242).then(|| Account {
            uid,
            gid: uid,
            name: format!("user{uid}"),
            home: format!("/home/user{uid}"),
            shell: "/bin/sh".into(),
            groups: vec![uid, 44],
        })
    }
}

fn session(id: &str, class: &str, kind: &str, uid: u32, display: &str) -> LogindSession {
    LogindSession {
        id: id.into(),
        class: class.into(),
        kind: kind.into(),
        active: true,
        remote: false,
        state: "active".into(),
        uid,
        leader: 100,
        display: display.into(),
        timestamp: 7,
        runtime_path: format!("/run/user/{uid}"),
    }
}

fn value<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn x11_table() -> FakeTable {
    let mut table = FakeTable::default()
        // The root leader (display manager) shows nothing.
        .process(100, 0, "session-2.scope", &[("PATH", "/usr/bin")])
        .process(
            101,
            1000,
            "session-2.scope",
            &[
                ("DISPLAY", ":0"),
                ("XAUTHORITY", "/home/user1000/.Xauthority"),
                ("XDG_CURRENT_DESKTOP", "XFCE"),
            ],
        )
        // Another account names its own cookie for the same display.
        .process(
            200,
            1001,
            "session-3.scope",
            &[
                ("DISPLAY", ":0"),
                ("XAUTHORITY", "/home/user1001/.Xauthority"),
            ],
        );
    table.dirs.insert("/run/user/1000".into(), 1000);
    table
        .files
        .insert("/home/user1000/.Xauthority".into(), 1000);
    table
        .files
        .insert("/home/user1001/.Xauthority".into(), 1001);
    table
}

#[test]
fn bildschirm_sitzung_task_x11_session_gives_its_display_and_owned_xauthority() {
    let logind = FakeLogind(Some(session("2", "user", "x11", 1000, ":0")));
    let followed = resolve(&logind, &x11_table(), &FakeAccounts, 0).expect("x11 session");
    assert_eq!(followed.kind, SessionKind::X11);
    assert_eq!(followed.account.uid, 1000);
    assert_eq!(value(&followed.env, "DISPLAY"), Some(":0"));
    assert_eq!(
        value(&followed.env, "XAUTHORITY"),
        Some("/home/user1000/.Xauthority")
    );
    assert_eq!(value(&followed.env, "XDG_CURRENT_DESKTOP"), Some("XFCE"));
    assert_eq!(value(&followed.env, "XDG_SESSION_TYPE"), Some("x11"));
    assert_eq!(value(&followed.env, "WAYLAND_DISPLAY"), None);
    // The worker of the session's own account follows it too.
    assert!(resolve(&logind, &x11_table(), &FakeAccounts, 1000).is_ok());
}

#[test]
fn bildschirm_sitzung_task_wayland_session_runs_as_its_account_with_its_bus() {
    let mut table = FakeTable::default().process(
        300,
        1000,
        "app.slice",
        &[
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("XDG_CURRENT_DESKTOP", "GNOME"),
        ],
    );
    table.dirs.insert("/run/user/1000".into(), 1000);
    table
        .sockets
        .insert("/run/user/1000/wayland-0".into(), 1000);
    let logind = FakeLogind(Some(session("4", "user", "wayland", 1000, "")));
    let followed = resolve(&logind, &table, &FakeAccounts, 0).expect("wayland session");
    assert_eq!(followed.kind, SessionKind::Wayland);
    assert_eq!(value(&followed.env, "WAYLAND_DISPLAY"), Some("wayland-0"));
    assert_eq!(
        value(&followed.env, "XDG_RUNTIME_DIR"),
        Some("/run/user/1000")
    );
    assert_eq!(
        value(&followed.env, "DBUS_SESSION_BUS_ADDRESS"),
        Some("unix:path=/run/user/1000/bus")
    );
    // The process environment is the account's, not root's.
    let env = environment(
        &followed,
        [
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/root".to_string()),
            ("SUDO_USER".to_string(), "admin".to_string()),
        ],
    );
    assert_eq!(value(&env, "HOME"), Some("/home/user1000"));
    assert_eq!(value(&env, "USER"), Some("user1000"));
    assert_eq!(value(&env, "PATH"), Some("/usr/bin"));
    assert_eq!(value(&env, "SUDO_USER"), None);
    assert_eq!(env.iter().filter(|(key, _)| key == "HOME").count(), 1);
    let mut command = Command::new("/bin/true");
    super::spawn::apply(&followed, &mut command);
    assert_eq!(command.get_current_dir(), Some(std::path::Path::new("/")));
    let named = |name: &str| {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
    };
    assert_eq!(named("WAYLAND_DISPLAY").as_deref(), Some("wayland-0"));
    // A socket of another account is no session display.
    table
        .sockets
        .insert("/run/user/1000/wayland-0".into(), 1001);
    assert_eq!(
        resolve(&logind, &table, &FakeAccounts, 0),
        Err(Unresolved::Environment("wayland_socket_foreign"))
    );
}

#[test]
fn bildschirm_sitzung_task_greeter_session_is_followed() {
    // LightDM: the root leader lacks DISPLAY; the greeter process in the
    // scope names it (also light-locker's greeter on another display).
    let mut table = FakeTable::default()
        .process(100, 0, "session-c1.scope", &[("PATH", "/usr/bin")])
        .process(
            110,
            113,
            "session-c1.scope",
            &[
                ("DISPLAY", ":1"),
                ("XAUTHORITY", "/var/lib/lightdm/.Xauthority"),
            ],
        );
    table.dirs.insert("/run/user/113".into(), 113);
    table
        .files
        .insert("/var/lib/lightdm/.Xauthority".into(), 113);
    let logind = FakeLogind(Some(session("c1", "greeter", "x11", 113, ":1")));
    let followed = resolve(&logind, &table, &FakeAccounts, 0).expect("greeter");
    assert_eq!(followed.account.uid, 113);
    assert_eq!(value(&followed.env, "DISPLAY"), Some(":1"));
    let lock = FakeLogind(Some(session("c1", "lock-screen", "x11", 113, ":1")));
    assert!(resolve(&lock, &table, &FakeAccounts, 0).is_ok());
}

#[test]
fn bildschirm_sitzung_task_session_switch_ends_the_old_host() {
    let user = session("2", "user", "x11", 1000, ":0");
    let before = resolve(
        &FakeLogind(Some(user.clone())),
        &x11_table(),
        &FakeAccounts,
        0,
    )
    .expect("user session");
    assert!(is_current(before.generation(), Some(&user.generation())));
    // Greeter, another login of the same account and no session at all
    // are other sessions; the host of `before` ends.
    let greeter = session("c1", "greeter", "x11", 113, ":1");
    assert!(!is_current(
        before.generation(),
        Some(&greeter.generation())
    ));
    let relogin = LogindSession {
        timestamp: 99,
        ..user.clone()
    };
    assert!(!is_current(
        before.generation(),
        Some(&relogin.generation())
    ));
    assert!(!is_current(before.generation(), None));
}

#[test]
fn bildschirm_sitzung_task_foreign_or_ambiguous_xauthority_is_refused() {
    let logind = FakeLogind(Some(session("2", "user", "x11", 1000, ":0")));
    let mut foreign = x11_table();
    foreign
        .files
        .insert("/home/user1000/.Xauthority".into(), 1001);
    assert_eq!(
        resolve(&logind, &foreign, &FakeAccounts, 0),
        Err(Unresolved::Environment("xauthority_foreign"))
    );
    let mut missing = x11_table();
    missing.files.remove("/home/user1000/.Xauthority");
    assert_eq!(
        resolve(&logind, &missing, &FakeAccounts, 0),
        Err(Unresolved::Environment("xauthority_foreign"))
    );
    let ambiguous = x11_table().process(
        102,
        1000,
        "session-2.scope",
        &[("DISPLAY", ":0"), ("XAUTHORITY", "/tmp/planted")],
    );
    assert_eq!(
        resolve(&logind, &ambiguous, &FakeAccounts, 0),
        Err(Unresolved::Environment("xauthority_ambiguous"))
    );
    let relative = FakeTable {
        dirs: x11_table().dirs,
        ..FakeTable::default()
    }
    .process(
        101,
        1000,
        "session-2.scope",
        &[("DISPLAY", ":0"), ("XAUTHORITY", "x")],
    );
    assert_eq!(
        resolve(&logind, &relative, &FakeAccounts, 0),
        Err(Unresolved::Environment("xauthority_not_absolute"))
    );
}

#[test]
fn bildschirm_sitzung_task_no_active_session_gives_no_display() {
    let table = x11_table();
    let refused = |logind: &dyn Logind, euid: u32| resolve(logind, &table, &FakeAccounts, euid);
    assert_eq!(
        refused(&FakeLogind(None), 0),
        Err(Unresolved::NoActiveSession)
    );
    assert_eq!(refused(&DownLogind, 0), Err(Unresolved::LogindUnavailable));
    let base = session("2", "user", "x11", 1000, ":0");
    let cases = [
        (
            LogindSession {
                active: false,
                ..base.clone()
            },
            Unresolved::NotLocalForeground,
        ),
        (
            LogindSession {
                remote: true,
                ..base.clone()
            },
            Unresolved::NotLocalForeground,
        ),
        (
            LogindSession {
                state: "closing".into(),
                ..base.clone()
            },
            Unresolved::NotLocalForeground,
        ),
        (
            session("5", "user", "tty", 1000, ""),
            Unresolved::NotGraphical,
        ),
        (
            session("6", "background", "x11", 1000, ":0"),
            Unresolved::NotGraphical,
        ),
        (
            session("7", "user", "x11", 0, ":0"),
            Unresolved::RootSession,
        ),
        (
            session("8", "user", "x11", 4242, ":0"),
            Unresolved::UnknownAccount,
        ),
        (
            session("9", "user", "x11", 1000, "remote:0"),
            Unresolved::Environment("display_not_local"),
        ),
        (
            session("10", "user", "x11", 1000, ":5"),
            Unresolved::Environment("display_without_process"),
        ),
    ];
    for (session, expected) in cases {
        assert_eq!(refused(&FakeLogind(Some(session)), 0), Err(expected));
    }
    // A worker of a third account never follows another account's session.
    assert_eq!(
        refused(&FakeLogind(Some(base.clone())), 1001),
        Err(Unresolved::ForeignAccount)
    );
    // A runtime directory of another account is no session environment.
    let mut foreign_runtime = x11_table();
    foreign_runtime.dirs.insert("/run/user/1000".into(), 0);
    assert_eq!(
        resolve(&FakeLogind(Some(base)), &foreign_runtime, &FakeAccounts, 0),
        Err(Unresolved::Environment("runtime_dir_not_owned"))
    );
}

#[test]
fn bildschirm_sitzung_task_own_session_variables_keep_their_path() {
    let env_of = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    };
    for pairs in [
        &[("DISPLAY", ":0")][..],
        &[("WAYLAND_DISPLAY", "wayland-0")][..],
        &[("XDG_SESSION_TYPE", "wayland")][..],
    ] {
        let env = env_of(pairs);
        assert!(has_own_session(&env));
        assert_ne!(detect(&env), SessionKind::None);
    }
    let none = env_of(&[("PATH", "/usr/bin")]);
    assert!(!has_own_session(&none));
    assert_eq!(detect(&none), SessionKind::None);
}
