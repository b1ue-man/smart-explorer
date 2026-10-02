# Anfragen A-CLIENT

Stand: 2026-10-02 (Teil 1).

## 1. A35 „Im Explorer öffnen“ behält den Ort – `app/core/app_models.rs`, `app/core/picker_impl.rs` (ohne Besitzer)
- Stelle: `StorageScanSource::Remote { backend, root, label }` (`app_models.rs:193-197`) und die Analyse-/
  Reclaim-Übergabe der Ordnerauswahl (`picker_impl.rs:350-378`, ruft `start_analytics_scan_remote(backend,
  root, conn_label)` ohne `picker.endpoint_prefix`).
- Änderung: `StorageScanSource::Remote` additiv um `endpoint_prefix: Option<String>` (und `account:
  Option<String>`) erweitern; Konstruktor `StorageScanSource::remote_at(backend, root, label,
  endpoint_prefix, account)` (der bisherige `remote(...)` bleibt mit `None`). `picker_impl.rs` übergibt
  `picker.endpoint_prefix` (leer → `None`).
- Grund: `navigate_storage_source` (A-CLIENT, `analytics_core.rs`) baut sonst einen `RemoteState` ohne
  Endpunkt-Präfix: der Bereich ist nicht als Sync-Ort wählbar, `pane_endpoint` scheitert, Ordner-
  Einstellungen kollidieren mit gleichnamigen lokalen Pfaden (AGENTS.md „Explorer locations“).
- Danach (A-CLIENT): `analytics_ui.rs`/`reclaim_ui.rs` übergeben die Felder des aktuellen `RemoteState`,
  `navigate_storage_source` übernimmt sie (bzw. nutzt den Explorer-Zustand bei gleichem Präfix statt nur bei
  gleichem `Arc`), Kontextmenü-Analysen nutzen das Verbindungs-Label statt des Pfads.
- Bis dahin: unverändert (Explorer-Zustand wird bei identischem Backend weiterverwendet).

## 2. Veralteter Listing-Assembly-Test – `analytics/os/shared/analytics.rs` (K2) und `analytics_tests.rs` (ohne Besitzer)
- Stelle: `analytics.rs` `#[cfg(test)] use backend::{build_from_listings, ChildMeta};` und Test
  `parallel_tree_assembly` in `analytics_tests.rs`.
- Änderung: Import und Test entfernen.
- Grund: Der Listing-Walker (`analytics_backend.rs`) setzt den Baum jetzt ordnerweise zusammen (Work-Stealing,
  Aufbewahrungsbudget); `build_from_listings`/`ChildMeta` gibt es nur noch `#[cfg(test)]` für diesen Test. Die
  Walker-Tests stehen in `analytics_backend_tests.rs`. Nach der Änderung entfernt A-CLIENT die beiden
  Test-Reste.

## 3. Begrenzte Bereichs-Lesungen (optional, Leistung des Duplikat-Rückfalls) – V1 (K1) / V2 (K2)
- Änderung: optionaler Haken `Backend::open_read_range(path, id, offset, len) -> VfsResult<Option<Box<dyn
  Read + Send>>>` (Standard: `open_read_at` + `take(len)`), weitergereicht von Caching-/Agent-Backend; Share
  `FsRequest::ReadAt` additiv `len: Option<u64>` (Host liest höchstens `len` Bytes), Agent-Protokoll hat
  `Read.len` bereits.
- Grund: Der Rückfall der Duplikatsuche (alte Hosts, FTP/WebDAV/SMB) liest je Kandidat nur 2 × 64 KiB; ohne
  Länge schickt die Gegenseite bis zum Abbruch einen ganzen Stromfenster-Vorlauf (QUIC bis 16 MiB je Strom).
- A-CLIENT nutzt den Haken in `reclaim/backend_compare.rs`, sobald er existiert.

## 4. Hinweis an Orchestrator/SUITE: Agent-Nutzlasten
- Ab Teil 2 ändert A-CLIENT `native/src/agent_proto/**` (Feld `special` im Drahtformat, Hash-Walk mit
  Auslassungen, sicheres Öffnen im SSH-Agenten; `PROTO_VERSION` steigt). Die eingebetteten Agenten
  (`native/agent-bin/se-agent-*`) müssen dann über `native/build-agent-bundles.sh` auf dem Remote-Runner neu
  gebaut werden (Frische-Prüfung der CI).
