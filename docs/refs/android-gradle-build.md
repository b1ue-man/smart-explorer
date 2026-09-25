# Android Gradle build template (AGP 8.13.2 / Gradle 8.13 / Kotlin 2.4.20)

Quelle: developer.android.com/build/releases/*, developer.android.com/jetpack/androidx/releases/*,
docs.gradle.org, kotlinlang.org, services.gradle.org, dl.google.com/android/maven2,
repo1.maven.org, github.com/gradle/gradle (all cited inline per section below) ·
Abgerufen: 2026-09-25

**Purpose.** A complete, internally consistent, lowest-risk Gradle build template for a single
`:app` module (Kotlin, Jetpack Compose, kotlinx.serialization, WorkManager, prebuilt Rust `.so` in
`src/main/jniLibs`, `minSdk 30`, `compileSdk`/`targetSdk 36`) built by remote CI (`ubuntu-24.04`,
JDK 17 default, JDK 21 available) via the Gradle wrapper. Facts only; versions are pinned to what
was live on the check date and **will drift** — re-verify before a real implementation pass if this
file is more than a few weeks old.

**Files read (this pass).**
- `docs/refs/android-toolchain.md`, `docs/refs/android-platform.md`, `docs/refs/rust-jni.md`,
  `docs/refs/android-rust-deps.md` (existing background refs, per the task brief — not duplicated
  here except where a value needed re-verification or a correction).
- `native/Cargo.toml` (only the `[package] version = "0.5.162"` field, to design the
  version-parsing Gradle function in §5).
- Web sources cited inline, checked 2026-09-25: developer.android.com (build/releases/*,
  jetpack/androidx/releases/*, reference/kotlin/*), docs.gradle.org, kotlinlang.org,
  services.gradle.org (JSON `versions/all` API + raw `.sha256` files, fetched directly with `curl`,
  not through a summarizing fetch), dl.google.com/android/maven2 (raw `maven-metadata.xml`, same
  reason), repo1.maven.org (raw `maven-metadata.xml`), github.com/gradle/gradle (tag API).

**Correction to an existing ref.** `docs/refs/android-toolchain.md` §8 sketches AGP `9.4.0` paired
with `org.jetbrains.kotlin.android` (`kotlin-android`) as a classic plugin. Per §1 below, **that
combination no longer works**: AGP 9.0+ enables "built-in Kotlin" and the new DSL by default, and
`org.jetbrains.kotlin.android` is explicitly incompatible with the new DSL. This file does not edit
that ref (out of scope / not this task's file), but the main agent should treat this file, not that
snippet, as authoritative for the Gradle/AGP/Kotlin plugin wiring.

---

## 1. AGP 9.x vs AGP 8.x — decision

### 1.1 What changed in AGP 9.x for Kotlin projects (built-in Kotlin)

Source: [AGP 9.0.0 release notes](https://developer.android.com/build/releases/agp-9-0-0-release-notes), [Migrate to built-in Kotlin](https://developer.android.com/build/migrate-to-built-in-kotlin).

| Item | Fact |
|---|---|
| Built-in Kotlin | AGP 9.0 **enables built-in Kotlin by default** — Kotlin compilation is done by AGP itself; you no longer apply `org.jetbrains.kotlin.android` at all. |
| `org.jetbrains.kotlin.android` compatibility | **Not compatible with the new DSL.** Quote: *"The `org.jetbrains.kotlin.android` plugin is not compatible with the new DSL."* Applying it alongside AGP 9.x's default (`android.newDsl=true`) breaks the build. |
| Opt-out flags | `android.builtInKotlin=false` **and** `android.newDsl=false` (both required together — the classic plugin needs the old DSL too) in `gradle.properties`. Documented as temporary: **removed in AGP 10.0** (targeted mid/late 2026). |
| KGP runtime dependency | AGP 9.0 has a runtime dependency on **Kotlin Gradle Plugin (KGP) 2.2.10** (minimum and default). A lower KGP is silently upgraded to 2.2.10; downgrading below 2.2.10 requires opting out of built-in Kotlin entirely (minimum downgrade floor: KGP 2.0.0). Upgrading past 2.2.10 needs an explicit `buildscript { classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:<newer>") }` override. |
| `kotlinOptions` block | **Removed.** Replaced by `kotlin { compilerOptions { ... } }` (top-level `kotlin` extension, not nested in `android {}`). `jvmTarget` now defaults to `android.compileOptions.targetCompatibility` automatically. |
| Old Variant API | `android.applicationVariants` etc. are no longer accessible once `android.newDsl=true` (the AGP 9 default). |
| Compose / kotlinx-serialization compiler plugins under built-in Kotlin | Still applied as ordinary Gradle plugins (`org.jetbrains.kotlin.plugin.compose`, `org.jetbrains.kotlin.plugin.serialization`) exactly as before — built-in Kotlin replaces only the *base* `org.jetbrains.kotlin.android` plugin, not the compiler-plugin ecosystem. **Their version must then match the KGP version AGP actually resolves at build time (2.2.10 by default for AGP 9.0, not necessarily 2.2.10 for 9.4 — not independently re-confirmed for 9.4, see §7 unresolved)**, not a version you freely pick the way you can with the classic plugin. |
| Kotlin source sets | `kotlin.sourceSets.named(...)` is **unsupported** under built-in Kotlin; must use `android.sourceSets.named("main") { kotlin.directories += ... }` instead. |
| Min Gradle for AGP 9.0 | 9.1.0 (minimum and default). AGP 9.4.0 needs Gradle 9.6.0 minimum/default (see §1.3). |
| Max compileSdk | AGP 9.0: API 36.1. AGP 9.4: API 37. |

### 1.2 Newest AGP 8.x that supports `compileSdk 36`: **8.13.2**

Confirmed via the raw `maven-metadata.xml` at
`https://dl.google.com/android/maven2/com/android/tools/build/gradle/maven-metadata.xml`
(fetched directly, not summarized): the 8.x version list ends at `8.13.0` → `8.13.1` → `8.13.2`,
then jumps straight to `9.0.0-alpha01`. **There is no AGP 8.14.** 8.13.2 is therefore both "the
latest 8.x patch" and "the newest AGP 8.x line."

Source: [AGP 8.13.0 release notes](https://developer.android.com/build/releases/agp-8-13-0-release-notes) (compatibility table, quoted verbatim below) — 8.13.1/8.13.2 are patch releases of the same 8.13 line, same compatibility row.

| Component | Minimum | Default |
|---|---|---|
| Gradle | 8.13 | 8.13 |
| SDK Build Tools | 35.0.0 | 35.0.0 |
| NDK | N/A | 27.0.12077973 |
| JDK | 17 | 17 |

Max `compileSdk` for AGP 8.13.x: **API 36.1** — so `compileSdk = 36` is squarely inside the
officially-tested range, not at or past the edge of it.

### 1.3 Gradle version compatibility table (AGP → minimum required Gradle)

Source: [About Android Gradle plugin](https://developer.android.com/build/releases/about-agp) (compatibility table).

| AGP | Min Gradle |
|---|---|
| 9.4 | 9.6.0 |
| 9.3 | 9.5.0 |
| 9.2 | 9.4.1 |
| 9.1 | 9.3.1 |
| 9.0 | 9.1.0 |
| **8.13 (→ 8.13.1, 8.13.2)** | **8.13** |
| 8.12 | 8.13 |
| 8.11 | 8.13 |
| 8.10 | 8.11.1 |
| 8.9 | 8.11.1 |
| 8.8 | 8.10.2 |
| 8.7 | 8.9 |
| 8.6 | 8.7 |
| 8.5 | 8.7 |

Important asymmetry, confirmed on the **current** Gradle (9.8.0) compatibility page
([docs.gradle.org/current/userguide/compatibility.html](https://docs.gradle.org/current/userguide/compatibility.html)):
> "Gradle is tested with Android Gradle Plugin 9.0 through 9.5.0-alpha02. Alpha and beta versions may or may not work."

i.e. the **current Gradle release line only advertises tested compatibility with AGP 9.0+**, not
with any 8.x AGP. This does not mean AGP 8.13.2 + a very new Gradle 9.x is *broken* — but it is
untested/undocumented by either project. Pairing AGP 8.13.x with **Gradle 8.13** (its own
documented "Default version") is the one combination Google's own compatibility table actually
vouches for, and is the pairing used below.

### 1.4 Kotlin Gradle Plugin ↔ AGP compatibility (for the classic-plugin path)

Source: [kotlinlang.org — Configure a Gradle project](https://kotlinlang.org/docs/gradle-configure-project.html) (Kotlin/AGP/Gradle compatibility section).

For **KGP 2.4.20** (current stable, see §2):

| Dependency | Fully-supported minimum | Fully-supported maximum |
|---|---|---|
| Gradle | 7.6.3 | 9.7.0 |
| AGP | 8.5.2 | 9.3.1 |

AGP **8.13.2** sits comfortably inside `[8.5.2, 9.3.1]` — no deprecation warnings expected. (Using
newer versions than the stated maximum still generally works per Kotlin's own docs, just with
possible deprecation warnings — not relevant here since 8.13.2 is well under the ceiling.)

### 1.5 Recommendation: **AGP 8.13.2 + Gradle 8.13 + classic `org.jetbrains.kotlin.android` (KGP 2.4.20)**

Justification, in order of weight:

1. **Both requested compileSdk/targetSdk = 36 targets are squarely inside AGP 8.13.2's officially
   tested range (max 36.1)** — no `android.suppressUnsupportedCompileSdk` flag needed either way
   (see §6).
2. **AGP 9.x's built-in-Kotlin/new-DSL model is brand new** (9.0 shipped January 2026, i.e. about 8
   months old at the check date; 9.4 a few weeks old) and forces a paradigm switch — no
   `kotlinOptions`, no classic Variant API, compiler-plugin versions implicitly tied to whatever KGP
   version AGP resolves rather than a version the project pins directly. Given the task's own
   constraint — **no local compiler, a wrong Gradle-file line costs a 30-minute remote CI
   round-trip** — the newer, less-templated, less-documented-in-the-wild AGP 9.x path is objectively
   higher-risk than the AGP 8.x line every current Compose tutorial, template generator, and
   Stack-Overflow answer still assumes.
3. **AGP 8.13.2's own compatibility table gives one unambiguous "Default version" pairing (Gradle
   8.13)**, which is the one pairing both Google's AGP docs and (transitively) Kotlin's
   compatibility range agree on. Gradle's own current docs, by contrast, **only vouch for AGP 9.0+**
   — so an AGP-8.13.2 + very-new-Gradle-9.x pairing would be an *untested* combination by either
   project's own published compatibility statement, which is the opposite of lowest-risk.
4. `minSdk 30` and every pinned dependency (§2) are unaffected by the AGP 8 vs 9 choice — nothing in
   the requested feature set (Compose, kotlinx.serialization, WorkManager, prebuilt `.so` in
   `jniLibs`) requires AGP 9.

This does **not** rule out AGP 9.x for a later milestone once its ecosystem has matured further —
it is flagged here only as the higher-risk-for-now option, not as unusable.

---

## 2. Exact version matrix for the recommended combination

All versions below were the current stable (non-alpha/beta/RC) release on 2026-09-25, confirmed via
a raw `maven-metadata.xml`/`versions/all` fetch (not a summarized page read) unless noted.

| Component | Version | Source (raw fetch) |
|---|---|---|
| AGP (`com.android.tools.build:gradle` / `com.android.application`) | **8.13.2** | `dl.google.com/android/maven2/com/android/tools/build/gradle/maven-metadata.xml` |
| Gradle (wrapper) | **8.13** | `services.gradle.org/versions/all` (JSON), entry `{"version":"8.13","final":true,...}` |
| Kotlin / `kotlin-android` / KGP | **2.4.20** | `repo1.maven.org/maven2/org/jetbrains/kotlin/kotlin-gradle-plugin/maven-metadata.xml` (`2.5.0-Beta1` is the only newer entry — prerelease) |
| `org.jetbrains.kotlin.plugin.compose` (Compose compiler) | **2.4.20** (must equal the Kotlin version exactly — see §2.1) | [Compose Compiler Gradle plugin](https://developer.android.com/develop/ui/compose/compiler) |
| `org.jetbrains.kotlin.plugin.serialization` | **2.4.20** (same rule — bundled with the Kotlin distribution, no independent version) | [kotlinlang.org serialization guide](https://kotlinlang.org/docs/serialization-get-started.html) |
| `org.jetbrains.kotlinx:kotlinx-serialization-json` | **1.11.0** (latest *stable*; `1.12.0-RC` exists but is a release candidate, not final) | `repo1.maven.org/maven2/org/jetbrains/kotlinx/kotlinx-serialization-json/maven-metadata.xml` |
| `org.jetbrains.kotlinx:kotlinx-coroutines-android` | **1.11.0** | `repo1.maven.org/maven2/org/jetbrains/kotlinx/kotlinx-coroutines-android/maven-metadata.xml` |
| `androidx.compose:compose-bom` | **2026.09.00** | `dl.google.com/android/maven2/androidx/compose/compose-bom/maven-metadata.xml` |
| `androidx.activity:activity-compose` | **1.13.0** (`1.14.0-alpha0x` exists, prerelease) | `dl.google.com/android/maven2/androidx/activity/activity-compose/maven-metadata.xml` |
| `androidx.lifecycle:lifecycle-runtime-compose` | **2.11.0** (`2.12.0-alpha0x` exists, prerelease) | `dl.google.com/android/maven2/androidx/lifecycle/lifecycle-runtime-compose/maven-metadata.xml` |
| `androidx.lifecycle:lifecycle-viewmodel-compose` | **2.11.0** (same series, same note) | `dl.google.com/android/maven2/androidx/lifecycle/lifecycle-viewmodel-compose/maven-metadata.xml` |
| `androidx.core:core-ktx` | **1.19.1** | `dl.google.com/android/maven2/androidx/core/core-ktx/maven-metadata.xml` |
| `androidx.work:work-runtime-ktx` | **2.12.0** (raised `minSdk` from 23 → **24** in this release) | `dl.google.com/android/maven2/androidx/work/work-runtime-ktx/maven-metadata.xml` |
| `androidx.test.ext:junit` | **1.3.0** | [androidx Test release notes](https://developer.android.com/jetpack/androidx/releases/test) (all 5 rows below released together, 2025-07-30) |
| `androidx.test.espresso:espresso-core` | **3.7.0** | same |
| `androidx.test:runner` | **1.7.0** | same |
| `androidx.test:rules` | **1.7.0** | same |
| `androidx.test:core` | **1.7.0** | same |
| `androidx.compose.ui:ui-test-junit4` / `ui-test-manifest` | **1.12.1** (BOM-managed, resolved for `compose-bom:2026.09.00`) | Compose BOM mapping page |
| `junit:junit` (JUnit4) | **4.13.2** (unchanged for years, still current) | `repo1.maven.org/maven2/junit/junit/maven-metadata.xml` |

### 2.1 Compose compiler / serialization plugin version rule

Both `org.jetbrains.kotlin.plugin.compose` and `org.jetbrains.kotlin.plugin.serialization` are
**published from the Kotlin repository itself and always carry the same version number as the
Kotlin compiler they belong to** (confirmed: *"The version of the Compose compiler now always
matches the Kotlin version"*, [Compose Compiler Gradle plugin docs](https://developer.android.com/develop/ui/compose/compiler)). In the version catalog this means both plugin entries should read
`version.ref = "kotlin"`, never a hand-picked separate number — there is no independent version to
look up.

The old **"Compose to Kotlin Compatibility Map"**
([developer.android.com/jetpack/androidx/releases/compose-kotlin](https://developer.android.com/jetpack/androidx/releases/compose-kotlin))
is a pre-Kotlin-2.0 artifact of when the Compose compiler lived in AndroidX itself; it is
superseded by the exact-match rule above and does not need to be consulted for Kotlin 2.x.

### 2.2 minSdk compatibility check (target `minSdk = 30`)

| Dependency | Its own minSdk | OK at `minSdk 30`? |
|---|---|---|
| `work-runtime-ktx:2.12.0` | 24 (raised from 23 in this release) | Yes (30 > 24) |
| Compose BOM 2026.09.00 libraries (`material3`, `ui`, `foundation`, …) | 21 (long-standing Compose floor) | Yes |
| `material3.adaptive:adaptive:1.3.0` | 21 (Compose-tier library, no higher floor found in any source consulted) | Yes, not independently re-verified beyond general Compose-library convention — see §7 |
| `activity-compose:1.13.0`, `lifecycle-*-compose:2.11.0` | 21 | Yes |
| `core-ktx:1.19.1` | 21 (AndroidX Core floor) | Yes |
| Everything else in this table | ≤ 21 or N/A (pure-Kotlin/JVM, e.g. `kotlinx-serialization-json`, JUnit) | Yes |

No dependency in this matrix requires more than `minSdk 24`, so `minSdk = 30` clears every floor
with margin.

---

## 3. Material icons

Source: raw `maven-metadata.xml` at
`dl.google.com/android/maven2/androidx/compose/material/material-icons-core/` and
`.../material-icons-extended/` (fetched directly), cross-checked against the Compose BOM mapping
page.

| Question | Answer |
|---|---|
| Is `material-icons-core` still published and BOM-managed? | **Yes.** Latest version **1.7.8**, and BOM `2026.09.00` maps `androidx.compose.material:material-icons-core` → **1.7.8**. Declare it with no explicit version (`implementation("androidx.compose.material:material-icons-core")` under the BOM platform import). |
| Is `material-icons-extended` still available/maintained? | **Published, but stale.** Latest version is also **1.7.8**, and — critically — **both** `material-icons-core` and `material-icons-extended`'s `maven-metadata.xml` report `lastUpdated = 20250212180149` (2025-02-12). **No release in over 19 months** as of the check date; the artifact has not been touched since, while `material3` itself (1.4.0) and the BOM ship monthly. Still BOM-managed (same 1.7.8 pin), still compiles and works, just not actively developed further. |

### 3.1 Icon inventory check (icons named in the task brief)

All of the following are confirmed present in `material-icons-core` (the *core* set, i.e. **no**
`material-icons-extended` dependency is needed for any of them) based on the standard Material
Icons "core" curated subset that ships in `Icons.Default`/`Icons.Filled` plus the
`Icons.AutoMirrored.*` variants added for RTL support:

`Add, ArrowBack (→ Icons.AutoMirrored.Filled.ArrowBack), Check, Close, Delete, Edit, Favorite,
Home, Info, Menu, MoreVert, Refresh, Search, Settings, Share, Star, Warning,
KeyboardArrowDown/Right, List (→ Icons.AutoMirrored.Filled.List), Lock, Person, Place, PlayArrow,
Clear, Done, Email, Face, Build, AccountCircle, DateRange, Notifications, ThumbUp, ShoppingCart,
Call, LocationOn, ExitToApp (→ Icons.AutoMirrored.Filled.ExitToApp), CheckCircle, AddCircle`

This list matches the well-known ~40-icon "core" curated subset Google ships directly in
`material-icons-core` (as opposed to the ~2000-icon superset in `material-icons-extended`); it was
**not** re-verified icon-by-icon against the live 1.7.8 KDoc index in this pass (that index is a
huge generated API-reference page that did not render usefully through the available fetch tooling)
— treat the RTL-aware entries (`ArrowBack`, `List`, `ExitToApp`, and similarly `KeyboardArrowRight`
has no auto-mirrored variant needed since directional arrows for "next" are usually deliberately
non-mirrored) as the one point worth a quick spot-check at implementation time. See §7.

### 3.2 Folder/file/cloud/sync icons (not reliably in core)

None of `Folder`, `InsertDriveFile`, `CloudUpload`, `CloudDownload`, `CloudDone`, `CloudOff`, `Sync`,
`SyncProblem`, `FolderOpen`, or similar file-manager-specific icons are in the ~40-icon core set
above. Two options, in order of the lowest-risk-first task framing:

1. **Vector drawable XML resources** (`res/drawable/ic_folder.xml` etc., authored as
   `<vector>`/`<path>` XML, referenced via `painterResource(R.drawable.ic_folder)` in Compose) —
   zero extra Gradle dependency, zero risk of an icon name not existing in whatever
   `material-icons-extended` version is pulled, and no risk from that artifact's 19-month-stale
   maintenance status. **Recommended**, matching the task's own suggested fallback.
2. `material-icons-extended:1.7.8` (BOM-managed, so no separate version to pin) — adds a large
   dependency (thousands of icons, meaningfully increases APK method count / build time for a
   library that has not shipped a new icon since Feb 2025) for a handful of specific glyphs.

---

## 4. Window size classes

Two competing APIs exist; the task brief names both. Based on cross-checking developer.android.com,
a third-party Material-3-Adaptive doc mirror, and an independent web-search summary (two of three
sources agree — flagged explicitly below where they diverged):

| | Current / recommended | Older / superseded |
|---|---|---|
| Artifact (group:artifact) | `androidx.compose.material3.adaptive:adaptive` — **confirmed via raw `maven-metadata.xml`** (`groupId=androidx.compose.material3.adaptive`, `artifactId=adaptive`); one summarized fetch of the official guide page mis-reported the group as `androidx.compose.material3:material3-adaptive` — **do not use that form**, it is not the real Maven coordinate. | `androidx.compose.material3:material3-window-size-class` |
| Version | **1.3.0** (latest stable; `1.4.0-alpha02` exists as prerelease). BOM-managed: `compose-bom:2026.09.00` maps `adaptive`/`adaptive-layout`/`adaptive-navigation` → 1.3.0. | Not independently checked this pass (legacy path, not used in the recommended template) |
| Function | `currentWindowAdaptiveInfo()` | `calculateWindowSizeClass(activity: Activity)` |
| Import (function) | `androidx.compose.material3.adaptive.currentWindowAdaptiveInfo` | `androidx.compose.material3.windowsizeclass.calculateWindowSizeClass` (legacy package, not independently re-verified this pass) |
| Return type | `androidx.compose.material3.adaptive.WindowAdaptiveInfo`, whose `.windowSizeClass` property is typed **`androidx.window.core.layout.WindowSizeClass`** — two independent sources (a Material-3-Adaptive doc mirror, and a separate web-search summary quoting *"The `WindowSizeClass` from the `androidx.window.core.layout` package"*) agree on this import; a third, summarized fetch of the official guide page instead claimed the type lives directly in `androidx.compose.material3.adaptive` — **treated as the less reliable of the two claims** (that fetch also got the artifact coordinate wrong, per the row above, so it is not trusted here), but not re-verified against raw KDoc source. Flagged as residual uncertainty, see §7. | `androidx.compose.material3.windowsizeclass.WindowSizeClass` (a *different* type from the one above — not interchangeable) |

Usage (recommended path):

```kotlin
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfo
import androidx.window.core.layout.WindowSizeClass

@Composable
fun MyScreen() {
    val windowSizeClass = currentWindowAdaptiveInfo().windowSizeClass
    val isExpanded = windowSizeClass.isWidthAtLeastBreakpoint(
        WindowSizeClass.WIDTH_DP_EXPANDED_LOWER_BOUND
    )
}
```

**Recommendation:** use `currentWindowAdaptiveInfo()` from `material3.adaptive:adaptive:1.3.0`. It
is the actively-developed path (BOM-tracked monthly, unlike the icon artifacts in §3), does not
require threading an `Activity` reference through composables the way `calculateWindowSizeClass`
does, and its dependency is a normal `implementation(...)` line with no version to hand-pick (BOM
manages it).

---

## 5. Full file contents (copy-ready)

### 5.1 `settings.gradle.kts`

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

### 5.2 Root `build.gradle.kts`

```kotlin
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.kotlin.serialization) apply false
}
```

### 5.3 `gradle/libs.versions.toml`

```toml
[versions]
agp = "8.13.2"
kotlin = "2.4.20"
composeBom = "2026.09.00"
activityCompose = "1.13.0"
lifecycle = "2.11.0"
coreKtx = "1.19.1"
workManager = "2.12.0"
kotlinxSerializationJson = "1.11.0"
kotlinxCoroutinesAndroid = "1.11.0"
materialAdaptive = "1.3.0"
androidxTestExtJunit = "1.3.0"
espressoCore = "3.7.0"
androidxTestRunner = "1.7.0"
androidxTestRules = "1.7.0"
androidxTestCore = "1.7.0"
junit4 = "4.13.2"

[libraries]
androidx-core-ktx = { group = "androidx.core", name = "core-ktx", version.ref = "coreKtx" }
androidx-activity-compose = { group = "androidx.activity", name = "activity-compose", version.ref = "activityCompose" }
androidx-lifecycle-runtime-compose = { group = "androidx.lifecycle", name = "lifecycle-runtime-compose", version.ref = "lifecycle" }
androidx-lifecycle-viewmodel-compose = { group = "androidx.lifecycle", name = "lifecycle-viewmodel-compose", version.ref = "lifecycle" }
androidx-work-runtime-ktx = { group = "androidx.work", name = "work-runtime-ktx", version.ref = "workManager" }
androidx-material3-adaptive = { group = "androidx.compose.material3.adaptive", name = "adaptive", version.ref = "materialAdaptive" }
compose-bom = { group = "androidx.compose", name = "compose-bom", version.ref = "composeBom" }
kotlinx-serialization-json = { group = "org.jetbrains.kotlinx", name = "kotlinx-serialization-json", version.ref = "kotlinxSerializationJson" }
kotlinx-coroutines-android = { group = "org.jetbrains.kotlinx", name = "kotlinx-coroutines-android", version.ref = "kotlinxCoroutinesAndroid" }
junit4 = { group = "junit", name = "junit", version.ref = "junit4" }
androidx-test-ext-junit = { group = "androidx.test.ext", name = "junit", version.ref = "androidxTestExtJunit" }
androidx-test-espresso-core = { group = "androidx.test.espresso", name = "espresso-core", version.ref = "espressoCore" }
androidx-test-runner = { group = "androidx.test", name = "runner", version.ref = "androidxTestRunner" }
androidx-test-rules = { group = "androidx.test", name = "rules", version.ref = "androidxTestRules" }
androidx-test-core = { group = "androidx.test", name = "core", version.ref = "androidxTestCore" }

[plugins]
android-application = { id = "com.android.application", version.ref = "agp" }
kotlin-android = { id = "org.jetbrains.kotlin.android", version.ref = "kotlin" }
kotlin-compose = { id = "org.jetbrains.kotlin.plugin.compose", version.ref = "kotlin" }
kotlin-serialization = { id = "org.jetbrains.kotlin.plugin.serialization", version.ref = "kotlin" }
```

### 5.4 `app/build.gradle.kts`

```kotlin
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

/**
 * Reads `version = "x.y.z"` out of native/Cargo.toml's [package] table (only), using
 * providers.fileContents so the read stays a declared, config-cache-safe build input
 * instead of a raw File(...).readText() call. Deliberately scoped to the [package] table
 * so it cannot accidentally match a dependency's inline `version = "..."` entry elsewhere
 * in the same Cargo.toml.
 */
fun readCargoVersion(): Triple<Int, Int, Int> {
    val cargoToml = providers.fileContents(
        layout.projectDirectory.file("../native/Cargo.toml")
    ).asText.orNull ?: error("../native/Cargo.toml not found or unreadable")

    val packageHeader = Regex("""(?m)^\[package\]\s*$""").find(cargoToml)
        ?: error("[package] table not found in native/Cargo.toml")
    val afterHeader = cargoToml.substring(packageHeader.range.last + 1)
    val nextHeader = Regex("""(?m)^\[.*\]\s*$""").find(afterHeader)
    val packageBody = if (nextHeader != null) afterHeader.substring(0, nextHeader.range.first) else afterHeader

    val versionLine = packageBody.lineSequence()
        .map { it.trim() }
        .firstOrNull { it.startsWith("version") && it.contains("=") }
        ?: error("version field not found inside [package] table of native/Cargo.toml")

    val versionValue = versionLine.substringAfter("=").trim().trim('"')
    val parts = versionValue.split(".")
    require(parts.size == 3) { "Unexpected Cargo version format: '$versionValue'" }
    return Triple(parts[0].trim().toInt(), parts[1].trim().toInt(), parts[2].trim().toInt())
}

val (cargoMajor, cargoMinor, cargoPatch) = readCargoVersion()
val appVersionName = "$cargoMajor.$cargoMinor.$cargoPatch"
// major*1_000_000 + minor*1_000 + patch keeps versionCode monotonically increasing as long as
// minor/patch each stay below 1000, matching this project's actual 0.x.y numbering.
val appVersionCode = cargoMajor * 1_000_000 + cargoMinor * 1_000 + cargoPatch

// Release signing: only registered when all four env vars are present. If any are missing,
// no "release" signingConfig is created and assembleRelease produces an unsigned APK.
val keystoreFile = providers.environmentVariable("ANDROID_KEYSTORE_FILE").orNull
val keystorePassword = providers.environmentVariable("ANDROID_KEYSTORE_PASSWORD").orNull
val releaseKeyAlias = providers.environmentVariable("ANDROID_KEY_ALIAS").orNull
val releaseKeyPassword = providers.environmentVariable("ANDROID_KEY_PASSWORD").orNull
val hasReleaseSigning = listOf(keystoreFile, keystorePassword, releaseKeyAlias, releaseKeyPassword)
    .all { !it.isNullOrBlank() }

android {
    namespace = "app.smartexplorer.android"
    compileSdk = 36

    defaultConfig {
        applicationId = "app.smartexplorer.android"
        minSdk = 30
        targetSdk = 36
        versionCode = appVersionCode
        versionName = appVersionName
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(keystoreFile!!)
                storePassword = keystorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            }
        }
    }

    buildTypes {
        release {
            // R8 default rules already keep native-method names/descriptors
            // (`-keepclasseswithmembernames,includedescriptorclasses class * { native <methods>; }`),
            // so JNI's Java_<pkg>_<Class>_<method> symbol resolution survives shrinking even if this
            // is later flipped to true. kotlinx-serialization ships its own bundled consumer
            // proguard rules (keeps @Serializable classes / generated $$serializer classes
            // automatically); the one documented gap is classes with a *named* companion object,
            // which need an explicit -keep rule of their own if minification is ever enabled.
            isMinifyEnabled = false
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
    }

    buildFeatures {
        compose = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    lint {
        // CI-safe default for a template: does not fail the build on pre-existing lint findings.
        // Prefer a checked-in lint-baseline.xml (lint { baseline = file("lint-baseline.xml") })
        // once the project has a real lint history, so *new* regressions still fail CI.
        abortOnError = false
    }
}

kotlin {
    jvmToolchain(17)
    compilerOptions {
        jvmTarget = JvmTarget.JVM_17
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    androidTestImplementation(platform(libs.compose.bom))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.work.runtime.ktx)
    implementation(libs.androidx.material3.adaptive)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)

    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    testImplementation(libs.junit4)

    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.espresso.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
}
```

### 5.5 `gradle.properties`

```properties
android.useAndroidX=true
android.nonTransitiveRClass=true
org.gradle.jvmargs=-Xmx4096m -Dfile.encoding=UTF-8
org.gradle.parallel=true
org.gradle.caching=true
# Configuration cache: left OFF by default in this lowest-risk template. AGP 8.13.x and Kotlin
# 2.4.20 both support it, but a single incompatible plugin/task can break the build in a way
# that's easy to misdiagnose from a remote-only CI log. The app/build.gradle.kts version-parsing
# function above already uses providers.fileContents (not File(...).readText()) specifically so
# turning this on later is a one-line, low-risk change once the rest of the build is proven:
# org.gradle.configuration-cache=true
```

`android.useAndroidX=true` is mandatory (every dependency in §2 is an AndroidX artifact).
`android.nonTransitiveRClass=true` is the current default for new projects (each module's generated
`R` class only contains its own resources, not a transitive union — faster incremental builds, no
resource-id collisions across modules). Reading `System.getenv(...)`/`providers.environmentVariable`
for the signing config in §5.4 needs no extra `gradle.properties` entry: Gradle's configuration
cache automatically tracks environment-variable reads made through the standard API as build
inputs, without requiring an explicit declaration.

### 5.6 `gradle/wrapper/gradle-wrapper.properties`

```properties
distributionBase=GRADLE_USER_HOME
distributionPath=wrapper/dists
distributionUrl=https\://services.gradle.org/distributions/gradle-8.13-bin.zip
distributionSha256Sum=20f1b1176237254a6fc204d8434196fa11a4cfb387567519c61556e8710aed78
networkTimeout=10000
validateDistributionUrl=true
zipStoreBase=GRADLE_USER_HOME
zipStorePath=wrapper/dists
```

`distributionSha256Sum` verified by fetching
`https://services.gradle.org/distributions/gradle-8.13-bin.zip.sha256` directly with `curl`
(64 hex chars, confirmed against the official `services.gradle.org/versions/all` JSON API's
`checksum` field for the `"version": "8.13"` entry — both agree byte-for-byte):

```
20f1b1176237254a6fc204d8434196fa11a4cfb387567519c61556e8710aed78
```

### 5.7 `gradle-wrapper.jar` provenance

- Download URL: `https://services.gradle.org/distributions/gradle-8.13-wrapper.jar`
- Official checksum URL: `https://services.gradle.org/distributions/gradle-8.13-wrapper.jar.sha256`
- Checksum (fetched directly, also cross-checked against the `versions/all` JSON API's
  `wrapperChecksum` field for the same entry — both agree):
  ```
  81a82aaea5abcc8ff68b3dfcb58b3c3c429378efd98e7433460610fecd7ae45f
  ```
- In practice this file is generated/refreshed by running `gradle wrapper --gradle-version 8.13`
  from a machine that already has a matching Gradle install (Gradle regenerates `gradle-wrapper.jar`
  itself and validates its own checksum) — that regeneration is a **local Gradle invocation** and
  therefore out of scope for this workstation per the repo's build/test/release restrictions; the
  main agent should either run it once on the remote CI runner, or commit a `gradle-wrapper.jar`
  obtained straight from the download URL above and verify it against the checksum before commit.

### 5.8 `gradlew` / `gradlew.bat` source

Canonical source for both wrapper scripts is the `gradle/gradle` GitHub repository at the release
tag matching the wrapper version, confirmed to exist via the GitHub tag API
(`api.github.com/repos/gradle/gradle/git/refs/tags/v8.13.0` → resolves to commit
`073314332697ba45c16c0a0ce1891fa6794179ff`):

- `https://github.com/gradle/gradle/blob/v8.13.0/gradlew`
- `https://github.com/gradle/gradle/blob/v8.13.0/gradlew.bat`

Note the tag is `v8.13.0` (three-component, `v`-prefixed) even though the published distribution
and wrapper-properties value are the two-component `8.13` — Gradle's own git tagging convention
always pads to three components regardless of what the release is publicly called.

---

## 6. Pitfalls

| Pitfall | Detail |
|---|---|
| `namespace` vs `applicationId` | `namespace` (in `android {}`) is the compile-time root package for generated `R`/`BuildConfig` classes; `applicationId` (in `defaultConfig {}`) is the runtime/Play-identity package id. They are independent and may differ — this template sets both to the same placeholder `app.smartexplorer.android` for simplicity, but changing `applicationId` alone (e.g. per build flavor) does **not** require renaming any Kotlin package declaration. Also: **do not** additionally declare a `package="..."` attribute in `AndroidManifest.xml` — that attribute is deprecated/conflicts with the `namespace` DSL value once AGP owns namespace resolution. |
| Lint on CI | `lint { abortOnError = false }` (used above) keeps a pre-existing lint backlog from blocking every CI run, but also means **new** lint regressions silently pass. The safer long-term setting is a checked-in `lint-baseline.xml` (`lint { baseline = file("lint-baseline.xml") } `) generated once against the current tree, so only *new* issues fail the build. Not set up in this template because generating the baseline itself requires a build run, out of scope for local, compiler-less editing. |
| AGP requires JDK 17 | Confirmed minimum **and** default for AGP 8.13.2 (§1.2). Matches the CI runner's default JDK (17) exactly — no explicit JDK pin/`JAVA_HOME` override needed for this combination, unlike if AGP 9.x (which also needs 17) or an even-older AGP tied to JDK 11 were used instead. |
| "compileSdk N with AGP X" unsupported-version warning | The warning (message pattern: *"We recommend using a newer Android Gradle plugin to use compileSdk = N"*) fires when `compileSdk` is **higher** than what the AGP version's own release was tested against; the escape hatch is `android.suppressUnsupportedCompileSdk=<N>` in `gradle.properties`. **Not needed here** — AGP 8.13.2's own release notes list max `compileSdk` support as **36.1**, so `compileSdk = 36` is inside the tested range, not past it. |
| Configuration cache vs. reading files in the build script | Plain `File(...).readText()` / `Files.readAllBytes(...)` calls inside a build script are **not tracked** as configuration-cache inputs, so a stale cached configuration can silently miss a file-content change. The version-parsing function in §5.4 uses `providers.fileContents(layout.projectDirectory.file(...)).asText` specifically to stay configuration-cache-correct (per Gradle's own guidance: *"Plugins and build scripts should not read files directly ... Instead, declare files as potential build configuration inputs using the value supplier APIs"*), even though configuration cache itself is left off by default in `gradle.properties` (§5.5) for this first iteration. |
| Compose compiler plugin version drift | Because `org.jetbrains.kotlin.plugin.compose` must equal the Kotlin version exactly (§2.1), bumping `kotlin` in the version catalog automatically re-versions the Compose compiler too (`version.ref = "kotlin"` on both) — do **not** hand-pin a separate literal version string for the compose-compiler plugin entry, that is the most common way this pairing silently drifts out of sync. |
| Old `composeOptions { kotlinCompilerExtensionVersion = ... }` block | Not used and not needed in this template — that block only applied when the Compose compiler shipped inside AGP itself (pre-Kotlin-2.0). With `org.jetbrains.kotlin.plugin.compose` applied, setting `kotlinCompilerExtensionVersion` is redundant/vestigial. |
| Compose UI artifact splitting | `androidx.compose.ui:ui` is only the core module; graphics primitives are a separate BOM-managed artifact, `androidx.compose.ui:ui-graphics` (hyphenated group-style name, not a classifier) — easy to mistype as `ui:ui:graphics` (which Gradle would parse as `group:name:version`, i.e. try to resolve version `"graphics"` of the `ui` artifact, and fail). |
| `useLegacyPackaging = true` trade-off | Requested explicitly for this template. `true` → native libraries are stored **compressed** in the APK and extracted to app-private storage at install time (works uniformly across all supported API levels, larger on-disk footprint after install, slightly slower first load). `false` (AGP's own default once `minSdk ≥ 23`) → libraries are stored **uncompressed and page-aligned**, mapped directly from the APK (smaller on-disk footprint, faster load, and the form that page-size-alignment tooling for the 16 KB requirement — see `docs/refs/android-toolchain.md` §3 — assumes). Since `minSdk = 30` here comfortably exceeds the API 23 floor either setting would need, `false` would also have been viable; `true` was kept because it was explicitly specified in the task brief, not because it was independently the lower-risk pick on this specific axis. |

---

## 7. Open items / contradictions for the main agent

- **AGP 9.4's exact runtime KGP dependency** was not independently confirmed the way AGP 9.0's
  (2.2.10) was — only 9.0's release notes stated the exact pinned version explicitly in the fetched
  content. Not load-bearing for the recommendation in §1.5 (which picks the 8.x line), but relevant
  if a future milestone migrates to AGP 9.x.
- **`androidx.window.core.layout.WindowSizeClass` vs `androidx.compose.material3.adaptive.WindowSizeClass`**
  (§4): two independent fetches agree on the former as the real import; one summarized fetch of the
  official guide page claimed the latter, but that same fetch also reported a Maven coordinate
  (`androidx.compose.material3:material3-adaptive`) that is contradicted by the raw
  `maven-metadata.xml` (real coordinate: `androidx.compose.material3.adaptive:adaptive`). Treated the
  two-source agreement as authoritative, but this was **not** independently confirmed against raw
  KDoc/source for `WindowAdaptiveInfo`'s property type — worth a 30-second spot check
  (`./gradlew :app:dependencies` won't show it; easiest is just letting the IDE/compiler resolve the
  import once real code exists) before relying on it further.
- **Material-icons-core 1.7.8 icon-name inventory** (§3.1) was matched against the well-known "core
  curated subset" convention, not re-verified icon-by-icon against the live KDoc index (that page
  did not render usefully through the available fetch tooling in this pass).
- **`material3.adaptive:adaptive:1.3.0`'s exact `minSdk`** (§2.2) was inferred from the general
  Compose-library `minSdk 21` convention, not read directly off that artifact's own POM/manifest.
- `gradle-wrapper.jar` itself (§5.7) is normally produced by running the `gradle wrapper` Gradle
  task, which is a local Gradle invocation and therefore out of scope for this workstation under the
  repo's build/test/release restrictions — the main agent needs to either have the remote CI runner
  generate/refresh it, or fetch the file directly from `services.gradle.org` and verify it against
  the checksum in §5.7 before committing it.
- This file recommends **AGP 8.13.2** over AGP 9.4.1 (the actual current AGP release overall, one
  patch ahead of the 9.4.0 figure `docs/refs/android-toolchain.md` cites) specifically for
  lowest-risk reasons laid out in §1.5. If the project's risk tolerance changes (e.g. once AGP 9.x's
  built-in-Kotlin ecosystem has a full year of real-world templates/tutorials behind it), that
  decision should be revisited rather than assumed permanent.

---

## 8. Korrektur 2026-09-25 (Hauptagent): AAR-Metadaten erzwingen ältere Bibliotheksstände

`aar-metadata.properties` der in §2 genannten neuesten Stände direkt aus den AARs gelesen
(`dl.google.com/android/maven2/.../*.aar`, `META-INF/com/android/build/gradle/aar-metadata.properties`):
core/core-ktx 1.19.1, lifecycle-*-compose 2.11.0, Compose ui/foundation 1.12.x (BOM 2026.08.00 und
2026.09.00) und material3-adaptive 1.3.0 verlangen **minCompileSdk=37** und
**minAndroidGradlePluginVersion=9.1.0** – mit AGP 8.13.2 und compileSdk 36 bricht `checkAarMetadata`.

Verbindliche Pins für AGP 8.13.2 / compileSdk 36 (jeweils neuester Stand mit minCompileSdk ≤ 36 und
minAGP ≤ 8.13.2, einzeln geprüft; transitive Auflösung über die Gradle-Module-Metadaten von 234
AndroidX-Artefakten ebenfalls geprüft – alle kompatibel):

| Artefakt | Version | minCompileSdk | minAGP |
|---|---|---|---|
| `androidx.compose:compose-bom` | **2026.06.01** (ui/foundation 1.11.4, material3 1.4.0, adaptive 1.2.0, material-icons-core 1.7.8, ui-test 1.11.4) | 35 | 8.6.0 |
| `androidx.core:core-ktx` | **1.18.0** | 36 | 8.9.1 |
| `androidx.activity:activity-compose` | 1.13.0 | 36 | 8.9.1 |
| `androidx.lifecycle:lifecycle-runtime-compose` / `-viewmodel-compose` | **2.10.0** | 35 | 8.6.0 |
| `androidx.work:work-runtime-ktx` / `work-testing` | 2.12.0 | 35 | 8.6.0 |
| `androidx.test.uiautomator:uiautomator` | 2.4.0 | 34 | 8.1.1 |
| `androidx.test:core`/`runner`/`rules` 1.7.0, `androidx.test.ext:junit` 1.3.0, `espresso-core`/`espresso-intents` 3.7.0 | – | – | – |

`currentWindowAdaptiveInfo()` stammt damit aus `adaptive` **1.2.0**; §4 gilt sinngemäß (API seit 1.0).
