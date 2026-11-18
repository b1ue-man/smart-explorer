//! Linux side of the sign-in with the grant's account (plan 2026-11-17 P4):
//! xrdp signs the viewer in with a one-time se token (decision 6,
//! `xrdp_token.rs`), for every account of the grant this worker may use (as
//! Exec). KRdp signs in with se's own KRdp user when the grant names the
//! account of the Plasma session KRdp shows (`krdp_login.rs`). The GNOME
//! desktop sharing se set up keeps its own credentials (`RdpCredential`);
//! the other GNOME modes name their reason. se checks no typed Linux
//! password. A root worker's own account is the account at the desktop
//! (`session_account.rs`), since xrdp refuses root by default.
use super::host_setup::SystemStep;
use super::{krdp_login, session_account, xrdp_token};
use crate::rdp::core::xrdp_login::{root_login_allowed, user_allowed, SESMAN_INI};
use crate::rdp::core::{HostAccount, LoginInfo, SetupState};
use crate::rdp::os::shared::host_login::{Automatic, Viewer};
use crate::share::access_grant::AccountProfile;
use crate::share::apps_wire::ManagedSecret;
use std::sync::{Mutex, PoisonError};

const XRDP: &str = "xrdp";
const KRDP: &str = "krdp";

pub fn supports(service_id: &str) -> Result<(), String> {
    match service_id {
        XRDP | KRDP => Ok(()),
        "gnome-console" => Err(
            "die von se eingerichtete GNOME-Desktopfreigabe meldet mit eigenen Zugangsdaten an"
                .into(),
        ),
        other => Err(format!(
            "{other} meldet mit dem Systemkonto an; se gibt dafür kein Kennwort weiter"
        )),
    }
}

/// The account of the grant: the worker's own, `se-remote` or the named
/// one; Linux accounts have no domain.
pub fn account(profile: &AccountProfile) -> Result<HostAccount, String> {
    let name = match profile.account_name() {
        Some(name) => name.to_string(),
        None => own_account()?,
    };
    HostAccount::new(&name, None)
}

/// The worker's own account. For a root worker that is the person at the
/// desktop (active local graphical session); without one `root` only where
/// xrdp-sesman admits root (`AllowRootLogin`).
fn own_account() -> Result<String, String> {
    if !xrdp_token::running_as_root() {
        return xrdp_token::own_user()
            .ok_or_else(|| "Konto des Hintergrunddienstes nicht ermittelbar".to_string());
    }
    if let Some(user) = session_account::desktop_account() {
        return Ok(user);
    }
    let ini = sesman_ini();
    if std::fs::read_to_string(&ini).is_ok_and(|text| root_login_allowed(&text)) {
        return Ok("root".into());
    }
    Err(format!(
        "Der Hintergrunddienst läuft als root und an diesem Gerät ist keine grafische Sitzung eines anderen Kontos aktiv; xrdp lässt root nicht zu (AllowRootLogin in {} fehlt oder ist aus). Eine Freigabe mit benanntem Konto oder eine Anmeldung am Gerät behebt das",
        ini.display()
    ))
}

/// The sesman configuration of the xrdp-sesman unit (`--config`/`-c`),
/// else the packaged default.
fn sesman_ini() -> std::path::PathBuf {
    super::xrdp::sesman_config().unwrap_or_else(|| SESMAN_INI.into())
}

/// The result of this process's root step `xrdp-login` (one approval per
/// process; the removal after the last „Bildschirm steuern“ resets it).
static STEP: Mutex<Option<SetupState>> = Mutex::new(None);

/// xrdp: a token for this session; the PAM line and the token directory are
/// set up first when they are missing (root, or one polkit approval).
pub fn automatic(
    service_id: &str,
    _profile: &AccountProfile,
    account: &HostAccount,
    viewer: &Viewer,
    status_only: bool,
) -> Automatic {
    if service_id == KRDP {
        // KRdp shows the session of the account this worker runs as.
        let own = xrdp_token::own_user().unwrap_or_default();
        return krdp_login::automatic(&own, account, viewer, status_only);
    }
    if service_id != XRDP {
        return Automatic::No;
    }
    // SAFETY: geteuid takes no pointer and has no side effects.
    let owner = unsafe { libc::geteuid() };
    let user = account.username();
    let user_uid = xrdp_token::user_uid(user);
    let listed = owner != 0
        && user_uid.is_some_and(|uid| uid != owner)
        && crate::share::exec_account_helper_allows(owner, user);
    if !user_allowed(owner, user_uid, listed) {
        return Automatic::Unsupported(format!(
            "die xrdp-Anmeldung als {user} ist für den Hintergrunddienst nicht freigegeben (root, unbekannt oder nicht im Kontodienst)"
        ));
    }
    if status_only {
        return Automatic::Available;
    }
    if let Err(reason) = ready() {
        return Automatic::Unsupported(reason);
    }
    let port = match super::xrdp::configured_port(&super::xrdp::config_path()) {
        Ok(port) => port,
        Err(reason) => return Automatic::Unsupported(format!("xrdp-Port: {reason}")),
    };
    match xrdp_token::issue(user, port, viewer) {
        Ok(token) => Automatic::Secret(ManagedSecret::new(token)),
        Err(reason) => Automatic::Unsupported(reason),
    }
}

/// The PAM line and this worker's token directory, set up once if missing.
fn ready() -> Result<(), String> {
    if xrdp_token::ready().is_ok() {
        return Ok(());
    }
    let mut step = STEP.lock().unwrap_or_else(PoisonError::into_inner);
    let state = step.get_or_insert_with(|| super::host_setup::system(SystemStep::XrdpLogin));
    match state {
        SetupState::Done | SetupState::NotNeeded => xrdp_token::ready().map(|_| ()),
        other => Err(format!(
            "xrdp-Anmeldung mit Einmal-Token nicht eingerichtet: {}",
            other.text()
        )),
    }
}

/// Unter Linux prüft se kein Kennwort: xrdp meldet mit einem Token an.
pub fn verify(_account: &HostAccount, _password: &str) -> Result<bool, String> {
    Err("Unter Linux meldet xrdp mit einem Einmal-Token von se an; se prüft und speichert hier kein Kennwort".into())
}

/// A right that ended during the request: the viewer's tokens go, and a
/// KRdp password it was just given is replaced.
pub fn withdraw(viewer: &Viewer, _issued: &LoginInfo) {
    xrdp_token::revoke(|record| {
        record.device_id == viewer.device_id && record.public_key == viewer.public_key
    });
    krdp_login::coverage_ended(&|known: &Viewer| known != viewer, true);
}

/// „Bildschirm steuern“ ended for a viewer: its tokens go; after the last
/// one every token goes, and a root worker also takes the PAM line out
/// (decision 6, 7). A worker without root keeps the line: removing it would
/// ask for polkit approval now and again at the next setup, and without a
/// listed token the line accepts nobody (manager decision 2026-11-17: no
/// prompts during use).
pub fn coverage_ended(steers: &dyn Fn(&Viewer) -> bool, steering_remains: bool) {
    xrdp_token::revoke(|record| {
        !steering_remains || !steers(&Viewer::new(&record.device_id, &record.public_key))
    });
    krdp_login::coverage_ended(steers, steering_remains);
    if steering_remains || !xrdp_token::pam_installed() || !xrdp_token::running_as_root() {
        return;
    }
    let mut step = STEP.lock().unwrap_or_else(PoisonError::into_inner);
    let state = super::host_setup::system(SystemStep::XrdpLoginRemove);
    if !matches!(state, SetupState::Done | SetupState::NotNeeded) {
        eprintln!(
            "RDP-Anmeldung: se-Zeile in xrdp-sesman nicht entfernt: {}",
            state.text()
        );
    }
    *step = None;
}
