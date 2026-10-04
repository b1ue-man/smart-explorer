# CI-6 A-CLIENT: fallible Dateifähigkeiten ohne Mountwechsel

## Vor Edit belegte Ursache und Umsetzungsgrenze

Run `37172335052` für Kandidat `2035ed266fa201195d39162dfdd7967e31a526d9` erreicht in beiden Desktopläufen die erste echte Share-Kopie einschließlich der ursprünglichen Byte- und privaten `.se-versions`-Prüfungen. Erst der nachfolgende Auto-Overwrite im vorhandenen Leaf `sync_paths_task_real_share_cross_peer_sync_and_local_roundtrip` scheitert mit `Stage gehört zu einer anderen Share-Freigabe` und `Pfad liegt ausserhalb der eingebundenen Peer-Wurzel /A/first Ü.txt`. Der kandidatengebundene Remote-Formatpatch ist laut Root als `e27bac22` übernommen. Dies sind Eingangsbelege, keine Abnahme der folgenden Änderung.

Die Quelle ist vor Produktionsänderungen frisch belegt: `apply_transaction.rs` fragt nach Stage-Erstellung und Backup `mount_path_capabilities(destination_path)` allein für `staged_write.namespace_replace` ab. `replacement_publish.rs` wiederholt dieselbe Abfrage für `destination`; `merge_execution.rs` hat denselben Publish-Vertrag für `paths[index]`. Beim `PeerBackend` bedeutet diese API `query_mount_path_capabilities(..., true, ...)`: Die aktuelle Lease wird freigegeben und durch eine neue, auf den angefragten Pfad gebundene Lease ersetzt. `fs_capabilities.rs` bindet die virtuelle Wurzel an genau diesen Pfad. Die vor der Abfrage bestätigte eigene Stage bleibt hingegen an ihren ursprünglichen Peer-/Lease-Token gebunden; der strikte Creator-Abgleich verwirft den neuen Token. Die auf `/A/first Ü.txt` verengte Lease kann außerdem den Geschwisterpfad für Versionen nicht auflösen. Der Unixmode-Dispatcher ist kein Producer dieses Fehlers: Das bestehende Peer-Extensions-Default liefert dort `Ok(None)`.

Die bestehende fallible Peer-Probe `probe_mount_path_capabilities` verwendet `acquire_lease = false`. `PeerMountLeaseClient::accept_capabilities` verlangt dabei eine Antwort ohne neue Lease und verändert die bestehende Bindung nicht. Der Anschluss verwendet diesen vorhandenen Producer, ohne den Creator-/Root-/Principal-/Leasevergleich zu lockern.

## Bounded Meilensteine und Abnahmesignale

1. Additives fallibles `Backend::probe_staged_write_capabilities` mit unverändert falliblem Default für andere Provider; Peer delegiert die bestehende nicht akquirierende Probe. Erwartung: Ein Dateifähigkeitsfehler bleibt ein typisierter Fehler, und eine bereits bestätigte Stage behält ihre Mountbindung.
2. Transparente Weiterleitung in `CachingBackend` und die vorhandene Read-/Write-/Authority-/Name-Grenze in `GuardedBackend`. Erwartung: Wrapper lösen keine neue Lease aus; Readfehler propagieren und nicht schreibbare Ziele melden konservative Schreibfähigkeiten.
3. Ausschließlich die drei belegten `namespace_replace`-Abfragen auf die neue Probe umstellen. Erwartung: Der Auto-Overwrite und der gleiche Merge-Publish-Vertrag ändern ihre eigene bestehende Mount-Lease nicht; Backup, Bind/Intent, NoReplace, Reversibilität, unsicherer Ausgang und Retry bleiben gleich.
4. Die erforderliche kleine kohäsive Default-/Cachelimit-Auslagerung innerhalb zuvor angeforderter und freigegebener Grenzen durchführen. Erwartung: Keine fachliche Dokumentation oder Assertion wird zur Einhaltung des Remote-Formatguards entfernt; die betroffenen Dateien bleiben nach der Remote-Formatierung unter 500 Zeilen.

Die Abnahme erfolgt ausschließlich über denselben vollständigen Root-eigenen RV1-Eintritt. Das vorhandene reale Share-/Versions-/Restore-Leaf behält seine ursprünglichen Assertions einschließlich privater Versionsgrenze, exakter alter und neuer Bytes, AppData-Backup, PairLock-Restore, erneuter Synchronisation und lokalem Roundtrip. Es gibt keine lokale Ausführung und keinen neuen Testeintritt.

## Umsetzung und Entscheidungen

Der zugeordnete Anschluss ist implementiert. `Backend::probe_staged_write_capabilities(&self, root: &str) -> VfsResult<StagedWriteCapabilities>` ist additiv. Sein Default projiziert die bisherige fallible `mount_path_capabilities`-Antwort; andere Provider behalten damit ihre vorhandene Fehlerweitergabe und Defaultfähigkeiten. Der Peer-Override projiziert die bestehende nicht akquirierende `probe_mount_path_capabilities`-Antwort. Er verschluckt keine Fehler. Die bisherige infallible `staged_write_capabilities`-API und die tatsächliche Mount-Akquisition durch `mount_path_capabilities` bleiben unverändert.

`CachingBackend` leitet direkt an den inneren Backend-Probe weiter. `GuardedBackend` übernimmt exakt die Reihenfolge der bisherigen falliblen Mount-Abfrage: `read(root)?` einschließlich Authority und Name, fallible innere Abfrage, anschließend konservative leere Schreibfähigkeiten bei fehlendem `write(root)`. Ein Read- oder innerer Probe-Fehler wird weitergegeben; die Write-Maske gewährt keine Rechte. Ausschließlich die drei belegten Publish-Abfragen in Apply, Replacement und Merge verwenden die neue API und deren `namespace_replace`.

Die zwei bestehenden Trait-Defaultkörper sind als `pub(super)` generische Helfer in das vorhandene Capabilitymodul verlegt. `B: Backend + ?Sized` erhält den Aufruf für Traitobjekte. Die beiden `rename_overwrites`-Aufrufe sowie die Reihenfolge `staged_write_capabilities` vor `root_confinement` bleiben erhalten. `cache_limits.rs` enthält ausschließlich den bisherigen `CacheLimits`-Typ, seine Konstanten und den vorhandenen Kommentar. Die Sichtbarkeit reicht zum privaten Parent und seinen Kindern; Werte, Typen, Derive und Auswahl von Browsing-/Mountlimits sind gleich. Die eigene Modulregistrierung und der eigene Import sind additiv.

Es entstehen keine Wirefelder, neuen Grants, Lease-Erneuerungen, Root-Ersetzungen, Namensadoptionen oder neuen Testeintritte. Der Host, die Transport-/Lease-Produzenten und der bestehende Creator-Ledger sind unverändert. Unklare Publikationen bleiben in Quarantäne, die vorhandene Call-once-/NoReplay-Grenze und die an den konkreten Creator gebundenen Tickets bleiben bestehen. Backup, Cancellation, NoReplace, dokumentiertes `Ok(false)` bei unveränderter reversibler Publikation, Fehler-/Rollback- und Retrypfade der drei Caller bleiben gleich.

## Eigener statischer Self-Review

- Die Ursache wurde in diesem Bericht vor Source-Edit festgehalten. Die benötigten Interface- und Extraktionsflächen wurden vor ihrer Lektüre beziehungsweise Änderung von Root im eigenen Scope gespeichert.
- Der Textvergleich der drei freigegebenen Caller-Körper gegen den vor Edit gelesenen Zustand ergibt ausschließlich den Austausch der Fähigkeitsabfrage und das Entfernen der jetzt unnötigen `.staged_write`-Projektion. Der gesamte ursprüngliche reale Share-/Auto-Versionen-/PairLock-Restore-/Resync-/Local-Roundtrip-Leaf ist bytegenau unverändert.
- Der Capabilitymodell-/Dokumentationsteil ist bytegenau unverändert. Die ausgelagerten Defaultkörper stimmen abgesehen von Receiverbezeichnung und Einrückung mit den alten Körpern überein. Der Cachetyp-/Konstantenblock stimmt abgesehen von der erforderlichen Parent-Sichtbarkeit bytegenau überein.
- Peer-Probe, beide Weiterleitungen und die drei Consumer haben dieselbe fallible Signatur. Der Peer verwendet den vorhandenen `acquire_lease = false`-Producer; kein Probe-Override fällt auf eine neue Mount-Akquisition zurück. Der Guard behält seine bestehenden Read-/Write-/Authority-/Name-Prüfungen.
- Nur statischer Text-/Parsing-Abgleich und Größenmessung wurden verwendet. Es wurde kein Formatter, Compiler, Build, Test, Server, Git-, Graph-, CI- oder Releaseprozess ausgeführt und kein weiterer Agent eingesetzt.

Statischer Sourcezustand, ohne lokale Formatierung:

| Rustdatei | Zeilen | Bytes |
| --- | ---: | ---: |
| `native/src/vfs/core/core.rs` | 496 | 19879 |
| `native/src/vfs/core/capabilities.rs` | 75 | 2419 |
| `native/src/vfs/core/cache.rs` | 483 | 16225 |
| `native/src/vfs/core/cache_limits.rs` | 20 | 533 |
| `native/src/share/core/backend.rs` | 462 | 14409 |
| `native/src/share/core/fs_guard_backend.rs` | 469 | 15835 |
| `native/src/bisync/os/shared/apply_transaction.rs` | 309 | 11035 |
| `native/src/bisync/os/shared/replacement_publish.rs` | 131 | 5055 |
| `native/src/bisync/os/shared/merge_execution.rs` | 309 | 10142 |

Der formatierte Zustand wird weiterhin vom gemeinsamen Remote-Formatguard beurteilt; die obigen Werte sind keine Formatter- oder Compilerabnahme.

## Gelesene Dateien

Exakte Quellen für diesen CI-6-Auftrag; bei begrenzter Freigabe wurden die benannten Verträge beziehungsweise Symbolbereiche gelesen:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-6-a-client.json` — eigener Scope einschließlich der vorab gespeicherten Grants.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-sixth-fixes.md` — eigener Auftrag und Eingangsbelege.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-5-A-CLIENT.md` — vorherige Creator-/Stage-Grenze.
- `docs/refs/local-fs-identity-durability.md`.
- `docs/refs/private-file-access.md`.
- `docs/refs/post-publication-namespace.md`.
- `docs/refs/rv1-remote-suite.md`.
- `docs/refs/share-server-tls-auth.md`.
- `/tmp/rv1-ci-sixth/linux/native-suite.log` — konkretes reales Share-Leaf und dessen Fehler.
- `/tmp/rv1-ci-sixth/windows/native-suite.log` — derselbe konkrete Fehler.
- `native/src/share/core/backend.rs` — Peer-Backend und vorhandene Fähigkeits-/Ownershipverträge.
- `native/src/share/core/peer_stages.rs` — bestehender strikter Creator-/Lease-/Ticketvergleich.
- `native/src/share/core/peer_transfer.rs` — zugeordnete Fähigkeits-/Stage-Consumer.
- `native/src/share/core/peer_extensions.rs` — vorhandene Erweiterungen und Defaults.
- `native/src/share/core/peer_reversible_replace.rs` — bestehende reversible Publikationsgrenze.
- `native/src/share/core/peer_stream.rs` — vorhandener Stream-/Leaseanschluss.
- `native/src/share/core/peer_writer.rs` — vorhandene Ready-/WriteDone-/Ownership-ACKs.
- `native/src/share/core/peer_request.rs` — ausschließlich `PeerBackend::open_writer`.
- `native/src/share/core/server_transfer.rs` — tatsächlicher WriteDone-/WriteJob-/Writerbereich; außerdem die unten ausdrücklich genannte versehentliche Reader-Lesung.
- `native/src/share/core/fs_capabilities.rs` — tatsächliche ausgehandelte Fähigkeiten und virtuelle Rootbindung.
- `native/src/share/core/backend_capabilities.rs` — freigegebene Capability-/Legacy-/Probe-/Querymethoden bis Zeile 111.
- `native/src/share/core/mount_lease_client.rs` — Token-/Accept-/Current-/Clear-Lifecycle.
- `native/src/share/core/mount_lease.rs` — freigegebener Lease-/Bindungs-/Resolvevertrag bis Zeile 149.
- `native/src/share/core/reversible_replace_task_tests.rs` — zugeordnete Imports und bestehende Fixture-/Backendhelfer.
- `native/src/share/core/fs_guard_backend.rs` — freigegebene Read-/Write-/Authority-/Name-Grenze und Capabilityweiterleitungen.
- `native/src/vfs/core/core.rs` — relevante vorhandene Backendverträge, Capabilitydefaultkörper und additive Probe.
- `native/src/vfs/core/extensions.rs` — relevante Defaults, insbesondere `unix_mode`.
- `native/src/vfs/core/extension_types.rs` — gezielte Suche nach den Capabilitytypen; dort keine passende Definition.
- `native/src/vfs/core/extension_calls.rs` — ausschließlich der freigegebene `unix_mode`-Dispatcher.
- `native/src/vfs/core/capabilities.rs` — bestehendes enges Capabilitymodell und ausgelagerte Defaults.
- `native/src/vfs/core/cache.rs` — freigegebene Imports, Typ-/Konstantenblock, Capabilityweiterleitungen und konkrete `CacheLimits`-Verwendungen.
- `native/src/bisync/os/shared/apply_stage.rs` — zugeordnete Create-/Finishaufrufe und relevante Anschlussverträge.
- `native/src/bisync/os/shared/version_save.rs` — zugeordnete exklusive Backup-/Finishaufrufe.
- `native/src/bisync/os/shared/apply_transaction.rs` — freigegebener bestehender Publish-/Backup-/Bind-Vertrag.
- `native/src/bisync/os/shared/replacement_publish.rs` — freigegebener bestehender Publish-/Reversible-/Durability-Vertrag.
- `native/src/bisync/os/shared/merge_execution.rs` — freigegebener bestehender Publish-/Backup-/Recovery-Vertrag.
- `native/src/app/os/shared/sync_paths_task_tests.rs` — ausschließlich `synchronize` und das vorhandene reale Share-/Versions-/Restore-Leaf.
- `native/src/app/os/shared/sync_paths_task_fixture.rs` — bestehende zugeordnete Fixtures.
- `native/src/vfs/core/cache_limits.rs` — eigene neue Datei.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-6-A-CLIENT.md` — eigener Bericht.

## Geänderte und erstellte Dateien

Geändert:

- `native/src/vfs/core/core.rs` — additive fallible Probe und die genau zwei Defaultdelegationen.
- `native/src/vfs/core/capabilities.rs` — genau zwei unveränderte generische Defaulthelfer.
- `native/src/share/core/backend.rs` — Peer-Override der Probe.
- `native/src/vfs/core/cache.rs` — direkte Probeweiterleitung, eigener privater Moduleintrag/Import, Auslagerung des bisherigen Limitblocks.
- `native/src/share/core/fs_guard_backend.rs` — Probeweiterleitung mit bisherigem Read-/Write-/Authority-/Name-Vertrag.
- `native/src/bisync/os/shared/apply_transaction.rs` — genau die belegte Publish-Fähigkeitsabfrage.
- `native/src/bisync/os/shared/replacement_publish.rs` — genau die belegte Publish-Fähigkeitsabfrage.
- `native/src/bisync/os/shared/merge_execution.rs` — genau die entsprechende Publish-Fähigkeitsabfrage.

Erstellt:

- `native/src/vfs/core/cache_limits.rs` — kohäsive Auslagerung direkt neben dem zugeordneten Cache.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-6-A-CLIENT.md` — Ursache vor Edit, Umsetzung, Inventar, Self-Review und Grenzen.

Es wurden keine Testdateien, globalen Plan-/Scope-/Suite-/Graph-/Releaseflächen oder weiteren Dateien geändert.

## Lesungsabweichung und offene Grenzen

Eine frühere Lektüre von `server_transfer.rs` nach alten Zeilenpositionen enthielt nach dem Remote-Formatpatch in Zeilen 66–140 Reader-Code außerhalb der zugeordneten Writegrenze. Diese Lesungsabweichung ist Root gemeldet. Für diesen Abschnitt wird keine vollständige Scopeeinhaltung behauptet. Der Reader ist unverändert, wurde weder als Ursache verwendet noch weiter untersucht; danach wurden die tatsächlichen Writerbereiche verwendet. Es gibt keinen Reader-Änderungs- oder Hypothesenauftrag.

Es ist keine weitere konkrete Source-/API-Scope-Lücke für den beschriebenen Anschluss offen. Die technische Abnahme bleibt offen bis zur Auswertung desselben vollständigen Root-eigenen Remote-RV1-Eintritts. Root besitzt Integration, Remote-Formatierung, Compiler-/Geräteprüfung und alle unveränderten eigentlichen Assertions. Dieser Bericht behauptet keine erfolgreiche Ausführung nach der Änderung. Der bounded Workerauftrag endet mit diesem Handoff.
