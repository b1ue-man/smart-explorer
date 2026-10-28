# G5: Messrahmen und Lesart der Ergebnisse

Stand: 2026-10-28. Suite: `.github/workflows/speed-setup-task.yml`, Einstieg
`native/test-speed-setup-task.py` (läuft nur auf GitHub Actions, nie lokal).

## Auslösen

`workflow_dispatch` mit `candidate_sha` (volle SHA des gepushten Kandidaten,
gleich dem gewählten Ref). Weitere Eingaben:

- `measure` (Standard `true`): `false` lässt nur `gate`, `functional` und
  `android` laufen, etwa für eine Bestätigung im Fehlerkreislauf.
- `repetitions` (Standard `3`): Wiederholungen je Messung; berichtet wird der
  Median.
- `netem` (Standard `true`): Fernwege zusätzlich mit 1 ms und 20 ms
  Verzögerung je Richtung auf `lo` (nur Linux, Job `measure remote-cli`, eine
  Wiederholung, Daten `small` und `medium`, Werkzeuge rclone, v0.5.188,
  Kandidat).

## Was gemessen wird

Maßstab (analyse-plan.md): Gesamtdurchsatz des Auftrags vom Prozessstart bis
zum Ende des letzten Bytes, auf demselben Runner verglichen mit dem
schnellsten etablierten Werkzeug und mit v0.5.188. Die Baseline wird aus dem
Tag `v0.5.188` in einem git worktree im Release-Profil gebaut und mit
denselben Nutzerbefehlen gemessen wie der Kandidat.

| Job | Wege | Vergleich |
|---|---|---|
| `measure local` (Linux, Windows) | `se cp -r` auf demselben Datenträger, `se mv` auf einen anderen Datenträger (Linux `/dev/shm`, Windows zweites Laufwerk oder NTFS-VHDX); danach jedes ignorierte `speed_task_`-Benchmark der Bibliothek | `cp -r`, `rsync -a`, `mv` bzw. `robocopy /MT:16`; iperf3 auf Loopback |
| `measure local` (nur Windows) | Datensatz `small` zusätzlich mit eingeschaltetem Defender-Echtzeitschutz und mit Ausnahme für den Arbeitsordner | dieselben Werkzeuge |
| `measure remote-bench` (Linux) | jedes `speed_task_remote_*` aus `--test transfer_task` gegen SFTP (atmoz/sftp), FTP (vsftpd), SMB (dockurr/samba), WebDAV (httpd mod_dav); die FTP-Fälle zusätzlich gegen pyftpdlib (MLST) | rclone, lftp, scp, smbclient auf denselben Daten |
| `measure remote-cli` (Linux) | `se cp -r` hoch und runter zu SFTP, FTP, SMB; Share Direct zwischen zwei lokalen Geräten je Version | rclone, lftp, scp, smbclient; iperf3 |
| `measure windows-remote` (Windows) | `se cp -r` und `speed_task_remote_ftp_*` gegen pyftpdlib | rclone |

Datensätze: `small` = 5000 × 4 KiB in 50 Ordnern, `medium` = 1000 × 1 MiB,
`large` = 1 × 2 GiB (nur bei mindestens 12 GiB freiem Platz). Der Inhalt ist
zufällig (nicht komprimierbar). Unter Linux wird vor jedem Lauf der
Seitencache geleert (`drop_caches`); unter Windows nicht.

## Lesart

Jeder Mess-Job hängt eine Zusammenfassung an die Step-Summary und lädt das
Artefakt `speed-setup-measure-<OS>-<part>-<sha>-<attempt>` hoch.

- **Per path**: eine Zeile je Weg, Datensatz, Strecke und Variante.
  - „Reference (fastest)“ ist das schnellste Referenzwerkzeug dieser Zeile
    mit seinem Median.
  - Die Spalten `v0.5.188` und „Candidate“ zeigen MiB/s und Dateien/s mit der
    Zahl der gültigen Läufe (`n`).
  - „Candidate ÷ reference“ ist das Verhältnis der MiB/s. Der Zielwert ist
    eine Annahme aus analyse-plan.md: mindestens 0,90.
  - „Candidate ÷ baseline“ über 1 heißt schneller als v0.5.188.
  - Bei `remote-bench` heißt der Kandidat `engine (plain)` oder
    `engine (crash log)`. Das sind die Raten der `BENCH`-Zeilen; eine
    Baseline gibt es dort nicht, weil v0.5.188 diese Testfälle nicht hat.
- **Every reference tool**: alle Werkzeuge einzeln, auch iperf3 als Rohgrenze
  der Loopback-Strecke.
- **BENCH lines**: Median jeder Kennzahl je `BENCH <case>` über die
  Wiederholungen. „÷ reference line“ steht, wenn das Benchmark selbst eine
  Referenzzeile druckt (`local_<daten>_reference_cp` bzw. `_robocopy`).
- **Not measured / notes**: was nicht lief, mit Grund (Zeitbudget, Platz,
  fehlender Server, Baseline-Build).
- **Run errors**: Fehler von Baseline- und Referenzläufen. Sie lassen den Job
  nicht scheitern. Fehler des Kandidaten stehen unter „Checks“ und lassen den
  Job scheitern.

Rohdaten: `measurements.json` enthält jeden Lauf mit Sekunden, MiB/s und
Dateien/s. `bench.json` enthält jede `BENCH`-Zeile je Wiederholung und die
Mediane. Die Befehlsprotokolle liegen unter `runs/` und `bench.log`.

## Grenzen der Messung

- Alle Server laufen auf demselben Runner. Docker-Ports gehen über
  docker-proxy; vsftpd und pyftpdlib laufen ohne diesen Hop. Gemessen wird
  also Loopback, kein echtes Netz. netem verzögert nur den `lo`-Abschnitt.
- pyftpdlib ist ein Python-Server. Gegen ihn begrenzt oft der Server die
  Rate, nicht der Client.
- WebDAV wird über `se cp` nicht gemessen. Gespeicherte WebDAV-Verbindungen
  sprechen nur HTTPS, und die Freigabe einer Test-CA gibt es nur in
  Testbuilds. WebDAV erscheint daher nur in `remote-bench`, über
  `transfer_task` mit HTTP.
- Ob Defender auf GitHub-Runnern beim Start aktiv ist, steht als
  „defender at start“ in der Zusammenfassung. Die Varianten nennen den
  tatsächlich gemessenen Zustand.
