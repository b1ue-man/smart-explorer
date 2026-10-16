//! `se share room …`: the settings of saved rooms that the desktop page
//! „Räume“ and the Android section „Räume“ change – create, list with
//! members, join, leave, rejoin, rename, invite code, remove, and the
//! admission of members. Every edit is one profile transaction
//! (`crate::share::ShareProfiles::mutate_persisted`) followed by a worker
//! refresh.
use clap::{Args, Subcommand, ValueEnum};

use crate::share::{RoomMember, RoomMemberAdmission, RoomProfile, ShareProfiles};

#[derive(Args)]
pub(super) struct RoomArgs {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Option<RoomCommand>,
}

#[derive(Subcommand)]
enum RoomCommand {
    #[command(about = "Create a Share room and print its invite code")]
    Create {
        #[arg(long, default_value = "", help = "Local room display name")]
        name: String,
    },
    #[command(about = "List saved rooms with state, rights and members (default)")]
    List,
    #[command(about = "Join a room with its invite code")]
    Join {
        #[arg(long, help = "Room invite code copied from Smart Explorer")]
        code: String,
        #[arg(long, default_value = "", help = "Local room display name")]
        name: String,
    },
    #[command(about = "Leave a room: it stays saved but is no longer joined automatically")]
    Leave(RoomSelector),
    #[command(about = "Join a left room again automatically")]
    Rejoin(RoomSelector),
    #[command(about = "Change the local display name of a room (empty = Raum)")]
    Rename {
        #[command(flatten)]
        room: RoomSelector,
        #[arg(help = "New display name")]
        name: String,
    },
    #[command(about = "Print the invite code of a saved room")]
    Code(RoomSelector),
    #[command(about = "Remove a saved room, its secret and its local state")]
    Remove(RoomSelector),
    #[command(
        about = "Admit a waiting member, block a member, allow a blocked member again, or reset its cached presence"
    )]
    Member(MemberArgs),
}

#[derive(Args)]
struct RoomSelector {
    #[arg(help = "Exact room profile id, room id, or room name")]
    room: String,
}

#[derive(Args)]
struct MemberArgs {
    #[arg(help = "Exact room profile id, room id, or room name")]
    room: String,
    #[arg(help = "Member device id, exact device name, or fingerprint")]
    member: String,
    #[arg(value_enum)]
    action: MemberAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum MemberAction {
    /// Admit a member waiting for confirmation
    Admit,
    /// Block a member (its Exec turns off; the room then confirms new members)
    Block,
    /// Allow a blocked member again
    Allow,
    /// Forget its cached presence and look it up again
    ResetTrust,
}

pub(super) fn run(args: RoomArgs) -> Result<(), String> {
    let json = args.json;
    match args.command.unwrap_or(RoomCommand::List) {
        RoomCommand::Create { name } => create(&name),
        RoomCommand::List => list(json),
        RoomCommand::Join { code, name } => join(&code, &name),
        RoomCommand::Leave(selector) => leave(&selector.room),
        RoomCommand::Rejoin(selector) => {
            let room = find(&super::checked_profiles()?, &selector.room)?.clone();
            let changed = persist(|profiles| profiles.set_room_auto_join(&room.id, true))?;
            report(&room, "rejoined", changed)
        }
        RoomCommand::Rename { room, name } => {
            let room = find(&super::checked_profiles()?, &room.room)?.clone();
            let changed = persist(|profiles| profiles.rename_room(&room.id, &name))?;
            report(&room, "renamed", changed)
        }
        RoomCommand::Code(selector) => code(&selector.room, json),
        RoomCommand::Remove(selector) => {
            let room = find(&super::checked_profiles()?, &selector.room)?.clone();
            println!("{}", crate::cli::setup::remove_room(&room.id)?);
            Ok(())
        }
        RoomCommand::Member(args) => member(args),
    }
}

fn create(name: &str) -> Result<(), String> {
    let code = ShareProfiles::new_room_code()?;
    let (_profiles, id) =
        ShareProfiles::add_room_from_code_persisted(Some(super::default_home()), &code, name)?;
    println!("room_id\t{id}");
    println!("room_code\t{code}");
    println!("worker\t{}", super::refresh_note().trim_start_matches("; "));
    Ok(())
}

fn join(code: &str, name: &str) -> Result<(), String> {
    let (_profiles, id) =
        ShareProfiles::add_room_from_code_persisted(Some(super::default_home()), code, name)?;
    println!("room\t{id}\tjoined{}", super::refresh_note());
    Ok(())
}

/// Desktop/Android „Verlassen“: `auto_join` off, then the leave signal to
/// the room (sent only while the worker runs; the setting holds anyway).
fn leave(selector: &str) -> Result<(), String> {
    let room = find(&super::checked_profiles()?, selector)?.clone();
    let changed = persist(|profiles| profiles.set_room_auto_join(&room.id, false))?;
    if let Err(error) = crate::daemon::send_share_command(crate::share::ShareCmd::LeaveRoom {
        room_id: room.room_id.clone(),
    }) {
        println!("leave_signal\tnot sent: {}", clean(&error));
    }
    report(&room, "left", changed)
}

fn code(selector: &str, json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let room = find(&profiles, selector)?;
    let code = ShareProfiles::room_code_checked(room)?
        .ok_or_else(|| format!("the secret of room {} is missing", room.name))?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "room_id": room.id, "code": code })
        );
    } else {
        println!("room_code\t{code}");
    }
    Ok(())
}

fn member(args: MemberArgs) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let room = find(&profiles, &args.room)?.clone();
    let member = find_member(&room, &args.member)?.clone();
    let now = crate::share::core_now_secs();
    let changed = persist(|profiles| {
        if args.action == MemberAction::ResetTrust {
            return profiles.reset_member_trust(&room.id, &member.device_id);
        }
        let latest = profiles
            .rooms
            .iter_mut()
            .find(|candidate| candidate.id == room.id && candidate.room_id == room.room_id)
            .ok_or_else(|| "room identity changed; list the rooms again".to_string())?;
        let pinned = latest
            .members
            .iter()
            .filter(|candidate| candidate.device_id == member.device_id)
            .collect::<Vec<_>>();
        let [current] = pinned.as_slice() else {
            return Err("member identity changed; list the rooms again".into());
        };
        if current.public_key != member.public_key
            || current.node_id != member.node_id
            || current.fingerprint != member.fingerprint
        {
            return Err("member identity changed; list the rooms again".into());
        }
        Ok(match args.action {
            MemberAction::Admit => latest.admit_member(&member.device_id),
            MemberAction::Block => latest.set_member_blocked(&member.device_id, true, now),
            MemberAction::Allow => latest.set_member_blocked(&member.device_id, false, now),
            MemberAction::ResetTrust => false,
        })
    })?;
    println!(
        "member\t{}\t{}\taction={}\tchanged={changed}{}",
        clean(&room.name),
        clean(&member.device_name),
        action_name(args.action),
        super::refresh_note()
    );
    Ok(())
}

fn action_name(action: MemberAction) -> &'static str {
    match action {
        MemberAction::Admit => "admit",
        MemberAction::Block => "block",
        MemberAction::Allow => "allow",
        MemberAction::ResetTrust => "reset-trust",
    }
}

fn list(json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    if json {
        let rooms = profiles.rooms.iter().map(room_value).collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "count": rooms.len(),
                "rooms": rooms,
            }))
            .map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    if profiles.rooms.is_empty() {
        println!("rooms\t0");
    }
    for line in text(&profiles) {
        println!("{line}");
    }
    Ok(())
}

pub(super) fn room_value(room: &RoomProfile) -> serde_json::Value {
    serde_json::json!({
        "id": room.id,
        "room_id": room.room_id,
        "name": room.name,
        "auto_join": room.auto_join,
        "status": room.status.label(),
        "policy": {
            "members_may_write": room.policy.members_may_write,
            "confirm_new_members": room.policy.confirm_new_members,
        },
        "members": room.members.iter().map(member_value).collect::<Vec<_>>(),
    })
}

fn member_value(member: &RoomMember) -> serde_json::Value {
    serde_json::json!({
        "device_id": member.device_id,
        "device_name": member.device_name,
        "fingerprint": member.fingerprint,
        "admission": admission(member),
        "blocked": member.blocked,
        "status": member.status.label(),
        "exec": {
            "enabled": member.exec.enabled,
            "policy_revision": member.exec.policy_revision,
            "run_as": super::grants::exec_run_as(&member.exec),
            "programs": member.exec.programs,
        },
    })
}

pub(super) fn text(profiles: &ShareProfiles) -> Vec<String> {
    let mut lines = Vec::new();
    for room in &profiles.rooms {
        lines.push(format!(
            "room\t{}\tid={}\troom_id={}\tjoined={}\tstatus={}\tmembers={}\tmembers_may_write={}\tconfirm_new_members={}",
            clean(&room.name),
            room.id,
            room.room_id,
            room.auto_join,
            clean(&room.status.label()),
            room.members.len(),
            room.policy.members_may_write,
            room.policy.confirm_new_members,
        ));
        for member in &room.members {
            lines.push(format!(
                "member\t{}\t{}\tdevice_id={}\tfingerprint={}\tadmission={}\tblocked={}\tstatus={}\texec={}\texec_run_as={}\texec_programs={}",
                clean(&room.name),
                clean(&member.device_name),
                member.device_id,
                member.fingerprint,
                admission(member),
                member.blocked,
                clean(&member.status.label()),
                if member.exec.enabled { "enabled" } else { "disabled" },
                super::grants::exec_run_as(&member.exec),
                super::grants::exec_programs_text(&member.exec),
            ));
        }
    }
    lines
}

fn admission(member: &RoomMember) -> &'static str {
    match member.relation.admission {
        RoomMemberAdmission::Admitted => "admitted",
        RoomMemberAdmission::Pending => "pending",
    }
}

fn report(room: &RoomProfile, what: &str, changed: bool) -> Result<(), String> {
    println!(
        "room\t{}\t{what}\tchanged={changed}{}",
        clean(&room.name),
        super::refresh_note()
    );
    Ok(())
}

/// One profile transaction; returns whether `edit` changed anything.
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

pub(super) fn find<'a>(
    profiles: &'a ShareProfiles,
    selector: &str,
) -> Result<&'a RoomProfile, String> {
    let matches = profiles
        .rooms
        .iter()
        .filter(|room| room.id == selector || room.room_id == selector || room.name == selector)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [room] => Ok(room),
        [] => Err(format!(
            "room not found: {selector}; run `se share room list`"
        )),
        _ => Err(format!(
            "room selector is ambiguous: {selector}; use the room id"
        )),
    }
}

fn find_member<'a>(room: &'a RoomProfile, selector: &str) -> Result<&'a RoomMember, String> {
    let matches = room
        .members
        .iter()
        .filter(|member| {
            member.device_id == selector
                || member.device_name == selector
                || member.fingerprint.eq_ignore_ascii_case(selector)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [member] => Ok(member),
        [] => Err(format!("member not found in {}: {selector}", room.name)),
        _ => Err(format!(
            "member selector is ambiguous: {selector}; use the device id"
        )),
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn luecken_task_room_commands_parse_like_the_gui_actions() {
        for command in [
            vec!["se", "share", "room"],
            vec!["se", "share", "room", "list", "--json"],
            vec!["se", "share", "room", "create", "--name", "Team"],
            vec![
                "se", "share", "room", "join", "--code", "SE-R3-x", "--name", "Team",
            ],
            vec!["se", "share", "room", "leave", "Team"],
            vec!["se", "share", "room", "rejoin", "Team"],
            vec!["se", "share", "room", "rename", "Team", "Werkstatt"],
            vec!["se", "share", "room", "code", "Team"],
            vec!["se", "share", "room", "remove", "Team"],
            vec!["se", "share", "room", "member", "Team", "Laptop", "admit"],
            vec![
                "se",
                "share",
                "room",
                "member",
                "Team",
                "Laptop",
                "reset-trust",
            ],
        ] {
            assert!(
                crate::cli::Cli::try_parse_from(command.clone()).is_ok(),
                "{command:?}"
            );
        }
        assert!(crate::cli::Cli::try_parse_from([
            "se", "share", "room", "member", "Team", "x", "kick"
        ])
        .is_err());
    }

    #[test]
    fn luecken_task_room_list_names_state_rights_members_and_exec() {
        let mut profiles = ShareProfiles::default();
        let mut room = RoomProfile {
            id: "profile-1".into(),
            name: "Team".into(),
            room_id: "wire-1".into(),
            auto_join: false,
            last_seen: None,
            status: crate::share::ShareStatus::Offline,
            members: Vec::new(),
            exports: Default::default(),
            policy: crate::share::RoomPolicy::new_room(),
        };
        let mut member = RoomMember {
            device_id: "device-b".into(),
            device_name: "Laptop".into(),
            fingerprint: "fp-b".into(),
            public_key: "key-b".into(),
            node_id: "node-b".into(),
            relay_url: String::new(),
            candidates: Vec::new(),
            last_seen: None,
            status: crate::share::ShareStatus::Waiting,
            blocked: true,
            exec: Default::default(),
            presence: None,
            relation: Default::default(),
        };
        member.relation.admission = RoomMemberAdmission::Pending;
        member.exec.programs = vec!["uptime".into()];
        room.members.push(member);
        profiles.rooms.push(room);
        let lines = text(&profiles);
        assert!(lines[0].starts_with("room\tTeam\tid=profile-1\troom_id=wire-1\tjoined=false\t"));
        assert!(lines[1].contains("\tadmission=pending\tblocked=true\t"));
        assert!(lines[1].ends_with("\texec=disabled\texec_run_as=standard\texec_programs=uptime"));
        let value = room_value(&profiles.rooms[0]);
        assert_eq!(value["auto_join"], false);
        assert_eq!(value["members"][0]["admission"], "pending");
        assert_eq!(value["members"][0]["exec"]["programs"][0], "uptime");
        assert!(find(&profiles, "wire-1").is_ok());
        assert!(find(&profiles, "nope").is_err());
        assert!(find_member(&profiles.rooms[0], "FP-B").is_ok());
    }
}
