//! Two actions of the context menu of a vault site that run in the
//! background (U29-4): „Austragen…“ (with its question) and „Als Laufwerk
//! einhängen“.
use std::sync::{Mutex, OnceLock};

use crossbeam_channel::Receiver;
use eframe::egui;

use super::vault_feed::{guard, poke, poll, spawn, Poll};
use super::vault_sidebar_ui::Effect;

struct RemoveAsk {
    site: String,
    label: String,
    rx: Option<Receiver<Result<(), String>>>,
}

struct MountJob {
    label: String,
    rx: Option<Receiver<Result<String, String>>>,
}

#[derive(Default)]
struct State {
    remove: Option<RemoveAsk>,
    mounts: Vec<MountJob>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

/// Asks whether `site` is removed from the list.
pub(crate) fn ask_remove(site: &str, label: &str) {
    guard(state()).remove = Some(RemoveAsk {
        site: site.to_string(),
        label: label.to_string(),
        rx: None,
    });
}

/// „Als Laufwerk einhängen“: the drive of the site (`vault://<site-id>`,
/// `MountSource::Vault`) over the drive path of the daemon, the same as
/// `se drive mount vault://<SITE>`; a drive entered earlier is started again.
pub(crate) fn mount_drive(ctx: &egui::Context, site: &str, label: &str) -> Vec<Effect> {
    let (site, name) = (site.to_string(), label.to_string());
    match spawn(ctx, "vault-mount-drive", move || mount_site(&site, &name)) {
        Ok(rx) => {
            guard(state()).mounts.push(MountJob {
                label: label.to_string(),
                rx: Some(rx),
            });
            vec![Effect::Notice(format!(
                "Tresor „{label}“ wird als Laufwerk eingehängt …"
            ))]
        }
        Err(error) => vec![Effect::Error(error)],
    }
}

fn mount_site(site: &str, label: &str) -> Result<String, String> {
    use crate::mount::{DriveSelection, MountConfig, MountId, MountMode, MountSource, MountStatus};
    let existing = crate::daemon::list_mounts()
        .unwrap_or_default()
        .into_iter()
        .find(|snapshot| {
            matches!(&snapshot.config.source, MountSource::Vault { site_id } if site_id == site)
        });
    let snapshot = match existing {
        Some(snapshot) => match snapshot.status {
            MountStatus::Mounted { .. } | MountStatus::MountedAt { .. } => snapshot,
            _ => crate::daemon::retry_mount(snapshot.config.id.clone())?,
        },
        None => {
            let id = MountId::new_random().map_err(|error| error.to_string())?;
            let label: String = label.chars().take(128).collect();
            let config = MountConfig::new(
                id,
                MountSource::Vault {
                    site_id: site.to_string(),
                },
                DriveSelection::Automatic,
                MountMode::ReadWrite,
                label,
            )
            .map_err(|error| error.to_string())?;
            crate::daemon::start_mount(config)?
        }
    };
    Ok(
        match crate::app::mount_ui_open::mounted_location(&snapshot.status) {
            Some(path) => format!("Laufwerk bereit: {}", path.display()),
            None => "Das Laufwerk wird eingehängt …".to_string(),
        },
    )
}

/// Draws the question and collects the answers of the threads.
pub(crate) fn show(ctx: &egui::Context) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut held = guard(state());
    let state: &mut State = &mut held;
    let mut index = 0;
    while index < state.mounts.len() {
        let job = &mut state.mounts[index];
        match poll(&mut job.rx) {
            Poll::Pending => {
                index += 1;
                continue;
            }
            Poll::Ready(Ok(text)) => {
                effects.push(Effect::Notice(format!("Tresor „{}“: {text}", job.label)));
            }
            Poll::Ready(Err(error)) => {
                effects.push(Effect::Error(format!("Laufwerk: {error}")));
            }
            Poll::Gone => effects.push(Effect::Error(
                "Das Einhängen endete ohne Antwort des Dienstes.".to_string(),
            )),
        }
        state.mounts.remove(index);
    }
    if let Some(ask) = state.remove.as_mut() {
        let mut done = false;
        match poll(&mut ask.rx) {
            Poll::Pending => {}
            Poll::Ready(Ok(())) => {
                effects.push(Effect::Removed(ask.site.clone()));
                effects.push(Effect::Notice(format!(
                    "Tresor „{}“ ausgetragen.",
                    ask.label
                )));
                done = true;
            }
            Poll::Ready(Err(error)) => {
                effects.push(Effect::Error(format!("Austragen: {error}")));
                done = true;
            }
            Poll::Gone => {
                effects.push(Effect::Error(
                    "Das Austragen endete ohne Antwort.".to_string(),
                ));
                done = true;
            }
        }
        if done {
            state.remove = None;
            poke();
        }
    }
    if let Some(ask) = state.remove.as_mut() {
        let busy = ask.rx.is_some();
        let mut yes = false;
        let mut no = false;
        crate::app::dialog_geometry::window(ctx, "vault_actions_ui.0", "Tresor austragen?")
            .id(egui::Id::new("vault_remove_ask"))
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!(
                    "Tresor „{}“ austragen? Die Tresor-Dateien bleiben unverändert.",
                    ask.label
                ));
                ui.horizontal(|ui| {
                    if busy {
                        ui.spinner();
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("Austragen"))
                        .clicked()
                    {
                        yes = true;
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("Abbrechen"))
                        .clicked()
                    {
                        no = true;
                    }
                });
            });
        if no {
            state.remove = None;
        } else if yes {
            let site = ask.site.clone();
            match spawn(ctx, "vault-remove", move || {
                crate::vault::remove_entered(&site)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }) {
                Ok(rx) => ask.rx = Some(rx),
                Err(error) => {
                    effects.push(Effect::Error(error));
                    state.remove = None;
                }
            }
        }
    }
    effects
}
