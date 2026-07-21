# Editing remote files: portable temp copy and optional Windows drive

> **Current decision:** CfAPI is historical and superseded. Smart Explorer has
> two deliberately separate editing paths: the portable temp-copy/save-back
> default, and an explicit Cryptomator-style Windows drive implemented with
> Dokany and a whole-file spool. Neither path registers a Cloud Files sync root
> or creates placeholders.

The requirement for the drive path is concrete: an arbitrary Windows program
must be able to open `M:\...`, and every read/write still has to pass through
Smart Explorer's selected `Backend`, credentials and existing transport/fallback
logic. Obsidian is an acceptance workload because it may repeatedly truncate,
write, flush and close a note, while other editors save by writing a sibling
temporary file and atomically replacing the original.

## The two supported workflows

### Portable open/edit/save-back

Double-clicking one remote file in the normal Smart Explorer view downloads an
ordinary temp copy, launches its associated application, watches it through
`RemoteEdit` / `poll_remote_edits`, and uploads changes after the debounce. This
continues to be the backend-independent default on Windows and Linux. The
application sees a temp path rather than a stable remote namespace.

The recovery marker below the app-data `open-temp` directory is written
atomically only after the download has produced a real local file. Startup
deletes a legacy manifest only when it is complete, declares no entries and has
no payload; malformed or truncated manifests fail closed. A manifest that
declares a real file remains recovery-required even while an atomic editor save
temporarily makes the path disappear. Genuine recovery is a user notice, not an
application error.

Finishing the process returned by `ShellExecute` is deliberately not proof that
the editor is finished. Electron applications such as Obsidian can forward the
open request to an existing single-instance process and let the short-lived
launcher exit immediately. Smart Explorer therefore retains the registered temp
file and marker for later changes and save-back; a transient `NotFound` during
delete/rename is an error that preserves the prior recovery state rather than a
signal to discard it.

### Explicit Windows drive

An explicitly selected saved connection, Google Drive root or Share endpoint
can be exposed as a real drive letter through Dokany. Explorer and applications
issue ordinary filesystem calls; Dokany forwards those calls to the isolated
Smart Explorer mount host, which talks over a rooted loopback proxy to the
daemon-owned backend. This is the same *kind* of user experience as
Cryptomator's virtual volume, but Smart Explorer is not using Cryptomator or its
vault format. Cryptomator's documentation confirms that a virtual-drive volume
is distinct from a WebDAV-only presentation: [Cryptomator volume
types](https://docs.cryptomator.org/desktop/volume-type/) (checked 2026-07-21).

GUI entry points are the drive icon beside saved connections, Google Drive and
Share devices, plus the toolbar drive manager. CLI equivalents are:

```powershell
se drive runtime
se drive install-runtime
se drive mount @prod:/srv --letter M
se drive mount @prod:/notes --letter N --read-write
se drive mount sftp://host/srv --letter S --trust-remote-root
se drive list
se drive unmount N:
se drive retry <mount-id>
```

Targets may be an `@label:/path`, an exact saved remote URL or UNC path,
`gdrive://...`, or `share://...`. `--letter auto` is the default. The mount is
read-only unless `--read-write` is supplied.

## Why Dokany, and what Windows needs

This feature does **not** use CfAPI, `cldflt.sys`, placeholder hydration or a
registered sync root. Dokany supplies the signed kernel bridge and invokes a
user-mode filesystem's callbacks; that is the mechanism needed for a normal
drive letter without writing a Smart Explorer kernel driver.

Smart Explorer supports the official [Dokany 2.3.1
release](https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000), API 231.
The recommended NSIS installer embeds the pinned official x64 MSI offline as a
standard-selected optional component. Smart Explorer itself remains per-user;
only the machine-wide MSI invokes UAC. Silent installation deliberately skips
that elevation unless `/S /INSTALLDOKANY=1` is supplied, and uninstalling Smart
Explorer never removes Dokany. Portable and auto-updated installations can use
the GUI action or `se drive install-runtime` to download and install the same
pin.

The single source of dependency truth is
[`native/dokany-runtime.nsh`](../native/dokany-runtime.nsh):

- version `2.3.1.1000`, API `231`, filename `Dokan_x64.msi`;
- URL `https://github.com/dokan-dev/dokany/releases/download/v2.3.1.1000/Dokan_x64.msi`;
- exact size `9,269,248` bytes; and
- SHA-256 `69ff8cb37bfec3a75921c85ffd1c6370b50a9ec4ecef2cf3a009d488dcbf5465`.

The application download accepts HTTPS only, at most two redirects, and a final
URL that is either the exact pin or GitHub's
`release-assets.githubusercontent.com` host. It reads no more than the pinned
size plus one byte and requires the final size and SHA-256. On Windows the MSI
is opened without write/delete sharing and as a non-reparse regular file; the
same locked handle is hashed and passed to `WinVerifyTrust` with chain
revocation checking. Smart Explorer also opens every normalized parent below
the stable volume/share root as a direct non-reparse directory handle without
delete sharing. The file and parent-chain handles remain held while the UAC
`runas` process starts `%WINDIR%\System32\msiexec.exe`, binding the pathname
that the elevated process reopens, with `/passive /norestart
ADDLOCAL=DokanDriverFeature INSTALLDEVFILES=0`. Exit code 0 is accepted only
after `%WINDIR%\System32\dokan2.dll` and the driver both report exactly API 231.
An already installed wrong API is reported rather than overwritten silently.

The release fetchers `native/fetch-dokany-runtime.{ps1,sh}` verify the same
manifest before NSIS embeds the MSI, and the release wrapper extracts it again
from the built installer and compares its bytes. Copyright notices and the GPL,
LGPL and MIT license texts are kept under `third-party/dokany/` and installed
beside Smart Explorer's notices; the MSI itself is not a separate update-feed or
GitHub Release asset.

At runtime Smart Explorer delay-loads only
`%WINDIR%\System32\dokan2.dll` and requires both the DLL and driver to report
exactly API 231. The official project provides signed release drivers, so the
official runtime needs neither Developer Mode nor Windows `TESTSIGNING`.

Primary sources checked 2026-07-21:

- [Dokany tagged README: architecture, signed releases and installer](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/README.md)
- [Dokany 2.3.1 header: `DOKAN_VERSION 231`, `DokanVersion`, `DokanDriverVersion`](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/dokan.h)
- [Dokany callback/API documentation](https://dokan-dev.github.io/dokany-doc/html/group___dokan.html)

## Authority and transport boundary

The mount engine is deliberately backend-neutral. The daemon resolves the
saved account and active protocol path, including the normal Smart Explorer
direct/relay/SSH choices and connection fallback. It wraps that backend at the
selected root and rejects traversal, link-like ancestors and unsafe Windows
names. The daemon starts its current executable with the exact private
`--mount-host <id>` argument. That isolated process receives neither
credentials, endpoint/account metadata, global daemon authority nor
unrestricted provider IDs; it receives separate one-use launch/backend loopback
capabilities plus a session capability for one rooted backend.

The default root policy is independently fail-closed for both RO and RW. A
backend may claim `RootConfinement::Enforced` only when every operation is
technically bound to the exact selected hierarchy. The deployed Linux SSH
Agent does that with `--serve-root`: `openat2` rejects symlinks and magic links
in every root component, then Landlock ABI 3+ confines read, write, rename and
truncate before worker threads start. Google Drive resolves descendants through
provider parent IDs. These are the strict-mode paths checked on 2026-07-21
against the [Landlock userspace API](https://docs.kernel.org/userspace-api/landlock.html)
and [`openat2(2)`](https://man7.org/linux/man-pages/man2/openat2.2.html).

Plain SFTP, Local/UNC, Peer/Share, WebDAV and FTP cannot atomically bind a prior
path validation to a later protocol operation when another actor can exchange
a symlink, junction or directory. They therefore require the explicit GUI
choice **Remote-Wurzel ohne technische Sandbox vertrauen** or CLI
`--trust-remote-root`, even for RO. Trusted mode still rejects traversal,
link-like observations, unsafe Windows names and case collisions and serializes
Smart Explorer operations; it explicitly trusts the server and concurrent
writers during each check-to-operation interval.

Consequently, a connection fallback does not need a second mount
implementation. It can, however, change which write guarantees are available.
The daemon reevaluates root confinement and write guarantees on the active
fallback before starting. Losing write primitives can still leave an RO option;
losing strict root confinement requires explicit trusted-root admission even
for RO. SFTP is stricter: a strict SFTP mount requires a saved connection whose
Agent option is enabled and whose Agent deployment and handshake succeeded.
That failure is surfaced and never silently converted to plain SFTP for the
mount. Ordinary browsing may retain the already-established plain-SFTP backend
when Agent deployment itself fails. Once the Agent handshake succeeds, failed
operations are surfaced rather than replayed blindly over SFTP.

SSH address candidates are deduplicated and interleaved across IPv6 and IPv4,
with a new attempt staggered every 250 ms while the original hostname remains
the known-host identity. Connect, handshake, channel and exec waits have typed
timeouts; captured stdout and stderr are bounded independently to 64 KiB each.
Deployment uses one SFTP subsystem plus exec channels. The remote filename
includes protocol v9 and the full SHA-256 of the embedded agent. Smart Explorer
verifies the staged upload and installed file through SFTP, and re-verifies the installed bytes
before launch after reconnect rather than trusting an old version probe.

## Read-write admission

Read-only is the safe default. `--read-write` is accepted only if the exact
active backend/root reports all three guarantees:

| Capability | Required filesystem property |
|---|---|
| `create` | atomically acquire the random staging name with exclusive create, then publish it to a still-missing name without replacing a concurrent creator |
| `replace` | publish a complete staged file over an existing regular file |
| `namespace_replace` | atomically replace an existing name for editor temp-file saves |

Current backend result:

| Active backend/root | Strict root | RW primitives |
|---|---|---|
| local filesystem or authenticated UNC | no; explicit trusted-root mode | yes |
| deployed Smart Explorer SSH Agent | yes, with `--serve-root` and Landlock ABI 3+ | yes |
| plain SFTP v3 | no; explicit trusted-root mode | no complete capability; standard SFTP-v3 rename is no-replace |
| concrete or synthetic Share target | no; explicit trusted-root mode | concrete export only if its resolved backend/root reports all three; synthetic containers no |
| Google Drive | yes, provider parent-ID hierarchy | no: create/replace exist, but no atomic namespace replace |
| WebDAV, FTP/FTPS | no; explicit trusted-root mode | no complete guarantee at present |

Smart Explorer deliberately keeps one SFTP subsystem and relies only on
standard SFTP-v3 behavior. `SSH_FXP_RENAME` must fail when the destination
exists, so plain SFTP advertises none of the complete RW staged capability. It
does not open a second subsystem merely to negotiate a rename extension, does
not pretend that prior stat plus normal rename is atomic, and does not fall back
to a remote shell command. Source checked 2026-07-21: [SFTP v3 section
6.5](https://datatracker.ietf.org/doc/html/draft-spaghetti-sshm-filexfer#section-6.5).

## Obsidian and editor-save semantics

Remote backends generally do not provide Windows random-write handles. The
mount therefore materializes the complete file into a local spool. Reads and
writes operate on that spool. Before the first mutation, a dirty record is
durably synchronized. A flush then:

1. synchronizes the local whole-file spool;
2. compares the opening baseline with current remote metadata;
3. uploads the entire file to a unique staging path;
4. compares the remote baseline again immediately before commit;
5. promotes the staging file with the backend's declared safe primitive; and
6. clears the recovery record only after post-commit verification.

That ordering supports repeated `truncate → write → flush → close` without ever
publishing the truncated or partly written target. It also supports the common
`write sibling temp → flush → rename over destination` pattern only on a backend
with `namespace_replace`. Existing open handles keep referring to their old
spool object when Windows delete-sharing semantics require it.

If the remote changes while a local copy is open, Smart Explorer records and
surfaces a conflict instead of blindly overwriting it. The baseline uses
provider identity, size and mtime, plus content MD5 when the backend supplies
one. If a promotion may already have committed but verification or journal
cleanup fails, Windows receives success rather than an unsafe retry while the
mount enters visible recovery/conflict state and retains the spool.

Persisted mount recovery is tri-state: `Clean`, `Required`, or `Unknown`.
Backward-compatible records with the old dirty flag map deterministically;
records that never supplied trustworthy status become `Unknown` rather than
clean. At daemon startup and again in the host, Smart Explorer acquires the
mount-cache lease and audits local journal/spool state before attempting a
remote connection. Thus an SSH timeout cannot hide local unsaved data. Only a
provably `Clean` entry may be offered for removal after a pre-mount failure;
`Required` and `Unknown` are retained for recovery.

## Honest limits

- This is whole-file caching, not ranged remote I/O. First open downloads the
  complete file; every changed flush uploads it completely. Obsidian saving on
  each edit therefore multiplies bandwidth and latency, especially for large
  vault attachments.
- Baseline checks are not universal server-side compare-and-swap. Smart Explorer
  rechecks immediately before promotion, but without a backend conditional
  commit a small TOCTOU window remains between stat and mutation.
- After exclusive staging ownership has crossed a network boundary, an aborted
  stream or lost acknowledgement can leave a hidden staging object behind.
  Smart Explorer deliberately does not unlink that spelling blindly: another
  actor could have moved the owned object and reused the name between a check
  and delete. Future cleanup needs a stable provider object ID or lease.
- Network or fallback latency is visible to the calling application during
  materialization and flush. Long Dokany callbacks renew a five-minute timeout,
  but that prevents false timeout rather than making a slow remote fast.
- The displayed free-space value is bounded by local spool capacity; a remote
  quota can still reject the eventual upload.
- The volume supports the ordinary file operations needed for Explorer and
  editors, not the complete NTFS surface. Setting ACL/security descriptors,
  timestamps, arbitrary attributes, Alternate Data Streams, open-by-ID and
  reparse points is unsupported. Remote symlinks are not followed.
- A real Windows + Dokany + Explorer + Obsidian smoke test remains required; see
  `docs/TODO.md`. Cross-compilation cannot validate driver/callback lifecycle or
  real application behavior.

## CfAPI history

Earlier research proposed `CfRegisterSyncRoot`, placeholders and hydration
callbacks. That direction was scratched after the experiment caused file-open
failures and its sync-engine lifecycle did not fit this feature. There is no
active `cfprovider.rs` or `cfsync.rs` in `native/src`. `docs/CFAPI_REVIEW.md` is
historical evidence only; it is not the implementation plan for remote drives.
