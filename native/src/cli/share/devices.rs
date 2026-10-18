//! `se share device …`: the per-device settings of the desktop device page
//! and the Android device menu – connect and open automatically, open my
//! exports for the device as well (Rückfreigabe), and reset its pinned
//! identity. Devices are the saved direct contacts; their authorization,
//! write right and access (account, commands, terminal, end; D.2.4, the
//! same fields as `se share grants`) are listed with them.
use clap::{Args, Subcommand};
use serde_json::Value;

use crate::share::access_grant::{AccessGrantView, Capability};
use crate::share::{DirectContact, DirectGrant, ExecGrant, ShareProfiles};

#[derive(Args)]
pub(super) struct DeviceArgs {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Option<DeviceCommand>,
}

#[derive(Subcommand)]
enum DeviceCommand {
    #[command(
        about = "List saved devices with their settings, authorization, write right and Exec (default)"
    )]
    List,
    #[command(about = "Change the settings of one saved device")]
    Set(SetArgs),
    #[command(
        about = "Forget the pinned identity of one device and withdraw its authorization (Exec off); it must be confirmed again"
    )]
    ResetTrust(Selector),
}

#[derive(Args)]
struct Selector {
    #[arg(
        allow_hyphen_values = true,
        help = "Device id, name, contact id or fingerprint shown by `se share device list`; omit when exactly one device is saved"
    )]
    selector: Option<String>,
}

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("setting")
        .args(["auto_connect", "auto_open", "share_back"])
        .required(true)
        .multiple(true)
))]
struct SetArgs {
    #[command(flatten)]
    device: Selector,
    #[arg(
        long,
        value_name = "true|false",
        help = "Connect to this device automatically"
    )]
    auto_connect: Option<bool>,
    #[arg(
        long,
        value_name = "true|false",
        help = "Open this device's files automatically once it is reachable"
    )]
    auto_open: Option<bool>,
    #[arg(
        long,
        value_name = "true|false",
        help = "Also open my exports for this device (Rückfreigabe)"
    )]
    share_back: Option<bool>,
}

pub(super) fn run(args: DeviceArgs) -> Result<(), String> {
    match args.command.unwrap_or(DeviceCommand::List) {
        DeviceCommand::List => list(args.json),
        DeviceCommand::Set(set) => {
            let contact = select(&super::checked_profiles()?, set.device.selector.as_deref())?;
            let now = crate::share::core_now_secs();
            let changed = persist(|profiles| {
                let mut changed =
                    profiles.set_contact_options(&contact.id, set.auto_connect, set.auto_open)?;
                if let Some(share_back) = set.share_back {
                    changed |= profiles.set_contact_share_back(&contact.id, share_back, now)?;
                }
                Ok(changed)
            })?;
            report(&contact, "settings", changed)
        }
        DeviceCommand::ResetTrust(selector) => {
            let contact = select(&super::checked_profiles()?, selector.selector.as_deref())?;
            let now = crate::share::core_now_secs();
            let changed = persist(|profiles| profiles.reset_contact_trust(&contact.id, now))?;
            report(&contact, "trust_reset", changed)
        }
    }
}

fn list(json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let access = AccessFields::now();
    if json {
        let devices = profiles
            .direct_contacts
            .iter()
            .map(|contact| value(&profiles, contact, &access))
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "count": devices.len(),
                "devices": devices,
            }))
            .map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    if profiles.direct_contacts.is_empty() {
        println!("devices\t0\tadd one with `se connections add-peer --code …` or `se share discoverable`");
    }
    for contact in &profiles.direct_contacts {
        println!("{}", text(&profiles, contact, &access));
    }
    Ok(())
}

/// The access of a grant as `se share grants` shows it (D.2.4): `exec.enabled`
/// means commands or terminal are on and the grant has not ended;
/// `exec.access` is the account with its label, the decisions and the end.
/// Shared with `se share room list`.
pub(super) struct AccessFields {
    /// The account the worker runs as: the label of the profile `own`.
    own: String,
    now: i64,
}

impl AccessFields {
    pub(super) fn now() -> Self {
        Self {
            own: crate::share::exec_provider_status().user_label,
            now: crate::share::core_now_secs(),
        }
    }

    #[cfg(test)]
    pub(super) fn fixed(own: &str, now: i64) -> Self {
        Self {
            own: own.into(),
            now,
        }
    }

    pub(super) fn enabled(&self, exec: &ExecGrant) -> bool {
        let access = exec.access_grant();
        Capability::EXEC
            .into_iter()
            .any(|capability| access.decision_at(capability, self.now).is_on())
    }

    pub(super) fn state(&self, exec: &ExecGrant) -> &'static str {
        if self.enabled(exec) {
            "enabled"
        } else {
            "disabled"
        }
    }

    pub(super) fn value(&self, exec: &ExecGrant) -> Value {
        let view = AccessGrantView::new(&exec.access_grant(), &self.own, self.now);
        serde_json::to_value(view).unwrap_or(Value::Null)
    }

    /// `exec_as=…\texec_commands=…\texec_terminal=…\texec_until=…`.
    pub(super) fn text(&self, exec: &ExecGrant) -> String {
        let access = exec.access_grant();
        let until = match access.valid_until {
            None => "none".to_string(),
            Some(end) => chrono::DateTime::<chrono::Utc>::from_timestamp(end, 0)
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
                })
                .unwrap_or_else(|| end.to_string()),
        };
        format!(
            "exec_as={}\texec_commands={}\texec_terminal={}\texec_until={until}",
            clean(&access.account.label(&self.own)),
            access.decision(Capability::ExecCommands).key(),
            access.decision(Capability::ExecTerminal).key(),
        )
    }
}

/// The authorization of a contact: the grant of its pinned remote device.
fn grant_of<'a>(profiles: &'a ShareProfiles, contact: &DirectContact) -> Option<&'a DirectGrant> {
    let device_id = contact.remote_device_id.as_deref()?;
    profiles.direct_grants.iter().find(|grant| {
        grant.device_id == device_id
            && contact
                .remote_public_key
                .as_deref()
                .is_none_or(|key| key == grant.public_key)
    })
}

fn value(
    profiles: &ShareProfiles,
    contact: &DirectContact,
    access: &AccessFields,
) -> serde_json::Value {
    let grant = grant_of(profiles, contact);
    serde_json::json!({
        "id": contact.id,
        "name": contact.display_name,
        "device_id": contact.remote_device_id,
        "fingerprint": contact.expected_fingerprint,
        "status": contact.status.label(),
        "access": contact.access_state.label(),
        "auto_connect": contact.auto_connect,
        "auto_open": contact.auto_open,
        "share_back": contact.relation.share_back,
        "authorization": grant.map(|grant| serde_json::json!({
            "active": grant.state == crate::share::DirectGrantState::Accepted,
            "write": grant.write,
            "exec": {
                "enabled": access.enabled(&grant.exec),
                "policy_revision": grant.exec.policy_revision,
                "run_as": super::grants::exec_run_as(&grant.exec),
                "programs": grant.exec.programs,
                "access": access.value(&grant.exec),
            },
        })),
    })
}

fn text(profiles: &ShareProfiles, contact: &DirectContact, access: &AccessFields) -> String {
    let grant = grant_of(profiles, contact);
    let authorization = match grant {
        None => "none".to_string(),
        Some(grant) => format!(
            "{}\twrite={}\texec={}\texec_run_as={}\texec_programs={}\t{}",
            if grant.state == crate::share::DirectGrantState::Accepted {
                "active"
            } else {
                "inactive"
            },
            grant.write,
            access.state(&grant.exec),
            super::grants::exec_run_as(&grant.exec),
            super::grants::exec_programs_text(&grant.exec),
            access.text(&grant.exec),
        ),
    };
    format!(
        "device\t{}\tid={}\tdevice_id={}\tfingerprint={}\tstatus={}\taccess={}\tauto_connect={}\tauto_open={}\tshare_back={}\tauthorization={authorization}",
        clean(&contact.display_name),
        contact.id,
        contact.remote_device_id.as_deref().unwrap_or("-"),
        contact.expected_fingerprint,
        clean(&contact.status.label()),
        clean(&contact.access_state.label()),
        contact.auto_connect,
        contact.auto_open,
        contact.relation.share_back,
    )
}

fn select(profiles: &ShareProfiles, selector: Option<&str>) -> Result<DirectContact, String> {
    let selector = selector.map(str::trim).filter(|value| !value.is_empty());
    let matches = profiles
        .direct_contacts
        .iter()
        .filter(|contact| selector.is_none_or(|value| matches(contact, value)))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [contact] => Ok((*contact).clone()),
        [] => Err("no matching device; run `se share device list`".into()),
        _ => Err(format!(
            "several devices match; choose one id: {}",
            matches
                .iter()
                .map(|contact| contact.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn matches(contact: &DirectContact, selector: &str) -> bool {
    let prefix = |candidate: &str| {
        candidate.eq_ignore_ascii_case(selector)
            || (selector.len() >= 4
                && candidate
                    .to_ascii_lowercase()
                    .starts_with(&selector.to_ascii_lowercase()))
    };
    contact.id == selector
        || contact.display_name.eq_ignore_ascii_case(selector)
        || contact.remote_device_id.as_deref().is_some_and(prefix)
        || prefix(&contact.expected_fingerprint)
}

fn report(contact: &DirectContact, what: &str, changed: bool) -> Result<(), String> {
    println!(
        "device\t{}\t{what}\tchanged={changed}{}",
        clean(&contact.display_name),
        super::refresh_note()
    );
    Ok(())
}

fn persist(
    mut edit: impl FnMut(&mut ShareProfiles) -> Result<bool, String>,
) -> Result<bool, String> {
    let mut changed = false;
    ShareProfiles::mutate_persisted(Some(super::default_home()), |profiles| {
        changed = edit(profiles)?;
        Ok(())
    })?;
    Ok(changed)
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn contact(id: &str, name: &str) -> DirectContact {
        DirectContact {
            id: id.into(),
            display_name: name.into(),
            lookup_id: format!("lookup-{id}"),
            expected_fingerprint: format!("fingerprint-{id}"),
            expected_node_id: format!("node-{id}"),
            remote_device_id: Some(format!("device-{id}")),
            remote_public_key: Some(format!("key-{id}")),
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: crate::share::ShareStatus::Waiting,
            last_error: None,
            presence: None,
            access_state: crate::share::DirectAccessState::Accepted,
            request_sent_at: None,
            accepted_at: None,
            accepted_public_key: None,
            lan_candidates: Vec::new(),
            lan_seen_at: None,
            lan_uplink: None,
            relation: Default::default(),
        }
    }

    #[test]
    fn luecken_task_device_commands_parse_and_need_a_setting() {
        for command in [
            vec!["se", "share", "device"],
            vec!["se", "share", "device", "list", "--json"],
            vec![
                "se",
                "share",
                "device",
                "set",
                "Laptop",
                "--auto-connect",
                "false",
            ],
            vec![
                "se",
                "share",
                "device",
                "set",
                "--share-back",
                "true",
                "--auto-open",
                "true",
            ],
            vec!["se", "share", "device", "reset-trust", "Laptop"],
        ] {
            assert!(
                crate::cli::Cli::try_parse_from(command.clone()).is_ok(),
                "{command:?}"
            );
        }
        assert!(
            crate::cli::Cli::try_parse_from(["se", "share", "device", "set", "Laptop"]).is_err()
        );
    }

    #[test]
    fn luecken_task_device_list_and_selection_name_every_setting() {
        let mut profiles = ShareProfiles::default();
        profiles.direct_contacts.push(contact("a", "Laptop"));
        profiles.direct_contacts.push(contact("b", "Desktop"));
        profiles.direct_grants.push(DirectGrant {
            device_id: "device-a".into(),
            device_name: "Laptop".into(),
            public_key: "key-a".into(),
            fingerprint: "fingerprint-a".into(),
            node_id: "node-a".into(),
            state: crate::share::DirectGrantState::Accepted,
            updated_at: 1,
            exec: Default::default(),
            write: true,
        });
        let access = AccessFields::fixed("alice", 1);
        let line = text(&profiles, &profiles.direct_contacts[0], &access);
        assert!(line.contains("\tauto_connect=true\tauto_open=false\tshare_back=false\t"));
        assert!(line.contains(
            "authorization=active\twrite=true\texec=disabled\texec_run_as=standard\texec_programs=full-shell\t"
        ));
        assert!(line
            .ends_with("\texec_as=alice\texec_commands=off\texec_terminal=off\texec_until=none"));
        let other = text(&profiles, &profiles.direct_contacts[1], &access);
        assert!(other.ends_with("authorization=none"));
        let json = value(&profiles, &profiles.direct_contacts[0], &access);
        assert_eq!(json["authorization"]["write"], true);
        assert_eq!(json["authorization"]["exec"]["enabled"], false);
        assert_eq!(
            json["authorization"]["exec"]["access"]["account_label"],
            "alice"
        );
        assert_eq!(json["share_back"], false);
        assert_eq!(select(&profiles, Some("laptop")).unwrap().id, "a");
        assert_eq!(select(&profiles, Some("device-b")).unwrap().id, "b");
        assert_eq!(select(&profiles, Some("fingerprint-a")).unwrap().id, "a");
        assert!(select(&profiles, None).is_err());
        assert!(select(&profiles, Some("nobody")).is_err());
    }
}
