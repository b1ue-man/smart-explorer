# AND-SYNC – Anfragen und Fremdgrenzen

Stand: 2026-10-03. Der eigene Consumerblock ist in Quellen abgeschlossen. [Abnahme](../abnahme/AND-SYNC.md) und [API-Delta](../api-delta/AND-SYNC.md) enthalten die Umsetzung, konkreten Signale und Verträge. Das [exakte gelesene/geänderte/erstellte Inventar](../abnahme/AND-SYNC.md#exaktes-dateiinventar) gilt unverändert auch für diesen Bericht; keine zusätzlichen Lese- oder Schreibpfade.

## Erledigte gezielte Anfragen

- Leserechte für shared JobState/Editor/Versionen/Resolve/Orchestration, CatchUpStatus/ProblemNotice, CoreEvent(s), ServiceNotifications/AppNav und VFS Regular-Read wurden vom Parent erteilt und konsumiert.
- Kleiner Daemon-Grant umgesetzt: initiale Allfiles-Denial vor Cancelprüfung, processlokales Weak-Register in host_state, tatsächlicher Worker-Guard im job_supervisor und genau eigener additiver Reexport. Keine neue Daemon-State-/Pair-/Instanzlogik.
- Eigene Android StorageStats-Produzentenbrücke benötigt keine Änderung an AnalyzeApi/StorageStatsAccess. Nur HostMonitor/sys_platform und vorhandene öffentliche Figuren-/Platform-APIs; gemeinsamer Parser aus analyze_platform extrahiert und unverändert delegiert.
- Recorded Merge ist angeschlossen. Freigegebene Originalpfad-API liefert persistierte Seitenschreibweisen auf ursprünglichen Endpunkten. Vollständige Originalbytes, Beobachtungssignaturen und StateKey werden an E-APPLY gegeben; keine eigene Merge-/Hash-/Path-Encoder-Fallback-Implementierung.
- Nachgereichte read-only Recovery-API ist angeschlossen. Fresh-Check und Prozessneustart zeigen gespeicherten Auftrag/Bestätigungsseiten; eigene `sync.mergeRetry`-Aktion wiederholt unveränderte Wahl. Ursprünglicher Draft bzw. dauerhafte Originalbytes bleiben erhalten. E-APPLYs zuvor fehlende API-Datei ist inzwischen vorhanden und gelesen.
- Problem-Emitter/Decoder wurden ausschließlich additiv registriert. MainActivity und bestehender privater AppNav-PendingIntent-Vertrag bleiben unverändert. A-CLIENT TaskForegroundService/TaskKeeper bleiben erhalten.

## Enger Engine-Handoff

1. **Atomarer Konfliktauflösungs-Guard.** Parent wurde informiert: `resolve_recorded` muss innerhalb seiner eigenen durchgehend gehaltenen PairLock einen gleichzeitig neu entstandenen Pending-Merge derselben Rel ablehnen. UI sperrt A/B-Aktionen vorhandener Pending-Einträge; Native prüft `pending_merge_for_key` vor Resolve. Dieser read-only Precheck besitzt die nachfolgende Resolve-Sperre nicht und ersetzt den atomaren Engine-Check daher nicht. Änderung/Testquelle beim E-APPLY-Owner, keine zusätzliche Consumer-Änderung angefragt.
2. **Normale Läufe gegen offene Recovery.** E-APPLYs `pending_merge_relatives(lock,key)` ist Engine-Vertrag: offene Original- und KeepBoth-Siblingpfade auf beiden Seiten vor regulärer Planung, vollständigem Index und Konvergenz-Checkpoint schützen/deferieren. Parent hat diesen Anschluss an E-ENGINE weitergegeben. Außerhalb des AND-SYNC-Lesescope wurde die fremde Implementierung nicht erneut erkundet. Remote-Signal: ein normaler Lauf zwischen Teilpublikation und ausdrücklichem Retry verändert diese Originale/Recoverybasis nicht.

Beide Grenzen gehören zur gemeinsamen Integration; der eigene Consumer ist fertig und behauptet keine außerhalb des Scopes geprüfte Engine-Durchsetzung.

## Bewusst begrenzter Dienstvertrag

Nach Parent-Entscheidung ist `deferScheduling` reine Admission. Dienstverlust bzw. später veränderte Batterie-/Netzbedingungen sperren neue unabhängige Daemonstarts, führen aber keinen globalen Cancel schon laufender unabhängiger Daemon- oder manueller Tasks aus. Das ist sichtbar dokumentiert und muss bei einem späteren globalen Constraint-Abbruch gezielt am zuständigen Lauf-Owner verbunden werden.

Eigene WorkManagerfenster werden dagegen bei Stop/Timeout über ihre tatsächliche Catch-up-/Task-ID abgebrochen; Shared-Storage-Rechteverlust erreicht alle ausgewählten tatsächlichen Cancel-Arcs. Native RAII hält Guard/Runmark/Wake bis zum Ende, auch wenn ein einzelner blockierender Backendaufruf Cancel erst verzögert beantwortet. Keine Behauptung, ein Android-FGS allein halte die CPU wach.

## Parent-Abnahme und Abschluss

Einzige finale Remote-Task-Suite, Graph-Aktualisierung, Integration, Commit/Push und Release gehören dem Parent. [Abnahmequellen und konkreten Signale](../abnahme/AND-SYNC.md#vorhandene-abnahmequellen-und-finale-suite) decken Zustandsversuche, Bestätigung einmal, Storage-Revoke-Race/Targetauswahl, manifestes Restore, MediaStore-Hybrid, tatsächliche Jobalarme/Screen-off, Worker-Retry/Cancel, Notificationroute, echte primäre Hostzahlen und Recorded-Merge-Neustart ab.

Dieser Agent hat keine lokale Compiler-/Build-/Test-/Formatter-/Server-/Installations-, Commit-/Push-/CI-/Graph-/Release-Aktivität ausgeführt. Keine weitere außer-scope Anfrage für den fertigen Consumer; nach Übergabe stoppen.

