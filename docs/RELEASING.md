# Releasing & the update flow (end-to-end)

How a new version goes from a commit to an installed app updating itself.
One version number drives everything: `native/Cargo.toml`.

```
 bump Cargo.toml ─▶ build (CI or publish-feed.sh)
                     ├─▶ update feed   release-native/update-feed/{version.txt, smart_explorer.exe}   (committed → served over raw.githubusercontent)
                     ├─▶ installer     release-native/Smart Explorer Setup X.Y.Z.exe
                     └─▶ GitHub Release vX.Y.Z  (exe + installer + dll + version.txt)
                                         │
 installed app on launch ──▶ reads update_source (default: the Git feed on main)
                          ──▶ feed version.txt > my version?  ──▶ download exe, swap, restart
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
2. **Build + stage** the Windows artifacts:
   - Linux / WSL / macOS (cross): `native/publish-feed.sh`
     — builds the exe, refreshes `release-native/update-feed/`, copies the
       portable exe, and (if `makensis` is installed) builds the installer.
   - Windows (native): `cd native; .\publish-update.ps1` — same, plus the
     installer via NSIS.
3. **Commit** `release-native/` (`update-feed/{version.txt, smart_explorer.exe}`,
   `Smart Explorer.exe`, `Smart Explorer Setup X.Y.Z.exe`).
4. **Merge to `main`** — the feed is served from `main`, so updates only go live
   once `main` has the new feed:
   ```
   git push origin <branch>:main          # fast-forward
   ```
5. **Publish the GitHub Release** (attaches exe + installer + dll + version.txt):
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
`x86_64-pc-windows-gnu`, the verified cross-compile): check → host tests →
release build → COM DLL → build installer (NSIS) → upload artifact → publish the
Release. Before publishing it **fails the release if the committed feed
`version.txt` ≠ `Cargo.toml`** — so a release can never ship while the
auto-update feed is stale (forces step 2–3 above).

## The update feed (what the app reads)

A folder with exactly two files, identical for a local folder or an http(s)/Git
URL — only the transport differs (`updater.rs`'s `Feed` enum):

```
release-native/update-feed/
  version.txt          first line = "0.5.3"
  smart_explorer.exe   the new binary (the app downloads + swaps this)
```

The update **source** the app points at (Sidebar → UPDATE, or
`%APPDATA%\smart_explorer\update_source.txt`) may be:

- a **GitHub repo link** — `https://github.com/b1ue-man/smart-explorer`
  (translated to the `main` raw feed automatically), **or**
- any **https URL** to a feed folder, **or**
- a **local folder / `\\server\share`** path.

## How the app self-updates (`updater.rs`)

On every launch (and on "Jetzt prüfen"):
1. resolve the update source; fetch the feed's `version.txt`;
2. if the feed version is **newer** than the running exe (`CARGO_PKG_VERSION`),
   download `smart_explorer.exe`, archive the current binary (for rollback),
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
