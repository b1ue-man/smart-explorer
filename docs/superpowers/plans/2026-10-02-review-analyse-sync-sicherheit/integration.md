# RV1 – offene Integrationen der Fortsetzung

Stand: 2026-10-03. Diese Liste verbindet ausschließlich die bereits dokumentierten Umsetzungsblöcke;
sie ist kein weiterer Review-Bericht. Details und Status stehen in den jeweiligen `anfragen/`-Dateien.

| Besitzer | Betroffene Grenze | Erwartetes Ergebnis |
|---|---|---|
| E-APPLY | Drive-Zielduplikate | `apply::apply_dedupe_reporting(candidates: &[vfs::DedupeCandidate], side: PairSide, planned_a: &Tree, planned_b: &Tree, endpoints: SyncEndpoints, opts: BisyncOptions, scope: &ApplyScope, errors: &mut Vec<(String,String)>, cancel: &AtomicBool) -> ApplyReport` schützt entfernte IDs über Versionen/ID-Revalidierung und meldet pro Gruppe Erfolg. Erhaltener Primärpfad → `Copied` mit unverändertem Zustand, nur ganz fehlender Pfad → `Deleted`; kein ungesichertes `Backend::apply_dedupe_plan` aus dem Orchestrator. |
| E-APPLY | Optionsbewusster Kontroll-Snapshot | `walk_snapshot_with_options` erhält `BisyncOptions`; alte `walk_snapshot`-Signatur bleibt Hülle; `cross_mounts`, `OwnFile` und `filtered` gelten auch für Kontroll-Quellwalks. |
| E-APPLY | `snapshot_pair.rs`, `snapshot_hash.rs`, `snapshot.rs`, `apply.rs` | `PairSnapshot` mit zwei `SideSnapshot`; Laufmeldungen über `apply_planned_reporting`/`ApplyScope.sink`; `Snapshot` additiv `filtered`/`dirs`; unvollständige Scans nie als vollständiger Index; `ApplySink::should_stop()` vor und zwischen Aktionen beachten. |
| E-APPLY/V-LOCAL | Basis-Zwischenstand nach Veröffentlichung | `Deferred` wird erst nach erfolgreichem `sync_filesystem(root)` dauerhaft; `PerFileOnly` bestätigt nur Dateidaten und garantiert keinen späteren Rename-Verzeichniseintrag. Dauerhaftigkeit der Veröffentlichung berücksichtigen, statt Datei-fsync als Namespace-Garantie auszugeben. |
| H-ANALYSIS → A-CLIENT | `analytics/core/progress.rs`, `analysis_transfer.rs`, IPC-Analyse | Ein gemeinsames Empfänger-Knotenbudget über `Progress::{node_budget,set_node_budget}`, `AnalysisReceiver::with_node_budget`, Share-Anfrage und Hostbudget; komprimierte Übertragung auch durch den Daemon. |
| V-LOCAL/H-ANALYSIS | Recycle nach SHA-/Identitätsprüfung | Bestätigtes reguläres Child handlegebunden in Quarantäne einfangen; anschließend no-replace in geöffneten Papierkorb bewegen, Fehler reversibel zurückstellen. Freedesktop-Speicherregeln stehen in `docs/refs/freedesktop-trash.md`; nie erneut pfadbasiert löschen. |
| V-LOCAL/H-ANALYSIS/A-CLIENT/T-JOBS | Partielle Watch-Abdeckung | `ChangeNotice::ReadyPartial { generation }`; SharedStorage/FUSE/SMB behalten Abfragen zusätzlich zu Ereignissen. Additives Share-`complete` hat konservativ `false` als Default. |
| V-LOCAL → H-ANALYSIS | konsentierter Windows-Broker | Sicherer Directory-Pin-/ShareMode-Vertrag erhält vorher erlaubte lokale Broker-Analyse; Host-Fernanalyse bleibt ordinary ohne neue Elevationsrechte. Keine unsicheren Pfad-Fallbacks und keine als abgeschlossen gemeldete A27-Regression. |
| V-LOCAL → H-ANALYSIS | `local_access` Directory-Adapter | Handlebasierter `DirectoryHandle::{open_root,open_child,read_directory,metadata,create_private_child,create_file_new}`; gewählte Wurzel einmal auflösen, Unterverzeichnisse ohne Link-Folgen und ohne Kanonisierung jedes Ordners durchsuchen. |
| H-ANALYSIS/H-DISPATCH | Gerätefairness | `PeerPrincipal::device_identity() -> PeerDeviceKey` (öffentlicher Schlüssel/Knoten, ohne Relation-/Geräte-ID-Alias) teilt Analysis-Quote über Direct/Room; Retention bleibt am vollständigen Principal. |
| H-ANALYSIS → H-DISPATCH | Legacy-StorageSnapshot | `storage_snapshot::serve_snapshot(send, root, access, principal)` erhält authentifizierten `PeerPrincipal`; fairer Host-Worker, kohärente Legacy-Aggregation vor Encode. |
| S-REVOKE → H-DISPATCH | Replay-/Signaturmarker | `seen_nonces` bei ConfigureProfiles erhalten; Nachrichten verwenden parsebare Expiry, Signaturmarker `i64::MAX`; unparsebare Sitzungseinträge nicht als bereits abgelaufen verdrängen. Gelernte Signaturflags bleiben OR-monoton. |
| H-DISPATCH | private Recycle-Quarantäne | `.held.se-recycle-<16hex>` für fremde Requests unzugänglich halten; eigene normale Transfer-Stages nicht durch pauschales Stage-Verbot brechen. |
| H-DISPATCH | `share/core/node.rs`, `session.rs` | Zertifikats-Pins auch für Relays über `NodeTransportOptions::ca_tls_config()` und `accepts_relay_url`; Signaling-only-Pinning ist kein vollständiger Fix. |
| S-REVOKE → H-DISPATCH | `share/core/node_sessions.rs::repair_direct_reciprocal` | Transition-Permit nur um den Persistenzschritt halten; vor Persistenz aktuellen Rechtezustand erneut prüfen. TLS-/Endpoint-Aufbau bleibt H-DISPATCH. |
| H-DISPATCH → S-REVOKE | `ShareIrohNode::invalidate_restrictions(&RestrictionSet) -> io::Result<usize>` | Gezielt nur betroffene Sitzungen/Leases schließen; `set_direct_online` berechnet die Rechte-Reduktionen; Wiederanschalten verschärft keine Epoche und trennt keine anderen Beziehungen. |
| T-JOBS/V-REMOTE | `connect` Locator-Grenze | `local_endpoint_path(&str) -> Result<Option<String>, String>` aus der gespeicherten `EndpointSpec`-Auswertung, ohne Netzwerköffnung; lokale Literale, UNC und Verbindungsidentität bewahren. |
| Hauptagent/T-JOBS | Main-/CLI-Start | `daemon::run_guardian` über den dokumentierten neuen Wächter-Einstieg anbinden. |
| T-JOBS/H-DISPATCH | Daemonstart/Serverkonfiguration | Bestehende schema-lose Serveradressen dauerhaft über `migrate_server_file` in explizite Klartextadressen umschreiben, alte Bedeutung bewahren. |
| S-POLICY | Profilvalidierung entfernter Direkt-Geräte | Sicherheits-Denials niemals wegen einer alten 64-Einträge-Grenze verdrängen; reale Dateigrößen-/Persistenzgrenzen müssen fehlschlagen, statt alte Schlüssel wieder zuzulassen. |
| S-POLICY | `ShareProfiles::withdraw_direct_key(&DirectPeerIdentity, now)` | Direkt-Entzug aus `profiles.rs`/`profile_edits` auf Schlüssel/Knoten anwenden; Geräte-ID-Aliase dürfen keine aktiven Grants/Exec-Rechte behalten. |
| S-REVOKE/S-POLICY → AND-SHARE-UI | bewusstes Wiederzulassen | `allow_direct_peer_again` für `Ignored`/`Reconfirm` explizit aufrufen; `share_back` separat wählen, Exec bleibt aus. |
| S-REVOKE | Desktop-Widerruf unbestätigter Kopplung | `unconfirmed` erst nach erfolgreicher persistierter Kontakt-/Raumentfernung entfernen; Fehler bleiben wiederholbar. `remove_direct_peer_completely`/`remove_room_completely` liefern nun `bool`; Tail-Matches müssen ihre Rückgabe bewusst behandeln. |
| SUITE | Legacy-/Klartext-Fixtures | Neue CLI-Eingaben für Loopback-Testserver verwenden `tcp://`/`ws://` plus `--allow-plaintext`; alte veröffentlichte CLI behält ihre eigene alte Syntax. Lifecycle Linux/Windows und Mixed-Version-Entrypoints aktualisieren, ohne doppelte Builds/Suites. |
| AND-SHARE-UI | zentrale Android-API-Dokumentation | Alle Deltas der Implementierungsblöcke in die bestehende `api.md` übernehmen, einschließlich Fern-Papierkorb und Host-Plattformzahlen. |

Integrationsanfragen gelten erst nach Implementierung und statischem Abgleich als erledigt. Die
gemeinsame Remote-Task-Suite bestätigt am Ende die direkt betroffenen Abläufe zusammen.

## Abschlussübergaben 2026-10-03

- S-SIGNAL `af77ead`, A-CLIENT `594ed68`: kohärente Implementierung committed; gemeinsame Registrierungen folgen zentral, keine lokalen Builds/Tests.
- Watch-Handoff: T-JOBS liefert `watch_confined(&DirectoryHandle, literal_root: &Path, WatchOptions, WatchFilter, WatchSink) -> io::Result<WatchHandle>`. H-ANALYSIS verbindet ausschließlich `share/os/shared/host_watch.rs::setup`. Fehlende Childroots bleiben `Unavailable`, bis eine sichere Handlebindung möglich ist.
- H-DISPATCH bindet authentifizierte Exec-Principals unmittelbar nach erfolgreichem `authorize_client_hello` in `share/core/exec_server.rs` an die konkrete Verbindung; Job-/Heartbeat-/Registry-Verhalten bleibt erhalten.
- `FsAccess::Authorized` verbindet aktuelle Sitzungsrechte mit den bisherigen Dynamic/Mounted-Grenzen. H-ANALYSIS übernimmt die eigenen Policy-/Retentionsmatches nach exakter API-Übergabe; ein bloßer Disconnect verwirft behaltene FA2-Ergebnisse nicht.

## Geklärte Restgrenzen 2026-10-03

- H-ANALYSIS `5098ee0`, T-JOBS `7806d24`, V-LOCAL `cd8632d` sind abgeschlossene Quellenblöcke; Consumer- und Laufabnahme stehen aus.
- Exec-Recoveryjournal `4f4d75b` schreibt über `support_dirs::write_private_atomic`; die Stage ist schon beim Erstellen privat, bevor Grantdaten hinein gelangen. Keine NamedTempFile-Rechtemigration nach dem Schreiben.
- Windows-Namespacebestätigung braucht keinen neuen V1-Durability-Haken: `windows_profile` ist `FlushModel::PerFile`, Stage-Flush und Write-Through-Publish sind vorhanden; `sync_filesystem` bestätigt diese eigene Semantik. Andere tatsächliche `PerFileOnly`-Backends liefern weiterhin false.
- Y124: additiver `BackendExtensions::sync_child_path(parent,literal_name)` und `vfs::sync_path(backend,root,literal_rel)` kodieren ausschließlich neue Literalnamen. Bestehende kodierte Locators behalten ihre Bedeutung, einschließlich `%61ux.c`. V-REMOTE liefert den Drive-Override; Engine und Guardwrapper müssen den Hook konsumieren.
- Y132/Y134: stabiler Drive-Accountkey mit einmaligem konservativem Alt-ID-Anschluss; Accountfeed wird nach Root/ID/Ancestry eingeordnet und unklare Änderungen lösen Kontrolle aus. Keine Vollständigkeit aus einem ungefilterten Accountfeed ableiten.
- Y140/Y142: additiver `replace_staged_reversible(stage,destination,retained)` erhält das Original am exakt vorab journalisierten `.se-replace-16lowerhex`-Sibling. Unsupported bleibt mutationsfrei; Provider löschen die alte Datei niemals und versprechen keine atomare Ersetzung. Engine besitzt Intent/Recovery/Backup und bestätigt Veröffentlichung getrennt.
- Allfilesverlust: AND-SYNC ergänzt dieselbe kurze Weak-Cancel-Registrierung für aktive manuelle und Hintergrundläufe; `platform::requires_storage_access` schützt nur lokale Shared-Storage-Endpunkte, Fern/Fern und App-privat bleiben erhalten.

Diese Ergänzungen schließen die vorhandenen Befunde; sie eröffnen keine neue Review-Runde.

## Abgeschlossene Anschlussquellen 2026-10-03

- H-TRASH-WINDOWS `708b6f1`: vollständiger sichtbarer Smart-Explorer-Host-Papierkorb und NoReplace-Restore; Windows-FA6-Unsupported aus dem früheren H-ANALYSIS-Handoff ist für diesen Anschluss superseded. Gemeinsame Remote-Abnahme bleibt offen.
- S-POLICY `d3cced7` und Native-Android-Facaden `f7d6af7`/`58aa0e7`: explizite Rechte, voll gepinnte Raum-/Grant-Zulassung, incoming-only Widerruf und Ask/AutoAccept verwenden die vorhandenen persistierten Autorisierungsgrenzen.
- Parent `787e893` bindet den tatsächlichen PrivateAncestor-Handle-Guard an storage_roots und Hash-Walk; `461fc73`/`a8b90ae`/`3116d16` forwarden Literal-/Recovery-/bewiesene Legacy-ID-Haken durch die vorhandenen Hüllen.
- Parent `d546712` hält automatische Duplicate-Repairs im laufenden ApplyScope statt einer zweiten unabhängigen Default-Applytransaktion.
- S-LOCAL-A2: `bin/se.rs` ruft den vorhandenen Uplink-Helper vor normaler CLI-Verarbeitung auf; Linux `net::run_uplink_helper_if_requested` erkennt exakt `--lan-uplink-cleanup`. Es existiert kein Linux-Uninstall-Script; `install-linux.sh` installiert ausschließlich. Canonical README/RELEASING dokumentieren diesen vorhandenen Disable-/Cleanup-Weg vor manueller Binärentfernung als ursprünglicher App-Benutzer (`7cb1f2ef`). Kein neuer Installer-Uninstall-Modus wird erfunden.
- S-LOCAL-A4 ist durch `4f4d75b` private Atomic-Exec-Journal-Erzeugung erledigt. S09-LINK bleibt vor Abschluss des gepinnten realen Session-/Interface-Kanals offen; signierte Beacons allein erlauben keinen privilegierten Start.

Diese Liste ist Quellenintegration, keine Behauptung einer Ausführungs-/Releaseabnahme. Alle erwarteten Ergebnisse gehen erst nach dem vollständigen Batch gemeinsam in die eine Remote-Task-Suite.

- Parent `810b0df9`: Recovery-Sibling heißt exakt `.se-replace-<16lowerhex>` ohne User-Dateibasename und ist im Stagefilter eine geschützte eigene Datei. Der frische Uplink-Cache-Getter macht keinen OS-/Netzwerkaufruf im Tick; fehlende Fakten bleiben unbekannt.
- Parent `a7014641`/`fc687a56`: `expectedAccess` und `expectedShared` werden im selben Share-Profil-CAS geprüft; eine veraltete Rechtebestätigung darf keinen zwischenzeitlichen Entzug überschreiben.
- E-APPLY → Desktop/Android: `recorded_original_paths_for_key` liefert die unter StateKey autorisierten tatsächlichen Seitenschreibweisen über `vfs::sync_path`. Consumer erzeugen keine eigenen Vergleichsschlüssel-/Locator-Encoder. Teilmerge-Wiederanlauf verwendet weiter die ursprünglichen Bytes; der Persistenzanschluss nach Neustart gehört zum selben E-APPLY-Ergebnis.
