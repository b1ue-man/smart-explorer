# CI-4-S-REVOKE: tatsächlicher zusätzlicher Altpeer-Nachweis

Stand: 2026-10-04. Der eng freigegebene Nachweisanschluss ist umgesetzt.
Die erneute gemeinsame Remote-Abnahme des integrierten Kandidaten liegt bei Root.
Keine Produktquellen gelesen oder geändert und keine lokalen App-/Altprogramme ausgeführt.

## Frischer Auftrag und belegter Befund

Grundlage: [CI-4-Plan](../ci-fourth-fixes.md), Abschnitt S-REVOKE,
Run `37162485159` auf `ac3b0c9098963fae386e94558f2f3ca1bb240740`.
Vor Änderung wurden der eigene Scope und Plan,
[RV1-Refs](../../../../refs/rv1-remote-suite.md) sowie die
[Share-TLS-/Identitätsrefs](../../../../refs/share-server-tls-auth.md)
frisch gelesen. Der Eingriff betrifft ausschließlich den zusätzlichen
Stage-Nachweis im vorhandenen Mixed-Version-Einstieg.

`/tmp/rv1-ci-fourth/linux/mixed-version.log` meldet:
`legacy target has not recorded an admitted peer session`,
Stage `NEW to OLD: keep capability RPCs out of the admitted legacy filesystem path`.
Die dort tatsächlich ausgewiesene Diagnosedirectory
`se-share-mixed-version.wrI9hZ` wurde innerhalb des freigegebenen
`/tmp/rv1-ci-fourth/linux` verwendet; keine Verzeichnisauflistung.

Die zugewiesenen Artefakte belegen den vorher bereits erfolgreichen Ablauf:

- `new-to-old-explicit-probe.txt` und `new-to-old-root-listing.txt`:
  tatsächliche CLI-Ausgabe `d-\t0\tHome`.
- `new-to-old-remote-stat.txt`: `path=/Home`, `name=Home`, `type=dir`.
- `new-to-old-accepted-lifecycle.json`: ausgehend, server_queued,
  legacy_forwarded, Request-Receipt unconfirmed, moderne Entscheidung Pending
  mit Revision null, effective Accepted/evidence legacy_relation und aktive
  Autorisierung mit basis legacy_contact_projection.
- Alter Daemonlog: wirklicher Start/Signaling und die tatsächlichen vorherigen
  `Iroh-Peer: Direktfreigabe nicht akzeptiert`-Verweigerungen.
  Im gesamten Log findet sich weder ein `Iroh-Session`-Marker noch
  eine `unknown variant`-Meldung. Die zusätzliche Erfolgsformulierung
  `Iroh-Session akzeptiert:` war in diesem Lauf daher kein verfügbarer Nachweis.
- Aktueller Callerlog am `2026-10-03 23:56:04`:
  `Iroh-Session authentifiziert:` für das tatsächlich adressierte Altgerät
  und `Share-Op list_dir: 2 ms, 1 Eintraege`.
  Derselbe Log dokumentiert das fortbestehende Verwerfen der ungebundenen
  Legacy-Entscheidung beim tracked_direct-Server.

## Änderungen und Entscheidungen

Der bestehende zusätzliche Nachweisblock prüft die Authentifizierungszeile
am tatsächlich vorhandenen aktuellen Callerlog statt am alten Daemonlog:

```text
new-requester/data/smart_explorer/sync/daemon.log
Iroh-Session authentifiziert:
```

Beide vorhandenen Daemonlogs müssen nichtleer sein. Der alte Log bleibt
die Quelle für die unveränderte Verweigerung eines unbekannten
`capabilities`-RPCs. Die alte nicht vorhandene Erfolgsformulierung wird
nicht durch eine weitere angenommene Altversionsformulierung ersetzt.

Logs sind zusätzliche Beobachtung, keine Produktzulassung.
Der erfolgreiche explizite gepinnte Open, echte Root-LIST und Directory-STAT
sowie die vollständigen Lifecycleassertions bleiben zwingende Pflichtsignale.
Der Stage liegt weiterhin unmittelbar danach, bevor die Worker beendet werden.
Es entsteht kein Capability-Probe-Request, kein neues Opt-in und kein neuer
Suiteeintritt. Keine Rechte-, Admission-, LegacyProof-, Signatur-,
Write-/Exec- oder Wiregrenze wurde verändert.

## Abnahme und statischer Self-Review

Einziger Remote-Einstieg bleibt `native/test-share-mixed-version-e2e.sh`
innerhalb derselben Root-eigenen vollständigen RV1-Suite.
Exakte relevante Stages:

- `NEW to OLD: prove an explicit probe cannot activate a pending legacy grant`:
  vorhandene echte Verweigerung vor Altannahme, unveränderte Pending-/Inactive-Query;
  Timeout/Kill bleibt ein Fehler.
- `NEW to OLD: accept using only values emitted by legacy share status`:
  unveränderte tatsächliche Altannahme und Fingerprintbindung.
- `NEW to OLD: explicitly probe the pinned peer after legacy acceptance`:
  wirklicher erfolgreicher Open und `new-to-old-explicit-probe.txt`.
- `new-to-old-accepted-lifecycle.json`: sämtliche vorhandenen
  Access-/Lifecyclebedingungen bleiben unverändert.
- `NEW to OLD: prove authorized filesystem access via CLI-discovered target`:
  wirkliche Root-LIST und Directory-STAT; keine Logzeile ersetzt sie.
- `NEW to OLD: keep capability RPCs out of the admitted legacy filesystem path`:
  beide Daemonlogs nichtleer, tatsächlicher aktueller Authentifizierungsmarker
  und keine unbekannten `capabilities`-RPCs im Altpeerlog.

Statisch ist der gesamte Einstieg außerhalb des einen korrigierten
Beobachtungsblocks bytegleich. Der vollständige NEW→OLD-Präfix und
OLD→NEW-/Reject-/Restart-/Retry-/Revoke-Suffix sowie die negative
Capability-RPC-Assertion bleiben unverändert. Keine vorhandene Schutzassertion
wurde gestrichen. `bash -n` prüfte ausschließlich Shellsyntax; kein
Befehl des Einstiegs wurde ausgeführt.

## Restgrenzen

Keine offenen Scope-/APIabhängigkeiten innerhalb dieses begrenzten Anschlusses.
Run `37162485159` belegt den erfolgreichen NEW→OLD-Zugriff vor dem zusätzlichen
Nachweisfehler, keine erfolgreiche Gesamtabnahme. Der korrigierte Nachweis und
alle nachfolgenden bisherigen Stages sind durch dieselbe Remote-Suite am
integrierten Kandidaten zu bestätigen. Root besitzt Git-Kontextgate,
Commit/Push und CI. Keine lokalen Builds, Compiler, Tests, Formatter, Server,
Installationen, Git-, CI-, Graph-, Releaseaktionen oder Unteragenten.

## Exaktes Dateiinventar

Nur freigegebene Doku, Einstieg, Daemonlogs und CLI-Operations-/Lifecycleartefakte
gelesen. Keine Profile, Identitäts-, Token-, Credential- oder Lockdateien gelesen.

### Gelesen

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-4-s-revoke.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fourth-fixes.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/share-server-tls-auth.md`
- `/tmp/rv1-ci-fourth/linux/mixed-version.log`
- `native/test-share-mixed-version-e2e.sh`
- `/tmp/rv1-ci-fourth/linux/se-share-mixed-version.wrI9hZ/old-target/data/smart_explorer/sync/daemon.log`
- `/tmp/rv1-ci-fourth/linux/se-share-mixed-version.wrI9hZ/new-requester/data/smart_explorer/sync/daemon.log`
- `/tmp/rv1-ci-fourth/linux/se-share-mixed-version.wrI9hZ/new-to-old-explicit-probe.txt`
- `/tmp/rv1-ci-fourth/linux/se-share-mixed-version.wrI9hZ/new-to-old-root-listing.txt`
- `/tmp/rv1-ci-fourth/linux/se-share-mixed-version.wrI9hZ/new-to-old-remote-stat.txt`
- `/tmp/rv1-ci-fourth/linux/se-share-mixed-version.wrI9hZ/new-to-old-accepted-lifecycle.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-S-REVOKE.md`

### Geändert

- `native/test-share-mixed-version-e2e.sh`

### Erstellt

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-S-REVOKE.md`

