# RV1: schmale Remote-Suite-APIs

Geprüft am 2026-10-03. Diese API-Lückenprüfung betrifft ausschließlich die gemeinsame
Abnahme des vorhandenen RV1-Plans, keinen zusätzlichen Projekt-Review.

## Rust/Cargo

Primärquellen: [cargo test](https://doc.rust-lang.org/cargo/commands/cargo-test.html),
[libtest](https://doc.rust-lang.org/rustc/tests/index.html),
[Cargo-JSON](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages).

`cargo test --locked -p smart_explorer --lib --no-run --message-format=json` baut nur den
gewählten Library-Testhost, ohne ihn auszuführen. `compiler-artifact` mit
`profile.test == true`, dem passenden Targetnamen und `executable` entdeckt die Ausgabe.
Der Share-Server hat einen eigenen Bin-Testhost; sein `--bin` wählt nur diesen.
Cargo-Ausgaben bleiben in ihrem inkrementellen Cache; kein Workspace-/All-target-Build.

Die direkte libtest-Datei akzeptiert mehrere Filter als OR von Teilstrings.
`--list --format terse` entdeckt tatsächliche Namen, `--include-ignored` nimmt die
explizit bereitgestellten Umgebungsfixtures dazu, `--test-threads=1` schützt gemeinsamen
Prozesszustand. `--exact` gilt für sämtliche Filter und eignet sich nur für volle Namen.
Windows darf statt einer überlangen Liste voller Namen kurze Filter verwenden, sofern
entdeckte Auswahl und tatsächliche erfolgreiche Ergebnisse vollständig abgeglichen werden.

## Subprozesse

Primärquelle: [Python subprocess](https://docs.python.org/3/library/subprocess.html).
Die vorhandene Suite `native/test-sync-paths-task.py` ist das geprüfte Projektmuster.
`Popen` erhält Argumentlisten, `cwd` und eine kopierte `env`; kein Shell-String für dynamische
Werte. POSIX `start_new_session=True` trennt die Kindgruppe. `wait(timeout=...)` wirft
`TimeoutExpired`; danach beendet der Besitzer die Gruppe und wartet auf Completion.
Windows benutzt `CREATE_NEW_PROCESS_GROUP` und das bestehende `taskkill /PID ... /T /F`-
Muster. Logs werden in Dateien geführt, damit keine Pipe durch ungeholte Ausgabe blockiert.

## Android und OS-Fixtures

Die etablierten, versionsgebundenen Android-Aufrufe bleiben in
[android-ci.md](android-ci.md), [android-toolchain.md](android-toolchain.md) und
[android-gradle-build.md](android-gradle-build.md). Der vorhandene Gerätehelfer liefert
`coreTest`, `Api`, `Fixture`, `SyncJobs`, `Share` und JNI-Calls; keine Mock-Erfolge ersetzen sie.
Die Suite baut nur x86_64-JNI/Debug/Test-APK und übergibt Entwicklungs-CLI/Server aus Linux.

Reales privates IPv4, ifindex und Interfacename kommen aus `ip -j` beziehungsweise
`Get-NetIPAddress` und werden vom Statusfixture gegen OS-Fakten erneut geprüft. FUSE,
FAT32 und exFAT werden auf dem isolierten Linux-Runner eingerichtet und in `finally`/Trap
abgehängt; alle Laufzeitpfade/Ports/Releaseassets stammen aus den Einrichtungskommandos.

## Isolierter FUSE-/Volume-Anschluss

Am 2026-10-03 geprüft: [SSHFS-Upstream](https://github.com/libfuse/sshfs),
[SSHFS-Manpage](https://raw.githubusercontent.com/libfuse/sshfs/master/sshfs.rst),
[losetup](https://man7.org/linux/man-pages/man8/losetup.8.html),
[mount](https://man7.org/linux/man-pages/man8/mount.8.html). SSHFS unterstützt
`-o directport=PORT` zu einem auf Loopback gebundenen `socat` mit OpenSSH
`sftp-server`; dies bleibt ein echter FUSE/SFTP-Mount auf dem isolierten Runner.
`fusermount3 -u` hängt ihn ab. Der Einstieg besitzt die eigene Socat-Prozessgruppe.
`losetup --find --show` entdeckt die jeweilige Loopdatei; die Suite hängt ihre
FAT32-/exFAT-Mounts vor `losetup --detach` ab. Nicht-interaktives sudo wird zuerst
geprüft. Die Entwicklungssuite verändert keine Produkt-Transportvorgabe.

Der zweite Runnerlauf vom 2026-10-03 meldet fehlende exFAT-Kernelunterstützung.
[exfat-fuse](https://github.com/relan/exfat/blob/master/fuse/mount.exfat-fuse.8)
unterstützt `mount.exfat-fuse -o uid=N,gid=N,umask=077 DEVICE DIR`.
Sein [Mountcode](https://github.com/relan/exfat/blob/master/fuse/main.c)
setzt `blkdev`, `allow_other`, `default_permissions` und den tatsächlichen
Gerätepfad als `fsname`; beliebige Subtypeoptionen werden nicht durchgereicht.
Deshalb erkennt der OS-Adapter den realen `fuseblk`-Quellblocktyp über
`MetadataExt::rdev`, `FileTypeExt::is_block_device` und udev `ID_FS_TYPE`
(Syntax in [local-fs-identity-durability.md](local-fs-identity-durability.md)).
Nur die Speicherlimits stammen vom unteren Typ; FUSE bleibt `PerFileOnly`.
Der Einstieg hängt jeden tatsächlich erzeugten Mount vor seinem Loopgerät ab.

## Öffentlicher Android-AppOps-Anschluss

Am 2026-10-03 gegen [AppOpsManager](https://developer.android.com/reference/android/app/AppOpsManager#permissionToOp(java.lang.String))
und [AOSP](https://android.googlesource.com/platform/frameworks/base/+/master/core/java/android/app/AppOpsManager.java)
geprüft: `permissionToOp(String)` (API 23) liefert den Operationsnamen oder
`null`. AOSP ordnet `MANAGE_EXTERNAL_STORAGE` diesem Namen zu, während seine
direkte `OPSTR_MANAGE_EXTERNAL_STORAGE`-Konstante versteckte System-API ist.
`startWatchingMode(String, String, OnOpChangedListener)` (API 19) beobachtet
nur die eigene UID. Bei fehlender Zuordnung/Runtimefehler bleibt die bestehende
periodische Rechteprüfung aktiv.

## JSON-Capability-Prefix

Am 2026-10-03 gegen den gepinnten
[serde_json 1.0.150 Decoder](https://github.com/serde-rs/json/blob/v1.0.150/src/de.rs)
und die lokale Registryquelle geprüft: `deserialize_map` ruft nach
`Visitor::visit_map` immer `end_map` auf. Frühes `Ok` reicht deshalb bei einem
noch folgenden Payloadfeld nicht. Ein vollständig gelesenes `Hint` wird separat
erfasst und der Visitor gezielt abgebrochen; es gibt kein Matching eines
Fehlertexts und kein Akzeptieren partiell gelesener Authfelder. Der Besitzer
arbeitet nur auf `TcpStream::peek`; die Originalbytes und die abschließende
Nachrichtenvalidierung verbleiben beim vorhandenen Clienthandler.

## Cache und Formatierung des Runners

Runner-APIs am 2026-10-03 gegen [setup-python](https://github.com/actions/setup-python),
[cache restore](https://github.com/actions/cache/blob/main/restore/README.md) und
[cache save](https://github.com/actions/cache/blob/main/save/README.md) abgeglichen.
Die bestehende Projektkonvention `setup-python@v6` mit `python-version: '3.12'`
bleibt verwendbar. Separate Restore-/Save-Schritte erhalten auch fehlgeschlagene
inkrementelle Ausgaben; die Suite validiert jede wiederverwendete ausführbare Datei
selbst gegen alle Buildinputs und SHA-256. Rustfmt wird nur remote über stdin/
stdout verwendet ([Upstream-README](https://github.com/rust-lang/rustfmt/blob/main/README.md)).
Der [ModResolver](https://github.com/rust-lang/rustfmt/blob/master/src/modules.rs)
überspringt externe Kindmodule für stdin; deshalb kann jede geänderte Datei separat
einen präzisen Patch liefern, einschließlich additiver Modulregistrierungen.

## Dritter Lauf: wirkliche FUSE-Zeitauflösung und Watchlimit

Geprüft am 2026-10-03 gegen [exfat-fuse 1.3.0 node.c](https://raw.githubusercontent.com/relan/exfat/v1.3.0/libexfat/node.c) und [exfat.h](https://raw.githubusercontent.com/relan/exfat/v1.3.0/libexfat/exfat.h): `exfat_utimes` übernimmt `tv[1].tv_sec` in `node->mtime`; das Feld ist `time_t`. Subsekunden werden nicht gespeichert. Folgerung: reale FUSE-exFAT-Blockvolumes melden Sekundenauflösung, ohne Kernel-exFAT von 10 ms herabzusetzen oder FUSE einen volumenweiten Flush zuzusichern.

[inotify(7)](https://man7.org/linux/man-pages/man7/inotify.7.html) beschreibt `/proc/sys/fs/inotify/max_user_watches` als echtes Limit pro UID. Die vorhandene Fixture erzeugt 129 Verzeichnisse; das normale Runnerlimit kann deshalb keinen WatchLimit belegen. Die Suite entdeckt ihren vollständigen Namen im vorhandenen Listing und startet genau diesen Fall innerhalb derselben Runtime separat mit einem echten Limit von 16. [sysctl(8)](https://man7.org/linux/man-pages/man8/sysctl.8.html) belegt `-n` und `-w`; `sysctl -n fs.inotify.max_user_watches` liest den ursprünglichen Wert; `sudo -n sysctl -w fs.inotify.max_user_watches=N` setzt und restauriert ihn. Alle anderen Fälle behalten das normale Limit, der eine gemeinsame Log enthält beide Ergebnisse, und EXIT/TERM/INT-Cleanup stellt das ursprüngliche Limit wieder her. Kein Fixturefall wird entfernt oder abgeschwächt.
