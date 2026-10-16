//! `se share export move`: the order in which peers see the exports (the
//! desktop buttons „Nach oben“ / „Nach unten“, Android „Nach oben/unten“).
use clap::Args;

use super::{exports_policy, ExportScope};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("direction").required(true).args(["up", "down"])))]
pub(super) struct MoveArgs {
    #[arg(help = "Exact export label or stored path")]
    selector: String,
    #[arg(long, help = "One place towards the top")]
    up: bool,
    #[arg(long, help = "One place towards the bottom")]
    down: bool,
    #[command(flatten)]
    scope: ExportScope,
}

pub(super) fn run(args: MoveArgs) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let scope = exports_policy::scope_id(&profiles, args.scope.room.as_deref())?;
    let roots = &super::export_config(&profiles, args.scope.room.as_deref())?.roots;
    let matches = roots
        .iter()
        .filter(|root| root.label == args.selector || root.path == args.selector)
        .collect::<Vec<_>>();
    let path = match matches.as_slice() {
        [root] => root.path.clone(),
        [] => return Err(format!("export not found: {}", args.selector)),
        _ => return Err(format!("export selector is ambiguous: {}", args.selector)),
    };
    let offset = if args.up { -1 } else { 1 };
    let mut moved = false;
    let committed = crate::share::ShareProfiles::mutate_persisted(
        Some(super::super::default_home()),
        |profiles| {
            moved = profiles.move_export_root(&scope, &path, offset)?;
            Ok(())
        },
    )?;
    let position = super::export_config(&committed, args.scope.room.as_deref())?
        .roots
        .iter()
        .position(|root| root.path == path)
        .map_or(0, |index| index + 1);
    println!(
        "export\t{path}\tposition={position}\tmoved={moved}{}",
        super::super::refresh_note()
    );
    Ok(())
}
