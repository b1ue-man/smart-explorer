//! "Herunterladen nach…": the entry lands in the folder chosen in the folder
//! picker. The picker's result keeps its connection (the same live location
//! a mirror or two-way sync destination takes), so equal relative paths on two
//! remotes stay two places. A local folder starts at once; a folder on a
//! connection is listed first ("verbindet…" in the transfer list) so an
//! unreachable target ends with a clear message instead of a failing copy.
use super::prelude::*;
use super::transfer_route::{TransferPlace, TransferSelection};
use super::*;

impl App {
    pub(in crate::app) fn download_to_picked(
        &mut self,
        src: String,
        source: TransferPlace,
        backend: Option<crate::vfs::BackendHandle>,
        is_remote: bool,
        dir: String,
        connection_label: String,
    ) {
        let selection = TransferSelection::roots(source, vec![src], None);
        if !is_remote {
            let target_dir = ensure_dir_root(&dir);
            self.submit_paste(
                &selection,
                &TransferPlace::local(),
                &target_dir,
                CopyMode::Copy,
            );
            return;
        }
        let Some(backend) = backend else {
            self.error_msg = Some("Der gewählte Ordner ist nicht verbunden.".to_string());
            return;
        };
        let target_label =
            TransferPlace::remote(backend.clone(), connection_label.clone()).describe(&dir);
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("transfer-target".into())
            .spawn(move || {
                // The picker listed this folder through the same handle; a
                // failure now means the connection is gone.
                let result = match backend.list_dir(&dir) {
                    Ok(_) => Ok((backend, dir)),
                    Err(error) => Err(error.to_string()),
                };
                let _ = tx.send(result);
            });
        match spawn {
            Ok(_) => {
                self.notice = Some((format!("⇄ verbindet… {target_label}"), Instant::now()));
                self.transfer_center
                    .connect_target(selection, connection_label, target_label, rx);
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Verbindung zum Ziel konnte nicht geprüft werden: {error}"
                ));
            }
        }
    }

    /// "Zielordner öffnen": a tab that shows the target's side goes there;
    /// otherwise a new tab opens it on the transfer's own connection.
    pub(in crate::app) fn open_transfer_target(&mut self, id: u64) {
        let Some(job) = self
            .transfer_center
            .finished_entry(id)
            .and_then(|entry| entry.job.clone())
        else {
            return;
        };
        let target = TransferPlace {
            endpoint: job.target.clone(),
            label: String::new(),
        };
        let shown = (0..self.tabs.len()).find(|&index| {
            self.tab_place(index)
                .is_some_and(|(place, _)| place.same_place(&target))
        });
        match shown {
            Some(index) if index != self.active_tab => self.switch_tab(index),
            Some(_) => {}
            None => {
                self.new_tab();
                self.remote = target.backend().map(|backend| crate::connect::RemoteState {
                    backend: backend.clone(),
                    label: connection_label(&job.target_label, &job.target_dir),
                    agent_version: None,
                    zip_return: None,
                    sftp: None,
                    account: None,
                    endpoint_prefix: None,
                });
            }
        }
        self.start_scan(PathBuf::from(
            job.target_dir.replace('/', std::path::MAIN_SEPARATOR_STR),
        ));
    }
}

/// The connection part of a target label built by `TransferPlace::describe`.
fn connection_label(target_label: &str, dir: &str) -> String {
    target_label
        .strip_suffix(dir)
        .and_then(|label| label.strip_suffix(": "))
        .filter(|label| !label.trim().is_empty())
        .unwrap_or(target_label)
        .to_string()
}
