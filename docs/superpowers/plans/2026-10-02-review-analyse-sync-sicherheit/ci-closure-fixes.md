# RV1 – Schließen der konkreten dritten Remote-Diagnosen

Stand: 2026-10-03. Derselbe [Run 37157166735](https://github.com/b1ue-man/smart-explorer/actions/runs/37157166735), Kandidat `71a8ca45697272453c213c9b0b5412d0cbed0f71`, ist vollständig beendet. Kein neuer Projekt-Review oder Kritiker. Der echte Android-Build erzeugt nun APK/JNI; Geräteverhalten wurde tatsächlich erreicht. Erfolgreiche Gesamtabnahme und Release stehen weiter aus.

## Stage eins – vorhandene Evidenz und Ansatz

Die hashgeprüften Artefakte unter `/tmp/rv1-ci-third/` enthalten die tatsächlichen Fehler. Windows-Checkpoints und abhängige Sync-Consumer melden weiterhin AccessDenied, während die vorherigen privaten Watchpfad-/Sharing-Fehler verschwunden sind. Android meldet PermissionDenied beim privaten Identitäts-/Verbindungsspeicher; Share-Worker-Fehler hängen daran. Linux erreicht nun echte FAT/exFAT-Volumes und zeigt eine abweichende Zeitauflösung des FUSE-Treibers. Die echte LAN-Diagnose nennt Pfadrevisionen bzw. einen belegten Snapshot. Der Altversions-Open erreicht zugelassene Sessions, aber v0.5.126 kennt den neuen `capabilities`-RPC nicht. Weitere benannte Baseline-/Providerfixtures, Papierkorb-Restore und Watchlimit bleiben offen.

Der dritte kandidaten-/hashgebundene reine Formatterpatch ist als `c8c0d2ef` übernommen. Er meldet eine kohäsiv zu extrahierende Drive-Fixture bei 500 formatierten Zeilen. Bestehende Primärrefs und aktuelle Reexports/Definitionen sind die Grundlage; neue Plattform-/Protokollfragen werden vor der jeweils betroffenen Änderung in lokalen Refs geschlossen.

## Stage zwei – detaillierte begrenzte Meilensteine

| Besitzer | Ein kohäsiver Ausgang / betroffene Grenze | Konkretes erwartetes Ergebnis derselben Suite |
| --- | --- | --- |
| V-LOCAL | Privater Dateizugriff Windows/Unix/Android, einschließlich tatsächlicher Ancestor-/Handlecapabilities | Checkpoints und App-private Identitäts-/Verbindungsspeicher öffnen unter wirklichen OS-Rechten; Owner, DACL/Mode, NoFollow, Hardlinkverweigerung und exklusive Veröffentlichung erhalten. Kein Herabsetzen von Sicherheit oder Umleiten privater Daten. |
| E-ENGINE | Benannte Baseline-/Konvergenz-/Agentfixtures und ihre Engine-Grenze | Erfolgreiche unabhängige Aktionen bekommen den richtigen Owner-/Pfadstand; Pending, fehlgeschlagene Aktionen und Links schützen ihre Basis; echte Agent-Hashwalks bleiben erhalten, während `include_hidden=false` wegen fehlender Remote-Hidden-Metadaten beim Metadata-Fallback bleibt. Windows-Private-Storage-Kaskaden werden nicht durch Fixtureänderungen maskiert. |
| A-CLIENT | Wirklicher Share→Share-Sync mit Versionsbackup und bestehendem Split-Consumer | Versionsbackup bleibt reversibel in den bestehenden Provider-/App-private-Archiven; eine vom Share-Host absichtlich private Archivroot verdeckt keine gewöhnlichen Zugriffsfehler; Literalnamen, Konten-/Verbindungsidentität und aktueller Workerabschluss bleiben erhalten. |
| H-ANALYSIS | Benannte Windows-Papierkorb-Restore-/Restartfehler | Vorhandene fremde Namen bleiben erhalten; gehaltene Payloads haben an jedem Restorehop eine dauerhafte Zuordnung und bleiben nach Fehlern erneut wiederherstellbar. |
| S-SIGNAL | Tatsächliche LAN-Pfadrevisionen beim echten Statusround | Eine bestätigte aktuelle Pfadbindung funktioniert; echte Pfadwechsel, Replay, Withdrawal, Stop und unbekannte/abgelaufene Fakten bleiben ablehnend. Keine verlängerte Frist oder ungeprüfte Bestätigung ersetzt den Beleg. |
| S-REVOKE | Zugelassener gepinnter Altpeer ohne `capabilities`-RPC | Explizites Öffnen und bestehende alte Lese-/Dateisystemabläufe funktionieren mit ehrlichem konservativem Fähigkeitsvertrag; kein ungebundenes Signaling-Accept und keine neue Write-/Exec-Zulassung. |
| Root | Drive-Fixtureverantwortung, benannte Weak-PIN-Fixture, echte FUSE-Zeitauflösung und Watchlimit | Dateien bleiben unter 500 formatierten Zeilen; PIN-Assertions entsprechen dem aktuellen dokumentierten Vertrag; tatsächliche Zeitauflösung und sichtbarer Watchlimitfallback stimmen ohne Skips. |

## Zweite API-Lückenprüfung und Durchführung

Exakte Read-/Modify-/Create-Surfaces liegen vor jedem Workerturn in `scopes/ci-3-*.json`. Die größeren verbundenen Flächen werden nur dort gewährt, wo gespeicherter Stand, private Zugriffscapabilities und ihr realer Consumer sonst getrennt würden. Fehlende konkrete Definitionen meldet der Worker; Root gewährt sie gezielt. Neue Plattformfragen benötigen aktuelle Primärquellen und lokale Syntax-/Fehlerrefs vor Änderung. Jeder Worker erledigt seinen eigenen statischen Self-Review und stoppt.

Root integriert alle Abschlüsse, ergänzt ausschließlich den bestehenden einzigen Suiteeintritt soweit ein tatsächlicher Consumeranschluss es verlangt, aktualisiert den vollständigen Rootgraph, committed/pusht und löst nur dieselbe Suite auf dem neuen exakten Kandidaten aus. Keine lokalen Builds/Tests/Formatter, unabhängigen Tests, neuen Reviews, weiteren Matrizen oder Zwischenreleases. Der terminale Release bleibt nach erfolgreicher Auswertung aller Stufen.

### Aufgelöste Root-API-Lücken

Der dritte Lauf startete den vorhandenen Watchlimitfall mit `--include-ignored`, ohne `max_user_watches` abzusenken. Die Linux-Runtime entdeckt den vollständigen Fallnamen aus dem vorhandenen libtest-Listing, führt ihn im selben Suiteeintritt in einem eigenen Prozess mit kurz abgesenktem echtem UID-Limit aus und stellt den ursprünglich gelesenen Wert im Cleanup wieder her. Andere Watchfälle behalten das normale Limit; kein Fall entfällt. Primärvertrag: inotify(7), geprüft 2026-10-03.

exfat-fuse 1.3.0 speichert `mtime` als `time_t` und übernimmt in `exfat_utimes` nur `tv_sec`. Der Linux-OS-Adapter erhält für reale FUSE-exFAT-Blockvolumes deshalb Sekundenauflösung, Namen-/Größenlimits des exFAT und weiter `PerFileOnly`. Kernel-exFAT bleibt bei 10 ms. Der reale Volumefall fordert den passenden Treibervertrag und überprüft weiterhin die tatsächlich gespeicherte Zeit und UUID. Syntaxrefs werden vor diesem Edit in `docs/refs/rv1-remote-suite.md` ergänzt.

### Integrierter Abschluss vor dem nächsten Kandidaten

Die sechs eigenen CI-3-Handoffs schließen ihre begrenzten Quellenanschlüsse; Root ergänzt nur die dokumentierten Runner-/Fixturegrenzen. Vorhandene Assertions und Suite-Selektoren bleiben erhalten. Die Agent-Fastpath-Fixture fordert ausdrücklich `include_hidden=true` und ihr vorhandenes Hidden-Glob; ohne diese Option bleibt die etablierte vollständige Metadata-Auswahl maßgeblich, weil das Hashprotokoll keine Hidden-Attribute überträgt. Pending-Konvergenz und Windows-Lost-ACK sind weiter offene Laufzeitdiagnosen mit nun konkretem Assertionkontext, keine behaupteten Fixes. Nach vollständigem Rootgraph, Commit/Push folgt ausschließlich derselbe Remote-Suiteeintritt.
