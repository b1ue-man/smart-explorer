# CI-4-T-JOBS – Android-Background- und Saved-Sync-Fixtureisolation

Stand: 2026-10-04. Enger Folgeauftrag aus [RV1-Run 37162485159](https://github.com/b1ue-man/smart-explorer/actions/runs/37162485159), Kandidat `ac3b0c9098963fae386e94558f2f3ca1bb240740`. Umsetzung und eigener statischer Self-Review abgeschlossen; Geräteabnahme noch offen. Grundlage: eigener T-JOBS-Abschnitt in [ci-fourth-fixes.md](../ci-fourth-fixes.md), gespeicherter Scope, darin erlaubte Geräteevidenz und frisch abgeglichene Definitionen. Keine neue Projektprüfung oder lokale Ausführung.

## Stage eins: konkrete Evidenz und erhaltenes Verhalten

- `background-worker.log`: `catchUpAdmitsADueIntervalJob` erhält `admitted=0`, `skipped=[]`, `Keine fälligen Jobs`. Der neue Job zählt tatsächlich ab seiner ID-Erzeugungssekunde; ein gerade gespeicherter Fünf-Minuten-Job ist noch nicht fällig. Ein fremder Aggregaterfolg belegt den eigenen Job nicht.
- Derselbe Log: In der zweiten periodischen Runde steht WorkInfo noch auf `RUNNING`, obwohl der Core-Catchup bereits beendet ist. `SyncWorker` erledigt danach erst Host-/Zeitplan-Cleanup. Die bestehende erste Runde wartet schon auf `ENQUEUED`; die zweite tat dies nicht.
- `sync-sharedStorageRevokeStopsOnlySharedJobs.log`: Der private Job liegt im gesamten geschützten Host-Cache; sein tatsächliches Ergebnis meldet `OwnFile`. Appprivate Nutzdaten müssen außerhalb Engine/Cache liegen. `snapshot_policy::own_path` und das Android-Storage-Predicate bleiben unverändert.
- `merge-prepare.log`: Die echte bestätigte erste Merge-Seite bleibt zwingend. Namespace-/Durabilityproduktion gehört E-ENGINE. Der Test akzeptiert keine fehlgeschlagene erste Veröffentlichung als bestätigten Teilerfolg.

Alle bisherigen Testmethoden/Fälle bleiben erhalten. Dazu gehören der reale Core/JNI-Vertrag, gespeicherte Jobzustände, beide periodischen Signale und Constraints, Shared-Entzug/private Unabhängigkeit, Versionen/Restore, Recorded-Merge-Partialreport und der echte FTP-/Prozessneustartpfad. Keine Produktion, UI, CI, Release oder Testklasse wurde erweitert.

## Stage zwei und zweiter begrenzter Lückenabgleich vor den abhängigen Edits

| Meilenstein | Gelesener Vertrag und Entscheidung | Konkretes bestehendes Abnahmesignal |
|---|---|---|
| Eigener fälliger Intervalljob | `due::anchor` verwendet letzte erfolgreiche Ausführung oder tatsächliche hexkodierte Erzeugungsnanosekunden. `intervalMin=5` bleibt unverändert; der Test wartet echte Fälligkeit, ohne ID/State zu manipulieren. | Eigene ID, vorher kein Attempt/Success/Runmark, eigener neuer Erfolg mit `lastCause=catch_up`, fehlerfreier Zustand und exakte UTF-8-Zielbytes; Admission oder tatsächlicher Skip mit eigener ID/Name/nichtleerem Grund. |
| Racefreie Fensterisolation | `BackgroundController.apply(MODE_OFF)` schließt die bestehenden Periodic-/Scheduled-/Content-Fenster. `state.rs`/`run_loop.rs`/`live.rs` belegen: Hostdeferral sperrt reguläre Admission, explizites `bg.catchUp` bleibt bei SyncEnabled und erlaubter Mutation offen. | Vor Setup/Cleanup sind periodische WorkInfo und Core-Catchups terminal, CatchUpRunning/SyncEnabled aus und reale Worker-Windows null. |
| Worker bis tatsächliches Ende | `SyncWorker` schließt den Host-Window erst in `finally`. WorkManager-Cancellation wartet diesen nicht ab. `HostMonitor.workerRuns` ist der reale AtomicInteger-Produzent; `measure().deferScheduling` wäre wegen seines zusätzlichen Hintergrund-/Moduszweigs kein Null-Nachweis. | Beide Core-Runden erreichen `done`, anschließend dieselbe WorkInfo `ENQUEUED` und workerRuns null; final dieselben Constraints/60-Minuten-Periode. |
| Zulässige private Fixture | `Core.initialize` delegiert an `InitConfig.build`, das echte `app.filesDir`/`app.cacheDir` übergibt. `support_dirs` legt Engine-Daten unter `<data_home>/smart_explorer` ab; Android unterscheidet appprivate von Shared. | Private Nutzdaten unter `filesDir/review-sync-<eigener Fall>` schreiben trotz Shared-Entzug; Shared wird abgebrochen und kann nach Freigabe erneut laufen. |

Alle fehlenden verbundenen Definitionen wurden exakt beim Root angefragt und vor den jeweiligen abhängigen Edits freigegeben/gelesen. `sync_state_json::attach` serialisiert `last_cause` direkt. Root bestätigte nach eigener aktueller Lektüre von `syncjobs/os/shared/job_state.rs:294–320` den Enumvertrag `#[serde(rename_all = "snake_case")]` plus `CatchUp`: Der erwartete Wert lautet genau `catch_up`. Diese Enumdatei wurde durch diesen Worker nicht gelesen.

Die bestehende Instrumentierungsfixture liest ausschließlich den bereits freigegebenen privaten `AtomicInteger workerRuns` per `getDeclaredField("workerRuns")`. Ein fehlendes/umbenanntes Feld scheitert sichtbar. Keine neue Produkt-API, kein UI-Start und keine willkürliche Ruhepause.

## Umgesetzte Änderungen und Entscheidungen

`BackgroundTaskTest.kt`: Eigene eindeutige Quelle/Ziel unter einem Fallroot, echte fünfminütige Fälligkeit, strenge eigene Job-ID-/State-/Byteassertions. Der Admission-/Skip-Vertrag bleibt; Skip benötigt nun die eigene ID und einen echten Grund. Native Task, eigener Runmark, Job und ausschließlich eigener Verzeichnisbaum werden in `finally` aufgeräumt. `Files.walk` verwendet kein FOLLOW_LINKS. Cleanup ist nicht abbrechbar und begrenzt; ein sekundärer Cleanupfehler wird am ursprünglichen Fehler erhalten.

Die bestehende WorkManager-Fixture behält denselben TestDriver und beide Constraints-/Periodensignale. Beide Runden folgen nun Core=`done` → WorkInfo=`ENQUEUED` + reales Window null. Der tatsächliche SyncEnabled-Flag wird vor den Signalen abgewartet. Die Isolation schließt beim Eintritt und im Cleanup die vorhandenen Hintergrundfenster, setzt gemessenen Hostdeferral und stellt geänderte Work-Preferences, Pause/Autopause sowie den gemessenen Hostzustand wieder her. Der gespeicherte `bgMode` bestimmt über den bestehenden `BackgroundController.apply` den wiederhergestellten nativen SyncEnabled-Flag. Die bereits vorhandene WorkManager-Testimplementierung bleibt für diesen Instrumentierungsprozess erhalten.

`ReviewSyncTaskTest.kt`: Die private Fixture und ihre Gates liegen unter `<app.filesDir>/review-sync-<eigener Fall>`. Der bestehende `sys.info.cacheDir`-Call bleibt; eine Pfadassertion verhindert eine Fixture im geschützten Cache. Derselbe `Case` registriert den Ordner für seinen vorhandenen Cleanup. Alle Shared-/Privat-/Hook-/Entzug-/State-/Byte-/Wiederholungsassertions bleiben erhalten. Die Merge-Prepare-Assertions für `partial` und `confirmedA` bleiben strikt und zeigen nun den tatsächlichen Task-/Partialreport; die weitere FTP-/Retry-/Baseline-/Byteprüfung ist unverändert.

Das echte Intervall benötigt knapp fünf Minuten innerhalb des vorhandenen 15-Minuten-Fallbudgets. Die vorhandenen 300-/180-/120-/60-Sekunden-Follow-Fristen werden nicht verlängert. Der begrenzte neue Due-Wait schläft maximal bis zur tatsächlichen Fälligkeit; er überzieht diese nicht durch eine volle Pollperiode. Native-/Task-/Job-Cleanup nutzt begrenzte 30-/60-Sekunden-Fenster.

## Statischer Self-Review und genaue Gerätesignale

Frische Lektüre der eigenen Änderungen sowie statischer Vergleich mit dem gelesenen Ausgangstext: bestehende Testmethodennamen und kompletter `daemonControlsPauseAutopauseAndLog`-Body unverändert; `ReviewSyncTaskTest` hat exakt die private Pfadkorrektur und strikten Meldungsergänzungen. Beide periodischen Driver-Signale, ihre Constraints/60-Minuten-Periode, alle bisherigen Follow-Fristen und die Merge-/Storage-/Versionen-/Block-/Saved-State-Fälle bleiben vorhanden. Kein Compiler, Formatter, Test, Server oder sonstiger Workload wurde gestartet. Dieser Textvergleich ist keine Geräteabnahme.

| Bestehendes Testsymbol | Erwartetes echtes Signal derselben Root-RV1-Suite |
|---|---|
| `BackgroundTaskTest.catchUpAdmitsADueIntervalJob` | `background-due` enthält eigene ID und echte Due-Zeit; eigener State vor/nach, tatsächlicher Admission-/Skipreport, exakte Zielbytes und `lastCatchUpMs`. |
| `BackgroundTaskTest.periodicWorkerRunsCatchUpWhenConstraintsAndPeriodAreMet` | Beide Core-Tasks `done`; beide Perioden `ENQUEUED` + null Worker-Windows; `periodic-worker-terminal` enthält echte Work-ID/State. `background-idle` belegt terminales Cleanup. |
| `ReviewSyncTaskTest.sharedStorageRevokeStopsOnlySharedJobs` | Beide echten Hooks aktiv; Shared `canceled`; Private weiterhin aktiv, danach `done`/errors=0/exakte Bytes; unveränderter Shared-Cancelzustand/Counterpart und erfolgreiche neue Shared-Ausführung. |
| `ReviewSyncTaskTest.recordedMergeRetrySurvivesProcessRestart` | Erster bestätigter Merge-Teilerfolg plus tatsächliche FTP-Ablehnung bleiben erforderlich. `review-merge-publication` enthält tatsächlichen Task/Partialreport. Beide vorhandenen Prepare-/Force-stop-/Retry-Phasen behalten alle Byte-/State-/Baselineassertions. |

## Exaktes Dateiinventar

Gelesen (zusätzliche Produzenten ausschließlich zu den im Scope festgelegten Themen/Bereichen; eigene Manifest-/Berichtdatei eingeschlossen):

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-4-t-jobs.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fourth-fixes.md`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/BackgroundTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ReviewSyncTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/TaskSupport.kt`
- `android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt`
- `native/src/mobile/os/shared/domains/background.rs`
- `native/src/mobile/os/shared/domains/sync_run.rs`
- `native/src/mobile/os/shared/domains/sync_jobs.rs`
- `native/src/support_dirs.rs`
- `native/src/bisync/os/shared/snapshot_policy.rs`
- `docs/refs/android-sync-triggers.md`
- `docs/refs/android-ci.md`
- `/tmp/rv1-ci-fourth/device/background-worker.log`
- `/tmp/rv1-ci-fourth/device/sync-sharedStorageRevokeStopsOnlySharedJobs.log`
- `/tmp/rv1-ci-fourth/device/merge-prepare.log`
- `native/src/daemon/os/shared/due.rs`
- `native/src/daemon/os/shared/catch_up.rs`
- `native/src/daemon/os/android/platform.rs`
- `android/app/src/main/java/app/smartexplorer/android/service/BackgroundController.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/Core.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/SyncSupport.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/InitConfig.kt`
- `native/src/daemon/os/shared/run_loop.rs`
- `native/src/daemon/os/shared/state.rs`
- `native/src/daemon/os/shared/live.rs`
- `android/app/src/main/java/app/smartexplorer/android/system/HostMonitor.kt`
- `native/src/mobile/os/shared/domains/sync_state_json.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-T-JOBS.md`

Geändert:

- `android/app/src/androidTest/java/app/smartexplorer/android/task/BackgroundTaskTest.kt` – 317 Zeilen.
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ReviewSyncTaskTest.kt` – 423 Zeilen.

Erstellt:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-T-JOBS.md`.

Keine weiteren Dateien erstellt oder geändert.

## Offene Fremdgrenzen und Handoff

- E-ENGINE schließt die tatsächliche Android-Namespace-/Durabilityproduktion. Die private Pfadkorrektur ersetzt keine fehlende Veröffentlichung; Private muss weiterhin errors=0 und echte Bytes liefern. Ebenso bleibt `confirmedA=true` im echten FTP-Merge-Faultfall zwingend.
- Root integriert, committet/pusht und verwendet ausschließlich dieselbe vollständige RV1-Remote-Suite. Dieses Ergebnis behauptet noch keinen bestandenen erneuten Geräte-/Compilerlauf. Kein eigener Suite-/CI-/Graph-/Release-Einstieg wurde geschaffen oder ausgeführt.
- Keine offene eigene Definitions-/Scope-Lücke verbleibt. Der testinterne Reflectionanschluss ist bewusst an den vorhandenen genauen Worker-Window-Produzenten gebunden und muss bei dessen Umbenennung zusammen mit der Fixture gepflegt werden.

Der begrenzte Implementierungsblock ist abgeschlossen. Dieser Worker stoppt.
