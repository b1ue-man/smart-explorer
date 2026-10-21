//! The calls of the GNOME session on the session bus (U34-2, U34-14,
//! U34-16, U34-17): the Shell's version and its extension list, enabling and
//! disabling an extension, and the logical monitors of Mutter. Every call
//! runs under the time limit of the probes (`probe_timeout`); without a bus,
//! a service or an answer the result is `None` or an error text.
//!
//! The extension calls sit behind [`ShellBus`], so the installation, the
//! take-over and the status are checked with a stand-in.
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

use super::monitor_keys::unique_keys;
use super::probe_timeout::{timed, timed_gated};

const SHELL: &str = "org.gnome.Shell";
const SHELL_PATH: &str = "/org/gnome/Shell";
/// The Shell exports `org.gnome.Shell.Extensions` on `/org/gnome/Shell`, the
/// object of `org.gnome.Shell` itself (`GnomeShellExtensions` in
/// `js/ui/shellDBus.js`, `export(Gio.DBus.session, '/org/gnome/Shell')`, the
/// same in the Shells 45 to 50). `/org/gnome/Shell/Extensions` is no object
/// of a running Shell: a call there fails with `UnknownObject`.
const EXTENSIONS_PATH: &str = SHELL_PATH;
const EXTENSIONS: &str = "org.gnome.Shell.Extensions";
const MUTTER: &str = "org.gnome.Mutter.DisplayConfig";
const MUTTER_PATH: &str = "/org/gnome/Mutter/DisplayConfig";

/// `state` of an extension in `ListExtensions` when it is enabled
/// (`ExtensionState.ENABLED`, a double on the wire).
pub(crate) const STATE_ENABLED: f64 = 1.0;

/// What the extension handling asks of `org.gnome.Shell`.
pub(crate) trait ShellBus: Send + Sync {
    /// The Shell's `ShellVersion` (`46.0`, `49.alpha.1`).
    fn shell_version(&self) -> Option<String>;
    /// `(uuid, state)` of every extension the running Shell knows.
    fn list_extensions(&self) -> Option<Vec<(String, f64)>>;
    /// `EnableExtension` or `DisableExtension`; the Shell's answer.
    fn set_extension(&self, uuid: &str, enable: bool) -> Result<bool, String>;
}

/// The real session bus.
pub(crate) struct SessionBus;

/// One Shell, shared by several owners (the steward and the integration it
/// builds for the return of a foreign extension).
impl ShellBus for std::sync::Arc<dyn ShellBus> {
    fn shell_version(&self) -> Option<String> {
        (**self).shell_version()
    }

    fn list_extensions(&self) -> Option<Vec<(String, f64)>> {
        (**self).list_extensions()
    }

    fn set_extension(&self, uuid: &str, enable: bool) -> Result<bool, String> {
        (**self).set_extension(uuid, enable)
    }
}

/// No Shell to ask (the suite's folders, a session without GNOME).
pub(crate) struct NoShellBus;

impl ShellBus for NoShellBus {
    fn shell_version(&self) -> Option<String> {
        None
    }

    fn list_extensions(&self) -> Option<Vec<(String, f64)>> {
        None
    }

    fn set_extension(&self, _uuid: &str, _enable: bool) -> Result<bool, String> {
        Err("keine GNOME Shell".into())
    }
}

static CONNECTION: Mutex<Option<Connection>> = Mutex::new(None);

/// The session connection, opened once and kept; any failed call drops it,
/// so a bus that came back is used again.
fn connection() -> zbus::Result<Connection> {
    let mut slot = CONNECTION.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(connection) = slot.as_ref() {
        return Ok(connection.clone());
    }
    let connection = Connection::session()?;
    *slot = Some(connection.clone());
    Ok(connection)
}

fn forget_connection() {
    *CONNECTION.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// Runs `call` on the session connection, under the time limit.
fn on_bus<T, F>(name: &'static str, call: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> zbus::Result<T> + Send + 'static,
{
    let answer = timed(name, move || connection().and_then(|bus| call(&bus)));
    match answer {
        Some(Ok(value)) => Some(value),
        Some(Err(_)) | None => {
            forget_connection();
            None
        }
    }
}

impl ShellBus for SessionBus {
    fn shell_version(&self) -> Option<String> {
        on_bus("gnome-version", |bus| {
            Proxy::new(bus, SHELL, SHELL_PATH, SHELL)?.get_property::<String>("ShellVersion")
        })
    }

    fn list_extensions(&self) -> Option<Vec<(String, f64)>> {
        on_bus("gnome-extensions", |bus| {
            let proxy = Proxy::new(bus, SHELL, EXTENSIONS_PATH, EXTENSIONS)?;
            let list: HashMap<String, HashMap<String, OwnedValue>> =
                proxy.call("ListExtensions", &())?;
            Ok(list
                .into_iter()
                .map(|(uuid, info)| {
                    let state = info.get("state").map_or(0.0, number);
                    (uuid, state)
                })
                .collect())
        })
    }

    fn set_extension(&self, uuid: &str, enable: bool) -> Result<bool, String> {
        let uuid = uuid.to_string();
        let method = if enable {
            "EnableExtension"
        } else {
            "DisableExtension"
        };
        let answer = timed("gnome-set-extension", move || {
            let bus = connection()?;
            let proxy = Proxy::new(&bus, SHELL, EXTENSIONS_PATH, EXTENSIONS)?;
            proxy.call::<_, _, bool>(method, &(uuid.as_str(),))
        });
        match answer {
            Some(Ok(done)) => Ok(done),
            Some(Err(error)) => {
                forget_connection();
                Err(format!("{method}: {error}"))
            }
            None => Err(format!("{method}: keine Antwort der GNOME Shell")),
        }
    }
}

/// A number of the extension info (`state` is a double; older Shells send
/// an integer).
fn number(value: &OwnedValue) -> f64 {
    match &**value {
        Value::F64(number) => *number,
        Value::U32(number) => f64::from(*number),
        Value::I32(number) => f64::from(*number),
        Value::U64(number) => *number as f64,
        Value::I64(number) => *number as f64,
        _ => 0.0,
    }
}

/// The major version of a `ShellVersion` text: the leading digits up to the
/// first `.` (`46.0` → 46, `49.alpha.1` → 49, `45` → 45); `None` for an
/// empty text or one without leading digits.
pub(crate) fn parse_shell_major(text: &str) -> Option<u32> {
    let head = text.trim().split('.').next()?;
    let digits: String = head.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

type Connector = (String, String, String, String);
type Mode = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    HashMap<String, OwnedValue>,
);
type PhysicalMonitor = (Connector, Vec<Mode>, HashMap<String, OwnedValue>);
type LogicalEntry = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<Connector>,
    HashMap<String, OwnedValue>,
);
/// The answer of `GetCurrentState`: serial, monitors, logical monitors,
/// properties.
pub(crate) type DisplayState = (
    u32,
    Vec<PhysicalMonitor>,
    Vec<LogicalEntry>,
    HashMap<String, OwnedValue>,
);

/// One logical monitor of Mutter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GnomeMonitor {
    /// Connector of the first physical monitor (`DP-1`), cleaned and unique.
    pub(crate) key: String,
    /// `[left, top, right, bottom]` in logical pixels.
    pub(crate) rect: [i32; 4],
    pub(crate) primary: bool,
}

/// The logical monitors of `GetCurrentState` in the order of the call: key
/// from the connector of the first physical monitor, position from the
/// logical monitor, size = the current mode divided by the scale (width and
/// height swap for a transform that turns by 90 or 270 degrees).
pub(crate) fn logical_monitors(state: &DisplayState) -> Vec<GnomeMonitor> {
    let (_, physical, logical, _) = state;
    let names: Vec<String> = logical
        .iter()
        .map(|entry| {
            entry
                .5
                .first()
                .map(|connector| connector.0.clone())
                .unwrap_or_default()
        })
        .collect();
    let keys = unique_keys(&names);
    logical
        .iter()
        .zip(keys)
        .map(|(entry, key)| {
            let (x, y, scale, transform, primary, connectors, _) = entry;
            let mode = connectors
                .first()
                .and_then(|connector| physical.iter().find(|monitor| monitor.0 .0 == connector.0))
                .and_then(|monitor| current_mode(&monitor.1));
            let scale = if *scale > 0.0 { *scale } else { 1.0 };
            let (mut width, mut height) = mode.map_or((0, 0), |mode| (mode.1, mode.2));
            // Transforms 1, 3, 5 and 7 turn the picture by 90 or 270 degrees.
            if transform % 2 == 1 {
                std::mem::swap(&mut width, &mut height);
            }
            let logical_width = (f64::from(width) / scale).round() as i32;
            let logical_height = (f64::from(height) / scale).round() as i32;
            GnomeMonitor {
                key,
                rect: [*x, *y, x + logical_width, y + logical_height],
                primary: *primary,
            }
        })
        .collect()
}

/// The mode that is on now (`is-current`), else the preferred one.
fn current_mode(modes: &[Mode]) -> Option<&Mode> {
    let flag = |mode: &Mode, name: &str| {
        mode.6
            .get(name)
            .is_some_and(|value| matches!(&**value, Value::Bool(true)))
    };
    modes
        .iter()
        .find(|mode| flag(mode, "is-current"))
        .or_else(|| modes.iter().find(|mode| flag(mode, "is-preferred")))
}

/// The logical monitors Mutter reports now; `None` without the service.
/// Gated: the shell asks once a second.
pub(crate) fn display_monitors() -> Option<Vec<GnomeMonitor>> {
    timed_gated("gnome-display-config", || {
        let bus = connection()?;
        let proxy = Proxy::new(&bus, MUTTER, MUTTER_PATH, MUTTER)?;
        let state: DisplayState = proxy.call("GetCurrentState", &())?;
        Ok::<_, zbus::Error>(logical_monitors(&state))
    })
    .and_then(|answer| match answer {
        Ok(monitors) => Some(monitors),
        Err(_) => {
            forget_connection();
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(value: Value<'_>) -> OwnedValue {
        value.try_into().expect("simple values convert")
    }

    fn mode(width: i32, height: i32, current: bool) -> Mode {
        let mut properties = HashMap::new();
        properties.insert("is-current".to_string(), property(Value::Bool(current)));
        (
            format!("{width}x{height}@60"),
            width,
            height,
            60.0,
            1.0,
            vec![1.0],
            properties,
        )
    }

    fn connector(name: &str) -> Connector {
        (name.into(), "V".into(), "P".into(), "S".into())
    }

    #[test]
    fn display_state_gives_logical_monitors() {
        let physical = vec![
            (
                connector("eDP-1"),
                vec![mode(1280, 720, false), mode(2560, 1440, true)],
                HashMap::new(),
            ),
            (
                connector("DP-1"),
                vec![mode(1080, 1920, true)],
                HashMap::new(),
            ),
        ];
        let logical = vec![
            (0, 0, 2.0, 0, true, vec![connector("eDP-1")], HashMap::new()),
            (
                1280,
                0,
                1.0,
                1,
                false,
                vec![connector("DP-1")],
                HashMap::new(),
            ),
        ];
        let state: DisplayState = (7, physical, logical, HashMap::new());
        let monitors = logical_monitors(&state);
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].key, "eDP-1");
        assert_eq!(monitors[0].rect, [0, 0, 1280, 720], "2560x1440 at scale 2");
        assert!(monitors[0].primary);
        assert_eq!(monitors[1].key, "DP-1");
        assert_eq!(
            monitors[1].rect,
            [1280, 0, 1280 + 1920, 1080],
            "a transform of 90 degrees swaps width and height"
        );
        assert!(!monitors[1].primary);
    }
}
