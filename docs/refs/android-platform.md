# Android Platform Reference for a Background-Sync File Explorer

Purpose: Primary-source facts on Android platform APIs/behavior needed to plan an Android (APK) port of Smart Explorer (Kotlin/Compose UI + Rust core via JNI, background sync daemon, no drive mounting). Web research only, no source-code reading. All checks performed 2026-09-25 against developer.android.com / developers.google.com unless noted. This document reports facts, not architecture recommendations.

## Files read

None — this is a web-only research assignment. No repository files were read or modified except this output file.

---

## 1. Foreground services (Android 14 / 15 / 16)

### Service types (`android:foregroundServiceType`)

| Manifest value | Runtime constant | Manifest permission | Notes |
|---|---|---|---|
| `dataSync` | `FOREGROUND_SERVICE_TYPE_DATA_SYNC` | `FOREGROUND_SERVICE_DATA_SYNC` | Android 15+: capped at 6h/24h while app is in background (see below) |
| `connectedDevice` | `FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE` | `FOREGROUND_SERVICE_CONNECTED_DEVICE` | For transferring data to another local device |
| `mediaProcessing` | `FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING` | `FOREGROUND_SERVICE_MEDIA_PROCESSING` | Also capped 6h/24h on Android 15+ |
| `shortService` | `FOREGROUND_SERVICE_TYPE_SHORT_SERVICE` | none (only base `FOREGROUND_SERVICE`) | Must finish in ~3 min; if combined with another type, `shortService` declaration is ignored but other type's prerequisites still apply |
| `specialUse` | `FOREGROUND_SERVICE_TYPE_SPECIAL_USE` | `FOREGROUND_SERVICE_SPECIAL_USE` | Requires a `<property android:name="android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE" android:value="..."/>` child element in the `<service>` declaring the use case in plain text |
| systemExempted, camera, health, location, mediaPlayback, mediaProjection, microphone, phoneCall, remoteMessaging | (not relevant here) | `FOREGROUND_SERVICE_<TYPE>` | listed for completeness |

Base requirement for every foreground service (except `shortService`): `<uses-permission android:name="android.permission.FOREGROUND_SERVICE"/>` plus one `FOREGROUND_SERVICE_<TYPE>` permission per declared type. Multiple types are combined in the manifest with `|`, e.g. `android:foregroundServiceType="dataSync|connectedDevice"`.
For apps targeting API 34+, omitting the type on `startForeground()`/manifest throws `MissingForegroundServiceTypeException`.

### Android 15 `dataSync`/`mediaProcessing` 6h/24h limit

- Apps **targeting Android 15+**: `dataSync` and `mediaProcessing` foreground services are limited to **6 hours total per rolling 24h window while the app is in the background**; each type has its own separate counter, shared across all service instances of that type.
- Timer **resets when the user brings the app to the foreground**.
- On reaching the limit the system calls `Service.onTimeout(int timedOutForegroundServiceType, int instanceId)` (new callback since Android 15). The service must call `stopSelf()` within a few seconds; otherwise the system throws a fatal `RemoteServiceException`: `"A foreground service of type [type] did not stop within its timeout: [component]"`.
- Recommendation from the docs: prefer WorkManager, or start long dataSync FGS only from direct foreground user interaction so the timer resets.

### Restrictions on starting an FGS from the background (Android 12+, targetSdk 31+)

Apps in the background generally **cannot** call `startForegroundService()`. Full exemption list (any one qualifies):
- App transitions from a visible state (e.g. an Activity)
- App can start activities from background (except with an existing back-stack task)
- High-priority FCM message received
- User interacts with an app UI element (bubble, notification, widget, activity)
- App fires an exact alarm to complete a user-requested action
- App is the current input method
- Geofencing / activity-recognition transition event
- `ACTION_BOOT_COMPLETED`, `ACTION_LOCKED_BOOT_COMPLETED`, `ACTION_MY_PACKAGE_REPLACED` broadcast
- `ACTION_TIMEZONE_CHANGED` / `ACTION_TIME_CHANGED` / `ACTION_LOCALE_CHANGED` broadcast
- `ACTION_TRANSACTION_DETECTED` from `NfcService`
- Device/profile-owner roles; Companion Device Manager with `REQUEST_COMPANION_START_FOREGROUND_SERVICES_FROM_BACKGROUND` / `REQUEST_COMPANION_RUN_IN_BACKGROUND`
- User has disabled battery optimization for the app
- App holds `SYSTEM_ALERT_WINDOW` (Android 15+: must also currently show a visible overlay window)

If none apply: `ForegroundServiceStartNotAllowedException` is thrown.

### BOOT_COMPLETED-specific type restrictions

Even though `ACTION_BOOT_COMPLETED` is a general FGS-start exemption, specific *types* are separately blocked from BOOT_COMPLETED-triggered launches:
- targetSdk 15+: `mediaPlayback` and `mediaProjection` types disallowed from `BOOT_COMPLETED`
- targetSdk 14+: `microphone` type disallowed from `BOOT_COMPLETED`
- targetSdk 15+: `phoneCall` type disallowed from `BOOT_COMPLETED`
- `dataSync`/`connectedDevice` were **not** listed among these disallowed-from-boot types in the fetched pages — not independently re-verified beyond the search/fetch results below; treat as open item if boot-triggered dataSync start is required (see Unresolved).

### Declaration / start syntax

```xml
<uses-permission android:name="android.permission.FOREGROUND_SERVICE"/>
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC"/>
<uses-permission android:name="android.permission.POST_NOTIFICATIONS"/> <!-- API 33+ runtime permission -->
...
<service android:name=".SyncService"
    android:foregroundServiceType="dataSync"
    android:exported="false"/>
```
```kotlin
ServiceCompat.startForeground(
    this, NOTIFICATION_ID, notification,
    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
)
```
Notification channel (required API 26+, `POST_NOTIFICATIONS` runtime permission required API 33+ before a notification is actually shown to the user):
```kotlin
val channel = NotificationChannel("channelId", "Channel Name", NotificationManager.IMPORTANCE_DEFAULT)
notificationManager.createNotificationChannel(channel)
```

Sources (checked 2026-09-25): [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types) · [Declare foreground services and request permissions](https://developer.android.com/develop/background-work/services/fgs/declare) · [Foreground service timeouts](https://developer.android.com/develop/background-work/services/fgs/timeout) · [Restrictions on starting a foreground service from the background](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start) · [Changes to foreground service types for Android 15](https://developer.android.com/about/versions/15/changes/foreground-service-types) · [Behavior changes: Android 15](https://developer.android.com/about/versions/15/behavior-changes-15)

---

## 2. WorkManager

Current stable version (checked 2026-09-25 via the Jetpack releases page): **2.12.0** (Sep 23, 2026; raised `minSdk` from API 23 to API 24; added experimental `work-analytics` module).

| Feature | API | Notes |
|---|---|---|
| Minimum periodic interval | `PeriodicWorkRequestBuilder` / `PeriodicWorkRequest.Builder` | **15 minutes** minimum repeat interval (same floor as `JobScheduler`) |
| Constraints | `Constraints.Builder().setRequiredNetworkType(NetworkType.UNMETERED / CONNECTED).setRequiresCharging(true).setRequiresBatteryNotLow(true).setRequiresStorageNotLow(true)` | `UNMETERED` recommended for large transfers, `CONNECTED` for small |
| Unique periodic work | `WorkManager.enqueueUniquePeriodicWork(uniqueName, existingPeriodicWorkPolicy, request)` | `ExistingPeriodicWorkPolicy`: `KEEP` (ignore new if one queued), `REPLACE`, `UPDATE` (since 2.8.0, via `updateWork()`/`enqueueUniquePeriodicWork(..., UPDATE, ...)`, preserves work history/id) |
| Coroutine worker | `class X(ctx, params): CoroutineWorker(ctx, params) { override suspend fun doWork(): Result }` | must call `Result.success()/retry()/failure()` |
| Long-running / foreground work | `setForeground(ForegroundInfo(id, notification, foregroundServiceType))` inside `doWork()` | wrap in try/catch for `IllegalStateException`; requires declaring the FGS type on the manifest's WorkManager service (below) |
| Expedited work | introduced WorkManager 2.7.0 | short, user-important tasks; different quota path than long-running FGS-backed work |
| Observing | `WorkManager.getWorkInfosLiveData()/getWorkInfosFlow()`, `WorkQuery` | LiveData and Kotlin Flow variants both exist |

### Manifest for FGS-backed WorkManager

WorkManager's own `SystemForegroundService` must have its FGS type declared/merged:
```xml
<service
    android:name="androidx.work.impl.foreground.SystemForegroundService"
    android:foregroundServiceType="dataSync"
    tools:node="merge" />
```

### CoroutineWorker + ForegroundInfo example
```kotlin
override suspend fun doWork(): Result {
    setForeground(createForegroundInfo("Syncing..."))
    performDataSync()
    return Result.success()
}
private fun createForegroundInfo(progress: String): ForegroundInfo =
    ForegroundInfo(NOTIFICATION_ID, notification, ServiceCompat.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
```

### WorkManager vs. a user-started foreground Service

Both can run simultaneously — a `CoroutineWorker` can itself call `setForeground()` **and** the app can independently `ContextCompat.startForegroundService()` a separate long-lived Service; they are independent components sharing the FGS type quota system. Per docs fetched: **"Starting with Android 16, long running workers (which use foreground services) can exhaust your app's job quota. If this happens, you can try launching the foreground service directly instead of using WorkManager."** For user-initiated large transfers, docs point to "User-Initiated Data Transfer (UIDT) jobs" as exempt from ordinary job quotas (not independently fetched in depth — see Unresolved).

Sources: [WorkManager releases](https://developer.android.com/jetpack/androidx/releases/work) · [Define work requests](https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work) · [Support for long-running workers](https://developer.android.com/develop/background-work/background-tasks/persistent/how-to/long-running) · [Managing work](https://developer.android.com/develop/background-work/background-tasks/persistent/how-to/manage-work) · [Update work that is already enqueued](https://developer.android.com/develop/background-work/background-tasks/persistent/how-to/update-work) · [Threading in CoroutineWorker](https://developer.android.com/develop/background-work/background-tasks/persistent/threading/coroutineworker)

---

## 3. All-files access & storage volumes

| Item | Fact |
|---|---|
| Manifest | `<uses-permission android:name="android.permission.MANAGE_EXTERNAL_STORAGE" />` |
| Check | `Environment.isExternalStorageManager()` (returns `Boolean`) |
| Request intent | `Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)` — note: the exact constant name returned by the fetch is `Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION`; the task brief's `ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION` is the per-package variant, built with `data = Uri.parse("package:$packageName")` (requires `QUERY_ALL_PACKAGES` if targeting another app, not needed for your own package) |
| Grant scope | Read/write to all files in shared storage, `MediaStore.Files`, root of SD card and USB-OTG |
| Google Play policy | Play restricts `MANAGE_EXTERNAL_STORAGE` to apps whose core function needs it (file managers, backup/restore, anti-virus, document managers, on-device search, disk/file encryption, device-to-device migration) — **this policy applies to Play-distributed apps; a sideloaded APK is not reviewed by Play**, but the user must still manually grant the permission via system settings since there is no in-place runtime dialog for it |
| Pre-Android-11 | `READ_EXTERNAL_STORAGE`/`WRITE_EXTERNAL_STORAGE` install-time permissions gave broad shared-storage access; `android:requestLegacyExternalStorage="true"` in `<application>` opted an Android-10-targeting app out of scoped storage. **Android 11+ ignores this flag entirely** regardless of what the app targets. |
| Volumes API | `StorageManager.getStorageVolumes()` (`Context.getSystemService(StorageManager::class.java)`) returns `List<StorageVolume>`; `StorageVolume.getDirectory()` returns the mount-point `File` and **was added in API level 30** (confirmed via reference doc: "apps are instead strongly encouraged to interact with media on storage volumes via the MediaStore APIs" — direct path access has emulation overhead) |
| `Android/data/`, `Android/obb/` | For apps **targeting Android 11+**, `ACTION_OPEN_DOCUMENT`/`ACTION_OPEN_DOCUMENT_TREE` cannot browse into `Android/data/` or `Android/obb/` (and their subdirectories) of other apps; this is enforced independent of `MANAGE_EXTERNAL_STORAGE`. Not independently re-verified whether `MANAGE_EXTERNAL_STORAGE` itself still permits raw file I/O into other apps' `Android/data` — the docs fetched describe the SAF-picker restriction specifically (see Unresolved). |

Sources: [Manage all files on a storage device](https://developer.android.com/training/data-storage/manage-all-files) · [Storage updates in Android 11](https://developer.android.com/about/versions/11/privacy/storage) · [StorageVolume reference](https://developer.android.com/reference/android/os/storage/StorageVolume) · [Request special permissions](https://developer.android.com/training/permissions/requesting-special)

---

## 4. Opening / sharing / receiving files

### FileProvider (manifest + `file_paths.xml`)

```xml
<provider
    android:name="androidx.core.content.FileProvider"
    android:authorities="${applicationId}.fileprovider"
    android:exported="false"
    android:grantUriPermissions="true">
    <meta-data android:name="android.support.FILE_PROVIDER_PATHS" android:resource="@xml/file_paths" />
</provider>
```
```xml
<!-- res/xml/file_paths.xml -->
<paths xmlns:android="http://schemas.android.com/apk/res/android">
    <cache-path name="cache" path="/" />
    <files-path name="files" path="/" />
    <external-files-path name="external_files" path="/" />
    <external-cache-path name="external_cache" path="/" />
</paths>
```
Caveats confirmed: **never use `<root-path>`** (exposes the whole filesystem through the provider); `<external-path>` points at the root of shared/external storage and bypasses app sandboxing, so it should be avoided or scoped narrowly — relevant because this app's whole purpose is browsing arbitrary user-chosen paths, so any FileProvider path spec used for "open in another app" must be built dynamically/narrowly per actual file rather than declared as a single broad `<external-path>`.

```kotlin
val uri = FileProvider.getUriForFile(context, "$packageName.fileprovider", file)
val intent = Intent(Intent.ACTION_VIEW).apply {
    setDataAndType(uri, mimeType)
    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
}
startActivity(Intent.createChooser(intent, title)) // createChooser always shows the picker, even if user set a default
```

### MIME type lookup
```kotlin
val mimeType = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension.lowercase()) // no leading dot; must lowercase
```

### Sharing out
- Single file: `ACTION_SEND` + `putExtra(Intent.EXTRA_STREAM, uri)` + MIME type + `FLAG_GRANT_READ_URI_PERMISSION`.
- Multiple files: `ACTION_SEND_MULTIPLE` + `putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList<Uri>)`.
- For image/file previews in the share sheet, also attach a `ClipData` built from the same URI(s) (per `Intent.ACTION_SEND` reference).

### Receiving shared files
```xml
<activity android:name=".ui.MyActivity">
    <intent-filter>
        <action android:name="android.intent.action.SEND" />
        <category android:name="android.intent.category.DEFAULT" />
        <data android:mimeType="*/*" />
    </intent-filter>
    <intent-filter>
        <action android:name="android.intent.action.SEND_MULTIPLE" />
        <category android:name="android.intent.category.DEFAULT" />
        <data android:mimeType="*/*" />
    </intent-filter>
</activity>
```
Docs explicitly caution against `*/*` unless the app can genuinely handle any content type.
```kotlin
val uri = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
contentResolver.openInputStream(uri)?.use { /* read content:// stream */ }
```

Sources: [FileProvider reference](https://developer.android.com/reference/androidx/core/content/FileProvider) · [Send simple data to other apps](https://developer.android.com/training/sharing/send) · [Receive simple data from other apps](https://developer.android.com/training/sharing/receive) · [MimeTypeMap reference](https://developer.android.com/reference/android/webkit/MimeTypeMap)

---

## 5. Networking (mDNS, connectivity, netlink)

| Item | Fact |
|---|---|
| Multicast lock (needed to receive mDNS packets at all) | `wifiManager.createMulticastLock("tag")`, then `.setReferenceCounted(true)`, `.acquire()` / `.release()`. Requires `<uses-permission android:name="android.permission.CHANGE_WIFI_MULTICAST_STATE" />`. |
| Network monitoring | `ConnectivityManager.registerDefaultNetworkCallback(NetworkCallback)` / `registerNetworkCallback(...)`; callback overrides `onAvailable()`, `onLost()`, `onCapabilitiesChanged()`. `registerDefaultNetworkCallback` fires immediately with current state. |
| Metered check | `connectivityManager.isActiveNetworkMetered` |
| Permissions | `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE`, `CHANGE_WIFI_MULTICAST_STATE` (install-time, not runtime-prompted) |
| Android 11+ netlink restriction | Apps **targeting API 30+** are denied `bind()` on `NETLINK_ROUTE` sockets and `RTM_GETLINK` (needs `nlmsg_readpriv`, unavailable to app UIDs) — this is enforced by SELinux, tracked as AOSP bug b/155595000. Direct use of Go's `net.Interfaces()`-equivalent or naive netlink-based interface enumeration fails with `EACCES`. |
| Bionic/system workaround | Android's own `getifaddrs()` in bionic works around this by opening an **unbound** netlink socket (kernel auto-binds on first `send()`, which the sandbox permits) and discovering interface names via the `SIOCGIFNAME` ioctl (on Android's unprivileged ioctl allow-list), plus `RTM_GETADDR` (still permitted) instead of `RTM_GETLINK`. |
| Relevance to Rust | Any Rust crate that enumerates network interfaces via raw `NETLINK_ROUTE`/`RTM_GETLINK` (rather than going through libc `getifaddrs()` or the ioctl-based workaround) will fail with `EACCES` on API 30+ targets. crates.io has at least one crate (`getifs`) advertised as working inside the Android app sandbox via a libc-free/ioctl-based backend rather than raw netlink. Whatever crate Smart Explorer's mDNS/interface-discovery code currently uses on desktop needs to be checked against this restriction for the Android target — not verified against the actual crate in use (out of scope: read surface was web-only). |

Sources: [WifiManager.MulticastLock reference](https://developer.android.com/reference/android/net/wifi/WifiManager.MulticastLock) · [Monitor connectivity status and connection metering](https://developer.android.com/training/monitoring-device-state/connectivity-status-type) · [ConnectivityManager.NetworkCallback reference](https://developer.android.com/reference/android/net/ConnectivityManager.NetworkCallback) · [AOSP bionic getifaddrs change](https://android.googlesource.com/platform/bionic/+/ed57b98%5E!/) (secondary, cross-checked against docs.rs `getifs`/`getifaddrs` crate pages)

---

## 6. Battery optimization

| Item | Fact |
|---|---|
| Manifest permission | `<uses-permission android:name="android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS" />` |
| Request intent | `Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply { data = Uri.parse("package:$packageName") }` — shows a **system dialog directly** (no separate settings page navigation needed) |
| Check | `(context.getSystemService(Context.POWER_SERVICE) as PowerManager).isIgnoringBatteryOptimizations(packageName)` |
| Google Play policy | Play policy prohibits requesting this exemption unless the app's core function is adversely affected by Doze/App Standby; acceptable cases listed: IM/chat/calling apps unable to use high-priority FCM, safety apps, task-automation apps, peripheral companion apps. (Sideloaded distribution is not Play-reviewed, but the restriction reflects Google's general guidance on appropriate use, and the prompt itself is user-facing/interruptive regardless of store.) |

Source: [Doze and App Standby docs (battery optimization exemption)](https://developer.android.com/training/monitoring-device-state/doze-standby)

---

## 7. Google OAuth for Drive access from a sideloaded, CI-built APK

This is the most consequential finding for planning purposes — reported as facts only.

| Question | Fact |
|---|---|
| Loopback redirect (`http://127.0.0.1:port`) with a "Desktop app" OAuth client, invoked from Android via Custom Tab/browser | Google's loopback-migration guide states the loopback flow deprecation is scoped to the **OAuth client's registered type** in Google Cloud Console (Android, iOS, Chrome app), not to the runtime platform physically making the request; loopback continues to be documented as supported for **Desktop app**-type clients. The guide does not explicitly bless or forbid a Desktop-type client's loopback flow being driven from Android code — this exact scenario is not addressed. Google's Android-specific guidance separately does **not** mention loopback at all as an Android option (see below), and instead requires migrating any loopback usage found on an Android/iOS/Chrome client type to the platform SDKs. |
| Custom URI scheme redirect for Android OAuth clients | Explicitly stated: **"Custom URI schemes are no longer supported on Android and Chrome apps"** (cited reason: risk of app impersonation). |
| Google's recommended Android path | `AuthorizationClient` from Google Identity Services (`Identity.getAuthorizationClient(activity)`), used as: `AuthorizationRequest.builder().setRequestedScopes(listOf(Scope(DriveScopes.DRIVE_FILE))).build()`, then `.authorize(request)`, handling `AuthorizationResult.hasResolution()` → launch `authorizationResult.pendingIntent.intentSender` for consent; result exposes `getAccessToken()` (short-lived) and optionally `getServerAuthCode()` via `.requestOfflineAccess(serverClientId)` for a refresh token on a backend. |
| Registration requirement | **Mandatory**: an **Android-type OAuth client** must be created in Google Cloud Console with the app's exact **package name and SHA-1 signing-certificate fingerprint** (`keytool -list -v -keystore ...`) before `AuthorizationClient` will work for that build. |
| Sideloaded app built in CI with an unpredictable/unknown signing SHA-1 | Not directly addressed by Google's docs as a supported scenario. The registration model is fundamentally SHA-1-pinned: every distinct signing key that will run in production needs its own pre-registered Android OAuth client (debug-keystore SHA-1 and release-keystore SHA-1 are commonly registered as separate clients). **A CI pipeline that generates a new/random signing key per build would not work** — each new key would need a new Cloud Console Android-client registration (package name + that key's SHA-1) before OAuth succeeds; there is no documented fallback for "unregistered SHA-1, any key". A stable, fixed CI signing key (registered once) is required for this to function at all. Optional "Verify App Ownership" via Play Console reduces impersonation risk but is not required for the client to function — it's an anti-impersonation feature. |

Sources: [Loopback IP Address flow Migration Guide](https://developers.google.com/identity/protocols/oauth2/resources/loopback-migration) · [OAuth 2.0 for iOS & Desktop Apps](https://developers.google.com/identity/protocols/oauth2/native-app) · [Authorize access to Google user data (Android)](https://developer.android.com/identity/authorization) · [Client authentication (Google Play services)](https://developers.google.com/android/guides/client-auth)

---

## 8. APK self-update

| Item | Fact |
|---|---|
| Manifest permission | `<uses-permission android:name="android.permission.REQUEST_INSTALL_PACKAGES" />` |
| Capability check | `context.packageManager.canRequestPackageInstalls()` — must be `true` before installing; if `false`, direct the user to the "Install unknown apps" per-source setting (the old global "Unknown sources" toggle was removed and replaced with this per-app-source permission, reached via `ACTION_MANAGE_UNKNOWN_APP_SOURCES`) |
| Simple path | `FileProvider.getUriForFile(...)` → `Intent(ACTION_VIEW).apply { setDataAndType(uri, "application/vnd.android.package-archive"); flags = FLAG_GRANT_READ_URI_PERMISSION or FLAG_ACTIVITY_NEW_TASK }` → `startActivity()`; delegates entirely to the system installer UI. |
| Full-control path | `PackageInstaller` session API: `packageManager.packageInstaller.createSession(SessionParams(MODE_FULL_INSTALL))` → `openSession(id)` → `session.openWrite(name, 0, length)` + copy APK bytes → `session.commit(pendingIntent.intentSender)`; result delivered to a receiver reading `PackageInstaller.EXTRA_STATUS` (`STATUS_PENDING_USER_ACTION` requires launching the returned confirmation `Intent`, `STATUS_SUCCESS`, `STATUS_FAILURE` + `EXTRA_STATUS_MESSAGE`). |

Sources: [PackageInstaller reference](https://developer.android.com/reference/android/content/pm/PackageInstaller) · [PackageInstaller.Session reference](https://developer.android.com/reference/android/content/pm/PackageInstaller.Session) · [Android 8.0 behavior changes (unknown sources → per-app permission)](https://developer.android.com/about/versions/oreo/android-8.0-changes)

---

## 9. Edge-to-edge (targetSdk 35+) and predictive back

| Item | Fact |
|---|---|
| Enforcement | Once an app **targets SDK 35 (Android 15) or higher**, edge-to-edge layout is enforced automatically by the system — opting out is not available the way it was pre-35. |
| Compose setup | `enableEdgeToEdge()` called in `Activity.onCreate()` before/around `setContent {}`; makes system bars transparent by default (translucent scrim kept behind 3-button nav for contrast); system bar icon color adapts to light/dark theme automatically. |
| Manifest companion setting | `android:windowSoftInputMode="adjustResize"` on the activity, to keep IME resizing behavior correct under edge-to-edge. |
| Required app-side work | The app **must** consume `WindowInsets` itself (status bars, nav bars, display cutouts, IME) via Compose insets APIs/padding modifiers — content is no longer auto-padded away from system bars/cutouts. |
| Predictive back — targetSdk 36 (Android 16)+ | Predictive back system animations (back-to-home, cross-task, cross-activity) are **enabled by default** when both the app targets API 36+ and runs on an Android 16+ device; `android:enableOnBackInvokedCallback` defaults to `true` at the package-parser level for API 36 targets. |
| Behavior change | For apps targeting Android 16, `Activity.onBackPressed()` is **no longer called** and `KeyEvent.KEYCODE_BACK` is **no longer dispatched** — back handling must use the predictive-back callback APIs (`OnBackPressedCallback`, or Compose's `PredictiveBackHandler`/`BackHandler`) instead of intercepting the key event or overriding `onBackPressed()`. |
| Compose API | `PredictiveBackHandler { progress: Flow<BackEventCompat> -> ... }` composable for custom in-flight back animations/progress. |
| Escape hatch | Explicitly setting `android:enableOnBackInvokedCallback="false"` in `<application>`/`<activity>` opts an activity back out if migration isn't complete — documented as a temporary measure. |

Sources: [Set up Edge-to-edge (Compose)](https://developer.android.com/develop/ui/compose/system/setup-e2e) · [About system bar protection](https://developer.android.com/develop/ui/compose/system/system-bars) · [About window insets (Compose)](https://developer.android.com/develop/ui/compose/system/insets) · [Add support for the predictive back gesture](https://developer.android.com/guide/navigation/custom-back/predictive-back-gesture) · [About Predictive back (Compose)](https://developer.android.com/develop/ui/compose/system/predictive-back) · [Behavior changes: Android 16](https://developer.android.com/about/versions/16/behavior-changes-16)

---

## Cross-cutting notes relevant to the Rust core (`target_os="android"`)

- `target_os = "android"` is distinct from `target_os = "linux"` in `#[cfg(...)]` even though both have `target_family = "unix"` — confirmed as stated in the task brief; not independently re-derived here since it is a Rust-toolchain fact, not an Android-platform-behavior fact within this web-only assignment's scope.
- The netlink/`getifaddrs` restriction (section 5) and the FGS 6h/24h dataSync cap (section 1) are the two platform behaviors most likely to require Android-specific branches in the sync engine rather than being purely a JNI/UI wrapping exercise.
- The OAuth SHA-1 pinning requirement (section 7) is an operational/CI constraint (a stable, pre-registered signing key), not a code-architecture constraint.

## Unresolved / out-of-scope for this research pass

- Whether `dataSync`/`connectedDevice` FGS types are themselves restricted from `BOOT_COMPLETED`-triggered starts was not conclusively found (only `mediaPlayback`, `mediaProjection`, `microphone`, `phoneCall` were confirmed disallowed-from-boot in the pages fetched).
- Whether `MANAGE_EXTERNAL_STORAGE` (as opposed to the SAF picker) grants raw-filesystem read/write into other apps' `Android/data/`/`Android/obb/` was not conclusively confirmed either way in the fetched pages.
- User-Initiated Data Transfer (UIDT) jobs (mentioned in WorkManager's long-running-worker doc as exempt from Android 16 job quotas) were not independently fetched/verified in detail.
- Whether a Desktop-app-type OAuth client's loopback redirect, driven from an Android Custom Tab, is technically functional and ToS-compliant was not conclusively resolved either way by primary sources — Google's docs simply don't address that specific combination; the only unambiguous Android-native path found is the Android-type OAuth client + SHA-1 registration route.
- Actual current Rust crate used by Smart Explorer for mDNS/interface discovery was not checked (read surface for this task was web-only) — needs matching against the netlink/EACCES finding in section 5 by whoever reads the native source.
- No context7/androidx-library-doc lookups were performed beyond what WebSearch/WebFetch against developer.android.com and developers.google.com already covered; all facts above are from primary Google sources.
