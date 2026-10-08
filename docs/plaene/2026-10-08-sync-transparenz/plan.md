# Sync-Transparenz und Drive-Tempo (2026-10-08)

Stand: 2026-10-08, Quelle aller Anforderungen: Nutzernachricht vom 2026-10-08.

## A — Erstplan aus dem Auftrag

| Nr. | Ergebnis | Quelle |
|---|---|---|
| R1 | Fehler- und Zustandszeilen eines Sync-Jobs zeigen eindeutig, ob sie aktuell sind; nach Erfolg oder erneuerter Anmeldung bleibt kein alter Fehler stehen. Beispiele: „Lauf ohne Fortschritt; Dienst prüft Wiederanlauf.“ nach `ok`, „Ausstehender Auslöser bleibt vorgemerkt.“, Notebook-Job mit `invalid_grant` seit 2026-10-05 und „Letzter Läufer meldet sich nicht mehr“. | Nachricht |
| R2 | „Lauf ohne Fortschritt“ erscheint nicht mehr bei Läufen, die tatsächlich arbeiten. | Nachricht |
| R3 | Live-Protokoll je Sync-Job: jede Aktion, jeder Vergleich, öffnen und live ansehen. | Nachricht |
| R4 | Gespeicherte `tcp://`-Share-Server-Adresse: klarer Weg zur verschlüsselten Einrichtung. | Nachricht |
| R5 | Google-Drive-Sync beschleunigen, soweit sicher. | Nachricht |
| R6 | Alle lokalen Installationen nach der Änderung aktuell. | Nachricht |
| R7 | Eine Remote-Task-Suite, ein Remote-Release, keine lokalen Builds/Tests. | AGENTS.md |

Autorisierung: Umsetzung, Suite, Release und lokales Update sind beauftragt
(Nachricht und Memory „decide-independently“: kein Planfreigabe-Halt).

## B — Bestand (gelesen am 2026-10-08, Stand `d9786d0d`)

1. **Fortschritt** (`daemon/os/shared/job_supervisor.rs::poll`): `stalled_since` wird gesetzt,
   wenn `progress` 180 s (`RUN_MARK_STALE_SECS`) alt ist. `progress` erneuern nur
   `own_writes::Observer::{completed,deferred}` (`daemon/os/shared/own_writes.rs`), also nur
   abgeschlossene Apply-Aktionen. Ein Lauf, der länger als 180 s scannt und vergleicht und
   nichts ändern muss, gilt deshalb als „ohne Fortschritt“. **Ursache von R2.**
2. **Anmeldefehler** (`syncjobs/os/shared/job_state_policy.rs::retry_at`, `daemon/os/shared/due.rs`,
   `job_supervisor.rs::admission_allowed`, `realtime.rs::refresh`): `FailureKind::{Auth,Access,Config}`
   (`needs_user`) setzen kein `retry_at`, sperren die geplante und die Echtzeit-Ausführung
   dauerhaft. Eine erneute Google-Anmeldung (`cloud/os/shared.rs::store_refresh_token` →
   `creds::set_secret`) wird nirgends ausgewertet. Der Notebook-Job bleibt daher seit
   2026-10-05 auf `invalid_grant`, obwohl der Private_Unterlagen-Job auf demselben Konto
   wieder erfolgreich läuft. Das Speichern im Job-Editor (`syncjobs::upsert`) setzt den
   Fehler ebenfalls nicht zurück. **Ursache von R1 (Fehler bleibt hängen).**
3. **Tote Laufmarke**: Eine `RunMark`, deren Läufer endete ohne `record_attempt`
   (Prozess beendet, Update), wird nie entfernt; `sync_job_state_ui.rs` zeigt dann
   dauerhaft „Letzter Läufer meldet sich nicht mehr“. **R1.**
4. **Statuszeilen ohne Zeitangabe**: `sync_job_state_ui.rs::render` zeigt Stillstand,
   ausstehenden Auslöser und Fehlerserie ohne „seit“; Android `JobCards.kt` zeigt
   „Lauf wartet auf E/A…“ ohne Zeit. **R1.**
5. **Drive-Erkennung** (`gdrive/core/extensions.rs::change_signal`): Kontoweiter
   `changes.list`-Feed alle `rt_poll_secs`; meldet `Ready` (vollständig, `complete = true`).
   `daemon/os/shared/realtime.rs` wertet aber nur `ChangeSignalMode::Push` als vollständig,
   daher zusätzlich alle 300 s ein erzwungener Volllauf. Jede Änderung irgendwo im Konto
   (auch außerhalb des Sync-Ordners, z. B. Schreibvorgänge des Notebook-Jobs) meldet
   `Changed` und löst einen Volllauf aus. **Ursache von R5.**
6. **Kein inkrementeller Zwei-Wege-Lauf**: `bisync/os/shared/incremental.rs` gilt nur für
   Einweg-Spiegel; „Beide Richtungen“ liest bei jedem Lauf beide Seiten vollständig
   (Baumscan bereits parallel, `snapshot_walk.rs`). Ein inkrementeller Zwei-Wege-Planer
   ist ein eigener Umbau und nicht Teil dieses Batches.
7. **Share-Server**: `share/core/signal_connection_config.rs` kennt `wss://`, Pins und den
   Plaintext-Schalter. Der eigene Server (`/etc/default/se-share-server`) terminiert TLS mit
   dem Let's-Encrypt-Zertifikat für `silasweis.de` auf 51820/51821 und erlaubt zusätzlich
   Klartext. Beobachtet am 2026-10-08: `openssl s_client -connect silasweis.de:51820` →
   `Verify return code: 0 (ok)`, ebenso 51821. Lokale CLI-Konfiguration:
   `tcp://silasweis.de:51820`. Die Desktop-Einstellungen bieten keinen Umstellweg.
8. **Lokale Installationen** auf diesem Rechner: CLI `se` mit Daemon
   (`/root/.local/opt/smart-explorer/se`) und Dienst `se-share-server`; keine lokalen Sync-Jobs.

## C — Entscheidungen

- **R3 Protokollablage**: Textdatei je Job `<sync data>/job-logs/<id>.log`, Anhängen
  zeilenweise, Rotation auf `<id>.log.1` ab 8 MiB (Plattenschranke; ein Volllauf mit
  ausführlichen Vergleichen über 100 000 Einträge erzeugt rund 10 MB). Verworfen: IPC-Stream
  vom Daemon (läuft nicht für Fenster-/Android-/CLI-Läufe, verliert Verlauf), SQLite
  (unnötige Schreiblast, nicht mit Editor lesbar).
- **R3 Erfassung**: Der Lauf setzt seinen Job-Protokollkontext thread-lokal in `bisync::run_at`;
  der Baumscan übernimmt ihn beim Anlegen von `WalkContext` (Ordner gelesen), Vergleiche
  werden nach der Planung aus Plan, beiden Seiten und Sync-Basis erzeugt (Planer bleibt
  unverändert), Apply-Ereignisse über einen umhüllenden `ApplySink`. Damit sind alle Läufer
  (Hintergrunddienst, Fenster, Android, Terminal) erfasst, ohne Signaturen von
  `RunRequest`/`RunSettings` zu ändern. Unveränderte Einträge werden als Anzahl protokolliert;
  der Schalter „Unveränderte Einträge einzeln protokollieren“ im Protokollfenster schreibt
  jede Einzelzeile.
- **R2**: Jede Protokollzeile aktualisiert die letzte Aktivität des Jobs im Prozess;
  der Supervisor nimmt das Maximum aus Apply-Fortschritt und Aktivität.
- **R1**: `needs_user`-Fehler werden genau einmal je neuem Beleg wiederholt
  (`JobState::recheck`). Belege: gespeicherte Anmeldung (`creds-revision`, geschrieben von
  `creds::set_secret`), späterer Erfolg eines anderen Jobs auf demselben Google-Konto (OAuth,
  kein Sperrrisiko) und gespeicherte Job-Einstellungen. Ein periodischer Kontrollversuch wurde
  verworfen: `due.rs` hält seit dem Review vom 2026-10-02 fest, dass Anmeldefehler keine
  automatischen Logins auslösen (fail2ban/Kontosperren bei Passwort-Backends;
  `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md`). Tote Laufmarken bereinigt der Daemon und vermerkt
  `interrupted` mit Läufer und Start; ein automatischer Job erhält einen Kontrolllauf.
  Alle Statuszeilen nennen ihre Zeit.
- **R5**: (a) `realtime.rs` wertet `Ready` als vollständige Abdeckung unabhängig von
  Push/Poll (Vertrag `ChangeNotice::Ready`, `vfs/core/extension_types.rs`); (b) Drive meldet
  `Ready` nur für Ordner in „Meine Ablage“, für Shared-Drive-Wurzeln `ReadyPartial`, weil
  `changes.list` ohne `includeItemsFromAllDrives`/`driveId` nur My-Drive-Elemente liefert
  ([changes.list, Stand 2026-07-07](https://developers.google.com/workspace/drive/api/reference/rest/v3/changes/list),
  [Änderungen abrufen, Stand 2026-09-03](https://developers.google.com/workspace/drive/api/guides/manage-changes),
  abgerufen 2026-10-08); (c) Feed-Änderungen werden auf den Sync-Ordner gefiltert
  (Elternkette per `files.get?fields=id,parents`, je Abonnement zwischengespeichert,
  unbekannte Entfernungen zählen konservativ). Kontrollläufe bleiben über
  `verify_interval_secs`. Verworfen: kürzeres Feed-Intervall (Akkulast Android, Nutzerwert
  bleibt maßgeblich), Mehrfach-Eltern-Listing (rclone ListR) — kein Messwert vorhanden;
  das neue Protokoll liefert je Ordner die Listingdauer als Grundlage.
- **R4**: `SignalServerConfig::encrypted_alternative()` bildet `tcp://`/`ws://`-Einträge auf
  `wss://` mit gleichem Host/Port/Pfad ab. Desktop, Android und CLI zeigen bei Klartext
  den Vorschlag mit Erklärung (Server braucht `SE_SHARE_TLS_CERT`/`SE_SHARE_TLS_KEY`, Name
  muss im Zertifikat stehen, sonst `#sha256=`-Pin) und übernehmen ihn mit einem Klick.

## D — Arbeitsplan

| M | Inhalt | Dateien | Erwartetes Ergebnis (Suite) |
|---|---|---|---|
| M1 | Job-Protokoll: Datei, Rotation, Leser mit Offset, Ausführlich-Schalter, Aktivität, Thread-Kontext, `ApplySink`-Hülle | neu `bisync/os/shared/run_log.rs`, `run_log_lines.rs`; `bisync/mod.rs` | Zeilen erscheinen mit Zeitstempel; Rotation bei Grenze; Leser liefert ab Offset und erkennt Rotation |
| M2 | Protokollpunkte: Laufstart/-ende, Paarsperre, Ordner gelesen, Scan-Summe, Vergleiche, Aktionen, Fehler; Daemon-Auslöser und Vorab-Fehler | `orchestration.rs`, `orchestration_full.rs`, `snapshot.rs`/`snapshot_walk.rs`, `incremental.rs`, `daemon/os/shared/job.rs` | Ein Vollauf schreibt Ordner-, Vergleichs- und Aktionszeilen |
| M3 | Fortschritt aus Aktivität | `job_supervisor.rs` | Kein `stalled_since`, solange Zeilen entstehen |
| M4 | Wiederaufnahme nach Anmeldung/Bearbeitung, Bereinigung toter Laufmarken, Zeiten in Statuszeilen (Desktop + Android-JSON + Kotlin) | `job_state*.rs`, `due.rs`, `job_supervisor.rs`, `run_loop.rs`, `creds/os/shared.rs`, `persistence.rs`, `sync_job_state_ui.rs`, `sync_state_json.rs`, `JobCards.kt`, `SyncApi.kt` | Auth-Fehler wird nach Anmeldeänderung wiederholt; tote Marke wird zu „unterbrochen“ |
| M5 | Drive-Feed vollständig und gefiltert | `realtime.rs`, `gdrive/core/extensions.rs`, neu `gdrive/core/change_scope.rs` | Änderung außerhalb des Ordners: kein Lauf; innerhalb: Lauf; Shared Drive: Teilabdeckung |
| M6 | Verschlüsselt-Vorschlag Share-Server | `signal_connection_config.rs`, `menus_settings.rs`, `cli/share.rs`, `share_settings.rs`, Kotlin Share-Einstellungen, `docs/SHARE_SERVER.md` | `tcp://h:51820` → `wss://h:51820` |
| M7 | Protokoll-Oberflächen | neu `app/core/sync_job_log_ui.rs`, `menus_sync_jobs.rs`; `mobile` `sync.log`/`sync.setLogVerbose`; Kotlin `SyncLogScreen.kt` | Fenster/Bildschirm zeigt neue Zeilen innerhalb 1 s |
| M8 | Suite `sync-transparency-task.yml` → `native/test-sync-transparency-task.sh` (Präfix `sync_transparency_task_`), Doku, Graph, Commit/Push, Remote-Suite, Release, lokales Update, Share-Adresse lokal auf `wss://` | `.github/workflows`, `docs/*` | Suite grün; Release vX.Y.Z sichtbar; `se --version` = vX.Y.Z |

## Nachrichten 2 und 3 (2026-10-08)

Open-Source-Vergleich für Drive, ergebnisoffen bewertet:
[docs/refs/gdrive-opensource-sync-2026-10-08.md](../../refs/gdrive-opensource-sync-2026-10-08.md).
Ergebnis: Unser Bestand ist bei Verbindungswiederverwendung, Backoff, `incompleteSearch`,
paralleler Ordnerliste und ID-sicherer Anlage gleichwertig oder sicherer; übernommen wird die
ID-basierte Feed-Zuordnung (Wegverschieben, Entfernen). Kein untersuchter Client vermeidet den
Zwei-Wege-Vollscan; der inkrementelle Zwei-Wege-Planer bleibt als `GDRIVE-INCR` offen.

## Fortschritt

- 2026-10-08: A–D angelegt.
- 2026-10-08: M1–M7 umgesetzt (Dateien siehe D), Tests mit Präfix `sync_transparency_task_`,
  Suite `sync-transparency-task.yml` → `native/test-sync-transparency-task.py`. Formatierung
  der Batchdateien lokal statisch mit `rustfmt` (stdin) geprüft; keine lokale Kompilierung.
- 2026-10-08: Remote-Suite `sync-transparency-task.yml`: Lauf 37785236569 (`1352a2d1`) scheiterte
  an einer fehlenden Lebensdauerangabe im Drive-Filtertest; Lauf 37788742856 (`9efb4ec5`) zeigte,
  dass die Seiten-Threads von `read_pair` den Protokollkontext nicht erbten (keine „Gelesen“-Zeilen),
  und dass der C04-Test echte Protokoll-Fixtures der Sync-Verlässlichkeits-Suite braucht (aus dieser
  Suite ausgenommen). [Lauf 37791272768](https://github.com/b1ue-man/smart-explorer/actions/runs/37791272768)
  auf `159d678b`: Linux 151/151, Windows 143/143 Tests bestanden, Android-Build, APKs und JVM-Tests
  bestanden. Formatgate sauber.
- 2026-10-08: [Vollständiger Release](https://github.com/b1ue-man/smart-explorer/actions/runs/37794039183)
  aus `159d678b` und [Publikationslauf](https://github.com/b1ue-man/smart-explorer/actions/runs/37807741762)
  erfolgreich; [v0.5.173](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.173) seit
  16:23:41 UTC veröffentlicht (kein Draft/Prerelease), Tag auf Release-Commit `b34c95cf`.
  `native/Cargo.toml`, `release-native/update-feed/version.txt` und Installer = 0.5.173. Alle 20
  Assets heruntergeladen: die sieben Payload-Sidecars stimmen mit ihren Payloads, jedes Asset
  stimmt byteidentisch mit der Datei im Release-Commit (Installer-SHA-256
  `b8e3f70b553eb27de644f8bc725a04e98215d780fa426adcb99f4f8d87c650d7`).
- 2026-10-08: Lokale Installation: `install-linux.sh --cli-only` mit
  `SMART_EXPLORER_REQUIRE_RELEASE_ASSETS=1` → `se 0.5.173` (Update-Quelle unverändert);
  `se update --complete-install 0.5.173` → `{"version":"0.5.173","worker":"replaced","worker_error":null}`;
  beide Daemonprozesse laufen aus der veröffentlichten Datei (SHA-256
  `10481ee259dd06ea6c39078a3841e9fb03976099649b6e3154807aae6725a1a9`). `se-share-server` nach
  verifiziertem Backup (`se-share-server.bak-0.5.172`) atomar ersetzt, Dienst `active`, laufende
  Prozessdatei = `se-share-server-linux` (`ce6a2056…f006`), Ports 51820/51821.
- 2026-10-08: Lokale Share-Adresse `tcp://silasweis.de:51820` → `wss://silasweis.de:51820`
  (Vorschlag aus `se share server show`); Daemonlog: „Share-Server verbunden (🔒 verschlüsselt,
  wss://silasweis.de:51820, Schluessel-Anmeldung=true)“, Relay `https://silasweis.de:51821/`.
  `SE_SHARE_ALLOW_PLAINTEXT=1` bleibt auf dem Server, solange Windows-/Android-Clients noch
  `tcp://` verwenden.
- Nicht belegt: Verhalten auf dem Windows-Rechner des Nutzers (Drive-Feed-Filter mit echtem Konto,
  Wiederaufnahme des Notebook-Jobs nach dem Update) und die Oberflächen von Protokollfenster/-bildschirm
  wurden nicht in echter Nutzung gesehen; belegt sind Kompilierung, Unit-/Integrationstests und Build.
