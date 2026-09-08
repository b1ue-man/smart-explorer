# Remote mount bulk-access correction

Code-first follow-up to [the Obsidian startup inspection](MOUNT_OBSIDIAN_STARTUP.md).
The only live status is D4 in [TODO.md](TODO.md). Baseline: `bad4e2f`, inspected
2026-09-08. The plan and source-only checkpoint below are retained history;
the exact remote acceptance and publication record follows.

## Goal and execution boundary

Correct demonstrated API and scaling defects across callback admission,
enumeration, metadata scheduling, rooted resolution and transport admission.
A 10,000-entry structure must not cause repeated whole-handle/tree scans for
each entry or each continuation. Retention limits govern disposable cache, not
whether valid directory results can be returned. Preserve confinement, sharing,
read-only admission, dirty-file recovery, and the official-runtime fallback.

The source-first review ended at `53dc64f`. On 2026-09-08 the user requested a
build to try the corrections. The resulting installable Windows release is
0.5.154, following one remote task suite and the existing terminal release
wrapper. Local builds and test execution remain prohibited. Source
milestones use `[task candidate]`; only the terminal wrapper bumps the version.
Release observations remain at least half an hour apart.

## Accepted exact candidate, 2026-09-08

The complete existing remote suite passed in
[run 34218461978](https://github.com/b1ue-man/smart-explorer/actions/runs/34218461978)
at `0bfdc0665ba746493304030dc18e89bde11e36d6`. Its `approval.json` records `PASS`.
The exact prepared dependency was copied into the canonical tracked asset paths,
not rebuilt or relabeled. Before import, raw payload/source hashes, sizes and
normalized recipe/patch/builder bindings were compared to the approval/manifest.

| Retained input | SHA-256 |
| --- | --- |
| `dokan2.dll` | `985735e1a36e33393f0ed7b733c17185953ed746e27232714a639b9447721b33` |
| `manifest.json` | `6e4f32962d054f9f68df0b12e8a5beda8f1c4ea4ef0bcda2183e75a3a7c2aa0e` |
| `corresponding-source.zip` | `7c7f7d3862419f74e9df1efd1c0052668450d521274852994f3f3c931190dbfd` |

Provenance: VS `17.14.37614.0`, MSVC `14.44.35207`, SDK `10.0.26100.0`, v143/static
CRT; upstream/recipe/patch/builder identities are in the retained manifest.
The approving fixture SHA-256 is
`f880e6ddd77919e38b2fa3b7cba6f25ed946fbf816a34924a4e30cf21cc2b66c`.
The official System32 DLL stayed
`75600aba867acbdfdb85fcd142b524da769bdc611b855a760aeb0c6e2eaae17a` and driver stayed
`9549a20e63c22a2b068e635600b65f6b55d8be5122a6623997b1274a1a1f6235`.

Both runtime modes completed cold Node names/lstat demand over 66,385 files and
4,608 descendant directories, with no backend content reads during metadata.
This run took about 16 seconds per cold pass; it is not a before/after benchmark
or a universal performance guarantee. Exact native continuation, callback-error,
creation-attribute, sharing/cache/scheduling contracts and save/replace/watch
behavior passed. Mounted PowerShell main/helper/child/data access completed in
535 ms privately and 413 ms officially; local startup took 4,641/308 ms. The
earlier single PowerShell timeout was not reproduced or explained by this run;
its added diagnostics and corrected fixture cleanup are not a claimed product
fix for that timeout. Private continuation was nonzero; official batching stayed
disabled. Recovery/teardown completed in both modes. Actual Obsidian/user-remote
certification remains outside this synthetic workload.

Publication completed on 2026-09-08: the existing terminal remote wrapper
[run 34220047698](https://github.com/b1ue-man/smart-explorer/actions/runs/34220047698)
consumed source `b66e68027c71c9eeb58399ee92daf7831594cba2`, including these exact
retained dependency bytes, and published
[v0.5.154](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.154) from
release commit `6f828e8228d88c7877f2a3b5e470a891fcc9a294`. Its single internal
[publication consumer](https://github.com/b1ue-man/smart-explorer/actions/runs/34227165765)
also succeeded. Cargo/feed version, matching installer/tag, visible non-draft
Release, all six payload hash sidecars, and all 18 published asset sizes and
SHA-256 digests were checked against the committed files. Installer SHA-256:
`8dbad9a7a57c03aa8b09130e40791b255e334b5a881e138ddee654db8f703055`.
No local build or test execution was used. This completes candidate delivery,
not certification of the user's actual Obsidian vault or physical remote.

## Resumed delivery plan, 2026-09-08

Stage-one inspection found that the existing mount task only accepts the old
approved DLL, although the new source recipe requires different bytes. Its
Node phase also runs after native cache warming and raises the worker pool.
Some old cache assertions describe authority that the correction intentionally
removed. These are gaps in the existing acceptance path, not new product scope.

The generic Windows 2025 inventory still listed VS 2022, but actual run
`34210045542` selected `windows-2025-vs2026/20260824.214.3` and failed its VS17
preflight before compiling. The actual job identity overrides that inventory.
The same task entrypoint now provisions missing VS 2022/v143 using Microsoft's
fixed, SHA-256/signature-verified 17.14.39 Build Tools bootstrapper in a dedicated
runner-only path. It preserves the DLL toolchain contract, existing VS instances
and the single pipeline. Microsoft's [installer switches](https://learn.microsoft.com/en-us/visualstudio/install/use-command-line-parameters-to-install-visual-studio?view=vs-2022)
and [component IDs](https://learn.microsoft.com/en-us/visualstudio/install/workload-component-id-vs-build-tools?view=vs-2022)
were checked 2026-09-08: quiet/wait/no-restart, required C++ workload plus explicit
x64 compiler and Windows SDK. A recipe-bound prepared dependency skips setup.
The second gap review checked Microsoft's
[directory-query contract](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntquerydirectoryfile)
and [handle-information errors](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandle).
Continuation needs an explicitly bounded native buffer, and metadata failures
must reach the callback on an already-open handle to exercise DLL dispatch.
Ordinary directory iteration and an unsupported by-name probe cannot substitute.
The pinned [Node 24.20.0 filesystem API](https://github.com/nodejs/node/blob/v24.20.0/doc/api/fs.md)
was checked for names-only `readdir`, per-child `lstat`, `realpath` and recursive
`watch`; the application-pattern phase keeps the inherited/default libuv pool.

Final execution milestones, depending on the source corrections below:

1. Extend only `test-mount-optimization-task.ps1`, using its unchanged workflow, to
   explicitly prepare or safely reuse recipe-bound private bytes. Preserve the
   approved-only inputs for ordinary consumers and exact System32 identities.
   Expected: one remote incremental library target embeds the new DLL, and the
   suite exports that exact DLL, manifest, source archive and candidate approval.
2. Consolidate acceptance into its existing `mount_vault_task` filter: cold
   realpath/watch/names-plus-every-child-lstat without a thread-pool override;
   native continuation/status/open-attribute boundaries; aggregate sharing and
   reset scheduling; host/generic fresh authority and preload cursor ownership;
   entry overlay/retirement indexes and progressing FIFO transport admission.
   Reuse existing fixtures, including the wide/nested tree, and extract narrowly
   scoped child modules where needed. Expected results are the seven behavioral
   milestone contracts below; do not infer algorithmic complexity from timings.
3. Commit and push all implementation and suite changes, refresh the native
   graph, then invoke the one existing remote suite. Evaluate its logs; any
   correction reuses that same entrypoint and incremental output, not new gates.
4. Import only the exact passing dependency artifact, bind its approval to the
   candidate/run/hashes, and commit/push it. Invoke the configured remote
   `publish-release-local.ps1` automation once after its normal preflight. It
   owns versioning, both-platform artifacts and publication. Expected: one
   visible release and Windows installer containing the corrected source/DLL.

The cold synthetic filesystem workload checks the affected application access
pattern. It is not a claim that actual Obsidian has opened the user's vault.

Acceptance mapping within the single `mount_vault_task` invocation:

| Source milestone | Focused acceptance boundary |
| --- | --- |
| 1: sharing/open contracts | `handle_state/*task_tests.rs`; real-request `vault_open_task_tests::exercise` checks ignored attributes, rejected creation and binary-ID ordering without content reads. |
| 2: private DLL | `vault_volume_enumeration::exercise` checks 10,000 matching one-entry continuations, restart/filter/EOF/overflow; `vault_volume_errors::exercise` checks already-open metadata failure dispatch, with official behavior recorded separately. |
| 3: host metadata | `bulk_metadata_task_tests`, `metadata_preload_task_tests`, existing metadata/flight/scheduler cases check publication fences, exact sibling-stat calls, cursor/retry/revision ownership and cache reuse. |
| 4: overlays/retirement | `entry_table_task_tests`, `retirement_queue_task_tests` check dense parent indexes, weak identity batches, final pins and clean/dirty/recovery spool ownership. |
| 5: admission/SSH | `mount_gate_bulk_task_tests`, selected existing gate cases and `server_bulk_task_tests` check FIFO, progress versus inactivity, cancellation, fairness and completed-worker capacity. |
| 6: below-preload authority | VFS `vault_cache_task_tests` checks fresh unretained observations and unrelated flights; `vault_task_backend` checks fresh terminal stat reuse and rooted daemon/TCP integration. |
| 7: callback supervision | `callback_timeout/*task_tests.rs` checks indexed resets, actual lease registration beyond the former cap, claim/finish ordering and no stale rearm. |

The mounted integration starts cold Node names/lstat demand with its recursive
watcher already installed, checks delivery to that same watcher after an owned
save, then exercises native metadata, external notifications, replacement saves,
sibling scripts and orderly teardown in both runtime modes. No standalone
per-milestone invocations are added.

Run `34211045997` prepared the DLL and built the Windows fixture. Its private
runtime completed the cold Node metadata traversal, then the owned-save watcher
check failed: events arrived, but the expected pathname was not recognized.
Counts alone cannot distinguish a filename mismatch from missing target events.
The same case now retains bounded raw event/name samples without weakening its
success condition. The pinned [libuv event implementation](https://github.com/libuv/libuv/blob/v1.52.1/src/win/fs-event.c)
and Node filename contract were reviewed on 2026-09-08; the actual returned names
remain a runtime question, not grounds for changing production notifications.

That run also exposed a fixture-only eviction error. Case-insensitive cache
identity uses Windows ordinal uppercase; the test passed literal `/a` to an
internal exact-key removal hook storing `/A`, so no removal occurred. The case
now derives `cache.key("/a")`, asserts presence before and absence after removal,
and retains its original exact-child rearm/cursor expectations. Production
eviction already supplies canonical keys. These corrections use only the same
remote suite and retained dependency; they are not a release approval.

Run `34213200355` confirmed the canonical-key correction. Its watcher samples
identified the other failure: creation/change events named
`\watch\node-startup-check.md`, while removal named
`watch\node-startup-check.md`. This matches the pinned libuv code's existing-name
long-path expansion versus raw removal-name branch when watching a drive root.
The fixture now compares resolved Windows path identities against the exact
owned target, preserving raw diagnostics and the delivery deadline. It does not
accept unrelated events or change production notifications. The next invocation
uses that same complete suite; no dependency rebuild or release is authorized
by either failed run.

Run `34214582060` delivered the controlled Node notification and completed the
native metadata, continuation and status-propagation phases. Its overwrite
attribute case failed; the Win32 truncation probe had not established the native
create disposition it expected to exercise. [Rust at the recorded compiler](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/library/std/src/sys/fs/windows.rs)
uses `TRUNCATE_EXISTING` for this exact
option combination, not its separate manual `OPEN_ALWAYS` truncation branch.
No production change follows from assuming how that Win32 request is translated.
The revised case explicitly calls [NtCreateFile](https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntcreatefile)
with `FILE_OVERWRITE`, synchronous non-directory options and owned native storage,
and verifies the received callback disposition before asserting rejection.
Microsoft's object attributes, Unicode-string, I/O-status and synchronous-call
contracts were rechecked 2026-09-08. The remaining Win32 existing-object/create
cases and backend-content invariants remain unchanged in the same suite.

Run `34216455307` confirmed native disposition 4 and `STATUS_NOT_SUPPORTED`,
then passed external create/change/delete notifications and the save/replace
workload. It stopped at the existing 30-second mounted PowerShell deadline.
That error alone does not identify shell startup, script loading, or a blocked
filesystem callback. The next same-suite candidate adds a local owned `-File`
startup control, flushed runtime/script phase messages, and bounded owned-PID/script-path
callback entry/exit diagnostics that retain the original callback chain. The
mounted invocation, its 30-second deadline and its real sibling script/data
accesses remain unchanged. No production timeout is raised.

Inspection also found a fixture cleanup coupling: dropping its child wrapper
waited for PowerShell before the coordinator could unmount a stuck filesystem.
The coordinator now owns child cleanup, stops further work, closes the mount,
and then boundedly reaps the child before another runtime can start. The CLI,
automatic-variable and process-termination contracts were reviewed against
Microsoft's [Windows PowerShell 5.1 CLI](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_powershell_exe?view=powershell-5.1)
and [.NET termination](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.process.kill?view=netframework-4.8.1)
documentation on 2026-09-08. The local control runs with the mount live; provider
startup may inspect drive roots, so its owned PID is traced in both phases. Expected
acceptance is the same mounted script result, with diagnostics sufficient to
locate any further delay; a local startup result cannot substitute for it.

## Stage one: current source findings

- `handle_state/validation.rs` scans every live handle under global locks on
  each file admission. N retained unrelated opens require at least N(N-1)/2
  record visits; grouping by path alone would leave repeated same-file opens
  quadratic.
- The pinned Dokany `directory.c::MatchFiles` starts each continuation at vector
  position zero. Small fixed buffers repeatedly revisit the delivered prefix.
  Smart Explorer supplies `FindFilesWithPattern`, so its successful callback
  results already have the pattern applied.
- `callbacks_open.rs` rejects creation-only attributes before determining
  whether the operation only opens an existing object. The pinned Dokany
  `fileinfo.c` separately replaces genuine metadata callback failures with
  `STATUS_INVALID_PARAMETER`.
- `metadata_schedule.rs` sorts all retained directories for every refresh and
  rescans every cached listing for every small preload batch. The existing
  child-name index does not eliminate these scheduling costs.
- `metadata.rs::overlay_listing` scans all materialized entries per directory.
  `entry_lifecycle.rs` sweeps all attached/detached entries on zero-pin events,
  including repeated linear pointer deduplication.
- `mount_request_gate.rs` has no same-class FIFO and broadcasts wakeups to all
  waiters on admission/release. Its absolute queue deadline can reject a large
  healthy backlog: 10,000 delivered cold requests, eight slots and 10 ms per
  service exceed ten seconds before the last request is admitted. This is a
  queue counterexample, not a claim about actual Obsidian callback concurrency.
- `agent_proto/core/server.rs` reaps workers before blocking on input, then
  checks their old count after input arrives. Completed workers can therefore
  produce a false capacity error.
- `vfs/core/cache_load.rs` removes an older listing only if its successful
  replacement fits retention. An oversized fresh result can leave older cached
  contents authoritative. Unrelated mutation invalidation also discards other
  in-flight directory completions through a global generation.
- Rooted resolution already uses indexed case lookup below preload. It still
  repeatedly constructs growing prefixes, and a cold case-sensitive stat can
  fetch the final metadata during validation and again for the result.
- The callback-timeout supervisor scans all active requests on every wakeup and
  again for each due reset, taking each request mutex under its global mutex.
  It also rejects the 4,097th live callback solely by a fixed count. This is
  live-request bookkeeping, not a cache-retention or directory-validity budget.

The wire encoder/decoder visits every result once. A 10,000-entry listing with
32-byte UTF-8 names and no MD5 occupies 550,013 body bytes (550,017 framed), below
the existing 64-MiB frame bound. The decoder's 4,096 initial vector capacity is
not an entry limit. Increasing frame/channel/worker limits is not the fix.

## Research and second gap review

Primary sources checked 2026-09-08:

- [ZwCreateFile contract](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-zwcreatefile):
  creation attributes are ignored when nothing is created or overwritten;
  binary open-by-ID names must be rejected before string interpretation.
- [IoCheckShareAccess](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-iocheckshareaccess):
  sharing belongs to the file object, with synchronized updates. Aggregate
  requested-access and denied-sharing counts can implement the existing masks.
- [Pinned Dokany directory implementation](https://github.com/dokan-dev/dokany/blob/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd/dokan/directory.c)
  and [metadata dispatch](https://github.com/dokan-dev/dokany/blob/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd/dokan/fileinfo.c):
  resume directly at `FileIndex` only when library-side filtering is disabled.
  With library filtering, matched-result indices are not raw vector positions.
  Preserve buffer-overflow, EOF, restart and callback status semantics.
- [Rust collection costs](https://doc.rust-lang.org/std/collections/),
  [condition variables](https://doc.rust-lang.org/std/sync/struct.Condvar.html),
  [completed thread detection](https://doc.rust-lang.org/std/thread/struct.JoinHandle.html#method.is_finished):
  hash traversal visits capacity, condition-variable wakeups require predicate
  rechecks under their one mutex, and completed workers can be reaped without
  waiting for an active operation. Retain randomized hashing.

Queue progress is not backend cancellation. A genuinely blocked synchronous
provider operation cannot be safely killed by forgetting its client request.
The admission correction must preserve no-progress timeouts and must not claim
to repair arbitrary hung backend workers. No unbounded thread spawning or
speculative worker-count increase is planned.

The supervisor follow-up was gap-reviewed before editing: all leases use one
fixed reset interval. Arm deadlines under the scheduler mutex, in monotonic
order, to allow an indexed FIFO rather than whole-table minima. The
[pinned timeout implementation](https://github.com/dokan-dev/dokany/blob/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd/dokan/timeout.c)
dereferences the request pointer during reset. Therefore removal must still
wait for a claimed reset; queue optimization must never shorten that lifetime.
[Instant's contract](https://doc.rust-lang.org/std/time/struct.Instant.html)
was checked on 2026-09-08; scheduling must keep its own nondecreasing order.

Complexity accounting includes input path/name bytes and returned metadata, not
only object count: revalidating a path's ancestors is required for confinement.
Balanced-tree prefix/expiry maintenance retains logarithmic index costs. The
deliverable removes repeated full scans/sorts from bulk per-entry paths; it is
not an unsupported claim that every filesystem operation, arbitrary-depth
path or unchanged official DLL has constant cost.

## Stage two: behavioral milestones and acceptance signals

1. **Constant-work sharing admission and correct open attributes.** Extract a
   per-path aggregate sharing index from Windows `handle_state`; update it on
   reserve, cleanup, abort, close, detach and rename. Both many independent paths
   and many opens of one path must avoid walking handle records. Preserve
   unbound reservations, MAXIMUM_ALLOWED fallback and directory exemptions.
   In `callbacks_open`, validate path options early and attributes only for
   actual create/overwrite outcomes. Expected: unchanged allowed/denied sharing
   decisions, idempotent cleanup, no stale counters after replacement, and
   existing metadata opens accept ignored creation attributes without writing.

2. **Linear private-DLL continuation and faithful metadata errors.** Extend
   `native/dokany-private/batching.patch` with the reviewed unfiltered-vector
   cursor and error-preservation corrections; update its recipe hash and
   corresponding-source documentation. No ABI, driver or System32 change.
   Expected: one-entry/small-buffer continuation does not revisit earlier
   entries; restart/filter/EOF/overflow remain correct; real metadata errors
   survive dispatch. Dependency: changed recipe requires newly prepared remote
   DLL/source bytes before a normal Windows build. Existing approved assets
   must not be relabeled or have their hashes falsified. Source preparation is
   now belongs to the resumed remote delivery plan; old assets cannot approve
   the new patch.

3. **Incremental metadata scheduling.** Replace whole-cache refresh sorting
   with maintained age/demand/recent ordering, and repeated preload scans with
   per-snapshot revision/cursor work. Couple insert/remove/expiry/cooldown hooks
   to scheduling in the metadata-cache modules. Do not retain uncharged old
   snapshots or unbounded stale tickets. Expected: a wide/nested stable tree is
   inspected once per snapshot revision, not once per small worker batch;
   shallow speculation, recent demand, cold fairness, retry and invalidation
   remain available. Demand itself has no preload-depth limit.
   Final gap resolution: snapshot-owned child records are charged to that
   snapshot and rearmed by exact parent/child lookup on eviction or cooldown
   expiry. Selection tickets carry parent revision; cancellation cannot lose
   selected work. Separate scan progress from successful loads so a bounded
   discovery turn does not accidentally put the worker to sleep. Temporary
   snapshot removal/rollback must preserve scheduler ownership. Exact alphabetic
   tie ordering is not required; starvation freedom and recent demand are.
   Final publication review also found the fresh-authority defect in this host
   cache: retention/notification pressure must not leave an older successful
   observation authoritative. Keep its comparison image, expire its authority
   only after validating the new observation's load/ticket fences, and fence
   affected point/descendant results. Failed or stale fetches must not do this.
   An explicit publication outcome carries the exact completed load revision:
   rejecting retention must still fence older fetches waiting to publish. A
   comparison-only image must not trigger an implicit whole-parent reload for
   each subsequent sibling stat; direct point requests remain available.
   Unchanged retained-parent refreshes must not repeatedly cancel child loads.
   Recheck snapshot authority atomically during point installation; use blanket
   direct-flight fencing only where the new listing cannot be retained.
   Eviction, invalidation and subtree reconciliation must carry retired wide
   snapshots out of the global locks before destroying their last references.

4. **Indexed live-file overlays and candidate retirement.** Encapsulate entry
   insertion/removal in a parent-indexed table. Replace global retirement sweeps
   with deduplicated weak candidates from final-pin and clean/delete/recovery
   transitions. Affected boundary includes engine initialization,
   materialization, namespace mutations and recovery. Expected: listing a
   directory considers only its local overlays; draining N held files does not
   rescan N remaining files after every close; dirty/recovery-referenced spools
   survive and failed cleanup is retryable.

5. **Work-conserving, fair admission and current worker accounting.** In the
   mount request gate, queue each class FIFO and directly notify admitted
   waiters instead of broadcasting. Bound lack of service progress, not total
   age in a progressing metadata backlog; preserve transfer stall protection,
   metadata/transfer fairness and advertised active-request capacity. Reap SSH
   workers after input arrives and before admission. Expected: no same-class
   overtaking or false full-pool response from already-completed workers; a
   healthy backlog drains, while a genuinely non-progressing queue times out.

6. **Fresh authority and efficient rooted lookup below preload.** In generic
   VFS cache acquisition, retire an old exact listing after a successful
   current-generation refresh even if the replacement cannot be retained.
   Fence affected directory flights individually for ordinary mutations;
   preserve prefix/global fencing for rename and explicit invalidation. Retire
   heavy snapshots outside the global cache lock. In rooted resolution, append
   path components in place and reuse a fresh final stat for both validation
   and result without trusting stale listing metadata. Expected: valid large
   listings are returned completely; old authority cannot reappear; unrelated
   writes do not restart unrelated cold loads; confinement/collision checks
   and failure propagation are unchanged.

7. **Incremental callback supervision.** Replace whole-request minima with
   an indexed FIFO of fixed-interval reset deadlines, assigned under its one
   mutex. Registration, removal and rearming inspect only the affected records;
   completion cannot leave stale queue entries. Keep the existing reset API,
   claim/finish pointer lifetime, failure reporting and shutdown behavior.
   Remove the arbitrary live-callback count rejection; memory tracks actual
   active leases, with no new worker per callback. Expected: N concurrent
   registrations/completions and one reset round require expected O(N)
   bookkeeping, not repeated N-record scans; finish/reset races never use a
   completed Dokany request pointer.

8. **Manual integration handoff (completed source-first phase).** Inspect all mutation hooks, API signatures,
   ownership/lock order, patched upstream context and complexity against the
   milestones. Perform only non-executing text/parsing checks; refresh the root
   Graphify graph after native changes. Commit coherent milestones and push the
   candidate. Report remaining dependencies plainly, including unprepared DLL
   bytes and absence of Windows/runtime acceptance. No test or release trigger.

The expected results above define the resumed remote acceptance questions.
Consolidate these and the existing lazy-read-only guard into the one checked-in remote mount suite,
with cold names-plus-every-child-lstat, realpath and the recursive watcher before
warming. Do not substitute a Dirent scan, raise the application's thread pool,
or publish intermediate milestones as separate releases.

## Historical source-only handoff, 2026-09-08

This records the earlier `53dc64f` checkpoint, superseded by remote acceptance above.

The source candidate implements the milestones above. Sharing admission now
uses per-path access counters; directory overlays use dense parent indexes;
last-pin/clean transitions enqueue affected retirement candidates. Refresh
and preload scheduling maintain their own ordering/cursors, and callback timeout
resets use an indexed deadline FIFO. The transport gate admits same-class work
FIFO and measures missing metadata service progress instead of total queue age.

Both metadata-cache layers retire obsolete authority after a successful current
observation, independently of retention. Host comparison-only images preserve
notification history without answering reads or forcing whole-parent fetches
for each sibling stat. Publication returns the exact load revision, and wide
snapshot destruction occurs after releasing global locks. Rooted stat reuses
its fresh terminal observation; path confinement remains enforced.

Manual review covered changed call sites, ownership and invalidation hooks,
including cancellation, rejected preload retry, same-path rename and callback
pointer lifetime. Static Rust AST parsing and source-size checks found no syntax
or size violations; recipe JSON and the patch digest agree. The complete private
patch also passed a non-applying context check against the hash-verified pinned
upstream archive. These checks do not perform Rust type checking or establish
Windows runtime behavior.

No build, test, application run or release was started. Existing private DLL
assets remain unchanged and cannot satisfy the new recipe: remote dependency
preparation and the later agreed task-level Windows acceptance are still needed
before distributing a working binary candidate. The official DLL/driver and
compatibility fallback are unchanged. Remaining global complexity boundaries
are described above; no measured speedup or universal O(N) guarantee is claimed.
