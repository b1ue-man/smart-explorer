# Tatsächliche Namespacebestätigung nach Veröffentlichung

Stand: 2026-10-04. Begrenzter V-NAMESPACE-Anschluss aus dem vollständig
beendeten RV1-Run 37162485159, gemäß `ci-fourth-fixes.md` und
`scopes/ci-4-v-namespace.json`. Die Quellen wurden vor dem jeweiligen
Sourceedit frisch gelesen; Laufzeitabnahme bleibt bei derselben Root-Suite.

## Grenze und Umsetzungsplan

Die aktuelle Local-Kette bewertet Android-FUSE in `sync_filesystem` ehrlich
als `PerFileOnly`. Ein bereits einzeln geflushter Stage braucht nach der
Veröffentlichung zusätzlich die Bestätigung seines tatsächlichen
Elternverzeichnisses. Das bedeutet keine Fähigkeit, alle Deferred-Stages
mit einem Whole-filesystem-Flush dauerhaft zu machen.

Mit E-ENGINE vereinbarte additive API:

```rust
fn confirm_namespace(&self, parent: &str) -> VfsResult<bool>;
pub fn confirm_namespace<B: Backend + ?Sized>(
    backend: &B,
    parent: &str,
) -> VfsResult<bool>;
```

Der Traitdefault und der Dispatch ohne Extensions liefern `Ok(false)`.
`parent` ist das Elternverzeichnis nach bereits erfolgreicher
Veröffentlichung. `Ok(true)` bestätigt dessen veröffentlichten Namespace
im jeweiligen unterstützten OS-Vertrag; es verspricht keinen Fileflush
und keine neue Flushfähigkeit für Deferred-Stages. Öffnungs-/Mount-/Sync-
Fehler propagieren. Unbekannte Implementierungen bleiben unbestätigt.

Die zusammenhängenden Schritte und späteren Prüfsignale:

1. Additiver Trait-/Dispatch-/Localadapter: E verwendet den neuen Hook
   im lokalen Post-publish-Consumer; der Remotezweig und
   `sync_filesystem` samt Deferred-Preflight behalten ihre Bedeutung.
2. Gepinnter Linux-/Android-Parent: der bereits vorhandene read-only
   `DirectoryHandle` liefert einen geliehenen `&File`; Namespace-Fsync
   und die FD-gebundene Mountzuordnung arbeiten auf diesem gehaltenen
   Verzeichnis. Unbekanntes FUSE bleibt false.
3. Windows: die bereits vorhandenen einzeln geflushten Stages und
   erfolgreichen Write-through-Publications werden durch die Query
   bestätigt. Neue Daten-/Rootrechte oder ein unprivilegierter
   Volume-/Directory-Flush werden daraus nicht abgeleitet.

Erwartet sind die bestehenden Android-Delete-/Versions-/SavedSync-/Merge-
und privaten Fixtures sowie Windows-Agent-Copy aus derselben RV1-Suite.
Fehler/unsupported dürfen keinen erfolgreichen Checkpoint erzeugen.
Tests oder Workloads werden durch diesen Plan nicht autorisiert.

## Linux-/Android-Primärvertrag

Frisch gelesen:
[fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html) und
[syncfs(2)](https://man7.org/linux/man-pages/man2/syncfs.2.html).
Native Syntax: `int fsync(int fd)` und `int syncfs(int fd)`.
File-fsync allein bestätigt keinen neuen Verzeichniseintrag; dafür ist
Directory-fsync am Eltern-FD nötig. Null bedeutet Erfolg, -1 einen errno-
Fehler. Fehler werden nicht als erfolgreiches Publish gewertet.
`syncfs` betrifft dagegen das gesamte Dateisystem des FD und behält
seinen separaten bereits vorhandenen Vertrag.

Die bestehende lokale Ref
[local-fs-identity-durability.md](local-fs-identity-durability.md)
dokumentiert außerdem: generisches FUSE kann ohne `fsyncdir`-Handler
einen Nullwert zurückgeben. Ein erfolgreicher syscall allein ist dort
kein Implementierungsnachweis.

Frisch vollständig gelesen: [AOSP MediaProvider FuseDaemon.cpp, main](https://android.googlesource.com/platform/packages/providers/MediaProvider/+/refs/heads/main/jni/FuseDaemon.cpp),
Blob `fb0468c5babc7288292e79bd6701666e0e73136c`.
Die Ops-Tabelle registriert `pf_fsyncdir`. Der Handler verwendet
`dirfd(h->d)` des tatsächlich geöffneten unteren Verzeichnisses;
`do_sync_common` führt Fsync aus und gibt errno an den FUSE-Caller zurück.
Opendir erhält vorher die reguläre MediaProvider-Zugriffsfreigabe.
Es gibt hier somit ein echtes Directory-Fsync. Die Implementierung
berechtigt den Client nicht, die unteren `/data/media`- oder
`/mnt/media_rw`-Pfade selbst zu öffnen.

Der vorab erlaubte [android16-release-Pfad](https://android.googlesource.com/platform/packages/providers/MediaProvider/+/refs/heads/android16-release/jni/FuseDaemon.cpp)
war zweimal nicht abrufbar. Aus diesem Pfad wird kein Beleg behauptet.
Die konkrete Android-Mountzulassung und Device-Syntax werden vor deren
Sourceedit weiter unten gesichert; die allgemeinen FsProfile werden
dafür nicht hochgestuft.

## Windows-Primärvertrag

Frisch gelesen: [Microsoft MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).
Syntax: `BOOL MoveFileExW(LPCWSTR source, LPCWSTR destination, DWORD flags)`.
`MOVEFILE_WRITE_THROUGH` lässt den Move erst nach seiner Durchführung
auf Disk zurückkehren; die explizite Flushzusage beschreibt insbesondere
den Copy/Delete-Fall. Daraus wird hier keine zusätzliche Power-loss- oder
allgemeine Directory-Fsync-Garantie behauptet. Null ist ein Fehler und
der bestehende Code propagiert `GetLastError`.

Die frisch gelesenen bestehenden Adapter veröffentlichen per
`MoveFileExW(..., MOVEFILE_WRITE_THROUGH)` ohne Copy-/Delay-Flags.
NoReplace lässt weiterhin `MOVEFILE_REPLACE_EXISTING` weg, Replace setzt
es wie bisher ausdrücklich. `finish_stage` öffnet einen regulären Stage
mit tatsächlichem Writezugriff und flusht ihn bei jedem erforderlichen
Durabilitywert. Der Agent erledigt somit auch Deferred-Stages einzeln.
Die vorhandene Windows-Query muss diesen bestehenden begrenzten Vertrag
bestätigen; sie ist kein neuer Volume-Flush und verspricht keine
Bestätigung beliebiger ungesicherter Writes oder Dateien.

## Konkrete Android-Mountzulassung und gepinnter FD

Vor dem Adapteredit zusätzlich frisch gelesen:
[vold Utils.cpp](https://android.googlesource.com/platform/system/vold/+/refs/heads/main/Utils.cpp)
(Blob `c4070d136a86a97aa238c8ff3742d7e282c36b9d`),
[EmulatedVolume.cpp](https://android.googlesource.com/platform/system/vold/+/refs/heads/main/model/EmulatedVolume.cpp)
(Blob `83d6c137078a1f314ead3bcdbec75ff59aaf3b26`) und
[PublicVolume.cpp](https://android.googlesource.com/platform/system/vold/+/refs/heads/main/model/PublicVolume.cpp)
(Blob `91b1ca236b3b3316f34ef1ad80186fef7458a808`).

`MountUserFuse` montiert Quelle `/dev/fuse`, Typ `fuse`, unter
`/mnt/user/<user>/<label>`; diese Root-Verzeichnisse erstellt vold als
Systemorte. EmulatedVolume verwendet die Labels `emulated` oder eine
Filesystem-UUID und exportiert `/storage/<label>`; PublicVolume verwendet
seine Filesystem-UUID oder `public:<major>,<minor>` und denselben Mountweg.
Die vorhandene per-user-Emulation und Bindaliase bleiben dieselbe FUSE-
Deviceinstanz. Diese Verbindung zwischen Systemmount und registriertem
MediaProvider-Handler ist die begrenzte Plattformzulassung; ein beliebiger
Pfadpräfix oder irgendein erfolgreiches FUSE-Fsync genügt nicht.

Positive Bedingungen vor Directory-fsync:

- Der reale Parent wird über den bestehenden gewöhnlichen
  `DirectoryHandle::open_root` gehalten. Eine additive
  `directory_file(&self) -> &File` leiht ausschließlich dessen read-only
  Directory-FD. Rootalias- und Child-NoFollow-Verträge ändern sich nicht.
- Die aufgelöste `/proc/self/fd/<fd>`-Position dieses lebenden Handles wird
  in der vorhandenen Mounttabelle eingeordnet. Das tatsächliche
  `st_dev` des Files muss mit `Mount.device` übereinstimmen.
- Bereits unterstützte native Linux-/Android-Filesysteme verwenden ihr
  bisheriges `FlushModel`; ihre Directory-fsync-Fehler propagieren.
- Die zusätzliche FUSE-Zulassung gilt ausschließlich auf Android, bei
  exakt Typ `fuse`, Quelle `/dev/fuse` und einer System-Storage-Root
  derselben Deviceinstanz. Zugelassen sind die oben belegten
  `/storage/<label>`- und `/mnt/user/<user>/<label>`-Roots, einschließlich
  ihrer per-user-Views mit übereinstimmendem `Mount.root`. Labels sind
  `emulated`, eine validierte FAT-/volle UUID oder der dokumentierte
  `public:<major>,<minor>`-Fallback.

Negative Bedingungen: unbekannter Mount, unpassende Devicezuordnung,
beliebiges Linux-FUSE, `fuseblk`, `fuse.<subtype>`, fremde Quelle,
Nicht-Systemmount und nicht zugeordnete Root-/Bindview bleiben
`Ok(false)`. Öffnungs-, Proc-/Mount- und Directory-fsync-Fehler bleiben
`Err`; es gibt keinen Fileflush-, unteren Pfad-, Rechte- oder
Whole-filesystem-Fallback. `FsProfile` bleibt vollständig unverändert.

Die vorab gezielt erlaubten vorhandenen libc-0.2.186-Definitionen wurden
frisch gelesen: Android `major(dev_t)->c_int`, `minor(dev_t)->c_int`
(`android/mod.rs:3392–3405`), Linux dieselben sicheren Constfunktionen mit
`c_uint` (`linux_l4re_shared.rs:1623–1637`). Der Code ruft diese Helper
auf und vergleicht das u32-Paar; er dupliziert keine ABI-Bitmasken.
`File::sync_all` arbeitet danach auf demselben weiterhin gehaltenen
Directory-FD, dessen Position und Device geprüft wurden.

E-ENGINE bestätigt die wichtige zeitliche Grenze: PRE-Publish-Fallbacks
bei nicht geflushten Now-Stages bleiben ausdrücklich `sync_filesystem`
mit echter true-Bestätigung. `confirm_namespace` wird ausschließlich
POST-Publish am Parent eingesetzt. Damit kann die neue Directory-
Bestätigung keine fehlenden Filecontents oder einen Deferred-Endflush
ersetzen.
