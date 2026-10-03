# CI-3-V-LOCAL – private Zugriffscapabilities

Stand: 2026-10-03. Die zugewiesenen Sourcefixes aus dem beendeten
[Run 37157166735](https://github.com/b1ue-man/smart-explorer/actions/runs/37157166735)
sind umgesetzt und statisch geprüft. Laufzeitabnahme bleibt ausschließlich
bei Root in der bestehenden vollständigen RV1-Remote-Suite. Keine neuen
Fixtures, Tests, Reviewkampagne oder lokale Produktläufe.

## Konkrete Zuordnung und Umsetzung

| Belegte Fehlerfläche | Tatsächliche Ursache und Änderung | Erhaltene Grenze |
| --- | --- | --- |
| Windows: `checkpoint_review_tests::review_task_checkpoint_journal_recovers_and_truncates_only_a_torn_tail`, Zeile 80, sowie die anderen Checkpoint-/Sync-Kaskaden mit `Zwischenstand: AccessDenied` | `Journal::append` öffnete `write(true).append(true)` und verlangte anschließend `set_len(valid_bytes)`. Rust entfernt im Appendmodus `FILE_WRITE_DATA`; der tatsächliche End-of-file-Informationsaufruf braucht es. Vorhandenes Journal nun über private `open_file(path, true)`, neue Datei über private `create_file(path)`; nach Kürzen explizit `SeekFrom::Start(valid_bytes)`. | Owner/geschützte DACL bzw. 0700/0600, NoFollow, reguläre Datei mit einem Hardlink und exklusive Neuerstellung vor Mutation. Framebudget, SHA-256, Reihenfolge und Flush bleiben erhalten. |
| Android: `ReviewSyncTaskTest#recordedMergeRetrySurvivesProcessRestart` meldet `Verbindungsspeicher sperren: Permission denied`; `#manifestVersionRestoreRejectsStaleTokens` meldet `Backend-Identität: Permission denied` | Private Unix-Verzeichnisauswahl verlangte `O_RDONLY` an sämtlichen physischen Vorfahren. Appdaten-Vorfahren können Suchrechte ohne Listenrechte geben. Nur Zwischenvorfahren verwenden nun `O_PATH | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`; finales privates Verzeichnis bleibt lesbar. | Suchrechte werden weiter vom OS geprüft. Kein Linkhop, Umleiten privater Daten oder Härten eines Systemvorfahren. Finaler Owner-/Mode-Check, handlegebundenes fchmod/fsync und Dateigrenzen bleiben unverändert. |

Die zunächst diskutierte Windows-Filesystem-Flush-Ursache ist durch die
konkrete Callchain ausgeschlossen: `flush_filesystem` ist dort `Ok(())`;
Profilfehler fallen auf `PerFileOnly` zurück. Es wurden deshalb weder
Volumerechte erhöht noch Flushfehler ignoriert. Zeile 80 der Fixture wurde
mit tatsächlicher Sourcezeilenzählung auf `journal.append` zugeordnet.

## Dateien

Geändert:

- `native/src/creds/os/private_storage_unix.rs` — 183 Zeilen, 6.313 Bytes.
- `native/src/bisync/os/shared/checkpoint_journal.rs` — 307 Zeilen, 11.237 Bytes; Root hat den konkreten Consumer nach Diagnose additiv im Scope freigegeben, E-ENGINE ändert ihn nicht.

Erstellt:

- `docs/refs/private-file-access.md` — frische API-, Syntax-, Fehler- und Rechtebelege samt dokumentierten nicht abrufbaren Primärlinks.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-V-LOCAL.md` — dieser Bericht.

Keine Dateien gelöscht, keine Registrierung oder öffentliche Signatur
geändert. Die freigegebenen Windows-Private-Storage-, Directory-/Create-/DACL-
Dateien wurden ausschließlich gelesen: ihre vorhandenen RW/create-new-
Fähigkeiten sind ausreichend. Keine neue WRITE-/DELETE-Sharing-Freigabe.

Gelesene Repository-/Evidenzdateien, zusätzlich zu eigenen neu erstellten
und eigenen geänderten Dateien beim abschließenden Textvergleich:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-3-v-local.json`
- `/tmp/rv1-ci-third/v-local.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-closure-fixes.md`
- `/tmp/rv1-ci-third/device/merge-prepare-evaluate.log`
- `/tmp/rv1-ci-third/device/sync-manifestVersionRestoreRejectsStaleTokens-evaluate.log`
- `native/src/creds/os/private_storage_unix.rs`
- `native/src/local_access/os/linux_os.rs`
- `native/src/creds/os/private_storage_windows.rs`
- `native/src/local_access/os/windows/private_access.rs`
- `native/src/local_access/os/windows/create.rs`
- `native/src/bisync/os/shared/checkpoint_journal.rs`
- `native/src/local_access/os/windows/private_security.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/read.rs`
- `native/src/local_access/mod.rs`
- `native/src/creds/os/private_storage_tests.rs`
- `native/src/creds/mod.rs`
- `native/src/share/os/shared/identity_store.rs`
- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/rv1-remote-suite.md`
- `native/src/bisync/os/shared/checkpoint_review_tests.rs`
- `native/src/local_access/os/windows/private_ancestors.rs`
- `native/src/bisync/os/shared/replica_state.rs`
- `native/src/bisync/os/shared/persistence.rs`
- `native/src/bisync/os/shared/state_metadata.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libc-0.2.186/src/unix/linux_like/android/mod.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/vfs/os/windows/volume_info.rs`

Die externe libc-Datei wurde ausschließlich in den freigegebenen
Zeilen 675–710 gelesen, darunter die reale Android-Definition
`O_PATH: c_int = 0o10000000` in Zeile 696. Der vorhandene statische
Tree-sitter-Parser wurde nur auf den eigenen geänderten Rustquellen benutzt.

## Entscheidungen und statischer Self-Review

Die Erweiterungen bleiben an der bestehenden `creds::private_storage`-
Objektgrenze. Keine Plattformabfrage oder OS-Verzweigung im Journalcode;
kein neuer freier OS-Zugriff im gemeinsamen Consumer. Ein wirklich
writable Journal benötigt Daten-Schreibrechte; synchronen read-only
Verzeichnis- und Dateipins wurden keine zusätzlichen Rechte gegeben.
Windows-Watchpfad, fehlendes Overlapped-Handle und Pollfallback bleiben
unberührt. Root-/Child-Link-, DACL-, Hardlink-, Sharing- und NoReplace-
Prüfungen werden weiter von den bestehenden privaten APIs angewandt.

Der vorhandene exklusive Pair-/sequenzielle Journalvertrag bleibt
Voraussetzung. Die explizite Schreibposition ersetzt allein die bisherige
Appendmodus-Positionierung: nur ein vom Recovery bestätigtes Präfix wird
behalten; ein vollständiger Frame mit falschem Digest bleibt ein Fehler.
Länge → SHA-256 → Payload → `sync_all` → Fortschreiben von `valid_bytes`
und der anschließende vorhandene Filesystem-Hook bleiben in derselben
Reihenfolge. Compaction schreibt weiter Baseline und Folderhistory vor
Retirement. Budget-, Pfad-, Casepolicy-, Stop-/Deferred-/Retry- und
Baseline-/Backupregeln wurden nicht verändert.

Der abschließende exakte Textvergleich bestätigt, dass außerhalb der
beschriebenen Öffnungs-/Flags-/Seek-/Parenthärtungsregionen keine Source
geändert wurde; insbesondere Recovery, Frameprüfung, Compaction und die
Unix-Owner-/Mode-/Hardlinkprüfungen sind bytegleich zum vor Edit gelesenen
Stand. Tree-sitter meldet keine Parsefehler oder fehlenden Syntaxknoten.
Beide Rustdateien bleiben deutlich unter 500 Zeilen und 50 KiB.
Keine Assertion oder abhängige Fixture wurde geändert. Keine lokale
Kompilation, Formatierung, Tests, Server, Installation, Gitmutation, CI,
Graphaktualisierung, Veröffentlichung oder Delegation ausgeführt.

## Vorhandene AcceptanceSelector und erwartete Signale

Nur die Root-eigene vollständige RV1-Suite wird wiederholt. Vorhandene
passende Selektoren, keine zusätzliche Ausführung:

- `bisync::checkpoint_review_tests::review_task_checkpoint_journal_recovers_and_truncates_only_a_torn_tail`: intaktes Präfix und geschützte Baselineeinträge bleiben; weiterer Frame liegt hinter dem bestätigten Präfix.
- `bisync::checkpoint_review_tests::review_task_checkpoint_rejects_a_complete_corrupt_frame`: vollständiger korrupter Frame bleibt ablehnend.
- `bisync::checkpoint_review_tests::review_task_checkpoint_keeps_successes_and_old_deferred_entries_on_stop`, `review_task_checkpoint_flushes_before_the_end_of_a_long_run`, `review_task_checkpoint_timer_saves_while_the_next_transfer_is_slow`, `review_task_external_merge_replays_checkpoints_before_updating_one_entry`: kein AccessDenied beim tatsächlichen Append; gespeicherte Erfolge, alte Deferredbasis, frühe/timerbasierte Speicherung und Replay bleiben vorhanden.
- Die Windows-Namen in `/tmp/rv1-ci-third/v-local.json`, einschließlich bestehender `app::sync_paths_task_tests::`, `app::sync_links_task_tests::`, `bisync::engine_provider_task_tests::identity_tests::` und Link-/Baseline-/Merge-Consumer: keine private Checkpoint-Fehlerkaskade; sämtliche vorhandenen Ergebnis-/Counterpart-/Retryassertions bleiben bestehen.
- `creds::private_storage::tests::review_task_private_objects_are_restrictive_before_data_is_written` und `review_task_private_objects_refuse_links_and_tighten_owned_old_modes`: neue private Objekte bleiben ab Erstellung restriktiv; Owned-Legacy-Härtung bleibt; Links, Hardlinks, Parentlinks und FIFO bleiben verweigert.
- Android `app.smartexplorer.android.task.ReviewSyncTaskTest#recordedMergeRetrySurvivesProcessRestart` und `#manifestVersionRestoreRejectsStaleTokens`: Verbindungsspeicher-/Identitätszugriff funktioniert durch echte Appdaten-Vorfahren; Restart/Retry und Staletokenverweigerung behalten ihre bestehenden Assertions.

## Offene Owner-Grenzen

Alle tatsächlich benötigten Source-/API-Grants wurden von Root ergänzt;
keine weitere Implementierungsabhängigkeit verbleibt. Die nicht abrufbaren
AOSP- und Rust-HTML-URLs sind im [API-Beleg](../../../../refs/private-file-access.md)
ehrlich genannt; Androids verwendetes Symbol ist in der vorhandenen
libc-Definition, Windows' Appendmaske und echte Truncate-/Seek-Implementierung
in der frischen Rust-1.99.0-Primärquelle überprüft.

Der separat dokumentierte `Versionen: Pfad ist nicht freigegeben`-
Share-/Versionsbackupfehler desselben Syncfixtures gehört laut
`ci-closure-fixes.md` zu A-CLIENT. Er wird durch diesen privaten
Checkpoint-Fix nicht als erledigt markiert. Übrige benannte CI-3-Fehler
bleiben bei ihren bestehenden Ownern. Gesamtabnahme, Commit/Push,
Graphrefresh, derselbe Remote-Suitelauf und terminaler Release gehören Root.
