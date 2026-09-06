# Remote vault metadata: follow-up design and evidence

Task evidence, not a second live board; open status belongs to [TODO.md](TODO.md).
Baseline: `905e442`, inspected 2026-09-06. The user reports slow/non-completing
Obsidian startup for a nested remote vault in both runtime modes after v0.5.151.
The earlier small synthetic mounted fixture did not exercise this scale or the
daemon/rooted-backend path. No particular Obsidian internal call sequence, measured
speedup, or user-specific root cause has yet been established.

## Stage one: source findings

- Host `metadata_loading::filter_listing` treats the 50,000-entry/16-MiB cache
  allowance as a filesystem validity limit. Valid wide directories can fail.
- Directory retention additionally stops at 4,096 directories. Installation and
  point observations scan unrelated retained paths under shared cache locks.
- Every parent snapshot install invalidates all active descendant loads, even
  when the parent's namespace is unchanged. Such children are returned but fail
   cache admission and must be fetched again.
- After a directory snapshot expires, stat falls back to individual child RPCs
  rather than refreshing the known parent once. An aggressive metadata pass can
  thus change from one listing per directory to one request per file.
- Same-path load locking shares work only when persistent admission succeeds.
  Under retention/notification pressure, waiting enumerations repeat the fetch.
- Daemon case resolution uses `CachingBackend::directory_snapshot`, which has
  no same-directory load sharing. Concurrent cold paths can repeatedly list the
  same ancestors. Expiry and eviction scan the whole retained cache.
- Background work starts fixed chunks and waits for their slowest member before
  refilling. Its four-worker ceiling is additional to backend admission. This is
  not the foreground callback limit.
- Ordinary Windows attribute opens and enumeration are already lazy/cached.
  Host `open_host_cache` disables per-callback raw ancestor checking; authorization
  remains enforced by the daemon's rooted backend. Do not remove root/link checks.
- Agent-backed SSH wrongly inherits raw SFTP's walk-width hint of one as mount
  admission capacity. However remote-agent worker accounting and shared stream
  backpressure prevent proving that simply increasing it is safe. This task must
  not silently change that protocol contract or claim to have removed that limit.

Initial direction: correct cache semantics and avoid redundant/whole-tree work;
make independent background work work-conserving; exercise the actual layered
mount path under recursive metadata demand. Leave private DLL bytes unchanged.

## Primary-source research, checked 2026-09-06

- [Obsidian Vault API](https://docs.obsidian.md/Plugins/Vault) describes a vault
  as a directory with subdirectories and permits Node filesystem access. It does
  not specify Obsidian's complete startup implementation.
- [libuv filesystem operations](https://docs.libuv.org/en/v1.x/fs.html) and its
  [thread-pool contract](https://docs.libuv.org/en/v1.x/threadpool.html) show why
  blocking a few filesystem calls can delay an application's other requests.
  SE must not change another application's thread-pool environment globally.
- [libuv v1.51 Windows source](https://raw.githubusercontent.com/libuv/libuv/v1.51.0/src/win/fs.c)
  uses attribute-only handle stat and directory-listing access, and attempts
  by-name metadata first. Cover these APIs without asserting the user's bundled
  libuv version. [Microsoft by-name contract](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getfileinformationbyname).
- [Rust HashMap traversal](https://doc.rust-lang.org/std/collections/struct.HashMap.html#method.iter)
  is capacity-proportional. [BTreeMap ranges](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html#method.range)
  permit affected-prefix traversal; [BTreeSet](https://doc.rust-lang.org/std/collections/struct.BTreeSet.html)
  provides maintained expiry/recency ordering without repeated whole-cache scans.
- [Mutex](https://doc.rust-lang.org/std/sync/struct.Mutex.html#method.lock) and
  [Condvar](https://doc.rust-lang.org/std/sync/struct.Condvar.html#method.wait)
  contracts require explicit lock ordering and predicate rechecks. Do not wait
  for backend work while holding global cache state or an opposing namespace lock.
- [Pinned russh-sftp request dispatch](https://docs.rs/russh-sftp/2.3.0/src/russh_sftp/client/rawsession.rs.html)
  has independent request identifiers; this does not establish the capacity of
  SE's separate remote-agent server or every SFTP server.

## Stage two: final behavioral milestones

1. **Scalable, correct host metadata retention.** `mount/core/metadata_cache*`,
   `metadata_point_cache`, `metadata_loading`, `metadata_changes`, and associated
   scheduling support. Remove entry/directory-count rejection from valid demand
   enumeration. Use byte-accounted retention (128 MiB directory snapshots,
   16 MiB point metadata), indexed affected-prefix/recency/expiry operations, and
   revision-valid sharing of completed concurrent loads even when not retained.
   Narrow descendant invalidation to actual affected names/identities. Notification
   memory stays bounded, concrete events remain retryable, and notification
   pressure must not multiply simultaneous foreground fetches. Keep fresh positive
   and negative authority, mutation invalidation, and root/name safety intact.
   Acceptance: complete old-limit-exceeding listings; >4,096 small directories
   remain reusable within budget; unchanged-parent races retain child results,
   removed/replaced ancestors reject them; concurrent pressure misses share work;
   unrelated insertion/reconciliation does not traverse the whole retained tree.
   For a previously observed but expired parent, share one parent refresh before
   falling back to individual child stats. Preserve point observations and fall
   back to exact stat when listing is unavailable; do not turn list permission
   into a new requirement for an otherwise permitted file stat. Acceptance:
   an expired directory's concurrent child-stat burst incurs one shared listing,
   not a per-child RPC burst, with unchanged fresh/negative/error semantics.

2. **Shared daemon ancestor loads.** `vfs/core/cache*` and the existing rooted
   backend integration. Add per-directory single-flight acquisition independent
   of persistent admission; retain the index with the shared snapshot, including
   a wide unretained snapshot. Replace repeated global expiry/LRU scans with
   maintained ordering. Mount instances use a 64-MiB byte budget without arbitrary
   directory/entry caps; keep UI defaults scoped and unchanged where possible.
   All mutating checks remain fresh and generation-fenced; no root validation or
   case-collision checks are weakened. Acceptance: simultaneous cold sibling
   resolution lists their shared ancestor once; unrelated listings overlap;
   mutation during a fetch cannot publish stale cached authority; pressure never
   becomes an enumeration failure or repeated index construction for waiters.
   The host already owns directory freshness. A host miss/refresh must therefore
   refresh the daemon's exact directory snapshot, while still reusing cached
   ancestor resolution. Otherwise two independent 20-second TTLs can present a
   nearly 40-second-old listing as newly fetched. Add a shared fresh-list path
   for this boundary; normal UI listing reuse remains unchanged. Acceptance:
   explicit host refresh observes external changes without waiting out another
   daemon TTL, and updates the daemon's ancestor index for subsequent children.

3. **Work-conserving background metadata.** `metadata_schedule`, a focused worker
   helper, `metadata_loading`, and Windows `metadata_refresh` if needed. Refill
   available workers without chunk barriers, preserving actual ancestor ordering,
   stopped checks, complete joins and backend capacity with foreground headroom.
   Avoid maintenance refetching a snapshot already refreshed after selection.
   Remove the redundant four-worker ceiling, not backend/resource safety limits.
   Acceptance: one stalled sibling cannot idle the remaining available workers;
   descendants remain fenced against ancestor replacement; a demand refresh
   satisfies previously selected maintenance without another remote request.

4. **One integrated acceptance and release.** After all implementation, update
   `native/test-mount-optimization-task.ps1` as the single task entrypoint with a
   focused `mount_vault_task` selection and necessary regression coverage. Add a
   generated nested/wide fixture through `RootedBackend -> backend_server ->
   AgentBackend -> MountProxy -> MountEngine -> Windows volume`, then cold/warm
   concurrent traversal via Windows and Node metadata APIs in both runtime modes.
   Discover runtime paths/drive/Node executable within the entrypoint/fixture.
   Record the exact manifest, backend request counts, maximum overlap, latency,
   callback errors, zero metadata-stage content downloads and clean teardown.
   Include a separate small mixed content/save/watch phase to catch directly
   affected integration regressions. Do not call it actual Obsidian certification.
   Commit/push all milestones, invoke this one remote suite, evaluate/fix within
   that loop, then one existing complete remote release transaction. No local
   build/test execution, DLL rebuild, intermediate release or duplicate pipeline.

## Second gap review and decisions

Ordered prefix ranges must respect slash boundaries and folded Windows keys;
`/a` must never invalidate `/ab`. Expiry remains absolute, not indefinitely
extended by hits. Shared load results need both revision and freshness checks;
they must not resurrect a removed/replaced parent or retain memory after the
last waiter. Prepared indexes belong to immutable observations rather than a
second cache lookup, which could select a newer/different listing.

Byte budgets constrain disposable retention, not valid filesystem size. The
existing configurable 500-MiB content cache and dirty/recovery durability are
unchanged. No unbounded proactive whole-share crawl is introduced. Conservative
transport admission, notification memory pressure and OS/path limits are not
blindly removed. The demonstrated SSH admission problem remains a separate
protocol-design boundary unless this task establishes an exact safe correction.

Status: plan finalized before implementation; remote acceptance not yet run.
