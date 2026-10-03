# RV1 – konkrete Verhaltensbefunde der gleichen Remote-Suite

Stand: 2026-10-03. [Run 37150409255](https://github.com/b1ue-man/smart-explorer/actions/runs/37150409255), Kandidat `ac9b475ff18f6320bedd408c5a03c091710ad01c`. Alle Jobs sind beendet. Kein neuer Review und kein neuer Kritiker.

## Stage eins: tatsächliche Evidenz

Linux-/Windows-Testhost, Entwicklungs-CLI/Server und Android-JNI konnten entstehen. Windows erreicht die ausgewählte Verhaltensprüfung. Wiederkehrende Fehler nennen den privaten Verzeichniszugriff, DACL-/Rename-Zugriff und den privaten Consumer von `watch_path` (der synchrone Readpin stellt absichtlich keinen Overlapped-Watchpfad bereit). Daneben sind Tombstone-Signaturen, ältere Locator-/Job-/HTTP-Fixtures, Agent-Cancel ohne Einträge und drei echte LAN-Statusrunden belegt. Der Server-Verweigerungsanschluss ist akzeptiert.

Linux erreicht die native Auswahl wegen `unknown filesystem type exfat` nicht. Der gemischte Lauf erreicht NEW→OLD mit tatsächlichem Accept im alten Profil, aber keine erwartete aktuelle Lifecycleprojektion. Android scheitert an `HostMonitor.kt:125`, einem nicht im öffentlichen SDK auflösbaren AppOps-Konstantennamen. Der zweite Formatpatch ist kandidaten-/hashgeprüft als `7b9424dd` übernommen; er enthält keine Dateigrößenverletzung.

## Stage zwei: kohäsive Korrekturen und API-Lückenprüfung

| Besitzer | Konkrete Grenze / Dateien im eigenen Scope | Erwartung derselben gemeinsamen Remote-Suite |
| --- | --- | --- |
| V-LOCAL | Windows Directory-/Quarantine-/Private-Storage-Handlevertrag | Private Dateien/DACL sind am tatsächlichen gepinnten Parent gebunden; Read-only und Root-/Child-Linkgrenzen bleiben erhalten. Kein Freigeben eines Watchpfads aus einem synchronen Pin und kein ungerechtfertigtes Delete-/Write-Sharing. |
| E-ENGINE | Index-Tombstone-Decoder/-Writer und bestehende Indexfixture | Gelöschte Einträge werden niemals als aktive Basis übernommen; andere Owner und vollständige Cache-/Fallbackgrenze bleiben erhalten. |
| H-ANALYSIS | Recycle-Rootformat und Agent-Walk-Cancel | Windows-Roottreffer bleiben tatsächlich eingeschränkt; Cancel erreicht den echten leeren Hashwalk ohne Listing-/Downloadrückfall. |
| A-CLIENT | Vorhandene GUI-/Job-Locatorfixtures | Aktuelle valide Config-/Run-/Identityverträge erhalten alle Backendkombinationen, Literalnamen, getrennte Remotes, Abbruch und Teilresultate. Kein Patch an Kaskaden nur zur Maskierung eines Windows-Handlefehlers. |
| T-JOBS | Quick-Mirror-Preflight-/Delete-Scan | Cancellation und neu auftauchende Quelle verhindern alle Deletes; Hookfixtures müssen den tatsächlich konsumierten Streamingvertrag erreichen. |
| S-SIGNAL | Reale LAN-Statusrounds mit aktuellen Pins/OS-Fakten | Tatsächlich beidseitige Bestätigung ohne FS-/Exec-/Uplinkrechte, Replay-/Deadline-/Withdrawalassertions bleiben zwingend. |
| S-REVOKE | Tatsächliche NEW→OLD-Lifecycleprojektion | Explizite alte Annahme erzeugt die erwartete aktuelle belegte Legacyprojektion; volle moderne Pins, explizite Unsicher-Opt-ins und Rechteentzug bleiben unverändert. |
| Root | Android-AppOps-Name, Linux-Volumeeinrichtung, eng benannte IPC-/Drive-Fixtures und Integration | Öffentlicher SDK-Vertrag, echtes exFAT/FUSE, gültiger authentifizierter JSON-Frame und aktuelle Literal-/HTTP-Verträge; keine neue Testsuite oder lokale Ausführung. |

Exakte Read-/Modify-/Create-Surfaces sind vor jedem Workerturn in `scopes/ci-2-*.json` festgelegt. Die zweite API-Lückenprüfung liest aktuelle Reexports/Adapter und vorhandene Primärrefs; fehlende konkrete Definitionen werden von Root eng nachgereicht. Kein Worker darf seinen Scope erweitern oder Builds, Tests, Formatter, Git, CI, Graph, Release oder Unteragenten starten. Root committed kohäsive Abschlüsse, aktualisiert nach allen Quellen den vollständigen Rootgraph, pusht gemeinsam und löst ausschließlich denselben Remote-Einstieg erneut aus. Release wartet weiter auf erfolgreiche Gesamtabnahme.

### Aufgelöste Root-API-Lücken vor der Änderung

`AppOpsManager.permissionToOp(Manifest.permission.MANAGE_EXTERNAL_STORAGE)` liefert den öffentlichen nullable Operationsnamen; die versteckte `OPSTR_...`-Konstante wird nicht direkt angesprochen. Watcher und periodische Rechteprüfung bleiben erhalten.

Der Linux-Einstieg versucht den echten Kernelmount und verwendet bei fehlender Unterstützung `mount.exfat-fuse` auf derselben formatierten Loopdatei. Der FUSE-Blockmount erhält seine belegten Speicherlimits aus dem tatsächlichen Blockgerät/udev-Typ; Flush bleibt konservativ FUSE. Die bestehende Volume-Fixture prüft die tatsächlichen Limits/Zeitstempel/UUID und den jeweils passenden Flushvertrag. Kein künstlicher Typ und kein Skip ersetzt den Mount.

Die IPC-Fehlermeldung ist ein echter Decoderfehler: serde_json beendet nach der frühen Visitor-Rückgabe noch die ganze Map. Der Capability-Hinweis muss deshalb vor dem absichtlichen Parseabbruch separat erfasst werden; nur vollständig decodierte Authfelder geben Admission. Die unverbrauchte Nachricht wird danach vollständig durch den bestehenden Clienthandler validiert. Prefixbudget, Deadline, Tokenvergleich und ungültige/unvollständige Felder bleiben fail-closed.

Die zwei Drive-Fixtures erhalten den aktuellen Literalnamenvertrag und den zusätzlichen exact-ID-/mtime-Verify-GET nach Rename/Promotion. Titel, ID, Abwesenheits- und Contentassertions bleiben erhalten; keine Produktions-HTTP-Prüfung wird entfernt.
