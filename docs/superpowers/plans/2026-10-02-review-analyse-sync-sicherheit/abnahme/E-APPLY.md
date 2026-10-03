# E-APPLY – Umsetzung und Abnahme

Stand: 2026-10-03. Der freigegebene verbundene Implementierungsblock ist im Arbeitsbaum abgeschlossen. Laufzeitabnahme bleibt Aufgabe der einen abschließenden Remote-Suite des Root. Keine lokale Ausführung, kein Commit/Push, kein Graph-Neubau und kein neues Review.

## Umsetzungsplan und Fundzuordnung

| Meilenstein | Fläche und bestehende Befunde | Umgesetztes Verhalten |
|---|---|---|
| FS1 Fortschritt behalten | ApplyScope, Reporting, Move-Finalisierung; Y29/Y55 | Jede bestätigte Teilaktion meldet ihre tatsächlichen Signaturen sofort. Fehler, spätes Cancel und fehlgeschlagene Move-Löschung verwerfen zuvor bestätigte Kopien nicht. |
| FS2 Zeiten/Inhalte | gemeinsame Transfer-/Stage-Grenze, Quick Mirror; Y30/Y57/Y89/Y113, Apply-Anteil Y80 | Zeit wird bei Upload/Stage-Finish übergeben; die Ziel-Sig verwendet die tatsächliche Zielzeit. Digest/Bytezahl stammen aus dem kopierten Stream. Kein Source-Reread nach Publikation erzeugt einen Kopiererfolg. |
| FS4 geschützte Beobachtung | Snapshot-Policy, tolerant listing, Mount-Memory; Y34/Y35/Y36/Y37/Y48/Y49/Y54/Y87/Y88/Y145/Y152/Y154 | Links/Junctions, Special, Ownfile, Filter, problematische Namen und unlesbare Kinder schützen Gegenstücke. Kontroll-Walks erhalten BisyncOptions. Unzuordenbare Namen schützen den Elternpfad, an der Wurzel schlägt diese Beobachtung sicher fehl. Normale node_modules bleiben zulässig. |
| FS5 Laufversionen | Manifest/Save/Listing/Retention/Restore/Remove; Y38/Y56/Y61/Y105/Y149, Versionskern Y150 | Auto nutzt Zielarchive je Lauf/Seite/Replika und privaten Appdatafallback. Vor Destruktion existieren dauerhafter Intent und bestätigte Sicherung. Aufbewahrung gruppiert je Originaldatei/Seite/Replika. |
| FS5/FS6 Veröffentlichung | exklusive Stage, Durability, Mode-/Name-/Größenlimits; Y60/Y90/Y96/Y106/Y72/Y74/Y154/Y155 | Stage ist create-only; fsync vor Rename. PerFileOnly bestätigt keinen neuen Namespace; Windows konsumiert vorhandene PerFile/write-through-Bestätigung. Verify prüft Inhalt. |
| FS5 Ordner | DirAction und Quick-Mirror-Delete; Y51/Y70/Y108 | Leere echte Ordner werden erstellt/entfernt. Löschung folgt vollständigem leerem Listing und Identitätsprüfung; eigene Kindlöschungen dürfen Verzeichniszeit ändern. Unklare/linkartige Kinder verhindern Entfernung. |
| FS7 Provider-Kooperation | VFS-Literalpfade, gemessene Uploads, ID-Dedupe, nicht-duplex Verbindung | Backend/Connection bleiben erhalten. Neue Kinder gehen durch sync_child_path/sync_path. Nicht-duplex Transfers und Exporte unbekannter Länge werden gemessen gespult. Dupevarianten werden vor Entfernung gesichert; Survivor bleibt als Copied in der Basis. |
| Quick Mirror | PairLock, Versions-/Stage-/Delete-Grenze | SyncOptions/Stats/Result und Dry-run bleiben kompatibel. Neue Quelle unter alternativer NFC-/Case-Schreibweise verhindert Löschung; Copy-Fehler verhindern Delete-Pass. Target refusal/Verbindungsverlust stoppt neue Aktionen. |
| Recorded Merge | freigegebener Root-Consumerauftrag; Review Y147/Y149 | Durchgehende PairLock vor jeder Textprüfung; beide Originaltexte erneut vor Backups exakt vergleichen. Beide Stages/Backups vor erster Publikation bereit. KeepBoth sichert Verlierer auf beiden Seiten mit tatsächlichen Elternschreibweisen. |
| Merge-Wiederanlauf | private Inputs/Choice/Recovery und resolve_recorded | Neustart erhält originale Texte/Sigs und denselben Auftrag. Basis bleibt bis beide Resultate bestätigt sind. resolve_recorded blockiert offene Original-/Siblingpfade unter derselben gehaltenen Sperre. |
| Worker-Completion | enger Desktop-Consumeranschluss; Y156 | SyncHandle hält den echten Spawn-JoinHandle und liefert ihn einmalig via take_worker. Cancel, SpawnFailure-Done und Drop-detach bleiben erhalten. |

Diese Meilensteine setzen E-PLAN und integration.md fort. Recherche und zweite Lückenklärung verwendeten aktuelle VFS-/Local-Adapterquellen und lokal gesicherte FS-/Remote-Metadatenreferenzen. Windows ist geklärt: windows_profile verwendet PerFile; local sync_filesystem liefert nach per-file Flush/write-through-Publikation true. Ein zusätzlicher unspezifischer Durability-Hook ist nicht nötig.

Der Root-Brief bezeichnet den Recorded-Merge-Consumerauftrag als Y152/Y153. Im vorhandenen Befundkatalog sind Line-Merge/Versionen Y147/Y149; Y152/Y153 betreffen Mounts/Ignore. Diese Übergabe führt die freigegebene Erweiterung aus und nummeriert den Katalog nicht um.

## Entscheidungen und Schutzgrenzen

Remote-Zielarchive verwenden Server-Rename ohne zusätzlichen Payload-Download, wenn native Prüfsumme oder der gewählte Metadatenvergleich das erlaubt. Ein vorhandener geplanter Inhaltsdigest wird vor Rename verifiziert. Exporte mit nicht autoritativer Größe und ID-mehrdeutige Duplikate erhalten private Kopien mit gemessenen Bytes/Digest. Immutable Manifestpfade verwenden kurze Tokens statt roher Fernnamen.

Deferred wird nur angeboten, wenn der Reporting-/Checkpoint-Vertrag später wirklich sync_filesystem == true bestätigt. Alte Collecting-Hüllen schließen unmittelbar ab. PerFileOnly kann einen geschriebenen Zustand unbestätigt lassen; dieser Zustand ist keine erfolgreiche Basisaktion. Remote-V1 nutzt die bestehende Provider-ACK-Semantik.

Versions-/Konfliktsicherung scheitert vor Destruktion oder erhält einen discoverable Intent. Rollback ist create-only in einen weiterhin fehlenden Originalpfad. Stage-Fehler nach möglicher Publikation sind keine harmlosen Precommit-Retrys.

Merge persistiert Presence, Originaltexte/-Sigs, Digests und Choice vor der ersten Mutation privat und dauerhaft. Ein nach Crash fehlendes Original wird aus passender Version create-only zurückgelegt; nötigenfalls stehen auf Digest geprüfte private Originalinputs bereit. Retry berücksichtigt tatsächliche Restore-Zeit. Teilfehler prunen keine dafür benötigten Laufversionen. Nach vollständig bestätigtem Erfolg werden Original-/Siblingschreibweisen vor Recovery-Entfernung gespeichert.

Die bestehende lokale Trash-Adaptergrenze in apply_delete bleibt erhalten. Keine neuen Plattformimporte/cfg-Verzweigungen in core. Quick Mirror behält seine Optionsstruktur und nutzt die geschützte Standardgrenze für untergeordnete Mounts.

## Konkrete Signale für die eine finale Remote-Suite

| Szenario | Überprüfbarer Zustand |
|---|---|
| Bestätigte Kopie, Folgefehler oder spätes Cancel | Erste Zielbytes/tatsächliche Sig bleiben im Checkpoint; Folgeaktion unbestätigt. Kein nachträglicher Source-Reread erzeugt andere erfolgreiche Sig. |
| ENOSPC, readonly target, ConnectionReset/TimedOut | Keine neuen Aktionen; laufende bestätigte Teile bleiben gemeldet, Originale bleiben gesichert. |
| Backup-/Konfliktsibling-Fehler | Keine nachfolgende Destruktion; Originalbytes unverändert oder create-only zurückgelegt; kein falscher Basis-Erfolg. |
| PerFileOnly/Deferred/Windows | File-flush vor Rename. false-Namespace bestätigt keine neue Basis. Deferred erst nach tatsächlichem Rootflush; Windows-Local konsumiert vorhandenes true. |
| Grobe/nicht setzbare Zielzeit | Ausgabe enthält tatsächliche Zielzeit; unveränderter zweiter Mirrorlauf kopiert/versioniert nicht wegen Copy-Zeit. |
| Export / nicht-duplex gleiche FTP-Verbindung | Upload erhält gemessene Länge; Reader/Writer stehen nicht gleichzeitig auf derselben nicht-duplex Session offen; Zielhash entspricht dem Stream. |
| Namen und FAT-Größenlimits | Schutz vor Nutzdatenstream; sichere Primärdatei erhalten; kurze Stage-/Versionsnamen. |
| Link/Junction/FIFO/Special/Ownfile/Filter | Ausgelassen, unabhängiges reguläres File läuft weiter, Gegenstück/alte Basis geschützt. Normale node_modules werden kopiert. |
| Partielle/unzuordenbare Beobachtung und entfernter Mount | Keine Deletion-Inferenz aus unbekanntem Teil; kein kompletter Index; bekannte entfernte Mountgrenze bleibt geschützt. |
| NFC-/Case-Gegenstück und Quick-Delete-Race | Tatsächliche Seitenschreibweisen genutzt; neu auftauchende Quelle unter anderer Schreibweise verhindert Zielentfernung. |
| Version-Retention verschiedener Originale/Repliken | Count/Staggered/GFS wählen je unabhängiger Gruppe; fremde/ersetzte/linkartige Einträge werden nicht entfernt. |
| List/Restore/Remove | Lesbare Job/Zeit/Originalpfad-Metadaten. Restore sichert bisheriges Original und ändert keine Basis. Remove folgt nur bekannten regulären eigenen Dateien/Records. |
| Dedupe gesamte Kandidatenliste | IDs/Inhalte vor Entfernung revalidiert und Varianten gesichert; zusätzlicher Dupe-Delete löscht Survivor-Basis nicht. |
| Leere Ordner/geschützter Unterbaum | Echte leere Ordner durabel erstellt/entfernt; nichtleere/unklare/geschützte bleiben. |
| Merge Originalbytes geändert bei gleicher Größe/Zeit | Abgelehnt; keine Seite überschrieben; alte Konfliktbasis erhalten. |
| Merge zweite Publikation scheitert und Neustart | Erste durable Sig bestätigt, alte Basis erhalten. pending_merge_for_key liefert Originaltexte/Sigs/Choice; Retry aus diesen Inputs bestätigt beide Seiten, ersetzt erst dann Basis und entfernt Recovery. |
| KeepBoth mit verschiedenen Elternschreibweisen | Verliererbytes an beiden tatsächlichen Seitenpfaden; Gewinner am Original; korrekte Basis-Sigs und gespeicherte Siblingschreibweisen. |
| Lock/StateKey/konkurrierendes Resolve | Lock vor erster Beobachtung; falscher Owner/Replika/Endpunkt abgewiesen; resolve_recorded blockiert offene Recovery atomar; merge_recorded bleibt Retryroute. |
| Root-E-ENGINE-Anschluss | pending_merge_relatives schützt beide Seiten vor normaler Planung, komplettem Index und Konvergenz-Checkpoint. Kein automatisches Überschreiben/offenes Merge als Konvergenzerfolg. |
| Close/Update nach Cancel | take_worker liefert einmalig tatsächlichen Worker; der Consumer kann begrenzt auf Completion warten. Spawn-Fehler bleibt Done und liefert None; ungenommener Handle detached. |
| Publish-Err/verlorener ACK | Keine Drop-Löschung der eventuell für Recovery nötigen Stage. Kein falscher Basiserfolg; dauerhafte Ersatz-Intent-Integration bleibt E-ENGINE. |

Gezielte vorbereitete Fälle stehen in merge_recorded_task_tests.rs, Fixture in merge_task_fixture.rs. Bestehende Transfer-Fixtures nutzen sichere exklusive Stage-/Regular-Read-/Extensions-Forwarder. Sämtliche Fälle/Integrationen gehören in dieselbe finale Remote-Suite; hier wurde nichts ausgeführt.

## Eigenprüfung und Owner-Grenzen

Statisch geprüft: Read/Modify/Create gegen Manifest/new_file_rule; eigene additive Registrierungen; lexikalische Klammer-/Kommentar-/String-Balance; alle eigenen Rust-Dateien unter 500 Zeilen/50 KiB; konservative Textabschätzung mit Formatierungsreserve; git diff --check ohne Befund. Keine Compiler-/Laufzeitabnahme. Root hält den Graph aktuell.

E-ENGINE übernimmt offene Mergepfade im regulären Lauf/Preview/single_recorded, verbleibende Literal-/Duplicate-Plan-Consumer, Drive-Identitymigration, Accountfeed-Indexauflösung und Provider-Replacement-Intent/Recovery. Genaue APIs/Quellgrenzen stehen in anfragen/E-APPLY.md und api-delta/E-APPLY.md. Zugeordnete Anschlussarbeiten, keine unerledigten E-APPLY-Stubs.

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
