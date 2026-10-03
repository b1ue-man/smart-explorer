//! Remote duplicate copies move on their storing device, bound to the
//! search's SHA-256. The core validates the selection independently of UI
//! state, serialises actions per report and remembers exact successes.
use std::collections::HashSet;
use serde_json::{json, Value};

use super::results::{bind_task, with_stored, with_stored_mut, Pending, Stored};
use super::super::args::{invalid, str_arg, string_list};
use super::super::locations::{is_local, location_for};
use crate::mobile::{ApiError, Runtime, TaskCtx};
use crate::vfs::RecycleOutcome;

struct Reservation(String);
impl Drop for Reservation {
    fn drop(&mut self) {
        let _ = with_stored_mut(&self.0, |stored| {
            if let Stored::Duplicates { recycling, .. } = stored { *recycling = false; }
            Ok(())
        });
    }
}

pub(super) fn start(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let source = str_arg(args, "taskId")?.to_string();
    let locations: HashSet<String> = string_list(args, "locations")?.into_iter().collect();
    if locations.is_empty() { return Err(invalid("Bitte Duplikatkopien auswählen.")); }
    let (base, plan) = with_stored_mut(&source, |stored| {
        let Stored::Duplicates { groups, base, recycling, .. } = stored else {
            return Err(invalid("Dieser Task ist keine Duplikatsuche."));
        };
        if is_local(base) {
            return Err(ApiError::new("unsupported", "Lokale Kopien verwenden den lokalen Papierkorb."));
        }
        if *recycling { return Err(ApiError::new("busy", "Papierkorbaktion läuft bereits.")); }
        let selected: HashSet<String> = groups.iter().flat_map(|group| &group.items)
            .filter(|item| locations.contains(&location_for(base, &item.path)))
            .map(|item| item.path.clone()).collect();
        if selected.len() != locations.len() {
            return Err(invalid("Auswahl gehört nicht eindeutig zu diesem Suchergebnis."));
        }
        let plan = crate::analytics::recycle_plan(groups, &selected);
        if plan.targets.is_empty() || plan.skipped > 0 || plan.kept > 0 {
            return Err(invalid("Nur geprüfte Duplikatkopien auswählen; mindestens eine Kopie je Gruppe muss bleiben."));
        }
        *recycling = true;
        Ok((base.clone(), plan))
    })?;
    let reservation = Reservation(source);
    let pending = Pending::open();
    let token = pending.token();
    let task = rt.spawn_task("reclaimRecycle", "Duplikatkopien in den Papierkorb".into(), move |ctx|
        run(ctx, reservation, pending, base, plan));
    bind_task(token, &task);
    Ok(super::started(task, true))
}

fn run(ctx: &TaskCtx, reservation: Reservation, pending: Pending, base: String,
    plan: crate::analytics::RecyclePlan) -> Result<Value, ApiError> {
    let (backend, _) = super::resolve_remote(&base)?;
    let total = plan.targets.len() as u64;
    let mut moved = Vec::new();
    for (index, (path, expected)) in plan.targets.into_iter().enumerate() {
        if ctx.cancelled() { break; }
        let location = location_for(&base, &path);
        ctx.message(&format!("Gegenstelle prüft und verschiebt: {location}"));
        match crate::vfs::recycle(&*backend, &path, &expected) {
            Ok(RecycleOutcome::Recycled) => {
                moved.push(location.clone());
                // The report now describes the remaining copies. Another
                // action cannot select the former keeper as the last copy.
                let _ = with_stored_mut(&reservation.0, |stored| {
                    if let Stored::Duplicates { groups, summary, .. } = stored {
                        for group in groups.iter_mut() {
                            group.items.retain(|item| item.path != path);
                            group.reclaimable = group.size.saturating_mul(group.items.len().saturating_sub(1) as u64);
                        }
                        groups.retain(|group| group.items.len() > 1);
                        summary.groups = groups.len() as u64;
                    }
                    Ok(())
                });
            }
            Ok(RecycleOutcome::Changed) => ctx.error(&location, "Inhalt hat sich geändert – nicht verschoben"),
            Err(error) => ctx.error(&location, &error.to_string()),
        }
        ctx.progress(0, 0, index as u64 + 1, total);
    }
    let value = json!({ "moved": moved.len() });
    pending.store(Stored::Recycled { moved });
    Ok(value)
}

pub(super) fn result(args: &Value) -> Result<Value, ApiError> {
    with_stored(str_arg(args, "taskId")?, |stored| match stored {
        Stored::Recycled { moved } => Ok(json!({ "moved": moved })),
        _ => Err(invalid("Dieser Task ist keine Papierkorbaktion.")),
    })
}
