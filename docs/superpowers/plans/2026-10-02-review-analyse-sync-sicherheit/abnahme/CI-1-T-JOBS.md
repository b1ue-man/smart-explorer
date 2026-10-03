# CI-1-T-JOBS – SyncAttempt, Daemon-Shutdown und Catchup-Größengrenze

Stand: 2026-10-03. Auftrag ausschließlich aus [RV1-Lauf 37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629), zugeordnet über `/tmp/rv1-ci-first/t-jobs.json` und [ci-fixes.md](../ci-fixes.md). Der Remote-Formatterpatch war vor diesem Anschluss bereits angewandt. Kein neuer Projekt-Review und keine lokale Ausführung.

## Stage eins und begrenzte API-Lückenprüfung

Die übergebenen Compilerdiagnosen benennen genau zwei Grenzen: `sync_attempt::api_outcome` ruft `as_str()` auf einem bestehenden `&str` auf (E0658); `LanRuntime::shutdown` erreicht das in einem Childmodul definierte `UplinkRuntime::shutdown` mit dessen bisherigem `pub(super)` nicht (E0624, Linux und Windows). `catch_up.rs` hat nach dem übernommenen Formatterpatch 520 Zeilen / 17.476 Byte.

Der bestehende Catchup-Vertrag verwendet den vorhandenen `JobSupervisor`: regulär geplante Jobs bleiben dessen Eigentum, Retry/Backoff und Nutzeraktionen bleiben in der bestehenden Fälligkeitsprüfung. Das Buch verarbeitet Completion vor Zulassung/Cancel und nochmals danach. Die Ausgliederung darf diese Reihenfolge und die vorhandenen Testmodulpfade nicht verändern.

Das Manifest nennt zunächst drei nicht vorhandene Read-Pfade: `native/src/daemon/os/shared/lan_uplink_state.rs`, `native/src/mobile/os/shared/domains/sync.rs`, `native/src/syncjobs/core/job_state.rs`. Diese Fehlpfade erlauben keine alternative Erkundung. Der Hauptagent hat anschließend genau `lan_uplink_runtime.rs`, `catch_up_tests.rs` und `catch_up_outcome_tests.rs` zum Lesen ergänzt. Dadurch sind die tatsächliche Operations-Modulbindung und die bestehenden Fixture-Namenszugriffe statisch abgeglichen; keine weitere Lesefläche war nötig.

## Stage zwei: begrenzte Korrekturmeilensteine

| Grenze | Umsetzung | Konkretes Abnahmesignal im bestehenden RV1-Einstieg |
|---|---|---|
| SyncAttempt | Beide `ApiError.kind`-Vergleiche verwenden den bestehenden String direkt. Keine Konvertierung, neuer Fehlertyp oder veränderte Zuordnung. | E0658 entfällt; vorhandene Startfehler-/Bestätigungsassertions bleiben erhalten. |
| Uplink-Shutdown | Nur diese Methode wird innerhalb `crate::daemon` sichtbar. Ihr Körper, die 30-Sekunden-Frist und durable Stop-/Retry-Reihenfolge bleiben erhalten. | E0624 entfällt; tatsächlicher Daemon-Aufrufer bleibt gebunden, ohne öffentliche Bibliotheks-API. |
| Catchup | Zulassung, Fortschritt und Darstellung eines einzelnen Nachhollaufs werden kohäsiv in `catch_up_attempt.rs` verlagert. Buch, Completion-Zuordnung, Owner-/Cancel-Steuerung und Retry-Status bleiben im Elternmodul. | Beide Dateien bleiben mit Reserve unter 500 Zeilen und 50 KiB; dieselben vorhandenen Catchup-Assertions prüfen weiterhin denselben Vertrag. |

Der Dokumentationskontext kommt aus den übergebenen CI-Diagnosen, dem CI-Korrekturplan und den freigegebenen aktuellen Quellen/Refs. Die für diesen Worker ausdrücklich verbotenen Git-/Graph-/Formatter-/Testaktivitäten gehören zur Integration des Hauptagenten; sie wurden hier nicht ausgeführt. Die Stage-zwei-Entscheidung ist eine Fortführung der bereits dokumentierten CI-Korrekturplanung.

## Ergebnis und Entscheidungen

Umsetzung abgeschlossen, statischer Self-Review abgeschlossen. Kompilierung und Verhalten wurden hier nicht ausgeführt; die tatsächliche Abnahme bleibt beim Hauptagenten im selben vollständigen RV1-Einstieg.

- `sync_attempt::api_outcome` matcht `error.kind` direkt. `canceled`/`busy` bleiben `Cancelled`; `invalid` bleibt `Config`, `permission`/`access` bleiben `Access`, `internal` bleibt `Internal`, `hook` bleibt `Hook`; der bisherige Fallback klassifiziert unverändert die Fehlermeldung. Kein neuer String-/Wire-Vertrag und keine Allokation.
- `UplinkRuntime::shutdown` hat ausschließlich `pub(in crate::daemon)`. Das Operations-Childmodul liegt unter `daemon::lan_uplink_runtime`, der tatsächliche Aufrufer unter `daemon::lan_runtime`. Damit ist die Methode im gemeinsamen Feature erreichbar. Modul-/Plattformauswahl und öffentliche Reexports wurden nicht geändert.
- `catch_up_attempt.rs` besitzt `start_run`, `update_progress` und deren private Text-/Namenshelfer. Nur die beiden aufgerufenen Funktionen sind `pub(super)` und werden im bisherigen Elternmodul privat importiert. `Run`, `Admitted`, `Phase`, `Cancel`, `CatchUpBook`, Fälligkeitsprüfung und Testmodule bleiben an ihrer bestehenden Grenze. Die zuvor 101 Zeilen umfassenden Funktionskörper sind vollständig verlagert.

| Tatsächlich geänderte/erstellte Rust-Datei | Zeilen | Byte | Reserve zur Grenze von weniger als 500 Zeilen |
|---|---:|---:|---:|
| `native/src/mobile/os/shared/domains/sync_attempt.rs` | 363 | 13.247 | 136 |
| `native/src/daemon/os/shared/lan_uplink_operations.rs` | 287 | 11.334 | 212 |
| `native/src/daemon/os/shared/catch_up.rs` | 423 | 14.299 | 76 |
| `native/src/daemon/os/shared/catch_up_attempt.rs` | 107 | 3.549 | 392 |

Alle vier Dateien liegen unter 50 KiB. Die Tabellenwerte stammen ausschließlich aus statischer Textzählung; kein Formatter-/Compilerlauf wurde verwendet.

## Erhaltene Reihenfolge und statischer Self-Review

Der Textvergleich gegen die vor dem Edit gespeicherten, formattergepatchten Inhalte bestätigt genau die angekündigten Änderungen: SyncAttempt enthält nur die beiden entfernten `.as_str()`-Aufrufe; der Uplink-Operations-Text enthält nur die neue Shutdown-Sichtbarkeit. Catchup entspricht exakt dem bisherigen Elterntext abzüglich des verlagerten Blocks und zuzüglich seiner privaten Modulbindung. Der neue Helfer enthält exakt diesen Block zuzüglich eigener Imports, Modulbeschreibung und der beiden nötigen `pub(super)`-Sichtbarkeiten.

Die folgenden vorhandenen Vertragsfolgen bleiben erhalten:

1. Mobile Laufzulassung prüft Storagezugriff und Cancel vor RunMark/Heartbeat. Ein echter Versuch schreibt seinen Beginn, prüft Einstellungen, prüft Cancel, führt `Before` aus, öffnet das Paar, prüft Cancel und konsumiert erst dann die einmalige Bestätigung. Engine-Ergebnis, Konfliktzustand, `Cleanup`/`After`, terminale Cancel-Klassifikation und `record_attempt` behalten ihre bisherige Reihenfolge. Der Lease räumt weiterhin nur seine eigene RunMark ab.
2. `LanRuntime::shutdown` entfernt seine Link-Fakten und veröffentlicht leere Evidence, bevor es den Uplink beendet; anschließend wird Presence zurückgezogen. Der unveränderte Uplink-Körper wartet bis zu 30 Sekunden auf einen laufenden Start/Stop, behält bei ausstehendem Ergebnis die dauerhafte Absicht und deaktiviert erst danach physisch. Nur ein erfolgreicher Stop erreicht `finish_stopped`; dessen dauerhafte Speicherung liegt weiterhin vor Policy-/Statusbereinigung. Fehler behalten ihren vorhandenen sichtbaren/retryfähigen Zustand.
3. Catchup verarbeitet abgeschlossene Jobs vor Gate/Zulassung/Cancel und danach erneut. Bei geschlossenem Gate werden nur die eigenen unerledigten Jobs abgebrochen. Bei explizitem Cancel werden regulär geplante Jobs nur nicht mehr abgewartet; ihr Supervisor-Eigentum bleibt erhalten. Eine Completion beendet weiterhin höchstens die älteste offene eigene Zulassung sowie sämtliche rein abwartenden Zulassungen desselben Jobs.
4. Der Helfer selektiert weiterhin in gespeicherter Job-Reihenfolge und verwendet ausschließlich `queue.eligible`/`queue.admit`. `AlreadyScheduled` bleibt fremdes Eigentum, `RecentlyAttempted` bleibt sichtbare Ablehnung, fehlende Joblisten bleiben angefordert. Fortschritt und Finish verwenden unverändert tatsächliche `done`-/`ran`-/Aktivzustände. Backoff, Nutzeraktionen, Startup-/Connect-Ursache einschließlich Volume und `retry_suggested` bleiben in den bestehenden Eltern-/Supervisor-Verträgen.

Die bestehenden Fixture-Dateien wurden nur gelesen. Ihre Testnamen, Assertions, Elternimports und `#[cfg(test)]`-/`#[path]`-Registrierungen bleiben erhalten. Es wurden keine neuen Szenarien oder Tests geschrieben.

## Konkrete Abnahmesymbole

Der Hauptagent führt ausschließlich die bereits definierte vollständige RV1-Suite über `review-task.yml` / `native/test-review-task.sh` erneut aus. Für diesen Anschluss gelten die vorhandenen Linux-/Windows-/Android-Buildinputs: kein E0658 bei `sync_attempt::api_outcome` und kein E0624 beim tatsächlichen Daemon-Shutdown-Aufruf. Die vorhandene Größengrenze erfasst beide Catchup-Dateien.

Unveränderte vorhandene Testfunktionen zur Vertragsabnahme:

- `android_sync_start_errors_keep_the_shared_failure_kind`
- `android_sync_confirmation_consumed_once_preserves_later_change`
- `android_task_catch_up_selects_due_timers_missed_calendar_and_realtime_once`
- `android_task_catch_up_run_finishes_when_admitted_jobs_are_done`
- `android_task_catch_up_lists_supervisor_rejections_with_reason`
- `android_task_catch_up_cancel_touches_only_this_runs_jobs`
- `android_task_catch_up_closed_gate_ends_open_runs_with_reason`
- `android_task_catch_up_reports_load_failures_and_bounds_history`
- `review_task_catch_up_aggregates_failures_and_retries_only_transient_errors`
- `review_task_empty_cancelled_or_interrupted_windows_do_not_replace_the_last_run`
- `review_task_catch_up_owns_saved_startup_and_connect_triggers_and_respects_auth_backoff`

Diese Liste benennt die vorhandenen Assertions und ihre konkrete Zuordnung; sie ist kein neuer Testeinstieg und kein Ausführungsnachweis.

## Exaktes Dateiinventar

Gelesen, bei Quellen/Refs teilweise nur gezielte Ausschnitte oder Definitionen:

```text
/tmp/rv1-ci-first/t-jobs.json
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-1-t-jobs.json
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fixes.md
docs/refs/egui-close-lifecycle.md
docs/refs/local-fs-identity-durability.md
docs/refs/rv1-remote-suite.md
docs/refs/share-server-tls-auth.md
native/src/bisync/core/run_types.rs
native/src/daemon/mod.rs
native/src/daemon/os/shared/catch_up.rs
native/src/daemon/os/shared/catch_up_tests.rs
native/src/daemon/os/shared/catch_up_outcome_tests.rs
native/src/daemon/os/shared/guardian.rs
native/src/daemon/os/shared/lan_runtime.rs
native/src/daemon/os/shared/lan_uplink_operations.rs
native/src/daemon/os/shared/lan_uplink_runtime.rs
native/src/mobile/os/shared/domains/sync_attempt.rs
native/src/syncjobs/core/types.rs
native/src/syncjobs/mod.rs
native/src/daemon/os/shared/catch_up_attempt.rs (eigene neue Datei)
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-T-JOBS.md (eigener Bericht)
```

Exakte erfolglose Lese-/Zählversuche auf im Manifest genannte, nicht vorhandene Dateien:

```text
native/src/daemon/os/shared/lan_uplink_state.rs
native/src/mobile/os/shared/domains/sync.rs
native/src/syncjobs/core/job_state.rs
```

Geändert:

```text
native/src/mobile/os/shared/domains/sync_attempt.rs
native/src/daemon/os/shared/lan_uplink_operations.rs
native/src/daemon/os/shared/catch_up.rs
```

Erstellt:

```text
native/src/daemon/os/shared/catch_up_attempt.rs
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-T-JOBS.md
```

Keine weiteren Dateien geändert; insbesondere bleiben Daemon-Fassade, Caller, Tests, globaler Plan, Scope und Suite diesem Worker gegenüber read-only.

## Offene Fremdgrenzen

Die benötigten Anschluss-Reads sind freigegeben und verbraucht; keine implementierungsblockierende Scope-Lücke verbleibt. Die drei nicht vorhandenen ursprünglichen Manifestpfade bleiben als nicht blockierende Inventarabweichung gemeldet. Commit, Push, Formatter-/Compiler-/Laufzeitabnahme, Rootgraph und etwaige weitere echte RV1-Diagnosen gehören zum Hauptagenten. Keine lokale Ausführung, Git-/CI-/Graph-/Releaseaktion, Installation, Serverstart oder Agentendelegation vorgenommen. Nach dieser Übergabe stoppt der Block.
