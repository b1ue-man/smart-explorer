# Android-Grenzen – Recherche

Stand 2026-09-26. Refs: `docs/refs/smb2.md`, `docs/refs/samba-container.md`,
`docs/refs/android-open-with.md`; Lesungen: `docs/lesungen/2026-09-26-exec-host-flow.md`,
`docs/lesungen/2026-09-26-smb-backend-integration.md`.

## E1 Ersetzen auf SFTP/WebDAV/FTP (umgesetzt, `e399889`)
- SFTP: `posix-rename@openssh.com` (rename(2) auf dem Server) über `RawSftpSession::extended` auf einem
  zweiten Kanal; `SftpSession` legt Erweiterungen nicht offen. Verworfen: Zwei-Schritt-Tausch im Client
  (Abbruch dazwischen ließe die Datei fehlen, der nächste Sync läse das als Löschung).
- WebDAV: ein `MOVE` mit `Overwrite: T` (RFC 4918 §9.9). FTP: `RNFR`/`RNTO` (rename(2) auf POSIX-Servern),
  Upload-Stufe und No-Replace über Abwesenheitsprüfung (FTP kennt kein exklusives Anlegen).
- Mounts unverändert (Fähigkeiten bleiben `false`).

## E2 Befehle auf dem Telefon (Exec-Host)
- Vertrag (Lesung §3): kein Endergebnis ohne `confirm_empty` = nachweislich keine Nachfahren mehr;
  `terminate_all` muss den ganzen Baum treffen. Linux löst das mit systemd-cgroup, Android hat das nicht.
- Entscheidung: In-Prozess-Supervisor (die vorhandene `exec_supervisor::run` über `UnixStream::pair()`,
  nach `share/os/unix/` verschoben, von Linux und Android eingebunden) plus Android-Eindämmung:
  der App-Prozess wird Subreaper (`prctl(PR_SET_CHILD_SUBREAPER)`), damit verwaiste Enkel nicht zu init
  entkommen; höchstens **ein** laufender Exec-Job je App; Nachfahren = alle Prozesse, deren
  Eltern-Kette in `/proc/<pid>/stat` zum App-Prozess führt (die App startet sonst keine Prozesse);
  `terminate_all` = SIGKILL an Prozessgruppe und alle Nachfahren bis keiner mehr da ist, verwaiste
  Zombies per `waitpid(pid)` einsammeln; `confirm_empty` = kein Nachfahre mehr (außer bereits
  eingesammelten). Scheitert `prctl` oder fehlt `/system/bin/sh`: Anbieter „nicht verfügbar“ mit Grund
  (wie heute, sicherer Rückfall).
- Freigaben über denselben Journal-Weg wie am Desktop (`daemon::mutate_exec_grant`), zweistufige
  Bestätigung wie `share_exec_ui.rs` (scharfschalten → Warnung + Häkchen → aktivieren).
- Verworfen: Kind-Prozess-cgroups (App darf keine anlegen); nur Prozessgruppe ohne Subreaper (`setsid`
  entkommt).

## E3 SMB
- `smb2` 0.26.0 (reines Rust, tokio, MIT/Apache, MSRV 1.85), exakt gepinnt (schnelle Minor-Folge).
  Vorbild SFTP: eigene Runtime, `block_on` je Aufruf, Neuaufbau bei toter Verbindung.
- Atomares Ersetzen per selbst gebautem Compound CREATE+SET_INFO(FileRenameInformation,
  `ReplaceIfExists=1`)+CLOSE (Ref smb2.md); damit `rename_overwrites = true` ehrlich, exklusives Anlegen
  über `create_file_writer_exclusive` → volle Staged-Write-Garantien.
- Endpunkt `smb://user@host:port/<freigabe>/<pfad>`; erste Pfadstufe = Freigabe; `Protocol::Smb` in
  `creds`, `LocKind::Smb`, Connector-Arm, Desktop-Dialog, Kotlin-Formular (Freigabe steckt im
  Startordner, Domäne optional als `DOMÄNE\benutzer` im Benutzerfeld).
- Testserver `dockurr/samba` (Env `NAME/USER/PASS/RW`, nur smbd, TCP-Probe auf 445 unkritisch).

## E4 „In Smart Explorer öffnen“
- `ACTION_VIEW` mit MIME `vnd.android.document/directory` (DocumentsUI) sowie `file://`-Ordner;
  `content://com.android.externalstorage.documents/…` Dokument-ID `root:pfad` (`primary` = interner
  Speicher, sonst FS-UUID) → Pfad unter den gemeldeten Volumes; app-private und unbekannte Pfade
  abweisen. Nur Ordner-MIME-Typen im Filter (ohne Schema gilt er für `content:` und `file:`); Filter
  für `*/*` verworfen, weil die App sonst bei jedem Datei-Öffnen als Betrachter angeboten würde.
