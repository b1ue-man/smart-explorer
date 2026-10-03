# S-SIGNAL – Umsetzung und Abnahme

Stand: 2026-10-03. Der zugewiesene Umsetzungsscope FC2/FC3/FC4, B20/B21 ist
abgeschlossen. Bestehende Teiländerungen wurden weitergeführt; es gab keine
neue Review-/Kritiker-Runde. Die vollständige Abnahme erfolgt mit der einen
Remote-Task-Suite nach Integration der gesamten Nutzeraufgabe. A2/A3 gehören
weiter H-DISPATCH, A4 wurde laut Orchestrator durch S-REVOKE integriert.

## Meilensteinplan und Ergebnis

1. **TLS-Start und Anmeldung:** Serveroptionen, gemeinsamer Zertifikatsresolver
   für Signaling und Relay, absolute Frist einschließlich Handshake-Schreibens,
   Challenge-Signatur vor Registrierung. Öffentlicher Start ohne Zertifikat/
   Opt-in verweigert; ungültiger Ersatz behält das bisherige Zertifikat.
2. **Einträge und Routing:** bewiesene Schlüssel binden Geräte-ID und Lookup;
   Direct-/Raum-Präsenz, Quittungen und Entscheidungen prüfen den Ursprung und
   erforderlichen Besitz. Optional atomar persistierte Bindungen verfallen
   nicht und werden bei Druck nicht verdrängt. Neue Watcher/Mitglieder brauchen
   den Beziehungsnachweis; Relay-Zugriff verlangt Signaling-Registrierung.
3. **Ressourcen und Kopplung:** konfigurierbare Adress-/IPv6-Netz-/Schlüssel-
   Grenzen, unbewiesene Registrierungen zuerst verdrängen, Burst für zwei volle
   Veröffentlichungswellen, WebSocket-Liveness und ereignisgetriebene Ausgabe.
   PIN-Angebote maximal 30 Minuten, einmalige Kopplung, fünf Fehlversuche,
   schwache PIN nur Opt-in, leere PIN immer verboten. Fehler nach Persistierung
   bewahren das Ergebnis einschließlich fehlerhafter finaler Commit-Pakete.
4. **Kompatibilität und Bedienung:** sichere neue Servereingaben, Altwerte
   bleiben TCP mit sichtbarer Warnung, kein TLS-Klartext-Fallback, Pins,
   Nachrichtengrenzen, Desktop/CLI/Android-Vertrag und Betriebsdokumentation.
   Erwartete Integrationssignale für die spätere Gesamt-Suite sind unten benannt.

Recherchebasis: `docs/refs/share-server-tls-auth.md`, recherche E8/E9 und Spec
FC2–FC4. Die zweite Gap-Prüfung fand die fehlende Servereinbindung sowie die
Endpoint-/Daemon-Grenzen A2–A4; keine neue Protokoll- oder Bibliotheksauswahl.
Zusätzliche primäre API-Belege und der Android-Vertrag stehen in
`api-delta/S-SIGNAL.md`. A1 wurde einschließlich Tokio-Features vom Orchestrator
übernommen. Keine Manifest-/Lock-Datei wurde von diesem Subagenten geändert.

## Fundzuordnung

| Plan/Funde | Konkrete Änderung und betroffene Grenze |
| --- | --- |
| FC2: S20/S51 | `discovery_pin`, `discovery_offer_guard`, Runtime und Bedienwege: sechs zufällige Ziffern, Stärke-Opt-in, maximal 30 Minuten, fünf Antworten ohne bewiesene PIN beenden das Angebot, nach bewiesener PIN keine weiteren Starts. Leere PIN ist auch an der Crypto-Port-Grenze verboten. |
| FC2: S24 | `discovery_signal_outcome` und `discovery_exchange_port_*`: bereits persistierte Ergebnisse bei Cancel/Timeout/Offline, vorzeitigem Completed oder ungültigem finalem Commit bleiben als „gekoppelt – Bestätigung fehlt“ sichtbar. Android-Widerruf entfernt den Hinweis erst nach erfolgreichem Entfernen; Desktop liegt bei S-REVOKE. |
| FC2: S50 | `discovery_state`: ein aktiver Start je proven Schlüssel/Angebot, 12 Starts/Minute je Verbinder/Angebot; Legacy nach Quelle, bei ausdrücklich vertrauenswürdigem Proxy nach Verbindung. Ein Verbinder verbraucht nicht das Budget anderer. |
| FC3: S01/S02/S27, B21 | `SignalServerConfig`, Desktop/CLI/Android und `migrate_server_file`: neue nackte Adresse ist WSS auf 51820, Klartext nur Opt-in; gespeicherte nackte Adresse bleibt TCP. Alte Mischlisten werden nicht umgedeutet und fallen unter TLS nie auf Klartext zurück. |
| FC3: S03/S18 | Relay-Ableitung über nächsten Port bzw. WebSocket-Ursprung; HTTPS unter TLS, HTTP nur mit derselben Erlaubnis, auch für Overrides. Peer-Relay-Filter A3 bleibt bei H-DISPATCH. |
| FC3: S13/S56 | Hello sendet leere LAN-/Namens-/Fingerprint-Felder bei kompatibler Feldform; WSS/WS-Frames und Nachrichten höchstens 256 KiB, Client-Drain ungefähr 4 MiB. |
| FC3/FC4: S43 | `config`, `server_tls`, Signaling-TLS und Relay-TLS: Dateipaar prüfen, Resolver teilen, neue Handshakes laden gültige Änderungen; öffentlicher Klartext benötigt Opt-in. Relay-Pin-Einbau A2 bleibt bei H-DISPATCH. |
| FC4: S44/S45/S49, S46-Serverteil | frische Schlüssel-Challenge vor Registrierung; IDs und Lookup-Besitzer an proven Schlüssel binden, Präsenz-/Entscheidungsursprung prüfen, Watch-/Raum-Nachweise; Besitzerbindung auch beim Neustart mit Zustandsdatei. Signed Legacy-Entscheidungspräsenz nutzt den fertigen S-REVOKE-Helfer (B03). |
| FC4: S47/S25/S08/S53/S54/S57 | Adresse `/64` plus `/56`, proven-Key-Limits und konfigurierbare Grenzen; Unterdruck-Verdrängung unbewiesener Registrierungen; passende Bursts; stille aktive WS/TCP schließen nach 60 s; WS wartet auf Readiness/Notify statt 500-ms-Polling. |
| FC4: S48, B20 | Relay akzeptiert nur beim Signaling registrierte Endpoint-Schlüssel, auch Legacy-Hello-Schlüssel nach Iroh-Besitznachweis; begrenzte Reconnect-Schonfrist; trusted Proxy delegiert nur seine Quellgrenzen, globale Grenzen bleiben. |

## Entscheidungen und bewahrtes Verhalten

- Neue Eingaben und gespeicherte Werte sind getrennte Lesarten. Bestehende
  nackte TCP-Endpunkte bekommen keine neue Bedeutung; aktive TLS-Endpunkte
  schließen Klartext-Rückfall aus, auch bei Zertifikatsfehlern.
- Strikter Betrieb ist `--require-key-login`. Kompatibilitätsbetrieb lässt alte
  Clients auf ungebundenen Einträgen und in gemischten Räumen arbeiten; alte
  Watcher können keinen Beweis liefern und bleiben deshalb ausdrücklich ohne
  Nachweis zugelassen. Sie können bewiesene Besitzer nicht übernehmen.
- Ohne `--state-file` ist Besitzerbindung nur im Speicher und wird gewarnt.
  Mit Datei bleibt sie dauerhaft; volle Tabellen und Schreibfehler verweigern
  neue Besitzer statt bestehende Besitzer zu vergessen.
- Server-Anmeldung und Peer-Dateiberechtigung bleiben unterschiedliche Grenzen.
  Der Server bekommt weder PIN noch Beziehungsgeheimnis; er prüft Nachweise und
  routet, die Endpoints prüfen weiter ihre Signaturen und eigentlichen Rechte.
- Trusted Proxy erhält keine Vertrauensstellung über Header. Die erlaubten
  Backend-IP-Adressen delegieren Quellbegrenzung an den Proxy, globale Grenzen
  und proven-Key-Grenzen bleiben aktiv; Legacy-Pairing kollabiert dort nicht
  alle Geräte auf eine einzige Backend-Adresse.
- Abbruch, Idle-Keepalive, Teil-TCP-Zeilen, Wiederverbindung, Wiederveröffentlichung,
  Besitzerwechsel derselben Verbindung/Schlüssels und gemischte Versionswege
  bleiben erhalten. Ein persistiertes Ergebnis wird auch bei späterem
  Crypto-Paketfehler genau einmal an die Runtime übergeben.
- Übergebene Raumdaten lassen sich nicht zurückholen. Der Hinweis benennt diese
  Grenze; Kontakt-/Rauminstallation kann über die bestehenden Entfernenwege
  widerrufen werden, und ein fehlgeschlagener Widerruf verliert den Hinweis nicht.

## Konkrete Abnahmesignale für die eine Remote-Task-Suite

Es wurden keine Tests, Server, Compiler, Cargo-/rustfmt-Aufrufe, Installationen,
Commits oder Pushes lokal ausgeführt. Die folgenden Quellen enthalten die
gezielten Prüfsignale; sie sind noch nicht als erfolgreich ausgeführt behauptet.

| Frage / Erwartung | Datei / Symbol |
| --- | --- |
| Öffentlicher Start verlangt TLS/Opt-in; Loopback-Proxy und konfigurierte positive Grenzen funktionieren | `share-server/src/config.rs::tests::review_task_public_listener_requires_tls_or_explicit_plaintext`, `review_task_limits_are_configurable_and_network_defaults_follow_source_caps` |
| Echter WSS-Client meldet sich proven an, wartende Ausgabe kommt ohne Heartbeat, ungültiger Zertifikatsersatz behält alten Schlüssel und gültiger Ersatz wird genutzt | `share-server/src/signal_security_transport_tests.rs::review_task_wss_login_output_wake_and_certificate_reload` |
| Klartext registriert bei verbotener Erlaubnis nichts; anderer Signer und Replay/Mehrfachantwort werden verweigert; Legacy-Hello bleibt möglich | dieselbe Datei: `review_task_plaintext_is_refused_before_registration`, `review_task_raw_tcp_key_login_rejects_another_signer`, `review_task_login_challenge_is_connection_bound_and_single_use` |
| Ausgabewake hängt weder an Socket-Eingang noch am Liveness-Timer | `share-server/src/websocket_socket.rs::tests::review_task_queued_output_wakes_readiness_without_socket_input` |
| Fremde Geräte-ID/Lookup-Übernahme, unberechtigte Entscheidung und fremdes Abmelden scheitern; Zustand bleibt nach Neustart gebunden | `share-server/src/signal_security_state_tests.rs::review_task_proven_device_binding_displaces_impostor_and_survives_cleanup`, `review_task_bound_lookup_requires_access_and_only_its_owner_mutates_it`, `review_task_device_and_lookup_bindings_survive_state_file_restart` |
| Strikter Modus/Key-Cap; Räume prüfen Ursprung und Nachweispartition, Legacy bleibt kompatibel | dieselbe Datei: `review_task_strict_mode_and_key_caps_cannot_be_bypassed_by_device_ids`, `review_task_room_partitions_and_member_origin_preserve_mixed_clients` |
| Wiederholte Starts eines Schlüssels blockieren keinen anderen; proxied Legacy-Verbindungen bleiben getrennt | dieselbe Datei: `review_task_pin_start_budget_is_per_connector_key_and_preserves_other_access` |
| Aktive WS-Zombies schließen, auch nach Idle-Wakeup; bestehender Idle-/Teilzeilen-Vertrag bleibt | `share-server/src/idle_transport_tests.rs::review_task_active_websocket_has_an_inbound_deadline` plus die vorhandenen `android_background_task_*`-Liveness-Fälle |
| Gemeinsame Client-/Server-Digests und Nachweisvektoren stimmen | `signal_handshake.rs::review_task_login_digest_matches_the_server_vector`, `share-server/src/login.rs::review_task_login_digest_matches_the_app_vector`, `signal_publish.rs` und `share-server/src/access.rs` bestehende `review_task_*`-Vektoren |
| Sichere neue Eingaben, Alt-TCP, Pins/kein Downgrade, IPv6 und Nachrichtengrenzen | `signal_connection_config_tests.rs`, `signal_connection_tls_tests.rs`, `share-server/src/limits.rs`, `websocket_read_limit.rs`, `rate_limits.rs`, vorhandene Mischversion-/Transport-Cleanup-Fälle |
| Bereits installierte Kopplung bleibt bei ungültigem finalem Commit oder vorzeitigem Completed reportierbar; leere Port-PIN verboten | `native/src/share/core/discovery_exchange_port_tests.rs::review_task_persisted_pairing_stays_reportable_after_malformed_commit`, `review_task_premature_completion_preserves_the_installed_pairing_outcome`, `review_task_empty_pin_is_rejected_by_publishing_and_connecting_ports` |
| Fünf Fehlversuche und erste Kopplung beenden das Angebot; zufällige PIN, Schwäche-Opt-in und Leer-Verbot | `discovery_offer_guard.rs`, `discovery_pin.rs`, `discovery_exchange_port_helpers.rs`, `cli/share/discoverable_input.rs` vorhandene/neue `review_task_*`-Fälle |
| OPAQUE-Primitive behält beliebige Bytes, ohne daraus Anwendungs-Erlaubnis abzuleiten | `share_remote_discovery_task_tests.rs::share_remote_task_pake_primitive_accepts_empty_and_zero_bytes` (umbenannt, Payload unverändert) |
| TLS-Relay mit Pin und Peer-HTTP-Filter funktionieren durch den echten Endpoint-/Session-Weg | A2/A3-Abnahme nach H-DISPATCH-Integration; WSS-Prüfung allein beweist diese beiden Grenzen nicht |

## Statischer Self-Review und Grenzen

Tree-sitter-Rust liest ausschließlich die zugewiesenen Rust-Dateien, eigenen
neuen Dateien und erlaubten Registrierungen. Keine neue/geänderte Rust-Syntax
hat einen Parserfehler. Eine bereits im HEAD vorhandene Grammatik-Meldung in
`vendor/iroh-relay-1.0.0/src/server/http_server.rs:59` betrifft `dyn 'static + …`
und ist keine neue Quelländerung. `git diff --check` ist für die zugewiesene
Fläche sauber. Alle substantiell geänderten Native-Featuredateien bleiben unter 500 Zeilen
und 50 KiB; bestehende Modulregister erhielten nur eigene additive Einträge.
Eigene neue Dateien liegen direkt neben ihrem Feature.

Parser/Whitespace-Prüfung ersetzen keine Typprüfung oder Verhaltensabnahme.
A2/A3 sind beim Orchestrator/H-DISPATCH konkret eingetragen. A4 ist nach
Orchestrator-Rückmeldung erledigt; diese außerhalb des Scopes liegende Datei
wurde nicht nachgelesen. Der FC5-Dokumentabschnitt bleibt bis zur anschließenden
Orchestrator-Integration dessen Eigentum. Der Hauptagent übernimmt Root-Graph,
Suite-Integration (auch neue CLI-TLS-Defaults in alten E2E-Fixtures), Commits,
Push, Remote-CI und die eine terminale Release-Transaktion.

## Dateien erstellt / geändert

### In dieser Fortsetzung erstellt

- `native/src/share/core/signal_connection_address.rs`
- `native/src/share/core/discovery_exchange_port_tests.rs`
- `share-server/src/config.rs`
- `share-server/src/direct_presence.rs`
- `share-server/src/hello_session.rs`
- `share-server/src/rooms.rs`
- `share-server/src/server_tls.rs`
- `share-server/src/signal_security_state_tests.rs`
- `share-server/src/signal_security_transport_tests.rs`
- `share-server/src/signal_stream.rs`
- `share-server/src/websocket_socket.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S-SIGNAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-SIGNAL.md`

### Bestehende Teiländerungen weitergeführt / im Block geändert

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/S-SIGNAL.md`
- `docs/SHARE_SERVER.md`
- `native/src/app/core/menus_settings.rs`
- `native/src/app/core/share_discovery_publish_ui.rs`
- `native/src/app/core/share_discovery_ui.rs`
- `native/src/cli/share.rs`
- `native/src/cli/share/discoverable.rs`
- `native/src/cli/share/discoverable_input.rs`
- `native/src/cli/share/discoverable_output.rs`
- `native/src/mobile/os/shared/domains/share_settings.rs`
- `native/src/share/core/discovery_exchange_port_helpers.rs`
- `native/src/share/core/discovery_exchange_port_impl.rs`
- `native/src/share/core/discovery_exchange_port_state.rs`
- `native/src/share/core/discovery_offer_book.rs`
- `native/src/share/core/discovery_offer_guard.rs`
- `native/src/share/core/discovery_pin.rs`
- `native/src/share/core/discovery_signal_cancellation.rs`
- `native/src/share/core/discovery_signal_commands.rs`
- `native/src/share/core/discovery_signal_dispatch.rs`
- `native/src/share/core/discovery_signal_exchange.rs`
- `native/src/share/core/discovery_signal_maintenance.rs`
- `native/src/share/core/discovery_signal_outcome.rs`
- `native/src/share/core/discovery_signal_port.rs`
- `native/src/share/core/discovery_signal_state.rs`
- `native/src/share/core/discovery_signal_types.rs`
- `native/src/share/core/endpoint_routes.rs`
- `native/src/share/core/share_remote_discovery_task_tests.rs`
- `native/src/share/core/signal_connected.rs`
- `native/src/share/core/signal_connection.rs`
- `native/src/share/core/signal_connection_config.rs`
- `native/src/share/core/signal_connection_config_tests.rs`
- `native/src/share/core/signal_connection_tls.rs`
- `native/src/share/core/signal_connection_tls_tests.rs`
- `native/src/share/core/signal_connector.rs`
- `native/src/share/core/signal_handshake.rs`
- `native/src/share/core/signal_publish.rs`
- `native/src/share/core/signal_subscriptions.rs`
- `native/src/share/core/signal_worker.rs`
- `native/src/share/core/signal_worker_tests.rs`
- `native/src/share/os/shared/discovery_events.rs`
- `native/src/share/os/shared/discovery_pairing_ui.rs`
- `native/src/share/os/shared/discovery_state.rs`
- `native/src/share/os/shared/transport_options.rs`
- `share-server/src/access.rs`
- `share-server/src/bindings.rs`
- `share-server/src/discovery.rs`
- `share-server/src/discovery_state.rs`
- `share-server/src/idle.rs`
- `share-server/src/idle_outbox_tests.rs`
- `share-server/src/idle_transport_tests.rs`
- `share-server/src/limits.rs`
- `share-server/src/login.rs`
- `share-server/src/main.rs`
- `share-server/src/main_tests.rs`
- `share-server/src/mixed_version_tests.rs`
- `share-server/src/protocol.rs`
- `share-server/src/rate_limits.rs`
- `share-server/src/relay.rs`
- `share-server/src/relay_access.rs`
- `share-server/src/signal_session.rs`
- `share-server/src/state.rs`
- `share-server/src/tracked_direct.rs`
- `share-server/src/tracked_direct_tests.rs`
- `share-server/src/transport.rs`
- `share-server/src/transport_serve.rs`
- `share-server/src/websocket_read_limit.rs`
- `share-server/src/writer.rs`
- `vendor/iroh-relay-1.0.0/src/server.rs`
- `vendor/iroh-relay-1.0.0/src/server/http_server.rs`
- `vendor/iroh-relay-1.0.0/src/server/testing.rs`

### Nur eigene additive Registrierungen

- `native/src/share/mod.rs`
- `native/src/mobile/os/shared/domains/mod.rs`

S24-Desktop-Widerruf in `native/src/app/core/share_discovery_events.rs` und V5-
Ergebnis-/Rechteanpassungen in `discovery_relation_store.rs` sind gemeinsame
Integrationen mit S-REVOKE; keine fremden Registrierungen wurden geändert.

## Gelesene Dateien

### Kontext / Primärreferenzen / Manifeste

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/SHARE_SERVER.md`
- `docs/lesungen/INDEX.md`
- `docs/refs/INDEX.md`
- `docs/refs/share-server-tls-auth.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/s-signal.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sicherheit.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-analyse.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/fortsetzung.md`
- `native/Cargo.toml`
- `share-server/Cargo.toml`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/S-SIGNAL.md`
- `/root/.codex/skills/arbeitsweise/SKILL.md`
- `/root/.codex/skills/graphify/SKILL.md`
- `graphify-out/graph.json` (scoped query; Root-Auffrischung bleibt beim Orchestrator)
- `native/src/share/core/signal_presence.rs` nur zum ausdrücklich freigegebenen `build_direct_decision_presence`-Helfer

### Statisch gelesene/geparste zugewiesene Rust-Fläche

Die Parserliste bedeutet Dateizugriff innerhalb des Scopes; sie behauptet keine
zusätzliche inhaltliche Review-Runde anderer Blöcke. Eigene Quell-Self-Reviews
folgen den oben benannten Vertragsgrenzen.

- `native/src/analytics/mod.rs`
- `native/src/app/core/menus_settings.rs`
- `native/src/app/core/share_discovery_events.rs`
- `native/src/app/core/share_discovery_publish_ui.rs`
- `native/src/app/core/share_discovery_state.rs`
- `native/src/app/core/share_discovery_ui.rs`
- `native/src/bisync/mod.rs`
- `native/src/cli/share.rs`
- `native/src/cli/share/discoverable.rs`
- `native/src/cli/share/discoverable_input.rs`
- `native/src/cli/share/discoverable_output.rs`
- `native/src/daemon/mod.rs`
- `native/src/lib.rs`
- `native/src/mobile/os/shared/domains/mod.rs`
- `native/src/mobile/os/shared/domains/share_settings.rs`
- `native/src/share/core/discovery_bundle.rs`
- `native/src/share/core/discovery_domain.rs`
- `native/src/share/core/discovery_exchange.rs`
- `native/src/share/core/discovery_exchange_port_helpers.rs`
- `native/src/share/core/discovery_exchange_port_impl.rs`
- `native/src/share/core/discovery_exchange_port_state.rs`
- `native/src/share/core/discovery_exchange_port_tests.rs`
- `native/src/share/core/discovery_offer_book.rs`
- `native/src/share/core/discovery_offer_guard.rs`
- `native/src/share/core/discovery_pake.rs`
- `native/src/share/core/discovery_pin.rs`
- `native/src/share/core/discovery_relation_store.rs`
- `native/src/share/core/discovery_signal_cancellation.rs`
- `native/src/share/core/discovery_signal_commands.rs`
- `native/src/share/core/discovery_signal_dispatch.rs`
- `native/src/share/core/discovery_signal_exchange.rs`
- `native/src/share/core/discovery_signal_maintenance.rs`
- `native/src/share/core/discovery_signal_offline.rs`
- `native/src/share/core/discovery_signal_outcome.rs`
- `native/src/share/core/discovery_signal_persisted.rs`
- `native/src/share/core/discovery_signal_port.rs`
- `native/src/share/core/discovery_signal_publication.rs`
- `native/src/share/core/discovery_signal_state.rs`
- `native/src/share/core/discovery_signal_types.rs`
- `native/src/share/core/discovery_signal_validation.rs`
- `native/src/share/core/discovery_signal_wire.rs`
- `native/src/share/core/discovery_wire.rs`
- `native/src/share/core/endpoint_routes.rs`
- `native/src/share/core/service.rs`
- `native/src/share/core/share_remote_discovery_task_tests.rs`
- `native/src/share/core/signal_connected.rs`
- `native/src/share/core/signal_connection.rs`
- `native/src/share/core/signal_connection_address.rs`
- `native/src/share/core/signal_connection_config.rs`
- `native/src/share/core/signal_connection_config_tests.rs`
- `native/src/share/core/signal_connection_tls.rs`
- `native/src/share/core/signal_connection_tls_tests.rs`
- `native/src/share/core/signal_connector.rs`
- `native/src/share/core/signal_handshake.rs`
- `native/src/share/core/signal_idle.rs`
- `native/src/share/core/signal_power.rs`
- `native/src/share/core/signal_publish.rs`
- `native/src/share/core/signal_readiness.rs`
- `native/src/share/core/signal_schedule.rs`
- `native/src/share/core/signal_session.rs`
- `native/src/share/core/signal_subscriptions.rs`
- `native/src/share/core/signal_worker.rs`
- `native/src/share/core/signal_worker_test_server.rs`
- `native/src/share/core/signal_worker_tests.rs`
- `native/src/share/mod.rs`
- `native/src/share/os/shared/discovery_events.rs`
- `native/src/share/os/shared/discovery_pairing_ui.rs`
- `native/src/share/os/shared/discovery_relation_store_adapter.rs`
- `native/src/share/os/shared/discovery_retention.rs`
- `native/src/share/os/shared/discovery_state.rs`
- `native/src/share/os/shared/transport_options.rs`
- `native/src/syncjobs/mod.rs`
- `native/src/vfs/mod.rs`
- `share-server/src/access.rs`
- `share-server/src/bindings.rs`
- `share-server/src/config.rs`
- `share-server/src/direct_messages.rs`
- `share-server/src/direct_presence.rs`
- `share-server/src/direct_validation.rs`
- `share-server/src/discovery.rs`
- `share-server/src/discovery_state.rs`
- `share-server/src/hello_session.rs`
- `share-server/src/idle.rs`
- `share-server/src/idle_outbox.rs`
- `share-server/src/idle_outbox_tests.rs`
- `share-server/src/idle_transport_tests.rs`
- `share-server/src/limits.rs`
- `share-server/src/line.rs`
- `share-server/src/login.rs`
- `share-server/src/main.rs`
- `share-server/src/main_tests.rs`
- `share-server/src/mixed_version_tests.rs`
- `share-server/src/protocol.rs`
- `share-server/src/rate_limits.rs`
- `share-server/src/registration_guard.rs`
- `share-server/src/relay.rs`
- `share-server/src/relay_access.rs`
- `share-server/src/resource_limits_tests.rs`
- `share-server/src/rooms.rs`
- `share-server/src/server_tls.rs`
- `share-server/src/share_remote_task_tests.rs`
- `share-server/src/share_remote_wire_task_tests.rs`
- `share-server/src/signal_security_state_tests.rs`
- `share-server/src/signal_security_transport_tests.rs`
- `share-server/src/signal_session.rs`
- `share-server/src/signal_stream.rs`
- `share-server/src/state.rs`
- `share-server/src/state_transition_tests.rs`
- `share-server/src/tracked_direct.rs`
- `share-server/src/tracked_direct_tests.rs`
- `share-server/src/transport.rs`
- `share-server/src/transport_cleanup_tests.rs`
- `share-server/src/transport_serve.rs`
- `share-server/src/websocket_read_limit.rs`
- `share-server/src/websocket_socket.rs`
- `share-server/src/writer.rs`
- `share-server/src/writer_idle.rs`
- `vendor/iroh-relay-1.0.0/src/server.rs`
- `vendor/iroh-relay-1.0.0/src/server/accept_rate_limits.rs`
- `vendor/iroh-relay-1.0.0/src/server/client.rs`
- `vendor/iroh-relay-1.0.0/src/server/clients.rs`
- `vendor/iroh-relay-1.0.0/src/server/connection_limits.rs`
- `vendor/iroh-relay-1.0.0/src/server/connection_source.rs`
- `vendor/iroh-relay-1.0.0/src/server/http_server.rs`
- `vendor/iroh-relay-1.0.0/src/server/metrics.rs`
- `vendor/iroh-relay-1.0.0/src/server/queue_budget.rs`
- `vendor/iroh-relay-1.0.0/src/server/resolver.rs`
- `vendor/iroh-relay-1.0.0/src/server/streams.rs`
- `vendor/iroh-relay-1.0.0/src/server/testing.rs`

Die eigenen drei Berichte wurden gelesen und aktualisiert. Keine außerhalb des
Scopes liegenden Endpoint-, Session-, Daemon-, Kotlin- oder Suite-Dateien wurden
erkundet. Offene konkrete Anfragen: A2/A3; Einzelheiten in `anfragen/S-SIGNAL.md`.
