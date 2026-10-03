# CI-1 – E-ENGINE

Stand: 2026-10-03. Konkrete Diagnosen aus [Run 37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629), Kandidat `395f912a30455ebd96799f61fdbe1fec2e1c7998`; Root hat dessen gebundenen Formatterpatch als `692fe162` angewandt. Auftrag: [ci-1-e-engine.json](../scopes/ci-1-e-engine.json), Meilenstein E-ENGINE in [ci-fixes.md](../ci-fixes.md). Keine neue Untersuchung oder unabhängige Tests.

## Diagnose → Korrektur → Abnahme

| Belegter Anschluss | Korrektur | Signal in derselben Root-Suite |
| --- | --- | --- |
| E0432/E0425: nicht vorhandenes `bisync::paths::validate_child_name` | Versionen-Listing/-Retention/-Manifest konsumieren die bestehende `crate::vfs::validate_child_name`-Fassade. Der tatsächliche Validator verweigert leere Namen, Punktpfade, Trennzeichen und NUL; keine neue oder gelockerte Validierung. | Native Linux-/Windows-Komponenten lösen den vorhandenen Reexport auf; bestehende Versionen-/Alias-/Restore-Assertions bleiben. |
| E0599: `TestRemote::try_exists` ohne Trait im Fixture-Scope | `use crate::vfs::Backend` im vorhandenen Identity-Fixture. Fallible Beobachtung bleibt erhalten; kein Wechsel auf `exists` oder unterdrückte Fehler. | Die unveränderten Publication-/Lost-ACK-/Foreign-Creator-/Recovery-Assertions kompilieren und werden ausgewertet. |
| E0282: unbestimmter Marker-Fehlertyp | Marker-Closure ausdrücklich als `io::Result<ReplicaRef>` gebunden. Schreiben/Flush/Publish, Fehler-Cleanup und AlreadyExists-Beobachtung bleiben unverändert. | Replica-/Owner-/Identity-Assertions verwenden denselben bestehenden Markerpfad. |
| E0432: entfernte `try_incremental_mirror`-Fixture-Fassade | Die vorhandene Rename-Swap-Fixture startet einen echten recorded Initiallauf über `run_with_store_path`. Er erzeugt Baseline, History und vollständigen Index. Danach derselbe Swap/Feed über denselben Engine-Einstieg; der Index wird aus dem tatsächlichen `StateKey` mit `index_id` geladen. | Beide bisherigen Zielpayloads und vertauschten Source-IDs werden weiterhin ausdrücklich geprüft. Der reine ActionPlan-Test enthält weiterhin beide finalen Upserts und keine Deletes; Ignore-/Cancel-/Budget-Szenarien bleiben unverändert. |
| Formatter: `incremental.rs` 539 Zeilen | Bestehende Indexretirement-/Bootstrap-/Invalidation-/Fingerprint-Verantwortung kohäsiv nach `incremental_index_commit.rs` ausgelagert. Quelle nun 426 Zeilen; Helper 133. | Dieselben Cache-/Pending-/Omission-/Checkpoint-Grenzen und Fingerprintbytes; Format-/Dateigrenze im unveränderten Remote-Einstieg. |

## Facaden, Registrierung und Entscheidungen

Die bisherigen Facaden `incremental::{retire_index, bootstrap_run, bootstrap_incremental_state, invalidate_incremental_state}` behalten ihre Signaturen und ihre `pub(super)`-Reichweite. Das private Kindmodul `index_commit` definiert sie als `pub(in crate::bisync)`; dadurch können sie mit derselben Sichtbarkeit reexportiert werden. `pub(super)` allein im neuen Kindmodul wäre für den bisherigen weiterreichenden Reexport zu eng. `open_store` und `mode` sind nur für ihren Parent `incremental` sichtbar; `pair_record` bleibt privat.

Registrierung ausschließlich in `incremental.rs`: privates `#[path = "incremental_index_commit.rs"] mod index_commit;` plus die genannten Facaden. `bisync/mod.rs` wurde nicht geändert. Die bestehenden Fixture-Helper liegen im privaten Kindmodul `fixture` von `tests/incremental_safety.rs`; keine zusätzliche Testregistrierung oder neue Testsymbole.

Der Feedadapter delegiert exclusive create, Stage-Cleanup, lokale Extensions und Case-Fakten an den tatsächlichen LocalBackend. Die kontrollierten Fixture-IDs und Cursor bilden denselben vorhandenen Root-relative Feed ab; ID-Zuordnung folgt dem Swap. Ihre Pfade werden aus der gespeicherten Fixturewurzel gebildet. Dies ist Adapterevidenz, kein Live-Provider-Nachweis. SQLite bleibt außerhalb beider Syncwurzeln; Cleanup ist auf den eigenen tatsächlichen Pair-Verzeichnispfad begrenzt.

Eigener statischer Self-Review: alle ausgelagerten Funktionsbodies wurden gegen die formattergebundene Ausgangsquelle textuell verglichen; nur Modulpfade/Sichtbarkeit und zugehörige Imports ändern sich. Die drei nicht an die entfernte Fassade angeschlossenen vorhandenen Testbodies sind bytegleich geblieben. Alle bisherigen Swap-Payload-/ID-Assertions sind erhalten. Import-/Closure-/Child-Facaden und Scopes geprüft; keine lokalen Compiler-, Test-, Build- oder Formatterläufe.

## Unveränderte Abnahmesymbole

Die bestehenden Registrierungen/Root-Auswahl bleiben. Exakte Leaf-Symbole:

- `rename_swap_copies_both_final_paths_without_deleting_them`
- `rename_swap_applies_and_persists_both_final_paths`
- `ignored_remove_feed_never_becomes_a_delete_action`
- `canceled_and_over_budget_feeds_fail_closed`
- `engine_provider_account_identity_preserves_state_locks_inputs_and_versions`
- `engine_provider_recorded_lost_ack_preserves_old_baseline_until_retry`
- `engine_provider_publication_and_lost_ack_use_exactly_one_contract`

Provider-Suitepräfix weiterhin `engine_provider_` / `bisync::engine_provider_task_tests::`; die drei oben genannten Provider-Symbole liegen im bestehenden Kindmodul `identity_tests`. Erfolg muss Root im selben vollständigen RV1-Remote-Einstieg nachweisen. Es wurde keine Runtime-Abnahme vorweggenommen.

Geänderte Bestandsdateien haben 214–442 Quellzeilen; neue Helper 133 bzw. 226, jeweils deutlich unter 50 KiB. Die kleinen neuen mehrzeiligen Aufrufe behalten Formatierungsreserve; die endgültige Formatterprüfung bleibt remote.

## Exaktes Inventar und Scope-Anschlüsse

| Datei | Leseumfang | Änderung |
| --- | --- | --- |
| `/tmp/rv1-ci-first/e-engine.json` | gelesen | – |
| `docs/refs/local-fs-identity-durability.md` | gelesen | – |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-E-ENGINE.md` | gelesen | erstellt |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fixes.md` | gelesen | – |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-1-e-engine.json` | gelesen | – |
| `native/src/bisync/core/paths.rs` | gelesen | – |
| `native/src/bisync/core/run_types.rs` | gelesen | – |
| `native/src/bisync/os/shared/engine_identity_task_tests.rs` | gelesen | geändert |
| `native/src/bisync/os/shared/incremental.rs` | gelesen | geändert |
| `native/src/bisync/os/shared/incremental_apply.rs` | Lesen versucht; Datei fehlt | – |
| `native/src/bisync/os/shared/incremental_changes.rs` | gelesen | – |
| `native/src/bisync/os/shared/incremental_collect.rs` | gelesen | – |
| `native/src/bisync/os/shared/incremental_index_commit.rs` | gelesen | erstellt |
| `native/src/bisync/os/shared/orchestration.rs` | gelesen | – |
| `native/src/bisync/os/shared/pair_lock.rs` | gelesen | – |
| `native/src/bisync/os/shared/replica.rs` | gelesen | geändert |
| `native/src/bisync/os/shared/replica_state.rs` | gelesen | – |
| `native/src/bisync/os/shared/state_bootstrap.rs` | gelesen | – |
| `native/src/bisync/os/shared/state_metadata.rs` | gelesen | – |
| `native/src/bisync/os/shared/state_store.rs` | gelesen | – |
| `native/src/bisync/os/shared/tests/incremental_safety.rs` | gelesen | geändert |
| `native/src/bisync/os/shared/tests/incremental_safety_fixture.rs` | gelesen | erstellt |
| `native/src/bisync/os/shared/version_listing.rs` | gelesen | geändert |
| `native/src/bisync/os/shared/version_manifest.rs` | gelesen | geändert |
| `native/src/bisync/os/shared/version_ops.rs` | gelesen | geändert |
| `native/src/daemon/os/shared/rooted_backend_case.rs` | gelesen | – |
| `native/src/vfs/core/core.rs` | gelesen | – |
| `native/src/vfs/core/delete.rs` | Validator 318–331; RG-Kontext zeigte auch join_child 333–340 (ungenutzt) | – |
| `native/src/vfs/core/path_validation.rs` | Lesen versucht; Datei fehlt | – |
| `native/src/vfs/mod.rs` | gelesen | – |

Die zusätzlichen Reads von Orchestration/StateStore/StateMetadata/ReplicaState waren auf die freigegebenen Run-/Owner-/StateKey-/Index-/History-Pfade begrenzt. Der oben protokollierte ungewollte Validator-Nachbarkontext wurde Root gemeldet und nicht benutzt oder geändert. Die zwei nicht vorhandenen Ausgangspfad-Einträge wurden durch die konkret freigegebene VFS-Fassade und den tatsächlichen Orchestratoranschluss aufgelöst. Alle nötigen Scope-Anfragen sind beantwortet; keine offene Produkt-/API-/Scope-Lücke. Suite, Graph, Commit/Push und Release liegen bei Root.
