# Android CI Task-Suite and Release Pipeline Integration

**Purpose:** Describe the existing remote CI task-suite pattern and the
`build.yml` release pipeline precisely enough that a future Android APK
build and a new Android task suite can be added without duplicating or
colliding with either.

## Files read

- `.github/workflows/*.yml` (all 11 files; `build.yml` and
  `filter-transfer-task.yml` read in full, the other 9 task workflows
  scanned for `paths:`/Android/Java/Gradle references only)
- `native/publish-release-local.ps1` (full, 1272 lines)
- `native/release-publication.ps1` (full, 1559 lines; asset-map, source-commit
  and publication-wait functions read in full, process/token helpers scanned)
- `native/release-lock.ps1` (full, 66 lines)
- `native/publish-update.ps1` (full, 338 lines — structure/param surface)
- `native/publish-feed.sh` (full, 424 lines)
- `native/publish-linux-feed-wsl.sh` (full, 710 lines)
- `docs/RELEASING.md` (full, 721 lines)
- `native/test-filter-transfer-task.sh` (full, 342 lines — task-suite pattern example)
- `native/run-task-memory-bounded.sh` (full, 120 lines — task-suite pattern example)
- `release-native/update-feed/` (names-only `ls -la`)
- `README.md` sections "⬇️ Installieren", "🔄 Updates bekommen", "Release veröffentlichen"

No file outside this list was read. No file was created or modified except
this one.

---

## 1. Task-suite workflow pattern (`filter-transfer-task.yml` as the template)

`.github/workflows/filter-transfer-task.yml:1-157` is the template every
per-batch task suite follows (`analytics-access-task.yml`,
`copy-paste-task.yml`, `gui-design-task.yml`, `lan-cleanup-task.yml`,
`mount-batching-task.yml`, `mount-optimization-task.yml`,
`search-recursive-access-task.yml`, `share-remote-task.yml`,
`sync-paths-task.yml` all share this shape; none of them declares a
`paths:` filter).

| Aspect | Value (filter-transfer-task.yml) |
|---|---|
| Trigger | `workflow_dispatch` only, one required input `candidate_sha` (full 40-hex commit SHA) — `:11-16` |
| Permissions | `contents: read` — `:18-19` |
| Concurrency | `group: <workflow-name>-${{ github.repository }}`, `cancel-in-progress: false` — `:21-23` |
| Jobs | One per OS that has directly-affected behavior; here `linux` (`:26-99`) and `windows` (`:100-157`) |
| Runners | `ubuntu-latest` (linux job) and `windows-2025` (windows job) |
| Job timeout | 170 min (linux), 180 min (windows) — `:28`, `:102` |
| Candidate binding | Each job re-checks out `ref: ${{ github.sha }}`, then a "Bind the run to the exact candidate" step asserts `candidate_sha` matches both `GITHUB_SHA` and `git rev-parse HEAD` (`:41-62`, `:115-133`) — this is the exact-SHA gate every task suite repeats |
| Toolchain | `dtolnay/rust-toolchain@stable` with `targets: x86_64-pc-windows-gnu`, `components: rustfmt, clippy` (linux job only) — `:64-67`; windows job just `dtolnay/rust-toolchain@stable` — `:135` |
| Extra Linux deps | `mingw-w64`, `jq` via `apt-get` — `:73-76` |
| Cache | `Swatinem/rust-cache@v2`; linux job caches `workspaces: native \n share-server`; windows job caches `workspaces: native` with `shared-key: windows-mount-batching`, both `cache-on-failure: true` — `:78-84`, `:137-141` |
| Suite invocation | `bash native/test-filter-transfer-task.sh --direct 2>&1 \| tee "$SMART_EXPLORER_TASK_LOG_ROOT/suite.log"` — one command, one script, `--direct` mode (relies on the runner's own resource limits instead of the local `systemd-run` memory-bounded wrapper) — `:85-91`, `:143-149` |
| Step timeout | 160 min (linux step) / 170 min (windows step), i.e. job timeout minus ~10 min headroom — `:87`, `:145` |
| Artifacts on failure | `actions/upload-artifact@v4`, name `<suite>-<os>-${{ inputs.candidate_sha }}-${{ github.run_attempt }}`, path `${{ runner.temp }}/<suite>-logs`, only `if: failure() || cancelled()` — `:93-98`, `:151-156` |
| Publishing | None — the workflow never publishes anything (comment `:5`) |

### `native/test-filter-transfer-task.sh` (the entrypoint the workflow calls)

- Single checked-in script, `set -Eeuo pipefail`, `usage()`/arg parsing for
  `--bounded` (default, uses `run-task-memory-bounded.sh` on Linux) vs.
  `--direct` (CI mode, no local cgroup wrapper) — `:1-31`.
- Detects platform via `uname -s` (`windows` vs `linux`) — `:28-31`.
- On failure, `report_failure()`/`trap ... ERR` prints the failing line and
  command, `cleanup()`/`trap ... EXIT` preserves diagnostics under
  `$SMART_EXPLORER_TASK_LOG_ROOT` (set by the workflow) on failure, deletes
  them on success — `:33-62`.
- Pins `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`,
  `CARGO_PROFILE_{TEST,DEV}_DEBUG=0`, `CARGO_TERM_COLOR=never`, and (Linux
  only) an explicit `CARGO_TARGET_DIR` — `:77-87`.
- `run_task()` wraps every Cargo invocation, optionally routing through
  `native/run-task-memory-bounded.sh` in `--bounded` mode — `:89-95`.
- Declares the milestone test list as bash arrays (`native_tests`,
  `integration_paths`), builds a platform-specific superset on Windows
  (`:99-140`), then runs exactly two `cargo test --locked --lib` invocations
  (`recursive_filter_task_` prefix, then the named integration tests) and
  verifies the **exact** pass count and **exact** test names via
  `verify_test_log()` against `test result: ok. N passed; 0 failed; ...` —
  `:167-209`. This exact-count assertion is the "acceptance signal" pattern:
  the suite fails if the milestone set drifts (missing or extra passing
  tests), not just on any failure.
- Linux-only continuation (`:211-339`): builds only the two needed dev
  binaries (`se`, `se-share-server`), runs a Share Room E2E bash script
  end-to-end, then gates only the batch's own changed files with
  `rustfmt --check` per file (stdin mode) and `cargo clippy` filtered to
  diagnostics on lines the batch changed (via a `git diff` line-range
  filter against a fixed `batch_base` commit SHA) — never a whole-crate
  format/lint gate.
- `native/run-task-memory-bounded.sh` (called only in `--bounded` mode) uses
  `systemd-run --scope` with decreasing property sets
  (`MemoryHigh=1792M`/`MemoryMax=2G`/`MemorySwapMax=256M` →
  `MemoryHigh=1792M`/`MemoryMax=2G` → `MemoryMax=2G`) and a probe that reads
  `/sys/fs/cgroup/.../memory.max` to confirm the limit is actually
  effective before trusting it; refuses to run (`exit 1`) if no usable
  cgroup boundary exists — `:1-121`. CI's `--direct` mode skips this
  entirely and relies on the runner's own limits instead.

**Implication for an Android suite:** a new
`.github/workflows/android-*-task.yml` should copy this exact shape
(workflow_dispatch + `candidate_sha`, one job per required host — an
Ubuntu job for the Gradle/NDK cross-compile plus JNI-linked Rust unit
tests, optionally an emulator job — one checked-in
`native/test-android-*-task.sh` or similarly named script, exact-count
milestone assertions, `--direct` execution, failure-only log upload) rather
than inventing a new pattern. It must never publish and must be dispatched
by full pushed `candidate_sha`, matching every existing task suite.

---

## 2. `build.yml`: triggers, jobs, complete-release job, `[task candidate]` skip

### Triggers (`:17-38`)

```yaml
on:
  push:
    branches: ["**"]
    tags: ["v*"]
  pull_request:
  workflow_dispatch:
    inputs:
      verify_release_candidate: {boolean, default false}
      publish_release:          {boolean, default false}
      complete_release_source_sha: {string, default ""}
```

No `paths:`/`paths-ignore:` filter exists on `build.yml` or on any of the
other ten workflow files (confirmed by `grep -n "paths:" .github/workflows/*.yml`
returning nothing). Every push to every branch, and every tag `v*`, fires
this workflow.

### Jobs (in file order)

| Job | `if:` condition (paraphrased) | Runner | Timeout |
|---|---|---|---|
| `complete-release` | `workflow_dispatch` with `complete_release_source_sha != ''` | `windows-2025` | 360 min job / 330 min for the wrapper step |
| `windows-native-tests` | PR, or an ordinary branch push whose head commit does **not** end in `[task candidate]`, is not on `verify/v*`/`release/v*`, and (if `main`) does not end in `[release candidate]` | `windows-latest` | (none set — GitHub default 360 min) |
| `windows-gnu` | `needs: windows-native-tests`, `if: needs.windows-native-tests.result == 'success'` | `ubuntu-latest` | (none set) |
| `release-candidate` | tag push `refs/tags/v*`, or `workflow_dispatch` with `publish_release` or `verify_release_candidate` true (and `complete_release_source_sha==''`), or push to `refs/heads/verify/v*` or `refs/heads/release/v*` | `ubuntu-latest` | (none set) |
| `windows-gnu-release-e2e` | `needs: release-candidate` succeeded, and (`verify_release_candidate` dispatch or `verify/v*` push) | `windows-latest` | (none set) |
| `publish-release` | `needs: release-candidate` succeeded, and (tag push, or `publish_release` dispatch, or `release/v*` push) | `ubuntu-latest` | (none set) |

### `complete-release` job (`:44-186`) — inputs, runner, timeout, steps

- **Guard** (`:45-47`): only fires on `workflow_dispatch` with
  `complete_release_source_sha != ''`.
- **Runner/timeout**: `windows-2025`, `timeout-minutes: 360` (`:48-49`).
- **Concurrency**: `group: smart-explorer-complete-release`,
  `cancel-in-progress: false` (`:50-52`) — this is the one serialization
  point preventing two complete releases from running at once on this
  workflow.
- **Permissions**: `actions: write`, `contents: write` (`:53-55`) — the only
  job in the file with write permissions beyond `publish-release`'s
  `contents: write`.
- **Steps**:
  1. `actions/checkout@v7` with `fetch-depth: 0`, `ref: main` (`:57-60`).
  2. "Bind complete release to exact main source" (pwsh, `:62-95`): asserts
     `GITHUB_REF == refs/heads/main`; asserts `publish_release`/
     `verify_release_candidate` were **not** also set (mutual exclusion);
     validates `complete_release_source_sha` is one full 40/64-hex object
     ID; requires `GH_TOKEN`/`GITHUB_TOKEN`; fetches `origin/main` and
     asserts `HEAD == origin/main == GITHUB_SHA == complete_release_source_sha`
     (all lower-cased); sets a bot git identity.
  3. "Prepare Windows release tools" (`:97-99`): `choco install
     strawberryperl nsis -y --no-progress`. **No JDK, Android SDK, NDK, or
     Gradle is installed here or anywhere else in this job.**
  4. "Select WSL1 before Ubuntu installation" (`:101-106`):
     `wsl.exe --set-default-version 1`.
  5. "Prepare Ubuntu WSL1 release environment" (`:108-112`): pinned
     `Ubuntu/WSL/.github/actions/wsl-install@<sha>` action, `distro:
     Ubuntu-24.04`, `useStore: false`.
  6. "Prepare and preflight WSL release tools" (`:114-173`): verifies
     exactly one `Ubuntu-24.04` WSL**1** distro exists, sets it default,
     then `apt-get install`s: `build-essential ca-certificates curl file
     git musl-tools p7zip-full pkg-config python3 xauth x11-utils xvfb
     libegl1 libgl1 libgl1-mesa-dri libx11-6 libx11-xcb1 libxcursor1
     libxi6 libxkbcommon0 libxkbcommon-x11-0 libxrandr2`, bootstraps
     `rustup` (profile minimal) if absent, adds `rustfmt clippy`
     components, then hard-checks that `bash, cargo, rustc, rustup, gcc,
     musl-gcc, file, git, xvfb-run, xauth, xwininfo` are all on `PATH`.
     **Again: no `java`, no `sdkmanager`/Android command-line tools, no
     `gradle` check anywhere in this list.**
  7. "Run one complete release transaction" (`:175-186`): `shell: pwsh`,
     `timeout-minutes: 330`, env `GH_TOKEN`/`GITHUB_TOKEN`/
     `SMART_EXPLORER_COMPLETE_RELEASE_SOURCE_SHA`; runs exactly
     `pwsh ./native/publish-release-local.ps1 -SkipLocalCliUpdate
     -PublicationTimeoutMinutes 180`. This is the **only** place the
     top-level release wrapper is invoked from CI.

### `[task candidate]` commit-message skip mechanism

- `windows-native-tests`'s `if:` (`:189-196`) excludes a push whose
  `github.event.head_commit.message` **ends with** `[task candidate]`
  (`endsWith(..., '[task candidate]') == false`), and separately excludes
  `main` pushes ending in `[release candidate]`.
- `windows-gnu` (`:248-250`) has `needs: windows-native-tests` and
  `if: needs.windows-native-tests.result == 'success'`. When
  `windows-native-tests` is skipped by the condition above, its `needs`
  result is `'skipped'`, not `'success'`, so `windows-gnu` is transitively
  skipped too.
- Net effect: any push whose head commit ends in `[task candidate]` (or, on
  `main`, `[release candidate]`) skips the entire ordinary
  Windows/Ubuntu development-CI matrix in this workflow, leaving only the
  dedicated, exact-SHA-dispatched task-suite workflow (e.g.
  `filter-transfer-task.yml`) to evaluate that commit — confirmed in prose
  at `docs/RELEASING.md:497-508` ("A pushed candidate whose head commit ends
  in `[task candidate]` is the deliberate exception ... never runs a
  complete release build").
- An Android task-suite workflow does not need to replicate this skip logic
  itself; it only needs its own `workflow_dispatch`/`candidate_sha` gate.
  If Android CI adds its own always-on push trigger (not recommended — see
  §5), it would need the same `endsWith(... '[task candidate]') == false`
  style guard to avoid running twice on the same task-candidate push.

---

## 3. `publish-release-local.ps1` (the terminal wrapper) — ordered stages

Entry point: `native/publish-release-local.ps1`, sourced helpers
`release-lock.ps1` and `release-publication.ps1` (`:25-36`).

| # | Stage | Function / lines | What it does |
|---|---|---|---|
| 1 | Global Cargo pins | `:15-23` | `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_RELEASE_LTO=thin`, `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8`, `CARGO_PROFILE_RELEASE_DEBUG=0` |
| 2 | Common preflight | `Assert-CommonReleasePreflight` `:1086-1116` | git present; GitHub-Actions-context binding (`Assert-GitHubActionsCompleteReleaseContext` `:601-615`); `Assert-GitReleasePreflight` (branch=main, HEAD==origin/main or exactly one pending `[release candidate]` commit, no tracked changes outside the release-mutable path set, `Assert-ReleaseVersionRecoveryState`) `:918-940`; `Assert-PublicationNoUntrackedBuildInputs`; Dokany private-DLL `-VerifyOnly -RequireApproved` check (refuses `SMART_EXPLORER_DOKANY_DLL_*` overrides); resolves `$script:githubRepository`; requires a GitHub token; asserts `build.yml` workflow is `active`; `Resolve-ReleasePlan` (Tagged/Bump/Resume decision `:977-1012`); `Assert-NonInteractiveGitWriteAccess` (dry-run push probes for `main`, the tag ref, and the `release/vX.Y.Z` fallback ref); then OS-specific environment assertion |
| 3 | Cross-host lock | `Enter-CompleteReleaseLock` (`release-lock.ps1:4-51`) | Atomic `FileMode.CreateNew` on `release-native/.complete-release.lock`; writes `token/owner/pid/host/started_utc` metadata; on failure to acquire, prints the file's metadata and tells the caller to verify no other release process is alive before removing a stale lock |
| 4 | Version bump / resume | `Set-NativeVersion` `:741-790`, driven by `Resolve-ReleasePlan` | Rewrites `native/Cargo.toml` (`^version\s*=\s*"..."`) and the sole `[[package]] name = "smart_explorer"` entry in `native/Cargo.lock`, only advancing to the exact next patch or resuming the same value; staged via temp files then `[System.IO.File]::Move(...,$true)`, **lock file renamed before Cargo.toml** so a crash mid-bump is recoverable |
| 5 | Windows build | `Invoke-WindowsReleaseBuild` `:348-559`, calling `publish-update.ps1 -Feed $stageFeed -ReleaseOutput $stageRelease -AllowPartialFeed -DeferFeedVersion` into an **isolated stage directory** (`.complete-release-stage.<pid>.<guid>`), then WSL `publish-linux-feed-wsl.sh` for the Linux payloads (env `SMART_EXPLORER_FEED_DIR`/`SMART_EXPLORER_SHARE_DIR` point at the same stage) | Builds `Smart Explorer Setup $version.exe`, `Smart Explorer.exe`, `Smart Explorer Updater.exe`, `se.exe`, `share-server\se-share-server.exe`, `smart_explorer_command.dll`, plus (unless `-SkipLinuxFeed`) `smart_explorer`, `smart_explorer_updater`, `se`, `se-share-server-linux`, `install-linux.sh` (repo-root, pre-existing) — all validated (`Assert-ContextDll`, `Assert-WindowsManifest`, `Assert-SameSha256`) **before** `Publish-CompleteRelease` atomically swaps them into `release-native/` |
| 5' | Linux/WSL build (Linux-host alternative) | `Invoke-LinuxCompleteReleaseBuild` `:1118-1134` → `native/run-release-memory-bounded.sh native/publish-feed.sh` | Same end result as stage 5 but run natively on Linux/WSL instead of dispatched from Windows |
| 6 | Staging → live promotion | `Publish-CompleteRelease` `:196-346` | Per-artifact: copy to a `.release-new.*` temp name, `Assert-SameSha256` against the source, back up any existing destination to `.release-backup.*`, move-in the new file. The feed directory is swapped as one unit (`Move-Item $feedCandidate -Destination $feed`), then `version.txt` is written **last** via `Publish-FileAtomic`. Any failure triggers ordered rollback (`$attemptRollback` closures) restoring every backup; only on full success are backups deleted |
| 7 | Commit | `Invoke-ReleasePublicationCommit` (`release-publication.ps1:657+`, not read in full — outside the read surface's line-limited scan but referenced from the wrapper) called at `publish-release-local.ps1:1188` | Creates the commit `Release Smart Explorer v$version [release candidate]` containing exactly the paths from `Get-PublicationReleaseCommitPaths` (`release-publication.ps1:594-628`, table below) |
| 8 | Revalidate candidate | `Assert-ReleasePublicationCandidate` `:1192` | Re-checks the committed bytes before any remote ref can see them |
| 9 | Push main | `Invoke-ReleasePublicationMainPush` `:1193` | Fast-forwards `origin/main` to the candidate commit |
| 10 | Tag / fallback push | `Invoke-ReleasePublicationTagPush` `:1194-1197` | Pushes immutable `refs/tags/v$version`; only if that push is technically rejected **and** the tag is still absent does it fall back to `refs/heads/release/v$version` (mutually exclusive with the tag path) |
| 11 | Trigger + monitor publication | `Get-GitHubActionsPublicationRun`/`Invoke-GitHubActionsPublicationDispatch`/`Wait-GitHubActionsPublicationWorkflow` `:617-727` (GitHub-Actions-hosted case) or `Wait-ReleasePublicationWorkflow` (human-operator case) | Under `GITHUB_ACTIONS=true`, since a workflow can't retrigger itself via the tag push using the job token, the wrapper explicitly dispatches `build.yml` with `publish_release=true` against the tag/fallback ref, captures the returned run ID, and polls that exact run (with an up-to-2-minute allowance for GitHub's binding metadata to settle, plus one automatic retry of a first-attempt failure) |
| 12 | Verify GitHub Release assets | `Wait-ReleasePublicationAssets` (`release-publication.ps1:1366-1457`) called at `publish-release-local.ps1:1243-1247` | Polls `/releases/tags/v$version` until exactly 18 assets exist, none unexpected, each name/size/`sha256:`-digest matches the locally computed expectation (`Get-PublicationExpectedReleaseAsset`) |
| 13 | Confirm tag didn't move | `:1248-1251` | `Get-RemoteTagCommit "v$version" == $candidateSha` |
| 14 | Local CLI update (Linux host only) | `Invoke-ReleasePublicationLinuxCliUpdate` `:1253-1259` | Skipped entirely by `-SkipLocalCliUpdate` (as CI passes) |

### Where the expected release asset list is defined and verified

- **Defined**: `native/release-publication.ps1:121-159`,
  `Get-PublicationReleaseAssetMap` — the canonical, single source-of-truth
  18-item `{LocalPath, PublishedName}` array, hard-asserted
  `if ($items.Count -ne 18) { throw ... }`.
- **Verified against the committed candidate**: same file,
  `Assert-ReleasePublicationCandidate` (`:392-534`, not fully quoted here —
  outside the specifically requested excerpt but its call sites at
  `publish-release-local.ps1:1138,1176,1185,1192,1200` confirm it is the
  gate run before commit, after commit, and before/instead-of a rebuild).
- **Verified against the live GitHub Release**:
  `Wait-ReleasePublicationAssets` (`release-publication.ps1:1366-1457`,
  quoted above) — this is the function that would need a 19th (APK) entry
  added to its `$expected` map, sourced from a corresponding addition to
  `Get-PublicationReleaseAssetMap`.
- **Re-verified independently in the workflow itself** (defense in depth,
  not reading from the same PowerShell function): `build.yml`'s
  `release-candidate` job hard-codes the same 18 filenames three times —
  the `test -s ...`/`cmp`/`sha256sum -c` block (`build.yml:410-467`), the
  `allowed_release_change` associative array bounding what the
  `[release candidate]` commit may touch (`build.yml:480-507`), and the
  `mkdir -p out; for asset in ...; do cp ...; done` staging loop with a
  hard `staged_count -ne 18` assertion (`build.yml:567-585`); then
  `windows-gnu-release-e2e`'s `$assetMappings` array
  (`build.yml:651-670`, `Count -ne 18` check `:676`) and
  `publish-release`'s `asset_mappings` bash array
  (`build.yml:743-763`, `Count -ne 18` check `:763`) repeat the same 18
  names a fourth and fifth time. **All five of these hard-coded lists
  (PowerShell map, three inline YAML lists, one more inline YAML list)
  would need the same new APK entry added in lockstep** — there is no
  single indirection point in the YAML; only the PowerShell side has one
  (`Get-PublicationReleaseAssetMap`), and even the YAML re-derives its own
  copies rather than calling into the PowerShell function (they run in
  different jobs/OSes).
- The GitHub Release body text (`build.yml:872-899`) listing each asset in
  prose would also need a bullet for the APK, though nothing enforces that
  automatically.

### Where the version is written

| File | Written by | Call site |
|---|---|---|
| `native/Cargo.toml` | `Set-NativeVersion` (regex replace `^version\s*=\s*"..."`) | `publish-release-local.ps1:741-790`, invoked at `:1168` |
| `native/Cargo.lock` (root `smart_explorer` package entry only) | Same function, same call | ditto |
| `release-native/update-feed/version.txt` | `Publish-FileAtomic` inside `Publish-CompleteRelease`, **last** step of the atomic swap | `publish-release-local.ps1:250` (Windows-host path); mirrored by `Publish-FeedDirectoryTransaction`'s `Publish-FileAtomic $VersionSource ...` in `publish-update.ps1:121-122` (partial/staged case) and by the `version_stage`/`mv ... "$feed/version.txt"` sequence in `publish-feed.sh:344-345,373-375` / `publish-linux-feed-wsl.sh:610-611,658-661` (Linux-host path) |
| Installer file name / embedded version string | `makensis /DVERSION=$version ...` | `publish-update.ps1:260`, mirrored in `publish-feed.sh:291-298` |
| `release-native/update-feed/windows-build.manifest` (`version=...` line) | Manifest array built inline | `publish-update.ps1:309,322-324`; `publish-feed.sh:237-243` |

---

## 4. Effect of a new top-level `android/` directory or new files

- **No workflow `paths:`/`paths-ignore:` filter exists anywhere in
  `.github/workflows/*.yml`** (verified by grep across all 11 files) —
  every `build.yml` push trigger and every task-suite `workflow_dispatch`
  fires regardless of which paths changed. Adding `android/` cannot
  accidentally suppress or accidentally trigger any existing workflow via
  path filtering, because none exists to affect.
- **Untracked-build-input check**
  (`Assert-PublicationNoUntrackedBuildInputs`,
  `release-publication.ps1:575-592`) only scans a fixed path allowlist for
  untracked files: `native`, `share-server`, `se-agent`,
  `install-linux.sh`, `.github/workflows/build.yml`, `.cargo`, `vendor`,
  `Cargo.toml`, `Cargo.lock`, `rust-toolchain(.toml)`. A **top-level**
  `android/` directory is outside that list, so leaving untracked files
  under `android/` would **not** be caught by this guard and would **not**
  block a release — this is a gap the Android integration should either
  accept consciously or close by adding `"android"` to that argument list
  if Android sources should be release-input-clean too.
- **Release-commit path allowlist**
  (`Get-PublicationReleaseCommitPaths`, `release-publication.ps1:594-628`,
  and the inline `allowed_release_change` copy at `build.yml:480-507`) is a
  fixed, explicit list of paths the `[release candidate]` commit is allowed
  to touch. Regular `android/` **source** changes must land in an ordinary
  (non-release) commit before the release wrapper runs, exactly like
  `native/src/**` changes today — the release wrapper never commits source,
  only build outputs. If an APK (or its hash sidecar) becomes a genuinely
  new *generated release artifact* living under, e.g.,
  `release-native/update-feed/smart_explorer.apk`, that path would need to
  be added to this allowlist (both the PowerShell list and the YAML copy)
  the same way each existing Windows/Linux payload path is listed.
- **Source-commit binding**
  (`Get-PublicationExpectedSourceCommit`, `release-publication.ps1:261-316`)
  walks the same allowlist to decide whether the release-candidate HEAD's
  single parent is "the" source commit; an unlisted changed path anywhere
  in that commit throws `"Release candidate commit contains non-release
  source changes"`. This reinforces the previous point: any new generated
  Android artifact path must be added to the same allowlist or the release
  wrapper's own commit will fail its own self-check.
- Conclusion: a plain `android/` app-source top-level directory is safe to
  add and will not perturb any existing trigger, gate, or consistency
  check, **as long as its build outputs are not committed as part of the
  `[release candidate]` commit** without also extending the four/five
  hard-coded path/asset lists identified in §3.

---

## 5. What adding an APK release asset would require; JDK/Android SDK availability

### Functions/lists to extend for a new "APK" release asset

| Location | Change needed |
|---|---|
| `native/release-publication.ps1:132-154` `Get-PublicationReleaseAssetMap` | Add one `[pscustomobject]@{ LocalPath = ...; PublishedName = "smart_explorer.apk" }` entry; bump the `Count -ne 18` guard at `:155-157` to 19 |
| `native/release-publication.ps1:594-627` `Get-PublicationReleaseCommitPaths` | Add the new committed artifact path (e.g. `release-native/update-feed/smart_explorer.apk` and its `.sha256`) if the APK is meant to be a committed, hash-verified release artifact like the other payloads |
| `.github/workflows/build.yml:410-467` (`test -s`/`cmp`/`sha256sum -c` block), `:480-507` (`allowed_release_change`), `:567-585` (staging loop + `staged_count -ne 18`) | Add the APK path in all three places, bump `18` → `19` |
| `.github/workflows/build.yml:651-670,676` (`windows-gnu-release-e2e` `$assetMappings`) | Add mapping entry, bump `18` → `19` |
| `.github/workflows/build.yml:743-763` (`publish-release` `asset_mappings`) | Add mapping entry, bump `18` → `19` |
| `.github/workflows/build.yml:900-918` (`files:` block for `softprops/action-gh-release@v2`) | Add `native/out/smart_explorer.apk` (and sidecar) line |
| `.github/workflows/build.yml:872-899` (release body prose) | Add a bullet describing the APK asset |
| A **new** payload-producing step | Neither `publish-update.ps1` (Windows-only) nor `publish-feed.sh`/`publish-linux-feed-wsl.sh` (Linux/WSL) currently build anything Android-related; a Gradle/NDK build step producing the signed APK would need to be added to one of these staging scripts (most naturally the Linux/WSL path, since Android's own toolchain is Linux-native) and its output copied into the same isolated stage directory (`$stageRelease`/`$feed_stage`) that the rest of stage 6 in §3 already treats atomically |
| `README.md:646-656` "Struktur" table and "Installieren"/"Release veröffentlichen" sections | Documentation-hygiene rule requires these to be updated in the same change per `AGENTS.md`'s "documentation hygiene" section |

### JDK / Android SDK / NDK availability on the existing runners

Explicitly checked in `complete-release`'s own preflight/tooling steps
(`build.yml:97-173`) and in every Linux/WSL release script
(`publish-feed.sh:43-49`, `publish-linux-feed-wsl.sh:91-125`):

- **Windows (`windows-2025`) host tools installed**: `strawberryperl`,
  `nsis` (via choco) — no JDK, no Android SDK/NDK, no Gradle.
- **WSL1 Ubuntu-24.04 tools installed**: `build-essential ca-certificates
  curl file git musl-tools p7zip-full pkg-config python3 xauth x11-utils
  xvfb libegl1 libgl1 libgl1-mesa-dri libx11-6 libx11-xcb1 libxcursor1
  libxi6 libxkbcommon0 libxkbcommon-x11-0 libxrandr2` plus `rustup` — no
  `openjdk`/`default-jdk`, no `android-sdk`/`cmdline-tools`, no `gradle`
  anywhere in this list, and the explicit tool-presence checks at
  `build.yml:159-169` (`bash, cargo, rustc, rustup, gcc, musl-gcc, file,
  git, xvfb-run, xauth, xwininfo`) do not include any Java/Android tool.
- **`native/publish-feed.sh:43-48`** required-tool loop: `cargo rustc
  rustup git curl pwsh x86_64-w64-mingw32-gcc x86_64-w64-mingw32-objdump
  makensis sha256sum file install 7z` — no Java/Android entries.
- **`native/publish-linux-feed-wsl.sh`** required/checked tools throughout
  (`:91-100`, `:207`, `:360-383`): `cargo`, `rustup`, `sha256sum`, `file`,
  `git`, plus the Zig/GCC/musl-gcc linker bootstrap and (in `--check-env`)
  `ldd readelf xvfb-run xauth xwininfo rustfmt` — again nothing
  Java/Android related.
- **Explicit grep across every file in the read surface** for
  `android|Android|JDK|java|Java|gradle|Gradle` returned **zero matches**.

**Conclusion**: per these scripts, neither the `windows-2025` runner nor
its WSL1 Ubuntu-24.04 environment currently provisions a JDK, the Android
SDK/cmdline-tools, an NDK, or Gradle. (This report does not guess about
what the base `windows-2025` GitHub-hosted runner image ships out of the box
beyond what these scripts explicitly install/require, per the task's
instruction; the runner-images catalog itself was outside the assigned
read surface.) Any Android build step — whether inside a new task suite or
inside the terminal release wrapper — must install its own JDK/Android
SDK/NDK/Gradle toolchain explicitly, the same way the existing WSL step
explicitly installs Rust via `rustup` rather than assuming it pre-exists.

---

## Decisions made (safe, local, non-binding)

- Treated `filter-transfer-task.yml` + `test-filter-transfer-task.sh` +
  `run-task-memory-bounded.sh` strictly as *pattern* references, as
  instructed, without proposing Android-specific test names or milestone
  content (out of this task's scope).
- Did not open `native/release-publication.ps1`'s
  `Assert-ReleasePublicationCandidate` (`:392-534`) or
  `Invoke-ReleasePublicationCommit`/`Invoke-ReleasePublicationTagPush`
  (`:657-1153`) bodies line-by-line, since they were not among the five
  explicit questions and the read surface listed `release-publication.ps1`
  without a line-range restriction but the assignment's question list only
  asked for the asset-map/version/publication-wait detail; their call
  sites and role are nonetheless documented above from the wrapper's call
  graph and surrounding context.
- Did not speculate about GitHub's `windows-2025` base image contents
  beyond what the scripts install/require, per explicit instruction.

## Unresolved / out-of-scope dependencies

- Whether the Android build should run on the Windows host (via a second
  WSL distro/toolchain step folded into `complete-release`) or as a
  separate Ubuntu-only job/workflow is an architecture decision for the
  main agent; this report only establishes that **no** existing job
  currently provisions Java/Android tooling either way.
- Whether the APK should be a *committed, hash-verified* release artifact
  (like the six existing Windows/Linux payloads, requiring the 18→19
  changes enumerated in §5) or an *uploaded-but-unverified* convenience
  asset is a product decision outside this report's scope; the mechanics
  above assume the former (matching this codebase's existing "every byte
  is statically verified before publication" pattern) but the main agent
  should confirm that's the desired guarantee level for a mobile artifact.
- `native/release-publication.ps1`'s `Assert-ReleasePublicationCandidate`
  (`:392-534`) internals were not read in full; before wiring in a 19th
  asset, the implementer should read that function completely to confirm
  it needs no changes beyond consuming the extended
  `Get-PublicationReleaseAssetMap`/`Get-PublicationReleaseCommitPaths`.
- Signing-key management for a release APK (keystore provisioning/secrets)
  was not investigated — out of the assigned read surface (no `.github`
  secrets/environments file was in scope) and is a prerequisite the main
  agent must resolve separately, analogous to how Windows/Linux artifacts
  currently ship unsigned per `docs/RELEASING.md`'s Bitdefender/trust
  section.
