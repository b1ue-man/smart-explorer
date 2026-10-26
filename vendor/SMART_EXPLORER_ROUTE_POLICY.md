# Smart Explorer transport policy patch

Source baseline: Iroh 1.3.0, commit
`0072d7d84b233f9e7185eb676f049beaf557ac03`; Noq and Noq-proto 1.3.0,
commit `c1f411562e6078852749b8bcf1190523096a107f`. The complete crates were
copied from the existing Cargo registry cache, without a local build. The Cargo manifests declare MIT OR Apache-2.0. Noq and Noq-proto retain both
license files. The published Iroh crate contains `LICENSE-BSD3` for its
Tailscale-derived socket code but omits the repository-root MIT/Apache license
files; the parent integration must include those pinned upstream notices in the
distributed third-party material. The BSD3 notice remains unchanged. This patch adds no
runtime dependency, network service, account or artifact download.

## Authorized scope and concrete plan (2026-10-25)

The authorized P28b/V3 outcome is an explicitly enabled application-data gate
inside the pinned QUIC sender, with typed NAT-round facts for the later app
adapter. App integration, interface classification, candidate snapshot policy,
UI, suite orchestration and release belong to the parent task.

Baseline findings: `Connection::scheduling_info` permits 0-RTT/early server data
and uses Backup paths when no Available path remains. `populate_packet` emits
DATAGRAM and STREAM; `StreamsState::write_stream_frames` writes a StreamMeta
header followed by bytes directly into the builder buffer. Noq `State` retains
an encrypted transmit when I/O returns Pending. Iroh `apply_selected_path`
closes redundant IP paths. NAT state has retry counters and queued challenges,
but no completed-round event or recorded last-response deadline.

The selected implementation keeps the existing transport scheduler for control
frames and adds independent application admission. A single admitted PathId is
bound to the connection's internal path generation, monotone policy generation
and mutation revision. An opaque observation token prevents granting a stale
path snapshot. A generation change or explicit block clears admission. Packet
serialization checks admission regardless of packet space; caller-side checks
preserve unconsumed queues. Noq invalidates a retained encrypted transmit before
retrying I/O if its stamp no longer matches. Already accepted socket/relay sends
are in flight and cannot be recalled.

Alternatives excluded by V3 and the source: Available/Backup status cannot block
all payload; `PathSelection::none()` retains prior selection and has no gate;
an appwrite check cannot constrain buffered data or retransmits; filtering
ciphertext cannot classify frames safely. The patch leaves default users on the
upstream behavior and opts in before creating a connection through transport
configuration. Existing bounded stream/datagram send buffers remain bounded;
the app adapter must also gate app writes and deadlines.

Milestones and later common-suite acceptance:

1. Noq-proto policy types/config and path observation: disabled is unchanged;
   enabled starts blocked, only a current validated token admits one path;
   stale tokens, new paths, migration and generation changes cannot inherit it.
2. STREAM/DATAGRAM serialization plus retained Noq transmit invalidation:
   unadmitted paths carry zero application frames in Initial/Handshake/0-RTT,
   early server data, ordinary sending, retransmission and GSO. Control traffic
   remains possible; no close reason on an unadmitted path includes app bytes.
3. Typed NAT facts: report actual serialized probe count, remaining retry slots,
   pending probes, response deadlines, actual retry timer and active path
   validation deadlines. Facts do not claim universal reachability or invent a
   Finished message. The app freezes its own finite candidate snapshot and must
   not extend it when Iroh starts a subsequent upgrade round.
4. Noq/Iroh API propagation and preservation of policy-enabled direct candidates:
   same connection/stream identity, cancellation, stream reliability, datagram
   best-effort semantics, IPv4/IPv6/NAT, relay, custom transports, path validation,
   congestion control, anti-amplification and non-policy users remain supported.

### B5-R02/R03/R05 correction plan and source outcome (2026-10-25)

The reviewed baseline left old on-path challenge tokens valid across an app
generation, preserved all direct paths without a replacement rule at the
negotiated limit, and left failed app revalidations blocked without a fresh
attempt on path reuse. The correction keeps the pinned transport and its path
limit. The selected boundaries and acceptance signals are:

1. Fresh proof: clear current outstanding challenges and their retry timer at
   generation start, while keeping QUIC validation, congestion state and previous
   migration data. A pre-generation response remains unknown and cannot cancel
   the new probe. A timeout rollback clears restored old tokens before they can
   clear the pending app proof. Handshake address validation cannot clear an
   already required policy proof.
2. Finite retry: use the actual tested-path PTO, the pre-revocation admitted-path
   PTO (or best validated PTO), initial PTO, and any previous migration PTO for
   the three-PTO deadline. A timeout ends that attempt without granting data.
   A subsequent existing-path open/NAT attempt or existing per-path keepalive
   starts a new finite attempt; an active attempt never gets a later deadline
   from these triggers. The app's frozen initial deadline remains unchanged.
3. Bounded replacement: after actual PathId-capacity exhaustion, retire at most
   one nonselected redundant IP path while its abandonment is draining. The
   atomic token mutation protects the grant, ongoing current/previous validation
   and the last current policy-validated CID-usable path. Iroh also applies this
   rule in its existing 60-second connection check when a successful NAT path
   open is waiting, since those opens occur inside Noq. Normal PATH_ABANDON,
   MAX_PATH_ID and CID exchange creates replacement capacity; no limit is raised.

Snapshot + ordinary `Path::close` was rejected because a concurrent Allow could
admit that path after observation. Unbounded path retention and a larger path
budget were rejected because they do not provide a bounded replacement rule.
Clearing the app gate on an ACK or on a failed proof was rejected because
neither supplies a fresh on-path response. New logic is isolated in
`connection/application_path_revalidation.rs` and
`socket/remote_map/application_path_retirement.rs`; the large upstream files
receive only integration hooks. Static parsing is not compilation or a remote
functional result.

The parent remote task suite must observe these correction oracles through its
existing isolated transport harness (no local execution was performed):

- R02: retain the response to C in generation g, begin g+1, then inject that
  response before serializing C1. The path remains app-unvalidated and blocked,
  the new challenge stays pending, and its deadline remains armed. Only the
  response to C1 permits a current-token Allow. Repeat across migration timeout
  rollback and keep a handshake-address-validation event from clearing the gate.
- R03: fill the negotiated eight active slots with relay plus seven reachable
  IP tuples, retain a grant, and queue a successfully traversed LAN candidate.
  Iroh's existing connection check retires one eligible nonselected IP tuple;
  grant and usable survivor remain. Deliver normal abandon/limit/CID control and
  validate the new path before selecting it for data. No more than eight active
  paths exist. A stale token, a newly admitted candidate, a validating candidate,
  a last usable candidate, or newly free capacity prevents retirement. Disabled
  policy retains the upstream pruning behavior.
- R05: give the tested path a higher PTO than the initial/relay reference. Its
  transport deadline includes three times that PTO (or the higher previous
  migration PTO). Lose all probe responses while control ACKs keep it alive,
  expire the attempt, then trigger an existing-path open and, separately, the
  per-path keepalive. Each trigger can start a new finite proof after failure;
  repeated triggers during one attempt cannot extend its deadline. No payload
  is admitted before the fresh response, and the initial app deadline is unchanged.

Research checked 2026-10-25: [RFC 9000 §8.2.4](https://www.rfc-editor.org/rfc/rfc9000.html#section-8.2.4)
uses three times the larger old/new PTO for path-validation failure. V3's use of
that measure for collection and NAT response waiting is project policy, not an
Iroh discovery guarantee. The pinned [Noq packet builder](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/packet_builder.rs)
is the pre-encryption frame boundary. Retry values come from the same helper as
the actual Noq retry scheduler, rather than the imprecise upstream “up to 4s”
comment.

## Verification status

Source implementation and static author review complete. No local builds, compilers or tests are permitted.
The parent owns the one remote task suite. Static source review is not functional
verification. Required remote evidence includes frame classification, payload
byte paths in both directions, stale encrypted transmit rejection, 0-RTT/early
server data, retransmission under policy changes, control-only progress,
cancellation and the preserved transport capabilities listed above.

## Rebase and removal contract

On any Iroh/Noq pin change, compare every changed hunk against the exact upstream
commit, classify every frame variant explicitly, trace every payload serializer
and every retained transmit, and rerun the same isolated policy cases remotely.
Unknown frame variants must require an explicit source decision. Do not replace
the gate with Backup status, an empty selector, app-only gating or ciphertext
filtering. Remove the patch only when an upstream API provides the same
pre-encryption admission, stale-plan invalidation and typed round facts and the
remote suite proves equivalence. Preserve upstream license and origin records.

## API and lifetime contract

Enable before connection construction with
`iroh::endpoint::QuicTransportConfig::builder().application_path_policy(true).build()`.
Carry that config through the endpoint's accepting and dialing configurations,
including explicit per-connection overrides. A disabled connection cannot be
silently switched to the policy after sending early application bytes.

`iroh::endpoint::Connection<HandshakeCompleted>` exposes:

- `application_path_policy() -> ApplicationPathPolicySnapshot`;
- `update_application_path_policy(ApplicationPathUpdate) -> Result<(), ApplicationPathPolicyError>`;
- `application_data_allowed() -> bool` for additional appwrite/deadline coordination.

The same methods exist on Noq Connection. Noq-proto's update method additionally
accepts its monotone `now: Instant` before the update argument. Noq supplies its
runtime clock while holding the connection mutex. The public token/update/error/
snapshot/path-observation types pass through all three crates. Iroh also exports
`NatTraversalRoundObservation` and `NatTraversalCandidateObservation`.

Initial policy generation/revision are zero, with no grant. `BeginGeneration(g)`
requires `g > current`, blocks data, invalidates old tokens and schedules fresh
on-path validation for existing paths. `Block(g)` requires the current generation
and invalidates observation tokens even when already blocked. `Allow(token)`
requires the same connection, generation, revision and internal path incarnation,
a completed handshake, valid current path, available CIDs and no outstanding
policy-required revalidation. Every successful update advances revision. Noq's
Available/Backup status is independent: the explicit grant chooses the one actual
application path, even if control scheduling currently calls it Backup. The app
adapter should choose the same PathId through its PathSelector and the gate.

`RetireRedundantPath(token)` is the additional capacity-recovery mutation. It
checks the same connection, generation, revision and path incarnation as Allow,
then atomically rechecks capacity and the protections described above. Success
uses ordinary path abandonment, advances revision once and preserves admission
on the independent surviving path. `ProtectedPath` means that this is the server
side, capacity is no
longer full, the requested path is admitted/validating/abandoned, an abandonment
is still draining, the generation is not current, or no usable survivor exists.
Existing disabled/closed/stale-generation/stale-observation/revision-exhaustion
errors remain applicable. An unsuccessful protection or stale-token check does
not close a path. Iroh attempts this only for nonselected IP candidates, choosing
the largest current PTO first; relay and custom paths are retained.

A Noq-observed network change immediately blocks data and sets
`requires_new_generation=true`; `network_change_count` is diagnostic. Allow then
returns `NetworkGenerationRequired` until BeginGeneration succeeds and any
prepared transport recovery has completed. Existing
paths need a fresh on-path response before their observation becomes validated.
A passive change between two known local IPs follows the same rule. Remote
migration revokes the affected path and invalidates old observation tokens;
restoring an old RFC 9000 path incarnation never restores its old grant.

### F42: confirmed change before transport reuse (2026-10-26)

Iroh's existing `Endpoint::network_change()` remains a potential-change hint.
The new `Endpoint::network_change_detected(is_major: bool)` is for a change the
local host has already confirmed. It uses the same actor path as Netwatch's
detected events and acknowledges that actor step; readiness can still be pending.
A fixture may inject this confirmed host event, but must not describe that as a
physical adapter switch or directly manufacture a counter/RouteStatus success.

Before a detected change rebinds transports, Iroh synchronously calls Noq's
`Endpoint::begin_application_network_change()`. Noq holds weak handles only for
enabled policies, releases the endpoint mutex before taking each connection's
send mutex, and revokes admission there. Connections inserted during this
prepared phase also start blocked. This changes the policy revision before any
subsequent retained-transmit retry; accepted I/O remains already in flight.

The existing default-route readiness/backoff and transport recovery remain.
While waiting, BeginGeneration still succeeds but starts no new path proof;
observation validation, Allow, payload admission and redundant path retirement
stay blocked by a separate internal phase. Snapshots expose
`network_change_in_progress` so app route monitors can remain explicitly paused
without consuming their collection/direct-choice windows. The saved PTO keeps
the pre-revocation validation reference. At recovery Noq advances revision again,
requires a new generation after readiness, and starts fresh finite on-path
validation, without counting the prepared change twice. The app monitor then
resets its origin and frozen candidate set; its absolute operation deadline is
unchanged. A current transport-epoch check also prevents an asynchronously
delivered old Offline status from ending that new network wait. Cancelled,
connection closure and the absolute deadline remain terminal. Passive IP-change invalidation is
unchanged. Disabled connections retain their original queued recovery event,
zero new payload counters and unrestricted policy gate. Weak entries are removed
at the existing Drained boundary and never keep a connection alive on their own.

Each path observation exposes token, Noq `network_path`, validated/data_allowed,
current and previous validation-in-progress flags, application_tx_bytes/application_rx_bytes, actual validation-failure and
challenge-retry deadlines, and current PTO. Iroh relay addresses in `network_path`
may be internal mapped socket addresses; resolve established PathIds through
Iroh `paths()` to obtain real transport addresses. Do not persist opaque tokens
or mapped addresses, or reuse tokens across connections or policy revisions.
Snapshots are owned values; they keep no socket/connection alive.
`retirement_allowed` additionally reports the current capacity/protection result;
it is an observation, never permission to bypass the atomic token mutation.

NAT facts are for the **current protocol round**, not an aggregate discovery
completion. The app owns the frozen initial candidate set and its absolute
collection/probe/response/validation deadline. A later protocol round supersedes
the current round observation; it never extends that app deadline. Each candidate
reports canonical remote, remaining retry slots, queued-probe flag, matching
response flag, actual serialized scheduled-probe count, separate responsive-probe
count, last-probe time, selected response wait and response deadline. A serialized
probe is not proof that the OS delivered it or the peer received it. The real
retry timer is reported separately. With initial RTT 333 ms the eight scheduled
intervals sum to 4.1312 s; I/O delay, the last response wait and subsequent path
validation are additional. No probe yet sent means no response deadline. Probe
exhaustion does not imply that outstanding responses or path opens have finished.
All deadline Instants use the connection's monotone clock. The response wait is
recorded as three times max(initial PTO, best validated PTO) at serialization.
An unrepresentable checked clock addition leaves the deadline absent; callers
must not interpret absence as a successful or completed attempt.

The gate changes sending only. New receiving code cannot guarantee the remote
sender's policy for an old peer. Existing receiver behavior, transport handshake,
stream offsets/IDs, retransmission bookkeeping, datagram buffer limits,
flow/congestion control and anti-amplification remain in their original modules.
Unadmitted paths continue to send the explicitly classified transport frames.
Close reasons are cleared before encoding, qlog recording and serializer logging
on policy-enabled unadmitted paths, while error codes remain available. Initial
metadata fanout in Iroh still carries control-only QUIC packets because policy
connections cannot obtain any application grant during the transport handshake.
Once the socket/relay sender has accepted a transmit it is in flight; revocation
can reject retained Pending I/O but cannot recall accepted I/O.

## Patch surface and static preservation evidence

Production changes are confined to:

- Iroh `src/endpoint.rs`, `src/endpoint/connection.rs`, `src/endpoint/quic.rs`:
  typed facade, opt-in config and explicit confirmed host-event entrypoint.
  Existing connect/accept/TLS paths remain.
- Iroh `src/socket.rs`: revoke enabled policies before a detected event rebinds
  transports; preserve the potential hint and existing readiness/recovery path.
- Iroh `src/socket/remote_map/remote_state.rs`: policy-enabled connections keep
  direct candidates instead of closing every nonselected IP path, with the
  bounded capacity-recovery helper `application_path_retirement.rs`. Existing
  Noq concurrent-path limits remain; disabled connections keep the original
  direct-path closing rule. The selector never creates a grant.
- Noq `src/lib.rs`, `src/connection.rs`: exports, serialized update methods and
  generation/path-aware invalidation of a retained encrypted transmit. Original
  blocked-writer/read wakeups, path events, stream and datagram APIs remain.
- Noq `src/endpoint.rs`: weak opt-in policy registration, synchronous preparation
  and completion under connection send mutexes, with the disabled event path
  and ordinary connection lifetime retained.
- Noq-proto `src/lib.rs`, `src/config/transport.rs`: typed exports and default-false
  config; the config Debug implementation reports the option.
- Noq-proto `src/connection/application_path_policy.rs`: new cohesive policy
  state, token/update/snapshot/error types, validation and generation boundaries.
- Noq-proto `src/connection/application_path_revalidation.rs`: finite fresh-proof
  rounds, failed-round retry and atomic redundant-path retirement protection.
- Noq-proto `src/connection/mod.rs`: application admission after path choice,
  distinct control scheduling, explicit network/migration invalidation and NAT
  timing hooks. Existing packet-space key selection, retransmit processing,
  congestion/anti-amplification, NAT challenge and path-response code remain.
- Noq-proto `src/frame.rs`, `src/connection/packet_builder.rs`: exhaustive frame
  classification, common pre-encryption gate, redacted close reason and test
  builder. Every frame variant in this pin has an explicit classification.
- Noq-proto `src/connection/streams/state.rs`, `src/connection/datagrams.rs`:
  check before dequeuing/mutating payload. Existing stream priority/fairness,
  reliable retransmission offsets and datagram queue/drop limits remain.
- Noq-proto `src/n0_nat_traversal.rs`: bounded per-current-candidate timing facts
  and shared retry-interval helper. Original negotiated address limits, IPv4/
  IPv6 conversion, challenge matching and retry scheduling remain. Removed
  candidates and new rounds release timing metadata. Supplemental responsive
  probes remain separate from the nine scheduled attempts.

No Cargo manifest, lockfile, UI, Android, CLI or existing iroh-relay vendor
source is changed by this patch surface. F42 also updates the native shared
route monitor, its private network-wait helper and the existing B5 fixtures;
the parent owns the corresponding root graph refresh and remote acceptance. Existing
upstream files exceed the project's new-native-file size threshold; no unrelated
vendor extraction or cleanup was performed. The new policy module stays below
500 lines. Memory added is constant policy metadata plus sets/maps bounded by
current QUIC paths and the already negotiated current NAT candidate limit;
observation snapshots allocate proportional to those current sets. The app must
retain only its explicitly bounded diagnostic history.

The F42 endpoint registration adds one weak handle per existing enabled
connection and a temporary weak-handle snapshot per confirmed network event.
It adds no persistent strong owner and is cleared at the existing Drained
boundary. The shared monitor's small private helper owns no connection and
adds no independent task or timer.

Remote-suite source cases use prefix `route_policy_task_` in the new policy
module and the existing NAT module. They cover payload rejection at the real
frame encoder, retained datagram-queue contents, control-only close serialization,
actual nine-probe exhaustion, the additional response wait, separate responsive
probes and clearing probe facts on a new round. These source cases have **not**
been executed locally. They do not replace the parent suite's complete handshake,
0-RTT, early-server, migration, stale-Pending-I/O and packet-byte route matrix.


Connection-global and per-PathId `application_tx_bytes`/`application_rx_bytes`
are monotone for their respective lifetimes while policy is enabled. TX counts
serialized STREAM/DATAGRAM payload, including retransmission and encrypted plans
later dropped before I/O acceptance. RX counts authenticated, successfully
processed STREAM/DATAGRAM payload, including repeated STREAM ranges. The counters
exclude frame headers, UDP/QUIC/crypto overhead and transport control. They are
transport-payload counters, not unique received file bytes, acknowledged goodput
or guaranteed socket-delivered rate. Global totals survive path discard and
policy-generation changes; discarded per-path counters are released. A PathId
may migrate, so the app must take generation/route-scoped deltas, not assume a
PathId always denotes the same physical route. Disabled users retain zero new
payload counters and the upstream transport behavior.

Pinned Iroh notices checked 2026-10-25: [MIT](https://raw.githubusercontent.com/n0-computer/iroh/0072d7d84b233f9e7185eb676f049beaf557ac03/LICENSE-MIT)
and [Apache-2.0](https://raw.githubusercontent.com/n0-computer/iroh/0072d7d84b233f9e7185eb676f049beaf557ac03/LICENSE-APACHE).
Every modified vendor source carries a prominent patch notice referring to this
contract; original notices remain. The license-file addition is outside the
worker's assigned write surface and was explicitly handed to the parent.
