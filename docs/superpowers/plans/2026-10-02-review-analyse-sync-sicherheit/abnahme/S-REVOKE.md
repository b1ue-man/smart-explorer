# S-REVOKE – Beziehungen und Rechteentzug

Stand: 2026-10-03. Umsetzung des freigegebenen V5-Vertrags; die Remote-Abnahme steht aus.

## Fortgeführter Meilensteinplan

| Ergebnis | Zuständiger Pfad | Erwartetes Abnahmesignal |
|---|---|---|
| Rechteänderungen treffen den Schlüssel in seiner Beziehung; Erweiterungen und Präsenzdaten stören keine laufende Arbeit | `relation_rights`, Exec-Registry, Daemon-Ereignisse | Zwei aktive Prinzipale, zwei Räume; Änderung an einem beendet nur dessen Arbeit. Laufzeitdaten und Hinzufügen eines Mitglieds lösen keine Epoche aus. |
| Entfernen und Sperren gelten auch bei anderer Geräte-ID; neue Anfragen warten ohne Opt-in | Direct/Legacy-Ledger, Entfernung, reziproke Kopplung | Derselbe Schlüssel unter neuer ID bleibt abgewiesen; ein fremder neuer Schlüssel wartet. Bewusste Wiederzulassung erhält Schreibrecht und lässt Exec aus. |
| Rotation verlangt Wiederbestätigung und lässt ausdrückliche Sperren bestehen | Identitätsabgleich, Projektion, Legacy-Entscheidungen | `Accepted → Reconfirm → Accepted` mit aktuellem Code; `Ignored` bleibt gesperrt; automatische Reparatur bestätigt niemals wieder. |
| Präsenzen binden Namen, Fingerprint, Knoten und Routen an den Geräteschlüssel | Signieren, Signaturprüfung, Daemon-Projektion | Manipulation und Rückstufung werden verworfen; gespeicherte Pins bleiben erhalten; Replay bleibt bis Ablauf abgewiesen. |
| Raum-Sperren verlangen Zulassung neuer Identitäten und schützen bekannte Pins | Raum-Mitglieder, Laufzeitübernahme, Exec | Derselbe gesperrte Schlüssel unter neuer ID kommt nicht hinein; neuer Schlüssel bleibt `Pending` und blockiert bis Nutzerzulassung. |
| Reparatur kann Rechteentzug nicht während Peer-I/O aufhalten | Direct-Transport und sein Ausgangsaufrufer | Gestallte Hello/Offer/Commit/ACK halten kein Transition-Permit; Persistenz prüft unter kurzem Permit die aktuelle Autorisierung erneut. |

Kompatibilität: neue Grants/Räume bleiben nur lesbar, Altdaten behalten gewählte Schreibrechte;
Endpunkt-Lokatoren und Backend-Identitäten werden unverändert verglichen; bestehende Grants werden durch
einseitige Kopplung nicht entzogen; Fehler-/Abbruch- und CAS-Pfade bleiben wiederholbar.


## Umsetzung und Fundzuordnung

| Fund / Pflichtteil | Konkrete Umsetzung | Abnahmesignal der zentralen Suite |
|---|---|---|
| FA3, B04 | Reduktionsdiff nach Relation/Principal; Export-Lokatoren bleiben exakt; Exec hat je Principal eine Mindestepoche, unabhängige Tokens bleiben gültig | `review_task_restrictions_ignore_runtime_and_extensions_but_revoke_exact_key`, `review_task_export_reduction_preserves_backend_identity_and_scopes_room`, `review_task_restriction_keeps_other_principal_launch_and_blocks_old_token` |
| FC1, S21, S23 | Einseitig als Standard; User-Wahl auch bei bestehenden Kontakten übernommen; automatische Reparatur erzeugt ohne Share-back kein Grant und bestätigt nicht neu; bewusste Wiederfreigabe schaltet Exec aus | `review_task_one_way_pairing_and_repair_never_create_share_back_grant`, `review_task_legacy_new_peer_waits_reconfirm_accepts_and_key_denial_wins` |
| FC5, S19, S28, S29 | Key-/Node-Aliase werden gemeinsam entzogen; entfernte Schlüssel bleiben über den historischen 64er-Wert hinaus gespeichert; neue unbekannte Geräte warten bei Ask | `review_task_withdrawal_denies_device_alias_until_deliberate_readmission`, `review_task_removed_denials_survive_historical_capacity`; historische AutoAccept-Fälle wählen Opt-in ausdrücklich |
| B15, S05, S22 | Raum-Sperre gilt je Schlüssel/Knoten, schaltet Exec aus und verlangt Bestätigung neuer Identitäten; späte Worker-Daten wenden die aktuelle Raumpolitik an | `review_task_room_block_rejects_key_alias_and_new_members_wait_without_exec`, `review_task_new_worker_room_member_uses_current_confirmation_policy` |
| B03, S06, S17, S26, S30, S46, S55 | Gerätesignatur über längenkodierten Kontext; Empfänger und Accept-Bit in 126-Byte-Nonce; Fingerprint lokal; bestehende Pins bleiben; Signaturlernen unmittelbar und dauerhaft monoton | `review_task_presence_signature_binds_names_fingerprint_node_and_routes`, `review_task_legacy_decision_nonce_binds_recipient_and_value_within_wire_limit`, `review_task_first_signature_is_remembered_before_daemon_event` |
| S16 | Voller Replay-Cache weist neue Nachrichten ab und verdrängt keine gültige Nonce; nur nachweislich abgelaufene Einträge werden freigegeben; Nonce-Formate anderer Autorisierungspfade bleiben erhalten | `review_task_replay_capacity_rejects_new_without_forgetting_old`, `review_task_replay_pruning_retains_other_authorization_nonce_formats` |
| S23, B04 | Temporäres Offline trennt Sitzungsrecht von einer gespeicherten Exec-Entscheidung; Online-Erweiterung braucht keine neue Policyrevision; aktuelle bewusste Freigabe schlägt alte Legacy-Widerrufshistorie | `review_task_online_extension_preserves_policy_after_offline_barrier`, `review_task_legacy_current_explicit_grant_supersedes_old_revocation_history` |
| S32 | Lokale Lifecycle-Beobachtung wird auf mindestens die authentisierte Remote-Zeit geklammert; signierte Envelopes werden nicht umgeschrieben | `review_task_future_request_is_received_without_waiting_for_local_clock`, `review_task_decision_delivery_accepts_clock_skew_in_both_directions` |
| S66 | Reale Ausgangs-/Daemon-Integration: Reload/Configure warten nicht auf den gesamten Reparaturaustausch; Wire-I/O ohne Transition-Permit; aktuelle Identität/Pins/Secret erst unter dem kurzen Persist-Gate geprüft; Store behält Permit nach Async-Abbruch; gecachte Receipts prüfen erneut durable Policy | `review_task_outgoing_store_gate_uses_current_peer_pins_without_global_epoch_equality`, `review_task_outgoing_repair_yields_immediately_to_configuration`, `review_task_reciprocal_auth_is_reread_only_at_durable_write`, `review_task_reciprocal_timeout_holds_permits_until_store_finishes` |
| FA3, S29, S30 | Late-Worker-Rebase erhält concurrent Entzug, Pins und monotone Signaturfakten statt volle Contact-/Member-Kopien zu übernehmen | `review_task_late_worker_cannot_restore_withdrawn_access_or_clear_signature`; bestehender Runtime-Übernahmefall |
| S24 | Unbestätigte Kopplung wird erst nach erfolgreicher persistierter Direct-/Room-Entfernung vergessen; Fehler bleibt wiederholbar | Integrierte GUI-Folge: Speicherfehler hält `unconfirmed`, Wiederholen mit erfolgreicher Entfernung löscht den Hinweis. Orchestrator bindet die UI-/Persistenz-Integration in die zentrale Suite ein. |
| B21-Anschluss | Daemon konsumiert den vorhandenen Migrationshelfer nach Regularfile-/16-KiB-Prüfung, canonical Some und Fehler unverändert weiter | S-SIGNAL-Migration + Daemon-Start über echte `share_server.txt`; schema-lose persistierte Adresse bleibt TCP. |

### Entscheidungen und Grenzen

- Autorisierung wird nur enger, wenn tatsächlich ein bisheriges Recht oder ein Pin wegfällt. Präsenz,
  Zeitstempel, Namen, Routen und Erweiterungen tragen keine neue Epoche. Relation-Scopes schließen bei
  fehlendem Principal-Pin die bekannte Beziehung konservativ; nicht zuordenbare globale Änderungen
  bleiben global.
- Denials werden niemals aus Alters-/Anzahlgründen still gelöscht. Eine zu große Profiltransaktion
  darf im vorhandenen Bytehaushalt fehlschlagen. Die zugehörige Profilvalidierung liegt bei S-POLICY.
- Der alte HMAC bleibt für Legacy-Geräte/Server lesbar. Die signierte Nonce bleibt in deren 128-Byte-
  Grenze. Alte unsignierte Geräte haben den vorhandenen eingeschränkten Pending-Kompatibilitätsweg;
  diese Kompatibilität ist keine nachträgliche Gerätesignatur. Freier Legacy-Text wird nicht vertraut.
- Direct-Online und Room-Aktivität sind Sitzungszustände; ihre Wiederaktivierung verfälscht keine
  abgeschaltete Exec-Policyrevision. Alte Tokens des eingeschränkten Principals bleiben dennoch
  gesperrt. Containment-/Terminal-/Startbarrieren und aktive Limits bleiben bestehen.
- Neue Rust-Dateien sind kohäsive Nachbarn. Es wurde kein gemeinsamer Registereintrag ersetzt.
  Vorhandene Extraktionen aus V5 wurden weitergeführt. Keine lokale Graph-Erneuerung.

### Statische Evidenz

`git diff --check` über die eigenen geänderten Pfade ohne Befund. Ein reiner Textscan prüfte
Klammern außerhalb von Strings/Kommentaren, angrenzende `#[path]`-Ziele, Whitespace und Dateigrenzen;
alle neu erstellten und wesentlich geänderten Rust-Dateien bleiben unter 500 Zeilen und 50 KiB.
Die bestehende große `node_sessions.rs` wurde nur am freigegebenen Aufrufer angeschlossen;
neue Verhaltenslogik liegt im angrenzenden Persist-Gate, die übrige Datei bleibt bei H-DISPATCH.
Das ist keine Rust-Typprüfung und keine Verhaltensabnahme. Keine Builds, Compiler, Linker, Cargo,
rustfmt, Tests, Server, Installationen, Commits, Pushes oder Releaseaktivitäten durch S-REVOKE.
Der Orchestrator hat den Graph-Kontext bereitgestellt und übernimmt die abschließende Graph-Aktualisierung.

Offene fremde Anschlüsse sind konkret in [Anfragen](../anfragen/S-REVOKE.md) und
[API-Delta](../api-delta/S-REVOKE.md) eingetragen. Eigener Block nach statischem Self-Review abgeschlossen;
die eine Remote-Suite muss die erwarteten Signale noch auswerten.

## Dateibericht

Vollständige Dateien bzw. fokussierte Definitionen gelesen; `rg`-Treffer sind mit aufgeführt.
Es erfolgte keine Exploration außerhalb des zugeteilten Scopes. Neue eigene Dateien wurden beim
Self-Review ebenfalls gelesen.

### Neue Source-Dateien

- `native/src/share/core/relation_rights_diff.rs`
- `native/src/share/core/relation_rights_task_tests.rs`
- `native/src/share/core/exec_registry_authority.rs`
- `native/src/share/core/direct_reciprocal_outgoing_gate.rs`
- `native/src/share/core/direct_reciprocal_outgoing_gate_task_tests.rs`
- `native/src/share/core/signal_presence_task_tests.rs`
- `native/src/share/core/signal_auth_replay.rs`
- `native/src/share/core/legacy_direct_request_decision_task_tests.rs`
- `native/src/share/core/direct_ledger_clock_task_tests.rs`
- `native/src/app/core/share_lifecycle_actions_ui.rs`
- `native/src/cli/share/grants_readmit.rs`

### Geänderte / fortgeführte Source-Dateien

- `native/src/app/core/share_direct_ui.rs`
- `native/src/app/core/share_discovery_events.rs`
- `native/src/app/core/share_lifecycle_ui.rs`
- `native/src/app/core/share_removal_ui.rs`
- `native/src/app/core/share_removed_devices_ui.rs`
- `native/src/cli/share/grants.rs`
- `native/src/daemon/os/shared/ipc_host.rs`
- `native/src/daemon/os/shared/ipc_host_direct_events.rs`
- `native/src/daemon/os/shared/ipc_host_profile_merge.rs`
- `native/src/daemon/os/shared/ipc_host_service.rs`
- `native/src/share/core/direct_ledger.rs`
- `native/src/share/core/direct_ledger_mutations.rs`
- `native/src/share/core/direct_ledger_projection.rs`
- `native/src/share/core/direct_reciprocal.rs`
- `native/src/share/core/direct_reciprocal_transport.rs`
- `native/src/share/core/direct_reciprocal_transport_task_tests.rs`
- `native/src/share/core/direct_relation.rs`
- `native/src/share/core/exec_auth.rs`
- `native/src/share/core/exec_grant_runtime.rs`
- `native/src/share/core/exec_grant_runtime_tests.rs`
- `native/src/share/core/exec_registry.rs`
- `native/src/share/core/exec_registry_tests.rs`
- `native/src/share/core/legacy_direct_request.rs`
- `native/src/share/core/legacy_direct_request_decision.rs`
- `native/src/share/core/legacy_direct_request_mutations.rs`
- `native/src/share/core/legacy_direct_request_reconciliation.rs`
- `native/src/share/core/legacy_direct_request_tests.rs`
- `native/src/share/core/node_sessions.rs` — nur `repair_direct_reciprocal`, anschließend an H-DISPATCH zurückgegeben.
- `native/src/share/core/relation_rights.rs`
- `native/src/share/core/removed_direct_peers.rs`
- `native/src/share/core/room_relation_members.rs`
- `native/src/share/core/signal_auth.rs`
- `native/src/share/core/signal_commands_local.rs`
- `native/src/share/core/signal_presence.rs`
- `native/src/share/os/shared/direct_repair_store_adapter.rs`
- `native/src/share/os/shared/removal.rs`

### Weitere gelesene Source-Dateien

- `native/src/app/core/share_exec_ui.rs`
- `native/src/app/core/share_identity_rotation.rs`
- `native/src/app/core/share_legacy_lifecycle_ui.rs`
- `native/src/app/core/share_lifecycle_view.rs`
- `native/src/cli/share/grants_exec.rs`
- `native/src/cli/share/grants_removed.rs`
- `native/src/daemon/os/shared/exec_grant_journal.rs`
- `native/src/daemon/os/shared/exec_grant_journal_storage.rs`
- `native/src/daemon/os/shared/exec_state.rs`
- `native/src/daemon/os/shared/ipc_host_commands.rs`
- `native/src/daemon/os/shared/ipc_host_direct_event_persistence.rs`
- `native/src/daemon/os/shared/ipc_host_direct_event_queue.rs`
- `native/src/daemon/os/shared/ipc_host_direct_event_schedule.rs`
- `native/src/daemon/os/shared/ipc_host_direct_events_tests.rs`
- `native/src/daemon/os/shared/ipc_host_events.rs`
- `native/src/daemon/os/shared/ipc_host_legacy_events.rs`
- `native/src/daemon/os/shared/ipc_host_relation_events.rs`
- `native/src/mobile/os/shared/domains/share_requests.rs`
- `native/src/share/core/crypto.rs`
- `native/src/share/core/direct_ledger_retention.rs`
- `native/src/share/core/direct_ledger_validation.rs`
- `native/src/share/core/direct_messages.rs`
- `native/src/share/core/direct_open_task_tests.rs`
- `native/src/share/core/direct_protocol.rs`
- `native/src/share/core/direct_protocol_lifetime_tests.rs`
- `native/src/share/core/direct_reciprocal_coordinator.rs`
- `native/src/share/core/direct_reciprocal_coordinator_task_test_support.rs`
- `native/src/share/core/direct_reciprocal_store.rs`
- `native/src/share/core/direct_reciprocal_worker.rs`
- `native/src/share/core/direct_reciprocal_worker_task_tests.rs`
- `native/src/share/core/exec_policy.rs`
- `native/src/share/core/exec_registry_view.rs`
- `native/src/share/core/export_config.rs`
- `native/src/share/core/identity.rs`
- `native/src/share/core/identity_repair.rs`
- `native/src/share/core/legacy_direct_request_validation.rs`
- `native/src/share/core/node.rs`
- `native/src/share/core/node_sessions_task_tests.rs`
- `native/src/share/core/room_relation.rs`
- `native/src/share/core/signal_commands.rs`
- `native/src/share/core/tracked_signal_dispatch.rs`
- `native/src/share/core/tracked_signal_verify.rs`
- `native/src/share/core/types.rs`
- `native/src/share/mod.rs`
- `native/src/share/os/shared/direct_actions.rs`
- `native/src/share/os/shared/direct_policy_store.rs`
- `native/src/share/os/shared/direct_reciprocal_persistence.rs`
- `native/src/share/os/shared/direct_relation_actions.rs`
- `native/src/share/os/shared/identity_store.rs`
- `native/src/share/os/shared/legacy_direct_actions.rs`
- `native/src/share/os/shared/lifecycle_view.rs`

### Gelesene Dokumentation / Vorgaben

- `/root/.codex/skills/arbeitsweise/SKILL.md`
- `AGENTS.md (Nutzerinhalt)`
- `docs/ARCHITEKTUR.md`
- `docs/lesungen/INDEX.md`
- `docs/refs/INDEX.md`
- `docs/refs/share-server-tls-auth.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/s-revoke.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/fortsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sicherheit.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/S-REVOKE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/README.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/README.md`

### Eigene Dokumentation

- Neu: `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S-REVOKE.md`
- Aktualisiert: `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/S-REVOKE.md`
- Neu: `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-REVOKE.md`

Zusätzliche zielgerichtete Lesefreigaben und API-Informationen des Orchestrators: S66-Ausgangsfunktion
in `node_sessions.rs`; S24-Aufrufer `share_discovery_events.rs` und dessen vorhandenes
`unconfirmed.get(...).cloned()`; Signalisierungsintegration bleibt ausschließlich bei S-SIGNAL.
Git-Status/-Log wurden als Dokumentationskontext geprüft; fremde Worktree-Änderungen bleiben unberührt.
