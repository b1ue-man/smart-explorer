# Übertragungs-Engine

Stand: 2026-09-29 (Batch „Übertragungs-Engine“, Plan
`docs/superpowers/plans/2026-09-28-transfer-engine/`). Beschreibt, wie Kopieren, Einfügen,
Ziehen, „Kopieren/Herunterladen nach…“ und Downloads zwischen lokalen Ordnern, Remotes
(SFTP, FTP/FTPS, WebDAV, SMB, Google Drive, ZIP), Direct-/Raum-Shares und dem SSH-Agent
laufen, welche Grenzen jedes Protokoll setzt und woher jede Zahl kommt.

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
- `native/src/transfer/os/shared/walk*.rs`: Walker. Ausgewählte Einträge werden parallel per
  `stat` geprüft; liegen in einem Ordner mehr ausgewählte Einträge, als gleichzeitig laufen
  dürfen, beantwortet eine Auflistung des Ordners alle. Links, Spezialdateien und ungültige
  Namen sind gemeldete Auslassungen, der App-Papierkorb eine stille Auslassung.
- `native/src/transfer/os/shared/flow.rs` + `core/flow_control.rs`: Flows (siehe unten).
- `native/src/transfer/os/shared/{access,memory}.rs`: eine Rechteanfrage je Übertragung für
  geschützte lokale Ordner; prozessweites Speicherbudget für gepufferte Bytes.
- `native/src/transfer/os/shared/lane.rs`: Lane ohne feste Grenze; fertige Übertragungen liefern
  Job, erste Fehler und aufgelöste Zielwurzeln für „Fehlende übertragen“.

## Parallelität: Flows statt fester Grenzen

Die richtige Zahl gleichzeitiger Operationen hängt von Bandbreite × Latenz, Dateigrößen,
Serverleistung und Speichermedium ab – nichts davon ist vorher bekannt. Jede Verbindung (bzw.
jedes lokale Volume) hat deshalb **einen** Flow, den alle Übertragungen, Explorer-Übergaben und
Sync-Läufe dieses Prozesses teilen; er misst die Nutzleistung (Bytes plus 64 KiB je fertiger
Operation, damit reine Kleinstdatei-Last messbar ist) und regelt die Grenze:

| Parameter | Wert | Grund |
|---|---|---|
| Startgrenze | 2 | eine Operation in Arbeit, eine im Anlauf |
| Messfenster | 2 × geglättete Latenz, 1–5 s | lang genug gegen Rauschen, kurz genug zum Reagieren |
| Slow Start | verdoppeln, solange ≥ +10 % | findet große Grenzen in wenigen Fenstern |
| Proben | Schritt Grenze/8, über 2 Fenster | symmetrische Schritte halten die Grenze unter Rauschen am Optimum (Simulation mit ±10 %: ≥ 93 % der Bestleistung bei ≤ 1,11-facher Parallelität) |
| Behalten | aufwärts ≥ +5 %, abwärts ≥ 97 % | nur Änderungen, die sich lohnen |
| Überlast | Grenze halbieren, einmal je Fenster | Ratenlimits, „busy“, Timeouts (typisiert als `vfs::congestion_error`) |
| Obergrenze | 256 bzw. Protokollgrenze (`transfer_ceiling`) | Threads/Puffer je Operation; echte Grenzen kommen vom Protokoll |
| Merken | 15 min nach letzter Nutzung | das nächste Einfügen beginnt, wo das letzte endete |

Regeln: Erlaubnisse gehen reihum zwischen Übertragungen (ein kleines Einfügen neben einem
großen Job kriecht nicht); Auflistungen haben einen reservierten Platz; Lokal↔Remote regelt nur
der Remote-Flow; nie mit gehaltener Erlaubnis auf etwas warten, das selbst Erlaubnis, Speicher
oder einen Kanal braucht; Speicher wird vor der Erlaubnis reserviert. Flows sind prozesslokal:
Sync-Jobs im Hintergrund-Dienst regeln getrennt von der GUI.

## Je Protokoll

| Protokoll | Was es erlaubt | Was die Engine nutzt | Grenzen und Zahlen |
|---|---|---|---|
| Lokal, UNC | Kernel-Kopie, Server-Offload | Kopie über die exklusiv erzeugte Stufe (Linux `copy_file_range` zwischen Handles, Windows `CopyFile2` „nicht ersetzen“ + Identitätsprüfung); innerhalb einer UNC-Freigabe kopiert so der SMB-Server selbst (Offload/COPYCHUNK); kein fsync bei neuen Kopien, Verschieben auf demselben Volume als eine Umbenennung | fsync nur bei Verschieben und Überschreiben; Abbrechen wirkt zwischen den Kopierblöcken |
| Direct/Raum-Share (Iroh/QUIC) | viele Ströme je Verbindung | `fs_transfer_v1`: Pakete für kleine Dateien (Statusabfrage bei verlorener Antwort), Lesen ab Offset, exklusive Ordner, serverseitige Kopie; alte Hosts: bisheriger Weg | 64 Ströme − 4 fürs Blättern = 60 Übertragungen je Gegenstelle (Host-Zulassung), davon nutzt der Flow höchstens 56, 4 bleiben für Vordergrund-Lesen; Host gesamt 256 (halbe Tokio-Blocking-Threads); alte Hosts 32; Paket ≤ 256 Dateien/16 MiB, Kopf ≤ 256 KiB; Fenster 16 MiB je Strom (1 Gbit/s × 100 ms), Verbindung 16–64 MiB je nach Speicher; voller Host antwortet „busy“ statt heimlich zu warten, alte Clients warten höchstens 45 s (ihre Frist ist 60 s) |
| SSH-Agent / Hintergrund-Dienst | Anfragen mit IDs über einen Kanal | Kredit je Anfrage (`+credit-v1`: ein langsamer Download blockiert kein Blättern), Pakete (`+batch-v1`), Stufen-Frames, Pool zusätzlicher Exec-Kanäle | 64 MiB Budget je Verbindung, 1 MiB Anfangskredit, Fenster bis 8 MiB je Strom; Kanal-Pool bis MaxSessions − 2 |
| SFTP | gepipelinete Anfragen, mehrere Kanäle | viele gleichzeitige READs mit wachsender Tiefe, gebündelte WRITEs, Kanal-Pool, Kopien ohne fsync, serverseitige Kopie per `copy-data` wo angeboten (in Bereichen von ~10 s, lehnt der Server ab, wird gestreamt) | Tiefe ≤ Fenster/Block = 64, SSH-Fenster 16 MiB, Paket 32 KiB; Pool bis zur Ablehnung, 2 Kanäle Reserve |
| FTP/FTPS | eine Übertragung je Steuerverbindung | Verbindungs-Pool, Größen-Stufe mit streamendem STOR, Fortsetzen per REST | Grenze lernt der Pool aus 421/530 nach erfolgreicher Anmeldung; eine Verbindung bleibt fürs Blättern |
| WebDAV | viele HTTP-Verbindungen, COPY | gepoolte Mutationen, gestreamtes PUT mit Länge und `If-None-Match: *`, serverseitiges COPY, Range | 429/503 → Überlast mit `Retry-After`; DELETE und leeres PUT ungepoolt (ureq würde sie wiederholen) |
| Google Drive | parallele Verbindungen, `generateIds` | gepoolte Verbindungen, neue Datei ≤ 5 MB mit einem Aufruf, Resumable mit wachsenden Blöcken, ID-Vorrat, Cache im Hintergrund, Sperren je Name, `files.copy` | Google begrenzt dauerhaft ~3 neue Dateien/s je Konto (Hinweis in der Zeile); Ratenlimits sofort als Überlast |
| SMB | Credits, COPYCHUNK | mehrere Lese-Blöcke gleichzeitig, serverseitige Kopie wo angeboten, neue Kopien ohne FLUSH | Block ≤ 512 KiB bzw. MaxReadSize, Tiefe nach Credits; eine Serverkopie sieht Abbrechen erst an ihrem Ende |
| ZIP (Quelle) | – | ein Parse je Archiv, Einträge gestreamt | Parallelität = Kernzahl (Entpacken ist CPU-gebunden) |

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

## Was erhalten bleibt

Stufen + Veröffentlichen ohne Ersetzen, keine halbfertigen Dateien unter dem Zielnamen,
Quelländerungen werden erkannt (Länge, bei Drive MD5, sonst nachgelagertes `stat`), keine
blinde Wiederholung mehrdeutiger Mutationen, Links werden nie gefolgt, der App-Papierkorb wird
ausgelassen, geschützte lokale Ordner werden genau einmal angefragt. Sync, Laufwerks-Mounts und
Ersetzen behalten ihre dauerhaften Schreibwege.

## Nachweis

Die Task-Suite `native/test-transfer-engine-task.sh` (Workflow `transfer-engine-task.yml`):
Meilenstein-Tests (`transfer_engine_task_`), Copy/Paste-Sicherheitstests über die Engine,
Tests aller betroffenen Module, ganze Bäume gegen SFTP- und FTP-Container, Durchsatz über eine
auf 50 ms/100 Mbit/s geformte Loopback-Leitung (`native/tests/transfer_throughput.rs`),
Share-Raum Ende-zu-Ende mit echten Binaries, Windows-Meilensteine (OLE-Rundreise) und die
Android-Gerätesuite. Nicht gemessen: echte Netze der Nutzer, das UI-Verhalten des Explorers
beim Überfahren mit großen Auswahlen (siehe `docs/refs/windows-virtual-files.md` §9).
