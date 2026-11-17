//! `se rdp`: Remote Desktop with one command (design 2.9, user rule r3 „die
//! eine freigabe muss reichen, der rest muss laufen“). `se rdp open
//! <device>` derives relation, fresh authenticated probe, host setup,
//! service, client, certificate trust and the session's life itself and
//! owns the native client until it ends. Older switches stay accepted so
//! scripts keep working; they no longer change anything. No CLI-supplied
//! inventory or certificate pin; the only password is the one `se rdp
//! password` reads without echo (or from standard input) and hands to the
//! device, which checks and stores it (plan 2026-11-17 P4).
use crate::desktop::core::tiles::identity_of;
use crate::{rdp::AuthenticatedProbe, share::PeerOpenTarget};
use clap::{Args, Subcommand};
use serde_json::json;

#[derive(Args)]
pub(super) struct RdpArgs {
    #[command(subcommand)]
    command: RdpCommand,
}
#[derive(Subcommand)]
enum RdpCommand {
    /// Read this host's real RDP services without changing its setup.
    LocalProbe {
        #[arg(long)]
        json: bool,
    },
    Probe {
        device: String,
        #[arg(long)]
        json: bool,
    },
    Clients {
        #[arg(long)]
        json: bool,
    },
    /// Open the device's desktop in this computer's RDP program; this
    /// command owns the program until it ends.
    Open(OpenArgs),
    List {
        #[arg(long)]
        json: bool,
    },
    Status {
        id: String,
        #[arg(long)]
        json: bool,
    },
    Close {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// The password of the device's account, typed once: the device checks
    /// it and stores it for this computer, so Remote Desktop signs in
    /// without asking. Read from the terminal without echo, or from standard
    /// input; never an argument.
    Password(PasswordArgs),
    /// The fixed elevated steps of this host's RDP and login self-setup,
    /// started through `pkexec` or UAC by the setup itself (design 3.3,
    /// E44). `--system` once or several times: the steps run in this order in
    /// this one process, one line each (`state=…`, `login_state=…`); the exit
    /// code is the first one other than 0, otherwise 0.
    #[command(hide = true)]
    HostSetup {
        #[arg(long, required = true)]
        system: Vec<String>,
        /// Windows: the signed-in user's SID the elevated setup admits.
        #[arg(long)]
        user_sid: Option<String>,
    },
}

/// `se rdp password <device> [--forget | --status]` (plan 2026-11-17 P4).
#[derive(Args, Debug, Default, PartialEq, Eq)]
pub(crate) struct PasswordArgs {
    /// Share endpoint, saved device name, device ID or fingerprint prefix.
    pub(crate) device: String,
    /// Deletes the password the device stored for this computer.
    #[arg(long, conflicts_with = "status")]
    pub(crate) forget: bool,
    /// Shows the account and how the device signs this computer in.
    #[arg(long)]
    pub(crate) status: bool,
    #[arg(long)]
    pub(crate) json: bool,
}

/// `se rdp open`: only the device, optionally one published RemoteApp.
#[derive(Args, Debug, Default, PartialEq, Eq)]
pub(crate) struct OpenArgs {
    /// Share endpoint, saved device name, device ID or fingerprint prefix.
    pub(crate) device: String,
    /// Exact alias of a RemoteApp the device publishes (Windows Server/RDS).
    #[arg(long)]
    pub(crate) remote_app: Option<String>,
    #[arg(long)]
    pub(crate) json: bool,
    /// se Desktop starts this command without a window: report a failure
    /// as a system notice.
    #[arg(long, hide = true)]
    pub(crate) notify_errors: bool,
    /// Overrides the derived service (scripts of 0.5.188 and earlier).
    #[arg(long, hide = true)]
    pub(crate) service: Option<String>,
    /// Overrides the derived client (scripts of 0.5.188 and earlier).
    #[arg(long, hide = true)]
    pub(crate) client: Option<String>,
    #[arg(long, hide = true)]
    pub(crate) certificate_policy: Option<String>,
    #[arg(long, hide = true)]
    pub(crate) acknowledge_local_access: bool,
    #[arg(long, hide = true)]
    pub(crate) acknowledge_session_effect: bool,
    #[arg(long, hide = true)]
    pub(crate) acknowledge_certificate_dialog: bool,
    #[arg(long, hide = true)]
    pub(crate) lifetime_secs: Option<u32>,
    /// An explicit limit in seconds for the device's answers; without it
    /// the command waits until the device answers.
    #[arg(long, hide = true)]
    pub(crate) wait: Option<u32>,
}

impl OpenArgs {
    /// The derived choices with the overrides this command still honors:
    /// `--service`, `--client`, `--wait` and `--remote-app`.
    pub(crate) fn options(&self) -> Result<crate::rdp::OpenOptions, String> {
        let wait = match self.wait {
            None => None,
            Some(0) => return Err("--wait must be at least one second".into()),
            Some(seconds) => Some(std::time::Duration::from_secs(seconds.into())),
        };
        Ok(crate::rdp::OpenOptions {
            remote_app: self.remote_app.clone(),
            service: self.service.clone(),
            client: self.client.clone(),
            wait,
            ..Default::default()
        })
    }

    /// The switches this version ignores, for the notice on stderr.
    pub(crate) fn ignored(&self) -> Vec<&'static str> {
        let mut ignored = Vec::new();
        for (name, given) in [
            ("--certificate-policy", self.certificate_policy.is_some()),
            ("--acknowledge-local-access", self.acknowledge_local_access),
            (
                "--acknowledge-session-effect",
                self.acknowledge_session_effect,
            ),
            (
                "--acknowledge-certificate-dialog",
                self.acknowledge_certificate_dialog,
            ),
            ("--lifetime-secs", self.lifetime_secs.is_some()),
        ] {
            if given {
                ignored.push(name);
            }
        }
        ignored
    }
}

/// Device ID, public key and name of the device `selector` names. An exact
/// relation endpoint (se Desktop passes its tile's) names the device even
/// when its room is stored under the local profile ID.
pub(crate) fn device_identity(selector: &str) -> Result<(String, String, String), String> {
    let snapshot = crate::daemon::share_worker_snapshot()?;
    if let Some((target, path)) = PeerOpenTarget::from_endpoint(selector.trim()) {
        if path == "/" {
            if let Some((device, key)) = identity_of(&snapshot.profiles, &target) {
                return Ok((device.clone(), key, device));
            }
        }
    }
    let choices = super::screen::device_choices(&snapshot.profiles, crate::share::core_now_secs());
    let choice = super::screen::select_from(&choices, selector)?;
    let name = choice.name;
    let (device, key) = identity_of(&snapshot.profiles, &choice.target)
        .ok_or_else(|| format!("Die Geräteidentität von {name} ist nicht bestätigt."))?;
    Ok((device, key, name))
}

pub(super) fn run(
    args: RdpArgs,
    mut probe: impl FnMut(PeerOpenTarget) -> Result<AuthenticatedProbe, String>,
) -> Result<i32, String> {
    use crate::share::tcp_forward::TunnelId;
    match args.command {
        RdpCommand::LocalProbe { json } => {
            let value = crate::rdp::probe_local_host();
            output(
                json,
                &serde_json::to_value(&value).map_err(|e| e.to_string())?,
            );
        }
        RdpCommand::Probe { device, json } => {
            let (target, _) = super::screen::device_target(&device)?;
            let report = probe(target)?;
            output(
                json,
                &serde_json::to_value(report.report()).map_err(|e| e.to_string())?,
            );
        }
        RdpCommand::Clients { json } => {
            output(json, &json!({"clients":crate::rdp::local_clients()}))
        }
        RdpCommand::Open(open) => {
            let notify = open.notify_errors;
            let result = open_and_wait(&open);
            if let (Err(error), true) = (&result, notify) {
                crate::notify_desktop::notify_access("Remote Desktop", error);
            }
            result?;
        }
        RdpCommand::List { json } => output(json, &json!({"tunnels":crate::daemon::tcp_list()?})),
        RdpCommand::Status { id, json } => output(
            json,
            &json!({"tunnel":crate::daemon::tcp_status(TunnelId::parse(&id)?)?}),
        ),
        RdpCommand::Close { id, json } => output(
            json,
            &json!({"id":id,"closed":crate::rdp::close(TunnelId::parse(&id)?)?}),
        ),
        RdpCommand::Password(args) => password(&args)?,
        RdpCommand::HostSetup { system, user_sid } => {
            let (lines, code) =
                crate::rdp::host_setup::run_system_steps(&system, user_sid.as_deref());
            for line in lines {
                println!("{line}");
            }
            return Ok(code);
        }
    }
    Ok(0)
}

/// Opens the device and waits as the session's owner until the native
/// client ends, without a time limit. Ending this command ends the client,
/// as before.
fn open_and_wait(open: &OpenArgs) -> Result<(), String> {
    let ignored = open.ignored();
    if !ignored.is_empty() {
        eprintln!(
            "Ignoriert (se wählt Vertrauen und Lebensdauer selbst): {}",
            ignored.join(", ")
        );
    }
    let options = open.options()?;
    let (device, key, name) = device_identity(&open.device)?;
    eprintln!("Remote Desktop zu {name} wird geöffnet …");
    let waiting = || eprintln!("Warte auf die Antwort von {name} …");
    let mut session = crate::rdp::open_device_session(&device, &key, options, waiting)?;
    let view = session.view();
    if open.json {
        let value = serde_json::to_value(view).map_err(|e| e.to_string())?;
        println!("{}", json!({ "session": value }));
    } else {
        eprintln!(
            "{}\nRDP tunnel={} client_pid={}",
            view.session_notice,
            view.tunnel.id.as_str(),
            view.process_id
        );
    }
    let status = session.wait()?;
    if !status.success() {
        return Err(format!("Native RDP client exited with {status}"));
    }
    Ok(())
}

/// `se rdp password`: the state, the forgetting, or the password typed once
/// (decision 3 of P4). The password leaves this computer only towards the
/// device over the authenticated share connection.
fn password(args: &PasswordArgs) -> Result<(), String> {
    use crate::rdp::core::LoginState;
    use crate::rdp::viewer_login::{login_status, store_password};
    let (device, key, name) = device_identity(&args.device)?;
    let info = if args.forget {
        store_password(&device, &key, None, None)?
    } else if args.status {
        login_status(&device, &key, None)?
    } else {
        let info = login_status(&device, &key, None)?;
        match &info.state {
            LoginState::Automatic => {
                return Err(format!("{}; es braucht kein Kennwort", info.view().text))
            }
            LoginState::Unsupported(_) => return Err(info.view().text),
            LoginState::EntryNeeded | LoginState::Stored => {}
        }
        let Some(account) = info.account.as_ref().map(|account| account.qualified()) else {
            return Err(info.view().text);
        };
        let prompt = format!(
            "Kennwort von {account} auf {name}; wird auf {name} gespeichert (Eingabe verborgen): "
        );
        let secret = crate::share::apps_wire::ManagedSecret::new(read_password(&prompt)?);
        store_password(&device, &key, Some(secret), None)?
    };
    let view = info.view();
    if args.json {
        println!("{}", json!({ "login": view }));
    } else {
        println!("{}", view.text);
    }
    Ok(())
}

/// The password without echo from a terminal, otherwise one line of
/// standard input; never empty.
fn read_password(prompt: &str) -> Result<String, String> {
    use std::io::IsTerminal;
    let typed = if std::io::stdin().is_terminal() {
        super::os::read_hidden_line(prompt)?
    } else {
        super::setup::read_stdin_secret()?
    };
    if typed.is_empty() {
        return Err("Kein Kennwort eingegeben".into());
    }
    Ok(typed)
}

fn output(json: bool, value: &serde_json::Value) {
    if json {
        println!("{value}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
    }
}
