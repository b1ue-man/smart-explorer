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

## G1 (2026-11-14, after the third suite run): ACK-only packets while the window is full, RTT samples to the controller

Written and reviewed without a local build or test run (`AGENTS.md`). Plan
`docs/plaene/2026-11-14-verbindungen-je-zweck/plan.md`, E2.

### Cause

Measurement M2 of the third suite run: a download and an upload on one `Files`
connection, both ends yielding (`yield_controller.rs`), moved 9.6 MiB/s of
112 MiB/s on `lan` and 0.0 MiB/s on `wan`, nearly independent of the target delay.
In `poll_transmit_path_space` a space with stream data (`can_send.other`) and a full
congestion window returned `NothingToSend` before any packet was built, so ACKs that
were due (`can_send.acks`, `immediate_ack_required`) stayed back as well, the
`max_ack_delay` timer included. With small windows at both ends each end waited for
the other's ACKs until a PTO, and the waiting time reached the peer as round trip,
which a delay-based controller reads as queueing and answers with a smaller window.

Review of this patch (2026-11-14): the withheld ACKs explain the standstill on `wan`
(windows at their floor), not the `lan` figure. `path_congestion_check` of noq 1.3.0
blocks a datagram when `in_flight + MTU >= window`, so a window of n MSS carries
n − 1 packets, and the controller bound its window to that flight + 1 MSS on every
ACK frame: the window could not pass the next multiple of the MSS above its start
(12 000 bytes, Cubic's initial window; at an MTU of 1452 bytes that multiple is
9 MSS, with 8 packets in flight). 8 packets per direction in a round trip of 2.3 ms
are 9.6 MiB/s, the measured figure for the targets 25 and 50 ms (two assumptions,
because M2 records neither value: MTU discovery had reached its default upper bound
of 1452 bytes, and the round trip was 0.3 ms above the link's 2 ms). That cause is
corrected in `native/crates/engine/src/share/core/yield_controller.rs`, not in the
vendored code.

References: RFC 9002 §2 (an ACK-only packet is not in flight), §7 (packets
containing only ACK frames are not congestion controlled) and §7.7 (they should not
be paced); RFC 9000 §13.2.1 (ack-eliciting 1-RTT packets are acknowledged within
`max_ack_delay`; at most one ACK-only packet per ack-eliciting packet received).
Upstream, read on 2026-11-14 (`n0-computer/noq`, branch `main`,
`noq-proto/src/connection/mod.rs`): `path_congestion_check` still returns
`Congestion` for a space with pending data, and no other place sends an ACK-only
packet then, so this patch is not in upstream. Upstream compares with `>` instead
of `>=` since commit `9612584d` ("fix(proto): Allow the cwnd to be filled exactly
(#823)", 2026-10-15), which is not part of 1.3.0 and not taken over here.

### Changed places

1. `noq-proto-1.3.0/src/connection/mod.rs`:
   - `Connection::ack_only_allowed` (new): a datagram held back by the congestion
     window may go out as one ACK-only packet when the space is Data, an ACK is due,
     no CONNECTION_CLOSE is pending, the path may send Data frames and is not
     abandoned, the transmit is still empty (no padding to an earlier datagram's GSO
     segment size) and `pad_to_mtu` is off (a padded packet would be in flight).
   - `poll_transmit_path_space`: uses it after `path_congestion_check`; the packet
     skips the pacer like every ACK-only packet (it consumes no pacing tokens,
     `finish_and_track` with size 0). After that packet the function returns
     `PollPathSpaceStatus::Send` (review, 2026-11-14): `clip_segment_size` has set
     the segment size to the size of this short datagram, and the next round of the
     loop tested the congestion window against that size instead of the MTU. With
     more than one datagram per transmit (GSO) it then filled the rest of the window,
     which is less than one MTU, with datagrams of that size: fragments of stream
     data, or padding only where no STREAM frame fitted.
   - `populate_packet`: new argument `acks_only`; when set, HANDSHAKE_DONE, PING and
     IMMEDIATE_ACK stay pending and the packet ends after the ACK frames.
   - After `path_data.rtt.update(ack_delay, rtt)`: `congestion.on_rtt_sample(now,
     rtt, ack_delay)`.
2. `noq-proto-1.3.0/src/congestion.rs`: `Controller::on_rtt_sample(now, latest_rtt,
   ack_delay)` with an empty default body: the sample of RFC 9002 §5.1 (largest newly
   acknowledged packet, one per ACK frame) and the ACK delay as `RttEstimator::update`
   receives it (limited to the peer's `max_ack_delay`, zero outside the Data space).
   Called after `on_ack` and `on_end_acks` of the same frame. Cubic, NewReno and BBRv3
   keep the default; Smart Explorer's yielding controller uses it.

### Contract

- With the window open nothing changes. With it full, a due ACK leaves at once in a
  packet of ACK frames only, which is the only datagram of its transmit; stream
  data, PING, IMMEDIATE_ACK and HANDSHAKE_DONE wait for the window as before, so
  bytes in flight never exceed the window by this patch.
- The packet is not ack-eliciting, not padded and not in flight: it arms no loss
  timer, takes no pacing tokens and no congestion window. The anti-amplification
  limit is checked before the congestion window and still blocks it. One such
  packet leaves per due ACK (`PendingAcks::acks_sent` clears the request).
- Not changed: the Initial and Handshake spaces, a transmit that already holds a
  datagram, `pad_to_mtu`, a pending CONNECTION_CLOSE, and a datagram the pacer holds
  back (`PathBlocked::Pacing`): there a due ACK waits as in noq 1.3.0.
- Controllers that do not implement `on_rtt_sample` behave as before.

### Oracles (remote suite, not executed)

- M2 (`verbindungen_messung_task_m2_rueckzug_*`): the transfer next to the picture
  moves data on `wan` (it stood still) and on `lan` reaches the share of the
  acceptance rule E2, with the picture latency still within it.
- `yield_controller_tests.rs` `verbindungen_task_yield_ack_delay_is_not_queueing`
  (the controller side of `on_rtt_sample`) and
  `verbindungen_task_yield_full_window_survives_acknowledgements_in_batches` (the
  window grows under the send gate of noq 1.3.0 and with several ACK frames per
  driver pass).
- Not observable through the public API: whether a single ACK-only packet left while
  the window was full; no unit oracle in the vendored crate (its dev-dependencies do
  not resolve in the workspace, see above).

### Rebase and removal

On a pin change: if upstream sends due ACKs while congestion blocked, drop the
`acks_only` part; if upstream passes RTT samples or ACK delays to controllers, switch
`yield_controller.rs` to that and drop `on_rtt_sample`. If the new pin contains
upstream commit `9612584d` (`in_flight + MTU > window`, the window can be filled
exactly), set `MIN_CWND` and `ALLOWED_INCREASE` in `yield_controller.rs` back to
RFC 6817's 2 and 1: both carry one MSS for the send gate of noq 1.3.0.
