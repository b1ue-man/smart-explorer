//! The catalog of a device's programs with search, pages and „Starten“.
//! While a start is unclear, „Starten“ waits and se clarifies it (design 6
//! F4); the double-start guard stays.
use super::{Icon, State, CLARIFYING};
use crate::apps::{CatalogApp, CatalogSnapshot};
use eframe::egui;

/// The catalog with search, pages and „Starten“; returns the icon batch of
/// the visible page.
pub(super) fn show(
    ui: &mut egui::Ui,
    state: &mut State,
    catalog: &CatalogSnapshot,
    open_receipt: bool,
    launch: &mut Option<CatalogApp>,
) -> Option<(String, Vec<CatalogApp>)> {
    ui.separator();
    ui.label(format!(
        "Benutzer: {} · Sitzung: {}",
        catalog.session.user_id, catalog.session.session_id
    ));
    for limit in &catalog.limits {
        ui.small(limit);
    }
    if !catalog.session.interactive || !catalog.session.unlocked {
        ui.label("Die Hostsitzung ist nicht interaktiv oder gesperrt; ein Start wartet, bis sie entsperrt ist.");
    }
    if ui.text_edit_singleline(&mut state.search).changed() {
        state.page = 0;
    }
    let needle = state.search.to_lowercase();
    let apps: Vec<_> = catalog
        .apps
        .iter()
        .filter(|app| {
            app.name.to_lowercase().contains(&needle)
                || app.app_id.as_str().to_lowercase().contains(&needle)
        })
        .cloned()
        .collect();
    const PAGE: usize = 12;
    let pages = apps.len().div_ceil(PAGE).max(1);
    state.page = state.page.min(pages - 1);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(state.page > 0, egui::Button::new("Zurück"))
            .clicked()
        {
            state.page -= 1;
        }
        ui.label(format!(
            "Seite {} von {pages} · {} Programme",
            state.page + 1,
            apps.len()
        ));
        if ui
            .add_enabled(state.page + 1 < pages, egui::Button::new("Weiter"))
            .clicked()
        {
            state.page += 1;
        }
    });
    let visible: Vec<_> = apps
        .into_iter()
        .skip(state.page * PAGE)
        .take(PAGE)
        .collect();
    // At most the current page's textures; an 8192-entry catalog never
    // allocates thousands of image buffers or parallel requests.
    state
        .icons
        .retain(|app, _| visible.iter().any(|v| v.app_id == *app));
    // The double-start guard stays: while a start is unclear, se clarifies
    // it and „Starten“ waits (design 6 F4).
    if open_receipt {
        ui.label(CLARIFYING);
    }
    for app in &visible {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                match state.icons.get(&app.app_id) {
                    Some(Icon::Ready(texture)) => {
                        ui.image((texture.id(), egui::vec2(32.0, 32.0)));
                    }
                    Some(Icon::Loading) => {
                        ui.spinner();
                    }
                    Some(Icon::Unavailable(reason)) => {
                        ui.label("◇")
                            .on_hover_text(format!("Appsymbol nicht verfügbar: {reason}"));
                    }
                    None => {
                        ui.label("◇")
                            .on_hover_text("Kein bestätigtes Appsymbol im Katalog.");
                    }
                }
                ui.label(&app.name);
                if ui
                    .add_enabled(
                        !state.busy && !open_receipt && app.can_launch,
                        egui::Button::new("Starten"),
                    )
                    .clicked()
                {
                    *launch = Some(app.clone());
                }
            });
            if let Some(reason) = &app.unavailable_reason {
                ui.small(format!("Start nicht verfügbar: {reason}"));
            }
            ui.collapsing("App-ID", |ui| {
                ui.label(app.app_id.as_str());
            });
        });
    }
    if visible.is_empty() {
        ui.label("Keine für diese Beziehung sichtbaren Programme passen zur Suche.");
    }
    Some((catalog.revision.clone(), visible))
}
