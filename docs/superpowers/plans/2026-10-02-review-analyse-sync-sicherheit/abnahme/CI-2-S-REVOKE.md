# CI-2-S-REVOKE: gebundene NEW→OLD-Lifecycleprojektion

Stand: 2026-10-03. Die freigegebene Implementierung der belegten
NEW→OLD-Grenze ist abgeschlossen; die gemeinsame Remote-Abnahme steht aus.
Grundlage: Run `37150409255`, Kandidat
`ac9b475ff18f6320bedd408c5a03c091710ad01c`, übernommene
Formatierung laut CI-Fixplan `7b9424dd`. Kein neues Projekt-Review.

## Befund und freigegebener Vertrag

Der erhaltene Lauf unter
`/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h` belegt die
echte Altannahme mit den von Alt-`share status` ausgegebenen
Fingerprint-/Gerätewerten. Das alte Profil hat den Grant Accepted,
während Kontakt und ausgehender Request im neuen Profil Pending bleiben,
mit vollständigen Zielpins, LegacyForwarded und ohne signierten Receipt
oder signierte Entscheidung. Der Daemon meldet konkret:
`Legacy-Entscheidung verworfen: der Server spricht tracked_direct`.

Der alte Presence-HMAC bindet weder das äußere `accepted`-Bit noch den
äußeren `requester_device_id`. Die gewöhnliche alte Nonce enthält keine
Entscheidungsbindung. [Spec](../spec.md), Grundsatz 6 und FC5/S26/S30,
sowie [S-REVOKE](S-REVOKE.md) erlauben Unsigned nur im eingeschränkten
Pending-Pfad. Die ursprüngliche Suite verlangte Accepted vor einem
authentifizierten Peerbeleg. Root hat diesen Konflikt im
[CI-Fixplan](../ci-behavior-fixes.md#gebundene-altversion-kompatibilität)
vor Umsetzung aufgelöst: Die ungebundene Antwort bleibt verworfen;
der vorhandene ausdrückliche Open verwendet eine gebundene Read-only-Probe.

## Umsetzung und Fundzuordnung

Die freigegebenen Meilensteine sind umgesetzt:

1. `legacy_probe.rs` begrenzt die Probe auf einen bestehenden
   Pending-Kontakt mit genau einer eigenen ausgehenden
   LegacyForwarded-Anfrage. `service.rs::probe_backend_for_target`
   verbindet diese Probe mit dem vorhandenen ausdrücklichen Open.
2. `legacy_probe_persist.rs` bindet die tatsächliche Peerzulassung an
   die aktuelle lokale Identität, den aktuellen Authsnapshot und die
   kanonische CAS-/Entzugsgrenze. Nur der bestehende ausgehende Kontakt
   wird bestätigt.
3. Der vorhandene Mixed-Version-Einstieg prüft Verweigerung vor
   Altannahme, ausdrückliche Probe danach und unveränderte
   Post-Accept-/OLD→NEW-/Reject-/Restart-/Widerrufsassertions.

Der feste Snapshot nutzt
`PeerBackend::new(endpoint, identity, node).probe_root()`.
Der vorhandene Iroh-Handshake adressiert den exakten gepinnten Knoten und
sendet PeerHello v3 mit Relation, lokaler Identität und Session-HMAC.
Erst die erfolgreiche Read-only-Rootoperation ist der Zulassungsbeleg.
Der zurückgegebene Backend bleibt der normale Live-Backend mit aktuellen
Rechte-/Pinchecks.

Die Prüfung verlangt denselben Kontakt, Lookup und Secure-store-Secret;
volle Geräte-/Schlüssel-/Knoten-/Fingerprintpins, lokal abgeleiteten
Fingerprint und `node_id == public_key`; denselben vollständigen
SignedRequest/Ledgerstand, Pending mit Revision null und keine moderne
Receipt/Decision; aktuelle Requestsignatur/HMAC und Frist sowie aktuelle
gebundene Presence-HMAC und Frist. Lokale Identitätsgeneration und
Direkt-Secret müssen unverändert sein; Service und Direct müssen laufen.
Bekannte Gerätesignaturen, unmittelbare Downgrade-Marker und aktuelle
Denial-/Entfernungs-/Tombstonepolitik verhindern die Probe bzw. Bestätigung.

Die vorhandenen `direct_auto_accept_denied`- und Signaturmarker-Helfer
werden wiederverwendet. Sowohl aktuelle Runtimepolitik als auch der
neu geladene vollständige Profilstand werden geprüft. Keine neue
Denialheuristik und kein allgemeiner Fallback nach Serverfähigkeit.

Vor Netzwerk-I/O prüft der OS-Adapter die wirkliche lokale Identität und
den aktuellen gespeicherten Profilstand. Nach I/O prüft er die
Identitätsgeneration erneut, nimmt erst dann den vorhandenen
opportunistischen Persistenz-Permit und hält den aktuellen Authsnapshot
durch die begrenzte CAS-Transaktion. Jeder Commitversuch prüft Stop,
Kontakt, Pins, Secret, Ledger, Fristen und Denials frisch. Konflikt,
Entfernung, Rotation, Modernisierung oder Entzug verhindern die
Bestätigung. Keine Sperre und kein Permit bleibt über Peer-I/O gehalten.

Die Mutation setzt nur Accepted-/accepted_at-/Key- und zugehörige
Runtimefelder des bestehenden ausgehenden Kontakts. Keine Grants,
Write-/Exec-Policies, signierten Entscheidungen oder Receipts werden
erzeugt oder verändert. Der selektive Live-Refresh erhält aktuelle
Routen, Pins, monotone Signaturmarker, Replaydaten und fremde Beziehungen.
Das bestehende `RuntimeProfilesCommitted` veranlasst den kanonischen
Daemon-Reload und `ConfigureProfiles` an der vorhandenen
Restriktionsgrenze. Kein verspäteter voller Profilsnapshot wird direkt
in den Livezustand kopiert.

Beide bisherigen Legacy-Sicherheitsgates bleiben erhalten:
`signal_auth::handle_server_msg` verwirft alte Antworten bei
`tracked_direct`; `ipc_host_relation_events::apply_legacy_decision`
verwirft Kontakte mit modernem ausgehendem Ledger. S26/S30, Signatur- und
Rückstufungsschutz sowie der vollständige Legacy-Fingerprintvertrag
werden nicht gelockert. Vorhandene Transport-Opt-ins bleiben bestehen;
es entsteht keine Unsigned-Entscheidungsoption.

## API und Integration

Nur interne additive APIs:

```rust
legacy_probe::confirm_pending(
    service: &ShareService,
    target: &PeerOpenTarget,
    endpoint: &PeerEndpoint,
) -> Result<bool, String>

legacy_probe_persist::validate_before_probe(
    service: &ShareService,
    probe: &PendingLegacyProbe,
) -> Result<(), String>

legacy_probe_persist::persist(
    service: &ShareService,
    probe: &PendingLegacyProbe,
) -> Result<(), String>
```

`signal_auth::peer_signature_seen` stellt den vorhandenen unmittelbaren
Marker rein lesend bereit. `ShareService.profile_home: Option<String>`
behält die vorhandene Default-home-Konfiguration für den kanonischen
Profilzugriff; Start und Clone übernehmen sie. Der von Root benannte
bestehende `service_tests::test_service`-Konstruktor erhält ausschließlich
`profile_home: None`, ohne Assertion-/Fixture-Verhaltensänderung.

`share/mod.rs` registriert ausschließlich die beiden eigenen Module.
Keine Wire-DTOs, geänderten öffentlichen Methodensignaturen, neuen
Optionen oder weiteren Parent-Registrierungen erforderlich.

## Konkrete Remote-Abnahme

Nur `native/test-share-mixed-version-e2e.sh` innerhalb derselben
Root-eigenen vollständigen RV1-Suite. Vorgesehene Selektoren/Artefakte:

- Stage `NEW to OLD: prove an explicit probe cannot activate a pending legacy grant`:
  `new-to-old-pending-probe.txt`, `new-to-old-pending-probe.stderr`
  und `new-to-old-after-pending-probe.json`. Echte `se ls`-Verweigerung;
  Timeout/Kill zählt nicht als Verweigerung. Danach: server_queued,
  legacy_forwarded, Pending/effective Pending, Request-Receipt
  unconfirmed und Autorisierung inactive.
- Stage `NEW to OLD: accept using only values emitted by legacy share status`:
  dieselbe echte Altannahme, ohne direkte Profil-/Grantmutation
  durch den Einstieg.
- Stage `NEW to OLD: explicitly probe the pinned peer after legacy acceptance`:
  `new-to-old-explicit-probe.txt`. Der ausdrückliche `se ls`-Open
  muss tatsächlich erfolgreich sein.
- `new-to-old-accepted-lifecycle.json`: die vollständige vorhandene
  Abfrage bleibt unverändert. `decision.state=pending`,
  `effective_state=accepted`, `evidence=legacy_relation`,
  Request-Receipt unconfirmed und `authorization.active=true`
  mit `basis=legacy_contact_projection`.
- Die folgenden CLI-entdeckten Root-/Statoperationen, verlorene
  Legacy-RAM-Inbox, Neustart und manueller Retry bleiben erhalten.
  Der vollständige OLD→NEW-/Reject-/Restart-/Widerrufsteil ist unverändert.

Bestehende S-REVOKE-Selektoren bleiben maßgeblich:
`review_task_legacy_decision_nonce_binds_recipient_and_value_within_wire_limit`,
`review_task_first_signature_is_remembered_before_daemon_event` und
`review_task_late_worker_cannot_restore_withdrawn_access_or_clear_signature`.
Keine zusätzliche Testdatei oder unabhängige Testkampagne.

## Self-Review und Restgrenze

Eigene Änderungen statisch geprüft: Altannahme- und Post-Accept-Abfragen
sowie der vollständige OLD→NEW-/Reject-/Restartteil bleiben bytegleich.
Beide Legacy-Gates unverändert; der neue Markerhelper liest ausschließlich.
Neue Rustmodule haben deutlich Reserve unter 500 Zeilen/50 KiB;
`service.rs` und `signal_auth.rs` bleiben ebenfalls darunter.
Konstruktoranschluss, additive Registrierung, Lock-/I/O-Reihenfolge,
aktuelle CAS-Prüfungen und selektive Livepublication anhand der
freigegebenen Definitionen abgeglichen. `bash -n` hat ausschließlich
den vorhandenen Einstieg geparst, keine Befehle ausgeführt.

Keine offenen API-/Consumeranschlüsse innerhalb des Blocks.
Compiler-/Verhaltensbestätigung, Rootgraph, Commit/Push und derselbe
Remote-RV1-Lauf bleiben bei Root. Keine lokalen Builds, Compiler,
Tests, Formatter, Server, Installationen, Git-, CI-, Graph- oder
Releaseaktionen; keine Unteragenten. Die Git-Dokumentationskontextprüfung
liegt wegen des ausdrücklichen Workerverbots ebenfalls bei Root.

## Dateiinventar

Nur zugeteilter Scope, gezielt nachgereichte Definitionen und eigene
neue Dateien gelesen; bei `service_tests.rs` nur der zugeteilte
Konstruktoranschluss. Keine Secrets-, Token- oder Lockdateien des
erhaltenen Runnerprofils gelesen oder geändert.

### Gelesen

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-2-s-revoke.json`
- `/tmp/rv1-ci-second/s-revoke.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-behavior-fixes.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/share-server-tls-auth.md`
- `native/src/share/core/direct_ledger_projection.rs`
- `native/src/share/core/legacy_direct_request_reconciliation.rs`
- `native/src/share/core/direct_relation.rs`
- `native/test-share-mixed-version-e2e.sh`
- `native/src/share/os/shared/profile_operations.rs`
- `native/src/share/os/shared/legacy_direct_actions.rs`
- `native/src/share/core/direct_lifecycle.rs`
- `/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h/new-to-old-accepted-lifecycle.json`
- `/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h/new-to-old-initial-lifecycle.json`
- `/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h/new-to-old-legacy-accept.txt`
- `/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h/new-to-old-old-status-pending.json`
- `/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h/old-target/data/smart_explorer/share_profiles.json`
- `/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h/new-requester/data/smart_explorer/share_profiles.json`
- `native/src/share/core/profiles.rs`
- `native/src/share/core/types.rs`
- `native/src/share/os/shared/profile_store.rs`
- `native/src/share/core/profile_policy.rs`
- `/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h/new-requester/data/smart_explorer/sync/daemon.log`
- `/tmp/rv1-ci-second/linux/se-share-mixed-version.42Cw0h/old-target/data/smart_explorer/sync/daemon.log`
- `native/src/share/mod.rs`
- `native/src/share/core/signal_auth.rs`
- `native/src/app/core/share_drain.rs`
- `native/src/share/core/direct_ledger.rs`
- `native/src/share/core/signal_presence.rs`
- `native/src/daemon/os/shared/ipc_host_relation_events.rs`
- `native/src/share/core/profile_migration.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S-REVOKE.md`
- `native/src/share/core/session.rs`
- `native/src/share/core/node_sessions.rs`
- `native/src/share/core/peer_endpoint_source.rs`
- `native/src/share/core/backend.rs`
- `native/src/share/core/service.rs`
- `native/src/share/os/shared/direct_repair_store_adapter.rs`
- `native/src/share/os/shared/profile_transaction.rs`
- `native/src/share/os/shared/direct_reciprocal_persistence.rs`
- `native/src/share/os/shared/discovery_relation_store_adapter.rs`
- `native/src/daemon/os/shared/ipc_host.rs`
- `native/src/daemon/os/shared/ipc_host_events.rs`
- `native/src/share/core/legacy_direct_request.rs`
- `native/src/share/core/direct_reciprocal_transport.rs`
- `native/src/share/core/node.rs`
- `native/src/share/core/configuration_runtime.rs`
- `native/src/share/core/identity.rs`
- `native/src/share/os/shared/identity_store.rs`
- `native/src/share/core/direct_protocol.rs`
- `native/src/share/core/removed_direct_peers.rs`
- `native/src/share/core/direct_reciprocal_outgoing_gate.rs`
- `native/src/share/core/service_tests.rs`
- `native/src/share/core/legacy_probe.rs`
- `native/src/share/os/shared/legacy_probe_persist.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-S-REVOKE.md`

### Geändert

- `native/src/share/core/service.rs`
- `native/src/share/core/signal_auth.rs`
- `native/src/share/mod.rs`
- `native/test-share-mixed-version-e2e.sh`
- `native/src/share/core/service_tests.rs` — nur `profile_home: None` im Konstruktor

### Erstellt

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-S-REVOKE.md`
- `native/src/share/core/legacy_probe.rs`
- `native/src/share/os/shared/legacy_probe_persist.rs`
