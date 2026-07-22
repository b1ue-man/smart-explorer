# Remote filesystem tradeoffs: placeholders, temp copies, and a whole-file drive

Smart Explorer has three materially different ways to make remote content
usable by local applications:

1. metadata browsing plus a downloaded temp copy for one opened file;
2. a Cloud Files/CfAPI placeholder sync root; or
3. a real Windows drive whose filesystem callbacks are backed by a durable
   whole-file cache.

The earlier version of this note compared only the first two and concluded that
a mount was synonymous with placeholders. That was too broad. The implemented
third option is intentionally closer to Cryptomator/rclone full-cache behavior:
Explorer sees a drive, while application I/O is absorbed into a complete local
spool before Smart Explorer publishes anything remotely. The Dokany primary
sources were refreshed on **2026-07-22**; the other cited protocol sources retain
their individual check dates below.

## Decision

- Keep **temp-copy/save-back** as the portable default for opening one remote
  file. It has the broadest backend and application compatibility and avoids a
  system filesystem dependency.
- Offer the **Dokany whole-file drive** as an explicit Windows feature for users
  who need a stable drive-letter namespace across Explorer and applications.
- Keep **CfAPI scratched**. No Cloud Files sync root, placeholders or hydration
  lifecycle is part of the drive implementation.
- Default every drive to **read-only**. Admit read-write only after the exact
  active backend/root proves safe create, staged replacement and atomic
  namespace replacement.
- Independently default root authority to **strict confinement**. The deployed
  SSH Agent (Landlock ABI 3+ after symlink-free `openat2` root resolution) and
  Google Drive's parent-ID hierarchy qualify. Protocol paths without technical
  confinement need an explicit trusted-root choice for RO or RW; Peer/Share
  inherits the concrete exported backend/root result.

## What the options actually provide

| Axis | Temp copy/save-back | CfAPI placeholders | Dokany + whole-file spool |
|---|---|---|---|
| Path seen by app | ordinary temp path | path in registered sync root | ordinary drive-letter path |
| Tree visible in Explorer | only Smart Explorer's own UI | placeholder tree | live backend tree through filesystem callbacks |
| First file read | download before launch | hydrate on demand | materialize complete file on first open |
| Write model | watch local file, then upload | sync-engine notifications over hydrated file | local random writes; complete staged upload on flush |
| Editor atomic-save | temp watcher must infer change | provider must correctly mirror placeholder rename lifecycle | explicit temp-file→atomic-replace callback, capability-gated |
| Offline behavior | downloaded temp exists | depends on hydration/pinning and provider | already-open spool exists; uncached namespace still needs backend |
| Platform scope | Windows and Linux | Windows/NTFS Cloud Files | Windows + Dokany runtime |
| System dependency | none | Windows `cldflt.sys` + registered provider | signed Dokany driver/runtime; optional MSI embedded in recommended installer |
| Backend requirement for RO | read/list/stat | complete sync-provider mapping | read/list/stat plus enforced exact-root confinement or explicit trusted-root admission |
| Backend requirement for RW | ordinary upload plus conflict policy | complete sync lifecycle | RO admission plus exclusive `create + replace + namespace_replace` |

The whole-file drive does not make remote object storage into NTFS. It is a
compatibility layer with deliberately narrower semantics and explicit recovery.

## Why the Cryptomator analogy is accurate but limited

Cryptomator can expose a vault through a virtual-drive volume so normal
applications see a mounted filesystem. That is the UX being requested here,
not its encryption format and not CfAPI. Smart Explorer's filesystem process
instead delegates every remote operation through its own rooted `Backend`
proxy. Cryptomator documents virtual-drive volume types separately from WebDAV:
[volume types](https://docs.cryptomator.org/desktop/volume-type/) (checked
2026-07-21).

Dokany's project documentation describes its user-mode filesystem model: the
installed driver forwards Windows I/O requests to application callbacks. In the
official 2.3.1.1000 runtime, `DokanVersion()` reports DLL API 231, while
`DokanDriverVersion()` queries the independent kernel protocol 0x190 (decimal
400). Smart Explorer loads only the System32 DLL and checks those two version
domains separately. Official release drivers are signed, so no Developer Mode
or `TESTSIGNING` is needed; installing the
machine-wide runtime may still prompt for admin approval. The recommended NSIS
installer embeds the pinned official x64 MSI offline as a standard-selected
optional component; Smart Explorer remains per-user and only that MSI step uses
UAC. Portable and auto-updated copies can invoke the same secure pin with the
GUI or `se drive install-runtime`. Silent setup requires explicit
`/S /INSTALLDOKANY=1`, and Smart Explorer never uninstalls the shared runtime.

The automatic installer accepts only the [pinned official
`Dokan_x64.msi` URL](https://github.com/dokan-dev/dokany/releases/download/v2.3.1.1000/Dokan_x64.msi),
exactly 9,269,248 bytes, and SHA-256
`69ff8cb37bfec3a75921c85ffd1c6370b50a9ec4ecef2cf3a009d488dcbf5465`.
It validates Authenticode while holding the non-reparse file and parent path,
then requests UAC only for System32 `msiexec`. It can repair an absent runtime
or unavailable driver; it deliberately does not replace or downgrade a
genuinely incompatible installed shared runtime.

Sources checked 2026-07-22:

- [Dokany 2.3.1.1000 release](https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000)
- [Dokany tagged README](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/README.md)
- [Dokany tagged API header](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/dokan.h)
- [Dokany tagged driver header](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/sys/public.h)
- [Dokany version-query implementation](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/version.c)
- [Dokany API documentation](https://dokan-dev.github.io/dokany-doc/html/group___dokan.html)

## Why whole-file caching is necessary

Remote protocols and object providers generally expose streams or complete
object updates, not a durable Windows random-write handle. A one-byte edit can
therefore require a full upload. rclone's VFS documentation reaches the same
practical boundary: applications that need normal filesystem operations require
write/full caching rather than an uncached remote stream
([rclone mount/VFS cache modes](https://rclone.org/commands/rclone_mount/),
checked 2026-07-21).

Smart Explorer materializes one complete regular file into a private local
spool. The first local mutation is journaled durably before it is applied. A
flush synchronizes that file, uploads a complete sibling staging object, checks
the opening remote baseline again, and promotes the staging object only through
a backend-declared safe primitive. Clean entries are evicted after the final
handle closes; dirty/conflicting entries survive for Retry and recovery.

For the separate portable temp-copy workflow, the recovery marker is created
atomically only after download succeeds. A real declared payload remains
recoverable if an Electron/Obsidian single-instance launcher exits or an atomic
save temporarily removes the path. Empty complete legacy manifests are cleaned;
invalid state fails closed and genuine recovery is a notice rather than an app
error.

This supports both important editor families:

- **Obsidian-style repeated save:** truncate/write/flush/close can repeat after
  each edit. The remote destination remains the last complete committed file
  until a new complete upload is promoted.
- **Atomic-save editors:** create a sibling temp file, flush it, then rename it
  over the original. Smart Explorer permits the replacing rename only when the
  backend proves atomic `namespace_replace` semantics.

The cost is unavoidable: an editor that flushes every edit can cause a full
remote upload every edit, and network latency appears in the application's
flush. This design prioritizes not publishing partial files over pretending a
remote is as fast or semantically rich as NTFS.

## Protocol capability, not protocol name alone

RW is based on the active backend/root after Smart Explorer has resolved its
normal connection path and fallback. The mount engine itself does not know
whether traffic is SSH, UNC, HTTP or Iroh/QUIC.

Root authority is evaluated separately from write semantics:

- The auto-deployed Linux Agent starts with the exact SFTP root as
  `--serve-root`. `openat2(RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS)` binds the
  whole root path, and Landlock ABI 3+ confines reads, writes, rename and
  truncate before worker threads start.
- Google Drive descendant lookup stays in its provider parent-ID namespace.
- Plain SFTP, Local/UNC, WebDAV and FTP remain unverified against an external
  path-component swap. They mount only after the explicit
  `--trust-remote-root`/GUI choice, including RO. Validation and serialization
  remain active, but the server and concurrent writers become part of the
  trust boundary.
- Peer/Share inherits the result of its concrete remotely resolved backend/root.
  An Agent-confined export can therefore remain `Enforced`; exporting
  Local/UNC/plain SFTP does not remove its TOCTOU window and remains
  `Unverified`.

Share discovery exposes concrete local exports as `/Label` and saved connection
roots as `/Verbindungen/<connection>`; the aggregate `/` is a synthetic RO
target. The GUI asks the daemon to probe the active remote `PeerBackend` rather
than trusting local metadata. At mount time the peer issues a capability lease
bound to the authenticated device principal, exact root, export identity and
authorization epoch. Switching between direct and relay routes or replacing a
physical QUIC connection reacquires current Presence routes and preserves the
lease while those authorization facts stay unchanged. A different identity,
root or policy epoch fails closed and requires Retry/remount. A fallback is
therefore admitted with its current guarantees and cannot silently weaken RW.
Stopping the share or revoking/changing its authorization synchronously rejects
new operations and invalidates active mount leases; a later re-share requires
Retry/remount. An already admitted single operation may finish, while
multi-stage writes recheck authorization before flush and promotion.

The daemon owns the shared backend and its live connection independently of GUI
tab lifetime; the isolated mount host receives only a private local proxy.
GUI, daemon and one mount host are therefore the expected three processes for
one drive, not three separately connected remote clients.

Mount metadata depth defaults to 2 and is configurable from 0 to 4. Readiness
loads one complete root snapshot synchronously; deeper complete directory
snapshots are filled breadth-first in bounded 8-target batches, with a rotating
16-target refresh every 20 seconds. Only names and metadata are cached: 4,096
directories, 50,000 entries, 32 MiB total, 4 MiB per directory, plus a separate
five-second/4-MiB point-stat cache. Mutations invalidate affected paths, while
open/create admission, overwrite/conflict checks and every mutation continue to
use live backend state. This reduces Explorer request storms without weakening
the existing staged-write guarantees.

A strict SFTP mount is therefore admitted only when its saved connection has
Agent enabled and deployment plus the protocol-v9 `--serve-root` handshake
succeeds. That path never silently changes to plain SFTP. Normal browsing may
retain the already-established SFTP backend when Agent deployment fails; after
a successful handshake, operations are not blindly replayed across transports.

- Local and authenticated UNC operations, plus the Smart Explorer SSH Agent,
  provide exclusive staging create, replacement and namespace replacement.
- Plain SFTP deliberately reports no complete RW staged capability. Smart
  Explorer keeps one SFTP-v3 subsystem and standard `SSH_FXP_RENAME` treats an
  existing destination as an error; it does not open a second extension
  subsystem or claim stat+rename/shell-command sequences are atomic.
- A concrete Share export (`/Label` or `/Verbindungen/<connection>`) asks its
  remotely resolved backend for the same capabilities. The synthetic aggregate
  `/` is not a writable namespace target and stays RO.
- Google Drive can safely stage a create/update but cannot promise one atomic
  pathname replacement for editor temp-rename; WebDAV `MOVE Overwrite:T` and
  the current FTP/FTPS surface likewise do not supply the full contract.

Smart Explorer never converts a check-then-create/rename sequence or an SSH
shell command into a claimed atomic primitive. A fallback is reevaluated for
both capability sets: losing a write guarantee rejects RW, while losing
technical root confinement also requires explicit trusted-root admission for
RO.

Sources checked 2026-07-21:

- [SFTP v3 rename semantics](https://datatracker.ietf.org/doc/html/draft-spaghetti-sshm-filexfer#section-6.5)
- [WebDAV `MOVE` and `Overwrite`](https://www.rfc-editor.org/rfc/rfc4918#section-9.9)
- [Linux Landlock userspace API](https://docs.kernel.org/userspace-api/landlock.html)
- [`openat2(2)` path-resolution controls](https://man7.org/linux/man-pages/man2/openat2.2.html)

## Conflict and consistency limits

The mount records a baseline from provider object identity, size, mtime and –
when available – a content MD5. It compares that baseline before the staging
upload and again immediately before promotion; a replacing rename also
revalidates source and destination. A mismatch becomes a visible conflict and
retains the local spool.

These checks are not a universal server-side compare-and-swap. Unless the
backend has a conditional mutation primitive, another client can still change
the destination in the short interval between the last stat and promotion.
Atomic rename guarantees that observers see an old or new complete namespace
object; it does not by itself prove that no concurrent writer was overwritten.
Documentation and UI must preserve that distinction.

An ambiguous transport result after dispatch is never blindly retried. If the
namespace mutation may already have committed, the filesystem response avoids
inviting an application retry, while Smart Explorer reports conflict/recovery
state and keeps its journal. The normal backend reconnect/fallback logic remains
in one place; the mount does not introduce mutation replay.

The same rule applies to cleanup. Once an exclusive staging create has crossed
a network boundary, a lost acknowledgement may leave a hidden stage. Deleting
that pathname later is unsafe without an identity-bound provider operation: a
concurrent actor may have moved the owned object and reused the spelling. The
current implementation retains such orphans rather than risk deleting foreign
content; stable-ID/lease garbage collection is a future refinement.

Mount recovery is persisted as `Clean`, `Required`, or `Unknown`. Local
journal/spool state is audited under the exclusive cache lease before any
remote connection is attempted, both at daemon startup and in the mount host;
old or incomplete records that cannot prove clean remain `Unknown`. This keeps
an SSH timeout from disguising recoverable local data and permits removal after
a pre-mount failure only when the record is provably clean. The daemon launches
the current executable with the exact private `--mount-host <id>` argument, so
GUI and CLI do not depend on a separately named host binary.

Drive-manager errors retain the terminal recovery/conflict state and append a
bounded mount-host process cause. They distinguish DLL API 231 from kernel
protocol 0x190/400 mismatches, name strict-root or staged-RW rejection, turn an
invalidated Peer lease into a Retry/remount instruction, and preserve any local
recovery data. A raw host exit code is supplemental context, not the diagnosis.

## Filesystem surface limits

The drive targets ordinary Explorer and editor operations, including open,
read, write, append, truncate, flush, create directory, rename, replacing rename
and delete-on-close. It does not emulate all NTFS metadata:

- setting timestamps, arbitrary attributes, ACLs or security descriptors is
  unsupported;
- Alternate Data Streams, open-by-ID and reparse-point access are unsupported;
- remote symlinks are hidden or rejected and are never traversed out of the
  authorized root;
- free-space reporting reflects the local spool lower bound, while a remote
  quota can fail only at commit;
- uncached files need a live backend, and a first open downloads the full file.

The remaining acceptance gap is a real Windows machine running the official
Dokany 2.3.1.1000 runtime with DLL API 231 and driver protocol 0x190/400,
Explorer and Obsidian. That smoke must cover repeated
flushes, temp-file replacement, external conflict, retry/reconnect and eject; it
is tracked as live work in `docs/TODO.md` and cannot be inferred from a GNU
cross-build.

## CfAPI is historical, not a fallback plan

CfAPI is appropriate for a Windows cloud sync engine that owns registered root
state, placeholders, hydration and change reconciliation. Microsoft's own
documentation frames it in those terms
([build a Cloud Files sync engine](https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine),
checked 2026-07-21). That is not the requested Cryptomator-style drive and is
not used as a runtime fallback. The previous prototype and analysis remain in
`docs/CFAPI_REVIEW.md` only as historical evidence.
