# Releasing & the update flow (end-to-end)

How a new version goes from a commit to an installed app updating itself.
One version number drives everything: `native/Cargo.toml`.

```
 bump Cargo.toml ─▶ build (CI or publish-feed.sh)
                     ├─▶ update feed   release-native/update-feed/{version.txt, OS payloads}   (committed → served over raw.githubusercontent)
                     ├─▶ installer     Windows NSIS + Linux install-linux.sh
                     └─▶ GitHub Release vX.Y.Z  (Windows + Linux binaries, installer, script, dll, version.txt)
                                         │
 installed app on launch ──▶ reads update_source (default: the Git feed on main)
                          ──▶ feed version.txt > my version?  ──▶ download OS payload, swap, restart
```

The version is consistent across all four outputs because each reads it from
`Cargo.toml`. Never hand-edit `version.txt` — the scripts/CI write it.

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

1. **Bump** `version` in `native/Cargo.toml`. Commit.
2. **Build + stage** the release artifacts:
   - Linux / WSL (cross): `native/publish-feed.sh`
     — builds Windows via `x86_64-pc-windows-gnu`, also builds
       `smart_explorer` + `smart_explorer_updater` for the Linux feed, and if
       `makensis` is installed it builds the Windows installer.
   - Windows (native): `cd native; .\publish-update.ps1` — builds the Windows
     feed payloads and NSIS installer. For a complete Windows+Linux release feed,
     run `native/publish-feed.sh` on Linux/WSL before committing. The Windows
     script refuses to update the shared repo feed when Linux payloads are
     present unless `-AllowPartialFeed` is passed for an explicit Windows-only
     feed.
3. **Commit** `release-native/` (`update-feed/{version.txt, smart_explorer.exe,
   smart_explorer_updater.exe, smart_explorer, smart_explorer_updater, *.sha256}`,
   `Smart Explorer.exe`, `Smart Explorer Updater.exe`,
   `Smart Explorer Setup X.Y.Z.exe`).
4. **Merge to `main`** — the feed is served from `main`, so updates only go live
   once `main` has the new feed:
   ```
   git push origin <branch>:main          # fast-forward
   ```
5. **Publish the GitHub Release** (attaches OS payloads + installer + dll + script + version.txt):
   - Normally: push a tag — CI's `build.yml` releases on `v*`:
     ```
     git tag v0.5.3 && git push origin v0.5.3
     ```
   - Where the git host rejects tag pushes (e.g. some sandboxes), push a release
     branch instead — CI releases on `release/**`, creating the tag from
     `Cargo.toml`'s version:
     ```
     git push origin <branch>:release/v0.5.3
     ```
     Delete the branch after the release is published; it's only a trigger.

`build.yml` does the whole thing on CI (ubuntu + mingw-w64 +
`x86_64-pc-windows-gnu`, the verified cross-compile): format check, dependency
audit, Windows-target check, host tests, Windows test-harness compile, clippy,
static-musl `se-agent` builds, COM DLL check/build, share-server checks/builds,
Windows + Linux release builds, installer build (NSIS), artifact upload, and
Release publication. Before publishing it **fails the release if the committed
feed `version.txt` ≠ `Cargo.toml`** — so a release can never ship while the
auto-update feed version is stale (forces step 2–3 above).

## The update feed (what the app reads)

A folder with OS-specific payloads, identical for a local folder or an
http(s)/Git URL — only the transport differs (`updater.rs`'s `Feed` enum):

```
release-native/update-feed/
  version.txt          first line = "0.5.3"
  smart_explorer.exe   Windows app payload
  smart_explorer.exe.sha256
  smart_explorer_updater.exe   Windows updater helper
  smart_explorer_updater.exe.sha256
  smart_explorer       Linux app payload
  smart_explorer.sha256
  smart_explorer_updater       Linux updater helper
  smart_explorer_updater.sha256
```

Since v0.5.77, the normal update path uses a separate
updater helper installed next to the app binary (`Smart Explorer Updater.exe`
on Windows, `smart_explorer_updater` on Linux). The app stages the OS-specific
payload, refreshes the helper from the same feed, then exits while the helper
performs the replacement and relaunches the app.
The one unavoidable migration exception is v0.5.76 -> v0.5.77: v0.5.76 does
not know how to fetch the helper yet, so it can only update the main exe. On
the first v0.5.77 launch, the app silently ensures the helper is present for
all later updates.

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
   download the OS-specific app payload, archive the current binary (for rollback),
   "rename-dance" the new one in, and prompt to restart;
3. equal/older → up to date. A manual rollback pins the version and pauses
   auto-update until "Auf neueste aktualisieren".

So a release is "done" when, for the new version: `Cargo.toml` = feed
`version.txt` = Release tag = installer version, and the feed + Release live on
`main`.

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
git show origin/main:release-native/update-feed/version.txt   # must match, on main
```

## Bitdefender / antivirus trust

The installer cannot reliably or appropriately tell Bitdefender "trust this app"
without the user's action. For Bitdefender Advanced Threat Defense, the user can
add explicit `.exe` exceptions. Add both installed executables if needed:

- `%LOCALAPPDATA%\Programs\Smart Explorer\Smart Explorer.exe`
- `%LOCALAPPDATA%\Programs\Smart Explorer\Smart Explorer Updater.exe`

The updater helper itself does not need outbound network access; it only applies
an already-downloaded staged update. Long-term, the accepted Windows pattern is
code signing every release with a stable publisher identity so SmartScreen and
security products can build reputation across versions.
