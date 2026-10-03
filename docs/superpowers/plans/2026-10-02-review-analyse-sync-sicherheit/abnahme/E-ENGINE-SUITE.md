# E-ENGINE-SUITE – Fixture-Übergabe

Stand: 2026-10-03. Produktgrundlage: `4a8130a2`. Auftrag und Dateigrenzen: [e-engine-suite.json](../scopes/e-engine-suite.json), einschließlich der expliziten Helper-/Identity- und `apply_reporting.rs:65–90`-Lesefreigaben. Bestehende Produktfund-Gruppe Y124/Y132/Y134/Y140/Y142; keine Produktänderung und kein neuer Review.

## Anschluss an die gemeinsame Remote-Suite

Pflichtpräfix der Testsymbole: `bisync::engine_provider_task_tests::`; auswählbarer Leaf-Präfix: `engine_provider_`. Exakte Symbole:

- `bisync::engine_provider_task_tests::engine_provider_pending_protects_owners_orientation_and_single_actions`
- `bisync::engine_provider_task_tests::engine_provider_account_feed_proves_ancestry_and_preserves_literal_names`
- `bisync::engine_provider_task_tests::engine_provider_ambiguous_feed_ids_and_changed_parent_require_rebuild`
- `bisync::engine_provider_task_tests::engine_provider_literal_walk_keeps_existing_encoded_root_and_connection`
- `bisync::engine_provider_task_tests::engine_provider_identity_bounds_and_foreign_account_fail_closed`
- `bisync::engine_provider_task_tests::identity_tests::engine_provider_account_identity_preserves_state_locks_inputs_and_versions`
- `bisync::engine_provider_task_tests::identity_tests::engine_provider_recorded_lost_ack_preserves_old_baseline_until_retry`
- `bisync::engine_provider_task_tests::identity_tests::engine_provider_publication_and_lost_ack_use_exactly_one_contract`

Die einzige Suite-Ausführung und ihre Auswertung gehören Root. Lokal wurden weder Tests, Build, Compiler, Linker noch Formatter gestartet. Die folgenden Assertions sind ausführbarer Suite-Code; ihr Laufbeleg steht noch aus.

## Fundzuordnung und konkrete Abnahmesignale

| Bestehende Fundgruppe / Vertrag | Assertion im Fixture |
| --- | --- |
| Y124/Y140/Y142, reversible Veröffentlichung | Der echte `Staged::publish` erreicht den VFS-Hook mit einer bereits gespeicherten, exakt an Stage/Destination/Retained gebundenen Intentdatei. `Ok(false)` erreicht anschließend genau einmal den echten `promote_staged_replace`; der echte NoReplace-Hook erreicht ihn überhaupt nicht. |
| Y140/Y142, verlorene ACKs und Retry | Fehler vor/nach atomic Promote sowie nach Original→Retained und nach Stage→Destination lassen Intent, nötige Stage und unveränderliches Originalbackup erhalten. Recovery bestätigt tatsächliche neue Bytes oder stellt ausschließlich NoReplace zurück; fremde neue Zielbytes bleiben samt Wiederanlaufevidenz erhalten. Weder Publish noch Recovery erzeugen eine Baseline. |
| Y140/Y142, vollständiger recorded Lauf | `run_with_store_path` durchläuft den tatsächlichen Reporting-Preflight mit `bind_lock`. Auto-Archiv-Rename ist im Adapter ausdrücklich unsupported und fällt auf das echte AppData-Backup zurück. Lost-ACK behält die historische Baseline; ein neuer Engine-Lauf bestätigt die tatsächlichen Zielbytes ohne zweiten Promote. Die SQLite-Datei liegt außerhalb beider Syncwurzeln. |
| Paarweiter Pending-Schutz | Andere Owner derselben Replikas, die umgekehrte Paarorientierung und Legacy-AdHoc werden geschützt; andere Replikas und ungültige Owner werden nicht übernommen. Merge-Original und KeepBoth-Sibling verschwinden aus beiden Planungssnapshots und der Convergence. Pending ReplacementIntents schützen auch andere Owner und die Gegenorientierung; Single-Aktionen verweigern unter gehaltenem PairLock. Incremental/Bootstrap verweigern vor Indexeröffnung. |
| Y132, nachgewiesene Account-Identität | Die reale Migration hält alte und neue PairLocks, bewahrt Owner/Replika/History/Seitenschreibweisen/Baseline und übernimmt auch einen gültigen Legacy-Journal ohne Basisdatei. Private Merge-Originaltexte/Choice bleiben unter neuer Pair-/Lock-Bindung lesbar. Alte Versionsmanifeste bleiben bytegleich; List/Restore erreichen sie über den nachgewiesenen Alias. Restore bindet den tatsächlichen AppData-Backupdigest und setzt `checkpoint_allowed=false`. |
| Y132, Accountgrenze und Tokenrotation | Neun alte Identitäten werden vor Import abgelehnt; acht werden von der vorhandenen VFS-Grenze akzeptiert. Ohne Provider-Nachweis wird die fremde Account-Basis nicht importiert und kein Versionsalias hergestellt. Ein neuer Token bei identischer stabiler Account-ID ändert die Pair-ID nicht. |
| Y134, Account-Changefeed | Nur über bekannte Root-/Parent-IDs nachgewiesene Abstammung liefert einen wörtlichen relativen Pfad. Nachgewiesene fremde Roots und unmanaged Remove-IDs werden ignoriert; unbekannte Eltern, bewegte Parent-IDs sowie doppelte Index-/Batch-IDs verlangen Rebuild. |
| Literale Pfade und bestehende Locations | Der echte `sync_path` und `walk_snapshot_with_options` verwenden den Backend-Literalhook, lassen die gespeicherte kodierte Wurzel unverändert und bewahren Prozent-, Leerzeichen-, Hash- und Unicode-Namen. Ein normales `node_modules` wird erfasst. Dieselben relativen Pfade anderer Verbindungen erhalten andere Pair-IDs. |

## Entscheidungen, Self-Review und offene Anschlüsse

Testadapter und frische, eindeutig identifizierte State-Aufräumdaten liegen im privaten Helper. Account-/Versions-/Journal-Publication-Szenarien liegen im privaten Identity-Kindmodul; Feed-/Planungs-/Literal-Szenarien im Haupttestmodul. Damit bleiben alle drei neuen Rustdateien mit Formatierungsreserve unter der Dateigrenze; aktuelle Quellgrößen: Helper 222 Zeilen/9.448 Bytes, Hauptmodul 235 Zeilen/12.961 Bytes, Identity-Modul 248 Zeilen/15.492 Bytes. Kein lokaler Formatterlauf.

Eigener statischer Self-Review: Sichtbarkeiten/Signaturen und additive Registrierung, typisierte Pair-/Owner-/Side-Bindung, Publish-Zähler, Fehler-/Recovery-Zweige, Baseline-Erhalt sowie frische, ausschließlich eigene Cleanup-Pfade geprüft. Der dokumentierte scheinbar fehlende Versions-Lock ist durch den tatsächlich gelesenen Reporting-Preflight aufgelöst; kein Produktpatch. `git diff --check` auf dem eigenen Anschluss ohne Befund.

Die Adapter verwenden reale Engine-/VFS-Aufrufe und lokale NoReplace-/Stage-Operationen des vorhandenen FakeRemote. Stabile Account-ID/Previous-ID-Nachweis und Accountfeed werden kontrolliert eingespeist; dies ist kein Live-OAuth-/Drive-Feed-Nachweis. Der Literaladapter prüft den Engine-Hook und die Parent-Schreibweise, keinen kompletten Providercodec. Der tatsächliche JNI/FTP-Merge-Lost-ACK-/Neustartfall wird von Root/`t_jobs` an die gemeinsame Suite angeschlossen. Offen ist ausschließlich die Remote-Ausführung/Auswertung; keine Produkt-/API-Lücke oder weitere Scope-Anfrage.

## Exaktes Dateiinventar

Neu erstellt:

- `native/src/bisync/os/shared/engine_provider_task_tests.rs`
- `native/src/bisync/os/shared/engine_provider_fixture.rs`
- `native/src/bisync/os/shared/engine_identity_task_tests.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/E-ENGINE-SUITE.md`

Geändert: `native/src/bisync/mod.rs` ausschließlich um die eigene additive `#[cfg(test)] #[path = "os/shared/engine_provider_task_tests.rs"] mod engine_provider_task_tests;`-Registrierung. Private Kindmodulregistrierungen `fixture` und `identity_tests` ausschließlich in der eigenen neuen Testdatei. Keine öffentliche Produktions-API geändert.

Im Fixture-Auftrag tatsächlich gelesen; vorher geladene AGENTS-/Skill-/Architekturvorgaben wurden wiederverwendet:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/E-ENGINE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/e-engine-suite.json`
- `native/src/bisync/core/run_types.rs`
- `native/src/bisync/core/types.rs`
- `native/src/bisync/mod.rs`
- `native/src/bisync/os/shared/apply_reporting.rs` – ausschließlich Zeilen 65–90.
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/os/shared/backend_identity_migration.rs`
- `native/src/bisync/os/shared/backend_identity_state.rs`
- `native/src/bisync/os/shared/checkpoint.rs`
- `native/src/bisync/os/shared/checkpoint_journal.rs`
- `native/src/bisync/os/shared/checkpoint_run.rs`
- `native/src/bisync/os/shared/engine_change_feed.rs`
- `native/src/bisync/os/shared/engine_identity_task_tests.rs`
- `native/src/bisync/os/shared/engine_provider_fixture.rs`
- `native/src/bisync/os/shared/engine_provider_task_tests.rs`
- `native/src/bisync/os/shared/incremental.rs`
- `native/src/bisync/os/shared/merge_inputs.rs`
- `native/src/bisync/os/shared/merge_recovery.rs`
- `native/src/bisync/os/shared/merge_resume.rs`
- `native/src/bisync/os/shared/orchestration.rs`
- `native/src/bisync/os/shared/orchestration_full.rs`
- `native/src/bisync/os/shared/orchestration_plan.rs`
- `native/src/bisync/os/shared/pair_lock.rs`
- `native/src/bisync/os/shared/persistence.rs`
- `native/src/bisync/os/shared/replacement_journal.rs`
- `native/src/bisync/os/shared/replacement_publish.rs`
- `native/src/bisync/os/shared/replacement_recovery.rs`
- `native/src/bisync/os/shared/replica.rs`
- `native/src/bisync/os/shared/replica_state.rs`
- `native/src/bisync/os/shared/single_recorded.rs`
- `native/src/bisync/os/shared/snapshot.rs`
- `native/src/bisync/os/shared/snapshot_pair.rs`
- `native/src/bisync/os/shared/state_metadata.rs`
- `native/src/bisync/os/shared/state_spellings.rs`
- `native/src/bisync/os/shared/state_store.rs`
- `native/src/bisync/os/shared/test_remote.rs`
- `native/src/bisync/os/shared/version_manifest.rs`
- `native/src/bisync/os/shared/version_ops.rs`
- `native/src/bisync/os/shared/version_restore.rs`
- `native/src/bisync/os/shared/version_save.rs`
- `native/src/bisync/os/shared/versions.rs`
- `native/src/gdrive/core/backend.rs`
- `native/src/gdrive/core/extensions.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/promotion.rs`
