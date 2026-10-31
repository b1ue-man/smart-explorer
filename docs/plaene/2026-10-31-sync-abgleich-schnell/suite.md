# Task-Suite Sync-Abgleich (D5): Lesehilfe

Stand 2026-10-31. Eine Suite, ein Einstieg: `.github/workflows/sync-abgleich-task.yml` (nur `workflow_dispatch`, Eingabe `candidate_sha`; Workflow und Checkout sind an genau diesen SHA gebunden; keine Release-Artefakte, keine Veröffentlichung) ruft `native/test-sync-abgleich-task.py --job <gate-Teil|functional|behaviour|measure>`. Die Suite lädt die Hilfen von `native/test-speed-setup-task.py` (Prozessaufruf, Testauswahl, Baseline-Vergleich, Server, pyftpdlib) und stellt dort `BASELINE_TAG` auf `v0.5.189`, die Testpräfixe auf `abgleich_task_` und die betroffenen Module auf die dieses Batches; die Muster-Suite selbst bleibt unverändert. Läuft nur auf GitHub Actions, nie auf dem Arbeitsplatz (AGENTS.md).

Eingaben: `measure` (Standard an: Messjobs), `repetitions` (Standard 3, der Median wird gezeigt).

## Jobs

| Job | Läuft auf | Was er belegt |
|---|---|---|
| `gate` | Linux, Windows | Clippy auf Bibliothek, Binärdateien und Tests; Befunde zählen nur auf seit `v0.5.189` geänderten Zeilen. Linux: rustfmt (`--job fmt`) der geänderten Rust-Dateien, nur Abweichungen auf geänderten Zeilen zählen |
| `functional` | Linux, Windows | (1) Linux baut vorher die eingebetteten se-agent-Bündel aus `se-agent/` (`native/build-agent-bundles.sh`); die Prüfung „eingechecktes `native/agent-bin` gleich frisch gebaut“ ist dieselbe wie die des Release-Workflows. (2) Jeder `#[test]` mit `abgleich_task_` im Namen steht in der Testliste und besteht; fehlt ein Quelltext-Test in der Liste, ist das ein Fehler, außer er ist per `cfg` auf die andere Plattform begrenzt (steht dann im Prüftext). Ignorierte Fälle und Integrationsziele (`native/tests`) mit dem Präfix laufen ebenfalls, mit den Containern aus der Muster-Suite. (3) Alle Tests der direkt betroffenen Module (bisync, rscan, scanner, share, agent, agent_proto, transfer, copy, vfs, cli, mobile, daemon, app, reclaim, syncjobs); ein Fehler dort zählt nur, wenn `v0.5.189` denselben Test besteht (Prüfung „every failing test fails on v0.5.189 too“, Liste in `pre-existing.txt`) |
| `behaviour` Linux | Linux | Gleichheit und Abwärtsverträglichkeit des Abgleichs, danach die Zusammenführen-Regel; siehe unten |
| `behaviour` Windows | Windows | Zusammenführen lokal–lokal (`se cp --conflict update`) |
| `measure` | Linux, Windows | Laufzeiten, siehe „Messtabelle“. Windows: nur lokal–lokal und nur warm |
| `android` | Linux | NDK-Prüfung der Bridge, APKs, alle JVM-Tests (`android/test-android-task.sh android-build`), danach muss jede Testklasse mit `AbgleichTask` im Namen eine Ergebnisdatei ohne Fehler und ohne ausschließlich übersprungene Fälle haben |

## Gleichheit und Abwärtsverträglichkeit (`behaviour`, Linux)

Vier `se` werden im Entwicklungsprofil gebaut (Worktrees neben dem Kandidaten, gemeinsamer Zielordner für die Abhängigkeiten):

- `old` = `v0.5.189` plus der native Teil von `bef433c4` (`se sync compare`) und `db93d3a6` (`se ls -R`), eingespielt mit `git diff | git apply`;
- `new` = Kandidat (mit den frisch gebauten Agent-Bündeln);
- `new_oldagent` = Kandidat-Quelltext mit den Agent-Bündeln von `v0.5.189` (der Client liefert den Agenten mit aus, also ist das „Kandidat-Client gegen Agent alter Version“);
- `old_newagent` = `old` mit den frisch gebauten Agent-Bündeln des Kandidaten („alter Client gegen Agent des Kandidaten“).

Erzeugte Bäume (Rolle a und b, jede Seite mit eigener Reihe): gleiche Dateien mit gleicher Zeit und Größe, Dateien mit Konflikt, Dateien nur einer Seite, versteckte Punktdateien und -ordner, Leerzeichen und Umlaute im Namen, leerer Ordner, `*.tmp` und `node_modules` als Filterziele, drei Links (Datei, Ordner, hängend), ein FIFO, ein tmpfs-Mount `mnt` (`sudo mount`, für `cross_mounts`; ohne sudo bleibt `mnt` ein gewöhnlicher Ordner, das steht in den Hinweisen) und ein eigener kleiner Baum `lk` mit nicht lesbarem Ordner. Alle Zeiten sind fest, jeder Ordner und jeder Link trägt dieselbe Zeit.

Seitenkombinationen und Gegenseiten:

| Weg | Seite A | Seite B |
|---|---|---|
| `LL` | lokal | lokal |
| `LS` | lokal | SFTP ohne Agent (OpenSSH-Container, dessen `/config` ein Ordner des Runners ist) |
| `LA` | lokal | dasselbe Konto mit se-agent, vom Client über SFTP/SSH ausgeliefert; die Prüfung „Agent mit sha256 … wurde ausgeliefert“ belegt, welche Fassung lief |
| `LH` | lokal | Direct Share (Share-Paar nach dem Muster `share_pair`, Host und Client je Fassung) |
| `HH` | Direct Share (Host 1) | Direct Share (Host 0) |
| `L` | nur `ls -R` der lokalen Seite A | – |

Je Weg: `se sync compare JOB --json` mit den Einstellungen `default`, `nohidden` (`--include-hidden false`), `mounts` (`--cross-mounts true`; ohne Angabe ist es aus), `filter` (`--ignore '**/*.tmp'`, `--ignore 'node_modules/**'`) auf `tree` und `default` auf `lk` (Form `locked`), dazu `se ls TARGET -R --json` auf `tree` und `lk`. Fassungspaare: `old/old` ist die Bezugsgröße; verglichen werden `new/new` immer, `new/old` und `old/new` für `LA`, `LH`, `HH`.

Verglichen wird nach Normierung: Reihenfolge, `seconds`, `id`, `name`, `source`, `target` entfallen; gleich sein müssen Dateizahlen, Zählung der Aktionen, die Aktionen (Art und Pfad), Konflikte, Auslassungen, Doppelentfernungen und Sperre; bei `ls -R` jede Zeile (Pfad, Art, Link, Größe, Zeit) und die Pfade der nicht gelesenen Stellen samt Rückgabewert. Pfade des Runners werden zu `<R>` normiert. Der Prüftext eines Fehlers nennt die ersten Unterschiede.

Eine Ausnahme ist bewusst und im Prüftext benannt: Auf einer Agent-Seite (`LA`, Form `nohidden`) wirkte `include_hidden = false` in `v0.5.189` nicht, weil die Agent-Auflistung Punktnamen nie als versteckt meldete (Plan C3, V1). Die Prüfung gilt dort als bestanden, wenn der Kandidat dasselbe liefert wie SFTP für dieselben Daten und `v0.5.189` davon abwich. Bei `ls -R` auf Agent-Seiten entfällt deshalb das Feld `hidden` im Vergleich; eigene Prüfung: das Feld `hidden` des Agenten stimmt beim Kandidaten mit dem von SFTP überein.

Hilfsprüfungen („fixture: …“): Der Lauf von `v0.5.189` muss beide erzeugten Bäume wirklich gesehen haben (mehr als 50 Einträge); sonst wäre „gleich“ leer.

## Zusammenführen (`behaviour`)

`se cp -r --conflict update QUELLE/tree ZIELELTERN` (Ziel enthält schon einen Ordner `tree`). Dateien: fehlend (auch in einem fehlenden Unterordner), gleich, Ziel älter (klein und 1 MiB), Ziel neuer, gleiche Zeit mit anderer Größe, nur am Ziel vorhanden; dazu der Ordner `pure` mit nur fehlenden, gleichen und älteren Dateien. Erwartung: fehlende eingefügt, ältere Ziele ersetzt, gleiche übersprungen, neuere Ziele behalten, Konflikt doppelt behalten (genau eine zusätzliche Kopie im Ordner `mix`, Inhalt der Quelle), Inhalte byte-genau, kopierte Dateien tragen die Zeit der Quelle (höchstens 2 s Abweichung), Quelle unverändert, die Meldungszeile zählt 3 ersetzt, 2 gleiche übersprungen, 1 Ziel neuer behalten, 1 als Konflikt doppelt behalten. Danach dieselbe Kopie rückwärts für `pure` (Gegenseite → Quelle): nur „3 gleiche übersprungen“, Quelle in Bytes und Zeiten unverändert. Der Rückweg gilt nur für `pure`, weil `mix` ein neueres Ziel und einen Konflikt enthält, die rückwärts absichtlich nicht gleich sind.

Ziele: lokal (Linux und Windows), SFTP, SSH mit se-agent, Direct Share (nur im Paar `new/new`), FTP (pyftpdlib, kann Zeiten setzen), SMB (Samba-Container, Dateien per `docker cp`). WebDAV ist nicht ausführbar: die `se`-Befehlszeile verbindet WebDAV nur über HTTPS mit den eingebauten Web-Wurzeln (`SE_SYNC_FIXTURE_CA_DER` liest nur ein Testbau), also erreicht `se cp` keinen lokalen Fixture-Server. Die Suite meldet das als „NOT RUN“ (nicht als Bestanden) mit dieser Begründung.

## Messtabelle (`measure`)

Baum: 100 000 Dateien (je 8 Byte) in 10 000 Ordnern je Seite — Annahme, weil der Nutzer keine Last genannt hat; beide Seiten sind gleich (alles „gleich“, der Abgleich muss beide Bäume ganz lesen). Gebaut werden beide Fassungen im Release-Profil (`v0.5.189` mit den zwei CLI-Commits). Wege `LL`, `LS`, `LA`, `LH`, `HH` wie oben (Windows: nur `LL`), je Weg `sync compare` und `ls -R`, `kalt` (Seitencache vor jedem Lauf verworfen: Linux mit sudo) und `warm` (ein ungemessener Lauf davor). Die Fassungen laufen abwechselnd (Reihenfolge rotiert je Wiederholung); die Share-Wege werden je Fassung mit eigenem Paar nacheinander gemessen, weil der Host die geprüfte Fassung ausführt.

Die Schrittzusammenfassung zeigt „Weg | v0.5.189 | Kandidat | Verhältnis“: Median der Wandzeit des ganzen `se`-Prozesses in Sekunden; Verhältnis = Kandidat ÷ `v0.5.189` (unter 1: Kandidat schneller), „Faktor“ = `v0.5.189` ÷ Kandidat, „n“ = Zahl der Läufe je Fassung. Darunter: die von `se sync compare` selbst gemeldeten `seconds` (ohne Prozessstart), die Referenzen auf denselben Daten (`find -printf`, `du -sb`, bei Windows `robocopy /L`, dazu `se search '*'` je Fassung) und die Prüfungen, dass beide Fassungen gleich viele Einträge sehen. Jede Zahl, auch jede Einzelprobe, steht in `measurements.json` des Artefakts `sync-abgleich-measure-<OS>-<sha>-<Versuch>`.

Die Speicheranalyse selbst hat in der `se`-Befehlszeile keinen Befehl; die Vergleichsgröße „Speicheranalyse derselben Seite“ ist deshalb nicht gemessen (Hinweis im Bericht der Suite). Als nächste Näherung dienen die Referenzen, die dieselben Daten ohne Verbindung (`find`, `du`) und mit einer Anfrage über die Verbindung (`se search '*'`) lesen.

## Grenzen

- Mount-Fall nur mit passwortlosem sudo (GitHub-Runner haben es); sonst steht der Hinweis im Bericht, `mounts` und `default` unterscheiden sich dann nicht.
- Nicht lesbarer Ordner setzt voraus, dass der Runner-Benutzer nicht root ist.
- Die Suite baut vier `se`, zwei davon nur für gemischte Fassungen; das kostet die meiste Zeit des Jobs `behaviour`.
