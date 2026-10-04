# CI-4 E-ENGINE

Gebunden an Run 37162485159 / Kandidat `ac3b0c9098963fae386e94558f2f3ca1bb240740`; dessen Formatpatch war vor dem Edit als `74a2b67d` integriert. Frischer eigener Planabschnitt und freigegebene Refs gelesen; Umsetzung ausschließlich gemäß `scopes/ci-4-e-engine.json`. Keine lokale Ausführung, Git-, Graph- oder CI-Aktion. Root integriert und verwendet denselben vollständigen Remote-Suiteeintritt.

## Findings, Umsetzung und Abnahme

| Beleg / Grenze | Änderung / Entscheidung | Signal der vorhandenen Suite |
| --- | --- | --- |
| Pending-Konvergenz: gefoldete `FILE`-Keys gelangten in Empfänger mit `fold_case=false`. | `SyncOmissions` bewahrt alle Literalpfade, auch stille OwnFile-Auslassungen, und bildet bei abweichender Schlüsselregel neue Keys daraus. Bei gleicher Regel bleibt die bisherige erste Gruppenbegründung erhalten. | `engine_provider_pending_protects_owners_orientation_and_single_actions`: keine Pending-Konvergenz/Basisänderung; Owner-, Replika-, Reverse- und Single-action-Schutz bleiben. |
| Safety: `omission_reason` klassifiziert auch gewöhnliches PermissionDenied/NotFound als Read-Auslassung; Apply verschluckte dadurch fehlgeschlagene Writes/Backups. | Sink-Auslassung schützt weiterhin die alte Basis; Unreadable/Vanished/NotRepresentable zählen als tatsächlicher Fehler. Klassifikation läuft immer; stiller geschützter Ausgang verlangt zusätzlich `AttemptError::before_commit()`. Retryphasen bleiben unverändert. | `failed_apply_paths_stay_out_of_new_baseline_and_retry`, `backup_failure_blocks_overwrite_and_delete`, `stat_failure_blocks_reversible_overwrite`, `keep_both_copy_failure_blocks_resolution_and_recovers`: Fehler sichtbar; unabhängiger Erfolg erhalten; keine Mutation nach fehlgeschlagenem Backup; Retry ersetzt nur bestätigte Basis. |
| Partieller Linkscan invalidiert den optionalen Index mit `forget_pair`. | Fixture fordert zuerst und nach vollständiger Recovery `bootstrapped=true`; nach Partialscan ist nur eine fehlende oder untrusted Zeile zulässig. Dateien und ursprüngliche geschützte Basis bleiben streng geprüft. Keine Cache-/Fallback-/Owneränderung. | `sync_links_task_nested_link_preserves_counterparts_baseline_and_incremental_recovery`. |
| NoOp-Fixture erwartete noch den entfernten Source-Rewalk nach Copy. | Erstlauf listet einmal pro Seite und beweist echte Zielbytes, Copy und persistierte Basis; zweiter NoOp listet weiterhin einmal und führt keine Dateiaktion aus. | `no_op_run_skips_rewalk`. |
| Fast-Hash-Fixture verwendete LocalBackend als Client-inner; AgentBackend delegiert `is_local`. | Nur der vorhandene Regular-Agent-Fall nutzt eine Remote-Clientlocation, deren lokaler I/O-Fallback Unsupported meldet. Framed Transport muss den Baum liefern. Hidden=false verlangt weiterhin Metadata; Hidden=true plus vorhandener .hidden-Glob behält alle Assertions. Agent/Daemon-Sync und Altpeer behalten den bisherigen lokalen Unterbau. | `sync_links_task_regular_agent_tree_keeps_fast_hash_path_and_filters`, bestehende Agent/Daemon-/Altpeer-Leaves. |
| Im Run antwortete Windows-Agent mit Namespace=false; Android-Local PerFileOnly hatte keine passende Namespacebestätigung. | Nach gespeicherter API/Ref ruft lokales `apply_stage::namespace` den echten `vfs::confirm_namespace(backend,parent)`-Hook auf. Defaultfalse bleibt unbestätigt. Die beiden PRE-Publish-Now-Fallbacks verlangen weiterhin tatsächliches `sync_filesystem=true` für Contents. `replacement_publish` liefert local+Deferred bereits false und bleibt unverändert. Counting, WriteFail/StatFail, FakeRemote und Transfer-Faultwrapper delegieren Namespacebestätigung an ihr echtes inner-Backend. | Agent/Daemon-Kopie und Android Saved-Sync benötigen eine echte Adapterbestätigung; unbestätigte Veröffentlichung bleibt ohne Erfolgsbasis. Namespace bestätigt keine ungeflushten Stagecontents oder Deferred-Stages. |
| Kandidatzeile links.rs:164 las die fehlende `independent.txt`, nicht den Baselinepfad. Windows Update/daily/LostACK belegen den Apply-Aufruf noch nicht. | Unmittelbare Status-/Omission-/Deferred-/State-Diagnose in denselben eigenen Leaves; Linkfall beweist initiale Persistenz und unveränderten StateKey. `SideSnapshot::is_empty` enthält bereits den Schutz für User-Omissions; der frühe Empty-Verdacht wurde verworfen. Keine geratenen Planner-, Empty- oder Owneränderungen. | `sync_links_task_incremental_target_junction_returns_to_full_protected_scan`, `review_task_corrupt_optional_index_does_not_block_completed_file_work`, `review_task_daily_target_verification_finds_untracked_mirror_orphans`, `engine_provider_recorded_lost_ack_preserves_old_baseline_until_retry`; alle ursprünglichen Byte-/Promo-/Retryassertions bleiben. |

## Integration und offene Grenzen

Eigene Quellenintegration vollständig; keine weitere notwendige API-/Scopeanfrage. V-NAMESPACE meldet seine OS-Produzenten gespeichert; die Local-Delegation wurde frisch gelesen. Root bestätigt die gespeicherten Produktforwarder für Agent (bestehende echte Query/Fallbackkette), Cache, Guard mit Writeprüfung und IPC-Unavailable. Hier wurden ausschließlich die zugewiesenen Fixtureforwarder geändert.

Windows private Versions-Access5 und Held-Rename Error87 bleiben bei ihren bestehenden Ownern. Fehlender unabhängiger Copy-/Update-/daily-/LostACK-Fortschritt wird anhand derselben Suite mit der konkreten Statusdiagnose beurteilt; der vorhandene Log belegt dafür keine weitere mutierende Ursache. Kein unbekannter FS-/Agentpfad wird als durable markiert. Keine Remote-Abnahme ausgeführt oder als erfolgreich behauptet.

Öffentliche Engine-APIs, literal/provider identity, PairLock/Owner/Replika, Backup/CAS, Omissions, Cancellation, Lost-ACK-Intent/Stage und Deferred-Checkpoint bleiben erhalten. Keine Modulregistrierung, kein neues Rustfile und kein neuer Leaf.

## Statischer Self-Review

Eigene Änderungen gegen die frisch freigegebenen Definitionen gelesen. Alle bisherigen Leaf-Symbole bleiben erhalten; keine Schutz- oder Byteassertion entfällt. Phasenbeleg: `AttemptError::before_commit`, geschützter Sink-Ausgang vor Commit, tatsächlicher Fehler danach; `replacement_publish.rs:116–119` bestätigt local+Deferred weiterhin nicht. Filecontents-Flush und Post-Publish-Parentbestätigung bleiben getrennt.

Statische Größe der geänderten Rustdateien: 280–454 Zeilen, maximal 17.3 KiB; neue Blöcke haben auch nach realistischer Formatierung Reserve unter 500 Zeilen/50 KiB. Keine Formatter-/Compiler-/Testausführung.

Einige Leseausgaben enthielten zusätzliche Kontextzeilen über die engen Definitionsgrenzen hinaus: `apply_reporting.rs:75–245` Run-Preflight/Folders, `error_classes.rs:1–34` Target-refusal, `guards.rs:100–108` unconfirmed sowie `state_metadata.rs:45` load_dirs-Signatur. Root informiert; keine Zusatzänderungen daraus. Dokumentations-Kontextprüfung ohne Git gemäß ausdrücklichem Arbeitsverbot, mit aktueller Source und gezieltem `rg`.

## Exaktes Dateiinventar

Gelesen (Definitionen/Ausschnitte; zusätzliche Kontextzeilen oben dokumentiert):

- `/tmp/rv1-ci-fourth/e-engine.json`
- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/post-publication-namespace.md`
- `docs/refs/private-file-access.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fourth-fixes.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-4-e-engine.json`
- `native/src/agent/core/backend.rs`
- `native/src/agent/core/extensions.rs`
- `native/src/agent/core/walk.rs`
- `native/src/agent_proto/core/features.rs`
- `native/src/agent_proto/os/linux_os/local_platform.rs`
- `native/src/agent_proto/os/shared/ext_ops.rs`
- `native/src/agent_proto/os/windows/local_platform.rs`
- `native/src/app/os/shared/sync_paths_task_tests.rs`
- `native/src/bisync/core/guards.rs`
- `native/src/bisync/core/keys.rs`
- `native/src/bisync/core/omissions.rs`
- `native/src/bisync/core/run_types.rs`
- `native/src/bisync/core/snapshot_types.rs`
- `native/src/bisync/core/types.rs`
- `native/src/bisync/os/shared/apply_actions.rs`
- `native/src/bisync/os/shared/apply_boundary.rs`
- `native/src/bisync/os/shared/apply_guard.rs`
- `native/src/bisync/os/shared/apply_reporting.rs`
- `native/src/bisync/os/shared/apply_retry.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/os/shared/apply_transaction.rs`
- `native/src/bisync/os/shared/apply_transfer_tests.rs`
- `native/src/bisync/os/shared/checkpoint_journal.rs`
- `native/src/bisync/os/shared/checkpoint_review_tests.rs`
- `native/src/bisync/os/shared/checkpoint_run.rs`
- `native/src/bisync/os/shared/engine_identity_task_tests.rs`
- `native/src/bisync/os/shared/engine_provider_fixture.rs`
- `native/src/bisync/os/shared/engine_provider_task_tests.rs`
- `native/src/bisync/os/shared/incremental.rs`
- `native/src/bisync/os/shared/incremental_collect.rs`
- `native/src/bisync/os/shared/incremental_index_commit.rs`
- `native/src/bisync/os/shared/orchestration.rs`
- `native/src/bisync/os/shared/orchestration_full.rs`
- `native/src/bisync/os/shared/orchestration_plan.rs`
- `native/src/bisync/os/shared/persistence.rs`
- `native/src/bisync/os/shared/replacement_publish.rs`
- `native/src/bisync/os/shared/replica.rs`
- `native/src/bisync/os/shared/replica_state.rs`
- `native/src/bisync/os/shared/snapshot_agent.rs`
- `native/src/bisync/os/shared/snapshot_pair.rs`
- `native/src/bisync/os/shared/state_metadata.rs`
- `native/src/bisync/os/shared/test_remote.rs`
- `native/src/bisync/os/shared/tests/hash_walk.rs`
- `native/src/bisync/os/shared/tests/links.rs`
- `native/src/bisync/os/shared/tests/links_remote.rs`
- `native/src/bisync/os/shared/tests/safety.rs`
- `native/src/bisync/os/shared/tests/safety_backends.rs`
- `native/src/support_dirs.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/error_classes.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/extension_types.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/fs_profile.rs`
- `native/src/vfs/core/promotion.rs`
- `native/src/vfs/mod.rs`
- `native/src/vfs/os/linux_os/local_platform.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/shared/local_stage.rs`
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/vfs/os/windows/local_writes.rs`

Zusätzlich wurde der selbst erstellte Bericht zum Abschluss gelesen.

Geändert:

- `native/src/bisync/core/omissions.rs`
- `native/src/bisync/os/shared/apply_reporting.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/os/shared/apply_transfer_tests.rs`
- `native/src/bisync/os/shared/checkpoint_review_tests.rs`
- `native/src/bisync/os/shared/engine_identity_task_tests.rs`
- `native/src/bisync/os/shared/test_remote.rs`
- `native/src/bisync/os/shared/tests/hash_walk.rs`
- `native/src/bisync/os/shared/tests/links.rs`
- `native/src/bisync/os/shared/tests/links_remote.rs`
- `native/src/bisync/os/shared/tests/safety_backends.rs`

Erstellt:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-E-ENGINE.md`
