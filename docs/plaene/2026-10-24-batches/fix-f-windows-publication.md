# M01 — Windows-Dateiveröffentlichung

Stand 2026-10-26; Autorzyklus begonnen auf `33257f82`, Remote-Fehlerevidenz von
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8`. Kein Laufzeitpass und keine
Releaseaussage.

## A — Erstplan und Autorisierung

Der delegierte Ergebnisauftrag M01 verlangt funktionierende normale
Windows-Dateiveröffentlichung, direkte Verschiebung und Auftragswiederaufnahme
mit ehrlichem Haltbarkeitsbeleg. Quellen, Backups und Quarantäne bleiben bis
zum bestätigten Abschluss erhalten. Die Nutzervorgabe vom 2026-10-26 verlangt
Fehlerfunktionsliste, direkte und indirekte Verknüpfungskarte, konkreten
Erhaltungsplan, Fix und eigene anschließende Anti-Regressionsreview desselben
Fixautors; Wortlaut: `/tmp/smartexplorer-final-review-user-contract.txt`.

Der explizite Scope steht in
`/tmp/smartexplorer-f-windows_publication-scope.json`; die von Main ergänzten
Graphify-/Arbeitsweiseanleitungen, Stage-/Dateisystem-/Persistenzgrenzen,
VFS-Weiterleitungen und konkreten Sync-/Backup-/Mount-Verbraucher sind dort
einzeln zum Lesen aufgeführt. Die Schreibfläche bleibt auf die dort genannten
M01-Dateien und ausdrücklich begrenzten Re-Export-/Replay-/Linuxanschlüsse
beschränkt. Keine lokale Ausführung
von Builds, Compilern, Tests, Formattern, Servern, Installationen, Unteragenten,
Commits, Pushes, Graphupdates oder Releasebefehlen. Main besitzt Integration,
gemeinsame Fallregistrierung und denselben Remote-Suiteeinstieg.

Promptwissen: F10–F12, F16-Windows, F44, F48–F49 und F51–F56 sind tatsächliche
Remote-Beobachtungen; mehrere Beobachtungen können dieselbe Ursache haben.
Die konkrete Windows-Aufrufgrenze und ein ausreichender Haltbarkeitsweg sind
vor der Codelektüre unbekannt. Die Grundfunktion besteht darin, veröffentlichte
Bytes und ihren Namen ausreichend zu bestätigen, bevor verbleibende
Wiederherstellungsdaten gelöscht werden. Bestehende Copy-/Move-/Retrywege,
Konfliktentscheidungen, Rechte, Linkgrenzen und Persistenz bleiben erhalten.
Vorläufige Planungstiefe 2: bestehende gekoppelte Windows-/Copy-/VFS-/Recovery-
Verträge; keine neue eigenständig nutzbare Funktion.

## B — Gesicherter Bestand und erste Planung

Die Dokumentations-Kontextprüfung ergab `33257f82` als HEAD und ausschließlich
die von Main betreuten ungetrackten Graphnebenprodukte als Worktreebestand.
Architektur und `rv1-funktionsplan.md` wurden vor der Quelländerung gelesen.

`local_access/os/windows/publication.rs::confirm_namespace` öffnet den
gehaltenen Parent mit `ReOpenFile(FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES)`
und ruft `NtFlushBuffersFileEx(flags=0)` auf. Die Quellkommentare behandeln
diesen Weg als Bestätigung des veröffentlichten Verzeichniseintrags.
Die Remote-Diagnosen zeigen nach bereits erfolgtem Move `Access is denied
(os error 5)`; F12 erreicht deshalb seinen beabsichtigten Receiptfehler nicht.
Dies beweist weder einen Fehler der Konfliktentscheidung noch einen
gesicherten Nichtpublikationszustand.

Erste Arbeitsgliederung: den konkreten Windows-Haltbarkeitsvertrag und die
vollständige zugewiesene Aufruferkette klären; einen belegten Ersatzweg unter
Erhalt aller Schutzentscheidungen planen; ausschließlich die dafür notwendigen
Adapter und fokussierten Fälle ändern; die vollständige eigene Änderung
statisch gegen Quelle und Primärverträge nachprüfen; Main den exakten
Registrierungsbedarf desselben Remote-Einstiegs melden.

## C — Entscheidungsrecherche

Quellenabgleich am 2026-10-26:

- [ReOpenFile](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-reopenfile)
  beschreibt das Wiederöffnen mit geänderten Rechten und verlangt einen durch
  `CreateFile` erzeugten Ursprung. Die Quelle bestätigt keine allgemeine
  Eignung für unsere nativen Child-Directoryhandles.
- [Wines Konformitätsquelle, Commit 74e2da14e6b45b906bf0ffba46a9cae1a5f73f0f](https://github.com/wine-mirror/wine/blob/74e2da14e6b45b906bf0ffba46a9cae1a5f73f0f/dlls/kernel32/tests/file.c#L5144)
  prüft in `test_ReOpenFile` ausdrücklich den verweigerten Directory-Reopen
  trotz Backup-Semantik. Dies ist selbst gelesener externer Quellcode, kein
  von diesem Autor ausgeführter Windows-Konformitätslauf.
- [Microsoft WSL, Commit 63fdf785d238d32d00914811d0c39531646ebbaf](https://github.com/microsoft/WSL/blob/63fdf785d238d32d00914811d0c39531646ebbaf/src/windows/common/filesystem.cpp#L1725)
  implementiert `filesystem::ReopenFile` durch einen leeren relativen
  Unicode-Namen, `RootDirectory=Handle`, `FILE_OPEN` und `NtCreateFile`.
  Dies ist der herstellereigene Implementierungsbeleg für den Ersatzweg;
  die genaue Projektausführung bleibt remote zu bestätigen.
- [NtCreateFile, Win32](https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntcreatefile)
  beschreibt die relative Rootgrenze, bestehendes Öffnen mit `FILE_OPEN`,
  synchrone Directoryoptionen und das benötigte Synchronisationsrecht.
  Neue Attribute und DACLs werden beim bestehenden Öffnen nicht angewandt.
- [File Access Rights Constants](https://learn.microsoft.com/en-us/windows/win32/fileio/file-access-rights-constants)
  beschreibt `FILE_ADD_FILE = 2` als Directoryrecht und als Directorybedeutung
  des Dateischreibdatenbits. Der Adapter fordert dieses schmale Recht,
  Attributlesen und Synchronisation an. Die ebenfalls gelesene
  [ZwCreateFile-Dokumentation](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-zwcreatefile)
  bestätigt `FILE_OPEN` und die synchronen Directoryoptionen. Die tatsächliche
  native Annahme dieser konkreten Kombination ist kein lokal erbrachter Lauf.
- [NtFlushBuffersFileEx](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntflushbuffersfileex)
  beschreibt normale Flushs mit Flags 0, Schreib-/Appendzugriff, Daten und
  Metadaten samt Storage-Synchronisierung sowie NTFS/ReFS/FAT/exFAT als
  unterstützte Dateisysteme. Teilflushflags ersetzen diesen Abschluss nicht.
- [FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
  verlangt Administratorrechte für den Volumeflush. Der bestehende Broker in
  `local_access/os/windows/broker.rs` bietet Lesezugriff; er ist kein bereits
  vorhandener privilegierter Flushdienst. M01 führt keinen solchen Dienst ein.
- [CreateFileW, Caching Behavior](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew#caching-behavior)
  beschreibt die Wirkung einer Write-through-Anfrage auf deren
  NTFS-Metadatenänderungen. Daraus wird keine allgemeine FAT-/exFAT-
  Namespacegarantie abgeleitet.
- [MS-FSA 2.1.5.7](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/0de7dc40-9627-437e-a4df-c4696cdc3d02)
  beschreibt Directorystrukturpersistenz; die zugehörige
  [Produktnote 80](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4e3695bd-7574-4f24-a223-b4679c065b63)
  beschränkt genau diese Wirkung auf NTFS und warnt vor wirkungslosem Erfolg
  der Directoryoperation auf nicht-NTFS. Allgemeiner Flushsupport und ein
  bestätigter Parent-Namespace sind deshalb getrennte Aussagen.
- [FILE_RENAME_INFORMATION](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information)
  bindet die vorhandene Renameoperation an das gehaltene Zielroot. Der Fix
  ersetzt weder die No-replace-Grenze noch die readonly-Ersetzungsbehandlung.

Die erwogene weitere Fehlerursache `windows.rs::open_staged` ist verworfen:
`staging.rs::stage_copy` nimmt `copy_by_path` nur bei `!durable`. Move und
Overwrite verwenden ihren bereits schreibfähigen Stagehandle. Der bloße
read-only Reopen in einem hiervon getrennten schnellen Kopierweg ist kein
Beleg der beobachteten Haltbarkeitsfehler und wird nicht geändert.

`support_dirs::write_private_atomic` flusht seine private Stage vor dem
Write-through-Ersatz. `creds/os/private_storage_windows.rs::sync_directory`
ist ein ausdrücklich dokumentierter No-op; F48 wird nicht als Fehler dieses
nicht aufgerufenen Directory-ReOpenFile-Pfades ausgegeben. F48s bestehende
Jobprüfung kann über `verify_saved` den defekten Dateipublikationsbeleg
aufrufen. Die tatsächliche Bestätigung bleibt im selben Remote-Einstieg.

Erste Entscheidung: `ReOpenFile` durch native Wiederöffnung desselben
gehaltenen Directoryobjekts ersetzen, Rechte auf den wirklich benötigten
Schreib-/Metadaten-/Synchronisationsumfang begrenzen und sämtliche Fehler
vor Cleanup weitergeben. Kein Pfad-Reopen als Ersatzautorität. Die zweite
Lückenprüfung hat die Trennung zwischen einem konkreten bestätigten
Write-through-Publikationsauftrag und einer allgemeinen Parentbestätigung
auf nicht-NTFS geklärt. Main hat diese Primärquellengrenze und die dazu
erforderlichen engen Adapter-/Replayanschlüsse ausdrücklich freigegeben.
Ein ignoriertes `PermissionDenied`, ein wirkungsloser Flush oder ein Test-Skip
erfüllt M01 nicht.

### Direkte und indirekte Verknüpfungskarte

Der AST-Graph enthält die tatsächlichen Definitionen der nachfolgenden
Windows-/Copy-/VFS-Funktionen. Die breite Graphabfrage ist wegen generischer
Begriffe unzureichend; die fokussierte Graphlektüre wurde durch eigene
Quelllektüre der angegebenen Definitionen und Weiterleitungen ergänzt.
Kein Graphupdate wurde ausgeführt. Die CLI meldete Skillstand 0.9.11 und
Paketstand 0.9.17; Installation ist in diesem Scope nicht erlaubt.
Die Abfrage enthielt keine neue Extraktion oder semantische Agentenarbeit;
die CLI hat keine Tokennutzungszahl für diese Query gemeldet.

| Einstieg und direkte Kette | Indirekte Verträge und Fehlerwirkung |
|---|---|
| `copy/mod.rs` wählt `copy/os/windows.rs`; `safe_file::transfer_local[_observed]` → `transfer_local_inner` → `staging::stage_copy` → `platform::commit_staged` → `publication_proof::confirm_handle_publication` → `platform::confirm_live_publication_namespace` | Neue Kopien behalten den vorhandenen nicht synchronisierten Schnellweg. Move/Overwrite behalten private Stage, ursprüngliche Quellenbeobachtung, Dateiflush, Konfliktentscheidung, optische Priorität und Abbruch. Windows hält den konkreten erfolgreichen Write-through-Auftrag, Linux denselben tatsächlichen Parent-FD. Keine Stage oder Quarantäne wird nach fehlendem Publikationsbeleg als erfolgreich bereinigt. |
| `safe_file::direct_move::try_move` → `PublicationAlias::create` beziehungsweise gehaltene Quarantäne → `open_live_move_source` → `preflight_direct_move` → `platform::move_file` → `validate_target` → `confirm_handle_publication` → `publication::after` → `remove_quarantine` → `finish_direct_move` | Hardlinks bleiben optional; FAT-/exFAT-artiger direkter Rename kopiert keine Bytes. Abbruch vor Publikation stellt zurück. Unbekannter Renameausgang erhält den Zielbestand. Ein Receiptfehler nach Erfolg erhält Ziel und vorhandene Quarantäne. Konflikte Skip/Rename/Overwrite und Cross-device-Fallback bleiben bestehen. |
| `publication::before/preparation_failed/commit_failed/after` ↔ `transfer::file_recovery::LocalObserver` | Vorbereitung und durable Rücknahme binden die genaue Stage, das genaue Ziel und die Quelle. `confirm_unpublished` verlangt gehaltenes Stageobjekt, aktuelle Parentidentitäten und zwei lokale Providerprüfungen; reine Fehlerrückgabe oder ähnliche Bytes beweisen Nichtpublikation nicht. |
| `file_recovery::after_checked` → `confirm_transfer_publication_handle`; `file_recovery_verify::verify_saved` → `publication_state` → strikt `confirm_transfer_publication` oder bei dauerhaft positiver Quittung `confirm_acknowledged_transfer_publication` → `recover_transfer_quarantine` | Digest und stabile Objektidentität vor und nach Flush, Parent-/Namensbindung und alle Receiptprüfungen bleiben vor Quellen-/Stagebereinigung. Unbestätigter Windows-Replay → `platform::confirm_publication_namespace` → `ConfinedDirectory::confirm_publication_namespace` → `publication::confirm_namespace`. Keine Änderung von Serdeform, Journal-ID, Endpointbindung oder Rootreservierung; M02 besitzt seine getrennte Aliasursachenkorrektur. |
| `job_store::prepare/JobRun`, `job_store_catalog::reopen_job`, `job_store_validation`, `missing_retry::resume` → `verify_saved`, `verify_sources`, `record_io::write_record` → `support_dirs::write_private_atomic` | Missing-only behält abgeschlossene Paare und arbeitet nur Restarbeit ab. Private Recordrechte, Sealing und die vorhandene Fileflush-/Write-through-Reihenfolge bleiben erhalten. Der private Directory-No-op ist kein nachgewiesener Parentflush. |
| `vfs::confirm_namespace` → `extension_calls::confirm_namespace` → `BackendExtensions::confirm_namespace` → `local_extensions` → Windows `local_platform::confirm_namespace` beziehungsweise `confined_local_extensions` → Windows `confined_local::confirm_namespace` | Gewöhnliche lokale Pfade und physisch gehaltene Confinedroots behalten ihre bisherige PerFile-/Write-through-Qualifikation. Diese positive Rückgabe ist kein unabhängiger Beleg für einen verlorenen Auftrag. Der neue strikte Transfer-Replay ruft die gehaltene Directorybestätigung direkt auf. Deferred-Dateiinhalt und `sync_filesystem` bleiben gesondert; Providerlocators werden nicht lokal umgedeutet. |
| `ConfinedDirectory` → `directory_handle::DirectoryHandle::open_root` und `child` → `confined_native`, `directory_identity`, `regular::validate_file` | Gepinnte physische Vorfahren, verweigertes Delete-Sharing, literal einteilige Namen, umleitende Reparsepunkte, Spezialdateien und volle 128-Bit-Identitäten mit bestehendem Legacyfallback bleiben erhalten. Wiederöffnung darf weder Schreibschutzattribute/DACLs ändern noch Rootpfade neu auflösen. |
| `durability::finish_direct_move/finish_staged_commit`, `move_guard::quarantine_source_observed/remove_quarantine` und `copy::recovery` → Windows `sync_parent` beziehungsweise `recover_expected_file` | Die Transferbereinigung bleibt objektgebunden und nach bestätigtem Ziel. Live- und positiv quittierte Windows-Publikationen verwenden ihren konkreten Operationsabschluss; unbestätigter Transfer-Replay benötigt einen wirklichen Parentbeleg. Quellparentfehler bleiben von Zielpublikationsfehlern unterscheidbar. Die gesonderten bestehenden Backup-/Syncqualifikationen sind in D vollständig aufgelistet. |
| Bestehende F10–F12-/F16-/F44-/F48–F49-/F51–F56-Fixtures | Ihre Assertions werden nicht abgeschwächt. F12 ist ein verdeckter Receiptfehlerfall, kein bewiesener Receiptdefekt. F52–F56 zeigen teilweise nur einen Fehlerzähler; ihre konkrete Zuordnung zur defekten Aufrufkette ist Quellableitung und wird nicht als geloggter Detailfehler erfunden. |

## D — Konkreter endgültiger Erhaltungsplan

Die folgenden zusammenhängenden Punkte sind nach der zweiten Lückenprüfung
vor der ersten Quelländerung endgültig geplant. Main hat bestätigt, dass die
bisherige Planung keinen allgemeinen nicht-NTFS-Namespacebeweis enthält.

1. `local_access/os/windows/publication.rs`: bestehendes Directoryobjekt mit
   leerem relativem Namen nativ wiederöffnen; Directorytyp und gleiche volle
   Objektidentität vor dem wirklichen Flush prüfen. Disposition `FILE_OPEN`,
   kein Name, kein Attributwechsel, keine DACLänderung, kein Rename. Alle
   asynchron noch benutzten Speicherblöcke und Handles bis zur bestätigten
   Completion halten. Erwartung: der bisher ständig verweigerte NTFS-Pfad
   kann einen wirklichen flags=0-Abschluss erreichen; offene/fehlgeschlagene
   Completion bleibt ein Fehler.
2. Korrigierter Plan vor der VFS-Grenzkorrektur: Der Vergleich mit Release
   `a814f263096fe69d3746e840b5f3fad3fd7e88a0` belegt, dass
   `local_platform::confirm_namespace` den vorhandenen PerFile-/Write-through-
   Vertrag qualifiziert: Verzeichnis-Metadaten, `FsProfile`, anschließend
   unveränderter `flush_filesystem` ohne privilegierten Volumeflush. Windows
   liefert auch für FAT/exFAT `PerFile`. Diese Qualifikation wird im bisherigen
   Funktionsumfang wiederhergestellt; sie wird ausdrücklich nicht zum neuen
   allgemeinen Replaybeleg erklärt. Der erst nach dem Release hinzugekommene
   Confinedadapter behält dieselbe Qualifikation am bereits gehaltenen
   Volumehandle. Mein zwischenzeitlicher `Ok(false)`-Pfad hätte normale Sync-,
   Backup- und Mount-Recoveryvorgänge gesperrt und wird entfernt. Es gibt weder
   einen neuen verpflichtenden Administratorzugriff noch einen neuen Broker.
   Die frühere Klassifikation dieser VFS-Qualifikation als zusätzlicher
   allgemeiner Fehler ist zurückgenommen: Ein fehlender allgemeiner
   Powerlossbeweis belegt keinen neuen beobachteten Endnutzerfehler.
   Die beiden Livehandle-Publikationsfunktionen in `publication_proof.rs`
   verwenden dagegen den bereits erfolgreich zurückgekehrten konkreten
   Plattform-Publikationsauftrag, weiterhin mit Dateiflush und vollständiger
   Parent-/Namensbindung. Windows `sync_parent` bleibt ausschließlich dieser
   Operationsabschluss und wird nicht zu einem beliebigen Parentbeleg erklärt.
   Grundlage ist [MoveFileExW, MOVEFILE_WRITE_THROUGH](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw),
   am 2026-10-26 vollständig gelesen: Erfolgsrückkehr nach Bewegung auf den
   Datenträger; die bestehende Copy-/Delete-Sonderbeschreibung wird nicht als
   allgemeiner Directoryflush ausgelegt. Die erfolgreiche aktuelle FAT-/exFAT-
   Kopie und Verschiebung verliert dadurch keinen neuen Capabilityzwang.
   Unbestätigter Replay bleibt bei `confirm_transfer_publication` mit wirklichem
   Parentabschluss. Für dauerhaft bestätigte operationsgebundene Quittungen
   verwendet `confirm_acknowledged_transfer_publication` die unveränderten
   Belegfelder. Der einzige produktive Aufruf wird in `verify_saved` vor
   Quarantänebereinigung nur bei `confirmed && acknowledged &&
   publication_authorized` gewählt. Main hat diesen Anschluss und genau den
   Re-Export in `copy/mod.rs` freigegeben und M02 die Grenze mitgeteilt.
   `copy/os/linux_os.rs` erhält ausschließlich die schmale Weiterleitung des
   neuen Liveeinstiegs an seinen bisherigen gehaltenen Parent-FD; Linux
   übernimmt den Windows-Operationsvertrag nicht. Erwartung: keine erfundene
   Directorygarantie, keine pauschale Dateisystemsperre und kein Quellenabbau
   allein wegen gleicher Bytes.
3. `publication_task_tests.rs` innerhalb des vorhandenen Moduls aufnehmen:
   reale Root-/Child-Wiederöffnung, gleiche Objektidentität nach Namenswechsel
   eines explizit delete-geteilten Testhandles, falscher Objekttyp, verweigerte
   Rechte, fehlgeschlagener Flush sowie Copy-/Move-Erhaltung nach injiziertem
   Abschlussfehler. Der VFS-Fall prüft nun die erhaltene Qualifikation bei
   fehlendem allgemeinen Parentbeleg und die weiterhin strikte direkte
   Transferbestätigung; er darf die Qualifikation nicht als Flush ausgeben.
   `batches_direct_move_compatibility_task_tests.rs` bleibt
   für Hardlinkfreiheit, Konflikte, Cancel und verlorene Antworten maßgeblich.
   Erwartung: die ursprünglichen Fehlerfälle und ihre Schutzorakel bleiben in
   derselben Remote-Suite registriert; Main besitzt die Registrierung.
4. Eigene statische Nachreview gegen die gesamte Karte, versionierte Quellen,
   Schreibrechte, Typ-/Identitätsprüfungen, Callbackreihenfolge, Cancel, Pending-
   Speicher, Quell-/Quarantäne-/Backupbehalt und Dateigrößen. Die lokale
   Autorprüfung umfasst nur statische Text-/Parsingkontrollen. Erwartung:
   keine entfernte Fallabnahme, kein verschwiegenes Garantiegefälle und keine
   unzulässige lokale Nativeausführung. Offene Laufzeitfragen gehen konkret
   an Main für denselben Remoteeinstieg.

### Zweite Lückenprüfung der erweiterten Consumergrenze

Main hat die folgenden Aufrufer genau lesend ergänzt. Ihr vorhandener Vertrag
wird erhalten; sie werden in M01 nicht in einen umfassenden Sync-/Delete-
Neuentwurf umgebaut. Diese Grenzentscheidung steht vor der Korrektur der
zwischenzeitlich gesperrten VFS-Rückgabe.

| Direkte und indirekte Kette | Erhaltener Vertrag und konkrete Grenze |
|---|---|
| `apply_stage::namespace/native_namespace/require_durable` → VFS; `replacement_publish::publish`, `Staged::publish_sibling` | Die bestehenden Verbraucher werten `false` als nicht checkpointbar. Die Windows-Qualifikation bleibt nach erfolgreicher Operation verwendbar; sie beweist keinen beliebigen verlorenen Auftrag. `Deferred` bleibt unbestätigt bis zu seinem vorhandenen Abschluss. |
| `duplicate_backup::save`, `apply_transfer::back_up_captured` → private Datei + `sync_all` → `native_namespace`; `version_save::{archive,copy_private,rollback}` → `version_manifest::write` / Namespace | Backupfehler verhindern die nachfolgende Mutation; Objekt-/Inhaltsprüfung, private Rechte, Archivbehalt und create-only-Rollback bleiben unverändert. FAT/exFAT erhalten keine neue allgemeine Flushpflicht. |
| `apply_transaction::delete`, `apply_delete::delete_guarded_with_progress_and_guard`, `apply_dirs::run`, `duplicate_apply::remove_extras`, `apply_dedupe::run`, `move_finalize` → Namespace | Bestehende Delete-/mkdir-/Absentzustände werden weder zu MoveFileEx-Erfolgen umgedeutet noch mit einem neuen Adminzwang gesperrt. Genaue ID, Backup-before-delete, Cancel, Baseline- und Checkpointregeln werden nicht geändert. |
| `version_ops::{erase,maintain_member}`, `merge_inputs::remove`, `merge_recovery::{remove,restore}`, `replacement_journal::remove` → Namespace / nativer Parent | Retention und Journalbereinigung behalten exakt den Releasevertrag. Daraus wird kein zusätzlicher allgemeiner Powerlossbeweis behauptet. |
| `replacement_recovery::{recover,finish_published,erase_known}`, `merge_precheck`, `merge_keep_both`, `merge_recorded` → bestehende Namespacequalifikation | Bereits im Release vorhandene Wiederaufnahmeentscheidungen bleiben unverändert; unbekannte Bytes, fremde IDs, Wurzeln und Versionen bleiben geschützt. Der neue strikte Transfer-Replaypfad verwendet diese Boolgrenze nicht. |
| `mount::recovery_import::{copy_recovery_package,restore_recovery_cache,sync_tree_namespaces}` → Windows `recovery_storage::confirm_namespace/publish` → VFS / tatsächliches `MoveFileExW(WRITE_THROUGH)` | Der Windowsadapter nennt `confirm_namespace` ausdrücklich Qualifikation vor der Operation. Alle kopierten Dateien werden separat synchronisiert; das tatsächliche Publish liefert seinen konkreten Operationsabschluss. Originalcaches und vorhandene fremde Zielcaches bleiben erhalten. Die Funktionen existieren im Release `a814f263` noch nicht; ihre vorhandene Qualifikation wird durch M01 nicht verengt. |
| `cache_extensions`, `ipc_backend_extensions`, `fs_guard_extensions`, `vault::backend_extensions`, `agent::extensions` → VFS | Cache und IPC leiten an den wirklichen Backendvertrag weiter. Guard prüft Schreibzugriff vor und nach der Bestätigung; Vault prüft die Session und bildet den Klartextparent auf das verschlüsselte Inhaltsverzeichnis ab; Agent verwendet seinen vorhandenen Hostabschluss. Keine lokale Garantie wird über einen entfernten Provider erfunden. |
| `folder_reservation::publish` → tatsächliche reservierte Verzeichnisverschiebung → Identitätsvergleich → VFSqualifikation | M02 besitzt ausschließlich die relative Aliasursache. Persistierte Folderfelder und Wiederaufnahmeprüfungen werden von M01 nicht geändert. |
| Linux `namespace_flush::confirm_namespace` und Copy `confirm_publication_namespace` | Linux behält den gehaltenen FD und seine vorhandene tatsächliche Flushqualifikation. Die Windows-Qualifikationsentscheidung wird nicht auf Linux übertragen. |

Primärquellen bleiben die gelesenen Microsoft-Dokumente in C: Der
Volumeflush benötigt Administratorrechte; `MOVEFILE_WRITE_THROUGH` gehört
zur konkreten abgeschlossenen Moveoperation; MS-FSA begrenzt die allgemeine
Directorypersistenz auf NTFS. Keine dieser Aussagen legitimiert einen
beliebigen nicht-NTFS-Replaybeleg. Die Grenzkorrektur benötigt ausschließlich
die bereits freigegebenen beiden Windows-VFS-Dateien und die eigene Fallquelle.

## E — Umsetzung und eigene anschließende Anti-Regressionsreview

Die Implementierung und die eigene anschließende statische Review sind
einschließlich der erweiterten Consumergrenze abgeschlossen. Main hat die
zwischenzeitlich durch meinen VFS-`Ok(false)`-Pfad eingeführte Verengung
benannt. Der korrigierte Plan in D wurde vor dieser Grenzkorrektur gesichert;
die Funktionskörper beider Windows-VFS-Qualifikationen sind wieder unverändert.
Dies ist die lokale Autorabgabe, kein nativer Laufzeitpass und keine
Releaseaussage.

### Belegte Fehlerfunktionen und Fixtureeinordnung

| Funktion / Beobachtungen | Ursache und Änderung |
|---|---|
| `local_access::os::confined::publication::confirm_namespace`; strikter Replay einschließlich F51 | Der Directory-ReOpenFile-Aufruf passt zur gelesenen externen Konformitätsquelle und zu `Access denied` im Remotelauf. Er wird durch den leeren nativen Objekt-Reopen ersetzt. Volumen, physischer Directorytyp und volle Objektidentität werden geprüft. Nur ein wirklicher bestätigter lokaler NTFS-Directoryflush ergibt den allgemeinen positiven Parentbeleg. |
| `confirm_handle_publication`, `confirm_transfer_publication_handle`; F10–F12, F16-Windows, F44, F49 und frische Applypfade in F52–F56 | Livepfade verlangten nach dem konkret erfolgreichen Write-through-Move zusätzlich den defekten allgemeinen Directoryabschluss. Der Plattform-Liveeinstieg erhält Windows-Operationserfolg, eigenständigen Dateiflush und erneute Parent-/Namensbindung. Hardlinkfreie Direktmoves bleiben ohne Bytekopie erreichbar. |
| `file_recovery_verify::verify_saved`; F48, F51–F56 | Replay unterscheidet den dauerhaft positiv quittierten konkreten Auftrag von einem nur vorbereiteten oder verlorenen Ausgang. Die positive Variante revalidiert weiterhin Identität, SHA-256, Dateiabschluss und aktuelle Bindung; die unbestätigte Variante verlangt weiterhin den wirklichen Parentabschluss. Kein Receiptfeld und kein Format werden verändert. |
| Beide Windows-VFS-Funktionen `confirm_namespace` | Meine erste Änderung hätte durch `false` auf Nicht-NTFS/Remote die bestehenden Sync-/Backup-/Mount-Verbraucher gesperrt. Diese eingeführte Verengung ist entfernt. Der Releasevergleich belegt die vorhandene Qualifikation, keinen beobachteten zusätzlichen allgemeinen VFS-Fehler. Nur die Kommentare werden präzisiert; der direkte strikte Transferbeweis bleibt davon getrennt. |

F12 erreichte seinen beabsichtigten Receiptfehler wegen des davor scheiternden
Abschlusses nicht. Seine Assertion bleibt unverändert; es liegt kein Beleg
eines falschen Fixtureorakels vor. F52–F56 enthalten teilweise nur den
Fehlerzähler als Diagnose: die gemeinsame Aufrufkette ist aus dem gelesenen
Applycode abgeleitet, kein zusätzlicher erfundener Logeintrag. F48 kann beim
Öffnen seines vorhandenen Journals in denselben Replaypfad laufen; der
private Recordwriter selbst ruft keinen Directory-ReOpenFile auf. Die
endgültige Zuordnung bestätigt derselbe Remoteeinstieg. Für M01 wurde kein
Fixturefehler anstelle eines Produktfehlers behauptet und keine vorhandene
Assertion entfernt. Die read-only-Stage-Hypothese ist in C widerlegt.
F16-Linux und die relative Ordneraliasursache bleiben M02 zugeordnet.

### Eigene anschließende Anti-Regressionsreview

| Grenze | Selbst geprüfter Erhalt und Nachweisgrenze |
|---|---|
| Wiederöffnung und Rechte | `FILE_OPEN`, leerer Name, gehaltenes Rootobjekt; kein Erstellen, Ersetzen oder Diagnosepfad-Reopen. `FILE_ADD_FILE`, Metadatenlesen und Synchronisation ersetzen unnötige generische EA-/Attribut-/DACLrechte. Physischer Directorytyp und volle Objektidentität werden erneut geprüft. Keine ACL oder readonly-Attribute werden geändert. Die tatsächliche Annahme dieses schmalen Zugriffs bleibt remote zu bestätigen. |
| Completion und Speicher | `DirectoryOpen` hält Output-Handle, Completion, Unicode-/Objectattributestrukturen und Parent in einer stabilen Heapallokation. Selbstreferenzielle Pointer werden erst nach dem Boxen gesetzt. Pending wird erst nach Wait und finalem Status akzeptiert. Nicht bestätigte Pendingoperationen behalten Speicher und Handle; Volumequery und Flush halten ihre eigenen Puffer ebenfalls. Der seltene unbestätigte Ausnahmefall nimmt einen Prozesslebensdauer-Leak zum Schutz vor Use-after-free bewusst in Kauf. |
| Dateisystem und Provider | Kein allgemeiner Directorybeleg aus wirkungslosem Nicht-NTFS-Erfolg oder entferntem NTFS-Label. Die Gerätenatur stammt vom gehaltenen Handle; Remote-/WebDAV-Charakteristiken werden abgewiesen. Tatsächlich erfolgreiche Windows-Write-through-Copy-/Moveaufträge behalten ihren eigenen Beleg. Kein neuer allgemeiner Flushzwang blockiert normale Liveübertragung auf FAT/exFAT. Ein verlorener nicht-NTFS-Ausgang bleibt bis zu ausreichendem Beleg unbestätigt. |
| Livewriter und readonly | Der bisherige flushfähige Writer bleibt bis nach Fileflush und Callback am selben Objekt. Der lesende Hashhandle ist mit diesem Objekt abgeglichen. Der bestehende staged readonly-Pfad bleibt erhalten. Der gespeicherte Replaypfad behält seine bisherige Anforderung eines tatsächlich flushfähigen Reopen; eine zusätzliche readonly-Replayfähigkeit wird nicht behauptet. |
| Gespeicherte Erfolgsquittung | `verify_saved` validiert weiterhin sämtliche Receipts vor Cleanup, Quelle, Ziel, Root, persistente Identität, Digest und Quarantäne. Nur drei gemeinsam wahre vorhandene Felder wählen den positiven Operationspfad. V1-Format, Serdeattribute, Defaults, Zähler, Reparatur, Widerruf und atomarer Recordwriter bleiben identisch. Fehlendes `acknowledged` bleibt durch seinen bestehenden Default false und wählt den strikten Pfad. |
| Austausch und Manipulation | Beide gespeicherten Varianten lesen SHA-256 vor und nach Fileflush, prüfen erwartete Objektidentität mehrfach, halten Parent/Name und prüfen das aktuelle Parentobjekt erneut. Gleich große fremde Dateien und geänderte Bytes bleiben Fehler. Die Quittung wird nicht zu einer allgemeinen Directoryflush-Fähigkeit umgedeutet. |
| Quelle, Quarantäne, Backup | Die Änderung sitzt vor der bisherigen Quarantänebereinigung. Unbestätigter Transfer-Replay, falscher Hash, fremdes Objekt, Cancel, Rootwechsel und Flushfehler bleiben Fehler vor Cleanup. Restore-/Delete-/Conflict-/Backupprimitive bleiben unverändert. Die erweiterten Verbraucher in D sind gelesen; ihre bisherige Qualifikation und Backup-before-mutation-Grenze werden erhalten. |
| VFS, Sync und Mount | `local_platform::confirm_namespace` hat denselben Funktionskörper wie `a814f263`; `confined_local::confirm_namespace` denselben wie vor M01. In beiden Dateien ändert sich ausschließlich der Kommentar dieser Funktion. Die Quelle verlangt weder ein neues Schreibrecht auf dem Parent noch Administratorrechte. Der neue VFS-Fall prüft positive Qualifikation trotz injiziert fehlenden allgemeinen Flushbelegs und parallel den Fehler der direkt aufgerufenen strikten Bestätigung. Keine Qualifikation wird als durchgeführter Flush gemeldet. |
| Linux/Androidauswahl | Der neue Linux-Liveeinstieg ruft exakt die bisherige `confirm_publication_namespace(parent)`-Funktion mit demselben gehaltenen FD auf. Keine Pfadwiederöffnung über `sync_parent` ersetzt ihn. Plattformauswahl bleibt unverändert; keine Windows-/Uniximporte werden in gemeinsame OS-Dateien verschoben. |
| Konflikte und Abbruch | No-replace- und Cross-device-Preflight, Skip/Rename/Overwrite, Konfliktretries, Vorbereitung/Widerruf, verlorene Moveantwort und Receiptfehler bleiben unverändert. Unbekannt veröffentlichte Ziele bleiben erhalten. Die bestehenden F10–F12-/F16-/F44-/F48–F49-/F51–F56-Assertions bestehen fort. |
| Medien, Quellen und Orte | CopyFile2 bleibt im nicht synchronisierten Neukopierweg. Optische Arbitration, Priorität, Sourceguard, Progress, Cancel, Parallelitätswahl, lokale/UNC-Pfadsemantik, literale Windows-Namen und Remote-Endpointdarstellungen ändern sich nicht. Parentqualifikation ersetzt keinen Providerlocator durch einen lokalen Pfad. |

Die erste statische Parsingabfrage mit Standard-`python3` scheiterte an dessen
fehlendem `tree_sitter`; kein Paket wurde installiert. Main gab den vorhandenen
Interpreter `/root/.local/share/uv/tools/graphifyy/bin/python` für reine
AST-Lektüre frei. Damit haben sämtliche bearbeiteten Rustdateien keinen
Tree-sitter-Syntaxfehler. `git diff --check` hat für die zugewiesene Änderung
keinen Whitespacebefund. Dies sind Parsing-/Textbelege, keine Rust-Typprüfung
und keine ausgeführten Fälle. Kein Compiler, Build, Test, Formatter, Server,
Installer, Unteragent, Commit, Push, Graphupdate oder Releasebefehl lief.
Die statische ASTlektüre vergleicht zusätzlich die beiden tatsächlichen
`confirm_namespace`-Funktionskörper ohne Kommentare: `local_platform.rs`
ist zu `a814f263` identisch, `confined_local.rs` zum Bestand vor M01. Die neue
Fallquelle und dieser ungetrackte Bericht haben keine nachgestellten
Whitespacezeichen und besitzen jeweils den abschließenden Zeilenumbruch.

Die veröffentlichte `windows-sys`-Quelle 0.59.0 wurde direkt aus dem
[Crates.io-Paket](https://crates.io/crates/windows-sys/0.59.0) im Speicher
gelesen: `FILE_ADD_FILE` ist im aktivierten FileSystem-Modul vorhanden und
entspricht dem Schreibdatenbit. Kein Paket wurde auf Platte entpackt oder
installiert. Dieses API-Inventar ersetzt keine Typprüfung.

## F — Dateiinventar, Entscheidungen und verbleibende Laufzeitfragen

Neue Dateien:

- `docs/plaene/2026-10-24-batches/fix-f-windows-publication.md`
- `native/src/local_access/os/windows/publication_task_tests.rs`

Geänderte vorhandene Dateien:

- `native/src/copy/mod.rs` — genau der freigegebene Re-Export.
- `native/src/copy/os/linux_os.rs` — ausschließlich die freigegebene Weiterleitung.
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/shared/publication_proof.rs`
- `native/src/local_access/os/windows/publication.rs`
- `native/src/transfer/os/shared/engine/file_recovery_verify.rs` — ausschließlich die freigegebene Auswahl vor Quarantänebereinigung.
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/vfs/os/windows/confined_local.rs`

Die größte berührte Rustdatei ist `copy/os/windows.rs`: 475 Zeilen / 17 344
Bytes. `publication_proof.rs`: 451 Zeilen / 16 322 Bytes. Directoryadapter:
312 Zeilen / 10 018 Bytes. Neue Fallquelle: 316 Zeilen / 11 909 Bytes. Alle
bearbeiteten Rustdateien liegen vor der ausschließlich remote erlaubten
Formatierung unter 500 Zeilen und 50 KiB.

### Vollständiges lokales Leseinventar dieses Autorzyklus

Gezielte Definitionen, Diagnoseeinträge, ASTknoten und `rg`-Abschnitte sind
gezielte Lektüre; eine zeilenweise Komplettlektüre sämtlicher unveränderter
Dateiteile wird nicht behauptet.

- `/root/.codex/AGENTS.md`
- `/root/.codex/skills/arbeitsweise/SKILL.md`
- `/root/.agents/skills/arbeitsweise/SKILL.md`
- `/root/.codex/skills/graphify/SKILL.md`
- `/root/.codex/skills/graphify/references/query.md`
- `/tmp/smartexplorer-final-review-user-contract.txt`
- `/tmp/smartexplorer-f-windows_publication-scope.json`
- `/tmp/smartexplorer-rv1-40406246032/native-failures.json`
- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`
- `docs/plaene/2026-10-24-batches/fix-regression-transfer.md`
- `docs/plaene/2026-10-24-batches/fix-f-windows-publication.md`
- `graphify-out/graph.json`
- `native/Cargo.toml`
- `native/Cargo.lock`
- `native/review-task-manifest.json`
- `native/src/copy/mod.rs`
- `native/src/copy/os/linux_os.rs`
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/windows_identity.rs`
- `native/src/copy/os/windows_file_proof.rs`
- `native/src/copy/os/windows_publication_failure.rs`
- `native/src/copy/os/windows_recovery.rs`
- `native/src/copy/os/shared/batches_direct_move_compatibility_task_tests.rs`
- `native/src/copy/os/shared/direct_move.rs`
- `native/src/copy/os/shared/durability.rs`
- `native/src/copy/os/shared/move_guard.rs`
- `native/src/copy/os/shared/publication.rs`
- `native/src/copy/os/shared/publication_proof.rs`
- `native/src/copy/os/shared/recovery.rs`
- `native/src/copy/os/shared/safe_file.rs`
- `native/src/copy/os/shared/staging.rs`
- `native/src/local_access/os/windows/confined.rs`
- `native/src/local_access/os/windows/access_task.rs`
- `native/src/local_access/os/windows/broker.rs`
- `native/src/local_access/os/windows/confined_native.rs`
- `native/src/local_access/os/windows/create.rs`
- `native/src/local_access/os/windows/directory.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/directory_identity.rs`
- `native/src/local_access/os/windows/directory_rename.rs`
- `native/src/local_access/os/windows/helper_task.rs`
- `native/src/local_access/os/windows/private_access.rs`
- `native/src/local_access/os/windows/private_ancestors.rs`
- `native/src/local_access/os/windows/private_security.rs`
- `native/src/local_access/os/windows/publication.rs`
- `native/src/local_access/os/windows/publication_task_tests.rs`
- `native/src/local_access/os/windows/regular.rs`
- `native/src/source_media/tests.rs`
- `native/src/support_dirs.rs`
- `native/src/creds/os/private_storage_windows.rs`
- `native/src/transfer/os/shared/engine/file_recovery.rs`
- `native/src/transfer/os/shared/engine/file_recovery_fix_tests.rs`
- `native/src/transfer/os/shared/engine/file_recovery_fixture.rs`
- `native/src/transfer/os/shared/engine/file_recovery_observer.rs`
- `native/src/transfer/os/shared/engine/file_recovery_receipts.rs`
- `native/src/transfer/os/shared/engine/file_recovery_tests.rs`
- `native/src/transfer/os/shared/engine/file_recovery_verify.rs`
- `native/src/transfer/os/shared/engine/folder_reservation.rs`
- `native/src/transfer/os/shared/engine/job_store.rs`
- `native/src/transfer/os/shared/engine/job_store_catalog.rs`
- `native/src/transfer/os/shared/engine/job_store_validation.rs`
- `native/src/transfer/os/shared/engine/local.rs`
- `native/src/transfer/os/shared/engine/local_plan.rs`
- `native/src/transfer/os/shared/engine/missing_retry.rs`
- `native/src/transfer/os/shared/engine/record_io.rs`
- `native/src/vfs/mod.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/fs_profile.rs`
- `native/src/vfs/core/cache_extensions.rs`
- `native/src/vfs/os/shared/local.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/shared/confined_local.rs`
- `native/src/vfs/os/shared/confined_local_extensions.rs`
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/vfs/os/windows/local_writes.rs`
- `native/src/vfs/os/windows/confined_local.rs`
- `native/src/vfs/os/windows/confined_local_volume.rs`
- `native/src/vfs/os/windows/volume_info.rs`
- `native/src/vfs/os/linux_os/namespace_flush.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/os/shared/duplicate_backup.rs`
- `native/src/bisync/os/shared/version_manifest.rs`
- `native/src/bisync/os/shared/duplicate_apply.rs`
- `native/src/bisync/os/shared/apply_dedupe.rs`
- `native/src/bisync/os/shared/apply_transaction.rs`
- `native/src/bisync/os/shared/apply_dirs.rs`
- `native/src/bisync/os/shared/replacement_recovery.rs`
- `native/src/bisync/os/shared/version_save.rs`
- `native/src/bisync/os/shared/merge_precheck.rs`
- `native/src/bisync/os/shared/merge_keep_both.rs`
- `native/src/bisync/os/shared/merge_recorded.rs`
- `native/src/bisync/os/shared/apply_transfer.rs`
- `native/src/bisync/os/shared/merge_recovery.rs`
- `native/src/bisync/os/shared/apply_delete.rs`
- `native/src/bisync/os/shared/replacement_journal.rs`
- `native/src/bisync/os/shared/move_finalize.rs`
- `native/src/bisync/os/shared/version_ops.rs`
- `native/src/bisync/os/shared/replacement_publish.rs`
- `native/src/bisync/os/shared/merge_inputs.rs`
- `native/src/mount/os/windows/recovery_storage.rs`
- `native/src/mount/os/shared/recovery_import.rs`
- `native/src/daemon/os/shared/ipc_backend_extensions.rs`
- `native/src/vault/os/shared/backend_extensions.rs`
- `native/src/share/core/fs_guard_extensions.rs`
- `native/src/agent/core/extensions.rs`

Zusätzlich wurden genau die in D verglichenen freigegebenen Dateipfade aus
Release `a814f263` über `git show` gelesen: Windows `local_platform.rs`,
VFS `core/extensions.rs`, Bisync `apply_stage.rs`, `apply_transaction.rs`,
`replacement_recovery.rs` sowie die fehlenden Mountpfade
`os/windows/recovery_storage.rs` und `os/shared/recovery_import.rs`.
Ein fehlender Pfad in diesem Commit wird als fehlend ausgewiesen; seine
heutige Funktion wird nicht rückwirkend als ausgelieferter Bestand behauptet.

Externe Quellen stehen in C und im Crates.io-Absatz in E. Die Arbeitsweise
wurde über den delegierten MCP-Kontext geladen. Ihre zuerst mitgelieferte
fremde Anbieter-/Modellkennung wurde ausdrücklich auf unbekannt zurückgesetzt;
sie wird nicht als tatsächliches Ausführungsmodell behauptet.

### Registrierung und noch unbestätigte Runtimefragen

Main muss diese exakten neuen Namen dem M01-Meilenstein desselben Manifest-
und Remoteeinstiegs zuordnen. Alle verwenden das bereits vorhandene
Pflichtpräfix `batches_task_transfer_compat_`; kein neuer Suiteeinstieg wird
angelegt. Die Fallquelle ist im Windows-Publikationsmodul mit `#[cfg(test)]`
eingebunden.

- `batches_task_transfer_compat_windows_publication_flushes_root_and_native_child`
- `batches_task_transfer_compat_windows_publication_reopens_the_held_directory_after_rename`
- `batches_task_transfer_compat_windows_publication_rejects_wrong_type_and_write_reservation`
- `batches_task_transfer_compat_windows_publication_never_promotes_non_ntfs_or_remote_labels`
- `batches_task_transfer_compat_windows_publication_propagates_directory_flush_failures`
- `batches_task_transfer_compat_windows_publication_vfs_keeps_qualification_separate_from_parent_proof`
- `batches_task_transfer_compat_windows_publication_live_write_through_needs_no_general_parent_proof`
- `batches_task_transfer_compat_windows_publication_recorded_success_stays_bound_to_identity_digest_and_cancel`

Die ursprünglichen F10–F12, F16-Windows, F44, F48–F49 und F51–F56 bleiben mit
unveränderten Assertions in derselben Suite. Capabilityinjektion im Livefall
beweist bei Ausführung die Codewegtrennung; sie ist kein realer FAT-/exFAT-
Datenträgertest. Echte NTFS-Child-/Root-Reopens, schmaler Schreibzugriff,
native Fehlerweitergabe, die beobachteten Desktopfälle und M02-Integration
bleiben durch Main remote zu bestätigen. Ein tatsächlicher FAT-/exFAT-/ReFS-
Datenträgernachweis und Hardware-Powerloss-Nachweis liegen nicht vor.
Fehlende echte Medien sind kein Releaseblocker und kein bestandener Test.

Die erweiterte M01-Consumergrenze ist geschlossen: Die eingeführte allgemeine
Nicht-NTFS-Sperre ist entfernt; `apply_stage::require_durable` sowie sämtliche
in D enumerierten Sync-/Backup-/Mount-Verbraucher behalten ihren bestehenden
Vertrag. Der neue destruktive Transfer-Replay verwendet weiterhin den
unabhängigen wirklichen Parentbeleg, sofern keine dauerhafte positive
Operationsquittung vorliegt. Es bleibt keine ungeklärte zusätzliche
Schreibscopeabhängigkeit für M01. Main besitzt Manifestregistrierung, Git,
Graphrefresh, denselben Remoteeinstieg und den terminalen Release; der
abgegrenzte Autorauftrag endet mit dieser lokalen statischen Abgabe.
