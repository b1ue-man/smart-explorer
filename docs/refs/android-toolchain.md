# Android app toolchain reference (Sept 2026)

**Purpose.** Web-research snapshot of the current (checked 2026-09-25) Android build toolchain —
Android Gradle Plugin (AGP), Gradle, Kotlin, Jetpack Compose, NDK/cargo-ndk, and GitHub Actions CI
— for planning a Kotlin/Compose Android shell around the existing Smart Explorer Rust core. Facts
only; no architecture judgment. All version numbers were live/current on the check date and will
drift; re-verify before implementation.

**Files read.**
- `native/Cargo.toml` (Rust edition `2021`, crate `smart_explorer` — see companion doc `rust-jni.md` for how this maps to Android targets)
- Web sources cited inline below (developer.android.com, kotlinlang.org, docs.gradle.org, github.com, crates.io), checked 2026-09-25.

---

## 1. Core toolchain versions (checked 2026-09-25)

| Component | Latest stable | Notes / source |
|---|---|---|
| Android Gradle Plugin (AGP) | **9.4.0** (Sept 2026) | [AGP 9.4.0 release notes](https://developer.android.com/build/releases/agp-9-4-0-release-notes) |
| Gradle | **9.8.0** (Sept 24, 2026); 9.7.1 also current | [Gradle 9.8.0 release notes](https://docs.gradle.org/current/release-notes.html) — note: GH Actions ubuntu-24.04 image ships **Gradle 9.7.1** preinstalled (§4) |
| Kotlin | **2.4.20** (Sept 2026 patch of the 2.4 line); 2.5.0 planned Dec 2026 | [Kotlin 2.4.20 release blog](https://blog.jetbrains.com/kotlin/2026/09/kotlin-2-4-20-released/), [Kotlin releases](https://kotlinlang.org/docs/releases.html) |
| Compose BOM | **2026.09.00** | [androidx.compose compose-bom](https://mvnrepository.com/artifact/androidx.compose/compose-bom) — use `implementation(platform("androidx.compose:compose-bom:2026.09.00"))`; module versions below are then resolved by the BOM, pins are only needed for non-BOM-managed artifacts |
| androidx.activity:activity-compose | **1.13.0** (Mar 11, 2026) | [Activity release notes](https://developer.android.com/jetpack/androidx/releases/activity) |
| androidx.lifecycle:lifecycle-viewmodel-compose | **2.11.0** (Jun 17, 2026) | [Lifecycle release notes](https://developer.android.com/jetpack/androidx/releases/lifecycle) |
| androidx.navigation:navigation-compose | **2.10.2** (Sept 23, 2026) | [Navigation release notes](https://developer.android.com/jetpack/androidx/releases/navigation) |
| androidx.work:work-runtime-ktx | **2.12.0** (Sept 23, 2026) — artifact is now an empty shim; `CoroutineWorker` etc. live in `androidx.work:work-runtime` itself since 2.9.0 | [WorkManager release notes](https://developer.android.com/jetpack/androidx/releases/work) |

## 2. compileSdk / targetSdk / minSdk

| Question | Answer (checked 2026-09-25) |
|---|---|
| Current max API level | **Android 16 = API 36** stable; Android 17 = API 37 in beta/preview (mentioned as `android-37.2` beta platform on the CI image, §4). `compileSdk = 36` is the current stable ceiling. |
| Google Play — new apps/updates | Must target **API 36** (Android 16) starting **Aug 31, 2026**; extension to Nov 1, 2026 obtainable. [Play target API requirements](https://support.google.com/googleplay/android-developer/answer/11926878?hl=en) |
| Google Play — existing apps | Must target **≥ API 35** (Android 15) by Aug 31, 2026 (same extension applies). |
| Sideloading (no Play Store) | **No enforced targetSdk minimum** — Play Console's target-API gate only applies to apps *submitted to Google Play*; a sideloaded/direct-distributed APK is not blocked by this policy. (No separate OS-level minimum found in sources; the policy pages above are explicitly Play-Console-scoped.) |
| minSdk trade-off | StatCounter-based reach figures (as of Apr 2026, via a secondary aggregator, not re-verified against StatCounter directly): `minSdk 30` ≈ 86.9% device reach; `minSdk 33` ≈ 68.9%. Android 16 (API 36) alone was ~54.6% of active devices end of Aug 2026, API 37 ~3.7%. Common guidance is to target ~90% reach, which in Sept 2026 points to **minSdk ≈ 26–30** depending on desired reach/feature trade-off (API 26 = Android 8, needed for some `WorkManager`/notification-channel behavior; API 30 buys scoped-storage and package-visibility baseline). Source: [capgo.app Android distribution chart](https://capgo.app/android-distribution-chart/), [telemetrydeck Android market share](https://telemetrydeck.com/survey/android/Android/sdkVersions/) — secondary aggregators, not Google's own distribution dashboard (Google retired the public one); treat as directional only. |

## 3. 16 KB page-size requirement (native code impact)

| Fact | Detail |
|---|---|
| Google Play policy | New apps/builds targeting Android 15+: must support 16 KB pages **since Nov 1, 2025**; **all** app updates: since **May 1, 2026** (one source notes a Play-Console-granted extension to May 31, 2026 in some cases). [Android Developers Blog announcement](https://android-developers.googleblog.com/2025/05/prepare-play-apps-for-devices-with-16kb-page-size.html) |
| What satisfies it for a Rust `cdylib` | **NDK r28 or newer compiles with 16 KB ELF segment alignment by default** — no extra flags needed. |
| Older NDK (≤ r27) | Must pass linker flags explicitly: `-Wl,-z,max-page-size=16384 -Wl,-z,common-page-size=16384` (e.g. via `RUSTFLAGS` or `cargo ndk`'s `--` passthrough to the linker). Source: [Android 16 KB page size guide](https://developer.android.com/guide/practices/page-sizes), [AOSP 16kb page size docs](https://source.android.com/docs/core/architecture/16kb-page-size/16kb) |
| Recommended NDK version | **r28 or r29** for default 16 KB compliance without manual flags (r30 is the upcoming LTS, see §4). r27 is LTS but requires the manual linker flags above. |

## 4. GitHub Actions CI environment (`ubuntu-24.04` runner, checked 2026-09-25)

| Item | Value |
|---|---|
| Android SDK root | `/usr/local/lib/android/sdk` (both `ANDROID_HOME` and `ANDROID_SDK_ROOT` point here) |
| NDK installed (live, pre-Oct-2026) | **27.3.13750724** (default, `ANDROID_NDK_HOME`), **28.2.13676358**, **29.0.14206865** (`ANDROID_NDK_LATEST_HOME`) |
| **Upcoming change** | GH issue [actions/runner-images#14745](https://github.com/actions/runner-images/issues/14745) (opened Sept 17, 2026, effective **Oct 1, 2026**): NDK 28 is removed, **NDK 30 (new LTS)** is added. Post-change installed set = **27, 29, 30**; default (`ANDROID_NDK_HOME`) **stays 27**. `ANDROID_NDK_LATEST_HOME` presumably moves to 30 (not explicitly confirmed in the issue text fetched). **Action item:** pin the exact NDK build number the Rust build uses (via `sdkmanager --install "ndk;<version>"`) rather than relying on "latest", since the default/latest mapping changes under the pipeline. |
| Build-tools installed | 34.0.0, 35.0.0/35.0.1, 36.0.0/36.1.0, 37.0.0 |
| SDK platforms installed | android-34 through android-37.2 (beta) |
| Java/JDK installed | 8.0.504, 11.0.32, **17.0.20 (default)**, 21.0.12, 25.0.4 |
| Gradle preinstalled | **9.7.1** |
| Source | [Ubuntu 24.04 runner-images readme](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md) (raw fetch), [runner-images#14745](https://github.com/actions/runner-images/issues/14745) |

Installing a specific NDK version not preinstalled:
```bash
sdkmanager --install "ndk;28.2.13676358"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/28.2.13676358"
```

## 5. GitHub Actions building blocks

| Action | Current major | Notes |
|---|---|---|
| `actions/setup-java` | **v6** (v1–v4 deprecated) | GitHub-hosted runners pre-cache Eclipse Temurin; `setup-java` also checksum-verifies Temurin/Corretto/Zulu/etc. [actions/setup-java](https://github.com/actions/setup-java) |
| `gradle/actions/setup-gradle` | **v6** | Successor to the deprecated `gradle/gradle-build-action`. [gradle/actions setup-gradle docs](https://github.com/gradle/actions/blob/main/docs/setup-gradle.md) |
| `reactivecircus/android-emulator-runner` | **v2** (tag `@v2`) | Runs an AVD for instrumentation tests. [android-emulator-runner README](https://github.com/ReactiveCircus/android-emulator-runner/blob/main/README.md) |

KVM enablement snippet for the emulator runner job (Linux runner):
```yaml
- name: Enable KVM
  run: |
    echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"' | sudo tee /etc/udev/rules.d/99-kvm4all.rules
    sudo udevadm control --reload-rules
    sudo udevadm trigger --name-match=kvm
- uses: reactivecircus/android-emulator-runner@v2
  with:
    api-level: 34
    script: ./gradlew connectedCheck
```

**Alternative to a launched emulator: Gradle Managed Devices.** AGP's build-managed virtual devices
(configured in Gradle, API level 27+) create/run/tear down an AVD as part of the Gradle task graph
instead of a separate emulator-runner step; supports an Automated Test Device (ATD) profile with
pre-installed apps/background services stripped for lighter CI runs. Source:
[Scale your tests with build-managed devices](https://developer.android.com/studio/test/managed-devices).

## 6. Signing an APK in CI

| Concern | How |
|---|---|
| Signature schemes | v1 (JAR), v2 and v3 (APK Signature Scheme) — modern `apksigner`/AGP defaults enable v1+v2, v3 opt-in. Verify with `apksigner verify --verbose --print-certs app-release.apk`. [`apksigner` docs](https://developer.android.com/tools/apksigner) |
| Gradle `signingConfigs` | Defined in `app/build.gradle.kts`, typically reading a `keystore.properties` file (itself git-ignored) or environment variables populated from CI secrets — if the properties file/secrets are absent, the release `signingConfig` is simply not registered and the build falls back to unsigned/debug. |
| CI secret pattern | Base64-encode the `.jks`/`.keystore` file into a GitHub Actions secret, decode it to a file at build time, and pass store/key passwords + alias as additional secrets to `signingConfigs.create("release")`. |
| Debug-keystore fallback | For unsigned-internal builds, third-party tools like `uber-apk-signer` embed a debug keystore and can v1/v2/v3-sign without a release key; Gradle itself also auto-signs `assembleDebug` output with the auto-generated `~/.android/debug.keystore`. |

## 7. cargo-ndk (Rust → Android `.so` build)

| Item | Value |
|---|---|
| Latest release | **v4.1.2**, published **2025-08-09** (crates.io `newest_version` field, confirmed via `crates.io/api/v1/crates/cargo-ndk`) — no newer release exists as of the 2026-09-25 check, i.e. it has gone ~13 months without a new release. [cargo-ndk repo](https://github.com/bbqsrc/cargo-ndk), [crates.io cargo-ndk](https://crates.io/crates/cargo-ndk) |
| Install | `cargo install cargo-ndk` |
| Required `rustup` targets | `rustup target add aarch64-linux-android x86_64-linux-android` (add `armv7-linux-androideabi`, `i686-linux-android` only if 32-bit device support is wanted) |
| Build command (per the planned architecture: arm64 + x86_64 only) | `cargo ndk -t arm64-v8a -t x86_64 -o app/src/main/jniLibs build --release` |
| Platform (API level) flag | `cargo ndk --platform 26 -t arm64-v8a -t x86_64 -o app/src/main/jniLibs build --release` (`--platform` sets the minimum Android API the `.so` is compiled against; should match/exceed the Gradle `minSdk`) |
| NDK discovery | `cargo-ndk` locates the NDK via `ANDROID_NDK_HOME` / `ANDROID_NDK_ROOT` / `ANDROID_NDK_LATEST_HOME`, in that general order (exact precedence not independently re-verified beyond the README); on the GH Actions image, pin explicitly per §4 rather than trust "latest". |

## 8. Gradle Kotlin DSL — single-module Compose app skeleton

`settings.gradle.kts`:
```kotlin
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "SmartExplorer"
include(":app")
```

`gradle/libs.versions.toml` (excerpt):
```toml
[versions]
agp = "9.4.0"
kotlin = "2.4.20"
composeBom = "2026.09.00"

[libraries]
androidx-activity-compose = { group = "androidx.activity", name = "activity-compose", version = "1.13.0" }
androidx-navigation-compose = { group = "androidx.navigation", name = "navigation-compose", version = "2.10.2" }
androidx-work-runtime-ktx = { group = "androidx.work", name = "work-runtime-ktx", version = "2.12.0" }
compose-bom = { group = "androidx.compose", name = "compose-bom", version.ref = "composeBom" }

[plugins]
android-application = { id = "com.android.application", version.ref = "agp" }
kotlin-android = { id = "org.jetbrains.kotlin.android", version.ref = "kotlin" }
kotlin-compose = { id = "org.jetbrains.kotlin.plugin.compose", version.ref = "kotlin" }
```
(`org.jetbrains.kotlin.plugin.compose` is required in **addition** to `kotlin-android` since Kotlin
2.0 moved the Compose compiler out of AGP into a Kotlin-repo-hosted Gradle plugin whose version
must track the Kotlin version exactly — see [Compose Compiler Gradle plugin docs](https://developer.android.com/develop/ui/compose/compiler).)

`app/build.gradle.kts` (excerpt):
```kotlin
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "com.smartexplorer.android"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.smartexplorer.android"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildFeatures { compose = true }

    packaging {
        jniLibs {
            useLegacyPackaging = false // uncompressed .so, faster load, larger APK-on-disk footprint
        }
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.work.runtime.ktx)
    implementation("androidx.compose.material3:material3")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose")
}
```
Notes: `useLegacyPackaging` only affects APK-producing (`com.android.application`) modules, not
library modules ([AGP native-packaging docs](https://developer.android.com/build/releases/agp-8-1-0-release-notes)
context found while researching). `ndk.abiFilters` restricts which prebuilt `.so` ABIs Gradle
packages — must list the same ABIs `cargo ndk -t ...` produced (§7). A `splits { abi { ... } }`
block is the alternative when per-ABI APKs (instead of one universal APK) are wanted; not needed
for a single fat APK/AAB distribution.

## 9. Compose UI / instrumentation tests — minimal syntax

Dependencies:
```kotlin
androidTestImplementation("androidx.compose.ui:ui-test-junit4")
debugImplementation("androidx.compose.ui:ui-test-manifest")
androidTestImplementation("androidx.test.ext:junit:1.x.x") // androidx.test.ext.junit
```

Test class:
```kotlin
@RunWith(AndroidJUnit4::class)
class MainActivityUiTest {
    @get:Rule
    val composeTestRule = createAndroidComposeRule<MainActivity>()

    @Test
    fun showsFileList() {
        composeTestRule.onNodeWithText("Documents").assertExists()
    }
}
```
`createComposeRule()` (no activity) is used for isolated composable tests; `createAndroidComposeRule<Activity>()`
launches a real Activity when the test needs it. Source: [Test your Compose layout](https://developer.android.com/develop/ui/compose/testing),
[Testing in Jetpack Compose codelab](https://developer.android.com/codelabs/jetpack-compose-testing).

---

### Open items for the main agent
- `ANDROID_NDK_LATEST_HOME`'s exact post-Oct-2026 value (NDK 30 build number) was not directly
  confirmed in the fetched GH issue text — re-check `runner-images` readme once the Oct 1, 2026
  rollout has landed, or pin the NDK version explicitly and sidestep the question.
- minSdk reach percentages come from secondary aggregators (capgo.app, telemetrydeck), not
  Google's own dashboard (which appears to no longer be public) — treat as directional.
- The 16 KB Play policy "May 1 vs May 31, 2026" extension detail is inconsistently stated across
  secondary sources; worth a primary-source (Play Console) re-check at implementation time.
