use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crossbeam_channel::Receiver;

use crate::app::vault_modules::vault_feed::{guard, poll, spawn, Poll};
use crate::app::vault_modules::vault_ui_settings;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VaultOpenStep {
    InDrive(PathBuf),
    AskForCopy(String),
    Blocked(String),
}

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
            Err(error) => failed_step(error),
        },
        Ok(None) => {
            VaultOpenStep::Blocked("Der Tresor ist nicht als Laufwerk eingehängt; ‚Beim Öffnen einhängen‘ ist ausgeschaltet".into())
        }
        Err(error) => failed_step(error),
    }
}

fn failed_step(error: String) -> VaultOpenStep {
    if error.starts_with(crate::vault::MOUNT_RUNTIME_MISSING) {
        VaultOpenStep::AskForCopy(error)
    } else {
        VaultOpenStep::Blocked(error)
    }
}

fn missing_runtime_for(location: &str) -> bool {
    let Some((site, _)) = crate::vault::split_place(location) else { return false; };
    let Ok(record) = crate::vault::RegistryStore::open_default().resolve(&site) else { return false; };
    crate::daemon::list_mounts().is_ok_and(|mounts| mounts.iter().any(|mount| {
        matches!(&mount.config.source, crate::mount::MountSource::Vault { site_id } if *site_id == record.id)
            && matches!(mount.status, crate::mount::MountStatus::RuntimeUnavailable { .. })
    }))
}

pub(crate) const COPY_TITLE: &str = "Klartextkopie öffnen?";

pub(crate) fn copy_question(reason: &str) -> String {
    format!(
        "Die Laufwerkslaufzeit fehlt ({reason}).\n\nUnverschlüsselte Kopie unter {} anlegen und öffnen? \
         Offene Bearbeitungen bleiben zur Wiederherstellung auch nach dem App-Ende erhalten; \
         automatische oder sichere Löschung ist dafür nicht zugesagt. Sperren des Tresors verhindert weitere \
         Zugriffe und Rückschreiben in den Tresor; bereits geöffnete Programme können ihre \
         Klartextdaten weiter behalten. Änderungen werden nur bei entsperrtem Tresor gespeichert.",
        crate::transfer::temp_root().display()
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VaultFileOpen {
    Opened,
    Declined,
    CopyAgreed,
    Pending,
}

#[cfg(test)]
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
        VaultOpenStep::Blocked(_) => VaultFileOpen::Declined,
        VaultOpenStep::AskForCopy(reason) => {
            if ask(COPY_TITLE, &copy_question(&reason)) {
                VaultFileOpen::CopyAgreed
            } else {
                VaultFileOpen::Declined
            }
        }
    }
}

#[derive(Clone)]
pub(in crate::app) enum OpenCopy {
    Ready {
        backend: crate::vfs::BackendHandle,
        path: String,
        name: String,
        id: Option<String>,
    },
    Lazy {
        site: String,
        path: String,
        name: String,
    },
}

type CopyParts = (crate::vfs::BackendHandle, String, String, Option<String>);

impl OpenCopy {
    fn into_parts(self) -> Result<CopyParts, String> {
        match self {
            Self::Ready {
                backend,
                path,
                name,
                id,
            } => {
                check_copy_backend(&backend)?;
                Ok((backend, path, name, id))
            },
            Self::Lazy { site, path, name } => {
                let root = crate::vault::vault_location(&site, "/");
                let (backend, _) = crate::connect::resolve_endpoint(&root)?;
                crate::vault::note_vault_backend(backend.as_ref(), &site);
                check_copy_backend(&backend)?;
                Ok((backend, path, name, None))
            }
        }
    }
}

fn check_copy_backend(backend: &crate::vfs::BackendHandle) -> Result<(), String> {
    let site = crate::vault::vault_site_of(backend.as_ref()).flatten()
        .ok_or_else(|| "Tresoridentität der Klartextquelle ist unbekannt".to_string())?;
    let policy = crate::vault::mount_policy(&site)?;
    let expected = format!("vault:{}#{}", policy.binding.account, policy.binding.root);
    let namespace = format!("vault:{}#{}", policy.binding.namespace, policy.binding.root);
    if backend.state_identity() != expected || backend.namespace_identity() != namespace {
        return Err("Konto oder Root des Tresors wurde geändert; den Tresor bitte erneut öffnen".into());
    }
    Ok(())
}

struct PendingOpen {
    rx: Option<Receiver<VaultOpenStep>>,
    mode: crate::app::OpenMode,
    copy: OpenCopy,
    location: String,
}

fn pending() -> &'static Mutex<Vec<PendingOpen>> {
    static PENDING: OnceLock<Mutex<Vec<PendingOpen>>> = OnceLock::new();
    PENDING.get_or_init(Mutex::default)
}

impl crate::app::App {
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

    pub(in crate::app) fn open_vault_file(
        &mut self,
        location: &str,
        mode: crate::app::OpenMode,
        copy: &OpenCopy,
    ) -> VaultFileOpen {
        match crate::daemon::mount_path_for(location) {
            Ok(Some(path)) => {
                crate::app::open_local_path(&path.to_string_lossy(), mode);
                VaultFileOpen::Opened
            }
            Ok(None) if vault_ui_settings::mount_on_open() => self.start_vault_mount(location, mode, copy),
            Ok(None) => self.vault_open_failure(location, "‚Beim Öffnen einhängen‘ ist ausgeschaltet", mode, copy),
            Err(error) => self.vault_open_failure(location, &error, mode, copy),
        }
    }

    pub(in crate::app) fn open_vault_path(&mut self, site: &str, path: &str) {
        let location = crate::vault::vault_location(site, path);
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        let copy = OpenCopy::Lazy {
            site: site.to_string(),
            path: path.to_string(),
            name,
        };
        let mode = crate::app::OpenMode::Default;
        if self.open_vault_file(&location, mode, &copy) == VaultFileOpen::CopyAgreed {
            self.continue_vault_copy(copy, mode);
        }
    }

    fn start_vault_mount(
        &mut self,
        location: &str,
        mode: crate::app::OpenMode,
        copy: &OpenCopy,
    ) -> VaultFileOpen {
        self.start_vault_mount_choice(location, mode, copy, None)
    }

    fn start_vault_mount_choice(&mut self, location: &str, mode: crate::app::OpenMode, copy: &OpenCopy, trust: Option<crate::vault::RootBinding>) -> VaultFileOpen {
        let target = location.to_string();
        let ctx = crate::app::vault_modules::vault_feed::context();
        let started = spawn(&ctx, "vault-mount", move || {
            if let Some(binding) = trust {
                if let Err(error) = crate::vault::confirm_root_trust(&binding) { return VaultOpenStep::Blocked(error); }
            }
            open_step(
                &target,
                true,
                &|location: &str| crate::daemon::mount_path_for(location),
                &|location: &str| crate::daemon::ensure_mounted_for(location),
            )
        });
        match started {
            Ok(rx) => {
                guard(pending()).push(PendingOpen {
                    rx: Some(rx),
                    mode,
                    copy: copy.clone(),
                    location: location.to_string(),
                });
                self.notice = Some((
                    "Tresor wird eingehängt …".to_string(),
                    std::time::Instant::now(),
                ));
                VaultFileOpen::Pending
            }
            Err(error) => self.vault_open_failure(location, &error, mode, copy),
        }
    }

    fn vault_open_failure(&mut self, location: &str, reason: &str, mode: crate::app::OpenMode, copy: &OpenCopy) -> VaultFileOpen {
        if reason.contains(crate::vault::ROOT_TRUST_REQUIRED) {
            let record = crate::vault::split_place(location)
                .and_then(|(site, _)| crate::vault::RegistryStore::open_default().resolve(&site).ok());
            if let Some(record) = record {
                match crate::vault::mount_policy(&record.id) {
                    Ok(policy) => {
                        let question = format!("Tresor „{}“\nSpeicherort: {}\nBackend: {}\nKonto/Peer: {}\nRoot: {}\n\nDie technische Rootbegrenzung ist nicht belegt. Diesem Speicherort für das Einhängen vertrauen? \
                            Die Freigabe gilt nur für diese Identitäten, erlaubt keine Klartextkopie und kann am Tresor widerrufen werden.", record.display_label(), record.location, policy.binding.backend, policy.binding.account, policy.binding.root);
                        if crate::app::confirm_yes_no("Diesem Tresorspeicher vertrauen?", &question) {
                            return self.start_vault_mount_choice(location, mode, copy, Some(policy.binding));
                        }
                    }
                    Err(error) => {
                        self.error_msg = Some(format!("Speicheridentität nicht bestätigt: {error}"));
                        return VaultFileOpen::Declined;
                    }
                }
            }
        } else if reason.starts_with(crate::vault::MOUNT_RUNTIME_MISSING) && missing_runtime_for(location) {
            return self.ask_vault_copy(location, reason);
        }
        self.error_msg = Some(format!("Tresordatei nicht geöffnet: {reason}"));
        VaultFileOpen::Declined
    }

    fn ask_vault_copy(&mut self, location: &str, reason: &str) -> VaultFileOpen {
        let site = crate::vault::split_place(location).map(|(site, _)| site);
        let policy = site.as_deref().and_then(|site| crate::vault::mount_policy(site).ok());
        if policy.as_ref().is_some_and(|policy| policy.cleartext_copy) {
            return VaultFileOpen::CopyAgreed;
        }
        if !crate::app::confirm_yes_no(COPY_TITLE, &copy_question(reason)) {
            self.notice = Some(("Keine Klartextkopie angelegt".into(), std::time::Instant::now()));
            return VaultFileOpen::Declined;
        }
        if let Some(site) = site {
            if crate::app::confirm_yes_no("Immer für diesen Tresor?", "Diese separate Klartextkopie-Freigabe bei fehlender Laufwerkslaufzeit speichern? Speicherort, fehlende Löschgarantie und Sperrgrenze gelten wie im vorigen Dialog. Die Freigabe ist am Tresor widerrufbar.") {
                if let Err(error) = crate::vault::set_cleartext_copy_policy(&site, true) {
                    self.error_msg = Some(format!("Klartextkopie-Freigabe nicht gespeichert: {error}"));
                    return VaultFileOpen::Declined;
                }
            }
        }
        VaultFileOpen::CopyAgreed
    }

    fn continue_vault_copy(&mut self, copy: OpenCopy, mode: crate::app::OpenMode) {
        match copy.into_parts() {
            Ok((backend, path, name, id)) => {
                self.open_remote_file_copy(backend, path, name, id, mode);
            }
            Err(error) => {
                self.error_msg = Some(format!("Klartextkopie nicht möglich: {error}"));
            }
        }
    }

    pub(in crate::app) fn drain_vault_mount(&mut self) {
        let mut answers = Vec::new();
        {
            let mut held = guard(pending());
            let mut index = 0;
            while index < held.len() {
                match poll(&mut held[index].rx) {
                    Poll::Pending => index += 1,
                    Poll::Ready(step) => {
                        let done = held.remove(index);
                        answers.push((done, Some(step)));
                    }
                    Poll::Gone => {
                        let done = held.remove(index);
                        answers.push((done, None));
                    }
                }
            }
        }
        for (done, step) in answers {
            match step {
                Some(VaultOpenStep::InDrive(path)) => {
                    crate::app::open_local_path(&path.to_string_lossy(), done.mode);
                }
                Some(VaultOpenStep::AskForCopy(reason) | VaultOpenStep::Blocked(reason)) => {
                    if self.vault_open_failure(&done.location, &reason, done.mode, &done.copy) == VaultFileOpen::CopyAgreed {
                        self.continue_vault_copy(done.copy, done.mode);
                    }
                }
                None => {
                    self.error_msg =
                        Some("Das Einhängen des Tresors endete ohne Antwort.".to_string());
                }
            }
        }
    }
}

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
#[path = "vault_open_tests.rs"]
mod tests;
