# CI-2-V-LOCAL: Windows-Verzeichnis- und Private-Storage-I/O

Stand: 2026-10-03. Umsetzung des V-LOCAL-Abschnitts aus
`ci-behavior-fixes.md`, anhand von `/tmp/rv1-ci-second/v-local.json`:
[Run 37150409255](https://github.com/b1ue-man/smart-explorer/actions/runs/37150409255),
Kandidat `ac9b475ff18f6320bedd408c5a03c091710ad01c`. Der kandidatengebundene
zweite Remote-Formatterpatch war vor diesen Änderungen bereits übernommen.
Scope: `scopes/ci-2-v-local.json`, einschließlich der vom Root ergänzten
Modify-Freigabe für `private_security.rs`.

## Belegte Fehler und gemeinsamer Anschluss

| Befund | Korrektur | Erwartung derselben Remote-Suite |
| --- | --- | --- |
| `private parent has no pinned path` in privaten Index-/Papierkorb-Consumern | Der private Dateizugriff verwendet jetzt `DirectoryHandle::open_private_child`, das an den gehaltenen Parent und einen validierten einzelnen Childnamen gebunden ist. `watch_path()` wird dafür nicht mehr konsumiert. | Private Dateien öffnen ohne erfundene Watchfähigkeit oder unkonfinierten Pfad-Fallback. |
| Windows Error 32 beim Quarantine-Capture und in der Directory-Pin-Fixture | Nur die gehaltenen validierten Verzeichnispins erlauben zusätzlich `FILE_SHARE_WRITE`. Das umfasst ausgewählten/physisch aufgelösten Root, physische Eltern, PinChild und privat erzeugte Directory-Pins. | Der normale, ACL-geprüfte Zielordnerzugriff für Rename/Create kollidiert nicht mehr mit den Lesepins; gehaltene Root-/Child-Pins verweigern weiter Delete-Sharing. |
| Windows Error 5 in neuen Journal-/Lock-/Checkpoint-/Merge-State-Consumern | Private `CREATE_NEW`-Dateien werden mit `GENERIC_READ | GENERIC_WRITE` erzeugt, statt lediglich mit `GENERIC_WRITE`. DACL, Nicht-Ersetzen und Sharing bleiben erhalten. | Der neu erzeugte private Handle ist auch für das eigene Lesen/Seek/Lock-Protokoll verwendbar; die fehlende Lesecapability löst keinen Berechtigungsfehler mehr aus. |

Die Zuordnung der Sharing- und Lesecapability-Fehler folgt aus den konkreten
Quellen und dem Primär-API-Vertrag. Eine erfolgreiche Remote-Verhaltensabnahme
wird hier nicht vorweggenommen.

## Geprüfte API-Lücke und Entscheidung

Die gezielte Microsoft-Recherche wurde am 2026-10-03 durchgeführt und vom Root
ausdrücklich freigegeben:

- `IopOpenLinkOrRenameTarget` öffnet den Zielordner mit
  `FILE_WRITE_DATA | SYNCHRONIZE`. Die WDK-Dokumentation erläutert hierzu die
  kompatible minimale RootDirectory-Öffnung. Das erklärt, weshalb ein aktiver
  LIST_DIRECTORY-Pin ohne Write-Sharing den Rename-Zielzugriff blockiert.
  [FILE_RENAME_INFORMATION](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information)
- Sharing regelt die Kompatibilität gleichzeitig geöffneter Handles; gewünschter
  Zugriff und ACL bleiben eigenständige Prüfungen. Nicht freigegebener
  Delete-Zugriff verhindert Rename-/Delete-Öffnungen. Lesen und Schreiben einer
  neu erstellten Datei benötigen die entsprechenden Zugriffsrechte.
  [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew),
  [File Access Rights Constants](https://learn.microsoft.com/en-us/windows/win32/fileio/file-access-rights-constants)
- `RootDirectory` kann laut Win32-Dokumentation mit relativem Namen verwendet
  werden; das beseitigt aber nicht automatisch die zusätzlich benötigte
  Zielordneröffnung. Deshalb wurde der vorhandene sichere NoReplace-Rename
  erhalten und kein unbewiesener relativer Win32-Rename oder neuer Native-
  Systemcall eingeführt.
  [FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info),
  [SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)

Der Root hat die Grenze präzisiert: notwendiges Write-Sharing ausschließlich
an Verzeichnispins ist zulässig; Root-/Elternpins erhalten kein Delete-Sharing.
Diese Erlaubnis ersetzt keine Autorisierung. Die gewünschten Rechte der
Lesepins bleiben READ_ATTRIBUTES/LIST_DIRECTORY; kein zusätzlicher gewünschter
Daten-Schreibzugriff wird an read-only Quellen eingeführt.

## Additive API und erhaltene Schutzgrenzen

```rust
// Windows DirectoryHandle, crate-intern:
fn open_private_child(&self, name: &OsStr, writable: bool) -> io::Result<File>;
```

`private_access.rs` validiert den Einzelkomponenten-Namen, prüft/härtet den
bereits gepinnten Parent und öffnet unter dessen behaltenem physischen Pfad
mit OPEN_REPARSE_POINT. Das geöffnete Objekt durchläuft die vorhandene
Owner-/DACL-/Reparse-/Special-/Hardlink-Prüfung, bevor sein Filehandle an den
Caller zurückkehrt. Es wird kein freier Pfad oder Watchhaken herausgegeben.

Der vorhandene private Writable-Consumer erhält seine bisherigen Rechte und
sein bisheriges Write-Sharing; die read-only Variante verlangt weiterhin
keinen GENERIC_WRITE-Datenzugriff. Die bereits vorgesehene ownergebundene
DACL-Migration mit READ_CONTROL/WRITE_DAC bleibt erhalten.

Weitere unveränderte Grenzen:

- `watch_path()` bleibt unter Windows exakt `None`; keine OVERLAPPED-Fähigkeit
  wird aus einem synchronen Readpin abgeleitet.
- Gewählte Rootauflösung, physische Elternkette, Objektidentitätsvergleich,
  Child-Komponentenvalidierung, OPEN_REPARSE_POINT und die handlegebundene
  Redirect-/Special-Klassifikation bleiben erhalten.
- Gewöhnliche reguläre Dateihandles erhalten keine neue Sharing-Freigabe.
  Quarantine-DELETE-Guards und neu erzeugte private Dateien bleiben bei
  `FILE_SHARE_READ`, ohne zusätzliches Write-/Delete-Sharing.
- Private Directory-Erstellung behält die geschützte owner-only DACL vor
  Erstellung. File-Erstellung behält CREATE_NEW, NoFollow, Owner-/DACL-
  Nachprüfung und Hardlink-Verweigerung; geändert wird nur die Lesecapability
  des neuen privaten Handles.
- Full-ID-/Volume-Prüfung, Name-Slotprüfung, NoReplace, Recycle-Restore,
  Collision-/Retryfähigkeit und die aufbewahrende Drop-Semantik bleiben
  unverändert. Kein Original oder fremdes Ersatzobjekt wird neu gelöscht.
- Consent-/Broker-/Backup-Pfade, read-only Fehler, Cancel-/Backupgrenzen und
  die vorhandene konservative Durability-Aussage sind nicht ersetzt worden.

## Eigener statischer Self-Review

Die drei bestehenden geänderten Dateien wurden vollständig gegen ihre zuvor
gelesenen Texte mit ausschließlich den vorgesehenen Ersetzungen abgeglichen.
`read.rs`, `create.rs`, `quarantine.rs`, `directory_handle_tests.rs` und
`host_trash/os/shared/review_task_tests.rs` sind bytegleich geblieben.
Sämtliche vorhandenen Assertions und Fixtures sind erhalten. Insbesondere
wird in der Windows-Pin-Fixture kein zusätzlicher Root-Pin frühzeitig
freigegeben: der bestehende Root bleibt während der letzten Rename-Probe aktiv.

Statisches Tree-sitter-Rust-Parsing der vier betroffenen Rustflächen ergab
keine ERROR-/MISSING-Knoten. Größen:

| Datei | Zeilen | Bytes |
| --- | ---: | ---: |
| `native/src/local_access/os/windows/directory_handle.rs` | 359 | 12671 |
| `native/src/local_access/os/windows/private_security.rs` | 366 | 13382 |
| `native/src/creds/os/private_storage_windows.rs` | 82 | 3126 |
| `native/src/local_access/os/windows/private_access.rs` | 34 | 1463 |

Keine lokalen Tests, Compiler, Formatter, Produktprozesse, Server, Git-, CI-,
Graph- oder Releaseaktionen. Keine neue Suite oder unabhängige Testanlage.
Die Abnahme erfolgt ausschließlich im nächsten Root-gesteuerten Lauf derselben
vollständigen Remote-RV1-Suite.

## Konkrete vorhandene AcceptanceSelector

Die diagnostizierten bestehenden Symbole bleiben unverändert ausgewählt:

- `bisync::checkpoint_review_tests::review_task_checkpoint_flushes_before_the_end_of_a_long_run`
- `bisync::checkpoint_review_tests::review_task_checkpoint_journal_recovers_and_truncates_only_a_torn_tail`
- `bisync::checkpoint_review_tests::review_task_checkpoint_keeps_successes_and_old_deferred_entries_on_stop`
- `bisync::checkpoint_review_tests::review_task_checkpoint_rejects_a_complete_corrupt_frame`
- `bisync::checkpoint_review_tests::review_task_checkpoint_timer_saves_while_the_next_transfer_is_slow`
- `bisync::checkpoint_review_tests::review_task_corrupt_optional_index_does_not_block_completed_file_work`
- `bisync::checkpoint_review_tests::review_task_daily_target_verification_finds_untracked_mirror_orphans`
- `bisync::checkpoint_review_tests::review_task_external_merge_replays_checkpoints_before_updating_one_entry`
- `bisync::merge_recorded::task_tests::review_task_merge_keep_both_preserves_loser_on_both_sides`
- `bisync::merge_recorded::task_tests::review_task_merge_partial_publication_keeps_conflict_basis_and_retries`
- `bisync::merge_recorded::task_tests::review_task_merge_rejects_changed_bytes_even_with_same_size_and_time`
- `bisync::replica_state::tests::review_task_merge_and_forget_keep_other_owners`
- `host_trash::review_task_tests::review_task_host_trash_changed_payload_is_never_restored`
- `host_trash::review_task_tests::review_task_host_trash_intent_precedes_capture_and_survives_restart`
- `host_trash::review_task_tests::review_task_host_trash_private_records_and_partial_intents_keep_other_entries`
- `host_trash::review_task_tests::review_task_host_trash_replaced_root_is_not_a_restore_target`
- `host_trash::review_task_tests::review_task_host_trash_restore_preserves_collision_and_is_retryable`
- `host_trash::review_task_tests::review_task_host_trash_restore_restart_hop_has_durable_mapping`
- `local_access::directory_handle_tests::review_task_directory_handles_pin_the_entered_windows_directory`
- `local_access::directory_handle_tests::review_task_recycle_quarantine_moves_only_to_a_free_anchored_name`
- `local_access::directory_handle_tests::review_task_recycle_quarantine_refuses_a_replaced_expected_child`
- `local_access::directory_handle_tests::review_task_recycle_quarantine_restore_never_replaces_a_new_child`

Weitere bereits vorhandene Schutzsignale, ohne neue Assertions:

- `local_access::directory_handle_tests::review_task_directory_handles_create_private_children_without_replacement`
- `local_access::directory_handle_tests::review_task_private_handle_hardening_refuses_hardlinked_records`
- `host_trash::review_task_tests::review_task_host_trash_failed_intent_and_expected_hash_leave_source_untouched`
- `host_trash::review_task_tests::review_task_host_trash_link_source_and_invalid_slots_are_refused`
- `host_trash::review_task_tests::review_task_host_trash_file_reservation_serializes_other_owners`
- `review_task_recycle_publication_failure_restores_without_replacing`

## Exaktes Inventar und Owner-Grenze

Gelesene lokale Quellen/Evidenz, teilweise nur der zugehörige Abschnitt;
einschließlich eigener neuer Quelle beim Self-Review:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-2-v-local.json`
- `/tmp/rv1-ci-second/v-local.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-behavior-fixes.md`
- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/rv1-remote-suite.md`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/read.rs`
- `native/src/local_access/os/windows/create.rs`
- `native/src/local_access/os/windows/quarantine.rs`
- `native/src/creds/os/private_storage_windows.rs`
- `native/src/local_access/os/windows/private_security.rs`
- `native/src/local_access/directory_handle_tests.rs`
- `native/src/local_access/os/windows/private_ancestors.rs`
- `native/src/local_access/os/windows/directory_identity.rs`
- `native/src/creds/mod.rs`
- `native/src/local_access/core/protocol.rs`
- `native/src/local_access/core/regular.rs`
- `native/src/local_access/mod.rs`
- `native/src/local_access/os/windows/mod.rs`
- `native/src/host_trash/os/shared/store.rs`
- `native/src/host_trash/os/shared/restore.rs`
- `native/src/analytics/os/shared/checked_recycle.rs`
- `native/src/local_access/os/linux_os.rs`
- `native/src/local_access/os/windows/directory.rs`
- `native/src/host_trash/os/shared/review_task_tests.rs`
- `native/src/local_access/os/windows/private_access.rs`

Geändert:

- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/private_security.rs`
- `native/src/creds/os/private_storage_windows.rs`

Erstellt:

- `native/src/local_access/os/windows/private_access.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-V-LOCAL.md`

Gelöscht: keine. Registrierungen: ausschließlich zwei additive Zeilen für das
eigene `private_access`-Modul in `directory_handle.rs`.

Der Root übernimmt die gemeldete einzelne Kommentarberichtigung an
`local_access/core/protocol.rs::ReadKind::PinRoot`: Die pauschale alte
NoWrite-Sharing-Aussage wird an die freigegebene Directory-Grenze angepasst.
Diese Datei wurde vom Worker ausschließlich gelesen; keine weitere
Vertragsdoku wurde bearbeitet. Keine benötigte Definition oder Scope-Lücke
verbleibt. Integration und Remote-Abnahme bleiben beim Root. Block beendet.
