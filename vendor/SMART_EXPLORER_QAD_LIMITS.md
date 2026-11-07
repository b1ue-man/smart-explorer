# Smart Explorer QAD limits patch (iroh-relay 1.0.0)

Source baseline: the vendored `vendor/iroh-relay-1.0.0` (crate version 1.0.0,
upstream commit `6520cd6c9bacea15823c53a38267c40d754980fe`, path `iroh-relay`,
from `.cargo_vcs_info.json`), used only by `share-server` through its
`[patch.crates-io]` entry. The Cargo manifest declares `MIT OR Apache-2.0`; the
published crate carries `LICENSE-BSD3` for Tailscale-derived code, which stays
unchanged, and omits the repository-root MIT/Apache license files: the
distribution must include those pinned upstream notices in its third-party
material, as for the Iroh patch in `SMART_EXPLORER_ROUTE_POLICY.md`. This patch
adds no dependency (`lru` was already an unconditional dependency), no network
service and no download. It was written on 2026-11-07 without a local build.

## Reason

`se-share-server` offers QUIC address discovery (QAD) on UDP 7842 beside its
relay (`share-server/src/relay_qad.rs`): a device behind NAT learns the public
address the server observes and publishes it as a direct candidate. Upstream
QAD accepts every incoming connection and holds it as long as the client keeps
it alive (the client sends a keepalive every 25 s and uses a 35 s idle
timeout), with no bound on the number of connections, and a panic in one
connection task ends the whole relay server. On a public UDP port that is a
memory and availability risk.

## Changed places (`vendor/iroh-relay-1.0.0/src/quic.rs` only)

1. New module `limits` with the public type `iroh_relay::quic::QadServerLimits`
   (held connections in total and per source, accept rate and burst in total
   and per source, server idle timeout) and the public function
   `iroh_relay::quic::set_server_limits`. A source is an IPv4 address (also
   IPv4-mapped IPv6) or an IPv6 /64 prefix; per-source rate buckets are kept in
   an LRU of 4096 entries. Every field left at `None` means no limit, so a user
   that never calls `set_server_limits` keeps the upstream behaviour.
2. `QuicServer::spawn` reads the limits set before the start. The accept loop
   checks rate and capacity before the handshake and refuses a surplus
   incoming connection with `Incoming::refuse()`. An admitted connection holds
   a permit; dropping it with the connection task (normal end, error, abort or
   panic) frees its place.
3. With `idle_timeout` set the server offers that idle timeout
   (`TransportConfig::max_idle_timeout`).
4. A panicked connection task is logged as a warning instead of `panic!`, so
   one failed connection no longer ends the QAD service and the relay with it.
5. `QuicSpawnError::InvalidLimits` reports invalid limits (rate not a positive
   finite number, burst zero, idle timeout out of range);
   `QuicSpawnError::EndpointServer` now names the socket error in its message.
6. Address validation before admission (added 2026-11-07, review finding
   R-SERVER-01): an incoming connection whose source address is not yet
   validated (`Incoming::remote_address_validated()` false) is answered with a
   stateless QUIC Retry (`Incoming::retry()`) and takes neither a place nor a
   rate token. Only the client's second Initial, which carries the Retry token
   and so proves that the sender receives packets at its source address, goes
   through the limits of point 2. Senders with forged source addresses can no
   longer fill the places or drain the rate bucket of a victim's address. A
   Retry is smaller than the client's padded Initial, so it amplifies nothing.
   Cost: one extra round trip per new QAD connection; the noq client follows
   Retry on its own. Should `retry()` ever be refused (the endpoint may not
   retry an Initial without a source connection id), the Initial is ignored
   without an answer.

Why not a field on `QuicConfig`: `QuicConfig` and the call of
`QuicServer::spawn` live in `src/server.rs`; the patch was confined to
`src/quic.rs`. The limits apply to every QAD server spawned afterwards in the
process; `se-share-server` runs one and always sets the same values.

## Values used by se-share-server

Derived from the relay limits in `share-server/src/relay.rs`: at most 1024 held
connections (two per relay slot, one per address family), 128 per source (two
times the relay's per-source TCP cap, for carrier-grade NAT), accept rate 256/s
with burst 512 overall and 32/s with burst 64 per source, idle timeout 35 s.

## Test support

Purpose: the tests of `share-server` (crate `smart-explorer-share-server`)
check the admission limits above and the QAD address validation (Retry) from
outside this crate. The module `iroh_relay::quic::qad_testing` is marked
`#[doc(hidden)]`, exists only with feature `server`, and is not part of the
upstream API. It wraps the existing types; it changes no limit, no admission
decision, no retry and no server behaviour.

API (all in `iroh_relay::quic::qad_testing`):

- `TestAdmission::new(QadServerLimits) -> Result<TestAdmission, String>`:
  the admission of the accept loop with the same checks (invalid limits are an
  error).
- `TestAdmission::try_admit_at(IpAddr, std::time::Instant) -> Result<TestPermit,
  &'static str>`: a refusal returns the text `"rate"` or `"capacity"`;
  dropping a `TestPermit` frees its place.
- `TestAdmission::held() -> usize`: connections held right now.
- `probe(SocketAddr, &str, rustls::ClientConfig) -> Result<QadProbe, String>`:
  a QAD client endpoint on the loopback address of the server's address family
  (port 0), connected with `QuicClient::create_conn` for the given server name.
- `QadProbe::observed(Duration).await -> Option<SocketAddr>` (the address the
  server observed), `QadProbe::local_addr() -> Option<SocketAddr>`,
  `QadProbe::close_reason() -> Option<String>`, `QadProbe::close(self).await`
  (closes and waits until the endpoint is idle).
- `reset_server_limits()`: sets `QadServerLimits::default()` again. The limits
  are process-wide (see point 1 above).

Change of the patch for this support: `Admission::held` is no longer
`#[cfg(test)]`, because `qad_testing` reads it in the feature build.

Removal condition: this support is removed together with the patch (see
"Removal condition" below); a test that uses it is then removed or switched to
upstream API in the same change.

## Removal condition

Drop this patch when
upstream iroh-relay bounds QAD connections and accept rate itself and no
longer panics on a failed QAD connection task; then configure the upstream
limits in `share-server/src/relay_qad.rs` instead.
