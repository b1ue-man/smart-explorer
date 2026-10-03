# E-APPLY – API-Delta

Stand: 2026-10-03. Implementierung im Arbeitsbaum; statisch geprüft, keine lokale Ausführung. Dieser Bericht ergänzt den freigegebenen E-PLAN-Vertrag.

## Recorded Merge und autorisierte Originalpfade

`bisync::recorded_original_paths_for_key(a, root_a, b, root_b, key: &StateKey, rel: &str) -> io::Result<RecordedPaths>` hält die Paarsperre durch StateKey-Prüfung, gespeicherte Seitenschreibweisen, Schutzgrenzen und die vorhandene VFS-Literalauflösung. `RecordedPaths { rel_a, rel_b, path_a, path_b: String }`. Der Consumer liest jede Seite über ihren ursprünglichen Backend-Endpunkt. Die API schreibt nichts.

`bisync::merge_recorded_for_key(a, root_a, b, root_b, key, conflict, original_a, original_b, choice, cancel, progress) -> Result<MergeReport, MergeFailure>`:

- `OriginalContent<'a> { signature: Option<Sig>, bytes: Option<&'a [u8]> }`
- `MergeChoice<'a> { Write(&'a [u8]), KeepBoth { keep_a: bool } }`
- `MergeReport { a, b: Option<Sig>, confirmed_a, confirmed_b: bool, baseline: Baseline, preserved: Vec<MergeFile> }`
- `MergeFile { rel: String, a, b: Option<Sig> }`
- `MergeFailure { error: io::Error, partial: MergeReport }`

Die Sperre liegt vor jedem Originalvergleich. Tatsächliche Originalbytes werden erneut vor Backups geprüft. Beide Seiten werden vorbereitet und reversibel gesichert, bevor Originals publiziert werden. Nur bestätigte Writes stehen als bestätigt im Ergebnis. Die ursprüngliche Konfliktbasis bleibt bei Teilfehlern erhalten. Private dauerhaft geschriebene Input-/Recovery-Dateien halten Originaltexte/-Sigs, Presence, Digests, Choice und bestätigte Teilergebnisse für denselben Retry. Ein fehlendes Original kann create-only aus passender Version oder geprüften privaten Originalinputs wiederhergestellt werden. KeepBoth verwendet und persistiert die tatsächlichen Elternschreibweisen beider Seiten.

## Wiederaufnahme nach Prozessneustart

`bisync::pending_merge_for_key(a, root_a, b, root_b, key: &StateKey, rel: &str) -> io::Result<Option<PendingMerge>>` hält die Paarsperre, validiert Endpunkte/Repliken/Owner und liest ausschließlich private, durable Inputs der vorhandenen Recovery. Der Consumer kann exakt denselben Auftrag wiederholen:

- `PendingMerge { conflict: Conflict, original_a, original_b: Option<Vec<u8>>, merged: Vec<u8>, choice: RecordedMergeChoice, confirmed_a, confirmed_b: Option<Sig> }`
- `RecordedMergeChoice { Write, KeepBoth { keep_a: bool } }`
- Für `Write`: `MergeChoice::Write(&pending.merged)`; OriginalContent verwendet die gespeicherten Conflict-Sigs und Originalbytes. Für KeepBoth bleibt dieselbe Gewinnerseite erhalten.
- Aktuell publizierte Mergebytes werden nicht als ursprünglicher Draft ausgegeben. Exakte Digests, Presence, Länge, StateKey und ursprüngliche Signaturen werden aus dem privaten Inputdatensatz validiert.
- Der eigentliche Wiederholungsauftrag prüft die aktuellen Originale oder bereits bestätigten Mergebytes nochmals unter seiner durchgehend gehaltenen Sperre.

`bisync::pending_merge_relatives(lock: &PairLock, key: &StateKey) -> io::Result<Vec<String>>` ist der schmale E-ENGINE-Anschluss. Der reguläre Lauf hält bereits diese Sperre und hat StateKey validiert. Rückgabe umfasst offene Originalpfade und gegebenenfalls KeepBoth-Siblingpfade, getrennt nach Owner/Repliken. **E-ENGINE muss diese Pfade auf beiden Seiten vor Planung, komplettem Indexaufbau und Konvergenz-Checkpoint schützen/deferieren.** Dieser Consumer liegt außerhalb des E-APPLY-Scopes. `resolve_recorded` ist bereits atomar angeschlossen: nach eigener Lock und validate_state blockiert ein normalisierter Treffer auf Original oder KeepBoth-Sibling mit WouldBlock. Preview/single_recorded und reguläre Engine-Läufe übernimmt E-ENGINE.

## Planner/Apply, Snapshot, Versionen und Quick Mirror

Die freigegebenen E-PLAN-Signaturen bleiben erhalten: `apply_planned_reporting`, `apply_dedupe_reporting` mit der gesamten Kandidatenliste, `PairSnapshot` mit vollständigen `SideSnapshot`s und `walk_snapshot_with_options` mit bisherigem Walk-Vertrag plus `BisyncOptions`. Alte Signaturen bleiben kompatible Hüllen. `duplicate_apply::resolve_scoped` verwendet `ApplyScope`; alte `resolve`-Hülle bleibt erhalten.

Quick Mirror hält dieselbe PairLock über Transfers, eigene Laufversionen und Retention. `SyncOptions`, `SyncStats` und `SyncResult` bleiben kompatibel. Die eigenen internen scoped Copy-/Delete-Pfade nutzen dieselben exklusiven Stage-/Backup-/Durability-Grenzen.

Versionen implementieren die vorhandenen `RunVersions`, List-/Restore-/Prune-/Remove-APIs. Neue Datensätze sind immutable und vor Destruktion durable; Auto archiviert am Ziel, mit privatem Appdatafallback bei fehlender Fähigkeit/Berechtigung. Version-Retention gruppiert pro Originaldatei/Seite/Replika. Altdaten bleiben lesbar.

Windows nutzt die bestehende tatsächliche Bestätigung von `vfs::sync_filesystem` nach LocalBackend-Publikation (PerFile-Profil mit file flush/write-through). Echte PerFileOnly-Backends bestätigen damit keinen neuen Namespace. Deferred wird nur beim später tatsächlich flushenden Reporting-Vertrag angeboten; kompatible Collecting-Hüllen schließen unmittelbar ab.

## Exakter Duplicate-Observer-Grant

`native/src/bisync/os/shared/duplicate_observation.rs` ergänzt `metadata_named`, `observe_named`, `verify_named` mit explizitem Literalnamen; Regular-Read und Special-/Link-Prüfungen erhalten Identitäts-/Content-Revalidierung. Eigene Apply-/Resolve-Consumer verwenden diese Beobachtungen. Restliche Plan-/Index-/Journal-Aufrufer gehören zum E-ENGINE-Folgeauftrag.

## Owner-Grenzen

E-ENGINE verbindet offene Mergepfade, noch alte Duplicate-Resolve-Aufrufer und restliche Literal-/DriveIdentity-/Accountfeed-Verträge. Reversible Replacement-Intent/Recovery für Provider ohne atomaren Replace bleibt dort; E-APPLY erhält vorhandene Originale/Backups und meldet Unsupported sicher. Keine pauschale Durability-Annahme und kein unsafe Stage-Fallback.

## Quick-Mirror-Worker-Completion (Y156)

`sync::SyncHandle::take_worker(&mut self) -> Option<std::thread::JoinHandle<()>>` überträgt einmalig den in start_sync erzeugten echten Worker. `cancel` bleibt unverändert öffentlich; fehlgeschlagenes Spawn liefert weiterhin Done mit Fehler und keinen Worker. Ein ungenommener JoinHandle wird beim Drop wie bisher detached. D-SYNCUI kann nach Cancel begrenzt auf is_finished prüfen und anschließend joinen; das bounded Wait bleibt Consumer-Aufgabe. Keine andere SyncOptions/Stats/Result-Signatur wurde verändert. Structliteral-Consumerprüfung gehört zum Root.

## Publikationsfehler und Stage-Eigentum

Nach einem versuchten Publish kann ein Fehler bzw. verlorener ACK einen schon wirksamen Schritt verdecken. Eigene Staged-Drop-Logik erhält diese Stage als Recovery-Evidenz; sie verwirft nur noch nicht zur Publikation versuchte eigene Stages. Eine definitive AlreadyExists-Siblingkollision bleibt ohne Mutation und kann den sicheren Kollisionsretry nutzen. Der dauerhafte Replacement-Intent/Recovery-Anschluss bleibt E-ENGINE/H-REPLACE; ein Err wird weder als erfolgreiche Basis noch als sichere Destruktionsfreigabe behandelt.

## Exakte Datei-Inventare

Gelesen einschließlich gezielter Textsuchen und statischer Eigenprüfung der eigenen neuen Dateien:

- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/sync-remote-metadata.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/E-APPLY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/E-APPLY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-APPLY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-PLAN.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/V-LOCAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/integration.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/e-apply.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `native/Cargo.toml`
- `native/src/bisync/core/baseline_records.rs`
- `native/src/bisync/core/completion.rs`
- `native/src/bisync/core/keys.rs`
- `native/src/bisync/core/limits.rs`
- `native/src/bisync/core/omissions.rs`
- `native/src/bisync/core/paths.rs`
- `native/src/bisync/core/run_types.rs`
- `native/src/bisync/core/types.rs`
- `native/src/bisync/mod.rs`
- `native/src/bisync/os/shared/apply.rs`
- `native/src/bisync/os/shared/apply_actions.rs`
- `native/src/bisync/os/shared/apply_boundary.rs`
- `native/src/bisync/os/shared/apply_dedupe.rs`
- `native/src/bisync/os/shared/apply_delete.rs`
- `native/src/bisync/os/shared/apply_dirs.rs`
- `native/src/bisync/os/shared/apply_guard.rs`
- `native/src/bisync/os/shared/apply_mirror.rs`
- `native/src/bisync/os/shared/apply_pool.rs`
- `native/src/bisync/os/shared/apply_pool_tests.rs`
- `native/src/bisync/os/shared/apply_reporting.rs`
- `native/src/bisync/os/shared/apply_retry.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/os/shared/apply_transaction.rs`
- `native/src/bisync/os/shared/apply_transfer.rs`
- `native/src/bisync/os/shared/apply_transfer_tests.rs`
- `native/src/bisync/os/shared/checkpoint.rs`
- `native/src/bisync/os/shared/duplicate_apply.rs`
- `native/src/bisync/os/shared/duplicate_backup.rs`
- `native/src/bisync/os/shared/duplicate_observation.rs`
- `native/src/bisync/os/shared/duplicate_plan.rs`
- `native/src/bisync/os/shared/incremental.rs`
- `native/src/bisync/os/shared/incremental_changes.rs`
- `native/src/bisync/os/shared/incremental_collect.rs`
- `native/src/bisync/os/shared/merge_execution.rs`
- `native/src/bisync/os/shared/merge_inputs.rs`
- `native/src/bisync/os/shared/merge_keep_both.rs`
- `native/src/bisync/os/shared/merge_precheck.rs`
- `native/src/bisync/os/shared/merge_recorded.rs`
- `native/src/bisync/os/shared/merge_recorded_task_tests.rs`
- `native/src/bisync/os/shared/merge_recovery.rs`
- `native/src/bisync/os/shared/merge_resume.rs`
- `native/src/bisync/os/shared/merge_task_fixture.rs`
- `native/src/bisync/os/shared/move_finalize.rs`
- `native/src/bisync/os/shared/orchestration_full.rs`
- `native/src/bisync/os/shared/pair_lock.rs`
- `native/src/bisync/os/shared/persistence.rs`
- `native/src/bisync/os/shared/recorded_paths.rs`
- `native/src/bisync/os/shared/replica.rs`
- `native/src/bisync/os/shared/replica_state.rs`
- `native/src/bisync/os/shared/replica_state_tests.rs`
- `native/src/bisync/os/shared/resolve.rs`
- `native/src/bisync/os/shared/resolve_tests.rs`
- `native/src/bisync/os/shared/single_recorded.rs`
- `native/src/bisync/os/shared/snapshot.rs`
- `native/src/bisync/os/shared/snapshot_agent.rs`
- `native/src/bisync/os/shared/snapshot_dir.rs`
- `native/src/bisync/os/shared/snapshot_duplicates.rs`
- `native/src/bisync/os/shared/snapshot_hash.rs`
- `native/src/bisync/os/shared/snapshot_mounts.rs`
- `native/src/bisync/os/shared/snapshot_pair.rs`
- `native/src/bisync/os/shared/snapshot_pair_tests.rs`
- `native/src/bisync/os/shared/snapshot_policy.rs`
- `native/src/bisync/os/shared/snapshot_walk.rs`
- `native/src/bisync/os/shared/snapshot_walk_tests.rs`
- `native/src/bisync/os/shared/state_spellings.rs`
- `native/src/bisync/os/shared/state_types.rs`
- `native/src/bisync/os/shared/state_validation.rs`
- `native/src/bisync/os/shared/test_remote.rs`
- `native/src/bisync/os/shared/tests.rs`
- `native/src/bisync/os/shared/transfer_stream.rs`
- `native/src/bisync/os/shared/version_listing.rs`
- `native/src/bisync/os/shared/version_manifest.rs`
- `native/src/bisync/os/shared/version_ops.rs`
- `native/src/bisync/os/shared/version_restore.rs`
- `native/src/bisync/os/shared/version_retention.rs`
- `native/src/bisync/os/shared/version_save.rs`
- `native/src/bisync/os/shared/versions.rs`
- `native/src/mobile/os/shared/domains/sync_merge.rs`
- `native/src/support_dirs.rs`
- `native/src/sync/mod.rs`
- `native/src/sync/os/shared/sync.rs`
- `native/src/sync/os/shared/sync_compare.rs`
- `native/src/sync/os/shared/sync_copy.rs`
- `native/src/sync/os/shared/sync_delete.rs`
- `native/src/sync/os/shared/sync_delete_walk.rs`
- `native/src/sync/os/shared/sync_link_tests.rs`
- `native/src/sync/os/shared/sync_parallel_tests.rs`
- `native/src/sync/os/shared/sync_pass.rs`
- `native/src/sync/os/shared/sync_pass_compat.rs`
- `native/src/sync/os/shared/sync_pass_start.rs`
- `native/src/sync/os/shared/sync_queue_tests.rs`
- `native/src/sync/os/shared/sync_robustness_tests.rs`
- `native/src/sync/os/shared/sync_run.rs`
- `native/src/sync/os/shared/sync_scan.rs`
- `native/src/sync/os/shared/sync_tasks.rs`
- `native/src/sync/os/shared/sync_tests.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/error_classes.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/extension_types.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/fs_profile.rs`
- `native/src/vfs/core/meta.rs`
- `native/src/vfs/mod.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/shared/local_stage.rs`

Geändert, bereits vorhandene Dateien:

- `native/src/bisync/mod.rs`
- `native/src/bisync/os/shared/apply.rs`
- `native/src/bisync/os/shared/apply_delete.rs`
- `native/src/bisync/os/shared/apply_guard.rs`
- `native/src/bisync/os/shared/apply_pool.rs`
- `native/src/bisync/os/shared/apply_transfer.rs`
- `native/src/bisync/os/shared/apply_transfer_tests.rs`
- `native/src/bisync/os/shared/duplicate_apply.rs`
- `native/src/bisync/os/shared/duplicate_backup.rs`
- `native/src/bisync/os/shared/duplicate_observation.rs`
- `native/src/bisync/os/shared/move_finalize.rs`
- `native/src/bisync/os/shared/resolve.rs`
- `native/src/bisync/os/shared/snapshot.rs`
- `native/src/bisync/os/shared/snapshot_agent.rs`
- `native/src/bisync/os/shared/snapshot_dir.rs`
- `native/src/bisync/os/shared/snapshot_duplicates.rs`
- `native/src/bisync/os/shared/snapshot_hash.rs`
- `native/src/bisync/os/shared/snapshot_pair.rs`
- `native/src/bisync/os/shared/snapshot_walk.rs`
- `native/src/bisync/os/shared/test_remote.rs`
- `native/src/bisync/os/shared/versions.rs`
- `native/src/sync/mod.rs`
- `native/src/sync/os/shared/sync.rs`
- `native/src/sync/os/shared/sync_copy.rs`
- `native/src/sync/os/shared/sync_delete.rs`
- `native/src/sync/os/shared/sync_pass.rs`
- `native/src/sync/os/shared/sync_scan.rs`
- `native/src/sync/os/shared/sync_tasks.rs`

Erstellt:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/E-APPLY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/E-APPLY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-APPLY.md`
- `native/src/bisync/os/shared/apply_actions.rs`
- `native/src/bisync/os/shared/apply_boundary.rs`
- `native/src/bisync/os/shared/apply_dedupe.rs`
- `native/src/bisync/os/shared/apply_dirs.rs`
- `native/src/bisync/os/shared/apply_mirror.rs`
- `native/src/bisync/os/shared/apply_reporting.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/os/shared/apply_transaction.rs`
- `native/src/bisync/os/shared/merge_execution.rs`
- `native/src/bisync/os/shared/merge_inputs.rs`
- `native/src/bisync/os/shared/merge_keep_both.rs`
- `native/src/bisync/os/shared/merge_precheck.rs`
- `native/src/bisync/os/shared/merge_recorded.rs`
- `native/src/bisync/os/shared/merge_recorded_task_tests.rs`
- `native/src/bisync/os/shared/merge_recovery.rs`
- `native/src/bisync/os/shared/merge_resume.rs`
- `native/src/bisync/os/shared/merge_task_fixture.rs`
- `native/src/bisync/os/shared/recorded_paths.rs`
- `native/src/bisync/os/shared/snapshot_mounts.rs`
- `native/src/bisync/os/shared/snapshot_policy.rs`
- `native/src/bisync/os/shared/transfer_stream.rs`
- `native/src/bisync/os/shared/version_listing.rs`
- `native/src/bisync/os/shared/version_manifest.rs`
- `native/src/bisync/os/shared/version_ops.rs`
- `native/src/bisync/os/shared/version_restore.rs`
- `native/src/bisync/os/shared/version_retention.rs`
- `native/src/bisync/os/shared/version_save.rs`
- `native/src/sync/os/shared/sync_compare.rs`
- `native/src/sync/os/shared/sync_delete_walk.rs`
- `native/src/sync/os/shared/sync_pass_compat.rs`
- `native/src/sync/os/shared/sync_pass_start.rs`
- `native/src/sync/os/shared/sync_run.rs`

`native/src/bisync/mod.rs` und `native/src/sync/mod.rs` erhalten ausschließlich eigene additive Modul-/Reexport-Einträge. Alle neuen Rust-Dateien liegen kohäsiv neben zugeordneten Dateien. Das Manifest wurde gelesen und vom Root gepflegt; E-APPLY hat es nicht geändert. Fremde Änderungen im gemeinsamen Arbeitsbaum gehören nicht zu diesem Inventar.
