//! Opening vault places in the desktop app (vault plan C7.2, K05 (1)):
//!
//! - A vault location (`vault://<site>/<path>` from favourites, recent
//!   places, the address) opens as a browsing tab through the vault layer
//!   (`connect::resolve_endpoint`: the worker's session), never as a local
//!   path; a locked vault names itself and how to unlock it.
//! - A vault file opens in its drive: `mount_path_for` gives its path there;
//!   without one, a drive runtime and „Beim Öffnen einhängen“ (default on)
//!   mount it (`ensure_mounted_for`); otherwise a cleartext copy is made
//!   only after the user agreed („Klartextkopie öffnen?“). The copy is the
//!   known remote edit of `remote_open.rs`; its connection to the worker
//!   holds the vault session while it is open.
use std::path::PathBuf;

/// What opening a vault file does (C7.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VaultOpenStep {
    /// Open this path of the vault's drive.
    InDrive(PathBuf),
    /// No drive path: ask for a cleartext copy (K05 (1)), with the reason.
    AskForCopy(String),
}

/// The open rule over the two drive functions of S2 (contract V2,
/// `daemon::mount_path_for`, `daemon::ensure_mounted_for`).
pub(crate) fn open_step(
    location: &str,
    mount_on_open: bool,
    mount_path_for: &dyn Fn(&str) -> Result<Option<PathBuf>, String>,
    ensure_mounted_for: &dyn Fn(&str) -> Result<PathBuf, String>,
) -> VaultOpenStep {
    match mount_path_for(location) {
        Ok(Some(path)) => VaultOpenStep::InDrive(path),
        Ok(None) if mount_on_open => match ensure_mounted_for(location) {
            Ok(path) => VaultOpenStep::InDrive(path),
            Err(error) => VaultOpenStep::AskForCopy(error),
        },
        Ok(None) => {
            VaultOpenStep::AskForCopy("Der Tresor ist nicht als Laufwerk eingehängt".into())
        }
        Err(error) => VaultOpenStep::AskForCopy(error),
    }
}

/// Title of the consent question (K05 (1)).
pub(crate) const COPY_TITLE: &str = "Klartextkopie öffnen?";

/// The consent question: why there is no drive, and what a copy means.
pub(crate) fn copy_question(reason: &str) -> String {
    format!(
        "Die Datei kann nicht im Tresor-Laufwerk geöffnet werden ({reason}).\n\nSoll eine \
         unverschlüsselte Kopie in den App-Daten angelegt und geöffnet werden? Sie bleibt bis \
         zum Beenden von Smart Explorer bestehen; Änderungen werden in den Tresor gespeichert."
    )
}

/// How a vault file of a tab is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VaultFileOpen {
    /// Opened from the drive; nothing else to do.
    Opened,
    /// The user declined the cleartext copy.
    Declined,
    /// The user agreed: the caller makes the copy.
    CopyAgreed,
}

/// The steps of C7.2 with the consent question `ask` (title, text).
pub(crate) fn open_vault_file_with(
    location: &str,
    mount_on_open: bool,
    mount_path_for: &dyn Fn(&str) -> Result<Option<PathBuf>, String>,
    ensure_mounted_for: &dyn Fn(&str) -> Result<PathBuf, String>,
    open_local: &dyn Fn(&PathBuf),
    ask: &dyn Fn(&str, &str) -> bool,
) -> VaultFileOpen {
    match open_step(location, mount_on_open, mount_path_for, ensure_mounted_for) {
        VaultOpenStep::InDrive(path) => {
            open_local(&path);
            VaultFileOpen::Opened
        }
        VaultOpenStep::AskForCopy(reason) => {
            if ask(COPY_TITLE, &copy_question(&reason)) {
                VaultFileOpen::CopyAgreed
            } else {
                VaultFileOpen::Declined
            }
        }
    }
}

/// „Beim Öffnen einhängen“ (C7.2): on until the vault settings of the
/// desktop (VLT-M10) store the choice.
const MOUNT_ON_OPEN: bool = true;

impl crate::app::App {
    /// Opens a vault location as a tab (favourites, recent places, address):
    /// through the vault layer, never as a local path.
    pub(in crate::app) fn open_vault_location(&mut self, location: &str) {
        if self.connecting || self.connect_rx.is_some() {
            return;
        }
        let location = location.to_string();
        let (tx, rx) = crossbeam_channel::unbounded();
        let spawn = std::thread::Builder::new()
            .name("vault-open".into())
            .spawn(move || {
                let _ = tx.send(open_location_result(&location));
            });
        match spawn {
            Ok(_) => {
                self.connect_rx = Some(rx);
                self.connecting = true;
                self.notice = Some(("Öffne Tresor…".to_string(), std::time::Instant::now()));
            }
            Err(error) => {
                self.error_msg = Some(format!("Tresor konnte nicht geöffnet werden: {error}"));
            }
        }
    }

    /// The vault hook of `open_file`: `location` is the vault location of
    /// the file (`vault://<site-id>/<path>`).
    pub(in crate::app) fn open_vault_file(
        &mut self,
        location: &str,
        mode: crate::app::OpenMode,
    ) -> VaultFileOpen {
        let opened = open_vault_file_with(
            location,
            MOUNT_ON_OPEN,
            &|location: &str| crate::daemon::mount_path_for(location),
            &|location: &str| crate::daemon::ensure_mounted_for(location),
            &|path: &PathBuf| crate::app::open_local_path(&path.to_string_lossy(), mode),
            &|title: &str, text: &str| crate::app::confirm_yes_no(title, text),
        );
        if opened == VaultFileOpen::Declined {
            self.notice = Some((
                "Keine Klartextkopie angelegt".to_string(),
                std::time::Instant::now(),
            ));
        }
        opened
    }
}

/// `vault://<SITE>/<path>` opened as a remote tab: the site's ID keys
/// favourites and folder settings (`vault://<site-id>/…`).
fn open_location_result(location: &str) -> crate::connect::ConnectResult {
    let Some((site, path)) = crate::vault::split_place(location) else {
        return crate::connect::ConnectResult::Err("Kein Tresorort".into());
    };
    let site = crate::vault::canonical_vault_location(location)
        .ok()
        .and_then(|canonical| crate::vault::split_place(&canonical))
        .map(|(id, _)| id)
        .unwrap_or(site);
    match crate::connect::resolve_endpoint(&crate::vault::vault_location(&site, "/")) {
        Ok((backend, _)) => {
            crate::vault::note_vault_backend(backend.as_ref(), &site);
            crate::connect::ConnectResult::Ok(crate::connect::Connected {
                remote: Some(crate::connect::RemoteState {
                    backend,
                    label: format!("Tresor {site}"),
                    agent_version: None,
                    zip_return: None,
                    sftp: None,
                    account: None,
                    endpoint_prefix: Some(format!("{}://{site}", crate::vault::VAULT_SCHEME)),
                }),
                net: None,
                target: path,
                label: "Tresor".to_string(),
            })
        }
        Err(error) => crate::connect::ConnectResult::Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn vault_task_open_fallback_requires_consent() {
        let location = "vault://0123456789abcdef0123456789abcdef/Plan.txt";
        let opened = RefCell::new(Vec::new());
        let asked = RefCell::new(Vec::new());
        let open_local = |path: &PathBuf| opened.borrow_mut().push(path.clone());
        let ask = |answer: bool| {
            let asked = &asked;
            move |title: &str, text: &str| {
                asked.borrow_mut().push(format!("{title}|{text}"));
                answer
            }
        };
        let mounted = |_: &str| -> Result<Option<PathBuf>, String> {
            Ok(Some(PathBuf::from(
                "/run/user/1000/smart-explorer/Tresore/A/Plan.txt",
            )))
        };
        let unmounted = |_: &str| -> Result<Option<PathBuf>, String> { Ok(None) };
        let mounts = |_: &str| -> Result<PathBuf, String> { Ok(PathBuf::from("T:\\Plan.txt")) };
        let no_runtime =
            |_: &str| -> Result<PathBuf, String> { Err("keine Laufwerkslaufzeit".into()) };
        // 1. Mounted: opened in the drive, no question, no copy.
        let result = open_vault_file_with(
            location,
            true,
            &mounted,
            &no_runtime,
            &open_local,
            &ask(true),
        );
        assert_eq!(result, VaultFileOpen::Opened);
        // 2. Not mounted, mount on open: mounted now and opened there.
        let result =
            open_vault_file_with(location, true, &unmounted, &mounts, &open_local, &ask(true));
        assert_eq!(result, VaultFileOpen::Opened);
        assert_eq!(opened.borrow().len(), 2);
        assert!(
            asked.borrow().is_empty(),
            "no copy question while a drive opens"
        );
        // 3. No drive runtime: a copy only after consent; declining makes
        //    none.
        let result = open_vault_file_with(
            location,
            true,
            &unmounted,
            &no_runtime,
            &open_local,
            &ask(false),
        );
        assert_eq!(result, VaultFileOpen::Declined);
        let result = open_vault_file_with(
            location,
            true,
            &unmounted,
            &no_runtime,
            &open_local,
            &ask(true),
        );
        assert_eq!(result, VaultFileOpen::CopyAgreed);
        // 4. „Beim Öffnen einhängen“ off: the question comes at once.
        let result = open_vault_file_with(
            location,
            false,
            &unmounted,
            &mounts,
            &open_local,
            &ask(false),
        );
        assert_eq!(result, VaultFileOpen::Declined);
        assert_eq!(opened.borrow().len(), 2, "nothing opened without a drive");
        let questions = asked.borrow();
        assert_eq!(questions.len(), 3);
        assert!(questions
            .iter()
            .all(|question| question.starts_with(COPY_TITLE)));
        assert!(questions[0].contains("keine Laufwerkslaufzeit"));
        // 5. A failing drive lookup asks as well, never copies on its own.
        let broken = |_: &str| -> Result<Option<PathBuf>, String> { Err("IPC".into()) };
        assert_eq!(
            open_step(location, true, &broken, &mounts),
            VaultOpenStep::AskForCopy("IPC".into())
        );
    }
}
