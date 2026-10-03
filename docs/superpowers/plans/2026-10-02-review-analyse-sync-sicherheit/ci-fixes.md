# RV1 – Korrekturen aus der einen Remote-Suite

Stand: 2026-10-03. Kandidat `395f912a30455ebd96799f61fdbe1fec2e1c7998`,
[Lauf 37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629).
Alle Jobs sind beendet; der Lauf ist fehlgeschlagen. Kein neuer Projekt-Review.

## Stage eins: belegte Grenzen

Die vollständigen Cargo-JSON-Diagnosen, Android- und Serverlogs sowie der
kandidatengebundene Formatterpatch wurden aus den drei nach SHA-256 geprüften
CI-Artefakten übernommen. Der Patch wurde erst nach Kandidaten-/Patchhashprüfung
angewandt. Die Typmeldungen zeigen fehlende Trait-/Facade-Imports, nach Extraktionen
zu enge Sichtbarkeiten, neue API-Parameter in alten Fixtures und inkompatible
Zahlen-/Texttypen. Windows meldet zusätzlich einen fehlenden Send-Vertrag des
Directory-Iterators und einen alten Copy-Guard-Aufruf. Der Server verweigert
Klartext bereits durch ConnectionReset; die Fixture unwrappt diese Verweigerung.

Die Formatterdiagnose nennt acht Dateien an oder über 500 Zeilen. Diese
Verantwortungen werden kohäsiv geteilt; API, Schutzwirkung und Szenarien bleiben
erhalten. Ein erfolgreicher Serverbuild bestätigt keine native Geräteabnahme;
Linux-/Windows-Library und Android-JNI konnten noch nicht vollständig entstehen.

## Stage zwei: Korrekturmeilensteine

| Besitzer | Betroffene Grenze | Erwartetes Ergebnis im selben Remote-Einstieg |
|---|---|---|
| E-ENGINE | Versionen, Replica, Incremental und eigene Providerfixtures | Tatsächliche Namen/Traits/Fehlertypen passen; Versionen-/Pending-/Lost-ACK-Assertions bleiben vollständig. Incremental bleibt unter der Dateigrenze. |
| H-ANALYSIS/Desktop | Phase-/Recycletypen und egui-Consumer | Hostanalyse behält echte Phase und reversible Recycle-Fehler; Desktop/Y156 verwenden den vorhandenen egui-Reexport. Analyse-UI bleibt unter der Dateigrenze. |
| A-CLIENT | Agent, IPC, Ergebnis-/Speicherbudgets | JNI/IPC und Empfänger verwenden tatsächliche Typen/Parameter, ohne Budgets oder Host-Principals zu verlieren. |
| T-JOBS | SyncAttempt und Daemon-Catchup | Stabiler Stringvertrag und vollständige Catchup-Abbruch-/Retry-Reihenfolge; kohäsive Extraktion unter 500 Zeilen. |
| S-SIGNAL | Node-/FS-Facaden und private Helper | Featuregebundene Sichtbarkeiten, bestehende Namen und cfg-Auswahl stimmen; Share-Fassade, Exec und LAN-Framing bleiben kohäsiv unter 500 Zeilen. |
| S-REVOKE | Kontakt-/Policyfixtures und TLS-Verweigerung | Aktuelle explizite Aufnahme-/Pinverträge erhalten alle bestehenden Assertions; Reset wird als reale Verweigerung ausgewertet, ohne Registrierung zuzulassen. |
| V-LOCAL/V-REMOTE | Windows Directory/Copy und WebDAV-Fixture | Send-Vertrag und tatsächlicher Copy-Guard erhalten Handle-/Linkgrenzen; WebDAV-Fixture initialisiert aktuelle Beobachtungsfelder. |
| Root | Vault-Fixture und Integration | Kohäsive Fixtureteilung, vollständiges Inventar, statisches Parsing und aktueller vollständiger Rootgraph. |

Die API-Lückenprüfung verwendet die jeweiligen vorhandenen Refs und die im
Compiler genannten Definitionen; kein weiterer Kritiker oder allgemeiner Review.
Exakte Read-/Edit-/Create-Surfaces stehen in `scopes/ci-1-*.json`. Worker bleiben
zero-compute. Nach Abschluss werden alle Korrekturen zusammen committed/gepusht
und ausschließlich `review-task.yml`/`native/test-review-task.sh` erneut ausgelöst.
Die bestehenden inkrementellen Ausgaben bleiben erhalten. Der Release wartet
weiter auf erfolgreiche Gesamtauswertung.
