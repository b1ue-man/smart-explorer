# Private Dateiobjekte: tatsächlich benötigte Zugriffscapabilities

Geprüft am 2026-10-03 für die konkreten Windows-/Android-Fehler aus
[Run 37157166735](https://github.com/b1ue-man/smart-explorer/actions/runs/37157166735).
Dies ergänzt den vorhandenen RV1-Plan, ohne eine weitere Prüfungskampagne.

## Linux-/Android-Vorfahren

Die aktuelle [Linux-Primärreferenz open(2)](https://man7.org/linux/man-pages/man2/open.2.html)
belegt: `O_PATH` liefert einen Positionshandle ohne Leserecht am Objekt;
Pfadauflösung und spätere Kindzugriffe prüfen weiterhin Suchrechte.
`openat` akzeptiert ihn als `dirfd`, `fstat` bleibt möglich. `fchmod` und
Datenoperationen sind darauf unzulässig. Zusammen mit `O_DIRECTORY`,
`O_NOFOLLOW` und `O_CLOEXEC` dient er ausschließlich als Verzeichnispin.
`O_DIRECTORY` schließt dabei auch einen durch `O_NOFOLLOW` gehaltenen
Symlink als Verzeichnis aus. `O_CREAT | O_EXCL` verweigert vorhandene Ziele.

Entscheidung: Im privaten Speicher werden nur physische Zwischenvorfahren
mit `libc::O_PATH` geöffnet. Das letzte App-Verzeichnis bleibt `O_RDONLY`;
sein bestehender Owner-/0700-Check, handlegebundenes `fchmod` und `fsync`
laufen auf einem wirklich lesbaren Handle. Datei-Owner, 0600, ein einzelner
Hardlink, NoFollow und exklusive 0600-Erstellung bleiben unverändert.
Die vorhandene Auswahl des physischen Parents einschließlich Android-Datenroot-
Alias bleibt bestehen; jeder anschließende einzelne Hop bleibt NoFollow.

Die erlaubten [Android-13-bionic-](https://android.googlesource.com/platform/bionic/+/refs/heads/android13-release/libc/kernel/uapi/asm-generic/fcntl.h)
und [aktuellen bionic-Header](https://android.googlesource.com/platform/bionic/+/refs/heads/main/libc/kernel/uapi/asm-generic/fcntl.h)
waren am Prüfdatum über den Webzugriff nicht abrufbar; beim ersten auch
nicht mit `format=TEXT`. Der zusätzlich gezielt freigegebene vorhandene
Cratequelltext `libc 0.2.186/src/unix/linux_like/android/mod.rs:696`
definiert tatsächlich `pub const O_PATH: c_int = 0o10000000;`;
Zeilen 675–710 wurden rein lesend geprüft. Der Code verwendet dieses
`libc::O_PATH`, ohne eine Zahl zu duplizieren. Laufzeitakzeptanz wird erst
die gemeinsame Remote-Suite belegen.

## Windows-Zugriff und Sharing

Aktuell gelesen: [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
und [SetSecurityInfo](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setsecurityinfo).
`dwDesiredAccess` bestimmt die Rechte des Handles; Sharing ergänzt diese
Rechte nicht. Shareflags bleiben bis zum Schließen wirksam und müssen zu
anderen offenen Handles passen. Fehlendes `FILE_SHARE_DELETE` verweigert
auch Rename eines gehaltenen Objekts. `SetSecurityInfo` ändert das über
einen Handle identifizierte Objekt; ein Fehler ist ein zurückgegebener
Win32-Code, kein erfolgreicher Sicherheitsnachweis. Die bestehende private
Grenze setzt ausschließlich eine nicht vererbende, geschützte Owner-DACL
am bereits auf Owner, Typ, Reparse und Hardlinkanzahl geprüften Objekt.

Die konkrete Kette `checkpoint_journal::Journal::append` →
`vfs::sync_filesystem` → `LocalBackend::sync_filesystem` →
`local_platform::{filesystem_profile,flush_filesystem}` wurde gelesen.
Der Windows-Flush selbst ist `Ok(())`, ein Profilefehler wird als
`PerFileOnly` behandelt. Dort entsteht kein AccessDenied. Das Journal
öffnet dagegen `append(true)` und ruft anschließend `set_len` auf.
Aus einem privaten Watchpfad oder
großzügigerem Sharing wird keine Schreib- oder Flushberechtigung abgeleitet.

## Windows-Checkpoint: Kürzen und appendierte Frames

Am selben Datum gelesen: [Rust 1.99 OpenOptions::append](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html#method.append),
[SetEndOfFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setendoffile)
und [SetFilePointerEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfilepointerex).
`write(true).append(true)` ist laut Rust identisch mit reinem Appendmodus.
Das explizite Kürzen eines Windows-Dateihandles verlangt dagegen
`GENERIC_WRITE`; ein Fehler wird über den Win32-Fehlercode gemeldet.
Die Dateiposition benötigt einen Lese-/Schreibhandle; mehrere gleichzeitige
Position-/Write-Operationen erfordern Synchronisation des Callers.
Die Rust-HTML-Pfade `windows.rs.html` und `windows/mod.rs.html` waren nicht
abrufbar. Die danach gezielt freigegebene [Rust-1.99.0-Primärquelle](https://github.com/rust-lang/rust/blob/1.99.0/library/std/src/sys/fs/windows.rs)
ist frisch gelesen: `get_access_mode` gibt für Append
`FILE_GENERIC_WRITE & !FILE_WRITE_DATA` zurück, unabhängig von `write(true)`.
`truncate` verwendet tatsächlich `FILE_END_OF_FILE_INFO` mit der
handlegebundenen SetFileInformationByHandle-Hülle; `seek` verwendet
`SetFilePointerEx`. `SetEndOfFile` ist der oben belegte parallele
Größenänderungsvertrag, nicht die behauptete Rust-Implementierung.
Die anschließend freigegebenen frischen Microsoft-Primärreferenzen
[SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)
und [NtSetInformationFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile)
schließen den tatsächlichen Hop: Win32 `FileEndOfFileInfo` nutzt die
End-of-file-Struktur, und die native Klasse `FileEndOfFileInformation`
verlangt ausdrücklich `FILE_WRITE_DATA`. Genau dieses Recht entfernt
Rust im Appendmodus. Das erklärt den belegten Windows-Fehler aus Source
und API-Vertrag; eine bereits bestandene Laufzeitabnahme wird damit nicht
behauptet. Der Anwendungscode führt keinen neuen nativen NT-Aufruf ein.

Entscheidung und vorhandene lokale Syntax: unter dem unverändert exklusiven
Pair-/Journalvertrag öffnet `creds::private_storage::open_file(path, true)`
eine bestehende Datei wirklich read/write und prüft sie vor Verwendung;
`create_file(path)` erstellt ausschließlich ein neues privates Objekt.
Windows verwendet dabei explizit `GENERIC_WRITE`, Unix `O_RDWR`.
Nach `file.set_len(valid_bytes)?` setzt
`file.seek(SeekFrom::Start(valid_bytes))?` die Schreibposition ausdrücklich.
Erst danach werden Länge, Digest und Payload geschrieben und `sync_all`
ausgeführt. IO- oder Validierungsfehler propagieren; es gibt keine fremde
Dateiersetzung, append-only-Rechteerhöhung per Sharing oder Ignorierung
einer beschädigten vollständig geschriebenen Frame.

Der Parent wird vor neuer Erstellung mit `ensure_directory` geprüft;
Owner-/DACL-/Mode-Migration und NoFollow-/Hardlink-Prüfung erfolgen auf den
tatsächlich geöffneten privaten Objekten. Das gültige Frame-Präfix, Budget,
SHA-256, begrenzte Tail-Recovery und bereits gespeicherte Baselines behalten
ihre Bedeutung. Dieser Vertrag erlaubt keine parallelen freien Appendwriter.

## Windows-Versionsbackup: Flush am privaten Datenhandle

Für den konkreten `version_save::save`-Fehler aus
[Run 37162485159](https://github.com/b1ue-man/smart-explorer/actions/runs/37162485159)
am 2026-10-04 frisch gelesen:
[Microsoft FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers),
[Rust File::sync_all](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all)
und die [Rust-1.99.0-Windows-Implementierung](https://github.com/rust-lang/rust/blob/1.99.0/library/std/src/sys/fs/windows.rs).
Die native Syntax lautet `BOOL FlushFileBuffers(HANDLE hFile)`; der
Dateihandle benötigt ausdrücklich `GENERIC_WRITE`. Null signalisiert einen
Fehler mit `GetLastError`. Rust stellt `sync_all(&self) -> io::Result<()>`
bereit und ruft in der Windows-Implementierung über `fsync` tatsächlich
`FlushFileBuffers` auf; ein Fehler wird propagiert. Ein Lesehandle erhält
diese Capability weder durch DACL-Härtung noch durch Sharing.

Die konkrete Sourcekette ist `version_save::copy_private` →
`support_dirs::open_private_file` → `private_storage::open_file(path, false)`
→ `DirectoryHandle::open_private_child(name, false)` → `File::sync_all`.
Dieser private Readpin hat unter Windows bewusst kein `GENERIC_WRITE`.
Die vorhandene RW-Variante
`creds::private_storage::open_file(path, true) -> io::Result<File>`
liefert dagegen den zum Flush erforderlichen Zugriff nach privater
Owner-/DACL-/NoFollow-/Hardlink-Prüfung. Sie öffnet ein vorhandenes Objekt,
ohne es zu erstellen oder zu kürzen.

Entscheidung: Die bestehende Öffnung und Härtung des privaten Backups als
Leseobjekt bleiben erhalten, damit ein aus einem schreibgeschützten
Quellmode entstandenes Backup zuerst privat gehärtet wird. Danach wird
der Readpin ausdrücklich geschlossen, bevor die vorhandene geprüfte
RW-API ausschließlich für `sync_all` verwendet wird. Das Schließen ist
auch für die Windows-Sharekompatibilität nötig: Der gewöhnliche Readpin
gewährt kein `FILE_SHARE_WRITE`. Reguläre Leseobjekte und Quellhandles
bekommen dadurch keine neuen Zugriffs- oder Sharingrechte.

Die anschließend gezielt freigegebenen aktuellen Unix-Definitionen
bestätigen diese Reihenfolge: `open_file(path, writable)` delegiert an
`open(path, writable, false)`; `secure(file, false)` prüft am geöffneten
Objekt UID, regulären Dateityp und genau einen Hardlink, setzt bei Bedarf
0600 über den Handle und kontrolliert Owner, Mode und Linkanzahl erneut.
Die private Härtung erfolgt somit weiterhin vor der RW-Öffnung.

Härtungs-, Öffnungs- und Flushfehler bleiben Fehler des Backupschritts.
`entry.json` wird weiterhin erst nach erfolgreichem Flush geschrieben;
Intent, exklusive Veröffentlichung, kopierter Digest und abschließende
Quellrevalidierung behalten ihre Bedeutung. Es wird weder ein Fehler
ignoriert noch eine neue Verzeichnis- oder Namespace-Durability zugesagt.
