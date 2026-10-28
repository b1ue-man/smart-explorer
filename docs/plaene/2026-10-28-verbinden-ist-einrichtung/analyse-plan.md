# Analyseplan: jeder Dateiübertragungsweg, höchstmöglicher Gesamtdurchsatz

Stand: 2026-10-28. Quellstand der Analyse: `a67d7296` (eingefrorene Kopie, damit
parallel laufende Umsetzungen die Befunde nicht verfälschen).

## Ziel und Maßstab (Nutzer r6, r8–r11)

- „analysiere extrem gründlich, jeden file transfer flow … calls, funktionen, länge
  prozessaufteilung und threading, alles … plane was du analysierst plane wie du
  analysierst plane wie du es so schnell wie möglich machen kannst“
- „ich verlange dass es so schnell wie irgendwie möglich ist“
- „nicht maximum speed per file, maximal möglicher datentransferspeed. at all times
  von anfang zu ende“

Maßstab je Weg: Gesamtdurchsatz des Auftrags vom ersten bis zum letzten Byte, zu
jedem Zeitpunkt begrenzt nur durch das knappste Mittel (Datenträger, Netz, CPU).
Vergleich mit dem schnellsten etablierten Werkzeug desselben Wegs (lokal `cp`/
`robocopy /MT`, Netz `rsync`/`rclone`/`scp`, Rohgrenze `fio`/`iperf3`). Eigene
Folgerung (Annahme): Zielwert ≥ 90 % des Werkzeugs bzw. der Rohgrenze, ohne Anlauf-
und Endphase mit unausgelastetem Mittel.

## Bereich (alle Wege)

Einstiege: Einfügen im se-Fenster (eigene Zwischenablage), Einfügen aus der
OS-Zwischenablage, Ziehen hinein/hinaus, Kopieren in andere Programme (lokal,
entfernt, virtuelle Dateien), „Für andere Programme bereitstellen“, Android-App,
CLI `se cp`/`se mv`, MCP-Werkzeuge, Ausschneiden/Verschieben, Sync-Aufträge des
Workers, se-Laufwerke (FUSE/CfAPI) über den OS-Dateimanager, Tresore (Cryptomator),
Archive (ZIP).

Quelle × Ziel: lokal (gleicher Datenträger, anderer Datenträger, Wechseldatenträger
FAT/exFAT, Netzlaufwerk/UNC), Direct Share (LAN direkt, Relay), Raum, SFTP,
SSH-Agent, SMB, FTP/FTPS, WebDAV, Google Drive, Tresor, se-Laufwerk; entfernt →
entfernt (gleiche und verschiedene Gegenstellen). Plattformen: Windows, Linux,
Android. Dateigrößen: 4 KiB, 64 KiB, 1–16 MiB, ≥ 1 GiB, gemischte Bäume, flach und tief.

## Methode je Weg

1. Statisch (Code, `file:line`): Aufrufkette vom Einstieg bis zum Byte auf Datenträger
   oder Leitung mit allen Funktionen; je Datei Systemaufrufe, Netz-Roundtrips,
   Synchronisierungen (fsync, FlushFileBuffers), Sperren, Allokationen, Prüfungen;
   Prozesse (GUI, Worker, Host-Worker, Share-Host) und Threads/Tasks mit Anzahl,
   Warteschlangen und Kanäle mit Kapazität, Puffergrößen; Phasen (Auflisten,
   Ordneranlage, Übertragung, Veröffentlichung, Belege, Fortschritt) mit Überlappung;
   Anlaufphase (Zeit bis zum ersten Byte, Hochlauf der Nebenläufigkeit) und Endphase
   (letzte Dateien, Abschlussarbeiten); jede Stelle, an der das knappste Mittel wartet.
2. Dynamisch (Remote-CI, ubuntu-24.04 und windows-2025): Messläufe je Weg und
   Größenklasse mit Durchsatz-Zeitreihe (Bytes je 100 ms), Phasenzeiten der Engine
   (`engine/bottleneck.rs`), Linux `strace -f -c`/`-T` (Systemaufrufe, Wartezeiten),
   `/proc/<pid>/io`, Netzverzögerung per `tc netem` auf Loopback, Windows mit und ohne
   Defender-Ausnahme; Referenzwerkzeuge auf denselben Daten.
3. Auswertung je Weg: Tabelle Referenz | se | Verhältnis | Leerlaufanteil des
   knappsten Mittels | Ursachen mit Beleg | Maßnahme | erwarteter Gewinn | berührte
   Invariante und wie sie gehalten wird.

## Aufteilung

| Teil | Wege | Ergebnisdatei |
|---|---|---|
| G1 | lokal: alle Einstiege, gleicher/anderer Datenträger, Wechseldatenträger, Netzlaufwerk, Verschieben, Zwischenablage-/Ziehwege, Kopiermodul je Plattform, Engine-Threading | `analyse-g1-lokal.md` |
| G2 | Gerät zu Gerät: Direct Share (LAN, Relay), Raum, Hostseite, iroh/QUIC-Parameter, Android-Speicher | `analyse-g2-geraete.md` |
| G3 | SFTP, SSH-Agent, SMB, FTP/FTPS, WebDAV, Google Drive, entfernt → entfernt | `analyse-g3-backends.md` |
| G4 | Sync-Aufträge, se-Laufwerke (Schreiben/Lesen über den OS-Dateimanager), Tresore, ZIP | `analyse-g4-sync-laufwerke.md` |
| G5 | Messrahmen auf CI (nach G1–G4): Messläufe aller auf CI erreichbaren Wege vorher/nachher | Suite + `messung.md` |

Bereits vorhanden: `analyse-kopieren.md` (erster Durchgang lokal und Direct Share);
G1/G2 prüfen und vertiefen ihn, statt ihn zu wiederholen.
