# GitHub Actions Android CI building blocks (ubuntu-24.04)

Quelle:
[actions/checkout releases](https://github.com/actions/checkout/releases) ·
[actions/checkout README](https://github.com/actions/checkout) ·
[actions/setup-java README](https://github.com/actions/setup-java) ·
[gradle/actions setup-gradle docs](https://github.com/gradle/actions/blob/main/docs/setup-gradle.md) ·
[dtolnay/rust-toolchain action.yml](https://github.com/dtolnay/rust-toolchain/blob/master/action.yml) ·
[Swatinem/rust-cache README](https://github.com/Swatinem/rust-cache) ·
[bbqsrc/cargo-ndk README + CHANGELOG](https://github.com/bbqsrc/cargo-ndk) ·
[docs.rs/crate/cargo-ndk/4.1.2](https://docs.rs/crate/cargo-ndk/4.1.2) ·
[ReactiveCircus/android-emulator-runner README + action.yml](https://github.com/ReactiveCircus/android-emulator-runner) ·
[actions/upload-artifact README](https://github.com/actions/upload-artifact) ·
[developer.android.com — Test from the command line](https://developer.android.com/studio/test/command-line) ·
[developer.android.com — Manage all files on a storage device](https://developer.android.com/training/data-storage/manage-all-files) ·
[atmoz/sftp README](https://github.com/atmoz/sftp) ·
[delfer/docker-alpine-ftp-server README](https://github.com/delfer/docker-alpine-ftp-server) ·
[rclone serve webdav docs](https://rclone.org/commands/rclone_serve_webdav/) ·
[BytemarkHosting/docker-webdav README](https://github.com/BytemarkHosting/docker-webdav) (superseded, see §9) ·
Abgerufen: 2026-09-25.

Companion to `docs/refs/android-toolchain.md` §4/§5 (installed NDK/SDK/Gradle/Java versions on the
`ubuntu-24.04` runner image, `actions/setup-java`/`gradle/actions/setup-gradle` major-version
baseline, KVM-enable snippet) — **not duplicated here**, only referenced. This file adds the pieces
that file didn't cover: exact `actions/checkout`/`upload-artifact` versions, Rust-side CI actions
(`dtolnay/rust-toolchain`, `Swatinem/rust-cache`), the exact `cargo-ndk` CLI, full
`android-emulator-runner` input surface + AVD caching, `adb`/`am instrument` command syntax and test
report paths, and Dockerized SFTP/WebDAV/FTP test servers reachable from the emulator via `10.0.2.2`.

---

## 1. `actions/checkout`

- **Current major: `v7`** (`v7.0.1`, published 2026-07-20). Confirmed via
  `github.com/actions/checkout/releases` and the README's own current example.
- `fetch-depth` default: **`1`** (single commit; `0` = full history for all branches/tags).
```yaml
- uses: actions/checkout@v7
  with:
    fetch-depth: 1   # default; explicit here for clarity
```

## 2. `actions/setup-java`

- **Current major: `v6`.** (Consistent with `docs/refs/android-toolchain.md` §5, re-confirmed here.)
```yaml
- uses: actions/setup-java@v6
  with:
    distribution: temurin
    java-version: '17'
    cache: gradle        # optional: enables actions/setup-java's own dependency cache for gradle/maven/sbt
```
`cache` input enables built-in dependency caching (`gradle`/`maven`/`sbt`); `check-latest` (default
`false`) forces a remote-metadata check instead of trusting the runner's tool cache.

## 3. `gradle/actions/setup-gradle`

- **Current major: `v6`.**
```yaml
- uses: gradle/actions/setup-gradle@v6
  with:
    gradle-version: wrapper      # default: use the project's own gradle/wrapper/gradle-wrapper.properties version
    cache-read-only: false       # default; set true on non-default branches to avoid evicting the shared cache
    add-job-summary: always      # default; 'never' | 'on-failure' also valid
```
`gradle-version` also accepts an explicit version string (e.g. `'8.10'`) or the aliases `current`,
`release-candidate`, `nightly`, `release-nightly`.

## 4. `dtolnay/rust-toolchain`

Exact inputs (read from `action.yml`): `toolchain` (required), `targets` (optional, comma-separated
target triples; `target` is an alias), `components` (optional, comma-separated).
```yaml
- uses: dtolnay/rust-toolchain@stable
  with:
    targets: aarch64-linux-android,x86_64-linux-android
    components: rustfmt,clippy   # only if the job also needs fmt/clippy binaries present
```
Outputs: `cachekey` (short rustc-version hash, useful as a manual cache-key component) and `name`
(the resolved toolchain name, usable as `cargo +${{ steps.<id>.outputs.name }}`). Pin an exact
version instead of `@stable` with `dtolnay/rust-toolchain@1.85.0`-style tag if reproducibility across
runner image updates matters (`jni-0.22.4` itself requires `rust-version = "1.85.0"`, so the
toolchain used in CI must be `>= 1.85.0`).

## 5. `Swatinem/rust-cache`

- **Current major: `v2`.**
- Relevant inputs and defaults: `workspaces` (default `. -> target`), `shared-key` (default empty —
  falls back to an automatic job-based key), `key` (default empty, additional differentiator),
  `cache-on-failure` (default `"false"`), `cache-all-crates` (default `"false"`), `save-if` (default
  `"true"`).
- **Nested crate path** (this repo's Rust crate lives at `native/`, not the repo root): the
  `workspaces` input takes `$workspace -> $target` pairs, one per line, where `$target` is a
  directory *relative to* `$workspace` (defaults to `target` if omitted):
```yaml
- uses: Swatinem/rust-cache@v2
  with:
    workspaces: |
      native -> target
    shared-key: android-ndk-build   # stable across jobs/matrix legs that should share one cache
    cache-on-failure: true          # keep the cache even if this job's build/test step fails
```
Note (from the README, worth keeping in mind when sizing the cache key): the action deliberately
does **not** cache the workspace's own crate build artifacts (only dependency artifacts), since
caching first-party crate output is generally not effective/safe across commits.

## 6. `cargo-ndk` 4.1.2 — install and exact CLI

Install (pin to the version the task brief specifies, matching what
`docs/refs/android-toolchain.md` §7 already recorded as the current release):
```bash
cargo install cargo-ndk --locked --version 4.1.2
```

Core invocation shape (confirmed against the README's own canonical example):
```bash
cargo ndk -t arm64-v8a -t x86_64 -o app/src/main/jniLibs build --release
```
- `-t <target>` (repeatable) — Android ABI name (`arm64-v8a`, `x86_64`, `armeabi-v7a`, `x86`) **or**
  a Rust target triple; both forms are accepted.
- `-o <dir>` — output directory; `cargo-ndk` copies each built `.so` into `<dir>/<abi>/`, matching
  the layout `android { sourceSets { main { jniLibs.srcDirs = [...] } } }` / the default
  `app/src/main/jniLibs` convention expects.
- `--platform <api-level>` — sets the minimum Android API level the `.so` is compiled/linked
  against (should match or exceed the Gradle `minSdk`); also settable via the `CARGO_NDK_PLATFORM`
  env var, with CLI flags taking precedence.
- `--manifest-path <path>` — **ordinary cargo flag, forwarded through to the underlying `cargo`
  invocation.** As of **cargo-ndk 4.0.0** (per the CHANGELOG entry: *"CLI flags can now be used in
  any order (e.g., `cargo ndk -t x86 build` and `cargo ndk build --target x86` are equivalent)"*,
  plus a same-version fix for `--flag=value`-style flags being passed to cargo twice), flag order
  between the `cargo-ndk`-specific flags (`-t`/`-o`/`--platform`) and the `build`/other cargo
  subcommand is **no longer strict** — but the conventional/safest form (matching every current
  README example and what earlier cargo-ndk versions required) still places `-t`/`-o`/`--platform`
  **before** the subcommand and `--manifest-path`/`--release`/etc. **after** it:
```bash
cargo ndk -t arm64-v8a -t x86_64 --platform 26 -o app/src/main/jniLibs \
  build --release --manifest-path native/Cargo.toml
```
  **Use this ordering** (flags-before-subcommand, cargo-flags-after) even though 4.1.2 is tolerant of
  reordering — it's the form every example in the upstream docs uses and avoids relying on the more
  recent reordering tolerance in case of a partial regression.
- NDK discovery: `cargo-ndk` auto-detects an Android-Studio-default-location NDK install, picking
  the most recent version found; override explicitly with the `ANDROID_NDK_HOME` env var pointing at
  the NDK root — **do this explicitly in CI** (don't rely on auto-detection) since
  `docs/refs/android-toolchain.md` §4 already flags that the runner image's default/latest NDK
  mapping is scheduled to change (NDK 28 removed, NDK 30 added, effective **2026-10-01**).
```yaml
- name: Select pinned NDK
  run: |
    NDK_VERSION="27.3.13750724"   # pin explicitly; see android-toolchain.md §4 for the installed set
    if [ ! -d "$ANDROID_HOME/ndk/$NDK_VERSION" ]; then
      "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --install "ndk;$NDK_VERSION"
    fi
    echo "ANDROID_NDK_HOME=$ANDROID_HOME/ndk/$NDK_VERSION" >> "$GITHUB_ENV"
```
`sdkmanager` license acceptance (only needed if installing anything not already pre-accepted on the
runner image): `yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses`.

## 7. `reactivecircus/android-emulator-runner@v2` — full input surface (from README, confirmed)

| Input | Required | Default | Notes |
|---|---|---|---|
| `api-level` | yes | — | e.g. `23`, `33`, `35-ext15`, `Baklava`; minimum supported is `15`. |
| `system-image-api-level` | no | = `api-level` | Lets the AVD's platform and system-image API levels differ. |
| `target` | no | `default` | `default`, `google_apis`, `google_apis_ps16k`, `google_apis_playstore`, `google_apis_playstore_ps16k`, `android-wear[-cn]`, `android-tv`, `google-tv`, `aosp_atd`, `google_atd`, `android-automotive[-playstore]`, `android-desktop`. |
| `arch` | no | `x86` | `x86`, `x86_64`, or `arm64-v8a`. **On a GitHub-hosted `ubuntu-24.04` x86_64 runner, use `x86_64`** for KVM hardware acceleration to apply; `arm64-v8a` needs a matching-arch host and is not applicable here. `x86_64` images require API ≥ 21. |
| `profile` | no | — | AVD hardware profile id, e.g. `pixel_7_pro`. |
| `cores` | no | `2` | Emulator CPU core count. |
| `ram-size` / `heap-size` | no | — | e.g. `2048M` / `512M`. |
| `disk-size` | no | — | e.g. `2048M`. |
| `sdcard-path-or-size` | no | — | Path to an existing SD-card image, or a size to create one. |
| `avd-name` | no | `test` | |
| `force-avd-creation` | no | `true` | Overwrites an existing AVD of the same name. |
| `emulator-boot-timeout` | no | `600` (seconds) | Job fails if boot exceeds this. |
| `emulator-port` | no | `5554` | Exposed as `$EMULATOR_PORT`. |
| `emulator-options` | no | `-no-window -gpu swiftshader_indirect -no-snapshot -noaudio -no-boot-anim` | **Replaces**, not appends to, the defaults — include everything needed, e.g. `-no-window -gpu swiftshader_indirect -noaudio -no-boot-anim -camera-back none`. |
| `disable-animations` | no | `true` | |
| `disable-spellchecker` | no | `false` | |
| `disable-linux-hw-accel` | no | `auto` | `true`/`false`/`auto`. |
| `enable-hw-keyboard` | no | `false` | |
| `emulator-build` | no | — | Pin a specific emulator binary build number. |
| `working-directory` | no | `./` | e.g. `./android` if the Gradle root isn't the repo root. |
| `ndk` / `cmake` | no | — | Extra SDK component versions to install alongside the AVD setup. |
| `channel` | no | `stable` | SDK component download channel. |
| `script` | **yes** | — | The command(s) to run once the emulator has booted. |
| `pre-emulator-launch-script` | no | — | Runs after AVD creation, before emulator launch (e.g. tweak AVD config files). |

**Multi-line `script` execution — confirmed pitfall:** each line of a multi-line `script:` block is
run as its **own separate shell invocation** (`sh -c` per line), so state (shell variables, `cd`,
exported env) does **not** persist from one line to the next. For anything beyond a single command,
write a real script file checked into the repo and reference it as a single line instead:
```yaml
script: ./.github/scripts/run-instrumented-tests.sh
```
rather than:
```yaml
script: |
  APK=$(find . -name '*.apk')   # this $APK is NOT visible on the next line
  adb install -r -g "$APK"
```

### AVD caching (from README, `actions/cache`)
```yaml
- name: AVD cache
  uses: actions/cache@v5
  id: avd-cache
  with:
    path: |
      ~/.android/avd/*
      ~/.android/adb*
    key: avd-${{ matrix.api-level }}-${{ matrix.arch }}
- name: Create AVD and generate snapshot for caching
  if: steps.avd-cache.outputs.cache-hit != 'true'
  uses: reactivecircus/android-emulator-runner@v2
  with:
    api-level: ${{ matrix.api-level }}
    arch: x86_64
    force-avd-creation: false
    emulator-options: -no-window -gpu swiftshader_indirect -noaudio -no-boot-anim -camera-back none
    disable-animations: false
    script: echo "Generated AVD snapshot for caching."
```
(Standard two-step pattern: a cheap "warm the AVD + snapshot" run gated on a cache miss, then the
real test run reuses the cached `~/.android/avd`/`~/.android/adb*` on every subsequent job.)

### System-image target/arch availability — **flag as needing a live CI-time check**
The README documents the full *set* of `target` values (table above) but does **not** state per-API-
level/arch availability matrices. A secondary web search found a `google_apis_playstore` x86_64
system image package listed for API 36 (`x86_64-36_r07.zip`), and a separate, inconclusive report
(Google Issue Tracker #432143095, sign-in-walled, not independently readable in this pass) of
**missing system images for API 36 on at least one variant**. **Do not hard-code an assumption about
which `target`×`arch` combination exists for the exact API level chosen** — resolve it at CI time
instead:
```bash
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --list | grep "system-images;android-<API>;"
```
and pick a `target`/`arch` pair that's actually listed, falling back to a slightly older API level
(e.g. 34 or 35, both long-established) if the newest one lacks the desired image. `x86_64` is
required (not `arm64-v8a`) for KVM acceleration on the `ubuntu-24.04` x86_64 host.

## 8. `adb` / `am instrument` / Gradle-based instrumentation — exact commands

```bash
# Install, granting all requested runtime permissions at install time:
adb install -r -g app-debug.apk

# Grant MANAGE_EXTERNAL_STORAGE (must already be declared in the manifest; this is the
# adb-only equivalent of the user granting "All files access" in Settings):
adb shell appops set --uid <package.name> MANAGE_EXTERNAL_STORAGE allow

# Grant POST_NOTIFICATIONS (API 33+ runtime permission):
adb shell pm grant <package.name> android.permission.POST_NOTIFICATIONS

# Run instrumented tests directly via am instrument (raw output, wait for completion):
adb shell am instrument -w -r \
  -e class <package.name>.SomeTestClass \
  <package.name>.test/androidx.test.runner.AndroidJUnitRunner
# Multiple classes/methods: -e class Class1,Class2#testMethod (comma-separated, '#' selects one method)

# Screenshot straight to a local file (no on-device temp file needed):
adb exec-out screencap -p > screenshot.png

# Dump the full logcat buffer once (not a live stream) to a local file:
adb logcat -d > logcat.txt
```

**`gradle connectedDebugAndroidTest` vs. `installDebug` + `am instrument`:** the Gradle task
(`./gradlew connectedDebugAndroidTest`, or plain `connectedAndroidTest` for whichever build type is
default) builds, installs, runs, and collects results in one step, and additionally aggregates
multi-device output as a single HTML/XML report — **prefer this for CI** over the manual
`installDebug` + raw `am instrument` sequence, which is only needed for finer-grained control (e.g.
running one specific test class ad hoc without a full Gradle invocation, or capturing raw
instrumentation output for a custom parser).

**Report locations** (Gradle-driven run): XML results at
`app/build/outputs/androidTest-results/connected/`, HTML report at
`app/build/reports/androidTests/connected/index.html`.

```yaml
- name: Upload androidTest reports
  if: always()
  uses: actions/upload-artifact@v7
  with:
    name: android-test-reports
    path: |
      app/build/outputs/androidTest-results/connected/
      app/build/reports/androidTests/connected/
    if-no-files-found: ignore
```

## 9. `actions/upload-artifact`

- **Current major: `v7`** (`v7.0.1`; README's own examples use `actions/upload-artifact@v7`).
`if: always()` (capture reports/logs even when a prior step failed) or `if: failure()` (only on
failure, e.g. for a screenshot/logcat debug bundle) are the standard guards, as used above.

## 10. Test servers reachable from the emulator via `10.0.2.2`

The Android emulator's userspace network stack maps the special host alias `10.0.2.2` to the host
machine's loopback — so a Docker container published on the GitHub Actions runner's `localhost` at
port `N` is reachable from code running inside the emulator as `10.0.2.2:N`. All three images below
were checked for current availability/maintenance status on 2026-09-25.

### (a) SFTP — `atmoz/sftp` (actively published; most recent tag push observed "3 days ago" on Docker Hub)
```bash
docker run -d -p 2222:22 atmoz/sftp user:pass:::upload
```
Creates user `user`/password `pass`, home-directory-relative `upload/` subfolder created
automatically. User-spec syntax: `user:pass[:e][:uid[:gid[:dir1[,dir2]...]]]` (`:e` marks the
password field as already-encrypted; empty fields between colons are skipped positionally as shown).
From the emulator: `sftp://10.0.2.2:2222/upload/`, credentials `user`/`pass`.

### (b) WebDAV over plain HTTP with Basic auth — **`rclone/rclone` image running `rclone serve webdav`**
(`bytemark/webdav`, the other commonly-cited image, is **not** presented here: its Docker Hub tags
(`latest`, `2.4`) were both last pushed "almost 8 years ago" — effectively unmaintained/abandoned as
of this check. `rclone/rclone` is actively published, most recent tag observed "1 day ago".)
```bash
mkdir -p ./webdav-data
docker run -d -p 8080:8080 -v "$(pwd)/webdav-data:/data" \
  rclone/rclone:latest \
  serve webdav /data --addr :8080 --user testuser --pass testpass --no-check-certificate
```
- `rclone serve webdav <remote:path> [flags]` — `/data` here addresses the mounted local directory
  directly (no named remote needed for a plain local path).
- `--addr <ip:port|:port>` — bind address (default `127.0.0.1:8080`, not reachable from outside the
  container without `:8080` or `0.0.0.0:8080`, hence the explicit `--addr :8080` above).
- `--user`/`--pass` — enables HTTP Basic auth (plain HTTP is fine for a CI-only test fixture; rclone
  itself warns that Basic-over-plain-HTTP is appropriate only for non-production/test use, matching
  this use case exactly).
- From the emulator: `http://10.0.2.2:8080/`, Basic auth `testuser`/`testpass`.

### (c) FTP with passive mode reachable from the emulator — `delfer/alpine-ftp-server`
```bash
docker run -d \
  -p 21:21 -p 21000-21010:21000-21010 \
  -e USERS="testuser|testpass" \
  -e ADDRESS=10.0.2.2 \
  -e MIN_PORT=21000 -e MAX_PORT=21010 \
  delfer/alpine-ftp-server
```
- `USERS` format: space-separated list of `name|password|[folder]|[uid]|[gid]` entries (pipe-
  separated fields; only `name|password` is required).
- `ADDRESS` must be set to the address passive-mode clients (the emulator, connecting as
  `10.0.2.2`) will use to reach the server — **set it to `10.0.2.2` itself** here (not the runner's
  real IP), since that's the address the emulator's networking will actually use for the passive
  data connections back to the host.
- `MIN_PORT`/`MAX_PORT` (defaults `21000`/`21010`) must be published 1:1 (`-p
  21000-21010:21000-21010`) since passive FTP needs the container-internal and host-external port
  numbers to match — the server advertises the literal port number to the client inside the PASV
  reply.
- From the emulator: `ftp://10.0.2.2:21/`, credentials `testuser`/`testpass`.

---

## Unresolved / needs live-CI confirmation

- Exact `target`×`arch` system-image availability for the specific API level the project settles on
  (§7) — resolve via `sdkmanager --list` at pipeline run time rather than trusting this document's
  API-36 x86_64 spot-check, since Google's system-image publication for the newest API level is the
  fastest-moving fact in this whole file and one secondary source hinted at gaps for API 36.
- `android-emulator-runner`'s README does not explicitly document the multi-line `script:`
  per-line-shell behavior; it was confirmed only via a secondary community source (a GitHub issue
  discussion), not the action's own README/action.yml text — the mitigation (use a checked-in script
  file) is safe regardless of whether the exact mechanism reported is precisely right.
- `google_apis_playstore` images are Play-Store-signed and licensed for Google's own testing
  purposes; using `google_apis_playstore` vs. plain `google_apis` for a CI emulator running
  third-party instrumented tests was not license-checked in this pass — prefer plain `google_apis`
  (or `default`/`aosp_atd` for a lighter/faster AOSP-only image) unless Play Services APIs are
  actually exercised by the tests.
