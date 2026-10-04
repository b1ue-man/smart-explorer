# Windows-Regression nach 0.5.170

Stand: 2026-10-04. Batch: die drei vom Nutzer gemeldeten Fehler beheben und einen
vollständigen korrigierten Release veröffentlichen. Keine weitere Projektreview.

## Stufe 1: Ursache und Ansatz

Vergleich mit v0.5.169: `support_dirs` härtet seit 0.5.170 App-/Sync-Verzeichnisse.
Die Windows-DACL enthält nur eine nicht vererbende Owner-ACE. `SetSecurityInfo`
entfernt damit geerbte Rechte vorhandener Kinder; neue gewöhnliche Kinder erhalten
keine Owner-Rechte. Der Handoff schreibt eine gewöhnliche temporäre Datei; Sync
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

Status: Milestones 1/2 implementiert in `e5920bdf` und gegen Source/API selbst geprüft.
Milestone 3 ist als kandidatengebundener Windows-Workflow und ein einzelner
Remote-Einstieg implementiert; Cache-, Prozess- und Fehlerausgabegrenzen bleiben
in diesem Einstieg. Die Suite prüft auch den Fehler beim erneuten Anlegen des
vorhandenen Jobsverzeichnisses mit der alten leeren Kinder-DACL.
Remote-Laufzeitabnahme und Veröffentlichung stehen aus.
