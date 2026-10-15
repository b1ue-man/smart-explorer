# Ordner schnell löschen auf Direct Shares und Remotes (2026-10-14)

Stand: 2026-10-14, aus der Nutzernachricht vom 2026-10-14 („löschen von ordnern dauert zu lange …“).

## A — Auftrag / Erstplan

| Nr. | Ergebnis | Quelle |
|---|---|---|
| R1 | Ein Ordner auf einem Direct Share (jede Art: Direkt-Freigabe, Raum mit Schreibrecht, vom Host weitergegebene Verbindung) wird mit **einem** Auftrag gelöscht, den die Gegenseite lokal ausführt, statt mit einem Roundtrip je Eintrag | Nutzer: „entweder ordner direkt oder via reiner lokaler ausführung, also dass nicht jeder löschauftrag bei remotes n roundtrip braucht“ |
| R2 | Weiteres Optimierungspotenzial beim Ordnerlöschen auf Remotes nutzen, so schnell und effizient wie möglich | Nutzer: „schau einfach nach optimierungspotential da und mach es so schnell und effizient wie möglich“ |
| R3 | Remote-Task-Suite und Release nach AGENTS.md | Projektregel |

Grundfunktion: ein ganzer Ordnerbaum verschwindet mit möglichst wenigen Netzlaufzeiten. Zwei Eigenschaften bleiben hart: Links werden als Eintrag entfernt und nie verfolgt, und jede Rechte- und Schutzprüfung des Hosts bleibt erhalten.

Erhalten bleiben die Bedienwege Entf/Umschalt+Entf (Desktop), `se rm -r`, MCP `remove` und Android-Löschen mit ihren Dialogen und der Fortschrittsanzeige. Neu ist nur der Weg darunter.

## B — Bestand (Lesestand `ad99f3a4`)

- **Generischer Löschlauf** `native/src/vfs/core/delete.rs::remove_entry_controlled`:
  - Erst wird geplant: `stat` je Eintrag und `list_dir` je Ordner.
  - Dann wird angewendet, streng nacheinander: frisches `stat` je Eintrag, dann `remove_dir` bzw. `remove_file_id`.
  - Grenzen: 1 000 000 Einträge, 128 MiB Pfadtext, Tiefe 512.
  - Desktop (`app/core/delete_actions.rs:282`), Android (`mobile/os/shared/delete.rs:152`), `se rm -r` und MCP `remove` (`cli/tree_ops.rs:183`) nutzen ihn alle.
- **Kette der Desktop-GUI zum Direct Share**: `CachingBackend` → `AgentBackend` (IPC zum Hintergrunddienst) → Daemon → `PeerBackend` → je Anfrage ein QUIC-Stream → Host.
  - Je Datei kostet das etwa 3 Peer-Roundtrips: `stat` beim Anwenden, das `stat` des Daemons in `remove_one_backend` und `RemoveFile`.
  - Dazu kommen 2 IPC-Sprünge, alles sequentiell.
- **Host**: `FsRequest::RemoveDir` löscht schon rekursiv und lokal (`share/core/server_fs.rs:287` → `fs_delete::remove_tree_checked`).
  - Vorher prüft `require_target_destructive` den ganzen Baum auf Schutz (`fs_host_destructive.rs`).
  - Gelöscht wird handlegebunden, Links sind Blätter (`fs_delete_local.rs`). Vor jedem Eintrag prüft `check_write` das Schreibrecht live.
  - Lücke: `RemoveDir` hat 60 s Antwortfrist und keinen Fortschritt (`peer_request.rs`, `io_deadline.rs:7`). Der Client schickt es erst, nachdem er alle Kinder selbst gelöscht hat.
- **Streaming-Muster** für lange Host-Arbeit mit Herzschlag und Abbruch über Stream-Stopp:
  - Host: `share/os/shared/host_stream.rs`.
  - Client: `share/core/peer_stream.rs::call`.
  - Freigabe von Fähigkeiten über `FsHostFeatures` (`wire_capabilities.rs`). Ein alter Host bekommt nie eine Anfrage, die er nicht anbietet.
- **Agent-Protokoll**:
  - `Frame::Remove { recursive }` gibt es. Daemon (`backend_delete.rs`) und SSH-Agent (`std::fs::remove_dir_all`) können es rekursiv, kein Client sendet `true`.
  - `Query`-Arten werden auf unbekannte Art mit 0 beantwortet (`ext_ops.rs::answer`, `daemon/os/shared/backend_ops.rs::query`).
- **Protokoll-Backends**:
  - SFTP, FTP und SMB löschen Ordner nur leer.
  - WebDAV-`DELETE` auf eine Collection wirkt nach RFC 4918 §9.6.1 mit Depth infinity und meldet Teilfehler mit 207. Abruf 2026-10-14: https://www.rfc-editor.org/rfc/rfc4918#section-9.6.1.
  - Drive löscht Ordner schon mit einem PATCH in den Papierkorb.
  - SFTP-Anfragen tragen IDs und laufen nebenläufig auf einer Sitzung (russh-sftp 2.3.0 `client/rawsession.rs`: `requests`-Tabelle, `use_next_id`).
  - SMB-Verbindungsklone teilen eine Sitzung und überlappen unter Credits (smb2 0.26.0 `client/connection.rs` `Connection: Clone`, `tree.rs` zu `delete_files`).
  - FTP: ein Befehl je Steuerverbindung.
- **Rust-Dokumentation zu `std::fs::remove_dir_all`** (doc.rust-lang.org, Rust 1.99.0, abgerufen 2026-10-14): „does not follow symbolic links“. Unter Linux ist die Funktion gegen Symlink-TOCTOU geschützt.
- **Projektregeln**:
  - Rekursives Löschen folgt nie Links aus der Wurzel (AGENTS.md).
  - Rekursion iterativ und budgetiert (RV1 H6).
  - Räume sind nur lesend, Direct Share standardmäßig lesen und schreiben.
  - Das se-agent-Paket baut erst der Release-Wrapper neu (`native/build-agent-bundles.sh`, `publish-release-local.ps1:601`).

## C — Entscheidungen

1. **Neue Backend-Erweiterung `remove_tree`** in `vfs::BackendExtensions` mit `supports_remove_tree`.
   - Fortschritt läuft als `RecursiveDeleteProgress`; Abbruch erzeugt den Bericht `Canceled`, ein Fehler den Teilstand als `RecursiveDeleteFailure`.
   - `remove_entry_controlled` nimmt diesen Weg für einen Ordner, sobald das Backend ihn anbietet. Sonst gilt unverändert der bisherige Lauf. Alle Bedienwege profitieren ohne eigene Änderung.
   - Verworfen: eine neue Methode im `Backend`-Trait, weil `vfs/core/core.rs` 496 Zeilen hat und der Extension-Weg Wrapper und Fallbacks schon regelt.
   - Verworfen: die Aufrufer einzeln umbauen, weil das vier Bedienwege doppelt umbaut.
2. **Share: neue Stream-Anfrage `RemoveTree`** mit Feature `remove_tree_v1`.
   - Der Host führt die vorhandene Schutzprüfung aus, zählt dabei die Einträge und löscht mit dem vorhandenen handlegebundenen Lauf.
   - Er meldet alle 250 ms Fortschritt und bricht beim Stream-Stopp oder Rechteentzug am nächsten Eintrag ab.
   - Verworfen: `RemoveDir` direkt senden, weil 60 s Frist, kein Fortschritt und kein sauberer Abbruch.
   - Verworfen: Share-Exec, weil es aus ist, eigene Rechte braucht und Export-, Raum- und Schutzgrenzen umgeht.
3. **Agent-Protokoll: Query `REMOVE_TREE` und Frame `RemoveTree`**, Antworten `Progress { done, total }`* dann `End`.
   - Der Hintergrunddienst bedient den Frame über `vfs::remove_tree` seines Backends, beim Share also über den Weg aus 2.
   - Der SSH-Agent löscht lokal: Kindordner mit `std::fs::remove_dir_all`, Dateien und Links mit `remove_file`, dabei Fortschritt je Kind und Abbruch zwischen Kindern. Die Landlock-Wurzel bleibt die Grenze.
   - Ein alter Dienst oder Agent antwortet 0. Dann folgt der Weg aus 4 bzw. der bisherige Lauf.
4. **Gelisteter Baumabbau** `vfs::remove_tree_listed` für Backends mit typstrengen Primitiven.
   - Typstreng heißt: `remove_file` entfernt nie einen Ordner, `remove_dir` nur leere Ordner und folgt keinem Link.
   - Einmal `list_dir` je Ordner, kein `stat` je Datei. Blätter gehen gebündelt an einen Backend-Entferner, der pipelinen darf. Ordner werden nach ihren Kindern entfernt.
   - SFTP: bis zu 64 offene Anfragen auf der Hauptsitzung, wie die vorhandene Lesetiefe. SMB: nebenläufig über Verbindungsklone unter Credits. FTP: sequentiell, aber ohne `stat` je Eintrag.
   - Verworfen: SSH-Exec `rm -rf`, weil keine Wurzelbindung, Exec vom Server abhängig und 30-s-Frist.
5. **WebDAV: ein `DELETE` auf die Collection** nach `stat`, das eine Collection und keinen Link verlangt. 207 bleibt ein Fehler.
6. **Drive** bleibt unverändert, weil der Papierkorb einen Ordner schon mit einem Aufruf entfernt.

## Sicherheit und zu erhaltendes Verhalten

- Links werden als Eintrag entfernt, nie verfolgt. Dafür sorgen der Host über Handles mit `O_NOFOLLOW`/`OPEN_REPARSE_POINT`, der SSH-Agent über `remove_dir_all` und der gelistete Lauf, der nur Einträge mit `is_dir && !is_symlink` betritt.
- Der Host löscht nie die Export-Wurzel. Private Namen, Systemdateien, nur lesende Exporte und Räume ohne Schreibrecht werden abgewiesen. Vor jedem Eintrag wird das Schreibrecht live geprüft, ein Rechteentzug stoppt den Lauf.
- Abbruch: Schon gelöschte Einträge bleiben gelöscht, der Bericht nennt den erreichten Zähler. Der erste Fehler bricht ab und meldet den Teilstand.
- Alte Hosts, Dienste und Agenten ohne die neue Fähigkeit bekommen den bisherigen Ablauf. `RemoveDir` bleibt für alte Clients unverändert.
- Unverändert bleiben: die Papierkorb-Route (Drive), das Abweisen von `Unsupported`, das Aufräumen der Quelle nach `se mv`, Bisync, Laufwerke und der Schutz `--no-preserve-root` von `se rm`.

## D — Arbeitsplan

| Meilenstein | Inhalt | Dateien | Erwartetes Ergebnis |
|---|---|---|---|
| M1 | VFS-Erweiterung `remove_tree`, Schnellweg in `remove_entry_controlled`, Weitergabe im Cache, gelisteter Baumabbau | `vfs/core/{extensions,extension_calls,delete,tree_remove_listed,cache_extensions}.rs`, `vfs/mod.rs` | Ein Backend mit `remove_tree` bekommt für einen Ordner genau einen Aufruf und kein `stat`/`list_dir`; ohne Fähigkeit läuft der bisherige Lauf |
| M2 | Share `RemoveTree` mit Feature, Host-Stream, PeerBackend | `share/core/{fs_request,fs_response,wire_capabilities,server_fs,host_requests,peer_extensions,peer_remove_tree,fs_delete,fs_delete_local}.rs`, `share/os/shared/{host_remove_tree,fs_host_destructive}.rs` | `se rm -r` eines Peer-Ordners: ein Share-Stream; Link-Ziel außerhalb bleibt; Raum ohne Schreibrecht wird abgewiesen |
| M3 | Agent-Protokoll `REMOVE_TREE`/`RemoveTree`, Daemon, se-agent, AgentBackend | `agent_proto/core/{types,frame_ext,ops_types,server}.rs`, `agent_proto/os/shared/{ext_ops,tree_remove}.rs`, `daemon/os/shared/{backend_server,backend_ops}.rs`, `agent/core/{extensions,tree_remove}.rs` | Desktop/CLI über den Dienst: ein Frame statt eines Laufs je Eintrag; ein alter Server antwortet 0 und es folgt der Fallback |
| M4 | SFTP, SMB, FTP gelistet (SFTP/SMB pipelined), WebDAV ein DELETE | `sftp/core/{extensions,tree_remove}.rs`, `smb/core/…`, `ftp/core/…`, `webdav/core/…` | SFTP-Ordner mit N Dateien: ein `list_dir` je Ordner, Löschen mit bis zu 64 offenen Anfragen; Link-Ziel bleibt |
| M5 | Suite | `native/test-delete-speed-task.py`, `.github/workflows/delete-speed-task.yml`, Testmodule mit Präfix `delete_speed_task_` | Ein Remote-Lauf prüft M1–M4 auf Linux und Windows |
| M6 | Doku | `docs/ARCHITEKTUR.md`, `docs/FILE_OPS_MATRIX.md`, `docs/TODO.md`, dieser Plan | Bestand und Grenzen beschrieben |
| M7 | Release | `build.yml` `complete_release_source_sha` | v0.5.179 mit allen Assets |

## Befunde der Umsetzung (2026-10-14)

- **Share-Host.** Ein Link im zu löschenden Ordner, dessen Ziel außerhalb der Freigabe liegt, wird von der vorhandenen Schutzprüfung abgewiesen (`share/os/shared/fs_host_destructive.rs`, `policy.write` je Eintrag). Der bisherige Eintragslauf wies ihn genauso ab, das Verhalten bleibt also gleich. Ein Link auf ein Ziel innerhalb der Freigabe wird als Link entfernt.
- **Export-Wurzeln.** `RemoveTree` weist alle Export-Wurzeln ab (`fs::is_export_root`: `/`, `/<Export>`, `/Verbindungen`, `/Verbindungen/<Name>`). Auch die Wurzel einer weitergegebenen Verbindung ist damit geschützt.
- **Weitergegebene Verbindungen.** Der Host löscht weiter Eintrag für Eintrag gegen die Gegenstelle der Verbindung, denn `GuardedBackend` reicht `remove_tree` nicht durch, damit Pfad- und Privatnamenprüfungen je Eintrag bleiben. Der Client braucht trotzdem nur eine Anfrage.
- **Agent-Protokoll.** `PROTO_VERSION` bleibt 11. Die neue Fähigkeit wird über die Query ausgehandelt, und eine neue GUI kann so mit einem noch laufenden alten Dienst weiterarbeiten. Der SSH-Agent zählt die direkten Kinder des Ordners und den Ordner selbst.
- **FTP** bleibt sequentiell. Weitere Steuerverbindungen kosten je eine Anmeldung und enden am Verbindungslimit des Servers in Congestion (`ftp/core/pool.rs`). Ein Abbruch greift vor dem nächsten `DELE`.
- **WebDAV.** Geht die Antwort des `DELETE` verloren, etwa weil ein großer Ordner das 60-s-Inaktivitätslimit überschreitet, entscheidet ein `PROPFIND`: Ist die Collection weg, war das Löschen erfolgreich.
- **Grenzen.**
  - SFTP: 64 offene `REMOVE` (`SSH_WINDOW`/256 KiB, wie die Download-Tiefe).
  - SMB: so viele Löschketten, wie das Credit-Fenster trägt (`credit_capacity_for`, höchstens 170 bei 512 Credits).

## Fortschritt

- [x] M1 (Code und Tests `vfs/core/delete_speed_task_tests.rs`, Compile-Beleg folgt mit der Suite)
- [x] M2 (Code und Tests, Compile-Beleg folgt mit der Suite)
- [x] M3 (Code und Tests, Compile-Beleg folgt mit der Suite)
- [x] M4 (Code und Tests, Compile-Beleg folgt mit der Suite)
- [x] M5 Suite geschrieben; Lauf 38619376635 scheiterte unter Linux und Windows, Korrekturen `ab41cd96` (Linux) und `2ce10314` (Windows), Graph `63733be5`; Wiederholung 38623831614 grün unter Linux und Windows
- [x] M6
- [x] M7 v0.5.179 (Lauf 38626320197)
