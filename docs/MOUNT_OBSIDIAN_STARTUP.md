# Obsidian large-vault startup investigation

Evidence and implementation planning, not another live board. Open status is D4
in [TODO.md](TODO.md). Inspected source: `59bbbd9`, 2026-09-07. The user reports
that small mounted vaults open but larger ones still fail; the relevant quantity
(files, directories, pending requests or something else) is unknown. No exact
Windows loading phase, error or failing-operation trace has been supplied.

## Goal and deliverables

Establish the actual failing startup boundary, correct that proven behavior,
cover the real startup request pattern and directly affected integrations in
one remote task suite, then publish once after acceptance. Do not present a
synthetic traversal, a plausible timeout mechanism or the Linux installation as
proof that the user's Windows vault has been fixed. No cache/worker/timeout
changes on speculation. Preserve root confinement, read-only access, dirty data,
compatibility fallback and the half-hour minimum between release observations.

## Stage one: inspected evidence

### Exact installed application, without opening personal vaults

The user identified the installed Linux desktop application as available for
investigation. Both `/opt/Obsidian/resources/app.asar` and `obsidian.asar` declare
Obsidian **1.12.7**. Static archive reading only; neither Obsidian nor its
extracted JavaScript was executed, and no personal vault/configuration was read.

- `app.asar` SHA-256:
  `a92d31a51840c50035919f7fc4a0caf1f34db50a6f9a9205613e9f95f6a1118e`
- `obsidian.asar` SHA-256:
  `2b2483b2e1246772e0d25367ec055cbc5047ea2f0091b667c35656678f86d712`

The following locations are UTF-8 byte offsets inside its `app.js`, not archive
offsets. This member starts at archive byte 692,353 and is 3,728,534 bytes long.
Record identities/offsets rather than copying proprietary application code.

| Method/boundary | Byte offset | Observed behavior |
|---|---:|---|
| Desktop adapter `Eu` | 566850 | Uses Electron `original-fs`; performs a synchronous case-sensitivity probe |
| `listAll` / `listRecursive` | 567781 / 568031 | Whole visible tree; `readdir` returns names, followed by concurrent child promises |
| `listRecursiveChild` | 568417 | `lstat` on every visible child, recursion on directories, extra link handling |
| `watch` / `startWatchPath` | 579320 / 580519 | Root `realpath`, watcher installation, then queued complete tree scan |
| `kill` / `queue` | 586019 / 586163 | A promise race rejects the queued operation without canceling underlying filesystem calls |
| App awaits Vault load | 3684141 | Tree discovery must finish before metadata-cache initialization |
| App awaits metadata-cache initialization | 3684466 | Later Markdown indexing uses a separate sequential work queue |

The adapter's inactivity watchdog is 60 seconds. It is reset when a queued
operation starts and when a recursive `readdir` finishes, **not when individual
`lstat` calls finish**. A sufficiently long final metadata burst can therefore
hit the application timeout despite individual metadata progress. This is an
established mechanism in this exact bundle, not the user's established cause.

The shared JavaScript explicitly branches by platform. Windows/macOS use a
recursive root watcher; Linux installs non-recursive watchers as directories are
discovered. A Linux run cannot certify Windows/Dokany watcher behavior. The
user's Windows application/runtime identity and inherited environment are still
unknown; the installed bundle contains no thread-pool override in the inspected
startup entrypoints.

### Existing acceptance does not reproduce that complete pattern

`native/mount-vault-node.cjs` uses `readdir({withFileTypes:true})`, explicitly
limits its own workers to 16, and checks only three representative stat/lstat
pairs in the 50,001-file wide directory. Obsidian initially calls `lstat` on
**every** visible child there. The native parent
`mount/os/windows/vault_volume_metadata.rs` forces `UV_THREADPOOL_SIZE=16` and
runs Node after a native full traversal and hot-subtree priming. The Node phase
does not install a watcher before traversal, lacks the realpath prelude and
uses a 280-second overall deadline instead of directory-progress inactivity.

Those are concrete coverage gaps. The earlier acceptance remains evidence for
its stated workload, not evidence of actual Obsidian startup success. The user's
reported failure remains open.

### Other source-proven affected boundaries, not selected causes

- `daemon/os/shared/mount_request_gate.rs`, `mount_proxy.rs`,
  `agent/core/backend.rs`, `mux.rs`, `daemon/os/shared/backend_server.rs` and
  `request_workers.rs`: an absolute metadata timeout unregisters only locally.
  Client admission is released while the synchronous server worker may still be
  blocked. Repeated timed-out waves can exhaust the server's 16 live workers
  despite the client's maximum of eight counted requests. Raising the count
  merely moves this boundary; a Cancel frame alone cannot interrupt a blocked
  synchronous backend call. No such wave is measured in the user's session.
- The transport's single response reader waits for a full per-request queue to
  drain; the server's upload dispatcher can likewise block on a full upload
  queue. These require a backpressured stream, not merely many tree entries.
- Windows handle share admission scans all live handles. That is a scaling
  cost, but Windows Obsidian does not create one ordinary watcher per directory.
- First data access materializes the whole file, syncs its local spool and
  rechecks its remote baseline. This may multiply per-file latency even for tiny
  notes, but must not be conflated with the earlier content-free tree scan.
- At the inspected baseline, `mount/core/file_io.rs::open_metadata_file` lacked
  the read-only/writable check already present in `open_file`. The daemon still
  independently rejects remote writes. The development correction now rejects
  writable lazy opens before validation or handle allocation; remote acceptance
  and publication remain pending in this batch. This is not an explanation of
  the large-vault failure.

Stage-one direction: identify the stalled application phase, distinguish pending
metadata from content/reconciliation work, and target the demonstrated boundary.
Do not simultaneously tune unrelated capacities or replace the filesystem DLL.

## Primary-source research

Checked 2026-09-07:

- [Electron ASAR format](https://github.com/electron/asar#format): read the
  archive index and exact member offsets without executing the packaged app.
- [Electron ASAR filesystem behavior](https://www.electronjs.org/docs/latest/tutorial/asar-archives):
  `original-fs` exposes the ordinary filesystem rather than ASAR emulation.
- [Node filesystem APIs](https://nodejs.org/api/fs.html#fspromisesreaddirpath-options):
  names and Dirents are different enumeration interfaces; a promise resolving
  directory names does not replace the application's subsequent per-name calls.
- [libuv thread-pool contract](https://docs.libuv.org/en/v1.x/threadpool.html):
  the default filesystem pool is four and is shared. Do not silently replace an
  application's inherited pool with the fixture's larger one and infer parity.
- [Windows file access rights](https://learn.microsoft.com/en-us/windows/win32/fileio/file-access-rights-constants):
  explicit data-write access differs from metadata-read access. Rejecting a
  writable lazy open must not make an allowed metadata-only read eager. The
  correction reuses the existing engine mode/error contract, with no new API.

## Stage two: bounded milestones and acceptance signals

1. **Identify the failing phase.** Obtain a minimal Windows startup observation:
   loading text/error and, if accessible, read-only counts from the already-open
   application. No vault contents, file names, modifications or forced reload.
   Expected result: distinguish incomplete tree scan, later metadata indexing,
   workspace/plugin loading, or a renderer that cannot run the observation.
   Static Linux bundle inspection supplies the call model, not this measurement.
2. **Correct the demonstrated request boundary.** Select and finalize the source
   change only after milestone 1 or equivalent runtime evidence establishes the
   failure chain. Candidate surfaces are the metadata/callback path, timeout
   ownership and content materialization; they are alternatives, not a list of
   changes to try. Expected result: the exact failing sequence completes without
   weakening authorization, concealing errors, or changing Obsidian's timeout.
3. **Preserve lazy-open read-only admission.** Restore the mode check in the
   existing lazy path as part of the affected implementation batch. Expected
   result: writable lazy opens on a read-only engine fail before creating a
   writable handle/spool; legitimate metadata-only reads remain lazy. The guard
   is implemented but unverified remotely. Reuse the `NavigationBackend` fixture
   for denied writable admission, unchanged backend/spool counts and successful
   lazy reads; retain a read-write admission case in the same final task suite.
   Before reusing its first-read phase, derive `note.md`'s advertised length from
   its payload: that older fixture currently advertises 12 bytes but returns the
   15-byte `remote contents`. Do not weaken materialization length checks.
4. **One faithful remote acceptance entrypoint.** Update the existing mount task
   entrypoint/fixture after implementation. Run an independently written cold
   names-plus-every-child-lstat consumer, with realpath and the recursive watcher
   already active, before native warming; do not override the normal pool upward.
   Separate wide and nested tiny-content shapes. Record outstanding operation
   categories, failures and maximum completed-directory gap, then exercise serial
   note reads and watcher/save reconciliation. Include the proven corrected
   boundary and read-only admission in this same suite. Expected result: complete
   tree and consistent contents, no hidden timeout/error, safe cleanup, exact
   source/runtime identity. Use only remote incremental affected binaries.
5. **Single terminal release.** Only after the same remote suite passes, invoke
   the existing complete remote wrapper once, verify publication and stop. Never
   infer a successful user-vault outcome from Linux startup or synthetic counts.

## Second gap review / implementation hold

The inspected application timer measures inactivity between directory results,
not the whole startup and not network byte throughput. Promise rejection is not
filesystem cancellation. A faithful workload must retain all child metadata
queries and must not warm the provider before the first consumer. A Windows
recursive watcher is not equivalent to Linux's per-directory watcher branch.

The remaining gap is the user's actual stalled phase/request, so milestone 2's
implementation is intentionally not selected yet. The only native edit is the
independently proven read-only admission guard from milestone 3. No limits, DLLs
or application settings changed; no suite/build or release was started. A
follow-up implementation plan must close the evidence gap before changing the
large-vault request path. Do not release this guard alone as an Obsidian fix.
