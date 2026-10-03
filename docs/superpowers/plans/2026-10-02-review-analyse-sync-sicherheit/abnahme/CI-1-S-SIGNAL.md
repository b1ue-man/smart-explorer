# CI-1-S-SIGNAL: Share-Facaden und Helper-Sichtbarkeit

Stand: 2026-10-03. Begrenzte Korrektur der zugeordneten Diagnosen aus
[Run 37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629)
nach Anwendung des kandidatengebundenen Remote-Formatterpatchs. Umsetzung und
statischer Self-Review abgeschlossen; die erneute gemeinsame Remote-Abnahme liegt
beim Hauptagenten. Keine lokale Ausführung, Git-, CI-, Graph- oder Releaseaktion.

## Fundzuordnung und Entscheidungen

| Belegter Fehler | Änderung | Erhaltene Grenze |
|---|---|---|
| E0432: `crate::share::fs_request::FsReversibleReplace` fehlt in FS-, Peer- und Replace-Fixture-Consumern | Private Share-Fassade projiziert ausschließlich `wire::FsReversibleReplace`. | Der bestehende, einmal unter `wire` registrierte Requesttyp bleibt identisch; keine zweite Definition, kein Wireformatwechsel. |
| E0624: `bind_incoming_principal` ist für FS-/Exec-Server privat | Sichtbarkeit auf `pub(in crate::share)` begrenzt. | Policyticket, Connection-Close-Abbruch, aktives Sharing und admitted Principal bleiben unverändert. |
| E0624: `fair_yield_requested`, `fair_yield_ready`, `incoming_stream_pending` sind für den FS-Server privat | Nur diese drei Methoden erhalten `pub(in crate::share)`. | Fairness, Pending-Guard und RAII-Freigabe behalten ihre ursprünglichen Funktionskörper. |
| E0624: `invalidate_restrictions_at` für Konfigurationsruntime und `invalidate_sessions` für bestehende Backend-Fixture privat | Beide Methoden erhalten `pub(in crate::share)`. | Autorisierungsepoche, Widerruf, Exec-Einschränkung und Session-/Lease-Abbau bleiben unverändert. |
| E0624: `release_stage` für Peer-Replace privat | Ausschließlich diese Methode erhält `pub(in crate::share)`. | Die Stagefreigabe erfolgt weiterhin nur über den bestehenden bestätigten Replace-Vertrag. |
| E0433: fehlende `log`-Dependency in Host-Recycle | Die vorhandene Diagnose verwendet `eprintln!("Host recycle {}: {error}", target.path)`. | Diagnose, Fehlerfortleitung und die folgende Sanitization bleiben erhalten; keine neue Dependency. |
| Sharemod nach Remote-Formatierung über Dateigrenze | Bestehende Exporte, Wrapper und zugehörige Testregistrierungen in `api_exports.rs` ausgelagert. | `include!` expandiert am bestehenden Share-Root. Öffentliche und crate-interne Namen, Testmodulpfade und alle cfg-Attribute bleiben erhalten. |
| Exec-Server nach Remote-Formatierung über Dateigrenze | Authentifizierung und Zulassung als kohäsives Kindmodul `exec_admission.rs`; bestehender Handlername wird zurückexportiert. | Gleiche Challenge, harte Handshakefrist, Principalbindung, Registryzulassung und Lebensdauer der Admission-/Activity-Guards; Streaming, Heartbeat und Ergebnis-ACK bleiben im Server. |
| LAN-Transport nach Remote-Formatierung über Dateigrenze | Bestehender outbound Probe-/Dial-Actor als `lan_link_dial.rs` ausgelagert. | Aktuelle volle Pins, private Dialhinweise, Peer-/Candidategrenzen, Fairnesscursor, eigene Permits, harte Connectfrist und Sessioncleanup bleiben unverändert. |

Die beiden Kindmodule passen ausschließlich ihre qualifizierten Share-Modulpfade
an die zusätzliche Ebene an. Die Dialmethode ist nur für ihr Transport-Elternmodul
sichtbar. Es gibt keine neue öffentliche API und keine zusätzliche Registrierung
des FS-Requestquelltexts. `lan_link_frames.rs` wurde nicht benötigt und nicht erstellt.

## Statische Prüfsignale

Alle zehn geänderten oder neuen Rustdateien wurden mit dem vorhandenen
Tree-sitter-Rustparser ohne Syntaxfehler eingelesen. Ein Textvergleich gegen den
vor der Änderung gesicherten Inhalt bestätigt die vollständigen ursprünglichen
Funktionskörper nach Rücknahme der Sichtbarkeits- und Modulpfadanpassungen.
Die rekonstruierte Share-Fassade hat dieselben Exporte, Wrapper, Registrierungen
und cfg-Attribute; nur Leerzeilen wurden bereinigt. Fixturequellen und Assertions
wurden nicht verändert. Das ist statische Evidenz, keine Compiler- oder Laufzeitabnahme.

| Rustdatei | Zeilen | Bytes |
|---|---:|---:|
| `native/src/share/mod.rs` | 472 | 14397 |
| `native/src/share/api_exports.rs` | 163 | 7063 |
| `native/src/share/core/exec_server.rs` | 334 | 12982 |
| `native/src/share/core/exec_admission.rs` | 175 | 6212 |
| `native/src/share/core/node_idle.rs` | 333 | 11906 |
| `native/src/share/core/node_restrictions.rs` | 135 | 5033 |
| `native/src/share/core/peer_transfer.rs` | 483 | 17438 |
| `native/src/share/os/shared/host_mutations.rs` | 112 | 3831 |
| `native/src/share/os/shared/lan_link_transport.rs` | 423 | 14805 |
| `native/src/share/os/shared/lan_link_dial.rs` | 104 | 3941 |

Die drei zugewiesenen Größenverletzungen sind mit Reserve geteilt. Die übrigen
Dateien liegen ebenfalls unter 500 Zeilen und 50 KiB. Der bereits remote formatierte
Peer-Transfer erhält ausschließlich die kurze Sichtbarkeitskorrektur.

## Abnahme in derselben Root-Suite

`review-task.yml` / `native/test-review-task.sh` müssen die zugeordneten E0432-,
E0433- und E0624-Meldungen in den bestehenden Linux-/Windows-Testhosts und dem
Linux-CLI-Build beseitigen. Der Remote-Formatter muss die zehn Dateien unter den
Dateigrenzen bestätigen. Alle bisherigen ausgewählten Assertions bleiben verbindlich;
es gibt keine neue Testkampagne und keinen veränderten Fixturevertrag.

Konkrete bestehende Symbole der direkt betroffenen Grenzen:

- `remote_drive_task_iroh_mount_reconnects_without_losing_lease`
- `review_task_h_replace_no_feature_is_mutation_free`
- `review_task_h_replace_idle_close_and_lost_ack_never_replay_or_release`
- `review_task_h_replace_only_confirmed_true_releases_own_stage`
- `review_task_h_replace_readonly_host_preserves_all_objects`
- `review_task_h_replace_retained_contract_keeps_literal_parent_and_nonce`
- `review_task_h_replace_wire_classifies_mutation_and_requires_explicit_boolean`
- `review_task_s09_transport_real_private_tls_round_has_only_status_rights`
- `review_task_s09_transport_rejects_wrong_pin_and_replayed_confirm`
- `review_task_s09_transport_malformed_and_stalled_frames_are_bounded`
- `review_task_s09_transport_path_revisions_reject_returned_path_evidence`
- `review_task_s09_transport_close_withdraw_disable_discard_cached_facts`
- `review_task_s09_transport_expired_or_unknown_facts_fail_closed`

Die bestehende `exec_server_tests.rs`-Registrierung bleibt an ihrem bisherigen
Elternmodul; deren Quelle war nicht zum Lesen freigegeben und wurde nicht gelesen.

## Exaktes Dateiinventar

Gelesene Belege und Scope:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-1-s-signal.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fixes.md`
- `docs/refs/rv1-remote-suite.md`
- `/tmp/rv1-ci-first/s-signal.json`

Gelesene vorhandene Quellen, teilweise ausschließlich betroffene Ausschnitte:

- `native/src/lib.rs` – nur vorhandenes `eprintln!`-Fehlerberichtsmuster
- `native/src/share/mod.rs`
- `native/src/share/core/backend_tests.rs`
- `native/src/share/core/configuration_runtime.rs`
- `native/src/share/core/exec_auth.rs`
- `native/src/share/core/exec_server.rs`
- `native/src/share/core/fs_access.rs`
- `native/src/share/core/fs_request.rs`
- `native/src/share/core/fs_response.rs`
- `native/src/share/core/fs_reversible_replace.rs`
- `native/src/share/core/node.rs`
- `native/src/share/core/node_idle.rs`
- `native/src/share/core/node_restrictions.rs`
- `native/src/share/core/peer_request.rs`
- `native/src/share/core/peer_reversible_replace.rs`
- `native/src/share/core/peer_transfer.rs`
- `native/src/share/core/reversible_replace_task_tests.rs`
- `native/src/share/core/server.rs`
- `native/src/share/core/server_fs.rs`
- `native/src/share/core/session.rs`
- `native/src/share/core/wire.rs`
- `native/src/share/os/shared/host_mutations.rs`
- `native/src/share/os/shared/lan_link_transport.rs`
- `native/src/share/os/shared/lan_link_transport_task_fixture.rs`
- `native/src/share/os/shared/lan_link_transport_task_lifecycle_tests.rs`
- `native/src/share/os/shared/lan_link_transport_task_tests.rs`

Geänderte vorhandene Dateien:

- `native/src/share/mod.rs`
- `native/src/share/core/exec_server.rs`
- `native/src/share/core/node_idle.rs`
- `native/src/share/core/node_restrictions.rs`
- `native/src/share/core/peer_transfer.rs` – nur `release_stage`-Sichtbarkeit
- `native/src/share/os/shared/host_mutations.rs`
- `native/src/share/os/shared/lan_link_transport.rs`

Erstellt und für den Self-Review gelesen:

- `native/src/share/api_exports.rs`
- `native/src/share/core/exec_admission.rs`
- `native/src/share/os/shared/lan_link_dial.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-S-SIGNAL.md`

## Restgrenzen

Keine offene Scope- oder Dependencyanfrage. Die im Manifest auch genannten
`exec_state.rs`, `node_fairness.rs`, `node_restriction_controls.rs`, `peer_core.rs`
und `peer_stage.rs` waren nicht vorhanden; die tatsächlichen Definitionen standen
in den freigegebenen Dateien. Es wurde keine Ersatzoberfläche erkundet.
Integration, Commit/Push, Rootgraph und die Auswertung derselben Remote-Suite
bleiben beim Hauptagenten. Keine Behauptung einer bereits erfolgreichen CI-Abnahme.
