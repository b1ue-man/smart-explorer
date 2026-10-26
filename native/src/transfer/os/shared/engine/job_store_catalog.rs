//! Background discovery and live vault monitoring of stored jobs.
use std::io;

use super::{
    path, read_record, record_context, record_io, root, valid_id, Endpoint,
    JobRecord, Mutex, RecordContext, RecordKind, TransferJob,
};

pub(crate) fn unconfirmed_jobs() -> io::Result<Vec<JobRecord>> {
    let entries = match std::fs::read_dir(root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| valid_id(id))
        else {
            continue;
        };
        let record: JobRecord = read_record(&path(id)?, RecordKind::Job, id, u64::MAX, false)?;
        if record.version == 1 && record.id == id && !record.complete {
            records.push(record);
        }
    }
    records.sort_by(|one, two| one.id.cmp(&two.id));
    Ok(records)
}

pub(crate) fn reopen_job(
    id: &str,
    source: Endpoint,
    target: Endpoint,
) -> Result<TransferJob, String> {
    let record: JobRecord = read_record(
        &path(id).map_err(|error| error.to_string())?,
        RecordKind::Job,
        id,
        u64::MAX,
        false,
    )
    .map_err(|error| error.to_string())?;
    record.bind(source, target)
}

#[derive(Clone, Default)]
pub(crate) struct CatalogState {
    pub ready: bool,
    pub records: Vec<JobRecord>,
    pub error: Option<String>,
    pub unavailable: Vec<String>,
}

fn load_catalog() -> io::Result<(CatalogState, Vec<(String, RecordContext)>)> {
    let entries = match std::fs::read_dir(root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((
                CatalogState {
                    ready: true,
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        Err(error) => return Err(error),
    };
    let mut state = CatalogState {
        ready: true,
        ..Default::default()
    };
    let mut contexts = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| valid_id(id))
        else {
            continue;
        };
        let loaded = (|| {
            let (record, sealed): (JobRecord, bool) =
                record_io::read_record_state(&path(id)?, RecordKind::Job, id, u64::MAX, false)?;
            if record.version != 1 || record.id != id {
                return Err(io::Error::other("Ungültiger Jobdatensatz"));
            }
            if record.complete {
                return Ok(None);
            }
            let context = record_context(&record.vault_locations)?;
            if sealed != context.is_some() {
                return Err(io::Error::other(
                    "Vaultbindung des Jobdatensatzes ist unbestätigt",
                ));
            }
            if let Some(context) = &context {
                crate::vault::check_transfer_blob_context_live(context)?;
            }
            Ok(Some((record, context)))
        })();
        match loaded {
            Ok(Some((record, context))) => {
                if let Some(context) = context {
                    contexts.push((id.to_string(), context));
                }
                state.records.push(record);
            }
            Ok(None) => {}
            Err(_) => state.unavailable.push(id.to_string()),
        }
    }
    if !state.unavailable.is_empty() {
        state.error = Some(format!(
            "Gesperrte oder unbestätigte Job-IDs: {}",
            state.unavailable.join(", ")
        ));
    }
    Ok((state, contexts))
}

pub(crate) struct RecoveryCatalog(std::sync::Arc<Mutex<CatalogState>>);

impl RecoveryCatalog {
    pub fn start() -> io::Result<Self> {
        let state = std::sync::Arc::new(Mutex::new(CatalogState::default()));
        let background = state.clone();
        std::thread::Builder::new()
            .name("transfer-recovery-catalog".into())
            .spawn(move || {
                let (result, contexts) = match load_catalog() {
                    Ok(loaded) => loaded,
                    Err(error) => (
                        CatalogState {
                            ready: true,
                            error: Some(error.to_string()),
                            ..Default::default()
                        },
                        Vec::new(),
                    ),
                };
                *background
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = result;
                let weak = std::sync::Arc::downgrade(&background);
                drop(background);
                record_io::monitor_contexts(weak, contexts, |state, id| {
                    state.records.retain(|record| record.id != id);
                    if !state.unavailable.iter().any(|record| record == id) {
                        state.unavailable.push(id.to_string());
                    }
                    state.error = Some(format!(
                        "Gesperrte oder unbestätigte Job-IDs: {}",
                        state.unavailable.join(", ")
                    ));
                });
            })?;
        Ok(Self(state))
    }
    pub fn snapshot(&self) -> CatalogState {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}
