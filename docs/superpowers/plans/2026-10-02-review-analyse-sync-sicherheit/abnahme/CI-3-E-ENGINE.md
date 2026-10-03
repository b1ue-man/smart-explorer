# CI-3 – E-ENGINE

Stand: 2026-10-03. Begrenzter Anschluss zu Run `37157166735`, Kandidat `71a8ca45697272453c213c9b0b5412d0cbed0f71`; Scope `scopes/ci-3-e-engine.json`, eigener Abschnitt in `ci-closure-fixes.md`, Diagnosen `/tmp/rv1-ci-third/e-engine.json` frisch gelesen.

## Plan-/API-Abschluss vor dem Directory-Edit

Frische Referenzen: `docs/refs/rv1-remote-suite.md` vollständig und `docs/refs/local-fs-identity-durability.md` mit den Abschnitten zu Veröffentlichung/Flush/Zeitstempeln und NoFollow-/regulärem Öffnen. Keine neue Plattformannahme. Ausschließlich aktuelle freigegebene Definitionen bestimmen die Engine-/Fixtureanschlüsse.

| Enger Meilenstein | Beleg / Entscheidung | Signal derselben Root-Suite |
| --- | --- | --- |
| Vollständiger Index mit Ancestry-Ordnern | `engine_change_feed::bootstrap` erzeugt Directoryrows mit `sig=None`; frisch gelesenes `state_validation::validate_item` verweigert noch alle Directoryrows. Root hat ausschließlich Directory ohne Dateisignatur und aktive File-Baselineprojektion freigegeben. | Vorhandene Link-Recovery verlangt echte `bootstrapped=true` nach Rückkehr zum regulären Ordner; alle Dateisignatur-/Pfad-/Budget-/Tombstonegrenzen erhalten. |
| Tatsächliche State-Auswahl | `Outcome.state` ist der ausgeführte Owner-/Replica-Key; `baseline_file` und `replica_state::index_id` sind die aktuellen Selektoren. | Die bestehenden Link-/Baseline-/Indexassertions lesen denselben tatsächlich ausgeführten Zustand. |
| Exklusiver Stage-/Statusvertrag der Fixture | `Backend::open_write_new` ist ohne Override Unsupported; lokale Wrapper müssen eigenes Fault-/Zählverhalten und die wirklichen LocalBackend-Extensions erhalten. DeleteProbe meldet bisher die Root als reguläre Datei; `apply_boundary::guard` schützt das vor Mutation. | Bestehende Copy-/Retry-/Backup-/Deleteassertions bleiben, einschließlich absoluten Remotepfads ohne lokalen Papierkorb. |
| Agent mit ausgeblendeten Einträgen | Nachtrag vor Korrektur: `HashWalkEntry`/Share-/Agent-Hashdarstellung liefern keine Hidden-Attribute; `snapshot_dir` prüft dagegen `m.hidden`. Die vorherige Entfernung der Hidden-Verweigerung deckte nur Punktnamen ab und wird zurückgenommen. | Metadata-Fallback bei `include_hidden=false`; vorhandener Fastpath-Fall mit `include_hidden=true` und explizitem `.hidden/**`-Glob behält `node_modules`, Auslassungen und echte Hashes. |
| Unbelegte Runtimewerte | Bei Pending-Konvergenz und Windows-Lost-ACK liefert die vorhandene Ausgabe keine konkreten Werte/Stufen. Keine geratenen Planner-/Permission-Reparaturen. | Nur zusätzlicher Assertionkontext in den vorhandenen Fixtures; Windows AccessDenied bleibt bei V-LOCAL. |

`state_store::load_side_with_budget` validiert Signaturencoding/Boolean/Pfade/Budgets und normalisiert historische Tombstones weiterhin ohne aktive Signatur. Der Directoryanschluss ändert weder Decoder/Schema noch aktive Dateivalidierung, Owner/Replica oder vollständigen Fehler-Fallback. Die folgenden Änderungen und Grenzen sind der abgeschlossene statische Handoff; die gemeinsame Remote-Bestätigung bleibt Root.

## Änderungen und Entscheidungen

- **Index/Baseline:** `validate_item` akzeptiert Ancestry-Verzeichnisse ausschließlich mit `sig=None`; `active_sig` liefert ausschließlich aktive Dateien. Dateisignatur-, Pfad-, Budget-, Owner-/Replica-, Tombstone- und vollständiger Fehler-Fallbackvertrag bleiben erhalten. Der tatsächliche Bootstrap erzeugt diese Verzeichniszeilen bereits; zuvor verweigerte die gemeinsame Validierung sie.
- **Link-/NoOp-Zustand:** Die betroffenen Fixtures wählen mit `Outcome.state` dieselbe ausgeführte Baseline über `baseline_file` und denselben Index über `replica_state::index_id`. Der alte rohe Pair-/Legacy-Selektor konnte bei nichtlegacy Owner-/Replica-Zustand keine Zeile/Datei finden. Link-Gegenstücke, vorige Baseline und die echte Rückkehr zu `bootstrapped=true` bleiben zwingend.
- **Stage-Fixtures:** `Counting`, `WriteFail` und `StatFail` behalten Zählung/Fault-Injection und liefern exklusive LocalBackend-Writer, reguläre NoFollow-Reads, Finish-/Flush-, Limits-, Mode- und Volume-Extensions. Default-Timed-Stage bleibt auf dem eigenen Writer, damit Faults erhalten bleiben. Es gibt weder einen nicht exklusiven Writer-Fallback noch eine erfundene Durability-Bestätigung.
- **Delete-Fixture:** `DeleteProbe` meldet die tatsächliche Directory-Root und ein konsistentes virtuelles Ziel vor/nach Löschung. DeleteB erhält einen unabhängigen, tatsächlich fehlenden A-Peer für die bestehenden Revalidierungen. Die Root war zuvor eine Datei und wurde von `apply_boundary` vor Mutation geschützt. Der lokale Recyclingfehler betrifft weiterhin eine nicht existierende Datei unter einer gültigen Root; kein permanenter Fallback.
- **Agent-Fastpath, korrigierter Integrationsnachtrag:** Der ursprüngliche Metadata-Fallback bei `include_hidden=false` bleibt erhalten. `HashWalkEntry` enthält keine Hidden-Metadaten; Rel-/Punktnamenfilter können Windows-Hidden-Dateien ohne Punktpräfix nicht sicher erkennen. Der vorhandene Agent-Fall verwendet `include_hidden=true` sowie `.hidden/**`/`ignored`-Globs, behält seine Tree-/Hash-Assertions und prüft zusätzlich `include_hidden=false -> None`. Cross-Mount-, Ownfile-, Link-/Legacy-, Budget-, Cancellation- und Teilwalkgrenzen bleiben unverändert; normale `node_modules` bleiben zulässig. Kein neuer Protokollvertrag.
- **Diagnose:** Die vorhandenen Pending-, Recorded-Lost-ACK-, NoOp- und Safety-Assertions behalten Bedingungen/Erwartungen und erhalten konkrete Werte. Kein erratener Planner- oder Windows-Permissionfix.

## Abnahmesignale derselben Remote-Suite

Unveränderte vorhandene Leaf-Symbole; keine neue unabhängige Suite und kein neuer Test:

- `engine_provider_pending_protects_owners_orientation_and_single_actions`: Pending-Originale/Siblings dürfen nicht konvergieren; Fehlermeldung enthält Pending, tatsächliche Converged, Omissions und Planning-Baseline.
- `engine_provider_recorded_lost_ack_preserves_old_baseline_until_retry`: genau eine tatsächliche Veröffentlichung, alter Checkpoint und Intent bis Retry; bei Promotions=0 enthält die Assertion Errors/Blocked/Stopped/Deferred/State.
- `no_op_run_skips_rewalk`: fehlerfreier erster und zweiter Lauf; ursprüngliche Listing-Erwartungen 2/2 und 1/1 sowie Copy/Delete=0 beim NoOp bleiben erhalten.
- `sync_links_task_nested_link_preserves_counterparts_baseline_and_incremental_recovery`, `sync_links_task_target_link_reverse_mirror_and_exclusions_are_protected`, `sync_links_task_incremental_target_junction_returns_to_full_protected_scan`: tatsächlicher Owner-/Replica-Zustand, geschützte Link-Gegenstücke/vorige Basis, unabhängige Aktionen und vollständige Recovery.
- `sync_links_task_agent_and_daemon_streams_fall_back_without_losing_protection`, `sync_links_task_regular_agent_tree_keeps_fast_hash_path_and_filters`: echte Agent-/Daemon-Verbindung, Link-Fallback bzw. Hash-Fastpath bei `include_hidden=true` mit `.hidden/**`/`ignored`-Globs; normale node_modules enthalten. Derselbe Fastpath-Fall verlangt zusätzlich `None` für `include_hidden=false`, damit der Metadata-Walk auch Hidden-Attribute berücksichtigt.
- `failed_apply_paths_stay_out_of_new_baseline_and_retry`, `backup_failure_blocks_overwrite_and_delete`, `remote_absolute_path_never_uses_local_recycle_bin`: bestätigte unabhängige Kopie, fehlgeschlagene Pfade ohne neue Basis, Retry, zwei Backupfehler ohne Mutation und genau ein Remote-Delete bei unveränderten lokalen Kollisionsbytes. Vorhandene Recycling-/Stat-/KeepBoth-Sicherheitsassertions bleiben ebenfalls.

## Eigener statischer Abschluss

Eigene Source-Differenz und tatsächliche Trait-/Facade-Verträge abgeglichen; alle bisherigen Testsymbole und Sicherheitsassertions erhalten. Keine öffentliche API geändert. Einzige Registrierung: privates Kindmodul `safety_backends` mit `#[path = "safety_backends.rs"]` in `tests/safety.rs`. Kein Compiler, Formatter, Test, Git-, Graph-, CI- oder Release-Lauf.

Aktuelle Zeilenstände: snapshot_agent 218; state_validation 244; engine_provider_task_tests 395; engine_identity_task_tests 451; hash_walk 362; links 186; links_remote 153; safety 301; neuer safety_backends 331. Alle unter 50 KiB. Neue größere Blöcke manuell mehrzeilig, ausreichende Reserve zur normalen Formatierung; kein lokales rustfmt.

Begrenzter Hidden-Nachtrag: Frisch gelesene Quellen aus dem ergänzten Scope belegen den fehlenden Hidden-Vertrag in VFS-/Share-/Agent-Hashdarstellungen gegenüber `snapshot_dir::m.hidden`. Ausschließlich eigene vorige Guard-Änderung korrigiert, bestehender Agent-Fall angepasst und Bericht berichtigt; keine Datei erstellt, keine API oder Registrierung erweitert. Statischer Self-Review bestätigt den ursprünglichen Guard, unveränderte bestehende Assertions und zusätzliche Fallback-Assertion. Keine neue offene Definition oder Owner-Abhängigkeit.

## Offene Grenzen

- **Pending-Konvergenz:** Die bisherige Remote-Ausgabe enthält keine tatsächlichen konvergierten Relatives. Im erlaubten Source-Anschluss sind Bäume und Planning-Baseline geschützt; es liegt kein konkreter weiterer Produktionsfix vor. Die neue Assertiondiagnose muss im selben Root-Remote-Lauf die Ursache belegen.
- **Windows Recorded-Lost-ACK:** Promotions=0 belegt keinen Provider-Publish. Der neue Kontext zeigt den tatsächlichen Preflight-/Laufzustand. Windows AccessDenied bleibt V-LOCAL; kein Fixture-Permissionsbypass und keine Erfolgsmaskierung.
- Keine zusätzliche Scope-Anfrage. Laufbestätigung einschließlich NoOp/Backup-Status gehört ausschließlich zur bestehenden Root-Suite.

## Exaktes Inventar

### Gelesen

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-3-e-engine.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-closure-fixes.md`
- `/tmp/rv1-ci-third/e-engine.json`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/local-fs-identity-durability.md`
- `native/src/bisync/core/plan_pair.rs`
- `native/src/bisync/core/plan_filter.rs`
- `native/src/bisync/core/plan_types.rs`
- `native/src/bisync/os/shared/orchestration_plan.rs`
- `native/src/bisync/os/shared/engine_provider_task_tests.rs`
- `native/src/bisync/os/shared/tests/links.rs`
- `native/src/bisync/os/shared/tests/links_remote.rs`
- `native/src/bisync/os/shared/snapshot_pair.rs`
- `native/src/bisync/os/shared/snapshot_agent.rs`
- `native/src/bisync/os/shared/snapshot_policy.rs`
- `native/src/bisync/os/shared/snapshot_walk.rs`
- `native/src/bisync/core/plan.rs`
- `native/src/bisync/os/shared/tests/hash_walk.rs`
- `native/src/bisync/os/shared/tests/safety.rs`
- `native/src/bisync/os/shared/orchestration.rs`
- `native/src/bisync/os/shared/orchestration_full.rs`
- `native/src/bisync/os/shared/test_remote.rs`
- `native/src/bisync/os/shared/replica_state.rs`
- `native/src/bisync/os/shared/state_bootstrap.rs`
- `native/src/bisync/os/shared/persistence.rs`
- `native/src/bisync/os/shared/engine_identity_task_tests.rs`
- `native/src/bisync/os/shared/snapshot.rs`
- `native/src/bisync/os/shared/snapshot_hash.rs`
- `native/src/bisync/os/shared/engine_provider_fixture.rs`
- `native/src/bisync/core/run_types.rs`
- `native/src/bisync/core/types.rs`
- `native/src/bisync/mod.rs`
- `native/src/bisync/core/omissions.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/bisync/os/shared/apply.rs`
- `native/src/bisync/os/shared/apply_reporting.rs`
- `native/src/bisync/os/shared/apply_transaction.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/extension_types.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/core/plan_index.rs`
- `native/src/bisync/os/shared/apply_boundary.rs`
- `native/src/bisync/os/shared/apply_actions.rs`
- `native/src/bisync/os/shared/apply_guard.rs`
- `native/src/bisync/os/shared/apply_transfer.rs`
- `native/src/bisync/os/shared/state_validation.rs`
- `native/src/bisync/os/shared/state_store.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-E-ENGINE.md`
- `native/src/bisync/os/shared/engine_change_feed.rs`
- `native/src/bisync/os/shared/tests/safety_backends.rs`
- `native/src/share/core/hash_walk_wire.rs`
- `native/src/agent_proto/os/shared/hash.rs`
- `native/src/agent_proto/core/ops_types.rs`
- `native/src/bisync/os/shared/snapshot_dir.rs`

Lesedetails: `apply_transfer.rs` nur `verify_expected_content` und direkt zugehörige Helfer; `state_store.rs` nur `load_side_with_budget`; run_types/types/mod gezielte Typ-/Registrierungsstellen; `engine_change_feed.rs` hier gezielt Bootstrap. Hidden-Nachtrag liest `snapshot_dir.rs` nur 117–139, `hash_walk_wire.rs` nur Entry/Conversions/Hidden-Fakten, `agent_proto/os/shared/hash.rs` nur Hash-Entry-Metadatendarstellung und `ops_types.rs` nur DTO-Nachschlag; `vfs/core/extension_types.rs` hier nur HashWalkEntry/HashWalkItem. Dazu Scope, eigene Guard-/Fixturestellen und bestehender Bericht. Transparenz: Beim Sichern der Änderungsbasis wurde `state_validation.rs` einmal vollständig gelesen statt nur `validate_item/active_sig`; außerhalb dieser beiden Funktionen wurde nichts ausgewertet oder geändert. Sonstige Reads bleiben im zugewiesenen Manifest.

### Geändert

- `native/src/bisync/os/shared/snapshot_agent.rs`
- `native/src/bisync/os/shared/engine_provider_task_tests.rs`
- `native/src/bisync/os/shared/engine_identity_task_tests.rs`
- `native/src/bisync/os/shared/tests/hash_walk.rs`
- `native/src/bisync/os/shared/state_validation.rs`
- `native/src/bisync/os/shared/tests/links.rs`
- `native/src/bisync/os/shared/tests/links_remote.rs`
- `native/src/bisync/os/shared/tests/safety.rs`

### Erstellt

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-E-ENGINE.md`
- `native/src/bisync/os/shared/tests/safety_backends.rs`
