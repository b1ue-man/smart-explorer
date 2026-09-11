//! Filtered local clipboard pairs retain their validated relative hierarchy.
use super::entries::{validate_transfer_name, TransferCollectionBudget};
use super::upload_plan::{DestinationNames, UploadEntry, UploadPlan};
use crate::app::app_models::TransferMsg;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub(in crate::app) fn upload_pairs_progress(
    backend: &dyn crate::vfs::Backend, pairs: &[(String, String)], destination: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>, cancel: &AtomicBool,
) {
    let plan = collect_pairs(backend, pairs, destination, cancel);
    super::uploads::run_upload_plan(backend, plan, destination, tx, cancel);
}

fn collect_pairs(
    backend: &dyn crate::vfs::Backend, pairs: &[(String, String)], destination: &str, cancel: &AtomicBool,
) -> Result<UploadPlan, String> {
    super::cancel::check(cancel)?;
    let mut plan = UploadPlan::default();
    let mut budget = TransferCollectionBudget::default();
    let mut names = DestinationNames::new(backend, destination);
    let mut roots = HashMap::<String, String>::new();
    let mut files = HashSet::new();
    let mut directories = HashSet::new();
    for (absolute, relative) in pairs {
        super::cancel::check(cancel)?;
        let source = PathBuf::from(absolute);
        if !source.is_absolute() {
            return Err(format!("Upload-Quelle ist nicht absolut: {absolute}"));
        }
        let parts = relative.split('/').collect::<Vec<_>>();
        if parts.is_empty() || relative.contains(['\\', '\0']) {
            return Err(format!("Ungültiger relativer Upload-Pfad: {relative}"));
        }
        for component in &parts {
            if component.is_empty() || matches!(*component, "." | "..") || component.contains(':') {
                return Err(format!("Ungültiger relativer Upload-Pfad: {relative}"));
            }
            validate_transfer_name(component, relative)?;
        }
        budget.record_node(parts.len() - 1, &[absolute, relative])?;
        if directories.contains(relative) || !files.insert(relative.clone()) {
            return Err(format!("Mehrdeutiges Upload-Dateiziel: {relative}"));
        }
        let mut parent = String::new();
        for component in &parts[..parts.len() - 1] {
            if !parent.is_empty() { parent.push('/'); }
            parent.push_str(component);
            if files.contains(&parent) {
                return Err(format!("Upload-Ziel ist zugleich Datei und Verzeichnis: {parent}"));
            }
            directories.insert(parent.clone());
        }
        let metadata = std::fs::symlink_metadata(&source)
            .map_err(|error| format!("{absolute}: Quelle prüfen: {error}"))?;
        super::cancel::check(cancel)?;
        if super::super::upload_is_link_like(&metadata) || !metadata.is_file() {
            return Err(format!("{absolute}: Gefilterte Upload-Quelle ist keine reguläre Datei ohne Link/Reparse-Punkt"));
        }
        let root = parts[0];
        let selected = match roots.get(root) {
            Some(selected) => selected.clone(),
            None => {
                let selected = names.reserve(backend, destination, root, cancel)?;
                budget.record_text(&[root, &selected])?;
                roots.insert(root.to_string(), selected.clone());
                selected
            }
        };
        let target = if parts.len() == 1 { selected } else {
            format!("{selected}/{}", parts[1..].join("/"))
        };
        // Explicit directories preserve the selected root name even when only
        // a small filtered subset is uploaded; no basename flattening occurs.
        let mut parent = target.as_str();
        while let Some((ancestor, _)) = parent.rsplit_once('/') {
            budget.record_text(&[ancestor])?;
            plan.dirs.push(ancestor.to_string());
            parent = ancestor;
        }
        plan.files.push(UploadEntry { src: source, rel: target, size: metadata.len() });
    }
    Ok(plan)
}
