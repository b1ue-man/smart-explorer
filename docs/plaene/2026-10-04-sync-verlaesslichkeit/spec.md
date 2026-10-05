# Sync-Verlässlichkeit – verbindliche Spec

Stand: 2026-10-05. Status: Umsetzung und Remote-C01–C09 bestätigt; vollständiger Release angefordert, ohne weiteren Testlauf.

## Vollständiger Auftrag und Preflight

Der gemeldete `/Notebook`-Fehler muss verschwinden und der Sync tatsächlich
abschließen. Google Drive darf mehrere Objekte mit demselben Namen enthalten.
Gespeicherte Jobs müssen nach Updates weiter dieselben Orte synchronisieren.
Die vollständige Arbeitsweise einschließlich Plan-Kritik und Gesamtablauf ist
angefordert. Der Auftrag umfasst alle unterstützten Sync-Situationen und die
direkt betroffenen anderen Funktionen; eine neue allgemeine Projektreview ist
nicht angefordert.

Einordnung: zusammenhängender Umbau des Drive-/Sync-Vertrags, volle Kette.
Ziel: verlässlich abgeschlossene Synchronisation, erhaltene Daten und ein
einziger vollständiger Release. Nach ausdrücklicher Nutzerkorrektur vom
2026-10-05 ist kein Echtwelttest eine Releasevoraussetzung; der Release erfolgt
ohne weiteren Testlauf. Der offene Google-Livefall bleibt unbestätigt und ist
keine Veröffentlichungssperre. Die bestehende
Windows-Startkorrektur aus 0.5.171 bleibt Teil der Kompatibilitätsabnahme.

## A Definition

Oberflächen: bestehende Desktop-Sync-Ansichten, Android-Sync-API/-Ansichten und
Hintergrund-Jobs; dieselben VFS-Grenzen für Explorer und Terminal-Dateizugriff.

### Funktionen

- **F1 Drive-Auflösung:** Ein eindeutiger Ordner wie `Notebook` ist auch bei
  zusätzlichen Suchtreffern mit anderer Schreibweise auffindbar. Vergleich und
  Mutationen verwenden den tatsächlichen Namen und die Objektidentität. Leere
  Seiten, überlappende Seiten, Root-Alias und vorhandene Pfad-Caches dürfen keine
  falsche Mehrdeutigkeit oder zusätzliche Objekte erzeugen.
- **F2 Gleiche Namen:** Gleichnamige Dateien behalten die bestehende inhaltliche
  Auswahl: gemeinsame identische Variante automatisch; bei verschiedenen
  Inhalten ausdrückliche Konfliktwahl mit Backup. Gleichnamige Ordner bleiben
  unabhängig zugängliche und synchronisierbare Bäume mit stabiler Zuordnung.
  Weder Titeländerung noch Löschen eines Geschwisters darf einen anderen Baum
  still an die Stelle eines gespeicherten Jobs setzen.
- **F3 Alte Jobs:** Bestehende Endpunkt-Strings, Jobdateien, alte TSV-Imports,
  Baselines, Versionen, Trigger und Optionen bleiben nach Neustart, Tokenwechsel
  und Update erhalten. Der nächste Lauf konvergiert wieder; Neuanlegen des Jobs
  ist keine Voraussetzung.
- **F4 Alle Gegenstellen:** Lokal, UNC/gemappte Laufwerke, SFTP mit/ohne Agent,
  FTP/FTPS, WebDAV, Drive und Direct/Room Share behalten ihre Verbindung und
  Pfadbedeutung, auch zwischen zwei Remotes. Unterstützte SMB- und ZIP-Quellen
  behalten ihren bestehenden Rechtevertrag. Gleiche relative Pfade in
  verschiedenen Konten sind verschiedene Orte.
- **F5 Alle Betriebsweisen:** Quelle→Ziel, Ziel→Quelle und beide Richtungen;
  Erstlauf, unveränderter Folgelauf, Änderung, Löschung, Konfliktauflösung,
  Vorschau, Spiegeln und inkrementeller Wiederanlauf arbeiten nach den
  gespeicherten Optionen. Filter, versteckte Dateien, Vergleichsverfahren,
  Zeitfenster, Löschregeln, Versionsaufbewahrung und eigene Versionsorte bleiben
  wirksam.
- **F6 Wiederanlauf und Datenerhalt:** Abbruch, Netzverlust, verlorene Antworten,
  Quota-/Speichergrenzen, gesperrte Dateien und zeitweilig fehlende Rechte dürfen
  keine unbestätigte Aktion als Erfolg speichern. Nach Wiederherstellung setzt
  derselbe Job fort und schließt ab. Fehlgeschlagene Backups verhindern
  destruktive Aktionen; geschützte ausgelassene Teilbäume bleiben auf beiden
  Seiten und in der Baseline erhalten. Unabhängige Dateien synchronisieren.
- **F7 Gesamtabnahme und Lieferung:** Eine kandidatengebundene Remote-Suite
  bestätigt komplette Abläufe einschließlich alter Jobs und der betroffenen
  Integrationen. Danach erfolgt ein vollständiger Remote-Release mit passenden
  Installer-, Feed-, Desktop-, Android- und Share-Server-Artefakten.

### Entscheidungen und bewahrtes Verhalten

Literalnamen bleiben literal; vorhandene Locators werden nicht global
URL-dekodiert oder umgeschrieben. Drive-IDs sind Identitäten, Namen sind
Darstellung. Vorhandene Marker für unabhängig adressierbare Drive-Geschwister
werden weiter benutzt; wörtlich markerähnliche Titel bleiben unterscheidbar.
Eine bereits gebundene Ordneridentität hat Vorrang vor einer neuen Auswahl nach
Änderungszeit. Neue Zuordnungen werden accountgebunden persistiert.

Die bestehenden Sicherheitsentscheidungen bleiben erhalten: ein echter
beidseitiger Inhaltskonflikt verlangt die bestehende Auswahl; read-only Quellen
bleiben lesbar; ein schreibgeschütztes Ziel wird nicht als erfolgreicher Lauf
ausgegeben. Wiederherstellbare Unterbrechungen behalten ihren Wiederanlaufzustand.
Es entstehen keine neuen Bedienpflichten oder neu erlaubten destruktiven
Aktionen. Die vom Nutzer bereits vorgegebene Spec wird damit konkretisiert;
ein weiterer Spec-Haltepunkt ist nach §2 der Arbeitsweise nicht erforderlich.

## B Bedienung

### Bestehenden Job ausführen (F1–F6)

Einstieg: Desktop „Sync-Setups“ → vorhandener Job → „Jetzt“ oder bestehender
automatischer Trigger; Android bestehende Jobliste → Ausführen. Eingaben sind
die gespeicherten Quelle, Ziel und Optionen. Neue Jobs benutzen unverändert den
Ordnerpicker bzw. die bestehenden Endpunktfelder.

Ablauf: auflösen → Vorschau/Scan → bestehende Konfliktentscheidung, soweit nötig
→ kopieren/veröffentlichen → Baseline bestätigen → fertiger Jobstatus.
Eindeutige Drive-Titel und vergleichbar gleiche Suchtreffer benötigen keinen
Dialog. Gleichnamige Ordner werden als getrennte Bäume sichtbar; Datei-Varianten
gehen durch das vorhandene Konfliktfenster. Nach der Entscheidung beendet der
Lauf die Synchronisation und sichert die verdrängten Inhalte.

Warten: vorhandene Scan-/Transferphasen und Fortschritt; Oberfläche bleibt
bedienbar. Lange Arbeit ist abbrechbar. Abbrechen wartet auf bestätigtes Ende
der laufenden Worker; ein Update übergibt erst danach an den neuen Worker.
Ausstieg: vorhandenes Schließen/Zurück, Abbrechen und Versionswiederherstellung.

Zustände: erster Lauf, bestehende Baseline, Konflikte offen, läuft, unterbrochen,
Wiederanlauf, abgeschlossen. Offline und ein zeitweilig blockiertes Ziel halten
den Job wiederanlaufbar; nach Rückkehr/Retry muss er mit den erhaltenen Daten
abschließen. Es gibt keine Aufforderung, einen intakten alten Job neu anzulegen.

### Verzeichnis und einzelne Inhalte öffnen (F1, F2, F4)

Einstieg: vorhandener Explorer/Ordnerpicker, Terminal `se ls`, `se stat`, `se cat`
und bestehende Kopierbefehle. Browser-Locator, Sync-Locator und Anzeigename
bleiben getrennte Begriffe an der gemeinsamen VFS-Grenze. Ein ausgewählter
Drive-Marker adressiert genau dieses Objekt; Literalnamen mit Prozentzeichen,
Leerzeichen, Unicode oder Markertext behalten ihren Inhalt.

Es wird kein neuer CLI-Sync-Befehl eingeführt: der aktuelle `se`-Befehlsbaum
bietet keinen. Die Abnahme ruft die tatsächliche gespeicherte Job-/Sync-Logik
und ihre vorhandenen Desktop-/Android-Einstiege codeseitig auf.

## C Layout und Bedienkosten

Die vorhandene Jobliste bleibt der regelmäßige Einstieg: Name, Quelle/Ziel,
aktiviert/Trigger, Laufstatus, „Jetzt“, Bearbeiten und Versionen. Der häufige
Fall bleibt ein Klick; zusätzliche Suchtreffer erzeugen keine zusätzliche
Benutzerentscheidung. Der bestehende Fortschritt steht am laufenden Job.

Das Konfliktfenster zeigt weiterhin betroffene Datei, Seite und tatsächlich
wählbare Inhaltsvarianten; die vorhandene Backup-/Wiederherstellungsinformation
bleibt erhalten. Unabhängige gleichnamige Ordner werden mit ihrer bestehenden
Drive-ID-Kennung unterschieden, ohne technische Abfragen in den Bedienfluss zu
tragen. Es werden keine neuen verschachtelten Menüs oder Werkzeuge eingeführt.

Die Abnahme der GUI bleibt Layout-/Aufrufkontrolle. Der Funktionsbeweis erfolgt
über die darunterliegende reale Job-, Resolver- und Sync-Logik.
