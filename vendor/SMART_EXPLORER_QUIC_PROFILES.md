# Smart Explorer QUIC purpose-profile patch (P1, 2026-11-14)

Source baseline: the vendored Iroh 1.3.0 (commit
`0072d7d84b233f9e7185eb676f049beaf557ac03`) and Noq / Noq-proto 1.3.0 (commit
`c1f411562e6078852749b8bcf1190523096a107f`), including every earlier Smart
Explorer patch described in `SMART_EXPLORER_ROUTE_POLICY.md`. Licenses as stated
there (MIT OR Apache-2.0). This patch adds no dependency, no network service and
no download. It was written on 2026-11-14 without a local build or test run
(`AGENTS.md`: no local builds).

Plan: `docs/plaene/2026-11-14-verbindungen-je-zweck/plan.md`, decision E5 (send
batches), package P1. The application side (purpose profiles) belongs to package
P2.

## Send batches from the transport configuration

### Purpose

Noq's connection driver used two constants: at most `MAX_TRANSMIT_SEGMENTS = 10`
datagrams (GSO segments) per transmit and `MAX_TRANSMIT_DATAGRAMS = 20` datagrams
per driver pass. Larger batches need fewer system calls per byte (noq PR #636 and
#645, Nesquic poster; `recherche.md` Q36, Q37). Smart Explorer sets them per
connection purpose (bulk file transfer and TCP tunnels larger, screen and control
connections unchanged), so they became transport-configuration values.

Reference: upstream PR n0-computer/noq#645 ("expose max_transmit_segments and
max_transmit_datagrams in TransportConfig"), state `closed`, not merged
(GitHub API read on 2026-11-14). Differences: #645 put the datagram count into
`EndpointConfig` (one value per endpoint) and capped segments inside
`noq_proto::Connection::poll_transmit`; this patch keeps both values in
`TransportConfig` (one value per connection, as the purpose profiles need) and
leaves `poll_transmit` unchanged, so direct noq-proto users see no change.

### Changed places

1. `noq-proto-1.3.0/src/config/transport.rs`: fields `max_transmit_segments` and
   `max_transmit_datagrams` (defaults `DEFAULT_MAX_TRANSMIT_SEGMENTS = 10`,
   `DEFAULT_MAX_TRANSMIT_DATAGRAMS = 20`, the former constants), setters of the
   same names, getters `effective_max_transmit_segments` and
   `effective_max_transmit_datagrams`, Debug fields, unit tests
   `smart_explorer_transmit_batch_tests`.
2. `noq-proto-1.3.0/src/connection/mod.rs`: `Connection::max_transmit_segments`
   and `Connection::max_transmit_datagrams` return the effective values of the
   connection's configuration. (Plan deviation: P1 did not list this file. The
   configuration field of `noq_proto::Connection` is private to its module and
   `noq::Connection` receives only the `proto::Connection`, so noq cannot read the
   values without this accessor.)
3. `noq-1.3.0/src/connection.rs`: `State::transmit_batch` returns
   `(min(sender segments, configured segments), configured datagrams)`;
   `drive_transmit` uses it instead of the removed constants; tests
   `smart_explorer_transmit_batch_tests`.
4. `iroh-1.3.0/src/endpoint/quic.rs`: `QuicTransportConfigBuilder::max_transmit_segments`
   and `::max_transmit_datagrams` pass the values to noq; test
   `smart_explorer_transmit_batch_tests`.

### Contract

- A configuration without the setters behaves exactly as before: 10 segments per
  transmit (further limited by the UDP sender), 20 datagrams per driver pass.
- Relay limit: Iroh hands one segmented transmit to the relay as one relay packet,
  and the relay refuses packets above `iroh_relay::protos::relay::MAX_PACKET_SIZE`
  (64 KiB, `iroh-relay-1.0.0/src/protos/relay.rs:23`, checked in
  `client/conn.rs:150`). The limit applies to the encoded frame
  (`ClientToRelayMsg::encoded_len`, `protos/relay.rs`): 1 byte frame type,
  32 bytes endpoint id, 1 byte ECN and 2 bytes segment size precede the
  datagrams, 36 bytes in all (`RELAY_FRAME_OVERHEAD`, review 2026-11-14). A
  segment value above 10 is therefore capped at
  `(64 KiB - 36) / largest possible segment`, the largest segment being the
  greatest of `initial_mtu`, `min_mtu` and the MTU discovery upper bound (default
  1452 bytes: cap 45, so the planned 40 passes unchanged). The cap never goes
  below 10: with segments above 6550 bytes (jumbo MTU) even the old constant
  exceeded the limit, and a raised value then falls back to the old behavior
  instead of below it.
- A pass ends after the transmit that reaches the datagram count, so a datagram
  count below the segment count still sends one full transmit per pass (unchanged
  loop logic).

## Verification status

Static only, 2026-11-14: rustfmt parsed every changed file (`--check`, no parse
errors). The remaining format differences lie outside this patch and predate it:
`noq-1.3.0/src/connection.rs` lines 780–809 and 1733,
`noq-proto-1.3.0/src/connection/mod.rs` (imports, policy code), and the `pub use`
lists of `iroh-1.3.0/src/endpoint.rs` and `endpoint/quic.rs`. Nothing was compiled
or run.

Test oracles (remote suite, not executed):

- In the vendored crates: `config/transport.rs`
  `smart_explorer_transmit_batch_tests::*`, `noq-1.3.0/src/connection.rs`
  `smart_explorer_transmit_batch_tests::*` (setter values reach
  `State::transmit_batch`, the input of `drive_transmit`, on both sides of a
  loopback connection), and the `iroh` `endpoint/quic.rs` test module. They need the crates' dev-dependencies,
  which `native/Cargo.lock` does not resolve for `[patch]` path dependencies
  (`assert_matches`, `test-strategy`, `directories-next` are absent from the
  lockfile on 2026-11-14), so `cargo test -p noq-proto` cannot build them inside
  the workspace.
- Therefore the same oracles run through the public API in
  `native/crates/engine/src/share/core/verbindungen_vendor_task_tests.rs`
  (prefix `verbindungen_task_vendor_`): transmit defaults, setters and relay cap;
  Iroh builder values in the noq configuration.
  Not observable through the public API: the datagram count per driver pass
  (`ConnectionStats::udp_tx.ios` is deprecated and always 0); only the in-crate
  noq test covers it.

## Rebase and removal contract

On a pin change of noq / noq-proto / iroh:

1. Check that `drive_transmit` still has its two batch limits and still takes the
   segment limit as the `max_datagrams` argument of `poll_transmit`; keep the
   defaults 10 and 20. If upstream exposes per-connection setters, switch to the
   upstream names and drop this patch.
2. Check that the relay packet limit is still 64 KiB, still applied to a whole
   transmit, and that a datagram frame still carries 36 bytes besides its
   datagrams.
3. Keep every changed file's header note; rerun the oracles above.
