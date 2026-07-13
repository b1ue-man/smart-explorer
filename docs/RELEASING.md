# Releasing & the update flow (end-to-end)

How a new version goes from a commit to an installed app updating itself.
One version number drives everything: `native/Cargo.toml`.

```
 finish task batch + development validation
   ─▶ bump Cargo.toml once
   ─▶ one complete local release build
       ├─▶ update feed   version + manifest + Windows/Linux app/updater/se + hashes
       └─▶ installer     Windows NSIS + Linux install-linux.sh
   ─▶ commit + push main
   ─▶ release trigger ─▶ exact-byte CI validation/E2E
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

## Cut a release

1. **Finish the complete task batch and development validation first.** Normal
   commits and pushes may run formatting, checks, tests, disposable builds, and
   E2E, but they do not bump a version, rewrite `release-native/`, stage release
   assets, or publish anything. Run the workstation preflight before entering
   the release stage.
2. **Bump once**: set `version` in `native/Cargo.toml` to the one intended patch
   version for the whole batch.
3. **Build + stage once** with one complete release workflow:
   - Windows workstation default: `.\native\publish-release-local.ps1`
     builds the Windows app/updater/`se`/installer with `publish-update.ps1`, then
     calls WSL to build the dynamic GNU/glibc Linux GUI plus static-musl
     updater/`se`/share-server payloads.
     Both platforms are built and verified in one isolated release tree. Only
     then does the wrapper promote the ancillary artifacts and complete feed
     with rollback backups, writing `version.txt` as the final commit marker.
     This is the preferred local release path.
   - Linux/WSL Linux payload repair only:
     `native/publish-linux-feed-wsl.sh --write-version`. This script prepares
     temporary Zig/LLD wrappers automatically and can bootstrap Zig into
     `~/.local/zig` when missing. Before it may write a version, the existing
     Windows payloads/hashes must be bound to the same version by
     `windows-build.manifest`; otherwise it refuses to publish.
   - Windows-only validation bundle:
     `.\native\publish-release-local.ps1 -SkipLinuxFeed`. It writes a clearly
     non-publishable `release-native/windows-partial-vX.Y.Z/` bundle without
     changing the shared update feed or its `version.txt`. A direct
     `publish-update.ps1` run also requires `-AllowPartialFeed`, a nonstandard
     `-Feed` path, and an explicit isolated `-ReleaseOutput` outside both the
     feed and the shared `release-native/` root; it always refuses to write the
     shared feed/artifacts.
   - Complete Linux/WSL cross-build path: `native/publish-feed.sh`, when that
     host has the Windows GNU and NSIS dependencies installed. It builds the
     same complete Windows/Linux feed, portable files, context-menu DLL, Share
     servers, and installer, verifies them in staging, and writes
     `version.txt` last. This is the supported full-release path on a Linux
     release host.
   Choose exactly one complete host path. The repair and partial-bundle commands
   resume or diagnose a failed stage; they are not additional release cycles
   after a successful complete build.
4. **Commit** the version and `release-native/` (`update-feed/{version.txt, windows-build.manifest, smart_explorer.exe,
   smart_explorer_updater.exe, se.exe, smart_explorer, smart_explorer_updater,
   se, *.sha256}`, `Smart Explorer.exe`, `Smart Explorer Updater.exe`, `se.exe`,
   `smart_explorer_command.dll`, both `share-server/` payloads, and
   `Smart Explorer Setup X.Y.Z.exe`).
5. **Merge to `main`** — the feed is served from `main`, so updates only go live
   once `main` has the new feed:
   ```
   git push origin <branch>:main          # fast-forward
   ```
6. **Verify the exact candidate before tagging.** Dispatch `build.yml` at that
   exact `main` commit with `verify_release_candidate=true` and
   `publish_release=false`. This does not rebuild or publish a release. It
   validates and temporarily stages the 18 committed assets, then runs the
   committed Linux and Windows GNU `se`/Share-server bytes through their exact
   platform E2E. Fix failures on the same intended version and repeat only the
   failed build stage or verification; do not create a tag until this run is
   green.
7. **Publish the GitHub Release** (attaches OS payloads/hashes, installer, DLL,
   install script, both Share servers, and `version.txt`):
   - Normally: push a tag — CI's `build.yml` releases on `v*`:
     ```
     git tag vX.Y.Z && git push origin vX.Y.Z
     ```
   - If tag push is unavailable but GitHub Actions dispatch is authorized, run
     `build.yml` with `workflow_dispatch` at the exact release commit and set
     the required Boolean input `publish_release` to `true`. A dispatch with the
     default `false` runs development validation only. The explicit release
     dispatch creates `vX.Y.Z` from `Cargo.toml` and publishes that release.
   - Where the git host rejects tag pushes (e.g. some sandboxes), push a release
     branch as the final fallback — CI releases only on `release/v*`, creating the
     tag from `Cargo.toml`'s version:
     ```
     git push origin <branch>:release/vX.Y.Z
     ```
     Delete the branch after the release is published; it's only a trigger.

   Every publication path requires the exact candidate commit to already be
   contained in `origin/main`. An existing `vX.Y.Z` must point to that exact
   commit; CI never moves or rewrites a tag. Dispatch and release-branch
   fallbacks create a missing tag immediately before publication and abort if
   another commit claimed it concurrently.

The local Windows release wrapper expects WSL with Rust installed. It ensures
both Linux targets are present: the desktop app uses
`x86_64-unknown-linux-gnu` with a Zig-pinned glibc 2.17 baseline because winit
loads X11/Wayland libraries dynamically, while the updater, `se`, and Share
server remain standalone `x86_64-unknown-linux-musl` executables. The staged
GUI must create a real window under Xvfb before promotion. Zig also supplies C
dependencies where WSL lacks a system compiler, and the musl linker wrapper
filters the musl-only `-ldl` mismatch. The live feed remains untouched while
either platform build or staged verification is in progress; promotion
failures restore the prior feed and ancillary files.

Before cutting a release on a workstation, the fast environment check is:

```powershell
.\native\publish-release-local.ps1 -CheckEnvOnly
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

An explicit `workflow_dispatch` with `verify_release_candidate=true` enables
the exact-candidate jobs without enabling publication. A `v*` tag, an explicit
dispatch with `publish_release=true`, or the documented `release/v*` fallback
also enables those gates and enables publication only after they pass. CI
deliberately does not rebuild the release there: it
checks the version, all six payload hashes, Windows build manifest, portable
Windows/feed byte equality, installer and all ancillary assets directly from
the exact commit. It also starts the committed GNU/glibc Linux GUI under Xvfb,
checks the static headless Linux payloads and DLL exports, stages exactly those
committed bytes, runs the committed static Linux `se` and Share server through
the full Share/Exec lifecycle, and uploads them for the native-Windows GNU `se.exe` and
`se-share-server.exe` lifecycle E2E. Publication is a separate dependent job
and cannot start before that exact-binary gate succeeds. Published
app/updater/`se` payloads, hashes, and `version.txt` are therefore byte-identical
to the auto-update feed; the installer, script, DLL, and Share servers are the
same verified committed ancillary artifacts. The publication action treats any
unmatched one of the 18 required asset paths as a hard failure.

The Linux installer first tries the verified GitHub Release payloads, so a
terminal-only installation needs no Rust or desktop toolchain:

```bash
curl -fsSL https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/install-linux.sh | sh -s -- --cli-only
```

If release assets are unavailable it falls back to a one-job local Cargo build.
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
  windows-build.manifest   binds the version to all three Windows payload hashes
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
six update payload hashes verify; and the GitHub Release is visible with the
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
