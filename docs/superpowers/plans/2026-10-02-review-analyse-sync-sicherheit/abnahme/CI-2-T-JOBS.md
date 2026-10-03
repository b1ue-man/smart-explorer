# CI-2-T-JOBS – Quick-Mirror-Preflight und vollständige Delete-Sperre

Stand: 2026-10-03. Enger Folgeauftrag aus [RV1-Run 37150409255](https://github.com/b1ue-man/smart-explorer/actions/runs/37150409255), Kandidat `ac9b475ff18f6320bedd408c5a03c091710ad01c`. Grundlage sind `/tmp/rv1-ci-second/t-jobs.json`, der frisch gelesene T-JOBS-Abschnitt in [ci-behavior-fixes.md](../ci-behavior-fixes.md) und ausschließlich der gespeicherte Scope `scopes/ci-2-t-jobs.json`. Keine Projektprüfung, neue Testanlage oder lokale Ausführung.

## Stage eins: belegte Grenzen

Die bestehende Fixture `mirror_cancel_during_preflight_deletes_nothing` erreicht ihre Null-Delete-Assertion, aber nicht die geforderte sichtbare Fehlermeldung. Der Delete-Preflight unterdrückt `Interrupted`. Ein tolerantes leeres Listing kann zudem Cancel während des Listings setzen, ohne den bisherigen Gate-Aufruf pro Eintrag zu erreichen.

`source_appearing_after_preflight_blocks_all_deletes` meldet zwei Deletes statt null. Vor dem ersten Delete werden die Ziel-Metadaten, aber nicht sämtliche Quellabwesenheiten erneut geprüft. Der vorhandene Appearance-Hook injiziert zugleich über Blatt-`stat`, während die echte normalisierte Abwesenheitsprüfung jedes relative Pfadsegment über `vfs::list_dir_tolerant` prüft.

Die eng angefragten Definitionsdateien `vfs/core/extension_calls.rs` und `bisync/os/shared/apply_boundary.rs` wurden vom Hauptagenten zum Lesen freigegeben und frisch abgeglichen. Die Listing-Fassade verwendet vorhandene `BackendExtensions::list_dir_tolerant` beziehungsweise den dokumentierten `Backend::list_dir`-Fallback. Es wird kein alternativer Listing-/Statvertrag erfunden. Geschützte Einträge, Auslassungen, Kollisionen und literalgetreue Providerpfade bleiben an diesen vorhandenen Grenzen.

## Stage zwei: begrenzte Meilensteine

| Grenze | Konkrete Umsetzung | Erwartung in derselben vollständigen Remote-RV1-Suite |
|---|---|---|
| Cancellation im Preflight | `Interrupted` bleibt sichtbar; Gates nach toleranter Enumeration, vor Walk-Rückgabe und um die normalisierte Abwesenheitsprüfung. | Bestehende Cancel-Fixture behält `deleted == 0`, eine sichtbare Fehlermeldung und unveränderte Gegenstückbytes. |
| Quelle nach erfasstem Plan | Vor dem ersten destruktiven Schritt sämtliche Kandidaten auf Zielzustand und frische normalisierte Quellabwesenheit prüfen; bei jeder Unsicherheit gesamte Deletephase beenden. | Bestehende Appearance-Fixture behält `deleted == 0` und beide Zielgegenstücke, unabhängig von der Kandidatenreihenfolge. |
| Tatsächlich konsumierter Fixture-Vertrag | Der vorhandene Listing-Hook lässt bei der zweiten Quell-Listingrunde eine echte Datei erscheinen; der nicht konsumierte Blatt-Stat-Hook entfällt. | Der bestehende Test belegt die reale neue Quelle und die beibehaltenen Zielbytes; kein neuer Testsymbol-/Suite-Einstieg. |

Copy-Pass, parallele/streamende Transfers, PairLock, Versionen, geschützte Auslassungen, `quick_delete` und dessen unmittelbar vor Apply konsumierte Abwesenheitsclosure bleiben erhalten. Es wird keine Anwendung durch rohe Dateilöschung ersetzt. Umsetzung und statischer Self-Review sind abgeschlossen; die tatsächliche Remote-Abnahme bleibt offen beim Hauptagenten.

Der Dokumentationskontext ist der übergebene tatsächliche CI-Befund samt aktuellem Plan, frisch gelesenen Refs und Quellen. Die für diesen Worker ausdrücklich verbotenen Git-/Graph-/Formatter-/Testaktivitäten verbleiben beim Hauptagenten.

## Ergebnis und Entscheidungen

- `sync_delete.rs` protokolliert auch `Interrupted` aus dem Preflight als sichtbaren fehlgeschlagenen Delete-Preflight. Danach kehrt die gesamte Deletephase ohne Delete zurück. Vor Dry-run-Zählung oder realer Anwendung werden sämtliche Kandidaten erneut auf Zielzustand und normalisierte Quellabwesenheit geprüft. Cancel, aufgetauchte Quelle oder unsichere Beobachtung beendet die gesamte Deletephase. Ein dabei erkannter geschützter Pfad wird weiterhin mit seinem tatsächlichen OmissionKind festgehalten.
- `sync_delete_walk.rs` prüft Cancel unmittelbar nach erfolgreicher toleranter Enumeration und vor erfolgreicher Walk-Rückgabe. `missing` prüft Cancel vor und nach dem vorhandenen `normalized_missing`-Aufruf. Auch die Abwesenheitsantwort aus einem leeren Listing kann damit kein inzwischen gesetztes Cancel übergehen. Signaturen, Budgets, Pfad-/Key-Regeln und Auslassungsbehandlung bleiben erhalten.
- Der bestehende `HookBackend` in `sync_tests.rs` erzeugt beim zweiten tatsächlich konsumierten Listing die echte Quelle `appears.txt` mit den Bytes `appeared`. `stat` delegiert wieder unverändert an den echten lokalen Backend. Der Cancel-Hook setzt seinen Marker nach dem tatsächlichen Listing mit `Release`; die bestehenden Gates verwenden `Acquire`. Die vorhandenen Assertions wurden beibehalten und um Hook-Nachweis, sichtbare Fehlermeldung beziehungsweise tatsächliche Dateibytes ergänzt.
- `sync_scan.rs` und der Copy-/Transferablauf sind unverändert. Es gibt keinen neuen Listing-Fallback, Endpointparser, Backendtyp, öffentlichen Vertrag, Testnamen oder Suite-Einstieg.

Die zusätzliche Abwesenheitsrunde verwendet dieselbe autoritative Key-/Pfad-/Tolerant-Listing-Grenze wie der vorhandene unmittelbar vor Apply konsumierte Check. Eine während dieser Runde neu auftauchende Quelle blockiert auch zuvor bereits validierte andere Kandidaten: Die komplette Validierung liegt vor jedem Delete. Wenn der neue Pfad beim ersten Kandidaten noch keinen passenden Namen hat, findet ihn der spätere passende Kandidat immer noch vor dem ersten Delete. Die Fixture hängt daher nicht von der Enumeration der beiden Zieldateien ab.

## Erhaltene Reihenfolge und statischer Self-Review

Der Ablauf bleibt: PairLock/Versionen beginnen → vollständige geschützte Quell-/Zielbeobachtung → Kandidaten bestimmen → vorhandene Reihenfolge für Dateien und tiefe Verzeichnisse → vollständige Ziel-/Quellrevalidierung → Dry-run oder Apply. Dateien behalten ihren bestehenden `quick_delete`-Aufruf mit Signatur, Versionen, Cancelmarker und frischer Abwesenheitsclosure. Verzeichnisse behalten Identitätsprüfung, tolerantes leeres Listing, erneute Abwesenheitsprüfung, Cancel-Gate, nichtrekursives Entfernen und Namespace-Durability. Auslassungen schützen weiterhin ihre Gegenstücke; Terminalfehler und tatsächliche Teilresultate bleiben an den vorhandenen Apply-Grenzen.

Der statische Textvergleich gegen die vor dem Edit gespeicherten aktuellen Inhalte bestätigt:

- Der gesamte `sync_delete.rs`-Text ab `if dry_run` einschließlich Apply-/Backup-Aufruf, per-Delete-Revalidierung, Ergebnisbehandlung und Metadatenhelfern ist identisch.
- `sync_scan.rs` ist vollständig identisch. Der vorhandene Streaming-/Parallel-Copy-Pass wurde nicht verändert oder durch eine eigene Kopierimplementierung ersetzt.
- Alle bisherigen freien Fixture-/Testsymbole in `sync_tests.rs` sind erhalten. Der Dateiteil vor `HookBackend` sowie beide bestehenden Link-/Victim-Fixtures danach sind identisch. Änderungen liegen ausschließlich am vorhandenen Hook und den beiden zugeordneten Fehlerfällen; keine Assertion wurde entfernt oder abgeschwächt.
- Die neu hinzugefügten Cancel-Gates liegen vor erfolgreicher Rückgabe; die gesamte neue Kandidatenvalidierung liegt vor der ersten destruktiven Anwendung. Normalisierte Quelle, literalgetreue Providerpfade und `AlreadyExists` bleiben echte Beobachtungen, keine erfolgreiche Fehlbaseline.

| Geänderte Rust-Datei | Zeilen | Byte | Reserve bis maximal 499 Zeilen |
|---|---:|---:|---:|
| `native/src/sync/os/shared/sync_delete.rs` | 272 | 9.937 | 227 |
| `native/src/sync/os/shared/sync_delete_walk.rs` | 140 | 4.702 | 359 |
| `native/src/sync/os/shared/sync_tests.rs` | 458 | 14.483 | 41 |

Die Größen stammen ausschließlich aus statischer Textzählung. Alle geänderten Rust-Dateien bleiben unter 500 Zeilen und 50 KiB. Keine Compiler-/Formatter-/Test-/Serverprozesse; ausschließlich statische Textarbeit.

## Konkrete Abnahme

Unveränderte volle Testsymbole aus der CI-Evidenz, ausschließlich innerhalb derselben Root-verantworteten vollständigen Remote-RV1-Suite:

| Bestehendes Symbol | Beibehaltene und verstärkte konkrete Assertions |
|---|---|
| `sync::imp::tests::mirror_cancel_during_preflight_deletes_nothing` | Tatsächlicher Hook setzt Cancel; `stats.deleted == 0`; sichtbarer Fehler und Fehlerzähler; `orphan.txt` existiert weiterhin mit Bytes `x`. |
| `sync::imp::tests::source_appearing_after_preflight_blocks_all_deletes` | Mindestens zwei echte Quell-Listingrunden; `stats.deleted == 0`; sichtbarer Revalidierungsfehler; die reale Quelle enthält `appeared`, beide bestehenden Zielgegenstücke behalten `x` beziehungsweise `y`. |

Die tatsächliche vollständige RV1-Ausführung einschließlich ihrer bestehenden Streaming-/Backups-/Link-/Auslassungsabnahme übernimmt der Hauptagent. Hier sind ausschließlich Implementierung und statischer Self-Review belegt; kein lokaler oder neuer separater Testlauf wurde ausgeführt.

## Exaktes Dateiinventar

Gelesen, bei Plan/Refs und VFS-Definitionen teilweise gezielte Abschnitte oder Definitionen:

```text
/tmp/rv1-ci-second/t-jobs.json
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-2-t-jobs.json
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-behavior-fixes.md (eigener T-JOBS-Abschnitt/Evidenzrahmen)
docs/refs/local-fs-identity-durability.md
docs/refs/rv1-remote-suite.md
native/src/sync/mod.rs
native/src/sync/os/shared/sync.rs
native/src/sync/os/shared/sync_delete.rs
native/src/sync/os/shared/sync_delete_walk.rs
native/src/sync/os/shared/sync_scan.rs
native/src/sync/os/shared/sync_tests.rs
native/src/vfs/core/core.rs
native/src/vfs/core/extension_types.rs
native/src/vfs/core/extensions.rs
native/src/vfs/core/extension_calls.rs (eng nachgereichte Definition)
native/src/vfs/mod.rs
native/src/bisync/os/shared/apply_boundary.rs (eng nachgereichte Definition)
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-T-JOBS.md (eigener neuer Bericht)
```

Geändert:

```text
native/src/sync/os/shared/sync_delete.rs
native/src/sync/os/shared/sync_delete_walk.rs
native/src/sync/os/shared/sync_tests.rs
```

Erstellt:

```text
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-T-JOBS.md
```

Keine weiteren Dateien geändert oder angelegt. Keine alternative Erkundung außerhalb des gespeicherten Scopes.

## Offene Abhängigkeiten

Die einzige angefragte Definitionsgrenze wurde vom Hauptagenten eng freigegeben und ist abgeschlossen; keine weitere API-/Lesefreigabe oder Produktänderung außerhalb des Scopes ist nötig. Commit/Push, Rootgraph und tatsächliche Abnahme im selben vollständigen Remote-RV1-Einstieg bleiben beim Hauptagenten. Keine Git-/CI-/Graph-/Releaseaktion, lokale Ausführung, Installation, Testneuanlage oder Delegation. Dieser Block stoppt nach der Übergabe.
