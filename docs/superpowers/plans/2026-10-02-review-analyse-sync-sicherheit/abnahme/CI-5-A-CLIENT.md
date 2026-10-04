# CI-5-A-CLIENT: bestätigte eigene Share-Stufen

Stand 2026-10-04: Der konkret belegte Clientanschluss ist implementiert und statisch selbst geprüft. Es gab keine lokale Ausführung und keine neue Review-Runde. Die Laufzeitabnahme gehört ausschließlich zur bestehenden, vom Root betriebenen RV1-Suite. Dieser Bericht behauptet keinen erfolgreichen Folgelauf.

**Auftrag und Beleg.** Der vollständig beendete fünfte Lauf 37167542206 meldet auf Linux und Windows beim echten Share-Sync `CopyAtoB("first Ü.txt")` beziehungsweise `CopyBtoA("second %20.txt")` den Fehler `Stage wurde nicht von diesem Backend angelegt`. `apply_stage::stage` erzeugt `.se-bisync-<16 hex>` und `stage_bytes` `.se-merge-<16 hex>`. Das bisherige `track_stage` verwendet ausschließlich die K17-Discard-Namensprüfung `.se-upload-<16 hex>`. Dadurch fehlte dem bestätigten Engine-Writer die Ownership für `finish_stage` und reversible Veröffentlichung. Der Windows-Fehler an `stable.errors.is_empty()` enthielt keine Providerdiagnose; er wird durch diesen Block nicht als derselbe Defekt klassifiziert.

**Plan und aufgelöste Anschlussfragen.** Vor dem Edit wurden der eigene Abschnitt in `ci-fifth-fixes.md`, die gesicherten Referenzen und die tatsächlichen Erstellungs-/Verbrauchspfade gelesen. Das zusammenhängende Ergebnis besteht aus Creator-Lifecycle, konsumierenden Operationen und der vorhandenen Ergebnisdiagnose. Die zuerst fehlenden Definitionen wurden vor ihrer Lektüre vom Root freigegeben: tatsächlicher Peer-Writer, hostseitiger `WriteNew`/`WriteDone`, der vollständige `CopyFile`-Producer, der `Outcome`-Rückgabevertrag und die `VfsMeta`-Feldtypen. Der Root bestätigte den scoped Caller-Abgleich für `track_stage`, `owns_stage` und `release_stage`: keine weiteren Producer außerhalb der freigegebenen Dateien.

**Umsetzung und Entscheidungen.**

- `peer_stages.rs` enthält ein begrenztes Ledger je `PeerBackend`, konkrete serielle Creator-Tickets und die Zustände Creating, Writing, Ready und Pending. Die eigene additive Registrierung liegt unmittelbar in `backend.rs`. Akzeptiert werden die vorhandenen Clientzwecke upload, bisync, merge, transfer und der belegte VFS-Zweck copy mit dem bestehenden Nonceformat. Ein Dateiname allein registriert keine Ownership; normale, fremde und hostinterne batch-/peer-Dateien werden nicht als Clientstufe übernommen.
- `open_write_new` reserviert das Ticket vor dem tatsächlichen `WriteNew`. Erst `Ready` nach dem hostseitigen `open_write_new` eröffnet den Writer. Der neue Wrapper in `peer_writer.rs` bestätigt das Ticket erst nach erfolgreichem Flush des unveränderten echten Peer-Writers und speichert die tatsächlich geschriebenen Bytes. Kurze Sized-Writer, Abbruch durch Drop, Schreibfehler und verlorene Abschlussantworten bestätigen keine Stufe. Fehler bleiben wiederholbar sichtbar, ohne dass ein erneuter Flush Ownership erzeugt.
- Ein eigener Verbraucher prüft zuerst den Ready-Zustand, dann den aktuellen regulären Metadatensatz, Bytezahl, vorhandene Provider-ID und die Peer-/Relation-/Mount-Lease-Bindung. So gibt es keine Stat-Anfrage gegen einen noch offenen Writer auf einem Provider mit nur einer Sitzung. Der erste bestätigte Verbrauch bindet ID, Größe, Zeit und vorhandenen Gratis-Hash; weitere Verwendungen müssen diesen Snapshot erhalten. Erkannter Austausch, Link/Special, verschwundene Datei oder geänderte Freigabe sperren das Ticket. Es gibt keinen ungedrosselten zweiten Content-Download ohne Cancellation.
- `finish_stage` bewahrt Featureverhandlung und den bisherigen Legacy-Default. Bei bestätigter Metadatenänderung wird ausschließlich der erwartete Timestampwechsel übernommen; ID, Bytezahl und vorhandener Inhaltshash müssen erhalten bleiben. Eine unklare Finish-Antwort gibt die Stufe nicht für Veröffentlichung oder Cleanup frei.
- Promote, NoReplace, reversible Ersetzung und Mutationen eines bereits getrackten Pfads verwenden den vorhandenen einzelnen `call_once`-Vertrag. Der Host frame wird einmal gesendet; Deadline, Principal/Identity und aktuelle Mount-Lease bleiben erhalten. Ein bestätigter Erfolg entfernt nur das konkrete Ticket. Eine verlorene/unerwartete Antwort behält eine gesperrte Stufe im Ledger, ohne Replay oder Discard. Nur das ausdrücklich bestätigte `ReversibleReplaced { replaced: false }` stellt den vorherigen Ready-Zustand für den sicheren bestehenden Fallback wieder her. Der reine `negotiated`-Entscheidungskörper und seine ursprünglichen Assertions sind unverändert.
- K17 bleibt eng: `discard_copy_stage` benötigt zusätzlich zum bestätigten Ticket weiterhin die unveränderte Upload-Namensregel und transfer v1. Bisync-/Merge-/Copy-/Transfer-Stufen erhalten dadurch kein neues Pfad-Cleanuprecht. Normale Rename-/Remove-Aufrufe außerhalb des Ledgers behalten ihren bestehenden Requestpfad.
- `CopyFile` erstellt intern zwar eine exklusive `.se-copy`-Stufe, publiziert aber über `promote_staged_replace` zum angefragten Dst. Das beweist keine exklusive Erstellung dieses Dst und könnte einen fremden oder zwischenzeitlich erzeugten Stagepfad ersetzen. Deshalb ruft `server_copy_to_stage` hierfür keinen CopyFile-RPC mehr auf und liefert das vorhandene vertragliche `Ok(None)` für den WriteNew-Streaming-Rückfall. Es gibt weder Pre-RPC-Registrierung noch Metadata-Adoption nach CopyFile. Der normale `Backend::copy_file` und seine echte Servercopy-Fähigkeit bleiben unverändert. Die bisherigen Budget-Assertions bleiben mit ihrem reinen, nun `cfg(test)`-gebundenen Budgethelper erhalten.
- Die unveränderte `stable.errors.is_empty()`-Bedingung meldet jetzt linkes/rechtes Scheme und die tatsächlichen `Outcome`-Felder errors, blocked, stopped, deferred und state. `stopped` ist `Option<RunStop>` und wird wie die anderen Felder mit Debug ausgegeben. Kein Busy-/Namespace-/Providerfehler wird umgangen.

**Geänderte Dateien.**

- `native/src/share/core/backend.rs` — eigener privater Moduleintrag, Writeranschluss sowie getrackte Rename-/Remove-/Promote-Verbraucher.
- `native/src/share/core/peer_transfer.rs` — Creator-Ledger statt Namens-HashSet, sichere NoReplace-/Discard-Verbraucher, verbindlicher Streaming-Rückfall für private Stufen.
- `native/src/share/core/peer_extensions.rs` — geprüfter Finish-Lifecycle samt Legacy-Default.
- `native/src/share/core/peer_reversible_replace.rs` — konkretes Publication-Ticket, bestätigtes false und Wiederverwendung des einmaligen Requestvertrags.
- `native/src/share/core/peer_writer.rs` — bestätigender Wrapper; der eigentliche PeerWriter ist unverändert.
- `native/src/share/core/reversible_replace_task_tests.rs` — zusätzliche passende Assertfälle in den bestehenden Leaves, keine neuen Testentrypoints.
- `native/src/app/os/shared/sync_paths_task_tests.rs` — ausschließlich die Diagnose der vorhandenen AllBackend-stable-Assertion; realer Share-Leaf unverändert.

**Erstellte Dateien.**

- `native/src/share/core/peer_stages.rs` — unmittelbar neben dem zugeordneten Backend, eine kohäsive Ownership-Verantwortung.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-5-A-CLIENT.md` — dieser Bericht.

**Gelesene Dateien.** Die folgenden Pfade wurden nur innerhalb der im Scope gespeicherten Definitionen beziehungsweise Leaves gelesen; eigene Änderungen wurden anschließend selbst gelesen. Die Scope-Datei wurde nicht von diesem Worker verändert.

```text
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-5-a-client.json
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fifth-fixes.md
docs/refs/local-fs-identity-durability.md
docs/refs/private-file-access.md
docs/refs/post-publication-namespace.md
docs/refs/rv1-remote-suite.md
docs/refs/share-server-tls-auth.md
native/src/share/core/backend.rs
native/src/share/core/peer_transfer.rs
native/src/share/core/peer_extensions.rs
native/src/share/core/peer_reversible_replace.rs
native/src/share/core/peer_stream.rs
native/src/share/core/batch_wire.rs
native/src/share/core/server_fs.rs
native/src/share/core/fs_request.rs
native/src/share/core/fs_access.rs
native/src/share/core/fs_authority.rs
native/src/share/core/fs_capabilities.rs
native/src/share/core/reversible_replace_task_tests.rs
native/src/share/core/peer_request.rs
native/src/share/core/peer_writer.rs
native/src/share/core/server_transfer.rs
native/src/share/core/fs_copy.rs
native/src/vfs/core/promotion.rs
native/src/vfs/core/core.rs
native/src/vfs/core/extensions.rs
native/src/vfs/core/extension_types.rs
native/src/vfs/core/meta.rs
native/src/vfs/mod.rs
native/src/vfs/os/shared/copy_transfer.rs
native/src/bisync/os/shared/apply_stage.rs
native/src/bisync/os/shared/version_save.rs
native/src/bisync/mod.rs
native/src/bisync/os/shared/orchestration.rs
native/src/app/os/shared/sync_paths_task_tests.rs
native/src/app/os/shared/sync_paths_task_fixture.rs
/tmp/rv1-ci-fifth/linux/native-suite.log
/tmp/rv1-ci-fifth/windows/native-suite.log
native/src/share/core/peer_stages.rs
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-5-A-CLIENT.md
```

**Self-Review und Abnahmesignale für denselben RV1-Lauf.** Die statische Textprüfung gegen die eigenen gespeicherten Ausgangsdateien bestätigt die Erhaltung aller ursprünglichen Assertionmacros und Testsymbole in `peer_transfer.rs` und `reversible_replace_task_tests.rs`; es wurden keine Testentrypoints hinzugefügt. Der tatsächliche PeerWriter-Körper ab seiner ursprünglichen Factory bleibt textgleich, ebenso `negotiated`. Sämtliche angefassten/neuen Rust-Dateien liegen unter 500 Zeilen und 50 KiB. Kein Compiler, Formatter, Build, Test, Server, Installationsprozess, Git-, CI-, Graph- oder Releasebefehl wurde gestartet.

- `sync_paths_task_real_share_cross_peer_sync_and_local_roundtrip`: dieselben echten Peerhosts müssen die literal erhaltenen Dateien in beiden Richtungen kopieren. Bestehende Bytes-, private `.se-versions`-Denial-, Auto-Overwrite-/AppData-Oldbytes-, PairLock-Restore- und Local-Roundtrip-Assertions bleiben erhalten.
- `review_task_h_replace_only_confirmed_true_releases_own_stage`: die bestehenden Assertions für true/false, fremde Ownership und unerwartete Antworten bleiben bestehen. Zusätzliche Fälle binden den echten Wrapperanschluss an bestätigten Flush, verweigern unbestätigte/fremde/geänderte Stufen, sperren veränderte Bindungen und verhindern die Wiedervergabe eines alten seriellen Tickets. Drop ohne Flush und fehlender Writer-ACK bleiben unbestätigt.
- `review_task_h_replace_idle_close_and_lost_ack_never_replay_or_release`: ursprüngliche Einmal-/Oldbytes-/Newbytes-/NoRelease-Assertions bleiben erhalten; ein Pending-Ticket berechtigt nicht zur erneuten Mutation oder Cleanup.
- `transfer_engine_task_sized_stage_commits_only_exact_length`: bestehende exakte Längen-/Short-/Long-/Flush-/Congestion-/Budget-Assertions bleiben erhalten.
- `sync_paths_task_all_backend_pairs_transfer_changes_and_keep_baselines_separate`: alle Backendkombinationen, literal Bytes und geordnet getrennte Pair-Identitäten bleiben unverändert; ein weiterer Fehler liefert jetzt seine tatsächlichen Ergebnisfelder.

**Restgrenzen und Owner.** Die stärkere Object-ID wird nicht erfunden: `Ready` und `WriteDone` liefern im vorhandenen Drahtvertrag keinen atomaren Creator-Handle. Der Client beweist seine konkrete exklusive WriteNew-Erstellung und den bestätigten Abschluss, kann aber einen Austausch mit identischen Metadaten vor dem ersten Snapshot oder in der Stat→Mutation-Lücke ohne stärkeren Hostvertrag nicht atomar ausschließen. Für Provider mit `id: None` gelten die bestehenden Pfad-/Größen-/Zeitverträge; Hashwerte werden nur verwendet, wenn vorhanden. Dieser Block ändert keine Host-/Transportoberfläche und beansprucht keine darüber hinausgehende Sicherheit. Root bewertet eine eventuell geforderte stärkere Creator-ID-Bindung an dieser genauen Fremdgrenze.

Die Windows-AllBackend-Ursache bleibt ohne neue Laufdiagnose bei E-ENGINE; der Client enthält dafür nur die zugewiesene unveränderte Assertion mit besserer Evidenz. Die parallel bearbeitete private Windows-Handlebasis und deren Kaskaden werden nicht maskiert. Root besitzt Integration, Formatterpatch/Commit sowie die erneute vollständige RV1-Abnahme. Es gibt keine weiteren ungelesenen Definitionen, die für den hier implementierten Clientanschluss benötigt werden. Der Worker stoppt nach diesem Handoff.
