//! Sign-in with the account of the sharing device (plan 2026-11-17 P4, TODO
//! RDP-ZIELKONTO). The host names the account of the grant
//! (`AccountProfile`, as Exec and the machine login) and how it signs the
//! viewer in; the viewer hands that account to its RDP client. Portable:
//! account names and their checks, the wire state, the viewer's plan and the
//! client invocations, in which a password travels only on standard input or
//! in the session-bound Windows credential, never in argv or a file.
use crate::share::apps_wire::ManagedSecret;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Longest user or domain part se hands on.
pub const MAX_ACCOUNT_PART: usize = 256;
/// Longest password in characters (Windows accepts 256).
pub const MAX_PASSWORD: usize = 256;
/// The domain mstsc and Windows use for a Microsoft account.
pub const MICROSOFT_ACCOUNT_DOMAIN: &str = "MicrosoftAccount";
/// Prefix of the start error that asks the desktop GUI for the password.
pub const LOGIN_ENTRY_NEEDED: &str = "rdp_login_entry_needed";
/// Shown when the host does not answer `RdpLogin` (up to 0.5.197) or refuses it.
pub const LOGIN_FALLBACK: &str = "Anmeldung mit dem Konto des Geräts nicht verfügbar (Gegenstelle vor 0.5.198 oder abgelehnt); das RDP-Programm fragt nach Benutzer und Kennwort";
/// Windows workstations host one session: signing in as `se-remote`
/// disconnects the person signed in at the device (Windows asks them first).
pub const REMOTE_NOTICE: &str = "Die Anmeldung als se-remote trennt die Sitzung der am Gerät angemeldeten Person (Windows erlaubt dort eine Sitzung).";

/// Characters no Windows account name contains and no RDP field may carry:
/// `\` separates domain and user, the others are reserved in account names.
const FORBIDDEN: &[char] = &['\\', '/', '"', '[', ']', ':', ';', '|', '<', '>', '*', '?'];

/// A user or domain part: 1 to 256 bytes, no control character (a line
/// break would end a `.rdp` line or a FreeRDP argument), no reserved
/// separator, no surrounding blanks.
pub fn valid_account_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ACCOUNT_PART
        && value.trim() == value
        && !value
            .chars()
            .any(|c| c.is_control() || FORBIDDEN.contains(&c))
}

/// A password se hands on: 1 to 256 characters, no NUL and no line break
/// (FreeRDP reads one argument per line, `pam_exec` ends it at NUL).
pub fn valid_password(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_PASSWORD
        && !value.contains(['\0', '\n', '\r'])
}

/// The account of a host as RDP clients name it; checked on creation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAccount {
    username: String,
    domain: Option<String>,
}
impl HostAccount {
    pub fn new(username: &str, domain: Option<&str>) -> Result<Self, String> {
        if !valid_account_part(username) {
            return Err(format!("Ungültiger Kontoname {username:?}"));
        }
        if let Some(domain) = domain {
            if !valid_account_part(domain) {
                return Err(format!("Ungültige Domäne {domain:?}"));
            }
        }
        Ok(Self {
            username: username.to_string(),
            domain: domain.map(str::to_string),
        })
    }
    pub fn username(&self) -> &str {
        &self.username
    }
    pub fn domain(&self) -> Option<&str> {
        self.domain.as_deref()
    }
    /// `DOMAIN\user`, or the bare user without a domain.
    pub fn qualified(&self) -> String {
        match &self.domain {
            Some(domain) => format!("{domain}\\{}", self.username),
            None => self.username.clone(),
        }
    }
}

/// The Windows name of an account (assumption K2): a Microsoft account as
/// `MicrosoftAccount\<principal>` (`NetUserGetInfo` level 24), otherwise the
/// `NameSamCompatible` form `DOMAIN\user` (for a local account the domain is
/// the computer name).
pub fn windows_account(
    sam_compatible: &str,
    microsoft_principal: Option<&str>,
) -> Result<HostAccount, String> {
    if let Some(principal) = microsoft_principal.map(str::trim).filter(|p| !p.is_empty()) {
        return HostAccount::new(principal, Some(MICROSOFT_ACCOUNT_DOMAIN));
    }
    match sam_compatible.split_once('\\') {
        Some((domain, user)) => HostAccount::new(user, Some(domain)),
        None => HostAccount::new(sam_compatible, None),
    }
}

/// How the host signs the viewer in (wire field `state`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum LoginState {
    /// se sets the secret itself: `se-remote` with a new password per
    /// session, xrdp with a one-time token.
    Automatic,
    /// The password the viewer typed once, stored on the host.
    Stored,
    /// The password must be typed once (GUI dialog, `se rdp password`).
    EntryNeeded,
    /// No sign-in with the device's account here; the reason.
    Unsupported(String),
}
impl LoginState {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.len() > 1024 || value.chars().any(char::is_control) {
            return Err("RDP login state is oversized or contains control characters".into());
        }
        Ok(match value {
            "automatic" => Self::Automatic,
            "stored" => Self::Stored,
            "entry_needed" => Self::EntryNeeded,
            other => match other.strip_prefix("unsupported:") {
                Some(reason) if !reason.is_empty() => Self::Unsupported(reason.into()),
                _ => return Err(format!("unknown RDP login state {other:?}")),
            },
        })
    }
    /// The German line every surface shows.
    pub fn text(&self, account: Option<&str>) -> String {
        let account = account.unwrap_or("dem Konto des Geräts");
        match self {
            Self::Automatic => format!("Anmeldung als {account}: automatisch"),
            Self::Stored => {
                format!("Anmeldung als {account}: mit dem auf dem Gerät hinterlegten Kennwort")
            }
            Self::EntryNeeded => format!(
                "Anmeldung als {account}: Kennwort einmal hinterlegen (Geräteseite oder `se rdp password`), sonst fragt das RDP-Programm"
            ),
            Self::Unsupported(reason) => format!(
                "Anmeldung mit dem Konto des Geräts hier nicht möglich ({reason}); das RDP-Programm fragt nach dem Konto"
            ),
        }
    }
}
impl std::fmt::Display for LoginState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Automatic => f.write_str("automatic"),
            Self::Stored => f.write_str("stored"),
            Self::EntryNeeded => f.write_str("entry_needed"),
            Self::Unsupported(reason) => write!(f, "unsupported:{reason}"),
        }
    }
}
impl TryFrom<String> for LoginState {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Self::parse(&value)
    }
}
impl From<LoginState> for String {
    fn from(value: LoginState) -> Self {
        value.to_string()
    }
}

/// The checked content of an `RdpLoginInfo` reply.
pub struct LoginInfo {
    pub account: Option<HostAccount>,
    pub secret: Option<ManagedSecret>,
    pub state: LoginState,
}
impl std::fmt::Debug for LoginInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginInfo")
            .field("account", &self.account)
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .field("state", &self.state)
            .finish()
    }
}
impl LoginInfo {
    pub fn new(
        account: Option<HostAccount>,
        secret: Option<ManagedSecret>,
        state: LoginState,
    ) -> Self {
        Self {
            account,
            secret,
            state,
        }
    }
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::new(None, None, LoginState::Unsupported(reason.into()))
    }
    /// The reply fields as a peer sent them, checked: a password only for
    /// `automatic`/`stored` and only with an account.
    pub fn from_wire(
        username: String,
        domain: Option<String>,
        secret: Option<ManagedSecret>,
        state: LoginState,
    ) -> Result<Self, String> {
        let account = if username.is_empty() {
            None
        } else {
            Some(HostAccount::new(&username, domain.as_deref())?)
        };
        if let Some(secret) = &secret {
            let allowed = matches!(state, LoginState::Automatic | LoginState::Stored);
            if !allowed || account.is_none() || !valid_password(secret.expose()) {
                return Err("RDP-Anmeldeantwort enthält ein unzulässiges Kennwort".into());
            }
        }
        Ok(Self::new(account, secret, state))
    }
    /// `(username, domain, secret, state)` of the `RdpLoginInfo` reply.
    pub fn into_wire(self) -> (String, Option<String>, Option<ManagedSecret>, LoginState) {
        let (username, domain) = match self.account {
            Some(account) => (account.username, account.domain),
            None => (String::new(), None),
        };
        (username, domain, self.secret, self.state)
    }
    /// What surfaces show: account, state and the German line.
    pub fn view(&self) -> LoginView {
        let account = self.account.as_ref().map(HostAccount::qualified);
        let mut text = self.state.text(account.as_deref());
        let remote = crate::share::access_grant::REMOTE_ACCOUNT_NAME;
        if self
            .account
            .as_ref()
            .is_some_and(|account| account.username().eq_ignore_ascii_case(remote))
        {
            text.push(' ');
            text.push_str(REMOTE_NOTICE);
        }
        LoginView {
            account,
            state: self.state.to_string(),
            text,
        }
    }
}

/// The sign-in of one session as surfaces show it (`SessionView.login`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LoginView {
    pub account: Option<String>,
    /// `automatic`, `stored`, `entry_needed`, `unsupported:<grund>` or
    /// `unavailable` (the host did not answer `RdpLogin`).
    pub state: String,
    pub text: String,
}

/// What the viewer hands its client: the full credentials, only the
/// account (the client asks for the password), or nothing.
pub struct ViewerPlan {
    pub credentials: Option<(HostAccount, ManagedSecret)>,
    pub account: Option<HostAccount>,
    pub view: LoginView,
}

/// The viewer's decision from the host's answer. A failed `RdpLogin` (host
/// up to 0.5.197, refusal) starts as before and names the reason. With
/// `ask_password` (desktop GUI) a needed entry ends the start with
/// `LOGIN_ENTRY_NEEDED: <account>` before any tunnel exists.
pub fn viewer_plan(
    reply: Result<LoginInfo, String>,
    ask_password: bool,
) -> Result<ViewerPlan, String> {
    let info = match reply {
        Ok(info) => info,
        Err(error) => {
            return Ok(ViewerPlan {
                credentials: None,
                account: None,
                view: LoginView {
                    account: None,
                    state: "unavailable".into(),
                    text: format!("{LOGIN_FALLBACK} ({error})"),
                },
            })
        }
    };
    let view = info.view();
    match (info.state, info.account, info.secret) {
        (LoginState::Automatic | LoginState::Stored, Some(account), Some(secret)) => {
            Ok(ViewerPlan {
                credentials: Some((account.clone(), secret)),
                account: Some(account),
                view,
            })
        }
        (LoginState::EntryNeeded, Some(account), _) if ask_password => {
            Err(format!("{LOGIN_ENTRY_NEEDED}: {}", account.qualified()))
        }
        (_, account, _) => Ok(ViewerPlan {
            credentials: None,
            account,
            view,
        }),
    }
}

/// The account of a `LOGIN_ENTRY_NEEDED` start error, for the GUI dialog.
pub fn entry_needed_account(error: &str) -> Option<&str> {
    error
        .strip_prefix(LOGIN_ENTRY_NEEDED)?
        .strip_prefix(": ")
        .filter(|account| !account.is_empty())
}

/// How mstsc starts: the whole desktop by address (`mstsc /v:`), because
/// mstsc warns with redirection checkboxes on every opened unsigned `.rdp`
/// file since April 2026 (CVE-2026-26151) and not for a start by name; a
/// RemoteApp needs its profile file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MstscStart {
    Address(String),
    Profile,
}
pub fn mstsc_start(listener: SocketAddr, remote_app: Option<&str>) -> MstscStart {
    match remote_app {
        None => MstscStart::Address(format!("/v:{listener}")),
        Some(_) => MstscStart::Profile,
    }
}

/// The RemoteApp profile of mstsc: the device address, explicit certificate
/// warning and CredSSP, the published alias and the host's account
/// (`username:s:`, `domain:s:`); never a password.
pub fn mstsc_profile_body(
    listener: SocketAddr,
    alias: Option<&str>,
    account: Option<&HostAccount>,
) -> Result<String, String> {
    let mut body = format!(
        "full address:s:{listener}\r\nauthentication level:i:2\r\nenablecredsspsupport:i:1\r\nprompt for credentials:i:0\r\n"
    );
    if let Some(account) = account {
        body.push_str(&format!("username:s:{}\r\n", account.username()));
        if let Some(domain) = account.domain() {
            body.push_str(&format!("domain:s:{domain}\r\n"));
        }
    }
    if let Some(alias) = alias {
        if !super::PublishedApp::valid_alias(alias) {
            return Err("Invalid published RemoteApp alias".into());
        }
        body.push_str(&format!("disableconnectionsharing:i:1\r\nremoteapplicationmode:i:1\r\nremoteapplicationprogram:s:||{alias}\r\nremoteapplicationname:s:{alias}\r\nalternate shell:s:||{alias}\r\n"));
    } else {
        body.push_str("remoteapplicationmode:i:0\r\n");
    }
    Ok(body)
}

/// FreeRDP 3 with `/args-from:stdin` (`client/common/cmdline.c`): argv holds
/// only that switch; every argument comes from standard input, one per line,
/// the password (`/p:`) last. Returns argv and the input; the input is wiped
/// when dropped.
pub fn freerdp_stdin_invocation(
    args: &[String],
    username: &str,
    domain: Option<&str>,
    password: &str,
) -> Result<(Vec<String>, zeroize::Zeroizing<Vec<u8>>), String> {
    fn line_safe(value: &str) -> bool {
        !value.contains(['\n', '\r', '\0'])
    }
    fn push_line(input: &mut Vec<u8>, text: &str) {
        input.extend_from_slice(text.as_bytes());
        input.push(b'\n');
    }
    if !line_safe(username)
        || !domain.is_none_or(line_safe)
        || !line_safe(password)
        || !args.iter().all(|arg| line_safe(arg.as_str()))
    {
        return Err("RDP credentials contain a line break and cannot be handed over".into());
    }
    let mut input: zeroize::Zeroizing<Vec<u8>> = zeroize::Zeroizing::new(Vec::new());
    for arg in args {
        push_line(&mut input, arg.as_str());
    }
    push_line(&mut input, &format!("/u:{username}"));
    if let Some(domain) = domain {
        push_line(&mut input, &format!("/d:{domain}"));
    }
    let secret = zeroize::Zeroizing::new(format!("/p:{password}"));
    push_line(&mut input, secret.as_str());
    Ok((vec!["/args-from:stdin".to_string()], input))
}

/// FreeRDP arguments of an account without a password (the client asks for
/// it): user and domain are no secret and may stand in argv.
pub fn freerdp_account_args(account: &HostAccount) -> Vec<String> {
    let mut args = vec![format!("/u:{}", account.username())];
    if let Some(domain) = account.domain() {
        args.push(format!("/d:{domain}"));
    }
    args
}

#[cfg(test)]
#[path = "login_task_tests.rs"]
mod login_task_tests;
