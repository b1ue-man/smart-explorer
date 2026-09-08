# Remote mount bulk-access correction

Code-first follow-up to [the Obsidian startup inspection](MOUNT_OBSIDIAN_STARTUP.md).
The only live status is D4 in [TODO.md](TODO.md). Baseline: `bad4e2f`, inspected
2026-09-08. This is an implementation plan, not a runtime acceptance claim.

## Goal and execution boundary

Correct demonstrated API and scaling defects across callback admission,
enumeration, metadata scheduling, rooted resolution and transport admission.
A 10,000-entry structure must not cause repeated whole-handle/tree scans for
each entry or each continuation. Retention limits govern disposable cache, not
whether valid directory results can be returned. Preserve confinement, sharing,
read-only admission, dirty-file recovery, and the official-runtime fallback.

The user explicitly requested manual source/API validation and correction before
discussing further execution. No local or remote builds, tests, application/VM
runs or release workflows are authorized in this phase. Commit and push source
milestones with `[task candidate]`; do not bump the version. The later single
remote task suite/release remains deferred, not silently replaced with local
commands. Release observations remain at least half an hour apart.

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
   deferred in this code-only phase; old assets cannot approve the new patch.

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

8. **Manual integration handoff.** Inspect all mutation hooks, API signatures,
   ownership/lock order, patched upstream context and complexity against the
   milestones. Perform only non-executing text/parsing checks; refresh the root
   Graphify graph after native changes. Commit coherent milestones and push the
   candidate. Report remaining dependencies plainly, including unprepared DLL
   bytes and absence of Windows/runtime acceptance. No test or release trigger.

The expected results above define later acceptance questions, not permission to
execute them now. If execution is subsequently agreed, consolidate these and
the existing lazy-read-only guard into the one checked-in remote mount suite,
with cold names-plus-every-child-lstat, realpath and the recursive watcher before
warming. Do not substitute a Dirent scan, raise the application's thread pool,
or publish intermediate milestones as separate releases.

## Source handoff, 2026-09-08

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
