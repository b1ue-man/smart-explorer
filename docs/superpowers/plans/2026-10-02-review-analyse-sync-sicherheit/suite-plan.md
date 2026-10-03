# RV1 – eine gemeinsame Remote-Abnahme

Stand: 2026-10-03. Diese Abnahme setzt ausschließlich den bestehenden Meilensteinplan
und seine Übergaben um. Sie startet keinen weiteren Projekt-Review. Produktänderungen
werden vollständig abgeschlossen, bevor der Einstieg implementiert und ausgelöst wird.

## Einstieg und Reihenfolge

Ein Dispatch von `.github/workflows/review-task.yml` bindet Ref, Checkout und die volle
Kandidaten-SHA. Alle Stufen gehören zu diesem Lauf und rufen denselben checked-in
Einstieg `native/test-review-task.sh` auf. Es gibt keinen Compile-only-Modus und keine
parallelen Verifikationspipelines für diesen Kandidaten.

1. Linux und natives Windows kompilieren nur den betroffenen Library-Testhost
   inkrementell, sofern kein quell- und hashgebundener Entwicklungs-Testhost vorhanden
   ist. Cargo-JSON entdeckt die tatsächliche ausführbare Datei. Beide verwenden diese
   Ausgabe für die gesamte Abnahme. Windows wird tatsächlich ausgeführt, nicht auf
   Linux als GNU-Ziel mitgeprüft.
2. Der Linux-Anschluss erstellt zusätzlich ausschließlich die benötigten Entwicklungs-
   CLI-/Share-Server-Binaries und übernimmt sie in die Geräte-Stufe. Die veröffentlichte
   CLI v0.5.126 und ihr Hash werden über die Release-API entdeckt und überprüft.
3. Android erstellt nur x86_64-JNI, Debug- und Instrumentierungs-APK für den Emulator.
   Der vorhandene NDK-/Maven-/Gradle-Vertrag wird wiederverwendet. Kein arm64-Zwischen-
   build, Installer, Feed oder vollständiger Releasebuild gehört zu dieser Abnahme.
4. Die Geräte-Stufe verwendet die übernommenen Binärdateien. Begrenzte Android-
   Instrumentierung prüft die veränderten Sync-, Share-, Analyse- und Dienstanschlüsse.
   Fehlgeschlagene Stufen behalten Logs und Runtimewerte; der Einstieg besitzt Cleanup,
   Subprozesse und Fristen.

Jeder ausführende Job erhält mindestens 30 Minuten. Formatter-Diagnostik liefert bei
Bedarf einen an den Kandidaten gebundenen Patch als Artefakt; lokal wird kein Formatter
gestartet. Ein Fehler verhindert den Release. Ein Fix wird committed/gepusht und nur
über denselben Einstieg bestätigt; Cache und überprüfte Entwicklungsausgaben bleiben
wiederverwendbar. Der vollständige Release folgt erst nach erfolgreicher Auswertung.

## Meilensteine und konkrete Erwartungen

| Quellenblock | Gemeinsames Signal |
|---|---|
| V-LOCAL, V-REMOTE und H-POLICY-BOUNDARY | Reguläre Dateien bleiben nutzbar; Links, Junctions, Spezialdateien und eigene Recoveryobjekte schützen Gegenstücke/Basis. Literalnamen, Zeitpräzision, Zielgrenzen und tatsächliche Providerrechte gehen durch die gemeinsame VFS-Grenze. |
| E-PLAN und E-APPLY | Erfolgreiche Teilaktionen überleben Fehler/Cancel; Backupfehler verhindern Destruktion. Mirror-Folgeläufe kopieren unveränderte Dateien nicht erneut. Leere-/Replikaschutz benötigt seine passende einmalige Bestätigung. Ordner-, Versionen-, Restore- und Recorded-Merge-Anschlüsse behalten ihre Rückwege. |
| E-ENGINE und H-REPLACE | Drive-Altzustand wird nur für nachgewiesenen gleichen Account übernommen. Fremde Accountfeed-Einträge erzeugen keine Rootaktionen. ReplacementIntent existiert vor Mutation; Lost-ACK bewahrt Original, Stage und Intent ohne Replay. Pending-Merge-Pfade bleiben auch zwischen Neustart und ausdrücklichem Retry geschützt. |
| T-JOBS und Watch-Adapter | Echte OS-Ereignisse, Debounce/Maxwait, Overflow, Wiederanmeldung und partielle Abdeckung lösen konservative Kontrolle aus. Jobs nutzen aktuelle Endpunkte und gemeinsamen JobState; Wiederholungen, Abbruch und Hooks behalten ihre Reihenfolge. |
| H-ANALYSIS und A-CLIENT | Hostanalyse/Duplikatsuche laufen auf dem Exportgerät; Ergebnis-/Transferbudgets, Cancel/Reattach und geschützte Bereiche bleiben erhalten. Ein Transportabbruch allein ist kein Rechtewiderruf. Papierkorb/Restore ersetzen keine fremden Bytes. |
| H-DISPATCH, S-POLICY und S-REVOKE | Neue Rechte sind explizit, private/Systemgrenzen bleiben gebunden. Widerruf trifft nur betroffene Principals und Exec; Aliase und veraltete Bestätigungen erteilen keine Rechte. Alte Locator-/Profilbedeutungen bleiben erhalten. |
| S-SIGNAL und Share-Server | TLS ohne Klartext-Fallback, Pins, Schlüsselbindung, Replay-/Präsenzsignaturen und Fairness. Gemischte veröffentlichte/current CLI-Versionen behalten den kompatiblen Anfrageablauf. |
| S-LOCAL und S09-LINK | Private Erzeugung/ACL und Disable/Cleanup erhalten retrybare Fehler. Echte gepinnte TLS/IP-Statusrunden binden das tatsächliche Interface; Close, Pinentzug, Pfadrevision und TTL invalidieren Fakten. mDNS liefert keine Uplink-Autorität, der Statuskanal keine FS-/Exec-Rechte. |
| D-SYNCUI und Y156 | Jobowner, geplante Einzelaktion, Versionen und gespeicherte Originalbytes werden von der Engine verwendet. Close und Update bleiben bei verzögerten Workern abbrechbar, verarbeiten Ergebnisse weiter und schließen erst nach tatsächlicher Completion. |
| AND-SHARE-UI und AND-SYNC | Persistierte Rechte/Pins, JobState, Versionen und Merge-Retry bleiben sichtbar/retrybar. Lokaler Storage-Verlust cancelt betroffene aktive Arbeit; private und Fern/Fern-Jobs bleiben unabhängig. Dienst-/Alarm-/Worker-Anschlüsse und Hostzahlen verwenden tatsächliche Plattformfakten. |
| Direkt betroffene Kompatibilität | Lokale/UNC-/SSH/Agent-/FTP/FTPS-/WebDAV-/Drive-/Direct/Room-Paare und Picker behalten Backend, Verbindung, Literalpfade, Omissions und bekannte Baselines. |

Quellselektoren stehen in `abnahme/<block>.md`. Erwartete Szenarionamen sind keine
vorhandenen Testfunktionen; der Einstieg prüft die tatsächlich entdeckten Fixtures und
verweigert fehlende Pflichtsignale. Die ausdrücklich unter „Nicht in RV1“ zurückgestellten
Funktionen und in den Übergaben benannten allgemeinen Plattformgrenzen werden nicht
aus erfolgreichen engeren Fixtures als behoben abgeleitet.
