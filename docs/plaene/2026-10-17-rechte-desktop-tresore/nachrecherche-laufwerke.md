# S2 — Nachrecherche N1–N27 und Revalidierung Abschnitt C (Laufwerke)

Stand und Abrufdatum 2026-10-17. Alle Web-Quellen am 2026-10-17 abgerufen. Lesestand des Repositorys: `da2646d2`.

Bezug: `strang-laufwerke.md` (Abschnitt C „Entscheidungen“, Abschnitt F „Risiken“, Abschnitt G „Nachrecherche“). Diese Datei bewertet nicht neu und entscheidet nicht. Kennzeichnung: **Folgerung:** = eigener Schluss für den Plan; **Annahme:** = ungeprüft; **Lücke:** = genau benannte fehlende Angabe.

Quellenarten und Schreibweise:
- `N/…` = `native/src/…` im Repository.
- Kernel: `git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/<Pfad>` auf `master` (Makefile-Stand **7.3-rc7**) und mit `?h=v6.8` (Kernel von Ubuntu 24.04, lokal `6.8.0-134-generic`). Zeilenangaben „Kernel master“ bzw. „Kernel v6.8“ beziehen sich auf diese Abrufe.
- `fuser 0.18.0`: Archiv `https://static.crates.io/crates/fuser/fuser-0.18.0.crate` (nicht im lokalen Cargo-Register), nur gelesen; Zeilenangaben `fuser-0.18.0/<Pfad>:<Zeile>`.
- Lokales Cargo-Register (nur gelesen): `~/.cargo/registry/src/index.crates.io-*/` mit `chacha20poly1305-0.10.1`, `hkdf-0.12.4`.
- libfuse: `raw.githubusercontent.com/libfuse/libfuse/fuse-3.18.3/util/fusermount.c` (Tag `fuse-3.18.3`, veröffentlicht 2026-09-08, neuestes Release).
- GLib/GTK/Nautilus/GVfs: `gitlab.gnome.org/GNOME/<Projekt>/-/raw/<Zweig>/<Pfad>` (GLib `main`, GTK `gtk-3-24` und `main`, Nautilus `main` = nach Tag 51.1 vom 2026-10-11, GVfs `master`); Nemo `github.com/linuxmint/nemo` `master`; Thunar `gitlab.xfce.org/xfce/thunar` `master`; KDE `invent.kde.org/frameworks/{kio,ktexteditor,karchive,kbookmarks,kcoreaddons}` `master`.
- vim `master` `src/bufwrite.c`; coreutils `master` `src/cp.c`, `src/copy.c` (GitHub-Spiegel); rsync `master` `rsync.c`; libuv `v1.x` `src/win/fs.c` (neuestes Release v1.53.0 vom 2026-09-24); Node.js `main` `lib/fs.js`, `lib/internal/fs/promises.js`, `src/node_file.cc`.
- Paketmetadaten: lokales `apt-cache show`/`apt-cache rdepends` (Ubuntu 24.04.4, nur Abfrage, keine Installation), `packages.debian.org/trixie/…`, `archlinux.org/packages/…`, `src.fedoraproject.org/rpms/<Paket>/raw/f42/f/<Paket>.spec`.
- GitHub-Issues über die GitHub-REST-API (lesend).
- Keine Builds, keine Tests, keine Ausführung von Programmen aus Archiven.

---

## 1. Antworten N1–N27

### N1 — `ENOSYS` auf `FUSE_INTERRUPT` und `request_wait_answer`

**Antwort.**
1. Ja. Antwortet der Server auf eine INTERRUPT-Anfrage mit `-ENOSYS`, setzt der Kernel `no_interrupt = 1` für die gesamte Verbindung (Kernel master: `fch->no_interrupt`; Kernel v6.8: `fc->no_interrupt`). Danach werden keine INTERRUPT-Anfragen mehr erzeugt.
2. Ablauf `request_wait_answer` (in v6.8 und master gleich aufgebaut):
   - Solange `no_interrupt == 0`: Wartet mit `wait_event_interruptible`. Ein beliebiges Signal setzt `FR_INTERRUPTED`; ist die Anfrage schon an den Server übergeben (`FR_SENT`), wird eine INTERRUPT-Anfrage eingereiht.
   - Danach (und von Beginn an, wenn `no_interrupt == 1`): Wartet mit `wait_event_killable` — nur tödliche Signale wecken.
   - Tödliches Signal, Anfrage **noch nicht** beim Server: Anfrage wird aus der Warteschlange entfernt, Aufrufer erhält `-EINTR` (v6.8) bzw. kehrt zurück (master, `fuse_remove_pending_req`).
   - Tödliches Signal, Anfrage **schon beim Server**: unbedingtes `wait_event` (nicht unterbrechbar) bis zur Antwort. Ausnahme in master: `args->abort_on_kill` bricht die Verbindung ab; dieses Feld setzt nur die INIT-Anfrage (`fs/fuse/inode.c:1512`).
3. Folge für normale Signale nach `no_interrupt`: Ein normales Signal (zum Beispiel `SIGINT` mit Handler) unterbricht das Warten nicht mehr; der Aufrufer wartet bis zur Antwort; der Signalhandler läuft erst danach.
4. Erste Unterbrechung vor dem Umschalten: Die **erste** INTERRUPT-Anfrage erreicht `fuser`; `fuser 0.18.0` antwortet darauf mit `ENOSYS` (`fuser-0.18.0/src/request.rs:117-119`, Kommentar `// TODO: handle FUSE_INTERRUPT`). Die ursprüngliche Anfrage bleibt davon unberührt und wird normal beantwortet.

**Belege.**
- Kernel master `fs/fuse/dev.c:702-750` (`request_wait_answer`), `:1938-1957` (Antwort mit `FUSE_INT_REQ_BIT`: `else if (oh.error == -ENOSYS) fch->no_interrupt = 1;`).
- Kernel v6.8 `fs/fuse/dev.c:364-408` (`request_wait_answer`), `:1904` (`fc->no_interrupt = 1`).
- Kernel master `Documentation/filesystems/fuse/fuse.rst:148-186` („Interrupting filesystem operations“: „If the request is already sent to userspace, then an INTERRUPT request is queued.“; „The userspace filesystem may ignore the INTERRUPT requests entirely“).
- Kernel master `fs/fuse/inode.c:1512` (`ia->args.abort_on_kill = true;` nur für INIT).
- `fuser-0.18.0/src/request.rs:117-119`.

**Folgerung für den Plan:** Die Annahme in C2 „der Kernel stellt danach keine Interrupts mehr zu“ ist belegt. R3 („Prozesse in zugestellten FUSE-Anfragen sind bei hängendem Backend nicht beendbar“) ist belegt — auch `SIGKILL` beendet einen Prozess in einer schon zugestellten Anfrage nicht, unabhängig von `no_interrupt`. Ausweg bleibt der Verbindungsabbruch (N2).

**Verbleibende Lücke:** keine für die Kernel-Seite. Keine Laufzeitbeobachtung (nicht ausgeführt).

### N2 — `fusectl`-`abort` für den einhängenden Nutzer; Einhängen von `fusectl`

**Antwort.**
1. Schreibbar für den einhängenden Nutzer: **ja**. `fusectl` legt je Verbindung ein Verzeichnis (Modus `0500`) und darin `abort` mit Modus `0200` an; Besitzer jeder Datei ist `fc->user_id`, also der Nutzer, der eingehängt hat (bei `fusermount3` der aufrufende Nutzer, nicht root). Kernel-Doku: „Only the owner of the mount may read or write these files.“ Schreiben beliebiger Bytes ruft `fuse_chan_abort(fc->chan, fc->abort_err)` (master) auf: alle wartenden Anfragen enden mit Fehler, neue ebenfalls.
2. Standardmäßig eingehängt: Die systemd-Unit `sys-fs-fuse-connections.mount` (`What=fusectl`, `Where=/sys/fs/fuse/connections`, `ConditionPathExists=/sys/fs/fuse/connections`, `ConditionCapability=CAP_SYS_ADMIN`, `ConditionVirtualization=!private-users`, `Requires=modprobe@fuse.service`) wird im systemd-Upstream mit dem Symlink `sysinit.target.wants/` installiert; zusätzlich zieht die udev-Regel `99-systemd.rules` sie beim Laden des Moduls `fuse` nach.
   - Ubuntu 24.04: **belegt** (lokal: Symlink `/lib/systemd/system/sysinit.target.wants/sys-fs-fuse-connections.mount`, Unit `active (mounted)`, `/proc/mounts` enthält `fusectl /sys/fs/fuse/connections fusectl rw,nosuid,nodev,noexec,relatime`; udev-Regel `/lib/udev/rules.d/99-systemd.rules:83`).
   - Debian 13 (trixie): **belegt** über die Paketdateiliste `systemd` (`/usr/lib/systemd/system/sysinit.target.wants/sys-fs-fuse-connections.mount`).
   - Arch Linux: **belegt** über die Paketdateiliste `core/systemd` (`usr/lib/systemd/system/sysinit.target.wants/sys-fs-fuse-connections.mount`).
   - Fedora 42+: **nicht belegt** (Dateiliste nicht abrufbar). **Annahme:** Fedora übernimmt die Upstream-Unit-Verknüpfung.
3. Einschränkung: In Containern mit eigenem Nutzernamensraum (`ConditionVirtualization=!private-users`) und ohne `CAP_SYS_ADMIN` hängt systemd `fusectl` nicht ein.

**Belege.**
- Kernel master `fs/fuse/control.c:34-43` (`fuse_conn_abort_write`), `:219-220` (`inode->i_uid = fc->user_id; inode->i_gid = fc->group_id;`), `:256` (Verzeichnis `S_IFDIR | 0500`), `:262-266` (`waiting` `0400`, `abort` `0200`, `max_background` `0600`).
- Kernel master `Documentation/filesystems/fuse/fuse.rst:105-146` (Control filesystem, `abort`, „Only the owner of the mount may read or write these files.“), `:215-216` („Abort filesystem through the FUSE control filesystem. Most powerful method, always works.“).
- systemd `main` `units/meson.build:233-234` (`'file' : 'sys-fs-fuse-connections.mount', 'symlinks' : ['sysinit.target.wants/']`).
- `https://packages.debian.org/trixie/amd64/systemd/filelist`; `https://archlinux.org/packages/core/x86_64/systemd/files/`.

**Folgerung für den Plan:** `se drive unmount --force` über `/sys/fs/fuse/connections/<n>/abort` ist ohne Rechteerhöhung möglich, wenn `fusectl` eingehängt ist. Der Verzeichnisname `<n>` ist `fc->dev` als `%u` (`fs/fuse/control.c:250-256`), gesetzt aus `sb->s_dev` (`fs/fuse/inode.c:1760`); für FUSE-Mounts (Major 0) entspricht das `minor(st_dev)` des Einhängepunkts (**Folgerung** aus der Kernel-Kodierung von `dev_t`, nicht beobachtet). Fehlt `/sys/fs/fuse/connections/<n>`, bleibt nur `fusermount3 -u -z`.

**Verbleibende Lücke:** Fedora-42-Dateiliste von `systemd`.

### N3 — libuv `fs__realpath_handle` und `GetFinalPathNameByHandleW`

**Antwort.**
1. Ja. libuv (`v1.x`, neuestes Release v1.53.0) löst `uv_fs_realpath` unter Windows mit `CreateFileW(…, FILE_FLAG_BACKUP_SEMANTICS)` und `GetFinalPathNameByHandleW(handle, …, VOLUME_NAME_DOS)` auf. Liefert die Funktion 0, gibt libuv `-1` mit dem Win32-Fehler zurück.
2. Node.js: `fs.realpath.native`, `fs.realpathSync.native` und `fs.promises.realpath` rufen `binding.realpath` → `uv_fs_realpath` auf. `fs.realpath`/`fs.realpathSync` **ohne** `.native` sind JavaScript-Umsetzungen (eigener `lstat`-Weg) und nutzen `GetFinalPathNameByHandleW` nicht.
3. WinFsp-Laufwerk über `DefineDosDevice` (Standardweg ohne Administratorrechte, `-m M:`): `GetFinalPathNameByHandleW` scheitert mit `ERROR_UNRECOGNIZED_VOLUME` (1005). WinFsp-Pfleger (2022-01-18): „The `GetFinalPathNameByHandleW` API (unfortunately) does not recognize drives created by `DefineDosDeviceW`. It only works for drives created via the Mount Manager or network drives.“ #690 (2026-08-31, geschlossen 2026-09-17) bestätigt dasselbe für WinFsp-FUSE und Microsofts `edit.exe` 1.2.1/2.0.0; Lösung laut Pfleger: Mount Manager (`\\.\M:`), der Administratorrechte verlangt (Plan C1, Ref-Win).
4. Dokany mit `OPTION_MOUNT_MANAGER`: Nach der Aussage des WinFsp-Pflegers erkennt die API Mount-Manager-Laufwerke. Dokany #1240 (Windows-11-Notepad speichert nicht) wurde mit `DOKAN_OPTION_MOUNT_MANAGER` behoben (Kommentare 2024-08-14 und 2024-08-19); dort ist `GetFinalPathNameByHandleW` nicht ausdrücklich genannt. Der Bestand setzt `OPTION_MOUNT_MANAGER` (`N/mount/os/windows/host.rs:193-194`). Eine Messung von `GetFinalPathNameByHandleW` auf einem se-Dokany-Laufwerk gibt es nicht; im Repository prüft kein Test unter `N/mount/` diese API (Fundstellen nur `N/vfs/os/windows/volume_info.rs`, `N/host_trash/os/windows.rs`).

**Belege.**
- libuv `v1.x` `src/win/fs.c:3026-3073` (`fs__realpath_handle`), `:3075-3100` (`fs__realpath`).
- Node.js `main` `lib/fs.js:3357` (`realpathSync`, JS), `:3506-3513` (`realpathSync.native` → `binding.realpath`), `:3683-3691` (`realpath.native`); `lib/internal/fs/promises.js:2105-2113` (`binding.realpath`); `src/node_file.cc:2199,2207` (`uv_fs_realpath`).
- `https://github.com/winfsp/winfsp/issues/410` (Kommentar billziss-gh 2022-01-18T14:41:03Z); `https://github.com/winfsp/winfsp/issues/690` (Kommentare 2026-08-31, 2026-09-17).
- `https://github.com/dokan-dev/dokany/issues/1240` (Kommentare Liryna 2024-08-14, cji80 2024-08-19).

**Folgerung für den Plan:** Grund 3 der C1-Antwort ist für WinFsp ohne Mount Manager belegt. Die **Annahme**, dass Obsidian und VS Code Pfade über libuv mit dieser Funktion auflösen, ist nur für Aufrufe von `fs.promises.realpath` und `fs.realpath(.Sync).native` belegt; welche dieser Aufrufe Obsidian und VS Code nutzen, ist nicht belegt.

**Verbleibende Lücke:** (a) Aufrufstellen von `realpath` in VS Code und Obsidian (Obsidian ist nicht quelloffen); (b) Messwert `GetFinalPathNameByHandleW` auf einem Dokany-2.3.1-Laufwerk mit `OPTION_MOUNT_MANAGER | OPTION_CURRENT_SESSION` — gehört in den A/B-Lauf T2.

### N4 — `max_pages_limit` und `set_max_write(1 MiB)` in `fuser 0.18`

**Antwort.**
1. Standardwert `/proc/sys/fs/fuse/max_pages_limit` = **256** Seiten (`unsigned int fuse_max_pages_limit = 256;`), einstellbar 1–65535. Der Sysctl existiert seit dem Commit `2b3933b1e0a0` („fuse: enable dynamic configuration of fuse max pages limit (FUSE_MAX_MAX_PAGES)“, 2024-10-25); **Annahme:** erstmals in Kernel 6.13 (aus dem Commitdatum, Release-Zuordnung nicht geprüft). Kernel v6.8 (Ubuntu 24.04) hat stattdessen die Konstante `FUSE_MAX_MAX_PAGES 256`.
2. Kernel-Standard ohne Aushandlung: `max_pages = FUSE_DEFAULT_MAX_PAGES_PER_REQ` = 32 Seiten (128 KiB bei 4-KiB-Seiten). Nur mit dem INIT-Flag `FUSE_MAX_PAGES` übernimmt der Kernel `min(max_pages_limit, max(arg->max_pages, 1))`.
3. `fuser 0.18.0`: Standard-`max_write` = `MAX_WRITE_SIZE` = 16 MiB; `set_max_write(v)` erlaubt 1 ≤ v ≤ 16 MiB. `fuser` fordert `FUSE_MAX_PAGES` an, wenn der Kernel es anbietet, und meldet `max_pages = ceil(max(max_write, max_readahead) / Seitengröße)`. Mit `set_max_write(1 MiB)` und 4-KiB-Seiten ergibt das 256 Seiten (wenn `max_readahead` ≤ 1 MiB); der Kernel übernimmt `min(256, 256) = 256` → Schreibanfragen bis 1 MiB. Ohne `set_max_write` meldet `fuser` 4096 Seiten, der Kernel kappt auf 256; `max_write` bliebe 16 MiB, die tatsächliche Schreibgröße 1 MiB.
4. Kernel setzt `fc->max_write = max(4096, arg->max_write)`.

**Belege.**
- Kernel master `fs/fuse/inode.c:38` (`fuse_max_pages_limit = 256`), `:977-978`, `:1345-1355` (`FUSE_MAX_PAGES`-Übernahme), `:1425-1426` (`max_write`); `fs/fuse/fuse_i.h:35` (`FUSE_DEFAULT_MAX_PAGES_PER_REQ 32`); `fs/fuse/sysctl.c:15,25-31` (Grenze 65535, Mindestwert 1).
- Kernel v6.8 `fs/fuse/fuse_i.h:36,39` (`FUSE_MAX_MAX_PAGES 256`), `fs/fuse/inode.c:924-925`.
- Kernel master `Documentation/admin-guide/sysctl/fs.rst:347-349`.
- `fuser-0.18.0/src/session.rs:55` (`MAX_WRITE_SIZE = 16 * 1024 * 1024`), `src/lib.rs:222-236` (Standardwerte `KernelConfig`, `max_background: 16`), `:299-309` (`set_max_write`), `:388-390` (`max_pages`), `:121-122` (`FUSE_MAX_PAGES` angefordert), `src/ll/request.rs:1012`.

**Folgerung für den Plan:** Die **Annahme** „Kernel-Standard `max_pages_limit` 256 Seiten“ ist bestätigt; `max_write` 1 MiB ist auf Kernel 6.8 und auf aktuellen Kernels mit Standard-Sysctl wirksam.

**Verbleibende Lücke:** Kernelversion, in der `max_pages_limit` erschien (nur Commitdatum belegt). Auf Architekturen mit 16-KiB- oder 64-KiB-Seiten ergibt `set_max_write(1 MiB)` weniger Seiten (64 bzw. 16); keine Wirkung auf die Zielplattformen x86_64 mit 4-KiB-Seiten.

### N5 — Schreibrecht in `fusermount3` und im direkten `mount(2)` von `fuser`

**Antwort.**
1. `fusermount3` (Tag `fuse-3.18.3`): Für Nicht-root prüft `check_perm` bei einem Verzeichnis: (a) Sticky-Bit gesetzt und Besitzer ≠ Aufrufer → Abbruch „mountpoint … not owned by user“; (b) `access(".", W_OK)` → ohne Schreibrecht Abbruch „user has no write access to mountpoint“. Vorher öffnet `pin_mountpoint` den Pfad mit `O_PATH | O_CLOEXEC | O_NOFOLLOW` (Symlink wird abgelehnt). **Schreibrecht ist Pflicht.**
2. `fuser 0.18.0` versucht zuerst `mount(2)` direkt (`fuse_mount_sys`): öffnet den Einhängepunkt lesend (`File::open(mountpoint)` für `rootmode`), öffnet `/dev/fuse` und ruft `mount("…", mountpoint, "fuse", flags, "fd=…,rootmode=…,user_id=…,group_id=…")`. Ein Nicht-root-Prozess ohne `CAP_SYS_ADMIN` erhält `EPERM`; `fuser` fällt dann auf `fusermount3` zurück. Der direkte Weg prüft **kein** Schreibrecht (das ist Sache von `fusermount3`; der Kernel verlangt für `mount(2)` die Fähigkeit, nicht das Schreibrecht). Mit `AutoUnmount` geht `fuser` direkt über `fusermount3`.
3. Lokale Version Ubuntu 24.04: `fuse3 3.14.0-5build1`, `/usr/bin/fusermount3` mit Setuid-Bit.

**Belege.**
- libfuse `fuse-3.18.3` `util/fusermount.c:1113-1125` (`pin_mountpoint`, `O_NOFOLLOW` für Nicht-root), `:1138-1183` (`check_perm`: Sticky-Prüfung, `access(*mntp, W_OK)`).
- `fuser-0.18.0/src/mnt/fuse_pure.rs:106-130` (`fuse_mount_pure`: direkt, dann `fusermount`), `:384-497` (`fuse_mount_sys`, `Err(EPERM) => Ok(None) // Retry with fusermount`), `:161-187` (`detect_fusermount_bin`: `FUSERMOUNT_PATH`, `fusermount3`, `fusermount`, `mount_fusefs`, `/sbin/…`, `/bin/…`).

**Folgerung für den Plan:** Die **Annahme** R4 ist bestätigt: Ein auf `0500` verriegelter Einhängepunkt lässt `fusermount3` scheitern; `0700` unmittelbar vor dem Einhängen ist nötig. Da `fuser` den Einhängepunkt nur lesend öffnet, stört `0500` den direkten Versuch nicht (er scheitert ohnehin an `EPERM`).

**Verbleibende Lücke:** keine.

### N6 — Systemaufrufe beim Speichern und Wirkung von `ENOTSUP` für `fchmod`/`fchown`/`utimensat`

**Antwort (je Programm aus dem Quelltext; keine Laufzeitbeobachtung).**

| Programm | Speicherweg | `fchown`/`chown` | `fchmod`/`chmod` | `utimensat`/`utimes` | Wirkung von `ENOTSUP` |
|---|---|---|---|---|---|
| GIO `g_file_replace` (GNOME Text Editor, gedit über GtkSourceView) | neue Datei `.goutputstream-XXXXXX` mit `O_CREAT\|O_EXCL`, Schreiben, optional `fsync`, `rename` auf den Zielnamen | `fchown(tmpfd, uid, gid)` | `fchmod(tmpfd, mode)` | keiner | Scheitern von `fchown` oder `fchmod` führt nur dann zum Rückfall, wenn danach `fstat` andere uid/gid/mode zeigt; Rückfall = Original kürzen und in place neu schreiben (kein Fehler, aber nicht atomar) |
| vim (`backupcopy=auto`, Standard) | Probe `.4913.swp`-artig (`O_CREAT\|O_EXCL\|O_NOFOLLOW`), `fchown`, `fchmod`, `stat`, Löschen; danach je nach Ergebnis Umbenennen-und-neu-Schreiben oder Kopie-und-Überschreiben; `fsync` | Ergebnis ignoriert | Ergebnis ignoriert | nur für Sicherungsdatei/Rückfall, Ergebnis ignoriert | kein Fehler; weicht `stat` ab, wählt vim Kopieren-und-Überschreiben; **`fsync`-Fehler meldet vim als „E667: Fsync failed“** |
| Kate/KWrite (KTextEditor `master`, unprivilegierter Weg) | `KCompressionDevice(new QFile(filename))`, `open(WriteOnly)` = Kürzen und in place schreiben | keiner | keiner | keiner | nicht betroffen |
| KIO-Worker `file` `put` (Dolphin-Kopieren, KIO-Apps) | `<Ziel>.part`, schreiben, `rename` | `chown` → bei Fehler Warnung | `QFile::setPermissions` → bei Fehler Warnung „Could not change permissions“ | `utimes` → Ergebnis ignoriert | Kopie gelingt; Warnung bei `chmod`/`chown` |
| `cp -a` (coreutils `master`) | Ziel anlegen, kopieren | `fchown`; Fehler toleriert nur bei `EPERM`, `EINVAL`, `EACCES` (und ohne Privilegien) | `fchmod`; Fehler → Meldung „preserving permissions for …“, `require_preserve` → Rückgabe falsch | `fdutimensat`; Fehler → „preserving times for …“, Abbruch der Datei | **`ENOTSUP` → Fehlermeldung und Exitstatus ≠ 0** (für `chown`, `chmod`, Zeiten) |
| `rsync -a` (`master`) | temporäre Datei, `rename` | nur wenn uid/gid geändert werden müssen; Fehler → „chown … failed“ | bei abweichendem Modus `chmod`; Fehler → „failed to set permissions“ | `set_times`; Fehler → „failed to set times on …“ | **`ENOTSUP` → Fehler je Datei, Exitcode 23 (Annahme aus `FERROR_XFER`, nicht im Quelltext nachgelesen)** |
| LibreOffice | nicht im Quelltext geprüft | Bugzilla #36827 berichtet `chown` beim Speichern (sshfs, Synology) | — | — | **Lücke** |

Zusatzbefund `fsync`: Antwortet der FUSE-Server auf `FSYNC` mit `ENOSYS`, setzt der Kernel `no_fsync` und meldet dem Aufrufer Erfolg (`fs/fuse/file.c:576-581`); dasselbe gilt für `FLUSH` (`:491-506`). Antwortet er mit einem anderen Fehler, sieht vim „E667“.

**Belege.**
- GLib `main` `gio/glocalfileoutputstream.c:1022-1092` (Strategie, `fchown`/`fchmod`, Vergleich per `fstat`, `goto fallback_strategy`), `:330` (`g_fsync`), `:421` (`g_rename`).
- vim `master` `src/bufwrite.c:1181-1255` (`BKC_AUTO`, Probe, `fchown`/`mch_fsetperm` ohne Fehlerprüfung, Vergleich), `:2218-2224` (`vim_fsync` → `e_fsync_failed`).
- KTextEditor `master` `src/buffer/katetextbuffer.cpp:911-935` (`saveBufferUnprivileged`); KArchive `master` `src/kcompressiondevice.cpp:220-221` (`new QFile(fileName)`).
- KIO `master` `src/kioworkers/file/file.cpp:562` (`.part`), `:705-760` (Umbenennen, `setPermissions` → `warning`, `chown` → `warning`, `utimes` ohne Prüfung).
- coreutils `master` `src/cp.c:1071-1084` (`-a`: `require_preserve = true`), `src/copy.c:1050-1057` (`fdutimensat`), `:1108-1116` (`fchmod_or_lchmod`), `:2846-2870` (`chown_failure_ok`, `owner_failure_ok`: nur `EPERM`, `EINVAL`, `EACCES`).
- rsync `master` `rsync.c:676-688` (chown), `:772-785` (`failed to set times`), `:811-815` (`failed to set permissions`).
- Kernel master `fs/fuse/file.c:491-506`, `:576-581`.
- LibreOffice Bugzilla `https://bugs.libreoffice.org/show_bug.cgi?id=36827` (nur Nutzerbericht).

**Folgerung für den Plan:** R5 ist für `cp -a` und `rsync -a` **eingetreten** (Quelltextlage): `ENOTSUP` auf `utimensat`, `fchmod` oder `fchown` lässt beide mit Fehler enden. GIO, vim, Kate und KIO speichern trotz `ENOTSUP`. Für `fchown` toleriert `cp` nur `EPERM`/`EINVAL`/`EACCES`; für Zeiten und Modus toleriert `cp -a` keinen Fehler. Die Entscheidung zwischen „`ENOTSUP`“, „Erfolg ohne Wirkung“ und „Zeit als Backend-Fähigkeit setzen“ trifft der Planverfasser (siehe Abschnitt 3).

**Verbleibende Lücke:** (a) LibreOffice-Speicherweg (Quelltext `sal/osl/unx/file.cxx`, `ucb/source/ucp/file/` nicht gelesen); (b) Exitcode von rsync bei `FERROR_XFER` (Annahme 23); (c) Laufzeitbestätigung aller Zeilen.

### N7 — Erkennen GTK und KDE eine per Umbenennen ersetzte Lesezeichendatei sofort?

**Antwort.**
1. GTK 3 und GTK 4 (`GtkBookmarksManager`, Datei `$XDG_CONFIG_HOME/gtk-3.0/bookmarks` in beiden): `g_file_monitor_file(…, G_FILE_MONITOR_NONE, …)`. GLib meldet ein Umbenennen **auf** den überwachten Namen ohne `WATCH_MOVES` als synthetisches `CREATED` + `CHANGES_DONE_HINT` (Paar `IN_MOVED_FROM`/`IN_MOVED_TO` im selben Ordner → `RENAMED` → ohne Flags `DELETED` für den alten Namen und `send_synthetic_created` für den neuen). GTK lädt bei `CHANGED`, `CHANGES_DONE_HINT`, `CREATED` (GTK 3 zusätzlich `DELETED`) neu. **Ja, sofort** (Ereignis ohne Ratenbegrenzung).
2. Nautilus (`main`): eigene `NautilusBookmarkList`, `g_file_monitor_file(file, 0, …)` mit `g_file_monitor_set_rate_limit(…, 1000)`, lädt bei `CHANGED` und `CREATED`. **Ja**; die Ratenbegrenzung 1000 ms gilt in GLib für `CHANGED`-Ereignisse.
3. KDE (`KBookmarkManager`, KBookmarks `master`): `KDirWatch::self()->addFile(datei)`, Signale `dirty`, `created`, `deleted` → `slotFileChanged` → `parse()`. KDirWatch (inotify) behandelt `IN_DELETE_SELF` der alten Inode mit `emitEvent(e, Deleted)` und überwacht danach den Elternordner; `IN_CREATE | IN_MOVED_TO` lösen `Created` bzw. einen Nachscan mit `m_PollInterval` = 500 ms aus. Beim Ersetzen per `rename` erhält der Manager mindestens `deleted` und liest dann den Pfad neu, der bereits die neue Datei ist. **Ja**, innerhalb von 0–500 ms (Quelltextlage).

**Belege.**
- GTK `gtk-3-24` `gtk/gtkbookmarksmanager.c:76` (Pfad), `:188-201` (Ereignisse), `:244-254` (Monitor); GTK `main` `gtk/gtkbookmarksmanager.c:78-82`, `:231-256`, `:292-302`.
- GLib `main` `gio/glocalfilemonitor.c:325-331` (`send_synthetic_created`), `:403-409` (`MOVED_IN` → `CREATED`), `:421-450` (`RENAMED` ohne Flags → `DELETED` + `CREATED`).
- Nautilus `main` `src/nautilus-bookmark-list.c:69-76` (Pfad `gtk-3.0/bookmarks`), `:172-188`.
- KBookmarks `master` `src/kbookmarkmanager.cpp:141-160`; KCoreAddons `master` `src/lib/io/kdirwatch.cpp:232-233` (`m_PollInterval` 500), `:350-366` (`IN_DELETE_SELF`), `:372-396` (`IN_CREATE | IN_MOVED_TO`), `:745` (inotify-Maske).

**Folgerung für den Plan:** DRV-M5 („atomar über Hilfsdatei ersetzt“) wird von GTK, Nautilus und KDE erkannt. Die Hilfsdatei muss im selben Ordner liegen, damit inotify das Paar `IN_MOVED_FROM`/`IN_MOVED_TO` liefert; ein Umbenennen über Dateisystemgrenzen ist ohnehin kein `rename(2)`.

**Verbleibende Lücke:** Welche KDE-Komponente `user-places.xbel` hält (`KFilePlacesModel` in KIO) und ob sie ein im Speicher gehaltenes Dokument später über die se-Änderung zurückschreibt — nicht gelesen. Laufzeitbestätigung fehlt.

### N8 — Anzeigename und Sichtbarkeit eines FUSE-Mounts unter `~/SmartExplorer/`

**Antwort.**
1. Name: `g_unix_mount_entry_guess_name` liefert für jeden Pfad außer `/` `g_filename_display_basename(mount_path)` — für `~/SmartExplorer/Geräte` also „Geräte“, für `~/SmartExplorer/<Name>` „<Name>“. Der Dateisystemtyp und `FSName` gehen nicht in den Namen ein.
2. Aufnahme in die Mount-Liste:
   - GVfs (`gvfs-udisks2-volume-monitor`, auf Systemen mit installiertem GVfs): `should_include`: `x-gvfs-show`/`x-gvfs-hide` in den Optionen haben Vorrang; sonst ausgeschlossen bei systeminternem Pfad und bei „/.“ im Pfad; **aufgenommen**, wenn der Pfad mit `$HOME/` beginnt.
   - GLib ohne GVfs (`GUnixVolumeMonitor`): `g_unix_mount_entry_guess_should_display`: ausgeschlossen bei systeminternem Typ oder „/.“ im Pfad; **aufgenommen** unter `$HOME/`, `/media/`, `/run/media/$USER/`.
   - Systeminterne FUSE-Typen in GLib: `fuse.gvfsd-fuse`, `fuse.portal`, `fusectl` (nicht `fuse.smartexplorer`).
3. Seitenleisten (Quelltext):
   - Nautilus `main` (nach 51.1): Mounts ohne Volume (`g_volume_monitor_get_mounts`, nicht verschattet, `g_file_is_native(root)`) → Abschnitt `NAUTILUS_SIDEBAR_SECTION_MOUNTS` mit `g_mount_get_name`.
   - Nemo `master`: derselbe Ablauf in `nemo-places-sidebar.c`.
   - Thunar `master`: `ThunarDeviceMonitor` nimmt Mounts ohne Volume mit `file://`-Wurzel als `THUNAR_DEVICE_KIND_MOUNT_LOCAL` auf; die Seitenleiste zeigt Geräte des Monitors.

**Belege.**
- GLib `main` `gio/gunixmounts.c:3513-3523` (`guess_name`), `:3700-3775` (`guess_should_display`), `:329-350` (Liste systeminterner Typen).
- GVfs `master` `monitor/udisks2/gvfsudisks2volumemonitor.c:848-917` (`should_include`), `:933-972` (`should_include_mount`); `monitor/udisks2/gvfsudisks2mount.c:345` (`g_unix_mount_entry_guess_name`).
- Nautilus `main` `src/nautilus-sidebar.c:964-1005`; Nemo `master` `src/nemo-places-sidebar.c:956-985`; Thunar `master` `thunar/thunar-device-monitor.c:492-552`.

**Folgerung für den Plan:** Die Aussage in C2 (Sichtbarkeit unter `$HOME/` ohne „/.“) ist aus dem Quelltext von GLib, GVfs, Nautilus, Nemo und Thunar bestätigt. Der Anzeigename ist der letzte Pfadteil; ein Gerätebaum unter `~/SmartExplorer/Geräte` erscheint als „Geräte“. Zusammen mit dem GTK-Lesezeichen (Name aus der Lesezeichenzeile) können in Nautilus zwei Einträge erscheinen (Abschnitt „Lesezeichen“ und Abschnitt „Mounts“) — **Folgerung aus dem Quelltext, nicht beobachtet**; Nautilus unterdrückt Lesezeichen auf Mount-Wurzeln nicht in dem gelesenen Abschnitt.

**Verbleibende Lücke:** (a) Nautilus 47–50 nicht einzeln gelesen (**Annahme:** gleicher Ablauf, da `nautilus-sidebar.c` aus dem GTK-Places-Sidebar-Fork stammt); (b) Laufzeitbeobachtung; (c) Doppelanzeige Lesezeichen + Mount nicht beobachtet; (d) Dolphin/Solid nicht erneut geprüft (Plan stützt sich auf Ref-FUSE 2.4.4).

### N9 — Ist `fuse3`/`fusermount3` vorinstalliert?

**Antwort.**

| Distribution, Desktop | Ergebnis | Kette |
|---|---|---|
| Ubuntu 24.04 Desktop (GNOME) | **ja** (über Recommends, die der Installer und `apt` standardmäßig installieren) | `ubuntu-desktop-minimal 1.539.2` **Recommends** `gvfs-fuse` und `xdg-desktop-portal-gnome`; `gvfs-fuse 1.54.4-0ubuntu1~24.04.4` **Depends** `fuse3`; `xdg-desktop-portal 1.18.4-1ubuntu2.24.04.3` **Depends** `fuse3` |
| Kubuntu 24.04 | **ja** | `kubuntu-desktop` **Depends** `xdg-desktop-portal-kde` (→ **Depends** `xdg-desktop-portal` → `fuse3`) und **Recommends** `kio-fuse` (→ **Depends** `fuse3`) |
| Debian 13 GNOME (`task-gnome-desktop`) | **ja** | `task-gnome-desktop` → `gnome-core`; `gnome-core` **Depends** `gvfs-fuse`; `xdg-desktop-portal 1.20.3+ds-1` **Depends** `fuse3` |
| Debian 13 KDE (`task-kde-desktop`) | **ja** (über Recommends) | `task-kde-desktop` → `kde-standard`; `kde-plasma-desktop 5:162` → `plasma-desktop 4:6.3.6-1` **Recommends** `xdg-desktop-portal-kde 6.3.5-1` (**Depends** `kio-fuse`, `xdg-desktop-portal`); `plasma-workspace 4:6.3.6-2` **Recommends** `kio-fuse`; `kio-fuse 5.1.0-1` **Depends** `fuse3` |
| Fedora Workstation 42 | **ja, mit Annahme** | `xdg-desktop-portal.spec` (f42) `Requires: fuse3`; `flatpak.spec` (f42) `Requires: xdg-desktop-portal > 1.5.3`; `gvfs-fuse` `Requires: fuse3`; **Annahme:** Workstation installiert `flatpak` standardmäßig (Comps-Datei nicht abrufbar) |
| Arch mit GNOME/KDE | **ja, wenn die Gruppen `gnome` bzw. `plasma` installiert sind** | `extra/xdg-desktop-portal` hängt von `fuse3` ab; Arch hat keine Vorinstallation — Gruppenbestand nicht einzeln geprüft (**Lücke**) |

Fedora `fuse3.spec` (f42) setzt `%attr(4755,root,root) %{_bindir}/fusermount3` (Setuid).

**Belege.**
- Lokal: `apt-cache show ubuntu-desktop-minimal=1.539.2`, `apt-cache show gvfs-fuse`, `apt-cache show xdg-desktop-portal`, `apt-cache show kubuntu-desktop`, `apt-cache show kio-fuse`, `apt-cache show xdg-desktop-portal-kde`, `apt-cache rdepends --installed fuse3` (Ausgabe: `open-vm-tools`, `xdg-desktop-portal`, `ntfs-3g`, `libfuse3-3`).
- `https://packages.debian.org/trixie/{xdg-desktop-portal,gnome-core,task-gnome-desktop,task-kde-desktop,kde-plasma-desktop,plasma-desktop,plasma-workspace,xdg-desktop-portal-kde,kio-fuse}`.
- `https://src.fedoraproject.org/rpms/xdg-desktop-portal/raw/f42/f/xdg-desktop-portal.spec:56`, `…/flatpak/raw/f42/f/flatpak.spec:95-97`, `…/gvfs/raw/f42/f/gvfs.spec:96`, `…/fuse3/raw/f42/f/fuse3.spec:110`.
- `https://archlinux.org/packages/extra/x86_64/xdg-desktop-portal/` (Abhängigkeit `fuse3`).

**Folgerung für den Plan:** Die Voreinstellung „ein, wenn die Laufzeit bereit ist“ (C5) trifft auf allen geprüften Desktop-Standardinstallationen eine vorhandene `fusermount3` an; die Kandidaten `gvfs-fuse` und `xdg-desktop-portal` sind beide belegt. Minimal-Installationen ohne Recommends (`--no-install-recommends`) können `fuse3` fehlen lassen; dafür bleibt der Hinweis in C2.

**Verbleibende Lücke:** Fedora-Workstation-Comps (Vorinstallation `flatpak`); Arch-Gruppeninhalte `gnome`/`plasma` mit `xdg-desktop-portal-*`.

### N10 — Läuft `ContentProvider.onCreate` vor `Application.onCreate`?

**Antwort.** Ja, in Android 11 bis 16 und auf `main`. `ActivityThread.handleBindApplication` erzeugt die `Application` (über `makeApplication`), ruft dann `installContentProviders(app, data.providers)` (nur außerhalb des eingeschränkten Sicherungsmodus `restrictedBackupMode`) und erst danach `mInstrumentation.callApplicationOnCreate(app)`. Kommentar im Quelltext: „Do this after providers, since instrumentation tests generally start their test thread at this point“.

**Belege.**
- AOSP `frameworks/base` `main` `core/java/android/app/ActivityThread.java:7406` (`handleBindApplication`), `:7700-7705` (`installContentProviders`), `:7720` (`callApplicationOnCreate`).
- `android11-release` derselben Datei: `:6697` / `:6712`; `android16-release`: `:7814` / `:7830`.
- Bestand: `android/app/src/main/java/app/smartexplorer/android/SmartExplorerApp.kt:18-22` initialisiert `AppPrefs` und `WakeKeeper` in `Application.onCreate`.

**Folgerung für den Plan:** Die **Annahme** in C3/R7 ist bestätigt. `SeDocumentsProvider.onCreate` darf sich nicht auf `AppPrefs.init`/`WakeKeeper.init` aus `SmartExplorerApp.onCreate` verlassen; die idempotente Initialisierung im Anbieter ist nötig. Im eingeschränkten Sicherungsmodus werden Anbieter gar nicht installiert.

**Verbleibende Lücke:** keine.

### N11 — vold-Sperre und Watchdog bei blockiertem `FUSE_LOOKUP`/`onGetSize`

**Antwort (Quelltextlage, nicht beobachtet).**
1. `VoldNativeService::openAppFuseFile`, `mountAppFuse` und `unmountAppFuse` laufen unter `ACQUIRE_LOCK` (globale vold-Sperre). `openAppFuseFile` ruft `AppFuseUtil::OpenAppFuseFile` → `open(path, flags)` auf dem AppFuse-Mount; dieses `open` erzeugt `FUSE_LOOKUP` (und `FUSE_OPEN`) an die App. In der App beantwortet `FuseAppLoop` `FUSE_LOOKUP` und `FUSE_GETATTR` mit `callback.onGetSize()` auf dem Handler des Rückrufs. Blockiert `onGetSize`, hält vold die Sperre.
2. `StorageManagerService` ist ein `Watchdog.Monitor` (`WATCHDOG_ENABLE = true` in Android 14, 15, 16 und `main`); `monitor()` ruft `mVold.monitor()`, das `ACQUIRE_LOCK` nimmt. Der system_server-Watchdog hat `DEFAULT_TIMEOUT` = 60 s; bleibt ein Monitor so lange hängen, beendet der Watchdog system_server (Neustart der Bedienoberfläche).
3. Ab Android 15 (`android15-release`, `android16-release`, `main`) ruft `AppFuseMountScope.openFile` vorher `extendWatchdogTimeout("#openFile might be slow")` auf: `pauseWatchingMonitorsFor(30 s)` und `pauseWatchingCurrentThreadFor(30 s)`. In `android14-release` fehlt dieser Aufruf in `openFile`.
4. `AppFuseMountScope.close` hängt `unmountAppFuse` absichtlich auf einen Hintergrund-Thread, mit dem Quelltextkommentar zur Verklemmung „the vold lock is held by another thread, which called: mVold.openAppFuseFile() … vold calls open() on the underlying file, which is a call that needs to be handled by the AppFuseThread“.

**Belege.**
- AOSP `system/vold` `main` `VoldNativeService.cpp:153-161` (`monitor`), `:584-597` (`mountAppFuse`, `unmountAppFuse`), `:599-609` (`openAppFuseFile`); `AppFuseUtil.cpp:152-163` (`OpenAppFuseFile` → `open`).
- AOSP `frameworks/base` `main` `services/core/java/com/android/server/StorageManagerService.java:226` (`SLOW_OPERATION_WATCHDOG_TIMEOUT_MS = 30 * 1000`), `:311` (`WATCHDOG_ENABLE = true`), `:1258-1262` (`extendWatchdogTimeout`), `:2029-2031` (`addMonitor`), `:3693-3745` (`AppFuseMountScope`), `:4875-4880` (`monitor`); `android14-release`: `:291`, `:1918-1920`, `:3517` (`openFile` ohne Verlängerung); `android15-release`: `:3656`; `android16-release`: `:230`, `:315`, `:3756`.
- AOSP `frameworks/base` `main` `services/core/java/com/android/server/Watchdog.java:101` (`DEFAULT_TIMEOUT = DB ? 10 * 1000 : 60 * 1000`).
- AOSP `frameworks/base` `main` `core/java/com/android/internal/os/FuseAppLoop.java:160-176` (`FUSE_LOOKUP`/`FUSE_GETATTR` → `onGetSize`).

**Folgerung für den Plan:** R6 ist im Mechanismus belegt (nicht nur RSAF-Kommentar). Ein blockierendes `onGetSize` blockiert vold; ab 60 s (Android 14) bzw. 60 s + 30 s Pause (Android 15/16) droht der Watchdog-Neustart von system_server. `onGetSize` muss ohne Netz aus dem Handle antworten (wie in C3 festgelegt).

**Verbleibende Lücke:** Laufzeitbeobachtung; ob OEM-Builds `WATCHDOG_ENABLE` ändern.

### N12 — Indiziert MediaStore Inhalte von Drittanbieter-DocumentsProvidern?

**Antwort.** Nicht belegt, dass es das tut; die Primärquelle beschreibt nur das Scannen von Speichervolumes: „The system automatically scans an external storage volume and adds media files to the following well-defined collections“ (Images, Videos, Audio, Downloads). Eine Schnittstelle, über die ein `DocumentsProvider` seine Dokumente in MediaStore einträgt, nennt die Seite nicht. Ein `DocumentsProvider` stellt kein Speichervolume bereit.

**Belege.** `https://developer.android.com/training/data-storage/shared/media` (Stand der Seite laut Abruf; Abruf 2026-10-17).

**Folgerung für den Plan:** Die **Annahme** in C3 („Galerie-Apps über MediaStore zeigen die Inhalte nicht“) bleibt eine Annahme, aber ohne Gegenbeleg; sie folgt aus der Scan-Beschreibung.

**Verbleibende Lücke:** Ausdrückliche Negativaussage in AOSP `MediaProvider` (Scanner-Quellen nicht gelesen).

### N13 — `flock` auf `filesDir` unter Android 11–16

**Antwort.** Ja, mit Linux-Semantik. Bionic leitet `flock(int, int)` als direkten Systemaufruf weiter (`SYSCALLS.TXT`), die Sperre gehört zur offenen Dateibeschreibung (Kernel-VFS, nicht dateisystemabhängig für ext4/f2fs). SELinux erlaubt `appdomain` auf `app_data_file:file` `create_file_perms`; diese umfassen `rw_file_perms` und damit `lock` (`r_file_perms` und `w_file_perms` enthalten `lock`). Der Bestand nutzt `flock(LOCK_EX | LOCK_NB)` bereits für den Daemon-Singleton auf Android (Kommentar in `N/daemon/os/android/platform.rs:1-7`, Umsetzung `N/daemon/os/linux_os/platform.rs:237`).

**Belege.**
- AOSP `bionic` `main` `libc/SYSCALLS.TXT:143` (`flock(int, int)   all`).
- AOSP `system/sepolicy` `main` `public/global_macros:25-29` (`r_file_perms`, `w_file_perms` mit `lock`); `private/app.te:249-258` (`app_data_file:file create_file_perms`).
- `N/daemon/os/android/platform.rs:1-7`, `N/daemon/os/linux_os/platform.rs:43`, `:237`.

**Folgerung für den Plan:** DRV-M2 „Lease über `flock` auf Linux und Android“ ist tragfähig; zwei Threads im selben Prozess sperren sich gegenseitig nur, wenn sie die Datei getrennt öffnen (eigene offene Dateibeschreibung) — das entspricht dem Linux-Verhalten.

**Verbleibende Lücke:** Laufzeitbestätigung auf Android 11 (minSdk 30).

### N14 — Vordergrunddienst aus einem Prozess, der einen `ContentProvider` bedient; Fristen für expedited WorkManager-Jobs

**Antwort.**
1. Die Liste „Exemptions from background start restrictions“ (Stand der Seite 2026-10-16) nennt 14 Ausnahmen: (1) Übergang aus einem sichtbaren Zustand, (2) App darf eine Activity aus dem Hintergrund starten (außer Activity im Backstack einer bestehenden Aufgabe), (3) FCM-Nachricht hoher Priorität, (4) Nutzeraktion an einem UI-Element der App (Bubble, Benachrichtigung, Widget, Activity), (5) exakter Alarm für eine Nutzeranfrage, (6) aktuelle Eingabemethode, (7) Geofencing/Aktivitätserkennung, (8) `ACTION_BOOT_COMPLETED`/`ACTION_LOCKED_BOOT_COMPLETED`/`ACTION_MY_PACKAGE_REPLACED`, (9) `ACTION_TIMEZONE_CHANGED`/`ACTION_TIME_CHANGED`/`ACTION_LOCALE_CHANGED`, (10) `ACTION_TRANSACTION_DETECTED` von `NfcService`, (11) Systemrollen/Berechtigungen (Geräte-/Profilbesitzer), (12) Companion Device Manager mit `REQUEST_COMPANION_START_FOREGROUND_SERVICES_FROM_BACKGROUND` oder `REQUEST_COMPANION_RUN_IN_BACKGROUND`, (13) **der Nutzer hat die Akkuoptimierung für die App abgeschaltet**, (14) `SYSTEM_ALERT_WINDOW` (ab Ziel-SDK 35 zusätzlich sichtbares Overlay-Fenster). **Das Bedienen eines `ContentProvider` ist keine Ausnahme**; die Seite erwähnt Content Provider nicht.
2. Expedited WorkManager: Ausführungszeit aus einem Kontingent nach App-Standby-Bucket und Prozesswichtigkeit; im Vordergrund kein Kontingentlimit; bei erschöpftem Kontingent `OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST` oder `DROP_WORK_REQUEST`. Eine Frist in Sekunden nennt die Seite nicht („complete within a few minutes“). Vor Android 12 kann WorkManager einen Vordergrunddienst nutzen; dann muss der Worker `getForegroundInfo()` umsetzen („Failing to implement the corresponding `getForegroundInfo` method can lead to runtime crashes when calling `setExpedited` on older platform versions.“).

**Belege.**
- `https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start` (zuletzt aktualisiert 2026-10-16).
- `https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work` (zuletzt aktualisiert 2026-10-16), Abschnitte „Quotas“, „Quota policies“, „Backwards compatibility and foreground services“.
- Bestand: `android/app/build.gradle.kts:89` (`minSdk = 30`, Android 11); `AndroidManifest.xml:27` (`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`); `work/SyncWorker.kt:102-103` (bestehender Worker mit `setForeground(ForegroundInfo(…))`).

**Folgerung für den Plan:** R8 bleibt: Nach `onRelease` darf die App keinen Vordergrunddienst aus dem Hintergrund voraussetzen, außer der Nutzer hat die Akkuoptimierung abgeschaltet (Ausnahme 13; der Bestand fordert diese Ausnahme bereits an). `DocsCommitWorker` (expedited) muss wegen minSdk 30 (Android 11) `getForegroundInfo()` umsetzen.

**Verbleibende Lücke:** Prozesszustand eines Anbieterprozesses während einer aktiven Client-Verbindung und dessen Wirkung auf die AMS-Prüfung des Hintergrundstarts (AOSP `ActiveServices` nicht gelesen); Kontingentwerte je Bucket.

### N15 — Wertet DocumentsUI `EXTRA_ERROR` und `EXTRA_LOADING` sichtbar aus?

**Antwort.** Ja, in Android 14, 16 und `main` gleich. `Model.update` liest `DocumentsContract.EXTRA_INFO`, `EXTRA_ERROR` und `EXTRA_LOADING` aus den Cursor-Extras. `DirectoryFragment` zeigt bei `isLoading()` die Fortschrittsleiste. `Message.HeaderMessage.update` zeigt bei gesetztem `error` den Text als Kopfzeile mit Symbol `ic_dialog_alert` (Vorrang vor `info`, das mit `ic_dialog_info` erscheint; Authentifizierungsfehler haben noch höheren Vorrang).

**Belege.**
- AOSP `packages/apps/DocumentsUI` `main` `src/com/android/documentsui/Model.java:68`, `:144-146`, `:181-182`; `src/com/android/documentsui/dirlist/DirectoryFragment.java:1441-1444`; `src/com/android/documentsui/dirlist/Message.java:167-180`.
- Gleiche Zeilen `Model.java:145-146` in `android14-release` und `android16-release`.

**Folgerung für den Plan:** Die Offline-Anzeige in C3 („Cursor mit `EXTRA_ERROR` … Ordner bleibt“) wird von DocumentsUI als Kopfzeile dargestellt; `EXTRA_LOADING` als Fortschrittsleiste.

**Verbleibende Lücke:** Darstellung in Hersteller-Dateiauswahlen, die nicht AOSP-DocumentsUI sind; Darstellung in Drittanbieter-Apps, die `ACTION_OPEN_DOCUMENT_TREE` direkt lesen.

### N16 — FUSE-Mounts in WSL2 im Windows-Explorer; `fusermount3` in WSL-Ubuntu

**Antwort.**
1. Sichtbarkeit: Der Windows-Explorer greift über `\\wsl.localhost`/`\\wsl$` als **Standardnutzer der Distribution** zu. Ein WSL-Mitarbeiter (OneBlue, COLLABORATOR, 2025-10-14/15) nennt zwei Ursachen für unsichtbare FUSE-Inhalte: (1) Mount-Namensräume („try to mount the folder in `/mnt/wsl`“) und (2) Rechte des Standardnutzers; „The explorer always uses the distribution's default filesystem.“ Issue #8423 (2022): sshfs-Inhalte nur sichtbar, wenn unter `/mnt/wsl` oder `/mnt/wslg` eingehängt. Ein Mount unter `~/SmartExplorer/…` durch den Standardnutzer ist danach nicht zugesagt sichtbar.
2. `fusermount3` in WSL-Ubuntu 24.04: Paket `ubuntu-wsl 1.539.2` **Recommends** `snapd`; `snapd 2.77.1+ubuntu24.04` **Depends** `fuse3 (>= 3.10.5-1) | fuse`. **Annahme:** Das WSL-Image installiert Recommends; dann ist `fuse3` vorhanden. Kernel: FUSE ist im WSL2-Kernel fest eingebaut (Plan C2, Ref-FUSE 2.2.8; hier nicht erneut geprüft).

**Belege.**
- `https://github.com/microsoft/WSL/issues/13594` (Kommentare OneBlue 2025-10-14, 2025-10-15); `https://github.com/microsoft/WSL/issues/8423`; `https://github.com/microsoft/WSL/issues/7729` (OneBlue 2021-11-23: „That's what the explorer file access runs as.“).
- Lokal `apt-cache show ubuntu-wsl`, `apt-cache show snapd`.

**Folgerung für den Plan:** C2 „Sichtbarkeit im Windows-Explorer nicht zugesagt“ ist bestätigt; ein Einhängeort unter `/mnt/wsl/…` wäre der in den Issues genannte sichtbare Ort (nicht in C2 enthalten; Entscheidung beim Planverfasser).

**Verbleibende Lücke:** Microsoft-Learn-Dokumentation zu FUSE-Mounts und Mount-Namensräumen (keine gefunden); tatsächliche Paketliste des WSL-Ubuntu-24.04-Images.

### N17 — Vorschaubilder für einen Einhängepunkt gezielt unterdrücken

**Antwort.**
- GNOME/Nautilus: nur global. `org.gnome.nautilus.preferences show-image-thumbnails` (`always`, `local-only` = Standard, `never`); `org.gnome.desktop.thumbnailers disable-all` und `disable` (Liste von MIME-Typen). Kein Schlüssel je Pfad. GLib stuft als „remote“ nur `nfs`, `nfs4`, `cifs`, `smb`, `smb2`, `fuse.sshfs` ein; `fuse.smartexplorer` gilt als lokal → mit Standard `local-only` erzeugt Nautilus Vorschaubilder für se-Laufwerke.
- KDE/Dolphin: **Lücke** (KIO-Vorschau-Einstellungen und Dolphins ordnerbezogene Ansichtseinstellungen nicht gelesen).
- Windows: Richtlinien `DisableThumbnails` (alle Ordner), `DisableThumbnailsOnNetworkFolders` (nur Netzwerkordner), `DisableThumbsDBOnNetworkFolders`, `NoThumbnailCache` („Turn off caching of thumbnail pictures“, `Software\Microsoft\Windows\CurrentVersion\Policies\Explorer`); keine Richtlinie je Laufwerk oder Ordner. Ein Dokany-Laufwerk mit `OPTION_MOUNT_MANAGER` ohne Netzwerkmodus ist kein Netzwerkordner (**Annahme**, nicht geprüft, wie Explorer „Netzwerkordner“ bestimmt).

**Belege.**
- Nautilus `main` `data/org.gnome.nautilus.gschema.xml:3-7`, `:131-136`; gsettings-desktop-schemas `master` `schemas/org.gnome.desktop.thumbnailers.gschema.xml.in:4-16`; GLib `main` `gio/glocalfile.c` (`is_remote_fs_type`).
- `https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-admx-windowsexplorer` (`NoCacheThumbNailPictures` → Wert `NoThumbnailCache`); `policy-csp-admx-thumbnails` (über die Suchergebnisse und `github.com/MicrosoftDocs/windows-itpro-docs/.../policy-csp-admx-thumbnails.md`; englische Seite nicht direkt gelesen).

**Folgerung für den Plan:** R15 und die Grenzen in C6 sind bestätigt: Vorschaubild-Caches lassen sich für ein einzelnes se-Laufwerk weder unter GNOME noch unter Windows gezielt abschalten; nur globale Schalter.

**Verbleibende Lücke:** KDE; englischer Text der Seite `policy-csp-admx-thumbnails`; Explorer-Kriterium „Netzwerkordner“.

### N18 — Stabiles WinFsp 2.2; FLOSS-Ausnahme „link“; Bedingung 3

**Antwort.**
1. Stand 2026-10-17: kein stabiles WinFsp 2.2. Releases: `v2.2B5` (Pre-release, 2026-10-13, MSI `winfsp-2.2.26286.msi`), `v2.2B4` (2026-08-03), `v2.2B3` (2026-07-13), `v2.2B2` (2026-07-02), `v2.2B1` (2026-04-22); letzte stabile `v2.1` (2025-06-05). B5 nennt „Fixes vulnerability CVE-2026-3006“, „Fixes vulnerability CVE-2026-7162“ und „Fixes vulnerability reported by Luigino Camastra — AISLE Research“ (Commit `b8103265`, 2026-09-17, in B5 enthalten; NVD verweist für CVE-2026-93689 auf diesen Commit). Termin für ein stabiles 2.2: in keiner Quelle genannt.
2. NVD: CVE-2026-3006 (veröffentlicht 2026-04-27, „Awaiting Analysis“, CVSS 3.1 7.0, Wettlaufsituation → Kernel-Heap-Überlauf, lokale Rechteausweitung; Referenz `v2.2B1`); CVE-2026-7162 (2026-07-13, „Deferred“, CVSS 3.1 7.8, Ganzzahlüberlauf; Referenz `v2.2B2`); CVE-2026-93689 (2026-09-18, „Awaiting Analysis“, „WinFsp through 2.2.26215 contains a null pointer dereference … denial of service“, CVSS 3.1 5.5, CVSS 4.0 6.8) — **Dienstverweigerung, keine Rechteausweitung**.
3. Lizenz (`License.txt`): Ausnahme 1 „Permission to link with a platform specific version of the WinFsp DLL (one of: winfsp-a64.dll, winfsp-x64.dll, winfsp-x86.dll, winfsp-msil.dll)“; Ausnahme 2 „distribute unmodified binary releases of the WinFsp installer“; Bedingungen: (1) FLOSS-Lizenz, (2) Copyright-Hinweis „WinFsp - Windows File System Proxy, Copyright (C) Bill Zissimopoulos“ und Link zum Repository in Oberfläche und nutzerseitiger Doku, (3) „Is not linked or distributed with proprietary (non-FLOSS) software. [You cannot mix FLOSS and proprietary software while using WinFsp under this special exception.]“. Eine Auslegung von „link“ für Laufzeitladen (`LoadLibrary`) steht nicht im Text. Kommerzielle Lizenz: Kontakt `billziss at navimatics.com`.
4. Bedingung 3 für se: Der Windows-Installer liefert `smart_explorer`-EXE, Updater, `se`-CLI, `LICENSE`, Dokany-Lizenzdateien und die Dokany-MSI (`N/../installer.nsi:105-139`, `:196`); proprietäre Teile sind im Installer nicht aufgeführt. **Lücke:** Ob künftige Stränge proprietäre Binärdateien mitliefern (zum Beispiel Cisco-OpenH264-Binärdatei im Bildschirm-Strang, `nachrecherche-bildschirm.md`), ist hier nicht geprüft.

**Belege.** GitHub-API `repos/winfsp/winfsp/releases`, `releases/tags/v2.2B5`, `compare/v2.2B4...v2.2B5`, `commits/b8103265…`, `contents/License.txt`; NVD-API `services.nvd.nist.gov/rest/json/cves/2.0?cveId=…` für die drei CVEs.

**Folgerung für den Plan:** Auslöser T1 ist nicht erfüllt. Die Einordnung in C1 „CVE-2026-93689 bis 2.2.26215“ stimmt; als Rechteausweitung ist sie nicht eingestuft (die C1-Antwort nennt korrekt „zwei“ Rechteausweitungen). T3 bleibt offen; nur der Rechteinhaber kann „link“ auslegen.

**Verbleibende Lücke:** Termin WinFsp 2.2 stabil; Antwort des Rechteinhabers zu „link“.

### N19 — SMB: `supports_mounted_writes()` und Grund des Desktop-Ausschlusses

**Antwort.**
1. Ja. Das SMB-Backend liefert `StagedWriteCapabilities::complete()` (`create`, `replace`, `namespace_replace` alle wahr), also `supports_mounted_writes() == true`. Begründung im Code: „Replacing is one ReplaceIfExists rename and new files are created exclusively, so every staged-write guarantee holds. Drive mounts still refuse SMB explicitly (`daemon` mount source).“
2. Grund des Ausschlusses: `mount_source.rs` lehnt SMB ab mit dem Kommentar „SMB is browsed, copied and synced, but never mounted as a drive, whatever its staged-write guarantees report.“ und dem Text „SMB lässt sich nicht als Laufwerk einbinden“. Commit `7251ab33` (2026-09-26, „Add SMB2/3 connections to the Android app“) schreibt: „the desktop keeps UNC and mounting SMB is refused“. Ein **ursächlicher** Grund (etwa „wegen des UNC-Wegs“) steht weder im Commit noch im Code noch in `docs/`. Inzwischen bietet der Desktop-Dialog SMB auf allen Plattformen an („SMB überall, UNC nur Windows“, `docs/ARCHITEKTUR.md:35`).

**Belege.** `N/smb/core/backend.rs:410-420`; `N/vfs/core/capabilities.rs:22-32`; `N/daemon/os/shared/mount_source.rs:20-24`; `N/daemon/os/shared/ipc_protocol.rs:21-23`; `git show 7251ab33`; `docs/ARCHITEKTUR.md:35`.

**Folgerung für den Plan:** Die Begründung in C3 („weil der Desktop-Ausschluss mit dem Windows-UNC-Weg begründet ist (B2 Nr. 8)“) ist im Bestand nicht belegt. Die Fähigkeitsprüfung `supports_mounted_writes()` würde SMB-Roots auf Android schreibbar machen.

**Verbleibende Lücke:** Der Grund für „never mounted as a drive“ (nur der Verfasser von `7251ab33` könnte ihn nennen).

### N20 — Übersetzt AppFuse/`openProxyFileDescriptor` `O_APPEND`?

**Antwort.** Nein — der Modus wird vorher abgeschnitten. `StorageManagerService.openProxyFileDescriptor(mountId, fileId, mode)` führt `mode &= MODE_READ_WRITE;` aus („We only support a narrow set of incoming mode flags“); `MODE_APPEND` und `MODE_TRUNCATE` gehen damit verloren. `AppFuseBridge.openFile` wandelt den Rest mit `FileUtils.translateModePfdToPosix` in Flags für vold. Der Client-FD trägt also nie `O_APPEND`; Schreibzugriffe landen am Offset des FD (Start 0). `FuseAppLoop` reicht den vom Kernel gelieferten `offset` an `onWrite(offset, size, data)` weiter. Gleiche Zeile in Android 11, 14, 16 und `main`. Zusätzlich: libappfuse beantwortet jeden nicht behandelten Befehl (unter anderem `FUSE_SETATTR`, also `ftruncate`) über `HandleNotImpl`.

**Belege.**
- AOSP `frameworks/base` `main` `services/core/java/com/android/server/StorageManagerService.java:3788-3807`; `android11-release:3505`, `android14-release:3576`, `android16-release:3834` (je `mode &= MODE_READ_WRITE;`).
- `services/core/java/com/android/server/storage/AppFuseBridge.java:99-114`.
- `core/java/com/android/internal/os/FuseAppLoop.java:137-143` (behandelte Befehle), `:190-191` (`onWrite(offset, …)`).
- AOSP `system/core` `main` `libappfuse/FuseAppLoop.cc:96-126` (`default: buffer->HandleNotImpl()`), `:143-147` (ENOSYS wird zu `-EBADF` umgeschrieben, nur in `ReplySimple`).

**Folgerung für den Plan:** Die Ablehnung von „wa“ und „rwa“ in C3 ist nicht nur vorläufig, sondern durch die Plattform begründet: Ein Proxy-FD kann nicht anhängen. Ebenso bestätigt: kein `ftruncate` über den Proxy-FD; „t“ muss der Anbieter beim Öffnen ausführen.

**Verbleibende Lücke:** keine.

### N21 — Welche Desktop-Umgebungen führen `~/.config/autostart/*.desktop` aus (auch unter Wayland)?

**Antwort.**
- GNOME: `gnome-session` (`main`) liest `$XDG_CONFIG_HOME/autostart` und `<XDG_CONFIG_DIRS>/gnome/autostart` (`gsm-session-fill.c:37-44`) und wertet `X-GNOME-Autostart-enabled` und `X-GNOME-Autostart-Phase` aus (`gsm-app.c:35-36`). `gnome-session` startet die Sitzung unter X11 und Wayland.
- KDE Plasma: `plasma-workspace.target` hat `Wants=xdg-desktop-autostart.target`; systemd erzeugt die Dienste mit `systemd-xdg-autostart-generator` (beachtet `Hidden=`, `X-systemd-skip=`, `OnlyShowIn=`/`NotShowIn=`, `TryExec=`, `AutostartCondition=`, `X-GNOME-Autostart-Phase=`, `X-KDE-autostart-condition=`; **nicht** in seiner Tabelle: `X-GNOME-Autostart-enabled`).
- Xfce: `xfce4-session` `xfsm_startup_autostart` → `xfsm_launch_desktop_files_on_login` (`xfsm-startup.c:382-385`).
- Cinnamon: `cinnamon-session` verwaltet Autostart-Verzeichnisse (`csm-util.c:232-235`, Option `--autostart` in `main.c:285-288`).
- MATE: `mate-session` fügt Autostart-Programme über `gsm_manager_add_autostart_app` hinzu (`mate-session/main.c:202-230`).

**Belege.** GNOME `gnome-session` `main` `gnome-session/gsm-session-fill.c`, `gnome-session/gsm-app.c`; KDE `plasma-workspace` `master` `startkde/systemd/plasma-workspace.target:16,19`; lokale Manpage `systemd-xdg-autostart-generator(8)` (systemd von Ubuntu 24.04), systemd `main` `src/xdg-autostart-generator/xdg-autostart-service.c:332-336`; Xfce `xfce4-session` `master` `xfce4-session/xfsm-startup.c:382-385`; `linuxmint/cinnamon-session` `master` `cinnamon-session/main.c`, `cinnamon-session/csm-util.c`; `mate-desktop/mate-session-manager` `master` `mate-session/main.c`.

**Folgerung für den Plan:** Ein XDG-Autostart-Eintrag erreicht alle fünf genannten Umgebungen. Abschalten durch den Nutzer in den Desktop-Einstellungen setzt bei GNOME `X-GNOME-Autostart-enabled=false`, das `systemd-xdg-autostart-generator` (KDE-Weg) laut Tabelle nicht auswertet; der Bestand wertet beide Schlüssel als „vom System abgeschaltet“ aus (`N/autostart/os/linux_os.rs:69-90`).

**Verbleibende Lücke:** Xfce-Funktion `xfsm_launch_desktop_files_on_login` selbst (Datei nicht gelesen); Wayland-Sitzungen von Xfce, Cinnamon und MATE nicht einzeln geprüft (**Annahme:** dieselben Sitzungsmanager).

### N22 — Jobplaner „nur Laufwerke“ ohne `pause.until`

**Antwort.** Der Bestand trennt bereits: Der Daemon plant Sync-Jobs nur, wenn `crate::autostart::is_enabled()` wahr ist (`sync_enabled`); `poll_jobs`, `realtime.refresh`, `boot_marker::register_startup`, `connections.poll` und `controls.may_schedule(sync_enabled)` hängen daran. Unter Linux ist `is_enabled()` = „Datei `~/.config/autostart/smart-explorer-sync-daemon.desktop` existiert“ (`Exec=<exe> --sync-daemon`). Ein Daemon, der aus einem anderen Grund läuft (zum Beispiel `--drive-restore`), führt also keine Sync-Setups aus, solange der Nutzer den Hintergrund-Sync nicht eingeschaltet hat — **ohne** `pause.until` zu berühren. Zusätzlich gibt es `HostState::defer_scheduling` (Android), das nur das Einreihen geplanter Läufe zurückhält. Unabhängig von `sync_enabled` startet der Daemon immer den Share-Host (`ShareHost::new`, `reload_now`) und den IPC-Listener.

Randfall: Hat der Nutzer den Eintrag in den Desktop-Einstellungen abgeschaltet (`Hidden=true` bzw. `X-GNOME-Autostart-enabled=false`), existiert die Datei weiter; `is_enabled()` bleibt wahr (`disabled_by_system()` meldet es getrennt) — ein durch `--drive-restore` gestarteter Daemon würde dann Sync-Jobs planen.

**Belege.** `N/daemon/os/shared/run_loop.rs:117-200` (insbesondere `:118` `let mut sync_enabled = crate::autostart::is_enabled();`, `:131-149`, `:176-179`); `N/daemon/os/shared/state.rs:150-185` (`SchedulingControls`, `may_schedule`, `startup_pass`); `N/autostart/os/linux_os.rs:4`, `:64-66`, `:69-90`, `:104-109`; `N/daemon/os/shared/run_loop.rs:86-110` (Share-Host-Start).

**Folgerung für den Plan:** R13 ist im Kern widerlegt: Eine eigene Daemon-Rolle „nur Laufwerke“ ist für die Sync-Trennung nicht nötig. Offen bleiben zwei Punkte für den Planverfasser: (a) der Randfall „vom System abgeschaltet“; (b) dass ein `--drive-restore`-Start auch Share-Host und Freigaben startet.

**Verbleibende Lücke:** Ob Share-Freigaben beim Daemonstart von einer eigenen Einstellung abhängen (`ShareHost` nicht gelesen).

### N23 — Frist für „offline“ in Share

**Antwort.**
- Signierte Präsenz: `expires_at = now_secs() + 300` (300 s, `PRESENCE_LIFETIME`); LAN-Präsenz: `LAN_PRESENCE_TTL_SECS = 150`.
- Ereignis `DirectOffline` kommt vom Signalserver, wenn die Verbindung des Partners endet (`unpublish` → `Out::DirectOffline` an alle Beobachter). Der Server schließt eine nicht ruhende Verbindung nach `ACTIVE_READ_WINDOW` = 60 s ohne eingehende Daten; ruhende Clients (`idle_keepalive_v1`) erhalten alle K Sekunden (`DEFAULT_KEEPALIVE_SECS` = 180, Bereich 30–210) ein Keepalive und werden nach `IDLE_REPLY_WINDOW` = 60 s ohne Antwort getrennt. Ein Partner, der ohne Abmeldung verschwindet, erscheint damit nach höchstens etwa 60 s (aktiv) bzw. bis etwa 180 s + 60 s (ruhend) als offline; eine saubere Abmeldung meldet sofort.

**Belege.** `N/share/core/signal_presence.rs:188`; `N/share/core/signal_schedule.rs:25-40`; `N/share/core/lan_presence_match.rs:15`; `N/share/core/signal_auth.rs:58-59`; `share-server/src/direct_presence.rs:235-255`; `share-server/src/idle.rs:14-28`.

**Folgerung für den Plan:** Die **Annahme** in C5/R17 („60 s nach offline = doppelte Präsenz-Ablaufzeit“) trifft nicht zu: Die Präsenz-Ablaufzeit ist 300 s; die doppelte wäre 600 s. 60 s entspricht stattdessen `ACTIVE_READ_WINDOW`/`IDLE_REPLY_WINDOW` des Servers. Der Wert muss neu begründet werden.

**Verbleibende Lücke:** Fristen für Raumgeräte (`RoomRoster`, `RoomLeft`; nicht gelesen).

### N24 — GitHub-Runner: `/dev/fuse`, `fuse3`, Dokany-MSI und WinFsp-MSI

**Antwort.**
1. `ubuntu-24.04` (Image `20261012.334.1`, Kernel `6.17.0-1022-azure`): unprivilegiertes FUSE funktioniert. Beleg: Die eigene Suite `review-task.yml` installiert `sshfs` per `apt-get` (zieht `fuse3` nach) und `native/review-task-runtime.sh:81-82` hängt mit `sshfs` **ohne** `sudo` ein und prüft `mountpoint -q` unter `set -Eeuo pipefail`; Lauf `37175826251`, Job `linux` (`111358168079`, 2026-10-04) endete mit „success“. `fuse3` ist nicht in der Image-Liste, aber per `apt` installierbar.
2. Kernel 6.17 auf dem Runner: `max_pages_limit`- und `default_request_timeout`-Sysctls existieren dort (N4, Abschnitt 2 Zeile C2-Zeitgrenzen).
3. `windows-2025` (Image `20261013.288.2`): Dokany-MSI-Installation per `msiexec` ist in den Bestandsskripten umgesetzt (`native/test-windows-remote-task.ps1:115-119`, `native/test-mount-optimization-task.ps1:141-145`, `native/test-mount-batching-task.ps1:107-111`); letzte erfolgreiche Läufe: `windows-remote-task.yml` 2026-09-28, `mount-optimization-task.yml` 2026-09-08, `mount-batching-task.yml` 2026-09-05. WinFsp ist im Image nicht vorinstalliert (Image-Liste ohne Treffer). Gemeinsame Installation Dokany + WinFsp auf einem Runner: **nicht belegt**.

**Belege.** GitHub-API `actions/runner-images` `images/ubuntu/Ubuntu2404-Readme.md:10-11`, `images/windows/Windows2025-Readme.md:7-8`; `.github/workflows/review-task.yml:24,54`; `native/review-task-runtime.sh:3,37,81-82`; Run-Liste `actions/workflows/<datei>/runs`.

**Folgerung für den Plan:** R18 ist für `ubuntu-24.04` ausgeräumt (Bestandsbeleg). Das Image kündigt an: „`ubuntu-latest` label will use Ubuntu 26.04 in November 2026“ — die Suite sollte `ubuntu-24.04` fest nennen (wie der Bestand).

**Verbleibende Lücke:** Dokany und WinFsp nebeneinander auf `windows-2025` (für T2).

### N25 — Lizenzübersicht für Rust-Abhängigkeiten

**Antwort.** Nein. Der Installer legt nur `LICENSE` (MIT, se) und die Dokany-Dateien `licenses\Dokany\NOTICE.txt`, `LICENSE-GPL-3.0.txt`, `LICENSE-LGPL-3.0.txt`, `LICENSE-MIT.txt` ab; `README.md:925` nennt nur die se-Lizenz; `docs/RELEASING.md:112,154` erwähnt „license notices“ für Laufzeit-Staging (Dokany). Eine Übersicht der Rust-Crates (zum Beispiel `cargo about`, `THIRD_PARTY*`) gibt es im Repository nicht (`git ls-files` ohne Treffer außer Dokany-Dateien, `native/dokany-private/LICENSE.*`, `N/mount/os/windows/runtime_notices.rs`, `vendor/iroh-relay-1.0.0/LICENSE-BSD3`).

**Belege.** `native/installer.nsi:134-139`, `:325-331`; `README.md:925`; `docs/RELEASING.md:112,154`; `git ls-files | grep -i -E 'third.?party|licen[cs]e|NOTICE'`.

**Folgerung für den Plan:** C8 „neue Zeile in der Lizenzübersicht, falls das Projekt eine führt“ entfällt mangels Übersicht. Die MIT-Lizenz von `fuser` verlangt, den Copyright- und Lizenzhinweis „in all copies or substantial portions of the Software“ beizulegen; dieselbe Pflicht gilt heute schon für jede MIT- oder Apache-2.0-lizenzierte Crate in `native/Cargo.lock`, die in die ausgelieferten Binärdateien eingeht (Anzahl in dieser Runde nicht gezählt) — eine bestehende Lücke, nicht spezifisch für `fuser`.

**Verbleibende Lücke:** keine zur Frage.

### N26 — `fuser 0.18.0`: `Config`, `n_threads` mit `clone_fd`, Antworten aus Arbeiter-Threads

**Antwort.**
1. `Config` ist `#[derive(Debug, Clone, Default, Eq, PartialEq)]` und `#[non_exhaustive]` mit öffentlichen Feldern `mount_options: Vec<MountOption>`, `acl: SessionACL`, `n_threads: Option<usize>`, `clone_fd: bool`. Außerhalb der Crate ist ein Strukturliteral `Config { … }` **nicht** möglich; Weg: `let mut c = Config::default(); c.n_threads = Some(2); c.clone_fd = true; c.acl = SessionACL::Owner;`. `n_threads` ist `Option<usize>` (Standard: 1 Thread).
2. `n_threads > 1` nur unter Linux; je zusätzlichem Thread mit `clone_fd` ein neues `/dev/fuse` mit `FUSE_DEV_IOC_CLONE` (sonst geteilter FD). Jeder Thread läuft eine eigene Leseschleife `fuser-<i>`.
3. Antworten: `Reply: Send + 'static`; die Antwort schreibt per `writev` auf den FD des Kanals, der die Anfrage empfangen hat (`RequestWithSender::new(self.ch.sender(), …)`); das ist für `clone_fd` nötig, weil der Kernel Antworten in der Verarbeitungsliste des empfangenden FD sucht (Kernel master `fs/fuse/dev.c:1894` `struct fuse_pqueue *fpq = &fud->pq;`, `:1938` `fuse_request_find(fpq, …)`). Eine Antwort darf daher aus einem anderen Thread kommen. Die Leseschleife ruft die `Filesystem`-Methode synchron auf; kehrt sie sofort zurück (Antwort an einen Pool übergeben), blockiert die Schleife nicht. Fällt ein `Reply` ohne Antwort weg, sendet `Drop` `EIO`.
4. `Filesystem: Send + Sync + 'static`; Methoden mit `&self`, außer `init(&mut self, …)` und `destroy`.
5. `Notifier::inval_entry(parent, name)` und `Notifier::inval_inode(ino, offset, len)` vorhanden; `Session::notifier()` bzw. `BackgroundSession::notifier()`.
6. `AutoUnmount` mit `acl == Owner` → `InvalidInput` „auto_unmount requires acl != Owner“.
7. Pflege: README „Version 0.18.0 is the last version that was primarily developed without coding agents.“, „Future releases will be developed primarily by a coding agent“, „**Pull requests are no longer being accepted.**“. Seit 0.18.0 (2026-07-22) 58 Commits auf `master` ohne neues Release (zum Beispiel `Notifier::inc_epoch`, `Filesystem::statx`, `syncfs`, `tmpfile`, „Refuse capabilities fuser cannot honor“ #719). RustSec: `RUSTSEC-2021-0154` betrifft `fuser::Session::new` ≥ 0.5.0 mit libfuse, behoben ab 0.16.0 → 0.18.0 nicht betroffen.

**Belege.** `fuser-0.18.0/src/mnt/mount_options.rs:7-21`; `src/session.rs:55-75`, `:154-175`, `:250-300`, `:493-495`, `:526-540`, `:579-581`; `src/channel.rs:53-56`, `:67-79`, `:82-88`; `src/reply.rs:104`, `:150-160`; `src/notify.rs:53`, `:72-84`; `src/lib.rs:400-406`; `README.md:20-24`, `:132`; GitHub-API `repos/cberner/fuser/commits?since=2026-07-22` und `pulls/719`; `github.com/rustsec/advisory-db` `crates/fuser/RUSTSEC-2021-0154.md`.

**Folgerung für den Plan:** Die Schreibweise in C2 „`Config { n_threads: 2, clone_fd: true, acl: SessionACL::Owner }`“ ist so nicht kompilierbar (nicht erschöpfend, `n_threads` ist `Option`); die Absicht ist über `Default` plus Feldzuweisung umsetzbar. Threading-Modell (zwei Leseschleifen, Antwort aus Pool) ist bestätigt.

**Verbleibende Lücke:** keine.

### N27 — Explorer-Meldung bei `ERROR_GEN_FAILURE` und Alternativen

**Antwort.**
1. Systemtexte (Microsoft Learn, englisch): `ERROR_GEN_FAILURE` 31 „A device attached to the system is not functioning.“; `ERROR_NOT_READY` 21 „The device is not ready.“; `ERROR_BAD_NETPATH` 53 „The network path was not found.“; `ERROR_HOST_UNREACHABLE` 1232 und `ERROR_NETWORK_UNREACHABLE` 1231 „The network location cannot be reached.“; `ERROR_ACCESS_DENIED` 5 „Access is denied.“; `ERROR_INVALID_DATA` 13 „The data is invalid.“
2. Weg im Bestand: `io_status` bildet `HostUnreachable` (und `ConnectionRefused`, `ConnectionReset`, `NotConnected`, `AddrInUse`, `AddrNotAvailable`, `NetworkDown`, `NetworkUnreachable`) auf `ERROR_GEN_FAILURE` ab und ruft `DokanNtStatusFromWin32` auf. Dokany 2.3.1.1000 (`ntstatus.i`): `ERROR_GEN_FAILURE` → `STATUS_UNSUCCESSFUL`; `ERROR_NOT_READY` → `STATUS_DEVICE_NOT_READY`; `ERROR_BAD_NETPATH` → `STATUS_BAD_NETWORK_PATH`; `ERROR_HOST_UNREACHABLE` → `STATUS_HOST_UNREACHABLE`; `ERROR_NETWORK_UNREACHABLE` → `STATUS_NETWORK_UNREACHABLE`; `ERROR_NOT_SAME_DEVICE` → `STATUS_NOT_SAME_DEVICE`; `ERROR_SEM_TIMEOUT` → `STATUS_IO_TIMEOUT`. Unbekannte Codes → `STATUS_ACCESS_DENIED` (`ntstatus.c` `default`).
3. **Zusatzbefund (Bestandsfehler, außerhalb der Frage):** Der Rückfall `_ => ERROR_INVALID_DATA` in `io_status` steht **nicht** in Dokanys Tabelle (`ERROR_INVALID_DATA` fehlt in `ntstatus.i`); `DokanNtStatusFromWin32(ERROR_INVALID_DATA)` liefert daher `STATUS_ACCESS_DENIED`. Jede nicht einzeln abgebildete `io::ErrorKind` erscheint im Explorer als „Zugriff verweigert“, nicht als „Daten ungültig“. Die private DLL baut auf Commit `f1d5de68` (= v2.3.1.1000) und ändert `ntstatus` nicht (`native/dokany-private/recipe.json:3-6`, `batching.patch` ohne `ntstatus`).
4. Welche Meldung der Explorer genau anzeigt (Dialogtext um den Systemtext): **Lücke** — keine Primärquelle; die Rückabbildung NTSTATUS → Win32 durch `RtlNtStatusToDosError` ist **Annahme** (gleicher Code wie vor der Abbildung).

**Belege.** `https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-` und `…--1000-1299-`; `N/mount/os/windows/callback_status.rs:215-247`; `N/mount/os/windows/runtime.rs:275-277`; Dokany `v2.3.1.1000` `dokan/ntstatus.c:25-31`, `dokan/ntstatus.i:31,129,219,224,304,322,324,409`.

**Folgerung für den Plan:** Für den Gerätebaum wäre `ERROR_HOST_UNREACHABLE` („The network location cannot be reached.“) inhaltlich näher als `ERROR_GEN_FAILURE`; alle drei Kandidaten haben eine Dokany-Abbildung. Der Rückfall `ERROR_INVALID_DATA` → `STATUS_ACCESS_DENIED` betrifft bestehende Laufwerke (Bestandsfehler; Meldung an den Planverfasser).

**Verbleibende Lücke:** Explorer-Dialogtext; deutsche Systemtexte (nicht abgerufen).

---

## 2. Revalidierung Abschnitt C

Ergebnis je Aussage: **bestätigt**, **widerlegt** (mit richtiger Angabe) oder **nicht belegbar** (mit genauer Lücke). „Stelle“ nennt den Unterabschnitt von `strang-laufwerke.md` Abschnitt C und das Merkmal bzw. den Absatz.

### 2.1 C1 — Windows-Frontend

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe / Lücke |
|---|---|---|---|---|
| C1-1 | Dokany letzter Release 2025-09-28 (v2.3.1.1000), 2026 kein Release (Tabelle „Pflege“) | bestätigt | GitHub-API `dokan-dev/dokany/releases`: `v2.3.1.1000` 2025-09-28, davor `v2.3.0.1000` 2025-04-19 | — |
| C1-2 | 7 unveröffentlichte Commits (Tabelle „Pflege“) | bestätigt | Commits nach `f1d5de68` („Release 2.3.1.1000“): `507f864b`, `ead52a97`, `847cc487`, `d5f1c72f`, `f8a542a1`, `767da4ba`, `c7a59fc6` | — |
| C1-3 | BSOD-Korrektur `f8a542a1` vom 2026-03-22 unveröffentlicht (Tabelle „Sicherheitslage“) | bestätigt | Commit `f8a542a1` 2026-03-22 „Driver - Fix IRP cancellation race condition and IRQL BSOD“, nach dem letzten Release | — |
| C1-4 | #1335 offen, #1339 offen (Tabelle „Sicherheitslage“) | bestätigt | #1335 `open` (2026-07-22), #1339 `open` (2026-09-29) | „auch auf `master`“ nicht erneut geprüft (Quelltextvergleich nicht durchgeführt) |
| C1-5 | Treiberzertifikat „next 3 years“ ab 2023-12 (Tabelle „Pflege“) | bestätigt (Zitat), Ablaufdatum nicht belegbar | Release-Text `v2.1.0.1000`, veröffentlicht 2023-12-22: „new certificate valid for the next 3 years“ | Lücke G1 unverändert: Ablaufdatum und Signaturart von `dokan2.sys` |
| C1-6 | WinFsp stabil v2.1 betroffen von CVE-2026-3006 (CVSS 7.0, lokale Rechteausweitung) und CVE-2026-7162 (CVSS 7.8) | bestätigt | NVD: 3006 CVSS 3.1 7.0 `AV:L/AC:H/PR:L`, 7162 CVSS 3.1 7.8 `S:C`; Referenzen `v2.2B1`/`v2.2B2` | — |
| C1-7 | CVE-2026-93689 bis 2.2.26215 | bestätigt, mit Ergänzung | NVD 2026-09-18: „WinFsp through 2.2.26215 … null pointer dereference … denial of service“, CVSS 3.1 5.5 | Art: Dienstverweigerung (keine Rechteausweitung); Korrektur `b8103265` ist in `v2.2B5` |
| C1-8 | Korrekturen nur in Pre-Releases v2.2B1–B5; Termin stabil unbekannt; 2026 fünf Betas | bestätigt | `v2.2B1` 2026-04-22 … `v2.2B5` 2026-10-13, alle `prerelease=true`; stabil zuletzt `v2.1` 2025-06-05 | — |
| C1-9 | WinFsp GPLv3 mit FLOSS-Ausnahme, Hinweis in Oberfläche und Doku, keine Mischung mit proprietärer Software | bestätigt | `License.txt` (Bedingungen 1–3) | Auslegung „link“ für Laufzeitladen steht nicht im Text (G9 bleibt) |
| C1-10 | Crate `winfsp` GPL-3.0 ohne Ausnahme; `winfsp_wrs` MIT | bestätigt | crates.io: `winfsp 0.13.1+winfsp-2.1` GPL-3.0; `winfsp_wrs 0.4.1` MIT | — |
| C1-11 | `cloud-filter` MIT; `windows-projfs` GPL-2.0; `projfs` MIT | bestätigt | crates.io: `cloud-filter 0.0.6` MIT, `windows-projfs 0.1.8` GPL-2.0, `projfs 0.1.2` MIT | — |
| C1-12 | WinFsp-MSI perMachine, Dienst `WinFsp.Launcher` Start „auto“ | bestätigt | `build/VStudio/installer/Product.wxs` (Tag `v2.1`) `:20` `InstallScope="perMachine"`, `:197`, `:228` `Start="auto"` | Dienstname nicht im gelesenen Ausschnitt geprüft |
| C1-13 | `DefineDosDevice`-Laufwerk: `GetFinalPathNameByHandleW` schlägt fehl (#410, #690, `edit.exe`) | bestätigt | N3 | — |
| C1-14 | `OPTION_MOUNT_MANAGER` aus dem nicht erhöhten Host wirksam (Annahme G3) | nicht belegbar | — | Lücke G3 unverändert (in dieser Runde nicht geprüft) |
| C1-15 | Ordner nur „existing empty folder“; #1337 Zugriff verweigert ohne Mount Manager | bestätigt (#1337) | #1337 `open`, Titel „mirror.exe - Access denied if mounted without mount manager“ | „existing empty folder“ nicht erneut geprüft |
| C1-16 | Netzwerkmodus-Fehler #1326, #1305; #1320 (25 000 Unterordner), #1341 (Auswerfen bei hängendem Fenster) | bestätigt (Zustand) | alle vier `open`; #1341 Titel „DokanCloseHandle takes 5 or 10 seconds when any top-level window is hung“ | — |
| C1-17 | WinFsp-Zeitlimit beendet nur die Anfrage mit `STATUS_CANCELLED` | nicht belegbar | — | in dieser Runde nicht nachgelesen (WinFsp `src/sys/`) |
| C1-18 | Lebenszyklussperre `MountManager::lifecycle` (`mount_manager.rs:31`) | bestätigt | `N/daemon/os/shared/mount_manager.rs:31`, `:99`, `:114` | — |
| C1-19 | Annahme: Electron-Apps lösen Pfade über libuv mit `GetFinalPathNameByHandleW` auf (Grund 3) | teilweise bestätigt | N3: `fs.promises.realpath`, `fs.realpath(.Sync).native` → `uv_fs_realpath` → `GetFinalPathNameByHandleW` | `fs.realpath` ohne `.native` nutzt die Funktion nicht; Aufrufstellen in VS Code/Obsidian nicht belegt |

### 2.2 C2 — Linux-Laufwerke

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe / Lücke |
|---|---|---|---|---|
| C2-1 | `fuser 0.18.0`: MIT, reiner Rust-Modus, `&self`, `Send + Sync`, `n_threads` mit `clone_fd`, Antworten aus beliebigem Thread, `Notifier::inval_inode`/`inval_entry`, MSRV 1.85 (L1) | bestätigt | N26; crates.io `0.18.0` 2026-07-22 MIT `rust_version 1.85`; `build.rs:10-15` (`pure-rust` für Linux ohne Feature `libfuse`) | `init` ist `&mut self` |
| C2-2 | `fuse3 0.9.0` MSRV 1.91 (L2) | bestätigt | crates.io `fuse3 0.9.0` (2026-03-12) MIT `rust_version 1.91` | Quelltext-TODO in `inode_path_bridge.rs` nicht erneut gelesen |
| C2-3 | `polyfuse 0.5.1` hängt fest über `/usr/bin/fusermount` aus; Fedora- und Arch-`fuse3` liefern es nicht (L3) | bestätigt, mit Ergänzung | `polyfuse-0.5.1/src/conn.rs:13` (`FUSERMOUNT_PROG = "/usr/bin/fusermount"`), `:295-296` (`unmount` nutzt die Konstante); Arch `extra/fuse3` Dateiliste nur `usr/bin/fusermount3`; Fedora `fuse3.spec:110` nur `fusermount3` | Einhängen ist über die verborgene Methode `fusermount_path()` (`session.rs:96-104`) änderbar, Aushängen nicht |
| C2-4 | `easy_fuser 0.7.1` (L4) | bestätigt (Version) | crates.io `0.7.1` 2026-09-30 | Funktionsumfang (kein Notifier, kein Interrupt, `acl` fest) nicht erneut geprüft |
| C2-5 | libfuse „does not have any active, regular contributors“; SONAME-Wechsel `.so.3` → `.so.4` (L5) | bestätigt | libfuse `fuse-3.18.3` `README.md:32`; Debian trixie Paket `libfuse3-4` (Abhängigkeit von `xdg-desktop-portal 1.20.3+ds-1`) | — |
| C2-6 | Android ist nicht `target_os = "linux"`; `fuser/build.rs` hat für Android keinen Zweig ohne pkg-config | bestätigt | `docs/ARCHITEKTUR.md:72`; `fuser-0.18.0/build.rs:7-46` (Android fällt in den `else`-Zweig mit `pkg_config` für `fuse3`/`fuse`, sonst `panic!`) | — |
| C2-7 | `Config { n_threads: 2, clone_fd: true, acl: SessionACL::Owner }` (Threading) | widerlegt (Schreibweise) | `fuser-0.18.0/src/mnt/mount_options.rs:7-21` (`#[non_exhaustive]`, `n_threads: Option<usize>`) | `let mut c = Config::default(); c.n_threads = Some(2); c.clone_fd = true; c.acl = SessionACL::Owner;` |
| C2-8 | `max_background` fuser-Standard 16 | bestätigt | `fuser-0.18.0/src/lib.rs:229` | — |
| C2-9 | `fuser 0.18` antwortet `ENOSYS` auf INTERRUPT; Kernel stellt danach keine Interrupts zu (Annahme) | bestätigt | N1 | — |
| C2-10 | `se drive unmount --force` über `/sys/fs/fuse/connections/<n>/abort` | bestätigt | N2 | `<n>` = `fc->dev` als `%u` (`fs/fuse/control.c:250-256`, `fs/fuse/inode.c:1760`); für FUSE (Major 0) gleich `minor(st_dev)` des Einhängepunkts (**Folgerung** aus der Kernel-Kodierung `MKDEV`, nicht beobachtet) |
| C2-11 | keine Kernel-Anfragefrist (`FUSE_REQUEST_TIMEOUT` lehnt `fuser` auf `master` ab) | bestätigt mit Einschränkung | `fuser` PR #719 (master, nach 0.18.0): Bit 42 abgelehnt; in 0.18.0 sendet `fuse_init_out` dort Nullen → `fuse_init_server_timeout(fc, 0)` | Kernel ≥ Commit `9b17cb59` (2025-03-31) hat die Sysctls `/proc/sys/fs/fuse/default_request_timeout` und `max_request_timeout` (Standard 0); setzt ein Administrator sie, bricht der Kernel die **Verbindung** ab, wenn eine Anfrage länger dauert (`fs/fuse/req_timeout.c:100-142`), auch bei `fuser`. Runner-Kernel 6.17 hat diese Sysctls |
| C2-12 | Eintrags-/Attribut-TTL 1 s, negative TTL 0 s (libfuse-Standard) | bestätigt | libfuse `fuse-3.18.3` `lib/fuse.c:5010-5011` (`entry_timeout = 1.0`, `attr_timeout = 1.0`); `negative_timeout` nicht gesetzt | Nullinitialisierung von `negative_timeout` ist **Annahme** (Struktur-Initialisierung nicht nachgelesen) |
| C2-13 | kein `writeback_cache` (Kernel-Doku „not suitable for network filesystems“) | bestätigt | Kernel master `Documentation/filesystems/fuse/fuse-io.rst:40-42` („it's generally not suitable for network filesystems“) | Wortlaut im Plan verkürzt („generally“ fehlt) |
| C2-14 | `max_write` 1 MiB (Annahme: `max_pages_limit` 256) | bestätigt | N4 | — |
| C2-15 | GLib zeigt Mounts unter `$HOME/` ohne „/.“ automatisch in Nautilus, Nemo, Thunar | bestätigt (Quelltext) | N8 | Anzeigename = letzter Pfadteil |
| C2-16 | Eigener Ort: „leerer Ordner im Besitz des Nutzers, kein Symlink, nicht in einem fremden Sticky-Verzeichnis (fusermount3-Regel)“ | teilweise widerlegt | `fusermount.c:1113-1125` (`O_NOFOLLOW`), `:1171-1183` (Sticky-Prüfung **am Einhängepunkt selbst**, Schreibrecht), `:985` (`nonempty` wird nur noch angenommen) | `fusermount3` verlangt: kein Symlink, Schreibrecht, und falls der **Einhängepunkt selbst** das Sticky-Bit trägt, Besitz durch den Nutzer. „Leer“ und „im Besitz des Nutzers“ sind se-Regeln, nicht `fusermount3`-Regeln |
| C2-17 | `fuser` verlangt für `auto_unmount` `acl != Owner`; `user_allow_other` auskommentiert; mount.fuse3(8) „dangerous“ | bestätigt | `fuser-0.18.0/src/session.rs:163-172`; libfuse `fuse-3.18.3` `util/fuse.conf:10` (`#user_allow_other`), lokal Ubuntu `/etc/fuse.conf:10`; `doc/mount.fuse3.8:142-150` („This option is dangerous“) | Debian- und Fedora-Dateien nicht einzeln geprüft (Ubuntu-Paket `fuse3` stammt von Debian) |
| C2-18 | Einhängepunkt `0500` außerhalb des Mounts, `0700` vor dem Einhängen (fusermount3 verlangt Schreibrecht) | bestätigt | N5 | — |
| C2-19 | Dolphin: Solid-fstab-Backend zeigt unbekannte FUSE-Untertypen nicht | nicht belegbar | — | Solid-Quelltext in dieser Runde nicht gelesen |
| C2-20 | XDG-Autostart-Eintrag erreicht die Desktop-Umgebungen | bestätigt | N21 | `X-GNOME-Autostart-enabled` wertet `systemd-xdg-autostart-generator` (KDE-Weg) nicht aus |
| C2-21 | `--drive-restore` braucht Daemon-Rolle „nur Laufwerke“ (N22) | widerlegt | N22: Sync-Planung hängt bereits an `autostart::is_enabled()` | Randfall „vom System abgeschaltet“ und Share-Host-Start bleiben |
| C2-22 | `fusermount3` mit Setuid auf Desktop-Standardinstallationen | bestätigt | N9; Fedora `fuse3.spec:110` `%attr(4755,root,root)`; lokal `-rwsrwxr-x … /usr/bin/fusermount3` | Arch nur bei installierten Gruppen |
| C2-23 | WSL2: Sichtbarkeit im Windows-Explorer nicht zugesagt | bestätigt | N16 | Unter `/mnt/wsl` laut WSL-Mitarbeiter sichtbar |
| C2-24 | Flatpak/Snap/Container-Grenzen (bubblewrap, `fuse-support`) | nicht belegbar | — | in dieser Runde nicht nachgeprüft (Ref-FUSE 2.2.5–2.2.6) |

### 2.3 C3 — Android-Dokumentanbieter

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe / Lücke |
|---|---|---|---|---|
| C3-1 | Manifest: `exported="true"`, `grantUriPermissions="true"`, `permission="android.permission.MANAGE_DOCUMENTS"`, Filter `android.content.action.DOCUMENTS_PROVIDER` | bestätigt | `developer.android.com/guide/topics/providers/create-document-provider` (Stand 2025-02-10) | — |
| C3-2 | Gesperrter Tresor → keine Root („return zero roots“), `notifyChange(buildRootsUri)` | bestätigt | dieselbe Seite: „The solution is to return zero roots in your implementation of `queryRoots()`.“ | — |
| C3-3 | `FLAG_LOCAL_ONLY` wahrheitsgemäß (Ref-Android A3) | nicht belegbar | Leitfadenseite nennt `FLAG_LOCAL_ONLY` nicht | `DocumentsContract.Root`-Referenz nicht gelesen |
| C3-4 | SMB auf Android angeboten, „weil der Desktop-Ausschluss mit dem Windows-UNC-Weg begründet ist (B2 Nr. 8)“ | nicht belegbar (Begründung) | N19 | Bestand nennt keinen Grund; `supports_mounted_writes()` ist für SMB wahr |
| C3-5 | „t“ kürzt beim Öffnen (Proxy-FD kennt kein `ftruncate`) | bestätigt | N20: `FuseAppLoop` behandelt nur `LOOKUP`, `GETATTR`, `OPEN`, `READ`, `WRITE`, `RELEASE`, `FSYNC`; libappfuse `HandleNotImpl` für jeden nicht aufgeführten Opcode, darunter `FUSE_SETATTR` | — |
| C3-6 | „wa“/„rwa“ abgelehnt, bis N20 belegt, dass AppFuse Anhängen übersetzt | bestätigt, Grund präzisiert | N20: `mode &= MODE_READ_WRITE` entfernt `MODE_APPEND` | Ablehnung ist dauerhaft plattformbedingt |
| C3-7 | Eigener `HandlerThread` je Datei wegen vold-Sperre/Watchdog (N11) | bestätigt (Mechanismus) | N11 | Watchdog 60 s; ab Android 15 zusätzlich 30 s Pause |
| C3-8 | `ContentProvider.onCreate` vor `Application.onCreate` (Annahme) | bestätigt | N10 | — |
| C3-9 | DocumentsUI wartet beim ersten Laden 5 s | bestätigt | DocumentsUI `main` `src/com/android/documentsui/roots/ProvidersCache.java:95` (`FIRST_LOAD_TIMEOUT_MS = 5000`), `:257`, `:577` (später 15 000 ms) | — |
| C3-10 | Offline → `EXTRA_ERROR` sichtbar, Laden → `EXTRA_LOADING` | bestätigt | N15 | — |
| C3-11 | Galerie-Apps über MediaStore zeigen die Inhalte nicht (Annahme) | nicht belegbar | N12 | Keine ausdrückliche Negativaussage gefunden |
| C3-12 | Hintergrund-Veröffentlichung nach `onRelease` über expedited WorkManager | bestätigt mit Auflage | N14 | Wegen minSdk 30 muss der Worker `getForegroundInfo()` umsetzen; Vordergrunddienst aus dem Hintergrund nur mit abgeschalteter Akkuoptimierung |
| C3-13 | Pipes (`createReliablePipe`) erlauben nur „r“ oder „w“, kein „rw“ mit Seek | nicht belegbar | — | Javadoc `openDocument` in dieser Runde nicht gelesen |

### 2.4 C4 — Gemeinsame Frontend-Grenze

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe / Lücke |
|---|---|---|---|---|
| C4-1 | `flock(LOCK_EX \| LOCK_NB)` für Linux und Android nach dem Muster `N/daemon/mod.rs:116-117` | bestätigt | `N/daemon/mod.rs:114-116` (`#[cfg(any(target_os = "linux", target_os = "android"))]`); N13 | Zeilenangabe im Plan um zwei Zeilen verschoben (`:114-116`) |
| C4-2 | Fehlerabbildung FUSE nach `ErrorKind` (Tabelle) | — (Entwurf, keine Tatsachenaussage) | — | Bestand Windows: `WouldBlock` → `ERROR_SHARING_VIOLATION`, Rückfall `ERROR_INVALID_DATA` → in Dokany `STATUS_ACCESS_DENIED` (N27) |
| C4-3 | `setattr` mit `ENOTSUP` für `chmod`/`utimensat` (über R5, DRV-M4) | Grundlage geändert | N6 | `cp -a` und `rsync -a` scheitern an `ENOTSUP` |

### 2.5 C5 — Gerätebaum

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe / Lücke |
|---|---|---|---|---|
| C5-1 | `open_share` mit 45-s-Transportfenster (Bestand) | bestätigt | `N/daemon/os/shared/ipc_host.rs:355-359` (`Duration::from_secs(45)`) | — |
| C5-2 | `HostUnreachable` → Windows `ERROR_GEN_FAILURE` aus der unveränderten Tabelle | bestätigt | `N/mount/os/windows/callback_status.rs:238-244` | Dokany → `STATUS_UNSUCCESSFUL` |
| C5-3 | „einzige Fundstelle“ für `CrossesDevices`: `N/cli/transfer.rs:148`; kein Mount-Pfad erzeugt diese Art | widerlegt (Aufzählung), Kernaussage bestätigt | `N/cli/transfer.rs:148`, `N/copy/os/linux_os.rs:91`, `N/copy/os/windows.rs:140`, `N/apptrash/os/shared/store.rs:71` | Vier Fundstellen, alle prüfen nur empfangene Fehler; unter `N/mount/` keine |
| C5-4 | 60 s nach „offline“ = doppelte Präsenz-Ablaufzeit (Annahme) | widerlegt | N23 | Präsenz-Ablaufzeit 300 s; 60 s = Server-Lesefenster |
| C5-5 | Bezeichnung „Smart Explorer Geräte“ (21 Zeichen), Grenze 32 | bestätigt | `N/mount/os/windows/callbacks_metadata.rs:130` (`write_wide(…, label, Some(32))`) | Die 32 ist die Grenze im se-Code; Dokany-Grenze selbst nicht geprüft |
| C5-6 | Linux-Gerätebaum `~/SmartExplorer/Geräte` sichtbar | bestätigt (Quelltext) | N8 | Anzeigename „Geräte“; mögliche Doppelanzeige mit GTK-Lesezeichen |

### 2.6 C6 — Klartextgrenze, Krypto-Crates

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe / Lücke |
|---|---|---|---|---|
| C6-1 | XChaCha20-Poly1305 aus `chacha20poly1305 0.10.1` (vorhanden) | bestätigt | `native/Cargo.toml:93` (`version = "0.10.1", default-features = false, features = ["alloc"]`), `Cargo.lock` `chacha20poly1305 0.10.1`; `chacha20poly1305-0.10.1/src/lib.rs:173` (`XNonce` 24 Byte), `:184` (`XChaCha20Poly1305`, ohne `cfg`-Schranke) | Neueste Version `0.11.0` (2026-06-28, MSRV 1.85); keine RustSec-Meldung für `chacha20poly1305` (Verzeichnis `crates/chacha20poly1305` fehlt in `rustsec/advisory-db`). Zufalls-Nonce: Feature `getrandom` ist abgeschaltet; `getrandom 0.2` ist direkte Abhängigkeit (`Cargo.toml:99`) |
| C6-2 | `HKDF-SHA256(ikm, salt, info)` mit `hkdf` | bestätigt | `native/Cargo.toml:94` (`hkdf = "0.12.4"`); `hkdf-0.12.4/src/lib.rs:199` (`new(salt: Option<&[u8]>, ikm: &[u8])`), `:269` (`expand(&self, info, okm) -> Result<(), InvalidLength>`) | Neueste `0.13.0` (2026-03-30) liegt zusätzlich im `Cargo.lock` (transitiv); keine RustSec-Meldung für `hkdf` |
| C6-3 | Vorschaubild-Caches außerhalb der Kontrolle von se (N17) | bestätigt | N17 | KDE nicht geprüft |

### 2.7 C8 — Installer, Release, Rechte

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe / Lücke |
|---|---|---|---|---|
| C8-1 | Dokany-MSI 2.3.1.1000, private DLL, Lizenzdateien `native/dokany-runtime.nsh`, `installer.nsi:135-139` | bestätigt | `native/installer.nsi:134-139`, `:196`; `native/dokany-private/recipe.json:3-6` (Quelle `f1d5de68`) | — |
| C8-2 | Kein `user_allow_other` nötig (kein `allow_other`, kein `auto_unmount`) | bestätigt | C2-17 | — |
| C8-3 | Lizenzübersicht „falls das Projekt eine führt“ (N25) | widerlegt (Voraussetzung) | N25 | Keine Übersicht vorhanden |

---

## 3. Folgen für Entscheidungen

Nur Entscheidungen, deren Grundlage sich durch diese Nachrecherche geändert hat. Keine Neubewertung; je Punkt die geänderte Tatsache und die betroffene Stelle.

1. **C4/DRV-M4 `setattr`, Risiko R5 (N6).** Die Grundlage „Speichern aus einzelnen Editoren meldet Fehler (falls)“ ist überholt: GIO, vim, Kate und KIO speichern trotz `ENOTSUP`; **`cp -a` und `rsync -a` enden dagegen mit Fehler** bei `ENOTSUP` auf `utimensat`, `fchmod` und (bei `cp`) `fchown` (nur `EPERM`/`EINVAL`/`EACCES` toleriert `cp` für `chown`). Die in R5 aufgeschobene Entscheidung („Entscheidung nach N6“) ist jetzt fällig.
2. **C2 Zeitgrenzen (C2-11).** „Keine Kernel-Anfragefrist“ gilt nur bei den Standardwerten 0 der Sysctls `default_request_timeout`/`max_request_timeout`. Setzt ein Administrator sie, bricht der Kernel die ganze Verbindung ab, wenn zum Beispiel ein langes Hochladen in `flush` die Frist überschreitet. Betrifft die Aussage „Damit blockiert ein langes Hochladen in `flush` keine Verzeichnisanfrage“ nicht, aber die Grenzanzeige (`se drive runtime`) und R3.
3. **C2 Threading (C2-7).** Die `Config`-Schreibweise ist in `fuser 0.18.0` nicht möglich; Umsetzung über `Config::default()` plus Feldzuweisung, `n_threads: Some(2)`.
4. **C2 Einhängeort (C2-16).** Die Regeln „leer“ und „im Besitz des Nutzers“ sind keine `fusermount3`-Regeln; die Sticky-Prüfung gilt dem Einhängepunkt selbst, nicht dem Elternordner. Die Begründung im Plan („fusermount3-Regel“) trifft für diese Teile nicht zu.
5. **C2 Wiederherstellung, R13, DRV-M3 (N22).** Eine Daemon-Rolle „nur Laufwerke“ ist für die Trennung vom Sync nicht nötig: Der Daemon plant Sync-Jobs nur bei `autostart::is_enabled()`. Neu zu entscheiden sind (a) der Randfall „Eintrag existiert, vom Desktop abgeschaltet“ (`is_enabled()` wahr) und (b) dass jeder Daemonstart auch den Share-Host startet.
6. **C2 Grenzen WSL2 (N16).** Ein Einhängeort unter `/mnt/wsl/…` ist laut WSL-Mitarbeiter im Explorer sichtbar; der Plan sagt die Sichtbarkeit nur ab.
7. **C2/C5 Sichtbarkeit Linux (N8).** Nautilus zeigt den Mount im Abschnitt „Mounts“ unter dem letzten Pfadteil; das zusätzliche GTK-Lesezeichen kann einen zweiten Eintrag erzeugen (Quelltextfolgerung).
8. **C3 SMB-Roots (N19).** Die Begründung „Desktop-Ausschluss wegen Windows-UNC-Weg“ ist im Bestand nicht belegt; der Code lehnt SMB-Laufwerke ohne genannten Grund ab („whatever its staged-write guarantees report“), obwohl SMB alle Staged-Write-Garantien meldet.
9. **C3 `openDocument`-Modi (N20).** Die Ablehnung von „wa“/„rwa“ ist plattformbedingt dauerhaft (`mode &= MODE_READ_WRITE`), nicht vorläufig.
10. **C3 Hintergrund-Veröffentlichung, R8 (N14).** `DocsCommitWorker` braucht wegen minSdk 30 `getForegroundInfo()`; ein Vordergrunddienst aus dem Anbieterprozess ist nur mit abgeschalteter Akkuoptimierung (Ausnahme 13) zulässig.
11. **C3 `HandlerThread`, R6 (N11).** R6 ist nicht mehr nur RSAF-Kommentar, sondern im AOSP-Quelltext belegt (vold-Sperre + system_server-Watchdog 60 s; ab Android 15 + 30 s).
12. **C5 Schließen der Kind-Engines, R17 (N23).** Der Wert „60 s = doppelte Präsenz-Ablaufzeit“ beruht auf einer falschen Ablaufzeit (tatsächlich 300 s); der Wert braucht eine neue Begründung.
13. **C5 Offline-Anzeige Windows (N27).** `ERROR_HOST_UNREACHABLE` („The network location cannot be reached.“) hat eine Dokany-Abbildung (`STATUS_HOST_UNREACHABLE`). Zusätzlich gefundener Bestandsfehler: Der Rückfall `_ => ERROR_INVALID_DATA` in `io_status` wird von Dokany 2.3.1.1000 auf `STATUS_ACCESS_DENIED` abgebildet; nicht einzeln abgebildete Fehlerarten erscheinen auf bestehenden Laufwerken als „Zugriff verweigert“. Das berührt die Planaussage „Bestandstabelle unverändert“.
14. **C8 Lizenzen (N25).** Es gibt keine Lizenzübersicht; die vorgesehene „neue Zeile“ hat keinen Ort. Die MIT-Hinweispflicht besteht für `fuser` wie für jede bereits ausgelieferte MIT- oder Apache-2.0-lizenzierte Crate aus `native/Cargo.lock`.
15. **C1 Wiedervorlage T1 (N18).** Unverändert nicht erfüllt; CVE-2026-93689 ist eine Dienstverweigerung (CVSS 3.1 5.5), die Korrektur liegt in `v2.2B5` (2026-10-13).
16. **DRV-M10 Suite, R18 (N24).** `/dev/fuse` auf `ubuntu-24.04` ist durch den eigenen Lauf `37175826251` belegt (unprivilegiertes `sshfs`); `ubuntu-latest` wechselt im November 2026 auf 26.04.
