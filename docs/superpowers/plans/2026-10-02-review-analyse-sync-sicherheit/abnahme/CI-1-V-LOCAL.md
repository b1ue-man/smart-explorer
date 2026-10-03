# CI-1-V-LOCAL: Windows-Iterator, Transfer-Guard und WebDAV-Fixture

Stand: 2026-10-03. Enger Anschluss der konkreten Compilerdiagnostik aus Run
`37145175629`, gemäß `scopes/ci-1-v-local.json` und `ci-fixes.md`. Der bereits
angewandte kandidatengebundene Remote-Formatterpatch wurde weitergeführt.
Keine neue Review-Runde, kein Testlauf und keine Änderung der Root-Suite.

## Diagnosezuordnung und Umsetzung

| Diagnose | Änderung | Erhaltener Vertrag / Abnahmesignal |
| --- | --- | --- |
| Windows `E0277`: `DirectoryEntries` lässt sich in den beiden Share-Löschläufen nicht als `Iterator + Send` speichern. | `directory.rs`: Das gespeicherte `Query`-Traitobjekt und beide privaten Query-Konstruktoren verlangen additiv `Send`. | Der Compiler kann `Send` aus den tatsächlichen Feldern ableiten. Kein `unsafe impl Send`, keine Entfernung des `Send`-Bounds am Consumer, kein anderer Iterator und keine Änderung der gehaltenen Ancestry-/Root-Handles. |
| Windows `E0061`: Transfer ruft `metadata_is_link_like` ohne den erforderlichen Pfad auf. | Windows- und Unix-Adapter erhalten gemeinsam `upload_is_link_like(path, metadata)`; alle zehn freigegebenen Upload-/Recovery-Caller übergeben den Pfad ihrer vorhandenen `symlink_metadata`/Observation. | Windows kann Datenreparsepunkte von Umleitungen unterscheiden; unbekannte Tags bleiben geschützt. Unix prüft weiterhin ausschließlich `metadata.file_type().is_symlink()`. Keine Plattformverzweigung im shared-Code. |
| Linux/Windows `E0063`: Der direkte WebDAV-Fixture-Konstruktor lässt zwei Backendfelder aus. | `backend_for_timeout` initialisiert `hashes_observed` mit `Arc::new(AtomicBool::new(false))` und `stage_times` mit einer leeren `Arc<Mutex<HashMap<String, (i64, Option<String>)>>>`, wie der vorhandene `connect()`-Konstruktor. | Alle bisherigen Fixture-Agenten, Timeouts, Redirect-/Replay-Grenzen und Assertions bleiben bytegleich. |

## Entscheidungen und Self-Review

- `Send` wird am gespeicherten Callback und seinen Erzeugungsgrenzen vom
  Typsystem verlangt; die Query bleibt `FnMut` mit unveränderter Lebensdauer.
  Die vorhandenen injizierten Queries erfassen mutable `Vec`-/`usize`-Werte
  bzw. kopierbare Werte, keine nicht übertragbaren Callbackzustände.
- `DirectoryEntries` und `DirectoryHandle` bleiben unverändert. Extended-/Full-
  Klassen, Provider-Fallback, Deduplizierung, Zugriffsfehler und die bestehende
  Reparse-/Special-Klassifikation sind nicht verändert.
- `observation` übergibt seinen schon vorhandenen `path`. Recovery verwendet
  jeweils `directory`, `path` oder `marker`. In `is_recovery_directory` wird
  der vorhandene `directory.join(PRESERVE_MARKER)` einmal als `marker_path`
  gebunden und für Metadaten und Klassifikation gemeinsam verwendet.
- Die App- und Transfer-Reexports bleiben unverändert. Keine Rechte-, Owner-,
  Marker-, Root-, Cancellation-, Backup-, Retry- oder Löschgrenze wird ersetzt
  oder abgeschwächt; die Calleränderungen beschränken sich auf Pfadargumente
  und die zugehörige Markerpfadbindung.
- Der direkte WebDAV-Fixture-Konstruktor bleibt bestehen; `connect()` wird
  dort nicht aufgerufen und erzeugt keinen zusätzlichen Netzwerkrequest.
- Statisches Tree-sitter-Rust-Parsing der sieben geänderten Rustdateien ergab
  keine `ERROR`-/`MISSING`-Knoten. In `directory.rs`, beiden OS-Adaptern und der
  WebDAV-Fixture wurde der vollständige Text gegen den vor der Änderung
  gelesenen Text mit ausschließlich den vorgesehenen Ersetzungen abgeglichen.
  Der übrige Quelltext einschließlich sämtlicher Assertions ist bytegleich.
- Alle geänderten Rustdateien bleiben unter 500 Zeilen und 50 KiB:

| Datei | Zeilen | Bytes |
| --- | ---: | ---: |
| `native/src/local_access/os/windows/directory.rs` | 477 | 17695 |
| `native/src/transfer/os/windows.rs` | 125 | 4894 |
| `native/src/transfer/os/unix.rs` | 47 | 1754 |
| `native/src/transfer/os/shared/upload_stream.rs` | 122 | 4067 |
| `native/src/app/os/shared/remote_helpers/recovery.rs` | 124 | 3977 |
| `native/src/app/os/shared/remote_helpers/recovery_manifest.rs` | 390 | 13804 |
| `native/src/webdav/core/connection_tests.rs` | 359 | 13447 |

## Abnahme durch die vorhandene Root-Suite

Die Compilerdiagnosen müssen im nächsten Lauf derselben vollständigen
Remote-RV1-Suite verschwinden. Dieser Worker hat weder Compiler noch Tests,
Formatter, Server, Git, CI, Graph oder Release ausgeführt. Parsing ersetzt
keine Remote-Typprüfung oder Verhaltensabnahme.

Bestehende passende AcceptanceSelector, unverändert erhalten:

- `analytics_access_task_automatic_query_fallback_preserves_access_denial`
- `analytics_access_task_midway_query_failure_finishes_through_ordinary_listing`
- `analytics_access_task_full_record_fallback_and_reparse_classification`
- `review_task_special_reparse_tags_are_special_files`
- `sync_links_task_windows_cloud_data_tags_are_not_redirecting_links`
- `transfer_engine_task_canonical_folders_drop_the_verbatim_prefix_only_when_safe`
- `propfind_retries_when_body_drops_after_headers`
- `propfind_body_blackhole_stops_after_one_bounded_retry`
- `propfind_reconnects_after_ambiguous_stale_pool_close`
- `get_reconnects_before_exposing_body_after_stale_pool_close`
- `delete_response_loss_is_not_replayed`
- `mutation_redirect_is_not_followed_or_reported_as_success`
- `put_redirect_is_terminal_and_never_followed`

Zusätzliches konkretes Integrationssignal aus den vorhandenen Aufrufen:
`fs_delete_local::frame` und `fs_host_destructive::frame` behalten ihren
`Box<dyn Iterator<Item = io::Result<LocalEntry>> + Send>`; alle Upload-/Recovery-
Klassifikationen verwenden zwei Argumente und denselben tatsächlichen Pfad
wie die zuvor erhobenen Metadaten. Keine neue Assertion wurde eingeführt.

## Exaktes Inventar

Gelesen, bei den nachträglich freigegebenen Callerdateien nur die im Scope
genannten Aufruf-/Pfadgrenzen; App-Facaden nur die Reexports:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-1-v-local.json`
- `/tmp/rv1-ci-first/v-local.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fixes.md`
- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/rv1-remote-suite.md`
- `native/src/local_access/mod.rs`
- `native/src/local_access/os/windows/directory.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/directory_records.rs`
- `native/src/local_access/os/windows/directory_tests.rs`
- `native/src/local_access/os/windows/sync_link_task_tests.rs`
- `native/src/share/core/fs_delete_local.rs`
- `native/src/share/os/shared/fs_host_destructive.rs`
- `native/src/copy/mod.rs`
- `native/src/transfer/mod.rs`
- `native/src/transfer/os/windows.rs`
- `native/src/transfer/os/unix.rs`
- `native/src/transfer/os/shared/upload_stream.rs`
- `native/src/app/os/shared/remote_helpers/recovery.rs`
- `native/src/app/os/shared/remote_helpers/recovery_manifest.rs`
- `native/src/app/os/windows/platform.rs`
- `native/src/app/os/linux_os.rs`
- `native/src/webdav/mod.rs`
- `native/src/webdav/core/extensions.rs`
- `native/src/webdav/core/webdav.rs`
- `native/src/webdav/core/connection_tests.rs`

Geändert:

- `native/src/local_access/os/windows/directory.rs`
- `native/src/transfer/os/windows.rs`
- `native/src/transfer/os/unix.rs`
- `native/src/transfer/os/shared/upload_stream.rs`
- `native/src/app/os/shared/remote_helpers/recovery.rs`
- `native/src/app/os/shared/remote_helpers/recovery_manifest.rs`
- `native/src/webdav/core/connection_tests.rs`

Erstellt:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-V-LOCAL.md`

Gelöscht: keine. Registrierungsänderungen: keine.

Nur fehlgeschlagene Leseversuche an ausdrücklich gelisteten, fehlenden Pfaden:

- `native/src/boxed.rs`
- `native/src/local_access/os/windows/metadata.rs`
- `native/src/vfs/os/windows/backend.rs`
- `native/src/webdav/core/connection.rs`
- `native/src/webdav/core/core.rs`

Die tatsächliche WebDAV-Definition, Query-Fixtures, beide Transferadapter und
sämtliche vom Parent lokalisierten Caller wurden danach ausdrücklich im Scope
freigegeben. Es verbleibt keine für diese Korrektur benötigte Scope-Lücke.
Remote-Abnahme und Integration bleiben beim Parent. Dieser Block ist beendet.
