# Abnahme A-CLIENT

Stand: 2026-10-03, Fortsetzung des vorhandenen Blocks. Rust-Tests mit Präfix `review_task_` (Linux, Windows-2025-Job; die
`mobile`-Tests nur auf Linux/Android-Host), Kotlin-Unit-Tests unter `android/app/src/test`.

## Finaler Umsetzungsplan des Blocks

Vorhandene Teiländerungen bleiben erhalten. Kein neues Review; Self-Review gegen V1/V2 und die
Primärquellen. Kein lokaler Build oder Test; die folgenden Signale gehören gemeinsam in die vom
Hauptagenten geführte Remote-Task-Suite.

| Meilenstein | Betroffene Grenze | Konkretes erwartetes Ergebnis |
|---|---|---|
| Host-Routing und Berichte vollständig verbinden | `remote.rs`, Mobile-Analyse, Analyse-IPC, Desktop-Analyse | Host vor Walk; Hinweise, geschützte Bereiche und Host-Plattformzahlen bleiben sichtbar. Dasselbe Empfängerbudget erreicht Host und Decoder; IPC sendet komprimierte Daten. Alte Hosts behalten den sparsamen Rückfall. |
| V1/V2 durch Agent und Dienst reichen | Agent-Erweiterungen, `backend_ops.rs`, `backend_hash.rs`, Unavailable-Hülle | Fähigkeiten beziehen sich auf denselben Pfad und Peer; Host-Duplikate brauchen keine Datei-Downloads. Tolerante Listen/Hash-Walk-Auslassungen bleiben strukturiert; Abbruch erreicht laufende Arbeit ohne nächsten Eintrag. |
| Agent-Dateizugriff absichern | Agent-Protokoll und OS-Adapter | `special` bleibt im Draht erhalten. Hashen/Stage-Finishing folgen keinem ausgetauschten End-Link und warten nicht auf FIFOs. Metadatenänderungen betreffen das geprüfte Handle. NOREPLACE überschreibt auch im sicheren Hardlink-Rückfall kein fremdes Ziel. |
| Analyseorte erhalten | `StorageScanSource`, Picker, Desktop-Analyse/Aufräumen | Öffnen eines Ergebnisses erhält Endpunkt-Präfix und Konto; gleiche backendrelative Pfade verschiedener Verbindungen bleiben verschiedene Orte. |
| Fern-Duplikate sicher aufräumen | Desktop-Aufräumen, Mobile-Domäne, Kotlin-API/Ansicht | Nur ausgewählte, eindeutig adressierbare Kopien werden nach Host-Inhaltsprüfung verschoben. Mindestens eine Kopie bleibt; Änderungen/Fehler/Abbruch sind wiederholbar und erfolgreiche Verschiebungen werden einzeln gemeldet. |

Zweite Lückenrecherche, 2026-10-03: V1/V2-Signaturen aus `umsetzung.md`, bestehende
Agent-/IPC-Implementierungen und `docs/refs/local-fs-identity-durability.md` abgeglichen. Sichere
Handle-Zugriffe erneut gegen [open(2)](https://man7.org/linux/man-pages/man2/open.2.html),
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew) und
[File::set_permissions](https://doc.rust-lang.org/std/fs/struct.File.html#method.set_permissions)
geprüft: `O_NOFOLLOW` schützt den End-Link, `O_NONBLOCK` verhindert FIFO-Warten; unter Windows
Reparse-Handle prüfen und Daten-Reparse-Punkte erhalten. Vorhandene Sandbox/Wurzelprüfung bleibt
verantwortlich für Elternkomponenten. Plattformimporte bleiben in den jeweiligen OS-Adaptern.

## Rust-Meilensteintests

| Test | Datei | Erwartung |
|---|---|---|
| `review_task_remote_analysis_runs_on_the_host_worker` | `mobile/os/shared/domains/analyze_tests.rs` | FA1/A02/A15: ein entfernter Ort der App wird über `scan_remote` analysiert – `scan_storage` des Hosts einmal, **0** `list_dir`; Hinweise des Hosts bleiben erhalten |
| `review_task_local_results_answer_remote_flag_notes_and_release` | `analyze_tests.rs` | `analyze.start`/`reclaim.start` über den Dispatcher antworten `remote:false` für lokale Pfade; `analyze.issues` hat `notes`; `analyze.release`/`reclaim.release` → `released:true`, danach `analyze.node` → `not_found`; lokale Duplikatsuche findet die Gruppe |
| `review_task_issues_keep_notes_apart_from_read_problems` | `analyze_tests.rs` | A06: `text` nur Leseprobleme, `notes` getrennt (auch ohne Leseprobleme) |
| `review_task_remote_status_names_every_phase` | `domains/analyze_progress.rs` | FA1: Statuszeilen je Phase (Warten, Vorbereiten, Durchsuchen + Ordner, Zusammenstellen, Übertragen + `X von Y · P %`, Prüfen, älterer Pfad), „Letzte Meldung … vor N s“ ab 5 s; Übertragung als `doneBytes/totalBytes`; lokal unverändert |
| `review_task_results_trim_oldest_finished_within_budget` | `domains/analyze_results.rs` | A23/A07: über dem Speicherbudget gehen die ältesten fertigen Ergebnisse, laufende und das neueste bleiben |
| `review_task_results_tree_bytes_count_every_node` | `analyze_results.rs` | Speicherschätzung zählt jeden Knoten (iterativ) |
| `review_task_results_pending_slot_frees_without_result` | `analyze_results.rs` | Abbruch/Fehler/Panik ohne Ergebnis gibt den Platz frei; `release` entfernt ein Ergebnis genau einmal |
| `review_task_legacy_walk_counts_the_tree_and_names_only_old_peers` | `analytics/os/shared/remote.rs` | A37/A10: SSH-Agent-Walk ohne „beide Geräte aktualisieren“-Hinweis, alter Share-Peer mit Hinweis; Zähler = Baum (nachlaufender Fortschritt korrigiert) |
| `review_task_backend_walk_is_exact_serial_and_parallel` | `analytics/os/shared/analytics_backend_tests.rs` | B26/A15: Listing-Walker seriell und mit 4 Threads (Work-Stealing) exakt: Größen, Dateien, Ordner |
| `review_task_backend_walk_honours_the_retention_budget` | `analytics_backend_tests.rs` | A07: Budget wird eingehalten (Details fallen in Aggregate, Größen exakt, eine Notiz) |
| `review_task_backend_walk_keeps_folders_with_unusable_entries` | `analytics_backend_tests.rs` | A15b/A24-Client: nicht darstellbarer Dateiname zählt mit, nicht darstellbarer/doppelter Ordnername wird einzeln gemeldet, der Ordner bleibt lesbar; gleichnamige Dateien (Drive) zählen beide |
| `review_task_hashless_remote_reads_only_same_size_candidates` | `analytics/os/shared/reclaim/backend_tests.rs` | FA2-Rückfall/A01/A16: nur gleich große Kandidaten werden gelesen (kleine Dateien einmal), eine Datei eindeutiger Größe nie; Gruppe gefunden; `candidates`/`compared` stimmen |
| `review_task_agent_walk_failure_falls_back_to_listing` | `backend_tests.rs` | A03: Link-Grenze/Fehler im Baum → Listing-Walk von vorn, kein Doppelzählen, kein `root_error` |
| `review_task_share_location_never_walks_with_downloaded_hashes` | `backend_tests.rs` | A16: Share-Ort (Schema `Peer`) nutzt nie den emulierten Hash-Walk des Dienstes |
| `review_task_agent_walk_cancel_arrives_without_entries` | `backend_tests.rs` | A05: Abbruch erreicht den Walk ohne weiteren Eintrag (< 5 s, Walk sieht den Abbruch) |
| `review_task_candidate_memory_keeps_the_largest_files` | `backend_tests.rs` | A04: Kandidatenspeicher begrenzt Text, nicht Anzahl; die größten bleiben |
| `backend_retains_candidates_by_memory_and_groups_by_the_display_cap` (angepasst) | `backend_tests.rs` | Kandidaten nicht mehr auf `max_items` gekappt; Gruppenliste weiter auf `max_items` |
| `review_task_streaming_walks_leave_the_entry_limit_to_the_client` | `daemon/os/shared/backend_budget.rs` | A13: streamende Walks des Dienstes begrenzen nur die Tiefe; der Baum-Walk weiter Knoten/Text |
| `review_task_mounted_drive_paths_route_to_the_remote` | `app/core/analytics_mounts.rs` | A09: Pfad auf einem eingehängten Laufwerk → Backend-Pfad der Quelle (Wurzel und Unterordner), andere Pfade unverändert |

## Kotlin-Unit-Tests

| Test | Datei | Erwartung |
|---|---|---|
| `ReviewTaskCpuTasksTest.cpuIsNeededOnlyWhileARegisteredTaskRuns` | `android/app/src/test/.../service/` | FA1/A33: Wakelock nur, solange ein angemeldeter Fern-Task läuft; beendete werden vergessen |
| `ReviewTaskAnalyzeShapesTest.*` | `android/app/src/test/.../api/` | `AnalyzeIssues.notes`, `ReclaimSummary.compared` dekodieren; alte Antworten ohne die Felder ebenso |

## Suite-Stufen (Umgebung nötig)

- **Fern-Analyse über die Fassade (Gesamtablauf „Fern-Analyse“)**: `mobile::call("analyze.start",
  {location: "share://direct/<host>/Daten"})` gegen einen Share-Host im selben Prozess (Host-Baum mit vielen
  Ordnern, Zähler je `FsRequest` am Host). Erfolg: 0 `ListDir` für die Analyse, Ergebnisbaum = lokale Analyse
  des Hosts, Task-`message` durchläuft die Phasen, `analyze.issues.notes` trägt Host-Hinweise;
  `task.cancel` beendet den Host-Worker sofort. Gleich mit `reclaim.start`: alter Host (ohne
  `duplicate_search_v1`) → nur gleich große Kandidaten werden per `ReadAt` gelesen, Dateien eindeutiger Größe
  nie (Zähler `ReadAt` je Pfad am Host).
- **Android-Gerät (Emulator)**: Fern-Analyse eines Desktop-Hosts mit ausgeschaltetem Bildschirm;
  `adb shell dumpsys power | grep SmartExplorer:remote-task` zeigt die Sperre während des Tasks und nicht
  mehr danach; Ergebnis vollständig. Bestehender Gerätetest `AnalysisProtectedTaskTest` bleibt grün
  (zwei Analysen nacheinander, die erste bleibt abrufbar – Freigabe nur auf ausdrücklichen Aufruf oder bei
  Speichermangel).
- **Windows-Desktop**: Share als Laufwerk `Z:` eingehängt, „Scannen: Z:\“ → Host-Worker statt Walk durch
  das Laufwerk (Zähler am Host: 0 `ListDir`).


## Abschluss der Fortsetzung: Umsetzung und Fundzuordnung

Stand 2026-10-03: A-CLIENT-Aufrufer und Wire-/Android-Grenzen sind implementiert. Vorhandene
Teiländerungen wurden weitergeführt. Die folgende Zuordnung beschreibt Quellstand, keinen
bestandenen Lauf. Host-Ausführung und Host-/Peer-Scanner bleiben H-ANALYSIS zugeordnet.

| Befund/Plan | Konkrete Umsetzung im A-CLIENT-Block |
|---|---|
| FA1; A02/A06/A11/A15/A33/A37 | Gemeinsames `scan_remote`, lebende Konto-/Peer-Auflösung, echte Phasen und Host-Hinweise, explizite Ergebnisfreigabe und CPU-Hold. Legacy-Zähler werden am fertigen Baum berichtigt und frühere Segmente erhalten. |
| FA2; A01/A03/A04/A05/A16 | Host-Duplikatsuche zuerst, Agent-/Daemon-/Unavailable-Weiterreichung; bei fehlendem Angebot oder `None` bleibt derselbe sparsame Vergleich von Kandidaten gleicher Größe. Hash-Walk nur für Dateikandidaten, nicht als vollständiger Cleanup-Verzeichnisindex. Strukturierte Auslassungen bleiben erhalten. |
| A07/A19-Client/A23; B26 | Listing-Retention über `AnalyticsBudget::for_progress`, gemeinsames IPC-Knotenlimit und `AnalysisReceiver::with_node_budget`; vor Empfang gebundener SSH-`TreeDecodeBudget`. Ergebnisretention zählt auch Host-App-Vektoren, Namen und Diagnose-/Ortstexte. Größen und Zähler werden nicht wegen Detailaggregation gekürzt. |
| A09/A14/A18-IPC/A22 | Eingehängte Share-Pfade gelangen zur Host-Analyse; komprimiertes IPC-Ergebnis; Android merkt die Zahlen seines primären Volumes für den Android-Host. Desktop und Android zeigen Host-Volumen-/Plattformzahlen ohne Ersatz durch Client-Daten. Entfernte App-Details öffnen keine Client-App-Einstellungen. |
| A13/A24-Client; FA6 | Daemon-Streaming-Budget begrenzt die Tiefe, Client behält die Kandidatengrenze. Tolerante Einträge und Auslassungen in 1-MiB-Portionen; große Duplikatgruppen werden vollständig transportiert und zusammengeführt. `special` bleibt im Agent-Flag erhalten. |
| A34; FA6 | Desktop-/Android-Fern-Papierkorb erhält Such-SHA-256 und Größe; unbestätigte oder mehrdeutige Gruppen sind anzeigbar, aber nicht verschiebbar. Mindestens eine Kopie bleibt. Der Kern serialisiert je Suchergebnis, aktualisiert verbleibende Gruppen und meldet exakt erfolgreiche Orte auch nach Abbruch. |
| A35 | Picker und direkte Analyse-/Reclaim-Aufrufer behalten Backend, gespeichertes Konto und Endpunkt-Präfix; Explorer-Navigation verwendet bei bekannten Identitäten deren Gleichheit. Entfernte Literalnamen mit Backslash bleiben erhalten. |
| FA7/FS4/FS5, betroffene Agent-Grenze | Linux NOFOLLOW/NONBLOCK und Windows Reparse-Handle unterscheiden umleitende Links, besondere Dateien und normale Daten-Reparse-Punkte. Hashen im neuen und alten Walk verwendet den geprüften Handle und prüft Größe/Änderung/Abbruch. Stage-Mtime/Mode ändern dasselbe Handle. Standalone-Agent hat eine exklusive NOREPLACE-Leiter ohne App-Abhängigkeit. |
| V1 Watch-Abdeckung | `ReadyPartial` wird separat als `WireChange.kind=4` transportiert; volle `Ready` bleibt kind 0. Teilabdeckung kann keine vollständige Änderungsüberwachung behaupten. |
| Übergebener Remote-Fehler | Alle zugeordneten `WireMeta`-Literale besitzen `special` oder einen Default, insbesondere `backend_server.rs`; das konkret übergebene fehlende Feld ist korrigiert. |

## Zusätzliche Quelltests für die eine Remote-Suite

Diese Tests sind geschrieben, lokal nicht ausgeführt. Sie ergänzen die obenstehenden
Meilensteinsignale und gehören in denselben vom Hauptagenten orchestrierten Gesamtablauf.

| Test | Datei | Erwartung |
|---|---|---|
| `review_task_host_search_is_preferred_over_any_walk` | `analytics/os/shared/reclaim/backend_tests.rs` | Host-Suche für Desktop/Mobile; keine Kandidaten-Walks oder Downloads. |
| `review_task_withdrawn_host_search_keeps_the_sparse_fallback` | dieselbe Datei | Beworbenes Angebot antwortet `None`: Rückfall findet Kopien, eindeutige Größe wird nicht gelesen, keine verdoppelten Zähler. |
| `review_task_listing_fallback_honours_the_offered_node_budget` | `analytics/os/shared/analytics_backend_tests.rs` | Angebotenes Budget 2 wird mit Aggregatreserve eingehalten; vollständige Größe, Zusammenfassungshinweis. |
| `review_task_legacy_tree_budget_keeps_the_next_request_readable` | `agent_proto/core/extension_parts_task_tests.rs` | Knoten-/Frame-Budget greift vor unbeschränkter Allokation; danach ist eine andere Anfrage im selben Strom lesbar. |
| `review_task_extension_parts_keep_special_files_and_every_omission` | dieselbe Datei | Besondere Einträge und Auslassungen bleiben in begrenzten Portionen vollständig. |
| `review_task_large_duplicate_group_is_one_group_after_transport` | dieselbe Datei | Mehrteilige große Gruppe wird mit denselben Pfaden, SHA-256 und rückgewinnbaren Bytes wieder eine Gruppe. |
| `review_task_agent_watch_distinguishes_partial_coverage` | `agent/core/ext_wire.rs` | Ready und ReadyPartial bleiben durch binären Roundtrip verschieden. |
| `review_task_agent_regular_handles_refuse_leaf_links_and_fifos` | `agent_proto/os/linux_os/local_platform.rs` | Link/FIFO-Öffnen blockiert nicht; Rechteänderung nach Pfadaustausch betrifft nur das zuvor geprüfte Handle. |
| `review_task_agent_noreplace_hardlink_fallback_preserves_existing_target` | dieselbe Datei | Bestehendes Ziel wird nicht überschrieben; frisches Ziel wird exklusiv veröffentlicht. |
| `review_task_legacy_walk_keeps_earlier_segment_counters` | `analytics/os/shared/remote.rs` | Nach fertigem Legacy-Baum bleiben die vorherigen Datei-/Bytezähler erhalten. |
| `review_task_remote_recycle_rejects_unverified_and_ambiguous_copies` | `analytics/os/shared/reclaim/backend_recycle.rs` | MD5, ungültiger SHA-256, Mehrdeutigkeit und Einzelkopie autorisieren keine Verschiebung; gültige Kopie trägt erwarteten Hash. |
| `review_task_remote_trash_never_takes_the_last_copy` | `app/core/reclaim_remote_trash.rs` | Desktop bewahrt eine Kopie und reicht den Such-SHA-256 unverändert weiter. |
| `review_task_host_app_figures_participate_in_result_retention` | `mobile/os/shared/domains/analyze_results.rs` | Host-App-Arrays belegen Retentionsbudget; beim nächsten Ergebnis wird das alte große Ergebnis freigegeben. |
| `ReviewTaskAnalyzeShapesTest.hostFiguresAndContentBoundTrashRemainOptional` | Kotlin-API-Unit-Datei | Host-Zahlen/Remote-Flag, Papierkorbfähigkeit, überprüfter Inhalt und exakte Erfolgsorte; Legacy-Defaults lassen Antworten dekodieren und behaupten keine Papierkorbfähigkeit. |

Zusätzliche Umgebungssignale für die bereits geplante Fassade:

- Neuer Host: `analyze.start` und `reclaim.start` über `mobile::call`; Host-Worker liefert das Ergebnis,
  kein vollständiger Client-Dateidownload. Gleicher kleiner Knotenetat durch IPC/Peer/Host, anschließend
  älterer Baum-Walk und Listing-Rückfall mit vollständiger Größe und begrenzter Detailansicht.
- Zwei gespeicherte Konten/Präfixe mit gleichem relativen Pfad; Ergebnisse nacheinander im Explorer öffnen
  und erneut als Ort wählen. Die jeweilige Verbindung, literal Backslash und Unterwurzel bleiben erhalten.
  Volumen-/App-Zahlen eines Android-Hosts erscheinen auch auf Desktop/anderem Android ohne Client-Ersatz.
- Fern-Papierkorb nach Host-Suche: eine Kopie vor der Aktion ändern, unabhängige Kopie erfolgreich
  verschieben, Aktion abbrechen und wiederholen. Erfolgsorte sind exakt, Fehler bleiben wählbar,
  Host-Inhaltsprüfung verhindert den geänderten Pfad und der verbliebene letzte Pfad bleibt bestehen.
- Abbruch bei leerer bzw. voller Hash-/Watch-Queue erreicht den Host ohne nächsten Dateieintrag;
  Teil-Watch-Bereitschaft erhält Abfragen. Linux-Spezialdateien und Windows umleitende Reparse-Punkte
  bleiben Auslassungen; normale Daten-Reparse-Punkte behalten ihre bisherigen Dateisemantiken.

## Entscheidungen und verbleibende Integration

- Keine lokalen Builds, Compiler, Formatter, Tests, Server, Installationen oder CI-Anstöße durch diesen
  Block. Kein Commit/Push, Graph-Neubau oder Release; keine weiteren Agenten. Self-Review der eigenen
  Grenzänderungen anhand V1–V5, aktueller Quelltypen und der genannten Primärquellen.
- Keine neue VFS-API für Bereichslesungen. Der optionale Leistungswunsch bleibt in `anfragen/A-CLIENT.md`;
  bestehende Backends und alte gespeicherte Orte behalten ihre Bedeutung. Schutzfunktionen erfinden
  keine Unix-Rechte oder Volumenidentität des Client-Geräts, wenn der Host sie nicht anbietet.
- Binäres Agent-Protokoll 11 verlangt passende Nutzlasten im gemeinsamen Remote-Release. Additive
  Kotlin-/IPC-/Desktop-Felder und Legacy-Defaults sind in `api-delta/A-CLIENT.md` aufgeführt;
  AND-SHARE-UI übernimmt die zentrale API-Dokumentation.
- Host-/Peer-Implementierung gehört H-ANALYSIS; dessen letzte Progress-Flag-Rücksetzung ist im
  read-only-Quellstand bestätigt. Die konkrete Integrationsmeldung und der Status stehen in `anfragen/A-CLIENT.md`. Remote-Suite und Release bleiben
  beim Hauptagenten; deren Erfolg wird hier nicht vorweggenommen.

## Dateien dieser Fortsetzung

Neue zusammenhängende Featuredateien unmittelbar neben zugeordneten Dateien:

- `native/src/agent/core/analysis.rs`
- `native/src/agent_proto/core/extension_parts.rs`
- `native/src/agent_proto/core/extension_parts_task_tests.rs`
- `native/src/analytics/os/shared/reclaim/backend_recycle.rs`
- `native/src/daemon/os/shared/ipc_backend_extensions.rs`
- `native/src/mobile/os/shared/domains/analyze_recycle.rs`


Geänderte zugeordnete Quellfläche im Arbeitsstand, vorhandene Teiländerungen eingeschlossen.
Registrierungen in `analytics/mod.rs`, `daemon/mod.rs` und `domains/mod.rs` sind nur eigene additive
Einträge; Registrierungen anderer Blöcke wurden nicht umgebaut:

- `android/app/src/main/java/app/smartexplorer/android/api/AnalyzeApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/TaskForegroundService.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/TaskKeeper.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/AnalysisParts.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/AnalysisScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/AnalysisViewModel.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/AppDetailDialog.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/DuplicatesScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/DuplicatesViewModel.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/ScanPages.kt`
- `android/app/src/test/java/app/smartexplorer/android/api/ReviewTaskAnalyzeShapesTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/service/ReviewTaskCpuTasksTest.kt`
- `native/src/agent/core/backend.rs`
- `native/src/agent/core/ext_wire.rs`
- `native/src/agent/core/extensions.rs`
- `native/src/agent/core/metadata.rs`
- `native/src/agent/core/mux.rs`
- `native/src/agent/core/route.rs`
- `native/src/agent/core/transport.rs`
- `native/src/agent/core/walk.rs`
- `native/src/agent/mod.rs`
- `native/src/agent_proto/core/codec.rs`
- `native/src/agent_proto/core/codec_tests.rs`
- `native/src/agent_proto/core/credit.rs`
- `native/src/agent_proto/core/features.rs`
- `native/src/agent_proto/core/frame_encode.rs`
- `native/src/agent_proto/core/frame_io.rs`
- `native/src/agent_proto/core/frame_ops.rs`
- `native/src/agent_proto/core/node_codec.rs`
- `native/src/agent_proto/core/ops_types.rs`
- `native/src/agent_proto/core/server.rs`
- `native/src/agent_proto/core/types.rs`
- `native/src/agent_proto/core/vault_frame_task_tests.rs`
- `native/src/agent_proto/mod.rs`
- `native/src/agent_proto/os/linux_os/local_platform.rs`
- `native/src/agent_proto/os/shared/ext_ops.rs`
- `native/src/agent_proto/os/shared/fs.rs`
- `native/src/agent_proto/os/shared/hash.rs`
- `native/src/agent_proto/os/shared/transfer.rs`
- `native/src/agent_proto/os/windows/local_platform.rs`
- `native/src/analytics/mod.rs`
- `native/src/analytics/os/shared/analytics_backend.rs`
- `native/src/analytics/os/shared/analytics_backend_tests.rs`
- `native/src/analytics/os/shared/reclaim/backend.rs`
- `native/src/analytics/os/shared/reclaim/backend_agent.rs`
- `native/src/analytics/os/shared/reclaim/backend_compare.rs`
- `native/src/analytics/os/shared/reclaim/backend_compare_tests.rs`
- `native/src/analytics/os/shared/reclaim/backend_duplicates.rs`
- `native/src/analytics/os/shared/reclaim/backend_tests.rs`
- `native/src/analytics/os/shared/remote.rs`
- `native/src/app/core/analytics_core.rs`
- `native/src/app/core/analytics_mounts.rs`
- `native/src/app/core/analytics_ui.rs`
- `native/src/app/core/app_models.rs`
- `native/src/app/core/picker_impl.rs`
- `native/src/app/core/reclaim_core.rs`
- `native/src/app/core/reclaim_remote_trash.rs`
- `native/src/app/core/reclaim_ui.rs`
- `native/src/daemon/mod.rs`
- `native/src/daemon/os/shared/backend_budget.rs`
- `native/src/daemon/os/shared/backend_hash.rs`
- `native/src/daemon/os/shared/backend_ops.rs`
- `native/src/daemon/os/shared/backend_server.rs`
- `native/src/daemon/os/shared/backend_walk.rs`
- `native/src/daemon/os/shared/ipc.rs`
- `native/src/daemon/os/shared/ipc_analysis.rs`
- `native/src/daemon/os/shared/ipc_client.rs`
- `native/src/daemon/os/shared/ipc_protocol.rs`
- `native/src/mobile/os/shared/domains/analyze.rs`
- `native/src/mobile/os/shared/domains/analyze_platform.rs`
- `native/src/mobile/os/shared/domains/analyze_progress.rs`
- `native/src/mobile/os/shared/domains/analyze_results.rs`
- `native/src/mobile/os/shared/domains/analyze_tests.rs`
- `native/src/mobile/os/shared/domains/mod.rs`

Eigene erstellte/aktualisierte Berichte:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/A-CLIENT.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/A-CLIENT.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/A-CLIENT.md`


Gelesene Dateien: die folgenden Quellpfade wurden im Scope statisch für die Vertragsinventur
(`ScanPhase`, `ChangeNotice`, `WireMeta`-Literale und Dateigrenzen) gelesen; die oben genannten Grenzen
wurden zusätzlich gezielt im Self-Review gelesen. Dies behauptet kein neues Repository-Review.

- `android/app/src/main/java/app/smartexplorer/android/api/AnalyzeApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/TaskForegroundService.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/TaskKeeper.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/AnalysisParts.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/AnalysisScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/AnalysisViewModel.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/AppDetailDialog.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/DuplicatesScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/DuplicatesViewModel.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/ScanPages.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/Treemap.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/analytics/TreemapView.kt`
- `android/app/src/test/java/app/smartexplorer/android/api/ReviewTaskAnalyzeShapesTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/service/ReviewTaskCpuTasksTest.kt`
- `native/src/agent/core/agent_error.rs`
- `native/src/agent/core/analysis.rs`
- `native/src/agent/core/backend.rs`
- `native/src/agent/core/batch_get.rs`
- `native/src/agent/core/batch_put.rs`
- `native/src/agent/core/deploy.rs`
- `native/src/agent/core/engine_ops.rs`
- `native/src/agent/core/error_tests.rs`
- `native/src/agent/core/ext_wire.rs`
- `native/src/agent/core/extensions.rs`
- `native/src/agent/core/heartbeat_tests.rs`
- `native/src/agent/core/lanes.rs`
- `native/src/agent/core/metadata.rs`
- `native/src/agent/core/mux.rs`
- `native/src/agent/core/mux_tests.rs`
- `native/src/agent/core/pool.rs`
- `native/src/agent/core/remote_drive_task_deploy_tests.rs`
- `native/src/agent/core/route.rs`
- `native/src/agent/core/search.rs`
- `native/src/agent/core/stream.rs`
- `native/src/agent/core/tests.rs`
- `native/src/agent/core/transfer.rs`
- `native/src/agent/core/transfer_engine_task_service_tests.rs`
- `native/src/agent/core/transfer_engine_task_tests.rs`
- `native/src/agent/core/transport.rs`
- `native/src/agent/core/walk.rs`
- `native/src/agent/mod.rs`
- `native/src/agent_proto/core/batch_limits.rs`
- `native/src/agent_proto/core/codec.rs`
- `native/src/agent_proto/core/codec_tests.rs`
- `native/src/agent_proto/core/credit.rs`
- `native/src/agent_proto/core/extension_parts.rs`
- `native/src/agent_proto/core/extension_parts_task_tests.rs`
- `native/src/agent_proto/core/features.rs`
- `native/src/agent_proto/core/frame_encode.rs`
- `native/src/agent_proto/core/frame_ext.rs`
- `native/src/agent_proto/core/frame_io.rs`
- `native/src/agent_proto/core/frame_ops.rs`
- `native/src/agent_proto/core/node_codec.rs`
- `native/src/agent_proto/core/ops_types.rs`
- `native/src/agent_proto/core/relative_path.rs`
- `native/src/agent_proto/core/remote_drive_task_tests.rs`
- `native/src/agent_proto/core/server.rs`
- `native/src/agent_proto/core/server_bulk_task_tests.rs`
- `native/src/agent_proto/core/server_session.rs`
- `native/src/agent_proto/core/server_tests.rs`
- `native/src/agent_proto/core/session.rs`
- `native/src/agent_proto/core/transfer_engine_task_bounds_tests.rs`
- `native/src/agent_proto/core/transfer_engine_task_tests.rs`
- `native/src/agent_proto/core/transport_error.rs`
- `native/src/agent_proto/core/types.rs`
- `native/src/agent_proto/core/vault_frame_task_tests.rs`
- `native/src/agent_proto/mod.rs`
- `native/src/agent_proto/os/linux_os/local_platform.rs`
- `native/src/agent_proto/os/linux_os/sandbox.rs`
- `native/src/agent_proto/os/shared/batch_get.rs`
- `native/src/agent_proto/os/shared/batch_put.rs`
- `native/src/agent_proto/os/shared/ext_ops.rs`
- `native/src/agent_proto/os/shared/fs.rs`
- `native/src/agent_proto/os/shared/hash.rs`
- `native/src/agent_proto/os/shared/promotion.rs`
- `native/src/agent_proto/os/shared/put_tree.rs`
- `native/src/agent_proto/os/shared/search.rs`
- `native/src/agent_proto/os/shared/stage_ops.rs`
- `native/src/agent_proto/os/shared/transfer.rs`
- `native/src/agent_proto/os/shared/write_new.rs`
- `native/src/agent_proto/os/windows/local_platform.rs`
- `native/src/analytics/core/analysis_transfer.rs`
- `native/src/analytics/core/progress.rs`
- `native/src/analytics/core/storage_view.rs`
- `native/src/analytics/core/tree_deflate.rs`
- `native/src/analytics/core/tree_transfer.rs`
- `native/src/analytics/mod.rs`
- `native/src/analytics/os/shared/analytics_backend.rs`
- `native/src/analytics/os/shared/analytics_backend_tests.rs`
- `native/src/analytics/os/shared/analytics_budget.rs`
- `native/src/analytics/os/shared/reclaim/backend.rs`
- `native/src/analytics/os/shared/reclaim/backend_agent.rs`
- `native/src/analytics/os/shared/reclaim/backend_compare.rs`
- `native/src/analytics/os/shared/reclaim/backend_compare_tests.rs`
- `native/src/analytics/os/shared/reclaim/backend_duplicates.rs`
- `native/src/analytics/os/shared/reclaim/backend_recycle.rs`
- `native/src/analytics/os/shared/reclaim/backend_tests.rs`
- `native/src/analytics/os/shared/remote.rs`
- `native/src/app/core/analytics_access.rs`
- `native/src/app/core/analytics_accessibility.rs`
- `native/src/app/core/analytics_core.rs`
- `native/src/app/core/analytics_mounts.rs`
- `native/src/app/core/analytics_paint.rs`
- `native/src/app/core/analytics_ui.rs`
- `native/src/app/core/app_models.rs`
- `native/src/app/core/picker_impl.rs`
- `native/src/app/core/reclaim_core.rs`
- `native/src/app/core/reclaim_remote_trash.rs`
- `native/src/app/core/reclaim_results_ui.rs`
- `native/src/app/core/reclaim_ui.rs`
- `native/src/bisync/core/plan_types.rs`
- `native/src/bisync/core/run_types.rs`
- `native/src/bisync/core/types.rs`
- `native/src/bisync/mod.rs`
- `native/src/bisync/os/shared/apply.rs`
- `native/src/bisync/os/shared/snapshot.rs`
- `native/src/connect/core/location.rs`
- `native/src/connect/mod.rs`
- `native/src/connect/os/shared/resolution.rs`
- `native/src/daemon/mod.rs`
- `native/src/daemon/os/shared/backend_batch.rs`
- `native/src/daemon/os/shared/backend_budget.rs`
- `native/src/daemon/os/shared/backend_hash.rs`
- `native/src/daemon/os/shared/backend_ops.rs`
- `native/src/daemon/os/shared/backend_server.rs`
- `native/src/daemon/os/shared/backend_stream.rs`
- `native/src/daemon/os/shared/backend_transfer.rs`
- `native/src/daemon/os/shared/backend_tree_send.rs`
- `native/src/daemon/os/shared/backend_walk.rs`
- `native/src/daemon/os/shared/ipc.rs`
- `native/src/daemon/os/shared/ipc_analysis.rs`
- `native/src/daemon/os/shared/ipc_backend_extensions.rs`
- `native/src/daemon/os/shared/ipc_client.rs`
- `native/src/daemon/os/shared/ipc_protocol.rs`
- `native/src/daemon/os/shared/ipc_protocol_bounds.rs`
- `native/src/daemon/os/shared/request_workers.rs`
- `native/src/keep_awake/mod.rs`
- `native/src/lib.rs`
- `native/src/mobile/core/error.rs`
- `native/src/mobile/os/shared/domains/analyze.rs`
- `native/src/mobile/os/shared/domains/analyze_platform.rs`
- `native/src/mobile/os/shared/domains/analyze_progress.rs`
- `native/src/mobile/os/shared/domains/analyze_recycle.rs`
- `native/src/mobile/os/shared/domains/analyze_results.rs`
- `native/src/mobile/os/shared/domains/analyze_tests.rs`
- `native/src/mobile/os/shared/domains/mod.rs`
- `native/src/mobile/os/shared/runtime.rs`
- `native/src/share/core/export_config.rs`
- `native/src/share/core/fs_request.rs`
- `native/src/share/core/fs_response.rs`
- `native/src/share/core/relation_rights.rs`
- `native/src/share/core/types.rs`
- `native/src/share/core/wire_capabilities.rs`
- `native/src/share/mod.rs`
- `native/src/support_dirs.rs`
- `native/src/syncjobs/core/types.rs`
- `native/src/syncjobs/mod.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/extension_types.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/mod.rs`
- `native/src/watch/mod.rs`

Gezielt gelesene Dokumentation und Scope:

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/lesungen/INDEX.md`
- `docs/refs/INDEX.md`
- `docs/refs/android-storage-scan.md`
- `docs/refs/local-fs-identity-durability.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-analyse.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sicherheit.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/A-CLIENT.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/A-CLIENT.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/A-CLIENT.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/a-client.json`


Zusätzlich angewandt/gelesen: `/root/.codex/skills/arbeitsweise/SKILL.md`,
`/root/.codex/skills/graphify/SKILL.md` und passende Skill-Referenzen; die anfängliche Graph-Abfrage
(`TaskCtx`, `AgentBackend`, `PeerBackend`, `Loc`) kam vom Hauptagenten. Keine Graph-Erneuerung.


## Statische Abschlusskontrollen

Der Scope wurde als JSON geparst. Der gezielte `git diff --check` über zugeordnete Änderungsdateien,
eigene Registrierungen, neue Featuredateien und die drei Berichte ist sauber. Die zugeordneten
bedeutend geänderten Rust-Dateien liegen unter 500 Zeilen und 50 KiB. Die scoped Textinventur findet
nur gültige `ScanPhase`-/`ChangeNotice`-Varianten und kein `WireMeta`-Literal ohne `special` oder Default
(einschließlich Rust-Feldkurzschreibweise). Die drei eigenen Berichte enthalten keine überholten
Teil-2-Wartevermerke oder ungeprüften Release-Erfolgsbehauptungen. Dies sind Text-/Parsing-Kontrollen;
kein Compiler, Formatter oder Test wurde lokal gestartet.
