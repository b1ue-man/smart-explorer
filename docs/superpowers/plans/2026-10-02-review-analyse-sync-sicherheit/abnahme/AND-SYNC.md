# AND-SYNC – Umsetzung und Abnahme

Stand: 2026-10-03. Auftrag: bestehende Befunde FS9/FS11/B19/B22 und die Android-Consumer von T-JOBS abschließen. Kein neues Review. Graph-Anschluss und API-Verträge wurden vom Hauptagenten bereitgestellt. Nur der exakte AND-SYNC-Scope und ausdrücklich ergänzte Grants gelten; keine lokale Ausführung.

## Stufe zwei vor der Implementierung

1. **Gemeinsamer Zustand und sichere manuelle Läufe.** Mobile `sync_run`, `sync_jobs` und `job_json` übernehmen `JobState`, `record_attempt`, `classify_run`, Job-RunSettings und die gemeinsame Paarsperre. Start-/Verbindungsfehler zählen als Versuche, Erfolg wird nur tatsächlich abgeschlossenen Läufen zugeschrieben, Abbruch erhält offene Trigger und Zwischenstände. RAII hält Runmark, Wake-Anforderung und Shared-Storage-Cancel bis zum wirklichen Ende. Erwartung: Android, Daemon und Desktop zeigen dieselben Zeiten, Probleme und Sperren; Quellen/Backendidentitäten bleiben unverändert.
2. **Sperren und Versionen.** Eine reine Prüfung liefert den tatsächlichen Block ohne Lauf-Erfolg. Bestätigen bindet exakt den gezeigten Block; der nächste echte Lauf verbraucht diese Freigabe einmal. Android listet manifeste Versionen je Job mit Pfad/Zeit und führt Wiederherstellung über die vorhandene sichere Versions-API aus. Konflikt-/Merge-Schreibungen verwenden Jobzustand und Sicherungs-/Checkpoint-Vertrag. Erwartung: geänderte Sperre fordert erneute Prüfung, Backupfehler verhindern Überschreiben, keine fremden Namen/Paarzustände werden geöffnet.
3. **Android-Host und Workerfenster.** Allfiles-Status wird vor Öffnen berichtet und laufende Shared-Storage-Consumer werden über dasselbe Weak-Cancel-Register gestoppt. MediaStore liefert nur Hinweise und einen Volume/Version/Generation-Cursor; Poll und Vollkontrolle bleiben zwingend. Catch-up übernimmt `failed`/`retry_suggested`, Worker-Abbruch erreicht den nativen Auftrag. Erwartung: kein Erfolg/Fehlbaseline bei Rechteverlust, Fern/Fern und appprivate Orte bleiben verfügbar; ein unveränderter MediaStore-Cursor unterdrückt keine nötige Abfrage.
4. **Zeitplanung und Wachbleiben.** `bg.status` liefert nächste echte Jobzeit und letztes Catch-up-Ergebnis. Separate Jobalarme werden nach Jobs/Status/Boot/Rechteänderung erneuert; exakte Alarme nur bei Recht, inexacte Alarme führen ausschließlich in WorkManager. Hintergrundläufe halten begrenzte erneuerbare CPU-Holds; Workerfenster erlauben keine unabhängigen neuen Daemonläufe. Dienst-/Timeout-Grenzen bleiben sichtbar und geben Abbruch weiter. Erwartung: Screen-off verschluckt fällige Arbeit nicht; keine unzulässige FGS-Promotion aus Boot oder inexactem Alarm; Wiederaufnahme bleibt möglich.
5. **Host-Zahlen und Android-Bedienung.** HostMonitor verwendet den vorhandenen erfolgreichen StorageStats-Produzenten für das tatsächlich primäre lokale Volume und übergibt dieselben PlatformTotals an den Rust-Hostspeicher. Jobkarten zeigen letzte Versuche/Erfolge, Problem/Folgelauf/Watchmodus und bieten Prüfen, einmalige Bestätigung und Versionen. Probleme erscheinen im eigenen Notificationkanal und öffnen den betreffenden Job. Erwartung: Hosting ohne lokale Analyse erhält echte aktuelle Telefonzahlen; Remote-Clientzahlen werden nie als lokale Plattformzahlen gespeichert; unbekannte Zahlen bleiben unbekannt.
6. **Gezielter Merge-Wiederanlauf.** Der nachgereichte E-APPLY-Vertrag `pending_merge_for_key` liefert durable Originalbytes, ursprüngliche Conflict-Sigs und die unveränderte Write-/KeepBoth-Wahl. Eine frische Prüfung ergänzt offene Aufträge aus dem gerade validierten StateKey; der Dialog zeigt Auftrag und bestätigte Seiten und bietet einen ausdrücklichen unveränderten Retry. Erwartung: Prozessneustart oder erneute Prüfung ersetzt Originale nicht durch bereits publizierte Mergebytes; normale A/B-Auflösung umgeht keinen offenen Auftrag.

## Zweite Lückenprüfung vor der Implementierung

- Vorhandene Locator-Auflösung bleibt alleinige Ortsgrenze; keine Stringheuristik routet Fernorte zu lokalen APIs. Shared-Storage-Auswahl übernimmt `platform::requires_storage_access` einschließlich appprivater Ausnahmen.
- Das bestehende Daemon-Cancel prüfte Allfiles nur vor dem Öffnen. Der Zusatzgrant erlaubt ein kurzes Weak-Register in `host_state` und seinen Worker-Guard in `job_supervisor`; Anmeldung und Rechteentzug müssen denselben Mutex benutzen, damit kein Lauf durch die Race fällt.
- Native Versionen- und aufgezeichnete Konflikt-API sowie Android Event-Decoder benötigen eng begrenzte Lesefreigaben/Registrierungen; konkrete Anfragen gehen an den Hauptagenten. Keine außer-scope Exploration.
- Gesicherte Android-Primärquellen vom 2026-10-02: MediaStore deckt Dateipfad-/Raw-/Android-data-Änderungen nicht vollständig ab; unveränderte Generation ist kein Beleg für unveränderten Baum. `setAndAllowWhileIdle` gewährt keine FGS-Startausnahme, `dataSync` hat Plattform-Zeitgrenzen; WorkManager-Abbruch stoppt eigene native Threads nur mit explizitem Cancel.
- A-CLIENTs `TaskForegroundService` und `TaskKeeper` werden unverändert erhalten. Bestehende öffentliche CPU-Registrierung wird für manuelle Sync-/Mirror-Aufträge konsumiert; Hintergrunddienste ergänzen eigene begrenzte Holds.
- Plattformzahlen werden ausschließlich von erfolgreicher eigener StorageStats-Erhebung und der tatsächlichen primären Volumezuordnung angenommen. Gemeinsamer Parser statt fremdem Encoderduplikat; Produzent bleibt unverändert, soweit der vorhandene öffentliche Aufruf genügt.
- Gezielte nachgereichte Lücke: Recovery-Lesen erfolgt ausschließlich über E-APPLYs `pending_merge_for_key`; die Liste offener Relatives wird unter PairLock mit dem frischen Lauf-StateKey gelesen. Originalbytes/Digests werden nicht im Consumer encodiert. Ein unveränderter Retry bleibt eine separate explizite Nutzeraktion.

## Abnahmesignale für die eine finale Remote-Suite

`android_sync_attempt_state_and_block_once`, `android_shared_storage_revoke_race_and_selection`, `android_sync_versions_manifest_restore`, `android_catch_up_retry_and_cancel_window`, `android_media_cursor_stays_hybrid`, `android_schedule_alarm_permission_and_screen_off`, `android_problem_notification_job_route`, `android_host_totals_primary_without_analysis`, `android_recorded_merge_restart_retry` sowie vorhandene Locator-/Omission-/Conflict-Checkpoint-Verträge. Diese Namen beschreiben gezieltes erwartetes Verhalten; es wurde kein Test ausgeführt. Konkrete Testquellnamen und Dateien stehen unten.

## Ergebnis und Fundzuordnung

Quellen für AND-SYNC sind abgeschlossen. Die nachgereichten Recorded-Merge-, Originalpfad- und Recovery-APIs sind angeschlossen. Dies ist eine statisch geprüfte Umsetzung; Laufzeitabnahme bleibt Teil der einen finalen Remote-Suite.

| Befund / Anschluss | Umgesetzt | Konkretes Abnahmesignal |
|---|---|---|
| FS9: Versuche, Fehler und echte Erfolge | Manuelle Jobläufe verwenden `record_attempt`/`classify_run`, jobeigenen StateKey, aktuelle Endpunkte, Hooks und tatsächliche Fehlerarten. Runmark/Heartbeat und Locks verhindern parallele UI-Aufträge desselben Jobs. Kaputte Konfigurationen bleiben als eigene Problemzeilen sichtbar. | Startfehler aktualisiert Versuch/Access/Auth/Config, niemals Erfolg. Abbruch lässt Trigger/Checkpoint offen; unabhängiger gültiger Job bleibt bedienbar. |
| FS9 / FS3: Prüfung und einmalige Bestätigung | Full-Probelauf verändert keine Erfolgszeit. Exaktes BlockKind wird bestätigt und vom nächsten echten Lauf einmal verbraucht; späteres Change-Pending bleibt erhalten. | Nach geänderter Löschmenge/Seite ist alte Bestätigung ungültig. Zweiter Lauf braucht neue Freigabe; kein globales Bypassflag. |
| FS11 / B28: Storage, Worker und Termine | Allfiles-Bericht vor Öffnen, gemeinsames Weak-Cancel-Register für wirkliche Shared-Storage-Consumer, Catch-up-Cancel bis zum tatsächlichen Ende, achtminütiges Workerfenster, `retrySuggested`→Retry, separate echte Jobalarme und Berechtigungsanzeige. | Entzug vor Anmeldung und nach Anmeldung setzt denselben tatsächlichen Cancel; keine neuen Shared-Storage-Öffnungen/Fehlbaseline. Fern/Fern und appprivat laufen weiter. Exakt nur mit Recht, inexact ausschließlich WorkManager. |
| B19 / FS8: sparsame, verlässliche Kontrolle | UI erhält Poll-/Höchstwarte-/Verify-Timings. Inkrementeller echter Lauf und Full-Prüfung nutzen gemeinsamen Vertrag. MediaStore-Version+Generation sind partielle Hinweise; Native-Probe-Nonce verhindert die Behauptung eines unveränderten Gesamtbaums. | Gleichbleibende Generation, DB-Reset und unbeobachtete Dateiänderung behalten Hybrid-Poll/Vollkontrolle. Content-Trigger bleibt OneTime-Work; periodischer Rückfall bleibt aktiv. |
| B22: eigene Schreibereignisse | Android verwirft MediaStore-Hinweise nicht pauschal. Selbstsignatur-/Generation-Entscheidung bleibt vollständig beim T-JOBS-Consumer; ohne Nachweis bleibt Folgelauf. | Gleich große fremde Änderung mit wiederhergestelltem mtime wird weiter erkannt; ein eigener MediaStore-Hinweis verhindert keinen nötigen Folgelauf. |
| FS9 / FS12: echte Versionen | Manifestliste mit Originalpfad/Zeit/Seite/Grund, opaque Auswahl-token, erneute Liste unter PairLock, sichere Restore-API und explizite Seite für Altversionen. | Gelöschter/geänderter Token oder retargeteter Job wird abgelehnt; Backupfehler verhindert Überschreiben. Kein Versionsordner-Heuristik-Erfolg. |
| Konflikt/Merge-Checkpoint | Originalpfade kommen aus StateKey/Seitenschreibweisen; Regular-Read ist begrenzt und abbrechbar. Recorded Merge liefert bestätigte Seiten/Preserved/Teilresultat. Erfolg verlangt vollständige Bestätigung und passende Engine-Baseline. | Fehler nach erster Publikation bleibt Fehler mit sichtbarem Teilresultat; keine zweite Consumer-Baselineschreibung und keine ungeschützte Bytes-Fallback-Schreibung. |
| Merge nach Neustart / Fresh-Check | Offene Originalpfade werden mit frischem StateKey unter PairLock gefunden, gespeicherte ursprüngliche Konflikte in die Liste übernommen. Auftrag, Gewinnerseite und bestätigte Seiten erscheinen im Dialog; separater `sync.mergeRetry` wiederholt exakt gespeicherte Bytes. | Write- und KeepBoth-Teilfall nach Prozessneustart: ursprüngliche Originalbytes bleiben Eingaben, gleiche Wahl ist explizit wiederholbar; anderer Digest/Input bleibt Fehler. Kein A/B-UI-Bypass für offene Aufträge. |
| Probleme und Hosting-Zahlen | Gedrosselter nativer ProblemNotice wird zu `syncProblem` und eigener Android-Notification mit bestehendem privaten Job-PendingIntent. HostMonitor erhebt erfolgreiche echte primäre StorageStats auch ohne Analyse-UI und meldet dieselben PlatformTotals. | Notification öffnet richtigen Job; unbekannte Zahlen bleiben unbekannt, Client-/Fernzahlen überschreiben keine lokalen Telefonzahlen. |

## Entscheidungen und Self-Review

- Locatorstrings, Backend-/Verbindungsidentität und gespeicherte Seitennamen bleiben erhalten. Jeder Endpoint wird über die bestehende gemeinsame Resolvergrenze geöffnet; keine neuen Pfadparser, Hash-Encoder oder freien Merge-/Version-Dateipfade.
- Geteilte Paarsperren, StateOwner, Backup-/Publish-/Checkpoint-Grenzen gehören weiterhin der Engine. Die Consumer-Liste offener Recovery-Pfade besitzt einen gerade validierten StateKey; der anschließende read-only Auftrag prüft Endpunkte/Repliken erneut.
- Guard-Lebenszeiten folgen der tatsächlichen Rust-Closure, auch wenn ein blockierender Backendaufruf erst später auf Cancel reagiert. Auswahl und Widerruf nutzen denselben kurzen Mutex; Weak-Einträge halten keinen Lauf künstlich am Leben.
- Manuelle Prüfen-/Versionen-/Konfliktaufträge zeigen eine lebende Laufmarke, schreiben aber keinen erfundenen regulären Sync-Erfolg. Tabellen zeigen Versuch und Erfolg getrennt; Dateizähler werden bei vorhandenem JobState neutral dargestellt.
- A-CLIENTs `TaskForegroundService`/`TaskKeeper` und der erfolgreiche StorageStats-Produzent blieben unverändert. Vorhandene CPU-Registrierung und öffentliche Figuren-/Plattform-API werden konsumiert.
- Exaktalarm-, Notification-, FGS- und Allfiles-Races werden als recoverable Fehler behandelt. Das nächste echte Jobdatum stammt aus dem nativen Status; WorkManager bleibt für Android-Verzögerung/fehlende Exaktberechtigung verfügbar.
- `deferScheduling` ist nach Parent-Entscheidung eine Admission-Sperre. Dienstverlust und spätere Batterie-/Netzbedingungen brechen bereits laufende unabhängige Daemon-/manuelle Aufträge nicht global ab. Eigene Workerfenster und Shared-Storage-Entzug besitzen dagegen tatsächlichen Cancel; diese Restgrenze ist in UI/Handoff sichtbar.
- Eigene Änderungen wurden auf Modulpfade, API-Signaturen, tatsächliche Originaldaten, Callback-Lebenszeiten, Fehler-/Teilresultate, Receiver/PendingIntent und additive Registrierungen geprüft. Scope-JSON und eigenes Manifest-XML wurden rein statisch geparst. Alle eigenen neuen und bearbeiteten Rust-Dateien liegen unter 500 Zeilen und 50 KiB; keine Compiler-/Formatter-Ausführung.
- Offene atomare Fremdgrenze: `resolve_recorded` muss einen gleichzeitig neu entstehenden Pending-Merge unter seiner eigenen Paarsperre ablehnen. UI und Consumer prüfen vorhandene Pending-Aufträge bereits; der außerhalb unserer Sperre gelesene Precheck ersetzt diese Engine-Grenze nicht. Siehe [Anfragen](../anfragen/AND-SYNC.md).

## Vorhandene Abnahmequellen und finale Suite

Eigene gezielte Testquellen wurden geschrieben, **nicht ausgeführt**:

- `daemon::host_state::tests::android_shared_storage_revoke_selection_and_weak_lifetime`
- `daemon::host_state::tests::android_shared_storage_registration_covers_both_revoke_orderings`
- `mobile::domains::sync_run::attempt::tests::android_sync_confirmation_consumed_once_preserves_later_change`
- `mobile::domains::sync_run::attempt::tests::android_sync_start_errors_keep_the_shared_failure_kind`
- `mobile::domains::sync_state_json::tests::android_sync_state_keeps_attempt_success_and_live_runner_distinct`
- `mobile::sys::platform::tests::android_host_platform_unknown_totals_are_not_zero`

Vorhandene Fixtures bleiben unverändert: `android_task_job_json_round_trip_keeps_every_desktop_field`, `android_task_sync_options_list_every_mode_without_device_triggers`, `sync_conflict_task_mobile_exposes_variants_and_disables_ambiguous_merge`. Die benannten Android-Abnahmesignale einschließlich `android_recorded_merge_restart_retry` sind **Suite-Erwartungen**, keine behaupteten existierenden Testfunktionen. Der Hauptagent verbindet sie einmal mit den passenden Native-/Android-Gerätefällen sowie Locator-/Omission-/StateKey-/Backup-Verträgen. Keine lokale oder zusätzliche Remote-Ausführung durch diesen Agenten.

## Exaktes Dateiinventar

Alle folgenden geänderten und neu erstellten Quellen wurden auch gelesen; weitere Lesedateien folgen separat. Lesen umfasst teilweise gezielte relevante Ausschnitte. Vorangegangene T-JOBS-Dateien werden nicht nochmals als AND-SYNC-Änderungen beansprucht. Bereits geladene Skills: `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/graphify/SKILL.md`.

### Geändert – Rust

Nur eigene additive Reexports/Modul-/Methoden-/Hook-Einträge in `daemon/mod.rs` und `domains/mod.rs`; bestehende fremde Einträge erhalten.

- `native/src/daemon/mod.rs`
- `native/src/daemon/os/shared/host_state.rs`
- `native/src/daemon/os/shared/job.rs`
- `native/src/daemon/os/shared/job_supervisor.rs`
- `native/src/mobile/os/shared/domains/analyze_platform.rs`
- `native/src/mobile/os/shared/domains/background.rs`
- `native/src/mobile/os/shared/domains/job_json.rs`
- `native/src/mobile/os/shared/domains/mod.rs`
- `native/src/mobile/os/shared/domains/sync_conflicts.rs`
- `native/src/mobile/os/shared/domains/sync_jobs.rs`
- `native/src/mobile/os/shared/domains/sync_merge.rs`
- `native/src/mobile/os/shared/domains/sync_run.rs`
- `native/src/mobile/os/shared/sys.rs`

### Geändert – Android

- `android/app/src/main/AndroidManifest.xml`
- `android/app/src/main/java/app/smartexplorer/android/api/SyncApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/CoreEvent.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/CoreEvents.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/BackgroundController.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/BackgroundService.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/BackgroundText.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/BootReceiver.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/HostMonitor.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/KeepAliveAlarm.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/Notifications.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/Permissions.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/BackgroundSettings.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/BackgroundParts.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/ConflictSession.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/ConflictsScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/JobCards.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/JobDraft.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/JobEditorScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/MergeScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/SyncScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/SyncViewModel.kt`
- `android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt`

### Neu erstellt – Rust

- `native/src/mobile/os/shared/domains/sync_attempt.rs`
- `native/src/mobile/os/shared/domains/sync_merge_recovery.rs`
- `native/src/mobile/os/shared/domains/sync_state_json.rs`
- `native/src/mobile/os/shared/domains/sync_versions.rs`
- `native/src/mobile/os/shared/sys_platform.rs`

### Neu erstellt – Android

- `android/app/src/main/java/app/smartexplorer/android/api/SyncState.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/AndroidHostFigures.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/MediaStoreSync.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/SyncScheduleAlarm.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/BlockDialog.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/sync/VersionsScreen.kt`

### Neu erstellt – eigene Berichte

Der Abnahmebericht wurde zuerst als Stufe-zwei-Plan angelegt und anschließend fortgeschrieben.

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/AND-SYNC.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/AND-SYNC.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/AND-SYNC.md`

### Zusätzlich gelesen – Rust, unverändert

- `native/src/analytics/core/host_figures.rs`
- `native/src/bisync/core/completion.rs`
- `native/src/bisync/core/keys.rs`
- `native/src/bisync/core/run_types.rs`
- `native/src/bisync/core/types.rs`
- `native/src/bisync/mod.rs`
- `native/src/bisync/os/shared/merge_inputs.rs`
- `native/src/bisync/os/shared/merge_recorded.rs`
- `native/src/bisync/os/shared/orchestration.rs`
- `native/src/bisync/os/shared/recorded_paths.rs`
- `native/src/bisync/os/shared/resolve.rs`
- `native/src/bisync/os/shared/version_ops.rs`
- `native/src/bisync/os/shared/versions.rs`
- `native/src/daemon/os/android/platform.rs`
- `native/src/daemon/os/linux_os/platform.rs`
- `native/src/daemon/os/shared/catch_up.rs`
- `native/src/daemon/os/shared/catch_up_types.rs`
- `native/src/daemon/os/shared/hooks.rs`
- `native/src/daemon/os/shared/problem_notify.rs`
- `native/src/daemon/os/windows/platform.rs`
- `native/src/keep_awake/mod.rs`
- `native/src/mobile/os/shared/domains/sync_conflict_variant_task_tests.rs`
- `native/src/mobile/os/shared/domains/tests.rs`
- `native/src/mobile/os/shared/runtime.rs`
- `native/src/syncjobs/core/types.rs`
- `native/src/syncjobs/core/validation.rs`
- `native/src/syncjobs/mod.rs`
- `native/src/syncjobs/os/shared/editor.rs`
- `native/src/syncjobs/os/shared/job_state.rs`
- `native/src/syncjobs/os/shared/job_state_classify.rs`
- `native/src/syncjobs/os/shared/job_state_policy.rs`
- `native/src/syncjobs/os/shared/job_state_store.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/os/shared/remote_util.rs`
- `native/src/watch/mod.rs`
- `native/src/watch/os/shared/host_signal.rs`

### Zusätzlich gelesen – Android, unverändert

- `android/app/src/main/java/app/smartexplorer/android/api/AnalyzeApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/Core.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/ServiceNotifications.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/TaskForegroundService.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/TaskKeeper.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/KeepAliveNetwork.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/KeepAliveProbe.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/StorageStatsAccess.kt`
- `android/app/src/main/java/app/smartexplorer/android/system/WakeKeeper.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/AppNav.kt`

### Zusätzlich gelesen – Plan, Verträge und lokale Primärquellen, unverändert

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/refs/android-sync-triggers.md`
- `docs/refs/sync-change-detection.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/A-CLIENT.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-APPLY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-PLAN.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-ANALYSIS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/T-JOBS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/T-JOBS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/and-sync.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`

Keine globale Planänderung, kein Build/Test/Formatter/Server/Installationsprozess, kein Commit/Push/CI/Graph-Neubau/Release durch diesen Agenten.
