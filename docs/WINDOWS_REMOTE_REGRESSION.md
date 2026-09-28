# Windows remote mount, Direct lifetime and storage analysis repair

Investigation date: 2026-09-28. Reported client: 0.5.164. The complete candidate,
including the callback repair, exact dependency and GUI capacity correction,
passed remote acceptance. Publication remains pending.

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

Acceptance review, M9: the GUI used to discard the running receiver on cancel
and immediately label the operation canceled. Retain the receiver, show the
pending cancellation request until the worker responds, and preserve the final
measured counters for canceled and disconnected workers. Cover these transitions
through the same remote suite using the isolated GUI fixture. Starting a new
scan may still detach the canceled predecessor; it must never reuse its results.

M9 source-identity gap review: the capacity bar looked up the remote root's first
two characters in the client's local drive table. A remote `C:/...` could thus
display the client's `C:` capacity. Make that lookup accept `StorageScanSource`
and only consult local drive data for a local source. Keep existing local and
mapped-drive readings; omit unreported remote capacity. Extend the same GUI
acceptance fixture with equal local/remote paths and an unreported UNC source.

Remote evidence: [candidate da354c4](https://github.com/b1ue-man/smart-explorer/actions/runs/36423440788)
completed real Windows drive access and the full GUI/worker/QUIC analysis path.
For 5,289 files the measured local/host/GUI durations were 43/12/18 ms, with no
per-file metadata calls. Direct ran second with a warmed filesystem cache; this
is loopback evidence, not a WAN throughput guarantee. That suite failed on an
incorrect expected repair-fixture result, corrected in 2699c42.

The [2699c42 run](https://github.com/b1ue-man/smart-explorer/actions/runs/36425313803)
recorded 10/10/15 ms for the same analysis, then terminated without a Rust panic
during real drive access. That crash was unresolved at this checkpoint: a prior
mount pass did not establish reliability. Extend M5's same entrypoint with process exit codes,
Windows Application Error/WER events, a task-executable-only minidump, and
mount-stage traces. Remove credential environment variables from the isolated
fixture before enabling dump capture. Investigate the failed boundary, correct
its cause, and rerun only this suite before any release.

Primary diagnostic reference checked 2026-09-28:
[Microsoft WER local dumps](https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps).
No release is claimed at this checkpoint.

The [60c8dba run](https://github.com/b1ue-man/smart-explorer/actions/runs/36428857419)
exits with an access violation (`0xC0000005`) after successful alias reads,
read-only rejection, filesystem close and drive removal. The implicit runtime
and peer destruction boundary remains unproven. WER returned no dump or event;
that absence does not indicate a normal exit. Keep M5's single entrypoint and
add explicit destruction traces plus an external exception monitor attached
only to the owned fixture PID. Use the reviewed SHA-256-pinned, Microsoft-signed
[ProcDump](https://learn.microsoft.com/en-us/sysinternals/downloads/procdump)
to capture one minidump on an unhandled exception; the fixture's exit code still
controls acceptance. No production unload workaround is justified without the
crash location. The analysis timings in this run were 20/13/21 ms for local,
exporting host and GUI completion, again loopback with Direct running second.

Gap research checked 2026-09-28: Dokany closes its per-instance cleanup group
before returning from `DokanCloseHandle`, then its global pool at shutdown.
[CloseThreadpool](https://learn.microsoft.com/en-us/windows/win32/api/threadpoolapiset/nf-threadpoolapiset-closethreadpool)
can release asynchronously when outstanding objects remain; the dynamic loader
lifetime therefore needs direct evidence, not a guessed delay or permanent DLL
pin. Success requires all bounded mount lifecycles and the whole existing suite
to finish normally before terminal publication.

Related teardown defect found by source review: `RuntimeSelection::complete`
removes the private-runtime recovery marker before its runtime field is dropped.
A failure during shutdown/unload can therefore erase the evidence that should
select the official compatibility runtime on retry. Extend M5's controlled
teardown boundary: release the runtime before completing the marker, retain
cache ownership throughout, and exercise the production selection/completion
path in the real-volume case. The runtime must remain explicitly private in
that case, so compatibility fallback cannot conceal a private-runtime failure.

## Teardown evidence and final repair boundary

The [1a3273f dump](https://github.com/b1ue-man/smart-explorer/actions/runs/36431808950)
records an execute access violation at unloaded `smart-explorer-dokan2.dll`
RVA `0x9aa3`. Disassembly of the exact approved DLL maps that address to the
return from `BroadcastSystemMessage` inside `DokanBroadcastCallback` (entry
`0x9a20`). Runtime destruction had already returned. This is direct evidence of
code unloading while a notification callback is still executing.

Source review identifies the race: one I/O worker owns `DokanNotifyUnmounted`,
but every failing worker signals `DeviceClosedWaitHandle`. Another worker can
therefore start cleanup before the owner has queued its notification. Windows
documents that new members created during `CloseThreadpoolCleanupGroupMembers`
need synchronization and can miss that cleanup operation.

M11 extends the same task suite and the existing dependency preparation path:

- In `native/dokany-private/batching.patch`, only the notification owner signals
  device closure after submitting notifications. Serialize work creation and
  submission with a per-instance closing flag, set before cleanup begins.
  Rejected I/O work returns its batch/event resources without invoking a new
  filesystem callback. Private shutdown must then drain callbacks before unload.
- The official non-batched System32 fallback cannot receive this source patch.
  Before it creates any filesystem callbacks, pin that already loaded module
  by address until the isolated mount-host process exits. Filesystems, pools,
  cache and callback context still close normally; the late notification uses
  only its encoded event data. Preflight without filesystem creation stays
  unloadable. The corrected private DLL remains normally unloadable.
- Extend the real-volume case with concurrent directory requests during close,
  repeated private and official lifecycles, private unload and recovery-marker
  completion checks. Keep all prior Direct analysis/repair cases in this suite.
- The same remote entrypoint uses `prepare-dokany-private.ps1` only for this
  affected dependency, caches its exact recipe-bound output, then embeds its
  verified hash in the single incremental library target. Successful acceptance
  retains that exact DLL/manifest/source set for the terminal release; release
  never rebuilds the dependency. No extra workflow or local build is introduced.

Gap review completed 2026-09-28 against the pinned C source and Microsoft
[cleanup-group synchronization](https://learn.microsoft.com/en-us/windows/win32/api/threadpoolapiset/nf-threadpoolapiset-closethreadpoolcleanupgroupmembers),
[module pinning](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-getmodulehandleexw)
and callback-lifetime documentation. The acceptance signal is normal completion
of the entire existing suite with these lifecycles, exact dependency hashes and
no native crash. The old dump remains failure evidence, not acceptance.

## Accepted dependency checkpoint, 2026-09-28

[Run 36435534429](https://github.com/b1ue-man/smart-explorer/actions/runs/36435534429)
completed the whole task entrypoint normally at
`9398fcf433390991641ba91c4e6b0b86565c5003` (exit `0x00000000`). The private and
official volume lifecycles, concurrent directory access during close, alias
content/permission checks, private unload and recovery-marker completion all
passed. Direct repair and GUI/worker/QUIC analysis completed in the same run.
The measured local/host/GUI durations for 5,289 files were 17/14/23 ms, with
zero per-file metadata calls. Direct ran second on loopback with a warm cache;
these figures do not predict WAN completion time.

The exact accepted dependency set is retained under `native/assets/dokany-private/`:

- DLL SHA-256: `d05a3c8ad19038b48808fc13a5ed097f2d277ae873f8bb1ccc1fa609017f4df1`.
- Corresponding-source ZIP SHA-256: `8ba3d9b2f870a334153e9748eaa9df01098ebe075ddacca3160482eb2317f880`.
- Recipe SHA-256: `5f24957ac1d9f26bfcdb1e59d37016fd412409b304dbff255333f545a498bbd1`.
- Patch SHA-256: `95da1312e99c54aaef06c29a8f39ce3441e901db8a2065a1b4f8b0409dcc0653`.
- Fixture executable SHA-256: `54da33431a1b638a4355043d9e78d67f6710772457b3fb64b1abf8bc4e775a72`.

The archived manifest, patch, builder and patched callback sources were compared
with the candidate before retaining the files; the source ZIP is unchanged.
This checkpoint approved the dependency, not a release. M9's later source-identity
capacity correction in `2f99d90` required the same suite on the combined candidate.

## Complete candidate accepted, 2026-09-28

[Run 36438291005](https://github.com/b1ue-man/smart-explorer/actions/runs/36438291005)
passed the same entrypoint at `b6a7b2a272258de7558752d39be2769533bbdd0b` with
exit `0x00000000`. This includes M9's source-identity capacity correction, all
mount/private-and-official teardown behavior and the Direct/GUI analysis path.
The accepted dependency files match the committed DLL, manifest and source ZIP
byte for byte. The fixture executable SHA-256 is
`4a3bf679db8095a8bc278114fd8f35ff84a0e025f1e5c327b619528daba37cc7`.
Measured local/host/GUI analysis durations were 12/14/23 ms for 5,289 files,
with zero per-file metadata calls and the same loopback/warm-cache limitation.
All implementation milestones are accepted; only terminal publication remains.

No local builds, compilers, native formatters or tests. Static parsing and diff
inspection only during implementation. The one remote suite must have at least
30 minutes; terminal release uses the existing six-hour remote job and
`native/publish-release-local.ps1`, never a workstation invocation.
