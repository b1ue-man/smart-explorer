# Android-Grenzen – Recherche

Stand 2026-09-26. Refs: `docs/refs/smb2.md`, `docs/refs/samba-container.md`,
`docs/refs/android-open-with.md`; Lesungen: `docs/lesungen/2026-09-26-exec-host-flow.md`,
`docs/lesungen/2026-09-26-smb-backend-integration.md`.

## E1 Ersetzen auf SFTP/WebDAV/FTP (umgesetzt, `77d6e0e`)
- SFTP: `posix-rename@openssh.com` (rename(2) auf dem Server) über `RawSftpSession::extended` auf einem
  zweiten Kanal; `SftpSession` legt Erweiterungen nicht offen. Verworfen: Zwei-Schritt-Tausch im Client
  (Abbruch dazwischen ließe die Datei fehlen, der nächste Sync läse das als Löschung).
- WebDAV: ein `MOVE` mit `Overwrite: T` (RFC 4918 §9.9). FTP: `RNFR`/`RNTO` (rename(2) auf POSIX-Servern),
  Upload-Stufe und Veröffentlichen über Abwesenheitsprüfung (FTP kennt kein exklusives Anlegen);
  `rename_no_replace` bleibt für FTP „nicht unterstützt“ (Trait-Vertrag), die Ausnahme gilt nur beim
  Veröffentlichen einer Stufe (`promote_staged_no_replace`).
- Mounts unverändert (Fähigkeiten bleiben `false`).

## E2 Befehle auf dem Telefon (Exec-Host)
- Vertrag (Lesung §3): kein Endergebnis ohne `confirm_empty` = nachweislich keine Nachfahren mehr;
  `terminate_all` muss den ganzen Baum treffen. Linux löst das mit systemd-cgroup, Android hat das nicht.
- Verworfen nach Kritik: App-weiter Subreaper + `/proc`-Nachfahren der App + ein Job. Die App startet
  selbst Prozesse (Sync-Hooks „Befehl vorher/nachher“ über `/system/bin/sh -c`,
  `daemon/os/android/platform.rs`): ein App-weiter Scan hätte Hooks getötet, ihre Zombies eingesammelt
  (ECHILD im Hook) und `confirm_empty` blockiert; ein App-weiter Subreaper sammelt zudem Zombies
  daemonisierter Hook-Enkel nie ein.
- Entscheidung: Subreaper **je Job**. Der Supervisor (`linux_os/exec_supervisor.rs`, von Android per
  `#[path]` eingebunden und über eine `SpawnPolicy` parametrisiert; Linux ruft mit den heutigen Werten
  auf) startet über `Command::pre_exec` einen Zwischenprozess, der nur async-signal-sichere Aufrufe
  macht: `prctl(PR_SET_CHILD_SUBREAPER)`, rohes `clone(SIGCHLD)`; das Enkelkind kehrt zurück und wird
  `/system/bin/sh`. Der Zwischenprozess schließt alle übrigen Deskriptoren, meldet den Exit der Shell
  über eine Status-Pipe (→ `RootExited`), sammelt danach mit `waitpid(-1)` bis ECHILD alles ein und
  endet. `terminate_all` = SIGKILL wiederholt an alle `/proc`-Nachfahren des Zwischenprozesses (er
  lebt bis zuletzt, Ketten brechen nie, `setsid` entkommt nicht); `confirm_empty` = der Zwischenprozess
  ist beendet. Keine App-weiten Eingriffe, fremde PIDs werden nie eingesammelt, mehrere Jobs möglich
  (Registry-Standardgrenzen).
- Nach einem App-Absturz: Job-Wurzel (PID + Startzeit aus `/proc/<pid>/stat`) in einer app-privaten
  Datei; der nächste Start beendet verbliebene Bäume (Startzeit schützt vor PID-Wiederverwendung).
- Freigaben wie am Desktop je exakter Identität: Ziele aus `exec_device_views` (egui-frei nach
  `share/` verschoben), Schlüssel `direct/<deviceId>/<fingerprint>` bzw.
  `room/<roomId>/<deviceId>/<fingerprint>`, aufgelöst gegen den aktuellen Profilstand; Erfolg nur bei
  `persisted && applied && error == None` (`ExecGrantPersistResult`).
- Android-Grenzen (Ref `android-child-processes.md`): stirbt der App-Prozess, tötet das System über
  `killProcessGroup` die ganze App-cgroup `uid_<uid>/pid_<pid>` – auch `setsid`-/Doppel-Fork-Nachfahren
  (die Job-Wurzel-Datei ist nur die zweite Sicherung); Phantom-Prozess-Limit ab Android 12 (32
  systemweit, Kinder mit hoher Hintergrund-CPU werden beendet); gecachte Apps werden samt Kindern
  eingefroren (nicht mit Vordergrunddienst); `prctl`/`clone` sind für Apps erlaubt, `/proc` der eigenen
  UID sichtbar. Erreichbar nur bei laufendem Share-Dienst (App offen oder Dauerbetrieb).
- Verworfen: Kind-Prozess-cgroups (App darf keine anlegen); nur Prozessgruppe (`setsid` entkommt).

## E3 SMB
- `smb2` 0.26.0 (reines Rust, tokio, MIT/Apache, MSRV 1.85), exakt gepinnt (schnelle Minor-Folge).
  Lock-Auflösung (Hauptagent, `cargo metadata`, kein Build) hebt `aes-gcm 0.11.0-rc.4 → 0.11.1` an
  (gemeinsam mit `ssh-cipher` aus russh); die SFTP-Gerätetests decken das ab.
- Vorbild SFTP: eigene Runtime, `block_on` je Aufruf, Neuaufbau bei toter Verbindung; nach
  `reconnect()` Trees neu verbinden. `ClientConfig.dfs_enabled = false` (DFS als Grenze).
- Backend-Primitive: `rename_no_replace` = `Tree::rename` (ReplaceIfExists=0); `rename` = eigenes
  Compound CREATE+SET_INFO(FileRenameInformation, ReplaceIfExists=1)+CLOSE (`rename_overwrites =
  true`); `open_write_new` = `create_file_writer_exclusive`; `flush` = `finish()` als Commit-Grenze;
  Drop ohne Flush = `abort()` und Teildatei entfernen; `staged_write_capabilities` bewusst gesetzt.
- Links: `DirectoryEntry`/`FileInfo` von smb2 verwerfen die FileAttributes. Eigenes Listing über die
  öffentliche Nachrichten-API (`Connection::execute`, `msg::create`/`query_directory`/`close`):
  FileBothDirectoryInformation mit Attributen; `FILE_ATTRIBUTE_REPARSE_POINT` → `is_symlink = true`
  (rekursives Löschen und Sync betreten es nie). `stat` ebenso mit Attributen; Löschen öffnet mit
  `FILE_OPEN_REPARSE_POINT` (löscht den Link, nie das Ziel).
- Desktop: nicht in diesem Batch. `Protocol::Smb`/`Scheme::Smb` plattformübergreifend,
  `MountBackendScheme::Smb` wird im Mount-Pfad abgewiesen; Desktop-Dialog unverändert.
- Endpunkt `smb://user@host:port/<freigabe>/<pfad>`; erste Pfadstufe = Freigabe; je Freigabe ein
  lazy `Tree`; `/` listet nur die konfigurierte Freigabe; Umbenennen über Freigaben → `unsupported`.
- Fehler: NEGOTIATE abgelehnt → „Server spricht nur SMB1“; `BAD_NETWORK_NAME` bei `connect_share` →
  `not_found` „Freigabe <x> nicht gefunden“; Anmeldung → `auth`.
- Testserver `dockurr/samba:4.23.10` (Env `NAME/USER/PASS/RW`, nur smbd, TCP-Probe auf 445).

## E4 „In Smart Explorer öffnen“
- `ACTION_VIEW` mit MIME `vnd.android.document/directory` (DocumentsUI) sowie `file://`-Ordner;
  `content://com.android.externalstorage.documents/…` Dokument-ID `root:pfad` (`primary` = interner
  Speicher, sonst FS-UUID) → Pfad unter den gemeldeten Volumes; app-private und unbekannte Pfade
  abweisen. Nur Ordner-MIME-Typen im Filter (ohne Schema gilt er für `content:` und `file:`); Filter
  für `*/*` verworfen, weil die App sonst bei jedem Datei-Öffnen als Betrachter angeboten würde.

## E5 `Android/data` und `Android/obb` (Kritik 18)
- Ref `android-data-obb-access.md`: `MANAGE_EXTERNAL_STORAGE` nimmt die Ordner anderer Apps ausdrücklich
  aus, SAF (`ACTION_OPEN_DOCUMENT_TREE`) kann sie seit Android 11 nicht wählen; die alte
  DocumentsUI-Lücke ist seit Android 13 geschlossen (kein Weg).
- Einziger Weg ohne Root: Shizuku (Apache-2.0, `dev.rikka.shizuku:api`/`provider`), Dateizugriff über einen
  UserService als `shell` (UID 2000). Voraussetzungen: Shizuku-App installieren, per Wireless-Debugging
  koppeln und nach **jedem** Neustart erneut starten. Unter Android 16 meldet ein offenes Shizuku-Issue
  (#1574) fehlschlagenden Zugriff in mehreren Dateimanagern.
- Entscheidung: nicht in diesem Batch. Eigene Integration (Binder-Dienst, zweiter Dateizugriffsweg für
  alle Operationen, eigene Bedienung zum Koppeln) mit unsicherem Ergebnis auf aktuellen Android-Versionen;
  als Option `AND3` in `docs/TODO.md` festgehalten.
