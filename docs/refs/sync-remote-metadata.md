# Remote-Ziele beim Sync – Änderungszeit, Einzelabfrage, Fehlerklassen (Überblick)
Quelle: die Quellen der folgenden Abschnitte · Abgerufen: 2026-10-02

Faktenreferenz, keine Entwurfsentscheidung. **(unsicher)** = aus Quelltext oder Drittquelle gefolgert, nicht gegen
einen echten Server geprüft. `Datei:Zeile` ohne Präfix = Crate des jeweiligen Abschnitts.

Crate-Quellcode: `https://static.crates.io/crates/<name>/<name>-<version>.crate`. Die SHA-256-Summe jedes Archivs
wurde mit `native/Cargo.lock` verglichen und stimmt überein (russh-sftp 2.3.0, suppaftp 6.3.0, ureq 2.12.1,
roxmltree 0.20.0, flate2 1.1.9, smb2 0.26.0, quick-xml 0.39.2, miniz_oxide 0.8.9, socket2 0.6.4 und 0.5.10).
quick-xml 0.42.0 steht nicht im Lock (aktuelle Version, crates.io-API: 2026-08-22, MIT).

Nicht erneut recherchiert, nur verwiesen: [ftp-pool.md](ftp-pool.md) (`FtpError`/`Response`/`Status`, 421/530 bei
Anmeldung, RETR/STOR/MKD/REST), [gdrive-ureq-throughput.md](gdrive-ureq-throughput.md) (Upload-Typen, vorab erzeugte
IDs/409, Quoten/Backoff, `files.list` `pageSize`/`fields`, ureq-Pool und Wiederholungsregeln),
[quic-sftp-throughput.md](quic-sftp-throughput.md) (`RawSftpSession`-Grundlagen, `limits@openssh.com`, `copy-data`,
MaxSessions), [smb2.md](smb2.md) (`stat`, Rename-Bauplan mit `CompoundOp`, ErrorKind-Tabelle).

## Kurzmatrix

| Ziel | Quell-mtime übernehmen | Auflösung | Einzeleintrag exakt | „Platz voll“ | Rename auf vorhandenes Ziel |
|---|---|---|---|---|---|
| SFTP (OpenSSH) | SETSTAT/FSETSTAT, atime **und** mtime | 1 s (`u32`) | STAT/LSTAT, fehlt = `NoSuchFile` | `Failure` (mehrdeutig) | RENAME: `Failure`; `posix-rename@openssh.com` ersetzt |
| FTP | `MFMT` (nur wenn FEAT es nennt); vsftpd: `MDTM <zeit> <datei>` | 1 s (Antwort nennt den Istwert) | `MLST` (FEAT), sonst SIZE+MDTM (550 mehrdeutig) | 452/552; vsftpd 451 | servergebunden: vsftpd/Pure-FTPd ersetzen, ProFTPD-Standard/FileZilla 0.9/IIS 550 |
| WebDAV Nextcloud/ownCloud | PUT-Header `X-OC-Mtime`; sonst PROPPATCH `{DAV:}lastmodified` | 1 s | PROPFIND `Depth: 0` | 507 vorab, 413 beim Schreiben | MOVE `Overwrite: F` → 412 |
| WebDAV nginx-dav | PUT-Header `Date` | 1 s | kein PROPFIND im Modul (Doku: nur PUT, DELETE, MKCOL, COPY, MOVE); HEAD über den statischen Handler | 507 | `Overwrite: F` → 412 |
| WebDAV Apache 2.4 | nein (trunk: `DavHonorMtimeHeader`) | – | PROPFIND | 507 | `Overwrite: F` → 412 |
| Google Drive | `modifiedTime` in den Metadaten | 1 ms | `files.get` / `files.list` mit `q` | 403 `storageQuotaExceeded` | Namen nicht eindeutig; Ersetzen per ID |
| SMB (smb2 0.26.0) | **keine API**; eigenes SET_INFO(FileBasicInformation) | 100 ns (Dateisystem-abhängig) | `stat` (1 RTT) | `ErrorKind::DiskFull` | `rename`: `AlreadyExists` (smb2.md) |

---

# russh-sftp 2.3.0 + OpenSSH sftp-server (SFTP v3)
Quelle: Crate `russh-sftp-2.3.0` (`src/protocol/{file_attrs,status,mod,extended}.rs`,
`src/client/{rawsession,session,error,mod}.rs`, `src/client/fs/{dir,file,mod}.rs`, `src/{ser,extensions,buf}.rs`) ·
openssh-portable master `sftp-server.c` v1.156 (2026-07-01) und `PROTOCOL` §4.3/4.4/4.7
(`raw.githubusercontent.com/openssh/openssh-portable/master/`) · draft-ietf-secsh-filexfer-02 (§5 Attribute, §6.5
Rename, §7 Status) und -13 (§8.3, §9.1) auf `ietf.org/archive/id/` · `man7.org/linux/man-pages/man2/utimes.2.html` ·
rclone `backend/sftp/sftp.go` und `rclone.org/sftp/` · `proftpd.org/docs/contrib/mod_sftp.html` · Abgerufen: 2026-10-02

Das Crate spricht nur Protokoll **3** (`protocol/mod.rs:67` `VERSION = 3`, `Init::default()` sendet es). Die Codes 9–19
(`FILE_ALREADY_EXISTS`, `NO_SPACE_ON_FILESYSTEM`, `QUOTA_EXCEEDED` …) und die Rename-Flags `OVERWRITE/ATOMIC` stehen
erst in draft-13 (v6, §8.3/§9.1) und sind hier nicht erreichbar.

## Dateiattribute: mtime setzen (SETSTAT / FSETSTAT)
```rust
// protocol/file_attrs.rs:193-202 (alle Felder pub; „flags“ entsteht beim Serialisieren aus den Some-Feldern)
pub struct FileAttributes {
    pub size: Option<u64>, pub uid: Option<u32>, pub user: Option<String>,
    pub gid: Option<u32>,  pub group: Option<String>, pub permissions: Option<u32>,
    pub atime: Option<u32>, pub mtime: Option<u32>,
}
// client/fs/mod.rs:12   pub type Metadata = FileAttributes;
// client/session.rs:243 SftpSession::set_metadata<P: Into<String>>(&self, path: P, metadata: Metadata) -> Result<(), Error>      // SETSTAT
// client/fs/file.rs:76  File::set_metadata(&self, metadata: Metadata) -> SftpResult<()>                                          // FSETSTAT, offenes Handle
// rawsession.rs:434/455 RawSftpSession::setstat<P: Into<String>>(&self, path: P, attrs: FileAttributes) -> SftpResult<Status>
//                       RawSftpSession::fsetstat<H: Into<String>>(&self, handle: H, attrs: FileAttributes) -> SftpResult<Status>
```
- **Auflösung**: `atime`/`mtime` sind `u32` Sekunden seit 1970-01-01 UTC (draft-02 §5: „seconds from Jan 1, 1970 in
  UTC“). Darstellbar 1970-01-01 … 2106-02-07T06:28:15Z, keine Bruchteile, nichts vor 1970. OpenSSH ruft `utimes()` mit
  `tv_usec = 0` (`attrib_to_tv`, sftp-server.c:955-964; SETSTAT :979-1023, FSETSTAT mit `futimes` :1026-1088).
  Außerhalb `0..=u32::MAX` nicht senden (`From<&std::fs::Metadata>` und `utils::unix`, utils.rs:9, schneiden mit `as u32`
  ab; file_attrs.rs:314-340).
- **Paar-Falle**: Serialize setzt das Flag `ACMODTIME` (0x08), sobald `atime` **oder** `mtime` Some ist, und schreibt
  beide Felder, ein fehlendes als `0` (file_attrs.rs:365-369, 386-389). Nur `mtime` zu setzen stellt also atime auf
  1970-01-01. Dasselbe bei `uid`/`gid` (:355-358, 377-380): nur `uid` ⇒ `gid = 0` (root). Immer beide Zeiten setzen
  (rclone: `Chtimes(path, modTime, modTime)`, sftp.go:2680) und als Basis `FileAttributes::empty()` (:283) nehmen,
  **nie** `..Default::default()` (`Default` füllt size 0, uid/gid 0, `0o777|DIR`, Zeiten 0, :298-311).
- Server: OpenSSH arbeitet size → permissions → ACMODTIME → UIDGID nacheinander ab; ein Fehler ändert nur den
  Status, spätere Schritte laufen weiter (letzter Fehler gewinnt). `utimes(2)` mit expliziter Zeit liefert **EPERM,
  wenn der Aufrufer nicht Eigentümer ist** (auch bei Schreibrecht) ⇒ `PermissionDenied`.
- **OK beweist nichts**: ProFTPD `SFTPOptions IgnoreSFTPSetTimes` verwirft Zeiten „silently“; rclone-Doku: manche
  Server verbieten das Setzen nach dem Upload (Option `set_modtime = false`). Zeit also best effort setzen, per
  `metadata()` zurücklesen (rclone liest nach `Chtimes` per `stat` nach, sftp.go:2680-2690) und „Server übernimmt mtime
  nicht“ pro Server merken. Ob jeder Server die per FSETSTAT gesetzte Zeit bis CLOSE hält, ist nicht belegt **(unsicher)**;
  rclone setzt nach dem fertigen Upload per Pfad.
```rust
use russh_sftp::{client::error::Error as SftpError, protocol::{FileAttributes, StatusCode}};
let attrs = FileAttributes { atime: Some(secs), mtime: Some(secs), ..FileAttributes::empty() };
match sftp.set_metadata(path, attrs).await {
    Ok(()) => { /* danach metadata() lesen und mtime vergleichen */ }
    Err(SftpError::Status(s)) if s.status_code == StatusCode::PermissionDenied => { /* kein Eigentümer / Policy */ }
    Err(e) => return Err(e),
}
```

## Einzelne Einträge abfragen
```rust
// session.rs:238/251/161   (alle &self, async)
metadata<P: Into<String>>(&self, path: P) -> SftpResult<Metadata>            // STAT, folgt Symlinks
symlink_metadata<P: Into<String>>(&self, path: P) -> SftpResult<Metadata>    // LSTAT
try_exists<P: Into<String>>(&self, path: P) -> SftpResult<bool>              // Ok(false) NUR bei Status NoSuchFile
// fs/file.rs:71   File::metadata(&self) -> SftpResult<Metadata>              // FSTAT
// rawsession.rs:589/402/418  RawSftpSession::{stat, lstat}<P: Into<String>>, fstat<H: Into<String>> -> SftpResult<Attrs>  // Attrs{id, attrs}
```
- Fehlende Felder sind `None` (Flags der Antwort); `FileAttributes::modified()` (:275) liefert dann `Err(InvalidData)`.
  `permissions` trägt die Typbits (`is_dir()`, `is_regular()`, `is_symlink()`, `file_type()`).
- `try_exists` reicht jeden anderen Fehler durch (z. B. `PermissionDenied`, `Timeout`) – nicht als „fehlt“ lesen.
- Namen kommen als `String` über `from_utf8_lossy` (buf.rs:25): Nicht-UTF-8-Namen werden zu U+FFFD und sind danach
  unter diesem Namen **nicht mehr ansprechbar** (STAT liefert `NoSuchFile`).
- `read_dir` (session.rs:170) sammelt alle READDIR-Antworten (jede Antwort 10-s-Timeout), der Iterator filtert `.`/`..`
  (fs/dir.rs:61).

## Fehler: wie Statuscodes in Rust ankommen
`StatusCode` (protocol/status.rs:7-41): `Ok=0, Eof=1, NoSuchFile=2, PermissionDenied=3, Failure=4, BadMessage=5,
NoConnection=6, ConnectionLost=7, OpUnsupported=8`. `Status{id, status_code, error_message, language_tag}` (:46-51).
`client::error::Error` (client/error.rs:10-29): `Status(Status)`, `IO(String)`, `Timeout`, `Limited(String)`,
`UnexpectedPacket`, `UnexpectedBehavior(String)`. Konvertierung: Methoden mit `into_with_status!`
(`stat`, `read`, `open`, …; rawsession.rs:128) und `into_status!` (`setstat`, `rename`, `remove`, …; :138) machen jeden
Status ≠ `Ok` zu `Err(Error::Status(..))`. **`extended()` (:665) gibt den rohen `Packet` zurück** – ein
`Packet::Status` mit Fehlercode ist dort **kein** `Err` und muss selbst geprüft werden.

| Rust-Wert | OpenSSH-Auslöser (`errno_to_portable`, sftp-server.c:213-244) | Einordnung |
|---|---|---|
| `Status{NoSuchFile}` | ENOENT, ENOTDIR, EBADF, ELOOP | Pfad oder Elternteil fehlt (auch „Elternteil ist Datei“) |
| `Status{PermissionDenied}` | EPERM, EACCES, EFAULT | keine Rechte; `utimes` mit Zeit als Nicht-Eigentümer |
| `Status{BadMessage}` | ENAMETOOLONG, EINVAL | **Name zu lang oder Argument ungültig**, nicht zwingend ein Protokollfehler |
| `Status{OpUnsupported}` | ENOSYS; unbekannte `SSH_FXP_EXTENDED`-Anfrage | Erweiterung fehlt |
| `Status{Failure}` | alles andere: ENOSPC, EDQUOT, EEXIST, EROFS, EXDEV …; Teilschreiben (`process_write`) | mehrdeutig; Text immer „Failure“ (`status_to_message` :523-538), die Ursache steht nur im Serverlog |
| `Status{Eof}` | READ am Dateiende, READDIR fertig | kein Fehler (`read_dir`/Leser werten ihn aus) |
| `Timeout` | keine Antwort in `Config.request_timeout_secs` (10 s), **oder die Antwort war nicht dekodierbar** | siehe unten |
| `UnexpectedBehavior("session closed")` | Kanal tot (rawsession.rs:192-194) | neu verbinden |
| `Limited` | Paket größer als `limits@openssh.com` meldet (:198-202) | Anfrage verkleinern |

- Nicht dekodierbare Antworten (z. B. ein Statuscode > 8, wie ihn ein Server außerhalb der v3-Spezifikation senden
  könnte, oder ein unbekannter Pakettyp) landen nur als `warn!` im Lese-Task (client/mod.rs:97); die wartende Anfrage
  endet erst per `Timeout` **(unsicher: aus Quelltext gefolgert)**. Nach Verbindungsverlust enden laufende Anfragen
  ebenfalls per `Timeout`, spätere mit `"session closed"`.
- Das EXTENDED-Flag (`0x80000000`) wird beim Dekodieren nicht ausgewertet (file_attrs.rs:415-450, „todo: extended
  implementation“ :391): Server mit erweiterten Attributpaaren würden NAME-Listen verschieben **(unsicher, nur
  Quelltext)**; OpenSSH sendet sie nicht (`stat_to_attrib` setzt nur SIZE/UIDGID/PERMISSIONS/ACMODTIME).
- „Platz voll“ ist hier **nicht unterscheidbar** von EEXIST/EROFS: bei `Failure` auf WRITE/CLOSE/SETSTAT bzw. RENAME
  `statvfs` (unten) und `stat` des Ziels heranziehen.

## Umbenennen: SSH_FXP_RENAME, `posix-rename@openssh.com`, Ersatz
- `SftpSession::rename<O: Into<String>, N: Into<String>>(&self, oldpath: O, newpath: N) -> SftpResult<()>`
  (session.rs:220) sendet `SSH_FXP_RENAME`. draft-02 §6.5: „It is an error if there already exists a file with the
  name specified by newpath.“
- OpenSSH `process_rename` (sftp-server.c:1261-1318): normale Datei → `link(old,new)`, schlägt bei vorhandenem Ziel
  fehl (EEXIST) und ergibt `Failure`, danach `unlink(old)` (rennfrei); fehlt Hardlink-Unterstützung
  (EOPNOTSUPP/ENOSYS/EXDEV): `stat(new)` + `rename`, vorhandenes Ziel lässt den Status auf `Failure`; Verzeichnisse und
  Nicht-Dateien: `stat(new)` fehlgeschlagen → `rename`, sonst `Failure`. Quelle fehlt → `NoSuchFile`.
- `posix-rename@openssh.com` (PROTOCOL §4.3): `SSH_FXP_EXTENDED`, Felder `string oldpath`, `string newpath`, Server
  macht `rename(oldpath,newpath)` (`process_extended_posix_rename`, sftp-server.c:1366-1382) – **ersetzt ein vorhandenes
  Ziel atomar**. Verfügbar, wenn `Version.extensions["posix-rename@openssh.com"] == "1"` (`process_init` :717).
  OpenSSH meldet außerdem `statvfs@openssh.com` „2“, `fstatvfs@…` „2“, `hardlink@…`, `fsync@…`, `lsetstat@…`,
  `limits@…`, `expand-path@…`, `copy-data`, `home-directory`, `users-groups-by-id@…` (:717-727).
- russh-sftp 2.3.0 hat dafür **keine Methode** (`extensions.rs:3-6`: nur `LIMITS`, `HARDLINK`, `FSYNC`, `STATVFS`).
  Bauweise: `RawSftpSession::extended<R: Into<String>>(&self, request: R, data: Vec<u8>) -> SftpResult<Packet>`
  (rawsession.rs:665); `data` wird **ohne Längenpräfix** angehängt (`Extended.data` mit `data_serialize`,
  protocol/extended.rs, ser.rs:26-35 / `serialize_seq(None)` :164-170). Zwei SSH-Strings (u32-BE-Länge + Bytes) liefert
  `russh_sftp::ser::to_bytes` (wie `HardlinkExtension`, extensions.rs:29-34); das Projekt tut das bereits
  (`sftp/core/posix_rename.rs`, laut quic-sftp-throughput.md).
```rust
#[derive(serde::Serialize)] struct PosixRename { oldpath: String, newpath: String }
let data = russh_sftp::ser::to_bytes(&PosixRename { oldpath, newpath })?.to_vec();
match raw.extended("posix-rename@openssh.com", data).await? {
    russh_sftp::protocol::Packet::Status(s) if s.status_code == StatusCode::Ok => Ok(()),
    russh_sftp::protocol::Packet::Status(s) => Err(SftpError::Status(s)),
    _ => Err(SftpError::UnexpectedPacket),
}
```
- **Server ohne die Erweiterung**: nur das v3-RENAME (Fehler bei vorhandenem Ziel). rclone-Fallback (sftp.go:1978-1986):
  `PosixRename`, wenn `HasExtension("posix-rename@openssh.com")`, sonst vorhandenes Ziel **löschen**, dann `Rename`
  (nicht atomar, kurzes Fenster ohne Ziel). ProFTPD mod_sftp bietet laut Doku `posix-rename@openssh.com`,
  `statvfs@`/`fstatvfs@`, `hardlink@` sowie `check-file`, `copy-file`, `version-select`, `vendor-id`.

## READDIR-Portionierung
OpenSSH `process_readdir` (sftp-server.c:1121-1176): liest per `readdir()`, `lstat` je Eintrag (Fehlschlag = Eintrag
entfällt), bis **100 Einträge** („send up to 100 entries in one message“, `if (count == 100) break`, Kommentar „XXX check
packet size instead“) in **einer** `SSH_FXP_NAME`-Antwort; danach nächste Antwort, am Ende `SSH_FX_EOF`. `.` und `..`
sind enthalten. Jeder Eintrag trägt zusätzlich den `ls -l`-Langnamen. N Einträge ⇒ `ceil(N/100) + 1` Roundtrips
nacheinander.

## Freier Platz: `statvfs@openssh.com`
`SftpSession::fs_info<P: Into<String>>(&self, path: P) -> SftpResult<Option<Statvfs>>` (session.rs:269) gibt `Ok(None)`,
wenn der Server `statvfs@openssh.com` „2“ nicht meldet. `Statvfs` (extensions.rs:51-75, alle `u64`): `block_size,
fragment_size, blocks, blocks_free, blocks_avail, inodes, inodes_free, inodes_avail, fs_id, flags, name_max`. Frei für
Nicht-root = `blocks_avail * fragment_size` (PROTOCOL §4.4: `f_bavail` „free blocks for non-root“, Einheit `f_frsize`);
`flags & 0x1` = `SSH_FXE_STATVFS_ST_RDONLY`. Nützlich vor großen Uploads und um `Failure` nach WRITE zu deuten.

---

# suppaftp 6.3.0 (FTP/FTPS, synchron)
Quelle: Crate `suppaftp-6.3.0` (`src/sync_ftp/{mod,data_stream}.rs`, `src/{types,status,command,regex,list}.rs`,
`src/command/feat.rs`) · RFC 959 §4.2, RFC 2389 §3, RFC 3659 §2.3/3/4/7 (`rfc-editor.org/rfc/`) ·
draft-somers-ftp-mfxx-04 §3 (`ietf.org/archive/id/`) · vsftpd 3.0.5 Quelltext
(`security.appspot.com/downloads/vsftpd-3.0.5.tar.gz`, `postlogin.c`, `features.c`, `vsftpd.conf.5`) · ProFTPD master
(`modules/{mod_core,mod_xfer,mod_facts}.c`, `src/data.c`, `proftpd.org/docs/modules/{mod_xfer,mod_core,mod_facts}.html`) ·
Pure-FTPd master (`src/ftpd.c`, `src/messages_en.h`) · FileZilla-Tickets `trac.filezilla-project.org/ticket/{173,2456,3669}` ·
`markwilson.co.uk/blog/2004/10/allowing-files-to-be-replaced-as-part.htm` · `socket2` 0.6.4 Quelltext · MS Learn
`SIO_KEEPALIVE_VALS` · `man7.org/linux/man-pages/man7/tcp.7.html` · Abgerufen: 2026-10-02

Alle Methoden hängen an `ImplFtpStream<T>`; Aliase `FtpStream`, `RustlsFtpStream` (lib.rs:167/188); öffentlich
`suppaftp::{Status, FtpError, FtpResult, Mode, DataStream, ImplFtpStream}`, `suppaftp::types::{Response, Features,
FileType}`, `suppaftp::list::File`. Typen/Fehlerform: siehe ftp-pool.md. Neu hier: `Response.body` enthält die
**Rohzeilen inklusive Code** (`read_response_in`, mod.rs:801-831), und `Status::from` kennt nur die Codes aus
status.rs; ein unbekannter Code ergibt `Status::Unknown` (`code() == 0`) – dann den Code aus `body[..3]` lesen.

## FEAT / OPTS
```rust
pub fn feat(&mut self) -> FtpResult<Features>                       // mod.rs:701; Features = HashMap<String, Option<String>> (types.rs:87)
pub fn opts(&mut self, option: impl ToString, value: Option<impl ToString>) -> FtpResult<()>   // :730, erwartet 200
```
- Schlüssel = erstes Wort der Zeile **wie gesendet** (keine Normalisierung), Wert = Rest mit Einzelleerzeichen,
  `None` ohne Argument (feat.rs:54-70), z. B. `"MLST" → Some("type*;size*;modify*;perm*;unique*;")`. Vergleiche
  case-insensitiv (RFC 3659 §3.3/4.3: Groß/Klein beliebig). Jede Merkmalszeile muss mit **einem** Leerzeichen beginnen,
  sonst `BadResponse` für den ganzen Aufruf (feat.rs:55-58).
- Kein FEAT-Support = Antwort 500/502 (RFC 2389 §3.2) ⇒ `Err(UnexpectedResponse(r))` mit `r.status` =
  `Status::BadCommand`/`Status::NotImplemented` – als „keine Erweiterungen“ behandeln, nicht als Fehler. `211 …`
  einzeilig = keine Merkmale.
- `OPTS UTF8 ON` und `OPTS MLST type;size;modify;unique;perm;` (RFC 3659 §7.9: die Faktenliste gilt für MLST **und**
  MLSD bis zum nächsten OPTS; Antwort `MLST OPTS …`).

## MLST / MLSD (RFC 3659 §7)
```rust
pub fn mlst(&mut self, pathname: Option<&str>) -> FtpResult<String>        // mod.rs:637, erwartet 250
pub fn mlsd(&mut self, pathname: Option<&str>) -> FtpResult<Vec<String>>   // :623, Datenverbindung, 150/125 dann 226/250
```
- Zeilenformat: `facts SP pathname`, `facts = 1*( fact ";" )`, `fact = name "=" value`; **genau ein** Leerzeichen trennt
  Fakten und Namen, weitere Leerzeichen gehören zum Namen; keine Leerzeichen in den Fakten; Namen sind Oktettfolgen
  (UTF-8 empfohlen); Faktennamen case-insensitiv. MLST antwortet `250-` + ` facts SP name` + `250 End` (§7.2).
- Fakten (§7.5): `type` = `file` | `dir` | `cdir` | `pdir` | `OS.<os>=<art>`; `size` (Oktette; „approximate“ – genau nur
  SIZE); `modify` = UTC `YYYYMMDDHHMMSS[.sss]`; `create`; `unique` (opaker, case-sensitiver Token); `perm` =
  Buchstaben `a c d e f l m p r w` (nur Hinweis, „can never imply that the appropriate command is guaranteed“);
  `lang`, `media-type`, `charset`. MLSD kann Einträge `cdir`/`pdir` enthalten (§7.3.1); Datenverbindung wie
  `TYPE L 8` (§7.2); Zeilenlänge unbegrenzt.
- suppaftp: `mlst` liefert nur die **erste** Faktenzeile und `trim()`t sie (mod.rs:637-650) – Leerzeichen am Namensende
  gehen verloren. `mlsd` liefert Rohzeilen, nicht-UTF-8 wird lossy gewandelt, Leerzeilen entfallen (mod.rs:763-795,
  1019-1025).
- Beobachtete Server: Pure-FTPd sendet `type=file;size=N;modify=…;UNIX.mode=0644;UNIX.uid=…;UNIX.gid=…;unique=…; name`,
  für Verzeichnisse **`sizd`** statt `size`, Typen `file|dir|cdir|pdir|OS.unix=symlink|OS.unix=slink:<ziel>|unknown`
  (ftpd.c:1085-1150, FEAT :3503-3507); ProFTPD `modify`, `perm`, `type` (`cdir`/`pdir`/`OS.unix=slink:`), `unique`,
  `UNIX.mode=0%o` (mod_facts.c:318-422); **vsftpd kennt weder MLST noch MLSD** (FEAT: AUTH TLS, EPRT, EPSV, MDTM, PASV,
  PBSZ, PROT, REST STREAM, SIZE, TVFS, UTF8; features.c:25-60).
- **`suppaftp::list::File::from_mlsx_line` (list.rs:191-267) ist für echte Server unbrauchbar**: akzeptiert nur
  `type=dir|file|link` (alles andere, auch `cdir`, `pdir`, `OS.unix=…`, `unknown`, ⇒ `Err(SyntaxError)` für die ganze
  Zeile); `modify` muss exakt `%Y%m%d%H%M%S` sein (Bruchteil ⇒ `InvalidDate`, :440); `unix.mode` muss genau 3 Zeichen
  haben – ProFTPD und Pure-FTPd senden `0644` ⇒ `SyntaxError`; der Name ist der Text nach dem **letzten** `;`
  (Namen mit `;` zerbrechen) und wird `trim_start()`ed; kein `unique`/`perm`. Eigener Parser: am **ersten** Leerzeichen
  teilen, Fakten an `;`, Schlüssel case-insensitiv, Unbekanntes ignorieren, `cdir`/`pdir` überspringen, `sizd` kennen.
- Ohne MLSx bleiben `LIST` im ls-Format (`Nov 5 13:46` **oder** `Nov 5 2019`: Minuten- bzw. Tagesauflösung, **keine
  Zeitzone**; suppaftp deutet die Zeit als UTC, list.rs:454-488) und je Datei `MDTM`+`SIZE`.

## MDTM, SIZE
```rust
pub fn mdtm<S: AsRef<str>>(&mut self, pathname: S) -> FtpResult<chrono::NaiveDateTime>   // mod.rs:652, erwartet 213
pub fn size<S: AsRef<str>>(&mut self, pathname: S) -> FtpResult<usize>                   // mod.rs:688, erwartet 213
```
- `mdtm` sucht per Regex `\b(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})\b` irgendwo im Antworttext (regex.rs:16): ein
  Bruchteil wird verworfen, ein Fehlschlag ist `BadResponse`. RFC 3659 §2.3/§3.1: Antwort `213 time-val`, **immer UTC**,
  einzeilig. **vsftpd** antwortet bei `use_localtime=YES` (Standard NO) in Serverlokalzeit (`handle_mdtm` nutzt
  `tunable_use_localtime`, postlogin.c:1605-1675).
- `size` liest `\s+(\d+)\s*$` (regex.rs:19) und `.parse().unwrap()` (mod.rs:695): auf 32-Bit-Zielen paniert eine Größe
  über `usize::MAX`. RFC 3659 §4: die Größe hängt von `TYPE`/`MODE`/`STRU` ab ⇒ vorher `transfer_type(FileType::Binary)`.
- **550 auf SIZE/MDTM ist mehrdeutig**: RFC 3659 §4.2: „MUST NOT be taken … as an indication that the file cannot be
  transferred“; vsftpd antwortet 550 für fehlende **und** für nicht reguläre Dateien („Could not get file size.“, jede
  Verzeichnisabfrage) sowie `550 Permission denied.` (postlogin.c:1469-1496). Existenz einer Datei: MLST, sonst
  SIZE; Verzeichnisse: MLST (`type=dir`) oder `CWD`/Eltern-Listing.

## Änderungszeit setzen: MFMT, vsftpd-MDTM (kein suppaftp-Aufruf)
```rust
pub fn custom_command(&mut self, command: impl ToString, expected_code: &[Status]) -> FtpResult<Response>   // mod.rs:749
```
Der String geht **unverändert + CRLF** auf die Leitung (`Command::Custom(c) => c.clone()`, command.rs:180-182). Namen
mit `\r`/`\n` vorher ablehnen: suppaftp maskiert in **keinem** Kommando (Befehlseinschleusung auf dem Steuerkanal).
- **MFMT** (draft-somers-ftp-mfxx-04 §3, abgelaufener Entwurf, „Experimental“): `MFMT SP time-val SP pathname CRLF`,
  `time-val` UTC `YYYYMMDDHHMMSS[.sss]`; Erfolg `213 Modify=<time-val>; <pathname>` (**tatsächlich gespeicherter**
  Wert, kann gröber sein), Fehler: 550 (Objekt fehlt), 500/501 (Syntax; ungültiges Datum = 501), 4xx möglich; im FEAT
  eine Zeile ` MFMT`. Umsetzung: ProFTPD mod_facts (`facts_mfmt`, `213 Modify=%s; %s`, nur mit Schreibrecht,
  mod_facts.c:1423-1540), Pure-FTPd meldet `MFMT` im FEAT (ftpd.c:3503); vsftpd nicht.
- **vsftpd**: `MDTM <YYYYMMDDHHMMSS[.s]> <datei>` setzt die mtime, wenn `mdtm_write=YES` (Standard JA) und Schreiben
  erlaubt ist; Antwort `213 File modification time set.` bzw. `550 Could not set file modification time.`; Zeit GMT,
  außer `use_localtime=YES` (postlogin.c:1605-1675, tunables.c:219). Der MFMT-Entwurf (§1/§3) nennt das Setzen per MDTM
  einen Missbrauch des Lese-Befehls.
```rust
use suppaftp::Status;
let ts = chrono::DateTime::<chrono::Utc>::from(system_time).format("%Y%m%d%H%M%S").to_string();   // UTC, ganze Sekunden
let r = ftp.custom_command(format!("MFMT {ts} {path}"), &[Status::File])?;                          // Status::File = 213
// r.as_string() == "213 Modify=20261002123456; /pfad/datei"   (zurücklesen, Istwert vergleichen)
```

## Antwortcodes (RFC 959 §4.2.1/4.2.2) und wo sie in suppaftp ankommen
| Code | RFC 959 | Beobachtet |
|---|---|---|
| 450 | Datei nicht verfügbar (z. B. belegt), vorübergehend | – |
| 451 | Aktion abgebrochen, lokaler Fehler, vorübergehend | **vsftpd: Schreibfehler beim Upload, auch Platte voll** (`Failure writing to local file.`, postlogin.c:1150); ProFTPD EAGAIN/ENOMEM/EBUSY (data.c:806-832); Pure-FTPd Lesefehler/„Rename/move failure“ |
| 452 | zu wenig Speicherplatz im System, vorübergehend | ProFTPD ENOSPC (data.c:834-837); Pure-FTPd Schreibfehler „Error during write to file“ (ftpd.c:3975); FileZilla-Ticket #3669: „452 Error writing file: Disc quota exceeded.“ (Server nicht genannt) |
| 550 | Datei nicht verfügbar (fehlt, kein Zugriff), dauerhaft | SIZE/MDTM s. o.; RNTO s. u.; ProFTPD STOR über vorhandene Datei „Overwrite permission denied“ (mod_xfer.c:1517/1823) |
| 552 | Speicherzuteilung überschritten, dauerhaft | ProFTPD EDQUOT/EFBIG (data.c:840-847) und `ALLO <größe>` über dem freien Platz: „…: No space left on device“ (ALLO wird standardmäßig behandelt, mod_xfer.c:64-67, 3452-3484); Pure-FTPd „Quota exceeded: […] won't be saved“ (ftpd.c:3449, 4402) und beim STOR-Start/ALLO „Disk full - please upload later“ (Freiplatzprüfung `ul_check_free_space`, ftpd.c:1218, 4181) |
| 553 | Dateiname nicht erlaubt, dauerhaft | vsftpd „Could not create file.“ beim Anlegen (postlogin.c:1067); Pure-FTPd „Prohibited file name“ |

- RFC-Matrix: STOR sofort `532, 450, 452, 553`, nach dem Datenfluss `425, 426, 451, 551, 552`; RNTO `250 | 532, 553`
  (kein 550 vorgesehen); DELE `450, 550`; MKD `550`.
- **STOR-Fehler kommen oft erst als Abschlussantwort**: `put_with_stream` (mod.rs:512) erwartet 125/150, sonst
  `UnexpectedResponse`; ein volles Ziel bricht aber meist erst während des Datenflusses ab – der Server schließt die
  Datenverbindung (Schreibfehler `EPIPE`/`ECONNRESET` im Datenstrom) und sendet 451/452/552 auf dem Steuerkanal. Dann
  `finalize_put_stream(stream)` (:522) aufrufen: es schließt den Strom zuerst und liest 226/250, sonst
  `UnexpectedResponse(452/552 …)` – **diese Antwort der io-Fehlermeldung vorziehen**. `put_file` (:500) kehrt bei einem
  `copy`-Fehler **ohne** Lesen der Antwort zurück; die Abschlussantwort bleibt ungelesen im Steuerkanal und wird vom
  nächsten Befehl als dessen Antwort gelesen **(unsicher: aus Quelltext gefolgert)**.

## RNFR/RNTO auf vorhandenes Ziel
`rename<S: AsRef<str>>(&mut self, from_name: S, to_name: S) -> FtpResult<()>` (mod.rs:383) erwartet 350 auf RNFR, 250 auf
RNTO. RFC 959 legt das Verhalten bei vorhandenem Ziel nicht fest.

| Server | Verhalten | Quelle |
|---|---|---|
| vsftpd 3.0.5 | `rename(2)`, **ersetzt still**; Fehler `550 Rename failed.` | `handle_rnto`, postlogin.c:1314-1353 („might overwrite destination file“) |
| Pure-FTPd (master) | `rename(2)`, **ersetzt**; Fehler `451 Rename/move failure`; Name unzulässig `553`; mit `disallow_rename` `550` | `dornto`, ftpd.c:4548-4614 |
| ProFTPD (master) | Ziel vorhanden und `AllowOverwrite` nicht `on` (**Standard off**): `550 <arg>: Rename permission denied`; mit `on` ersetzt | `core_rnto`, mod_core.c:6419-6438; Doku mod_xfer („By default, FTP clients cannot overwrite existing files.“) |
| FileZilla Server 0.9.x | verweigert, „550 file exists“; Wunsch „RNTO darf überschreiben“ abgelehnt | Trac #173 (Entwickler-Kommentar), #2456 (0.9.60); **Server 1.x nicht geprüft** |
| IIS FTP | `550 <name>: Cannot create a file when that file already exists.`; IIS 5.1: Metabase `AllowReplaceOnRename` erlaubt Ersetzen | markwilson.co.uk 2004; derselbe Text erscheint auf Windows 10/IIS 10 auch bei **Rechteproblemen** (MS Q&A 2016) ⇒ als „existiert“-Beleg unzuverlässig |

Folgerung: Atomares Ersetzen gibt es per FTP nicht; „temp hochladen, Ziel löschen, RNTO“ ist die portable Form, und
ein `550` auf RNTO ist ohne Gegenprobe (`MLST`/`SIZE` des Ziels) nicht als „Ziel existiert“ lesbar.

## TCP-Keepalive: Steuer- und Datenverbindung
- Beide Sockets sind zugänglich: `ImplFtpStream::get_ref(&self) -> &TcpStream` (Steuerkanal, mod.rs:275),
  `DataStream::get_ref(&self) -> &TcpStream` (data_stream.rs:40, auch bei TLS). Besser **vor** Gebrauch setzen:
  `ImplFtpStream::connect_with_stream(stream: TcpStream)` (mod.rs:79) nimmt einen vorbereiteten Socket;
  `passive_stream_builder<F>(self, f: F) where F: Fn(SocketAddr) -> FtpResult<TcpStream> + Send + Sync + 'static`
  (mod.rs:118, Typ :37) baut jede Passiv-**Datenverbindung** – dort Keepalive, `TCP_NODELAY` und einen **Connect-Timeout**
  setzen (der Standardbuilder ist `TcpStream::connect(addr)` ohne Timeout, :1028-1030).
- socket2 0.6.4 (liegt im Lock über tokio; 0.5.10 über mdns-sd; kein direkter Eintrag in `native/Cargo.toml`):
  `SockRef::from(&tcp_stream)` (`impl From<&S> for SockRef where S: AsFd` bzw. `AsSocket` auf Windows, sockref.rs),
  `Socket::set_tcp_keepalive(&self, params: &TcpKeepalive) -> io::Result<()>` (socket.rs:2346, schaltet `SO_KEEPALIVE`
  mit ein), `TcpKeepalive::new().with_time(Duration).with_interval(Duration)` (lib.rs:514/545). `with_interval` existiert
  auf Android, Linux, macOS, Windows u. a.; `with_retries` nur mit Cargo-Feature `all`. Windows: nicht gesetzte Werte
  können auf Standard zurückfallen (Doku zu `set_tcp_keepalive`), daher beide Werte setzen; auf Unix je in ganzen
  Sekunden (`into_secs`).
```rust
use socket2::{SockRef, TcpKeepalive};
let ka = TcpKeepalive::new().with_time(Duration::from_secs(60)).with_interval(Duration::from_secs(20));
let ctl = TcpStream::connect_timeout(&addr, Duration::from_secs(15))?;
SockRef::from(&ctl).set_tcp_keepalive(&ka)?;
let ftp = RustlsFtpStream::connect_with_stream(ctl)?
    .passive_stream_builder(move |a| {
        let d = TcpStream::connect_timeout(&a, Duration::from_secs(15)).map_err(FtpError::ConnectionError)?;
        SockRef::from(&d).set_tcp_keepalive(&ka).map_err(FtpError::ConnectionError)?;
        Ok(d)
    });
```
- Standardwerte sind für Mittelboxen wertlos: Linux `tcp_keepalive_time` 7200 s, `intvl` 75 s, `probes` 9 (tcp(7));
  Windows 2 h / 1 s, 10 Probes ab Vista (MS Learn `SIO_KEEPALIVE_VALS`). Typische NAT-/Firewall-Leerlaufzeiten liegen
  darunter **(Erfahrungswert, nicht belegt)**.
- Keepalive hält nur den TCP-Zustand, **nicht** die FTP-Zeitgeber der Server: vsftpd `idle_session_timeout` 300 s
  (zwischen Befehlen) und `data_connection_timeout` 300 s (Datenstillstand), ProFTPD `TimeoutIdle` 600 s (zurückgesetzt
  durch Daten auf Steuer- **oder** Datenverbindung), `TimeoutNoTransfer` 300 s, `TimeoutStalled` 3600 s. `NOOP`
  (`noop()`, erwartet 200, mod.rs:345) während einer Datenübertragung ist **nicht vorgesehen**: RFC 959 nennt für die
  Übertragungsphase ausdrücklich STAT („may be sent during a file transfer“, §4.1.3) und zum Abbrechen ABOR – von
  NOOP ist dort keine Rede.

---

# WebDAV-Server (Nextcloud/ownCloud, sabre/dav, Apache mod_dav, nginx dav, Synology)
Quelle: RFC 4918 §9.2.1/9.3.1/9.8.5/9.9.3-9.9.4/10.6/11.5/15.7 (`rfc-editor.org/rfc/rfc4918.txt`) · Nextcloud
Entwicklerhandbuch `docs.nextcloud.com/server/stable/developer_manual/client_apis/WebDAV/{basic,chunking}.html`
(Stand 2026-09-24) · nextcloud/server master (`remote.php`, `apps/dav/lib/Connector/Sabre/{File,Node,FilesPlugin,
QuotaPlugin,MtimeSanitizer}.php`, `apps/dav/lib/{RootCollection,Connector/Sabre/Exception/EntityTooLarge}.php`) ·
sabre-io/dav master (`lib/DAV/{Server,Collection,CorePlugin}.php`) · httpd trunk 2.5.1-dev (`modules/dav/main/{mod_dav,
props}.c`, `modules/dav/fs/repos.c`, `server/core.c`, `docs/manual/mod/mod_dav.xml`) und 2.4.x (Vergleich) · nginx master
(`src/http/modules/ngx_http_{dav,static}_module.c`, `nginx.org/en/docs/http/ngx_http_dav_module.html`) · rclone
`backend/webdav/webdav.go`, `rclone.org/webdav/` · Drittseiten (Synology) · Abgerufen: 2026-10-02

## PUT mit `X-OC-Mtime`, `OC-Checksum`, Auto-MKCOL (Nextcloud/ownCloud)
- Nextcloud-Doku (Tabelle „Request Headers“): `X-OC-MTime` „Allow to specify a modification time. The response will
  contain the header `X-OC-MTime: accepted` if the mtime was accepted.“ Beispiel `1675789581` = **Unix-Sekunden,
  Dezimalzahl**. Quelltext (`File::finalizeUpload`, File.php:360-395): Header `x-oc-mtime` ⇒ `MtimeSanitizer` ⇒
  `touch()`; **nur wenn `touch` gelang, kommt `X-OC-MTime: accepted`** (Antwort-Header fehlt sonst). Dasselbe für
  `X-OC-CTime` (Antwort `X-OC-CTime: accepted`).
- `MtimeSanitizer::sanitizeMtime` (MtimeSanitizer.php): nicht numerisch oder Hex ⇒ `InvalidArgumentException`; Wert
  `<= 86400` ⇒ `InvalidArgumentException` („greater than one day“); `(int)` kappt Bruchteile. `finalizeUpload` fängt das
  nicht und mappt es nicht auf eine Sabre-Ausnahme – der Statuscode ist vermutlich 500, **die Daten sind dann schon
  gespeichert (unsicher)**. Zeit also als ganze Sekunden > 86400 senden.
- `OC-Checksum` (Doku): „A checksum that will be stored in the DB. For regular PUT uploads, the server stores the value
  without validation. During bulk uploads, the checksum is validated against the uploaded content.“ Algorithmen `MD5`,
  `SHA1`, `SHA256`, `SHA3-256`, `Adler32`; Format `md5:04c36b…` (Doku-Beispiel; rclone sendet `SHA1:<hex>`/`MD5:<hex>`).
  Lesen: PROPFIND `oc:checksums` → `<oc:checksum>md5:04c36b…</oc:checksum>` (Doku-Beispiel). Quelltext (File.php:388-395):
  der Header wird bei PUT `trim()`-ed und **unverändert** gespeichert. rclone (webdav.go:1675-1700): ownCloud prüft
  **einen** Wert und speichert eigenes SHA1/MD5; Nextcloud speichert genau den gesendeten, aber nur einen.
- `X-Hash: md5|sha1|sha256|all` (PUT) lässt den Server beim Schreiben hashen, Antwort `X-Hash-MD5/-SHA1/-SHA256`
  (File.php:190-210). `OC-Total-Length`: Gesamtgröße bei Chunk-Uploads, „reject the chunk with a 507 Insufficient
  Storage“ bei zu wenig Quota (chunking.html); das abschließende MOVE trägt `X-OC-Mtime`.
- **`X-NC-WebDAV-Auto-Mkcol: 1`** (PUT, „Available since Nextcloud 32“) legt fehlende Elternordner selbst an (die Doku
  schreibt den Namen an einer Stelle `X-NC-WebDAV-AutoMkcol`; Quelltext-Beleg nicht gelesen **(unsicher)**).
- Antwortheader bei Anlage/MOVE/COPY: `OC-Etag`, `OC-FileId`, `X-NC-Permissions`, `X-NC-OwnerId`.
- Ohne `Content-Length` sendet ureq chunked (siehe ureq-Abschnitt); rclone setzt `Content-Length` immer
  (webdav.go:1719, Kommentar zu nextcloud-snap#365).
```rust
let r = agent.put(&url)
    .set("Content-Length", &len.to_string())
    .set("X-OC-Mtime", &mtime_secs.to_string())            // ganze Sekunden, > 86400
    .set("OC-Checksum", &format!("SHA1:{sha1_hex}"))        // optional
    .send(reader)?;                                          // ab Status 400: Err(ureq::Error::Status(code, resp))
let accepted = r.header("X-OC-MTime").is_some_and(|v| v.eq_ignore_ascii_case("accepted"));   // Header-Suche case-insensitiv (header.rs:137)
```

## PROPPATCH auf `getlastmodified`
- RFC 4918 §15.7: `DAV:getlastmodified` hat den Wert `rfc1123-date` und „SHOULD be protected“; §9.2.1: PROPPATCH auf
  ein geschütztes Merkmal ⇒ `403` mit Vorbedingung `cannot-modify-protected-property`.
- **sabre/dav** (Basis von Nextcloud/ownCloud): `{DAV:}getlastmodified` steht in `Server::$protectedProperties`
  (Server.php:104-134) ⇒ 403. **Nextcloud** bietet daneben ein **anderes** Merkmal `{DAV:}lastmodified` (ohne „get“,
  `FilesPlugin::LASTMODIFIED_PROPERTYNAME`, FilesPlugin.php:58, Handler :580-588): Wert = Unix-Sekunden (läuft durch
  `MtimeSanitizer`), setzt `touch`; in der Doku **nicht** beschrieben. rclone sendet es (webdav.go:1458-1500):
  `<D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><lastmodified xmlns="DAV:">SEKUNDEN</lastmodified></D:prop></D:set></D:propertyupdate>`.
  **Nebenwirkung: Das Setzen der mtime verwirft die gespeicherten Prüfsummen**; rclone schickt `oc:checksums` im selben
  PROPPATCH mit oder lässt per `PATCH` + `X-Recalculate-Hash: sha1|md5` neu berechnen (Antwort `OC-Checksum`,
  webdav.go:1650-1680). ownCloud/Nextcloud/oCIS: rclone aktiviert Header **und** PROPPATCH; Fastmail nur den Header;
  bei MOVE/COPY sendet rclone `X-OC-Mtime` mit und fällt auf PROPPATCH zurück, wenn `accepted` fehlt (:1186-1219).
- **Apache mod_dav** (Quelltext trunk gelesen, 2.4 nicht gesondert): `getlastmodified` ist Live-Property mit
  `is_writable = 0` ⇒ PROPPATCH-Fehler „Property is read-only.“ mit `HTTP_CONFLICT` (409) (props.c:1105-1115,
  repos.c:165-170); ein selbst erfundenes Merkmal wie
  `<lastmodified xmlns="DAV:">` wird als **Dead Property gespeichert und meldet Erfolg, ohne die mtime zu ändern**
  **(unsicher: aus Quelltext gefolgert)** ⇒ immer per PROPFIND `getlastmodified` zurücklesen. **Apache trunk (2.5.1-dev)**
  hat neu `DavHonorMtimeHeader on|off` (Standard off, Kontext `<Directory>`): bei PUT **und** MKCOL wird `X-OC-Mtime`
  (nur Ziffern, Sekunden) übernommen, ungültig ⇒ 400; kein `accepted`-Antwortheader (mod_dav.c:966-989, :1170-1180,
  :1276; Commit vom 2025-11-07). In 2.4.x gibt es das nicht (0 Treffer in Quelltext und Handbuch).
- **nginx ngx_http_dav_module**: kein PROPPATCH (Methoden nur PUT, DELETE, MKCOL, COPY, MOVE, `ngx_http_dav_methods_mask`).
  Dafür **PUT mit Header `Date`** (HTTP-Datum): „it is possible to specify the modification date by passing it in the
  „Date“ header field“ (Handbuch; `ngx_http_dav_module.c:270-283`, `ngx_parse_http_time`).
- **Synology WebDAV Server**: widersprüchliche Drittangaben (RaiDrive 2021 listet DSM 7.0 unter Servern, die das
  Ändern der Änderungszeit erlauben, ohne Mechanismus; ältere Forenberichte: mtime = Uploadzeit). **Keine offizielle
  Quelle gefunden.** Allgemein: „Plain WebDAV does not support modified times“ (rclone-Doku); Vendor-KB: „many servers
  do not allow PROPPATCH getlastmodified“.
- Konsequenz: Fähigkeit **pro Server prüfen und merken** (Header-`accepted`, sonst PROPFIND-Gegenprobe), nicht aus dem
  Statuscode schließen.

## MKCOL/PROPFIND oberhalb der DAV-Wurzel; MOVE `Overwrite: F`; 507
RFC 4918 §9.3.1 (MKCOL): `201` angelegt; `403` Ort nicht erlaubt oder Elternsammlung nimmt nichts auf; **`405` nur auf
unbelegter URL**; **`409` Eltern fehlen** (Server darf sie nicht automatisch anlegen); `415` Body-Typ; `507`.

| Server | MKCOL auf vorhandenem Pfad | Eltern fehlen | `Overwrite: F`, Ziel da | Platz voll |
|---|---|---|---|---|
| sabre/dav (`Server::createCollection`, Server.php:1175-1199; MOVE :744-772) | **405** („The resource you tried to create already exists“) | **409** (Eltern fehlen/keine Sammlung) | **412**; ungültiger Wert 400 (:752) | 507 (`Exception\InsufficientStorage`) |
| Nextcloud | wie sabre; Wurzel `/remote.php/dav/` ist eine `SimpleCollection` mit Kindern (`files`, `principals`, `uploads`, …, RootCollection.php): MKCOL dort **403** (`Collection::createDirectory` wirft `Forbidden`) bzw. **405** für vorhandene Kinder **(unsicher: aus Quelltext gefolgert)** | 409 | 412 | 507 vorab (QuotaPlugin.php:242-268, aus `X-Expected-Entity-Length`/`Content-Length`/`OC-Total-Length`), **413** beim Schreiben (`NotEnoughSpaceException` ⇒ `EntityTooLarge`, File.php:647-649) |
| Apache mod_dav | 405 (mod_dav.c:2836-2842) | 409 (`dav_fs_create_collection`, repos.c:1248-1250) | 412 („Destination is not empty and Overwrite is not "T"“, mod_dav.c:5045-5048) | 507 (`dav_fs_write_stream`, repos.c:1110) |
| nginx dav | 405 (EEXIST) | 409 (ENOENT) | 412 (:777-781); ungültig 400 | 507 (ENOSPC), 403 (EACCES) (`ngx_http_dav_error`, :1127-1152) |

- **Oberhalb der DAV-Wurzel** (z. B. `/remote.php`): Nextcloud `remote.php` wirft bei leerem PATH_INFO
  `RemoteException('Path not found', 404)` ⇒ **404** (bei `Content-Type: text/xml` als Sabre-`NotFound`, sonst als
  Fehlerseite); `/remote.php/dav/…` unterhalb der Wurzel siehe Tabelle. PROPFIND auf die Wurzel `/remote.php/dav/`
  ergibt 207 mit den Wurzelkindern (aus `RootCollection` gefolgert **(unsicher)**). `/remote.php/` mit Schrägstrich,
  unbekannte Dienste: nicht ermittelt **(unsicher)**.
- **Fallstrick Apache/nginx**: Liegt der Pfad **außerhalb** eines DAV-aktiven Bereichs, antworten der Apache-Standard-
  handler (`default_handler`, core.c: alles außer GET/POST/OPTIONS ⇒ `HTTP_METHOD_NOT_ALLOWED`, nur **unbekannte**
  Methodennamen ⇒ 501) und der nginx-Static-Handler (ngx_http_static_module.c:63-65) auf MKCOL/PUT/PROPFIND mit **405**
  – derselbe Code wie „Sammlung existiert schon“. Ein „405 = vorhanden“ in
  einer mkdir-p-Schleife führt dann in die Irre (rclone behandelt 405, 406 und 423 als „vorhanden“, prüft bei jedem
  anderen Fehler – im Kommentar: 409 bei 4shared – per PROPFIND nach und legt bei 409 zuerst die Eltern an,
  webdav.go:1061-1090). Gegenprobe: `PROPFIND` `Depth: 0` (207 = vorhanden, 404 = fehlt) oder `OPTIONS` mit `DAV:`-Header
  (RFC 4918 §10.1; hier nicht im Wortlaut gelesen).
- nginx im Detail: MKCOL ohne abschließenden `/` ⇒ 409, mit Body ⇒ 415 (:500-534); PUT auf `…/` ⇒ 409,
  `Content-Range` ⇒ 501, PUT schreibt in eine Temp-Datei und benennt um (ersetzend), 201 neu / 204 ersetzt;
  `create_full_put_path off` (Standard) legt keine Zwischenordner an; MOVE einer **Datei** liefert 204 (:868), auch bei
  neuem Ziel.
- RFC 4918 §9.9.3: `Overwrite: T` (Standard, §10.6) ⇒ der Server **löscht** ein vorhandenes Ziel (`Depth: infinity`)
  **vor** dem Verschieben – nicht als atomar annehmen. `Overwrite: F` + Ziel vorhanden ⇒ **412**. MOVE-Erfolg: 201 neu,
  204 überschrieben (§9.9.4).
- **507** (RFC 4918 §11.5): „unable to store the representation … considered to be temporary … MUST NOT be repeated
  until it is requested by a separate user action“; im RFC ausdrücklich bei PROPPATCH (§9.2.1), MKCOL (§9.3.1) und COPY (§9.8.5)
  aufgeführt, nicht in der MOVE-Liste (§9.9.4).
  Nextcloud kennt zusätzlich **413** für „Insufficient space“ **während** des Schreibens (s. Tabelle) – beides als
  „Platz/Quota“ behandeln, nicht wiederholen.

---

# ureq 2.12.1 (Antwortkörper) und XML-Parser: roxmltree 0.20.0 / quick-xml
Quelle: Crate `ureq-2.12.1` (`src/{response,request,unit,header}.rs`) · Crate `roxmltree-0.20.0` (`src/parse.rs`) ·
Crates `quick-xml-0.42.0` und `quick-xml-0.39.2` (`src/{lib,name,escape,errors}.rs`, `src/reader/*.rs`,
`src/events/mod.rs`) · crates.io-API (`crates.io/api/v1/crates/quick-xml`) · Abgerufen: 2026-10-02

## `into_string()` (10 MiB) und `into_reader()` mit Grenze
```rust
pub fn into_string(self) -> io::Result<String>                                   // response.rs:456
pub fn into_reader(self) -> Box<dyn Read + Send + Sync + 'static>               // :284
pub fn into_json<T: DeserializeOwned>(self) -> io::Result<T>                    // :532 (Feature "json"), serde_json::from_reader, KEINE Grenze
pub fn send(self, reader: impl Read) -> Result<Response>                         // request.rs:294
```
- `INTO_STRING_LIMIT = 10 * 1_024 * 1_024` (response.rs:33). `into_string` liest höchstens `LIMIT + 1` Bytes
  (`take`) und liefert bei Überschreitung `io::Error::new(ErrorKind::Other, "response too big for into_string")`
  (:464-470). Ohne Feature `charset` (Projekt: `["tls","json"]`) wird per `String::from_utf8_lossy` dekodiert, kein
  Fehler bei ungültigem UTF-8. Die Verbindung geht bei Überschreitung nicht zurück in den Pool (Rest ungelesen, siehe
  gdrive-ureq-throughput.md §9).
- `into_reader` begrenzt **nichts** außer durch `Content-Length`/Chunking; Doku: „If you use `read_to_end()` … a
  malicious server might return enough bytes to exhaust available memory … use `.take()`“ (:256-259).
```rust
use std::io::Read;
const MAX: u64 = 64 * 1024 * 1024;
let mut buf = Vec::new();
resp.into_reader().take(MAX + 1).read_to_end(&mut buf)?;
if buf.len() as u64 > MAX { /* zu groß: abbrechen, Verbindung wird verworfen */ }
```
- Eine PROPFIND-Antwort mit `Depth: 1` über einen Ordner mit vielen Einträgen kann 10 MiB überschreiten (grob
  ≥ 10 000 Einträge bei ~1 KiB je `response`-Element **(Schätzung)**) – `into_string()` scheitert dann mit dem Fehler oben.
- `send(reader)` ohne `Content-Length` und `Transfer-Encoding` nutzt **chunked** (request.rs:255-283; unit.rs:62-83), Puffer
  16 384 Byte.

## roxmltree 0.20.0
`Document::parse(text: &str) -> Result<Document>` und `Document::parse_with_options(text: &str, opt: ParsingOptions)`
(parse.rs:377/394). **Kein Streaming**: das ganze Dokument muss als `&str` im Speicher liegen und wird zu einem Baum.
`ParsingOptions { allow_dtd: bool, nodes_limit: u32 }` (:316-337; Standard: DTD aus, `nodes_limit = u32::MAX`). Die
README stellt es selbst gegen Streaming-Parser („slightly slower than quick-xml“).

## quick-xml (gepflegter Streaming-Parser)
- Aktuell **0.42.0** (2026-08-22; Edition 2024, `rust-version = 1.86`), **MIT**; 0.41.0 / 0.40.x: MSRV 1.79;
  **0.39.2 (MSRV 1.56, Edition 2021) steckt bereits im Lock** (über `plist` und `wayland-scanner`; `zbus_xml` nutzt
  zusätzlich 0.30.0). Reines Rust, `#![forbid(unsafe_code)]`, einzige Pflichtabhängigkeit `memchr`; Features (alle aus):
  `encoding`, `serialize`, `serde-types`, `async-tokio`, `escape-html`, `overlapped-lists`. Kein Plattformcode, daher für
  Android und Windows (auch windows-gnu) tauglich. Ob 1.86 mit der Projekt-Toolchain vereinbar ist, liegt außerhalb des
  Lesebereichs **(offen)**.
- **API 0.42.0** (Pfade `reader/…`): 
```rust
Reader::from_reader(reader: R) -> Reader<R>                                     // reader/mod.rs:809; R: BufRead nötig → BufReader::new(resp.into_reader().take(MAX))
Reader::<R: BufRead>::read_event_into<'b>(&mut self, buf: &'b mut Vec<u8>) -> Result<Event<'b>>   // buffered_reader.rs:411
NsReader::from_reader(reader: R) -> NsReader<R>                                 // ns_reader.rs:36
NsReader::<R: BufRead>::read_resolved_event_into<'b>(&mut self, buf: &'b mut Vec<u8>)
    -> Result<(ResolveResult<'_>, Event<'b>)>                                    // ns_reader.rs:251; ResolveResult = Unbound | Bound(Namespace<'_>) | Unknown(String) (name.rs:406)
reader.config_mut() -> &mut Config  // Felder: trim_text_start/_end (Standard false), expand_empty_elements (false), check_end_names (true), allow_unmatched_ends, check_comments, allow_dangling_amp, trim_markup_names_in_closing_tags
Event::{Start(BytesStart), End(BytesEnd), Empty(BytesStart), Text(BytesText), CData(BytesCData), Comment, Decl, PI, DocType, GeneralRef(BytesRef), Eof}   // events/mod.rs:1782
BytesStart::local_name() -> LocalName<'_>   // AsRef<str>;  BytesText::xml10_content(&self) -> Cow<'i, str>;  BytesRef::resolve_char_ref(&self) -> Result<Option<char>, Error>
quick_xml::escape::resolve_predefined_entity(entity: &str) -> Option<&'static str>   // escape.rs:851
reader.buffer_position() / error_position() -> u64
```
- Muster: `buf.clear()` am Schleifenende (der Event leiht `buf`); Elemente mit `<d:foo/>` kommen als `Event::Empty`
  (oder per `expand_empty_elements(true)` als Start+End); Namensräume über **`NsReader`** vergleichen (Präfixe `d:`, `D:`,
  Standard-Namensraum wechseln je Server), `ResolveResult::Bound(Namespace("DAV:"))`.
```rust
use quick_xml::{events::Event, name::{Namespace, ResolveResult::Bound}, reader::NsReader};
let mut rd = NsReader::from_reader(std::io::BufReader::new(resp.into_reader().take(MAX)));
let (mut buf, mut cur) = (Vec::new(), None::<String>);
loop {
    match rd.read_resolved_event_into(&mut buf)? {
        (Bound(Namespace("DAV:")), Event::Start(e)) if e.local_name().as_ref() == "href" => cur = Some(String::new()),
        (_, Event::Text(t)) => if let Some(s) = cur.as_mut() { s.push_str(&t.xml10_content()) },
        (_, Event::CData(c)) => if let Some(s) = cur.as_mut() { s.push_str(&c.xml10_content()) },
        (_, Event::GeneralRef(r)) => if let Some(s) = cur.as_mut() {
            if let Some(c) = r.resolve_char_ref()? { s.push(c) }
            else if let Some(x) = quick_xml::escape::resolve_predefined_entity(&r) { s.push_str(x) } },
        (_, Event::End(_)) => if let Some(h) = cur.take() { /* h = href, selbst trimmen */ },
        (_, Event::Eof) => break,
        _ => {}
    }
    buf.clear();
}
```
- **Stolperfallen**: (1) Text mit Entitäten kommt (in 0.39.2 und 0.42.0 gelesen) als **mehrere** Events (`Text`,
  `GeneralRef`, `CData`) – zusammensetzen; `Event::Text` allein enthält kein `&amp;` mehr (events/mod.rs:1782-1806).
  (2) `trim_text(true)` ist laut Doku fehlerhaft („Trimming applies to every Text event regardless of context“,
  reader/mod.rs:166-247) und verschluckt daher vermutlich auch Leerzeichen an Entitätsgrenzen **(abgeleitet)** – auf
  `false` lassen, fertigen String selbst trimmen. (3) Der Puffer wächst auf den größten
  **einzelnen** Event; es gibt keine Größenoption – Eingabe mit `Read::take` begrenzen. (4) Kein Entitäts-Expansion-
  Angriff: unbekannte/benutzerdefinierte Entitäten bleiben `GeneralRef`-Events, `<!DOCTYPE>` wird roh als
  `Event::DocType` geliefert (für WebDAV ablehnen). (5) Zwischen 0.39 und 0.42 ändern sich Namenstypen (`QName`/
  `LocalName`/`Namespace` sind in 0.39.2 `&[u8]`-Wrapper mit `AsRef<[u8]>`, in 0.42.0 `&str`) und die Textfunktionen
  (`BytesText::xml10_content` liefert in 0.39.2 `Result<Cow<str>, EncodingError>`, in 0.42.0 `Cow<str>`;
  `xml_content(version)` nimmt in 0.42.0 ein `XmlVersion`). Lesefunktionen `read_event_into`/`read_resolved_event_into`
  und `Config::trim_text` sind gleich.

---

# Google Drive API v3
Quelle: `developers.google.com/workspace/drive/api/reference/rest/v3/{files,files/create,files/update,files/export,
changes,changes/list,changes/getStartPageToken}` (Stand der Seiten 2025-04 bis 2026-07) und Guides
`…/guides/{manage-changes,manage-downloads,manage-uploads,create-file,shortcuts,ref-export-formats,ref-search-terms,
handle-errors}` · rclone `rclone.org/drive/` und `backend/drive/drive.go` (master) · MS Learn „Naming Files, Paths, and
Namespaces“ · Abgerufen: 2026-10-02

## `modifiedTime` beim Anlegen und Aktualisieren
- Files-Ressource: `createdTime`, `modifiedTime` sind Strings „RFC 3339 date-time“, schreibbar; „Note that setting
  `modifiedTime` also updates `modifiedByMeTime` for the user.“ Die Genauigkeit ist **offiziell nicht dokumentiert**;
  rclone: „Google drive stores modification times accurate to 1 ms“ (Precision 1 ms, drive.go:2816) und sendet das Format
  `2006-01-02T15:04:05.000000000Z07:00` (drive.go:65) – Nanosekunden-Ziffern werden akzeptiert **(Drittquelle)**.
- Anlegen (multipart/resumable-Metadaten) und Inhalt ersetzen (`PATCH /upload/drive/v3/files/{id}?uploadType=…`): die
  Zeit gehört in das **Metadaten-JSON** – sonst ändert Drive sie selbst („some fields might be changed automatically“,
  files.update; Simple Upload: „attributes are inferred … such as the MIME type or modifiedTime“, manage-uploads). rclone
  übergibt `ModifiedTime` bei Create und bei Content-Update (drive.go:2512-2516, 4547). Nur die Zeit ändern: Metadaten-
  `PATCH` (rclone `SetModTime`, drive.go:4272-4293).
```
POST  https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable     Body: {"name":"a.txt","parents":["<ordnerId>"],"id":"<vorab erzeugt>","modifiedTime":"2026-10-02T12:34:56.789Z"}
PATCH https://www.googleapis.com/upload/drive/v3/files/{id}?uploadType=resumable   Body: {"modifiedTime":"2026-10-02T12:34:56.789Z"}
PATCH https://www.googleapis.com/drive/v3/files/{id}?fields=id,modifiedTime&supportsAllDrives=true   Body: {"modifiedTime":"…"}   (nur Metadaten)
```
- Kosten Metadaten-PATCH: „Edit“ 50 Units im neuen Quota-Modell (gdrive-ureq-throughput.md §2).

## Prüfsummen, Größe, Version
`md5Checksum` (output only; „only applicable to files with binary content“), `sha1Checksum`, `sha256Checksum` („not
populated for Docs Editors or shortcut files“; rclone: ein Teil der vor 2018 hochgeladenen Dateien hat sie nicht), `size`
(int64-String; „Won't be populated for files that have no size, like shortcuts and folders“), `version` (monoton je
Datei, „reflects every change made … even those not visible to the user“), `originalFilename`. Per `fields=` anfordern
(Standard von `files.list`: nur `kind,id,name,mimeType`, siehe gdrive-ureq-throughput.md §4).

## Dateinamen und exakte Abfrage
- `name`: „isn't necessarily unique within a folder“ ⇒ **mehrere Dateien gleichen Namens im selben Ordner möglich**,
  `files.create` meldet keinen Namenskonflikt (rclone: „Drive unlike all the other remotes can have duplicated files“).
  Ein Ziel-Lookup per Name kann 0, 1 oder n Treffer liefern; ein Ordnerelternteil ist eindeutig („A file can only have one
  parent folder“).
- Zeichen: Die API-Referenz nennt **keine** Zeichenbeschränkung. rclone: „`/` can also be used in names and `.` or `..`
  are valid names“, nur ungültiges UTF-8 geht nicht (JSON). Eine offizielle Liste unzulässiger Zeichen oder eine
  Längengrenze für Namen fand sich **nicht**. Windows als Gegenseite verbietet `< > : " / \ | ? *`, NUL, 1–31, die Namen
  `CON PRN AUX NUL COM1-9 LPT1-9 COM¹²³ LPT¹²³` (auch mit Endung) sowie Namen mit Leerzeichen/Punkt am Ende (MS Learn):
  beim Schreiben auf die lokale Seite abbilden; `%` hat in der Drive-API keine Sonderrolle (nirgends erwähnt).
- Exakte Suche: `q = name = '<n>' and '<parentId>' in parents and trashed = false`; `name` unterstützt `contains`, `=`,
  `!=`; „Escape single quotes in queries with `\'`“. Das Maskieren von `\` als `\\` ist **undokumentiert, funktioniert
  aber** (rclone drive.go:1068-1071: erst `\` → `\\`, dann `'` → `\'`). Ob `name =` Groß/Klein unterscheidet, ist nicht
  dokumentiert (Drittangaben: unempfindlich) ⇒ Treffer clientseitig exakt vergleichen. Mehrere Ordner in einer Abfrage:
  `trashed=false and ('a' in parents or 'b' in parents …)`, rclone bündelt bis zu 50 (rclone.org/drive, ListR).
- Grenzen (handle-errors): My Drive höchstens 100 Ordnerebenen (`myDriveHierarchyDepthLimitExceeded`), höchstens
  500 000 Einträge direkt in einem Ordner (`numChildrenInNonRootLimitExceeded`).

## `changes.list` und Eingrenzung auf einen Teilbaum
```
GET https://www.googleapis.com/drive/v3/changes/startPageToken      → {"startPageToken": "…"}   (läuft nicht ab)
GET https://www.googleapis.com/drive/v3/changes?pageToken=…&pageSize=1000&includeRemoved=true&restrictToMyDrive=true
        &fields=nextPageToken,newStartPageToken,changes(fileId,removed,time,file(id,name,mimeType,parents,trashed,modifiedTime,md5Checksum,size,shortcutDetails))
```
- Parameter: `pageToken` (aus `nextPageToken` der letzten Antwort oder `getStartPageToken`), `pageSize` (Standard 100,
  höchstens 1000), `includeRemoved` („changes indicating that items have been removed … for example by deletion or loss
  of access“; ein Standardwert steht in der aktuellen Referenz **nicht** ⇒ explizit setzen),
  `restrictToMyDrive` („changes inside the My Drive hierarchy. This omits changes to files such as those in the
  Application Data folder or shared files which have not been added to My Drive“), `spaces` (`drive`, `appDataFolder`),
  `includeItemsFromAllDrives`, `supportsAllDrives`, `driveId`, `includeCorpusRemovals`, `fields`.
- Antwort: `changes[]` (chronologisch, älteste zuerst), `nextPageToken` (fehlt auf der letzten Seite), `newStartPageToken`
  (nur auf der letzten Seite; für den nächsten Lauf speichern; „doesn't expire“). `Change`: `removed`, `fileId`, `time`,
  `changeType` (`file`|`drive`), `file` (aktueller Zustand; **fehlt bei `removed`**: „Present if … the file has not been
  removed from this list of changes“). Ein Papierkorb-Vorgang erscheint vermutlich als Änderung mit `file.trashed = true`,
  endgültiges Löschen/Zugriffsverlust als `removed` **(abgeleitet aus der `removed`-Beschreibung und dem `trashed`-Feld)**.
- **Teilbaum-Filter gibt es nicht**: die Parameterliste enthält weder `q` noch einen Ordner. Der Feed liefert alle
  Änderungen des Kontos; eingrenzen nur clientseitig: lokales Verzeichnis `id → (parents, name, trashed, mimeType)` aus
  einem vollständigen Lauf (`files.list`, Ordner für Ordner), Änderung gehört zum Teilbaum, wenn `file.parents[0]` im
  Ordnerset liegt (Elternkette per `files.get?fields=id,parents` hochlaufen, Zwischenstände merken); `removed` hat nur
  die `fileId` ⇒ nur gegen den eigenen Index auflösen. Das Vorgehen ist abgeleitet, nicht von Google empfohlen
  **(unsicher)**.

## Verknüpfungen (Shortcuts) und Export
- Shortcut: `mimeType = application/vnd.google-apps.shortcut`, `shortcutDetails.{targetId, targetMimeType (Momentaufnahme),
  targetResourceKey}`; `shortcutDetails` ist nur bei `files.create` setzbar; ein Shortcut hat genau einen Elternordner,
  bricht beim Löschen des Ziels, **Name und MIME-Typ können vom Ziel abweichen** (shortcuts-Guide). `files.get` liefert
  die Shortcut-Ressource selbst, nicht das Ziel (abgeleitet aus „Only populated for shortcut files“). Suche:
  `mimeType='application/vnd.google-apps.shortcut'`. Für den Sync als **Link
  behandeln** (melden/auslassen, nicht folgen – AGENTS.md „Child links“); rclone folgt ihnen und hat dafür
  `--drive-skip-shortcuts` (Zyklen auf eigene Eltern möglich).
- Export nur für Docs-Editoren-Typen (`document`, `spreadsheet`, `presentation`, `drawing`, Apps-Script-JSON): `GET
  /drive/v3/files/{id}/export?mimeType=…`; „Exported content is limited to 10 MB“ (manage-downloads); Vids ⇒ 403
  `fileNotExportable`, Download nur über `files.download` (LRO). Binärdateien: `files.get?alt=media`. Docs-Editoren-Dateien
  haben **kein** `md5Checksum` und per `alt=media` nicht ladbar: Fehler 403 `fileNotDownloadable` („Only files with binary
  content can be downloaded. Use Export with Docs Editors files.“; die Seite beschreibt es für `revisions.get`).
  `capabilities.canDownload` vor dem Laden prüfen. Standardverhalten für den Sync: Docs-Editoren-Typen auslassen und
  melden (Größe/Hash unbestimmbar, rclone zeigt Größe −1).
- Quota-Fehler (handle-errors): `403 storageQuotaExceeded` („The user's Drive storage quota has been exceeded.“),
  `teamDriveFileLimitExceeded`, `myDriveHierarchyDepthLimitExceeded`; Rate-Fehler siehe gdrive-ureq-throughput.md §2.
  Revisionen: jeder Inhalts-Update legt eine Revision an (rclone: 30 Tage/100 Revisionen, zählen nicht zur Quota).

---

# smb2 0.26.0
Quelle: Crate `smb2-0.26.0` (`src/client/{tree,mod,connection,stream}.rs`, `src/msg/set_info.rs`,
`src/pack/filetime.rs`, `src/types/{flags,status}.rs`, `src/error.rs`) · MS-FSCC 2.4.7 (`learn.microsoft.com/…/ms-fscc/16023025-8a78-492f-8b96-c873b042ac50`,
Stand 2025-02-10) · MS-FSA 2.1.5.15.2 (`…/ms-fsa/a36513b4-73c8-4888-ad29-8f3a196567e8`) · MS-ERREF NTSTATUS
(`…/ms-erref/596a1078-e883-4972-9bbc-49e60bebca55`) · MS Learn `SetFileTime` (`learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfiletime`) ·
Abgerufen: 2026-10-02

## Änderungszeit setzen: keine API – Nachweis und Bauplan
- **Nachweis**: `Tree` und `SmbClient` haben keine `set_*`/`touch`-Funktion (Suche in `src/client/`); SET_INFO wird nur
  für `FileRenameInformation` (Klasse 10, tree.rs:129, :1195-1257) und `FileDispositionInformation` (Klasse 13, :2209-2230)
  gesendet. `FileBasicInformation` (Klasse 4, tree.rs:123) kommt nur als **QUERY_INFO** vor (`Tree::stat`, :837-900).
- **Bausteine sind öffentlich**: `SetInfoRequest { info_type: InfoType, file_info_class: u8, additional_information: u32,
  file_id: FileId, buffer: Vec<u8> }` (msg/set_info.rs:23-34), `CompoundOp`/`CreateRequest`/`CloseRequest`/`FileId::SENTINEL`
  und `SmbClient::connection(&self) -> &Connection` (mod.rs:887) + `async fn Connection::execute_compound(&self, ops:
  &[CompoundOp<'_>]) -> Result<Vec<Result<Frame>>>` (connection.rs:3880) – Gerüst wie im Rename-Bauplan in smb2.md. `smb2::pack::FileTime(pub u64)`
  mit `from_system_time(SystemTime) -> FileTime` und `to_system_time(self) -> Option<SystemTime>` (pack/filetime.rs:17-61);
  `FileAccessMask::FILE_WRITE_ATTRIBUTES = 0x0100` (types/flags.rs:300).
- **`FileWriter` gibt sein Handle nicht heraus** (`file_id` privat, stream.rs:1006-1021): die Zeit wird nach
  `finish()`/`abort()` über ein **eigenes** Compound CREATE + SET_INFO + CLOSE gesetzt (ein Roundtrip).
```rust
// CREATE: desired_access = FileAccessMask::new(FileAccessMask::FILE_WRITE_ATTRIBUTES | FileAccessMask::SYNCHRONIZE),
//         create_disposition = CreateDisposition::FileOpen, share_access = READ|WRITE|DELETE, create_options = 0 (wie Tree::stat, tree.rs:841-856)
let mut b = Vec::with_capacity(40);                    // FileBasicInformation, MS-FSCC 2.4.7, 40 Byte
b.extend_from_slice(&0u64.to_le_bytes());              // CreationTime    0 = nicht ändern
b.extend_from_slice(&0u64.to_le_bytes());              // LastAccessTime  0 = nicht ändern
b.extend_from_slice(&smb2::pack::FileTime::from_system_time(t).0.to_le_bytes());   // LastWriteTime
b.extend_from_slice(&0u64.to_le_bytes());              // ChangeTime      0 = nicht ändern
b.extend_from_slice(&0u32.to_le_bytes());              // FileAttributes  0 = nicht ändern
b.extend_from_slice(&0u32.to_le_bytes());              // Reserved
// SET_INFO: SetInfoRequest { info_type: InfoType::File, file_info_class: 4, additional_information: 0, file_id: FileId::SENTINEL, buffer: b }
```
- Regeln (MS-FSCC 2.4.7, MS-FSA 2.1.5.15.2): Zeitfeld `0` = unverändert; `-1` (`0xFFFF_FFFF_FFFF_FFFF`) = für alle späteren
  Operationen **auf demselben Handle** nicht ändern; `-2` = ändern; kleiner als −2 ⇒ `STATUS_INVALID_PARAMETER`; Puffer < 40
  Byte ⇒ `STATUS_INFO_LENGTH_MISMATCH`; `FileAttributes` wird nur bei `!= 0` verarbeitet. Ein echter Wert setzt
  `Open.UserSetModificationTime = TRUE` des **geöffneten Handles** (MS-FSA); wie sich das auf spätere WRITEs über dieses
  oder andere Handles auswirkt, regelt MS-FSA an anderer Stelle (nicht gelesen) – deshalb die Zeit **nach** dem letzten
  WRITE und nach dem Schließen des Schreib-Handles setzen. Das Handle braucht `FILE_WRITE_ATTRIBUTES` (Win32
  `SetFileTime`: „The handle must have been created using the CreateFile function with the FILE_WRITE_ATTRIBUTES access
  right“, learn.microsoft.com/…/fileapi/nf-fileapi-setfiletime); den Status bei fehlendem Recht nennt die gelesene Seite
  nicht (die FSCC-Tabelle führt `STATUS_ACCESS_DENIED` nur allgemein für die Klasse).
- Samba/Windows-Verhalten dieses Compounds nicht getestet **(unsicher)**.

## „Platte voll“ als Fehler
`Error::Protocol { status: NtStatus, command: Command }` (error.rs:61-68); `NtStatus(pub u32)` (types/status.rs). `STATUS_DISK_FULL`
`0xC000007F` („An operation failed because the disk was full.“, MS-ERREF) ⇒ `Error::kind() == ErrorKind::DiskFull` (error.rs:465/621,
`kind()` :534; `Error::status() -> Option<NtStatus>` :394). Der Status steht im Antwort-Header des fehlschlagenden Befehls
(typisch WRITE; bei Pipelining meldet ihn erst ein späterer `write_chunk` oder `finish`; FLUSH/CLOSE/CREATE ebenfalls
möglich) **(abgeleitet, nicht gegen einen Server geprüft)** – nach einem Fehler `abort()` und die Teildatei löschen (smb2.md).
**Quota-Codes werden nicht auf `DiskFull` abgebildet**: `STATUS_QUOTA_EXCEEDED 0xC0000044` („Insufficient quota exists …“) und
`STATUS_DISK_QUOTA_EXCEEDED 0xC0000802` („… the storage quota was exceeded“) sind in `NtStatus` nicht definiert und ergeben
`ErrorKind::Other`; per `err.status() == Some(NtStatus(0xC000_0044))` bzw. `0xC000_0802` erkennen. Welche Codes echte
Windows-/Samba-Server bei Quota-Überschreitung senden, ist nicht belegt **(unsicher)**. Freier Platz vorab: `Tree::fs_info`
(tree.rs:1020) → `FsInfo { total_bytes, free_bytes (für den Aufrufer verfügbar), total_free_bytes (gesamt frei, „may differ … if quotas are
in effect“), bytes_per_sector, sectors_per_unit }` (:234-246).

---

# flate2 1.1.9 (Streaming Deflate/Zlib)
Quelle: Crate `flate2-1.1.9` (`src/{lib,mem,zio}.rs`, `src/deflate/{write,read}.rs`, `src/zlib/write.rs`, `Cargo.toml`, `README.md`) ·
Crate `miniz_oxide-0.8.9` (`src/deflate/{buffer,core}.rs`, `src/inflate/{core,stream}.rs`) · zlib `zconf.h` (github.com/madler/zlib) ·
Abgerufen: 2026-10-02

Im Lock: flate2 1.1.9 mit Backend **miniz_oxide 0.8.9** (Standard `rust_backend`; Abhängigkeiten `crc32fast`, `miniz_oxide`), kommt über
`russh` (Feature `flate2`) und `zip`. Alternativen per Feature: `zlib-rs` (reines Rust, laut README am schnellsten),
`zlib`, `zlib-ng`, `cloudflare_zlib` (C-Abhängigkeiten).

## Signaturen
```rust
// flate2::write — unkomprimierte Bytes hinein, komprimierte nach W (deflate/write.rs)
DeflateEncoder::<W: Write>::new(w: W, level: Compression) -> DeflateEncoder<W>      // :40
  .get_ref(&self) -> &W   .get_mut(&mut self) -> &mut W
  .reset(&mut self, w: W) -> io::Result<W>            // beendet den Strom, neuer Ausgang
  .try_finish(&mut self) -> io::Result<()>            // danach kein write mehr (Panik möglich)
  .finish(self) -> io::Result<W>                      // Schlussblock schreiben, W zurück
  .flush_finish(self) -> io::Result<W>                // nur Sync-Flush, Strom bleibt erweiterbar
  .total_in(&self) -> u64   .total_out(&self) -> u64
impl Write for DeflateEncoder<W>                      // flush() = Z_SYNC_FLUSH (zio.rs:262-284)
DeflateDecoder::<W: Write>::new(w: W)                 // :217 – dekomprimiert beim Schreiben; finish()/try_finish()/reset()/total_*
// flate2::read — Quelle R, Ergebnis per Read
read::DeflateEncoder::<R: Read>::new(r: R, level: Compression)      // komprimiert beim Lesen
read::DeflateDecoder::<R: Read>::new(r: R)  /  ::new_with_buf(r: R, buf: Vec<u8>)   // Standardpuffer vec![0; 32*1024]
// ZlibEncoder/ZlibDecoder (2-Byte-Kopf + Adler-32, RFC 1950) und GzEncoder/GzDecoder (10-Byte-Kopf, CRC-32 + Länge, RFC 1952) mit gleicher Form,
// bufread::* für BufRead-Quellen. Einzelblöcke ohne Adapter:
Compress::new(level: Compression, zlib_header: bool) -> Compress                                   // mem.rs:198
Compress::compress_vec(&mut self, input: &[u8], output: &mut Vec<u8>, flush: FlushCompress) -> Result<Status, CompressError>   // :380 (Vec nicht vergrößern: vorher reservieren)
Compress::reset(&mut self)                                                                          // :300, ohne Neuanlage
Decompress::new(zlib_header: bool) -> Decompress  /  decompress_vec(&mut self, input, output: &mut Vec<u8>, flush: FlushDecompress) -> Result<Status, DecompressError>   // :403 / :521
enum FlushCompress { None, Partial, Sync, Full, Finish }   enum Status { Ok, BufError, StreamEnd }   // mem.rs:48 / :166
```
## Level
`Compression::new(level: u32)` (lib.rs:216-221; „0-9“, `const fn`), `none()` = 0, `fast()` = 1, `best()` = 9, `default()` = 6, `level()`.
Im miniz_oxide-Backend wird `min(10, level)` benutzt (core.rs:2338-2343; deflate/mod.rs:26 `UberCompression = 10`), Level ≤ 3 nutzt gierige Analyse,
0 = nur gespeicherte Blöcke („may actually inflate data slightly“).

## Speicherbedarf
- zlib-Referenz (zconf.h): Deflate `(1 << (windowBits+2)) + (1 << (memLevel+9))` = 128 K + 128 K = **256 K** plus wenige KB,
  Inflate `1 << windowBits` = 32 K plus ~7 KB.
- miniz_oxide 0.8.9 (aus den Konstanten **berechnet, nicht gemessen**; deflate/buffer.rs:10-21, core.rs:696-703,
  inflate/core.rs:17,109-116): Encoder = LZ-Codepuffer 65 536 B + Ausgabepuffer `OUT_BUF_SIZE` 85 196 B + Wörterbuch
  33 026 B + `next` 65 536 B + `hash` 65 536 B + Huffman-Tabellen 4 320 B ≈ 319 000 B (≈ 0,30 MiB), dazu der 32-KiB-Puffer
  des `write`-Adapters (zio.rs:169) ⇒ **≈ 0,34 MiB je `DeflateEncoder`**; Decoder = 32-KiB-Wörterbuch + 3 Huffman-Tabellen
  (je 2 048 + 1 152 B) + kleine Felder ≈ 43 KiB, mit 32-KiB-Adapterpuffer **≈ 75 KiB**. Das Fenster ist im Rust-Backend
  fest 32 KiB; `new_with_window_bits` (9–15) gibt es **nur** mit `zlib`/`zlib-ng`/`zlib-rs` (`any_zlib`, mem.rs:216/419).

## Stolperfallen
- `Drop` ruft `finish()` und **verschluckt Fehler** (zio.rs:287-293): immer `finish()` selbst aufrufen und das `io::Result` prüfen.
- `flush()` ist ein **Sync-Flush**: alles bisher Geschriebene wird beim Empfänger lesbar, kostet aber Kompressionsrate (mem.rs:
  „Flushing may degrade compression“). An Nachrichtengrenzen nötig, nicht je `write`.
- **Abgeschnittene Ströme werden nicht gemeldet**: `read::*Decoder` ruft am Eingabeende `Flush::finish` auf und gibt bei Status
  `Ok`/`BufError` einfach `Ok(0)` zurück (zio.rs:123-158), `write::*Decoder::finish` ebenso (zio.rs:173-185); nur `Err` gibt
  „corrupt deflate stream“ (`InvalidInput`). Raw Deflate hat keine Prüfsumme/Länge: Länge und Hash im eigenen Rahmen mitführen
  (`total_out()` vergleichen). `GzDecoder` meldet ein fehlendes Trailer als `UnexpectedEof` (gz/mod.rs:243, 266).
- Dekompressionsbombe: Ausgabe mit `Read::take(limit)` begrenzen bzw. `total_out()` prüfen.
- `Compress::compress_vec` vergrößert den `Vec` nicht (vorher `reserve`); für unabhängige Blöcke `reset()` statt Neuanlage.

---

# Offene Punkte
- **Nextcloud/ownCloud**: Statuscode bei ungültigem `X-OC-Mtime` (Quelltext: unabgefangene `InvalidArgumentException`, vermutlich 500,
  Daten schon gespeichert) und PROPPATCH-Antwortform für `{DAV:}lastmodified` nur aus Nextcloud-Quelltext gelesen, ownCloud-Quelltext und
  Live-Server nicht; `/remote.php/` mit Schrägstrich und unbekannte Dienste nicht ermittelt.
- **Synology WebDAV Server**: widersprüchliche Drittangaben, keine offizielle Quelle. Apache-2.4-Verhalten von `{DAV:}lastmodified` als
  Dead Property nur aus Quelltext gefolgert. Apache-MKCOL oberhalb der DAV-`<Location>`: 405 aus dem Core-Handler gefolgert, nicht getestet.
- **FTP**: IIS FTP (7.5+) – RNTO, MFMT, MLSx und Platzcodes nicht belegt; FileZilla Server **1.x** nicht belegt (nur Tickets zu 0.9.x,
  zweite Hand); `MFMT`/`MLST` bei FileZilla/IIS offen; keine Rest-Platz-Abfrage per FTP untersucht. Abbruchverhalten von `put_file`
  (liegengebliebene Abschlussantwort) nur aus Quelltext gefolgert.
- **Google Drive**: Genauigkeit von `modifiedTime` offiziell nicht dokumentiert (rclone: 1 ms); Groß/Klein bei `name =`, Namenslänge, offizielle
  Liste unzulässiger Zeichen und der Standard von `includeRemoved` nicht gefunden; Teilbaum-Filter für `changes.list` gibt es nicht (Beleg =
  Parameterliste), das clientseitige Vorgehen ist abgeleitet. `fileNotDownloadable` für `files.get?alt=media` auf Docs-Dateien: Seite nennt `revisions.get`.
- **SMB**: Verhalten des SET_INFO-Compounds auf Samba und Windows nicht getestet; welche Codes echte Server bei Quota-Überschreitung
  senden (`DISK_FULL` vs. `QUOTA_EXCEEDED`/`DISK_QUOTA_EXCEEDED`) nicht belegt.
- **russh-sftp**: Verhalten bei EXTENDED-Attributen und bei unbekannten Statuscodes (>8) nur aus Quelltext gefolgert; ob Server die per
  FSETSTAT gesetzte Zeit bis CLOSE halten, nicht belegt; Pipelining mehrerer READDIR auf einem Handle nicht untersucht.
- **flate2**: Speicherzahlen aus Konstanten berechnet, Durchsatz je Level nicht gemessen. **quick-xml**: MSRV 1.86 (0.42.0) gegen die
  Projekt-Toolchain nicht geprüft (außerhalb des Lesebereichs); 0.39.x (MSRV 1.56) ist bereits im Lock.
