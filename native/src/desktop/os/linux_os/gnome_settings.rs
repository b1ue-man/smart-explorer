//! GNOME Shell settings that se Desktop leaves behind or restores (C2.4,
//! C2.5, C5.7): the UUID `se-desktop@smartexplorer.app` in
//! `org.gnome.shell enabled-extensions`, and a foreign desktop-icons
//! extension that se switched off for the take-over (only with its ledger
//! entry). Writing them belongs to DSK-M8; the frame removes and restores.
//! The lists are changed through `gsettings` like `gnome-extensions` does;
//! without `gsettings` or the schema there is no GNOME and no such trace.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SCHEMA: &str = "org.gnome.shell";
pub(crate) const ENABLED: &str = "enabled-extensions";
pub(crate) const DISABLED: &str = "disabled-extensions";
/// `gsettings` answers at once; this only bounds a hanging settings backend.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Longest extension UUID accepted from a ledger entry.
const MAX_UUID: usize = 128;

/// Access to `org.gnome.shell`; `None` stands for a session without GNOME.
#[derive(Clone, Debug)]
pub(crate) struct GnomeSettings {
    program: Option<PathBuf>,
}

impl GnomeSettings {
    pub(crate) fn system() -> GnomeSettings {
        GnomeSettings {
            program: Some(PathBuf::from("gsettings")),
        }
    }

    pub(crate) fn none() -> GnomeSettings {
        GnomeSettings { program: None }
    }

    /// The list under `key`; `None` without `gsettings` or schema.
    pub(crate) fn read(&self, key: &str) -> Option<Vec<String>> {
        parse_string_array(&self.run(&["get", SCHEMA, key])?)
    }

    pub(crate) fn write(&self, key: &str, items: &[String]) -> Result<(), String> {
        let value = format_string_array(items);
        self.run(&["set", SCHEMA, key, &value])
            .map(drop)
            .ok_or_else(|| format!("gsettings set {SCHEMA} {key} ist gescheitert"))
    }

    fn run(&self, args: &[&str]) -> Option<String> {
        let program = self.program.as_ref()?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let reader = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stdout.read_to_string(&mut text);
            text
        });
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };
        let text = reader.join().ok()?;
        status.success().then_some(text)
    }
}

/// A GVariant string array as `gsettings get` prints it: `['a', 'b']` or
/// `@as []`.
pub(crate) fn parse_string_array(text: &str) -> Option<Vec<String>> {
    let text = text.trim();
    let text = text.strip_prefix("@as").map_or(text, str::trim_start);
    let inner = text.strip_prefix('[')?.strip_suffix(']')?;
    let mut items = Vec::new();
    let mut chars = inner.chars();
    loop {
        let quote = loop {
            match chars.next() {
                None => return Some(items),
                Some(character) if character.is_whitespace() || character == ',' => {}
                Some(quote @ ('\'' | '"')) => break quote,
                Some(_) => return None,
            }
        };
        let mut item = String::new();
        loop {
            match chars.next()? {
                '\\' => item.push(chars.next()?),
                character if character == quote => break,
                character => item.push(character),
            }
        }
        items.push(item);
    }
}

/// The value `gsettings set` takes for `items`.
pub(crate) fn format_string_array(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".into();
    }
    let quoted: Vec<String> = items
        .iter()
        .map(|item| format!("'{}'", item.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

/// Shape of a GNOME extension UUID (`name@domain`).
pub(crate) fn valid_uuid(uuid: &str) -> bool {
    !uuid.is_empty()
        && uuid.len() <= MAX_UUID
        && uuid.contains('@')
        && uuid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'@'))
}

/// The extension exists for this user or system-wide.
pub(crate) fn extension_installed(user_extensions: &Path, uuid: &str) -> bool {
    [
        user_extensions.to_path_buf(),
        PathBuf::from("/usr/share/gnome-shell/extensions"),
        PathBuf::from("/usr/local/share/gnome-shell/extensions"),
    ]
    .iter()
    .any(|root| root.join(uuid).join("metadata.json").is_file())
}
