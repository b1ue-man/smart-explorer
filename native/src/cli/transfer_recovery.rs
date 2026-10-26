//! Saved terminal transfers are rebound explicitly and monitored off the UI thread.
use super::{record_path, SavedCommand};
use crate::cli::target::Target;
use crate::transfer::{record_context, Endpoint, RecordContext, RecordKind, StoredEndpoint};

fn saved_commands() -> Result<(SavedCommandsState, Vec<(String, RecordContext)>), String> {
    let entries = match std::fs::read_dir(crate::support_dirs::app_data_file("transfer-cli-jobs")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((
                SavedCommandsState {
                    ready: true,
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut state = SavedCommandsState {
        ready: true,
        ..Default::default()
    };
    let mut contexts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
            continue;
        };
        let loaded = (|| {
            let path = record_path(id)?;
            let (record, sealed): (SavedCommand, bool) =
                crate::transfer::read_record_state(&path, RecordKind::Job, id, u64::MAX, false)
                    .map_err(|error| error.to_string())?;
            if record.version != 1 || record.id != id {
                return Err("Ungültiger gespeicherter Terminalauftrag".to_string());
            }
            if record.complete {
                return Ok(None);
            }
            let context =
                record_context(&record.vault_locations).map_err(|error| error.to_string())?;
            if sealed != context.is_some() {
                return Err("Vaultbindung des Auftrags ist unbestätigt".into());
            }
            if let Some(context) = &context {
                crate::vault::check_transfer_blob_context_live(context)
                    .map_err(|error| error.to_string())?;
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

fn recover_saved_command(
    record: &SavedCommand,
    source: Endpoint,
    target: Endpoint,
    observer: &crate::cli::tree_observer::TreeObserver,
) -> Result<(), String> {
    let bind = |endpoint: Endpoint,
                expected: &StoredEndpoint,
                local_root: &Option<String>,
                path: &str| match endpoint {
        Endpoint::Local if expected.local => Target::rebind_native(
            local_root
                .as_deref()
                .ok_or_else(|| "Lokale Wiederaufnahmewurzel fehlt".to_string())?,
            path.to_string(),
            expected,
        ),
        Endpoint::Remote(backend) => Target::rebind(backend, path.to_string(), expected),
        _ => Err("Ausgewählte Verbindung passt nicht zum gespeicherten Auftrag".to_string()),
    };
    let source = bind(
        source,
        &record.source,
        &record.source_local_root,
        &record.source_path,
    )?;
    let target = bind(
        target,
        &record.target,
        &record.target_local_root,
        &record.target_argument,
    )?;
    if record.moving {
        crate::cli::ops::move_observed_in_job(
            &source,
            &target,
            record.recursive,
            record.force,
            observer,
            None,
            Some(&record.id),
        )?;
    } else {
        crate::cli::ops::copy_observed_in_job(
            &source,
            &target,
            record.recursive,
            record.force,
            observer,
            None,
            Some(&record.id),
        )?;
    }
    Ok(())
}

pub(crate) struct RecoveredCommandTask {
    observer: std::sync::Arc<crate::cli::tree_observer::TreeObserver>,
    result: std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>>,
    _external: std::sync::Arc<crate::transfer::ExternalTransfer>,
}

impl RecoveredCommandTask {
    pub fn start(record: SavedCommand, source: Endpoint, target: Endpoint) -> Result<Self, String> {
        let observer = std::sync::Arc::new(crate::cli::tree_observer::TreeObserver::default());
        let result = std::sync::Arc::new(std::sync::Mutex::new(None));
        let external = crate::transfer::register_external(format!("Wiederaufnahme {}", record.id));
        external.set_note("Gespeicherter Auftrag wird wiederaufgenommen");
        let tracked = external.clone();
        let running = observer.clone();
        let completed = result.clone();
        std::thread::Builder::new()
            .name("saved-cli-transfer".into())
            .spawn(move || {
                let outcome = recover_saved_command(&record, source, target, &running);
                let progress = running.progress();
                tracked.set_files_total(progress.files_total);
                tracked.add_bytes(progress.bytes_written);
                for _ in 0..progress.files_written {
                    tracked.file_done();
                }
                if outcome.is_err() {
                    tracked.issue(
                        "",
                        "Wiederaufnahme bleibt unbestätigt; Details in der Wiederaufnahmezeile",
                    );
                }
                tracked.finish();
                *completed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(outcome);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            observer,
            result,
            _external: external,
        })
    }
    pub fn cancel(&self) {
        self.observer.cancel();
    }
    pub fn progress_text(&self) -> String {
        let progress = self.observer.progress();
        format!(
            "{}: {} von {} Dateien bestätigt",
            progress.phase.label(),
            progress.files_written,
            progress.files_total
        )
    }
    pub fn result(&self) -> Option<Result<(), String>> {
        self.result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[derive(Clone, Default)]
pub(crate) struct SavedCommandsState {
    pub ready: bool,
    pub records: Vec<SavedCommand>,
    pub error: Option<String>,
    pub unavailable: Vec<String>,
}
pub(crate) struct SavedCommandsCatalog(std::sync::Arc<std::sync::Mutex<SavedCommandsState>>);
impl SavedCommandsCatalog {
    pub fn start() -> Result<Self, String> {
        let state = std::sync::Arc::new(std::sync::Mutex::new(SavedCommandsState::default()));
        let output = state.clone();
        std::thread::Builder::new()
            .name("saved-cli-job-catalog".into())
            .spawn(move || {
                let (state, contexts) = match saved_commands() {
                    Ok(loaded) => loaded,
                    Err(error) => (
                        SavedCommandsState {
                            ready: true,
                            error: Some(error),
                            ..Default::default()
                        },
                        Vec::new(),
                    ),
                };
                *output
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
                let weak = std::sync::Arc::downgrade(&output);
                drop(output);
                crate::transfer::monitor_contexts(weak, contexts, |state, id| {
                    state.records.retain(|record| record.id != id);
                    if !state.unavailable.iter().any(|record| record == id) {
                        state.unavailable.push(id.to_string());
                    }
                    state.error = Some(format!(
                        "Gesperrte oder unbestätigte Job-IDs: {}",
                        state.unavailable.join(", ")
                    ));
                });
            })
            .map_err(|error| error.to_string())?;
        Ok(Self(state))
    }
    pub fn snapshot(&self) -> SavedCommandsState {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}
