# V-REMOTE – API-Delta

Stand: 2026-10-03. Providerimplementierung abgeschlossen. Shared VFS-Änderungen sind die vom Hauptagenten registrierten Commits `cbe505c`, `3116d16` und `810b0df9`; V-REMOTE ändert diese Dateien nicht.

## Gemeinsame Schnittstellen

Alle fünf konkreten Provider implementieren `BackendExtensions` und geben `Some(self)` über `Backend::extensions` zurück. `EndpointSpec`, `Loc`, `SavedConnection` und ihre gespeicherte Bedeutung wurden hier nicht umdefiniert. Shared Resolver übernimmt Local/UNC/Peer/Room und Remote-Paare; das Backendrelative wird niemals zum lokalen Dateisystempfad.

| Hook | Eigene Implementierungen | Ergebnis/Default |
| --- | --- | --- |
| `list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing>` | SFTP, FTP, DAV, Drive, SMB | Benannte Childfehler sind Omissions; gebrochene oder kollidierende Enumeration ist Err. Link-/Special-Flags bleiben erhalten. |
| `open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>>` | SFTP, Drive | SFTP LSTAT-Guard mit ehrlicher v3-OPEN-Grenze; Drive verweigert Native/Shortcut/Folder und verwendet die exakte ID. Andere Provider behalten ihre vorhandene Default-/Readgrenze. |
| `target_limits(&self, root: &str) -> TargetLimits` | alle | SFTP statvfs-Bytes/Seconds; FTP beobachtete MLSx-/LIST-Präzision oder Unknown; DAV Seconds; Drive Millis ohne pauschale Windowsnamen; SMB Windows/255 UTF-16/Unknown. Unbekannte FileSize-Limits bleiben None. |
| `unix_mode(&self, path: &str) -> VfsResult<Option<u32>>` | SFTP | Effektiv gelesenes `mode & 0o777`. Andere Provider None. |
| `open_write_copy_stage_timed(&self, path: &str, size: u64, mtime_ms: i64) -> VfsResult<Box<dyn Write + Send>>` | DAV, Drive | DAV PUT-Header/Drive Create-Metadaten; bestehende sized-Streaming-/Spool-/Backpressure-/Retry-Grenzen bleiben. SFTP/FTP/SMB behalten sized Default und setzen Zeit beim Finish. |
| `finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished>` | alle | Effektive Zeit/Modeprüfung, keine erfundene fsync-Garantie. SFTP/SMB durable nur nach bestätigt erfolgreichem Datei-Flush, FTP/DAV/Drive false. |
| `sync_child_path(&self, parent: &str, literal_name: &str) -> VfsResult<String>` | Drive | Genau einen neuen Literalnamen kodieren, encoded Parent unverändert; keine URI-Dekodierung. Andere Provider verwenden gemeinsamen Default. |
| `previous_state_identities(&self) -> VfsResult<Vec<String>>` | Drive | Maximal der aktuelle alte Tokenhash; bei mehrdeutigem früher getrimmten Root leer. Keine tokenfremden IDs. |
| `replace_staged_reversible(&self, staged: &str, destination: &str, retained: &str) -> VfsResult<bool>` | SFTP, FTP | SFTP true nach erfolgreicher Stage-Publikation mit Original am journalten retained; Fehler erhält bekannte Inhalte/Pfade oder stellt NoReplace zurück. FTP false vor Mutation. Andere Provider Default false. |
| `change_signal_mode(&self, root: &str) -> VfsResult<Option<ChangeSignalMode>>` / `change_signal(&self, root: &str, interval: Duration, tx: crossbeam_channel::Sender<ChangeNotice>) -> VfsResult<Option<ChangeSubscription>>` | Drive, DAV | Beide Poll; Drive accountweiter Changes-Feed/Ready, DAV vorhandener Root-ETag/ReadyPartial. Andere Provider Default ohne Push-Versprechen. |

Gemeinsame Freecalls bleiben der Consumer-Einstieg: `vfs::{sync_child_path, sync_path, previous_state_identities, replace_staged_reversible, list_dir_tolerant, open_read_regular, open_write_copy_stage_timed, finish_stage, target_limits, unix_mode, change_signal_mode, change_signal}`. `sync_path` verarbeitet literal relative Komponenten unter einem unveränderten Providerroot.

`previous_state_identities` wird im gemeinsamen Freecall auf nichtleere Identitäten/maximal acht/je 4 KiB begrenzt. Drive liefert ausschließlich `gdrive:path-v2:<Sha256(current_refresh_token)[0..12] als lowerhex>:<root>`. Neue `state_identity` ist `gdrive:path-v2:<drive_account_key>:<root>`; `namespace_identity` bleibt kontoweit.

## Reversible Ersatzgrenze

Retained ist exakt `<destination-parent>/.se-replace-<16lowerhex>`, ein vorher frei gewählter Sibling. Stage/Destination/Retained sind verschieden und innerhalb derselben Namensgrenze. Der Caller persistiert die private Recoveryabsicht **vor** dem ersten Request. Gemeinsamer Validator und Stagefilter sind in `810b0df9` daran angepasst.

SFTP verwendet v3-NoReplace-RENAME, nicht `posix-rename`, für Capture, Publish und Restore. Erfolg löscht das Original nicht; Cleanup/Versionszuordnung sind Enginearbeit. Unklare Antworten werden nicht mutierend wiederholt; Err nennt alle drei bereits bekannten Pfade. Belegte Typ-/Metadatenänderung beim Capture erhält das Objekt für Recovery. Namespace-Fähigkeiten und der bestehende atomare `promote_staged_with`-Callback werden nicht auf Multi-Rename umgedeutet.

FTP kann RFC-959-NoReplace weder für Capture noch für Restore belegen; `Ok(false)` erfolgt deshalb ohne Probe/Rename/Löschung. Vorhandene FTP staged-Create-/manual-Upload-Fähigkeiten bleiben mit ihren bisherigen Absenzprobe-Grenzen verfügbar. DAV MOVE und Drive Selected-ID-Ersetzen bleiben ihre eigenen vorhandenen, nicht atomaren Publishwege.

## Interne kohäsive APIs

- `crate::connect::{PollNotice, poll_subscription}` ist crate-intern. `poll_subscription(interval: Duration, tx: crossbeam_channel::Sender<ChangeNotice>, complete: bool, poll: impl FnMut(&AtomicBool) -> io::Result<PollNotice> + Send + 'static) -> io::Result<ChangeSubscription>` hält einen Drop-Cancel-Token, weckt die Ruhephase und begrenzt Queue-Backpressure mit cancelbaren Send-Scheiben. `PollNotice::{Quiet, Changed, Overflow}`; Mindestintervall eine Sekunde.
- Drive `legacy_state_identity(&self) -> VfsResult<String>` ist crate-intern; Engine verwendet den allgemeinen VFS-Hook. Neue private `drive_changes_since_poll` prüft den Poll-Cancel zwischen Seiten. `changes_since` bleibt accountweit; `VfsChange.rel == None`, literal Name/ID/Parent-ID, Removed ohne benötigte File-Metadaten. Nur vollständige terminale Pagination bestätigt den Cursor.
- DAV `propfind` liefert den privaten budgethaltenden `listing_body::Body` mit `Deref<Target=str>`; die Reservierung bleibt bis nach Parsing am Body. Depth-0-Parser ist eine private, exakte Resource-Grenze.
- `NewObject`/Drive `SizedWriter::open`/Selected-ID-Media-Update tragen optionales `mtime_ms`; alle betroffenen Konstruktoren/Fixturewerte wurden angeschlossen. Vorhandene Create-/Copy-/Exportaufrufe behalten ihre Bedeutung.
- SMB `stage_finish::{filetime, set_mtime, flush}` ist privat; die Wire-Nachricht wird über öffentliche `Connection::execute` gesendet. Kein private-API-Aufruf und keine zusätzliche Cargo-/Featureänderung.
- FTP `metadata::{list, list_browse, stat, read_time}` bleibt privat. Nur Browsing darf auf nacktes LIST ausweichen; `list_dir_for_sync`, V1-tolerante Enumeration und Abwesenheitsbeweise verwenden die stärkere Erfassung.

## Registrierungen und Dateien

Eigene additive Modulregistrierungen:
- `native/src/connect/mod.rs`: `os/shared/poll_signal.rs`, crate-interne Reexports.
- `native/src/ftp/mod.rs`: `core/{errors,metadata,extensions}.rs`.
- `native/src/sftp/mod.rs`: `core/{extensions,reversible_replace}.rs`.
- `native/src/webdav/mod.rs`: `core/{extensions,listing_body,metadata,stage_move}.rs`.
- `native/src/smb/mod.rs`: `core/{extensions,stage_finish}.rs`.
- `native/src/gdrive/mod.rs`: `core/{extensions,sync_listing,stage_time,remote_provider_task_tests}.rs`; der letzte Eintrag nur cfg(test).
- `native/src/ftp/core/transfer_engine_task_tests.rs`: ausgegliederte lokale Fixture `transfer_fixture.rs`.

Die ausschließlich für globale Registrierung freigegebenen `share/vfs/bisync/syncjobs/daemon/mod.rs` und `lib.rs` wurden durch V-REMOTE nicht geändert. Exakte erstellte/geänderte/gelesene Pfade stehen vollständig in [abnahme/V-REMOTE.md](../abnahme/V-REMOTE.md); dort sind alle eigenen Dateien, keine Verzeichnis-Wildcards als Arbeitsnachweis.

## Consumer-Grenzen

Engine konsumiert Literalpfad-Hook, Legacy-State-Hinweis, Accountfeed und vorher journalten Ersatz; Adapter dürfen deren Defaults nicht still verlieren. Cache/Unavailable-Weiterleitung für die neuen Parent-Hooks wurde laut Hauptagent bereits registriert, Agent-/Host-Weiterleitung bleibt beim Owner. Konkrete Owneraufträge und Akzeptanzsignale stehen in [anfragen/V-REMOTE.md](../anfragen/V-REMOTE.md). Y135/Y138 sind nicht umgesetzt. Lokale Ausführung und Graphaktualisierung wurden nicht vorgenommen.
