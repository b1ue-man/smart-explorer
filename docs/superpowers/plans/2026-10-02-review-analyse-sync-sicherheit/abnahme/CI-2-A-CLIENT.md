# CI-2-A-CLIENT – bestehende GUI-/Job-Locatorfixtures

Stand: 2026-10-03. Enge Korrekturen aus
[Run 37150409255](https://github.com/b1ue-man/smart-explorer/actions/runs/37150409255),
Kandidat `ac9b475ff18f6320bedd408c5a03c091710ad01c`, nach
[ci-2-a-client.json](../scopes/ci-2-a-client.json) und dem eigenen Abschnitt in
[ci-behavior-fixes.md](../ci-behavior-fixes.md). Kein neuer Projekt-Review,
keine neue unabhängige Fixture und keine lokale Ausführung. Root besitzt
weiterhin denselben vollständigen Remote-RV1-Einstieg.

## Frischer API-Abgleich und enger Plan

Die konkrete Diagnosenliste `/tmp/rv1-ci-second/a-client.json` wurde frisch
gelesen. Fünf Fälle nennen direkt `private parent has no pinned path`, ein
gespeicherter lokaler Job erwartet den alten Connector-Receiver, die
Migrationsfixture enthält einen nach aktuellem Vertrag leeren Namen und der
Split-Fall erreicht einen `NotFound` beim zwingenden Ergebnis-Dateilesen.

Vor dem Editieren wurden eigene Planzeile, aktuelle Suite-/Dateisystemrefs,
Jobvalidierung, tatsächlicher Desktop-Run-Produzent und Run-/Identitytypen
abgeglichen. Root ergänzte zwei enge Read-Handoffs für die konkreten
Backend-/Uncache-/Outcome-/Identity-Migrationssymbole; keine alternative
Datei oder nicht freigegebene Produktion wurde erkundet.

Korrekturreihenfolge und erwartetes Signal im selben Remote-Einstieg:

1. Gültige Eingabe im vorhandenen Delete-Guard-Migrationsszenario: ein
   nichtleerer Name; alle bisherigen Migrations-/Explizit-null-Assertions
   bleiben zwingend.
2. Gespeicherter GUI-Job: aktueller Desktop-Run samt Receiver,
   Cancellationhandle und konkreter Job-ID statt des nicht mehr verwendeten
   Connector-Receivers; dieselben lokalen Ergebnisbytes und Overlapprüfung.
3. Split-Szenario: aktuelle getrennte Namespace-/Cache-/Uncacheverträge
   zusätzlich festhalten, zwingende Publikations-/Byteprüfung erhalten und
   deren Fehlermeldung um die tatsächliche GUI-Projektion erweitern.
4. Die fünf privaten Windows-Parentfehler bleiben an ihren vorhandenen
   Assertions sichtbar. Ihre Produktionsbasis gehört dem V-LOCAL-Block;
   kein Fixture-Opt-out und keine abgeschaltete Schutzprüfung.

## Umsetzung / Diagnosezuordnung

| Bestehendes ausgewähltes Symbol | Konkreter Befund und Änderung / Abhängigkeit |
|---|---|
| `sync_paths_task_saved_local_job_uses_worker_resolution_and_preserves_old_behavior` | `App::run_job` ruft heute unmittelbar `start_saved_desktop_run` auf; `drain_job_connect` ist ein No-op. Die obsolete Erwartung `job_connect_rx.is_some()` wird durch `bisync_running`, `desktop_run`, `bisync_rx`, `bisync_cancel` und exakt `running_job == id` ersetzt. Quell-/Zielkonfiguration, persistierter Auftrag, 30-s-Drainfrist, Dateiinhalt und lokale Overlapverweigerung bleiben unverändert. |
| `review_task_delete_guard_migrates_once_and_preserves_explicit_later_zero` | Nur der vorhandene `id=explicit`-Datensatz erhält `name=X`. `SyncJob::validate_identity` verlangt einen nichtleeren Namen mit höchstens 4096 Bytes. Endpunkte, explizite 20 %, spätere explizite Null, Mountverhalten und alle Defaults-/Versionsassertions bleiben unverändert. |
| `sync_paths_task_split_same_paths_on_different_remotes_and_uncached_metadata` | Die Fixture prüft zusätzlich verschiedene Namespace-IDs, beide Identitäten des Cachewrappers und echte `Arc::ptr_eq`-Rückgabe von `sync_backend(cached)` auf das vorhandene Livebackend. Der zwingende `fresh.txt`-Read mit identischer Byteassertion bleibt erhalten; ein Fehler zeigt jetzt die tatsächlichen `app.notice`/`app.error_msg` statt nur `NotFound`. Keine eindeutige Ursache des ursprünglichen NotFound ist statisch nachgewiesen; der Remote-Nachweis bleibt offen. |
| `sync_paths_task_all_backend_pairs_transfer_changes_and_keep_baselines_separate` | Direkter privater Parentfehler vor dem Transfer; unverändert, abhängig von V-LOCAL. Alle sechs Backendklassen in sämtlichen Paarungen bleiben enthalten. |
| `sync_paths_task_picker_setup_and_quick_actions_retain_remote_provenance` | Privater Parentfehler beim Abschluss der Mirror-Versionen; unverändert, abhängig von V-LOCAL. Alle sieben vorhandenen Locatorpräfixe, Remote-Pickerwert und beidseitige Dateiassertions bleiben enthalten. |
| `sync_paths_task_real_share_cross_peer_sync_and_local_roundtrip` | Privater Parentfehler in der Backend-Identitätsvorbereitung; unverändert, abhängig von V-LOCAL. Beide echten Peerfixtures, Unicode-/Literaldateien und Local-Roundtrip bleiben zwingend. |
| `sync_links_task_saved_job_and_gui_retain_partial_result_notice` | Privater Parentfehler in der Backend-Identitätsvorbereitung; unverändert, abhängig von V-LOCAL. GUI-Hinweis, persistierte Auslassungsnotiz, null Fehler, normales Ergebnis und echte Child-Linkgrenze bleiben zwingend. |
| `sync_links_task_cross_remote_contract_protects_counterpart_subtree` | Privater Parentfehler in der Backend-Identitätsvorbereitung; unverändert, abhängig von V-LOCAL. SFTP→WebDAV, `node_modules`-Auslassung, normale Literaldatei, erhaltener Gegenbaum und kein Schreiben außerhalb der Wurzel bleiben zwingend. |

## Entscheidungen und erhaltene Verträge

- `MappedBackend::state_identity` enthält bereits Scheme und den eigenen
  isolierten Temp-Backendpfad; `Backend::namespace_identity` delegiert daran.
  Gleichnamige `/Docs Ü %20 #`-Ordner verschiedener Remotes sind weiterhin
  verschiedene Orte. Kein Namespacepatch und keine Änderung am Mapper.
- `CachingBackend` delegiert beide Identitäten und liefert über
  `uncached_backend` den Livebackend. `sync_backend` verwendet genau diese
  Facade. Die Fixture beweist dies im vorhandenen Split-Szenario zusätzlich;
  sie leert keinen Cache und ersetzt keinen Remote durch eine lokale Route.
- `Outcome::{busy,blocked,stopped,canceled}` sind aktuelle eigene
  Laufzustände. `busy` und `blocked` sind ausdrücklich keine Fehler. Die
  Identitymigration darf Locks mit `WouldBlock` verweigern; keine Freigabe,
  Bestätigung, Lockwarteverlängerung oder Erfolgssimulation ergänzt.
- Die fehlende Split-Ergebnisdatei bleibt ein harter Fehler. Eine andere
  Run-Projektion wird weder stillschweigend akzeptiert noch als erfolgreiche
  Kopie verbucht. Die zusätzliche GUI-Diagnostik liefert im selben echten
  Remote-Lauf die bisher fehlende Abgrenzung.
- Keine Produktionsdatei, Locatorpersistenz, Benutzerrechte, Cancellation,
  Baseline-, Backup-, Replica-, Link-, Quarantäne- oder Teilresultatsemantik
  geändert. Keine Schutzassertion entfernt oder gelockert.

## Read-Inventar

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-2-a-client.json` (einschließlich Root-Read-Ergänzungen).
- `/tmp/rv1-ci-second/a-client.json`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-behavior-fixes.md`.
- `docs/refs/rv1-remote-suite.md` (frisch).
- `docs/refs/local-fs-identity-durability.md` (frische Überschriften-/Linksuche und Abschnitt Windows-Dateinamen).
- `native/src/app/core/sync_core.rs` (Mirror-/Bisync-/Pane-/Split-Anschlüsse).
- `native/src/app/core/sync_run_state.rs`.
- `native/src/app/mod.rs` (nur konkrete Modul-/Fassadenregistrierungen).
- `native/src/app/os/shared/sync_jobs.rs`.
- `native/src/app/os/shared/sync_links_task_tests.rs`.
- `native/src/app/os/shared/sync_manual_run.rs`.
- `native/src/app/os/shared/sync_paths_task_fixture.rs`.
- `native/src/app/os/shared/sync_paths_task_tests.rs`.
- `native/src/bisync/core/run_types.rs`.
- `native/src/bisync/mod.rs` (aktuelle Reexports / benannte Modulpfade).
- `native/src/bisync/os/shared/orchestration.rs` (nur Outcome, RunRequest und frühe Rückgaben).
- `native/src/bisync/os/shared/backend_identity_migration.rs` (nur `migrate` / Identity- und Locksemantik).
- `native/src/syncjobs/core/types.rs` (gezielter Modell-/Default-/Optionsabgleich).
- `native/src/syncjobs/core/validation.rs` (gezielt `validate_identity`/`validate_text`).
- `native/src/syncjobs/os/shared/persistence_review_tests.rs`.
- `native/src/vfs/core/core.rs` (nur Backend-Scheme-/Root-/Identity-/Uncache-Defaults).
- `native/src/vfs/core/cache.rs` (nur die angefragten Cache-/Identity-/Uncache-/Listingmethoden).
- `native/src/vfs/mod.rs` (Facade / benannte Modulpfade).
- `native/src/vfs/os/shared/sync_roots.rs` (`sync_backend`/`validate_sync_roots`).
- Dieser neue Abnahmebericht beim Self-Review.

## Modify-/Create-Inventar

Geändert ausschließlich:

- `native/src/app/os/shared/sync_paths_task_tests.rs`.
- `native/src/syncjobs/os/shared/persistence_review_tests.rs`.

Erstellt ausschließlich
`docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-A-CLIENT.md`.
Die erlaubten `sync_paths_task_fixture.rs` und `sync_links_task_tests.rs`
bleiben unverändert. Keine andere Quelle, Registrierung oder Dokumentation
geändert.

## Eigener statischer Self-Review / offene Abhängigkeiten

Der Self-Review gleicht die eigenen Deltas mit dem gespeicherten
Ausgangstext und den frisch gelesenen Definitionen ab. Die bestehende
veraltete Connector-Receiverassertion wird fachlich durch die belegten
aktuellen Desktop-Run-Felder ersetzt; die übrigen ursprünglichen Assertions
werden vollständig erhalten. Die Split-Byteassertion hat nur ausführlichere
Fehlerdiagnostik. Der exakte Quellvergleich bestätigt ausschließlich die
benannten Ersetzungen und Ergänzungen; Mapper und Linkfixture sind bytegleich
zum Ausgangstext. Statische Rust-Lexik-/Delimiter-, Whitespace-/EOF- und
Scope-/Größenprüfungen sind abgeschlossen und sauber. Die beiden geänderten
Rustdateien haben 172 Zeilen / 9.247 Bytes beziehungsweise 187 / 7.098 und
bleiben unter 500 Zeilen / 50 KiB. Dies ist kein Compiler- oder Testnachweis.

Offen bei V-LOCAL/Root: der echte private Windows-Parent-/Handleanschluss
für die fünf direkt benannten Kaskaden. Offen bei Root nach dessen Integration:
derselbe Split-Fall muss die echte frische Datei liefern; der genaue frühere
NotFound-Grund bleibt statisch uneindeutig. Die neue Diagnostik darf im selben
Remote-RV1-Lauf weder übersprungen noch als Erfolg gewertet werden. Falls
die integrierte Handlebasis diesen Fall nicht schließt, benötigt Root die
aktuelle GUI-Meldung aus genau diesem Szenario für eine eng zugewiesene
Produktionskorrektur. Es wird kein zusätzlicher Suiteeintritt angelegt.

Keine lokale Ausführung, Compiler, Formatter, Suite, Server, Installation,
Git-, CI-, Graph-, Release- oder Unteragentoperation. Keine offene
Lesefreigabe nötig; keine nicht genehmigte Produktionsänderung vorgenommen.
