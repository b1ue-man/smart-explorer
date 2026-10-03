# E-ENGINE – Umsetzung und Abnahme

Stand: 2026-10-03. Produktanschluss zu E-APPLY `b9f9b2ae` abgeschlossen; ausschließlich dokumentierte Y124/Y132/Y134/Y140/Y142 und freigegebener Pending-Schutz. Laufzeitabnahme steht aus. Keine lokale Ausführung, Builds, Compiler, Formatter, Tests, Server, Commits, Pushes oder Graphänderungen.

## Fundzuordnung und konkrete Abnahmesignale

Die bereits geplante verbundene Oberfläche bleibt erhalten. Die gemeinsame Remote-Suite des Hauptagenten muss diese Ergebnisse zusammen belegen:

| Fund / Ergebnis | Umsetzung | Konkretes Signal |
|---|---|---|
| Y124: literale Providerpfade | Hashkontrollen, Indexstat/IDs, Replica-/Versionsmarker und Dupebeobachtung verwenden `sync_path`/`sync_child_path`; Dupebeobachtung erhält den expliziten Literalnamen. | Literalnamen mit Prozentfolgen, Leerzeichen, `#` und Unicode werden einmal kodiert; gespeicherte Rootschreibweise bleibt exakt; gleiche Relative verschiedener Connections bleiben getrennt. Filter, OwnFile, Links/Junctions und normale `node_modules` behalten ihre bisherige Bedeutung. |
| Y132: stabile Accountidentität | Migration vor `replica::identify`/State-Open; höchstens acht belegte alte IDs je Seite, exakt gebundene geordnete Paare und alle alten physischen PairLocks bis Laufende. Private Pending-/Complete-Aliase, unveränderte Versionsdaten/Manifeste. | Alte Baseline, Journal, Owner-/Replicadateien, Dirs, Spellings und Merge-Eingaben werden idempotent übernommen, einschließlich globaler Legacy-Sidecars ohne vorhandene Baseline. Beide Paarorientierungen bleiben getrennt; konkurrierender alter Lock blockiert. Fremdes Konto/Root/Replika wird nicht gleichgesetzt. Alte Versionen bleiben lesbar und mit gesicherter Contentsignatur wiederherstellbar. |
| Y134: paginierter Accountfeed | Eindeutiger vollständiger File-/Directory-ID-Index und belegte Parentkette; Cache-Modus `mirror-rv2-ancestry`. Voller Bootstrap schreibt beide Seiten und Cursor in einer SQLite-Transaktion. | Verschachtelte bekannte Eltern ergeben exakt den Rootpfad. Belegt fremde Änderungen und entfernte unbekannte IDs lösen keine Rootaktion aus. Unbekannte/zyklische/geänderte Eltern, verschobene Verzeichnisse oder wiederholte IDs ergeben Rebuild. Neue Dupegruppen bleiben vollständig beobachtet; partielle/gefilterte/geschützte Walks zertifizieren keinen vollständigen Index. |
| Y140/Y142: reversible Veröffentlichung | Typed RunVersions/VersionSide-Bindung und tatsächliche Backup-Signatur; private durable Intent vor Request, volle Original-/Stage-Contentsignaturen und Precommit-Revalidierung. Ausgabe enthält kopierte Bytes und tatsächliche Zielzeit. | Backupfehler und geänderte Originalbytes bei gleicher Größe/Zeit verhindern Mutation. Im echten NoReplace-Zweig bleiben Original, Stage soweit vorhanden und Intent nach Fehler/ACK-Verlust erhalten: vor Capture, nach Capture und nach Publish. Kein erfolgreicher Baselineeintrag für einen fehlgeschlagenen Schritt. |
| Y140/Y142: bestehender FTP/DAV-Anschluss | Nur Hook-`Ok(false)` erlaubt genau einen Aufruf des bestehenden `promote_staged_replace`; dessen Fehler erhält den Intent. Keine Retry-/Unlink-Kaskade und kein neuer Namespace-Atomicity-Claim. | Erfolgreicher bisheriger FTP/DAV-Overwrite bleibt verfügbar. Hook false mutiert nichts; Promotion wird genau einmal versucht. Fehler vor Promotion und Lost ACK nach tatsächlich erfolgter Promotion erhalten AppData-Originalbackup sowie verbleibende Stage/Intent; der fehlgeschlagene Aufruf meldet keinen Erfolg. Recovery bestätigt nur tatsächliche Zielbytes. |
| Y140/Y142: gebundene Recovery | Exakte Pair/Lock/Owner/Backend/Root/Side/Replika und volle Digests/IDs; Restore ausschließlich NoReplace in freie Destination. Recovery schreibt selbst keine Erfolgsbaseline, auch bei Merge/Restore nicht. | Fremde neue Zielbytes, geänderte Stage oder geänderter retained-Inhalt bleiben unberührt und blockieren Recovery. Belegtes Original wird nur create-only restauriert; belegtes publiziertes Ergebnis benötigt Namespace-Bestätigung vor Cleanup. Alte Baseline bleibt bis tatsächlich bestätigter Planung/Apply maßgeblich. |
| Pending Merge/Replacement | Vor Full-/Previewplanung, Dedupe, Index und Konvergenz geschützte Originale/KeepBoth-Siblings; beide geordneten Richtungen und streng validierte andere Owner desselben Paars. Single-Aktion prüft unter gehaltenem PairLock. | Parallel gestarteter normaler Job und umgekehrtes Paar greifen auf keine Pending-Relative zu, übernehmen keine fremde Baseline und säen keinen Teilindex. Fremde echte Replica bleibt getrennt. Single-/Previewaktion verweigert unter derselben Sperre; `preview_with(..., RunSettings)` erhält den Jobowner. Ownergebundene öffentliche Merge-Recovery-APIs bleiben erhalten. |

## Entscheidungen und eigene Schlussprüfung

- Kein Konto wird aus Pfaden oder Tokens geraten; Providerhinweise sind die einzige neue Same-account-Autorisierung. Mehrere historische Zustandsfamilien oder abweichender bereits vorhandener Zielzustand blockieren statt eine Basis auszuwählen. Alte Dateien bleiben erhalten; SQLite-Caches werden vollständig neu aufgebaut.
- Globale Legacy-AdHoc-Pending-Eingaben ohne Replikatoken bleiben konservativ geschützt, ohne ihre Basis einem neuen Owner/Replika zuzuordnen.
- `namespace_replace` bleibt eine Mountgarantie. Der bestehende Sync-Publish-Vertrag wird nach garantiert mutationsfreiem Hook-false konsumiert; Unsupported bei einem Backend ohne beide Primitiven bleibt ein sicherer Fehler. Ungebundene Low-level-Replacements erhalten keinen geratenen Kontext.
- Restore bindet den neuen Intent an die bereits erfolgreich gesicherte `Preserved.signature`. Merge/Restore verwenden `checkpoint_allowed=false`; Recovery erzeugt auch für normale Läufe keine synthetischen Erfolgseinträge.
- Eigene Source-Schlussprüfung: Pair-ID/Lock-ID-/Owner-/Replica-Grenzen, gehaltene Locks, Kontrolldigests, Cancel-/Fehlerwege und bestehende API-Signaturen abgeglichen. Die neuen Intent-Leser verwenden konsistent die vorhandenen Seitentags `a/b`. Den direkt betroffenen undefinierten `scope` beim Dedupeanschluss auf den vorhandenen `dedupe_scope` korrigiert; `pair_dir` als tatsächlichen PathBuf-Rückgabewert konsumiert.
- Statisch: `git diff --check` sauber; eigene Rustflächen höchstens 410 Quellzeilen/16.749 Bytes, neue Module mit Formatierungsreserve unter 500 Zeilen/50 KiB. Keine Compiler-/Formatter- oder Laufzeitaussage aus lokalen Checks.
- Produktanschluss vollständig, keine offene Scope-Anfrage. Eigene Abnahme-Fixtures wurden gemäß der späteren Parentgrenze aus dem Worktree entfernt; deren Anschluss folgt erst nach separater Freigabe zusammen mit der einen gemeinsamen Suite.

## Exaktes Datei-Inventar

Inventar dieses E-ENGINE-Blocks; gemeinsame bereits geladene AGENTS-/Architektur-/Skillvorgaben wurden weiterverwendet. `gelesen` umfasst gezielte Ausschnitte/Suchen und eigene statische Schlussprüfung. Keine Provider-, UI-, Host-, Job-, CI-, Release- oder Graphdatei geändert.

| Exakter Pfad | Tätigkeit |
|---|---|
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/E-ENGINE.md` | erstellt; eigene Schlussprüfung |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/E-ENGINE.md` | erstellt; eigene Schlussprüfung |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/V-REMOTE.md` | gelesen |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-APPLY.md` | gelesen |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-ENGINE.md` | erstellt; eigene Schlussprüfung |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-PLAN.md` | gelesen |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/V-REMOTE.md` | gelesen |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/integration.md` | gelesen |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/e-engine.json` | gelesen |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md` | gelesen |
| `native/src/bisync/core/omissions.rs` | gelesen |
| `native/src/bisync/core/paths.rs` | gelesen |
| `native/src/bisync/core/run_types.rs` | gelesen |
| `native/src/bisync/core/types.rs` | gelesen |
| `native/src/bisync/mod.rs` | gelesen; sechs eigene additive Registrierungen |
| `native/src/bisync/os/shared/apply_guard.rs` | gelesen |
| `native/src/bisync/os/shared/apply_stage.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/apply_transaction.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/backend_identity_migration.rs` | erstellt; eigene Schlussprüfung |
| `native/src/bisync/os/shared/backend_identity_state.rs` | erstellt; eigene Schlussprüfung |
| `native/src/bisync/os/shared/checkpoint_journal.rs` | gelesen |
| `native/src/bisync/os/shared/duplicate_observation.rs` | gelesen |
| `native/src/bisync/os/shared/duplicate_plan.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/engine_change_feed.rs` | erstellt; eigene Schlussprüfung |
| `native/src/bisync/os/shared/engine_provider_task_tests.rs` | gelesen; eigener Fixture-Entwurf entfernt |
| `native/src/bisync/os/shared/incremental.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/incremental_changes.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/incremental_collect.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/merge_execution.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/merge_inputs.rs` | gelesen |
| `native/src/bisync/os/shared/merge_recovery.rs` | gelesen |
| `native/src/bisync/os/shared/orchestration.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/orchestration_full.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/orchestration_plan.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/pair_lock.rs` | gelesen |
| `native/src/bisync/os/shared/persistence.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/preview.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/replacement_journal.rs` | erstellt; eigene Schlussprüfung |
| `native/src/bisync/os/shared/replacement_publish.rs` | erstellt; eigene Schlussprüfung |
| `native/src/bisync/os/shared/replacement_recovery.rs` | erstellt; eigene Schlussprüfung |
| `native/src/bisync/os/shared/replica.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/replica_state.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/single_recorded.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/snapshot.rs` | gelesen |
| `native/src/bisync/os/shared/snapshot_pair.rs` | gelesen |
| `native/src/bisync/os/shared/state_bootstrap.rs` | gelesen |
| `native/src/bisync/os/shared/state_metadata.rs` | gelesen |
| `native/src/bisync/os/shared/state_spellings.rs` | gelesen |
| `native/src/bisync/os/shared/state_store.rs` | gelesen |
| `native/src/bisync/os/shared/state_types.rs` | gelesen |
| `native/src/bisync/os/shared/test_remote.rs` | gelesen |
| `native/src/bisync/os/shared/version_manifest.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/version_ops.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/version_restore.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/version_save.rs` | gelesen und geändert |
| `native/src/bisync/os/shared/versions.rs` | gelesen |
| `native/src/ftp/core/extensions.rs` | gelesen |
| `native/src/gdrive/core/backend.rs` | gelesen |
| `native/src/gdrive/core/changes.rs` | gelesen |
| `native/src/gdrive/core/extensions.rs` | gelesen |
| `native/src/sftp/core/reversible_replace.rs` | gelesen |
| `native/src/support_dirs.rs` | gelesen |
| `native/src/vfs/core/capabilities.rs` | gelesen |
| `native/src/vfs/core/core.rs` | gelesen |
| `native/src/vfs/core/extension_calls.rs` | gelesen |
| `native/src/vfs/core/extension_types.rs` | gelesen |
| `native/src/vfs/core/extensions.rs` | gelesen |
| `native/src/vfs/core/promotion.rs` | gelesen |

Eigene Registrierung in `native/src/bisync/mod.rs`: `replacement_journal`, `replacement_recovery`, `replacement_publish`, `engine_change_feed`, `backend_identity_state`, `backend_identity_migration`, jeweils additiv mit exakt benannter Source-Datei. Bestehende gemeinsame Registrierungen gehören dem Hauptagenten.
