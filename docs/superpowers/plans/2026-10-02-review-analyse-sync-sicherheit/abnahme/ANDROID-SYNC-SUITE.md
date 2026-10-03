# ANDROID-SYNC-SUITE – Gerätefixture für AND-SYNC

Stand: 2026-10-03. Implementierung im freigegebenen Fixture-Scope nach Root-Go; **keine Geräteabnahme ausgeführt**. Der Hauptagent integriert und bewertet die Fälle ausschließlich im gemeinsamen RV1-Remote-Einstieg. Keine Produkt-, UI-, CI-, Release- oder Graphänderung durch diesen Block.

## Plan und geschlossene Lücken

Der erste Plan band die vorhandenen JNI-Helfer an die tatsächlichen JobState-, Block-, Versions- und Merge-Produzenten. Der zweite Plan ergänzte Byteassertionen, Laufmarken, endliche Before-Hook-Gates, Cleanup und den Neustartablauf. Die anschließende Lückenprüfung ergab: `conn.save` unterstützt WebDAV ausschließlich über HTTPS; ein HTTP-Loopbackserver würde den Android-Consumer nicht erreichen. Root gab dafür eine echte Plain-FTP-Fixture frei. Ihr Vertrag stammt aus den vorhandenen FTP-Protokollquellen und gesicherten Referenzen.

E-ENGINE hat den verbundenen Publikationsanschluss abgeschlossen: Nach dem mutationfreien `replace_staged_reversible -> Ok(false)` nutzt `replacement_publish` genau einmal die vorhandene Sync-Promotion. Ein Fehler erhält Stage/Intent. Die Fixture konsumiert diesen fertigen Anschluss; sie verändert keine Backend-Capability und keine Produktfehlerroute.

Der Neustartplan speichert den tatsächlich vergebenen FTP-Port, den vollständigen eigenen RAM-Namespace mit Bytes/UTC-mtimes und die unveränderten Job-/Verbindungsidentitäten. Root beendet zwischen den Phasen den echten Target-App-Prozess. Die zweite Phase bindet denselben Port und prüft durable Recovery über die reale JNI-Fassade.

## Konkrete Abnahmesignale

| Testsymbol in `app.smartexplorer.android.task.ReviewSyncTaskTest` | Reale Grenze und erwartetes Signal |
|---|---|
| `savedJobAttemptsKeepChecksAndAccessFailuresDistinct` | `sync.options/save/run/jobs/checkConflicts`, `sys.hostState`, `bg.status`: tatsächliche Kopie und JobState; reiner Check erhält Versuch/Erfolg; fehlender Shared-Zugriff erzeugt `lastError.kind=access`, `problem=needs_action`, einen neuen Versuch und keinen neuen Erfolg. Wiederholung nach Grant kopiert die richtigen Bytes. Native Taskzustände `failed`/`canceled` bleiben bei initialer Verweigerung unterscheidbar. |
| `checkedDeleteBlockConsumesOnlyOneMatchingConfirmation` | Reale Löschplanung nach vollständiger Baseline, bei weiterhin gefüllten Seiten. `checkConflicts` speichert die echte `blocked.kind`; vor Freigabe bleibt B unverändert und `sync.run` meldet `blocked`. Geänderte Planung verwirft die alte Kind-Bestätigung. Exakte Bestätigung erlaubt die vorgesehene Löschung; die nächste Löschserie benötigt wieder eine Bestätigung. |
| `manifestVersionRestoreRejectsStaleTokens` | `sync.versions` liefert manifestbasierte B-Version mit Token, relativer Originaldatei, Seite, Größe und Grund. Erneutes Listing invalidiert den alten Token. `restoreVersion` stellt die alten B-Bytes her, erhält A und unabhängige Dateien sowie regulären Job-Erfolg. Der folgende Sync übernimmt den Restore auf A. |
| `sharedStorageRevokeStopsOnlySharedJobs` | Zwei tatsächliche Jobaufträge stehen in endlichen `runBefore`-Gates mit lebenden Tasks/Runmarken. `sys.hostState(storageAccess=false)` cancelt den Shared-Auftrag; der Cache-Auftrag bleibt aktiv und kopiert nach Gate-Freigabe korrekt. Shared-Gegenstück bleibt erhalten, `lastSuccessMs` bleibt leer und Cancellation erzeugt keine Fehlerserie. Erneuter Shared-Lauf nach Grant funktioniert. |
| `recordedMergeRetrySurvivesProcessRestart` – `prepare` | Vollständiger Sync auf echtes FTP, beidseitige Änderungen, echte Zeilenwahl. Die Fixture lehnt genau einen RNTO eines nachweislich hochgeladenen Stages mit den erwarteten Mergebytes auf das B-Original ab. Ergebnis ist `failed`, `partial=true`, `confirmedA=true`, `confirmedB=false`, `baselineRecorded=false`, sichtbares Reload/Retry; A enthält den Merge und B seine ursprünglichen Bytes. |
| `recordedMergeRetrySurvivesProcessRestart` – `retry` | Neuer App-PID, kein alter Task-Snapshot mit gleicher ID/Startzeit, zunächst fehlender In-process-Konfliktkontext. Unveränderte Endpunkte/Port und echte A-/B-Zwischenstände. Full-Check lädt `pendingMerge` mit ursprünglichen Seitengrößen; `mergeRows.pending` enthält die gespeicherte Write-Wahl, Bestätigungen und vollständige Vorschau, während `rows` leer bleibt. Erst explizites `mergeRetry` schreibt dieselben CRLF-/Endnewline-Bytes auf beide Seiten, bestätigt die Baseline und lässt den Folgelauf konfliktfrei. |

Damit wird der tatsächliche HostState-/JNI-Cancelanschluss auf dem Gerät geprüft. Ein Android-AppOps-Entzug, FGS-/WorkManager-/Screen-off-Verhalten oder KeepBoth-Retry ist kein Abnahmesignal dieser Fixture; die verbundenen Fälle gehören zur übrigen RV1-Suite. Der Prozessneustart ist nur mit dem nachstehenden Runnerablauf belegt.

## Runnerargumente und Discovery

Root verwendet die tatsächlich entdeckte Target-/Instrumentierungs-Package und vorhandenen Instrumentierungsbefehle. Es werden keine zusätzlichen Remote-Server-, CA-, SFTP- oder Portargumente verlangt. Primärvolume kommt aus `Volumes.primary()`, private Pfade aus `sys.info.cacheDir`, Optionen aus `sync.options.defaults`, FTP-Port aus `ServerSocket.localPort`, der gespeicherte Locator aus `conn.save.location`.

1. Der eigene Class-Lauf erhält `-e class app.smartexplorer.android.task.ReviewSyncTaskTest -e reviewMergePhase prepare`.
2. Nach erfolgreicher Instrumentierung beendet Root die tatsächlich entdeckte Target-App mit `am force-stop`. Appdaten bleiben erhalten.
3. Root führt ausschließlich `-e class app.smartexplorer.android.task.ReviewSyncTaskTest#recordedMergeRetrySurvivesProcessRestart -e reviewMergePhase retry` aus.

Beide Instrumentierungsphasen und der Force-stop gehören zum selben RV1-Einstieg. Root wartet auf den ersten Instrumentierungsabschluss und startet keine parallele Fixture. Die App benötigt echten „Zugriff auf alle Dateien“ als Gerätevorbedingung; fehlender Zugriff führt zu einer ausdrücklichen Assertion.

Session: `<targetContext.filesDir>/task-report/review-sync-merge-session.json`. Sie enthält Schema-Version, Prepare-PID, alte fehlgeschlagene Task-ID/Startzeit, eigene Root-/Sourcepfade, vollständigen Targetlocator, Job-/Conn-ID, Port, originale A-/B-/Mergebytes sowie den FTP-Namespace mit Dateibytes und UTC-mtimes. Schreiben erfolgt über eigene temporäre Datei, `FileDescriptor.sync` und Rename. Prepare überschreibt keine bestehende Session. Retry validiert Schema, begrenzte Größe und eigene Pfadzuordnung. Fehlender Zustand, geänderte Endpunkte, ausgebliebener Neustart oder fehlgeschlagenes Rebind sind Fehler; die Identität wird nicht ersetzt.

Nach Prepare werden bereits alle Fixture-Sockets und Threads geschlossen. Nur erfolgreiche Prepare bewahrt Job, Verbindung, eigene Dateiwurzeln und Session für Retry. PID-, Port-, Job- und Ergebnisevidenz steht außerdem in `task-report/notes.txt`; die vorhandenen JNI-Helfer führen `calls.tsv` fort.

## Fristen, Ressourcen und Cleanup

- Vor HostState-Injektion wartet die Fixture auf den ersten tatsächlich gemessenen HostMonitor-Bericht. Nach jedem Fall restauriert sie die aktuell gemessenen Hostbedingungen. `deferScheduling=true` hält während des Falls neue automatische Admission zurück; Jobs verwenden `manual`.
- Die vorhandene `coreTest`-Gesamtfrist beträgt 15 Minuten, einzelne native Tasks höchstens 180 Sekunden. Storage-Gates laufen höchstens ungefähr 60 Sekunden; konkrete Aktivitäts-/Cancel-/Privatlaufassertionen sind auf 20/30 Sekunden begrenzt.
- FTP bindet ausschließlich `127.0.0.1`, mit echten Control- und passiven Datenverbindungen. Es verwendet vollständige MLSx-Listings inklusive Dotfiles, reale Größen, nachlesbare UTC-Sekunden, Authentifizierung mit eigenem Benutzer/Passwort und bounded Streaming. Es gibt keine behauptete NoReplace- oder fsync-Capability.
- Control-Reads haben 30 Sekunden, passive Accepts und Datenreads fünf Sekunden, Listener-Accepts eine Sekunde. Befehle sind auf 8192 Bytes, Bodies auf 8 MiB, Namespace auf 512 Einträge, Worker auf zwölf und Warteschlange auf sechzehn begrenzt. Die Fehlerauswahl benutzt STOR-Herkunft, Ziel und vollständige erwartete Bytes, keinen geratenen Stage-Präfix.
- `NonCancellable`-Cleanup besitzt eine 60-Sekunden-Frist: eigene Gates freigeben, nur eigene aktive Task-IDs canceln, auf deren Abschluss und freie Job-Laufmarken warten, eigene Jobs/Verbindungen über reale APIs entfernen und bekannte eigene Verzeichnisse mit `Files.walk` ohne Link-Follow löschen. Erfolgreiche Prepare erhält nur die notwendigen Sessionressourcen. Retry entfernt die Session auch bei einer fehlgeschlagenen Wiederholung, soweit native Cleanup erfolgreich endet.
- Host-Restore ist zusätzlich auf zehn Sekunden begrenzt. In jedem Cleanup werden Control-/Data-Sockets und Listener geschlossen und der Executor beendet; Accept-Thread-Join und Worker-Termination sind auf zwei/fünf Sekunden begrenzt. Fehler bleiben sichtbar und werden einer bestehenden Assertion als suppressed Fehler angefügt, sonst selbst geworfen.
- Native Persistenzbereinigung verwendet `sync.delete`/`conn.delete`. Die Fixture hat keinen direkten Löschvertrag für Engine-Historie außerhalb ihrer bekannten Verzeichnisse; deren bestehende Aufbewahrung bleibt beim Produkt. Bei unterbrochener erster Instrumentierung kann eine vorbereitete Session als konkrete Recovery-Evidenz erhalten bleiben.

Die Schließ-/Warteverträge wurden am 2026-10-03 mit den freigegebenen Primärquellen für [ServerSocket](https://docs.oracle.com/en/java/javase/17/docs/api/java.base/java/net/ServerSocket.html), [Socket](https://docs.oracle.com/en/java/javase/17/docs/api/java.base/java/net/Socket.html) und [ExecutorService](https://docs.oracle.com/en/java/javase/17/docs/api/java.base/java/util/concurrent/ExecutorService.html) abgeglichen: Socket-Close unterbricht blockiertes I/O; Executor-Shutdown benötigt die gesonderte Terminationswartezeit.

## Self-Review und Ausführungsstatus

Eigene Änderungen wurden statisch auf tatsächliche Methoden/Felder, Probe-vs.-Versuch, Sekundengranularität des Zustands, opaque Token/cid, einmalige Bestätigung, CRLF-/Endnewline-Goldenbytes, unveränderte Locatoridentität, Originaldaten und Ressourcenbesitz geprüft. Dabei wurden Cleanup der Session nach Prepare-Fehlern, Socket-Freigabe bei Bindfehlern, nullable Rename-Quellen und die Neustartprüfung mit ID **plus** tatsächlicher Task-Startzeit korrigiert. Reines Prüfen nach Neustart erhält die unterschiedlichen A-/B-Zwischenstände bis zum expliziten Retry.

Kein Compiler, Formatter, lokaler Test, Serverstart, Installations-, Commit-, Push-, CI-, Graph- oder Releaseaufruf. Statische Textkontrolle und Self-Review sind keine Geräteabnahme. Root muss die unveränderten echten Methoden in der gemeinsamen Remote-Suite ausführen und deren konkrete Ergebnisse bewerten.

## Exaktes Dateiinventar

Neu erstellt; weitere vorhandene Dateien wurden nicht geändert:

- `android/app/src/androidTest/java/app/smartexplorer/android/task/ReviewSyncTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ReviewMergeFtpFixture.kt`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/ANDROID-SYNC-SUITE.md`

Im Fixtureblock gelesen, teils gezielt/auszugsweise; bereits vorhandene Repository-/Skillinstruktionen gelten fort:

```text
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/android-sync-suite.json
docs/refs/android-apis.md
docs/refs/android-platform.md
docs/refs/android-ci.md
docs/refs/android-sync-triggers.md
docs/refs/android-child-processes.md
docs/superpowers/plans/2026-09-25-android-apk/api.md (nur sys/bg/sync/conn)
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/AND-SYNC.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/AND-SYNC.md
android/app/src/androidTest/java/app/smartexplorer/android/task/TaskSupport.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/SyncSupport.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/SyncTaskTest.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/BackgroundTaskTest.kt
android/app/src/main/java/app/smartexplorer/android/core/Core.kt
android/app/src/main/java/app/smartexplorer/android/core/CoreException.kt
android/app/src/main/java/app/smartexplorer/android/core/Dtos.kt
android/app/src/main/java/app/smartexplorer/android/api/SyncApi.kt
android/app/src/main/java/app/smartexplorer/android/api/SyncState.kt
android/app/src/main/java/app/smartexplorer/android/system/HostMonitor.kt
android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt
native/src/mobile/os/shared/domains/sync_jobs.rs
native/src/mobile/os/shared/domains/job_json.rs
native/src/mobile/os/shared/domains/sync_state_json.rs
native/src/mobile/os/shared/domains/sync_versions.rs
native/src/mobile/os/shared/domains/sync_merge.rs
native/src/mobile/os/shared/domains/sync_run.rs
native/src/mobile/os/shared/domains/connections.rs
native/src/mobile/os/shared/dispatch.rs
native/src/mobile/os/shared/sys.rs
native/src/mobile/os/shared/runtime.rs (nur is_app_internal und Config/Pfadgetter)
native/src/daemon/os/shared/host_state.rs
native/src/daemon/os/android/platform.rs
native/src/daemon/os/shared/hooks.rs (nur Shell-/Cancelordnung)
native/src/bisync/os/shared/merge_execution.rs
native/src/bisync/os/shared/merge_resume.rs
native/src/bisync/os/shared/merge_recovery.rs
native/src/bisync/os/shared/replacement_publish.rs
native/src/bisync/os/shared/version_save.rs
native/src/ftp/core/connection.rs
native/src/ftp/core/metadata.rs
native/src/ftp/core/staging.rs
native/src/ftp/core/extensions.rs
native/src/ftp/core/ftp.rs
native/src/ftp/core/streams.rs
native/src/ftp/core/transfer_fixture.rs
docs/refs/ftp-pool.md
docs/refs/sync-remote-metadata.md
android/app/src/androidTest/java/app/smartexplorer/android/task/ReviewSyncTaskTest.kt (eigene Quelle)
android/app/src/androidTest/java/app/smartexplorer/android/task/ReviewMergeFtpFixture.kt (eigene Quelle)
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/ANDROID-SYNC-SUITE.md (eigener Bericht)
```

In der ausdrücklich freigegebenen ersten DAV-Planphase zusätzlich gelesen; mit dem FTP-Scopewechsel nicht weiter gelesen und nicht verändert:

```text
native/src/webdav/core/webdav.rs
native/src/webdav/core/writer.rs
native/src/webdav/core/metadata.rs
native/src/webdav/core/multistatus.rs
native/src/webdav/core/stage_move.rs
native/src/webdav/core/connection_tests.rs (auszugsweise)
```

Offene Fremdgrenze ist die tatsächliche RV1-Remote-Ausführung einschließlich beider Instrumentierungsphasen und Force-stop. Keine weitere Produkt-API oder Lesefreigabe erforderlich.
