//! One ordered preflight, job binding and selected-root planning boundary.
use super::{folders, job_store, roots, side_flow, AtomicBool, FolderRegister, JobView, Side};

type Prepared<'a> = (
    FolderRegister<'a>,
    roots::RootPlan,
    Option<job_store::JobRun>,
);

pub(super) fn prepare<'a>(view: &JobView<'a>, cancel: &AtomicBool) -> Result<Prepared<'a>, String> {
    let saved_root = job_store::preflight(view)
        .map_err(|error| format!("Gespeicherten Zielroot bestätigen: {error}"))?;
    let root = if let Some(root) = saved_root {
        root
    } else {
        match view.target {
            Side::Local => folders::local_target_root(view.source.is_local(), view.target_dir)?,
            Side::Remote(_) => view.target_dir.to_string(),
        }
    };
    let target_flow = side_flow(view.target, &root);
    let record = job_store::begin(view, &root, cancel).map_err(|error| {
        format!(
            "Auftrag {} speichern/wiederaufnehmen: {error}",
            view.id.unwrap_or("unbestätigt")
        )
    })?;
    let mut folders = FolderRegister::new(view.target, &root, target_flow.clone());
    if let Some(planner) = roots::remote_planner(view, &target_flow, cancel)? {
        folders = folders.with_planner(planner);
    }
    let alias_required = record
        .as_ref()
        .map(|record| record.prepare_apply())
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or(false);
    let plan = roots::plan(
        view,
        &folders,
        cancel,
        record
            .as_ref()
            .map_or(&[][..], |record| record.completed_roots()),
        record
            .as_ref()
            .map_or(&[][..], |record| record.saved_file_roots()),
        alias_required,
    )?;
    roots::check_local_targets(view, &plan)?;
    if let Some(record) = &record {
        record
            .confirm_sources(view)
            .map_err(|error| format!("Quellbelege speichern: {error}"))?;
    }
    Ok((folders, plan, record))
}
