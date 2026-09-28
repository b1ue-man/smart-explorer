# Windows remote mount and long-running Direct repair

Investigation date: 2026-09-28. Reported client: 0.5.164. Acceptance is pending;
this document is a task plan and evidence, not a release claim.

## Goal and stage-one plan

Restore mounting a device share on Windows when its children collide under
Windows case comparison, and restore Direct connections after extended uptime
without terminating the worker. Preserve endpoint identity, stored paths,
permissions, mount leases, cancellation and the no-replay rule for mutations.
The same batch also includes Direct Share storage analysis: run the identical
local analytics worker on the exporting host, preserve its complete/partial/
failed/canceled outcome, and expose measured counters and current work. A spinner,
invented estimates, silent stalls or an unexplained fallback are not acceptance.

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

## Storage analysis extension: both planning stages

Stage-one evidence: `PeerBackend::walk_tree` requests a host snapshot, but
`storage_snapshot::build_snapshot` uses the serial `ServerWalker`, not
`analytics::scan`. Each directory repeats Share resolution and emits a blocking
progress update. The GUI's two-counter callback drops the directory count and
cannot carry the local worker's partial-result diagnostics or aggregation notes.
The local worker already uses batched Windows directory records and a bounded
Rayon pool. Reusing that exact worker, rather than adding another scanner, is the
implementation boundary.

Additional primary research checked on 2026-09-28:

- [Windows directory records](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_extd_dir_info)
  supply lengths, names, attributes and reparse tags in batches; retain the
  existing `local_access` enumerator and its provider fallbacks.
- [Rayon pool configuration](https://docs.rs/rayon/latest/rayon/struct.ThreadPoolBuilder.html)
  and [Windows impersonation tokens](https://learn.microsoft.com/en-us/windows/win32/secauthz/impersonation-tokens):
  retain the local worker's pool limits and authority check.
- [Tokio blocking tasks](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html):
  running synchronous work is not canceled by aborting a future. A stream-owned
  cancellation flag must reach the actual scanner, including queue and transfer.
- [Tokio watch semantics](https://docs.rs/tokio/latest/tokio/sync/watch/index.html):
  progress is sampled state, not a queue of events that may throttle traversal.

Stage-two milestones, integrated into M5's single suite before M6:

| Milestone | Boundary/files | Concrete expected result |
| --- | --- | --- |
| M7: identical host worker | `analytics` scanner/progress, new Share host adapter | Local/UNC exports resolve once and invoke `analytics::scan` with the same enumeration, pool, retention, counts and diagnostics as local analysis. A confined adapter preserves Share roots and skips links; synthetic roots combine real outcomes. Nonlocal exported backends retain their identity and existing scan capability. |
| M8: complete analysis transport | additive Share capability/request and bounded report/tree stream; VFS/cache forwarding | Direct/Room callers receive the local outcome, including partial errors and aggregate counts, without per-file network requests. Finished trees transfer in bounded chunks with exact length/count checks and SHA-256. Legacy peers remain accessible and explicitly identify the older analysis path. Cancellation stops the host worker; loss of contact is distinct from unchanged counters. |
| M9: truthful shared progress | shared analytics progress model and GUI | Local and Direct display actual files, directories and bytes, current phase/path, elapsed time without ETA or percentage guesses, time since new work and age of remote evidence. Transfer shows measured received bytes separately. Terminal partial/failure/canceled states remain visible. |
| M10: equivalence and throughput evidence | task fixtures selected by `windows_remote_task_` | Compare local and real authenticated Direct analysis of the same wide/deep tree, including aggregation. Assert identical counts/tree/outcome, bounded progress traffic independent of file count, cancellation and negative protocol/confinement cases; record measured local, host and end-to-end timing without claiming network transfer costs disappear. |

Second gap review: the existing snapshot-v1 wire format equates retained nodes
with scanned files and cannot represent the local worker's aggregation/partial
outcomes. Preserve v1 for older callers; negotiate a richer additive analysis
operation. Stream the local tree with an iterative codec rather than narrowing
the local worker to v1's tree limits or retaining a second full encoded copy.
Progress emission runs independently of scanning at a bounded cadence. Keep
directory authority/confinement checks at the OS adapter; never resolve a remote
backend path as a local path. Expose additional transport latency separately and
use measurements, not an unsupported promise of equal end-to-end elapsed time.

Integration gap found before extending the IPC implementation: the actual GUI
receives `AgentBackend` from `daemon::open_share_backend`. Its legacy WalkTree
handler unconditionally instantiates `TreeWalker`, whose recursive entry path
calls `backend.stat` even for files already present in a listing. This bypasses
`PeerBackend::walk_tree` entirely and makes latency proportional to file count.
M8/M10 therefore include this required boundary: an authenticated, cancellable
analysis IPC operation, selected through the agent wrapper's underlying Share
identity, forwards the rich analysis operation to the peer. Reuse the same
bounded tree/report receiver for IPC and QUIC. Keep SSH agent framing/version
unchanged; correct the legacy daemon tree handler to forward supported server
walks. The acceptance fixture must traverse the real agent/worker bridge and
assert that no per-file metadata request reaches the peer. Socket cancellation
must wake the real receiver, rather than discarding partially read frames on a
poll timeout (same TCP framing research as M4).

No local builds, compilers, native formatters or tests. Static parsing and diff
inspection only during implementation. The one remote suite must have at least
30 minutes; terminal release uses the existing six-hour remote job and
`native/publish-release-local.ps1`, never a workstation invocation.
