# AND-SYNC – API-Delta

Stand: 2026-10-03. Quellen abgeschlossen, statisch geprüft; keine lokale Ausführung. Vorhandene Methoden und Locator-/Verbindungsidentitäten bleiben kompatibel. [Abnahme und exaktes Datei-/Leseinventar](../abnahme/AND-SYNC.md#exaktes-dateiinventar) gelten auch für diesen Bericht; keine zusätzlichen Dateien oder Fremdänderungen.

## Gemeinsamer Zustand und mobile Methoden

`sync.jobs` ergänzt pro Job `state` mit `lastAttemptMs`, `lastSuccessMs`, `lastRunner`, `lastCause`, `consecutiveFailures`, `lastError`, `retryAtMs`, `blocked`, `problem`, `running`, `loadError`, `watch`, `pendingTrigger`, `lastVerifyMs`, `verifyCursor`. Zeiten werden saturierend von den gemeinsamen Sekunden in Millisekunden übertragen; `running` enthält ausschließlich die noch lebende Runmark. `lastResult` stammt aus JobState, nicht aus einer älteren Konfigurationszeile. Defekte Job-Dateien erscheinen als `brokenConfig` mit eigenem Load-Problem; gültige Jobs bleiben verfügbar.

`sync.run` nutzt aktuellen gespeicherten Job, jobeigenen StateKey, `RunSettings::for_job`, gemeinsame Engine-Paarsperre und `classify_run`/`record_attempt`. Before/After/Cleanup-Hooks und Cancellation bleiben erhalten. Tatsächliche Läufe allein verändern Versuch/Erfolg; reine Prüf-/Versions-/Konfliktaufträge bekommen RAII-Laufmarke/Heartbeat/Wake/Storage-Guard, aber keinen erfundenen regulären Erfolg.

| Methode | Eingabe | Ergebnis / Grenze |
|---|---|---|
| `sync.confirmBlock` | `{id,kind}`, exakt das angezeigte BlockKind | `{}`; gemeinsame `confirm_block` validiert aktuelle Sperre. Nur bestätigbare Arten; nächste echte Ausführung verbraucht die Freigabe einmal. |
| `sync.checkConflicts` | unverändert `{id}` | `{taskId}`; Full-Probelauf, reale Sperre wird nur bei sauberem Check gespeichert. Teilfehler bleiben Taskfehler. Offene Recorded-Merges kommen zusätzlich aus validierter Recovery. |
| `sync.versions` | `{id}` | `{taskId}`; Taskresultat `{items:[{token,path,side,runId,preservedMs,size,reason,store}]}`. Token ist in-process/opaque und an Job plus unveränderte Endpunkte gebunden. |
| `sync.restoreVersion` | `{id,token,side?}` | `{taskId}`; `side` für Legacy-Version ohne Seite erforderlich. Frische Verbindung, Paarsperre und erneuter exakter Manifestabgleich vor sicherem Restore. Keine Baselinebehauptung für einen regulären Sync. |
| `sync.options` / Job-JSON | additive `versionsLocations`, `rtMaxLatencySecs`, `rtPollSecs`, `verifyIntervalSecs`, `verifyTargetSecs`, `maxDeleteMin`, `retainCount`, `versionsLocation`, `crossMounts`, `runCleanup` | Bestehender JobEditor validiert/persistiert; versteckte Altoptionen bleiben erhalten. Feldfehler benutzen passende neue Android-Feldschlüssel. |

## Recorded Merge, Originalpfade und Neustart

Bestehende `sync.mergeApply`/`sync.mergeKeepBoth` verwenden ausschließlich E-APPLYs `merge_recorded_for_key`. Originalpfade kommen aus `recorded_original_paths_for_key`; Lesen erfolgt über `vfs::open_read_regular` auf dem ursprünglichen Backend und ist auf 16 MiB pro Textseite begrenzt. Beobachtete Conflict-Sigs und vollständige Originalbytes werden an die Engine gegeben. CRLF/finaler Zeilenumbruch bleiben über TextShape erhalten.

`sync.mergeRows {id,cid}` behält `rows` und ergänzt `pending: null | {kind,keepA,confirmedA,confirmedB,preview,previewTruncated}`. Kind ist `write` oder `keep_both`; Write-Vorschau ist auf 8192 Unicodezeichen begrenzt und niemals Schreibinput. Für einen offenen Auftrag werden keine bereits publizierten Mergebytes als neue Originalzeilen ausgegeben. Kotlin behält `mergeRows(...): List<MergeRow>` als Hülle; `mergeRowsData(...): SyncMergeRows` erhält die neuen Metadaten.

`sync.mergeRetry {id,cid} -> {taskId}` ist die separate explizite Wiederholung. `pending_merge_for_key` liest originale Bytes/Sigs und unveränderte Wahl unter Endpunkt-/State-/Paarsperrenprüfung. Write verwendet exakt `pending.merged`; KeepBoth exakt die gespeicherte Gewinnerseite. Ein Fresh-Check listet offene Relatives unter PairLock mit seinem gerade validierten StateKey und validiert jeden Auftrag nochmals über diese API. Ursprüngliche Konflikt-Sigs ersetzen neu gescannte Teil-Sigs desselben kanonischen Schlüssels.

Ergebnis-/Fehlerpayload: `confirmedA`, `confirmedB`, `partial`, `baselineRecorded`, `reload`, `retry`, `preserved:[{path,confirmedA,confirmedB}]`, bei vollständiger Auflösung zusätzlich `remaining`. Gesamt-Erfolg verlangt beide Bestätigungen und einen passenden Eintrag in der Engine-Baseline. Fehler verwendet `MergeFailure.partial`, behält den ursprünglichen Draft bzw. durable Auftrag und fordert sichtbaren Reload/Retry. Es gibt keine Consumer-`write_bytes`-Fallback-Schreibung.

Offene Engine-Grenzen sind in [Anfragen](../anfragen/AND-SYNC.md) dokumentiert: atomarer Pending-Check innerhalb `resolve_recorded` sowie Schutz der offenen Relatives in regulärer Planung/Index/Checkpoint.

## Catch-up, Host und Android-Ereignisse

- `bg.status` ergänzt `nextScheduledRunMs`, `storageAccess`, `lastCatchUpResult:{finishedMs,ran,succeeded,failed,message}`. Alter `lastCatchUpMs` bleibt kompatibel und ist allein kein Erfolgsbeleg.
- Catch-up-Taskresultate ergänzen echte `failed`, `retrySuggested`, `message`. Worker übernimmt `retrySuggested` und cancelt seinen tatsächlichen nativen Auftrag bei Stop/Timeout; Request-Guard bleibt bis zum Ende bestehen. Leeres/unterbrochenes Fenster erfindet keinen neuen erfolgreichen Hintergrundlauf.
- `sys.hostState` erhält tatsächliches `storageAccess`. Bestehende Bedingungen und `deferScheduling` bleiben unabhängig. `deferScheduling` sperrt Admission; globaler Cancel schon laufender unabhängiger Tasks ist kein Teil dieses Vertrags.
- `sys.watchHints {volumes:[{path,cursor,changed}]}` akzeptiert höchstens 128 bekannte absolute Volume-Orte und Cursor bis 2048 Bytes; Ausgabe `{accepted,complete:false}`. Cursor ist intern immer `partial:<Probe-Nonce>:<ProviderCursor>`; fehlender/ungültiger Cursor bleibt unbekannt. MediaStore-Generation ist kein vollständiger Dateibaum-Nachweis.
- `sys.platformTotals {volume,platform}` akzeptiert ausschließlich das tatsächliche primäre, von Runtime gemeldete lokale Volume. `platform` entspricht dem bestehenden AnalyzeApi-Produzenten; Ausgabe `{remembered:true}`. `analytics::remember_platform_totals` erhält erfolgreiche eigene StorageStats; unbekannte Werte werden nicht zu Null. Bestehender Analyze-Parser delegiert an denselben extrahierten Parser.
- Native ProblemNotice wird über den vorhandenen Activity-Hook zu `{type:"syncProblem",jobId,title,text}`; CoreEvent/CoreEvents ergänzen nur diesen Decoder. Android nutzt eigene gedrosselte Job-Notification und bestehenden privaten AppNav-PendingIntent-Vertrag.

## Kleine interne API und Registrierungen

`daemon::register_storage_run(source:&str,target:&str,cancel:&Arc<AtomicBool>) -> StorageRunGuard` ist `pub(crate)`. `StorageRunGuard::access_missing()` erkennt fehlenden initialen Shared-Storage-Zugriff; Drop entfernt den Weak-Marker. Auswahl verwendet ausschließlich `platform::requires_storage_access`; Registrierung und Widerruf teilen denselben kurzen Mutex. Supervisor und mobile Lease/Mirror/Originalleser halten den Guard mit ihrem tatsächlichen Cancel bis zum wirklichen Ende. Der eine neue Reexport in `daemon/mod.rs` ist additiv.

Neue kohäsive Module: `sync_attempt`, `sync_state_json`, `sync_versions`, `sync_merge_recovery`, `sys_platform`; Domains registriert nur eigene neue Methoden/Module und den Problem-Hook. Öffentliche bestehende Conflict-/Run-Hüllen bleiben für vorhandene Fixtures erhalten.

Android ergänzt privaten `SyncScheduleReceiver`, `SCHEDULE_EXACT_ALARM`, einmalige Jobalarm-ID 3002 (Keepalive bleibt 3001), `SyncScheduleAlarm`, `MediaStoreSync`, `AndroidHostFigures`, `BlockDialog`, `VersionsScreen`, `SyncState`. Exaktes Recht wird vor Setzen und Zustellung geprüft; inexacte Alarme starten ausschließlich WorkManager. A-CLIENTs Task-Service/Keeper und MainActivity bleiben unverändert.

Die tatsächlichen Testquellnamen, erwarteten Remote-Suite-Signale, Entscheidungen und vollständigen gelesenen/geänderten/erstellten Dateipfade stehen in [Abnahme](../abnahme/AND-SYNC.md#vorhandene-abnahmequellen-und-finale-suite). Keine lokalen Tests/Builds und keine eigene CI-Ausführung.

