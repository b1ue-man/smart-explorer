//! Bounded asynchronous catalog/icon work and one launch followed only by receipts.
use super::{store, Shared};
use crate::{
    apps::{AppId, CatalogApp, LaunchRequest},
    share::{
        apps_wire::{AppsCommand, AppsResult},
        PeerOpenTarget,
    },
};
use eframe::egui;
use std::{sync::PoisonError, time::Duration};

fn run(
    ctx: &egui::Context,
    state: &Shared,
    work: impl FnOnce(Shared, egui::Context) + Send + 'static,
) {
    let (shared, context) = (state.clone(), ctx.clone());
    if let Err(error) = std::thread::Builder::new()
        .name("device-apps".into())
        .spawn(move || work(shared, context))
    {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = false;
        state.pending = None;
        state.error = Some(format!("Programmanfrage nicht gestartet: {error}"));
    }
}
fn begin(state: &Shared, label: &str) {
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    state.busy = true;
    state.pending = Some(label.into());
    state.error = None;
}
pub(super) fn restore(ctx: &egui::Context, state: &Shared, target: PeerOpenTarget) {
    begin(state, "Gespeicherten Startauftrag laden …");
    run(ctx, state, move |shared, ctx| {
        let result = store::read(&target);
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        match result {
            Ok(saved) => state.saved = saved,
            Err(error) => state.error = Some(error),
        }
        state.restored = true;
        state.busy = false;
        state.pending = None;
        ctx.request_repaint();
    });
}
pub(super) fn catalog(
    ctx: &egui::Context,
    state: &Shared,
    target: PeerOpenTarget,
    expected: Option<String>,
) {
    begin(state, "Programmkatalog des Zielgeräts abfragen …");
    {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.catalog = None;
        state.icons.clear();
        state.page = 0;
    }
    run(ctx, state, move |shared, ctx| {
        let result = crate::daemon::apps_catalog(target, Some(Duration::from_secs(30)), || {});
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        match result {
            Ok(catalog) if expected.as_deref().is_none_or(|d|d==catalog.device_id.as_str())=>state.catalog=Some(catalog),
            Ok(_)=>state.error=Some("Der authentifizierte Katalog gehört zu einer geänderten Geräteidentität. Geräteseite neu öffnen.".into()),
            Err(error)=>state.error=Some(format!("Programmkatalog nicht verfügbar: {error}")),
        }
        state.busy = false;
        state.pending = None;
        ctx.request_repaint();
    });
}
pub(super) fn launch(
    ctx: &egui::Context,
    state: &Shared,
    target: PeerOpenTarget,
    app: CatalogApp,
) -> Result<(), String> {
    let request = {
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        let catalog = state
            .catalog
            .as_ref()
            .ok_or("Zuerst einen aktuellen Programmkatalog abfragen.")?;
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        LaunchRequest {
            request_id: id.try_into().map_err(|e: std::io::Error| e.to_string())?,
            target_device_id: catalog.device_id.clone(),
            app_id: app.app_id,
            user_id: catalog.session.user_id.clone(),
            session_id: catalog.session.session_id.clone(),
            catalog_revision: catalog.revision.clone(),
        }
    };
    let saved = store::Saved {
        target: target.clone(),
        request: request.clone(),
        name: app.name,
    };
    begin(
        state,
        "Startauftrag sicher speichern und einmal versenden …",
    );
    run(ctx, state, move |shared, ctx| {
        if let Err(error) = store::save(&saved) {
            let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
            state.busy = false;
            state.pending = None;
            state.error = Some(error);
            ctx.request_repaint();
            return;
        }
        {
            let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
            state.saved = Some(saved);
            state.receipt = None;
            state.node = None;
            state.new_request_ack = false;
        }
        let result = crate::daemon::apps_command(
            target.clone(),
            AppsCommand::Launch {
                request: request.clone(),
            },
            Some(Duration::from_secs(30)),
            || {
                let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
                state.pending = Some("Warte auf Bestätigung am Zielgerät …".into());
                ctx.request_repaint();
            },
        );
        // A lost launch response never causes another Launch command. Query the
        // exact persisted body/ID once; later refreshes are also Receipt only.
        let (result, launch_error) = match result {
            Ok(value) => (Ok(value), None),
            Err(error) => (
                crate::daemon::apps_command(
                    target,
                    AppsCommand::Receipt {
                        request: request.clone(),
                    },
                    Some(Duration::from_secs(30)),
                    || {},
                ),
                Some(error),
            ),
        };
        settle(&shared, &request, result, launch_error);
        ctx.request_repaint();
    });
    Ok(())
}
pub(super) fn receipt(
    ctx: &egui::Context,
    state: &Shared,
    target: PeerOpenTarget,
    request: LaunchRequest,
) {
    begin(
        state,
        "Ergebnis derselben gespeicherten Request-ID abfragen …",
    );
    run(ctx, state, move |shared, ctx| {
        let result = crate::daemon::apps_command(
            target,
            AppsCommand::Receipt {
                request: request.clone(),
            },
            Some(Duration::from_secs(30)),
            || {},
        );
        settle(&shared, &request, result, None);
        ctx.request_repaint();
    });
}
fn settle(
    shared: &Shared,
    request: &LaunchRequest,
    result: Result<(String, String, AppsResult), String>,
    launch_error: Option<String>,
) {
    let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
    state.busy = false;
    state.pending = None;
    state.receipt = None;
    state.node = None;
    match result {
        Ok((node, device, result)) if device == request.target_device_id.as_str() => {
            let receipt = match result {
                AppsResult::Launch { receipt } => Some(receipt),
                AppsResult::Receipt { receipt } => receipt,
                _ => {
                    state.error = Some("Unerwartete Antwort auf den Startauftrag.".into());
                    return;
                }
            };
            match receipt {
                Some(receipt) if receipt.request == *request => {
                    state.node = Some(node);
                    state.receipt = Some(receipt);
                    state.error = None;
                }
                Some(_) => {
                    state.error = Some("Startbeleg passt nicht zum gespeicherten Auftrag.".into())
                }
                None => {
                    state.error = Some(format!(
                        "Noch kein Startbeleg für diese Request-ID. Kein automatischer Neustart.{}",
                        launch_error
                            .map(|e| format!(" Ursprüngliche Antwort: {e}"))
                            .unwrap_or_default()
                    ))
                }
            }
        }
        Ok(_) => {
            state.error = Some("Startbeleg stammt von einer abweichenden Geräteidentität.".into())
        }
        Err(error) => {
            state.error = Some(format!(
                "Startergebnis unklar; ausschließlich denselben Beleg erneut abfragen: {error}"
            ))
        }
    }
}
pub(super) fn icons(
    ctx: &egui::Context,
    state: &Shared,
    target: PeerOpenTarget,
    revision: String,
    apps: Vec<CatalogApp>,
) {
    let requests: Vec<_> = {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.icons_busy {
            return;
        }
        let requests: Vec<_> = apps
            .into_iter()
            .filter_map(|app| {
                let icon = app.icon?;
                if state.icons.contains_key(&app.app_id) {
                    None
                } else {
                    state.icons.insert(app.app_id.clone(), super::Icon::Loading);
                    Some((app.app_id, icon))
                }
            })
            .collect();
        state.icons_busy = !requests.is_empty();
        requests
    };
    if requests.is_empty() {
        return;
    }
    let keys: Vec<AppId> = requests.iter().map(|(app, _)| app.clone()).collect();
    let (shared, context) = (state.clone(), ctx.clone());
    if let Err(error) = std::thread::Builder::new()
        .name("device-app-icons".into())
        .spawn(move || {
            for (app, icon) in requests {
                {
                    let state = shared.lock().unwrap_or_else(PoisonError::into_inner);
                    if state
                        .catalog
                        .as_ref()
                        .is_none_or(|c| c.revision != revision)
                    {
                        break;
                    }
                    if !state.icons.contains_key(&app) {
                        continue;
                    }
                }
                let result = (|| -> Result<egui::TextureHandle, String> {
                    let reply = crate::daemon::apps_icon(
                        target.clone(),
                        app.clone(),
                        icon.clone(),
                        revision.clone(),
                        Some(Duration::from_secs(15)),
                    )?
                    .ok_or("Symbol ist am Zielgerät nicht mehr verfügbar.")?;
                    let png = reply.bytes_for(&icon).map_err(|e| e.to_string())?;
                    let (w, h, rgba) = crate::screen::core::app_icon::decode_png(&png)
                        .map_err(|e| e.to_string())?;
                    Ok(context.load_texture(
                        format!("apps:{}:{}", target.endpoint_prefix(), icon.sha256),
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
                        egui::TextureOptions::LINEAR,
                    ))
                })();
                let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
                if state
                    .catalog
                    .as_ref()
                    .is_none_or(|c| c.revision != revision)
                {
                    break;
                }
                if state.catalog.as_ref().is_none_or(|c| {
                    !c.apps
                        .iter()
                        .any(|a| a.app_id == app && a.icon.as_ref() == Some(&icon))
                }) {
                    continue;
                }
                state.icons.insert(
                    app,
                    match result {
                        Ok(texture) => super::Icon::Ready(texture),
                        Err(error) => super::Icon::Unavailable(error),
                    },
                );
                context.request_repaint();
            }
            shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .icons_busy = false;
            context.request_repaint();
        })
    {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.icons_busy = false;
        for key in keys {
            state
                .icons
                .insert(key, super::Icon::Unavailable(error.to_string()));
        }
    }
}
