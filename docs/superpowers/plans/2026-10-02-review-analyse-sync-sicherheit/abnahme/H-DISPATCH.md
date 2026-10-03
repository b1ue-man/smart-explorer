# H-DISPATCH – Host-Durchsetzung

Stand: 2026-10-03. Der freigegebene Source-Anschluss für FA3/FC1/FC6 ist abgeschlossen; gemeinsame Remote-Abnahme und die ausdrücklich benannten Hauptanschlüsse stehen aus. S-REVOKE bleibt der abgeschlossene Commit `0d36bdd`. Dies ist die Umsetzung vorhandener Befunde und Verträge, kein neuer Projekt-Review.

## Umsetzung und Fundzuordnung

| Schritt/Funde | Implementierte Grenze | Erwartetes Abnahmesignal |
|---|---|---|
| H1 – FC1, S36/S63/S65 (Host-Anteil) | `SessionAuthorization.may_write` und `ExportAccess::ReadWrite` werden vor jeder Mutation und nochmals bei Ziel/Batch/Commit geprüft. Capability- und Lease-Schreibfähigkeiten werden entsprechend eingeschränkt. | Nur-lesen-Schreiben ergibt einen Rechtefehler; keine Provider-Mutation, kein irreführender Pfadformatfehler. |
| H2 – FA3/A30, V5, S40 | Principal-Generationen, Cancelmarker, Verbindungen, Mount-Leases und Exec werden durch `RestrictionSet` nach Schlüssel und Beziehung entzogen. Konfigurationsübergang und Barrieren liegen vor Snapshotübernahme. | Ein eingeschränkter Direct/Room verliert seine Arbeit; unabhängiger Peer und andere Beziehung laufen weiter. Erweiterungen und Laufzeit-Präsenz schließen keine fremde Arbeit. |
| H3 – FC3, S02/S03/S27/S43 (Relay-Anteil) | Endpoint-Builder verwendet `ca_tls_config()`; Peer-Relays passieren `accepts_relay_url()`. | Gepinnter HTTPS-Relay funktioniert; falscher Pin/TLS-Fehler und Klartext ohne Opt-in führen nicht zu einem Downgrade. |
| H4 – FC1, S36/S63/S65 | Eigene App-/Cache-Daten, `.se-versions` und `.held.se-recycle-<16hex>` sind private Pfade. System-/Start-/Schlüsselorte brauchen Export-Opt-in. Normale Transfer-Stages bleiben nutzbar. | Private Namen verschwinden aus Listen/Reports und sind explizit unzugänglich; normale autorisierte Stages lassen sich schreiben und veröffentlichen. Die unten benannten LocalBackend-Restgrenzen bleiben sichtbar. |
| H5 – FC6, S07/S37/S41, A21/S42 (Anschluss) | Bekannte/Unbekannte haben getrennte Handshake-Zulassung und Fristen; faire begrenzte Gerätewarteschlangen für Steueroperationen und Verbindungen, getrennte lange Arbeit. `PeerDeviceKey` enthält Schlüssel und Knoten, keine Relation-/device_id-Aliase. | Ein einzelnes Gerät nutzt freie Kapazität; ein wartendes zweites Gerät erhält den nächsten freigegebenen Platz. Direct/Room desselben Geräts teilen die Quote, interaktive Anfragen bleiben neben langen Läufen bedienbar. |
| H6 – FC6/S64, Legacy A21/A28 | Rekursive Löschung ist iterativ, budgetiert und ohne Link-Verfolgung. Local/UNC verwendet gehaltene Eltern-/DELETE-Handles. Legacy-Snapshot ruft den realen vierargumentigen H-ANALYSIS-Einstieg auf. | Kein rekursiver Backend-Delete, kein Stacküberlauf, außerhalb liegendes Linkziel bleibt erhalten; Legacy-Snapshot nutzt fairen Host-Worker mit authentifiziertem Principal. |
| H7 – A20/A31/A32, FC6/S40/S52 (Host-Anteil) | Guarded Reader/Writer/Providerströme prüfen lebende Rechte, Cancelmarker und Backpressure. Wake-/Admission-Guards bleiben bis Workerende erhalten. Retention entfernt nur den Transportmarker. | Abbruch/Widerruf beendet Arbeit; bloßer Verbindungsabbruch lässt behaltene FA2-Ergebnisse wieder anhängen, sofern aktuelle Rechte passen. Keine Wake-Anforderung durch ungeprüfte Host-Nachrichten. |
| H8 – S66/S16, V5 | Kurzes Reciprocal-Persist-Gate bleibt erhalten; kein Transition-Permit über Peer-I/O. Replayeinträge und Signaturflags werden monoton übernommen; unbekannte Sitzungs-Nonceformen werden nicht als abgelaufen weggeprunt. | Verzögerter Peer-I/O hält Profiländerungen nicht auf; ein widerrufener Reparaturstand wird nicht persistiert. Wiederübernahme alter Profile löscht keine gelernten Sicherheitsmarker. |
| H9 – Y124/Y140/Y142 (konkreter Provideranschluss) | Verhandeltes read-only `SyncChildPath`/`ChildPath`, Feature `literal_children_v1`, guarded Literal-/Reversible-/Identity-Hooks. Privatnamen in Drive-Providerpfaden werden einmal komponentenweise dekodiert, ohne I/O-Pfade umzuschreiben. | Kodierter gespeicherter Parent bleibt gleich; `aux.c`, `%61ux.c` und `100%.pdf` bleiben verschiedene Namen. Kein Legacy-Rückfall nach ausgehandeltem Fehler; keine private Namensumgehung durch `%2E...`. |

FC1-Konfiguration/Bedienung bleibt S-POLICY; Signaling, lokale IPC-Zulassung S60 und Uplink-Härtung bleiben ihren anderen Blöcken zugeordnet. H-DISPATCH beansprucht dort keine eigene Umsetzung.

## Entscheidungen und erhaltenes Verhalten

Local/UNC wird konkret mit `LocalBackend::new(authorisierter_physischer_root)` verbunden; `root_display` bleibt dieser Root. Es gibt keinen `target.parent()`-Fallback. Gespeicherte Remote-Roots, Backend-/Verbindungsidentität, Provider-Funktionen und normale Transfer-Stages bleiben erhalten. Der Guard bietet Provider-Fähigkeiten und delegiert sichere VFS-Erweiterungen; er gibt den rohen Backendzugriff nicht frei.

Dauerhafte Authentisierung und Transport-Lifetime sind getrennt: behaltene Ergebnisse halten vollständigen Principal und Autorität; vor Reattach und jeder Ausgabe werden aktuelle Rechte geprüft. Alte Generationen werden durch Wiedererlauben nicht wieder gültig. Der Geräte-Schlüssel teilt nur Zulassung/Quote; Retention und Rechteentzug behalten die vollständige Beziehung.

Die freie Steuerkapazität ist ausleihbar, mit begrenzten globalen und Gerätewarteschlangen. Lange Löschung/Walk/Flush läuft in einem getrennten Pool. Bestehende Transfer-Sicherheitsgrenzen bleiben erhalten. Verbindungsdruck lässt angenommene Streams und Antwortbestätigungen abschließen; alte physisch gebundene Leases werden nicht als verdrängbare v2-Leases behandelt. Fehlende Antwortbestätigung schließt mit einem Fehlercode, damit eine schon ausgeführte Mutation nicht als ungestarteter Idle-Retry erscheint.

Linux löscht finale Kindkomponenten über `unlinkat` am gehaltenen Eltern-FD. Windows konsumiert den ursprünglichen DELETE-Pin für den Disposition-Aufruf; es gibt keinen Drop-/Path-Reopen-Rückfall. Private Verzeichnis-Aliase werden physisch geprüft: Linux zusätzlich mit Mount-Root-Zuordnung für Bind-Mounts; Windows über Volume und vollständige FileId. Normales Lesen, Broker-/Consent-Verhalten und gewöhnliche Daten-Reparsepunkte werden dadurch nicht pauschal abgeschaltet.

Die neue Wire-Operation ist lesend. Neue Literalnamen laufen durch den Provider-Hook, gespeicherte Parent-Schreibweisen werden nicht erneut kodiert. `split_clean` akzeptiert weiterhin keine rohen Backslashes als Pfadsyntax. Nur ein Host ohne ausgehandeltes `literal_children_v1` benutzt die alte Literal-Join-Semantik. Der Reversible-Hook delegiert den bestehenden Vertrag mit vorab journaled Recovery-Sibling `.se-replace-<16lowerhex>`; `false` ändert nichts und `true` erhält das Original. Daraus wird keine atomare Overwrite-Garantie abgeleitet.

Der tatsächliche H-ANALYSIS-OS-Fakt `remote_trash_v1` bleibt zentral in `FsHostFeatures::host()`. B21-Dateimigration bleibt der bereits integrierte `ipc_host::load_share_server`-Anschluss; H-DISPATCH dupliziert sie nicht. S09-LINK-Registrierungen bleiben dem späteren Hauptanschluss vorbehalten.

## Konkrete Signale für die eine Remote-Suite

Diese Source-Fixtures sind hinter `#[cfg(test)]` angebunden. Sie wurden lokal weder ausgeführt noch kompiliert. Der Hauptagent bündelt sie und die folgenden Integrationsfälle in der bereits vorgesehenen einen Remote-Suite.

| Grenze | Vorhandene Selektoren |
|---|---|
| Eingrenzung/Generation/Cancel | `review_task_host_restrictions_hit_key_and_relation_only`; `review_task_host_room_restrictions_preserve_other_room_and_extensions`; `review_task_host_cancel_registration_after_restriction_fails_closed` |
| Private Namen/Nur-lesen/Systemorte | `review_task_host_private_names_hide_quarantine_and_versions_but_allow_transfer_stages`; `review_task_host_read_only_is_a_rights_error_for_normal_write_paths`; `review_task_host_system_write_classification_covers_startup_shell_and_keys`; `review_task_host_system_opt_in_never_opens_app_private_or_versions`; `review_task_host_private_provider_paths_decode_drive_components_once` |
| Provider/Funktionen/Reports | `review_task_host_read_only_backend_denies_write_commit_and_recycle_before_provider`; `review_task_host_guard_preserves_literal_provider_hook_and_safe_identity_aliases`; `review_task_host_fast_report_removes_private_sizes_from_every_ancestor`; `review_task_host_fast_report_preserves_provider_stage_and_rejects_path_injection` |
| Löschung/Lifetime | `review_task_host_delete_is_iterative_and_does_not_descend_into_child_links`; `review_task_host_delete_rechecks_write_authority_before_each_mutation`; `review_task_host_local_delete_uses_parent_handles_and_preserves_export_root` |
| OS-Delete | Linux: `review_task_host_delete_root_swap_never_uses_old_path_spelling`, `review_task_host_delete_child_symlink_is_a_nonrecursive_leaf`; Windows: `review_task_host_windows_delete_consumes_original_delete_pin` |
| Gerätefairness | `review_task_host_fairness_borrows_capacity_and_prioritizes_waiting_device`; `review_task_host_fairness_shares_direct_room_device_but_keeps_principal`; `review_task_host_fairness_cancelled_queue_is_reusable_and_bounded` |
| Literal-Wire | `review_task_literal_children_preserve_encoded_parent_and_distinct_names`; `review_task_literal_children_wire_is_read_only_and_legacy_flag_defaults_false` |

| Remote-Integrationsfall | Erfolgs-/Ablehnungssignal |
|---|---|
| Direct und Room: alle `mutates_filesystem()`-Varianten, Batch-Elemente, Stage-Commit/-Mtime/-Finish, Promote/Copy/Rename/Delete/Recycle | Kombinationen Beziehung nur lesend + Export RW sowie Beziehung RW + Export RO verändern keine Providerdaten; Fähigkeiten melden den effektiven Nur-lesen-Stand. Beide RW erlauben normale Ziele und Stages. |
| App-Daten/private Namen an beliebiger Tiefe, benannter Parent eines privaten Unterbaums, UNC-/Bind-Mount-Alias | Listen/Stat/Read/Write/Analyse/Hash/Duplicate/Watch geben keine privaten Daten aus; Delete/Rename/Recycle des geschützten Parents scheitert vor Mutation. Typed Handle-Hook wird beim schnellen lokalen Öffnen verwendet. |
| Dritter Kontakt online/offline, neues Room-Mitglied, Export-/Rechtserweiterung bei laufender Analyse/Übertragung/Exec | Verbindungs-ID, Worker und Lease des unabhängigen Principals bleiben gültig. Keine unnötige Epochenerhöhung. |
| Direct-/Room-Einschränkung und unbekannte Zuordnung | Passende Sitzungen/Leases/Exec/Transfers/Cancelmarker enden; unabhängige Beziehung desselben Schlüssels bleibt erhalten. Unzuordenbare Einschränkung wirkt konservativ global. |
| Retention und Cancel | Transportabbruch allein erlaubt Reattach mit demselben behaltenen Ergebnis; aktueller Entzug verhindert Reattach und jede neue Ausgabe; alte Generation bleibt auch nach Wiedererlauben ungültig. Benutzerabbruch erreicht Provider/Host-Worker ohne nächsten Hash-Fortschritt abzuwarten. |
| Hello-Flut/Verbindungsdruck, Direct-/Room-Aliase, lange Löschung neben Browse/Stat | Unbekannte Peers belegen keine bekannten Application-Hello-Plätze; kurze unbekannte Frist wird eingehalten. Wartendes zweites Gerät erhält nächste Kapazität; angenommene Mutation bekommt Antwortbestätigung oder expliziten Fehler, keinen irreführenden Idle-Retry. |
| Tiefer Delete-Baum, Link/Junction/Reparse-Child, Root-Swap, knappe Speichergrenze | Kein Auswärtslauf/Stacküberlauf; Exportroot bleibt erhalten. Budget-/Rechtefehler bleibt retrybar und zählt keinen fehlgeschlagenen Apply als Erfolg. |
| TLS-Relay über selbst signierten Server-Pin, falschen Pin und Peer-HTTP-URL | Nur passender Pin bzw. ausdrücklich erlaubter Klartext wird genutzt; kein stiller TLS-Rückfall. Schema-lose Alt-Konfig bleibt durch die vorhandene B21-Migration dauerhaft explizites TCP. |
| Legacy-Snapshot und reale V2-Operationen, Local/UNC/SFTP/FTP/WebDAV/Drive/Share | Vorhandene Host-Entry-Points bedienen die Requests mit aktuellem Principal; kein Stub/Provider-Unsupported wegen des neuen Guards. Provider-Fähigkeiten und schreibbare normale Stages bleiben nutzbar. |
| Host-Wake-Lifetime, ungültiges Hello/Operation-Frame, Android Hintergrund | Ungeprüfte Nachricht erhält keinen Service-Hold. Autorisierter Analyse-/Transferworker hält den bestehenden Wake-Hook bis seinem terminalen Ende; Cancel und alle Rückgabepfade geben ihn frei. |
| ConfigureProfiles/Replay/Signatur/S66 | Bereits gelernte signierte Präsenz bleibt verlangt; sofortige Marker und unbekannte Sitzungsnonces bleiben erhalten. Verzögerter Repair-Wire-I/O blockiert keinen Konfigurationsübergang und persistiert nach Entzug keine Rechte. |
| Reales Drive über Direct und Room, alter Host ohne Literal-Feature | Stored-Parent bleibt unverändert, die drei Literalnamen erhalten getrennte Objekte; nur alter Host nutzt die frühere Semantik. Negotiation-Fehler und malformed ChildPath fallen geschlossen aus. |

Die Remote-Suite muss die benannten Grenzen in [anfragen/H-DISPATCH.md](../anfragen/H-DISPATCH.md) offen ausweisen. Insbesondere wird die allgemeine pfadbasierte LocalBackend-Mutationsgrenze nicht als vollständig handlegehärtet behauptet.

## Statische Evidenz

Scope-Eigentum, existierende Modulpfade, Zeilen-/Bytegrenzen, Rust-Delimiter einschließlich Strings/Kommentare und gezielte API-/Wire-Matches wurden ausschließlich mit Text-/Parsing-Arbeit abgeglichen. `git diff --check` wird für die eigenen bestehenden Dateien und Dokumente benutzt. Alle neuen und wesentlich bearbeiteten Source-Dateien liegen unter 500 Zeilen/50 KiB. Der bereits größere gemeinsame `share/mod.rs` erhielt ausschließlich eigene additive Modulregistrierungen; fremde Einträge sind nicht Teil dieses Blocks.

Diese Evidenz ist kein Compiler- oder Laufzeitnachweis. Lokale Builds, Formatter, Tests, Server, Installationen, Graph-Neubau und Git-Mutationen wurden nicht ausgeführt. Remote-Suite, Root-Graph-Aktualisierung, Commit/Push und Release bleiben beim Hauptagenten.

## Dateien gelesen

Sämtliche unten aufgeführten eigenen Source- und Berichtdateien wurden bei Umsetzung/Self-Review gelesen. Zusätzlich wurden diese freigegebenen Abhängigkeiten gelesen, teils nur ihre relevante Definition oder eng benannte Funktion; der Graph wurde nur gezielt abgefragt:

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/SHARE_SERVER.md`
- `docs/refs/share-server-tls-auth.md`
- `docs/refs/local-fs-identity-durability.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/h-dispatch.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/integration.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-analyse.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sicherheit.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S-REVOKE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/S-REVOKE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-REVOKE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-ANALYSIS.md`
- `graphify-out/graph.json`
- `graphify-out/.vocab.txt`
- `native/src/agent_proto/core/types.rs`
- `native/src/analytics/core/analysis_report.rs`
- `native/src/analytics/os/shared/analytics_outcome.rs`
- `native/src/analytics/os/shared/reclaim/finder.rs`
- `native/src/analytics/os/shared/reclaim/types.rs`
- `native/src/daemon/os/shared/rooted_backend.rs`
- `native/src/daemon/os/shared/rooted_backend_case.rs`
- `native/src/daemon/os/shared/rooted_backend_gate.rs`
- `native/src/daemon/os/shared/rooted_backend_io.rs`
- `native/src/daemon/os/shared/rooted_backend_paths.rs`
- `native/src/keep_awake/mod.rs`
- `native/src/local_access/mod.rs`
- `native/src/local_access/os/linux/create.rs`
- `native/src/local_access/os/linux/quarantine.rs`
- `native/src/local_access/os/windows/create.rs`
- `native/src/local_access/os/windows/directory.rs`
- `native/src/local_access/os/windows/directory_identity.rs`
- `native/src/share/core/analysis_admission.rs`
- `native/src/share/core/direct_reciprocal_outgoing_gate.rs`
- `native/src/share/core/direct_reciprocal_transport.rs`
- `native/src/share/core/endpoint_routes.rs`
- `native/src/share/core/exec_grant_runtime.rs`
- `native/src/share/core/export_config.rs`
- `native/src/share/core/fs_copy.rs`
- `native/src/share/core/fs_paths.rs`
- `native/src/share/core/host_requests.rs`
- `native/src/share/core/io_deadline.rs`
- `native/src/share/core/mount_lease_cleanup.rs`
- `native/src/share/core/mount_lease_client.rs`
- `native/src/share/core/node_wake.rs`
- `native/src/share/core/peer_stream.rs`
- `native/src/share/core/power.rs`
- `native/src/share/core/relation_rights.rs`
- `native/src/share/core/relation_rights_diff.rs`
- `native/src/share/core/server_batch_put.rs`
- `native/src/share/core/signal_auth_replay.rs`
- `native/src/share/core/signal_connection_tls.rs`
- `native/src/share/core/storage_analysis_server.rs`
- `native/src/share/core/storage_snapshot.rs`
- `native/src/share/core/types.rs`
- `native/src/share/os/shared/analysis_tasks.rs`
- `native/src/share/os/shared/host_hash_walk.rs`
- `native/src/share/os/shared/host_list.rs`
- `native/src/share/os/shared/host_mutations.rs`
- `native/src/share/os/shared/host_stream.rs`
- `native/src/share/os/shared/host_watch.rs`
- `native/src/share/os/shared/storage_analysis_host.rs`
- `native/src/share/os/shared/storage_duplicate_host.rs`
- `native/src/share/os/shared/storage_roots.rs`
- `native/src/support_dirs.rs`
- `native/src/transfer/os/shared/memory.rs`
- `native/src/vfs/mod.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/meta.rs`
- `native/src/vfs/core/scheme.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/extension_types.rs`
- `native/src/vfs/core/capabilities.rs`
- `native/src/vfs/core/batch.rs`
- `native/src/vfs/core/staging_names.rs`

Zusätzlich die Skillquellen `/root/.codex/skills/arbeitsweise/SKILL.md` und `/root/.codex/skills/graphify/SKILL.md` gemäß der erteilten Skill-Lesefreigabe.

## Bestehende Dateien geändert

- `native/src/share/core/authorization_policy.rs`
- `native/src/share/core/blocking.rs`
- `native/src/share/core/configuration_runtime.rs`
- `native/src/share/core/exec_server.rs`
- `native/src/share/core/fs.rs`
- `native/src/share/core/fs_access.rs`
- `native/src/share/core/fs_request.rs`
- `native/src/share/core/fs_response.rs`
- `native/src/share/core/handshake_limits.rs`
- `native/src/share/core/mount_lease.rs`
- `native/src/share/core/node.rs`
- `native/src/share/core/node_accept.rs`
- `native/src/share/core/node_idle.rs`
- `native/src/share/core/node_sessions.rs`
- `native/src/share/core/peer_extensions.rs`
- `native/src/share/core/peer_fs_logging.rs`
- `native/src/share/core/peer_request.rs`
- `native/src/share/core/server.rs`
- `native/src/share/core/server_admission.rs`
- `native/src/share/core/server_batch_get.rs`
- `native/src/share/core/server_capabilities.rs`
- `native/src/share/core/server_fs.rs`
- `native/src/share/core/server_transfer.rs`
- `native/src/share/core/session.rs`
- `native/src/share/core/walk.rs`
- `native/src/share/core/wire_capabilities.rs`
- `native/src/local_access/os/linux/directory_handle.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/share/mod.rs`

Eigentumsausnahme: In `native/src/share/mod.rs` nur additive Registrierung von `fair_admission`, `node_policy`, `fs_policy`, `fs_guard_backend` und `fs_delete`. In `exec_server.rs` nur Bindung des erfolgreich authentifizierten Connection-Principals. In `peer_request.rs`/`peer_fs_logging.rs` nur die freigegebenen Request/Response-/Read-Retry-/Logging-Arme.

## Dateien erstellt

- `native/src/share/core/node_policy.rs`
- `native/src/share/core/node_restrictions.rs`
- `native/src/share/core/fs_authority.rs`
- `native/src/share/core/fs_policy.rs`
- `native/src/share/core/fs_policy_destructive.rs`
- `native/src/share/core/fs_local_paths.rs`
- `native/src/share/core/fs_guard_backend.rs`
- `native/src/share/core/fs_guard_extensions.rs`
- `native/src/share/core/fs_guard_reports.rs`
- `native/src/share/core/fs_guard_stream.rs`
- `native/src/share/core/fs_guard_bulk.rs`
- `native/src/share/core/fs_delete.rs`
- `native/src/share/core/fs_delete_local.rs`
- `native/src/share/core/fair_admission.rs`
- `native/src/share/core/peer_literal_paths.rs`
- `native/src/share/core/node_policy_task_tests.rs`
- `native/src/share/core/fs_policy_task_tests.rs`
- `native/src/share/core/fs_guard_backend_task_tests.rs`
- `native/src/share/core/fs_guard_reports_task_tests.rs`
- `native/src/share/core/fs_delete_task_tests.rs`
- `native/src/share/core/fair_admission_task_tests.rs`
- `native/src/local_access/os/linux/remove.rs`
- `native/src/local_access/os/linux/private_ancestors.rs`
- `native/src/local_access/os/windows/remove.rs`
- `native/src/local_access/os/windows/private_ancestors.rs`
- `native/src/share/core/fs_guard_names.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-DISPATCH.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-DISPATCH.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-DISPATCH.md`

Die drei Berichtdateien werden in diesem Block erstellt und finalisiert. Keine globalen Planänderungen; keine Änderung von Dateien des abgeschlossenen S-REVOKE-Blocks außerhalb der neuen Freigabe.

## Offene Anschlüsse

Exakte API-Semantik steht in [api-delta/H-DISPATCH.md](../api-delta/H-DISPATCH.md). Konkrete Hauptanschlüsse und verbleibende Provider-/Local-Grenzen stehen in [anfragen/H-DISPATCH.md](../anfragen/H-DISPATCH.md). Nach dieser Übergabe stoppt der H-DISPATCH-Worker.
