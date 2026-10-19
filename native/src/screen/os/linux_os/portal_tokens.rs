//! The restore tokens of the portal (SCR-M5, U19-9): after the Wayland user
//! chose a screen once, the portal hands out a token that lets the next
//! session start without the dialog. They live in this user's app data in
//! `screen-portal-tokens.json` (mode 0600, replaced atomically):
//! `{"version":1,"screencast":{"token":"…","saved_at":<unix seconds>},
//! "remote_desktop":{…}}`. A missing key means „no stored grant“; a file that
//! cannot be read counts as none and is replaced by the next save. The
//! remote-desktop token belongs to the input of U22, which shares the portal
//! session of the screen cast.
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// File name inside the app data directory.
pub(crate) const FILE_NAME: &str = "screen-portal-tokens.json";
const VERSION: u64 = 1;

/// The kinds of portal grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokenKind {
    ScreenCast,
    RemoteDesktop,
}

impl TokenKind {
    pub(crate) const ALL: [Self; 2] = [Self::ScreenCast, Self::RemoteDesktop];

    /// The key of the kind in the file and in `se screen portal forget`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::ScreenCast => "screencast",
            Self::RemoteDesktop => "remote_desktop",
        }
    }
}

/// One stored token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredToken {
    pub token: String,
    pub saved_at: i64,
}

/// All stored tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tokens {
    pub screencast: Option<StoredToken>,
    pub remote_desktop: Option<StoredToken>,
}

impl Tokens {
    pub(crate) fn get(&self, kind: TokenKind) -> Option<&StoredToken> {
        match kind {
            TokenKind::ScreenCast => self.screencast.as_ref(),
            TokenKind::RemoteDesktop => self.remote_desktop.as_ref(),
        }
    }

    fn slot(&mut self, kind: TokenKind) -> &mut Option<StoredToken> {
        match kind {
            TokenKind::ScreenCast => &mut self.screencast,
            TokenKind::RemoteDesktop => &mut self.remote_desktop,
        }
    }

    fn is_empty(&self) -> bool {
        self.screencast.is_none() && self.remote_desktop.is_none()
    }
}

fn default_path() -> PathBuf {
    crate::support_dirs::app_data_file(FILE_NAME)
}

/// The stored tokens of this user.
pub(crate) fn load() -> Tokens {
    load_from(&default_path())
}

/// Stores `token` for `kind`, replacing the old one.
pub(crate) fn save(kind: TokenKind, token: &str) -> io::Result<()> {
    save_to(&default_path(), kind, token, now_secs())
}

/// Deletes both tokens; the kinds that were stored, `screencast` first. The
/// file goes away with them.
pub(crate) fn forget() -> io::Result<Vec<TokenKind>> {
    forget_at(&default_path())
}

fn now_secs() -> i64 {
    crate::share::core_now_secs()
}

pub(crate) fn load_from(path: &Path) -> Tokens {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Tokens::default(),
        Err(error) => {
            eprintln!("screen portal tokens: {}: {error}", path.display());
            return Tokens::default();
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Tokens {
            screencast: entry(&map, TokenKind::ScreenCast),
            remote_desktop: entry(&map, TokenKind::RemoteDesktop),
        },
        _ => {
            eprintln!("screen portal tokens: {} is not valid", path.display());
            Tokens::default()
        }
    }
}

fn entry(map: &Map<String, Value>, kind: TokenKind) -> Option<StoredToken> {
    let stored: StoredToken = serde_json::from_value(map.get(kind.key())?.clone()).ok()?;
    (!stored.token.is_empty()).then_some(stored)
}

pub(crate) fn save_to(path: &Path, kind: TokenKind, token: &str, now: i64) -> io::Result<()> {
    let mut tokens = load_from(path);
    *tokens.slot(kind) = Some(StoredToken {
        token: token.into(),
        saved_at: now,
    });
    write(path, &tokens)
}

pub(crate) fn forget_at(path: &Path) -> io::Result<Vec<TokenKind>> {
    let tokens = load_from(path);
    let removed: Vec<TokenKind> = TokenKind::ALL
        .into_iter()
        .filter(|kind| tokens.get(*kind).is_some())
        .collect();
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(removed)
}

fn write(path: &Path, tokens: &Tokens) -> io::Result<()> {
    if tokens.is_empty() {
        return match fs::remove_file(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    let mut map = Map::new();
    map.insert("version".into(), VERSION.into());
    for kind in TokenKind::ALL {
        if let Some(stored) = tokens.get(kind) {
            let value = serde_json::to_value(stored).map_err(io::Error::other)?;
            map.insert(kind.key().into(), value);
        }
    }
    let bytes = serde_json::to_vec_pretty(&Value::Object(map)).map_err(io::Error::other)?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = directory.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    let written = write_private(&temporary, &bytes).and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Seconds since the epoch of a stored token, for the „seit …“ line of the
/// desktop app.
pub(crate) fn saved_at(kind: TokenKind) -> Option<i64> {
    load().get(kind).map(|stored| stored.saved_at)
}
