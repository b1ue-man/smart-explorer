# V-REMOTE – Providervertrag und Abnahme

Stand: 2026-10-03. **Providerimplementierung abgeschlossen; die gemeinsame Remote-Abnahme ist noch nicht ausgeführt.** Consumer-Anschlüsse bleiben bei den benannten Ownern in [anfragen/V-REMOTE.md](../anfragen/V-REMOTE.md). Signaturen und Registrierungen stehen in [api-delta/V-REMOTE.md](../api-delta/V-REMOTE.md).

## Grundlage und Grenze

Umgesetzt wird ausschließlich der freigegebene RV1-Abschnitt FS2/FS4/FS6/FS7 mit dem bestehenden Plan und abgeschlossenen Kritiker. `scopes/v-remote.json` begrenzt Lesen, Änderungen und neue kohäsive Dateien. `arbeitsweise` und `graphify` gelten; der Hauptagent hat die Graph-Einstiege geliefert. Keine neue Review-Kampagne und kein fremder Consumer wurden bearbeitet.

Doku-Kontextgate vor der Bearbeitung: Git-Status, HEAD und gezielte Befund-/Quellenabfragen; bei Abschluss erneut geprüft, zuletzt beobachteter HEAD `0a2a39e2`. Die sieben vorhandenen Provider-Teiländerungen an `special`/Formatierung wurden weitergeführt. Protokollbelege sind `docs/refs/sync-remote-metadata.md` (lokal gesicherter Abruf 2026-10-02), `ftp-pool.md`, `smb2.md` und `sync-change-detection.md`. SMB-FLUSH wurde zusätzlich gegen die zwei ausdrücklich freigegebenen Quellen von `smb2 0.26.0` geprüft: öffentliche Nachrichtentypen und `Connection::execute`; die private `Tree::flush_handle` wird nicht verwendet.

## Umsetzung und zugeordnete Befunde

| Grenze | Umgesetztes Verhalten | Befunde und verbleibender Anschluss |
| --- | --- | --- |
| R1 Locator/Identität | Bestehende gemeinsame `EndpointSpec`-/`resolve_endpoint`-/`SavedConnection`-Bedeutung bleibt bestehen. FTP/SMB-URL-Adapter und DAV/Drive-Rootöffnung entfernen keine literal abschließenden Leerzeichen. FTP/DAV nehmen geklammerte IPv6-Hosts an. Drive-State verwendet den bereits authentifizierten stabilen Accountkey; der aktuelle alte Tokenhash ist nur ein Migrationshinweis. | Y124/Y132; Paar-/Baseline-Migration, Agent-Weiterleitung und Account-Ancestry sind Engine-/Hüllenarbeit. |
| R2 SFTP/SSH-Agent-Fallback | Typisierte SFTP-Statusfehler; `NoSuchFile` allein ist NotFound. READDIR hat keinen zusätzlichen 20-s-Gesamtdeckel. Special-/Link-Flags, geschützte lossy/unadressierbare Namen und Kollisionsfehler. SETSTAT nutzt `FileAttributes::empty()`, erhält atime, setzt gültige v3-Sekunden und restriktiven Mode; erneutes LSTAT prüft Zeit/Mode/Typ. statvfs liefert Namenslimit und verfeinert nur belegte Readonly-/Full-Fehler. Bestätigter beworbener Datei-fsync ohne Namespace-Claim. Separater reversibler NoReplace-Hook erhält das alte Original am vorab bekannten Namen. MKDIR verschluckt keine beliebigen Statusfehler mehr. | Y115/Y121/Y129/Y130/Y136/Y142; FS2/4/6/7. Agent-Hülle muss effektive Seitenpräzision weitergeben. Reversible Recovery wird vorher journalisiert und später von der Engine abgeschlossen. |
| R3 FTP/FTPS | FEAT-verhandeltes MLST/MLSD, eigene literal MLSx-/UTC-/u64-Parser. SIZE/MDTM-Einzelstat und MLST-Promotion sparen Voll-Listen bei belegten Einträgen. 550 allein ist niemals Abwesenheitsbeweis. LIST-Fallback wechselt per CWD in die richtige Collection und fordert Dotnamen mit `-a`; reines LIST bleibt beim Browsen alter Server möglich, niemals als sicherer Sync-/Abwesenheitsbeweis. Fehlende Size-Facts erzeugen keinen erfundenen leeren Inhalt. MFMT nur beworben und per MDTM wirksam bestätigt. Terminale STOR-Antwort wird auch nach Datenfehlern abgeholt; 452/552 bleiben Full/Quota. TCP-Keepalive auch auf Datenkanälen; keine NOOP-Befehle in laufende Transfers. OS-DNS nutzt einen begrenzten Worker. | Y66/Y116/Y127/Y128/Y130/Y137; FS2/4/7. Bestehende sized Streaming-Stages bleiben für Y65/Y117/Y133 verfügbar. Y140: portables reversibles Ersetzen ist mangels protokollgesichertem NoReplace vor Mutation `Ok(false)`; kein Löschen des Originals. |
| R4 WebDAV | MKCOL ausschließlich unter der ausgewählten Wurzel; Root/Protokollvorfahren werden nicht angelegt. Depth-1 unterscheidet unabhängige Child-Omissions und kaputte/unvollständige Enumeration. UTF-8-Fehler werden als lossy Child gemeldet; echter U+FFFD bleibt literal adressierbar. PROPFIND umgeht die feste 10-MiB-`into_string`-Grenze über RAM-abgeleitete Limits und geteilte Reservierungen bis zum XML-Parse. Timed PUT setzt zulässiges `X-OC-Mtime`, Finish prüft effektive Zeit und verwendet nur belegten Nextcloud-Fallback. Die verifizierte Stage-Zeit und ein vorhandener starker ETag begleiten genau einen MOVE; verlorene Zeit nach Commit ist ein Fehler mit bekannten Pfaden. Hash-Fähigkeit wird erst bei tatsächlich beobachtetem MD5 wahr. Root-ETag-Poll ist partielle Abdeckung. | Y114/Y118/Y130/Y136/Y143; FS2/4/7. XML bleibt DOM-basiert; Budgetüberschreitung ist ein Fehler, kein vollständiger leerer Scan. |
| R5 Drive | Sync-Listings liefern literal `metadata.name` und exakte IDs; neue Child-Komponenten werden über den gemeinsamen Hook genau einmal kodiert, vorhandene kodierte Parent-Locators bleiben unverändert. Mehrdeutige Folder-Titel schützen den Teilbaum; Datei-Duplikate behalten ihre IDs. Unvollständige benannte Metadaten sind Omissions. Native Docs sind Special, Shortcuts Links; manuelle Export-/Browserpfade bleiben bestehen. Timed Create und exakte ID-Media-Updates übertragen `modifiedTime`; Ersatz erhält Ziel-ID, bestätigt Inhalt und Zeit vor Stage-Cleanup und bleibt retrybar. Accountweiter Feed trägt IDs/literal Titel, Removed bleibt ID-basiert. Seiten, Token, Metadaten und Speicherbudget sind geprüft; nur die vollständige terminale Seite liefert den neuen Cursor. Reset/410/Overflow fordert Vollplanung. | Y124/Y132/Y134/Y136/Y139; FS2/4/7. Y65/Y117/Y133: bestehende Upload-/Selected-ID-/KeepBoth-Primitiven erhalten; Engine-Anschluss bleibt separat. |
| R6 SMB und Poll-Lebensdauer | LastWriteTime wird als FileBasicInformation an einem geprüften regulären Handle gesetzt und effektiv gelesen; kein anderer Zeit-/Attributwert wird verändert. Öffnen prüft Reparse-/Data-Tag am selben Handle. FLUSH nutzt die öffentliche Wire-API und schließt den Handle auch bei Fehlern. Namenslimits sind Windows/255 UTF-16; Zeitauflösung bleibt ohne belegte Server-Dateisysteminformation Unknown. Quota/Readonly bleiben typisiert; vollständige Listing-Seiten dürfen nicht kollidieren. Owned Polls sind abwerfbar, beenden Backpressure-Warten und Drive-Paginierung bei Cancel. | Y57/Y113/Y129/Y130/Y136 sowie FS2/4/6/7. Kein Batch-/Rename-fsync erfunden; alle Provider behalten `sync_filesystem == false`. |

Zeitübernahme ist nur dann bestätigt, wenn die effektive Zeit in der gemeldeten Präzision stimmt. Nicht übertragbare Zeiten liefern `mtime_applied: false`; FTP/DAV/Drive bestätigen keine fsync-Dauerhaftigkeit. SFTP/SMB bestätigen nur den tatsächlich erfolgreichen Datei-Flush. SFTP v3 bietet bei OPEN kein O_NOFOLLOW; der Adapter behauptet deshalb keine lokale Handle-Konfinierung. FTP-RNTO und DAV-MOVE erhalten keine erfundene atomare Namespace-Fähigkeit.

## Signale für die eine spätere Remote-Suite

Diese erwarteten Ergebnisse sind die Abnahme des vorhandenen R1–R6-Plans. Keine lokale Invocation wurde ausgeführt; der Hauptagent sammelt sie im einen Suite-Einstieg mit den direkt betroffenen Integrationen.

| Suite-Stufe | Beobachtbares Ergebnis |
| --- | --- |
| R1 gemeinsame Locations | Bestehende `sync_paths_task_picker_locators_preserve_literal_paths_and_credentials` plus Local/UNC/mapped, SFTP-Agent/Fallback, FTP/FTPS, DAV, Drive, Direct/Room und zwei verschiedene Remotes mit gleichem Relativpfad: Host/Port/User/Auth/Connection-ID und literal `%20/#/?/Leerzeichen` bleiben erhalten, remote wird niemals als lokaler Path geöffnet. Readonly-/Known-host-/Backendfehler werden nicht zu Locator-Formatfehlern. |
| R2 SFTP-Servervarianten | OpenSSH sowie ein Server ohne posix-rename/fsync/statvfs: >20 s fortschreitendes READDIR gelingt, ein einzelner stockender Request scheitert weiterhin begrenzt. NoSuchFile/Permission/Failure bleiben getrennt. SETSTAT erhält atime, Mode 0600 erweitert keine Rechte; tatsächliche Sekunden werden bestätigt. Typwechsel, unadressierbare Childs und Geräte schützen deren Counterparts. Nur wirklich bestätigter fsync meldet durable. |
| R2 reversible Recovery | Caller persistiert `staged/destination/.se-replace-16lowerhex` vor dem ersten Rename. Fehler/Lost-ACK vor oder nach jedem Rename, belegte fremde Destination und fehlgeschlagene Rücknahme: kein Original wird gelöscht, keine belegte Ersatzdatei überschrieben; beide Inhalte bleiben unter den journalten Pfaden auffindbar. Recovery ist NoReplace, Cleanup erst nach Engine-Commit. Atomarer posix-rename-Callback bleibt eine einzelne Operation. |
| R3 FTP/FTPS | MLSx mit Leerzeichen/Semikola/Bruchsekunden, Dotfile, cdir/pdir, Special/Link, malformed Child, u64-SIZE jenseits 32 Bit, FEAT-freier Server, alter Plain-LIST-Browser, 550-Permission und echtes fehlendes Child. Gute Einzelstat-/Promotionpfade verwenden MLST/SIZE/MDTM; Abwesenheit fordert vollständige Parent-Erfassung. MFMT wirkt oder meldet false. Nach Daten-EPIPE bleibt terminales 452/552 ein TargetRefusal, nicht Verbindungsfehler. Idle-/Langtransfer hat TCP-Keepalive, kein in den STOR/RETR-Kanal eingeschobenes NOOP. OS-Resolver findet Hosts entsprechend der Hostkonfiguration; wartende Aufrufe enden innerhalb ihrer Setup-Deadline. |
| R3 Windows-FTP-Ersetzen | Provider-Hook liefert vor Mutation false. Engine entscheidet sicheren Alternate/Fehler und lässt das Original einschließlich Backup/Baseline unverändert; kein unsicheres rm+RNTO-Fallback. Bestehendes staged Create und manueller Upload behalten ihre ausdrücklich bekannten FTP-Protokollgrenzen. |
| R4 DAV/Nextcloud | Vorhandene Wurzel `/remote.php/dav/files/user`: nur Kind-MKCOL, nie Servervorfahren. Listing >10 MiB innerhalb des verfügbaren Budgets gelingt, über Budget scheitert sichtbar ohne Cursor/Index-Vollständigkeit. Child-403/404/invalid UTF-8 wird ausgelassen und schützt nur den benannten Teilbaum. Timed PUT/Finish/MOVE erhalten wirksame Source-Sekunden auf Nextcloud; generischer Dead-Property-Erfolg reicht nicht. Starker ETag wird beim bekannten Stage-MOVE verwendet. Ohne beobachtete MD5 ist Hash-Fähigkeit false. Poll startet mit ReadyPartial und Hybrid-Prüfungen laufen weiter. |
| R5 Drive-Vertrag | Hinterlegte `rv1_remote_provider_task_drive_*`-Fälle prüfen Accountidentität bei Tokenrotation, nur bewiesenen aktuellen Legacyhash, literal `%3A/CON/Leerzeichen`, Folder-Duplikat-/Native-/Shortcut-Omission, exakte Ziel-ID und Millisekunden bei Ersatz sowie accountweite paginierte Upsert-/Removed-IDs ohne erfundene Relativpfade. Wiederholter Token, fehlender terminaler Cursor oder falsch typisiertes Boolean ergeben keinen nutzbaren Teilcursor. Ergänzend Engine-Migration unter Owner-/PairLock, Removed-/Move-Ancestry und Duplicate-Beobachtung sowie 410/Overflow-Vollplanung prüfen. |
| R6 SMB | Samba und Windows-Share mit Link-/Data-Reparse-Varianten: nur die LastWriteTime-Bytes 16..24 des 40-Byte-Werts ändern sich; Null-/negative FILETIME-Kontrollwerte werden nicht gesetzt. Effective Stat und Wire-FLUSH werden unabhängig bestätigt. Handle bleibt beim Typwechsel geschützt; Doppelseiten/UTF-16-Fehler können keinen vollständigen Index erzeugen. Quota/Readonly-Fälle stoppen Apply beim Owner, Windowsnamen werden ausgelassen und niemals zu ADS. Share bleibt Teil des Root-Locators. |
| R1–R6 Lebensdauer/zweiter Lauf | Drop während Ruhe, voller bounded Ereignisqueue oder Feed-Seite beendet den Worker nach dem laufenden begrenzten Provideraufruf; kein Teilcursor wird fortgeschrieben. Abbruch/Backupfehler/Lost-ACK erhalten Retrydaten und alte Baseline. Unveränderter zweiter Mirror-Lauf überträgt weder Dateien noch Backups erneut, auch bei `mtime_applied: false` über die Engine-Basis. Eigene Stages, Links und Special-Einträge werden als Schutzgrenze behandelt; ein Teilscan seedet keinen vollständigen Index. |

Vorhandene FTP-Transferfixture wurde kohäsiv ausgegliedert und für CWD/PWD/LIST -a/SIZE angepasst. Vorhandene DAV-PROPFIND-Assertions wurden an den budgethaltenden Body angepasst; bestehende Drive-Fixtures tragen die wirksamen Metadata-Zeiten. Die neue DAV-Literal-Unicode-Regression gehört ebenfalls zur selben Suite.

## Entscheidungen und offene Grenzen

- Y140 bleibt eine akzeptierte Protokollgrenze: FTP verfügt über kein belastbares NoReplace-RNTO für Einfangen und Rücknahme. Der neue Hook mutiert deshalb nicht. Die engineeigene sichere Alternative ist keine erledigte Providerfähigkeit.
- Y142 ist additiv und reversibel, kein Multi-Rename unter einem atomaren Callback. Der Hauptagent hat gemeinsamen Validator und Stagefilter in `810b0df9` auf exakt `.se-replace-<16lowerhex>` ausgerichtet; die zuvor gemeldete Namensdiskrepanz ist geschlossen.
- Y132 migriert nur den nach dem alten Algorithmus berechneten Hash des aktuell vorhandenen Tokens. Verlorene Tokens/andere Konten/mehrdeutige alte Space-Roots werden nicht geraten.
- Y134 Feed/Signal ist accountweit. Ancestry, gelöschte IDs und Duplikate sind keine vom Provider erfundenen Pfade.
- FTP-OS-DNS lässt sich innerhalb `getaddrinfo` nicht abbrechen. Ein einzelner Worker plus begrenzte Queue verhindert unbeschränkte Worker; Aufrufer bleiben begrenzt.
- Y135 Cross-run-Transferresume und Y138 FTPS-/DAV-Zertifikatspins bleiben bewusst zurückgestellt. Share-/Host-Befunde Y123/Y126/Y141 gehören anderen Ownern.
- Offene Hüllen-/Engine-/Remote-Suite-Arbeit ist exakt im Anfragebericht festgehalten; V-REMOTE enthält keine Host-/Planner-/Apply-/Job-/UI-/CI-/Release-/Graphänderung.

## Eigener statischer Self-Review

Eigene geänderte und neue Quellen wurden per Text/Diff und vorhandenem Tree-sitter-Rust-Parser geprüft: keine Syntax-ERROR-/Missing-Nodes; sämtliche eigenen Rustdateien unter 500 Zeilen und 50 KiB. `git diff --check` für die eigene Dateiliste ist ohne Befund. Kohäsive Module sind registriert, Signaturen/Konstruktoren und relevante Protokollfehler wurden statisch gegengeprüft. Kein separater Prüfer, kein Compiler, Formatter, Build, Test, Server, Installations-, Commit-/Push-, CI-/Release- oder Graphlauf wurde ausgeführt. Dies ist Syntax-/Textbeleg, keine Typprüfung oder Remote-Abnahme.

## Erstellte Dateien

Die Pfade sind jeweils genau `Verzeichnis + Dateiname`.

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/`: `V-REMOTE.md`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/`: `V-REMOTE.md`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/`: `V-REMOTE.md`.
- `native/src/connect/os/shared/`: `poll_signal.rs`.
- `native/src/ftp/core/`: `errors.rs`, `extensions.rs`, `metadata.rs`, `transfer_fixture.rs`.
- `native/src/gdrive/core/`: `extensions.rs`, `remote_provider_task_tests.rs`, `stage_time.rs`, `sync_listing.rs`.
- `native/src/sftp/core/`: `extensions.rs`, `reversible_replace.rs`.
- `native/src/smb/core/`: `extensions.rs`, `stage_finish.rs`.
- `native/src/webdav/core/`: `extensions.rs`, `listing_body.rs`, `metadata.rs`, `stage_move.rs`.

## Geänderte bestehende Dateien

- `native/src/connect/`: `mod.rs`.
- `native/src/connect/os/shared/`: `connector.rs`.
- `native/src/ftp/core/`: `connection.rs`, `ftp.rs`, `io_adapters.rs`, `resolver.rs`, `staging.rs`, `streams.rs`, `transfer_engine_task_tests.rs`, `writer.rs`.
- `native/src/ftp/`: `mod.rs`.
- `native/src/gdrive/core/`: `backend.rs`, `changes.rs`, `core.rs`, `metadata.rs`, `names.rs`, `new_object.rs`, `promotion.rs`, `promotion_api.rs`, `promotion_tests.rs`, `sized_writer.rs`, `task_drive.rs`, `transfer.rs`, `transfer_ops.rs`.
- `native/src/gdrive/`: `mod.rs`.
- `native/src/sftp/core/`: `backend.rs`, `errors.rs`, `metadata.rs`, `posix_rename.rs`.
- `native/src/sftp/`: `mod.rs`.
- `native/src/smb/core/`: `backend.rs`, `errors.rs`, `listing.rs`, `url.rs`, `wire.rs`.
- `native/src/smb/`: `mod.rs`.
- `native/src/webdav/core/`: `connection_tests.rs`, `multistatus.rs`, `status.rs`, `stream_put.rs`, `webdav.rs`, `writer.rs`.
- `native/src/webdav/`: `mod.rs`.

## Gelesene Dateien

Die Liste umfasst Quellenlektüre, gezielte Textabfragen und eigene statisch geparste Dateien. Außerhalb davon wurden keine weiteren Repositoryinhalte erkundet.

- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/smb2-0.26.0/src/client/`: `tree.rs`.
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/smb2-0.26.0/src/msg/`: `flush.rs`.
- `docs/refs/`: `ftp-pool.md`, `smb2.md`, `sync-change-detection.md`, `sync-remote-metadata.md`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/`: `E-PLAN.md`, `V-REMOTE.md`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/`: `E-PLAN.md`, `K1.md`, `T-JOBS.md`, `V-REMOTE.md`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/`: `E-PLAN.md`, `V-REMOTE.md`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/`: `recherche.md`, `review-befunde-sync.md`, `spec.md`, `umsetzung.md`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/`: `v-remote.json`.
- `native/`: `Cargo.toml`.
- `native/src/connect/core/`: `endpoint.rs`, `location.rs`, `removal_scope.rs`, `sync_paths_task_tests.rs`, `types.rs`.
- `native/src/connect/`: `mod.rs`.
- `native/src/connect/os/shared/`: `cleanup.rs`, `connector.rs`, `location_prefs.rs`, `persistence.rs`, `poll_signal.rs`, `remote_drive_task_tests.rs`, `resolution.rs`.
- `native/src/ftp/core/`: `connection.rs`, `connection_tests.rs`, `errors.rs`, `extensions.rs`, `ftp.rs`, `ftp_tests.rs`, `io_adapters.rs`, `metadata.rs`, `pool.rs`, `resolver.rs`, `staging.rs`, `streams.rs`, `transfer_engine_task_tests.rs`, `transfer_fixture.rs`, `writer.rs`.
- `native/src/ftp/`: `mod.rs`.
- `native/src/gdrive/core/`: `api.rs`, `auth.rs`, `backend.rs`, `cache.rs`, `cache_store.rs`, `changes.rs`, `chunk_stream.rs`, `core.rs`, `duplicates.rs`, `extensions.rs`, `file_list.rs`, `folder_create_journal.rs`, `gui_task_http.rs`, `gui_task_tests.rs`, `http.rs`, `id_pool.rs`, `key_locks.rs`, `metadata.rs`, `mutation_reconcile_tests.rs`, `names.rs`, `new_object.rs`, `overload.rs`, `promotion.rs`, `promotion_api.rs`, `promotion_checks.rs`, `promotion_tests.rs`, `read_retry_tests.rs`, `remote_provider_task_tests.rs`, `resolution.rs`, `resumable.rs`, `resumable_session.rs`, `resumable_tests.rs`, `sized_writer.rs`, `stage_time.rs`, `state.rs`, `sync_conflict_task_fixture.rs`, `sync_conflict_task_safety_tests.rs`, `sync_conflict_task_tests.rs`, `sync_listing.rs`, `task_drive.rs`, `task_http.rs`, `transfer.rs`, `transfer_engine_task_ops_tests.rs`, `transfer_engine_task_tests.rs`, `transfer_ops.rs`, `trash.rs`.
- `native/src/gdrive/`: `mod.rs`.
- `native/src/gdrive/os/shared/`: `copy_writer.rs`, `copy_writer_task_tests.rs`.
- `native/src/sftp/core/`: `backend.rs`, `channel_pool.rs`, `config.rs`, `connection.rs`, `copy_data.rs`, `errors.rs`, `exec.rs`, `extensions.rs`, `io_adapters.rs`, `metadata.rs`, `pipelined_read.rs`, `pool_reader.rs`, `pool_writer.rs`, `posix_rename.rs`, `reconnect_gate.rs`, `remote_drive_task_tests.rs`, `reversible_replace.rs`, `session.rs`, `stage_copy_task_tests.rs`, `transfer_engine_task_tests.rs`, `transfer_ops.rs`, `url.rs`.
- `native/src/sftp/`: `mod.rs`.
- `native/src/sftp/os/shared/`: `known_hosts.rs`.
- `native/src/smb/core/`: `backend.rs`, `errors.rs`, `extensions.rs`, `io.rs`, `listing.rs`, `reader.rs`, `replace.rs`, `server_copy.rs`, `server_copy_task_tests.rs`, `session.rs`, `stage_finish.rs`, `tests.rs`, `transfer_engine_task_tests.rs`, `url.rs`, `wire.rs`.
- `native/src/smb/`: `mod.rs`.
- `native/src/transfer/os/shared/`: `memory.rs`.
- `native/src/vfs/core/`: `core.rs`, `error_classes.rs`, `extension_calls.rs`, `extension_types.rs`, `extensions.rs`, `fs_profile.rs`, `meta.rs`, `staging_names.rs`, `trait_defaults.rs`.
- `native/src/vfs/`: `mod.rs`.
- `native/src/webdav/core/`: `connection_tests.rs`, `copy_writer_task_tests.rs`, `extensions.rs`, `listing_body.rs`, `metadata.rs`, `multistatus.rs`, `promote_tests.rs`, `stage_move.rs`, `status.rs`, `stream_put.rs`, `transfer_engine_task_tests.rs`, `transfer_ops.rs`, `webdav.rs`, `writer.rs`.
- `native/src/webdav/`: `mod.rs`.

Zusätzliche freigegebene Skillquellen: `/root/.codex/skills/arbeitsweise/SKILL.md` und `/root/.codex/skills/graphify/SKILL.md`. Die bereitgestellten AGENTS-Instruktionen und die vom Hauptagenten ausgeführten Graphabfragen wurden als Rahmen verwendet; kein eigener Graph-Neubau.
