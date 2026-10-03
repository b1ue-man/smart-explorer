use clap::Args;

use super::{export_config, room_index, ExportScope};
use crate::share::{ExportAccess, ShareProfiles};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("access").required(true).args(["write", "read_only"])))]
pub(super) struct SetArgs {
    #[arg(help = "Exact export label or stored path")]
    selector: String,
    #[arg(
        long,
        help = "Allow writing; the contact/room must also have write rights"
    )]
    write: bool,
    #[arg(long, help = "Allow reading only")]
    read_only: bool,
    #[arg(
        long,
        conflicts_with = "protect_system_files",
        help = "Unsafe: also allow writing autostart, login and key files"
    )]
    allow_system_writes: bool,
    #[arg(long, help = "Protect autostart, login and key files")]
    protect_system_files: bool,
    #[command(flatten)]
    scope: ExportScope,
}

#[derive(Args)]
pub(super) struct RoomPolicyArgs {
    #[arg(long, help = "Exact room profile id, room id, or room name")]
    room: String,
    #[arg(
        long,
        conflicts_with = "read_only",
        help = "Allow admitted members to write writable exports"
    )]
    write: bool,
    #[arg(long, help = "Allow members reading only")]
    read_only: bool,
    #[arg(
        long,
        help = "Require an explicit admission for new members (true/false)"
    )]
    confirm_new_members: Option<bool>,
}

pub(super) fn scope_id(profiles: &ShareProfiles, room: Option<&str>) -> Result<String, String> {
    match room {
        None => Ok("direct".into()),
        Some(selector) => Ok(profiles.rooms[room_index(profiles, selector)?].id.clone()),
    }
}

pub(super) fn set(args: SetArgs) -> Result<(), String> {
    let before = super::super::checked_profiles()?;
    let scope = scope_id(&before, args.scope.room.as_deref())?;
    let matches = export_config(&before, args.scope.room.as_deref())?
        .roots
        .iter()
        .filter(|root| root.path == args.selector || root.label == args.selector)
        .collect::<Vec<_>>();
    let [root] = matches.as_slice() else {
        return Err("export selector is missing or ambiguous; use the exact stored path".into());
    };
    let path = root.path.clone();
    let access = if args.write {
        ExportAccess::ReadWrite
    } else {
        ExportAccess::ReadOnly
    };
    let system = if args.allow_system_writes {
        Some(true)
    } else if args.protect_system_files {
        Some(false)
    } else {
        None
    };
    ShareProfiles::mutate_persisted(Some(super::super::default_home()), |profiles| {
        profiles
            .export_config_mut(&scope)?
            .set_root_access(&path, access, system)?;
        Ok(())
    })?;
    println!(
        "Updated export {path}: {}{}",
        access_name(access),
        super::super::refresh_note()
    );
    Ok(())
}

pub(super) fn room_policy(args: RoomPolicyArgs) -> Result<(), String> {
    if !args.write && !args.read_only && args.confirm_new_members.is_none() {
        return Err("choose --write, --read-only or --confirm-new-members true/false".into());
    }
    let before = super::super::checked_profiles()?;
    let room = &before.rooms[room_index(&before, &args.room)?];
    let profile_id = room.id.clone();
    let room_id = room.room_id.clone();
    let write = if args.write {
        Some(true)
    } else if args.read_only {
        Some(false)
    } else {
        None
    };
    let committed =
        ShareProfiles::mutate_persisted(Some(super::super::default_home()), |profiles| {
            if !profiles
                .rooms
                .iter()
                .any(|room| room.id == profile_id && room.room_id == room_id)
            {
                return Err("room identity changed; reload and retry".into());
            }
            profiles.set_room_policy(&profile_id, write, args.confirm_new_members)?;
            Ok(())
        })?;
    let room = committed
        .rooms
        .iter()
        .find(|room| room.id == profile_id)
        .ok_or_else(|| "committed room missing".to_string())?;
    println!(
        "room_policy\t{}\tmembers_may_write={}\tconfirm_new_members={}{}",
        profile_id,
        room.policy.members_may_write,
        room.policy.confirm_new_members,
        super::super::refresh_note()
    );
    Ok(())
}

pub(super) fn access_name(access: ExportAccess) -> &'static str {
    match access {
        ExportAccess::ReadOnly => "read_only",
        ExportAccess::ReadWrite => "read_write",
    }
}
