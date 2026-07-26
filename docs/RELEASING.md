# Releasing & the update flow (end-to-end)

How a new version goes from a commit to an installed app updating itself.
One version number drives everything: `native/Cargo.toml`.

```
 finish task batch + development validation
   ─▶ top-level wrapper bumps Cargo.toml once
   ─▶ one complete local release build
       ├─▶ update feed   version + source-bound manifest + Windows/Linux app/updater/se + hashes
       └─▶ installer     Windows NSIS + Linux install-linux.sh
   ─▶ commit + push main
   ─▶ release trigger ─▶ static exact-byte CI validation/publication
                     ─▶ GitHub Release vX.Y.Z  (verified feed payloads/hashes + installer/script/dll/share servers)
                                         │
 installed app on launch ──▶ reads update_source (default: the Git feed on main)
                          ──▶ newer version? ──▶ stage + SHA-check app/updater/se
                                                ──▶ ask user ──▶ helper transaction + restart
```

The version is consistent across all four outputs because each reads it from
`Cargo.toml`. Never hand-edit `version.txt` — the complete local release script
writes it last.

## ⚠️ Prerequisite for auto-update to work: the repo must be PUBLIC

The default update source is the **raw Git feed on `main`**:

```
https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/release-native/update-feed
```

`raw.githubusercontent.com` (and private-repo Release assets) require auth for a
**private** repo, so an anonymous app gets `404` and can't self-update. Make the
repository **public** (Settings → General → Danger Zone → Change visibility) and
the feed + Release downloads work for everyone. Until then, only a signed-in
user can pull updates.

## Pinned Dokany installer dependency

The optional Windows remote-drive feature uses the official [Dokany
2.3.1.1000 runtime](https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000),
with DLL API 231 and kernel-driver protocol 0x190 (decimal 400).
The recommended NSIS installer embeds exactly one official dependency:
`Dokan_x64.msi` is an optional, standard-selected offline component. The Smart
Explorer application remains a per-user install; only this machine-wide MSI
step asks for UAC. Silent NSIS installs must explicitly pass
`/S /INSTALLDOKANY=1`; plain `/S` skips Dokany and never introduces an
unexpected elevation. Smart Explorer's uninstaller removes its installed
notices but never uninstalls or downgrades the shared Dokany runtime.

`native/dokany-runtime.nsh` is the single manifest consumed by Rust, NSIS and
both dependency fetchers. Its reviewed pin is:

- version `2.3.1.1000`, DLL API `231`, driver protocol `0x190`/`400`, file
  `Dokan_x64.msi`;
- `https://github.com/dokan-dev/dokany/releases/download/v2.3.1.1000/Dokan_x64.msi`;
- exactly `9,269,248` bytes; and
- SHA-256 `69ff8cb37bfec3a75921c85ffd1c6370b50a9ec4ecef2cf3a009d488dcbf5465`.

`native/fetch-dokany-runtime.ps1` and `.sh` fetch that exact pin into ignored
build state and reject a size or hash mismatch. The complete release wrapper
resolves it before the build, passes it to NSIS, then uses 7-Zip to extract the
embedded MSI from the completed installer and compares its size and SHA-256
again. This MSI remains inside the Windows installer: it is not a standalone
update-feed payload or GitHub Release asset, and portable/auto-updated users get
it on demand through the GUI or `se drive install-runtime`. `dokan2.dll`,
`dokan2.sys`, `DokanSetup.exe`, debug installers and any unreviewed Dokany files
must never be copied beside Smart Explorer or added as separate assets.

The GUI/CLI downloader allows HTTPS only, at most two redirects, and only the
exact source URL or GitHub's `release-assets.githubusercontent.com` host as the
final URL. It caps the response at the pinned size plus one byte, then verifies
exact size and SHA-256. On Windows it opens a direct regular, non-reparse MSI
without write/delete sharing, hashes that locked handle, and verifies its
Authenticode chain with `WinVerifyTrust` including revocation checks. It also
opens every normalized parent below the stable volume/share root as a direct
non-reparse directory handle without delete sharing. The file and parent-chain
handles stay open while a UAC `runas` launch invokes the `msiexec.exe` resolved
from System32, so the elevated reopen is bound to the verified pathname, with
`/passive /norestart ADDLOCAL=DokanDriverFeature
INSTALLDEVFILES=0`. An exit code 0 is not enough: the postcheck must load only
`%WINDIR%\System32\dokan2.dll`, see DLL API 231 from `DokanVersion()`, and see
driver protocol 0x190/400 from `DokanDriverVersion()`. The GUI/CLI automatic
path installs only for an absent runtime or an unavailable driver. A genuinely
incompatible installed shared runtime fails with the exact observed API or
protocol and is never automatically overwritten or downgraded.

The official project provides signed release drivers, so users need neither
Developer Mode nor `TESTSIGNING`. The application remains delay-loaded and its
non-drive features must work without Dokany. Install the notice plus GPL-3.0,
LGPL-3.0 and MIT texts from `third-party/dokany/` with Smart Explorer. These
packaging, signature and version-domain claims were checked against the
[tagged README](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/README.md)
and [tagged API header](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/dokan.h),
the [tagged driver header](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/sys/public.h),
and the [version-query implementation](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/version.c)
on 2026-07-22. If the supported ABI, driver protocol, or MSI changes, review and update the
manifest, Rust validation, fetchers, installer, license notices and user/native
documentation together.

## Cut a release

The release is one terminal transaction, started only after the complete task
batch and its single task-level suite are finished. Do not bump the version or
run an exact-candidate verification pipeline by hand first.

1. Run the non-building preflight from the repository's clean, synchronized
   `main` branch:

   ```powershell
   pwsh ./native/publish-release-local.ps1 -CheckEnvOnly
   ```

   It checks Windows/WSL or Linux cross-build tooling, Rust targets,
   `rustfmt`/Clippy, Zig, NSIS, MinGW, 7-Zip, network access, the active workflow, and
   non-interactive Git write authentication for `main` plus at least one of the
   exact tag or `release/vX.Y.Z` trigger paths. Resolve every failure before the
   complete build. The HTTPS remote needs a usable Git credential, and the
   long-running REST poll requires `GH_TOKEN`/`GITHUB_TOKEN` or a successful
   `gh auth login`; recovery of a failed tagged run additionally requires
   GitHub Actions write permission on that token. Anonymous API quota is
   deliberately not accepted. Untracked
   files below native, Share-server, agent, root Cargo configuration, or vendored
   dependency build roots are also rejected so release bytes can never depend
   on source absent from the candidate commit. Every Cargo release invocation
   also pins the exact target directory from which its payload is staged.
2. Invoke the same top-level wrapper exactly once without `-CheckEnvOnly`:

   ```powershell
   pwsh ./native/publish-release-local.ps1
   ```

   On Windows it builds Windows locally and Linux through WSL. On Linux/WSL it
   calls the checked-in `native/publish-feed.sh` internally for the complete
   Windows-GNU/Linux cross-build. Both paths inherit the same
   `release-native/.complete-release.lock`; a direct full invocation of
   `publish-feed.sh` is refused.

   Every canonical Cargo leaf uses all logical CPU workers, disables incremental
   compilation and cross-crate LTO, and uses 16 codegen units. These values are
   fixed by the release scripts. The large Linux build tree additionally
   runs through `native/run-release-memory-bounded.sh`. When systemd scopes are
   available it applies `MemoryHigh=3G`, `MemoryMax=4G`, and `MemorySwapMax=1G`,
   so an exceptional compiler allocation can terminate only that build scope
   rather than displace unrelated desktop processes. Hosts without a usable
   scope print a warning and retain the compiler-level limits.
3. The wrapper owns every remaining step. It bumps the patch version once,
   reuses that version after a pre-tag failure, builds and promotes the complete
   artifact set, verifies the six feed hashes, the installer's embedded
   app/updater/`se` bytes and pinned Dokany MSI, the manifest's exact
   source-parent binding, and the
   exact 18 publication assets,
   rejects any build-time drift in tracked sub-workspace lockfiles,
   creates `Release Smart Explorer vX.Y.Z [release candidate]`, fast-forwards
   `main`, and pushes exactly one immutable `vX.Y.Z` tag. If and only if that
   tag push is technically rejected while the remote tag is still absent, the
   same wrapper pushes the exact candidate once to `release/vX.Y.Z` instead and
   follows that mutually exclusive publication run. The marked main-branch
   commit skips its redundant development CI run; the tag run performs the
   static exact committed Linux/Windows candidate gates and publishes the
   GitHub Release. The wrapper polls that exact run, checks all 18 published asset
   digests against the local bytes, and only then reports success.
4. On Linux the wrapper finally installs `se` from that exact tag with release
   assets required, verifies its version and SHA-256, and requests the existing
   daemon's version-bound handoff. CLI-only installation leaves an existing
   desktop `update_source.txt` unchanged, so this exact one-time handoff cannot
   pin future app updates to the candidate SHA.

`verify_release_candidate=true` and `verify/v*` remain available only when a
user explicitly requests exact-candidate verification without a release. They
must not precede the normal tag publication, because that would create a second
pipeline for the same candidate. Likewise, do not use workflow dispatch or a
`release/v*` branch in addition to a successful wrapper tag run. The wrapper
alone may select that branch after proving the tag push failed and no remote
tag exists; it never dispatches a competing pipeline.

The Windows-only diagnostic
`.\native\publish-release-local.ps1 -SkipLinuxFeed` remains non-publishable: it
writes `release-native/windows-partial-vX.Y.Z/` without changing the shared
feed, bumping a version, committing, pushing, or tagging. Linux payload repair
scripts are recovery diagnostics, not alternate complete-release entrypoints.
A failed build may retain an isolated stage for diagnosis; never hand-promote
it. Fix the cause and rerun the top wrapper for the same intended version.
The recovery preflight accepts `Cargo.toml` and `Cargo.lock` only when their
working copies differ from `origin/main` by that exact next patch version; any
dependency, profile, or unrelated lockfile drift fails before a build starts.
Before an immutable tag exists, the wrapper recovers that candidate instead of
inventing another patch. Every build records its current source HEAD. Once the
bounded release candidate is committed, the manifest must bind that commit's
sole parent; a replacement build on an interrupted candidate therefore binds
that candidate as the replacement's parent. A later source fix rebuilds the
same intended version. After a tag exists the wrapper never moves or rewrites
it. A second wrapper invocation may retry the first exact workflow run once
through GitHub's existing-run API: an ordinary failure reruns only failed jobs
and their dependents, while a cancelled or otherwise run-wide failure reruns
that same run because it may contain no failed job. The static candidate gate
can safely restage the same committed bytes with overwrite-safe artifact
upload; SHA, ref, and run ID remain unchanged and no competing pipeline is
created. If that unchanged retry also fails, or a
source/artifact correction is required, the wrapper stops; only the latter case
requires the exceptional next patch version. This same-SHA/ref rerun behavior
was checked against the [official GitHub Actions documentation](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs)
on 2026-07-20.

If a `release/vX.Y.Z` fallback run fails its static candidate gate before it
can create the tag, a corrected candidate remains on the same intended version.
The wrapper first retries the tag path. If tags are still blocked, it may
fast-forward that same fallback branch only after proving the old SHA is an
ancestor of the new main candidate, exactly one attributable old run completed
unsuccessfully, and neither the tag nor GitHub Release exists. It never force
pushes or advances a successful, active, unattributable, or already published
fallback.

The cross-platform release wrapper ensures both Linux targets are present: the
desktop app uses
`x86_64-unknown-linux-gnu` with a Zig-pinned glibc 2.17 baseline because winit
loads X11/Wayland libraries dynamically, while the updater, `se`, and Share
server remain standalone `x86_64-unknown-linux-musl` executables. The staged
GUI must create a real window under Xvfb before promotion. Zig also supplies C
dependencies where WSL lacks a system compiler, and the musl linker wrapper
filters the musl-only `-ldl` mismatch. The live feed remains untouched while
either platform build or staged verification is in progress; promotion
failures restore the prior feed and ancillary files.

Every artifact-mutating release path uses the same repository lock:
`release-native/.complete-release.lock`. Its creation is atomic across the
Windows and WSL views of the checkout, so a second release fails before it can
build or promote anything. Clean exits remove it. A hard crash deliberately
leaves owner metadata behind because Windows and WSL PIDs cannot be compared
safely. Verify that no release-related Windows, WSL, or Linux process remains
before deleting only that stale lock file. Failure-retained `.release-stage.*`,
`.complete-release-stage.*`, and Linux candidate paths are ignored by Git and
must not be committed as release assets.

Before cutting a release on a workstation, the fast environment check is:

```powershell
pwsh ./native/publish-release-local.ps1 -CheckEnvOnly
```

When the Linux GUI packaging path changed and a targeted preflight is relevant,
run this before the single complete build:

```bash
native/publish-linux-feed-wsl.sh --check-gui
```

It builds only the GNU/glibc 2.17 desktop target into Cargo's normal target
directory and proves that it opens a real X window. It does not touch or
promote `release-native`, write `version.txt`, or publish anything.

On every ordinary branch push and pull request, `build.yml` runs development
validation on native Windows and Ubuntu/mingw: formatting, dependency audit,
Windows-target checks, native Windows library and standalone-`se` tests,
all-target host tests (including built-`se` subprocess coverage), Windows
test-harness compilation, clippy, deterministic static-musl `se-agent` bundle
verification, COM DLL checks, share-server checks, and the multi-profile tracked
Share lifecycle. That path may create disposable test binaries, but it never
runs a complete release build, stages or uploads a candidate, creates a tag, or
publishes a GitHub Release.

An explicit `workflow_dispatch` with `verify_release_candidate=true`, or the
non-publishing `verify/v*` push fallback, enables the exact-candidate jobs
without enabling publication when verification without a release was requested.
That explicit verify-only path adds the committed Linux, mixed-version, and
Windows Share/Exec lifecycle runs; it still does not rebuild or rewrite the
candidate.

The normal release wrapper does not use the verify-only path. Its one tag push,
or its mutually exclusive `release/v*` fallback, runs only the static
publication consumer: version consistency, all six payload hashes, Windows
build-manifest source-parent binding, portable Windows/feed equality, installer payload equality,
ELF linkage, DLL exports, and the exact 18-file map are checked directly from
the candidate commit. The publication job downloads that staged set, fails if
any byte differs from the same commit or any extra/missing asset exists, checks
the six sidecars again, binds the immutable tag, and uploads those bytes. It
never invokes Cargo, the task-level suite, a GUI/runtime E2E, or another
candidate pipeline. Published app/updater/`se` payloads, hashes, and
`version.txt` are therefore byte-identical to the auto-update feed; the
installer, script, DLL, and Share servers are the same statically verified
committed ancillary artifacts.

The Linux installer first tries the verified GitHub Release payloads, so a
terminal-only installation needs no Rust or desktop toolchain:

```bash
curl -fsSL https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/install-linux.sh | sh -s -- --cli-only
```

If release assets are unavailable it normally falls back to a one-job local
Cargo build. The complete release wrapper pins
`SMART_EXPLORER_RELEASE_TAG=vX.Y.Z` and sets
`SMART_EXPLORER_REQUIRE_RELEASE_ASSETS=1` for its final local CLI update, so that
transaction can neither drift to another `latest` release nor silently compile
different bytes.
The Windows installer registers its exact install directory in the per-user
`PATH`; the helper records ownership so uninstall removes only the component it
added and refuses to delete `se.exe` if safe PATH cleanup fails.

The desktop app embeds the two static-musl SSH-agent payloads from
`native/agent-bin/`. `native/build-agent-bundles.sh` remaps the repository and
Cargo source roots to stable virtual paths and forces its own target directory,
then CI rebuilds both architectures and compares them byte-for-byte with the
committed payloads. Run that script and commit both binaries before a release;
do not bypass the exact-byte guard or copy binaries from an unrelated target
directory. A caller needing custom compiler flags must use an ordinary
diagnostic `cargo build`; the canonical bundle script rejects
`RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` because release payloads must use the
same compiler flags on every machine.

## The update feed (what the app reads)

A folder with OS-specific payloads, identical for a local folder or an
http(s)/Git URL — only the transport differs (`updater.rs`'s `Feed` enum):

```
release-native/update-feed/
  version.txt          first line = "X.Y.Z"
  windows-build.manifest   binds source commit + version to all three Windows payload hashes
  smart_explorer.exe   Windows app payload
  smart_explorer.exe.sha256
  smart_explorer_updater.exe   Windows updater helper
  smart_explorer_updater.exe.sha256
  se.exe               Windows terminal companion
  se.exe.sha256
  smart_explorer       Linux GNU/glibc desktop app payload (glibc 2.17+)
  smart_explorer.sha256
  smart_explorer_updater       Linux updater helper
  smart_explorer_updater.sha256
  se                   Linux terminal companion
  se.sha256
```

The Linux desktop payload relies on the normal X11 or Wayland client libraries
provided by desktop distributions. The `--cli-only` installer path needs none
of those GUI libraries because `se` remains a static-musl executable.

The normal update path uses a separate helper installed next to the app binary
(`Smart Explorer Updater.exe` on Windows, `smart_explorer_updater` on Linux) and
the feed also ships the terminal companion (`se.exe` on Windows, `se` on Linux).
For a newer version, the app downloads all three OS-specific payloads into app
data, verifies their required SHA-256 files, and durably records one staging
manifest. No installed file or process changes during this check. The dialog's
**Later** action keeps that exact verified staging for the next launch;
**Discard** removes it. Only explicit **Restart now** consent starts the helper.

The helper's launch protocol is a release compatibility boundary because an
older app downloads the **new** helper before asking it to apply the update.
The current helper therefore retains a fail-closed bridge for the exact
v0.5.119 argument set: it requires the staged app SHA-256, replaces only the app
(v0.5.119 already refreshed the CLI and helper), and refuses every UAC handoff.
The modern helper also fails closed when an apply needs elevation: launching the
replacement GUI from an elevated worker would incorrectly keep the app running
as administrator. Use the matching installer for a protected installation.
Any modern-only argument selects the full transactional parser and a malformed
modern request never downgrades to legacy mode. The bridge waits for the exact
old parent without an arbitrary timeout (v0.5.119 can leave the helper waiting
after **Later**) and serializes duplicate workers for the same target. A worker
retires an older/equal request only after a target-bound completion receipt
proves the installed winner; a newer request rebases on that verified winner
and attempts to apply without ever downgrading it. If the freshly launched
winner still holds the executable, the queued newer worker fails visibly and
retains its payload instead of claiming success. Before replacement, the helper
durably publishes an exclusive target-keyed intent containing the requested
version plus the old and staged SHA-256 values. A helper interrupted between
replacement and status publication therefore cannot let a later stale worker
adopt the new binary as its baseline: the next worker must complete that exact
intent or fail closed and direct the user to the installer. The receipt binds
the target key, version, and app SHA-256; after its first GUI frame the new app
syncs a private sibling, atomically publishes the nonce-bound receipt, then
completes a two-way loopback handshake with the helper. Publishing that receipt
is the irreversible commit point; the helper retains rollback state and the
durable intent until then. Recovery waits for an already running exact target
to publish that receipt and fails closed if it does not; it never launches a
duplicate replacement instance.
Update status/error state is prepared before launch so startup cannot consume
stale state. The acknowledged replacement launch defers abandoned-staging
cleanup until the next ordinary start, allowing queued v0.5.119 workers to
retain their verified payloads. Keep the
legacy-argv, serialization/rebase, durable-intent, completion-receipt,
prelaunch-state, rollback, path-alias, and tamper regressions until the minimum
source is newer than v0.5.119.

The `.sha256` files are integrity checks for broken or partial downloads. They
are not a substitute for code signing. The industry-standard trust path for
Windows distribution is still: sign every release, keep one stable publisher
identity, publish every version as a GitHub Release, and let Windows/AV
reputation build on that identity.

The update **source** the app points at (Sidebar → UPDATE, or the app data
`update_source.txt`; `%APPDATA%\smart_explorer\` on Windows,
`$XDG_DATA_HOME/smart_explorer/` or `~/.local/share/smart_explorer/` on Linux)
may be:

- a **GitHub repo link** — `https://github.com/b1ue-man/smart-explorer`
  (translated to the `main` raw feed automatically), **or**
- any **https URL** to a feed folder, **or**
- a **local folder / `\\server\share`** path.

## How the app self-updates (`updater.rs`)

On every launch (and on "Jetzt prüfen"):
1. resolve the update source; fetch the feed's `version.txt`;
2. if the feed version is **newer** than the running binary (`CARGO_PKG_VERSION`),
   download and hash-verify the OS-specific app/updater/`se` bundle, persist its
   manifest, and prompt without changing the installation;
3. after explicit consent, preserve in-flight app state, launch the hash-bound
   helper, and close the GUI. The helper waits for that exact parent PID, asks
   the daemon to stop naturally, and refuses to replace while another matching
   app process remains;
4. verify and archive the outgoing app with a SHA-256 sidecar, revalidate every
   staged executable, and reject aliased target/staging/status paths. Replace
   helper, `se`, then app while keeping each installed target name continuously
   populated (`ReplaceFileW` on Windows; verified backup plus atomic rename on
   Linux). Prepare visible status, launch the verified app, and retain rollback
   files until its first GUI frame returns a durable nonce acknowledgement. A
   replacement or acknowledged-launch failure rolls all targets and status back
   and attempts to start the verified previous app;
5. equal/older → up to date. A manual rollback atomically pins the selected
   version and pauses automatic forward checks until "Auf neueste aktualisieren".

So a release is "done" only when, for the new version: `Cargo.toml` = feed
`version.txt` = Windows build manifest = Release tag = installer version; all
six update payload hashes verify; the manifest's `source_commit` equals the
release-candidate commit's sole parent; and the GitHub Release is visible with the
Windows/Linux app, updater, and `se` payloads and hashes, installer,
`install-linux.sh`, context-menu DLL, both share-server payloads, and
`version.txt`.

### Troubleshooting: socket access denied

If the update check fails with `os error 10013` / "Zugriff auf einen Socket war
aufgrund der Zugriffsrechte des Sockets unzulässig", the GitHub feed can still
be fine. Bitdefender Firewall has blocked Smart Explorer this way before. Check
the Bitdefender/Windows Firewall app rule for `Smart Explorer.exe` and allow
outbound HTTPS to `raw.githubusercontent.com`.

## Quick consistency check

```bash
grep '^version' native/Cargo.toml
cat release-native/update-feed/version.txt
ls "release-native/Smart Explorer Setup "*.exe
cd release-native/update-feed && sha256sum -c smart_explorer.exe.sha256 && sha256sum -c smart_explorer_updater.exe.sha256 && sha256sum -c se.exe.sha256 && sha256sum -c smart_explorer.sha256 && sha256sum -c smart_explorer_updater.sha256 && sha256sum -c se.sha256
grep -Fx "version=$(sed -nE 's/^version = \"([^\"]+)\".*/\1/p' ../../native/Cargo.toml | head -1)" windows-build.manifest
grep -Fx "source_commit=$(git -C ../.. rev-parse HEAD^)" windows-build.manifest  # release candidate's exact source parent
git show origin/main:release-native/update-feed/version.txt   # must match, on main
```

## Bitdefender / antivirus trust

The installer cannot reliably or appropriately tell Bitdefender "trust this app"
without the user's action. For Bitdefender Advanced Threat Defense, the user can
add explicit `.exe` exceptions. Add both installed executables if needed:

- `%LOCALAPPDATA%\Programs\Smart Explorer\Smart Explorer.exe`
- `%LOCALAPPDATA%\Programs\Smart Explorer\Smart Explorer Updater.exe`
- `%LOCALAPPDATA%\Programs\Smart Explorer\se.exe`

The updater helper itself does not need outbound network access; it only applies
an already-downloaded staged update. Long-term, the accepted Windows pattern is
code signing every release with a stable publisher identity so SmartScreen and
security products can build reputation across versions.
