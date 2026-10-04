# CI-4-H-ANALYSIS – nativer Rename am gehaltenen Zielroot

Stand: 2026-10-04. Ausschließlich Run 37162485159 auf
`ac3b0c9098963fae386e94558f2f3ca1bb240740` und die benannten Diagnosen in
`/tmp/rv1-ci-fourth/h-analysis.json`. Scope: `../scopes/ci-4-h-analysis.json`.

## Eigener Stage-eins-Plan

Die vorhandene Diagnose bestätigt Error87 vor dem ersten Capture: Sourcehandle
und frisch geöffnetes Original zeigen dieselbe ursprüngliche FileID und Länge;
keiner der dauerhaft vorgesehenen Held-Slots existiert. Alle betroffenen
Hosttrash-, Restore- und Publikationsassertions erreichen daher ihren eigentlichen
Verhaltensschritt noch nicht. Kein Assertionwechsel oder Permissionbypass.

Die Win32-Schlussfolgerung aus CI-3 war nicht tragfähig. Der native Vertrag wird
anhand aktueller freigegebener Microsoft-Definitionen und gepinnter Bindings
separat geschlossen; die lokale Referenz wird vor Sourceänderung korrigiert.

## Stage-zwei-Plan und zweite konkrete API-Klärung

| Kohäsiver Schritt | Source / Anschluss | Signal derselben bestehenden Suite |
| --- | --- | --- |
| Win32-/NT-Definition und Completion klären | `docs/refs/windows-held-rename.md`, erlaubte Primärseiten, gepinnte Bindingdefinitionen | Klasse 10 und nativer Record, NTSTATUS/IO_STATUS_BLOCK und korrekter Usermode-Linkvertrag sind vor Edit belegt; keine geratenen Parameter. |
| Kleiner nativer Rename-Adapter | neues `local_access/os/windows/directory_rename.rs`, `quarantine.rs` | Bestätigter DELETE-Sourcehandle, gehaltenes Zielroot, einfacher Childname, NoReplace und Mindestbuffer; tatsächliche Zielidentität bleibt zwingend. |
| Minimaler Feature-/Registrierungsanschluss | nur exakter Rootgrant | Windows-Bindings und neues OS-Modul sind erreichbar; keine andere Feature-/Modulfläche geändert. |
| Kompatibilität und bestehende Caller erhalten | vorhandene read-only Capture-/Restore-/Recyclequellen und Fixtures | SHA/Länge, NoFollow, volle FileID/Root-ID, vorab dauerhafte Held-Zuordnung, Collision und Retry bleiben unverändert stark. |
| Eigener statischer Abschluss | eigener Bericht | Exaktes Inventar und Text-/Parsingvergleich, kein nativer lokaler Lauf. |

Vor Sourceedit abgeschlossen: Microsoft-Definitionen und konkrete `windows-sys 0.59`
bestätigen `ntdll.dll`, Klasse 10, den nativen Union-/UTF-16-Record und IOSB.
Root hat ausschließlich die additive `Wdk_Storage_FileSystem`-Featurezeile und
private `mod directory_rename`-Registrierung freigegeben. Der bestehende Guard
öffnet synchron ohne OVERLAPPED; seine Rechte, ShareMode und NoFollow bleiben.
Der direkte NTSTATUS zählt, bei Pending erst Completionwait + finaler IOSB.
Ein nicht bestätigter Wait erhält die heapgebundenen I/O-Speicher bis Prozessende
und meldet einen Fehler mit Zielslot; er darf weder freigegeben noch als erfolgter
Hop behandelt werden. Die bestehende dauerhafte Intentzuordnung bleibt erhalten.

## Ergebnis, APIs und Entscheidungen

Die Referenz wurde vor Sourceedit korrigiert. Der gescheiterte Win32-Anschluss
ist durch den dokumentierten nativen Usermode-Aufruf ersetzt. Die Diagnose
belegt Error87 vor Capture; sie belegt noch keinen erfolgreichen Ersatzlauf.

- Neue private OS-API: `directory_rename::no_replace(file: &File, target: &DirectoryHandle, name: &OsStr) -> io::Result<()>`.
  Einziger Anschluss ist `quarantine::rename_no_replace` nach bestehender Childvalidierung.
- `NtSetInformationFile` aus `ntdll.dll`, `FileRenameInformation` (10), tatsächlicher
  `FILE_RENAME_INFORMATION`-Record, `Anonymous.ReplaceIfExists = 0` und gehaltenes
  Directoryhandle. Kein freier Zielpfad und kein Copy/Delete-Fallback.
- Geprüfte u32-Bytelänge und ausgerichteter Buffer mindestens
  `sizeof(FILE_RENAME_INFORMATION) + UTF16-Namebytes + 2`; expliziter NUL außerhalb
  `FileNameLength`, unveränderte UTF-16-Komponenten.
- Finaler NTSTATUS wird korrekt mit `RtlNtStatusToDosError` übersetzt.
  Pending wartet mit `WaitForSingleObject`; erst abgeschlossener IOSB zählt.
  Unbekannter Abschluss meldet den Zielslot und erhält I/O-Speicher bis Prozessende,
  ohne einen Hop zu bestätigen. Dieser defensive Ausnahmezweig ist kein Erfolgs-
  oder Retrynachweis und tritt laut synchronem Guardvertrag nicht regulär auf.
- Nur die erlaubte Featurezeile `Wdk_Storage_FileSystem` und private
  `mod directory_rename;` sind additiv registriert. Kein weiterer öffentlicher
  Verbraucher- oder Protokollvertrag verändert.

Die vorhandenen Guardrechte, FILE_SHARE_READ, NoFollow, volle 128bit FileID mit
Volume-ID und die frische Zielbestätigung bleiben unverändert. Hosttrash persistiert
beide Held-Slots vor Capture; SHA/Länge, Root-ID, Originalprüfung, Collision-Rollback,
Restartlokalisierung und erneutes Restore bleiben in den bestehenden Callern.
Keine Fixture-/Assertionsänderung, keine Shelloperation oder native Windows-Bin-Behauptung.

## Eigener statischer Self-Review

API-Signaturen, Unionfelder, Klasse, DLL und Completion-/Fehlerrückgaben sind mit
Primärseiten und dem gepinnten Binding abgeglichen. Der reine Textvergleich bestätigt:
Cargo nur erlaubte Addition, Modul nur erlaubte Registrierung, Capture-/Restore-Guards
und die komplette Ziel-Identitätsbestätigung in quarantine unverändert. Die gelesenen
Hosttrash-/Restore-/Recyclecaller und Fixtures sind identisch zu den vorigen Lesungen.
Eigene Rustdelimiter, Whitespace und Dateigrößen wurden statisch geprüft.
Neuer Adapter: 121 Zeilen / 5212 Bytes; quarantine: 223 Zeilen / 7905 Bytes.
Keine lokale Ausführung oder Rust-Typ-/Compilerprüfung.

## Bestehende Abnahmesignale

Nur dieselbe Root-eigene vollständige Remote-RV1-Suite bewertet diese bestehenden
Symbole; ihre Assertions und Hopdiagnosen bleiben erhalten:

- `host_trash::review_task_tests::review_task_host_trash_intent_precedes_capture_and_survives_restart`
- `host_trash::review_task_tests::review_task_host_trash_restore_restart_hop_has_durable_mapping`
- `host_trash::review_task_tests::review_task_host_trash_restore_preserves_collision_and_is_retryable`
- `host_trash::review_task_tests::review_task_host_trash_changed_payload_is_never_restored`
- `host_trash::review_task_tests::review_task_host_trash_private_records_and_partial_intents_keep_other_entries`
- `host_trash::review_task_tests::review_task_host_trash_replaced_root_is_not_a_restore_target`
- `local_access::directory_handle_tests::review_task_recycle_quarantine_moves_only_to_a_free_anchored_name`
- `local_access::directory_handle_tests::review_task_recycle_quarantine_restore_never_replaces_a_new_child`
- `analytics::os::checked_recycle::tests::review_task_recycle_publication_failure_restores_without_replacing`

Erwartet: erster Capture mit bestätigter Objektidentität; keine Ersetzung fremder
Original-/Heldnamen; Restart findet jeden vorab gespeicherten Hop; nach Collision
bleibt derselbe Record erneut restaurierbar; veränderte Bytes oder Root-ID bleiben
verweigert; Publikationsfehler restauriert ohne Ersetzung mit seinem tatsächlichen Fehler.

## Exaktes Dateiinventar

Gelesen (nur freigegebene Dateien; Local-FS-Referenz nur relevanter Windowsabschnitt):

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-4-h-analysis.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fourth-fixes.md`
- `/tmp/rv1-ci-fourth/h-analysis.json`
- `docs/refs/windows-held-rename.md`
- `docs/refs/local-fs-identity-durability.md`
- `native/Cargo.toml`
- `native/src/local_access/mod.rs`
- `native/src/local_access/os/windows/directory.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/directory_tests.rs`
- `native/src/local_access/os/windows/quarantine.rs`
- `native/src/local_access/os/windows/mod.rs`
- `native/src/host_trash/os/windows.rs`
- `native/src/host_trash/os/shared/restore.rs`
- `native/src/host_trash/os/shared/review_task_tests.rs`
- `native/src/analytics/os/shared/checked_recycle.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Wdk/Storage/FileSystem/mod.rs`:
  nur NtSetInformationFile-cfg/Signatur, FILE_RENAME_INFORMATION/Union und Klasse10.
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/System/IO/mod.rs`:
  nur IO_STATUS_BLOCK/Union.
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/Foundation/mod.rs`:
  nur RtlNtStatusToDosError, HANDLE/NTSTATUS/BOOLEAN, STATUS_PENDING, WAIT_OBJECT_0/WAIT_FAILED.
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/System/Threading/mod.rs`:
  nur WaitForSingleObject-Signatur und INFINITE.
- Eigene neue `native/src/local_access/os/windows/directory_rename.rs`.
- Eigene neue `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-H-ANALYSIS.md`.
- Workflowskill `/root/.codex/skills/arbeitsweise/SKILL.md`; Repositoryinstruktionen aus dem Auftrag.

Gelesene externe Primärseiten, aktualisiert 2026-10-04 (Syntax lokal in der oben
genannten Rename-Referenz gesichert):

- [FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)
- [SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)
- [FILE_RENAME_INFORMATION](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information)
- [NtSetInformationFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile)
- [IO_STATUS_BLOCK](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/ns-wdm-_io_status_block)
- [RtlNtStatusToDosError](https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-rtlntstatustodoserror)
- [WaitForSingleObject](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject)
- [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
- [NtCreateFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntcreatefile)

Geändert:

- `native/src/local_access/os/windows/quarantine.rs`
- `native/src/local_access/os/windows/mod.rs` – nur private additive Registrierung.
- `native/Cargo.toml` – nur erlaubte Windowsfeature-Addition mit Erklärung.
- `docs/refs/windows-held-rename.md`

Erstellt:

- `native/src/local_access/os/windows/directory_rename.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-H-ANALYSIS.md`

## Offene Grenzen und Handoff

Keine zusätzliche notwendige Source-/Feature-/Registrierungsfreigabe offen.
V-LOCAL behält die getrennte Private-Write-/Sicherheitsrechtegrenze; hier keine
parallele Rechteänderung. Root übernimmt Integration, Kandidatenformatierung,
Commit/Push, Graph und dieselbe Remote-RV1-Suite. Deren tatsächliche Bestätigung
von Capture, Restore, Collision und Restart ist noch offen; Error87 wird vor
diesem Lauf nicht als erfolgreich abgenommen bezeichnet. Eigener Scope abgeschlossen.
