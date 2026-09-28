# Übertragungs-Engine: sofort, schnell, nachvollziehbar – Spec

Stand: 2026-09-28. Auftrag (sinngemäß): Kopieren/Einfügen und Herunterladen über Remotes, Geräte,
Direct-/Raum-Shares und Google Drive sind für Dateien und besonders Ordner unbrauchbar langsam.
Egal ob 1 MB oder 1 TB: Nach dem Einfügen muss sofort etwas passieren – kein ewiges „Einfügen wird
vorbereitet“, kein vorheriges Scannen von allem, kein Verpacken von allem. Ziel ist die beste
Gesamt-Übertragungsrate vom Einfügen bis zur letzten Datei am Ziel, intuitiv, zuverlässig,
nachvollziehbar – ohne andere Funktionen zu beschädigen.

Befunde, die diese Spec begründen: `docs/lesungen/2026-09-28-*.md` (sechs Lesungen) und der Code
(`native/src/transfer`, `copy`, `app/os/shared/clipboard*.rs`).

## Ist-Zustand (Ursachen)

- **Kopieren im Remote-Tab lädt alles herunter:** Strg+C auf Remote-Ordnern lädt den ganzen Baum in
  einen Temp-Ordner (Direct/Raum-Share sogar als ganzer Baum „gepackt“ und erst nach dem Ende
  veröffentlicht); Einfügen wird bis dahin abgewiesen („bitte danach erneut einfügen“) und kopiert
  danach ein zweites Mal aus Temp.
- **Jeder Transfer scannt erst alles:** Upload, Download und Remote→Remote sammeln den kompletten
  Baum, bevor das erste Byte fließt; Remote-Quellen zusätzlich mit einem `stat` je Eintrag.
- **Streng nacheinander, viele Roundtrips je Datei:** pro Datei `mkdir_all` des Elternordners (auf
  FTP/WebDAV/SMB ein Roundtrip je Ordnerebene), Stufen-Namensprobe, Öffnen, Abschluss, Stat,
  Umbenennen – über Share etwa 6 Netz-Roundtrips pro Datei, alles sequenziell.
- **Remote→Remote über eine lokale Temp-Datei**, auch zwischen verschiedenen Servern.
- **Google Drive:** ~9 API-Aufrufe pro Datei; zwei globale Sperren serialisieren jede Ordneranlage
  und jede Veröffentlichung prozessweit über den Netz-Roundtrip; die komplette Pfad-Cache-Datei wird
  nach fast jeder Operation neu geschrieben.
- **Lokal→Lokal:** kompletter Vorab-Scan, dann jede Datei einmal zur Probe geöffnet, dann
  sequenziell mit zwei `fsync` je Datei; Verschieben auf demselben Laufwerk Datei für Datei.
- **Linux:** gar keine Datei-Zwischenablage (Strg+C/V zwischen Tabs geht nicht).
- **Rückmeldung:** kleine Statuszeilen-Chips; Fehlerdetails einer Übertragung gehen bis auf ein
  Beispiel verloren.

## A Definition

- **F1 Kopieren sofort.** Strg+C / „Kopieren“ (Tabelle, Kontextmenüs, lokal und jedes Remote inkl.
  Drive, SFTP, FTP, WebDAV, SMB, Direct/Raum-Share, ZIP) merkt sich nur die Auswahl – kein Download,
  kein Scan. Ausschneiden (Strg+X) wie bisher nur lokal; Remote-Ausschneiden bleibt mit klarer
  Meldung abgelehnt (Quellen unverändert).
- **F2 Einfügen startet sofort.** Strg+V / „Einfügen“ in einem lokalen oder Remote-Ordner startet
  die Übertragung sofort. Unterordner werden parallel zum Kopieren gesucht; die ersten Dateien kommen
  in der ersten Sekunde an. Gilt für jede Kombination Lokal/Remote/Share/Drive, auch zwischen zwei
  verschiedenen Remotes. Auf Linux funktioniert Kopieren/Einfügen innerhalb der App jetzt ebenfalls.
- **F3 Austausch mit dem Explorer (Windows).** In Smart Explorer kopierte Remote-Dateien lassen sich
  im Windows-Explorer einfügen und aus der App in den Explorer ziehen, ohne Vorab-Download: der
  Explorer holt die Inhalte bei Bedarf als virtuelle Dateien und zeigt dabei seinen eigenen
  Kopierdialog. Keine Größengrenze: die Liste entsteht beim Einfügen, die nächsten Dateien werden
  parallel vorausgeladen. Lokale Auswahl weiter als normale Dateien (wie bisher). Im Explorer kopierte Dateien
  werden in der App wie bisher eingefügt (jetzt ebenfalls sofort startend).
- **F4 Ziehen und Ablegen, „Herunterladen nach…“, „Kopieren/Verschieben nach…“** laufen über
  dieselbe Engine (sofortiger Start, gleiche Anzeige). „Herunterladen nach…“ akzeptiert jetzt auch
  ein Remote-Ziel aus der Ordnerauswahl (wird dann Remote→Remote).
- **F5 Höchster Gesamtdurchsatz** (Ziel: volle verfügbare Bandbreite; Analyse `recherche.md`).
  - Parallelität je Verbindung gemessen geregelt, gemeinsam für alle Übertragungen, Sync und
    Explorer-Übergaben auf derselben Verbindung; harte Grenzen nur aus Protokoll/Dienst/Server.
  - Zielordner je Ordner genau einmal angelegt; keine Proben, kein `mkdir`, kein `stat` je Datei.
  - Remote→Remote direkt gestreamt; Temp-Umweg nur, wo eine Verbindung Lesen und Schreiben nicht
    gleichzeitig kann. Kopie innerhalb desselben Shares/Servers serverseitig, wo der Server das kann.
  - Direct/Raum-Share und SSH-Agent: kleine Dateien als Pakete (ein Roundtrip für viele Dateien,
    Paketgröße nach gemessener Rate), große einzeln und parallel; alte Gegenstellen erkannt.
    QUIC-Fenster für schnelle Leitungen mit hoher Latenz; Dienst/Agent nehmen mehr gleichzeitige
    Anfragen an.
  - Google Drive: gepoolte Verbindungen (kein TLS-Handshake je Aufruf), Pfad-Cache entkoppelt
    geschrieben, Sperren nur je Ordner/Zielname, ID-Vorrat, Multipart für kleine Dateien, Backoff mit
    Zufallsanteil.
  - SFTP: vorausgelesene Blöcke und Kanal-Pool; FTP: Verbindungs-Pool; WebDAV: gepoolte Mutationen,
    Streaming-PUT ohne Temp-Puffer, 429/503-Behandlung; SMB: mehrere ausstehende Blöcke.
  - Lokal: Kernel-Kopie (`CopyFileExW`/`copy_file_range`), parallel je Volume geregelt, neue Kopien
    ohne `fsync` je Datei (Verschieben/Überschreiben unverändert sicher), Verschieben auf demselben
    Volume als eine Umbenennung.
  - Sync: Einweg-Spiegeln kopiert parallel zum Scannen; Zwei-Wege/Jobs mit derselben Regelung.
- **F6 Nachvollziehbar.** Eine Übertragungsliste zeigt jede Übertragung: Richtung, Quelle → Ziel,
  Zustand (wartet / sucht Dateien / läuft / fertig / abgebrochen / mit Fehlern), gefundene und
  erledigte Dateien und Bytes, aktuelle Rate, Restzeit (sobald alles gefunden ist), die gerade
  laufenden Dateien, übersprungene Einträge und Fehler; Fehler vollständig ansehbar und kopierbar
  (auch im Fehler-Protokoll der App). Android zeigt dieselben Zahlen in seiner Übertragungsliste,
  während der Suche mit „Suche Dateien… N gefunden“.
- **F7 Zuverlässig.**
  - Nie überschreiben, außer der Nutzer wählt „überschreiben“ im Dialog „Kopieren nach…“ (lokal).
    Einfügen/Ablegen: Namenskonflikte werden zu „Name (2)“; Ordner verhalten sich wie bisher
    (lokales Ziel: zusammenführen, Remote-Ziel: neuer Ordnername bei Konflikt).
  - Keine halbfertigen Dateien unter dem Zielnamen (private Stufe + atomares Veröffentlichen).
  - Quelländerungen während der Übertragung werden erkannt und als Fehler gemeldet.
  - Einzelne Fehler stoppen die Übertragung nicht; nach anhaltenden Fehlern in Folge (z. B.
    Verbindung weg) bricht sie mit klarer Meldung ab statt tausende Fehler zu sammeln. Vorübergehende
    Netzfehler werden je Datei einmal wiederholt, solange noch nichts veröffentlicht wurde.
  - Abbrechen stoppt zügig; fertige Dateien bleiben, lokale Teildateien werden entfernt.
- **F8 Android** nutzt dieselbe Engine (`fs.transfer`, Materialisieren, Hochladen-als-Kopie):
  schneller, sofortiger Start, Fortschritt mit wachsenden Summen.
- **Unverändert (Schutzgüter):** Sync (Einweg/Zwei-Wege/Jobs), Laufwerks-Mounts (volle atomare
  Garantien), Öffnen/Bearbeiten/Zurückspeichern, gespeicherte Orte und Verbindungen, Ordnerauswahl,
  Favoriten, Papierkorb/Löschen, Analyse, Filter-/Rekursiv-Ansichten und ihre gefilterte
  Zwischenablage (nur passende Dateien mit relativer Struktur), App-Papierkorb-Auslassung,
  Win32-Sonderfälle langer/ungewöhnlicher Namen, geschützte Lesezugriffe (einmalige Rechteanfrage).

## B Bedienung

- **Kopieren:** wie bisher (Strg+C, Kontextmenü). Meldung sofort: „✓ N Element(e) kopiert – Strg+V
  startet die Übertragung“. Kein Warten, keine zweite Einfüge-Aufforderung mehr.
- **Einfügen:** wie bisher (Strg+V, Kontextmenü „Einfügen“, im Remote-Hintergrundmenü). Die
  Übertragung erscheint sofort in der Übertragungsliste; Meldung „⇄ Übertragung gestartet: N
  Element(e) → <Ziel>“. Wartet sie hinter anderen (mehr als sechs gleichzeitig), steht dort
  „wartet (Position n)“.
- **Ziehen/Ablegen** zwischen Tabs/Bereichen und aus dem Betriebssystem: wie bisher, gleicher Start.
  Aus der App in den Explorer ziehen: Remote-Einträge ohne Warten (Explorer kopiert im Hintergrund).
- **Explorer-Einfügen (Windows):** Nach Strg+C im Remote-Tab im Explorer Strg+V → Explorer zeigt
  „Wird vorbereitet…“ (nur Auflisten, kein Inhalt) und dann seinen Kopierdialog mit Fortschritt.
- **Übertragungsliste:** Statuszeile zeigt weiterhin kompakte Chips; neuer Knopf „⇅ Übertragungen
  (n)“ öffnet/schließt die Liste als Fenster. Jede Zeile: Symbol (⬆/⬇/⇄/📋), Titel „Quelle → Ziel“,
  Fortschrittsbalken (Bytes; während der Suche wachsend, Hinweis „sucht… N gefunden“), Rate und
  Restzeit, „1.234 / 5.678 Dateien · 1,2 / 3,4 GB“, übersprungen/Fehler, bis zu drei laufende
  Dateinamen. Knöpfe: „Abbrechen“ (laufend/wartend), „Fehler anzeigen“ (Liste zum Kopieren),
  „Zielordner öffnen“ (fertig), „Entfernen“ bzw. „Fertige entfernen“. Abgeschlossene bleiben bis zum
  Entfernen oder Programmende sichtbar (höchstens 30).
- **Abschluss:** Meldung „✓ 1.234 Dateien (3,4 GB) übertragen in 1:23“ bzw. „⚠ … mit 3 Fehlern –
  Details in Übertragungen“; Fehler zusätzlich im Fehler-Protokoll.
- **Abbrechen:** „Abbrechen“ in Liste oder Chip; Zeile zeigt „wird abgebrochen…“ bis alle Worker
  stehen.
- **Fehlerfälle:** Ziel nicht beschreibbar/voll/Verbindung weg → Übertragung endet mit Meldung und
  Liste der betroffenen Dateien; Rechteanfrage für geschützte lokale Ordner wie bisher genau einmal
  (Ablehnen stoppt die Übertragung, bis dahin Kopiertes bleibt).
- **Android:** unverändert bedient (Kopieren/Ausschneiden → Einfügen-Leiste, „Kopieren nach…“);
  Übertragungsliste zeigt Fortschritt, während der Suche „Suche Dateien… N gefunden“.

## C Layout und Bedienkosten

- Kein zusätzlicher Klick für den Normalfall: Kopieren/Einfügen bleibt ein Tastendruck; die Liste
  ist optional (Chips reichen für den Überblick, ein Klick für Details).
- Fenster „Übertragungen“ statt fester Leiste: nimmt keinen Platz weg, wenn nichts läuft; öffnet
  sich nicht ungefragt. Neueste Übertragung oben, laufende vor fertigen.
- Restzeit erst anzeigen, wenn die Suche abgeschlossen ist (sonst irreführend); vorher „sucht…“.
- Rate über die letzten Sekunden geglättet (nicht Durchschnitt seit Start).
- Dialog „Kopieren nach…“ schließt nach dem Start; Fortschritt steht in der Liste (keine
  blockierende Einzelkopie mehr – mehrere lokale Kopien dürfen parallel laufen).

## Entscheidungen (nach Rückmeldung vom 2026-09-28)

1. Neue Kopien ohne `fsync` je Datei (Explorer-/cp-Verhalten); Verschieben und Überschreiben
   synchronisieren weiter vollständig. Selbst entschieden (nicht kritisch, deutlich schneller).
2. Explorer-Einfügen aus Remotes **ohne Obergrenze**: Die Dateiliste entsteht beim Einfügen im
   Explorer (paralleles Auflisten), Inhalte werden gestreamt und die nächsten Dateien parallel
   vorausgeladen. Einzige Grenze ist die Windows-Grenze von 260 Zeichen je relativem Pfad im
   Descriptor; betroffene Dateien werden in der App gemeldet (in der App einfügen überträgt sie).
3. Pakete für kleine Dateien bei Direct/Raum-Shares **und** beim SSH-Agent (Agent-Binärpakete werden
   im Release ohnehin neu gebaut; alte Agents werden am Hello-Label erkannt).
4. Parallelität wird je Verbindung gemessen geregelt (Nutzleistungs-Bergsteigen, Halbieren bei
   Überlast); feste Obergrenzen nur aus echten Protokoll-/Dienstgrenzen (FTP-Verbindungen des Servers,
   Anfragen je Dienst-/Agent-Verbindung, SFTP-Kanäle des Servers). Begründung: `recherche.md` §2–3.
5. Sync wird im selben Batch schneller: Einweg-Spiegeln kopiert parallel zum Scannen; Zwei-Wege/Jobs
   nutzen dieselbe Regelung; Google Drive ohne TLS-Handshake je Aufruf, ohne Cache-Neuschreiben je
   Operation und ohne globale Sperren (Ursache des langsamen ersten Herunterladens).
6. Weitere Protokoll-Maßnahmen, weil technisch machbar und zielführend: SFTP mit vorausgelesenen
   Blöcken und Kanal-Pool, FTP mit Verbindungs-Pool, WebDAV mit gepoolten Mutationen und
   Streaming-PUT, SMB mit mehreren ausstehenden Blöcken, lokale Kernel-Kopie (`CopyFileExW`,
   `copy_file_range`), größere QUIC-Fenster. Details und Grenzen: `recherche.md` §3.
