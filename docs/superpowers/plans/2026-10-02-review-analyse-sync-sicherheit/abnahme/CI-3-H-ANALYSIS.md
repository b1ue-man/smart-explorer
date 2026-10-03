# CI-3-H-ANALYSIS – Windows Hosttrash Restore, Collision, Restart

Stand: 2026-10-03. Ausschließlich die drei benannten Diagnosen in
`/tmp/rv1-ci-third/h-analysis.json`, Run 37157166735, Kandidat
`71a8ca45697272453c213c9b0b5412d0cbed0f71`. Verbindlicher Scope:
`../scopes/ci-3-h-analysis.json`. Kein neuer Projektreview und keine Ausführung.

## Eigener Stage-eins-Plan

Der unveränderliche Record besitzt Originalpfad-Komponenten, Root-ID, FileID,
Länge, SHA-256 sowie beide Held-Slots. `Store::persist` wird vor dem ersten
Capture-Rename ausgeführt. Restore wechselt zunächst in den anderen bereits
gespeicherten Slot, dann an den Originalort; Fehler rollen ohne Ersetzen zurück
oder behalten Record und Payload. Dieser Ablauf und die private Store-Fassade
bleiben erhalten.

Die Windows-Grenze benutzt durchgehend denselben `rename_no_replace`-Helper.
Er übergibt absolute UTF-16-Ziele ohne expliziten NUL; u64-Padding ist kein
Stringterminierungsvertrag. `locate` bestätigt am Original bisher nur FileID,
und Restore meldet dann sofort `AlreadyAtOriginal`. Die vorhandenen Diagnosen
belegen NotFound in zwei Abläufen und unerwarteten Erfolg bei Collision, noch
keine genaue Kernelursache.

## Zweite konkrete API-Klärung und Stage-zwei-Plan

Aktuelle freigegebene Microsoft-Definitionen und deren Grenzen sind vor jeder
Sourceänderung in `docs/refs/windows-held-rename.md` gesichert. Der dokumentierte
Zielordner-/Child-Vertrag wird mit unverändertem DELETE-Sourcehandle konsumiert.
Keine neue NT-FFI, kein freier Rename-/Delete-Fallback, keine Rechteänderung.

| Kohäsiver Schritt | Dateien | Erwartung derselben Remote-RV1-Suite |
| --- | --- | --- |
| Rename-Ziel eindeutig an gehaltenen Ordner binden | `local_access/os/windows/quarantine.rs` | Childnamen validiert, NUL explizit, Bytegrößen geprüft, NoReplace bleibt false; das frisch NoFollow-geöffnete Ziel besitzt die volle erwartete FileID/Volume-ID. |
| Erfolg am Original tatsächlich prüfen | `host_trash/os/windows.rs`, `host_trash/os/shared/restore.rs` | `AlreadyAtOriginal` setzt eine erneute Root-ID-/NoFollow-/FileID-/Längen-/SHA-Prüfung voraus; fremde oder geänderte Daten erzeugen keinen Erfolg. |
| Belegrelevante Hop-Diagnostik in vorhandenen Fixtures | `host_trash/os/shared/review_task_tests.rs`, Testadapter in `host_trash/os/windows.rs` | Capture, beide gespeicherten Slots, Restart und Restore lassen sich anhand tatsächlicher Handlepfade und Objektidentitäten zuordnen; sämtliche alten Assertions bleiben. |
| Eigener statischer Abschluss | eigener Bericht | Exaktes Inventar, Source-/Assertionvergleich, ausgewogene Klammern und Dateien unter 500 Zeilen/50 KiB; keine lokale native Ausführung. |

Root-ID, NoFollow, bestätigter DELETE-Handle, FILE_SHARE_READ, NoReplace, beide
durable Held-Zuordnungen und erneute Restore-Ausführbarkeit bleiben harte
Grenzen. V-LOCAL besitzt den parallelen Sicherheits-/Rechteanschluss.

## Abschluss

Der Quellenanschluss ist umgesetzt. `rename_no_replace` erhält jetzt den
gehaltenen Ziel-DirectoryHandle und genau einen validierten Childnamen.
`FILE_RENAME_INFO.RootDirectory` verweist auf dessen tatsächlichen Filehandle;
`ReplaceIfExists = 0` bleibt unverändert. Der Buffer besitzt einen expliziten
UTF-16-NUL außerhalb `FileNameLength`; die Bytegröße enthält diesen NUL und wird
weiterhin mit geprüfter Arithmetik berechnet. Beide Capture-Varianten,
`QuarantinedChild::restore` und `move_to` konsumieren diese Grenze.

Nach dem API-Aufruf wird das Ziel über `open_regular_child` ohne Linkfolge geöffnet
und dessen vollständige FileID/Volume-ID mit dem weiterhin reservierten
DELETE-Sourcehandle verglichen. Der Vergleich liest die Identität des lebenden
Guardhandles nach dem Rename. Erfolg erfordert damit ein tatsächlich bestätigtes
Ziel, zusätzlich zum API-Status. Ein Bestätigungsfehler liefert einen Fehler mit
konkretem Zielort und lässt die Payload erhalten; die beiden bereits dauerhaft
gespeicherten Vorgänger-/Nachfolger-Slots bleiben ihre Wiederanlaufzuordnung.

`restore_in` prüft im Original-Zweig erneut die aufgezeichnete Root, NoFollow,
FileID, Länge und SHA über `verify_original`. Nur die erwartete Payload kann
`AlreadyAtOriginal` liefern. Die Katalogsuche selbst bleibt unverändert; sie
bekommt keinen zusätzlichen vollständigen Dateihash beim Auflisten.

Die drei benannten Bestandsfixtures protokollieren Intent-ID, gespeicherte
Root-/File-ID, den Sourcehandlepfad und die frisch geöffneten Original-/Held-
Slotpfade samt tatsächlicher FileID/Volume-ID und Länge. Stage-Marker umfassen
Intent-Persist, Capture, Store-Reopen, Restorehop, Collision und Retry. Es gibt
keine neue Testfunktion, keinen Skip und keine Setup-Abschwächung.

## Entscheidungen und API-Delta

- Der dokumentierte Zielordner-/Child-Vertrag ersetzt die absolute Stringübergabe;
  keine neue NT-FFI und kein freier Move-/Delete-/Copy-Fallback.
- Guardzugriffe, `FILE_SHARE_READ`, NoFollow-/Regular-/Hardlinkprüfung,
  ursprünglicher Objektvergleich und Root-ID-Recheck bleiben unverändert.
- `Record`, `Store::persist/load`, `locate`, beide vorab durable Held-Slots sowie
  Restorehop und NoReplace-Rollback bleiben unverändert. Fehler löschen keinen
  Record und keine Payload; auch der zweite Slot bleibt nach Neustart auffindbar.
- Private interne Signatur:
  `rename_no_replace(file: &File, target: &DirectoryHandle, name: &OsStr) -> io::Result<()>`.
  Kein Reexport oder fremder Consumeranschluss.
- Additive interne OS-API:
  `host_trash::platform::verify_original(record: &Record) -> io::Result<()>`.
  Bestehender `restore_in` konsumiert sie ausschließlich im Original-Zweig.
- Nur `cfg(test)`:
  `host_trash::platform::test_path(file: &File) -> io::Result<PathBuf>` und der
  Fixturehelper `hop<T>(&Fixture, &str, io::Result<T>) -> io::Result<T>`.

Die fehlende explizite Terminierung und der ungeprüfte Original-Erfolgszweig sind
im Source belegt. Welche konkrete Kernel-/Bufferauswertung die drei Fehler im
Run erzeugt hat, bleibt ohne erneuten Runnerbeleg offen; eine behobene
Laufzeitursache wird hier nicht behauptet.

## Eigener statischer Self-Review

Textvergleich gegen die unmittelbar vor der Sourceänderung gesicherten Dateien:
Windows-Adapter außerhalb `verify_original` und `test_path` identisch; Restore
außerhalb seines Original-Zweigs identisch. Sämtliche Guardrechte und ursprüngliche
Objektidentitätsprüfungen identisch. Alle alten Assertion-Makros und Testnamen
wortgleich erhalten; nur die drei benannten Testfunktionen erhielten Diagnostik.
Klammern nach Kommentar-/Stringbereinigung ausgeglichen, keine nachgestellten
Leerzeichen. Keine Rust-Kompilierung oder Laufzeitprüfung.

| Geänderte Rust-Datei | Zeilen | Bytes |
| --- | ---: | ---: |
| `native/src/local_access/os/windows/quarantine.rs` | 256 | 9574 |
| `native/src/host_trash/os/windows.rs` | 438 | 14669 |
| `native/src/host_trash/os/shared/restore.rs` | 52 | 2065 |
| `native/src/host_trash/os/shared/review_task_tests.rs` | 339 | 10921 |

## Erwartete Signale derselben Remote-RV1-Suite

- `host_trash::review_task_tests::review_task_host_trash_intent_precedes_capture_and_survives_restart`:
  OriginalPresent vor Capture, genaue erste Slotzuordnung, Held nach Store-Reopen,
  Restored und unveränderte Originalbytes.
- `host_trash::review_task_tests::review_task_host_trash_restore_restart_hop_has_durable_mapping`:
  bestätigter erster Capture, zweiter Slot nach unterbrochenem Hop als
  RestorePending auffindbar, erneuter Restore mit Originalbytes.
- `host_trash::review_task_tests::review_task_host_trash_restore_preserves_collision_and_is_retryable`:
  Restore meldet Collision, fremde Bytes bleiben erhalten, Held bleibt
  wiederherstellbar, Retry nach Entfernen der fremden Datei liefert Originalbytes.

Alle weiteren vorhandenen Inhalts-, Rootersatz-, Link-, privaten Record-,
Paging- und Reservierungsassertions bleiben unverändert. Es wurde keine
zusätzliche Suite angelegt oder ausgelöst.

## Exaktes Dateiinventar

Gelesen (einschließlich Scope und eigener neuer Dokumentation):

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-3-h-analysis.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-closure-fixes.md`
- `/tmp/rv1-ci-third/h-analysis.json`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/local-fs-identity-durability.md`
- `native/src/host_trash/mod.rs`
- `native/src/host_trash/core/record.rs`
- `native/src/host_trash/os/shared/restore.rs`
- `native/src/host_trash/os/windows.rs`
- `native/src/local_access/os/windows/quarantine.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/directory_identity.rs`
- `native/src/local_access/os/windows/create.rs`
- `native/src/host_trash/os/shared/review_task_tests.rs`
- `native/src/host_trash/os/shared/store.rs`
- `native/src/host_trash/os/shared/catalog.rs`
- `native/src/local_access/directory_handle_tests.rs`
- `docs/refs/windows-checked-recycle.md`
- `docs/refs/windows-held-rename.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-H-ANALYSIS.md`

Extern gelesen, ausschließlich die im Scope freigegebenen Primärquellen:

- [windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)
- [windows/win32/api/winbase/ns-winbase-file_rename_info](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)
- [windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew)
- [windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile)

Geändert:

- `native/src/local_access/os/windows/quarantine.rs`
- `native/src/host_trash/os/windows.rs`
- `native/src/host_trash/os/shared/restore.rs`
- `native/src/host_trash/os/shared/review_task_tests.rs`

Erstellt:

- `docs/refs/windows-held-rename.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-H-ANALYSIS.md`

## Offene Fremdgrenzen

Keine fehlende Definition und keine neue API-/Scope-Anfrage. Die parallelen
Windows-Sicherheitsrechte gehören V-LOCAL und wurden hier nicht geändert.
Root integriert beide Blöcke und führt nur die bereits bestehende vollständige
Remote-RV1-Suite aus. Deren erneuter Windows-Nachweis ist noch ausstehend.
Keine lokale native Ausführung, Formatter, Git, CI, Graph, Release oder Agenten.
