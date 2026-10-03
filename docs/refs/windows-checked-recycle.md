# Windows – gebundenes Recycle und Wiederherstellen

Primärquellen, geprüft am 2026-10-03:
[IFileOperation::DeleteItem](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-deleteitem),
[IFileOperation::SetOperationFlags](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags).

`DeleteItem` übernimmt ein `IShellItem` und plant die Aktion; `PerformOperations`
führt sie aus. `FOFX_RECYCLEONDELETE` verlangt den Papierkorb. Die dokumentierte
Form enthält weder das von RV1 geprüfte Datei-Handle noch einen separaten
Originalpfad für die Wiederherstellung einer bereits umbenannten Quarantäne.

Folgerung für den vorhandenen FA6-Anschluss: Ein erneuter Shell-Aufruf auf dem
Quarantänepfad genügt dem hier verlangten Handle-/Originalpfadvertrag nicht.
Windows-Fern-Recycle erhält deshalb einen ausdrücklich benannten
Smart-Explorer-Papierkorb mit dauerhaftem Wiederherstellungsrecord und sichtbarer
Host-Bedienung. Das ist eine Entscheidung dieses Anschlusses, keine allgemeine
Aussage über alle möglichen Shell-Erweiterungen.

Der Record muss vor der ersten Umbenennung privat und dauerhaft geschrieben
sein, die exakte vorgesehene `.held.se-recycle-<16hex>`-Position und die ursprüngliche
autorisierte Wurzel enthalten. Erfasst werden nur reguläre Dateien desselben
geprüften Objekts; Größe und SHA-256 werden vor und nach der Handle-Umbenennung
geprüft. Fehler stellen ohne Ersetzen wieder her oder bewahren Record und Inhalt
mit einer konkreten Meldung. Ein Neustart findet auch unvollständige Intents.

Auflisten und Wiederherstellen laufen über dieselbe private Storage-Fassade und
ab der aufgezeichneten Wurzel über gehaltene DirectoryHandles. Childlinks werden
abgewiesen, Restore ersetzt keine vorhandene Datei, Einzelprobleme verlieren
keine anderen Einträge. Die Desktop-Bedienung zeigt Originalort, Zustand und
Wiederherstellung; sie bezeichnet diese Einträge als Smart-Explorer-Papierkorb.
Die bisherigen lokalen Systempapierkorb-Operationen bleiben erhalten.
