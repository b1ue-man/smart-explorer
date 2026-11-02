//! Configuration of the account service (contract D.2.8):
//! `/etc/smart-explorer/exec-accounts.toml`, root's file with mode 0644. Per
//! Smart Explorer user (uid) it lists the local accounts that user's worker
//! may run remote commands as:
//!
//! ```toml
//! [[user]]
//! uid = 1000
//! accounts = ["se-remote"]
//! ```
//!
//! Only this subset of TOML is read: comments, `[[user]]` tables, `uid` and
//! a one-line `accounts` list of plain names. Anything else, uid 0, the
//! account root or a file that someone besides root could change makes the
//! whole file invalid, and the helper then refuses every request.
use std::fmt::Write as _;
use std::io::{self, Read};
use std::path::Path;

use super::exec_account::unit_account_name;
use super::exec_binary_copy::{ensure_root_directory, opened_trusted, write_root_file};

pub(super) const CONFIG_DIRECTORY: &str = "/etc/smart-explorer";
pub(super) const CONFIG_FILE: &str = "/etc/smart-explorer/exec-accounts.toml";
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const HEADER: &str = "# Smart Explorer account service: the local accounts each Smart Explorer\n\
                      # user (uid) may run remote commands as. Changed by\n\
                      # `se share exec accounts setup|allow|disallow` as administrator.\n";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct HelperConfig {
    users: Vec<UserAccounts>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UserAccounts {
    uid: u32,
    accounts: Vec<String>,
}

impl HelperConfig {
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let mut users = Vec::new();
        let mut table: Option<(Option<u32>, Option<Vec<String>>)> = None;
        for (index, raw) in text.lines().enumerate() {
            let number = index + 1;
            let line = raw.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            if line == "[[user]]" {
                finish(&mut users, table.take())?;
                table = Some((None, None));
                continue;
            }
            let Some((uid, accounts)) = table.as_mut() else {
                return Err(format!("line {number}: a key outside a [[user]] table"));
            };
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {number}: expected `key = value`"))?;
            match key.trim() {
                "uid" if uid.is_none() => {
                    *uid = Some(
                        parse_uid(value.trim())
                            .ok_or_else(|| format!("line {number}: invalid uid"))?,
                    );
                }
                "accounts" if accounts.is_none() => {
                    *accounts = Some(
                        parse_accounts(value.trim())
                            .ok_or_else(|| format!("line {number}: invalid account list"))?,
                    );
                }
                other => return Err(format!("line {number}: unexpected key {other:?}")),
            }
        }
        finish(&mut users, table)?;
        Ok(Self { users })
    }

    pub(super) fn render(&self) -> String {
        let mut text = String::from(HEADER);
        for user in &self.users {
            let accounts: Vec<String> = user
                .accounts
                .iter()
                .map(|account| format!("\"{account}\""))
                .collect();
            let _ = write!(
                text,
                "\n[[user]]\nuid = {}\naccounts = [{}]\n",
                user.uid,
                accounts.join(", ")
            );
        }
        text
    }

    pub(super) fn has_user(&self, uid: u32) -> bool {
        self.users.iter().any(|user| user.uid == uid)
    }

    /// The accounts `uid` may use; none for a user the file does not list.
    pub(super) fn accounts_for(&self, uid: u32) -> &[String] {
        self.users
            .iter()
            .find(|user| user.uid == uid)
            .map(|user| user.accounts.as_slice())
            .unwrap_or_default()
    }

    pub(super) fn allows(&self, uid: u32, account: &str) -> bool {
        self.accounts_for(uid)
            .iter()
            .any(|allowed| allowed == account)
    }

    /// Lists `uid`, without accounts where it was missing.
    pub(super) fn add_user(&mut self, uid: u32) -> Result<bool, String> {
        if uid == 0 || uid == u32::MAX {
            return Err(format!("uid {uid} cannot use the account service"));
        }
        if self.has_user(uid) {
            return Ok(false);
        }
        self.users.push(UserAccounts {
            uid,
            accounts: Vec::new(),
        });
        Ok(true)
    }

    /// Allows `account` for `uid`; `Ok(false)` when it was allowed already.
    pub(super) fn allow(&mut self, uid: u32, account: &str) -> Result<bool, String> {
        if !unit_account_name(account) || account == "root" {
            return Err(format!("{account:?} cannot be allowed for remote commands"));
        }
        self.add_user(uid)?;
        let user = self
            .users
            .iter_mut()
            .find(|user| user.uid == uid)
            .ok_or_else(|| format!("uid {uid} is missing"))?;
        if user.accounts.iter().any(|allowed| allowed == account) {
            return Ok(false);
        }
        user.accounts.push(account.to_string());
        Ok(true)
    }

    /// Takes `account` away from `uid`; `false` when it was not allowed.
    pub(super) fn disallow(&mut self, uid: u32, account: &str) -> bool {
        let Some(user) = self.users.iter_mut().find(|user| user.uid == uid) else {
            return false;
        };
        let before = user.accounts.len();
        user.accounts.retain(|allowed| allowed != account);
        user.accounts.len() != before
    }
}

fn finish(
    users: &mut Vec<UserAccounts>,
    table: Option<(Option<u32>, Option<Vec<String>>)>,
) -> Result<(), String> {
    let Some((uid, accounts)) = table else {
        return Ok(());
    };
    let uid = uid.ok_or("a [[user]] table without uid")?;
    if users.iter().any(|user| user.uid == uid) {
        return Err(format!("uid {uid} is listed twice"));
    }
    users.push(UserAccounts {
        uid,
        accounts: accounts.unwrap_or_default(),
    });
    Ok(())
}

fn parse_uid(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|uid| *uid != 0 && *uid != u32::MAX)
}

fn parse_accounts(value: &str) -> Option<Vec<String>> {
    let inner = value.strip_prefix('[')?.strip_suffix(']')?;
    let mut accounts: Vec<String> = Vec::new();
    for item in inner.split(',').map(str::trim) {
        if item.is_empty() {
            continue;
        }
        let name = item.strip_prefix('"')?.strip_suffix('"')?;
        if !unit_account_name(name) || name == "root" {
            return None;
        }
        if !accounts.iter().any(|known| known == name) {
            accounts.push(name.to_string());
        }
    }
    Some(accounts)
}

/// Reads the configuration; `Ok(None)` when there is none. The file must be
/// root's regular file that only root can change, reached without a
/// symbolic link.
pub(super) fn load() -> io::Result<Option<HelperConfig>> {
    let path = Path::new(CONFIG_FILE);
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let file = opened_trusted(path)?;
    let mut text = String::new();
    file.take(MAX_CONFIG_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_CONFIG_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{CONFIG_FILE} exceeds 64 KiB"),
        ));
    }
    HelperConfig::parse(&text).map(Some).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{CONFIG_FILE}: {error}"),
        )
    })
}

/// Writes the configuration (root only).
pub(super) fn store(config: &HelperConfig) -> io::Result<()> {
    ensure_root_directory(Path::new(CONFIG_DIRECTORY))?;
    write_root_file(Path::new(CONFIG_FILE), config.render().as_bytes(), 0o644)
}

/// Removes the file and its directory where it is empty (root only).
pub(super) fn remove() -> io::Result<bool> {
    let removed = match std::fs::remove_file(CONFIG_FILE) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    let _ = std::fs::remove_dir(CONFIG_DIRECTORY);
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_helper_config_reads_only_its_subset_of_toml() {
        let text = "# comment\n[[user]]\nuid = 1000 # Alice\naccounts = [\"se-remote\", \
                    \"deploy\",]\n\n[[user]]\nuid = 1001\n";
        let config = HelperConfig::parse(text).unwrap();
        assert!(config.allows(1000, "se-remote"));
        assert!(config.allows(1000, "deploy"));
        assert!(!config.allows(1001, "se-remote"));
        assert!(config.has_user(1001));
        assert!(!config.has_user(1002));
        assert_eq!(HelperConfig::parse(&config.render()).unwrap(), config);
        for invalid in [
            "uid = 1000\n",
            "[[user]]\nuid = 0\n",
            "[[user]]\nuid = 1000\n[[user]]\nuid = 1000\n",
            "[[user]]\nuid = 1000\naccounts = [\"root\"]\n",
            "[[user]]\nuid = 1000\naccounts = [\"a b\"]\n",
            "[[user]]\nuid = 1000\nshell = \"/bin/sh\"\n",
            "[[user]]\naccounts = [\"se-remote\"]\n",
            "[user]\nuid = 1000\n",
        ] {
            assert!(HelperConfig::parse(invalid).is_err(), "{invalid:?}");
        }
        let mut changed = HelperConfig::default();
        assert!(changed.allow(1000, "se-remote").unwrap());
        assert!(!changed.allow(1000, "se-remote").unwrap());
        assert!(changed.allow(1000, "root").is_err());
        assert!(changed.add_user(0).is_err());
        assert!(changed.disallow(1000, "se-remote"));
        assert!(!changed.disallow(1000, "se-remote"));
        assert!(changed.has_user(1000));
    }
}
