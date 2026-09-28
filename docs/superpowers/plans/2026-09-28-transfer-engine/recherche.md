# Übertragungs-Engine – Recherche und Durchsatz-Analyse

Stand: 2026-09-28. Grundlage: sechs Code-Lesungen `docs/lesungen/2026-09-28-*.md`, Crate-Quellen im
lokalen Registry (`windows 0.58.0`, `iroh 1.0.1`, `russh-sftp 2.3.0`, `ureq 2.12.1`), Refs
`docs/refs/windows-virtual-files.md`, `docs/refs/gdrive-ureq-throughput.md`,
`docs/refs/quic-sftp-throughput.md` (API-Syntax, jeweils mit Quellen).

**Ziel (nicht Mindestanforderung):** so viele Mbit/s, wie Leitung, Gegenstelle und Datenträger
hergeben – für eine große Datei wie für eine Million kleiner Dateien –, gemessen vom Einfügen bis zur
letzten Datei am Ziel. Jede Zahl unten folgt aus einer Protokoll-, Plattform- oder Ressourcengrenze
oder wird zur Laufzeit gemessen; es gibt keine geratenen Sicherheitsdeckel.

## 1 Durchsatzmodell

- Leitung: Bandbreite `B`, Umlaufzeit `R`. Ein einzelner Strom schafft höchstens `min(B, W/R)`, wobei
  `W` das wirksame Fenster ist (TCP-Autotuning, QUIC-Strom-/Verbindungsfenster, SSH-Kanalfenster).
- Je Datei fallen feste Kosten `k·R + t_s` an (`k` Protokoll-Roundtrips, `t_s` Serverarbeit). Bei
  kleinen Dateien (Größe `s`) dominiert sie: ein Strom schafft `s/(k·R + t_s)`.
- Mit `N` gleichzeitigen Operationen und `m` Dateien je Operation (Paket) gilt grob
  `Durchsatz ≈ min(B, N·m·s/(k·R + t_s + m·s/B), Serverkapazität, Datenträger)`.
- Folgerungen, in dieser Reihenfolge: (1) `k` senken (keine Proben/`mkdir`/`stat` je Datei, kein
  TLS-Handshake je Aufruf, Pakete statt Einzeldateien), (2) Fenster ≥ `B·R` für große Dateien,
  (3) `N` so groß wie nötig, um `B` zu füllen – aber nicht größer, weil zu viele parallele Ströme
  Server, Festplatte (HDD-Kopfsprünge) oder Ratenlimits überlasten. `B`, `R`, Serverkapazität und
  Datenträger sind vorab unbekannt und schwanken ⇒ `N` wird **gemessen geregelt**, nicht festgelegt.

## 2 Parallelität: adaptive Regelung je Verbindung (Begründung statt Deckel)

- **Warum adaptiv:** Die richtige Parallelität hängt von `B·R`, Dateigröße, Serverkapazität und
  Datenträger ab: LAN-SSD mit kleinen Dateien profitiert von 16–64 gleichzeitigen Operationen, eine
  HDD bricht schon bei 4 ein, Google Drive drosselt ab einer Anfragerate, ein Relay limitiert die
  Bandbreite, FTP hat nur so viele Verbindungen, wie der Server zulässt. Keine feste Zahl ist für alle
  Fälle richtig; jede feste Zahl verschenkt Durchsatz oder überlastet.
- **Verfahren (Nutzleistungs-Bergsteigen, AIMD bei Überlast):** Je Verbindung (gemeinsam für alle
  Übertragungen, Sync und Explorer-Streams auf ihr) wird die Grenze `L` gleichzeitiger Operationen
  geregelt. Gemessen wird die Nutzleistung je Messfenster (≥ 1 s, mindestens zwei mittlere
  Operationsdauern): übertragene Bytes plus 64 KiB je abgeschlossener Operation (damit auch reine
  Kleindatei-Last messbar ist). Start `L = 2`; solange jede Verdopplung ≥ 10 % mehr bringt, wird
  verdoppelt (Slow Start); danach wird periodisch um ±1 geprobt und nur behalten, was ≥ 5 % bringt.
  Angepasst wird nur, wenn `L` im Fenster tatsächlich ausgeschöpft war (sonst fehlt Nachfrage, nicht
  Parallelität). Überlastsignale (Zeitüberschreitung, Verbindungsabbruch, HTTP 429/503, Drive
  `rateLimitExceeded`, „too many concurrent requests“, FTP 421) halbieren `L` sofort und sperren
  Erhöhungen für einige Fenster.
- **Harte Obergrenzen nur aus echten Grenzen:**
  - FTP: Anzahl Steuerverbindungen, die der Server annimmt (Pool wächst, bis der Server 421/530
    meldet; die Grenze wird gelernt).
  - Share über den Hintergrund-Dienst (Desktop/Android): Der Dienst nimmt je IPC-Verbindung höchstens
    `MAX_REQUEST_WORKERS` Anfragen an (heute 16, ein Thread je Anfrage). Die Peer-Verbindung erlaubt 64
    gleichzeitige QUIC-Ströme. ⇒ Dienst-Grenze auf 64 anheben (gleiche Größenordnung wie die
    QUIC-Grenze; Threads sind billig), vier davon bleiben für Blättern/Mounts reserviert. Alte Dienste
    (16) melden Überlast → Regelung passt sich an.
  - SSH-Agent: Agent nimmt `MAX_ACTIVE_REQUESTS` an (heute 8) ⇒ im neuen Agent 64, Label im Hello;
    alte Agents werden über ihr Label erkannt (Grenze 8, 2 für Blättern reserviert).
  - Speicher/Threads: jede laufende Operation hält einen Worker-Thread und bis zu 1 MiB Puffer
    (Pakete bis 16 MiB). Die Regelung stoppt, sobald mehr Parallelität keine Nutzleistung mehr
    bringt; als Schutz gegen Messrauschen gilt je Verbindung höchstens 256 laufende Operationen
    (256 Threads, ≤ 256 MiB Puffer im Extremfall) – oberhalb davon ist bei einer Verbindung kein
    Gewinn möglich, weil sich alle Ströme ein Überlastfenster teilen.
- **Lokale Datenträger:** Regelung je Volume (Windows Laufwerk/Volume, Unix `st_dev`), damit SSD und
  HDD je eigene Grenze bekommen.
- **Zwei Verbindungen (Remote→Remote):** Eine Operation braucht je eine Erlaubnis beider Seiten;
  Anforderung in fester Reihenfolge (nach Schlüssel), damit sich Übertragungen in Gegenrichtung nicht
  verklemmen.

## 3 Je Protokoll: was es erlaubt, Grenzen, Maßnahmen

### 3.1 Direct/Raum-Share (Iroh/QUIC, eigenes Protokoll; Desktop/Android über den Hintergrund-Dienst)
- **Erlaubt:** viele unabhängige QUIC-Ströme je Verbindung (Host: 64), Stromöffnung ohne Roundtrip auf
  warmer Sitzung, eigene Flusskontrolle je Strom; Protokoll und beide Enden gehören uns ⇒ Pakete
  möglich.
- **Grenzen heute:** Strom-Empfangsfenster auf Standard (noq: für 100 Mbit/s bei 100 ms ausgelegt)
  ⇒ ein Strom schafft höchstens Fenster/RTT; je Datei 4–6 Roundtrips (Stage-Probe, WriteNew, WriteDone,
  Stat, RenameNoReplace, `mkdir_all`); Dienst 16 Anfragen je IPC-Verbindung; GetTree/PutTree sammeln
  und puffern den ganzen Baum.
- **Maßnahmen:**
  - Fenster auf Bandbreite×RTT großer Leitungen: Strom-Empfangsfenster 16 MiB (1 Gbit/s × 100 ms ≈
    12,5 MB), Verbindungs-Empfangs- und Sendefenster 64 MiB (mehrere schnelle Ströme gleichzeitig).
    Speicher wird nur belegt, wenn die Anwendung langsamer liest als das Netz liefert. Wirkt je
    Empfangsrichtung: Downloads profitieren sofort, Uploads, sobald die Gegenstelle aktualisiert ist.
  - Pakete für kleine Dateien (`PutBatch`/`GetBatch`): ein Strom trägt viele Dateien, Kosten je
    Paket ≈ 1 Roundtrip. Paketgröße adaptiv: Ziel ≈ 250 ms Übertragungszeit bei gemessener Rate
    (mindestens 256 KiB, höchstens 16 MiB), höchstens 256 Dateien (erste Steuer-Frame-Grenze
    256 KiB). Dateien über der Paketgrenze gehen einzeln, parallel.
  - Einzeldatei-Weg ohne Stage-Probe und ohne `mkdir` je Datei (Ordner einmal); Hosts ohne
    Paket-Fähigkeit (Capabilities-Flag fehlt) bekommen diesen Weg.
  - Kopie innerhalb desselben Shares: serverseitig (`CopyFile` in private Stufe + No-Replace), Frist
    der Anfrage nach Dateigröße (Host kopiert lokal; kein Fortschritts-Frame im Protokoll).
  - Dienst-Grenze 64 Anfragen (s. §2).
- **Werkzeug ausreichend?** Ja. QUIC/Iroh trägt volle Bandbreite, wenn Fenster und Roundtrips stimmen;
  Relay-Bandbreite ist eine externe Grenze (Anzeige „über Relay“).

### 3.2 SSH-Agent (`se-agent`, Agent-Frames über einen SSH-Kanal)
- **Erlaubt:** Multiplexing vieler Anfragen, Server schiebt Lesedaten ohne Anfrage je Block,
  Schreiben ohne Quittung je Block.
- **Grenzen:** 8 gleichzeitige Anfragen im Agent; je Datei 3 Roundtrips; SSH-Kanalfenster.
- **Maßnahmen:** Agent nimmt 64 Anfragen an; dieselben Paket-Frames wie der Dienst (lokales Dateisystem
  auf dem Server); Fähigkeit per Hello-Label (`+batch-v1`), alte Agents bekommen den Einzeldatei-Weg.
  Die Agent-Binärpakete werden im Release neu gebaut (bestehender Ablauf), der Test baut sie aus der
  Quelle.

### 3.3 SFTP ohne Agent (russh 0.61 + russh-sftp 2.3, meist OpenSSH `sftp-server`)
- **Erlaubt:** Anfragen mit IDs, beliebig viele gleichzeitig auf einem Kanal (Pipelining); mehrere
  SFTP-Kanäle je SSH-Verbindung (OpenSSH `MaxSessions`, Standard 10); READ/WRITE bis ≈ 255 KiB
  (`limits@openssh.com`).
- **Grenzen heute:** `File::poll_read` stellt genau eine READ-Anfrage und wartet ⇒ eine Datei lädt mit
  ≈ 255 KiB je RTT (bei 50 ms ≈ 5 MB/s); Schreiben ist bereits gepipelined; ein Kanal ⇒ ein
  `sftp-server`-Prozess (eine CPU) und ein Kanalfenster für alles.
- **Maßnahmen:**
  - Lesen mit vielen ausstehenden READs über `RawSftpSession` auf einem eigenen Kanal
    (wie `posix_rename.rs`), Tiefe = Kanalfenster / READ-Länge.
  - Kanal-Pool: weitere SFTP-Kanäle bei Bedarf (Regelung), bis der Server Kanäle ablehnt (gelernte
    Grenze, meist `MaxSessions`); Dateien werden über Kanäle verteilt.
  - SSH-Kanalfenster des Clients (Downloads) groß genug für Bandbreite×RTT.
- **Werkzeug ausreichend?** Ja, mit der Raw-API; der High-Level-`File` allein genügt fürs Lesen nicht.

### 3.4 FTP/FTPS (suppaftp)
- **Erlaubt:** mehrere Anmeldungen je Nutzer (Serverkonfiguration, übliche Voreinstellungen erlauben
  mehrere Verbindungen je IP); je Steuerverbindung eine Übertragung.
- **Grenzen heute:** eine Steuerverbindung für alles ⇒ streng seriell; ein offener Leser blockiert
  jede andere Operation (Deadlock-Gefahr bei gleichzeitigem Lesen und Schreiben).
- **Maßnahmen:** Verbindungs-Pool im Backend: Operationen leihen sich eine Verbindung; der Pool wächst
  auf Anforderung der Regelung und lernt die Servergrenze (421/530); damit auch Lesen+Schreiben auf
  demselben Server ohne Temp-Umweg, sobald ≥ 2 Verbindungen bestehen.
- **Werkzeug ausreichend?** Ja (eine Verbindung = eine Übertragung ist protokollbedingt; mehrere
  Verbindungen sind der Standardweg schneller FTP-Clients).

### 3.5 WebDAV (ureq, HTTP/1.1)
- **Erlaubt:** viele parallele HTTP-Verbindungen, Keep-Alive, bedingtes PUT, serverseitiges COPY/MOVE.
- **Grenzen heute:** Mutationen öffnen je Anfrage eine neue TCP+TLS-Verbindung (+2–3 RTT); der
  Schreiber puffert die ganze Datei in einer Temp-Datei und lädt erst beim Abschluss hoch; keine
  Behandlung von 429/503.
- **Maßnahmen:** gepoolte Verbindungen für alle Mutationen, die ureq nie wiederholt (PUT mit Inhalt,
  MOVE, MKCOL, COPY); DELETE und PUT ohne Inhalt bleiben ungepoolt (ureq würde sie nach einem
  Verbindungsverlust wiederholen). Bekannte Größe ⇒ PUT direkt streamen (Content-Length, kein
  Temp-Puffer); 429/503 mit `Retry-After`/Backoff wiederholen, wo nachweislich nicht verarbeitet.
- **Werkzeug ausreichend?** Ja; HTTP/2 brächte Multiplexing, parallele Verbindungen erreichen
  denselben Durchsatz.

### 3.6 Google Drive (ureq, REST v3)
- **Erlaubt:** viele parallele Verbindungen; `files.generateIds` liefert bis zu 1000 IDs je Aufruf;
  Multipart-Upload (ein Aufruf) für kleine Dateien, Resumable für große; Batch-Endpunkt für
  Metadaten (u. a. Ordneranlage).
- **Grenzen heute:** jeder Metadaten-Aufruf mit neuem Agent ⇒ neuer TCP+TLS-Handshake; Pfad-Cache wird
  nach fast jeder Operation komplett (hübsch formatiert) unter beiden Cache-Sperren neu geschrieben ⇒
  beim Scan großer Ordner (Sync-Erstlauf!) quadratischer Aufwand und faktische Serialisierung;
  globale Sperren für jede Ordneranlage und jede Veröffentlichung; je Datei ein `generateIds`; immer
  Resumable (2 Aufrufe) auch für Kleinstdateien; Backoff ohne Zufallsanteil.
- **Maßnahmen:** ein gepoolter Agent für alle API-Aufrufe (ureq wiederholt POST/PATCH mit Inhalt nie
  automatisch – verifiziert in `ureq-2.12.1/src/unit.rs`); Cache-Persistenz entkoppelt (als „schmutzig“
  markieren, Hintergrund-Schreiber höchstens alle paar Sekunden, Serialisierung außerhalb der Sperren,
  kompaktes JSON; synchron nur, wo die Journal-Logik es verlangt); Sperren je Ordner/Zielname statt
  global; ID-Vorrat per `generateIds count=…`; Multipart bis zur dokumentierten Grenze, darüber
  Resumable mit größeren Blöcken; Backoff mit Zufallsanteil; Ratenlimit-Signale in die Regelung.
- **Werkzeug ausreichend?** Ja (Quota ist die externe Grenze; siehe Ref).

### 3.7 SMB (smb2, im UI nur Android)
- **Erlaubt:** Anfragen-Multiplexing mit Credits, große READ/WRITE (serverseitige MaxRead/WriteSize).
- **Grenzen heute:** je Datei ein ausstehender 1-MiB-Block; Laufzeit mit 2 Worker-Threads.
- **Maßnahmen:** mehrere ausstehende Blöcke je Datei im Rahmen der Credits; Laufzeit-Threads nach
  Kernzahl; Regelung.

### 3.8 Lokal (std::fs; Windows NTFS/ReFS, Linux ext4/btrfs/xfs, Android FUSE)
- **Erlaubt:** Kernel-Kopie (`CopyFileExW`: inkl. Server-Offload auf SMB-Freigaben, Block-Cloning auf
  ReFS; Linux `copy_file_range`: Reflink auf btrfs/xfs, Server-Kopie auf NFS), parallele E/A auf
  SSD/NVMe, Umbenennen ganzer Ordner auf demselben Volume.
- **Grenzen heute:** Vorab-Scan, Probe-Öffnen jeder Datei, Kopierschleife im Userspace, zwei `fsync`
  je Datei, seriell, Verschieben Datei für Datei.
- **Maßnahmen:** `std::fs::copy` in die private Stufe (Kernel-Kopie; Rückfall auf die Leseschleife mit
  `local_access` bei geschützten Dateien); kein `fsync` für neue Kopien (Explorer-/cp-Verhalten; die
  Quelle bleibt unberührt, Verschieben/Überschreiben behalten volle Synchronisation); Verschieben auf
  demselben Volume als eine Umbenennung des ganzen Eintrags; adaptive Parallelität je Volume.
- **Plattformfragen:** Windows: Defender prüft jede neue Datei beim Schließen – Parallelität verdeckt
  diese Latenz; lange Pfade/Win32-Sondernamen weiter über `local_access` (verbatim). Android: `/sdcard`
  läuft über FUSE (jede Metadaten-Operation ein Kontextwechsel) – Parallelität und das Weglassen
  unnötiger `stat`s helfen dort besonders; `rename_no_replace` über die `android_fs`-Kette.

### 3.9 ZIP (nur Quelle)
- Jeder Lesevorgang öffnet das Archiv und entpackt in den Speicher ⇒ parallel lesbar, CPU-gebunden;
  Regelung wie oben.

## 4 Explorer-Einfügen/-Ziehen aus Remotes (Windows)

- Der Explorer verlangt die vollständige Dateiliste (`FILEGROUPDESCRIPTORW`) vor dem Kopieren. Die
  Liste entsteht erst, wenn der Explorer einfügt (paralleles Auflisten über die Regelung), nie beim
  Kopieren; keine Größengrenze. Einträge sind 592 Bytes groß; der Speicherbedarf wächst linear.
- Einzige echte Grenze: `cFileName` fasst 260 UTF-16-Zeichen (relativer Pfad). Längere Pfade kann der
  Explorer auf diesem Weg nicht empfangen; sie werden in der App gemeldet (Übertragungsliste und
  Fehler-Protokoll, mit Hinweis „in Smart Explorer einfügen überträgt sie“).
- Der Explorer holt Inhalte Datei für Datei nacheinander. Damit viele kleine Dateien trotzdem schnell
  sind, lädt das Datenobjekt die nächsten Dateien in Listenreihenfolge parallel voraus (Speicherbudget
  statt Dateizahl; große Dateien werden gestreamt).
- Das Objekt lebt auf einem eigenen STA-Thread mit Nachrichtenschleife; Netz-E/A blockiert nie die
  Oberfläche. Ziehen in den Explorer nutzt dasselbe Objekt (Proxy im GUI-Thread) mit
  `IDataObjectAsyncCapability`, sodass der Explorer im Hintergrund kopiert.
- Die App zeigt diese Explorer-Übergaben in der Übertragungsliste (gelieferte Dateien/Bytes).

## 5 Sync (Einweg-Spiegeln, Zwei-Wege, Jobs)

- Einweg-Spiegeln (`sync::start_sync`) kopiert heute streng nacheinander mit zwei zusätzlichen
  `stat` auf der Quelle je Datei; bei Drive + neuen TLS-Verbindungen + Cache-Neuschreiben ⇒ die
  gemeldete Langsamkeit beim ersten Herunterladen. Maßnahme: Kopieren parallel zum Scannen über
  dieselbe Regelung, Ordner einmal anlegen, Semantik (Aktualisieren nach Größe/Zeit, Validierungen,
  Auslassungen, Löschdurchlauf erst nach fehlerfreiem Kopierdurchlauf) unverändert.
- Zwei-Wege/Jobs (`bisync`) laufen schon parallel, festgelegt über `min(parallelism)`; sie nutzen die
  Regelung statt der festen Zahl (die Nutzeroption „max. Übertragungen“ bleibt Obergrenze); die
  Sicherheitslogik je Aktion (Erfassen, Revalidieren, Sicherung, Konfliktkopie) bleibt unverändert.
  Drive-Verbesserungen (§3.6) wirken auf Scan und Anwendung.

## 6 Sicherheit/Kompatibilität (bleibt erhalten)

- Nie ersetzen ohne ausdrückliche Wahl; private Stufe + No-Replace-Veröffentlichung; lokale `.part`
  werden bei Fehlern entfernt, entfernte Stufen nur, wo Eigentum sicher ist (wie bisher).
- Quelländerung/-wachstum erkennen; Links/Reparse-Punkte nicht verfolgen; App-Papierkorb auslassen;
  Win32-Sondernamen über verbatim-Pfade; geschützte lokale Lesezugriffe mit genau einer Rechteanfrage.
- Wiederholung einer Datei nur, solange nichts veröffentlicht wurde; nach mehrdeutigem Fehler beim
  Veröffentlichen keine Wiederholung (sonst Duplikat „Name (2)“).
- Protokoll-Erweiterungen nur nach Fähigkeitsnachweis der Gegenstelle (Share: Capabilities-Flag;
  Agent/Dienst: Hello-Label); alte Gegenstellen bekommen den bisherigen Weg.
