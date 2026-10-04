# Windows-Regression nach 0.5.170

Stand: 2026-10-04. Batch abgeschlossen und als 0.5.171 veröffentlicht: die drei
vom Nutzer gemeldeten Fehler beheben und einen vollständigen korrigierten Release
veröffentlichen. Keine weitere Projektreview.

## Stufe 1: Ursache und Ansatz

Vergleich mit v0.5.169: `support_dirs` härtet seit 0.5.170 App-/Sync-Verzeichnisse.
Die Windows-DACL enthält nur eine nicht vererbende Owner-ACE. `SetSecurityInfo`
entfernt damit geerbte Rechte vorhandener Kinder. Der Handoff schreibt eine
gewöhnliche temporäre Datei; Sync
erstellt/liest gewöhnliche Jobsverzeichnisse. Cloud-Konfiguration liest unverändert
gewöhnliche Dateien und verwandelt Lesefehler in eine leere Client-ID, die beim
Refresh nicht geprüft wird. Die OAuth-Requestimplementierung selbst ist seit
v0.5.169 unverändert.

Ansatz: Owner-Rechte bei privaten Windows-Verzeichnissen an Dateien/Verzeichnisse
vererben; private Dateien behalten ihre geschützte nicht vererbende DACL. Vorhandene
0.5.170-Verzeichnisse müssen beim nächsten Öffnen ebenfalls aktualisiert werden.
OAuth-Konfigurationsfehler propagieren und vor Netzwerkzugriff prüfen.

## Stufe 2: endgültige Milestones nach API-Lückenprüfung

1. `local_access/os/windows/private_security.rs`: `AddAccessAllowedAceEx` mit
   Object-/Container-Inheritance ausschließlich für Verzeichnisse; Validierung
   unterscheidet Datei und Verzeichnis. Owner, NoFollow, Hardlinkprüfung,
   geschützte DACL und Fehlerweitergabe bleiben. Erwartung: vorhandene gewöhnliche
   Konfigurationen und Sync-Jobs bleiben zugänglich; auch bereits durch 0.5.170
   entzogene geerbte Zugriffe werden durch den nächsten Root-/Sync-Open repariert.
   Nach Befund des ersten Remote-Laufs gehört dazu die Anerkennung von
   `TokenOwner` aus demselben effektiven Token neben `TokenUser`: Windows-Standard-
   Gruppenbesitz bleibt unverändert; fremde Owner und beliebige Gruppen bleiben
   abgelehnt. Die DACL gewährt weiterhin ausschließlich `TokenUser` Zugriff.
2. Cloud-Konfiguration: checked Loader liefert echte Lesefehler; Authorize und
   Refresh verwenden ihn. Refresh prüft Client-ID vor dem Request. Erwartung:
   vorhandene Client-ID und Token bleiben erhalten; ein Lesefehler erzeugt keinen
   leeren OAuth-Request. Ein lokaler HTTP-Provider auf dem Remote-Runner bestätigt
   die tatsächlich gesendeten Refresh-Felder.
3. Eine fokussierte Windows-Remote-Suite mit vorhandenen inkrementellen RV1-
   Development-Caches. Sie reproduziert die nicht vererbende 0.5.170-DACL auf
   echten Windows-Dateien, prüft Reparatur, gewöhnliche Handoff-Dateien,
   gespeicherte Jobs, Cloud-Refresh und tatsächlichen Worker-Start/-Handoff mit
   isolierten Appdaten. Bestehende private Hardlink-/NoReplace-Grenzen werden
   gezielt mitgeprüft. Kein lokaler Build oder Test; kein Android-/Workspace-Matrixlauf.
4. Source-Milestones committen; vollen Root-Graph aktualisieren; Kandidat pushen.
   Genau diese Remote-Suite auswerten und bei relevanten Fehlern denselben Einstieg
   erneut benutzen. Danach bestehende vollständige Remote-Releaseautomation
   `build.yml` → `native/publish-release-local.ps1`; erwartete Version 0.5.171.
   Veröffentlichung und Hashes prüfen; lokale se-/Share-Server-Dateien aktualisieren.

API-Belege und zweiter Lückencheck: [Windows-Owner-Vererbung](../../refs/windows-private-inheritance.md).
Der Windows-Runner muss insbesondere die automatische Reparatur vorhandener leerer
Kinder-DACLs belegen. Daten werden nicht gelöscht, Client-IDs nicht ersetzt und
Benutzer müssen keine Verbindung neu einrichten.

Milestones 1/2 implementiert in `e5920bdf`, die zusätzliche Windows-Ownerkorrektur
in `3f33b6f0`; Source/API selbst geprüft. Milestone 3 ist als kandidatengebundener
Windows-Workflow und ein einzelner Remote-Einstieg implementiert; Cache-, Prozess-
und Fehlerausgabegrenzen bleiben in diesem Einstieg. Erster Lauf
[37218777791](https://github.com/b1ue-man/smart-explorer/actions/runs/37218777791):
vorhandene Cloud-/Jobdateien verlieren nach der alten DACL tatsächlich Zugriff;
OAuth-Konfiguration/Request und NoReplace bestanden. Die erste Fixture hatte
einen schon geschützten Sync-Parent und reproduzierte deshalb beim vorhandenen
Jobsverzeichnis noch keinen Anlagefehler. Windows-Standardbesitz wurde fälschlich abgewiesen;
dieser reale Kompatibilitätsfehler wurde im selben Milestone korrigiert. Die Altzustands-
Fixture umfasst nun auch den vor 0.5.170 gewöhnlich erbenden Sync-Parent, damit
der denied Handoff nicht durch einen schon geschützten Fixture-Parent verdeckt wird.
Gespeicherte Jobs werden sowohl im aktuellen Format byte-identisch als auch mit
der echten Vor-Update-Migration (`config_version=0`, bestehende `.conf` ersetzen,
Baselineberechtigung behalten) im selben Startup-Szenario geprüft.

## Abnahme und Auslieferung

Dieselbe fokussierte [Remote-Suite 37220814819](https://github.com/b1ue-man/smart-explorer/actions/runs/37220814819)
hat den endgültigen Source-Kandidaten `e388ea36624e472d2de5c595312e7434e34893ff`
erfolgreich abgenommen; Logs und Ergebnisartefakt wurden ausgewertet. Die
korrigierte Altzustands-Fixture reproduziert jetzt ausdrücklich auch das reale
`os error 183` beim Jobsverzeichnis. Danach gelingen die Reparatur der vorhandenen
Cloud-/Jobdateien, wiederholte Control-Dateischreibvorgänge, das Laden der
unveränderten aktuellen Jobs und die echte Vor-Update-Migration älterer Jobs mit
erhaltener Baselineberechtigung und Remote-Endpunktidentität. Der tatsächliche
Refresh-Request am lokalen HTTP-Provider des Windows-Runners enthält die
gespeicherte Client-ID und das Token; ungültige Konfiguration sendet keinen Request.
Der veröffentlichte 0.5.169-Worker wurde durch den Kandidaten ersetzt und war
danach erreichbar; alle von der Suite gestarteten Prozesse wurden geschlossen.
Damit ist der erste fehlgeschlagene Lauf durch die korrigierte Abnahme ersetzt.

Der einmalige vollständige [Release-Lauf 37223197372](https://github.com/b1ue-man/smart-explorer/actions/runs/37223197372)
hat `native/publish-release-local.ps1` auf dem Remote-Runner ausgeführt und
Release-Commit `2641fe9ed488754f361c26570ff5fec6aaa6d953` mit Parent `e388ea36`
erstellt und gepusht. Der zugehörige einzelne
[Publikationslauf 37231116590](https://github.com/b1ue-man/smart-explorer/actions/runs/37231116590)
war erfolgreich; [v0.5.171](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.171)
ist seit 2026-10-04 20:15:35 UTC sichtbar. Cargo-Version, Feed-Version, Tag und
Installer stimmen überein. Alle veröffentlichten Asset-Größen und SHA-256-Digests
entsprechen den committed Bytes; die Desktop-/Android-Feed-Sidecars stimmen ebenfalls.
Kein lokaler Build, Test oder Release-Lauf wurde ausgeführt.

Die bereits beauftragten lokalen Linux-Installationen wurden anschließend aus
diesen veröffentlichten Bytes aktualisiert. `se --version` liefert `0.5.171`;
der operative Worker-Handoff meldet `worker=replaced`, `worker_error=null` und
die laufenden Worker verwenden denselben Digest wie das veröffentlichte CLI.
Der Share-Server wurde nach gesicherter Kopie seines vorherigen Binaries atomar
ersetzt; Service, TLS-/Proxy-Konfiguration und Daten blieben erhalten. Der Service
ist `active/running`, ohne Neustartschleife, und lauscht weiterhin auf 51820/51821.
Der Digest seines laufenden Prozesses entspricht dem veröffentlichten Server:

- Linux `se`: `8b18239450e130df5f2ee73f395bf9986786bf883e93b166499805d88e470238`
- Linux Share-Server: `91e7b177e875784864cacc4e64e89f8987985630c82cbcc2cadae2b1d85eef45`
