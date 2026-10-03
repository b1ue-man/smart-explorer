# D-SYNCUI – Umsetzung und Abnahme

Stand: 2026-10-03. Ausschließlich der bestehende dokumentierte Desktop-Anschluss
FS3/FS9/FS10/FS12, Y147/Y148/Y156/Y19. Kein neues Review. Maßgeblich ist
`scopes/d-syncui.json`; H-TRASH-WINDOWS bleibt unverändert.

## Stage one und zweite Vertragsrecherche vor Änderungen

Die vorhandenen Desktop-Quellen zeigen noch Legacy-Konfigurationszeitstempel,
paarweite Run-/Baseline-Aufrufe und unaufgezeichnete direkte Merge-/Einzeldatei-Writes.
Die bereits implementierten Engine-/JobState-Verträge sind die autoritative Grenze:
`RunSettings::for_job`, Outcome.state, `classify_run/record_attempt/confirm_block`,
`resolve_recorded`, `recorded_original_paths_for_key`,
`merge_recorded_for_key/pending_merge_for_key` und Versions-List/Restore.

Die zweite konkrete API-Lesung schließt die Definitionen von BisyncCtx/MergeUi,
RunRequest/Outcome, Preview.planned/state und den Restart-Inputdatensatz ein.
Der Hauptagent liefert die additive `preview_with(..., RunSettings)`-API für echte
Jobowner-Vorschauen; die UI etikettiert keinen gefundenen Key um.
`linemerge` bietet den bestehenden Myers-Diff mit 16 MiB pro Seite, 500000
Gesamtzeilen und 2s Defaultfrist sowie `TextShape` für CRLF/Endnewline.
Originalbytes bleiben separat erhalten; KeepBoth rekonstruiert keine Bytes aus Rows.
Nicht-UTF8 oder gemischte Zeilenformen werden für die Textbearbeitung sichtbar abgewiesen.

Es existiert keine öffentliche BackgroundSettings-/GuardianStatus-Struktur. Bedienung
und Zustandsanzeige verwenden die vorhandenen daemon-/autostart-Funktionen.
Ein Heartbeat beweist den Dienst, keinen unabhängigen Guardian-Livenesszustand.
Unsupported-Autopause wird nicht als funktionierender Plattformschalter dargestellt.

## Stage two – konkrete kohäsive Meilensteine

| Ergebnis | Eigene Fläche | Erwartetes gemeinsames Remote-Abnahmesignal |
| --- | --- | --- |
| Persistierte JobState-Anzeige | menus_sync_jobs.rs, landing.rs; neue sync_job_state_ui.rs | Versuch/Erfolg, Fehler/Fehlerserie, Block, Trigger/Retry, Watch und RunMark stammen aus JobState; ein Fehler/Abbruch/BUSY erscheint nicht als neuer Erfolg. Zustand wird begrenzt gecacht. |
| Manuelle Läufe mit korrektem Owner | sync_core.rs, app/os/shared/sync_jobs.rs; eigene kleine Runtypen/Worker | Frische Jobkonfiguration und Endpointresolver, Before vor Verbindung, gemeinsamer RunRequest/StateKey, keep_awake und klassifizierte Attempt-Persistenz; bestätigter Block gilt exakt für einen Lauf. |
| Recorded Konflikt-/Merge-Consumer | bisync_conflicts.rs, bisync_merge.rs, merge_ui.rs; eigene Merge-Typen/Worker | Originalpfade und Bytes werden autorisiert geladen, Diff begrenzt, CRLF/Endnewline erhalten, Engine schreibt reversible/journaled. Teilfehler bleiben derselbe Auftrag mit sichtbaren bestätigten Teilschritten; Restart nutzt private PendingMerge-Inputs. |
| Geplante Einzelaktion | preview_core.rs, bisync_ui.rs | Jobowner-preview_with speichert geplante Signaturen; nur apply_preview_action schreibt/aktualisiert Basis. Veränderte Inputs bleiben in der Vorschau und melden Fehler. |
| Versionen je Job und Restore | menus_sync_jobs.rs; neue sync_versions_ui.rs + kleiner OS-Worker | Sichtbare Liste mit Original, Seite, Grund und Datum; frische Endpoints und PairLock/manifestautorisierter Restore, bestehende Originale gesichert; Fehler/retry/cancel sichtbar. |
| Bestehender Hintergrundzustand | settings_background.rs und JobState-Consumer | Autostart, Heartbeat, Pause, Prüfintervall und unterstützte Akku-/Netzpause werden korrekt gezeigt; keine erfundene Guardian-Aktivmeldung oder Änderung des Dienstworkflows. |
| Eigener statischer Abschluss | drei eigene Berichte | Exakte APIs/Dateien/Abnahmesignale, eigene Diff-/Text-/Parsingprüfung; keine lokale Laufabnahme. |

Kompatibilität: PickerPurpose::SyncSource/SyncTarget und gespeicherte Locatorwerte werden
unverändert durch den gemeinsamen Endpointresolver benutzt; gleiche relative Pfade auf
verschiedenen Backends bleiben verschieden. Lokale/UNC-, SSH/Agent-, FTP/FTPS-, WebDAV-,
Drive- und Direct/Room-Orte werden nicht in lokale Pfade umgewandelt.
Keine Engine-/Protokoll-/CLI-/Android-/Share-/Release-Änderung.
Modelgrant umfasst nur die genannten StateKey-/Merge-/Run-/Versionsdaten und Defaults;
Verhalten liegt in kleinen eigenen Featuredateien, Appmod nur additive Registrierungen.

## Ergebnis und Entscheidungen

Quellseitig abgeschlossen; Laufabnahme durch die einzige abschließende Remote-Suite.

- **FS3/FS9, Y19:** Jobliste und Startseite zeigen gecachten JobState: Versuch/Erfolg,
  Ergebnisdatum, Fehler/Fehlerserie, Sperre, Retry/Trigger, RunMark und zuletzt gemeldete
  Watch-Erkennung. Keine GUI-Writes zu Legacy-last_run/record_result.
- **Manuelle Läufe:** frische Konfiguration und vollständige Locators, gemeinsamer
  Endpointresolver, Before vor Verbindung und gemeinsame After/Cleanup-Hooks.
  RunRequest/RunSettings::for_job und echter Outcome.state; eigener RunMark-Besitz,
  erneuerte Laufmeldung und klassifizierter Attempt vor Ergebnisübergabe. BUSY
  überschreibt keinen fremden Versuch. Vorbereitungs-/Hookfehler behalten FailureKind.
- **Bestätigen:** zweiter Dialog mit Quelle/Ziel/Ursache; confirm_block prüft die
  exakte BlockKind-Frische. Zusätzlich sind die angezeigten Locatorwerte vor
  Hook/Connect gebunden; ein umgestelltes Setup erhält keine alte Bestätigung.
  Unbekannte Stops haben keinen pauschalen Override.
- **FS12/Y147:** Konfliktwahl mit resolve_recorded, Originalpfade über
  recorded_original_paths_for_key. Begrenzte Regular-Reads behalten Originalbytes;
  vorhandene 16-MiB-/500000-Zeilen-/2s-Diffgrenzen, virtuelle Zeilen und begrenzte
  Darstellung. Assembling im Worker; CRLF, Endnewline und einzelne Leerzeilen
  erhalten. Binär-/gemischte Formate bleiben für Textbearbeitung gesperrt;
  KeepBoth verwendet unveränderte rohe Originale.
- **Retry/Restart:** merge_recorded_for_key besitzt Versions-/Journal-/Baseline-Writes.
  Teilfehler halten Originale, exakte Entscheidung und bestätigte Seiten sichtbar.
  PendingMerge lädt nach Neustart dieselbe Originalsession über den echten Job-Key;
  offene geschützte Merges erscheinen wieder im Konfliktresultat. Keine direkten
  write_bytes- oder GUI-Baseline-Fallbacks.
- **FS12/Y148:** jobeigene preview_with und apply_preview_action mit der vollständig
  erhaltenen ursprünglichen Preview (planned/state/options/spellings). Nur Erfolg
  entfernt die Zeile; Fehler gibt denselben Plan zurück. Einzelaktion und Restore
  behaupten keinen erfolgreichen vollständigen Joblauf.
- **Versionen:** per Job sichtbar mit Original, Datum, Größe, Grund, Store und Seite;
  frische Endpoints/Paar-/Eigentümerprüfung, PairLock und manifestautorisierter
  reversibler restore_version. Legacy ohne Seite braucht explizite Zielwahl.
  Erfolgreicher Restore und anschließender Listenfehler werden getrennt angezeigt.
- **FS10/Y156:** keep_awake für eigene Worker/Mirror, gehaltene Worker-Handles und
  geteilte Cancelmarker. Nur beendete Worker werden gejoint. Root integrierte den
  gemeldeten Close/Update-Anschluss mit Abbruchwahl und 10s-Frist; lebende Worker
  halten das Fenster offen. Fremde Exit-Dateien wurden hier nicht gelesen/geändert.
- **Hintergrund/Kompatibilität:** vorhandene Autostart-/Daemon-APIs zeigen Heartbeat,
  Pause, Kadenz und tatsächliche Autopause-Unterstützung. Kein erfundenes Alter 0
  oder Guardian-Liveness. Unsupported Schalter behalten gespeicherte Werte.
  PickerPurpose::SyncSource/SyncTarget, volle Locators und Backendidentitäten für
  lokal/UNC/SSH/Agent/FTP/FTPS/WebDAV/Drive/Direct/Room bleiben erhalten.

Eigener Self-Review: API-Lesung/Parent-Handoffs, Diffprüfung, lexikalische Klammer-/
String-/Kommentarprüfung und Größenkontrolle ohne Compiler. Keine offenen Befunde
dieser statischen Prüfung. Eigene neue/geänderte Rustdateien unter 500 Zeilen/50 KiB;
state.rs aktuell 488 Zeilen einschließlich Root-Gatefeld. Kein Rust-Typcheck/Laufbeweis.

preview_with, PendingMerge, recorded Resolve/Merge/Versions und SyncHandle::take_worker
sind quellseitig geliefert, Root hat den äußeren Close-/Updategate angeschlossen.
Keine ungelöste Fremdanfrage verhindert diesen Consumer; Integration/Remote-Abnahme
bleiben beim Hauptagenten.

## Erwartete Remote-Abnahmesignale und genaue Testnamen

Alle Signale gehören gemeinsam in die eine abschließende Remote-Suite:

| Signal | Erwarteter Nachweis |
| --- | --- |
| D-SYNCUI-FS3 | Fehlendes/falsches Laufwerk blockiert; exakter Stop einmal bestätigt; geänderter Block/Locator zurückgewiesen. |
| D-SYNCUI-FS9-Y19 | Desktop/Daemon teilen RunMark/Job-Key; Fehler/Cancel/BUSY erzeugen keinen neuen Erfolg oder verlorenen Trigger. |
| D-SYNCUI-Y147 | Originale gesichert, concurrent edit abgewiesen, Partial/Cancel/Restart wiederholt dieselbe Originalsession/Entscheidung. |
| D-SYNCUI-Y148 | Planned-State geprüft, Zeile erst nach Erfolg entfernt, Basis durch Engine aufgezeichnet. |
| D-SYNCUI-VERSIONS | Eigene Versionen inkl. AppData sichtbar; fremdes Paar/Seite/Owner abgewiesen; Restore sichert Original und bleibt reversibel. |
| D-SYNCUI-FS10-Y156 | Wake-Holds/Endpointidentitäten erhalten; Close/Update fragt, cancelt/wartet begrenzt ohne lebenden Worker zu töten. |
| D-SYNCUI-BACKGROUND | Pause/Retry/Watch-Fakten und unbekannter Heartbeat ehrlich, Autopause nur unterstützt, keine falsche Guardian-Liveness. |

Neu geschrieben, nicht ausgeführt:

- `app::sync_merge_types::tests::desktop_merge_preserves_crlf_final_separator_and_empty_line`
- `app::sync_merge_types::tests::desktop_merge_excluding_all_lines_does_not_create_newline`
- `app::sync_merge_types::tests::desktop_merge_rejects_lossy_and_mixed_text_without_changing_bytes`
- `app::sync_job_state_ui::tests::desktop_job_state_failed_attempt_does_not_hide_prior_success`
- `app::sync_job_state_ui::tests::desktop_job_state_block_takes_precedence_over_old_success_result`

Erhalten: `app::preview_core::tests::apply_one_removes_action_only_after_success`.
Diese Logiktests ersetzen keine Engine-/Lifecycle-Integrationsbeweise.
Keine lokalen Builds/Tests/rustfmt/Server/CI/Graph/Commits/Pushes/Releases und keine Agenten.

## Exaktes Dateiinventar

Für diesen Block gelesen (eigene neue Quellen und Berichte eingeschlossen):

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/D-SYNCUI.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/D-SYNCUI.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/D-SYNCUI.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-APPLY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/T-JOBS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/d-syncui.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `native/src/app/core/app_models.rs`
- `native/src/app/core/bisync_conflict_ui.rs`
- `native/src/app/core/bisync_conflicts.rs`
- `native/src/app/core/bisync_merge.rs`
- `native/src/app/core/bisync_ui.rs`
- `native/src/app/core/init.rs`
- `native/src/app/core/job_editor.rs`
- `native/src/app/core/job_editor_ui.rs`
- `native/src/app/core/job_editor_validation.rs`
- `native/src/app/core/landing.rs`
- `native/src/app/core/menus_sync_jobs.rs`
- `native/src/app/core/merge_ui.rs`
- `native/src/app/core/prelude.rs`
- `native/src/app/core/preview_core.rs`
- `native/src/app/core/settings_background.rs`
- `native/src/app/core/state.rs`
- `native/src/app/core/support_paths.rs`
- `native/src/app/core/sync_core.rs`
- `native/src/app/core/sync_job_state_ui.rs`
- `native/src/app/core/sync_merge_types.rs`
- `native/src/app/core/sync_preview_types.rs`
- `native/src/app/core/sync_run_state.rs`
- `native/src/app/core/sync_versions_ui.rs`
- `native/src/app/mod.rs`
- `native/src/app/os/shared/remote_helpers.rs`
- `native/src/app/os/shared/sync_jobs.rs`
- `native/src/app/os/shared/sync_manual_run.rs`
- `native/src/app/os/shared/sync_merge_task.rs`
- `native/src/app/os/shared/sync_versions_task.rs`
- `native/src/autostart/mod.rs`
- `native/src/bisync/core/keys.rs`
- `native/src/bisync/core/limits.rs`
- `native/src/bisync/core/plan_types.rs`
- `native/src/bisync/core/run_types.rs`
- `native/src/bisync/core/types.rs`
- `native/src/bisync/mod.rs`
- `native/src/bisync/os/shared/merge_inputs.rs`
- `native/src/bisync/os/shared/merge_recorded.rs`
- `native/src/bisync/os/shared/orchestration.rs`
- `native/src/bisync/os/shared/preview.rs`
- `native/src/bisync/os/shared/recorded_paths.rs`
- `native/src/bisync/os/shared/replica_state.rs`
- `native/src/bisync/os/shared/resolve.rs`
- `native/src/bisync/os/shared/version_restore.rs`
- `native/src/bisync/os/shared/versions.rs`
- `native/src/daemon/mod.rs`
- `native/src/daemon/os/shared/guardian.rs`
- `native/src/daemon/os/shared/hooks.rs`
- `native/src/daemon/os/shared/state.rs`
- `native/src/keep_awake/mod.rs`
- `native/src/linemerge/core/linemerge.rs`
- `native/src/linemerge/core/linemerge_tests.rs`
- `native/src/linemerge/mod.rs`
- `native/src/syncjobs/core/types.rs`
- `native/src/syncjobs/mod.rs`
- `native/src/syncjobs/os/shared/job_state.rs`
- `native/src/syncjobs/os/shared/job_state_classify.rs`
- `native/src/syncjobs/os/shared/job_state_policy.rs`
- `native/src/syncjobs/os/shared/job_state_store.rs`

Geänderte vorhandene Dateien:

- `native/src/app/core/bisync_conflict_ui.rs`
- `native/src/app/core/bisync_conflicts.rs`
- `native/src/app/core/bisync_merge.rs`
- `native/src/app/core/bisync_ui.rs`
- `native/src/app/core/init.rs`
- `native/src/app/core/job_editor_ui.rs`
- `native/src/app/core/landing.rs`
- `native/src/app/core/menus_sync_jobs.rs`
- `native/src/app/core/merge_ui.rs`
- `native/src/app/core/preview_core.rs`
- `native/src/app/core/settings_background.rs`
- `native/src/app/core/state.rs`
- `native/src/app/core/support_paths.rs`
- `native/src/app/core/sync_core.rs`
- `native/src/app/mod.rs`
- `native/src/app/os/shared/remote_helpers.rs`
- `native/src/app/os/shared/sync_jobs.rs`

Erstellte Dateien:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/D-SYNCUI.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/D-SYNCUI.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/D-SYNCUI.md`
- `native/src/app/core/sync_job_state_ui.rs`
- `native/src/app/core/sync_merge_types.rs`
- `native/src/app/core/sync_preview_types.rs`
- `native/src/app/core/sync_run_state.rs`
- `native/src/app/core/sync_versions_ui.rs`
- `native/src/app/os/shared/sync_manual_run.rs`
- `native/src/app/os/shared/sync_merge_task.rs`
- `native/src/app/os/shared/sync_versions_task.rs`

`app/mod.rs` erhielt ausschließlich die acht eigenen additiven Registrierungen in api-delta/D-SYNCUI.md. Die gemeinsam bearbeiteten `state.rs`/`init.rs` enthalten die freigegebenen eigenen Metadaten/Defaults und den vom Hauptagenten integrierten `sync_exit_gate`-Eintrag. Root-eigene Exit-Dateien wurden hier nicht gelesen oder geändert. H-TRASH-WINDOWS bleibt unverändert.
