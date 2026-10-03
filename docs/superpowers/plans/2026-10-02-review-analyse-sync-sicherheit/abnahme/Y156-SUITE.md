# Y156-SUITE – verzögerte Desktop-Worker

Stand: 2026-10-03. Ausschließlich Abnahme der vorhandenen Y156-Produktquellen;
kein neuer Produktreview. Scope: `../scopes/y156-suite.json`. Root-Go liegt vor.
Keine lokale Ausführung; alle Laufbeweise gehören zur gemeinsamen RV1-Remote-Suite.

## Stage one und konkrete API-Klärung

Der vorhandene Gate-Frame verarbeitet die tatsächlich registrierten JoinHandles.
`CancelClose` hält ein Close-/Update-Intent, solange `desktop_sync_active()` wahr ist.
Cancel und die 10s-UI-Frist dürfen keinen laufenden Handle entfernen; Shutdown
verweigert aktive Worker vor Session-Cleanup und Receiver-Abbau.
`frame_update` pumpt Ergebnisse vor dem Gate. Die Fixture ruft denselben
Einzelaktionsdrain vor dem echten Gate auf, ohne fremde Hintergrunddienste zu starten.

Die versionsgebundenen egui-Primärquellen liefern `RawInput.viewports`,
`ViewportEvent::Close`, `Context::run` und tatsächliche `ViewportOutput.commands`.
Die vorhandene isolierte App-Fassade ist `App::new_for_copy_task()`;
RV1 muss `SMART_EXPLORER_COPY_PASTE_TASK=1` und isoliertes AppData bereitstellen.

Zweite API-Klärung: `StagedUpdate` ist über seine tatsächliche serde-Form erzeugbar.
`verify_staged_update` prüft zuerst das Manifest-Schema. Eine Fixture mit `schema=0`
und fehlenden Payloads kann sichtbar scheitern, bevor Recovery-/Shutdown-Preflight
oder ein realer Updatehelfer starten. Kein produktiver Konstruktor wird ergänzt.

## Stage two – Fixtures und erwartete Signale

Neue kohäsive Datei `native/src/app/os/shared/sync_exit_gate_task_tests.rs`;
einzige vorhandene Änderung: cfg(test)-Kindmodulregistrierung in `sync_exit_gate.rs`.

| Implementiertes Testsymbol unter `app::sync_exit_gate::task_tests` | Erwartung |
| --- | --- |
| `review_task_y156_close_waits_for_completion_after_cancel_and_deadline` | Echter Close-Input ergibt CancelClose. Cancel und rückdatierte 10s-Frist lassen den per Channel gehaltenen Worker aktiv; Shutdown bleibt gesperrt. Erst reale Completion erlaubt Close. |
| `review_task_y156_results_drain_while_worker_is_still_alive` | PreviewApplyResult wird abgeholt, ursprüngliche Fehleraktion bleibt erhalten. Leere Receiver und zurückgesetzte UI-Laufflags geben den noch lebenden JoinHandle nicht frei. |
| `review_task_y156_update_preflight_waits_for_actual_completion` | Aktiver Worker hält Update ohne Ready-Verifikation. Nach Cancel/Frist und tatsächlicher Completion meldet der echte Ready-Preflight den Schemafehler, behält den Kandidaten und startet keinen Helfer. |

Worker startet und bestätigt seine Bereitschaft, publiziert erst auf explizites
Signal und bleibt anschließend bis zu einem separaten Release gehalten. Ein
begrenzter Rettungs-Timeout sowie Drop-Cleanup verhindern verwaiste Fixtureworker.
Die Fixture bestätigt tatsächliche Completion über die bestehenden Tracking-APIs;
keine erfolgreiche Ergebnisnachricht ersetzt den JoinHandle-Nachweis.
Die UI-Frist wird im cfg(test)-Kindmodul rückdatiert, ohne 10s Schlaf oder Produkt-Hook.

## Abschluss

Quellseitig implementiert, noch kein Laufbeweis. RV1 entdeckt die drei vollständigen
Testsymbole unter `app::sync_exit_gate::task_tests::review_task_y156_*` und führt sie
im vorhandenen isolierten Desktop-Profil mit `--include-ignored --test-threads=1`
auf Linux und nativem Windows aus. Es gibt keinen eigenen Einstieg oder Buildlauf.

Entscheidungen: Der Abbruch nutzt die vorhandene gemeinsame Cancel-API; die
10s-Grenze wird ausschließlich im Testkindmodul rückdatiert. Tatsächliche egui-
Close-Inputs und Output-Kommandos werden geprüft. Die Fixture simuliert keinen
fertigen Worker durch Laufflags oder Ergebnisnachrichten. Sie benutzt den vorhandenen
PreviewApplyResult-Drain in derselben Reihenfolge vor dem echten Gate, ohne den
gesamten Hintergrundframe mit Netzwerk-/Dienststarts auszuführen. Native Fenster-
oder Mausklickautomation wird hier nicht behauptet.

Der ungültige Ready-Kandidat entsteht nur im Fixture-Speicher; seine fehlenden
Payloads liegen unter einem privaten temporären Testordner. Das vorhandene erste
Schema-Check muss die konkrete Fehlermeldung liefern. Kein Apply-Helfer wird direkt
aufgerufen, kein Staging-Manifest oder produktiver Constructor geändert.

Eigener Self-Review: zugewiesene APIs und egui-Primärquellen gelesen; nur eigene
Fixture-/Registrierungstexte geprüft. Lexikalische Klammer-/String-/Kommentarprüfung,
Whitespace, Symbol-/Registrierungs- und Größenprüfung unauffällig. Neue Rustquelle
272 Zeilen/10376 Bytes; Gate 135 Zeilen/5779 Bytes einschließlich Registrierung.
Kein Compiler-/Formatter-/Test- oder Fixturelauf, kein Git/CI/Graph/Release oder Agent.

Offen: ausschließlich die gemeinsame Remote-Laufabnahme und dort bereitgestellte
isolierte Umgebung; keine ungelöste Produkt- oder API-Anfrage. Der freigegebene
Pfad `native/src/app/core/mod.rs` existiert nicht; keine Ersatzfläche erkundet.

## Exaktes Dateiinventar

Gelesen (Quellen teilweise gezielt, eigene neue Dateien eingeschlossen):

- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui-0.29.1/src/context.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui-0.29.1/src/data/input.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui-0.29.1/src/viewport.rs`
- `docs/refs/egui-close-lifecycle.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/Y156-SUITE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/y156-suite.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/suite-plan.md`
- `native/src/app/core/bisync_merge.rs`
- `native/src/app/core/frame_update.rs`
- `native/src/app/core/init.rs`
- `native/src/app/core/preview_core.rs`
- `native/src/app/core/shutdown.rs`
- `native/src/app/core/state.rs`
- `native/src/app/core/sync_preview_types.rs`
- `native/src/app/core/sync_run_state.rs`
- `native/src/app/mod.rs`
- `native/src/app/os/shared/sync_exit_gate.rs`
- `native/src/app/os/shared/sync_exit_gate_task_tests.rs`
- `native/src/app/os/shared/sync_paths_task_tests.rs`
- `native/src/updater/core/types.rs`
- `native/src/updater/os/shared/staging.rs`

Geändert:

- `native/src/app/os/shared/sync_exit_gate.rs`: ausschließlich additive
  `#[cfg(test)]`-Kindmodulregistrierung `task_tests` mit explizitem Dateipfad.

Erstellt:

- `native/src/app/os/shared/sync_exit_gate_task_tests.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/Y156-SUITE.md`
