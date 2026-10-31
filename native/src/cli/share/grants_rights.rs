//! `se share grants presets|pending|confirm|cancel` (plan 2026-10-31 §F,
//! F1/F4): the rights presets, the connections whose rights are open, and
//! „Bestätigen“ / „Abbrechen“ of the compact rights menu. Confirming sets
//! exactly the chosen rights in one profile transaction; cancelling keeps
//! the connection without rights.
use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;
use serde_json::json;

use super::super::rights_args::{presets_value, rights_text, rights_value, RightsArgs};
use crate::share::connection_rights::{RightsPreset, RightsTarget};
use crate::share::ShareProfiles;

#[derive(Subcommand)]
pub(super) enum RightsCommand {
    #[command(
        about = "List the rights presets (none, read-only, read-write, files-exec, all) and what each allows"
    )]
    Presets,
    #[command(about = "List the connections whose rights wait for a confirmation (Rechte offen)")]
    Pending,
    #[command(
        about = "Confirm the rights of a connection: exactly the preset and options given; ends Rechte offen"
    )]
    Confirm(ConfirmArgs),
    #[command(about = "Cancel open rights: the connection stays, without rights")]
    Cancel(TargetArgs),
}

#[derive(Args)]
pub(super) struct TargetArgs {
    /// Device id, name or fingerprint of the Direct grant; omit with --room or when only one waits
    #[arg(conflicts_with = "room",
        add = ArgValueCandidates::new(crate::cli::completions_access::pending_rights_candidates))]
    selector: Option<String>,
    /// Room profile id, room id or name instead of a device. For a room, folders "all" copies the Direct folders of now into the room; folders added later are not added to it
    #[arg(long, value_name = "ROOM",
        add = ArgValueCandidates::new(crate::cli::completions_access::room_candidates))]
    room: Option<String>,
}

#[derive(Args)]
pub(super) struct ConfirmArgs {
    #[command(flatten)]
    target: TargetArgs,
    #[command(flatten)]
    rights: RightsArgs,
}

pub(super) fn run(command: RightsCommand, json: bool) -> Result<(), String> {
    match command {
        RightsCommand::Presets => presets(json),
        RightsCommand::Pending => pending(json),
        RightsCommand::Confirm(args) => confirm(args, json),
        RightsCommand::Cancel(args) => cancel(args, json),
    }
}

fn presets(json: bool) -> Result<(), String> {
    if json {
        return print_json(&json!({ "presets": presets_value() }));
    }
    for preset in RightsPreset::ALL {
        println!(
            "preset\t{}\t{}\t{}",
            preset.key(),
            preset.label(),
            preset.description()
        );
    }
    Ok(())
}

/// The open confirmations as JSON (also part of `se share status --json`).
pub(in crate::cli) fn pending_value(profiles: &ShareProfiles) -> serde_json::Value {
    serde_json::Value::Array(
        profiles
            .pending_rights_views()
            .iter()
            .map(|view| {
                json!({
                    "target": view.target,
                    "origin": view.origin.key(),
                    "since": view.since,
                    "name": view.name,
                    "rights": rights_value(&view.rights),
                })
            })
            .collect(),
    )
}

/// The rights of one Direct grant for `se share grants list --json`: the
/// connection rights and whether they wait for confirmation.
pub(super) fn grant_rights_value(
    profiles: &ShareProfiles,
    grant: &crate::share::DirectGrant,
) -> serde_json::Value {
    let target = RightsTarget::Direct {
        device_id: grant.device_id.clone(),
        public_key: grant.public_key.clone(),
    };
    let mut value = profiles
        .current_direct_rights(&grant.device_id, &grant.public_key)
        .map(|rights| rights_value(&rights))
        .unwrap_or(serde_json::Value::Null);
    if let serde_json::Value::Object(map) = &mut value {
        map.insert(
            "pending".into(),
            json!(profiles.pending_rights_of(&target).is_some()),
        );
    }
    value
}

/// The open confirmations as text lines (`rights_pending<TAB>…`).
pub(in crate::cli) fn pending_lines(profiles: &ShareProfiles) -> Vec<String> {
    profiles
        .pending_rights_views()
        .iter()
        .map(|view| {
            let (kind, id) = match &view.target {
                RightsTarget::Direct { device_id, .. } => ("device", device_id.as_str()),
                RightsTarget::Room { profile_id } => ("room", profile_id.as_str()),
            };
            format!(
                "rights_pending\t{kind}\t{id}\t{}\t{}\tconfirm: se share grants confirm {}{id} --preset NAME",
                super::clean(&view.name),
                view.origin.key(),
                if kind == "room" { "--room " } else { "" },
            )
        })
        .collect()
}

fn pending(json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    if json {
        return print_json(&json!({ "pending": pending_value(&profiles) }));
    }
    let lines = pending_lines(&profiles);
    if lines.is_empty() {
        println!("rights_pending\tnone");
    }
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// The target of `args`: a room, a device selector, or the only waiting
/// Direct connection.
fn target(profiles: &ShareProfiles, args: &TargetArgs) -> Result<RightsTarget, String> {
    if let Some(room) = &args.room {
        let room = room.trim();
        let found = profiles
            .rooms
            .iter()
            .filter(|candidate| {
                candidate.id == room
                    || candidate.room_id == room
                    || candidate.name.eq_ignore_ascii_case(room)
            })
            .collect::<Vec<_>>();
        let [room] = found.as_slice() else {
            return Err(format!("room not found or ambiguous: {room}"));
        };
        return Ok(RightsTarget::Room {
            profile_id: room.id.clone(),
        });
    }
    let grants = profiles
        .direct_grants
        .iter()
        .filter(|grant| match args.selector.as_deref().map(str::trim) {
            Some(selector) => super::grant_selector_matches(profiles, grant, selector),
            None => profiles
                .pending_rights_of(&RightsTarget::Direct {
                    device_id: grant.device_id.clone(),
                    public_key: grant.public_key.clone(),
                })
                .is_some(),
        })
        .collect::<Vec<_>>();
    let [grant] = grants.as_slice() else {
        return Err(if args.selector.is_some() {
            "device authorization is missing or ambiguous; use the device id from `se share grants list`".into()
        } else {
            "no single connection waits for its rights; name the device or use --room".into()
        });
    };
    Ok(RightsTarget::Direct {
        device_id: grant.device_id.clone(),
        public_key: grant.public_key.clone(),
    })
}

fn current(
    profiles: &ShareProfiles,
    target: &RightsTarget,
) -> Option<crate::share::connection_rights::ConnectionRights> {
    match target {
        RightsTarget::Direct {
            device_id,
            public_key,
        } => profiles.current_direct_rights(device_id, public_key),
        RightsTarget::Room { profile_id } => profiles.current_room_rights(profile_id),
    }
}

fn confirm(args: ConfirmArgs, json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let target = target(&profiles, &args.target)?;
    if !args.rights.given() {
        return Err("choose the rights: --preset NAME (see `se share grants presets`) or the single options".into());
    }
    let mut roots = profiles.default_direct_exports.roots.clone();
    if let RightsTarget::Room { profile_id } = &target {
        if let Some(room) = profiles.rooms.iter().find(|room| &room.id == profile_id) {
            roots.extend(room.exports.roots.iter().cloned());
        }
    }
    let now = crate::share::core_now_secs();
    let rights = args
        .rights
        .rights(current(&profiles, &target), &roots, now)?
        .ok_or("choose the rights")?;
    let change = crate::share::connection_rights_actions::confirm_connection_rights(
        Some(super::super::default_home()),
        &target,
        &rights,
    )?;
    report(
        "rights_confirmed",
        &target,
        Some(&rights),
        change.changed,
        json,
    )
}

fn cancel(args: TargetArgs, json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let target = target(&profiles, &args)?;
    let change = crate::share::connection_rights_actions::cancel_connection_rights(
        Some(super::super::default_home()),
        &target,
    )?;
    let rights = current(&change.profiles, &target);
    report(
        "rights_cancelled",
        &target,
        rights.as_ref(),
        change.changed,
        json,
    )
}

fn report(
    action: &str,
    target: &RightsTarget,
    rights: Option<&crate::share::connection_rights::ConnectionRights>,
    changed: bool,
    json: bool,
) -> Result<(), String> {
    let (state, error) = super::worker_refresh();
    if json {
        return print_json(&json!({
            "action": action,
            "target": target,
            "rights": rights.map(rights_value),
            "changed": changed,
            "persisted": true,
            "worker_refresh": {"state": state, "error": error},
        }));
    }
    let id = match target {
        RightsTarget::Direct { device_id, .. } => device_id,
        RightsTarget::Room { profile_id } => profile_id,
    };
    let summary = rights
        .map(|rights| rights_text("", rights))
        .unwrap_or_default();
    println!("{action}\t{id}{summary}\tchanged={changed}\tworker_refresh={state}");
    if let Some(error) = error {
        println!("worker_error\t{}", super::clean(&error));
    }
    Ok(())
}

fn print_json(value: &serde_json::Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}
