# RV1 – Recherche und Entscheidungen

Stand: 2026-10-02. Je Thema: Entscheidung, Grund, verworfene Alternativen, Belege. Die Refs unter
`docs/refs/` sind die Syntax-Quelle beim Umsetzen; hier stehen nur die Entscheidungen.

## E1 Echtzeit-Überwachung (FS8) – `docs/refs/sync-change-detection.md`, `docs/refs/android-sync-triggers.md`

- **Eigene schlanke Adapter statt `notify`.** Windows: `ReadDirectoryChangesW` je Job-Wurzel (rekursiv,
  Filter Name/Größe/Schreibzeit/Attribute), Puffer groß lokal, ≤ 64 KB auf Netzpfaden; Abschluss mit 0 Bytes
  oder `ERROR_NOTIFY_ENUM_DIR` = Überlauf → Kontroll-Lauf. Linux/Android: `inotify` (libc) mit einem Watch
  je Verzeichnis, `IN_ONLYDIR|IN_DONT_FOLLOW|IN_EXCL_UNLINK`, Links werden nie verfolgt; neue Unterordner:
  erst Watch setzen, dann den Ordner lesen (Rennen wie bei Unison geschlossen); `IN_Q_OVERFLOW` →
  Kontroll-Lauf; `ENOSPC` beim Watch → Job fällt sichtbar auf Abfrage zurück.
  Grund: `notify` 6.1.1 meldet Windows-Überläufe nicht (fester 16-KiB-Puffer) und folgt unter Linux Links;
  8.x abonniert `IN_OPEN` (Ereignisflut) und hat keinen Windows-Rescan; 9.0 ist Vorabversion (MSRV 1.88).
  Verworfen: fanotify (ohne Root keine Rekursion, auf Android per seccomp gesperrt), USN-Journal
  (laut Microsoft Adminrechte; der unprivilegierte FSCTL ist undokumentiert).
- **Sicherheitsnetz wie Syncthing:** Kontroll-Lauf stündlich (`verify_interval_secs` 3600), Start des
  Dienstes = einmaliger Kontroll-Lauf je Echtzeit-Job (ersetzt fehlende Persistenz über Neustarts),
  Entprellung (`rt_debounce_secs`) mit Höchstwartezeit (`rt_max_latency_secs`, Standard
  max(5 × Entprellung, 300 s)).
- **Netzlaufwerke/Fernziele:** keine Ereignisse (SMB-RDCW ist begrenzt und verliert bei Überlauf; inotify
  sieht nur lokal ausgelöste Änderungen auf NFS/FUSE) → Abfrage alle `rt_poll_secs` (300 s) mit billigem
  Signal (Änderungs-Cursor, wo vorhanden; sonst Wurzel-Liste + Größen), sichtbar als „Abfrage“.
- **Android:** inotify auf den FUSE-Pfaden plus `ContentObserver` auf
  `MediaStore.Files.getContentUri("external")` mit `notifyForDescendants=true`, solange der Prozess lebt
  (Meldungen kommen bei FGS sofort, sonst +10 s); MediaProvider sieht Erstellen/Löschen/Umbenennen, nicht
  jedes Schreiben (FUSE ohne Write-Hook) – deshalb zusätzlich Kontroll-Lauf. Die App setzt kein
  `requestRawExternalStorageAccess`, eigene Änderungen laufen also über MediaProvider.
- **Selbst ausgelöste Ereignisse:** Ereignisse auf Pfaden, die der laufende Job selbst schreibt (Zwischen-
  und Zielnamen), und auf App-Daten-Ordnern werden verworfen; Filter des Jobs gelten vor dem Auslösen.

## E2 Lange Läufe, Termine, Energie (FS10, FS11, FA5) – `android-sync-triggers.md`, `android-background-reachability.md`

- Android: Ein Vordergrunddienst hält die CPU nicht wach → Partial-Wakelock mit Zeitlimit und Erneuerung
  während Daemon-Läufen und Fern-Tasks. Termine: `setExactAndAllowWhileIdle`, wenn
  `canScheduleExactAlarms()`, sonst `setAndAllowWhileIdle` + WorkManager-Nachholen (nur exakte Alarme dürfen
  einen FGS starten). Worker: vorübergehende Fehler → `Result.retry()`; Abbruch durch WorkManager wird aktiv
  in den Kern geleitet (Rust-Threads laufen sonst weiter). Allzugriff (`isExternalStorageManager`) wird
  vor jedem Lauf geprüft und über `sys.hostState` gemeldet.
- Desktop: Systemwach-Anforderung während Sync-Läufen und eingehender Analyse-/Übertragungsströme
  (Windows Power Request, Linux logind-Sperre über `zbus`; Details siehe E11).

## E3 Änderungszeit übernehmen (FS2) – `docs/refs/sync-remote-metadata.md`

| Backend | Weg | Genauigkeit |
|---|---|---|
| lokal | `File::set_times` auf die Zwischendatei vor dem Veröffentlichen | ns (NTFS 100 ns, FAT 2 s, exFAT 10 ms) |
| SFTP | `setstat` mit atime **und** mtime (u32-Sekunden, `FileAttributes::empty()`), danach zurücklesen (ProFTPD ignoriert still) | s |
| FTP | `MFMT` wenn FEAT es nennt; vsftpd: `MDTM <zeit> <pfad>` per `custom_command`; sonst keine | s |
| WebDAV | `X-OC-Mtime` beim PUT (Nextcloud/ownCloud, Antwort `X-OC-MTime: accepted`); sonst keine | s |
| Drive | `modifiedTime` (RFC 3339) im Metadaten-JSON des Uploads | ms |
| SMB | `smb2` 0.26 hat kein SET_INFO-Zeiten → keine (Bauplan in der Ref, nicht in RV1) | – |
| Share | neue Fähigkeit `stage_mtime_v1`, Host setzt die Zeit lokal | wie Host-FS |

Wo keine Zeit übertragbar ist, vergleicht der Planer nicht Quelle gegen Ziel, sondern jede Seite gegen
ihre gespeicherte Basis (Spiegel-Jobs konvergieren). Verworfen: PROPPATCH `{DAV:}lastmodified` (löscht bei
Nextcloud die Prüfsummen), nginx-`Date`-Header (undokumentiertes Verhalten).

## E4 FTP (FS7) – `sync-remote-metadata.md`, `ftp-pool.md`

MLST/MLSD wenn FEAT es nennt (eigener RFC-3659-Parser, `suppaftp::File::from_mlsx_line` scheitert an
ProFTPD/Pure-FTPd-Fakten), sonst `SIZE`+`MDTM`, LIST nur als letzter Weg; Listen je Ordner und Lauf
zwischenspeichern und nach eigenem Veröffentlichen aktualisieren. „Platte voll“: 451 (vsftpd), 452/552
→ `StorageFull`. RNTO auf vorhandenes Ziel scheitert bei ProFTPD (Standard) und FileZilla 0.9 mit 550 →
gesichertes Ersetzen (altes beiseite, neues umbenennen, altes löschen; Wiederherstellung beim nächsten
Lauf). TCP-Keepalive über `socket2` für Steuer- und Datenverbindung (`connect_with_stream`,
`passive_stream_builder`; `socket2` ist schon im Lock). Hostnamen: Betriebssystem-Auflösung als Rückfall,
wenn Hickory nichts findet (`.local`, NetBIOS-Namen).

## E5 WebDAV (FS7)

Ordner nur unterhalb der Sync-Wurzel anlegen (ein 405 oberhalb der DAV-Wurzel ist von „vorhanden“ nicht
unterscheidbar); MKCOL 405 = vorhanden, 409 = Eltern fehlen; `Overwrite: F` → 412 bei vorhandenem Ziel;
507/413 → `StorageFull`. Große Listen: `into_reader()` mit Größengrenze aus dem Speicher und Streaming-
Parser `quick-xml` (0.39.2 liegt schon im Lock, MSRV passt; 0.42 nicht nötig). Prüfsummen nur melden, wenn
der Server sie liefert (erste PROPFIND-Antwort); `OC-Checksum` beim Hochladen zu Nextcloud.

## E6 SFTP (FS7)

Statuscodes: `NoSuchFile` → NotFound, `PermissionDenied` → PermissionDenied, `OpUnsupported` →
Unsupported, `Eof` → UnexpectedEof; `Failure` ist mehrdeutig (OpenSSH meldet ENOSPC/EEXIST/EROFS/EDQUOT so) →
bei Schreibfehlern `statvfs@openssh.com` fragen und volle Platte als `StorageFull` melden. Ohne
`posix-rename@openssh.com`: gesichertes Ersetzen wie bei FTP. Listen: Frist je READDIR-Antwort (≤ 100
Einträge), nicht eine Gesamtfrist.

## E7 Google Drive (FS7)

`modifiedTime` beim Hochladen; Namen sind frei (auch `/`) und nicht eindeutig → Titel wörtlich nehmen, nur
die eigene, kanonische Escape-Form dekodieren, Windows-Regeln nur anwenden, wo die Gegenseite sie braucht;
Identität aus dem Konto-Schlüssel (einmalige Übernahme der alten Paar-ID); `changes.list` kennt keinen
Teilbaum → Ordner-Index des Sync-Baums im Zustandsspeicher, fremde Änderungen ignorieren statt neu
aufzubauen; Verknüpfungen = Links (geschützte Auslassung), Export nur für Docs-Editor-Typen (≤ 10 MB),
andere Google-Typen als Auslassung.

## E8 Share-Server TLS und Anmeldung (FC3, FC4) – `docs/refs/share-server-tls-auth.md`

- Server: rustls `ServerConfig` mit ausdrücklichem `ring`-Provider (`builder_with_provider`, keine
  aws-lc-Standardfeatures), PEM über `rustls-pki-types` (`PemObject`), `StreamOwned` mit Handshake-Frist
  über Socket-Timeouts, WebSocket über `tungstenite::accept_hdr_with_config` mit Grenzen; Zertifikat neu
  laden per eigenem `ResolvesServerCert` (Datei-Änderungszeit, Fehler sichtbar). Relay: iroh-relay
  `CertConfig::Manual`/`Reloading`; Let's Encrypt nur mit TLS-ALPN-01 auf Port 443 – nicht in RV1.
  Klartext nur mit `--allow-plaintext` oder Loopback-Bindung.
- Relay-Zugang: `AccessControl::on_connect` nach dem Schlüsselbeweis lässt nur Endpoint-IDs zu, die gerade
  beim Signaling angemeldet und bewiesen sind (gleicher Prozess).
- Client: `client_tls_with_config` + `Connector::Rustls`; optionales Zertifikats-Pinning (`#sha256=…` an der
  Adresse) über eigenen `ServerCertVerifier`, der die Handshake-Signatur selbst prüft
  (`verify_tls12_signature`/`verify_tls13_signature`); `WebSocketConfig` mit `max_message_size`/
  `max_frame_size` = Signalgrenze. IPv6-Literale in `wss://[..]` vorher prüfen (Servername ohne Klammern).
- Anmeldung: Server schickt eine 16-Byte-Nonce; Client signiert
  `"se-signal-hello-v1" ‖ Nonce ‖ device_id ‖ node_id` mit dem iroh-Schlüssel (`SecretKey::sign`), Server
  prüft strikt (`PublicKey::verify`) und bindet `device_id → Schlüssel` (dauerhaft in einer Server-Datei,
  sonst erste Bindung nach Neustart gewinnt). Beziehungs-Nachweise: der Client berechnet
  `HMAC-SHA256(Beziehungsgeheimnis, "se-server-access-v1" ‖ id)`; der Besitzer hinterlegt beim
  Veröffentlichen/Raum-Anlegen nur `SHA-256(Nachweis)`; der Server vergleicht Hashes (braucht kein HMAC
  und kein Geheimnis). Alte Clients: zugelassen, aber ohne Recht, gebundene Einträge zu überschreiben.

## E9 Kopplung (FC2)

OPAQUE/Argon2id bleiben. PIN-Vorschlag: 6 zufällige Ziffern (`getrandom`); unter 6 Ziffern oder triviale
Muster nur mit ausdrücklichem Opt-in; Angebot einmalig, Abbruch nach 5 nicht abgeschlossenen Austauschen
(die der Veröffentlicher als „abgebrochen“ sieht), höchstens 30 min.

## E10 Analyse-Übertragung (FA5) – `sync-remote-metadata.md` (flate2)

Baum-Daten mit `flate2` (über `zip` schon im Lock) als Deflate-Strom hinter Fähigkeit
`analysis_deflate_v1`; Integrität sichert weiter die vorhandene SHA-256-Prüfung des Baums (flate2 meldet
abgeschnittene Ströme nicht). Speicher je Strom ~0,34 MiB (Encoder) – unkritisch.

## E11 Lokale Datenträger, Dauerhaftigkeit, Wach halten – `docs/refs/local-fs-identity-durability.md`

- **Wurzel-Identität:** Windows `FILE_ID_INFO` (Volume-Seriennummer + 128-Bit-Datei-ID der Wurzel) plus
  Dateisystemname/Flags; `same-file` reicht nicht (ReFS-Lücke). Linux `st_dev` + `st_ino` der Wurzel +
  Mount-ID (`statx` `stx_mnt_id`, Kernel ≥ 5.8; ältere Kernel: mountinfo-Zeile) + Dateisystemtyp aus
  mountinfo; `f_fsid` taugt nicht (FUSE/NFS = 0). Android: wie Linux, aber `statx`/`renameat2` auf API ≤ 29
  nicht aufrufen (minSdk 30 – unkritisch). Vergleich: Abweichung bei leerer Seite = Stopp, bei gefüllter
  Seite = Abgleich ohne Löschübernahme (Spec FS3).
- **Veröffentlichen ohne Ersetzen (Linux/Android):** Leiter wie systemd `rename_noreplace()`:
  `renameat2(RENAME_NOREPLACE)` → bei `EINVAL`/`ENOSYS`/`EOPNOTSUPP` `linkat`+`unlink` (NFS, sshfs,
  ntfs-3g; FAT/exFAT/Android-FUSE haben keine Hardlinks) → zuletzt `access`-Prüfung + `rename` nur für
  eigene Zwischendateien mit zufälligem Namen (bewusst, dokumentiert). Windows: `MoveFileExW(alt, neu, 0)`.
- **Dauerhaftigkeit:** `fsync(Datei)` → veröffentlichen → `fsync(Verzeichnis)` (Linux/Android; `fdatasync`
  sichert keine Zeitstempel); Windows `FlushFileBuffers` auf die Zwischendatei (Verzeichnis-Flush ist
  undokumentiert, entfällt).
- **Sonderdateien:** fremde Quelldateien mit `O_NONBLOCK|O_NOFOLLOW|O_NOCTTY` öffnen, `fstat` = `S_ISREG`
  prüfen, dann `O_NONBLOCK` löschen; Listen tragen `special`.
- **Zeiten/Rechte:** `File::set_times` (Rust ≥ 1.75) auf die Zwischendatei; FAT speichert Ortszeit (2 s,
  Sommerzeit ±1 h), exFAT 10 ms + UTC-Versatz → Vergleich mit Genauigkeit und ±1-h-Toleranz bei gleicher
  Größe auf FAT. `fchmod` auf vfat scheitert, Android-FUSE ignoriert chmod → Rechte nur übertragen, wo das
  Ziel Unix-Rechte hat; Fehler dort sind keine Lauf-Fehler.
- **Windows-Namen:** `:` würde einen Alternate Data Stream anlegen → Namen mit `:` und `?*"<>|` sind auf
  Windows-Zielen Auslassungen „Name auf Ziel nicht möglich“; Cloud-Platzhalter/WOF/Dedup über Attribute +
  Reparse-Tag (Name-Surrogate-Bit = Link, sonst Datei). Nur-lesen-Ziel ersetzen: Attribut nach
  Identitätsprüfung entfernen, ersetzen, Attribut auf der neuen Datei setzen (FileRenameInfoEx-Flags wirken
  nur mit `REPLACE_IF_EXISTS`, Mindestversionen undokumentiert). `ERROR_SHARING_VIOLATION`/
  `ERROR_LOCK_VIOLATION` (raw 32/33) gelten als vorübergehend (Wiederholung mit Backoff).
- **Anschluss-Erkennung:** Windows `CM_Register_Notification` (ohne Fenster; erst registrieren, dann
  aufzählen, Laufwerksbuchstaben mit Wiederholung) + Bustyp per `IOCTL_STORAGE_QUERY_PROPERTY`
  (USB 7, SD 12, MMC 13) + `SetThreadErrorMode(SEM_FAILCRITICALERRORS)`. Linux: `poll(POLLPRI)` auf
  `/proc/self/mountinfo` (öffnen, lesen, dann warten), Unterschied bilden, Gerät über udev-DB
  (`ID_BUS=usb`, Label/UUID) statt `removable` klassifizieren.
- **Wach halten (nach Kritik B10):** Windows `PowerCreateRequest` + `PowerSetRequest`
  (`PowerRequestSystemRequired`, zusätzlich `PowerRequestExecutionRequired` wo verfügbar), freigeben mit
  `PowerClearRequest`, dazu `ProcessPowerThrottling` aus für die Dauer. Linux: logind
  `Inhibit("idle:sleep", …, "block")` über das schon genutzte `zbus` (blockierende API, nicht aus einem
  Tokio-Kontext) – die Sperre endet mit dem Prozess, auch nach einem Absturz; ohne Berechtigung Rückfall
  auf `idle`; ohne logind/elogind einmal protokolliert. Verworfen: `systemd-inhibit` als Kindprozess
  (bliebe nach einem Absturz des Dienstes stehen). Abgeschalteter Windows-
  Autostart: `…\Explorer\StartupApproved\Run` (`03` + FILETIME = aus) nur lesen und anzeigen.
- **Unicode:** `icu_normalizer` 2.2 (liegt schon im Lock) mit `default-features = false,
  features = ["compiled_data"]`; NFC für Planungsschlüssel.
- **Abhängigkeiten zentral:** `native/Cargo.toml`, `native/Cargo.lock`, `share-server/Cargo.toml` und
  `share-server/Cargo.lock` ändert nur der Orchestrator (Anfrage über `anfragen/`); neue Einträge werden
  statisch mit `cargo metadata --offline` aufgelöst. Vorab ergänzt: windows-sys-Features
  `Win32_System_Ioctl`, `Win32_System_Power`, `Win32_Devices_DeviceAndDriverInstallation`,
  `Win32_System_WindowsProgramming`, `Win32_System_Diagnostics_Debug`; direkte Abhängigkeiten
  `quick-xml 0.39`, `socket2 0.6`, `icu_normalizer 2.2`, `flate2 1` (alle schon im Lock).
