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
  (lokal) kennt „Überschreiben“ als ausdrückliche Wahl.
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

- `native/src/transfer/core/job.rs`: `TransferJob` (Endpunkte, Auswahl als Wurzeln oder Paare mit
  Größe/Zeit/ID, Layout, Filter, Konfliktpolitik, Modus, `resume`), `validate()`.
- `native/src/transfer/os/shared/engine/`: Ablauf, Worker, Operationen je Richtung
  (`upload.rs`, `download.rs`, `remote_copy.rs`, `local.rs`, `batch*.rs`), Veröffentlichung
  (`publish.rs`), Ordner-Register (`folders.rs`), Fehlerprotokoll (`issues.rs`).
- Warteschlange: Dateien vor Ordnern. Ein Ordner mit Dateien entsteht mit seiner ersten Datei (Ordner-Register), eigene Ordner-Arbeit brauchen nur leere Ordner und das Aufräumen beim Verschieben; so halten die Ordner einer Auflistung die ersten Dateien nicht auf.
- `native/src/transfer/os/shared/walk*.rs`: Walker. Ausgewählte Einträge werden parallel per
  `stat` geprüft; liegen in einem Ordner mehr ausgewählte Einträge, als gleichzeitig laufen
  dürfen, beantwortet eine Auflistung des Ordners alle. Links, Spezialdateien und ungültige
  Namen sind gemeldete Auslassungen, der App-Papierkorb eine stille Auslassung.
- `native/src/transfer/os/shared/flow.rs` + `core/flow_control.rs`: Flows (siehe unten).
- `native/src/transfer/os/shared/{access,memory}.rs`: eine Rechteanfrage je Übertragung für
  geschützte lokale Ordner; prozessweites Speicherbudget für gepufferte Bytes.
- `native/src/transfer/os/shared/lane.rs`: Lane ohne feste Grenze; fertige Übertragungen liefern
  Job, erste Fehler und aufgelöste Zielwurzeln für „Fehlende übertragen“.

## Parallelität: Fenster, Startwert und Speicherbudget

Die richtige Zahl gleichzeitiger Operationen hängt von Bandbreite × Latenz, Dateigrößen,
Serverleistung und Speichermedium ab – nichts davon ist vorher bekannt. Jede Verbindung (bzw.
jedes lokale Volume) hat deshalb **einen** Flow, den alle Übertragungen, Explorer-Übergaben und
Sync-Läufe dieses Prozesses teilen; er misst die Nutzleistung (Bytes plus 64 KiB je fertiger
Operation, `OP_WEIGHT_BYTES`, damit reine Kleinstdatei-Last messbar ist) und regelt die Grenze
(`native/src/transfer/core/flow_control.rs`):

| Parameter | Wert | Grund |
|---|---|---|
| Startgrenze | 2 (`START_LIMIT`); gelernt: die halbe zuletzt eingestellte Grenze, mindestens 2 (`start_from_learned`, `native/src/transfer/core/flow_window.rs:19-21`). Mindeststart je Art (`start_floor`, `native/src/transfer/os/shared/flow_registry.rs`, Stand 2026-10-28): Share 8; SFTP, SMB, WebDAV 8; FTP und Google Drive 4; lokales Volume, das nicht dreht oder dessen Art unbekannt ist, 2 × Kerne innerhalb 8–16; ein drehendes Laufwerk (Linux `queue/rotational`, Windows Seek-Penalty-Abfrage) und Tresore behalten 2. Die Protokollgrenze (`transfer_ceiling`) begrenzt jeden Start | eine Operation in Arbeit, eine im Anlauf; ein bekannter Server startet nicht bei null; Flash und Server mit vielen Strömen erreichen den stetigen Zustand ohne Hochlauf |
| Fenster | Abschluss, wenn mindestens 100 ms (`MIN_WINDOW_MS`) und 8 Operationen (`MIN_COMPLETIONS`) vergangen sind; ohne Messung 1 s (`FALLBACK_WINDOW_MS`), höchstens 5 s (`MAX_WINDOW_MS`) | kurz genug zum Reagieren, lang genug gegen Rauschen |
| Slow Start | verdoppeln, solange jede Messung um ≥ 10 % zulegt (`SLOW_START_GAIN` = 1,10; `flow_control.rs:7-8`, `:317-320`) | findet große Grenzen in wenigen Fenstern |
| Proben | Schritt Grenze/8 (`PROBE_STEP_DIVISOR`), über 2 Fenster (`PROBE_WINDOWS`), alle 4 Fenster im Leerlauf (`PROBE_INTERVAL_WINDOWS`) | symmetrische Schritte halten die Grenze unter Rauschen am Optimum (Simulation mit ±10 %: ≥ 93 % der Bestleistung bei ≤ 1,11-facher Parallelität) |
| Behalten | aufwärts ≥ +5 % (`PROBE_GAIN`), abwärts ≥ 97 % (`PROBE_KEEP_LOWER`) | nur Änderungen, die sich lohnen |
| Überlast | Grenze halbieren; danach halten 3 Fenster die Grenze (`OVERLOAD_HOLD_WINDOWS`, `flow_control.rs:267`) | Ratenlimits, „busy“, Timeouts (typisiert als `vfs::congestion_error`) |
| Obergrenze | 256 (`RESOURCE_CEILING`) und die Protokollgrenze (`transfer_ceiling`) | Threads und Puffer je Operation; echte Grenzen kommen vom Protokoll |
| Merken | 15 min nach letzter Nutzung (`FLOW_RETENTION`, `native/src/transfer/os/shared/flow_registry.rs:12`) | das nächste Einfügen beginnt, wo das letzte endete |

**Startwert aus `transfer-hints.json`.** Der Flow lernt die Grenze, bei der eine Verbindung
oder ein Volume sich eingependelt hat, und schreibt sie in `<app data>/transfer-hints.json`
(`native/src/transfer/os/shared/hints.rs`, Datei `transfer-hints.json`, `:23`). Ein Eintrag ist
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
`native/src/transfer/os/shared/memory.rs:14-35`. Das Budget widerruft nie einen Puffer, den schon
eine Operation hält.

Regeln: Erlaubnisse gehen reihum zwischen Übertragungen (ein kleines Einfügen neben einem
großen Job kriecht nicht); Auflistungen haben einen reservierten Platz; Lokal↔Remote regelt nur
der Remote-Flow; nie mit gehaltener Erlaubnis auf etwas warten, das selbst Erlaubnis, Speicher
oder einen Kanal braucht; Speicher wird vor der Erlaubnis reserviert. Flows sind prozesslokal:
Sync-Jobs im Hintergrund-Dienst regeln getrennt von der GUI. Die QUIC-Fenster der Share-Verbindung
(16 bis 64 MiB, `share/core/keepalive.rs`) bleiben unverändert; das Speicherbudget erhöht sie nicht.

## Teilbereiche großer Dateien

- **Wann.** Eine Datei ab 16 MiB (`SPLIT_FROM` = zwei kürzeste Teile) wird in Teile zerlegt
  (`native/src/transfer/core/segments.rs:11`, `:13`, `splits` in `:23`). Kleinere Dateien laufen als ein Strom.
- **Wie lang.** Ein Teil ist mindestens 8 MiB (`MIN_SEGMENT`). Der Teil gibt höchstens ein Zehntel
  seiner Zeit für den Aufbau aus (`SETUP_SHARE`) und dauert höchstens 30 s als ein Strom
  (`MAX_SEGMENT_SECONDS`). Die Länge des nächsten Teils ergibt sich aus der gemessenen Anlaufzeit
  und Streamrate der fertigen Teile dieser Verbindung (`SegmentSizer`, `segments.rs:23-60`).
- **Ablage.** Die Stufe wird vor dem ersten Teil auf die Länge der Datei gesetzt (`set_len`,
  `native/src/transfer/os/shared/engine/download.rs:73`); die Teile landen als dünn besetzte Datei,
  die fertig ist, wenn alle Teile fertig sind. Ein Abbruch behält die fertige Präfixlänge
  (`prefix()`), und das Fortsetzen beginnt dort (`segments.rs:126`).
- **Wo.** Die Engine führt Teile in `segments_download`, `segments_upload` und `segments_copy` aus
  (`native/src/transfer/os/shared/engine/`), unter den Erlaubnissen des Flows. Welches Protokoll
  wie teilt, steht in der Tabelle „Je Protokoll“.

## Wer begrenzt

Jede Operation bucht die Zeit ihres Fadens: Lesen von der Quelle, Schreiben zum Ziel, Rechnen
(Hash, Kompression, Verschlüsselung) und Warten auf Erlaubnis, Speicher oder Ordner. Eine Sekunde
lang ergibt die Summe ein **Urteil** über den Engpass (`native/src/transfer/core/limiter.rs`,
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
  (`note`, `native/src/transfer/core/types.rs:58`; Vorrang hat „wartet auf <Verbindung>“,
  `engine/mod.rs:359-366`, sonst der Hinweis des Ziels, dann der der Quelle, `engine/mod.rs:367-376`).
  Der Hinweis ist z. B. „2 Verbindungen zu <Host>“ (SFTP, siehe unten). Das Übertragungsfenster
  zeigt den Hinweis unter der Aktivitätszeile (`native/src/app/core/transfer_window.rs:240-241`).
  Das MCP-Werkzeug `copy` gibt ihn im Feld `progress.note` aus (`native/src/cli/mcp/transfer_tools.rs:104-106`).
  Android zeigt ihn als Meldung der Aufgabe neben den Zahlen (`native/src/mobile/os/shared/drive.rs:99`,
  `task_message`; während der Suche steht dort „Suche Dateien…“).
  `se cp --stats` gibt ihn nicht aus (keine Lesung in `native/src/cli/`); es gibt dort die Zeile mit Rate und Engpass.
- **Namen.** Die Texte nennen Laufwerke, Geräte und Arten von Endpunkten, nie eine Datei oder einen
  Ordner (`bottleneck.rs:9-12`).

## Je Protokoll

| Protokoll | Was es erlaubt | Was die Engine nutzt | Grenzen und Zahlen |
|---|---|---|---|
| Lokal, UNC | Kernel-Kopie, Server-Offload | Kopie über die exklusiv erzeugte Stufe (Linux `copy_file_range` zwischen Handles, Windows `CopyFile2` „nicht ersetzen“ + Identitätsprüfung); innerhalb einer UNC-Freigabe kopiert der SMB-Server selbst (Offload/COPYCHUNK); kein fsync bei neuen Kopien, Verschieben auf demselben Volume als eine Umbenennung; Ring und Bereiche: siehe „Lokal, UNC“ unten | fsync nur bei Verschieben und Überschreiben; Abbrechen wirkt zwischen den Kopierblöcken |
| Direct/Raum-Share (Iroh/QUIC) | viele Ströme je Verbindung | `fs_transfer_v1`: Pakete für kleine Dateien (Statusabfrage bei verlorener Antwort), Lesen ab Offset, exklusive Ordner, serverseitige Kopie; Bereiche mit der Capability `ranges_v1` (`native/src/share/core/batch_wire.rs:58`), LZ4 mit `lz4_v1` (`batch_wire.rs:63`, Abschnitt „Kompression“); alte Hosts: bisheriger Weg | 64 Ströme − 4 fürs Blättern = 60 Übertragungen je Gegenstelle (Host-Zulassung), davon nutzt der Flow höchstens 56, 4 bleiben für Vordergrund-Lesen; Host gesamt 256 (halbe Tokio-Blocking-Threads); alte Hosts 32; Paket ≤ 256 Dateien/16 MiB, Kopf ≤ 256 KiB; Fenster 16 MiB je Strom (1 Gbit/s × 100 ms), Verbindung 16–64 MiB je nach Speicher; voller Host antwortet „busy“ statt heimlich zu warten, alte Clients warten höchstens 45 s (ihre Frist ist 60 s). Stufen-Bereiche nur bei lokalen Exporten; für nicht lokale Exporte fehlt eine Handle-Schnittstelle in `vfs` (Stand U53). Unfertige Bereichsstufen des Wirts werden beim Verbindungsende, nach 300 s ohne Fortschritt und beim nächsten Start entfernt (Stand U53) |
| SSH-Agent / Hintergrund-Dienst | Anfragen mit IDs über einen Kanal | Kredit je Anfrage (`+credit-v1`: ein langsamer Download blockiert kein Blättern), Pakete (`+batch-v1`), Stufen-Frames, Pool zusätzlicher Exec-Kanäle; Bereiche mit `Read {offset, len}`, Label `range-v1` (`native/src/agent_proto/core/features.rs:17`); die Bereichsstufe bleibt auf dem Kanal, der sie geöffnet hat (Stand U53) | 64 MiB Budget je Verbindung, 1 MiB Anfangskredit, Fenster bis 8 MiB je Strom; Kanal-Pool bis MaxSessions − 2; |
| SFTP | gepipelinete Anfragen, mehrere Kanäle | viele gleichzeitige READs mit wachsender Tiefe, gebündelte WRITEs (Pakete, 16 MiB je Paket, `BATCH_BYTES`), Kanal-Pool, Kopien ohne fsync, serverseitige Kopie per `copy-data` wo angeboten (in Bereichen von ~10 s, lehnt der Server ab, wird gestreamt). Bereiche über Pool-Kanäle, die sich Handles teilen, und über Zusatzverbindungen (Regeln unter „SFTP-Kanäle und -Zusatzverbindungen“); jeder Kanal hat sein eigenes Serverfenster (OpenSSH 2 MiB). Schreiben von Bereichen nur, wenn die Ordnerkette bis `/` dem Konto oder root gehört und für Gruppe und andere gesperrt ist (oder Sticky-Bit und root; Sicherheitsantwort S7). Dateien mit gelistetem MD5 bleiben ein Strom | Dateien je Kanal gleichzeitig offen: höchstens `min(open-handles, 64)` (`OPEN_HANDLES`, `native/src/sftp/core/batch_ops.rs:52`; ohne Angabe des Servers 64); ein OPEN mit einfachem Fehlerstatus oder ohne Antwort ist Überlast, ein bestimmter Status (nicht gefunden, Zugriff verweigert) ein Dateifehler (`open_failure`); jede Datei hat vier Wartezeiten (OPEN, WRITE, CLOSE, RENAME), die sich über bis zu `open-handles` Dateien überlappen; Pool bis zur Ablehnung, 2 Kanäle Reserve; Kanäle in Einrichtung zählen bei der gelernten Grenze mit (`channel_pool.rs:57-59`, `:168-170`, `:357`); Obergrenze des Flows `transfer_ceiling` = Kanäle × Handles je Kanal (`backend.rs:428-430`, `channel_pool.rs:288-301`); `mkdir_all` fragt alle Pfadteile mit einem Schwung `LSTAT` ab (eine Rundreise) und legt nur den fehlenden Rest an, jeder vorhandene Teil muss ein echter Ordner sein (MS7, `native/src/sftp/core/transfer_ops.rs`); `finish_stage`: `LSTAT`, `OPEN`, dann `FSETSTAT`, `fsync@openssh.com`, `FSTAT`, `CLOSE` und `LSTAT` zusammen (3 statt 6 Rundreisen; MS8, `native/src/sftp/core/stage_finish.rs`) |
| FTP/FTPS | eine Übertragung je Steuerverbindung | Verbindungs-Pool, Größen-Stufe mit streamendem STOR, Fortsetzen per REST; Prüfen und Veröffentlichen auf der Verbindung der Übertragung (SIZE für den zufälligen Stufennamen, eine Auflistung des Ordners für Stufe und Ziel, dann RNFR/RNTO), ohne freie Verbindung auf der Blätter-Verbindung. Bereiche über `REST` auf einer freien oder für den Bereich neu angemeldeten Pool-Verbindung (gleiche Zugangsdaten, gleiche TLS-Regel; nie die Blätter-Verbindung), Abbruch mit `ABOR` (G3-11, Entscheidung 2026-10-28, ersetzt S12; Abschnitt „Entscheidung G3-10/G3-11“) | STOR und RETR gehen vor dem TCP-Aufbau der Datenverbindung hinaus (suppaftp); es gibt kein Befehls-Pipelining: das nächste Kommando geht erst nach der Antwort auf das vorige (Testserver `native/src/ftp/core/data_overlap_server.rs`). Grenze lernt der Pool aus 421/530 nach erfolgreicher Anmeldung, auch bei Anmeldungen für Bereiche; wie viele Bereiche gleichzeitig laufen, bestimmt das gemessene Fenster des Flows; Bereiche ab 16 MiB Dateigröße (`SPLIT_FROM`); Bereiche nur, wenn der Server mindestens zwei Übertragungsverbindungen zulässt (`ftp/core/pool.rs` `ranges_possible`); 421/530 auf REST/RETR: 15 Minuten keine Bereiche; eine Verbindung bleibt fürs Blättern; Stufenprobe entfällt nur für Namen `.se-upload-<16 Hex>` (`FTP_PROBE_STAGE_NAME`, S6) |
| WebDAV | viele HTTP-Verbindungen, COPY | gepoolte Mutationen, gestreamtes PUT mit Länge und `If-None-Match: *`, serverseitiges COPY, Range. Engine-Weg: PUT, PROPFIND (Stufe prüfen), MOVE. Sync-Weg: die Bestätigung der Zeit nach MOVE entfällt, wenn die MOVE-Antwort `X-OC-Mtime: accepted` trägt (`native/src/webdav/core/stage_move.rs:4-10`); ein PROPFIND je Sync-Upload weniger. Bereiche über `Range: bytes=a-b`; antwortet der Server mit 200, läuft die Datei als ein Strom | 429/503 → Überlast mit `Retry-After`; DELETE und leeres PUT ungepoolt (ureq würde sie wiederholen) |
| Google Drive | parallele Verbindungen, `generateIds` | gepoolte Verbindungen, neue Datei ≤ 5 MB mit einem Aufruf, Resumable mit wachsenden Blöcken und zwei Puffern (ein Abschnitt geht auf einem eigenen Faden hinaus, während der nächste gefüllt wird; nur wenn der Speicher für den zweiten Puffer reserviert werden kann; G3-08c, `native/src/gdrive/core/chunk_stream.rs`), ID-Vorrat, Cache im Hintergrund, Sperren je Name, `files.copy`; Bereiche über `Range: bytes=a-b`, antwortet der Server mit 200, läuft die Datei als ein Strom | Google begrenzt dauerhaft ~3 neue Dateien/s je Konto (Hinweis in der Zeile); Ratenlimits sofort als Überlast |
| SMB | Credits, COPYCHUNK | Pakete über Compounds: CREATE, WRITE und CLOSE je Datei und das Umbenennen in zwei Rundreisen; Reparse-Punkte werden nie über `read_file_compound*` gelesen (eigene Anfragen, damit Links nicht gefolgt werden); mehrere Lese-Blöcke gleichzeitig, serverseitige Kopie wo angeboten, neue Kopien ohne FLUSH; Bereiche über `read_at`; Schreiben in Bereichen über eigene Handles, die vor dem ersten Byte gegen die Kennung der Stufe geprüft werden (S14); Belege für Kopierstufen der Engine (G3-02): der `stat` des Ziels geht mit dem CLOSE der Stufe hinaus, der `stat` der Stufe direkt danach, die Veröffentlichung ist ein Compound CREATE (kein Ordner, Link als er selbst) + SET_INFO (Umbenennen ohne Ersetzen) + CLOSE, dessen CREATE-Antwort die veröffentlichte Datei beschreibt (`native/src/smb/core/stage_evidence.rs`) | Block ≤ 512 KiB bzw. MaxReadSize, Tiefe nach Credits; eine Serverkopie sieht Abbrechen erst an ihrem Ende |
| ZIP (Quelle) | – | ein Parse je Archiv, Einträge gestreamt | Parallelität = Kernzahl (Entpacken ist CPU-gebunden) |

**SFTP-Kanäle und -Zusatzverbindungen.** Ein Bereich nimmt einen leeren Kanal. Gibt es keinen, öffnet er
einen neuen, solange die gelernte Grenze es erlaubt; sonst teilt er den am wenigsten belasteten
Kanal mit anderen Bereichen, sofern dort noch Handles frei sind (`native/src/sftp/core/link_rules.rs:272-284`,
`:351-359`). Sind alle Kanäle an ihrer Handle-Grenze, wartet ein Bereich höchstens 30 s auf ein Ende
(`channel_pool.rs:50`, `:318`, `:331`), danach nutzt er die Hauptsitzung. Eine ganze Datei wartet
nicht: Sind alle Kanäle voll, nutzt sie sofort die Hauptsitzung (`channel_pool.rs:331`). Kanäle, deren Einrichtung noch
läuft, zählen bei der gelernten Grenze mit (`channel_pool.rs:57-59`, `:168-170`).

Zusatzverbindungen sind eigene SSH-Verbindungen mit je einem Kanal im Subsystem `sftp`
(`native/src/sftp/core/pool_channel.rs:284`). Sie öffnen nur Übertragungen; Auflistung, Metadaten,
Befehle und die Exec-Kanäle des se-agent bleiben auf der Hauptverbindung (Sicherheitsantwort E13-S15,
`docs/plaene/2026-10-17-rechte-desktop-tresore/stand/SEC.md`, Punkt 15, Frage 1 (vi); die
Anmeldung über den SSH-Agenten geändert durch G3-10, Abschnitt „Entscheidung G3-10/G3-11“). Sie
gelten nur unter diesen Bedingungen:

- **Anmeldung ohne Eingabe:** gespeichertes Passwort oder Schlüssel (`link_rules.rs` `plan`), oder
  der SSH-Agent mit der Identität, mit der sich die Hauptverbindung angemeldet hat (G3-10). Ein
  Schlüssel hat diese Herkunft, wenn das Formular keine Passphrase enthält
  (`native/src/connect/os/shared/connector.rs:100-102`); lässt sich der Schlüssel dann nicht laden,
  entsteht keine Zusatzverbindung. Die Herkunft wird nur über `SftpBackend::connect_with_origin`
  übergeben; `SftpBackend::connect` gilt als interaktiv: ein getipptes Passwort oder eine getippte
  Passphrase wird für Zusatzverbindungen nie aufbewahrt, nur der SSH-Agent ist dort möglich.
- **SSH-Agent nur ohne Rückfrage** (`native/src/sftp/core/agent_grant.rs`): nur die Identität der
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
  (`native/src/sftp/core/extra_link.rs` `ExtraClient::judge`; `link_rules.rs` `same_host_key`). Die
  Ablehnung fällt vor jeder Anmeldung: an einen abgelehnten Server geht kein Passwort und keine
  Signatur. Sie beendet nur diesen einen Aufbau; die Hauptverbindung und schon arbeitende
  Zusatzverbindungen laufen weiter, die Übertragung nutzt deren Kanäle (Test
  `native/src/sftp/core/extra_link_tests.rs`).
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
  und Grund in `daemon.log` (`native/src/sftp/core/link_log.rs:1-4`, `:17-27`;
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
(`HANDLE_COPY_BUFFER`, `HANDLE_COPY_DEPTH` in `native/src/copy/os/windows.rs:148-151`). Die
Kernel-Kopie (`CopyFile2`) setzt `NO_BUFFERING`, sobald die Datei mindestens ein Viertel des
verfügbaren Speichers groß ist; ist der verfügbare Speicher unbekannt, nie
(`native/src/copy/os/shared/copy_flags.rs:25`, `:47-58`). `REQUEST_COMPRESSED_TRAFFIC` ist aus
(`copy_flags.rs:19`). Unter Linux und Android laufen parallele Bereiche mit `copy_file_range` an
festen Offsets, die Bereichsgrenzen liegen auf ganzen MiB (`native/src/copy/os/linux_range.rs:17`).
Sie laufen nur mit Erlaubnissen aus dem Flow und nur bei leerer Warteschlange, gepuffert nur auf einem
Laufwerk, das als nicht rotierend bekannt ist (`queue/rotational = 0`). Sehr große Dateien kopiert
Linux ungepuffert wie Windows mit `NO_BUFFERING`: ab einem Viertel von `MemAvailable` (unlesbar:
nie), zwischen zwei Blockgeräten und nicht dauerhaft (Verschieben, Ersetzen) öffnet se die schon
geprüften Handles über `/proc/self/fd` zusätzlich mit `O_DIRECT` (Identitätsvergleich, kein Öffnen
per Pfad) und kopiert den an der Einheit aus `statx(STATX_DIOALIGN)` (sonst 4096) ausgerichteten
Hauptteil mit 4-MiB-Anfragen auf bis zu vier Fäden; der Rest läuft gepuffert. Kann das Paar
klonen (`FICLONERANGE`), bleibt der Kernel-Klon; lehnt das Dateisystem `O_DIRECT` oder die erste
Anfrage ab, nimmt se den bisherigen Weg (`native/src/copy/os/linux_direct.rs`,
`linux_direct_plan.rs`). Meldet das Laufwerk „rotierend“ oder nichts, misst se die Direktkopie
einmal je Gerätepaar: einen Abschnitt mit einem Faden, einen gleich großen mit zwei (je
`clamp(Größe/8, 32 MiB, 64 MiB)`); bringen zwei Fäden mindestens 10 %, läuft der Rest direkt
parallel, sonst als ein gepufferter Strom; das Urteil gilt für den Prozess
(`native/src/copy/os/linux_probe.rs`, `native/src/transfer/os/shared/engine/local_plan.rs`).
Ursache: Eine gepufferte Kopie (wie `cp`) bleibt durch Vorauslesen und Writeback bei geringer
Tiefe (Suite-Lauf 3/4: rund 145 MB/s gegen 203 MB/s, die das Laufwerk mit Tiefe schafft).
Netz-, FUSE- und virtuelle Dateisysteme ohne Blockgerät bleiben bei einem Faden. Windows hat in
diesem Stapel keine parallelen Bereiche.

**Terminal.** `se cp` läuft über die Engine (`CLI_ENGINE_PATH = true`,
`native/src/cli/engine_copy.rs:44`). `se cp --stats` schreibt nach den Hinweisen eine Zeile mit
Rate und Engpass auf die Fehlerausgabe (`native/src/cli/mod.rs`, `CopyArgs`; Stand U49). Fehler einzelner Dateien
beenden den Lauf nicht; der Exitcode ist dann 1. Eine Zugriffsfrage stellt die Engine im Terminal
und im MCP-Dienst nicht: Eine lokale Quelle, für die der Zugriffsschutz nach Leserechten fragt
(nur Windows), bleibt im Bestandsweg (`access_prompt_possible`, Stand U49). Vom Bestandsweg
offen (U49): `--stats` zeigt beim Kopieren eines Verknüpfungsziels auf einem Remote 0 Bytes.

## Kompression

- **Wo.** LZ4 nur für Dateiinhalte der Share-Ströme und für `Data`-Rahmen des SSH-Agenten
  (`native/src/transfer/core/compress_policy.rs`, Modul-Kopf). Dateilisten und Pakete werden nicht
  komprimiert (TODO: LZ4 in den Paketströmen des Share fehlt; Grund: Pakete zählen Bytes je Eintrag,
  kaum Gewinn unter 2 MiB; Stand U53).
- **Wann.** Vier Bedingungen müssen zugleich gelten: (1) der Schreiber wartet in mehr als der Hälfte
  des letzten Fensters auf die Leitung; (2) ein Kern komprimiert mindestens 1,3-mal so schnell, wie
  die Leitung trägt; (3) das Verhältnis gepackt ÷ roh liegt bei höchstens 0,85; (4) ein Kern bleibt
  frei. Eine Entscheidung gilt für 8 Blöcke; das Verhältnis wird aus den ersten 64 KiB jeder Datei
  und danach aus jedem 8. Mebibyte gemessen.
- **Format.** Jeder Block ist für sich entpackbar; die Rohlänge ist höchstens 256 KiB
  (`CHUNK`, `native/src/agent_proto/core/types.rs:25`). Ein Block mit Rohlänge null oder über dem
  Maximum wird vor dem Entpacken abgelehnt (`native/src/agent_proto/core/lz4_frames.rs:45`, `:68`).
  Der Share-Rahmen trägt den Tag `TAG_DATA_LZ4` = 2 (`native/src/share/core/framing.rs:18`). Entpackt
  wird in einen Puffer genau dieser Länge.
- **Kredit.** Die Anfrage wird nach der Rohlänge berechnet, nicht nach der gepackten Länge
  (`credit_cost`, `encoded_cost`, Tag 67 in `CHARGED_TAGS`, `native/src/agent_proto/core/credit.rs`).
- **Aushandlung.** Nur in der authentisierten Sitzung, mit der Fähigkeit `lz4_v1` im Share
  (`batch_wire.rs:63`) und dem Label `lz4-v1` im Agenten (`features.rs:20`). Ohne das Label sendet der
  Client keine LZ4-Rahmen (Stand U53).
- **Schalter.** `SE_TEST_NO_LZ4` schaltet LZ4 für Messungen ab; `SE_SHARE_QUIC_CC` wählt die
  Staukontrolle, nur `bbr3` wählt BBRv3, sonst Cubic (`native/src/share/core/keepalive.rs:105`).
  Beide sind Messschalter, keine Einstellungen (Stand U53).
- **Windows.** `REQUEST_COMPRESSED_TRAFFIC` für Kopien mit einer Freigabe ist aus
  (`native/src/copy/os/shared/copy_flags.rs:19`).

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
  höchstens 2 MiB (`SMALL_FILE_CEILING`, `native/src/transfer/core/engine_policy.rs`), gehen in
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
  über die Engine (`native/src/cli/tree_engine.rs`, M16). Die Ordnervorschau der GUI liest
  lokale Wurzeln mit einem `lstat` und weckt die GUI, sobald ihr Ergebnis steht (M12). Eine
  Ordnerreservierung über eine Stufe schreibt das „veröffentlicht“ nach dem Umbenennen nicht mehr
  dauerhaft für sich; nach einem Absturz bestätigt die gespeicherte Identität der Stufe das Ziel
  (`engine/folder_reservation.rs`, M13).
- **Server-Backends (KP3).** SFTP- und SMB-Paket-Downloads holen auf einem eigenen Faden, die Senke
  der Engine läuft außerhalb der Laufzeit (G3-01). SFTP schickt CLOSE direkt hinter das letzte
  WRITE, FSTAT/LSTAT als Beleg mit und RENAME zusammen mit LSTAT (G3-02/G3-03/G3-06). WebDAV
  verschiebt eine mit `If-None-Match: *` bestätigte Stufe ohne PROPFIND davor (G3-12). Google Drive
  drosselt neue Dateien erst nach einer Ablehnung (G3-08a). FTP kann die Auflistungen der Ordner,
  in die ein Auftrag veröffentlicht, behalten, solange der Auftrag läuft (`vfs::transfer_scope`,
  G3-04b); die Engine hält den Rahmen, das FTP-Backend gibt ihn noch nicht zurück (offen). Jede
  Veröffentlichung prüft den Zielnamen weiterhin einzeln.
- **Unverändert:** Ein Einzel-Download prüft die Quelle nach dem Lesen weiter mit einem `stat`,
  auch wenn der Host beim Öffnen einen Beleg lieferte: Der Beleg stammt von vor dem Lesen und
  beweist nicht, dass die Datei während des Lesens gleich blieb.

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
nur in solchen Builds wirken (`native/src/share/core/transfer_meter.rs:23-27`). Der Messdienst und
das Messprogramm sprechen über den IPC-Port des Dienstes (`daemon.ipc`); diese Verbindung bleibt von
der netem-Regel auf `lo` ausgenommen. Dann ist die Wurzel ein `prio` mit zwei Bändern: netem liegt auf Band
1:1, die IPC-Ports des Dienstes gehen per u32-Filter auf das ungeformte Band 1:2
(`native/rdt_task/speed_net.py:44`, `:107-124`, `:127-137`).

**Share-Statistik (`share-quic-stats.jsonl`).** Jede Share-Verbindung schreibt je Übertragungsstoß
eine Zeile in `share-quic-stats.jsonl` im App-Datenordner (`native/src/share/core/quic_stats_hub.rs:36`,
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

Lesehilfe für `burst` (`native/src/share/core/quic_stats.rs:205-245`, Ableitung in
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
