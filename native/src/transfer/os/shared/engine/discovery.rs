//! Discovery: walks the selection (W1 walker) or reads the explicit pairs in
//! its own thread and feeds the work queue while workers already transfer.
//! Totals grow as entries are found; `discovering` ends with the walk.
use super::super::engine_names::{join_rel, parent_rel};
use super::super::entries::compile_remote_filter;
use super::super::walk::{walk, WalkEvent, WalkOptions};
use super::super::walk_listers::{BackendLister, Listed, Lister, LocalLister};
use super::queue::{FileWork, Work};
use super::roots::RootPlan;
use super::view::Side;
use super::Engine;
use std::collections::HashSet;
use std::io;

/// Runs discovery to its end, then lets the workers drain the queue.
pub(super) fn discover(engine: &Engine<'_>, plan: &RootPlan) {
    if plan.walk_roots.is_empty() {
        feed_pairs(engine, plan);
    } else {
        walk_roots(engine, plan);
    }
    engine.stats.discovery_finished();
    engine.queue.finish_producing();
}

fn walk_roots(engine: &Engine<'_>, plan: &RootPlan) {
    let view = &engine.view;
    let options = WalkOptions {
        filter: compile_remote_filter(view.filter.cloned()),
        folders: engine.keeps_folders(),
        flatten: view.layout == super::super::job::Layout::Flatten,
        access: engine.gate.clone(),
        allow_backslash: view.allow_backslash(),
    };
    let local = LocalLister;
    let remote;
    let inner: &dyn Lister = match view.source {
        Side::Local => &local,
        Side::Remote(backend) => {
            remote = BackendLister(backend);
            &remote
        }
    };
    let excluded = exclusions(engine, plan);
    let lister = Excluding {
        inner,
        excluded: &excluded,
    };
    walk(
        &lister,
        &plan.walk_roots,
        &options,
        &engine.source_flow,
        engine.stop_flag(),
        &|event| on_event(engine, event),
    );
}

/// The new top-level destinations in the source's own namespace: a walk of
/// an overlapping namespace never copies the copy again (K1 safety net; the
/// planned paths never exist in a different namespace, so nothing else is
/// excluded there).
fn exclusions(engine: &Engine<'_>, plan: &RootPlan) -> HashSet<String> {
    if !engine.view.source.same_namespace(engine.view.target) {
        return HashSet::new();
    }
    plan.roots
        .iter()
        .map(|root| {
            let first = super::super::engine_names::first_component(&root.planned);
            comparable(&join_rel(engine.view.target_dir, first))
        })
        .collect()
}

fn comparable(path: &str) -> String {
    path.trim_end_matches('/').to_string()
}

struct Excluding<'a> {
    inner: &'a dyn Lister,
    excluded: &'a HashSet<String>,
}

impl Lister for Excluding<'_> {
    fn list(&self, dir: &str) -> io::Result<Vec<Listed>> {
        let mut listed = self.inner.list(dir)?;
        if !self.excluded.is_empty() {
            listed.retain(|entry| {
                !self
                    .excluded
                    .contains(&comparable(&join_rel(dir, &entry.name)))
            });
        }
        Ok(listed)
    }

    fn stat(&self, path: &str) -> io::Result<Listed> {
        self.inner.stat(path)
    }
}

/// One walker event; false stops the walk.
pub(super) fn on_event(engine: &Engine<'_>, event: WalkEvent) -> bool {
    if engine.stopped() {
        return false;
    }
    match event {
        WalkEvent::Dir { path, rel } => engine
            .queue
            .push(Work::Dir { rel, source: path }, engine.stop_flag()),
        WalkEvent::File {
            path,
            rel,
            size,
            mtime_ms,
            id,
            md5,
        } => {
            let rel = export_name(engine, &path, rel);
            engine.stats.found(size);
            let mut file = FileWork::new(path, rel, size, mtime_ms);
            file.id = id;
            file.md5 = md5;
            engine.queue.push(Work::File(file), engine.stop_flag())
        }
        WalkEvent::Omitted { .. } => {
            engine.stats.omitted();
            true
        }
        WalkEvent::Problem { path, message } => {
            engine.issue(&path, &message);
            true
        }
        WalkEvent::AccessRefused { path } => {
            engine.fatal(format!(
                "Lesezugriff auf „{path}“ wurde abgelehnt – Übertragung beendet; bereits Übertragenes bleibt erhalten"
            ));
            false
        }
    }
}

/// Provider exports (Drive documents) are saved with their format's
/// extension, at every depth.
fn export_name(engine: &Engine<'_>, path: &str, rel: String) -> String {
    let Some(backend) = engine.view.source.backend() else {
        return rel;
    };
    if !engine.export_names() {
        return rel;
    }
    let (parent, name) = match parent_rel(&rel) {
        Some(parent) => (Some(parent), &rel[parent.len() + 1..]),
        None => (None, rel.as_str()),
    };
    let exported = backend.download_name(path, name);
    if exported == name {
        return rel;
    }
    match parent {
        Some(parent) => format!("{parent}/{exported}"),
        None => exported,
    }
}

fn feed_pairs(engine: &Engine<'_>, plan: &RootPlan) {
    for (work, size_known) in &plan.pairs {
        if engine.stopped() {
            return;
        }
        let mut work = work.clone();
        if !size_known {
            match inspect(engine, &work.source) {
                Ok(listed) => {
                    work.size = listed.size;
                    work.mtime_ms = listed.mtime_ms;
                    if work.id.is_none() {
                        work.id = listed.id;
                    }
                }
                Err(message) => {
                    engine.issue(&work.source, &message);
                    continue;
                }
            }
        }
        let rel = std::mem::take(&mut work.rel);
        work.rel = export_name(engine, &work.source, rel);
        engine.stats.found(work.size);
        if !engine.queue.push(Work::File(work), engine.stop_flag()) {
            return;
        }
    }
}

/// Size and time of an explicit file whose producer did not know them.
fn inspect(engine: &Engine<'_>, path: &str) -> Result<Listed, String> {
    let permit = engine
        .source_flow
        .acquire_meta(engine.stop_flag())
        .ok_or_else(|| super::super::cancel::CANCELED_ERROR.to_string())?;
    let listed = match engine.view.source {
        Side::Local => LocalLister.stat(path),
        Side::Remote(backend) => BackendLister(backend).stat(path),
    };
    permit.finish(match &listed {
        Ok(_) => super::super::flow_control::OpOutcome::Done,
        Err(error) => super::super::flow::classify_error(error),
    });
    let listed = listed.map_err(|error| error.to_string())?;
    if listed.is_dir || listed.is_link {
        return Err("Links, Ordner und Spezialdateien werden hier nicht übertragen".to_string());
    }
    Ok(listed)
}
