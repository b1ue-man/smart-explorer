# E-APPLY – offene Anschlussanfragen

Stand: 2026-10-03. E-APPLY ist als kohärenter Implementierungsblock abgeschlossen; keine zusätzliche Review-/Scope-Ausweitung und keine lokale Ausführung.

## E-ENGINE: offene Mergepfade atomar schützen

Nach eigener PairLock und StateKey-Prüfung bisync::pending_merge_relatives(&lock, &key)? lesen. Rückgabe enthält logische Original- und KeepBoth-Siblingpfade dieses Owners/Replikapaars. Auf beiden Seiten vor normaler Planung, Basis-Konvergenz und komplettem Index-Seeding schützen/deferieren; gespeicherte tatsächliche Seitenschreibweisen weiter verwenden. Recovery-Lesefehler dürfen nicht als leere Pending-Liste gelten.

Readonly-Consumer verwenden pending_merge_for_key(a, root_a, b, root_b, &key, rel). PendingMerge liefert ursprüngliche Conflict-Sigs/-Bytes und Choice für denselben Retry. Aktuelle Mergebytes sind keine neuen Originaldrafts. Android erhielt den Vertrag über Root; fehlende In-Memory-Sessions können den persisted Auftrag wieder aufnehmen.

resolve_recorded ist hier atomar angeschlossen: unter derselben gehaltenen Lock nach validate_state blockiert ein normalisierter Original-/Siblingtreffer mit WouldBlock. Retry bleibt merge_recorded_for_key. Reguläre Full-/Incremental-Läufe und Preview/single_recorded-Schritte übernimmt E-ENGINE. Zugeordnete vorhandene Grenzen: native/src/bisync/os/shared/orchestration_full.rs, incremental.rs, incremental_collect.rs, single_recorded.rs und die Journal-/Indexgrenze aus E-PLAN. Fehlende Engine-Abschirmung ist bis zum Anschluss funktionaler Integrationsbedarf; kein ungeprüfter Konvergenzerfolg.

## E-ENGINE / V-REMOTE: verbliebene Provider-Anschlüsse

- Literalpfade: Eigene Snapshot-/Apply-/Versions-/Merge-/Quick-Pfade verwenden sync_child_path/sync_path. native/src/bisync/os/shared/duplicate_plan.rs verwendet noch metadata/observe mit alten join-Pfaden; auf metadata_named/observe_named/verify_named und explizite Literalnamen umstellen. General Resolve und restliche Index-/Journal-/Kontroll-Consumer gehören Root; keine breite Änderung hier.
- Drive Y132: Providerumstellung auf drive_account_key ist von Root gemeldet. Alte tokenbasierte Paar-/Basis-IDs einmalig unter Sperre migrieren, Backend/Parent/Root/Owner/Replika unverändert bewahren. E-APPLY erfindet keine Alias-Identitäten.
- Accountfeed Y134: Paginierte Accountsignale sind keine komplette Rootbeobachtung. Erst belegte Ancestry und Remove-ID-/Dupegruppenbeobachtung im Engine-Index erlauben Rootaktionen/Index. Zugeordnete Grenzen incremental_collect.rs, incremental_changes.rs und E-PLAN-Persistenz/Index.
- Replacement Y140/Y142: V1 replace_staged_reversible mit dauerhaften Ersatz-Intent und Recovery im Engine-/Journalvertrag verbinden. Hier bewahrt fehlender atomarer Replace Originale/Backups und ergibt Unsupported; kein unsicherer remove-then-rename-Fallback. Auto-Archival plus create-only Publish ist implementiert.
- Namen Y124: Eigene Pfade konsumieren target_limits und Literalhook. Alte kodierte gespeicherte Locators bleiben erhalten; keine pauschalen Windows-Regeln für Linux/Android/Drive.

## V1 abgeschlossen und Observer-Grant

Kein zusätzlicher Windows-Durability-Hook: native/src/vfs/core/fs_profile.rs meldet PerFile, native/src/vfs/os/shared/local_extensions.rs bestätigt nach file flush/write-through über sync_filesystem true. Echte PerFileOnly bleiben false; kein bestätigter neuer Namespace.

native/src/bisync/os/shared/test_remote.rs wurde eng für sichere Stage/ExclusiveCopy/Regular-Read-/Extensions-Forwarder geändert. native/src/bisync/os/shared/duplicate_observation.rs wurde nur gemäß Grant für Regular-Read, Special-/Link-Prüfung und expliziten Literalnamen verändert. Eigene Apply-/Resolve-Consumer nutzen das; verbleibender Plan-Consumer ist oben zugeordnet.

## Zuständigkeit

Der enge Y156-Anschluss ist fertig: sync::SyncHandle::take_worker(&mut self) -> Option<JoinHandle<()>> liefert einmalig den echten Worker; D-SYNCUI begrenzt das Wait, Root prüft übrige Structliteral-Caller. Cancel/SpawnFailure-Done/Drop-detach bleiben erhalten.

Die eigene Staged-Drop-Logik bewahrt nach versuchter Publikation mit Err/verlorenem ACK mögliche Recovery-Evidenz. E-ENGINE/H-REPLACE verbindet den dauerhaften Replacement-Intent und dessen Recovery; keine ungesicherte Stage-Entfernung oder automatische Konvergenz aus unklarer Publikation.

Root führt die eine finale Remote-Suite aus; konkrete Signale stehen in abnahme/E-APPLY.md. Keine lokalen Builds/Tests/Formatter/Server/Installationen, Graphänderungen, Commits/Pushes oder CI/Release ausgeführt. Keine weiteren E-APPLY-Freigaben nötig. Genannte Owner-Verträge sind vor vollständiger Integrationsabnahme anzuschließen.

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
