# CI-1-S-REVOKE – belegte RV1-Korrekturen

Stand: 2026-10-03. Begrenzter Anschluss aus [Run 37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629), Kandidat `395f912a30455ebd96799f61fdbe1fec2e1c7998`. Der vom Hauptagenten geprüfte Remote-Formatterpatch war vor diesen Änderungen angewandt. Der Source-Anschluss ist abgeschlossen; die erneute gemeinsame Remote-Abnahme bleibt beim Hauptagenten. Kein neuer Projekt-Review und keine eigenständige Testsuite.

## Belegte Grenzen und Entscheidungen

Der Kontext kommt aus `ci-fixes.md`, den gelieferten Cargo-JSON-Diagnosen und den freigegebenen tatsächlichen Definitionen. Die Korrekturmeilensteine sind: veraltete Policy-Aufrufe an den vorhandenen Vertrag anschließen, den tatsächlichen Write-Reset bei Klartext-Verweigerung auswerten und ausschließlich Ledger-Erzeugungshelfer kohäsiv auslagern. Erwartet sind unveränderte Aufnahme-/Pin-/Rechteassertions und erfolgreiche Auswertung derselben Remote-Suite.

| Beleg | Umsetzung | Erhaltenes Signal |
|---|---|---|
| E0061: Legacy-Aufnahme hat einen vierten Parameter | Alle betroffenen Aufrufe übergeben explizit `DirectRequestPolicy`. | Vorhandene Receive-, Konflikt-, Revoke-, Replay-, Tombstone-, Retry- und Persistenzassertions bleiben erhalten. |
| E0609: entferntes `ShareProfiles::direct_request_policy` | Die früheren Autoaccept-Fixtures binden ihre lokale Policy und übergeben sie an die Aufnahme. | Keine Policy wird neu in den Profilen gespeichert; kein Produktionsvertrag wird verändert. |
| Server-Klartext wird vor Registrierung verweigert, Client-Write kann `ConnectionReset` melden | Das Write-Ergebnis wird bis nach dem Server-Join aufbewahrt. Ausschließlich `ConnectionReset` wird als transportseitige Verweigerung akzeptiert. | Der Server muss weiterhin mit `PermissionDenied` enden und `clients.is_empty()` muss weiterhin erfüllt sein. Andere Write-Fehler bestehen den Fall nicht. |
| Formatter-Dateigrenze: Ledger-Fixture 500 Zeilen | Nur Konstanten und Signatur-/Kontakt-/Envelope-Erzeugung werden in `direct_ledger_fixture.rs` verschoben; lokale Registrierung in `direct_ledger_tests.rs`. | Sämtliche bisherigen Testkörper bleiben wörtlich im bisherigen Modul. Kontaktpins, Grant-/Exec-Felder und alle signierten Envelope-Inputs bleiben unverändert. |

Die eng nachfreigegebene Definition bestätigt `DirectRequestPolicy::Ask` als Standard. Default-, Konflikt- und Validierungsfälle verwenden deshalb ausdrücklich `Ask`. Nur die bereits ausdrücklich auf Autoaccept ausgelegten Fälle verwenden `AutoAccept`: die fünf zuvor über das entfernte Profilfeld konfigurierten Legacy-Fixtures und `share_remote_task_legacy_direct_autoaccept_retry_tombstone_and_denial`. `authenticated_decision` priorisiert weiterhin Identitäts-/Key-Verweigerung und vorhandene genaue Grants; diese produktiven Regeln wurden nicht geändert.

Der Reset ersetzt keinen Autorisierungsnachweis. Die ursprüngliche Server-`PermissionDenied`-Assertion und der leere Clientbestand werden auch nach einem fehlgeschlagenen Write zwingend ausgewertet. TLS-Konfiguration, Challenge-/Loginfluss, Registrierungsablauf und produktive Transport-/Policydateien bleiben unverändert.

## Statisches Self-Review

Kurze lexikalische Text-/Parsing-Prüfungen bestätigen ausgeglichene Rust-Delimiter, erlaubte Änderungs-/Create-Pfade und die Größenlimits. Beim Vergleich der drei Policy-Fixtures wurden ausschließlich die neue explizite Policybindung/-übergabe und die Ersetzung des entfernten Profilfelds ausgeblendet: alle übrigen Rust-Tokens, Assertions, Aufnahme-/Pininputs und Literalwerte stimmen mit dem vorliegenden Formatterstand überein.

Die Ledger-Testkörper sind textidentisch. Die ausgelagerten Helper sind nach Abzug notwendiger Sichtbarkeit und Deklarationsformatierung tokenidentisch. Beim Klartextfall bleiben nach Abzug der gezielten Write-Ergebnis-/Reset-Auswertung ebenfalls alle bisherigen Fixture-Tokens erhalten. Es wurden keine neuen `#[test]`-Funktionen hinzugefügt; das neue Ledger-Modul enthält nur Erzeugungshelfer.

| Datei | Zeilen | Bytes |
|---|---:|---:|
| `native/src/share/core/direct_identity_conflict_tests.rs` | 247 | 9014 |
| `native/src/share/core/legacy_direct_request_tests.rs` | 438 | 16233 |
| `native/src/share/core/share_remote_direct_task_tests.rs` | 497 | 17583 |
| `native/src/share/core/direct_ledger_tests.rs` | 383 | 12193 |
| `share-server/src/signal_security_transport_tests.rs` | 266 | 9854 |
| `native/src/share/core/direct_ledger_fixture.rs` | 127 | 3460 |

Die Ledger-Teilung lässt normale Formatierung mit deutlicher Reserve zu. Remote-Direct hat nur eine zusätzliche lokale Policybindung; sämtliche Zeilen bleiben höchstens 99 Spalten lang. Kein lokaler Formatter, Compiler, Build, Test, Server, Git-/CI-/Graph-/Releaseprozess oder weiterer Agent wurde gestartet. Die Prüfungen belegen Source-/Textkonsistenz; Compiler- und Verhaltensnachweis kommt aus der bestehenden Remote-Suite.

## Acceptance im unveränderten Remote-Einstieg

Der Hauptagent verwendet erneut denselben vollständigen RV1-Einstieg `review-task.yml`/`native/test-review-task.sh`. Die bestehenden Symbolnamen bleiben erhalten. Relevant sind die folgenden vorhandenen Selektoren; keine zusätzliche manuelle Testsammlung oder neue Suite wird verlangt.

### `native/src/share/core/direct_identity_conflict_tests.rs`

- `tracked_conflicts_are_symmetric_in_both_arrival_orders`
- `rejecting_a_spoof_never_changes_the_legitimate_grant_or_exec_policy`
- `explicit_accept_replaces_only_an_inactive_different_key_pin`
- `ci_remote_task_tracked_reject_preserves_legacy_denial_without_a_false_live_conflict`
- `ci_remote_task_legacy_revoke_resolves_conflict_before_tracked_accept`

### `native/src/share/core/legacy_direct_request_tests.rs`

- `verified_receive_survives_reload_and_new_nonce_updates_same_selector`
- `ci_remote_task_replay_requires_revoke_before_delete_and_retains_denial`
- `ci_remote_task_autoaccept_revoke_and_manual_answer_retry_remain_truthful`
- `ci_remote_task_identity_conflict_is_rejected_without_replacing_the_grant`
- `ci_remote_task_first_verified_identity_wins_in_both_arrival_orders`
- `ci_remote_task_generic_grant_upsert_cannot_replace_an_autoaccepted_identity`
- `ci_remote_task_load_reconciles_autoaccepted_history_when_its_grant_was_lost`
- `far_future_presence_cannot_pin_inbox_or_tombstone_capacity`
- `identity_rotation_disables_even_unlinked_direct_and_exec_grants`
- `corrupt_evidence_and_future_schema_fail_closed_while_v6_defaults_empty`

### `native/src/share/core/share_remote_direct_task_tests.rs`

- `share_remote_task_reciprocal_direct_fresh_autoaccepts_both_sides`
- `lan_cleanup_task_removed_peer_blocks_automatic_repair_until_user_pairs_again`
- `share_remote_task_reciprocal_direct_repairs_legacy_pins_and_retries_idempotently`
- `share_remote_task_reciprocal_direct_denial_unsupported_and_identity_conflict_fail_closed`
- `share_remote_task_legacy_direct_autoaccept_retry_tombstone_and_denial`

### `native/src/share/core/direct_ledger_tests.rs`

- `outgoing_request_retains_every_signed_envelope_and_peer_confirmed_state`
- `incoming_decision_outbox_survives_accept_and_newer_revoke`
- `history_deletion_waits_for_terminal_peer_delivery`
- `retry_and_relay_updates_are_absolute_monotonic_and_idempotent`
- `legacy_forwarding_stops_automatic_request_retries_until_manual_retry`
- `request_ids_and_signed_artifacts_cannot_be_rebound`
- `persisted_ledger_validation_rejects_duplicate_request_ids`

### `share-server/src/signal_security_transport_tests.rs`

- `review_task_wss_login_output_wake_and_certificate_reload`
- `review_task_plaintext_is_refused_before_registration`
- `review_task_raw_tcp_key_login_rejects_another_signer`
- `review_task_login_challenge_is_connection_bound_and_single_use`

Konkrete Auswertung: keine E0061-/E0609-Meldungen an den korrigierten Aufrufen; die jeweiligen bisherigen Assertions bestehen. Insbesondere bleiben konkurrierende Identitäten verweigert, Revoke vor History-Delete erforderlich, Exec-Rechte nach Widerruf deaktiviert und die aufgenommenen Pins unverändert. Klartext darf trotz erfolgreichem TCP-Connect und begonnenem Hello niemals einen Client registrieren. Sowohl ein abgeschlossener Write als auch der belegte frühe Write-Reset müssen denselben Server-`PermissionDenied`- und No-Registration-Nachweis erreichen.

## Dateien gelesen

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-1-s-revoke.json`
- `/tmp/rv1-ci-first/s-revoke.json`
- `native/src/share/core/direct_identity_conflict_tests.rs`
- `native/src/share/core/legacy_direct_request_tests.rs`
- `native/src/share/core/share_remote_direct_task_tests.rs`
- `share-server/src/signal_security_transport_tests.rs`
- `native/src/share/core/direct_ledger_tests.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fixes.md`
- `native/src/share/core/legacy_direct_request_mutations.rs`
- `native/src/share/core/direct_ledger.rs`
- `native/src/share/mod.rs`
- `share-server/src/transport.rs`
- `share-server/src/transport_serve.rs`
- `docs/refs/share-server-tls-auth.md`
- `docs/refs/rv1-remote-suite.md`
- `native/src/share/core/direct_ledger_fixture.rs`
- `native/src/share/core/profiles.rs`
- `native/src/share/core/direct_relation.rs`
- `native/src/share/core/legacy_direct_request_decision.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-S-REVOKE.md`

Die Reads von `direct_relation.rs` waren ausschließlich auf `DirectRequestPolicy` begrenzt; `legacy_direct_request_decision.rs` auf `Refusal` und `authenticated_decision`. Bereits vorliegende AGENTS-/Arbeitsweise-Vorgaben wurden weiter angewandt; außerhalb des aktuellen Manifests wurde keine Quelle neu gelesen.

Diese zusätzlich aufgeführten Manifestpfade wurden versucht, existieren aber nicht und wurden nicht durch eigene Suche ersetzt:

- `native/src/share/core/direct_request_policy.rs`
- `native/src/share/core/request_policy.rs`
- `native/src/share/core/direct_profiles.rs`
- `native/src/share/core/profile_edits.rs`
- `native/src/share/core/request.rs`
- `native/src/share/core/share_remote_task_fixture.rs`

## Dateien geändert

- `native/src/share/core/direct_identity_conflict_tests.rs`
- `native/src/share/core/legacy_direct_request_tests.rs`
- `native/src/share/core/share_remote_direct_task_tests.rs`
- `native/src/share/core/direct_ledger_tests.rs`
- `share-server/src/signal_security_transport_tests.rs`

## Dateien erstellt

- `native/src/share/core/direct_ledger_fixture.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-S-REVOKE.md`

Die Registrierung des neuen Fixture-Moduls liegt ausschließlich in der zugeordneten `direct_ledger_tests.rs`; `share/mod.rs` wurde nur gezielt gelesen und nicht geändert. Es gab keine Produktions-APIänderung.

## Scope-Lücken und Übergabe

Die fehlenden tatsächlichen Policydefinitionen wurden gezielt an den Hauptagenten gemeldet und von ihm in diesem Manifest nachfreigegeben; dieser Anschluss ist erledigt. Für den begrenzten Korrekturblock besteht keine weitere offene Source-/API-Anfrage. Andere CI-Diagnosen, Suiteausführung, Git-Kontext/Integration, Commit/Push, Graph und Release bleiben beim Hauptagenten. Nach Übergabe stoppt der Worker.
