# H-ANALYSIS – Umsetzung und Abnahme

Stand: 2026-10-03. Bestehender RV1-Plan, Vertrag V2; keine neue Review-Runde. Der Block wird ausschließlich
statisch bearbeitet. Builds, Formatierung und alle `review_task_`-Abnahmen gehören zur abschließenden
Remote-Suite des Hauptagenten.

## Detailplan vor der Umsetzung

1. Analyseübertragung: vorhandene Berichtsfelder/Deflate fortsetzen, beliebige Deflate-Stückgrenzen korrekt
   dekodieren, empfangene Baumgröße gegen das eigene Budget prüfen; Abschlussgrößen aus dem verifizierten
   Baum übernehmen. Erwartung: komprimierte/unterschiedlich portionierte Daten liefern denselben Baum;
   manipulierte Längen/Hash/Struktur bleiben Fehler (A10/A18/A23/A25/A29).
2. Host-Analyse: ein Budget für sämtliche Freigaben, physisch überlappende Wurzeln einmal scannen,
   geschützte Bereiche und Android-Zahlen strukturiert weitergeben, alle Pfadformen sichtbar abbilden;
   sicherer Directory-Adapter ersetzt die Auflösung je Ordner. Erwartung: lokal/Fern dieselben Größen,
   keine fremden Pfade, keine Link-Flucht (A06/A08/A14/A17/A19/A22/A26/A27/A36/A37).
3. Gemeinsame Host-Arbeitsverteilung und Wiederanbindung: pro authentifiziertem Gerät höchstens ein Worker;
   andere Geräte passieren wartende Mehrfachanfragen desselben Geräts. Analyse-/Duplikatergebnisse werden
   unter Principal, Auftrags-ID, Anfrageparametern und Freigabezustand für zehn Minuten aufbewahrt,
   stromweise von einer privaten Zwischendatei wiedergegeben. Expliziter Abbruch beendet die Arbeit,
   Transportverlust erlaubt einmalige Wiederanbindung. Erwartung: kein neuer Scan nach Verbindungsverlust,
   kein Abruf unter fremder Identität/anderen Wurzeln (A20/A21).
4. Host-Anfragen: lokale Duplikatsuche mit dem vorhandenen Finder, Hash-Walk mit Einzelauslassungen und
   sicherem Lesen, tolerante Listen in begrenzten Portionen, Watch-Abonnement, geprüfter Papierkorb und
   Stage-Abschluss/Dauerhaftigkeit. Erwartung: kein Dateiinhalt beim Peer für Duplikatsuche; unabhängige
   Dateien trotz Link/Lesefehler; große Listen ohne Einzelframe-/Gesamtlauf-Frist (A01/A03/A05/A16/A24/A34).
5. Peer-Erweiterungen: V1-Haken und Fähigkeiten vollständig übersetzen; Stromabbruch und Kanalrückstau
   dürfen keine Teilliste als vollständig markieren. Legacy-Snapshot nutzt denselben schnellen Worker
   und eine begrenzte Detailansicht. Erwartung: alte Peers behalten Grundfunktionen und erhalten lesbare
   Teilbäume, neue Peers erhalten die V2-Erweiterungen (A12/A28/FA7).
6. Self-Review und Abnahmefälle: ausschließlich eigene Änderungen; Zuordnung der Befunde, konkrete
   Tests/Suite-Stufen und fremde Integrationsanfragen in diesen beiden Blockdateien festhalten.

Lokale Grundlagen: `docs/ARCHITEKTUR.md`, RV1 `spec.md` FA2/FA4–FA7, `umsetzung.md` V1/V2/V4,
`recherche.md` E10/E11; gesicherte Primärsyntax `docs/refs/sync-remote-metadata.md` (flate2),
`local-fs-identity-durability.md` (sicheres Öffnen), `android-storage-scan.md` (geschützte Bereiche).
Die zweite Lückenprüfung ergab notwendige OS-Adapter für sichere Verzeichnisse, Volumenbelegung und
Papierkorb; der Hauptagent koordiniert diese Grenzen, ohne fremde Dateien durch H-ANALYSIS zu ändern.

Kompatibilität: Freigabe-/Verbindungsidentität und Backends bleiben erhalten; Remote-Pfade gehen nie
durch lokale Dateisystem-APIs. Links/Junctions/Spezialdateien sind Auslassungen, keine Abwesenheit.
Unlesbare Kinder schützen ihre Gegenstücke im Hash-Walk. Keine Änderungen an export_config.rs.

## Ergebnis und Befundzuordnung

Der eigene Host-/Protokollblock ist implementiert und statisch abgeschlossen. Die abschließende
Integration und die einzige Remote-Suite bleiben beim Hauptagenten. FA6 ist für Windows ausdrücklich
noch keine vollständige Papierkorb-Abnahme; der Host bietet diese Fähigkeit dort nicht an.

| Dokumentierte Befunde | Umsetzung in diesem Block | Abnahmesignal der gemeinsamen Remote-Suite |
| --- | --- | --- |
| A01/A03/A04/A05/A13/A16, FA2 | Lokale Freigaben benutzen den gemeinsamen Host-Finder mit Größe, Fingerprint und vollständiger SHA-256. Geschützte/unlesbare Kinder werden einzeln gemeldet; große Gruppen werden mit `more` portioniert. Bei fremden Provider-Backends werden MD5-/Providergruppen am Host mit SHA-256 verifiziert. | Dateiinhalt bleibt auf dem Host; gleich große veränderte Dateien bilden keine bestätigte Gruppe; unabhängige Dateien bleiben trotz Auslassungen nutzbar; Abbruch wirkt während Lesen/Warten. |
| A08/A17/A19/A36 | Ein ScanBudget gilt für sämtliche Wurzeln. Physisch gleiche/überlappende lokale Freigaben werden einmal besucht; andere Backend-/Verbindungsidentitäten bleiben getrennt. DirectoryHandle öffnet Kinder relativ zum gehaltenen Elternhandle. | Kein Canonicalize pro Kindordner, keine Link-Flucht, exakte Größen ohne Doppelzählung, gemeinsames Empfängerbudget inklusive Container und Aggregate. |
| A06/A14/A22/A26/A27/A37 | Strukturierte geschützte Bereiche und Legacy-Hinweise, Host-Volumenwerte, Android-Host-Figuren, sichtbare Pfadzuordnung einschließlich Windows-Anzeigeformen. Host öffnet ordinary; lokal konsentierte Duplikatsuche behält `open_root_consented`. Provider-/Legacy-Grenzen werden sachlich gemeldet. | Kein physischer Hostpfad beim Peer; keine vom Client übernommenen Volumenzahlen; lokale Broker-Fähigkeit bleibt erhalten. Android-Produzent siehe Anfrage 3. |
| A07/A10/A18/A23/A25/A29 | Deflate ist ein durchgängiger Strom, Baumparser akzeptiert beliebige Stückgrenzen. Größen/Shape/Hash/StreamEnd und Empfängerspeicher werden geprüft; Abschlussgrößen stammen aus dem verifizierten Baum. Progress-Budget gilt in scoped/remote_segment. Ergebnis-Figuren zählen gehaltene Kapazitäten mit. | Neu portionierter Strom liefert denselben Baum; Manipulation/Trunkierung wird zurückgewiesen; ein kleiner Empfänger lehnt ein zu großes Ergebnis vor der Baumallokation ab; finale Bytes ersetzen abweichende Live-Zähler. |
| A20/A21, FA5 | Gerätefaire CPU-Queue, separate Listen-/Watch-Aufnahme, eigene Scanthreads mit KeepAwake. Zehnminütige Wiederanbindung an privaten Spool; Schlüssel enthält vollständigen Principal, ID, Parameter und geprüfte Export-/Lease-/Restriktionsbindung. | Zweite Anfrage desselben Geräts blockiert kein anderes Gerät; Transportverlust startet keinen neuen Scan; fremde Parameter/Identität/Rechte werden abgewiesen; expliziter STOP beendet statt Retention. |
| A12/A24/A28, FA7 | Tolerante ListDirBatch-Portionen, genaue Done-Zähler, explizite Auslassungen. Legacy-Snapshot nutzt den Host-Scanner und faltet Detailknoten innerhalb der tatsächlichen alten Drahtgrenzen. | Große/teilweise unlesbare Listen benötigen keinen einzelnen Gesamtframe; Teillisten werden nie als vollständig ausgegeben; alte Peers behalten ihre Grundfunktionen. |
| A34, FA6, V5 | Erwartete Länge/SHA vor und nach identitätsgebundenem Quarantäne-Einfang; Linux freedesktop-Trash, Android vorhandenes Apptrash-Record/Restore-Schema. Veröffentlichung ohne Ersetzung; Fehler stellen ohne Ersetzung wieder her. Zentrale OS-Fähigkeit maskiert Windows. | Änderungsrennen treffen keine Ersatzdatei; fehlgeschlagene Veröffentlichung erhält den Inhalt; Restore-Konflikt nennt retained_location; Windows `remote_trash_v1=false`. Offener nativer Windows-Anschluss siehe Anfrage 1. |
| V1/V2/V4, S38/S42 | Peer-Erweiterungen für Hash-Walk, tolerante Listen, StageFinish/SyncFilesystem, echte TargetLimits und Watch. Frische FsAccess-Prüfung pro Ausgabe, registrierte Abbruchmarker, konservative Ready-Abdeckung. | Rechteentzug stoppt den betroffenen Principal; Verbindungsverlust allein widerruft keine Retention; unvollständige Watch-Abdeckung lässt Polling laufen; Stage-Ownership/Write-Prüfung wird durch H-DISPATCH verbunden. |

Nichtlokale Provider-Wurzeln behalten ihre Backend-Identität. Der lokale gemeinsame Finder vergleicht
alle zugelassenen lokalen Wurzeln miteinander; Duplikate zwischen unterschiedlichen fremden Backends
werden nicht stillschweigend behauptet, sondern als Grenze gemeldet.

## Entscheidungen und Integrationsverträge

- Die 6M-Reserve ist keine neue Knotenobergrenze: das bestehende Format erlaubt
  `MAX_NODES=12_000_002` und `MAX_BYTES=2 GiB`. `(MAX_NODES-2)/2` reserviert je gehaltenem
  Verzeichnis höchstens einen Aggregatknoten plus Container. Zusätzlich begrenzen tatsächlicher
  Speicher, Namen und das empfangene Budget die Retention. `fit_tree` prüft sämtliche fertigen Knoten.
- `Progress::node_budget() -> u64`, `set_node_budget(u64)`,
  `AnalysisReceiver::with_node_budget(u64)` und
  `AnalyticsBudget::for_progress(&Progress)` sind bereitgestellt. Das Budget wird in allen
  Progress-Scopes geteilt. `set_phase(Scanning)` entfernt die vorangegangene Legacy-Kennzeichnung.
- `PlatformFigures::estimated_heap_bytes(&self) -> u64` und
  `Approximations::estimated_heap_bytes(&self) -> u64` rechnen saturierend mit tatsächlicher
  Vec-/String-Kapazität. Ihr privates Modul ist additiv in `storage_view.rs` registriert.
- Fairness verwendet authentifizierten Public-Key/Node-ID über `PeerDeviceKey`, ohne Gerätealias
  oder Direct/Room-Relation als zusätzliche Quote. Retention verwendet weiterhin den vollständigen
  `PeerPrincipal`.
- `FsAccess::{policy_key,retained_snapshot,is_dynamic,register_cancel,check_read}` sind angebunden.
  Live-Rechte bleiben im eingefrorenen Auftrag erhalten; reine Transportliveness ist kein Widerruf.
  `cancel_principal` ist der zusätzliche H-DISPATCH-Eingang für betroffene Retention.
- Fortschritt kommt direkt aus dem Host-Progress, auch beim Queue-Warten. Fehler erscheinen als
  typisierte FsErr/Analysefehler; fehlende oder falsche Abschlussdaten werden nicht als Erfolg
  gewertet. STOP-Code `0x5345` ist expliziter Abbruch, andere Transportfehler erlauben einen
  Wiederanbindungsversuch. Die Sendefehler-/STOP-Race wird vor Beginn der zehnminütigen Retention geprüft.
- `host_watch.rs` ruft `watch::watch_confined(&DirectoryHandle, &Path, ...)` auf und hält
  DirectoryHandle/WatchHandle während der gesamten Sitzung. Linux/Android melden nur direkte
  Kindhinweise mit `Ready.complete=false`; `ReadyPartial` lässt Hybrid-Polling weiterlaufen.
  Das fehlende alte `complete`-Feld hat serde-Default false. Windows und fehlende Childroots
  ergeben `Unavailable`. Ein Watch-Reset/Transportende erzeugt `Ended` und erneute Prüfung;
  Cancelled/Drop beendet den eigenen Strom/Worker. Overflow und Ready gehen bei Kanalrückstau
  nicht verloren. Aktive partielle Hinweise erzeugen kein falsches `Ended`.
- `analytics::host_recycle_available()` ist die einzige OS-Maske in
  `FsHostFeatures::host.remote_trash_v1`. Windows ist false; kein freier Quarantänepfad wird
  an eine native Shelloperation übergeben. Kein Permanent-Fallback und keine globale
  Desktop-`apptrash::set_volumes`-Aktivierung.
- `server_capabilities.rs` wurde nach den TargetLimits-/Lease-Access-Anschlüssen an H-DISPATCH
  übergeben. Rechte-/Lease-Masken werden dort vom Owner abgeschlossen. `export_config.rs` blieb
  ausschließlich lesbar. Keine globalen Planänderungen durch diesen Block.
- FC1/FA3: `analytics_walk` filtert `apptrash::excluded_name`, `bisync::is_engine_name` und
  `vfs::is_staging_name` vor dem Aufnehmen und Öffnen eines Kindes; `finder_walk::skipped_name`
  ebenso vor Kandidaten/Childöffnung; `host_hash_walk` nutzt denselben frühen Filter über
  `host_list::hidden`, auch beim Provider-Walk. `.se-versions`, exakt erzeugte
  `.held.se-recycle-<16 lowerhex>` und `.se-private-<32 lowerhex>.tmp` bleiben so an jeder Tiefe
  außerhalb des Scans. Der freigegebene zentrale Stage-Parser wurde gelesen: die Quarantäne
  erfüllt `is_unique_stage`, die private Datei `is_private_stage`. Kein bloßer Ergebnisfilter
  und keine neue universelle Linux-Aliasregel. H-DISPATCH schützt direkte Windows-Aliaspfade.
  V-LOCAL bestätigt für lebende Windows-Quarantäne-/private Dateihandles ausschließlich
  FILE_SHARE_READ: fremde Delete-/Rename-Handles einschließlich Case-Rename scheitern;
  Enumeration liefert gespeicherte OS-Namen. Dies ist keine Alias-Autorisierung und kein
  Namensversprechen nach Schließen der Handles; DOS-/Hardlink-/Lookup-Aliase bleiben H-DISPATCH.

## Statische Abnahme und noch auszuführende Signale

Eigener Self-Review wurde abgeschlossen, ohne zusätzlichen Prüfer oder neues Review. Der letzte
statische Durchgang prüfte ausschließlich den eigenen verbundenen Quellbestand: balancierte
Rust-Textdelimiters, keine nachgestellten Leerzeichen, `git diff --check` ohne Befund.
Alle neuen Featuredateien liegen unter 500 Zeilen/50 KiB. Die bestehende übergroße
`share/mod.rs` erhielt ausschließlich eigene additive Registrierungen; ausführende Logik wurde
in kohäsive Featuremodule ausgelagert. `storage_view.rs` bleibt mit 491 Zeilen unter der Grenze.

Die folgenden Abnahmesignale sind als Quellfälle vorbereitet bzw. weitergeführt und **nicht lokal
ausgeführt**. Der Hauptagent bindet sie mit den Integrationsfällen aus der Tabelle in seine eine Suite:

- `review_task_analysis_deflate_reframing_and_receiver_budget`
- `review_task_receiver_budget_includes_aggregate_nodes`
- `review_task_result_figures_charge_spare_capacity`
- `review_task_admission_skips_waiting_alias_of_active_device`
- `review_task_watch_backpressure_keeps_partial_ready_and_reports_overflow`
- `review_task_share_protected_areas_keep_structured_and_legacy_forms`
- `review_task_recycle_changed_content_is_not_captured`
- `review_task_recycle_publication_failure_restores_without_replacing`
- `windows_remote_task_analysis_combines_export_roots_and_rejects_escape`
- `windows_remote_task_analysis_precancel_preserves_browsing`
- bestehende Codec-/Korruptions-/Fortschrittsfälle in `windows_analysis_transfer_task_tests.rs`

Keine Builds, Compiler, Linker, rustfmt, Tests, Server, Installationen, Commits, Pushes, Graph-Neubauten
oder Releases durch H-ANALYSIS. Der Hauptagent hält die Root-Graph-Aktualisierung und Remote-Abnahme.

## Dateien erstellt

Neue kohäsive Featuredateien dieses Blocks; genaue Registrierungen sind additiv in den jeweils
zugeordneten Modulen enthalten:

```text
native/src/analytics/core/analysis_budget.rs
native/src/analytics/core/storage_retention.rs
native/src/analytics/os/windows.rs
native/src/analytics/os/linux_os.rs
native/src/analytics/os/android.rs
native/src/analytics/os/linux_trash.rs
native/src/analytics/os/shared/analytics_walk.rs
native/src/analytics/os/shared/checked_recycle.rs
native/src/apptrash/os/shared/quarantine.rs
native/src/share/core/analysis_admission.rs
native/src/share/core/analysis_resources.rs
native/src/share/core/peer_stream.rs
native/src/share/core/peer_extensions.rs
native/src/share/core/peer_list_batch.rs
native/src/share/core/peer_duplicates.rs
native/src/share/core/peer_hash_walk.rs
native/src/share/core/peer_watch.rs
native/src/share/os/shared/analysis_spool.rs
native/src/share/os/shared/analysis_tasks.rs
native/src/share/os/shared/storage_roots.rs
native/src/share/os/shared/storage_duplicate_host.rs
native/src/share/os/shared/host_duplicate_verify.rs
native/src/share/os/shared/host_stream.rs
native/src/share/os/shared/host_list.rs
native/src/share/os/shared/host_hash_walk.rs
native/src/share/os/shared/host_mutations.rs
native/src/share/os/shared/host_watch.rs
```

## Vorhandene Dateien geändert

Bestehende K2-Teiländerungen wurden weitergeführt. `server_capabilities.rs` enthält die eigene
Vorarbeit bis zur ausdrücklich koordinierten Owner-Übergabe; weitere Änderungen daran sind H-DISPATCH.

```text
native/src/analytics/core/analysis_report.rs
native/src/analytics/core/analysis_transfer.rs
native/src/analytics/core/progress.rs
native/src/analytics/core/protected.rs
native/src/analytics/core/storage_view.rs
native/src/analytics/core/tree_deflate.rs
native/src/analytics/core/tree_transfer.rs
native/src/analytics/core/windows_analysis_transfer_task_tests.rs
native/src/analytics/mod.rs
native/src/analytics/os/mod.rs
native/src/analytics/os/shared/analytics.rs
native/src/analytics/os/shared/analytics_budget.rs
native/src/analytics/os/shared/analytics_outcome.rs
native/src/analytics/os/shared/analytics_tests.rs
native/src/analytics/os/shared/reclaim/finder.rs
native/src/analytics/os/shared/reclaim/finder_compare.rs
native/src/analytics/os/shared/reclaim/finder_walk.rs
native/src/analytics/os/shared/reclaim/mod.rs
native/src/analytics/os/shared/reclaim/stage.rs
native/src/apptrash/mod.rs
native/src/share/core/backend.rs
native/src/share/core/duplicate_wire.rs
native/src/share/core/host_requests.rs
native/src/share/core/peer_storage_analysis.rs
native/src/share/core/peer_transfer.rs
native/src/share/core/server_capabilities.rs
native/src/share/core/storage_analysis_server.rs
native/src/share/core/storage_analysis_task_tests.rs
native/src/share/core/storage_snapshot.rs
native/src/share/core/watch_wire.rs
native/src/share/core/wire.rs
native/src/share/core/wire_capabilities.rs
native/src/share/mod.rs
native/src/share/os/shared/storage_analysis_host.rs
```

Eigene Berichtdateien:
`docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-ANALYSIS.md` (fortgeführt) und
`docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-ANALYSIS.md` (erstellt).

## Gelesene Dateien

Alle oben genannten Feature-/Änderungsdateien wurden gelesen, teils symbol- oder abschnittsweise.
Zusätzliche gelesene Grundlagen, Schnittstellen und bestehende K2-Typen:

```text
AGENTS.md
docs/ARCHITEKTUR.md
docs/lesungen/INDEX.md
docs/refs/INDEX.md
docs/refs/android-storage-scan.md
docs/refs/freedesktop-trash.md
docs/refs/local-fs-identity-durability.md
docs/refs/quic-sftp-throughput.md
docs/refs/sync-remote-metadata.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/h-analysis.json
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-analyse.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sicherheit.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/K2.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/K1.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/K2.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/A-CLIENT.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/T-JOBS.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/A-CLIENT.md
graphify-out/graph.json
graphify-out/.vocab.txt
native/Cargo.toml
share-server/Cargo.toml
native/src/analytics/core/status_text.rs
native/src/analytics/core/storage_view_tests.rs
native/src/analytics/os/shared/reclaim/backend.rs
native/src/analytics/os/shared/reclaim/verify.rs
native/src/analytics/os/shared/reclaim/util.rs
native/src/app/os/windows/platform.rs
native/src/app/os/shared/platform_helpers.rs
native/src/apptrash/core/record.rs
native/src/apptrash/os/shared/store.rs
native/src/lib.rs
native/src/local_access/mod.rs
native/src/mobile/os/shared/trash.rs
native/src/share/core/blocking.rs
native/src/share/core/export_config.rs
native/src/share/core/framing.rs
native/src/share/core/fs.rs
native/src/share/core/fs_access.rs
native/src/share/core/fs_error.rs
native/src/share/core/fs_response.rs
native/src/share/core/peer_storage_snapshot.rs
native/src/share/core/peer_walk.rs
native/src/share/core/session.rs
native/src/support_dirs.rs
native/src/vfs/core/core.rs
native/src/vfs/core/extension_calls.rs
native/src/vfs/core/extension_types.rs
native/src/vfs/core/extensions.rs
native/src/vfs/core/staging_names.rs
native/src/vfs/mod.rs
native/src/bisync/mod.rs
native/src/bisync/core/types.rs
native/src/bisync/core/plan_types.rs
native/src/watch/core/types.rs
native/src/watch/mod.rs
native/src/watch/os/shared/service.rs
/root/.codex/skills/arbeitsweise/SKILL.md
/root/.codex/skills/graphify/SKILL.md
native/src/analytics/core/host_figures.rs
native/src/share/core/fs_request.rs
native/src/share/core/hash_walk_wire.rs
native/src/share/core/list_batch_wire.rs
```

Die Graph-Dateien wurden über die bereits genehmigte begrenzte Graph-Abfrage ausgewertet. Die
bekannten DirectoryHandle-Implementierungen wurden vom Owner als Vertrag übergeben; H-ANALYSIS hat
deren fremde OS-Quelldateien nicht erkundet. Verbliebene konkrete Anfragen stehen in
[anfragen/H-ANALYSIS.md](../anfragen/H-ANALYSIS.md).
