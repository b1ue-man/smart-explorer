# Windows: überprüfte Held-Rename-Hops

Geprüft am 2026-10-03 für die drei benannten Hosttrash-Diagnosen aus RV1-Run
37157166735. Kein neuer Plattformreview; Laufzeitbelege folgen in derselben Suite.

`SetFileInformationByHandle` erhält den lebenden Dateihandle, die Informationsklasse,
den Recordbuffer und dessen Bytegröße. Klasse `FileRenameInfo` benutzt
`FILE_RENAME_INFO`; ein Fehler liefert null und den OS-Fehler über `GetLastError`.
Der hier bereits reservierte DELETE-Handle bleibt der Sourcehandle.
[Microsoft: SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle).

Bei `FileRenameInfo` bleibt `ReplaceIfExists` false. Ein vorhandenes Ziel muss
dadurch einen Fehler liefern. Für relative Namen darf `RootDirectory` den
geöffneten Zielordner benennen. Die Dokumentation beschreibt `FileNameLength` in
Bytes und erlaubt dort fehlende Nullterminierung, beschreibt `FileName` zugleich
als NUL-terminierten Text. Die sichere konkrete Übergabe verwendet deshalb einen
validierten Childnamen, einen expliziten UTF-16-NUL im Buffer und eine Länge ohne
diesen NUL. Die Buffergröße enthält den NUL. Der Ziel-DirectoryHandle hält den
Ordner und seine physischen Vorfahren; kein DOS-/UNC-String muss erneut aufgelöst
werden. Die vorhandene Recordalignment-Berechnung bleibt erhalten.
[Microsoft: FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info).

`GetFinalPathNameByHandleW` liest einen Handlepfad. Bei Flags null liefert es einen
normalisierten DOS-/UNC-Pfad mit Verbatim-Präfix; Rückgabelänge ohne NUL, bei zu
kleinem Buffer benötigte Größe einschließlich NUL. Null meldet einen OS-Fehler.
Die Fixture-Diagnostik stellt diesen Pfad neben die beiden dauerhaft gespeicherten
Held-Slots und deren frisch geöffnete 128-Bit-FileID/Volume-Zuordnung. Ein
Sourcehandlepfad allein ersetzt keine Zielbestätigung.
[Microsoft: GetFinalPathNameByHandleW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew).

Der native Rename-Vertrag verlangt DELETE-Zugriff und liefert den endgültigen
Operationsstatus. Es wird keine neue NT-FFI oder Rechteausweitung eingeführt;
der vorhandene Win32-Aufruf bleibt. Erfolg wird zusätzlich durch NoFollow-Öffnen
des Zielchilds und Vergleich der vollständigen Objektidentität bestätigt.
[Microsoft: NtSetInformationFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile).

Die bisherige absolute Übergabe hat keinen expliziten NUL; nur u64-Rundung kann
Padding liefern. Dieser Buffervertrag ist im Source belegt. Dass genau dies die
drei Runnerfehler verursacht hat, ist noch nicht bewiesen. Die ergänzte Diagnose
benennt Capture, Restart, Restorehop und Original-Veröffentlichung; vorhandene
Collision-/Inhalts-/Restart-Assertions bleiben vollständig.
