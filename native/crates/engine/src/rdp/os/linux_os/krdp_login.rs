//! KRdp sign-in with the grant's account (RDP-ZIELKONTO, second step), the
//! I/O: se's KRdp user `se-krdp` in `krdpserverrc` (`kreadconfig6`,
//! `kwriteconfig6`) and its password in the session's KWallet under
//! folder/application `KRDP`, exactly where KRdp's QtKeychain reads it
//! (`core::krdp_login`). KRdp reads the password only when it starts, so a
//! change restarts its user unit (`try-restart`: a stopped KRdp stays
//! stopped). se never opens a locked wallet: that would ask the person.
use super::krdp::{general, KRDP_UNIT};
use crate::rdp::core::krdp_login::{
    account_allowed, coverage_change, new_password, users_with, users_without, CoverageChange,
    KEYCHAIN_SERVICE, KRDP_USER,
};
use crate::rdp::core::HostAccount;
use crate::rdp::os::shared::host_login::{Automatic, Viewer};
use crate::rdp::os::shared::process::find_tool;
use crate::share::apps_wire::ManagedSecret;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use zbus::blocking::{Connection, Proxy};

/// kwalletd6, then kwalletd5 (QtKeychain's order on Plasma 6 and 5).
const WALLETS: [(&str, &str); 2] = [
    ("org.kde.kwalletd6", "/modules/kwalletd6"),
    ("org.kde.kwalletd5", "/modules/kwalletd5"),
];

/// One provisioning or rotation at a time.
static CHANGE: Mutex<()> = Mutex::new(());

/// The open network wallet and the handle KWallet gave application `KRDP`.
struct Wallet {
    proxy: Proxy<'static>,
    handle: i32,
}

impl Wallet {
    /// The session's network wallet when it is open already.
    fn open() -> Result<Self, String> {
        let connection = Connection::session()
            .map_err(|error| format!("Sitzungsbus nicht erreichbar: {error}"))?;
        for (service, path) in WALLETS {
            let Ok(proxy) = Proxy::new(&connection, service, path, "org.kde.KWallet") else {
                continue;
            };
            let wallet: String = match proxy.call("networkWallet", &()) {
                Ok(wallet) => wallet,
                Err(_) => continue,
            };
            let open: bool = proxy
                .call("isOpen", &(wallet.as_str(),))
                .map_err(|error| format!("KWallet: {error}"))?;
            if !open {
                return Err(format!("KWallet „{wallet}“ ist gesperrt; se öffnet es nicht, weil das eine Abfrage an der Sitzung auslöste. Nach dem Entsperren (Anmeldung an der Sitzung) meldet se automatisch an"));
            }
            let handle: i32 = proxy
                .call("open", &(wallet.as_str(), 0i64, KEYCHAIN_SERVICE))
                .map_err(|error| format!("KWallet: {error}"))?;
            if handle < 0 {
                return Err(format!(
                    "KWallet verweigert der Anwendung {KEYCHAIN_SERVICE} den Zugriff"
                ));
            }
            return Ok(Self { proxy, handle });
        }
        Err("Kein KWallet-Dienst in dieser Sitzung (kwalletd6 oder kwalletd5); KRdp liest seine Kennwörter dort".into())
    }

    fn read(&self) -> Result<String, String> {
        self.proxy
            .call(
                "readPassword",
                &(self.handle, KEYCHAIN_SERVICE, KRDP_USER, KEYCHAIN_SERVICE),
            )
            .map_err(|error| format!("KWallet lesen: {error}"))
    }

    fn write(&self, password: &str) -> Result<(), String> {
        let code: i32 = self
            .proxy
            .call(
                "writePassword",
                &(
                    self.handle,
                    KEYCHAIN_SERVICE,
                    KRDP_USER,
                    password,
                    KEYCHAIN_SERVICE,
                ),
            )
            .map_err(|error| format!("KWallet schreiben: {error}"))?;
        if code != 0 {
            return Err(format!("KWallet schreiben: Code {code}"));
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        let _: i32 = self
            .proxy
            .call(
                "removeEntry",
                &(self.handle, KEYCHAIN_SERVICE, KRDP_USER, KEYCHAIN_SERVICE),
            )
            .map_err(|error| format!("KWallet löschen: {error}"))?;
        Ok(())
    }
}

/// `kwriteconfig6` for `krdpserverrc` `[General] Users`; an empty list
/// deletes the key.
fn write_users(value: &str) -> Result<(), String> {
    let tool = find_tool(&["kwriteconfig6"]).ok_or("kwriteconfig6 fehlt")?;
    let mut args = vec![
        "--file",
        "krdpserverrc",
        "--group",
        "General",
        "--key",
        "Users",
    ];
    if value.is_empty() {
        args.push("--delete");
    } else {
        args.push(value);
    }
    super::host_setup::run_tool(&tool, &args, None).map(|_| ())
}

/// KRdp reads users and passwords when it starts.
fn restart() -> Result<(), String> {
    let systemctl = find_tool(&["systemctl"]).ok_or("systemctl fehlt")?;
    super::host_setup::run_tool(&systemctl, &["--user", "try-restart", KRDP_UNIT], None).map(|_| ())
}

/// The devices that received the current password (bindings only, no
/// secret): a steering end of one of them replaces it.
fn recipients_path() -> PathBuf {
    crate::support_dirs::sync_data_dir().join("rdp-krdp-recipients.json")
}

fn read_recipients() -> Vec<Viewer> {
    std::fs::read_to_string(recipients_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_recipients(viewers: &[Viewer]) {
    let path = recipients_path();
    let staged = path.with_extension("json.tmp");
    let written = serde_json::to_vec(viewers)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&staged, bytes))
        .and_then(|()| std::fs::rename(&staged, &path));
    if let Err(error) = written {
        eprintln!("RDP-Anmeldung: Empfänger des KRdp-Kennworts nicht speicherbar: {error}");
    }
}

/// The KRdp user and its password, added once; `status_only` checks the
/// account only.
pub(super) fn automatic(
    own: &str,
    account: &HostAccount,
    viewer: &Viewer,
    status_only: bool,
) -> Automatic {
    if let Err(reason) = account_allowed(own, account.username()) {
        return Automatic::Unsupported(reason);
    }
    let service_account = match HostAccount::new(KRDP_USER, None) {
        Ok(found) => found,
        Err(reason) => return Automatic::Unsupported(reason),
    };
    if status_only {
        return Automatic::Service {
            account: service_account,
            secret: None,
        };
    }
    match provision(viewer) {
        Ok(password) => Automatic::Service {
            account: service_account,
            secret: Some(ManagedSecret::new(password)),
        },
        Err(reason) => Automatic::Unsupported(format!("KRdp-Anmeldung: {reason}")),
    }
}

/// The password of se's KRdp user; a missing user or password is added and
/// KRdp restarted so that it reads them. `viewer` is noted as a recipient.
fn provision(viewer: &Viewer) -> Result<String, String> {
    let _change = CHANGE.lock().unwrap_or_else(PoisonError::into_inner);
    let wallet = Wallet::open()?;
    let mut password = wallet.read()?;
    let mut changed = false;
    let mut recipients = read_recipients();
    if password.is_empty() {
        password = new_password()?;
        wallet.write(&password)?;
        recipients.clear();
        changed = true;
    }
    if let Some(users) = users_with(&general("Users")?, KRDP_USER) {
        write_users(&users)?;
        changed = true;
    }
    if !recipients.contains(viewer) {
        recipients.push(viewer.clone());
    }
    write_recipients(&recipients);
    if changed {
        restart()?;
    }
    Ok(password)
}

/// A device's right ended (or a request was undone): when it had received
/// the password and others still steer, se's KRdp user gets a new one;
/// after the last steering device the user goes from KRdp's list and the
/// wallet (`coverage_change`). Either restarts a running KRdp.
pub(super) fn coverage_ended(steers: &dyn Fn(&Viewer) -> bool, steering_remains: bool) {
    // No KRdp here, or a root worker that never added the user.
    // SAFETY: geteuid takes no pointer and has no side effects.
    if find_tool(&["krdpserver"]).is_none() || unsafe { libc::geteuid() } == 0 {
        return;
    }
    let _change = CHANGE.lock().unwrap_or_else(PoisonError::into_inner);
    let recipients = read_recipients();
    let recipient_ended = recipients.iter().any(|viewer| !steers(viewer));
    let result = (|| -> Result<(), String> {
        let listed = users_without(&general("Users")?, KRDP_USER);
        if !recipient_ended && steering_remains {
            return Ok(());
        }
        let wallet = Wallet::open()?;
        let exists = !wallet.read()?.is_empty() || listed.is_some();
        match coverage_change(exists, recipient_ended, steering_remains) {
            CoverageChange::Keep => return Ok(()),
            CoverageChange::Rotate => wallet.write(&new_password()?)?,
            CoverageChange::Remove => {
                wallet.remove()?;
                if let Some(users) = listed {
                    write_users(&users)?;
                }
            }
        }
        write_recipients(&[]);
        restart()
    })();
    if let Err(error) = result {
        eprintln!("RDP-Anmeldung: KRdp-Kennwort von {KRDP_USER} nicht erneuert: {error}");
    }
}
