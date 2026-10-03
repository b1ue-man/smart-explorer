# Anfragen von S-SIGNAL

Stand: 2026-10-03. Der eigene Umsetzungsscope ist abgeschlossen. Die offenen
Integrationen A2/A3 sind vom Orchestrator an H-DISPATCH übergeben; keine lokale
Cargo-Ausführung, Auflösung, Installation oder Build-Prüfung wurde vorgenommen.

## A1 – Manifest und bestehende Lock-Verknüpfungen: erledigt durch Orchestrator

`share-server/Cargo.toml` und `share-server/Cargo.lock` bleiben außerhalb meines
Änderungsscope. Der Orchestrator hat die erforderlichen Einträge übernommen:

```toml
[dependencies]
iroh-base = { version = "1.0.0", features = ["key"] }
rustls = { version = "0.23.41", default-features = false, features = ["ring", "std", "tls12"] }
ring = "0.17"
tokio = { version = "1", features = ["rt-multi-thread", "net", "sync", "time", "macros"] }

[dev-dependencies]
rcgen = "0.14"
```

Die Versionen/Lock-Packages waren bereits vorhanden. Der Tokio-Reaktor benötigt
explizit `net`, `sync`, `time` und `macros`; der Orchestrator meldete auch diese
Features als eingetragen (Commit `bdef9ed`). Keine weitere Abhängigkeit angefragt.

## A2 – Gepinnte Relay-Zertifikate: offen bei H-DISPATCH

- Datei/Stelle: `native/src/share/core/node.rs`, Erzeugung des Iroh-Endpoints.
- Fertige API: `NodeTransportOptions::ca_tls_config() -> Option<iroh::tls::CaTlsConfig>`
  in `native/src/share/core/endpoint_routes.rs`.
- Erforderlich: den gelieferten Trust am Endpoint-Builder anwenden. Er akzeptiert
  die Pins der aktiven TLS-Server und weiterhin regulär öffentlich gültige
  Zertifikate fremder Peer-Relays.
- Abnahmesignal: selbst signierter Share-Server samt HTTPS-Relay funktioniert mit
  passendem Pin; falscher Pin scheitert, ohne HTTP-Rückfall. Öffentlich gültige
  Peer-Relays bleiben erreichbar.
- Stand laut Orchestrator: in `integration.md` bereits H-DISPATCH zugeordnet,
  noch nicht als erledigt gemeldet. Die Datei wurde von mir weder gelesen noch geändert.

## A3 – Peer-Relay-URLs: offen bei H-DISPATCH

- Datei/Stelle: `native/src/share/core/session.rs`, Erzeugung der Endpoint-Adresse
  aus Peer-Präsenz.
- Fertige API: `NodeTransportOptions::accepts_relay_url(&str) -> bool` in
  `native/src/share/core/endpoint_routes.rs`.
- Erforderlich: jede fremde Relay-URL durch diese Regel führen; HTTPS bleibt
  erlaubt, HTTP benötigt die gespeicherte Klartext-Erlaubnis. Direkte IP-/LAN-
  Kandidaten bleiben erhalten.
- Abnahmesignal: HTTP aus einer Peer-Präsenz wird bei TLS-Konfiguration verworfen;
  dieselbe URL funktioniert bei ausdrücklich erlaubtem, gespeichertem Klartext.
- Stand laut Orchestrator: H-DISPATCH zugeordnet, noch nicht als erledigt gemeldet.
  Die Datei wurde von mir weder gelesen noch geändert.

## A4 – Daemon-Migration alter Server-Dateien: erledigt durch S-REVOKE

Der Orchestrator bestätigt den Einbau im tatsächlichen Lesepfad
`ipc_host::load_share_server`: nach regulärer Datei-/16-KiB-Prüfung
`crate::share::migrate_server_file(&path)` mit Fehlerfortleitung. Die Funktion
und ihr Export sind im S-SIGNAL-Scope fertig; nackte gespeicherte Adressen
werden atomar als `tcp://host:port` kanonisiert und behalten ihre Bedeutung.
Die außerhalb meines Scopes liegende Daemon-Datei wurde nicht gelesen.

## Gemeinsame Abschlussarbeit

Desktop-S24 `share_discovery_events::revoke_unconfirmed_pairing` ist gezielt
S-REVOKE zugeordnet: den Hinweis erst nach erfolgreichem Entfernen verwerfen.
Android `share.resolvePairing` erfüllt diese Regel im S-SIGNAL-Scope.
Der Orchestrator aktualisiert nach meinem Dokumentabschluss den FC5-Abschnitt
von `docs/SHARE_SERVER.md` anhand der zuständigen Implementierungen.

Die zusätzlich freigegebene Datei
`native/src/share/core/share_remote_discovery_task_tests.rs` benötigt keine
Änderung des mathematischen PAKE-Vertrags: der Primitive-Test heißt jetzt
`share_remote_task_pake_primitive_accepts_empty_and_zero_bytes` und erläutert
explizit, dass die Anwendung/Ports leere PINs verbieten. Die Primitive bleibt
unverändert. Konkrete neue Abnahmesignale für die Port-Grenze stehen in
`discovery_exchange_port_tests.rs` und `discovery_exchange_port_helpers.rs`.

Bei der Suite-Integration müssen alte E2E-Aufrufe, sofern sie einen nackten
Server als neue CLI-Eingabe verwenden, explizit `tcp://`/`ws://` mit
`--allow-plaintext` wählen. Die aus `docs/SHARE_SERVER.md` bekannten
Entrypoints sind `native/test-share-lifecycle-e2e.sh`,
`native/test-share-lifecycle-e2e-windows.ps1` und
`native/test-share-mixed-version-e2e.sh`; deren Inhalte wurden außerhalb meines
Scopes nicht gelesen. Gespeicherte Worker-Testadressen bleiben TCP.

Suite-Integration, Root-Graph, Commits, Push, Remote-CI und die abschließende
Release-Transaktion gehören dem Orchestrator. Diese Schritte sind keine
unerledigten Implementierungsaufträge dieses Subagenten.
