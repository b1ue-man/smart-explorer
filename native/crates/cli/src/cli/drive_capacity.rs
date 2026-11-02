//! Uses the same cached, authenticated snapshot as the desktop details.
use crate::mount::MountSnapshot;

pub(super) fn run(args: super::SelectArgs) -> Result<i32, String> {
    let id = super::resolve_selector(&args.selector)?;
    let snapshot = crate::daemon::list_mounts()?
        .into_iter()
        .find(|mount| mount.config.id == id)
        .ok_or_else(|| "Laufwerk nicht gefunden".to_string())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot).map_err(|error| error.to_string())?
        );
    } else {
        print(&snapshot);
    }
    Ok(0)
}

pub(super) fn print(snapshot: &MountSnapshot) {
    if let Some(capacity) = &snapshot.capacity {
        let lines = capacity.display_lines();
        let uploads = capacity
            .uploads
            .as_ref()
            .map(|uploads| {
                uploads
                    .display_lines()
                    .iter()
                    .map(|line| super::drive_print::clean(line))
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        for line in lines.iter().chain(&uploads) {
            println!("{}\t{line}", snapshot.config.id);
        }
    } else {
        println!("{}\tRemote verfügbar: unbekannt; Lokaler Zwischenspeicher verfügbar: unbekannt (kein Hostbericht)", snapshot.config.id);
    }
}
