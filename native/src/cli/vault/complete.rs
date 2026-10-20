//! Completion candidates of `se vault` (vault plan C7.3, U26-12): sites by
//! ID with their label as help, vault jobs by ID with their name as help.
//! The completion of `vault://<SITE>/` targets of `se drive mount` belongs to
//! `cli/drive_options.rs` (TODO `VAULT-COMPLETE-DRIVE-TARGET`).
use clap::builder::StyledStr;
use clap_complete::CompletionCandidate;

use crate::vault::RegistryStore;

fn plain(text: &str) -> StyledStr {
    StyledStr::from(text.replace(['\t', '\r', '\n'], " "))
}

/// Every entered site: its ID, its label as help.
pub(super) fn site_candidates() -> Vec<CompletionCandidate> {
    RegistryStore::open_default()
        .load()
        .map(|load| load.sites)
        .unwrap_or_default()
        .iter()
        .map(|site| {
            CompletionCandidate::new(site.id.as_str()).help(Some(plain(site.display_label())))
        })
        .collect()
}

/// Every vault sync job: its ID, its name as help.
pub(super) fn job_candidates() -> Vec<CompletionCandidate> {
    crate::syncjobs::load_report()
        .map(|report| report.jobs)
        .unwrap_or_default()
        .iter()
        .filter(|job| job.is_vault())
        .map(|job| CompletionCandidate::new(job.id.as_str()).help(Some(plain(&job.name))))
        .collect()
}
