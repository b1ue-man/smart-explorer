# Remote-mount optimization: design evidence

This is task-scoped design/evidence, not a second live work board. Open work is
tracked in [TODO.md](TODO.md). Baseline inspected: source `80a75ef` (2026-09-06).
The user confirmed that the preceding request-delivery fix resolved the freeze.

Candidate status, checked 2026-09-06: `native/Cargo.toml` and
`release-native/update-feed/version.txt` still read **0.5.150**. The cache,
metadata/notification and private-DLL changes described here are source work,
not a shipped version. Their single remote suite, exact dependency approval and
terminal release remain pending. This document records design/evidence, not a
second live checklist or an Obsidian certification.

The [first remote attempt](https://github.com/b1ue-man/smart-explorer/actions/runs/34048575411)
stopped before DLL compilation or mount execution: preparation incorrectly
reported a BOM in the BOM-free committed recipe. The `StartsWith(string)`
overload uses culture-sensitive comparison; format-marker matching now explicitly
uses `StringComparison.Ordinal`, following [.NET's string-comparison contract](https://learn.microsoft.com/en-us/dotnet/standard/base-types/best-practices-strings)
(checked 2026-09-06). BOM rejection itself remains enforced. This failed attempt
does not approve any runtime artifact or mounted behavior.

## Complete requested batch

Improve local-like remote-drive use: repeated file access, an Obsidian-style
vault/save workload, scripts loading other mounted files, useful concurrency,
bounded configurable caching, and safe reclamation of disposable data. Explain
the previous batching workaround's performance implications without inventing
measurements. The user subsequently authorized attempting a corrected private
Dokany DLL **with an official-runtime fallback**, in the same release batch.

No workstation builds or tests are permitted. One checked-in task suite must run
on remote Windows CI after all implementation is committed/pushed. One complete
remote release follows successful evaluation of that suite.

## Stage one: source findings and implementation direction

These findings describe the inspected pre-implementation baseline above; the
candidate changes below address them and supersede those old source behaviors.

- `mount/core/delete.rs::cleanup_committed_entry` deletes clean downloaded data
  on last materialized close. A later first read downloads the whole file again.
  Keep a separate, bounded idle-clean cache, rather than enlarging the live-entry
  namespace overlay.
- `file_io.rs::materialize_at` releases ownership guards before the caller
  installs/upgrades its handle. Disposal currently checks only published handles.
  Resolve operation/handle ownership before introducing automatic eviction.
- `file_io.rs::materialize_fetch` ignores the completed copy length. Reject short
  or overlong content before admitting a clean spool.
- `metadata.rs::overlay_listing` locks every live entry before testing its parent;
  `flush_entry` holds an entry lock across upload. Select entries by their
  namespace-protected map keys first, so another folder's upload does not block
  this listing.
- Background metadata preload and refresh are serial. The proxy already admits
  bounded concurrent requests. Overlap independent targets within that capacity,
  preserving ancestor/descendant installation ordering and cancellation checks.
- Directory snapshots have no expiry; repeated missing point lookups are not
  cached. Add explicit freshness rules, short NotFound reuse, fair refresh, and
  capacity-aware speculation. Retain old snapshots only as non-authoritative
  change-detection baselines, not permanent positive/negative answers.
- Successful remote refresh does not emit Windows filesystem notifications.
  Bridge verified snapshot changes to notifications from the owning host loop;
  never pass its raw DLL instance to a worker that can outlive DLL closure.
- Full local-filesystem equivalence is not currently implemented: attribute/time
  setters, alternate streams and security setters remain unsupported. Exercise
  concrete script/read/save/replace behavior; do not label that Obsidian product
  certification.

The old workaround disables only IPC batching. Parallel Dokany callbacks,
transport concurrency and metadata caching remain enabled. More kernel/userspace
transactions can cost throughput in small-operation-heavy workloads; no measured
regression or numeric speed claim is established.

## Primary-source research (checked 2026-09-06)

- [Rust `io::copy`](https://doc.rust-lang.org/std/io/fn.copy.html) returns the
  transferred byte count; a clean EOF does not prove the advertised remote length
  was delivered. Keep synchronization and durable dirty-state ordering intact.
- [Rclone VFS caching](https://rclone.org/commands/rclone_mount/#vfs-file-caching)
  distinguishes disposable cache limits from open files that cannot be evicted.
  Its fingerprint discussion also makes explicit that metadata/hash availability
  affects change detection. Our reuse must validate the existing backend baseline
  and document its limits; metadata equality is not a cryptographic proof.
- [FUSE cache contracts](https://libfuse.github.io/doxygen/structfuse__config.html)
  distinguish bounded negative lookup caching from indefinitely retaining content
  on filesystems that can change externally. These are design comparisons, not
  evidence about this application's Windows implementation.
- [GetDiskFreeSpaceExW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getdiskfreespaceexw)
  accepts the actual cache directory and returns 64-bit caller-available space,
  including quota effects. It is a snapshot, not a reservation against other apps.
- [Temporary-file attributes](https://learn.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants)
  are caching hints, not an automatic pressure-driven deletion contract.
  [Disk Cleanup registration](https://learn.microsoft.com/en-us/windows/win32/lwef/disk-cleanup)
  likewise does not guarantee timely Storage Sense reclamation of a custom cache.
  Use app-owned limits and free-space checks, not packaging/cloud-placeholder
  changes or global Windows settings.
- [Obsidian's Vault documentation](https://docs.obsidian.md/Plugins/Vault)
  explains that filesystem notifications invalidate its external-change read
  cache. [Libuv's Windows watcher](https://github.com/libuv/libuv/blob/v1.51.0/src/win/fs-event.c)
  requests attribute notifications and maps modified actions to `UV_CHANGE`.
  This supports a watcher bridge, not certification of an unknown Obsidian build.
- [Pinned Dokany notifications](https://github.com/dokan-dev/dokany/blob/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd/dokan/dokan.c)
  require an absolute mounted path and a live instance. The official
  `DokanNotifyUpdate` uses the attributes filter, not last-write/size filters.
  Through-mount mutations are already notified by the driver and must not be
  duplicated in callbacks.

## Stage two: final milestone plan

Second-pass review resolved ownership handoff, stale snapshot provenance,
speculative admission races, notification lifetime/filter contracts, and private
DLL delivery. These milestones form one batch; dependencies describe integration
order, not permission to run separate suites.

### M1 — Selectable, persisted cache/runtime policy

Files: new `mount/core/cache_policy.rs`, existing `types.rs`, mount exports,
`daemon/os/shared/ipc_protocol.rs`, `app/core/mount_ui_draft.rs` with a small
settings renderer, and `cli/drive.rs` with a cohesive options extraction if needed.

Retained clean data defaults to 500 MiB per drive; accept 0–65,536 MiB, with zero
disabling retention. Old serialized configurations receive that default. Expose
the same setting in GUI and `--cache-mib <value>`. `--system-runtime` and the GUI
compatibility checkbox independently select the official system runtime rather
than automatic private selection.
Keep identity/credentials out of sanitized runtime policy.

Acceptance: default/backward-compatible decoding, invalid-limit rejection,
GUI/CLI/config/host agreement, and compatibility selection surviving retry.

### M2 — Safe reusable clean content and working-space protection

Files: new focused cache, space-admission, lifecycle and materialization modules;
`engine.rs`, `file_io.rs`, `open_handle.rs`, `delete.rs`, `mutations.rs`,
`replace.rs`, `metadata.rs`, `spool.rs`, `startup.rs`; Windows space adapter and
host wiring. Extract materialization/flush responsibility from oversized
`file_io.rs`; extract engine recovery construction if needed to keep files bounded.
Depends on M1.

Retire only attached, clean, unpinned objects into a separate idle LRU, bounded by
both configured bytes and 10,000 records. Reuse requires a fresh backend stat,
regular-file/type and existing baseline agreement, and valid local spool length.
Cap a clean content generation's reuse age at five minutes, so unchanged weak
metadata cannot extend one cached copy forever. This is not cryptographic remote
change detection when the backend lacks a content/version identifier.

Use explicit RAII operation/handle pins acquired before ownership guards are
released; pin destructors must not acquire namespace/state locks. Retirement
must not remove an object while an acquired operation can still dirty/read it.
Prepared/claimed idle data has exclusive ownership outside the eviction index.
Validate a live per-path namespace generation at installation; mutation invalidates
affected generations without waiting on materialization locks. Preserve existing
case/root checks, journals, synchronous remote commits and ambiguous-save recovery.
Preserve a lazy destination handle's pre-replace object before namespace promotion,
or fail the replacement without destroying either object.

Reject downloads whose delivered length differs from the advertised length;
bound copying to expected length plus one detection byte. Filter live overlay
map keys by parent before taking entry-state locks. Failed disposal retains
ownership/accounting instead of pretending disk bytes were freed.

Inject a typed caller-available-space probe from Windows; core does not call OS
APIs. Keep a 512-MiB reserve, conservatively account concurrent pending growth,
and reclaim idle clean data before downloads/local growth and during background
maintenance. Open and dirty/recovery data can exceed the retained-cache cap;
never evict it. Space queries cannot reserve capacity against unrelated apps.

Acceptance: reopen download-count reduction, stale/type/missing/short/overlong
rejection, LRU and space-pressure behavior, concurrent ownership/generation
races, protected dirty/conflict/delete/recovery bytes, independent-folder listing
during upload, and correct old-handle/atomic-replace semantics.

### M3 — Fresh, bounded and concurrent metadata

Files: metadata cache/loading/point/support modules, new focused scheduler/change
record modules as needed, and Windows `metadata_refresh.rs`. Depends on M2's
invalidation contract, but independent scheduling code can be developed alongside it.

Directory listing observations expire after 20 seconds; expired snapshots remain
only as comparison baselines. Positive point observations last five seconds,
exact NotFound observations one second. Other errors are never negative-cached.
Do not renew old directory metadata merely because its child listing is refreshed;
track provenance or obtain fresh metadata. Local mutation removes affected
positive/negative authority immediately.

Background width is `parallelism.clamp(1,8).saturating_sub(1).clamp(1,4)`.
Process ancestor waves before descendants, join every successfully spawned worker,
handle spawn/panic errors, and check stop between targets. This leaves nominal
capacity only on multi-request backends; it is not a strict foreground-priority
or backend-cancellation guarantee. Run due refresh before speculation, reserve
rotating refresh capacity, and admit speculation only into available capacity.
Final admission is atomic and rejected candidates must not invalidate descendants.
Account replacement sizes after subtracting the previous snapshot.

Capture successful current snapshot replacements atomically for bounded ordered
change delivery; initial/failed/stale/rejected installs emit no invented changes.
Keep prior comparison data when a point observation invalidates its authority.
The implemented queue retains at most 64 pending snapshot pairs with 64 MiB
conservative byte accounting. It drains concrete create/delete/modify records
in bounded batches; only a wholly undrained same-directory tail coalesces.
When that queue cannot represent a new diff, reject the cache commit and protect
its old baseline from unrelated eviction until pressure clears. Do not discard
child changes in favor of a root-only update that watchers might not rescan.

Acceptance: expiry through every lookup path, negative coalescing/invalidation,
sibling overlap and width limits, ancestor race rejection, non-starving refresh,
settled over-capacity preload, correct replacement accounting, and ordered diffs.

### M4 — Windows application integration

Files: Windows runtime notification adapter, host owner loop, bounded core change
delivery, and directly affected save/open handling. Depends on M2 and M3.

Drain successful remote diffs from the host owner loop and invoke documented
create/delete/update notifications only while the owned DLL instance is alive.
Keep all network work and DLL calls outside cache locks. Do not duplicate driver
notifications for through-mount mutations. Drain the bounded concrete snapshot
diffs described in M3; do not fabricate root rescan notifications on overflow.
Preserve the official DLL's attributes-only update limitation explicitly.
The 20-second refresh schedule is not an end-to-end notification deadline when
I/O is slow, unavailable or subject to queue backpressure.

Acceptance: actual Windows watcher delivery for external create/delete/modify,
fresh content after notification, unchanged/failed/raced refresh, bounded backlog,
safe unmount, through-mount save/replace, and a mounted script reading a sibling
and invoking another mounted script. No unsupported Obsidian certification claim.

### M5 — Corrected private DLL with an official compatibility fallback

Files: checked-in pinned source recipe/patch and remote-only preparation script,
verified DLL/provenance/source artifacts, `native/build.rs`, focused Windows
runtime staging/selection modules, `runtime.rs`, `dokany_abi.rs`, host wiring,
stable task/release entrypoints and canonical install/release documentation.
Depends on M1's compatibility policy; joins M4 at runtime ownership.

Normalize effective batching/SingleThread options before `DokanStart`; convert
the mask comparison to BOOLEAN rather than narrowing `0x1000`. Add one private
versioned instance counter query for completed second-or-later batch records, so
acceptance proves actual multi-event consumption rather than merely navigation.
Keep the official driver, MSI and System32 DLL untouched.

One source-bound remote preparation stage, `native/prepare-dokany-private.ps1`,
builds only the user-mode x64 DLL from
the pinned archive and committed patch. Capture toolchain/source/patch/payload
identity and corresponding LGPL source/notices. Embed the verified payload in
app/se so existing installer/update payload boundaries stay coherent. Retain the
successful suite's exact DLL bytes/provenance as approved repository artifacts;
the terminal release consumes and rehashes them, never silently rebuilding a
different private DLL. The approved set under `native/assets/dokany-private/`
is exactly `dokan2.dll`, `manifest.json` and `corresponding-source.zip`.
Release preflight uses `-VerifyOnly -RequireApproved`
and rejects bootstrap `SMART_EXPLORER_DOKANY_DLL_DIR`/`SMART_EXPLORER_DOKANY_DLL_SHA256`
overrides. GNU Rust consumers can embed the same x64 DLL without
MSBuild; release preflight must reject absent/mismatched approved inputs before
the expensive build. Ordinary developer builds may remain official-only when no
private payload is available, but release/task validation must not silently do so.

Stage under an audited hash-addressed executable-runtime directory, outside the
disposable remote-content LRU. Verify expected SHA-256 immediately before loading,
hold write/delete-denying file ownership across the load, use an absolute path and
restricted DLL dependency search, and verify loaded-module identity. Enable
batching only for this verified corrected runtime. Missing/corrupt/unloadable
private data falls back to System32 with batching disabled. An unsuccessful
private mount attempt leaves a per-mount/payload failure marker so the next Retry
uses the official path; successful controlled teardown clears its own marker.
Never hot-swap a live DLL or replay uncertain writes. Fallback is not a promise
that arbitrary future DLL defects are transparently recoverable.

Acceptance: exact private/driver identity, real completed continuation records,
parallel/deep Windows navigation and teardown, SingleThread option normalization,
corrupt/missing/incompatible private input fallback, persistent failed-attempt
fallback, explicit compatibility mode, and preserved dirty recovery on failure.

### Final task suite and release

After all source changes, the checked-in `native/test-mount-optimization-task.ps1`
entrypoint runs through `.github/workflows/mount-optimization-task.yml`, bound
to the exact pushed `candidate_sha`. Reuse one incremental Windows
library development binary for all selected core/IPC/policy and real-driver cases;
no broad existing collection or cross-platform matrix. Include the previous
bounded request-delivery regression fixture within this same task invocation.

The fixture cache identity includes both committed inputs and the exact embedded
DLL/source-package hashes. The suite retains `approval.json` only after all
selected behavior succeeds; preparation alone does not approve the DLL. Its
remote job allows 180 minutes, with one 170-minute task invocation. Retained
dependency artifacts are verified and reused through the same fix loop.
Map every milestone above to explicit expected results and log backend-call/
transfer/concurrency measurements without presenting synthetic latency as a user's
network benchmark. Reuse the same suite for relevant fixes and approval of the
captured private-DLL artifact; no competing pipelines.

Commit coherent implementation milestones, refresh the required root graph, push
the complete candidate, then evaluate the remote suite. Only afterward invoke the
existing complete remote release entrypoint once through publication. Runtime
asset/source identity is preflighted before the release build. Persistent clean
caching across remount, range downloads, asynchronous writeback and generic NTFS
feature emulation are deferred, not prerequisites invented for this batch.
