# Übertragungs-Engine

Stand: 2026-10-21 (Lauf 3 des Doku-Laufs U42, E13-Doku). Grundlage: der Batch „Übertragungs-Engine“ (TE1, 2026-09-29, Plan `docs/superpowers/plans/2026-09-28-transfer-engine/`) und die E13-Pakete U45 bis U53 (Festlegung `docs/plaene/2026-10-17-rechte-desktop-tresore/festlegungen.md`, Abschnitt U44). Beschreibt, wie Kopieren, Einfügen, Ziehen, „Kopieren/Herunterladen nach…“ und Downloads zwischen lokalen Ordnern, Remotes (SFTP, FTP/FTPS, WebDAV, SMB, Google Drive, ZIP), Direct-/Raum-Shares und dem SSH-Agent laufen, welche Grenzen jedes Protokoll setzt und woher jede Zahl kommt. Ergänzt um die SFTP-Zusatzverbindungen (Sicherheitsantwort E13-S15, `docs/plaene/2026-10-17-rechte-desktop-tresore/stand/SEC.md`, Punkt 15), den Hinweis `note` und die Messstufe `speed` (Abschnitt „Nachweis“). Nicht beschrieben: die Anzeige auf Android (U54); Android zeigt den Hinweis `note` derzeit nicht (Abschnitt „Wo es erscheint“). Die Messläufe der Stufe `speed` sind noch nicht ausgeführt.

## Was der Nutzer sieht

- **Kopieren merkt sich nur die Auswahl.** Strg+C lädt nichts herunter und scannt nichts;
  Einfügen startet sofort. Unter Linux gibt es dieselbe Zwischenablage innerhalb der App.
- **Einfügen startet sofort.** Unterordner werden parallel zum Kopieren gesucht; die ersten
  Dateien kommen an, während der Rest noch gelistet wird. Summen wachsen, bis die Suche fertig
  ist („sucht… N gefunden“); die Restzeit erscheint erst danach.
- **Keine feste Grenze gleichzeitiger Übertragungen.** Jede Übertragung startet sofort;
  Übertragungen auf derselben Verbindung wechseln sich ab, andere Verbindungen konkurrieren um
  nichts. Ist eine Verbindung ausgelastet, steht „wartet auf <Verbindung>“ in der Zeile.
- **Übertragungsfenster** („⇅ Übertragungen (n)“ in der Statuszeile): Richtung, Quelle → Ziel,
  Zustand, Dateien/Bytes, Rate über die letzten Sekunden, Restzeit, laufende Dateien,
  Parallelität, Hinweise der Verbindung (z. B. Drives Schreibrate), Fehlerliste mit „Alle Fehler
  kopieren“ und „Protokoll öffnen“ (JSON-Zeilen je Fehler), „Fehlende übertragen“, Abbrechen,
  Zielordner öffnen, Entfernen.
- **Nie ersetzen:** belegte Namen werden „Name (2)“; lokale Ordner werden zusammengeführt,
  Remote-Ordner bekommen bei Konflikt einen neuen Namen. Nur „Kopieren/Verschieben nach…“
  (lokal) kennt „Überschreiben“ als ausdrückliche Wahl. Ausnahme seit dem Quellstand
  2026-10-31 (unveröffentlicht): die Dateiregel „Neuere ersetzen ältere“ (Abschnitt
  „Dateiregel“) ersetzt eine ältere Datei durch eine neuere, wenn die Person Ordner
  zusammenführt oder die Regel ausdrücklich wählt.
- **Ordner in sich selbst** (oder einen seiner Unterordner) wird für jede Kombination abgelehnt.
- **Windows-Explorer:** Remote-Auswahlen gehen als virtuelle Dateien auf die Zwischenablage
  bzw. per Ziehen in den Explorer; der Explorer holt die Liste erst beim Einfügen und die
  Inhalte bei Bedarf. „Für andere Programme bereitstellen“ lädt eine Remote-Auswahl mit
  Fortschritt herunter und legt danach echte Dateien (CF_HDROP) auf die Zwischenablage.

## Aufbau

```
App/Android ── TransferRequest::Job(TransferJob) ──► Lane (ein Worker je Übertragung)
                                                        │
                                                        ▼
                                      engine::run_job / run_view
   ┌───────────────┬──────────────────┬───────────────────┬──────────────────────┐
   Walker          Warteschlange       Worker (auf Bedarf)  Ordner-Register         Fortschritt
   (paralleles     (begrenzt nach      je Datei: Stufe,     (jeder Zielordner       (~150 ms,
   Listen, Ordner  Pfadtext, nicht     Stream/Serverkopie/  genau einmal, Eltern    Rate über 3 s,
   vor Inhalt)     nach Anzahl)        Paket, Veröffentl.)  zuerst)                 Fehlerprotokoll)
          └──────────── Flow je Verbindung (Erlaubnisse, adaptive Grenze) ───────────┘
```

- `native/crates/transfer/src/transfer/core/job.rs`: `TransferJob` (Endpunkte, Auswahl als Wurzeln oder Paare mit
  Größe/Zeit/ID, Layout, Filter, Konfliktpolitik, Modus, `resume`), `validate()`.
- `native/crates/transfer/src/transfer/os/shared/engine/`: Ablauf, Worker, Operationen je Richtung
  (`upload.rs`, `download.rs`, `remote_copy.rs`, `local.rs`, `batch*.rs`), Veröffentlichung
  (`publish.rs`), Ordner-Register (`folders.rs`), Fehlerprotokoll (`issues.rs`).
- Warteschlange: Dateien vor Ordnern. Ein Ordner mit Dateien entsteht mit seiner ersten Datei (Ordner-Register), eigene Ordner-Arbeit brauchen nur leere Ordner und das Aufräumen beim Verschieben; so halten die Ordner einer Auflistung die ersten Dateien nicht auf.
- `native/crates/transfer/src/transfer/os/shared/walk*.rs`: Walker. Ausgewählte Einträge werden parallel per
  `stat` geprüft; liegen in einem Ordner mehr ausgewählte Einträge, als gleichzeitig laufen
  dürfen, beantwortet eine Auflistung des Ordners alle. Links, Spezialdateien und ungültige
  Namen sind gemeldete Auslassungen, der App-Papierkorb eine stille Auslassung.
- `native/crates/transfer/src/transfer/os/shared/flow.rs` + `core/flow_control.rs`: Flows (siehe unten).
- `native/crates/transfer/src/transfer/os/shared/{access,memory}.rs`: eine Rechteanfrage je Übertragung für
  geschützte lokale Ordner; prozessweites Speicherbudget für gepufferte Bytes.
- `native/crates/transfer/src/transfer/os/shared/lane.rs`: Lane ohne feste Grenze; fertige Übertragungen liefern
  Job, erste Fehler und aufgelöste Zielwurzeln für „Fehlende übertragen“.

## Parallelität: Fenster, Startwert und Speicherbudget

Die richtige Zahl gleichzeitiger Operationen hängt von Bandbreite × Latenz, Dateigrößen,
Serverleistung und Speichermedium ab – nichts davon ist vorher bekannt. Jede Verbindung (bzw.
jedes lokale Volume) hat deshalb **einen** Flow, den alle Übertragungen, Explorer-Übergaben und
Sync-Läufe dieses Prozesses teilen; er misst die Nutzleistung (Bytes plus 64 KiB je fertiger
Operation, `OP_WEIGHT_BYTES`, damit reine Kleinstdatei-Last messbar ist) und regelt die Grenze
(`native/crates/transfer/src/transfer/core/flow_control.rs`):

| Parameter | Wert | Grund |
|---|---|---|
| Startgrenze | 2 (`START_LIMIT`); gelernt: die halbe zuletzt eingestellte Grenze, mindestens 2 (`start_from_learned`, `native/crates/transfer/src/transfer/core/flow_window.rs:19-21`). Mindeststart je Art (`start_floor`, `native/crates/transfer/src/transfer/os/shared/flow_registry.rs`, Stand 2026-10-29): Share 8; SFTP, SMB, WebDAV 8; FTP und Google Drive 4; lokales Volume, das nicht dreht oder dessen Art unbekannt ist, 2 × Kerne innerhalb 8–16; ein als drehend gemeldetes Laufwerk (Linux `queue/rotational`, Windows Seek-Penalty-Abfrage) startet bei 8 (`ROTATING_START`, robocopy-Standard `/MT:8`; virtuelle Platten melden Drehung oft fälschlich; Stand 2026-10-29), Tresore behalten 2. Die Protokollgrenze (`transfer_ceiling`) begrenzt jeden Start | eine Operation in Arbeit, eine im Anlauf; ein bekannter Server startet nicht bei null; Flash und Server mit vielen Strömen erreichen den stetigen Zustand ohne Hochlauf |
| Fenster | Abschluss, wenn mindestens 100 ms (`MIN_WINDOW_MS`) und 8 Operationen (`MIN_COMPLETIONS`) vergangen sind; ohne Messung 1 s (`FALLBACK_WINDOW_MS`), höchstens 5 s (`MAX_WINDOW_MS`) | kurz genug zum Reagieren, lang genug gegen Rauschen |
| Slow Start | verdoppeln, solange jede Messung um ≥ 10 % zulegt (`SLOW_START_GAIN` = 1,10; `flow_control.rs:7-8`, `:317-320`) | findet große Grenzen in wenigen Fenstern |
| Proben | Schritt Grenze/8 (`PROBE_STEP_DIVISOR`), über 2 Fenster (`PROBE_WINDOWS`), alle 4 Fenster im Leerlauf (`PROBE_INTERVAL_WINDOWS`) | symmetrische Schritte halten die Grenze unter Rauschen am Optimum (Simulation mit ±10 %: ≥ 93 % der Bestleistung bei ≤ 1,11-facher Parallelität) |
| Behalten | aufwärts ≥ +5 % (`PROBE_GAIN`), abwärts ≥ 97 % (`PROBE_KEEP_LOWER`) | nur Änderungen, die sich lohnen |
| Überlast | Grenze halbieren; danach halten 3 Fenster die Grenze (`OVERLOAD_HOLD_WINDOWS`, `flow_control.rs:267`) | Ratenlimits, „busy“, Timeouts (typisiert als `vfs::congestion_error`) |
| Obergrenze | 256 (`RESOURCE_CEILING`) und die Protokollgrenze (`transfer_ceiling`) | Threads und Puffer je Operation; echte Grenzen kommen vom Protokoll |
| Merken | 15 min nach letzter Nutzung (`FLOW_RETENTION`, `native/crates/transfer/src/transfer/os/shared/flow_registry.rs:12`) | das nächste Einfügen beginnt, wo das letzte endete |

**Startwert aus `transfer-hints.json`.** Der Flow lernt die Grenze, bei der eine Verbindung
oder ein Volume sich eingependelt hat, und schreibt sie in `<app data>/transfer-hints.json`
(`native/crates/transfer/src/transfer/os/shared/hints.rs`, Datei `transfer-hints.json`, `:23`). Ein Eintrag ist
nur ein Startwert; die erste Messung ersetzt ihn. Die Datei enthält keinen Rechnernamen, kein
Konto, keinen Pfad und keinen Namen: Schlüssel ist der SHA-256 eines zufälligen 32-Byte-Salzes
dieser Installation mit der Identität der Verbindung (`hints.rs:1-12`). Höchstens 512 Einträge
(`MAX_ENTRIES`), die Datei wird höchstens alle 60 s geschrieben (`WRITE_INTERVAL_MS`) und hat
höchstens 256 KiB (`MAX_FILE_BYTES`). Ungültige Einträge, Grenzen außerhalb von 1 bis zur
Obergrenze und Zeiten in der Zukunft verwirft das Lesen.

**Dynamisches Speicherbudget.** Gepufferte Bytes aller Übertragungen eines Prozesses
teilen sich ein Budget. Es ist ein Viertel des verfügbaren Speichers, begrenzt auf 64 MiB bis
2 GiB (`MIN_BUDGET`, `MAX_BUDGET`); der verfügbare Speicher wird höchstens alle 2 s neu gelesen
(`REFRESH_INTERVAL`). Ist er unbekannt, gilt 256 MiB (`FALLBACK_BUDGET`), wie in
`native/crates/transfer/src/transfer/os/shared/memory.rs:14-35`. Das Budget widerruft nie einen Puffer, den schon
eine Operation hält.

Regeln: Erlaubnisse gehen reihum zwischen Übertragungen (ein kleines Einfügen neben einem
großen Job kriecht nicht); Auflistungen haben einen reservierten Platz; Lokal↔Remote regelt nur
der Remote-Flow; nie mit gehaltener Erlaubnis auf etwas warten, das selbst Erlaubnis, Speicher
oder einen Kanal braucht; Speicher wird vor der Erlaubnis reserviert. Flows sind prozesslokal:
Sync-Jobs im Hintergrund-Dienst regeln getrennt von der GUI. Die QUIC-Fenster der Share-Verbindung
(16 bis 64 MiB, `share/core/keepalive.rs`) bleiben unverändert; das Speicherbudget erhöht sie nicht.

## Teilbereiche großer Dateien

- **Wann.** Eine Datei ab 16 MiB (`SPLIT_FROM` = zwei kürzeste Teile) wird in Teile zerlegt
  (`native/crates/transfer/src/transfer/core/segments.rs:11`, `:13`, `splits` in `:23`). Kleinere Dateien laufen als ein Strom.
- **Wie lang.** Ein Teil ist mindestens 8 MiB (`MIN_SEGMENT`). Der Teil gibt höchstens ein Zehntel
  seiner Zeit für den Aufbau aus (`SETUP_SHARE`) und dauert höchstens 30 s als ein Strom
  (`MAX_SEGMENT_SECONDS`). Die Länge des nächsten Teils ergibt sich aus der gemessenen Anlaufzeit
  und Streamrate der fertigen Teile dieser Verbindung (`SegmentSizer`, `segments.rs:23-60`).
- **Ablage.** Die Stufe wird vor dem ersten Teil auf die Länge der Datei gesetzt (`set_len`,
  `native/crates/transfer/src/transfer/os/shared/engine/download.rs:73`); die Teile landen als dünn besetzte Datei,
  die fertig ist, wenn alle Teile fertig sind. Ein Abbruch behält die fertige Präfixlänge
  (`prefix()`), und das Fortsetzen beginnt dort (`segments.rs:126`).
- **Wo.** Die Engine führt Teile in `segments_download`, `segments_upload` und `segments_copy` aus
  (`native/crates/transfer/src/transfer/os/shared/engine/`), unter den Erlaubnissen des Flows. Welches Protokoll
  wie teilt, steht in der Tabelle „Je Protokoll“.

## Wer begrenzt

Jede Operation bucht die Zeit ihres Fadens: Lesen von der Quelle, Schreiben zum Ziel, Rechnen
(Hash, Kompression, Verschlüsselung) und Warten auf Erlaubnis, Speicher oder Ordner. Eine Sekunde
lang ergibt die Summe ein **Urteil** über den Engpass (`native/crates/transfer/src/transfer/core/limiter.rs`,
Fenster in `os/shared/engine/bottleneck.rs`, `WINDOW` = 1 s).

Urteile und ihre Texte (`limiter.rs:12-40`, Texte `limiter.rs:277-291`; Code für MCP und Bridge):

| Urteil | Code | Text |
|---|---|---|
| kein Urteil | `none` | – |
| Anlauf: die Parallelität wächst noch | `ramp` | „Läuft an …“ |
| Nachschub: die Quelle liefert nicht schnell genug | `idle` | „Wartet auf Nachschub (Quelle liefert nicht schnell genug)“ |
| lokale Quelle | `source` | „Begrenzt durch das Laufwerk <Name>“ |
| lokales Ziel | `target` | „Begrenzt durch das Laufwerk <Name>“ |
| Leitung oder Server der Gegenseite | `remote` | „Begrenzt durch Leitung oder Server <Name>“ |
| Leitung selbst (Rate nahe der Linkrate) | `link` | „Begrenzt durch die Leitung (<Rate> von <Linkrate>)“ |
| Rechenzeit von Smart Explorer (alle Kerne belegt) | `se_cpu` | „Begrenzt durch Smart Explorer (Rechenzeit)“ |
| Rechnen auf einem Faden, weitere Kerne frei | `se_thread` | „Smart Explorer rechnet mit einem Faden; weitere Kerne sind frei“ |
| Warten auf Arbeitsspeicher (Puffer) | `memory` | „Wartet auf Arbeitsspeicher (Puffer)“ |

Die Regeln in dieser Reihenfolge (`judge`, `limiter.rs:121`): Nachfrage füllt die Grenze nicht →
`idle`; Parallelität wächst noch → `ramp`; meist Wartezeit auf Speicher → `memory`; eine Seite mit
mehr als der Hälfte der Arbeitszeit → diese Seite; sonst `remote`. Leitung: die Rate erreicht
9/10 der Linkrate (`LINK_SHARE`, `limiter.rs:109`); eigene Rechenzeit: alle Kerne zu mindestens
90 % belegt (`CPU_SHARE`, `limiter.rs:111`).

- **Drei Fenster.** Ein Urteil wird erst angezeigt, wenn drei Fenster hintereinander dasselbe
  Urteil ergeben (`CONFIRM_WINDOWS` = 3, `limiter.rs:114`). Ein einzelnes Fenster kippt die
  Anzeige nicht.
- **Abschlusswert.** Die letzte Meldung trägt das letzte Urteil, das einen Engpass benannt hat
  (`bottleneck.rs:1-8`). So zeigen `--stats` und die fertige Liste den Engpass, auch wenn die
  Warteschlange am Ende leer war.
- **Wo es erscheint.** Die Fortschrittsmeldung trägt das Urteil (`limiter`), den Text
  (`limiter_text`), die Rate (`rate_bps`), die Teile (`segments`) und den Hinweis der Verbindung
  (`note`, `native/crates/transfer/src/transfer/core/types.rs:58`; Vorrang hat „wartet auf <Verbindung>“,
  `engine/mod.rs:359-366`, sonst der Hinweis des Ziels, dann der der Quelle, `engine/mod.rs:367-376`).
  Der Hinweis ist z. B. „2 Verbindungen zu <Host>“ (SFTP, siehe unten). Das Übertragungsfenster
  zeigt den Hinweis unter der Aktivitätszeile (`native/crates/gui/src/app/core/transfer_window.rs:240-241`).
  Das MCP-Werkzeug `copy` gibt ihn im Feld `progress.note` aus (`native/crates/cli/src/cli/mcp/transfer_tools.rs:104-106`).
  Android zeigt ihn als Meldung der Aufgabe neben den Zahlen (`native/crates/mobile/src/mobile/os/shared/drive.rs:99`,
  `task_message`; während der Suche steht dort „Suche Dateien…“).
  `se cp --stats` gibt ihn nicht aus (keine Lesung in `native/crates/cli/src/cli/`); es gibt dort die Zeile mit Rate und Engpass.
- **Namen.** Die Texte nennen Laufwerke, Geräte und Arten von Endpunkten, nie eine Datei oder einen
  Ordner (`bottleneck.rs:9-12`).

## Je Protokoll

| Protokoll | Was es erlaubt | Was die Engine nutzt | Grenzen und Zahlen |
|---|---|---|---|
| Lokal, UNC | Kernel-Kopie, Server-Offload | Kopie über die exklusiv erzeugte Stufe (Linux `copy_file_range` zwischen Handles, Windows `CopyFile2` „nicht ersetzen“ + Identitätsprüfung); innerhalb einer UNC-Freigabe kopiert der SMB-Server selbst (Offload/COPYCHUNK); kein fsync bei neuen Kopien, Verschieben auf demselben Volume als eine Umbenennung; Ring und Bereiche: siehe „Lokal, UNC“ unten | fsync nur bei Verschieben und Überschreiben; Abbrechen wirkt zwischen den Kopierblöcken |
| Direct/Raum-Share (Iroh/QUIC) | viele Ströme je Verbindung | `fs_transfer_v1`: Pakete für kleine Dateien (Statusabfrage bei verlorener Antwort), Lesen ab Offset, exklusive Ordner, serverseitige Kopie; Bereiche mit der Capability `ranges_v1` (`native/crates/engine/src/share/core/batch_wire.rs:58`), LZ4 mit `lz4_v1` (`batch_wire.rs:63`, Abschnitt „Kompression“); alte Hosts: bisheriger Weg | 64 Ströme − 4 fürs Blättern = 60 Übertragungen je Gegenstelle (Host-Zulassung), davon nutzt der Flow höchstens 56, 4 bleiben für Vordergrund-Lesen; Host gesamt 256 (halbe Tokio-Blocking-Threads); alte Hosts 32; Paket ≤ 256 Dateien/16 MiB, Kopf ≤ 256 KiB; Fenster 16 MiB je Strom (1 Gbit/s × 100 ms), Verbindung 16–64 MiB je nach Speicher; voller Host antwortet „busy“ statt heimlich zu warten, alte Clients warten höchstens 45 s (ihre Frist ist 60 s). Stufen-Bereiche nur bei lokalen Exporten; für nicht lokale Exporte fehlt eine Handle-Schnittstelle in `vfs` (Stand U53). Unfertige Bereichsstufen des Wirts werden beim Verbindungsende, nach 300 s ohne Fortschritt und beim nächsten Start entfernt (Stand U53) |
| SSH-Agent / Hintergrund-Dienst | Anfragen mit IDs über einen Kanal | Kredit je Anfrage (`+credit-v1`: ein langsamer Download blockiert kein Blättern), Pakete (`+batch-v1`), Stufen-Frames, Pool zusätzlicher Exec-Kanäle; Bereiche mit `Read {offset, len}`, Label `range-v1` (`native/crates/storage/src/agent_proto/core/features.rs:17`); die Bereichsstufe bleibt auf dem Kanal, der sie geöffnet hat (Stand U53) | 64 MiB Budget je Verbindung, 1 MiB Anfangskredit, Fenster bis 8 MiB je Strom; Kanal-Pool bis MaxSessions − 2; |
| SFTP | gepipelinete Anfragen, mehrere Kanäle | viele gleichzeitige READs mit wachsender Tiefe, gebündelte WRITEs (Pakete, 16 MiB je Paket, `BATCH_BYTES`), Kanal-Pool, Kopien ohne fsync, serverseitige Kopie per `copy-data` wo angeboten (in Bereichen von ~10 s, lehnt der Server ab, wird gestreamt); innerhalb eines Kontos bildet die Engine Pakete aus Serverkopien: bis zu 64 Kopierketten gleichzeitig auf einem Kanal (`vfs::server_copy_batch`, `engine/batch_remote_server.rs`, Arbeitsbaum 2026-10-28). Bereiche über Pool-Kanäle, die sich Handles teilen, und über Zusatzverbindungen (Regeln unter „SFTP-Kanäle und -Zusatzverbindungen“); jeder Kanal hat sein eigenes Serverfenster (OpenSSH 2 MiB). Schreiben von Bereichen nur, wenn die Ordnerkette bis `/` dem Konto oder root gehört und für Gruppe und andere gesperrt ist (oder Sticky-Bit und root; Sicherheitsantwort S7). Dateien mit gelistetem MD5 bleiben ein Strom | Dateien je Kanal gleichzeitig offen: höchstens `min(open-handles, 64)` (`OPEN_HANDLES`, `native/crates/transfer/src/sftp/core/batch_ops.rs:52`; ohne Angabe des Servers 64); ein OPEN mit einfachem Fehlerstatus oder ohne Antwort ist Überlast, ein bestimmter Status (nicht gefunden, Zugriff verweigert) ein Dateifehler (`open_failure`); jede Datei hat vier Wartezeiten (OPEN, WRITE, CLOSE, RENAME), die sich über bis zu `open-handles` Dateien überlappen; Pool bis zur Ablehnung, 2 Kanäle Reserve; Kanäle in Einrichtung zählen bei der gelernten Grenze mit (`channel_pool.rs:57-59`, `:168-170`, `:357`); Obergrenze des Flows `transfer_ceiling` = Kanäle × Handles je Kanal (`backend.rs:428-430`, `channel_pool.rs:288-301`); `mkdir_all` fragt alle Pfadteile mit einem Schwung `LSTAT` ab (eine Rundreise) und legt nur den fehlenden Rest an, jeder vorhandene Teil muss ein echter Ordner sein (MS7, `native/crates/transfer/src/sftp/core/transfer_ops.rs`); `finish_stage`: `LSTAT`, `OPEN`, dann `FSETSTAT`, `fsync@openssh.com`, `FSTAT`, `CLOSE` und `LSTAT` zusammen (3 statt 6 Rundreisen; MS8, `native/crates/transfer/src/sftp/core/stage_finish.rs`) |
| FTP/FTPS | eine Übertragung je Steuerverbindung | Verbindungs-Pool, Größen-Stufe mit streamendem STOR, Fortsetzen per REST; Prüfen und Veröffentlichen auf der Verbindung der Übertragung (SIZE für den zufälligen Stufennamen, eine Auflistung des Ordners für Stufe und Ziel, dann RNFR/RNTO), ohne freie Verbindung auf der Blätter-Verbindung. Bereiche über `REST` auf einer freien oder für den Bereich neu angemeldeten Pool-Verbindung (gleiche Zugangsdaten, gleiche TLS-Regel; nie die Blätter-Verbindung), Abbruch mit `ABOR` (G3-11, Entscheidung 2026-10-28, ersetzt S12; Abschnitt „Entscheidung G3-10/G3-11“) | STOR und RETR gehen vor dem TCP-Aufbau der Datenverbindung hinaus (suppaftp); es gibt kein Befehls-Pipelining: das nächste Kommando geht erst nach der Antwort auf das vorige (Testserver `native/crates/storage/src/ftp/core/data_overlap_server.rs`). Grenze lernt der Pool aus 421/530 nach erfolgreicher Anmeldung, auch bei Anmeldungen für Bereiche; wie viele Bereiche gleichzeitig laufen, bestimmt das gemessene Fenster des Flows; Bereiche ab 16 MiB Dateigröße (`SPLIT_FROM`); Bereiche nur, wenn der Server mindestens zwei Übertragungsverbindungen zulässt (`ftp/core/pool.rs` `ranges_possible`); 421/530 auf REST/RETR: 15 Minuten keine Bereiche; eine Verbindung bleibt fürs Blättern; Stufenprobe entfällt nur für Namen `.se-upload-<16 Hex>` (`FTP_PROBE_STAGE_NAME`, S6) |
| WebDAV | viele HTTP-Verbindungen, COPY | gepoolte Mutationen, gestreamtes PUT mit Länge und `If-None-Match: *`, serverseitiges COPY, Range. Engine-Weg: PUT, PROPFIND (Stufe prüfen), MOVE. Sync-Weg: die Bestätigung der Zeit nach MOVE entfällt, wenn die MOVE-Antwort `X-OC-Mtime: accepted` trägt (`native/crates/engine/src/webdav/core/stage_move.rs:4-10`); ein PROPFIND je Sync-Upload weniger. Bereiche über `Range: bytes=a-b`; antwortet der Server mit 200, läuft die Datei als ein Strom. Nextcloud (Wurzel unter `/remote.php/dav/files/<user>/`, G3-12, Arbeitsbaum 2026-10-28): große Dateien als segmentierte Stufe über den Chunk-Upload: Upload-Ordner (`MKCOL` mit `Destination`), je Bereich nummerierte Teil-`PUT`s auf eigenen Verbindungen (`Destination`, `OC-Total-Length`), `MOVE …/.file` in die private Stufe, Länge per PROPFIND geprüft, dann das übliche `MOVE` mit `Overwrite: F`; ein Server, der den Upload-Ordner ablehnt, behält je Verbindung das einzelne `PUT` (`native/crates/engine/src/webdav/core/chunked_stage.rs`) | 429/503 → Überlast mit `Retry-After`; DELETE und leeres PUT ungepoolt (ureq würde sie wiederholen); Chunk-Upload höchstens 10000 Teile, ab ≈ 78 GiB ein `PUT` |
| Google Drive | parallele Verbindungen, `generateIds` | gepoolte Verbindungen, neue Datei ≤ 5 MB mit einem Aufruf, Resumable mit wachsenden Blöcken und zwei Puffern (ein Abschnitt geht auf einem eigenen Faden hinaus, während der nächste gefüllt wird; nur wenn der Speicher für den zweiten Puffer reserviert werden kann; G3-08c, `native/crates/engine/src/gdrive/core/chunk_stream.rs`), ID-Vorrat, Cache im Hintergrund, Sperren je Name, `files.copy`; Bereiche über `Range: bytes=a-b`, antwortet der Server mit 200, läuft die Datei als ein Strom. Neue Datei in einem bestehenden Ordner mit einem Schreibaufruf (`vfs::open_write_absent`, G3-08b, Arbeitsbaum 2026-10-28): Anlegen unter dem Endnamen, danach muss der Name genau zur eigenen ID führen, sonst geht das eigene Objekt per ID in den Papierkorb und der Schreiber meldet `AlreadyExists`; die Engine nimmt den Weg für eine neue Datei der obersten Ebene, deren Name die Auflistung des Zielordners zu Auftragsbeginn als frei zeigte, unter „beide behalten“ (`Destination::Absent` in `engine/publish.rs`); ist der Name inzwischen belegt, läuft die Datei sofort über die private Stufe und bekommt die nächste Nummer, ohne ihren einen Wiederholungsversuch zu verbrauchen (`native/crates/engine/src/gdrive/core/absent_write.rs`) | Google begrenzt dauerhaft ~3 neue Dateien/s je Konto (Hinweis in der Zeile); Ratenlimits sofort als Überlast |
| SMB | Credits, COPYCHUNK | Pakete über Compounds: CREATE, WRITE und CLOSE je Datei und das Umbenennen in zwei Rundreisen; Reparse-Punkte werden nie über `read_file_compound*` gelesen (eigene Anfragen, damit Links nicht gefolgt werden); mehrere Lese-Blöcke gleichzeitig, serverseitige Kopie wo angeboten, neue Kopien ohne FLUSH; Bereiche über `read_at`; Schreiben in Bereichen über eigene Handles, die vor dem ersten Byte gegen die Kennung der Stufe geprüft werden (S14); Belege für Kopierstufen der Engine (G3-02): der `stat` des Ziels geht mit dem CLOSE der Stufe hinaus, der `stat` der Stufe direkt danach, die Veröffentlichung ist ein Compound CREATE (kein Ordner, Link als er selbst) + SET_INFO (Umbenennen ohne Ersetzen) + CLOSE, dessen CREATE-Antwort die veröffentlichte Datei beschreibt (`native/crates/transfer/src/smb/core/stage_evidence.rs`). Einzeldateien (G3-07, Arbeitsbaum 2026-10-28): Lesen öffnet mit CREATE + erstem READ (512 KiB) in einem Compound, ein passender Bereich mit CLOSE (`compound_read.rs`); eine Engine-Stufe bis MaxWriteSize wird beim Öffnen exklusiv angelegt (belegter Name dort `AlreadyExists`) und beim Commit mit WRITE + CLOSE (`POSTQUERY_ATTRIB`, liefert den Beleg der Stufe) in einem Compound abgeschlossen, der Stat des Ziels daneben (`compound_put.rs`) | Block ≤ 512 KiB bzw. MaxReadSize, Tiefe nach Credits; eine Serverkopie sieht Abbrechen erst an ihrem Ende |
| ZIP (Quelle) | – | ein Parse je Archiv, Einträge gestreamt | Parallelität = Kernzahl (Entpacken ist CPU-gebunden) |

**Prüfschlüssel ohne Dateiübertragung (`vfs::content_digest`, Stand 2026-10-31, nicht gebaut).** Wo die
Gegenseite selbst hashen kann, bekommt jeder Aufrufer (Sync im Prüfsummen-Modus, Duplikatsuche, Überprüfen
nach Kopien, Eigentumsprüfung beim Fortsetzen) die SHA-256 als Antwort, ohne die Datei zu laden:

| Backend | Weg | Voraussetzung, sonst `Ok(None)` und der bisherige Weg | Quelle |
|---|---|---|---|
| SFTP ohne Agent | Exec-Anfrage auf einem zusätzlichen Sitzungskanal der Verbindung mit der festen Befehlszeile `sh -c '<fester Text>'` (gleich für jede Datei, ohne Pfad, Namen oder sonst ein Byte von Nutzer oder Dateisystem) mit `sha256sum`, `shasum -a 256` oder `sha256 -q`; der Pfad geht danach über die Standardeingabe des Kanals (`P`, die Bytes des Pfads, dann EOF), das Skript liest ihn mit `$(cat; echo x)`, die Datei über eine Umleitung; jede Antwort ist eine Zeile mit einer Marke, die das Skript erst zur Laufzeit zusammensetzt (`se-digest-v1 ok <Größe> <hex>`, `none`, `notool`, `fail`); Größe und Zeit per SFTP vor und nach dem Befehl müssen gleich sein und die vom Skript gelesene Größe gleich der SFTP-Größe (`sftp/core/digest_command.rs`, `digest_exec.rs`); Wartezeit 60 s plus 1 s je 8 MiB (langsamer lohnt sich gegenüber dem Laden nicht), Abbruch sendet TERM | das Konto darf Befehle ausführen, seine Login-Shell führt den Text als POSIX-sh aus, der Host hat ein SHA-256-Werkzeug. Eine abgelehnte Anfrage, eine Antwort ohne Marke (Windows-OpenSSH mit cmd.exe oder PowerShell, eingeschränkte Shells, erzwungene Befehle), `notool`, `none` bei per SFTP unveränderter Datei oder eine andere Größe des Skripts schließen den Weg für die Verbindung (`Ok(None)`, Download wie bisher); drei `fail`, Zeitabläufe oder Statusfehler in Folge ebenfalls. „Datei geändert“ bedeutet nur eine per SFTP vorher/nachher abweichende Größe, Zeit oder Art. Ein Konto mit `ForceCommand internal-sftp` endet sofort: OpenSSH antwortet auf exec mit „This service allows sftp connections only.“ und Status 1 (`session.c`), ein SFTP-Server am Kanal liest `P/…` als Paketlänge über 256 KiB und endet mit 11 bzw. bei EOF mit 0 (`sftp-server.c` `process`) | RFC 4254 Abschnitt 5.3 (EOF), 6.5 (exec), 6.9 (signal); Quotierung siehe unten |
| FTP/FTPS | `OPTS HASH SHA-256`, dann `HASH <Pfad>` (Antwort `213 SHA-256 0-<Ende> <hex> <Pfad>`), sonst `XSHA256 <Pfad>` (Antwort `213`/`250` mit einem 64-stelligen Token); keine Datenverbindung, die Lesezeit der Steuerverbindung wird auf die Wartezeit gehoben und danach zurückgesetzt; Größe und Zeit vorher/nachher (`ftp/core/content_digest.rs`) | FEAT nennt `HASH` mit `SHA-256` bzw. `XSHA256`; Algorithmus, Bereich (ganze Datei, Ende gleich Größe oder Größe − 1) und Antwortcode müssen stimmen; eine Ablehnung gibt `Ok(None)`. Ein laufender Befehl lässt sich auf der Steuerverbindung nicht abbrechen (RFC 959 kennt `ABOR` nur für Datenverbindungen), er ist durch die Wartezeit begrenzt. Antwortcode von `XSHA256` bei echten Servern: nicht gemessen (213, 250 und 251 werden angenommen) | draft-bryan-ftpext-hash-02; ProFTPD mod_digest |
| WebDAV, Nextcloud, ownCloud | eine PROPFIND-Anfrage, `oc:checksums` mit `SHA256:<hex>`; Größe aus derselben Antwort (`webdav/core/content_digest.rs`) | der Server speichert eine SHA-256 (Nextcloud-Entwicklerhandbuch: MD5, SHA1, SHA256, SHA3-256, Adler32 als Typen); MD5 und SHA1 passen nicht zum SHA-256-Vertrag. Der Wert ist, was ein Hochladender mitsandte (bei einfachem `PUT` ungeprüft gespeichert); diese Engine sendet ihn nie. Deshalb stützt die Engine ihre Belege (Eigentum, Überprüfen) nicht darauf: `file_evidence::on_device` lässt WebDAV aus, sie liest zurück wie zuvor | Nextcloud Developer Manual, „Basic file and folder operations“ |
| Google Drive | `files.get` mit `fields=sha256Checksum` auf der genauen ID (`gdrive/core/content_digest.rs`) | Datei mit gespeichertem Inhalt; für Docs-Editors-Dateien und Verknüpfungen fehlt das Feld (`Ok(None)`); „if available“: ist es noch nicht berechnet, liest die Engine zurück | Drive API v3, `files` |
| SMB | bleibt bei Übertragung | SMB 2/3 kennt keine serverseitige Prüfsumme des Dateiinhalts (kein Befehl in MS-SMB2; `FSCTL_SRV_COPYCHUNK` kopiert, hasht nicht) | MS-SMB2 |

Warum die feste Befehlszeile in jeder Login-Shell gleich gelesen wird (sshd startet
`<Login-Shell> -c <Text>`, OpenSSH `session.c` `do_child`; geprüft 2026-10-31): der Text zwischen den
einfachen Anführungszeichen enthält kein `'`, kein `\`, kein `!`, keinen Zeilenumbruch. POSIX-sh, bash,
dash, ksh: einfache Anführungszeichen erhalten jedes Zeichen wörtlich (POSIX.1-2024 XCU 2.2.2
„Single-Quotes“). zsh: ebenso, nur `RC_QUOTES` ändert `''` (zsh-Handbuch 6.9 „Quoting“). fish: in
einfachen Anführungszeichen sind nur `\'` und `\\` Escapes (fish-Dokumentation „Quotes“) – daran
scheiterte die frühere Fassung mit dem Pfad als `'\''`-gequotetem Wort (Name `a\'; id; #`). tcsh/csh:
einfache Anführungszeichen verhindern jede Ersetzung außer der History-Ersetzung `!` (tcsh(1) 6.24,
„Lexical structure“). Eine Shell, die den Text anders liest, kann die Marke nicht ausgeben, weil sie
nicht im Text steht (`a=se-; m=${a}digest-v1`). Windows-OpenSSH startet standardmäßig cmd.exe, wahlweise
PowerShell (`HKLM\SOFTWARE\OpenSSH` `DefaultShell`, Microsoft Learn „OpenSSH Server configuration for
Windows“): dort kommt keine Marke, der Weg schließt sich. Ein Test führt den Text mit jeder auf dem
Runner vorhandenen Shell aus (`sftp/core/abgleich_task_digest_shell_tests.rs`).

Überprüfen nach Kopien und Eigentumsprüfung (`engine/file_evidence.rs` `current`, `on_device`) fragen
zuerst `vfs::content_digest` der Seite, auf der die Datei liegt, und lesen nur zurück, wenn sie keinen
Prüfschlüssel bilden kann; lokal, Share, Agent und jetzt SFTP mit Befehlen, FTP mit `HASH`/`XSHA256` und Drive
liefern ihn ohne Übertragung. Ein Prüfschlüssel, den der Server nur als Behauptung eines Hochladenden führt
(WebDAV), zählt dort nicht.

**MD5 am Ort der Datei (`vfs::content_md5`, Stand 2026-10-31, nicht gebaut).** Die Signatur des Syncs ist
eine MD5 (gespeicherte Basen). Der Sync fragt vor jedem Download `vfs::content_md5` der Seite
(`bisync/os/shared/snapshot_dir.rs` `content_hash`), im Durchlauf zuvor die Dateien eines Ordners zusammen
(`vfs::content_md5_many`, Block „MD5 vieler Dateien in einer Anfrage“ unten); die Backends überschreiben `BackendExtensions::content_md5`
und liefern die MD5 ohne Dateiübertragung, sofern die Gegenseite sie bilden kann. Sonst gibt der Aufruf
`Ok(None)` und der Sync nimmt den bisherigen Weg (Datei laden und hashen). Das Ergebnis muss 32 Hex-Ziffern
sein (`vfs::content_md5` lehnt anderes mit `InvalidData` ab).

| Backend | Weg | Voraussetzung, sonst `Ok(None)` und der bisherige Weg | Quelle |
|---|---|---|---|
| SFTP ohne Agent | derselbe Exec-Weg wie bei SHA-256 mit einem zweiten festen Text (`MD5_SCRIPT`): `md5sum` (GNU coreutils, busybox) oder `md5 -q` (BSD, macOS), Pfad über die Standardeingabe, Datei über eine Umleitung, Antwortzeile mit Marke; kein `openssl`, kein weiteres Werkzeug; Größe und Zeit per SFTP vor und nach dem Befehl müssen gleich sein (`sftp/core/digest_command.rs` `hash_by`, `Algorithm::Md5`) | das Konto darf Befehle ausführen, die Login-Shell führt den Text als POSIX-sh aus, der Host hat `md5sum` oder `md5`; die Sperre „Weg geschlossen“ gilt je Algorithmus (ein Host ohne `sha256sum` mit `md5sum` bleibt für MD5 offen); dieselben Gründe wie bei SHA-256 schließen den MD5-Weg der Verbindung | RFC 4254 Abschnitt 5.3, 6.5 |
| FTP/FTPS | `OPTS HASH MD5`, dann `HASH <Pfad>` (Antwort `213 MD5 0-<Ende> <hex> <Pfad>`), sonst `XMD5 <Pfad>` (Antwort `251`, `250` oder `213` mit genau einem 32-stelligen Token); keine Datenverbindung, Lesezeit wie bei SHA-256; Größe und Zeit vorher/nachher (`ftp/core/content_digest.rs`, `Algorithm::Md5`) | FEAT nennt `HASH` mit `MD5` bzw. `XMD5`; Algorithmus, Bereich (ganze Datei) und Antwortcode müssen stimmen; Ablehnung gibt `Ok(None)`. Antwortcode von `XMD5` bei echten Servern: nicht gemessen und an keiner Quelle geprüft (251, 250 und 213 werden angenommen) | draft-bryan-ftpext-hash-02 |
| WebDAV, Nextcloud, ownCloud | eine PROPFIND-Anfrage (Tiefe 0), `oc:checksums` mit `MD5:<hex>` (`webdav/core/content_digest.rs` `md5_from_propfind`) | der Server nennt eine MD5; der Wert ist wie bei SHA-256 die Angabe, die ein Hochladender mitsandte, und derselbe Wert, den die Auflistungen schon tragen; diese Engine sendet ihn nie. Ohne MD5-Eintrag: `Ok(None)`. Je Verbindung gemerkt (`ChecksumMemory`, je Algorithmus): zeigte eine Auflistung mit Dateien keine MD5 oder die erste Tiefe-0-Antwort keine MD5 bzw. SHA-256, gibt die Verbindung `Ok(None)` ohne weitere PROPFIND (Apache `mod_dav`, nginx, IIS); eine spätere Auflistung oder Antwort mit Prüfsumme öffnet den Weg wieder | Nextcloud Developer Manual, „Basic file and folder operations“ |
| Google Drive | die MD5 steht schon in der Auflistung (`md5Checksum`, `gdrive/core/backend.rs`, `provides_content_hash`); kein eigener Aufruf nötig | – | Drive API v3, `files` |
| SMB | bleibt bei Übertragung | SMB 2/3 kennt keine serverseitige Prüfsumme des Dateiinhalts | MS-SMB2 |

**MD5 vieler Dateien in einer Anfrage (`vfs::content_md5_many`, Stand 2026-11-01, nicht gebaut).** Der
Durchlauf einer Seite im Prüfsummen-Modus fragt die MD5 aller Dateien eines Ordners, deren Inhalt er hashen
muss, zusammen ab (`bisync/os/shared/snapshot_dir.rs` `batched_md5`), bevor eine Datei einzeln gefragt
oder geladen wird. Ursache (Messlauf 41253633650, 20 000 kleine Dateien in 2 000 Ordnern: 44,36 s und
97,0 MiB gegen 4,82 s und 74,9 MiB in v0.5.189): der Einzelweg öffnete je Datei einen Sitzungskanal und
startete je Datei eine Shell mit `cat`, `wc` und `md5sum`; Kanal, Shellstart und vier SFTP-Statusabfragen
kosteten je Datei mehr Zeit und Bytes als das Laden der kleinen Datei. SFTP ohne Agent beantwortet die
Bündelanfrage mit einer Exec-Anfrage je Gruppe (`sftp/core/digest_batch.rs`, `digest_batch_flow.rs`):

- feste Befehlszeile `sh -c '<fester Text>'` (`MD5_BATCH_SCRIPT`) ohne `'`, `\`, `!`, Zeilenumbruch und ohne
  Pfad; die Pfade kommen über die Standardeingabe: `P`, ein Trennbyte, dann die Einträge (Kennbyte und Pfad)
  durch dieses Byte getrennt. Das Trennbyte ist ein Steuerzeichen (0x02 bis 0x1F ohne Tabulator und
  Zeilenumbruch), das in keinem Pfad der Gruppe vorkommt; das Skript teilt mit `IFS` auf genau dieses Byte,
  `set -f` und `LC_ALL=C`. Ein Pfad mit NUL, ein leerer Pfad oder ein Pfad, der jedes Trennbyte enthält,
  geht den Einzelweg;
- das Skript ruft `wc -c` und `md5sum` (bzw. `md5 -r`) je Gruppe einmal für alle Dateien auf, nicht je
  Datei; ein Pfad mit Zeilenumbruch (Kennbyte `o`, Werkzeuge könnten seinen Namen auf zwei Zeilen
  schreiben) wird innerhalb derselben Anfrage über Umleitungen einzeln gehasht. Gelesen wird nur das erste
  Wort jeder Werkzeugzeile; stimmt die Zeilenzahl eines Abschnitts nicht mit der Zahl der Dateien überein
  (Datei zwischen Test und Werkzeug verschwunden), gilt der Abschnitt für keine Datei;
- jede Antwortzeile beginnt mit der zur Laufzeit gebildeten Marke (`se-digest-v1 file|none|fail|one`,
  `sizes`, `digests`, `end`); ohne Marke (cmd.exe, PowerShell, erzwungener Befehl), bei `notool`, bei
  abgelehnter Exec-Anfrage, bei `none` für eine per SFTP unveränderte Datei oder bei einer vom Skript
  gelesenen Größe ungleich der SFTP-Größe schließt sich der MD5-Weg der Verbindung (dieselbe Sperre wie
  beim Einzelweg); ein Konto mit `ForceCommand internal-sftp` endet sofort wie beim Einzelweg;
- Größe, Zeit und Art jeder Datei per SFTP vor dem Befehl (gleich der Auflistung) und danach (gleich der
  Auflistung), beide Male als viele LSTAT-Anfragen gleichzeitig auf der Hauptsitzung; nur dann gilt die
  MD5; jede andere Datei bekommt `None` und geht den Einzelweg (`vfs::content_md5`, danach der Download)
  mit dessen Fehlern und Auslassungen;
- eine Gruppe hat höchstens 1 024 Dateien, eine geschätzte Ausgabe unter 56 KiB (die Erfassung behält
  64 KiB) und höchstens 256 MiB Inhalt (Wartezeit 60 s plus 1 s je 8 MiB); drei nicht aufgehende
  Gruppenantworten in Folge schließen nur den Bündelweg der Verbindung (`md5_batch_gate`), der Einzelweg
  bleibt;
- höchstens 8 Prüfsummenbefehle laufen je Verbindung gleichzeitig (`sftp/core/exec_slots.rs`; OpenSSH
  öffnet je Verbindung `MaxSessions` = 10 Sitzungskanäle, einen davon hält die SFTP-Sitzung). Der
  Durchlauf listet mehrere Ordner gleichzeitig, und jeder fragt seine MD5s über einen eigenen Kanal ab.
  Lehnt der Server einen Kanal ab, während andere Prüfsummenbefehle laufen, sinkt die Grenze auf deren
  Zahl, und die Anfrage wartet, bis einer endet. Nur eine Ablehnung ohne laufenden Prüfsummenbefehl gibt
  `Ok(None)`. Ursache: Im Messlauf 41262127011 (Bündelweg ohne diese Grenze) brauchte LS im
  Prüfsummenmodus 8,30 s bei 36,6 MiB (v0.5.189, das den Inhalt lädt: 5,73 s bei 67,2 MiB). Ein Lauf von
  dreien brauchte 19,3 s bei 72,2 MiB, also so viel wie mit Download. Die Dateien abgelehnter Kanäle nahmen
  den Einzelweg, bekamen auch dort keinen Kanal und luden deshalb den Inhalt.

Durchgereicht wird die Bündelanfrage von Agent-Verbindung (an die SFTP-Verbindung), Einhängeproxy,
Wurzel-Backend des Dienstes (jeder Pfad wie bei `content_md5` aufgelöst), Share-Freigabe (nur lesbare
Pfade, Lesbarkeit danach erneut geprüft), Gerätebaum und Browsing-Cache. Seiten ohne Bündelweg (FTP
`HASH`, WebDAV, Share-Gegenstelle) fragen wie bisher je Datei. Nicht gebündelt sind die Einzelprüfungen
außerhalb des Durchlaufs (`incremental_changes.rs` `target_rel_drifted`, `incremental_index_commit.rs`,
`move_finalize.rs`, eigene Schreibvorgänge im Dienst).

**SFTP-Kanäle und -Zusatzverbindungen.** Ein Bereich nimmt einen leeren Kanal. Gibt es keinen, öffnet er
einen neuen, solange die gelernte Grenze es erlaubt; sonst teilt er den am wenigsten belasteten
Kanal mit anderen Bereichen, sofern dort noch Handles frei sind (`native/crates/transfer/src/sftp/core/link_rules.rs:272-284`,
`:351-359`). Sind alle Kanäle an ihrer Handle-Grenze, wartet ein Bereich höchstens 30 s auf ein Ende
(`channel_pool.rs:50`, `:318`, `:331`), danach nutzt er die Hauptsitzung. Eine ganze Datei wartet
nicht: Sind alle Kanäle voll, nutzt sie sofort die Hauptsitzung (`channel_pool.rs:331`). Kanäle, deren Einrichtung noch
läuft, zählen bei der gelernten Grenze mit (`channel_pool.rs:57-59`, `:168-170`).

Zusatzverbindungen sind eigene SSH-Verbindungen mit je einem Kanal im Subsystem `sftp`
(`native/crates/transfer/src/sftp/core/pool_channel.rs:284`). Sie öffnen nur Übertragungen; Auflistung, Metadaten,
Befehle und die Exec-Kanäle des se-agent bleiben auf der Hauptverbindung (Sicherheitsantwort E13-S15,
`docs/plaene/2026-10-17-rechte-desktop-tresore/stand/SEC.md`, Punkt 15, Frage 1 (vi); die
Anmeldung über den SSH-Agenten geändert durch G3-10, Abschnitt „Entscheidung G3-10/G3-11“). Sie
gelten nur unter diesen Bedingungen:

- **Anmeldung ohne Eingabe:** gespeichertes Passwort oder Schlüssel (`link_rules.rs` `plan`), oder
  der SSH-Agent mit der Identität, mit der sich die Hauptverbindung angemeldet hat (G3-10). Ein
  Schlüssel hat diese Herkunft, wenn das Formular keine Passphrase enthält
  (`native/crates/engine/src/connect/os/shared/connector.rs:100-102`); lässt sich der Schlüssel dann nicht laden,
  entsteht keine Zusatzverbindung. Die Herkunft wird nur über `SftpBackend::connect_with_origin`
  übergeben; `SftpBackend::connect` gilt als interaktiv: ein getipptes Passwort oder eine getippte
  Passphrase wird für Zusatzverbindungen nie aufbewahrt, nur der SSH-Agent ist dort möglich.
- **SSH-Agent nur ohne Rückfrage** (`native/crates/transfer/src/sftp/core/agent_grant.rs`): nur die Identität der
  Hauptanmeldung (nach Fingerabdruck), nie ein Sicherheitsschlüssel (`sk-ssh-ed25519@openssh.com`,
  `sk-ecdsa-sha2-nistp256@openssh.com`, fragt nach einer Berührung), und nur wenn die Signatur der
  Hauptanmeldung höchstens 750 ms dauerte (`QUICK_SIGNATURE`; eine Bestätigung `ssh-add -c` oder eine
  PIN dauert länger). Braucht die Signatur einer Zusatzanmeldung länger oder fehlt die Identität im
  Agenten, nutzt keine weitere Zusatzanmeldung dieses Kontos den Agenten mehr; höchstens diese eine
  Rückfrage kann also erscheinen. Der Agent wird dabei je Zusatzanmeldung über denselben
  Betriebssystem-Adapter erreicht wie bei der Hauptanmeldung (`sftp/os/ssh_agent_*.rs`).
- **Einzeln, bei Zuwachs:** Eine neue Verbindung öffnet sich erst, wenn die vorige den Durchsatz
  um mindestens 10 % angehoben hat (`link_rules.rs` `GROWTH_FACTOR`, `Growth::window`). Gemessen
  wird in Fenstern von 0,5 s (`WINDOW`, G3-10). Beurteilt wird eine neue Verbindung im ersten
  Fenster, das beginnt, nachdem sie ihr erstes Byte trug und danach so lange wie ihr Aufbau dauerte
  (mindestens ein Fenster, höchstens 2 s, `SETTLE`; `Growth::carried`, `pool_channel.rs` `moved`);
  trägt sie kein Byte, 2 s nach dem Aufbau. Ein Versuch läuft nur, wenn mindestens zwei Ströme je
  offener Verbindung laufen (`pool_growth.rs` `demand`). Bleibt der Zuwachs aus, folgt keine weitere
  Verbindung, bis ein Fenster ohne Übertragung die Regel zurücksetzt. Scheitert der Aufbau, gilt die
  Sperre unten.
- **Eine Anmeldung je Host und Port:** Während eine unangemeldete Verbindung zu Host und Port
  aufgebaut wird, beginnt keine zweite; der Versuch wird verschoben, nicht gesperrt
  (`link_rules.rs:236-240`, `pool_extra.rs:99-102`).
- **Host-Schlüssel:** Der Schlüssel muss im Vertrauensspeicher im Modus `Verify` stehen und in
  Algorithmus und Fingerabdruck dem der Hauptverbindung gleichen; unbekannte, geänderte oder
  widerrufene Schlüssel werden abgelehnt, und Zertifikate werden nicht anerkannt
  (`native/crates/transfer/src/sftp/core/extra_link.rs` `ExtraClient::judge`; `link_rules.rs` `same_host_key`). Die
  Ablehnung fällt vor jeder Anmeldung: an einen abgelehnten Server geht kein Passwort und keine
  Signatur. Sie beendet nur diesen einen Aufbau; die Hauptverbindung und schon arbeitende
  Zusatzverbindungen laufen weiter, die Übertragung nutzt deren Kanäle (Test
  `native/crates/transfer/src/sftp/core/extra_link_tests.rs`).
- **Zeitgrenze:** Von der TCP-Verbindung bis zur geöffneten Sitzung höchstens 30 s
  (`extra_link.rs:24`, `:309-311`).
- **Sperre:** Scheitert ein Aufbau, sind Zusatzverbindungen für Benutzer, Host und Port 15 Minuten
  gesperrt, für den ganzen Prozess (`link_rules.rs:15`, `:169-186`, `:219-222`;
  `pool_extra.rs:148`). Ein Abbruch einer schon arbeitenden Zusatzverbindung sperrt nicht; sie wird
  nur nicht ersetzt (Sicherheitsantwort E13-S15 (iii)).
- **Leerlauf:** Eine Zusatzverbindung ohne Übertragung schließt nach 60 s (`channel_pool.rs:46`,
  `:108-109`); eine Wache prüft alle 5 s (`pool_extra.rs:20`, `:166-190`).
- **Dienstprotokoll:** Jede Anmeldung („geöffnet“), jedes Schließen („geschlossen“) und jede Sperre
  („gesperrt für 15 Minuten“) steht mit Zeit, Host, Port, Benutzer, Methode, offenen Verbindungen
  und Grund in `daemon.log` (`native/crates/transfer/src/sftp/core/link_log.rs:1-4`, `:17-27`;
  `pool_extra.rs:138-160`; `extra_link.rs:204-209`). Passwörter, Passphrasen und Schlüsselmaterial
  stehen nie darin.
- **Hinweistext:** Neben der Übertragung steht „<n> Verbindungen zu <Host>“, sobald mehr als eine
  Verbindung offen ist, und während einer Sperre „Weitere Verbindungen zu <Host> 15 Minuten
  gesperrt: <Grund>“; beide Teile trennt „; “ (`link_rules.rs:288-299`, `pool_extra.rs:57-66`).
  Der Text läuft über das Feld `note` (Abschnitt „Wo es erscheint“).

**Entscheidung G3-10/G3-11 (2026-10-28).** Der Nutzer verlangte „so schnell wie irgendwie möglich“
und „maximal möglicher Datentransferspeed, at all times von Anfang zu Ende“ und überließ die
Entscheidung dem Hauptagenten („decide yourself“). Der Hauptagent hat für diesen Zweck entschieden;
die Entscheidung ersetzt die früheren Sicherheitsantworten S12 und E13-S15 (i) in diesen Punkten:

- **G3-10 (SFTP):** Große Dateien und Anmeldungen über den SSH-Agenten dürfen zusätzliche,
  angemeldete Verbindungen zum selben Host für parallele Ströme nutzen. Sie verwenden die
  gespeicherten Zugangsdaten bzw. die Agent-Identität der bestehenden Anmeldung, dieselbe
  Host-Schlüssel-Bindung, und lösen keine neue Rückfrage aus. Die Zahl der Ströme folgt dem
  gemessenen Durchsatz (Regeln oben). Unverändert aus E13-S15: eine unangemeldete Verbindung je Host
  und Port gleichzeitig, gleicher Host-Schlüssel, 15 Minuten Sperre nach einem Fehlschlag,
  Dienstprotokoll ohne Geheimnisse.
- **G3-11 (FTP/FTPS):** Große Downloads (ab 16 MiB) laufen in Bereichen über zusätzliche
  Anmeldungen mit denselben Zugangsdaten und derselben TLS-Regel wie die Hauptverbindung
  (`ftp/core/pool.rs` `lease_range`). Unverändert: die Blätter-Verbindung bleibt frei, 421/530 lehrt
  die Grenze, 421/530 auf einen Bereich schaltet Bereiche 15 Minuten ab.

**Lokal, UNC.** Auf Windows kopieren geschützte Quellen, Verschieben und Ersetzen über die
Handle-Kopie; zwischen Lesen und Schreiben liegt ein Ring aus 4 Blöcken zu je 1 MiB
(`HANDLE_COPY_BUFFER`, `HANDLE_COPY_DEPTH` in `native/crates/transfer/src/copy/os/windows.rs:148-151`). Die
Kernel-Kopie (`CopyFile2`) setzt `NO_BUFFERING`, sobald die Datei mindestens ein Viertel des
verfügbaren Speichers groß ist; ist der verfügbare Speicher unbekannt, nie
(`native/crates/transfer/src/copy/os/shared/copy_flags.rs:25`, `:47-58`). `REQUEST_COMPRESSED_TRAFFIC` ist aus
(`copy_flags.rs:19`). Unter Linux und Android laufen parallele Bereiche mit `copy_file_range` an
festen Offsets, die Bereichsgrenzen liegen auf ganzen MiB (`native/crates/transfer/src/copy/os/linux_range.rs:17`).
Sie laufen nur mit Erlaubnissen aus dem Flow und nur bei leerer Warteschlange, gepuffert nur auf einem
Laufwerk, das als nicht rotierend bekannt ist (`queue/rotational = 0`). Sehr große Dateien kopiert
Linux ungepuffert wie Windows mit `NO_BUFFERING`: ab einem Viertel von `MemAvailable` (unlesbar:
nie), zwischen zwei Blockgeräten und nicht dauerhaft (Verschieben, Ersetzen) öffnet se die schon
geprüften Handles über `/proc/self/fd` zusätzlich mit `O_DIRECT` (Identitätsvergleich, kein Öffnen
per Pfad) und kopiert den an der Einheit aus `statx(STATX_DIOALIGN)` (sonst 4096) ausgerichteten
Hauptteil mit 4-MiB-Anfragen auf bis zu vier Fäden; der Rest läuft gepuffert. Kann das Paar
klonen (`FICLONERANGE`), bleibt der Kernel-Klon; lehnt das Dateisystem `O_DIRECT` oder die erste
Anfrage ab, nimmt se den bisherigen Weg (`native/crates/transfer/src/copy/os/linux_direct.rs`,
`linux_direct_plan.rs`). Meldet das Laufwerk „rotierend“ oder nichts, misst se die Direktkopie
einmal je Gerätepaar: einen Abschnitt mit einem Faden, einen gleich großen mit zwei (je
`clamp(Größe/8, 32 MiB, 64 MiB)`); bringen zwei Fäden mindestens 10 %, läuft der Rest direkt
parallel, sonst als ein gepufferter Strom; das Urteil gilt für den Prozess
(`native/crates/transfer/src/copy/os/linux_probe.rs`, `native/crates/transfer/src/transfer/os/shared/engine/local_plan.rs`).
Ursache: Eine gepufferte Kopie (wie `cp`) bleibt durch Vorauslesen und Writeback bei geringer
Tiefe (Suite-Lauf 3/4: rund 145 MB/s gegen 203 MB/s, die das Laufwerk mit Tiefe schafft).
Netz-, FUSE- und virtuelle Dateisysteme ohne Blockgerät bleiben bei einem Faden. Windows hat in
diesem Stapel keine parallelen Bereiche.

**Terminal.** `se cp` läuft über die Engine (`CLI_ENGINE_PATH = true`,
`native/crates/cli/src/cli/engine_copy.rs:44`). `se cp --stats` schreibt nach den Hinweisen eine Zeile mit
Rate und Engpass auf die Fehlerausgabe (`native/crates/cli/src/cli/mod.rs`, `CopyArgs`; Stand U49). Fehler einzelner Dateien
beenden den Lauf nicht; der Exitcode ist dann 1. Eine Zugriffsfrage stellt die Engine im Terminal
und im MCP-Dienst nicht: Eine lokale Quelle, für die der Zugriffsschutz nach Leserechten fragt
(nur Windows), bleibt im Bestandsweg (`access_prompt_possible`, Stand U49). Vom Bestandsweg
offen (U49): `--stats` zeigt beim Kopieren eines Verknüpfungsziels auf einem Remote 0 Bytes.

## Kompression

- **Wo.** LZ4 nur für Dateiinhalte der Share-Ströme und für `Data`-Rahmen des SSH-Agenten
  (`native/crates/transfer/src/transfer/core/compress_policy.rs`, Modul-Kopf). Dateilisten und Pakete werden nicht
  komprimiert (TODO: LZ4 in den Paketströmen des Share fehlt; Grund: Pakete zählen Bytes je Eintrag,
  kaum Gewinn unter 2 MiB; Stand U53).
- **Wann.** Vier Bedingungen müssen zugleich gelten: (1) der Schreiber wartet in mehr als der Hälfte
  des letzten Fensters auf die Leitung; (2) ein Kern komprimiert mindestens 1,3-mal so schnell, wie
  die Leitung trägt; (3) das Verhältnis gepackt ÷ roh liegt bei höchstens 0,85; (4) ein Kern bleibt
  frei. Eine Entscheidung gilt für 8 Blöcke; das Verhältnis wird aus den ersten 64 KiB jeder Datei
  und danach aus jedem 8. Mebibyte gemessen.
- **Format.** Jeder Block ist für sich entpackbar; die Rohlänge ist höchstens 256 KiB
  (`CHUNK`, `native/crates/storage/src/agent_proto/core/types.rs:25`). Ein Block mit Rohlänge null oder über dem
  Maximum wird vor dem Entpacken abgelehnt (`native/crates/storage/src/agent_proto/core/lz4_frames.rs:45`, `:68`).
  Der Share-Rahmen trägt den Tag `TAG_DATA_LZ4` = 2 (`native/crates/engine/src/share/core/framing.rs:18`). Entpackt
  wird in einen Puffer genau dieser Länge.
- **Kredit.** Die Anfrage wird nach der Rohlänge berechnet, nicht nach der gepackten Länge
  (`credit_cost`, `encoded_cost`, Tag 67 in `CHARGED_TAGS`, `native/crates/storage/src/agent_proto/core/credit.rs`).
- **Aushandlung.** Nur in der authentisierten Sitzung, mit der Fähigkeit `lz4_v1` im Share
  (`batch_wire.rs:63`) und dem Label `lz4-v1` im Agenten (`features.rs:20`). Ohne das Label sendet der
  Client keine LZ4-Rahmen (Stand U53).
- **Schalter.** `SE_TEST_NO_LZ4` schaltet LZ4 für Messungen ab; `SE_SHARE_QUIC_CC` wählt die
  Staukontrolle, nur `bbr3` wählt BBRv3, sonst Cubic (`native/crates/engine/src/share/core/keepalive.rs:105`).
  Beide sind Messschalter, keine Einstellungen (Stand U53).
- **Windows.** `REQUEST_COMPRESSED_TRAFFIC` für Kopien mit einer Freigabe ist aus
  (`native/crates/transfer/src/copy/os/shared/copy_flags.rs:19`).

## Überlast, Fehler und Wiederholung

- **Überlast ist Gegendruck, kein Dateifehler.** Meldet eine Verbindung „zu viele Anfragen“
  (FTP-Pool voll, WebDAV 429/503, Drive-Ratenlimit, Share „busy“, Agent/Dienst ausgelastet),
  gibt der Worker seine Erlaubnis als Überlast zurück (der Flow halbiert), wartet die genannte
  Zeit (`Retry-After`, sonst 1 s ± 50 %, höchstens 60 s) ohne Erlaubnis und versucht es erneut –
  solange nichts veröffentlicht ist und die Verbindung innerhalb von 5 Minuten irgendeinen
  Fortschritt macht (`OVERLOAD_PATIENCE`, gleich in Engine, Ordneranlage und Sync). Dieselbe
  Regel gilt im Einweg-Spiegeln und im Zwei-Wege-Sync.
- **Andere vorübergehende Fehler** (Verbindungsabbruch, Timeout) werden je Datei einmal
  wiederholt, nur vor der Veröffentlichung; Downloads setzen dabei ab dem geschriebenen Stand
  fort, wo das Protokoll Offsets kann.
- **Dauerfehler am Ziel** (voll, Kontingent erschöpft, schreibgeschützt, keine Rechte) beenden
  die Übertragung mit Klartext; der Grund steht in der Fehlerliste immer an erster Stelle.
- **Leistungsschalter:** viele Verbindungsfehler in Folge ohne jeden Erfolg (mindestens 8, sonst
  doppelt so viele wie gleichzeitig laufen) beenden die Übertragung; ein verlorenes Paket zählt
  dabei einmal.
- **Pakete:** eine Datei, die sich während des Sendens ändert, wird nicht veröffentlicht;
  wiederholt werden nur Mitglieder, die sicher nicht veröffentlicht wurden (sonst „Ergebnis
  unbekannt“, nie ein Duplikat).
- **Download-Ziel hinter einem Link** (z. B. `/home` als Link, verlegter Downloads-Ordner): der
  gewählte Ordner wird einmal aufgelöst; Links werden nur in Ordnern geprüft, die die
  Übertragung selbst anlegt.
- **Verschieben** (lokal) entfernt einen Quellordner erst, wenn sein Gegenstück am Ziel existiert.

## Explorer-Übergabe (Windows)

- Eigener STA-Thread je Übergabe; er endet, wenn das letzte COM-Objekt freigegeben ist
  (`OleFlushClipboard` wird nie aufgerufen). Das Ziehen übergibt das Datenobjekt per
  `CoMarshalInterface(MSHCTX_LOCAL)`, damit Explorers Aufrufe nicht auf dem GUI-Thread laufen.
- Dateiliste erst bei Explorers erster Anfrage, einmal, zwischengespeichert; Pfade ab
  260 Zeichen, reservierte Gerätenamen, `..` und Datenströme werden ausgelassen und gemeldet.
  Kann Windows die Liste nicht aufnehmen: „Auswahl zu groß für den Explorer – bitte in Smart
  Explorer einfügen“.
- Inhalte als IStreams mit Vorausladen der nächsten Dateien (16 MiB Vorauslesepuffer,
  höchstens das halbe Speicherbudget) unter dem Flow der Verbindung;
  `IDataObjectAsyncCapability` lässt den Explorer im Hintergrund kopieren.

## Stand 2026-10-28: Tempo-Pakete KP1–KP6

Plan `docs/plaene/2026-10-28-verbinden-ist-einrichtung/plan.md`. Diese Fassung ist nicht gebaut
und nicht ausgeführt; die Messfälle (`speed_task_*`) laufen erst in der Task-Suite.

- **Absturzprotokoll je Auftrag (KP1, K-A).** Ein Auftrag mit Absturzschutz schreibt eine
  angehängte Protokolldatei (`journal.log`: 8 Byte Kennung, dann je Datensatz Länge, CRC-32 und
  Nutzlast; `engine/file_recovery_log.rs`). Ein Commit-Faden fasst die `sync_data` gleichzeitiger
  Absichten zusammen; ein Paket schreibt eine Absicht für alle Mitglieder. Ein abgerissener letzter
  Datensatz wird beim Lesen abgeschnitten. Aufträge im alten Format (`index.json` und eine Datei je
  Beleg) werden beim Öffnen umgewandelt (`engine/file_recovery_legacy.rs`). Tresoraufträge
  versiegeln jede Nutzlast. Abgeschlossene Aufträge werden nach 14 Tagen gelöscht, wenn kein Lauf
  ihre Sperre hält (`COMPLETED_RETENTION`, `engine/job_store_catalog.rs`, M21).
- **Pakete (KP1 K-E, KP3 G3-05).** Dateien bis zu einem Achtel des Paketlimits des Backends,
  höchstens 2 MiB (`SMALL_FILE_CEILING`, `native/crates/transfer/src/transfer/core/engine_policy.rs`), gehen in
  Pakete; ein Paket fasst mindestens zwei solche Dateien (`packet_bytes`). Paketgrenzen gelten für
  den gewählten Ordner selbst, sonst für seinen Elternordner. Neue Worker starten, solange die
  Flows freie Erlaubnisse haben, höchstens Grenze × 1,5 + 2 (`engine/dispatch.rs`). Heruntergeladene
  Paketdateien veröffentlicht ein eigener Faden in Ankunftsreihenfolge (`engine/publish_queue.rs`,
  K-C), mit dem Beleg, den der Strom der Gegenstelle trug.
- **Belege aus den Antworten des Hosts (KP2, K-B/K-F).** Share-Hosts mit `evidence_v1` liefern
  Größe, Zeit und Dateiidentität in PutBatch-, GetBatch-, ReadAt- und Promote-Antworten; die Engine
  fragt dann nicht mehr einzeln nach Stat und FileIdentity. `create_dirs_v1` legt bis zu 256 Ordner
  in einer Anfrage an (`CreateDirs`). Hosts ohne diese Kennzeichen behalten die Einzelanfragen. Über
  den Worker-Proxy (GUI, Android, CLI, MCP) gehen Pakete, aber noch keine Belege (`agent_proto` hat
  dafür keine Frames).
- **Einzeldatei als Paket aus einem Eintrag (KP5a, Z-P4).** Kündigt der Host `single_put_v1` an (der
  Worker als Label `.single-put-v1`), geht eine neue Datei bis 16 MiB als Paket aus einem Eintrag:
  Absicht vor dem Commit, Veröffentlichen ohne Ersetzen mit Nummerierung, eine verlorene Antwort wird
  über die Statusabfrage geklärt und nie erneut gesendet.
- **Weitere QUIC-Verbindungen (KP5a, Z-P6).** Bei `bulk_connections_v1` öffnet der Client bis zu 3
  weitere Verbindungen für Lesen, Einzelschreiben und Pakete, sobald die erste Verbindung ≥ 1 %
  Verlust zeigt oder etwa 250 MiB/s erreicht (`share/core/node_bulk.rs`). Jede durchläuft Hallo,
  Sitzungsnachweis und die Direkt-zuerst-Zulassung. Bereichs-Uploads bleiben auf der ersten.
- **Worker (KP5a, Z-P3/Z-P8).** Der Worker nimmt Anfragen auf wiederverwendeten Fäden an (leer nach
  30 s beendet, `daemon/os/shared/backend_pool.rs`) und kündigt auf dem Loopback-Sprung kein
  `lz4-v1` mehr an.
- **Lokal (KP4).** Kopien veröffentlichen unter Windows ohne `MOVEFILE_WRITE_THROUGH` (Verschieben
  und Ersetzen behalten es, M9); Worker und Linux-Bereichsfäden senken ihre Priorität nicht mehr
  (M20); `se cp` in einen vorhandenen Ordner oder mit `--force`, die MCP-Kopie „beide behalten“
  und `se mv`/MCP-Verschieben, das keine einzelne Umbenennung ist, planen den ganzen Baum und laufen
  über die Engine (`native/crates/cli/src/cli/tree_engine.rs`, M16). Die Ordnervorschau der GUI liest
  lokale Wurzeln mit einem `lstat` und weckt die GUI, sobald ihr Ergebnis steht (M12). Eine
  Ordnerreservierung über eine Stufe schreibt das „veröffentlicht“ nach dem Umbenennen nicht mehr
  dauerhaft für sich; nach einem Absturz bestätigt die gespeicherte Identität der Stufe das Ziel
  (`engine/folder_reservation.rs`, M13).
- **Server-Backends (KP3).** SFTP- und SMB-Paket-Downloads holen auf einem eigenen Faden, die Senke
  der Engine läuft außerhalb der Laufzeit (G3-01). SFTP schickt CLOSE direkt hinter das letzte
  WRITE, FSTAT/LSTAT als Beleg mit und RENAME zusammen mit LSTAT (G3-02/G3-03/G3-06). WebDAV
  verschiebt eine mit `If-None-Match: *` bestätigte Stufe ohne PROPFIND davor (G3-12; der
  Nextcloud-Chunk-Upload kam danach im Arbeitsbaum dazu, Zeile WebDAV oben). Google Drive
  drosselt neue Dateien erst nach einer Ablehnung (G3-08a). FTP kann die Auflistungen der Ordner,
  in die ein Auftrag veröffentlicht, behalten, solange der Auftrag läuft (`vfs::transfer_scope`,
  G3-04b); die Engine hält den Rahmen, das FTP-Backend gibt ihn noch nicht zurück (offen). Jede
  Veröffentlichung prüft den Zielnamen weiterhin einzeln.
- **Unverändert:** Ein Einzel-Download prüft die Quelle nach dem Lesen weiter mit einem `stat`,
  auch wenn der Host beim Öffnen einen Beleg lieferte: Der Beleg stammt von vor dem Lesen und
  beweist nicht, dass die Datei während des Lesens gleich blieb.

## Stand 2026-10-28: Job-Dienst des Workers, Pakete und Server-Backends

Plan `docs/plaene/2026-10-28-verbinden-ist-einrichtung/plan.md`. Nicht gebaut und nicht ausgeführt;
die Prüfungen laufen erst in der Task-Suite `speed-setup-task.yml`.

- **Job-Dienst des Workers (Z-P2).** Desktop-App, `se cp/get/put`, `se mcp` und se Desktop starten
  jeden Auftrag über `run_view`. Ist jede Remote-Seite ein Share-Gerät, das dieser Prozess über den
  Worker geöffnet hat, und kündigt der Worker in `Transfer{Hello}` den Dienst `transfer-jobs-v1` an,
  läuft der Auftrag im Worker (`daemon/os/shared/transfer_job_ipc.rs`, Client
  `transfer_job_client.rs`, Leitungsformat `transfer_job_wire.rs`). Fortschritt, Hinweise und die
  Endmeldung kommen über dieselbe Verbindung zurück, Abbrechen geht als eigene IPC-Anfrage. Der
  Worker nimmt einen Auftrag nur an, wenn der verbundene Socket genau seinem Konto gehört; eine
  Auftrags-ID läuft höchstens einmal. Ein Auftrag mit Absturzprotokoll läuft weiter, wenn der Client
  verschwindet; jeder andere endet mit seinem Client.
  **Rückfall auf den Proxy-Weg** (Bytes über den Loopback-Proxy des Workers wie bisher): ein Worker
  ohne den Dienst, eine Ablehnung `transfer_not_here` (anderes Konto, unklare Identität), ein Auftrag,
  der nicht in eine IPC-Zeile passt, und eine gemischte Seite (Share und SFTP). Nach `Started` gibt es
  keinen Rückfall: Eine verlorene Startantwort wird abgebrochen und gemeldet, eine verlorene oder
  60 s stumme Verbindung (der Worker sendet alle 5 s ein Lebenszeichen) wird gemeldet und verweist
  auf „Fehlende übertragen“. Android öffnet Share-Geräte mit dem Host des eingebetteten Workers im
  selben Prozess (`mobile/os/shared/pool.rs`), solange dieser noch nicht bedient, über IPC.
- **Identitätsregister (`engine/stored_identity.rs`).** Direkt erreichte Share-Backends tragen in
  Belegen, Ordneraliasen und Wiederanlauf die Identität der App, `Peer:<label>`, wie der Proxy-Weg.
  So binden die Wiederanlaufliste der GUI und „Fehlende übertragen“ Aufträge aus Worker, Android und
  App gleich.
- **Pakete entfernt → entfernt (G3-13).** Zwischen zwei Verbindungen, die beide Pakete können,
  holt ein Paket kleine Dateien mit einem `get_batch` in reservierten Speicher und gibt sie mit
  einem `put_batch` ab (`engine/batch_remote.rs`, `batch_remote_gather.rs`); die Grenzen sind das
  Kleinere beider Seiten. Ein Mitglied erreicht das Ziel nur mit genau den gelisteten Bytes (und dem
  gelisteten MD5); eine geänderte Quelle geht allein mit ihrer neuen Länge. Innerhalb eines Kontos
  kopiert der Server: Ein Paket fragt bis zu 64 Serverkopien in private Stufen mit einem Aufruf an
  (`vfs::server_copy_batch`, `engine/batch_remote_server.rs`); jedes Mitglied wird danach wie eine
  einzelne Serverkopie behandelt (Länge, Quelle noch die gelistete, Veröffentlichen ohne Ersetzen),
  eines, das der Server nicht kopierte, geht allein. Kopiert der Server keines eines Pakets, bildet
  der Auftrag keine weiteren solchen Pakete.
- **Lokale Pakete (M3, R4).** Kleine neue lokale Kopien (bis 1 MiB, höchstens 64 Dateien und
  64 MiB) eines Auftrags mit Absturzprotokoll, unter „überspringen“ oder „beide behalten“, teilen
  eine Erlaubnis und einen Absichtsdatensatz (`engine/local_packet.rs`). Die Mitglieder, deren
  gewünschter Name frei ist, werden in Gruppen zu höchstens 16 gestuft
  (`engine/local_packet_stage.rs`, `copy/os/shared/packet_stage.rs`); der eine Datensatz nennt danach
  je Mitglied Quelle, Stufe und Ziel (`PacketIntent`, `engine/file_recovery_packet.rs`), erst dann
  wird veröffentlicht, und die Bestätigung vergleicht über den noch offenen Handle der Stufe. Ein
  Absturz nach dem Datensatz hinterlässt nur Stufen, die der wiederaufgenommene Auftrag kennt und
  entfernt; ein Absturz während eine Gruppe gestuft wird, hinterlässt deren Stufen (wie eine einzelne
  Datei ihre eine zwischen Stufe und Absicht). Mitglieder, die nummeriert werden müssen, gehen danach
  einzeln mit eigener Absicht.
- **Bereichsfäden für dauerhafte Kopien (M15).** Verschieben auf ein anderes Volume und Ersetzen
  kopieren mit den Bereichsfäden und der Messung, die die Engine vergibt; die Stufe wird danach als
  Ganzes synchronisiert und erst dann veröffentlicht. Die Engine gibt einem Verschieben nur dann
  weitere Fäden, wenn beide Seiten auf bekannten Blockgeräten mit verschiedener Gerätenummer liegen;
  Windows kopiert dauerhaft weiter über `pipe_copy` mit einem Faden.
- **Server-Backends.** Google Drive: neue Datei mit einem Schreibaufruf (G3-08b, Zeile Drive oben).
  Nextcloud: große Dateien als Chunk-Upload (G3-12, Zeile WebDAV oben). SMB: Lesen öffnet mit dem
  ersten READ in einem Compound, eine Engine-Stufe bis MaxWriteSize schließt mit WRITE + CLOSE in
  einem Compound ab (G3-07, Zeile SMB oben). SFTP: Pakete aus Serverkopien (Zeile SFTP oben).

## Dateiregel „Neuere ersetzen ältere“ (Quellstand 2026-10-31, nicht gebaut, nicht veröffentlicht)

Auftrag des Nutzers (2026-10-31): beim Einfügen mit „Zusammenführen“ werden fehlende Dateien
eingefügt, neuere (nach „zuletzt bearbeitet“) ersetzen ältere, gleiche werden übersprungen;
gemeint sind alle Dateioperationen dieser Art. Plan: `docs/plaene/2026-10-31-sync-abgleich-schnell/`
(C7, D4); Vergleich mit rsync, rclone, robocopy, FreeFileSync, Explorer: dort `referenzen-abgleich.md`
Abschnitte 3.12, 4, 6.

**Regel je gleichnamiger Datei** (`Conflict::Update`, `native/crates/core/src/types/core/types.rs:133`; reine
Entscheidung `transfer/core/update_rule.rs:50`):

| Ziel | Ergebnis |
|---|---|
| fehlt | eingefügt |
| gleiche Größe, Zeitabstand höchstens das Fenster | gleich, übersprungen |
| gleiche Größe, Zeitabstand genau eine Stunde (± Fenster), eine Seite speichert Ortszeit (FAT, `MtimePrecision::local_time_shifts`) und beide Seiten zeigen mit am Ort der Datei gebildeten Prüfschlüsseln denselben Inhalt | gleich, übersprungen (Sommerzeitumstellung, `transfer/core/time_window.rs` `dst_shift_equal`, `hour_proof.rs` `same_content_where_stored`; die Vorschau zählt ebenso; Stand 2026-10-31) |
| wie zuvor, aber anderer Inhalt oder eine Seite kann am Ort der Datei keinen Prüfschlüssel bilden | die Zeit entscheidet wie ohne Regel: eine Stunde neuer ersetzt, eine Stunde älter bleibt |
| Quelle um mehr als das Fenster neuer (jede Größe) | ersetzt das Ziel |
| Ziel um mehr als das Fenster neuer (jede Größe) | Ziel bleibt, „Ziel neuer“ |
| Zeit im Fenster, andere Größe | Konflikt: beide behalten (`Name (2)`) |
| Ordner, Link, Sonderdatei oder mehrere Einträge unter dem Namen | beide behalten |
| Zeit einer Seite unbekannt (0) | Konflikt, beide behalten – auch bei gleicher Größe (Größe allein beweist keine Gleichheit; eine geänderte Quelle gleicher Größe muss ankommen oder daneben liegen) |
| Ziel inzwischen anders als bei der Entscheidung (Größe oder Zeit außerhalb des Fensters, jetzt Ordner oder Link) | beide behalten, siehe „Ersetzen“ |
| Name, den der Host beim Auflisten ausgelassen hat (nicht lesbar, nicht darstellbar) | beide behalten; die übrigen Dateien des Ordners laufen normal |

Das **Fenster** ist die gröbere Zeitauflösung beider Seiten (`vfs::mtime_precision` je Seite: lokale
Dateisystemprofile, SFTP/FTP/WebDAV nach Protokoll); weiß eine Seite ihre Auflösung nicht, zählt sie
mit 2 s (`update_rule.rs:44`, `engine/update.rs:151`). Das Fenster wird je Auftrag einmal aus dem
Zielordner und dem Quellordner bestimmt, nicht je Unterordner.

**Wer die Regel anwendet:** alle Dateien eines Auftrags mit Dateiregel `Update` (Kopierdialog, CLI,
MCP, Android-Auswahl) und, mit `FolderConflictPolicy::merge_update`
(`transfer/core/folder_conflict.rs:28`), die Dateien unterhalb eines ausgewählten Ordners, für den
„Zusammenführen“ gewählt wurde. Einfügen, Ziehen und „Herunterladen nach…“ setzen das Kennzeichen
(`app/core/transfer_route.rs:127`, Android `mergeFiles: "update"`); eine einzeln ausgewählte Datei
und der Kopierdialog mit ausdrücklicher Wahl behalten ihre Regel.

**Wie sie wirkt:** Die Engine entscheidet **einmal je Datei, bevor ein Kopierweg läuft**
(`engine/update.rs:166`, aufgerufen im Worker vor der Paketbildung `worker.rs:52`, für Paketkandidaten
`batch.rs:82` und für Einzeldateien nach der Belegprüfung `ops.rs:291`) und speichert das Ergebnis in
der Datei (`FileWork::conflict`): „einfügen“ und „Konflikt“ laufen als `Rename`, „ersetzen“ als
`Overwrite`, gleiche und neuere Ziele werden gezählt und nicht übertragen. Eine Datei, für die ein
gespeicherter Auftrag einen Beleg schrieb, entscheidet zuerst der Beleg (`file_recovery::resume`); gibt er
kein Ergebnis, entscheidet danach die Regel (`update::settle_recovered`, `ops.rs`), nicht die
Auftragsregel „beide behalten“. Die neuen Elternordner der Paketkandidaten werden gesammelt angelegt
(`update::ensure_parents` → `FolderRegister::ensure_many`, `batch.rs` `admit`), nicht je Kandidat. Darunter sieht jeder Weg
nur die bekannten Regeln und behält deren Schutz: lokal und Download ersetzen über Stufe,
`replace_file_atomic` und Haltepunkt wie das Überschreiben bisher (`local.rs:55`, `download.rs:316`);
lokale Pakete nehmen nur neue Namen, eine Ersetzung geht allein (`batch.rs:82`).

**Das Ziel wird gelesen:** lokal mit einem `stat`, entfernt mit einer **toleranten** Auflistung je
Zielordner (`vfs::list_dir_tolerant`), im Auftrag gehalten, begrenzt auf 16 MiB Namen
(`engine/update_listing.rs`); in einem Ordner, den der Auftrag selbst anlegte, gar nicht. Ein Name, den der
Host als Auslassung meldet (Share-Host: nicht lesbar, nicht darstellbar), gilt als vorhanden und
unbekannt: die Datei dieses Namens behält beide, die übrigen Dateien des Ordners laufen wie sonst. Ein Ziel, das nicht gelesen werden kann, lässt die Datei allein laufen, wo der
gewöhnliche Weg den Fehler meldet und wiederholt.

**Ordnerschlüssel beim Zusammenführen** (Quellstand 2026-10-31, nicht gebaut; `engine/update_keys.rs`):
Hat ein Quellordner denselben Ordnerschlüssel wie der Zielordner gleichen Namens, gilt er für die Regel als
gleich: das Ziel wird nicht aufgelistet, keine Datei darin verglichen, alle Dateien zählen als „gleich übersprungen“
(`Decision::Equal`, Fortschritt wie bei einer Einzelentscheidung), und die Ordner darin werden auch nicht
angelegt (`update::folder_equal` in `worker.rs`; ein Ordner mit gleichem Schlüssel besteht samt Inhalt).
- *Schlüssel dort bilden, wo die Dateien liegen:* eine lokale Seite im eigenen Prozess mit dem Durchlauf des
  Agenten (`agent_proto` `LocalKeyed::keys_of`, ohne die Läufe zu behalten), eine entfernte Seite über den
  Schlüssel-Durchlauf ihres Hosts (`vfs::hash_walk_keyed` mit `KeyedWalk::KeysOnly`: Share `hash_walk_keys_v1`,
  Agent `walk-keys-v1`, Hintergrunddienst für ein Backend mit eigenem Durchlauf); über die Verbindung geht
  je Ordner ein Schlüssel, kein Eintrag. Quelle und Ziel werden gleichzeitig durchlaufen.
- *Gleichheit:* der Schlüssel deckt je Datei Name, Größe und Änderungszeit **in Millisekunden**
  (`agent_proto/core/folder_key.rs`), dazu Art, versteckt und Auslassungen. Gleicher Schlüssel bedeutet damit
  gleiche Größe und Zeitabstand 0, also „gleich“ für jedes Fenster der Regel. Ungleiche Schlüssel bedeuten nur
  „hineinsehen“: diese Ordner nehmen den Weg ohne Schlüssel (Auflistung des Zielordners, Entscheidung je Datei),
  die Unterordner mit gleichem Schlüssel darin werden trotzdem ausgelassen (Abstieg nach Schlüsseln wie im
  Sync). Ergebnis (Kopien, Ersetzungen, Zählwerte, Meldungen) gleich dem Weg ohne Schlüssel.
- *Auch die Quelle wird nicht aufgelistet:* der Schlüssel-Durchlauf `KeysOnly` liefert je Ordner zusätzlich,
  was sein Teilbaum enthält (`vfs::KeyTally`, `vfs/core/key_tally.rs`): Zahl der Dateien und ihre Bytes, Zahl der
  Auslassungen (Links, Spezialdateien, nicht lesbare Einträge, der eigene Lesefehler eines Ordners und Namen, die
  keine Übertragung nimmt: Backslash, NUL, `.`/`..`, der Papierkorbordner der App), Zahl der Dateien mit unbekannter
  Zeit (0) und die Tiefe darunter. Quelle des Zählens: `RunKeys::entry/omission/failure`
  (`agent_proto/core/folder_key.rs`), summiert über die Unterordner in `KeyTree::completed`
  (`folder_key_tree.rs`); am Draht ein `FolderKey` mit Merkmalsbit 2 und fünf LEB128-Zahlen
  (`walk_key_frames.rs`), bei Share `Key.tally` (`hash_walk_wire.rs`), lokal `LocalKeyed::keys_of`, über den
  Hintergrunddienst unverändert weitergereicht (`backend_hash_keyed.rs`). Die Fähigkeiten `walk-keys-v1` und
  `hash_walk_keys_v1` sind unveröffentlicht und tragen das Feld ohne neue Bezeichnung; eine Gegenstelle ohne
  Zählung (`tally = None`) verhält sich wie bisher (Quelle wird aufgelistet).
  Ein Ordner mit gleichem Schlüssel **und** sauberer Zählung (keine Auslassung, keine Datei mit Zeit 0, Tiefe
  im Rahmen der Auflistung, `Keys::settled`) ist erledigt, bevor er aufgelistet wird: der Durchlauf
  (`walk.rs`, `WalkOptions::settled`, `walk_settled.rs`) fragt vor dem Auflisten jedes Ordners
  `Engine::settled_folder` und meldet ihn als ein `WalkEvent::Equal` statt `Dir` und Inhalt. Der Fortschritt
  zählt seine Dateien daraus wie je Datei einzeln (`Stats::equal_folder`: gefunden, gleich übersprungen
  `skipped_equal`, `skipped`, Bytes gutgeschrieben), ohne irgendeine anzusehen. Eine Auslassung oder eine Datei mit
  Zeit 0 im Teilbaum führt zum bisherigen Weg für genau diesen Teilbaum: der Ordner wird aufgelistet, der Durchlauf
  meldet die Auslassung mit ihrem Pfad (`WalkEvent::Problem`), die Datei mit Zeit 0 behält beide; Unterordner
  darin ohne Auslassung können für sich erledigt sein. Nicht angewandt bei Filter, Layout `Flatten`, wiederaufgenommenem
  Auftrag (Belege je Datei) und Kopie innerhalb eines Namensraums (der Durchlauf schließt dort das eigene Ziel
  aus).
- *Ausnahme Zeit 0:* eine Quelldatei mit unbekannter Zeit (0) nimmt nie den Schlüsselweg; die Regel behält dort
  beide (Tabelle oben). Die Vorschau listet das Ziel eines Quellordners, in dem eine solche Datei liegt, wie bisher auf.
- *Wann Schlüssel gelten:* nur bei **entferntem Ziel** (Upload, Remote–Remote), bei Dateiregel `Update` oder
  `merge_update`, Layout `Tree` und einem Quellordner mit Entscheidung „Zusammenführen“. Ein lokales Ziel kostet ein
  `stat` je Datei – weniger als beide Bäume zu durchlaufen und zu hashen. Gegenstellen ohne Schlüssel-Durchlauf
  (ältere Hosts, SFTP ohne Agent, FTP, WebDAV, SMB, Drive) und fehlgeschlagene oder abgebrochene Durchläufe: Weg
  ohne Schlüssel.
- *Größe des Zielbaums:* ist der Zielbaum viel größer als der zusammengeführte Ordner, wäre die Auflistung je
  Quellordner billiger als der Durchlauf. Der Schlüssel-Durchlauf des Ziels wird deshalb abgebrochen, sobald er
  mehr als 16 Ordner je Quellordner plus 1024 gemeldet hat (Annahme zu den Kosten: eine Auflistung ist eine
  Rundreise, das Lesen eines Ordners neben den Daten nicht; `FOLDERS_PER_SOURCE_FOLDER`, `FOLDERS_SLACK`). Das
  wählt nur den Weg, nie das Ergebnis.
- *Zeitpunkt:* die Schlüssel werden vor der ersten Entscheidung unter dem Quellordner gebildet, vor jeder Änderung
  des Auftrags am Ziel; eine Datei, die sich zwischen Schlüssel und Entscheidung ändert, hat dasselbe Fenster wie
  die Auflistung des Ziels.

**Änderungszeit der Kopie:** Jede Datei, die die Engine kopiert oder ersetzt, trägt die Änderungszeit
ihrer Quelle (Entscheidung des Koordinators 2026-10-31, `plan.md` C7: ohne sie ersetzte ein
Zusammenführen zurück jede Kopie; Windows `CopyFile2` tat es schon), gesetzt vor der Veröffentlichung auf
Stufe oder Teildatei (kein Name zeigt je eine falsche Zeit; Rechteprüfung und No-Replace-Veröffentlichung
bleiben). Das kostet **keine Rundreise je Datei gegenüber v0.5.189**, wo das Protokoll die Zeit in einem
ohnehin gesendeten Schritt tragen kann (Reihenfolge der Wege unten); nur wo es keinen solchen Schritt gibt,
folgt eine eigene Anfrage (Grenzen). Windows lokal: `CopyFile2` (`copy/os/
windows_time_rules.rs`, unverändert). Linux und Android lokal, lokale Pakete und Serverkopien: auf dem
offenen Stufenhandle neben den Rechten (`copy/os/linux_os.rs:239`, `File::set_modified`; ein Dateisystem,
das sie ablehnt, behält die Kopierzeit, kein Fehler). Download und Download-Pakete: auf der fertigen
Teildatei vor dem Schließen (`engine/download.rs:109`, `:318`). Entfernte Ziele über Stufen (Upload,
Remote–Remote, Serverkopie, Bereiche, Stufen der Remote–Remote-Pakete), in dieser Reihenfolge
(`engine/publish_time.rs`):
1. im Schreibschritt der Stufe, ohne Anfrage: WebDAV (`X-OC-Mtime`) und Drive (`modifiedTime`) beim
   Hochladen (`open_write_copy_stage_timed`); SFTP als `FSETSTAT` auf dem offenen Handle, gepipelinet hinter
   dem letzten WRITE und vor FSTAT und CLOSE (`sftp/core/pool_writer_stamp.rs`); SMB als SET_INFO im
   Abschluss-Compound einer Stufe, die in ein WRITE passt (`smb/core/compound_put.rs`). Der FSTAT/das CLOSE
   dahinter melden die Zeit schon als Stufenbeleg; der Beleg zeigt der Engine, dass die Stufe die Zeit trägt
   (`written_with_time`), und bleibt gültig (Empfänger-Belege ohne eigene Anfrage);
2. in Paketen im Paket selbst (`BatchPut::mtime_ms`): Share (`batch_put_mtime_v1`), Agent
   (`batch-mtime-v1`), SFTP (`FSETSTAT` hinter den WRITEs, vor dem CLOSE jeder Datei,
   `sftp/core/batch_put.rs`) und SMB (SET_INFO im Compound CREATE+WRITE+CLOSE, `smb/core/batch.rs`);
3. sonst `vfs::finish_stage` mit `StageFinish::mtime_ms` vor `publish::publish` (eigene Anfrage[n]). Was ein
   Peer vorher über die Stufe gemeldet hat, wird dann verworfen, wenn das Setzen die Zeit änderte (die
   Belege lesen die Stufe selbst). Ersetzungen machen die Stufe dabei zusätzlich dauerhaft
   (`StageDurability::Now`, deshalb immer dieser Weg); scheitert das, wird die Stufe verworfen und die Datei
   gemeldet.

**Ersetzen:** neu. Die Entscheidung „Quelle neuer“ merkt sich, was sie vom Ziel sah (`Replacing`:
Größe, Zeit, Fenster, Objekt-ID des Anbieters). **Unmittelbar vor der Veröffentlichung, die das
vorhandene Ziel ersetzt, wird das Ziel noch einmal gegen diesen Stand verglichen** (Größe gleich, Zeit
im Fenster, noch eine reguläre Datei); weicht es ab – jemand hat es inzwischen neu gespeichert –, wird
nicht ersetzt, sondern beides behalten. Entfernt: ein `stat` des Ziels in `still_decided` (`engine/publish_replace.rs`) vor
`promote_staged_to_id` (auch für Ziele ohne Objekt-ID wie SFTP, SMB, FTP, WebDAV, Direct Share, SSH-Agent,
deren Promotion allein `None == None` verglich); die Datei geht in eine dauerhafte Stufe
(`open_write_copy_stage_sized`) und wird mit `promote_staged_to_id` an das gesehene Objekt gebunden
veröffentlicht (`publish.rs`). Lokal: `copy::transfer_local_replacing` vergleicht das Ziel in
`local_publish::publish` (und in `direct_move::try_move` bei Verschieben mit Ersetzen) unmittelbar vor
dem Ersetzen; Download: `download::publish_local_known` vor `replace_file_atomic`. Kann das Ziel das nicht
(`Unsupported`, auch ein nicht lesbares Ziel), hat sich die Datei geändert (`InvalidData`) oder ist
verschwunden (`NotFound`), bleibt beides: die Stufe wird unter `Name (2)` veröffentlicht, im Protokoll
steht der Grund, gezählt als „doppelt behalten“, nicht als ersetzt (`publish.rs`,
`Stats::replace_refused`). Das Fenster zwischen diesem Vergleich und dem Umbenennen bleibt (ein Aufruf
des Anbieters, nicht atomar, außer wo die Objekt-ID bindet: Drive, Tresor). Namen oberster Ebene werden unter der Regel nicht
vorab nummeriert (`roots.rs:325`, `roots.rs:383`), damit ein belegter Name verglichen werden kann.

**Vorschau „Zielordner entscheiden“** (`engine/selection_count.rs`): zählt neu, neuer, gleich,
Ziel neuer und Konflikt (`FolderConflictPreview::{new,newer,equal,target_newer,conflict}_files`;
`colliding_files` bleibt die Summe der gleichnamigen). Das Ziel wird je Ordner einmal **tolerant**
aufgelistet (`vfs::list_dir_tolerant`), nicht je Datei mit `stat`; ein ausgelassener Name zählt als
Konflikt, die übrigen Dateien des Ordners werden gezählt; die Ordner werden von so vielen Fäden gezählt,
wie die langsamere Seite zulässt (`Backend::parallelism`); Meldungen kommen nur vom aufrufenden Faden.
Die fünf Zählwerte beschreiben die Regel und erscheinen nur, wenn der Auftrag sie anwendet
(`TransferJob::applies_update_rule`: `merge_update` oder Dateiregel `Update`): Desktop
(`app/core/folder_conflict_ui.rs`), Android (`fs.conflicts`/`fs.folderPreview`: Feld `rule`, Dialog
`FileDialogs.kt`; die Vorschau bekommt dasselbe `mergeFiles: "update"` wie die Übertragung). Sonst
zeigen beide „neue und gleichnamige Dateien“ wie vor der Regel; die Auftragsregel (überschreiben,
beide behalten, überspringen) gilt dann weiter. Ohne bekannte Zeit einer Seite zählt eine gleichnamige
Datei als Konflikt, nicht als gleich (dieselbe `classify` wie die Engine). Mit Ordnerschlüsseln (oben)
wird das Ziel eines Quellordners mit gleichem Schlüssel nicht aufgelistet und seine Dateien zählen als gleich
(`engine/selection_count_target.rs`, `Target::Equal`). Ist auch der Teilbaum sauber (keine Auslassung, keine
Datei mit Zeit 0, `Keys::settled`), wird der Quellordner ebenfalls nicht aufgelistet: seine Dateizahl kommt aus der
Zählung des Schlüssel-Durchlaufs und zählt als gleich (`settled_by_key`); nur die Beispielnamen (höchstens fünf)
kommen dann aus anderen Ordnern. Bei einem Filter wird die Quelle wie bisher aufgelistet. Die fünf Zählwerte,
`colliding_files` und Auslassungen sind dieselben wie ohne Schlüssel; die Grenzen für Einträge und Pfadtext der
Vorschau greifen für einen so erledigten Ordner nicht, weil nichts von ihm gelesen wird.

**Meldungen nach dem Lauf:** `TransferProgress::{replaced, skipped_equal, kept_newer, kept_both}`
(Standardwert 0 beim Lesen älterer Meldungen); Übertragungsfenster (`app/core/transfer_rows.rs:159`,
Hinweis `:211`), CLI-Zeile `se: update: …` auf stderr, MCP-Ergebnis `update`, Android-Ergebnis
`replaced/skippedEqual/keptNewer/keptBoth` und Hinweiszeile. Übersprungene gleiche und neuere Ziele
zählen als bearbeitet, damit der Fortschritt sein Ende erreicht.

**Gespeicherte Aufträge und ältere Versionen:** `JobRecord::file_update` (`#[serde(default)]`) hält die
Regel, `file_policy` bleibt dabei 0 – eine ältere Version öffnet den Auftrag als „beide behalten“ und
ersetzt nie etwas. Dasselbe gilt für Aufträge mit `folders.merge_update`, die eine ältere Version nicht
liest: `file_policy` ist dann **immer 0** und die Wahl der obersten Ebene (beide behalten, überschreiben,
überspringen, Regel) steht im eigenen Feld `JobRecord::file_choice` (0–3, `#[serde(default)]`, nur mit
`merge_update` geschrieben); die neue Version liest beide Formen (`job_record.rs` `file_rule`,
`stored_conflict`). Eine ältere Version setzt einen solchen Auftrag also nie als „überschreiben“ oder
„überspringen“ der zusammengeführten Ordner fort. `merge_update` wird nur geschrieben, wenn es gesetzt ist. Zwischen App und Worker: Dateiregelcode 3 und die Fähigkeit `transfer-update-v1`
(`daemon/os/shared/transfer_job_wire.rs:26`); ein Worker ohne sie bekommt den Auftrag nicht, der
Auftrag läuft dann im eigenen Prozess (`transfer_job_client.rs:155`).

**Grenzen (nicht gemessen, nicht gebaut):**
- **Pakete an entfernte Ziele tragen die Änderungszeit, wo die Gegenstelle sie setzt** (Quellstand
  2026-10-31, nicht gebaut): `vfs::BatchPut::mtime_ms` nennt je Datei die Quellzeit (`batch_put.rs`,
  `batch_put_one.rs`, `batch_remote.rs`); der Share-Host setzt sie mit `finish_stage` auf die Stufe vor
  der Veröffentlichung (Fähigkeit `batch_put_mtime_v1`, Feld `FsBatchPut.mtime_ms`,
  `share/core/server_batch_stage.rs`), der SSH-Agent mit `set_modified` (Etikett `batch-mtime-v1`,
  `BatchPut` mit Zeiten als Tag 81, `agent_proto/os/shared/batch_put.rs`), der Worker meldet das
  Etikett nur für einen Peer, der die Zeit setzt (`backend_server.rs`). Abfrage: `vfs::batch_put_times`.
  **SFTP und SMB tragen die Zeit im Paket** (immer, Fähigkeit `batch_put_times`): SFTP als `FSETSTAT`
  auf dem offenen Handle zwischen den WRITEs und dem CLOSE jeder Datei (`stage_written`, drei Anfragen in
  dieser Reihenfolge gesendet, keine Rundreise mehr), SMB als SET_INFO im Compound
  CREATE+WRITE(+SET_INFO)+CLOSE; passt eine SMB-Datei nicht in den ersten Block, geht SET_INFO+CLOSE in einem
  Compound. Neue Dateien der Regel gehen damit wieder in Pakete und Ein-Datei-Pakete (`update.rs`
  `packets_keep_time`, `upload.rs`); nur an **Gegenstellen ohne die Fähigkeit** (ältere Share-Hosts und
  Agenten) gehen sie einzeln – das bleibt die Grenze –, und dort behalten andere Paket-Uploads die Zeit
  der Gegenstelle wie zuvor. Eine Ersetzung geht immer allein.
- **Share-Host und SSH-Agent tragen die Zeit im Schreibschritt** auch für Dateien, die nicht in ein Paket
  gehen: Share `stage_mtime_v1` (`WriteNew.mtime_ms`, der Host setzt sie beim Abschluss vor dem Stufenbeleg,
  `share/core/server_transfer.rs`), Agent `stage-mtime-v1` (`WriteNewTimed`, Antwort `StageWritten` mit
  Größe, Zeit und Dateikennung als Stufenbeleg); ältere Gegenstellen und der Hintergrunddienst behalten
  `finish_stage`.
- **Wo das Protokoll die Zeit nur mit einer eigenen Anfrage setzen lässt**, folgt `finish_stage` nach dem
  Schreiben (Rundreisen mehr je Datei gegenüber v0.5.189): FTP (`MFMT`), SMB-Stufen größer als ein WRITE,
  SFTP ohne Pool-Kanal, Share-Hosts und Agenten ohne `stage_mtime_v1`/`stage-mtime-v1` und Ersetzungen
  (dauerhafte Stufe). Der Stufenbeleg der Gegenstelle entfällt dort, wenn das Setzen die Zeit änderte.
  Ein Ziel, das die Zeit nicht speichern kann, behält seine eigene (kein Fehler des Eintrags). Eine
  Drive-Datei, die ohne Stufe entsteht (in einem vom Auftrag angelegten Ordner, `open_write_fresh`, oder unter dem
  Endnamen, `open_write_absent`), trägt die Zeit seit 2026-10-31 schon in der Anlegeanfrage (`modifiedTime`): die
  Engine bietet sie dem Schreiber an, während sie ihn öffnet (`transfer/core/creation_time.rs`,
  `engine/publish.rs` `open`; Annahme: der Aufruf läuft auf dem Faden der Engine, ein Schreiber liest das Angebot
  nur beim Öffnen), `gdrive/core/sized_writer.rs` übernimmt es in `NewObject::mtime_ms`; keine Anfrage mehr.
- Ein Ziel, das keine Zeit speichern kann (`finish_stage` meldet `mtime_applied: false`), behält
  seine eigene; die Regel vergleicht dann wie zuvor.
- Ein Konflikt (gleiche Zeit, andere Größe) legt bei jedem Lauf eine weitere nummerierte Kopie an.
- Sommerzeit (seit 2026-10-31): speichert eine Seite Ortszeit (laut `fs_profile` FAT/VFAT, Zeitauflösung zwei
  Sekunden; exFAT meldet zehn Millisekunden und zählt nicht dazu), gelten Dateien gleicher Größe mit einem
  Zeitabstand von genau 3600 s ± Fenster nur dann als gleich, wenn beide Seiten am Ort der Datei denselben
  Prüfschlüssel bilden (`classify_local_time` mit `time_window::same_content_where_stored`: SHA-256 über
  `vfs::content_digest`, sonst MD5 über `vfs::content_md5` auf beiden Seiten; lokal gehasht, entfernt vom
  Server, se-agent, Share-Host, FTP `HASH` oder Drive; WebDAV-`oc:checksums` ist die Angabe eines Hochladenden
  und zählt nicht; zuerst die entfernte Seite, damit eine Seite ohne Prüfschlüssel die lokale nicht lesen
  lässt). Ohne Prüfschlüssel oder bei anderem Inhalt gilt die Zeit wie ohne Regel (eine Stunde neuer ersetzt).
  Gefragt wird nur in diesem Fall (`engine/update.rs` `hour_proof`, Vorschau `selection_count.rs` `hour_proof`).
- Gleichheit ist Größe plus Zeit im Fenster; nur die Stunde der Sommerzeit verlangt zusätzlich gleichen Inhalt. Mit Ordnerschlüsseln ist sie Größe plus Zeit in Millisekunden
  (strenger, nie anders).
- Ordnerschlüssel: ein Quellordner mit gleichem Schlüssel und sauberer Zählung (keine Auslassung, keine Datei
  mit Zeit 0) wird weder am Ziel noch an der Quelle aufgelistet; Dateizahl und Bytes für den Fortschritt kommen
  aus der Zählung des Schlüssel-Durchlaufs. Ein Teilbaum mit Auslassung oder Zeit 0 wird aufgelistet wie
  ohne Schlüssel (Meldung mit Pfad, beide behalten). Bei einem lokalen Ziel gelten Schlüssel nicht (siehe
  oben). Der Schlüssel enthält keinen Inhalt der Dateien, nur ihre Größe und Zeit.
- `se mv --conflict update` und der MCP `move` mit `file_conflict: update` nur lokal; gleiche Dateien
  bleiben in der Quelle, der Quellordner bleibt dann stehen.

## Sync: Ordnerschlüssel, Prüfschlüssel und Zeitvergleich (Quellstand 2026-10-31, nicht gebaut, nicht veröffentlicht)

Plan: `docs/plaene/2026-10-31-sync-abgleich-schnell/plan.md` Abschnitt E (E8–E11, Validierung mit
Primärquellen). Oberflächen und Einstellungen bleiben unverändert; „Vergleichen“ und Sync-Läufe nutzen
den Weg selbst, das Lauf-Protokoll nennt je Seite „… Ordner gleich ihrem Ordnerschlüssel (nicht
übertragen), … Ordner übertragen“.

- **Zählung am Schlüssel.** Der Schlüssel-Durchlauf `KeysOnly` sendet je Ordner zusätzlich die Zählung
  seines Teilbaums (`vfs::KeyTally`: Dateien, Bytes, Auslassungen, Dateien mit Zeit 0, Tiefe); das
  Zusammenführen unter der Dateiregel (Abschnitt „Ordnerschlüssel beim Zusammenführen“) listet damit auch
  die Quelle eines gleichen Ordners nicht auf. Der Sync selbst braucht die Zählung nicht: er vergleicht
  Schlüssel und Läufe.
- **Ordnerschlüssel (E10).** Share-Host, se-agent und der Hintergrunddienst (wenn sein Backend einen
  eigenen Durchlauf hat) bilden beim Durchlauf für jeden Ordner einen SHA-256-Schlüssel über die
  Angaben, die der Vergleich aus ihm nimmt, und die Schlüssel seiner Unterordner
  (`native/crates/storage/src/agent_proto/core/folder_key.rs`). Der Client bietet die Schlüssel an, die diese und die
  andere Seite des Paars beim letzten Durchlauf gespeichert haben; ein Ordner mit bekanntem Schlüssel
  kommt als eine Marke statt seines Teilbaums, seine Einträge kommen aus der gespeicherten Datei
  (`native/crates/transfer/src/bisync/os/shared/snapshot_keyed.rs`). Unveränderte Seite: eine Marke für die Wurzel,
  keine Auflistung, kein Eintrag über die Verbindung. Geänderte Datei: nur die Ordner auf ihrem Pfad.
  Der Schnappschuss, und damit Aktionen, Konflikte, Auslassungen, Basis und Löschschutz, ist derselbe
  wie ohne Schlüssel. Ältere Gegenstellen: bisheriger Durchlauf. Erstvergleich ohne gespeicherte
  Schlüssel: beide Seiten senden zuerst nur ihre Schlüssel, gleichzeitig; danach jede Seite nur die
  Läufe der Ordner, deren Inhalt noch niemand hält, jeden ungleichen Inhalt einmal
  (`snapshot_pair_keys.rs`). Eine lokale Seite bildet ihre Schlüssel mit ihrem einen Schnappschuss-
  Durchlauf (`snapshot_local_keys.rs`, Quellstand 2026-10-31, nicht gebaut): jeder aufgelistete Ordner
  gibt seinen Lauf (Einträge der Auflistung mit Art, Name, versteckt, bei Dateien Größe, Zeit und der im
  Durchlauf gelesenen MD5; Auslassungen der Auflistung; betretene Unterordner mit ihrem Schlüssel), am
  Ende folgen die Schlüssel von unten nach oben. Der lokale Baum wird damit einmal aufgelistet und jede
  Datei höchstens einmal gelesen. Einen Ordner, den der Durchlauf nicht betritt (Filter des Jobs,
  geschützter Ordner wie Einhängepunkt oder Systemordner), der Durchlauf der Gegenseite mit ihrem Filter
  aber betritt, liest dessen Schlüssel-Durchlauf im eigenen Prozess einmal (`LocalKeyed::below`), damit
  er und die Ordner darüber bei gleichem Inhalt den Schlüssel der Gegenseite haben; ausgenommen
  Pseudo-Dateisysteme und die eigenen Ordner der App (mit dem Prüfsummen-Cache). Der Schnappschuss
  selbst, mit Einhänge-, Link- und Junction-Grenzen, Abbruch, Fortschritt und geschützten Auslassungen,
  bleibt der des Durchlaufs ohne Schlüssel. Ein Schlüssel aus weniger oder anderen Teilen als bei der
  Gegenseite ist nie gleich: der Ordner wird dann geholt, nie als gleich genommen. Gleiche Ordner der
  entfernten Gegenseite kommen gar nicht über die Verbindung.
- **Prüfschlüssel ohne Dateiübertragung (E9).** Im Prüfsummen-Modus rechnet die Seite mit der Datei die
  MD5 im Durchlauf (Share-Host, se-agent); sonst fragt der Durchlauf die Dateien eines Ordners zusammen
  (`vfs::content_md5_many`, SFTP ohne Agent: eine Exec-Anfrage je Gruppe) und vor jedem Download in
  `snapshot_dir::content_hash` die übrigen einzeln über `vfs::content_md5`. Ein persistenter Hash-Cache auf der Seite der Datei (se-agent,
  Share-Host, lokale Seite) erspart das erneute Lesen unveränderter Dateien: Schlüssel Gerät, Inode,
  Größe, Änderungs- und Statusänderungszeit (ns); kein Treffer, solange eine der Zeiten nicht mehr als
  2 s vor dem Erfassungszeitpunkt liegt (Git „racy-git“); höchstens 500 000 Einträge. Er gilt nur auf
  Dateisystemen mit dauerhaften Datei-Kennungen: nicht FAT/vfat/msdos/exFAT, FUSE, SMB/CIFS, 9p,
  VirtualBox-Ordner und optischen Medien unter Linux (`statfs`-Typ, `agent_proto/os/linux_os/file_ids.rs`),
  unter Windows nur NTFS und ReFS (`GetVolumeInformationByHandleW`, `os/windows/file_ids.rs`); dort wird
  jedes Mal gelesen. Unter Windows kommen Stempel aus `GetFileInformationByHandleEx` (`FileIdInfo`,
  `FileBasicInfo.ChangeTime`). Der Ordner ist `support_dirs::app_data_dir()` (Android: der des Hosts);
  der eigenständige se-agent nutzt `$XDG_CACHE_HOME/se-agent` bzw. `~/.cache/se-agent`.
  *Frisch prüfen:* Im Prüfsummen-Modus liest eine lokale Seite jede Datei neu (`LocalReads::fresh`,
  `hash_cache::digest_fresh`: der gespeicherte Wert wird nicht genommen, der gelesene gespeichert); auch
  der Vergleich vor dem Überschreiben (`orchestration_plan::hash_checked`) liest frisch. Ebenso ein
  entfernter Host: die Durchläufe des Syncs im Prüfsummen-Modus tragen `HashWalkRequest::fresh`
  (se-agent und Hintergrunddienst `walk-filter-v1`: Optionsbit in `WalkHashed3`, `WalkKeyed`, `ListRuns`;
  Share `hash_walk_filter_v1`: `FsHashWalk::scope.fresh`); se-agent (`LocalReads::fresh`) und Share-Host
  (`host_hash_walk_local.rs`, `hash_cache::digest_fresh`) lesen dann jede Datei, deren Prüfschlüssel sie
  bilden, und speichern den gelesenen Wert. Die Einzelanfrage des Syncs vor einem Download
  (`vfs::content_md5`) fragt keinen Hash-Cache (SFTP-Befehl, FTP `HASH`, WebDAV-Eigenschaft; Share und
  Dienst bieten sie nicht an). Duplikatsuche, rekursive Ansicht, Kopierprüfung und die anderen
  Vergleichsmodi senden kein `fresh` und behalten den Cache. Eine Gegenstelle ohne die Fähigkeit
  antwortet wie bisher aus ihrem Cache. Das Leeren des Caches (`se doctor --clear-hash-cache`,
  Desktop-Einstellungen Seite Sync, Android Einstellungen Speicher, MCP über `se`) vergisst alle
  Prüfsummen dieses Geräts: im aufrufenden Prozess, in der Datei (neue Generation,
  `agent_proto/os/shared/hash_cache_file.rs`) und im Hintergrunddienst über seine IPC
  (`daemon/os/shared/hash_cache_clear.rs`). Ein Prozess, den das Leeren nicht erreicht, vergisst
  seine Prüfsummen, sobald er die neue Generation sieht (`forget_if_cleared`, spätestens nach einer
  Sekunde), und schreibt sie nie zurück.
  *Schreiben:* neue Prüfsummen werden im Speicher gesammelt und gemeinsam geschrieben
  (`hash_cache::flush`: höchstens alle 30 s; `flush_now`: am Ende eines Durchlaufs, einer Verbindung
  oder eines Syncs), als atomarer Ersatz der Datei, die unter der Sperre gebildet und außerhalb der Sperre
  geschrieben wird; kein Faden wartet auf das Schreiben.
  *Einmal lesen:* Schlüsselphase und Schnappschuss einer lokalen Seite sind ein Durchlauf
  (`snapshot_local_keys.rs`); er liest jede Datei einmal und gibt ihre MD5 an den Lauf ihres Ordners.
  Ein SSH-Agent ohne eigene Datei-Prüfsumme gibt `content_digest` an
  seine SSH-Verbindung weiter (Exec-Befehl, siehe Tabelle oben). SFTP ohne Agent, FTP und WebDAV liefern die MD5 am Ort der Datei über
  `vfs::content_md5` (Block „MD5 am Ort der Datei“ oben), bevor `snapshot_dir::content_hash` lädt.
- **Filter des Sync-Jobs auf der Seite mit den Dateien.** Die Durchläufe mit Eintragsmerkmalen und mit
  Ordnerschlüsseln tragen den Filter des Jobs (`HashWalkRequest::filter`, `agent_proto::walk_filter`;
  se-agent/Dienst `walk-filter-v1`, Share `hash_walk_filter_v1`; die lokale Seite der Paarphase
  entscheidet mit demselben Filter, welche vom eigenen Filter ausgelassenen Ordner sie für ihre
  Schlüssel liest): versteckte Einträge, Größen- und Zeitgrenzen und die Ausschlussmuster, die die Seite
  mit genau der Bedeutung des Clients auswerten kann. Der Client übernimmt dafür aus globset 0.4.18 den
  regulären Ausdruck jedes Musters (`Glob::regex`, gebaut wie `compile_patterns`, `syncjobs::JobIgnore`);
  die Seite liest ihn in einen Automaten über Bytes (`agent_proto/core/walk_pattern.rs`: `.` jedes Byte,
  Groß-/Kleinschreibung nur für ASCII, eine Klasse erst gefaltet, dann verneint, ganze Pfade). Ein
  Ausdruck außerhalb dieser Form wird dort nicht angewandt; der Client wendet jedes Muster weiter selbst
  an. Unter Windows zählt ein `\` im Namen wie `/` (globset-Kandidat). Was der Filter ausschließt, kommt
  weiter als Eintrag (der Planer führt ausgeschlossene Dateien als gefiltert, sie schützen ihren Ordner
  und gelten nie als gelöscht), aber eine Datei ohne Prüfschlüssel und ungelesen, ein Ordner ohne seinen
  Inhalt. So lässt die Seite nie mehr aus als der Client: was der Client nimmt, kommt mit Prüfschlüssel
  und Inhalt. Ordnerschlüssel bleiben gleichheitsrichtig: ein ausgeschlossener Ordner ist ein Eintrag im
  Lauf seines Elternordners ohne eigenen Schlüssel (`RunKeys::skip_dir`), eine ausgeschlossene Datei
  trägt im Prüfsummen-Modus keinen Prüfschlüssel; ein Schlüssel eines Laufs mit anderem Filter ist
  darum ein anderer, und ein gleicher Schlüssel bedeutet den gleichen gefilterten Ordner an jedem Pfad
  (`folder_key_cache::Record::walked_children`: ein Unterordner ohne Schlüssel wurde nicht betreten).
  Agent, Share-Host und lokale Seite schlüsseln mit derselben Musterliste: `agent_proto::host_patterns`
  nimmt höchstens so viel Mustertext, wie eine Share-Anfrage trägt (`HOST_PATTERN_WIRE`, 16 KiB
  JSON-Text neben dem größten Text bekannter Schlüssel oder der Ordnerliste), und diese Liste geht an
  jede Gegenstelle und an den lokalen Schlüssel-Durchlauf; die Kürzung der Share-Anfrage
  (`WalkFilterSpec::bounded`) nimmt sie ganz. Ein Muster jenseits dieser Grenze wendet nur der Client an.
  Gegenstellen ohne die Fähigkeit durchlaufen ungefiltert; ebenso ein Share-Host oder Agent, der einen
  gesendeten Filter nicht übersetzen kann (etwa ein Konstrukt eines späteren Parsers): der Share-Host
  läuft dann ohne Filter durch (`host_hash_walk.rs` `scope_of`), der Agent antwortet „nicht
  unterstützt“, und der Client nimmt den bisherigen Weg. Der Client filtert das Empfangene immer selbst.
- **Sommerzeit.** Ist eine Seite FAT (Ortszeit), gelten gleiche Größen mit genau einer Stunde Abstand
  (± ein Schritt der gemeinsamen Genauigkeit) nur dann als gleich, wenn beide Beobachtungen einen am Ort
  der Datei gebildeten Prüfschlüssel tragen (`Sig::hash`: lokal gehasht, Server-MD5 der Auflistung,
  Hash-Durchlauf des Hosts) und dieser gleich ist (`bisync/core/compare.rs` `same_file`). Ohne Prüfschlüssel
  gilt die Stunde wie jeder Zeitabstand: eine Stunde neuer ist neuer, und eine FAT-Seite eine Stunde neben
  ihrer eigenen Basis gilt als geändert. `compare::hour_apart` benennt diese Paare (für sie gilt
  `worth_hashing`); der Planer prüft sie dort mit `Decision::Verify` (beide Seiten am Ort hashen), wo er
  `worth_hashing` fragt (beidseitig geändert, Spiegeln, Verschieben). Offen: im Zweig „nur eine Seite
  geändert“ (`bisync/core/plan_decide.rs` `decide`, `propagate`) fragt der Planer noch nicht nach
  `hour_apart` und kopiert. exFAT speichert einen UTC-Abstand (exFAT-Spezifikation 7.4.10) und bleibt ohne
  diese Regel.
- **Explorer-Listen über den Dienst und den Agenten** tragen `hidden` wie das bediente Backend bzw. nach
  Punktnamen und die Erstellzeit (`meta-list-v1`: `ListDirMeta`, `StatMeta`); ältere Gegenstellen wie
  bisher.

## Was erhalten bleibt

Stufen + Veröffentlichen ohne Ersetzen, keine halbfertigen Dateien unter dem Zielnamen,
Quelländerungen werden erkannt (Länge, bei Drive MD5, sonst nachgelagertes `stat`), keine
blinde Wiederholung mehrdeutiger Mutationen, Links werden nie gefolgt, der App-Papierkorb wird
ausgelassen, geschützte lokale Ordner werden genau einmal angefragt. Sync, Laufwerks-Mounts und
Ersetzen behalten ihre dauerhaften Schreibwege.

Im Sync rechnet der MD5 ab dem ersten Mebibyte in einem eigenen Faden mit (`StreamHasher`, ein
Faden je Kopie ab 1 MiB; Erzeugungsfehler bleiben inline; Stand U52).

## Nachweis

Die Task-Suite `native/test-transfer-engine-task.sh` (Workflow `transfer-engine-task.yml`):
Meilenstein-Tests (`transfer_engine_task_`), Copy/Paste-Sicherheitstests über die Engine,
Tests aller betroffenen Module, ganze Bäume gegen SFTP- und FTP-Container, Durchsatz über eine
auf 50 ms/100 Mbit/s geformte Loopback-Leitung (`native/tests/transfer_throughput.rs`),
Share-Raum Ende-zu-Ende mit echten Binaries, Windows-Meilensteine (OLE-Rundreise) und die
Android-Gerätesuite. Nicht gemessen: echte Netze der Nutzer, das UI-Verhalten des Explorers
beim Überfahren mit großen Auswahlen (siehe `docs/refs/windows-virtual-files.md` §9).

E13-Tests laufen mit dem Präfix `speed_task_` in der Suite (Teil B, U41). Diese Fassung ist nicht gebaut und nicht ausgeführt.

**Messstufe `speed`.** Die Messfälle laufen im Cargo-Profil `speed` (`native/Cargo.toml:378-387`).
Es erbt `release` und schaltet Debug-Assertions ein, weil `SE_TEST_NO_LZ4` und die Share-Statistik
nur in solchen Builds wirken (`native/crates/engine/src/share/core/transfer_meter.rs:23-27`). Der Messdienst und
das Messprogramm sprechen über den IPC-Port des Dienstes (`daemon.ipc`); diese Verbindung bleibt von
der netem-Regel auf `lo` ausgenommen. Dann ist die Wurzel ein `prio` mit zwei Bändern: netem liegt auf Band
1:1, die IPC-Ports des Dienstes gehen per u32-Filter auf das ungeformte Band 1:2
(`native/rdt_task/speed_net.py:44`, `:107-124`, `:127-137`).

**Share-Statistik (`share-quic-stats.jsonl`).** Jede Share-Verbindung schreibt je Übertragungsstoß
eine Zeile in `share-quic-stats.jsonl` im App-Datenordner (`native/crates/engine/src/share/core/quic_stats_hub.rs:36`,
`:300-310`). Ein Stoß beginnt mit einem Takt von 500 ms (`quic_stats.rs:19-20`), in dem mindestens 256 KiB
UDP-Nutzlast in beide Richtungen zusammen fließen; er endet nach zwei ruhigen Takten, und gemeldet
wird er erst ab zwei belegten Takten (`quic_stats.rs:6-10`, `:19-27`). Beide Enden schreiben, mit
`side` `client` oder `host`. Die Datei dreht bei 4 MiB auf `<name>.1` (`quic_stats_hub.rs:39`). Die
Statistik gibt es nur in Entwicklungsbuilds, nicht in Release-Builds und nicht unter
`SE_TEST_NO_SHARE_STATS` (`transfer_meter.rs:23-27`, `quic_stats_hub.rs:103`).

Die Messstufe übernimmt die Zeilen in `speed_report.json` (`native/rdt_task/speed_report.py:133`).
Jeder Fall in `cases[]` trägt dort, wenn Zeilen vorliegen, die Liste `share_quic` und die Textzeilen
`share_quic_summary` (`native/tests/speed_task/support.rs:198-200`). Eine Zeile von `share_quic`
enthält `kind`, `ts_ms`, `pid`, `side`, `conn`, `seq`, `cc`, `windows` (`stream_bytes`,
`connection_bytes`, `send_bytes`), `socket_buffers` (`rmem_max`, `wmem_max`), `busiest_threads`
(je `name`, `tid`, `cpu_ms`), `burst` und `device` (vom Sammler ergänzt,
`native/tests/speed_task/share_stats.rs:213-216`; Aufbau `quic_stats_hub.rs:247-266`).

Lesehilfe für `burst` (`native/crates/engine/src/share/core/quic_stats.rs:205-245`, Ableitung in
`into_report`, `quic_stats.rs:304-355`): `rtt_ms`, `cwnd_bytes`, `mtu`, `path_kind` und `paths_open`
stehen für den letzten belegten Takt. `rtt_ms_min` und `rtt_ms_max` sowie `cwnd_bytes_min`,
`cwnd_bytes_max` und `cwnd_bytes_mean` sind Extremwerte und Mittel über die belegten Takte. Alle
übrigen Zähler sind Differenzen zum Takt vor dem Stoß: `congestion_events`,
`spurious_congestion_events`, `lost_packets`, `lost_bytes`, `udp_tx_datagrams`, `udp_tx_bytes`,
`udp_rx_datagrams`, `udp_rx_bytes`, `plpmtud_probes_sent`, `plpmtud_probes_lost`,
`black_holes_detected`, `frames_tx`, `frames_rx`, `streams`, `meter`, `process_cpu_ms` und
`kernel_udp`. `udp_tx_mb_per_s` und `udp_rx_mb_per_s` teilen die Bytes durch `duration_ms`.
`meter` (`MeterReport`, `quic_stats.rs:166-179`) enthält `raw_bytes`, `wire_bytes`, `lz4_blocks`,
`lz4_ms`, `disk_read_ms`, `stage_write_ms`, `stage_wait_ms`, `net_wait_ms` und `disk_wait_ms`; die
Zeitfelder sind Millisekunden. `kernel_udp` (`rcvbuf_errors`: Datagramme, die ein voller
Empfangspuffer verworfen hat) ist die Differenz der systemweiten UDP-Zähler. Die Rahmenzähler
`frames_tx` und `frames_rx` haben die Felder `stream`, `acks`, `ack_frequency`, `immediate_ack`,
`ping`, `data_blocked`, `stream_data_blocked`, `streams_blocked_bidi`, `max_data` und
`max_stream_data` (`quic_stats.rs:41-52`).
