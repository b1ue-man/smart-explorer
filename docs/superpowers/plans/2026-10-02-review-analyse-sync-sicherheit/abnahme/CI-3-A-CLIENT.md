# CI-3-A-CLIENT – Share-Versionen und abgeschlossener Split-Lauf

Stand: 2026-10-03. Umsetzung im Worktree; ausschließlich statischer Self-Review, keine lokale Ausführung und keine Aussage über eine erfolgreiche Remote-Abnahme. Grundlage sind der Root-Plan `ci-closure-fixes.md`, der gespeicherte Scope `scopes/ci-3-a-client.json` und die zugewiesenen Diagnosen aus Run `37157166735` / Kandidat `71a8ca45697272453c213c9b0b5412d0cbed0f71`. Root besitzt Integration und dieselbe RV1-Suite.

## Befund und Umsetzung

1. Beide Betriebssysteme melden im echten Share→Share-Lauf `("Versionen", "Pfad ist nicht freigegeben")`. `share::fs_policy::private_name` schützt `.se-versions` absichtlich. `version_save::save` besitzt für Auto bereits den sicheren privaten Fallback: Quelle erneut validieren, privates Manifest/Stage durabel veröffentlichen und vor dem destruktiven Apply die gesicherte Quelle prüfen. Listing und Retention fragten unabhängig davon bislang immer den Provider-Archivpfad ab.
2. `version_listing::managed_sync_root(&VersionSide, pair, cancel) -> io::Result<Vec<Managed>>` fasst die Archivprobe zusammen. Ausschließlich `PermissionDenied` beim **ersten** `stat(<side.root>/.se-versions)` auf `Scheme::Peer` führt zur privaten Archivwahl. Davor und danach gilt Cancellation; `stat(side.root)` muss weiterhin eine lesbare, gewöhnliche Root bestätigen. AppData- und Legacy-Discovery sowie Retention laufen danach vollständig weiter. Andere Provider, Root-Verweigerungen, ReadOnlyFilesystem, innere Archivfehler, Links, unvollständige Listings und Manifest-/Bytefehler bleiben Fehler. Der vorhandene Save/Fallback und die Sharepolicy bleiben unverändert.
3. Der Windows-Splitbefund zeigt die laufende Notice bei fehlender Zieldatei. Die aktuellen Definitionen belegen keinen vorzeitigen Produktionsreset: `drain_job_connect` ist leer, `drain_conflict_resolution` kehrt ohne Aufgabe/Terminalresultat zurück, und `drain_bisync` löscht Receiver/Running erst nach Outcome oder Disconnect. Deshalb wird keine hypothetische Produktionsursache behauptet oder geändert.
4. Der bestehende Fixture-Consumer `finish` ruft nun auch `drain_desktop_sync_workers` auf, wartet auf alle relevanten Receiver/Flags und joined ausschließlich tatsächlich fertige Worker über den vorhandenen Vertrag. Die Frist bleibt **30 Sekunden**. Workerpanik und Disconnect bleiben Fehler. Der Split prüft danach den wirklichen Engine-State, beide unverpackten Backendhandles, die unveränderten Rootstrings und die terminale Resultprojektion, bevor die vorhandene harte Byteassertion läuft.
5. Im vorhandenen echten Share-Testsymbol folgt auf den unveränderten Default-Lauf eine ausdrücklich mit `VersionsLocation::Auto` gewählte Überschreibung. Die Fixture prüft die weiterhin gesperrten Archivpfade, die private `VersionStore::AppData`-Version mit den alten Bytes, `PairLock::acquire(state.lock_id)` und `restore_version`. Restore muss seinerseits die ersetzten neuen Bytes als `VersionReason::Restored` behalten. Ein weiterer echter Share-Lauf überträgt das Restore zurück; die ursprüngliche Local-Roundtrip-Assertion mit `b"first peer"` bleibt unverändert. Unterschiedliche Bytelängen machen diese Änderung ohne Uhrzeit-/Sleep-Annahme sichtbar.

## Übernommener enger Milestoneplan

| Milestone | Konkrete Grenze | Erwartetes Signal in derselben RV1-Suite |
| --- | --- | --- |
| Share-Archivwahl | `version_listing::list` und `version_ops::maintain_member` verwenden dieselbe begrenzte Probe | Echte Share-Defaults erzeugen keinen falschen Versionsfehler; private Retention bleibt wirksam. |
| Reversibles Share-Overwrite | Bestehendes Share-Testsymbol und bestehende öffentliche Versions-/PairLock-APIs | Alte Bytes sind privat erhalten, Restore erhält die ersetzte Datei und wird zum anderen Peer übertragen. |
| Split-Abschluss | Bestehendes `finish` und Split-Testsymbol | Erst nach Workerabschluss und Ergebnisprojektion werden echte ungecachte Zielbytes geprüft; kein längeres Timeout oder Busy-Bypass. |

## Dateien geändert und erstellt

Geändert:

- `native/src/bisync/os/shared/version_listing.rs`: ausschließlich `Scheme`-Import, `managed_sync_root` und dessen Listing-Consumer.
- `native/src/bisync/os/shared/version_ops.rs`: ausschließlich Import und Probe-Consumer in `maintain_member`.
- `native/src/app/os/shared/sync_paths_task_tests.rs`: bestehender Abschlusshelper, Split-Abnahmeassertionen und zusätzliche Backup-/Restore-Phase im vorhandenen echten Share-Symbol.

Erstellt:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-A-CLIENT.md` (dieser Bericht).

Keine neuen Quellmodule, Registrierungen oder Testsymbole. `sync_paths_task_fixture.rs`, `sync_run_state.rs`, Produktionsdrainer und Transport bleiben unverändert.

## Gelesene Dateien

Die gespeicherten Symbol-/Zeilenlimits gelten für die unten genannten ergänzten Flächen; außerhalb des gespeicherten Scopes wurde nichts untersucht.

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-3-a-client.json`
- `/tmp/rv1-ci-third/a-client.json`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/local-fs-identity-durability.md` (passender Dateinamen-/Durability-Vertrag)
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-closure-fixes.md`
- `native/src/app/mod.rs` (zugewiesene Modulregistrierungen)
- `native/src/app/core/sync_core.rs`
- `native/src/app/core/sync_run_state.rs`
- `native/src/app/os/shared/sync_manual_run.rs`
- `native/src/app/os/shared/sync_paths_task_fixture.rs`
- `native/src/app/os/shared/sync_paths_task_tests.rs`
- `native/src/app/core/bisync_ui.rs` (`drain_bisync`/Resultprojektion)
- `native/src/app/core/bisync_conflicts.rs` (`drain_conflict_resolution`/Resultprojektion)
- `native/src/app/os/shared/sync_jobs.rs` (`drain_job_connect`)
- `native/src/bisync/mod.rs` (Versions-/PairLock-/PairSide-/StateKey-Reexports)
- `native/src/bisync/core/types.rs` (BisyncOptions, Versioning, VersionsLocation, PairSide und Defaults)
- `native/src/bisync/core/run_types.rs`
- `native/src/bisync/os/shared/versions.rs`
- `native/src/bisync/os/shared/version_save.rs`
- `native/src/bisync/os/shared/version_listing.rs`
- `native/src/bisync/os/shared/version_ops.rs` (zugewiesene Listing-/Retention-/Sides-Flächen)
- `native/src/bisync/os/shared/persistence_versions.rs`
- `native/src/share/core/fs.rs`
- `native/src/share/core/fs_access.rs`
- `native/src/share/core/fs_capabilities.rs`
- `native/src/share/core/fs_policy.rs` (private_path/system_write)
- `native/src/share/core/backend.rs` (freigegebene Scheme-/Identitäts-/Extensions-Symbole und `stat`)
- `native/src/share/core/peer_request.rs` (`request`/`request_with_lease_until`, freigegebener Erfolgs-/Decodezweig)
- `native/src/share/core/framing.rs` (`decode_resp`, freigegebene Zeilen 50–62)
- `native/src/share/core/fs_error.rs` (`kind_of`/`into_io`)
- `native/src/share/os/shared/fs_host_policy.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/cache.rs`
- `native/src/vfs/core/extensions.rs` (vorhandene Feature-/Capability-Grenzen)
- `native/src/vfs/os/shared/sync_roots.rs`

## Self-Review und Entscheidungen

- Statischer Vergleich mit den gelesenen Ausgangstexten: alle bisherigen Assertions bleiben erhalten. Nur die Deadline-Assertion erhält bei derselben Bedingung eine ausführlichere Zustandsdiagnose. Die vorhandenen Testsymbole und ihre Suite-Einbindung bleiben identisch.
- `version_listing::{children,managed,legacy,budget}` sind bytegleich erhalten. Der neue Ausnahmezweig liegt vor dem vollständigen Archivwalk; innere Fehler werden deshalb weiterhin propagiert. Die AppData-/Legacy-Retention in `maintain_member` bleibt unverändert.
- Die tatsächliche Fehlerkette wurde frisch gelesen: `Backend::stat → request → request_with_lease_until → decode_resp(FsResponse::Err { kind, msg }) → into_io`. `PermissionDenied` bleibt derselbe IO-Fehlertyp; `Unknown`/fehlendes Kind bleibt `Other` und wird von der neuen Probe nicht behandelt. Kein Fehlertextmatching.
- Öffentliches `VersionSide`/`VersionEntry`/`VersionStore`/`VersionReason`, authoritative `StateKey::{pair_id,lock_id}`, die bestehenden Reexports und `restore_version`-Argumente wurden gegen die tatsächlichen Definitionen abgeglichen.
- Keine Änderungen an RO/RW, Principals, Grants, Literalnamen, Providerroots, Endpoints, Verbindungsidentitäten, Abbruch, Teilresultaten oder Baseline-Bedeutung. Die Standardoption bleibt tatsächlich `VersionsLocation::AppData`; Auto ist nur ausdrücklich in der ergänzten Fixturephase gewählt.
- Kein globaler PermissionDenied-/Read-only-Catch, kein Scheme-only-Skip und kein neues Archivformat. Save verweigert weiterhin mehrdeutige/committed Renames und sichert vor Destruktion.
- Geänderte Rustdateien liegen unter 500 Zeilen und 50 KiB: `sync_paths_task_tests.rs` 404 Zeilen/14.748 Bytes, `version_listing.rs` 249 Zeilen/9.290 Bytes, `version_ops.rs` 299 Zeilen/11.285 Bytes. Keine lokale Ausführung, Compiler-/Formatter-/Test-/Git-/CI-/Graph-/Releaseaktion.

## Abnahmesignale und offene Grenzen

Root führt ausschließlich dieselbe RV1-Suite aus. Betroffene vorhandene Symbole:

- `app::sync_paths_task_tests::sync_paths_task_real_share_cross_peer_sync_and_local_roundtrip`: echte Sharetransfers, explizites Auto-Overwrite, private alte Bytes, Restore mit erhaltenen neuen Bytes, Rückübertragung und unveränderte Local-Bytes.
- `app::sync_paths_task_tests::sync_paths_task_split_same_paths_on_different_remotes_and_uncached_metadata`: getrennte Namespace-/Verbindungsidentitäten, gleiche relative Literalroots, gejointe Worker, konsumierte Receiver, aktueller Engine-State und echte Zielbytes.
- Der geteilte `finish` bleibt auch beim bestehenden Picker-/Quick-Action- und Saved-Local-Job-Consumer aktiv; deren bisherigen Assertions bleiben unverändert.

Windows `("Zwischenstand", "Access is denied. (os error 5)")` ist weiterhin V-LOCALs private Handle-/Ancestorgrenze. Die neue Share-Archivprobe behandelt weder lokale AppData-Verweigerungen noch Checkpointfehler; sie maskiert diese Kaskade nicht. Die historische Splitursache ist durch statische Sourceinspektion nicht abschließend belegbar; der reale Abschluss ist jetzt eine harte Fixture-Voraussetzung, deren Ergebnis Root mit demselben Runpfad bewertet. Erfolgreiche Laufzeitabnahme und Release werden nicht behauptet.
