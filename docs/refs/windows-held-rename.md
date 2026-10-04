# Windows: Held-Rename am tatsächlich gehaltenen Zielordner

Aktueller Primärabgleich: 2026-10-04. RV1-Run 37162485159 auf
`ac3b0c9098963fae386e94558f2f3ca1bb240740` weist den CI-3-Win32-Aufruf bereits
beim ersten Capture mit Error87 ab. Die vorhandene Hopdiagnostik bestätigt:
Originalpfad und ursprüngliche FileID bestehen fort, beide Held-Slots fehlen.
Der vorige Anschluss war damit kein erfolgreicher Nachweis dieses API-Vertrags.

Die Win32-Seite `FILE_RENAME_INFO` beschreibt zwar einen relativen Namen mit
`RootDirectory`. Daraus allein folgt nach dem tatsächlichen Error87 kein
funktionsfähiger Aufruf am Zielrunner. `SetFileInformationByHandle` benutzt
die Win32-Klasse `FileRenameInfo` (3), BOOL-Rückgabe und `GetLastError`.
Diese Fassade wird nicht durch geratenes Umschalten ihrer Parameter repariert.
Die vorherige Folgerung, dieselbe relative Übergabe sei dort bereits tragfähig,
ist durch den Run widerlegt; keine pauschale Aussage über sämtliche Win32-Versionen.
[Microsoft: FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info),
[SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle).

Der native Vertrag ist separat: `FILE_RENAME_INFORMATION`,
`FileRenameInformation` (10), `NtSetInformationFile` und `IO_STATUS_BLOCK`.
Der Usermode-Aufruf heißt `NtSetInformationFile`, nicht `ZwSetInformationFile`;
NTSTATUS wird ausgewertet, nicht ein ungeändert ausgelesener Win32-LastError.
DELETE am Sourcehandle bleibt erforderlich. Der kleine OS-Adapter konsumiert
die konkret gelesenen `windows-sys 0.59`-Bindings: `NtSetInformationFile` linkt
`ntdll.dll`, sein IOSB-Anschluss setzt `Win32_System_IO` voraus. `FILE_RENAME_INFORMATION`
hat die Union `Anonymous.ReplaceIfExists: BOOLEAN`, `RootDirectory: HANDLE`,
`FileNameLength: u32` und `FileName: [u16; 1]`; Klasse 10 kommt aus demselben Binding.
Nur `Wdk_Storage_FileSystem` wird additiv freigeschaltet; keine handgeschriebene
ABI und keine Kernelbibliothek werden in die Desktopanwendung eingeführt.
[Microsoft: NtSetInformationFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile).

Bei der nativen relativen Form ist `RootDirectory` der tatsächliche gehaltene
Ziel-Directoryhandle; der Name ist genau ein validierter Childname.
`ReplaceIfExists` bleibt false, vorhandene Ziele müssen scheitern; Volumenwechsel
werden nicht durch Copy/Delete simuliert. `FileNameLength` zählt UTF-16-Bytes.
Die dokumentierte Mindestbuffergröße ist `sizeof(FILE_RENAME_INFORMATION)` plus
Namebytes. Deshalb genügt die vorherige Rechnung `offset(FileName)` plus
Namebytes plus zwei nicht als Nachweis dieser Mindestgröße. Alignment, geprüfte
Byterechnung und expliziter NUL außerhalb der Namenslänge bleiben erhalten.
Erfolg erfordert weiterhin ein frisches NoFollow-Zielhandle mit derselben vollen
FileID/Volume-ID wie der lebende DELETE-Guard.
[Microsoft: FILE_RENAME_INFORMATION](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information).

Die ursprünglichen Record-/Restore-Verträge bleiben: Intent mit beiden Held-Slots
dauerhaft vor Capture, Root-ID und SHA-/Längenprüfung, Rollback ohne Ersetzung,
erneuter Restore nach Fehlern. Die vorhandenen Hopdiagnosen und Assertions bleiben.

Der vorhandene DELETE-Guard wird ohne `FILE_FLAG_OVERLAPPED` geöffnet und ist damit
synchron: CreateFile beschreibt blockierende I/O-Aufrufe. NtCreateFile bindet
synchrone Dateiobjekte an `SYNCHRONIZE`; dieses Recht ist auch Voraussetzung eines
Completionwaits am Filehandle. Die vorhandenen Öffnungsrechte, `FILE_SHARE_READ`
und `FILE_FLAG_OPEN_REPARSE_POINT` werden nicht verändert.
[Microsoft: CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew),
[NtCreateFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntcreatefile).

Der direkte NTSTATUS ist maßgeblich, außer bei `STATUS_PENDING` (`0x103`). Dann
müssen Renamebuffer, IOSB und beide geliehenen Handles während des Aufrufs und
Completionwaits gültig bleiben; erst nach `WAIT_OBJECT_0` wird `IO_STATUS_BLOCK.Anonymous.Status`
gelesen. Negative finale NTSTATUS werden mit `RtlNtStatusToDosError` in den echten
OS-Fehler übersetzt; kein Win32-LastError vom NT-Aufruf. Ein unbekannter Abschluss
(Waitfehler oder weiter Pending) wird niemals als Hop-Erfolg gemeldet. Für diesen
Verstoß gegen den synchronen Guardvertrag verbleiben die heapgebundenen Buffer und
IOSB sicher bis Prozessende, statt mögliche laufende Kernelzugriffe freizugeben.
Die bereits dauerhaften Original-/Held-Zuordnungen bleiben der Recoveryweg.
[Microsoft: IO_STATUS_BLOCK](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/ns-wdm-_io_status_block),
[WaitForSingleObject](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject),
[RtlNtStatusToDosError](https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-rtlntstatustodoserror).

Das Binding und diese Completion-/Fehlerdefinition sind vor Sourceedit abgeglichen.
Der vollständige Laufzeitnachweis bleibt derselben Remote-RV1-Suite.
