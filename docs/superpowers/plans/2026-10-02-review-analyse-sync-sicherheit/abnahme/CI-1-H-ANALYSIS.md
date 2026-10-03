# CI-1-H-ANALYSIS – belegte Anschlusskorrekturen

Stand: 2026-10-03. Ausschließlich die zugewiesenen Diagnosen aus RV1-Lauf
37145175629 und die belegte Analyse-UI-Größengrenze. Kein neuer Review;
Scope `../scopes/ci-1-h-analysis.json`. Der Remote-Formatterpatch ist bereits angewandt.

## Plan und zweite konkrete API-Klärung

Die Cargo-JSON-Diagnosen nennen den fehlenden egui-Namensraum in den JobState-/
Y156-Consumern, falsche Phasepfade und eine uneindeutige Recycle-Resultableitung.
Die vorhandenen Facaden reexportieren ScanPhase und ReclaimPhase; der Recycle-
Publisher und die Verifikationen verwenden io::Result. Keine neue Typdefinition
oder Abhängigkeit ist erforderlich. Der egui-Consumer verwendet eframe::egui.

| Kohäsiver Korrekturschritt | Erwartung im selben RV1-Einstieg |
| --- | --- |
| Bestehende Phase-/Recycletypen explizit konsumieren | Scan bleibt Scanning, Provider-Rückfall bleibt Walking; SHA-/Längenprüfung, Capture und Restorefehler bleiben unverändert. |
| egui-Reexport in JobState und Y156 importieren | Gleiche UI-/Fixturetypen; sämtliche bestehenden Y156-Assertions bleiben erhalten. |
| Analyse-Bedienkopf auslagern | Scan-Ziele, Remote-Identität, Rescan, Breadcrumb, Fortschritt, Cancel und Ergebnisstatus bleiben gleich; Treemap und deferred Reihenfolge bleiben im bestehenden Modul. Beide UI-Quellen bleiben deutlich unter 500 Zeilen/50 KiB. |
| Eigener statischer Abschluss | Nur eigene Text-/Parsing-/Größen- und Assertionserhaltprüfung; kein neuer Test oder lokaler Lauf. |

Neue kohäsive Quelle: `native/src/app/core/analytics_controls_ui.rs`, mit einer
eigenen additiven Registrierung in app/mod.rs. Die genaue Read-Surface wurde nicht
erweitert; fehlende benannte Pfade erteilen keine Ersatzlesefreigabe.

## Abschluss

Quellseitig abgeschlossen; die Bestätigung erfolgt ausschließlich über denselben
Root-eigenen vollständigen RV1-Einstieg. Keine lokale Compile-/Laufabnahme.

- `sync_job_state_ui.rs` und die Y156-Fixture konsumieren `eframe::egui` explizit.
  Keine neue Abhängigkeit; Y156 erhielt ausschließlich diese Importzeile.
- `analytics_backend.rs` konsumiert `crate::analytics::ScanPhase::Scanning`,
  `backend_duplicates.rs` den vorhandenen Reexport `ReclaimPhase::Walking`.
  Zähler, Rückfall, Abbruchmarker und Budgets bleiben erhalten.
- Die Capture-/Publish-Closure ist als `io::Result<RecycleOutcome>` typisiert.
  SHA/Länge vor und nach Capture, Changed-Outcome, Restore/no-replace und
  Fehlermeldung mit retained_location bleiben unverändert.
- `analytics_controls_ui.rs` besitzt die Scan-Auswahl, Breadcrumb und gemessene
  Statusanzeige. `AnalyticsControls` sammelt dieselben verzögerten Aktionen;
  `App::ui_analytics_controls(&self, ui: &mut egui::Ui, controls: &mut AnalyticsControls)`
  rendert ohne Scan-/Navigationsmutation. Es bleibt eine interne App-Oberfläche.
- Treemap, Accessibility, Hostzahlen, Lese-/Rechteprobleme und Aktionsausführung
  verbleiben in `analytics_ui.rs`. Remote-Konstruktion behält Backend, Root,
  Label, endpoint_prefix und account; dieselbe Quelle wird erneut gescannt.
  Aktion und Panelwechsel werden weiterhin erst nach dem Ende der UI-Borrows angewandt.

Statischer Self-Review: lexikalische Klammer-/String-/Kommentarprüfung, Whitespace,
Import-/Registrierungskontrolle und Größenprüfung unauffällig. Der ausgelagerte
Bedienblock ist nach Rückbenennung der Aktionsfelder und identischer Context-
Zuordnung textgleich; der vollständige deferred Suffix einschließlich bestehender
Treemap-Assertions ist nach Feldrückbenennung exakt gleich. Keine neue Testfunktion,
kein abgeschwächter Schutz oder entfernte Assertion.

Größen mit Reserve: `analytics_ui.rs` 298 Zeilen/12623 Bytes;
`analytics_controls_ui.rs` 254 Zeilen/10438 Bytes. Alle eigenen geänderten
Rustquellen bleiben unter 500 Zeilen/50 KiB. Formatter-/Typ-/Laufbeweise sind remote.

## Unveränderte Acceptance-Symbole

Im selben RV1-Lauf müssen die zugewiesenen Typdiagnosen verschwinden; die bisherigen
Selector-/Szenarien und diese vorhandenen Assertions bleiben erforderlich:

- `app::sync_exit_gate::task_tests::review_task_y156_close_waits_for_completion_after_cancel_and_deadline`
- `app::sync_exit_gate::task_tests::review_task_y156_results_drain_while_worker_is_still_alive`
- `app::sync_exit_gate::task_tests::review_task_y156_update_preflight_waits_for_actual_completion`
- `app::sync_job_state_ui::tests::desktop_job_state_failed_attempt_does_not_hide_prior_success`
- `app::sync_job_state_ui::tests::desktop_job_state_block_takes_precedence_over_old_success_result`
- `app::analytics_ui::accessibility_tests::treemap_semantics_include_location_count_and_size_state`
- `app::analytics_ui::accessibility_tests::empty_treemap_is_disabled_and_named`
- `review_task_recycle_changed_content_is_not_captured`
- `review_task_recycle_publication_failure_restores_without_replacing`

RV1 entdeckt die vollständigen Namen der Recycle-Leafselektoren über seinen
vorhandenen Testhost. Kein separater Aufruf oder weitere Testkampagne.

## Scope-Lücken und offene Grenzen

Diese freigegebenen Read-Pfade fehlen; es wurde keine Alternative erkundet:

- `native/src/analytics/os/shared/reclaim/progress.rs`
- `native/src/local_access/os/shared/checked_recycle.rs`
- `native/src/app/core/analytics_scan.rs`
- `native/src/app/core/storage_view_ui.rs`

Die existierenden Analytics-/LocalAccess-Facaden und der vorhandene Recycle-
Implementierungspfad reichen für alle zugewiesenen Korrekturen. Keine offene
Implementierungs-/Scopeanfrage. Offen bleibt die zentrale Remote-Bestätigung.
Kein Build/Test/Formatter/Server/Install/Git/CI/Graph/Release oder weiterer Agent.

## Exaktes Dateiinventar

Gelesen (teilweise gezielt, eigene neue Dateien eingeschlossen):

- `/tmp/rv1-ci-first/h-analysis.json`
- `docs/refs/egui-close-lifecycle.md`
- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/share-server-tls-auth.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-H-ANALYSIS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fixes.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-1-h-analysis.json`
- `native/src/analytics/core/progress.rs`
- `native/src/analytics/mod.rs`
- `native/src/analytics/os/shared/analytics_backend.rs`
- `native/src/analytics/os/shared/checked_recycle.rs`
- `native/src/analytics/os/shared/reclaim/backend_duplicates.rs`
- `native/src/analytics/os/shared/reclaim/mod.rs`
- `native/src/app/core/analytics_controls_ui.rs`
- `native/src/app/core/analytics_ui.rs`
- `native/src/app/core/state.rs`
- `native/src/app/core/sync_job_state_ui.rs`
- `native/src/app/mod.rs`
- `native/src/app/os/shared/sync_exit_gate_task_tests.rs`
- `native/src/local_access/mod.rs`

Geänderte vorhandene Dateien:

- `native/src/analytics/os/shared/analytics_backend.rs`
- `native/src/analytics/os/shared/checked_recycle.rs`
- `native/src/analytics/os/shared/reclaim/backend_duplicates.rs`
- `native/src/app/core/analytics_ui.rs`
- `native/src/app/core/sync_job_state_ui.rs`
- `native/src/app/mod.rs`
- `native/src/app/os/shared/sync_exit_gate_task_tests.rs`

`app/mod.rs` erhielt ausschließlich die eigene additive Registrierung
`app::analytics_controls_ui` mit `core/analytics_controls_ui.rs` als Dateipfad.

Erstellte Dateien:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-H-ANALYSIS.md`
- `native/src/app/core/analytics_controls_ui.rs`
