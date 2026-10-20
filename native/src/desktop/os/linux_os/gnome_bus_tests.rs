//! The calls of `gnome_dbus` and the probes of `session_facts` against a test
//! service on a session bus (`org.gnome.Shell`, `org.gnome.Shell.Extensions`,
//! `org.gnome.Mutter.DisplayConfig`): the names, paths, signatures and the
//! reading of the answers as the GNOME Shell defines them. Only on the
//! session bus of the suite stage (`SE_DESKTOP_TASK_SESSION_BUS=1`, a private
//! `dbus-run-session`), never on the bus of the machine.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use zbus::zvariant::{OwnedValue, Value};

use super::gnome_dbus::{display_monitors, DisplayState, SessionBus, ShellBus, STATE_ENABLED};
use super::session_facts::{Probe, SystemProbe};
use crate::desktop::core::ledger::GNOME_EXTENSION_UUID;

const OTHER: &str = "other@example.org";

struct Shell;

#[zbus::interface(name = "org.gnome.Shell")]
impl Shell {
    #[zbus(property)]
    fn shell_version(&self) -> String {
        "46.0".into()
    }
}

struct Extensions {
    /// `(uuid, state)` of the loaded extensions.
    loaded: Arc<Mutex<Vec<(String, f64)>>>,
}

fn number(value: f64) -> OwnedValue {
    OwnedValue::try_from(Value::F64(value)).expect("a double converts")
}

#[zbus::interface(name = "org.gnome.Shell.Extensions")]
impl Extensions {
    fn list_extensions(&self) -> HashMap<String, HashMap<String, OwnedValue>> {
        let loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        loaded
            .iter()
            .map(|(uuid, state)| {
                let mut info = HashMap::new();
                info.insert("state".to_string(), number(*state));
                (uuid.clone(), info)
            })
            .collect()
    }

    fn enable_extension(&self, uuid: String) -> bool {
        let mut loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        match loaded.iter_mut().find(|(known, _)| *known == uuid) {
            Some(entry) => {
                entry.1 = STATE_ENABLED;
                true
            }
            None => false,
        }
    }

    fn disable_extension(&self, uuid: String) -> bool {
        let mut loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        match loaded.iter_mut().find(|(known, _)| *known == uuid) {
            Some(entry) => {
                entry.1 = 2.0;
                true
            }
            None => false,
        }
    }
}

struct DisplayConfig;

#[zbus::interface(name = "org.gnome.Mutter.DisplayConfig")]
impl DisplayConfig {
    /// Two monitors: `Virtual-1` at scale 1 and `DP-2` (2560x1440) at scale 2.
    fn get_current_state(&self) -> DisplayState {
        let current = || {
            let mut properties = HashMap::new();
            properties.insert(
                "is-current".to_string(),
                OwnedValue::try_from(Value::Bool(true)).expect("a bool converts"),
            );
            properties
        };
        let connector = |name: &str| {
            (
                name.to_string(),
                "V".to_string(),
                "P".to_string(),
                "S".to_string(),
            )
        };
        let physical = vec![
            (
                connector("Virtual-1"),
                vec![(
                    "1920x1080@60".to_string(),
                    1920,
                    1080,
                    60.0,
                    1.0,
                    vec![1.0],
                    current(),
                )],
                HashMap::new(),
            ),
            (
                connector("DP-2"),
                vec![(
                    "2560x1440@60".to_string(),
                    2560,
                    1440,
                    60.0,
                    1.0,
                    vec![2.0],
                    current(),
                )],
                HashMap::new(),
            ),
        ];
        let logical = vec![
            (
                0,
                0,
                1.0,
                0,
                true,
                vec![connector("Virtual-1")],
                HashMap::new(),
            ),
            (
                1920,
                0,
                2.0,
                0,
                false,
                vec![connector("DP-2")],
                HashMap::new(),
            ),
        ];
        (3, physical, logical, HashMap::new())
    }
}

fn service(loaded: &Arc<Mutex<Vec<(String, f64)>>>) -> zbus::Result<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()?
        .name("org.gnome.Shell")?
        .name("org.gnome.Mutter.DisplayConfig")?
        .serve_at("/org/gnome/Shell", Shell)?
        .serve_at(
            "/org/gnome/Shell/Extensions",
            Extensions {
                loaded: loaded.clone(),
            },
        )?
        .serve_at("/org/gnome/Mutter/DisplayConfig", DisplayConfig)?
        .build()
}

#[test]
#[ignore = "braucht den privaten Sitzungsbus der Suite (SE_DESKTOP_TASK_SESSION_BUS=1)"]
fn gnome_dbus_calls_against_a_test_service() {
    assert_eq!(
        std::env::var("SE_DESKTOP_TASK_SESSION_BUS").as_deref(),
        Ok("1"),
        "nicht ausgeführt: nur auf dem privaten Sitzungsbus der Suite"
    );
    let loaded = Arc::new(Mutex::new(vec![
        (GNOME_EXTENSION_UUID.to_string(), 2.0),
        (OTHER.to_string(), STATE_ENABLED),
        ("ding@rastersoft.com".to_string(), STATE_ENABLED),
    ]));
    let connection = service(&loaded).expect("Testdienst auf dem Sitzungsbus");

    assert_eq!(SessionBus.shell_version().as_deref(), Some("46.0"));
    let list = SessionBus.list_extensions().expect("ListExtensions");
    let state = |uuid: &str| {
        list.iter()
            .find(|(known, _)| known == uuid)
            .map(|(_, s)| *s)
    };
    assert_eq!(state(OTHER), Some(STATE_ENABLED));
    assert_eq!(state(GNOME_EXTENSION_UUID), Some(2.0));

    // The Shell's answers: true for an extension it knows, else false.
    assert_eq!(
        SessionBus.set_extension(GNOME_EXTENSION_UUID, true),
        Ok(true)
    );
    assert_eq!(SessionBus.set_extension(OTHER, false), Ok(true));
    assert_eq!(
        SessionBus.set_extension("unknown@example.org", true),
        Ok(false)
    );
    let list = SessionBus.list_extensions().unwrap();
    let state = |uuid: &str| {
        list.iter()
            .find(|(known, _)| known == uuid)
            .map(|(_, s)| *s)
    };
    assert_eq!(state(GNOME_EXTENSION_UUID), Some(STATE_ENABLED));
    assert_eq!(state(OTHER), Some(2.0));

    // Mutter: two logical monitors in the order of the call.
    let monitors = display_monitors().expect("GetCurrentState");
    assert_eq!(monitors.len(), 2);
    assert_eq!(monitors[0].key, "Virtual-1");
    assert_eq!(monitors[0].rect, [0, 0, 1920, 1080]);
    assert!(monitors[0].primary);
    assert_eq!(monitors[1].key, "DP-2");
    assert_eq!(monitors[1].rect, [1920, 0, 1920 + 1280, 720]);

    // The probes of the session read the same.
    assert_eq!(SystemProbe.gnome_shell_version().as_deref(), Some("46.0"));
    let active = SystemProbe.gnome_active_extensions();
    assert!(active.contains(&GNOME_EXTENSION_UUID.to_string()));
    assert!(active.contains(&"ding@rastersoft.com".to_string()));
    assert!(!active.contains(&OTHER.to_string()), "disabled above");

    // No service: nothing, not a hang.
    drop(connection);
    assert_eq!(SessionBus.shell_version(), None);
    assert!(SessionBus.set_extension(OTHER, true).is_err());
}
