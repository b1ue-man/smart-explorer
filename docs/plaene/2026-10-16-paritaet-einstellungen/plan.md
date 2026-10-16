# Alle Einstellungen auf allen Oberflächen (2026-10-16)

Stand: 2026-10-16. Teil „Programmierer 2“ des Stapels „GUI-Parität“: alle Einstellungen der App außer Share und Exec (Share/Exec: Programmierer 1, `docs/oberflaechen/share-exec.md`). Lesestand des Codes: `7e7203c9` (v0.5.181) plus Arbeitsbaum vom 2026-10-16. Nichts davon ist gebaut oder ausgeführt (Projektregel AGENTS.md); „umgesetzt“ heißt Code-Lesung, bis der eine Suite-Lauf und das Release es bestätigen.

Die Abgleichmatrix mit allen Belegen (`Datei:Zeile`) ist das Projektdokument [`docs/oberflaechen/einstellungen.md`](../../oberflaechen/einstellungen.md). Dieser Plan nennt nur Auftrag, Meilensteine und Abnahme.

## A — Auftrag (nur aus dem Prompt)

| Nr. | Ergebnis | Quelle |
|---|---|---|
| P1 | Jede Einstellung ist in der GUI verfügbar | Nutzer: „stelle sicher dass alle einstellungen in der gui verfügbar sind“ |
| P2 | CLI und GUI stimmen überein; Ausnahme nur die Terminal-Verbindung selbst und vergleichbar nur in einer Oberfläche Sinnvolles, mit Grund | Nutzer: „stelle sicher dass cli und gui matchen, bis auf natürlcih terminal verbindung selbst oder so“ |
| P3 | MCP ist eine Oberfläche mit denselben Regeln | Nutzer: „oberfläche ist auch mcp, selbe regeln“ |
| P4 | Keine Oberfläche wird in irgendeinem Schritt vergessen (CLI mit Ausgaben, JSON und Vervollständigung, Desktop-GUI, Android-GUI, Bridge, IPC, MCP, Doku) | Nutzer: „du DARFST ABSOLUT NIE irgendeine existierende oberfläche vergessen …“ |
| P5 | Details bekommen eigene Unterseiten statt Überladung | Nutzer: „details müssen nicht die gleiche seite teilen wie alles andere“ (für die Share-GUI gesagt; hier auf neue Einstellungsfelder angewandt — Annahme, ausdrücklich) |
| P6 | Inventur und Abgleich liegen als dauerhaftes Projektdokument vor | Nutzer: „die dokumente musst du lokal führen, das wissen musst du lokal führen“ |

Annahme (ausdrücklich, aus Nachricht 2 und dem Auftrag des Koordinators): „alle einstellungen“ meint alle Einstellungen der App, nicht nur Exec.

Grundfunktion: Wer eine Einstellung in einer Oberfläche sieht, findet dieselbe Einstellung mit derselben Bedeutung, denselben Werten und demselben Standard in jeder anderen Oberfläche, die diesen Bereich bedient; gespeicherte Formate und bestehende Bedienwege bleiben unverändert.

Grenzen: keine Datei unter Share/Exec und keine der gemeinsamen Dateien (Liste im Auftrag); dafür Ergänzungstexte im Übergabeordner. Keine Builds, Tests, Commits, Pushes, CI, Releases.

## B — Meilensteine

| Nr. | Ergebnis | Dateien | Abnahme (in der einen Task-Suite) |
|---|---|---|---|
| M1 | Inventur und Abgleichmatrix | `docs/oberflaechen/einstellungen.md`, dieser Plan | Dokument nennt je Einstellung jede Oberfläche mit Beleg und Ergebnis |
| M2 | `se background`: Hintergrund-Sync, Prüfintervall, Pause, automatische Pause, Status, Protokoll wie Desktop-Einstellungen „Integration → Hintergrund“ und Android `bg.*`; MCP-`status` zeigt dieselben Werte | `native/src/cli/background.rs`, `native/src/cli/mod.rs`, `native/src/cli/mcp/status_tools.rs` | `luecken_task_background_*`: Dauerangaben, Grenzen 2–3600 s, JSON-Schlüssel, Klap-Parser |
| M3 | `se sync`: Sync-Setups auflisten, anzeigen, anlegen, bearbeiten (jedes Feld des Desktop-Editors), entfernen, aktivieren, Protokoll lesen und Detailprotokoll schalten, über den gemeinsamen `JobEditor` | `native/src/cli/sync_jobs.rs`, `native/src/cli/sync_jobs/{options,output}.rs`, `native/src/syncjobs/core/describe.rs`, `native/src/syncjobs/mod.rs`, `native/src/cli/mod.rs` | `luecken_task_sync_cli_*`: jedes Feld landet im Job, Werte der Auswahlfelder = gespeicherte Werte, Fehltexte wie GUI |
| M4 | Desktop-Sync-Editor auf Unterseiten (Allgemein, Zeitplan, Vergleich & Versionen, Filter, Übertragung & Befehle) mit den acht fehlenden Feldern (Höchstwartezeit, Abfrageintervall, Kontroll-Lauf, Vollprüfung des Ziels, Mindestzahl Löschschutz, Versionsort, eingehängte Dateisysteme, Befehl bei Abbruch) | `native/src/app/core/job_editor_ui.rs`, `native/src/app/core/job_editor_pages.rs`, `native/src/app/core/mod.rs` | `luecken_task_job_editor_pages_cover_every_field` (jedes `JobEditor`-Feld ist genau einer Seite zugeordnet) |
| M5 | Android: fehlende Sync-Felder (Zeit-Toleranz, Größen- und Altersfilter, Bandbreite, parallele Übertragungen, sichere Kopien, Überprüfen, Wiederholungen, Pause) und Prüfintervall des Hintergrunddienstes; Bridge und Vertrag | `native/src/mobile/os/shared/domains/{job_json,background,mod}.rs`, `android/.../api/SyncApi.kt`, `android/.../ui/sync/{JobDraft,JobEditorScreen}.kt`, `android/.../ui/settings/BackgroundSettings.kt`, `docs/superpowers/plans/2026-09-25-android-apk/api.md` | `luecken_task_job_json_*` (Hin- und Rückweg jedes neuen Felds), `luecken_task_bg_set_cadence_*` |
| M6 | Verbindungen: CLI `add smb`, `edit`, `remove` mit derselben Bereinigung wie die GUI, `list` mit Anmeldeart; Desktop: SMB anbieten, UNC nur unter Windows, Verbindung bearbeiten, SSH-Hostschlüssel vergessen; ein gemeinsamer Speicherweg für bearbeitete Verbindungen (CLI, Desktop, Android) | `native/src/connect/os/shared/connection_edit.rs`, `native/src/connect/mod.rs`, `native/src/cli/connections.rs`, `native/src/cli/connections/{edit,output}.rs`, `native/src/cli/setup.rs`, `native/src/app/core/{dialogs,connection_edit_ui,sidebar,menus_sync,state,init}.rs`, `native/src/app/os/{linux_os,windows/platform}.rs`, `native/src/mobile/os/shared/domains/connections.rs` | `luecken_task_connection_edit_*` (Geheimnis wandert mit, Art-Wechsel löscht es, alte Zeile entfernt), `luecken_task_cli_connections_*` |
| M7 | CLI `se connections gdrive status/configure/sign-in/sign-out` und `se integration context-menu` (Windows) | `native/src/cli/gdrive.rs`, `native/src/cli/integration.rs`, `native/src/cli/connections.rs`, `native/src/cli/mod.rs` | `luecken_task_gdrive_cli_*`, `luecken_task_integration_cli_*` |
| M8 | Desktop: KI-Agenten-Plugin (Status, Installieren, Entfernen je Host) in „Einstellungen → Integration“, ausgeführt über das installierte `se` | `native/src/app/core/settings_ai_plugin.rs`, `native/src/app/core/settings_ui.rs`, `native/src/updater/mod.rs`, `native/src/updater/os/{linux_os,windows}.rs`, `native/src/app/os/{linux_os,windows/platform}.rs` | `luecken_task_ai_plugin_panel_parses_se_reports` |
| M9 | Updates: `se update --check` nennt die Auto-Update-Pause nach einem Rollback wie die GUI | `native/src/cli/update.rs` | `luecken_task_update_report_names_pinned_version` |
| M10 | MCP: `se`-Werkzeug nennt die neuen Befehle; `status` mit Hintergrundwerten | `native/src/cli/mcp/catalog.rs`, `native/src/cli/mcp/status_tools.rs` | bestehender `mcp_task_catalog_is_complete_and_within_host_limits` plus `luecken_task_mcp_status_has_background` |

Erhalten bleiben: gespeicherte Formate (`connections`-Zeilen, `sync/jobs/*.conf`, Steuerdateien des Workers, `ui_state`, `app_prefs`), alle bisherigen Befehle und Optionen, alle bestehenden GUI-Wege. Neu sind nur zusätzliche Befehle, Felder und Seiten.

## C — Umsetzung und Ergebnis (Arbeitsbaum 2026-10-16, nicht gebaut)

Ergebnis je Einstellung mit Belegen: `docs/oberflaechen/einstellungen.md`, Abschnitt „Stand nach dem Stapel“. Abweichungen vom Plan in B:

- M2 bis M10 umgesetzt wie geplant; zusätzlich (aus der Inventur): Google Drive als Zeile in `se connections list` und im MCP-Werkzeug `connections` (Ziel `gdrive:///`), Desktop-Schalter „Beim Start nach Updates suchen“ mit `se update --startup-check` (Android hatte „Beim Start prüfen“, der Desktop prüfte immer), Desktop-Dialog wählt bei Protokollwechsel den Standardport, wenn der alte Standardport unverändert war (vorher blieb z. B. 22 bei FTP stehen).
- Der Verbindungsdialog zog aus `app/core/dialogs.rs` in `app/core/connect_dialog_ui.rs` (dialogs.rs hätte sonst 500 Zeilen überschritten); die Tests von `cli/mod.rs` zogen nach `cli/parse_tests.rs` (gleiche Testnamen unter `cli::tests`), die Tests von `cli/sync_jobs/options.rs` nach `options_tests.rs`.
- Beim Bearbeiten berührte Dateien mit älterem nicht-rustfmt-Stand (`app/core/sidebar.rs`, `menus_sync.rs`, `settings_ui.rs`, `settings_updates.rs`) sind jetzt rustfmt-sauber (nur Formatierung).
- Die Vorschläge des Programmierers 1 für `cli/mcp/catalog.rs` und den Android-Vertrag §5 sind eingearbeitet, weil beide Dateien in diesem Teil ohnehin geändert wurden.

Bewusst nicht umgesetzt (Begründung in der Inventur): Sync-Aktionen im Terminal (Jetzt ausführen, Vergleichen, Versionen, Konflikte, Sicherheitsstopp), Darstellungseinstellungen der GUIs in der CLI, Update-Rollback im Terminal, Muster „Bei Anschluss“ auf Android.

Neue Tests (Präfix `luecken_task_`, 29; Linux 29, Windows 26 — die drei Bridge-Tests übersetzt nur Linux):
`cli::background` (3), `cli::sync_jobs` (2), `cli::sync_jobs::options` (6, Datei `options_tests.rs`), `cli::sync_jobs::output` (2), `cli::connections::gdrive` (2), `cli::connections::edit` (1), `cli::integration` (1), `cli::mcp::status_tools` (1), `cli::update` (1), `connect::connection_edit` (3), `app::job_editor_pages_tests` (2), `app::connection_edit_ui` (1), `app::settings_ai_plugin` (1), `mobile::…::domains::settings_parity_tests` (3, nur Linux); Android-JVM `JobDraftTest.filterAndTransferSettingsOfTheDesktopEditorRoundTrip`. Ende-zu-Ende-Stufe `settings` als Vorschlag für `native/test-luecken-task.py` im Übergabeordner (`vorschlaege/suite-ergaenzung.md`).
