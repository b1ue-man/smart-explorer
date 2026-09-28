# Windows remote mount and long-running Direct repair

Investigation date: 2026-09-28. Reported client: 0.5.164. Acceptance is pending;
this document is a task plan and evidence, not a release claim.

## Goal and stage-one plan

Restore mounting a device share on Windows when its children collide under
Windows case comparison, and restore Direct connections after extended uptime
without terminating the worker. Preserve endpoint identity, stored paths,
permissions, mount leases, cancellation and the no-replay rule for mutations.

Evidence from the current source and the user's log:

- `mount/core/metadata_loading.rs::filter_listing` rejects an entire directory
  with case-colliding children. Share exports only guarantee exact-name
  uniqueness, and `RootedBackend` deliberately treats peers as case insensitive.
- `share/core/node_sessions.rs::repair_direct_reciprocal` constructs a Tokio
  timer before entering `block_on`. The coordinator sets `RepairTask.running`
  before calling it and has no unwind recovery. An unwinding repair therefore
  leaves `repair_in_flight` true; `ShareHost::reload_now_locked` then keeps
  skipping runtime reconfiguration.
- `PeerEndpointSource` refreshes from server presence only, although initial
  Direct opens use `lan_presence_match::effective_presence`.
- The connection single-flight uses an unbounded blocking mutex before the
  operation deadline. The signaling TCP reader discards partial bytes on an
  ordinary read-poll timeout.

Stage one: repair the mount boundary, repair the Direct coordinator and route
refresh, then verify both through one Windows task entrypoint. Do not rename
exports or change stored locators to fix a display namespace conflict.

## Research and final design

Primary sources checked on 2026-09-28:

- [Windows naming rules](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file):
  normal Windows file access treats case variants as the same name.
- [Tokio timeout](https://docs.rs/tokio/latest/tokio/time/fn.timeout.html):
  timer construction outside a runtime panics; construct it inside an async
  block passed to the runtime.
- [Tokio mutex](https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html):
  asynchronous acquisition can be bounded by the existing operation deadline.
- [Rust TCP timeouts](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.set_read_timeout):
  polling may return WouldBlock or TimedOut depending on the platform. A partial
  protocol message must survive either result.

Second gap review: globally case-folding Share labels would change existing
locators; globally claiming case sensitivity for a mixed Share tree would be
false. Use a mount-only projection at the daemon authorization boundary for
peers with colliding names. Give every colliding child a deterministic suffix
derived from its exact original name. Reserve that alias form so a real child
cannot impersonate it. Resolve aliases back to the original name before every
backend operation, including nested paths and mutations. Reject duplicate
literal names, unresolved aliases and ambiguous unsuffixed opens. Preserve
ordinary spelling and the existing behavior of other backend classes.

## Stage-two milestones and acceptance

| Milestone | Boundary/files | Concrete expected result |
| --- | --- | --- |
| M1: mount namespace | new pure `mount/core/peer_names.rs`; `daemon/os/shared/rooted_backend{,_case}.rs` | A peer exposing `Docs` and `docs` mounts with both entries addressable to their original content; ordering, nesting, long Unicode names and reserved-alias lookalikes cannot retarget access. A read-only mount still rejects writes, and link/root confinement remains enforced. |
| M2: repair lifetime | `share/core/node_sessions.rs`, `direct_reciprocal_coordinator.rs` and extracted worker | Repair runs from a normal OS thread without a timer panic. Any unwinding attempt releases its running state, reports failure and permits subsequent bounded repair/configuration. Existing policy/conflict outcomes remain terminal; no filesystem mutation is retried. |
| M3: reconnection | `share/core/peer_endpoint_source.rs`, `node.rs`, `node_sessions.rs` | Live Direct handles use renewed LAN/server candidates, reject changed identity and revoked access, and obey their deadline while another handshake owns the connection gate. Room routing remains supported. |
| M4: signaling framing | `share/core/line.rs`, `signal_connection.rs` | Fragmented UTF-8/JSON survives Windows-style timeout polls; oversized, truncated and stalled frames fail explicitly without unbounded buffer growth. |
| M5: integration | one checked-in Windows task script/workflow, focused regression fixtures | Exercise mount preload and actual Windows drive access, repair from a plain thread, retry-state cleanup, route renewal and signal framing together, reusing a source-bound development test binary. |
| M6: graph and distribution | root `graphify-out`, canonical documentation, existing `build.yml` | Full root graph refreshed; candidate committed/pushed; one remote suite evaluated; only then the existing complete remote release wrapper builds/publishes once. |

M2 and M3 share session ownership; implement them together before verification.
M1 preserves Direct/Room identity and the existing saved remote/UNC/Drive
boundary; it does not change sync or stored path syntax. M4 preserves the
current TCP/WebSocket protocol. The suite must retain negative permission,
collision and stale-identity cases, not merely a successful open.

No local builds, compilers, native formatters or tests. Static parsing and diff
inspection only during implementation. The one remote suite must have at least
30 minutes; terminal release uses the existing six-hour remote job and
`native/publish-release-local.ps1`, never a workstation invocation.
