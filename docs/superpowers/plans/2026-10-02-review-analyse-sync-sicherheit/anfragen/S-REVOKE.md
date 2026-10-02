# Anfragen von S-REVOKE

Stand: 2026-10-02 (Vertrag V5). Je Eintrag: Datei, Stelle, Änderung, Grund. Signaturen: `umsetzung.md` V5.

## R0 (Orchestrator) – Besitz: bestätigt (2026-10-02)

- Neue Dateien des Vertrags: `native/src/share/core/direct_relation.rs` (Direkt-Typen aus `types.rs`
  verlegt), `native/src/share/core/relation_rights.rs` (Sitzungsrechte, Einschränkungs-Ereignis),
  `native/src/share/core/signal_commands_local.rs` (Untermodul von `signal_commands.rs`, das sonst über
  500 Zeilen gewachsen wäre). `relation_rights.rs` passt in kein Namensmuster der Besitztabelle; bitte als
  S-REVOKE-Datei bestätigen.
- Mit den eigenen Modulen geändert (Testbegleiter eigener Dateien, je nur Feldzeilen):
  `share/os/shared/{lifecycle_view_tests,identity_store_tests}.rs`,
  `share/core/signal_commands_task_tests.rs`, `cli/share/lifecycle_output_tests.rs`,
  `mobile/os/shared/domains/share_exec_tests.rs`. `share/core/direct_ledger_tests.rs` hat dadurch genau
  500 Zeilen (eine Feldzeile, keine inhaltliche Änderung).

## R1 – Konstruktionszeilen der V5-Felder: erledigt (Freigabe des Orchestrators, 2026-10-02)

Nur die Feldzeilen in Struktur-Literalen gesetzt, sonst nichts geändert:

| Typ | Zeile |
|---|---|
| `DirectGrant { … }` | `write: false,` (Vorrichtungen, die über Share schreiben: `write: true,`) |
| `DirectContact { … }` | `relation: Default::default(),` |
| `RoomProfile { … }` | `policy: crate::share::RoomPolicy::new_room(),` (in `profile_persistence.rs`: `super::room_relation::RoomPolicy::new_room()`) |
| `RoomMember { … }` | `relation: Default::default(),` |

| Datei | Funktion | Stand |
|---|---|---|
| `share/core/profiles.rs` | `set_direct_grant` (neue Freigabe, `write: false`) | erledigt |
| `share/core/profile_persistence.rs` | `add_direct_from_code_with`, `add_room_from_code_with`, Tests `direct_grant`, `room_with_member` | erledigt |
| `share/os/shared/profile_operations.rs` | `add_direct_from_code_persisted`, `add_room_material_persisted` | erledigt |
| `share/os/shared/profile_edits.rs` | Test `contact` | erledigt |
| `share/core/discovery_relation_store.rs` | `persist_room` | erledigt |
| `share/core/service_tests.rs` | `configure_requires_worker_ack…`, `direct_accept_or_reject…`, `presence_binds_node_id…` | erledigt |
| `share/core/signal_subscriptions.rs` | Tests `contact`, `room` | erledigt |
| `cli/share/discoverable_input.rs`, `cli/share/discoverable_output.rs` | Tests | erledigt |
| `share/core/backend_tests.rs` | `remote_drive_task_iroh_mount_reconnects…` (`write: true`) | erledigt |
| `share/core/power_test_support.rs` | `new` (`write: true`) | erledigt |
| `share/core/configuration_runtime_task_tests.rs` | `eligible_snapshot` | erledigt |
| `share/core/lan_presence_match.rs` | Test `contact` | erledigt |
| `share/core/copy_paste_task_fixture.rs` | `loopback` (`write: true`) | erledigt |
| `share/core/share_remote_direct_task_tests.rs` | `grant_for` | erledigt |
| `share/core/signal_configure_tests.rs` | `contact`, `room` | erledigt |
| `share/core/peer_endpoint_source.rs` | Test `contact` | erledigt |
| `share/core/identity_profile_reconciliation_tests.rs` | `contact` | erledigt |
| `cli/completions.rs`, `cli/completions_requests.rs` | Tests | erledigt |

Hinweis (Orchestrator): `share/core/copy_paste_task_fixture.rs` und `share/core/peer_endpoint_source.rs` waren
schon vor RV1 nicht rustfmt-sauber (gleicher Diff wie `HEAD`); da sie jetzt zum Batch gehören, schlägt das
rustfmt-Tor der Suite bei ihnen an. Ich habe sie wie verlangt nur um die Feldzeilen ergänzt; bitte einmal
formatieren lassen (oder mir freigeben). Neue Literale dieser vier Typen, die andere Blöcke inzwischen anlegen,
brauchen dieselbe Zeile.

## R2 (S-POLICY)

1. `native/src/share/core/profiles.rs`, `struct ShareProfiles` + `impl Default`: Feld
   `#[serde(default)] pub direct_request_policy: DirectRequestPolicy` (`use super::types::DirectRequestPolicy`),
   Standard `DirectRequestPolicy::Ask`. Grund FC5: Anfragen neuer Geräte warten auf Zustimmung; der Daemon
   (S-REVOKE, `ipc_host_direct_events.rs`) nimmt nur bei `AutoAccept` automatisch an. Auch Altprofile `Ask`
   (S19: automatisches Annehmen war nie bewusst gewählt).
2. `native/src/share/os/shared/profile_edits.rs`, `merge_room`: Sperren/Entsperren eines Mitglieds über
   `RoomProfile::set_member_blocked(device_id, blocked, now)` statt direktem `blocked = …` + Exec-Abschalten
   (B15: Sperre schaltet „Neue Mitglieder bestätigen“ ein; Entsperren lässt wartende Mitglieder zu); `policy`
   übernehmen, wenn `edited.policy != before.policy`. `merge_contact`: `relation.share_back` übernehmen, wenn
   geändert (`relation.signed_presence` gehört dem Worker).
3. Code-Hinzufügen (`add_direct_from_code_persisted`, `add_direct_from_code_with`, Desktop/CLI/Android): Wahl
   „Auch meine Freigaben für dieses Gerät öffnen“ (Standard aus) → `DirectContact.relation.share_back`.
4. „Darf schreiben“: Direkt `ShareProfiles::set_direct_grant_write(device_id, write, now)`, Raum
   `RoomPolicy.members_may_write`; „Neue Mitglieder bestätigen“ = `RoomPolicy.confirm_new_members`. Raum-UI
   zeigt wartende Mitglieder (`relation.admission == Pending`) mit „Zulassen“ (`admit_member`) und
   „Blockieren“ (`set_member_blocked(.., true, ..)`), Hinweis „Raum neu anlegen empfohlen“, wenn
   `requires_member_confirmation()`.

## R3 (S-SIGNAL)

1. `native/src/share/core/discovery_exchange_port_impl.rs`, Zustand `ConnectorAwaitingPublisherBundle`,
   `persist_direct(&peer, PairingOrigin::UserPairing)`: auf `PairingOrigin::UserPairingOneWay`, außer der
   Nutzer wählte beim Verbinden „Auch meine Freigaben für dieses Gerät öffnen“ (dann `UserPairing`); die Wahl
   kommt mit dem Verbinden-Befehl (Desktop, `se`, Android). Anbieter-Seite (`PublisherAwaitingKe3`) bleibt
   `UserPairing` (das Angebot ist die Zustimmung). Grund FC1/S21: Gegenseitigkeit nur auf ausdrückliche Wahl.
2. `discovery_relation_store.rs` `persist_room`: `policy: RoomPolicy::new_room()` (R1).
3. Präsenz-Nonce (B03): trägt künftig `"<Zufall>.ps1.<Signatur>"` (≤ 128 Byte); bitte keine engere Prüfung
   als heute (≤ 256 Byte, keine Steuerzeichen) einführen.

## R4 (H-DISPATCH, beim Start von Welle 2)

1. `native/src/share/core/session.rs`: `IncomingSession::authorize`/`authorize_state` liefern
   `relation_rights::SessionAuthorization`; Direkt über `DirectGrant::authorizes_session` + Fingerprint + Proof →
   `SessionAuthorization::direct(state.default_direct_exports.clone(), grant)`; Raum-Mitgliedssuche zusätzlich
   `RoomMember::is_admitted()` → `SessionAuthorization::room(room)`. Schreibende Anfragen nur bei
   `may_write && root.access.allows_write()`.
2. `authorization_policy.rs`/`configuration_runtime.rs`: `configuration_changed` durch
   `relation_rights::authorization_restrictions(&state, &candidate)` ersetzen; leer → keine Epoche, keine
   Invalidierung; `everything_reason()` → wie bisher global; sonst eingegrenzt über `RestrictionSet::affects`.
   `exec_grant_runtime::apply_configuration_transition` bei jeder übernommenen Änderung weiter aufrufen (Epoche
   unverändert, wenn die Menge leer ist).
3. Zur Kenntnis: `peer_endpoint_source.rs` und `service.rs` prüfen `blocked`; durch die Invariante
   `Pending ⇒ blocked` sind wartende Raum-Mitglieder dort schon ausgeschlossen.
