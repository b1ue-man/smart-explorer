# CI-2 – E-ENGINE

Stand: 2026-10-03. Beleg: [Run 37150409255](https://github.com/b1ue-man/smart-explorer/actions/runs/37150409255), Kandidat `ac9b475ff18f6320bedd408c5a03c091710ad01c`, Diagnose `/tmp/rv1-ci-second/e-engine.json`. Auftrag und eigener Planabschnitt: [Scope](../scopes/ci-2-e-engine.json), [CI-Verhaltensfixes](../ci-behavior-fixes.md).

## Beleg → Änderung → Abnahme

Die bestehende Indexfixture klont einen aktiven Datensatz und setzt `deleted = true`, wobei dessen letzte Signatur erhalten bleibt. `StateBudget::record_item`/`validate_item` verweigerten das vor dem bereits vorhandenen transaktionalen SQL-DELETE (`deleted sync item has a signature`).

- Der Validator akzeptiert die letzte Signatur eines Löschkommandos. Der Writer entfernt weiterhin genau `(pair, side, rel)`; sein SQL und seine Transaktionsgrenzen bleiben unverändert.
- Der Decoder prüft gespeicherte Signaturfelder weiterhin vollständig mit `parse_sig`. Bei einem gültigen Tombstone liefert er anschließend `sig = None`. Aktive Zeilen benötigen unverändert eine vollständige Signatur; fehlerhafte oder partielle gespeicherte Signaturen scheitern weiterhin als vollständiger Cache-Ladevorgang.
- Die bestehende Fixture behält den Löschversuch mit vorhandener Signatur, den leeren Source-Index und sämtliche Owner-Cleanup-Assertions. Ergänzt sind die unveränderte Gegenseite der Baseline, ein historischer gespeicherter Tombstone mit vollständiger Signatur, dessen Ausschluss aus Baseline/ID-Lookup und die Ablehnung eines partiellen Signaturencodings ohne Teilbaseline.

Keine Änderung an Schema, öffentlichen Signaturen oder Registrierungen. Keine Baseline-/Owner-/Replica-Umetikettierung. Historische gültige Tombstones bleiben gelöschte Datensätze, ihre alte Signatur wird nicht übernommen.

## Self-Review und Entscheidungen

Statischer Vergleich mit der frisch gelesenen Ausgangsquelle: Writer-/Transaktionsbodies, Collection-Budgets, Pfad-/Booleanprüfung, aktive Signaturdecodierung, Baselineprojektion und die Fehlerklassifizierung für den vollständigen Fallback sind unverändert. Die bestehende Bootstrap-/Rollback-Fixture und alle bisherigen Owner-Cleanup-Assertions sind unverändert. Keine neuen Testfunktionen; keine Abschwächung vorhandener Assertions.

`engine_change_feed` ignoriert unverändert gelöschte Indexeinträge; `baseline_from_items` und `rel_for_id` schließen diese ebenfalls weiterhin aus. Owner-/Replica-Schlüssel und deren Cleanup wurden nur gelesen. Dateien nach der Änderung: `state_store.rs` 372 Zeilen, `state_validation.rs` 239, `index_review_tests.rs` 145; jeweils unter 50 KiB und mit Formatierungsreserve.

Die freigegebenen Referenzen wurden frisch gelesen; die konkrete Persistenzentscheidung folgt den aktuellen typed Item-/Store-/Validator-Definitionen. Keine lokale Test-, Compiler-, Build- oder Formatterausführung; kein Git, CI, Graph oder Release. Runtime-Abnahme bleibt bei derselben vollständigen Root-RV1-Remote-Suite.

## Bestehende Abnahmesymbole

Direkt betroffener unveränderter Suite-Name:

- `bisync::index_review_tests::review_task_index_tombstones_remove_rows_and_owner_cleanup_stays_scoped`

Bestehende Schutzsignale, deren Implementierung/Assertions unverändert bleiben:

- `bisync::index_review_tests::review_task_index_bootstrap_rolls_back_rows_and_cursor_together`
- `bisync::state_store::tests::corrupt_side_signature_and_relative_path_are_rejected`
- `bisync::state_store::tests::state_load_budget_fails_before_growing_unbounded`
- `bisync::state_store::tests::corrupt_incremental_state_falls_back_to_a_safe_full_rebuild`

## Exaktes Inventar

| Datei | Gelesen | Änderung |
| --- | --- | --- |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-2-e-engine.json` | ja | – |
| `/tmp/rv1-ci-second/e-engine.json` | ja | – |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-behavior-fixes.md` | Kopf/Ownerabschnitt | – |
| `docs/refs/local-fs-identity-durability.md` | ausgegebene Refabschnitte; Gesamtausgabe war gekürzt | – |
| `docs/refs/rv1-remote-suite.md` | ja | – |
| `native/src/bisync/mod.rs` | State-/Index-Modulregistrierungen | – |
| `native/src/bisync/os/shared/state_types.rs` | ja | – |
| `native/src/bisync/os/shared/state_store.rs` | ja | geändert |
| `native/src/bisync/os/shared/state_validation.rs` | ja | geändert |
| `native/src/bisync/os/shared/index_review_tests.rs` | ja | geändert |
| `native/src/bisync/os/shared/tests/state_store.rs` | ja | – |
| `native/src/bisync/os/shared/engine_change_feed.rs` | ja | – |
| `native/src/bisync/os/shared/replica_state.rs` | ja | – |
| `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-E-ENGINE.md` | ja | erstellt |

Keine fehlende Definition, offene Schnittstellenabhängigkeit oder weitere Scope-Anfrage. Der belegte Block ist geschlossen; Root besitzt die gemeinsame Remote-Bestätigung.

