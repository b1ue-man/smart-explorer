# CI-5 E-ENGINE

Gebundene Evidenz: RV1-Run `37167542206` auf `1378bc8fdb796ac2102ccb1e62e8fb74fe0bd796`; Formatterpatch vor Bearbeitung als `36eaafe9` übernommen. Umfang gemäß `scopes/ci-5-e-engine.json` samt Roots vorab gespeicherten Definitions-/Consumerfreigaben. Keine neue Review oder lokale Ausführung. Dieser Bericht dokumentiert den eigenen statischen Anschluss; Laufzeitabnahme erfolgt ausschließlich durch Roots bestehende vollständige RV1-Remote-Suite.

## Befund → Änderung → vorhandenes Abnahmesignal

| Belegter Fehler | Quellenänderung und Entscheidung | Unverändertes Prüfsignal |
| --- | --- | --- |
| Linux/Windows: `SideEmpty { side: B, previous: 1 }` trotz `folder: Link` | `plan_pair` übernimmt beide Side-Omissions mit `mem::take`. `orchestration_plan::prepare` erhält vor beiden Rückgaben die finalen Plan-Auslassungen wieder auf beiden beobachteten Snapshots. Der spätere Empty-Guard und die Checkpointbeobachtung sehen weiterhin den geschützten Eintrag. Planner/Empty-Guard selbst unverändert. | `sync_links_task_incremental_target_junction_returns_to_full_protected_scan`: kein falscher Stopp; dieselbe StateKey/Basis für den ausgelassenen Pfad; Außenbytes unverändert; unabhängige Datei wird tatsächlich kopiert. |
| Windows: private Recoveryentfernung `NotFound`; Copy-Drift erhält vorher `NotADirectory` | Ein nativer `PathBuf.to_str()` wurde an den ausschließlich nach `/` trennenden Backend-Parentparser übergeben. Der neue interne `apply_stage::native_namespace(&Path)` verwendet `Path::parent` und den bestehenden `vfs::confirm_namespace` direkt. Vier eigene native Aufrufer verwenden ihn: Merge-Recovery, Merge-Inputs, Replacement-Journal und privates Copybackup. | `destination_drift_after_backup_blocks_promotion`: weiterhin `InvalidData` und unveränderte Concurrent-Destinationbytes. `engine_provider_account_identity_preserves_state_locks_inputs_and_versions`: State/Lock/Originalinputs/Versions bleiben gebunden und tatsächliche Recoveryentfernung gelingt. |
| Windows: beide Merge-Seiten bestätigt, danach Abschlussfehler; Replacement-Retry/Cleanup fehlgeschlagen | Die gleichen nativen Parentaufrufe schließen die tatsächlich fehlgeschlagenen privaten Abschlussstellen an. Lösch-, Rechte-, Namespace- und Flushfehler bleiben Fehler; weder Stage-/Providerpublikation noch Rollback/Versionssemantik verändert. | `review_task_merge_partial_publication_keeps_conflict_basis_and_retries`, `review_task_merge_keep_both_preserves_loser_on_both_sides`, `engine_provider_recorded_lost_ack_preserves_old_baseline_until_retry`, `engine_provider_publication_and_lost_ack_use_exactly_one_contract`. Teilbasis, Originalinputs, genau einmal erfolgte Publikation, Fremdcreator und Backups bleiben vollständig geprüft. |
| Android: `confirmedA=true`, `confirmedB=true`, aber `baselineRecorded=false` nach erfolgreichem Engine-Return | `RecordBook::record` speichert den Literalnamen als Baseline-Mapkey; `set_keys` ändert nur den separaten Namenindex. Der Mobile-Consumer verglich dagegen einen gefalteten Planningkey direkt mit dem Literal-Mapkey. `sync_merge::merge` sucht jetzt genau eine gespeicherte Zeile mit demselben Planningkey und verlangt deren exaktes bestätigtes Signaturpaar. Beide Confirmed-Flags und ein vollständiger erfolgreicher Engine-Return bleiben Pflicht. | Bestehende Gerätefälle `recordedMergeRetrySurvivesProcessRestart` und `localConflictsResolveMergeKeepBothAndSkip`: tatsächlicher `done`-/`baselineRecorded`-Abschluss, Original-/KeepBothbytes und sicherer Retry wie bisher. |

Vor Edit wurden Plan, gebundene Logs, lokale Primärrefs sowie die fehlenden Definitionen frisch gelesen. Der konkrete Plan war: beobachtete Auslassungen erhalten; native Parentbestätigung von Remote-Literalparsern getrennt anschließen; vollständigen Mobile-Baselinelookup erst nach nachgewiesener Keysemantik korrigieren. Keine neue Fixturedatei oder neues Szenario. Alle bisherigen Leaf-Symbole und Assertions bleiben erhalten. Lediglich zwei vorhandene Assertions melden ihren tatsächlichen Fehler genauer: direkter Copy-Drift zeigt den Fehlertext, der Recorded-LostACK-Retry zeigt Errors/Blocked/Stopped/Deferred/Omissions/State.

## Erhaltene Verträge und statischer Self-Review

- Wirklich leere Beobachtungen mit ausschließlich `OwnFile` bleiben leer; frühere Volume-/History-/Identitäts- und Bestätigungsgates sind unverändert. Gefilterte/protected Beobachtungen können keine Löschung oder vollständigen Incremental-Bootstrap vortäuschen.
- Die finalen Omissions bleiben auch nach der optionalen Hashverifikation auf beiden Snapshots; Planaktionen und Baseline-Auslassungsschutz bleiben derselbe endgültige Plan.
- Nur private native `&Path`-Aufrufer verwenden den neuen Helper. Remote-/Literalnamen, UNC/native Pfade, Endpoint-/Accountidentität, Replica-/Ownerbindung und gespeicherte Baselines werden nicht umkodiert oder umetikettiert.
- `native_namespace` delegiert den tatsächlichen nativen Parent an den vorhandenen OS-/VFS-Vertrag. `false` und I/O-Fehler bleiben unbestätigt. Fileflush vor privatem Backupabschluss, Source-Revalidierung, Now-Contentsflush und Deferred-Whole-filesystem-Flush sind unverändert.
- Merge-PairLock, Originalbytevergleich, Versionsbackup, Cancel, Teilergebnisse und sichere Wiederanlaufinputs bleiben unverändert. Das Ergebnis verlangt beide bestätigten Seiten, genau eine passende gespeicherte Literalzeile und exakt dieselben Signaturen. Ein Fehler oder eine mehrdeutige/fehlende Basis bleibt ein Fehler. Erst nach vollständig gespeicherter Basis bleibt das private Inputcleanup wie bisher best effort; dessen Fehler widerruft diesen bestätigten Stand nicht.
- Replacement NoReplace und `Hook(false) -> promote_staged_replace` behalten dieselbe einzelne Mutation, LostACK-Evidenz, Stage/Intent/Backups und Fremdcreator-CAS. Kein neuer Publish-/Retryfallback.
- Eigene geänderte Definitionen und ihre direkten Aufrufer wurden statisch gegengelesen. Scopezuordnung und Textgrößen wurden geprüft; kein Compiler, Test, Formatter, Server, Git, CI, Graph oder Release ausgeführt.

Neue interne Signatur: `apply_stage::native_namespace(path: &std::path::Path) -> std::io::Result<bool>`, Sichtbarkeit `pub(super)`. Keine öffentliche API-/Wire-/Persistenzformatänderung und keine Registrierung. `MergeReport.baseline` bleibt die tatsächlich gespeicherte Baseline mit Literalnamen. `sync_merge_recovery.rs` bleibt unverändert: es besitzt keinen vollständigen Baselinelookup und verwendet weiterhin denselben Merge-Consumer.

## Gelesene Dateien

Nur die freigegebenen Definitionen/Zeilen innerhalb begrenzter Dateien; kein Lesen außerhalb des Manifests. Native/VFS-/Private-/App-/Mobile-Reads dienen ausschließlich den oben benannten Aufrufer-, Parent- und Ergebnisgrenzen.

- `/tmp/rv1-ci-fifth/device/integration-localConflictsResolveMergeKeepBothAndSkip.log`
- `/tmp/rv1-ci-fifth/device/merge-retry.log`
- `/tmp/rv1-ci-fifth/linux/native-suite.log`
- `/tmp/rv1-ci-fifth/windows/native-suite.log`
- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/post-publication-namespace.md`
- `docs/refs/private-file-access.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-5-E-ENGINE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fifth-fixes.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-5-e-engine.json`
- `native/src/app/os/shared/sync_paths_task_fixture.rs`
- `native/src/app/os/shared/sync_paths_task_tests.rs`
- `native/src/bisync/core/baseline_records.rs`
- `native/src/bisync/core/guards.rs`
- `native/src/bisync/core/keys.rs`
- `native/src/bisync/core/omissions.rs`
- `native/src/bisync/core/paths.rs`
- `native/src/bisync/core/plan_filter.rs`
- `native/src/bisync/core/plan_pair.rs`
- `native/src/bisync/core/plan_types.rs`
- `native/src/bisync/core/snapshot_types.rs`
- `native/src/bisync/os/shared/apply_guard.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/os/shared/apply_transfer.rs`
- `native/src/bisync/os/shared/apply_transfer_tests.rs`
- `native/src/bisync/os/shared/checkpoint_journal.rs`
- `native/src/bisync/os/shared/engine_identity_task_tests.rs`
- `native/src/bisync/os/shared/merge_execution.rs`
- `native/src/bisync/os/shared/merge_inputs.rs`
- `native/src/bisync/os/shared/merge_recorded.rs`
- `native/src/bisync/os/shared/merge_recorded_task_tests.rs`
- `native/src/bisync/os/shared/merge_recovery.rs`
- `native/src/bisync/os/shared/merge_resume.rs`
- `native/src/bisync/os/shared/orchestration.rs`
- `native/src/bisync/os/shared/orchestration_full.rs`
- `native/src/bisync/os/shared/orchestration_plan.rs`
- `native/src/bisync/os/shared/replacement_journal.rs`
- `native/src/bisync/os/shared/replacement_publish.rs`
- `native/src/bisync/os/shared/replacement_recovery.rs`
- `native/src/bisync/os/shared/replica_state.rs`
- `native/src/bisync/os/shared/snapshot_pair.rs`
- `native/src/bisync/os/shared/state_spellings.rs`
- `native/src/bisync/os/shared/tests/links.rs`
- `native/src/bisync/os/shared/version_save.rs`
- `native/src/mobile/os/shared/domains/sync_merge.rs`
- `native/src/mobile/os/shared/domains/sync_merge_recovery.rs`
- `native/src/support_dirs.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/os/linux_os/namespace_flush.rs`
- `native/src/vfs/os/shared/local.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/windows/local_platform.rs`

Begrenzte Definitionsreads: `plan_pair`/`protect_directory_collisions`; `apply_pair_filter` samt Snapshotmutation; `PairPlan`/`PlanContext`-Felder; `RecordBook`-Felder/`record`/`set_keys`; `orchestration::pair_key_policy`; lokale Pfadkonversion/`new`/`is_local`/`extensions`; Windows/Linux `confirm_namespace`; private Read-/Write-/Remove-APIs; App-Failing-Leaves/Backendpaar-/nativer Rootaufbau; Mobile-Merge-/Retry-/Resultgrenze. `links.rs` wurde am bestehenden Incremental-Linkfall und dessen direkten Assertions gelesen, die übrigen Quellen entsprechend der verbundenen freigegebenen Enginefläche.

## Geänderte und erstellte Dateien

| Geändert | Zeilen | Bytes |
| --- | ---: | ---: |
| `native/src/bisync/os/shared/orchestration_plan.rs` | 290 | 9865 |
| `native/src/bisync/os/shared/apply_stage.rs` | 397 | 12593 |
| `native/src/bisync/os/shared/merge_recovery.rs` | 230 | 8506 |
| `native/src/bisync/os/shared/merge_inputs.rs` | 190 | 6611 |
| `native/src/bisync/os/shared/replacement_journal.rs` | 276 | 9892 |
| `native/src/bisync/os/shared/apply_transfer.rs` | 376 | 12020 |
| `native/src/mobile/os/shared/domains/sync_merge.rs` | 376 | 14084 |
| `native/src/bisync/os/shared/apply_transfer_tests.rs` | 361 | 10946 |
| `native/src/bisync/os/shared/engine_identity_task_tests.rs` | 463 | 17926 |

Aktuelle Textgrößen; neue Zeilen wurden mit normaler Mehrzeilenformatierung geschrieben. Größte geänderte Quelle: 463 Zeilen, unter 18 KiB. Kein lokaler Formatter wurde ausgeführt.

Erstellt und anschließend selbst gegengelesen: `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-5-E-ENGINE.md`. Keine Rustdatei erstellt. Keine weiteren Dateien geändert.

## Offene Grenzen

Keine eigene ausstehende Produkt-/API- oder Scopeabhängigkeit. Laufzeitverhalten ist noch nicht bestätigt; ausschließlich Roots gleicher vollständiger RV1-Remote-Einstieg liefert diesen Nachweis. Share-Stage-Ownership und die fehlende Windows-AllBackendPairs-Stable-Diagnose gehören gemäß Root an A-CLIENT; die jeweilige unveränderte Assertion bleibt maßgeblich. S-REVOKE und SavedJob-Zeitvertrag liegen bei ihren bereits zugeordneten Ownern. Keine Berechtigung für zusätzliche Plattform-, Provider-, Suite-, Graph- oder Releasearbeit.
