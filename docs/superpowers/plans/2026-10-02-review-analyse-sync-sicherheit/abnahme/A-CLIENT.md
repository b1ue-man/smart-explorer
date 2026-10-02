# Abnahme A-CLIENT

Stand: 2026-10-02, Teil 1. Rust-Tests mit Präfix `review_task_` (Linux, Windows-2025-Job; die
`mobile`-Tests nur auf Linux/Android-Host), Kotlin-Unit-Tests unter `android/app/src/test`.

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
