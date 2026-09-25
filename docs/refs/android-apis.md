# Android Framework & Jetpack (non-Compose) APIs — targetSdk 36 / minSdk 30

Quelle: developer.android.com (reference pages + guide pages cited per section), learn.microsoft.com/dotnet/api (Mono.Android field mirror, used only to confirm exact int/hex constant values that developer.android.com's JS-rendered reference pages did not yield as static text) · Abgerufen: 2026-09-25

**Purpose.** Exact Kotlin-callable signatures (package, class, method, parameter types, API level) plus manifest XML for the Android framework/Jetpack (non-Compose) surface needed to implement Smart Explorer's Android shell: foreground sync service, notifications, WorkManager scheduling, boot-triggered restart, all-files storage access, opening/sharing files with other apps, APK self-update, network/power awareness, and a handful of misc platform calls. `targetSdk = 36` (Android 16), `minSdk = 30` (Android 11) per `docs/refs/android-toolchain.md` §2.

**Scope note.** This file is Android-framework/Jetpack-API facts only. It does not re-derive toolchain versions (`android-toolchain.md`), general platform *behavior* already captured in `android-platform.md` (FGS quotas, background-start exemptions, OAuth, 16 KB pages, edge-to-edge), the Rust↔JNI binding choice (`rust-jni.md`), or per-crate Android cross-compile viability (`android-rust-deps.md`) — those are cited by reference, not repeated, except where new findings in this pass **correct or resolve** something those files left open (flagged inline as "Resolves ref:").

## Files read
- `docs/refs/android-toolchain.md`, `docs/refs/android-platform.md`, `docs/refs/rust-jni.md`, `docs/refs/android-rust-deps.md` (existing background refs, read first per instructions; not duplicated below except where corrected/resolved).

---

## 1. Foreground services

### 1.1 `Service` subclass (`android.app.Service`)

```kotlin
override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int   // return e.g. START_NOT_STICKY
override fun onBind(intent: Intent?): IBinder?                                // return null if not bindable

// API 34+ (UPSIDE_DOWN_CAKE) — shortService-specific, single-arg
override fun onTimeout(startId: Int)

// API 35+ — general FGS-type timeout, two-arg
override fun onTimeout(startId: Int, fgsType: Int)
```

| Overload | Added | Fires for | Contract |
|---|---|---|---|
| `onTimeout(startId: Int)` | API 34 (`UPSIDE_DOWN_CAKE`) | Only `FOREGROUND_SERVICE_TYPE_SHORT_SERVICE`, whose 3-minute timeout elapsed | Must call `stopSelf()` / `stopService()` / `stopForeground()` within the callback; if the service is still not stopped after a short grace period, the app is declared ANR. This callback **can be relied on only on API 34+**; the type itself is usable on lower `minSdk` builds too, but `onTimeout(int)` is simply never invoked there — the app must still self-stop. Does **not** exist on API 33 and below. |
| `onTimeout(startId: Int, fgsType: Int)` | API 35 | Any FGS type whose type-specific time limit (e.g. the `dataSync`/`mediaProcessing` 6h/24h cap documented in `android-platform.md` §1) was exceeded | Same contract — call `Service.stopSelf()` within a few seconds or the system throws a fatal `RemoteServiceException` ("A foreground service of type [type] did not stop within its timeout: [component]"). |

Both callbacks are called on the main thread. Neither stops the service automatically — the override must call one of the stop methods itself.

Sources: [Service onTimeout(int,int)](https://developer.android.com/reference/android/app/Service#onTimeout(int,%20int)), [Foreground service timeouts](https://developer.android.com/develop/background-work/services/fgs/timeout), [ServiceInfo.ForegroundServiceTypeShortService (Mono mirror, full remarks text incl. `onTimeout(int)` single-arg reference)](https://learn.microsoft.com/en-us/dotnet/api/android.content.pm.serviceinfo.foregroundservicetypeshortservice?view=net-android-34.0).

### 1.2 `ServiceCompat` (`androidx.core.app.ServiceCompat`, artifact `androidx.core:core-ktx`/`androidx.core:core`)

```kotlin
// androidx.core.app.ServiceCompat — the `foregroundServiceType`-aware overload requires
// androidx.core:core 1.12.0+ (the plain 2-arg startForeground(Service, int, Notification)
// predates this and has existed since core 1.0.0; do not confuse the two).
fun ServiceCompat.startForeground(
    service: Service,
    id: Int,
    notification: Notification,
    @ServiceInfo.ForegroundServiceType foregroundServiceType: Int
)   // throws: ForegroundServiceStartNotAllowedException (API 31+),
    //         ForegroundServiceTypeException, MissingForegroundServiceTypeException,
    //         SecurityException

fun ServiceCompat.stopForeground(service: Service, @StopForegroundFlags flags: Int)
```

Stop-foreground constants (also `androidx.core.app.ServiceCompat`):
| Constant | Meaning |
|---|---|
| `ServiceCompat.STOP_FOREGROUND_REMOVE` | Stop foreground state and remove the notification |
| `ServiceCompat.STOP_FOREGROUND_DETACH` | Stop foreground state, leave the notification posted |
| `ServiceCompat.STOP_FOREGROUND_LEGACY` | Pre-Android-N compatibility behavior |

Minimal usage:
```kotlin
ServiceCompat.startForeground(
    this, NOTIFICATION_ID, notification,
    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
)
// ...
ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
```

Sources: [ServiceCompat reference](https://developer.android.com/reference/androidx/core/app/ServiceCompat), [Foreground services overview](https://developer.android.com/guide/components/foreground-services). **Uncertainty:** the exact minimum `androidx.core` version for the 4-arg `startForeground` overload was returned inconsistently across fetches (one pass said "1.5.0+", a targeted follow-up search said "1.12"); **1.12 is the more specific/likely-correct answer** (it lines up with the `ForegroundServiceStartNotAllowedException`/type-exception handling this overload wraps, introduced alongside Android 14 API support) but was not independently confirmed against the artifact's own release notes in this pass — verify against `androidx.core` release notes before pinning the Gradle version.

### 1.3 `ContextCompat.startForegroundService` (`androidx.core.content.ContextCompat`)

```kotlin
fun ContextCompat.startForegroundService(context: Context, intent: Intent): ComponentName?
```
API 26+: calls `Context.startForegroundService(Intent)`. Below 26: falls back to `Context.startService(Intent)` (foreground services didn't exist pre-O, so no separate foreground start call is needed/available). Since `minSdk = 30`, the pre-26 fallback path is dead code for this app but the compat call remains the correct one to use for source clarity/future minSdk changes.

Source: [ContextCompat reference](https://developer.android.com/reference/androidx/core/content/ContextCompat).

### 1.4 `ServiceInfo` foreground-service-type constants (`android.content.pm.ServiceInfo`)

| Constant | Int value | Manifest `foregroundServiceType` value | Required permission |
|---|---|---|---|
| `ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC` | `1` (`0x1`) | `dataSync` | `android.permission.FOREGROUND_SERVICE_DATA_SYNC` |
| `ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE` | `1073741824` (`0x40000000`) | `specialUse` | `android.permission.FOREGROUND_SERVICE_SPECIAL_USE` |
| `ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE` | `2048` (`0x800`) | `shortService` | none beyond base `android.permission.FOREGROUND_SERVICE` |

Values confirmed via the Mono.Android field mirror of the same AOSP `ServiceInfo.java` constants (`ApiSince=34`/`29` annotations preserved in that mirror), cross-checked against the constant-name/behavior text on developer.android.com. `SHORT_SERVICE` and `SPECIAL_USE` types can be combined with other types via bitwise `|` in code (`FOREGROUND_SERVICE_TYPE_DATA_SYNC or FOREGROUND_SERVICE_TYPE_SHORT_SERVICE`) and with `|` in the manifest string (`android:foregroundServiceType="dataSync|shortService"`) — see `android-platform.md` §1 for the manifest combination syntax and the `shortService`-ignored-when-combined behavior.

`specialUse` requires the extra manifest `<property>` element (exact syntax, confirmed verbatim from the same source):
```xml
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_SPECIAL_USE"/>
<service
    android:name=".MySpecialForegroundService"
    android:foregroundServiceType="specialUse">
    <property
        android:name="android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE"
        android:value="foo" />
</service>
```
The `android:value` is free-text explaining the concrete use case; it is what a Play Console reviewer reads (not relevant for sideloaded distribution per `android-platform.md` §1/§3, but still required by the OS to accept the manifest).

Sources: [ServiceInfo.ForegroundServiceTypeSpecialUse (value + manifest example)](https://learn.microsoft.com/en-us/dotnet/api/android.content.pm.serviceinfo.foregroundservicetypespecialuse?view=net-android-34.0), [ServiceInfo.ForegroundServiceTypeShortService (value + full contract text)](https://learn.microsoft.com/en-us/dotnet/api/android.content.pm.serviceinfo.foregroundservicetypeshortservice?view=net-android-34.0), [ServiceInfo.ForegroundServiceTypeDataSync (value)](https://learn.microsoft.com/en-us/dotnet/api/android.content.pm.serviceinfo.foregroundservicetypedatasync?view=net-android-35.0), [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types).

### 1.5 Manifest `<service>` declaration

```xml
<uses-permission android:name="android.permission.FOREGROUND_SERVICE"/>
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC"/>
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_SPECIAL_USE"/>
<uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>

<service
    android:name=".sync.SyncService"
    android:foregroundServiceType="dataSync"
    android:exported="false" />
```
For `targetSdk 34+`, omitting `android:foregroundServiceType` (both manifest and the `startForeground`/`ServiceCompat.startForeground` call) throws `MissingForegroundServiceTypeException` (see `android-platform.md` §1, unchanged, cited not repeated).

---

## 2. Notifications

### 2.1 Channel (`android.app.NotificationChannel` / `NotificationManager`, API 26+)

```kotlin
// android.app.NotificationChannel, API 26
NotificationChannel(id: String, name: CharSequence, importance: Int)

// android.app.NotificationManager
fun createNotificationChannel(channel: NotificationChannel)
```
Importance constants (`android.app.NotificationManager`): `IMPORTANCE_NONE=0`, `IMPORTANCE_MIN=1`, `IMPORTANCE_LOW=2`, `IMPORTANCE_DEFAULT=3`, `IMPORTANCE_HIGH=4`.

```kotlin
val channel = NotificationChannel("sync", "Sync", NotificationManager.IMPORTANCE_LOW)
getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
```

### 2.2 `NotificationCompat.Builder` (`androidx.core.app.NotificationCompat.Builder`)

```kotlin
NotificationCompat.Builder(context: Context, channelId: String)          // constructor
fun setSmallIcon(@DrawableRes icon: Int): NotificationCompat.Builder
fun setContentTitle(title: CharSequence?): NotificationCompat.Builder
fun setContentText(text: CharSequence?): NotificationCompat.Builder
fun setProgress(max: Int, progress: Int, indeterminate: Boolean): NotificationCompat.Builder
fun setOngoing(ongoing: Boolean): NotificationCompat.Builder
fun setOnlyAlertOnce(onlyAlertOnce: Boolean): NotificationCompat.Builder
fun addAction(@DrawableRes icon: Int, title: CharSequence?, intent: PendingIntent?): NotificationCompat.Builder
fun setContentIntent(intent: PendingIntent?): NotificationCompat.Builder
fun setForegroundServiceBehavior(behavior: Int): NotificationCompat.Builder   // API 31+ platform effect
fun build(): Notification
```
All builder setters return `NotificationCompat.Builder` for chaining except `build()`, which returns `Notification`.

`setForegroundServiceBehavior` constants (`NotificationCompat.FOREGROUND_SERVICE_DEFAULT`, `NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE`): on API 31+ the platform defers showing an FGS notification for a short time by default (to avoid flicker for very short-lived services); `FOREGROUND_SERVICE_IMMEDIATE` (maps to platform `Notification.FOREGROUND_SERVICE_IMMEDIATE`, added API 31) forces immediate display. Below API 31 this setter is a no-op (`NotificationCompat` degrades gracefully).

```kotlin
val notification = NotificationCompat.Builder(this, "sync")
    .setSmallIcon(R.drawable.ic_sync)
    .setContentTitle("Syncing")
    .setProgress(100, 42, false)
    .setOngoing(true)
    .setOnlyAlertOnce(true)
    .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
    .addAction(R.drawable.ic_cancel, "Cancel", cancelPendingIntent)
    .setContentIntent(openAppPendingIntent)
    .build()
```

Sources: [NotificationCompat.Builder reference](https://developer.android.com/reference/androidx/core/app/NotificationCompat.Builder), [Notification.FOREGROUND_SERVICE_IMMEDIATE (Mono mirror, confirms API 31)](https://learn.microsoft.com/en-us/dotnet/api/android.app.notification.foregroundserviceimmediate?view=net-android-35.0).

### 2.3 `NotificationManagerCompat` (`androidx.core.app.NotificationManagerCompat`)

```kotlin
companion object { fun from(context: Context): NotificationManagerCompat }
fun notify(id: Int, notification: Notification)
fun areNotificationsEnabled(): Boolean
```
On API 33+ (`POST_NOTIFICATIONS` runtime permission introduced), calling `notify()` without that permission granted **throws `SecurityException`** — the caller must check/request the permission first (§2.4). Below API 33 no runtime permission is needed (only the manifest `<uses-permission>`).

```kotlin
if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
        == PackageManager.PERMISSION_GRANTED) {
    NotificationManagerCompat.from(this).notify(NOTIFICATION_ID, notification)
}
```

Source: [NotificationManagerCompat reference](https://developer.android.com/reference/androidx/core/app/NotificationManagerCompat).

### 2.4 `PendingIntent` (`android.app.PendingIntent`)

```kotlin
companion object {
    fun getActivity(context: Context, requestCode: Int, intent: Intent, flags: Int): PendingIntent?
    fun getService(context: Context, requestCode: Int, intent: Intent, flags: Int): PendingIntent?
    fun getBroadcast(context: Context, requestCode: Int, intent: Intent, flags: Int): PendingIntent?
}
```
| Flag | Added | Value |
|---|---|---|
| `PendingIntent.FLAG_IMMUTABLE` | API 23 | `0x04000000` |
| `PendingIntent.FLAG_MUTABLE` | API 31 (retroactively documented back to 23 for the constant's existence, but **required** from 31) | `0x02000000` |

**API 31+ (targeting S): omitting both `FLAG_IMMUTABLE` and `FLAG_MUTABLE` throws `IllegalArgumentException`** ("One of FLAG_IMMUTABLE or FLAG_MUTABLE should be specified..."). Since `minSdk = 30 < 31`, always pass `PendingIntent.FLAG_IMMUTABLE` explicitly (or `or FLAG_UPDATE_CURRENT` as needed) rather than relying on the platform default, which differs across the 30/31 boundary.

```kotlin
val contentIntent = PendingIntent.getActivity(
    this, 0, Intent(this, MainActivity::class.java),
    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
)
```
Source: [PendingIntent reference](https://developer.android.com/reference/android/app/PendingIntent).

### 2.5 Runtime `POST_NOTIFICATIONS` request (API 33+)

```kotlin
val requestPermissionLauncher: ActivityResultLauncher<String> =
    registerForActivityResult(ActivityResultContracts.RequestPermission()) { isGranted: Boolean ->
        // isGranted == true ⇒ safe to NotificationManagerCompat.notify(...)
    }

requestPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
```
`ActivityResultContracts.RequestPermission` (`androidx.activity.result.contract`) is `ActivityResultContract<String, Boolean>` — input a single permission string, output whether it was granted. On a plain (non-Compose) `ComponentActivity`, `registerForActivityResult(contract, callback)` returns `ActivityResultLauncher<I>` (not the Compose-only `ManagedActivityResultLauncher`, which comes from `rememberLauncherForActivityResult` and is irrelevant here since this section is framework/Jetpack-non-Compose). Manifest: `<uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>` (normal-but-runtime-prompted permission on API 33+; below 33 it's implicitly granted and the launcher call is simply unnecessary but harmless to keep behind an SDK_INT check).

Sources: [ActivityResultContracts.RequestPermission reference](https://developer.android.com/reference/kotlin/androidx/activity/result/contract/ActivityResultContracts.RequestPermission), [Request runtime permissions](https://developer.android.com/training/permissions/requesting).

---

## 3. WorkManager 2.12 (package `androidx.work`)

Version confirmed current stable per `android-toolchain.md`/`android-platform.md`: **2.12.0** (Sept 23, 2026; raised `minSdk` to 24, which is below this app's `minSdk=30` so no conflict). `androidx.work:work-runtime-ktx` is now an empty shim — `CoroutineWorker` etc. live directly in `androidx.work:work-runtime` since 2.9.0; depend on `work-runtime-ktx` anyway for the `workDataOf`/`PeriodicWorkRequestBuilder` Kotlin extension functions (§3.4), which *are* still ktx-only.

### 3.1 `CoroutineWorker`

```kotlin
abstract class CoroutineWorker(appContext: Context, params: WorkerParameters) : ListenableWorker {
    abstract suspend fun doWork(): Result
    open suspend fun getForegroundInfo(): ForegroundInfo
}
```
`Result` (`androidx.work.ListenableWorker.Result`) is produced via `Result.success()`, `Result.success(outputData: Data)`, `Result.retry()`, or `Result.failure()` / `Result.failure(outputData: Data)`.

### 3.2 `setForeground` + `ForegroundInfo`

```kotlin
// inside CoroutineWorker.doWork()
suspend fun setForeground(foregroundInfo: ForegroundInfo)   // ListenableWorker method

// androidx.work.ForegroundInfo constructors
ForegroundInfo(notificationId: Int, notification: Notification)
ForegroundInfo(notificationId: Int, notification: Notification, foregroundServiceType: Int)
```
```kotlin
override suspend fun doWork(): Result {
    setForeground(createForegroundInfo())
    performSync()
    return Result.success()
}
private fun createForegroundInfo(): ForegroundInfo =
    ForegroundInfo(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
```
`setForeground` can throw `IllegalStateException` if called when the worker is not allowed to run in the foreground (e.g. constraints not met); wrap in try/catch per `android-platform.md` §2.

### 3.3 `PeriodicWorkRequest.Builder` / `PeriodicWorkRequestBuilder`

```kotlin
// androidx.work.PeriodicWorkRequest.Builder — Java-style constructors, both usable from Kotlin
PeriodicWorkRequest.Builder(
    workerClass: Class<out ListenableWorker>,
    repeatInterval: Long,
    repeatIntervalTimeUnit: TimeUnit
)
PeriodicWorkRequest.Builder(
    workerClass: Class<out ListenableWorker>,
    repeatInterval: Duration
)

// androidx.work top-level Kotlin builder functions (work-runtime-ktx)
inline fun <reified W : ListenableWorker> PeriodicWorkRequestBuilder(
    repeatInterval: Long,
    repeatIntervalTimeUnit: TimeUnit
): PeriodicWorkRequest.Builder
inline fun <reified W : ListenableWorker> PeriodicWorkRequestBuilder(
    repeatInterval: Duration
): PeriodicWorkRequest.Builder
```
**Minimum repeat interval: 15 minutes** (same floor as `JobScheduler`); a shorter value is silently clamped up to 15 min by WorkManager, not rejected with an exception.

```kotlin
val request = PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES)
    .setConstraints(constraints)
    .build()
// or, Duration overload:
val request2 = PeriodicWorkRequestBuilder<SyncWorker>(Duration.ofMinutes(15)).build()
```

### 3.4 `Constraints.Builder`

```kotlin
Constraints.Builder()
fun setRequiredNetworkType(networkType: NetworkType): Constraints.Builder
fun setRequiresCharging(requiresCharging: Boolean): Constraints.Builder
fun setRequiresBatteryNotLow(requiresBatteryNotLow: Boolean): Constraints.Builder
fun setRequiresStorageNotLow(requiresStorageNotLow: Boolean): Constraints.Builder
fun build(): Constraints
```
`NetworkType` (`androidx.work.NetworkType`) enum values: `CONNECTED`, `UNMETERED`, `NOT_ROAMING`, `METERED`, `NOT_REQUIRED`, `TEMPORARILY_UNMETERED` (API 30+ "unmetered for now" signal).
```kotlin
val constraints = Constraints.Builder()
    .setRequiredNetworkType(NetworkType.UNMETERED)
    .setRequiresCharging(true)
    .setRequiresBatteryNotLow(true)
    .build()
```

### 3.5 `WorkManager` enqueue/cancel/observe

```kotlin
companion object { fun getInstance(context: Context): WorkManager }

fun enqueueUniquePeriodicWork(
    uniqueWorkName: String,
    existingPeriodicWorkPolicy: ExistingPeriodicWorkPolicy,
    periodicWork: PeriodicWorkRequest
): Operation

fun cancelUniqueWork(uniqueWorkName: String): Operation

fun enqueueUniqueWork(
    uniqueWorkName: String,
    existingWorkPolicy: ExistingWorkPolicy,
    work: OneTimeWorkRequest
): Operation
// also: enqueueUniqueWork(uniqueWorkName: String, existingWorkPolicy: ExistingWorkPolicy, work: List<OneTimeWorkRequest>): Operation

fun getWorkInfosForUniqueWorkFlow(uniqueWorkName: String): Flow<List<WorkInfo>>
```
`ExistingPeriodicWorkPolicy` values: `KEEP`, `REPLACE`, `UPDATE` (since 2.8.0 — preserves work history/id instead of cancel+re-enqueue). `ExistingWorkPolicy` values: `REPLACE`, `KEEP`, `APPEND`, `APPEND_OR_REPLACE`.

```kotlin
WorkManager.getInstance(context).enqueueUniquePeriodicWork(
    "background-sync", ExistingPeriodicWorkPolicy.UPDATE, periodicRequest
)
WorkManager.getInstance(context).cancelUniqueWork("background-sync")

val expeditedRequest = OneTimeWorkRequestBuilder<SyncWorker>()
    .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
    .build()
WorkManager.getInstance(context).enqueueUniqueWork(
    "manual-sync", ExistingWorkPolicy.KEEP, expeditedRequest
)
```
`OutOfQuotaPolicy` (`androidx.work.OutOfQuotaPolicy`, WorkManager 2.7+): `RUN_AS_NON_EXPEDITED_WORK_REQUEST` (fall back to ordinary background work if expedited quota is exhausted) or `DROP_WORK_REQUEST` (drop entirely). `setExpedited(policy: OutOfQuotaPolicy): OneTimeWorkRequest.Builder` is a method on `OneTimeWorkRequest.Builder` (which `OneTimeWorkRequestBuilder<W>()` returns).

### 3.6 `WorkInfo.State` (`androidx.work.WorkInfo.State`)

Enum constants: `ENQUEUED`, `RUNNING`, `SUCCEEDED`, `FAILED`, `BLOCKED`, `CANCELLED`. `WorkInfo.State.isFinished()` (instance method) is `true` for `SUCCEEDED`/`FAILED`/`CANCELLED`, `false` for the other three.

### 3.7 `Data` / `Data.Builder` / `workDataOf`

```kotlin
// androidx.work.Data.Builder
Data.Builder()
fun putString(key: String, value: String?): Data.Builder
fun putInt(key: String, value: Int): Data.Builder
fun putBoolean(key: String, value: Boolean): Data.Builder
fun putLong(key: String, value: Long): Data.Builder
fun putStringArray(key: String, value: Array<String?>): Data.Builder
fun build(): Data

// work-runtime-ktx top-level function
inline fun workDataOf(vararg pairs: Pair<String, Any?>): Data
```
**Hard limit: a `Data` instance must not exceed 10 240 bytes when serialized** (`Data.MAX_DATA_BYTES`) — building/enqueuing a larger payload throws `IllegalStateException("Data cannot occupy more than 10240 bytes when serialized")` at either `Data.Builder.build()`-adjacent serialization or work-insertion time. Use `Data` only for small identifiers/config, not file contents or large lists.
```kotlin
val input = workDataOf("root_path" to "/storage/emulated/0/Documents", "dry_run" to false)
val request = OneTimeWorkRequestBuilder<SyncWorker>().setInputData(input).build()
// inside the worker: val path = inputData.getString("root_path")
```

### 3.8 Manifest merge for `SystemForegroundService`

```xml
<service
    android:name="androidx.work.impl.foreground.SystemForegroundService"
    android:foregroundServiceType="dataSync"
    tools:node="merge" />
```
Required when a `CoroutineWorker` calls `setForeground()` with a `dataSync`-typed `ForegroundInfo`; without the merge entry the manifest-declared type on WorkManager's internal service won't match and the FGS start fails the same `MissingForegroundServiceTypeException`/type-exception path as §1.5.

Sources: [WorkManager releases](https://developer.android.com/jetpack/androidx/releases/work), [CoroutineWorker reference](https://developer.android.com/reference/androidx/work/CoroutineWorker), [PeriodicWorkRequest.Builder reference](https://developer.android.com/reference/androidx/work/PeriodicWorkRequest.Builder), [Constraints.Builder reference](https://developer.android.com/reference/androidx/work/Constraints.Builder), [WorkManager reference](https://developer.android.com/reference/androidx/work/WorkManager), [Define work requests](https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work), [Data cannot occupy more than 10240 bytes (issue confirming the limit + exception text)](https://github.com/google/android-fhir/issues/707).

---

## 4. Boot-triggered restart

### 4.1 Receiver

```kotlin
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED -> {
                ContextCompat.startForegroundService(context, Intent(context, SyncService::class.java))
                // or: WorkManager.getInstance(context).enqueueUniquePeriodicWork(...)
            }
        }
    }
}
```
`Intent.ACTION_BOOT_COMPLETED` = `"android.intent.action.BOOT_COMPLETED"`; `Intent.ACTION_MY_PACKAGE_REPLACED` = `"android.intent.action.MY_PACKAGE_REPLACED"` (sent to the app itself right after it's updated/reinstalled, does **not** need `RECEIVE_BOOT_COMPLETED` or any special permission — it's a normal implicit-broadcast exception for the app's own package).

### 4.2 Manifest

```xml
<uses-permission android:name="android.permission.RECEIVE_BOOT_COMPLETED"/>

<receiver android:name=".boot.BootReceiver" android:exported="false">
    <intent-filter>
        <action android:name="android.intent.action.BOOT_COMPLETED"/>
        <action android:name="android.intent.action.MY_PACKAGE_REPLACED"/>
    </intent-filter>
</receiver>
```
`RECEIVE_BOOT_COMPLETED` is only required for the `BOOT_COMPLETED` action; `MY_PACKAGE_REPLACED` needs no permission. `android:exported="false"` is correct/required since API 33 (Android 13) manifest-declared receivers with intent filters must explicitly set `android:exported`; system broadcasts like these are still delivered to unexported receivers because the system, not another app, sends them.

### 4.3 Starting a foreground service from the receiver — type restriction by targetSdk

Both `BOOT_COMPLETED` and `MY_PACKAGE_REPLACED` are general background-start exemptions (per `android-platform.md` §1's exemption list), **but specific FGS *types* are separately blocked from boot-triggered starts once the app's `targetSdk` crosses the relevant threshold.** For **`targetSdk 36`** (this app), the confirmed restricted-from-`BOOT_COMPLETED` type list (apps targeting Android 15+) is:

| Type | Allowed from `BOOT_COMPLETED`? |
|---|---|
| `dataSync` | **No** — throws at start time (`"FGS type dataSync not allowed to start from BOOT_COMPLETED!"`) for apps targeting Android 15+ |
| `mediaPlayback` | No |
| `mediaProjection` | No |
| `phoneCall` | No |
| `camera`, `microphone` | No (blocked via the separate while-in-use restriction path, not the same as the four above, but also boot-blocked) |
| `connectedDevice` | **Yes** — not in the restricted list |
| `specialUse` | **Yes** — not in the restricted list |
| `location`, `health`, `remoteMessaging`, `mediaProcessing`, `shortService`, `systemExempted` | Yes — none restricted |

**Resolves ref:** `android-platform.md` §1 and its "Unresolved" section left this exact question open ("Whether `dataSync`/`connectedDevice` FGS types are themselves restricted from `BOOT_COMPLETED`-triggered starts was not conclusively found"). This pass found the primary-source answer: **`dataSync` is now explicitly disallowed from `BOOT_COMPLETED` at `targetSdk 35+`; `connectedDevice` and `specialUse` are not restricted.** Practical implication for this app (which needs a `dataSync`-typed sync service, per `android-platform.md` §1): **do not declare/start the sync service as `dataSync` directly from `BootReceiver`.** Two compliant options: (a) start it as `specialUse` (or `connectedDevice`, if it genuinely fits) from the receiver instead, or (b) don't start an FGS at all from the receiver — enqueue/update a WorkManager `enqueueUniquePeriodicWork(..., UPDATE, ...)` call there instead (WorkManager's own background-start path is not subject to this same FGS-type-from-boot restriction), and let the *first* `dataSync` FGS start happen later from a normal foreground/user-interaction context per §1.

Sources: [Changes to foreground service types for Android 15](https://developer.android.com/about/versions/15/changes/foreground-service-types) (per-type restricted-from-`BOOT_COMPLETED` statements), [Behavior changes: Android 15 §fgs-boot-completed](https://developer.android.com/about/versions/15/behavior-changes-15#fgs-boot-completed), [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types), [Implicit broadcast exceptions](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions).

---

## 5. Storage

### 5.1 All-files access

```kotlin
Environment.isExternalStorageManager(): Boolean   // static, android.os.Environment, API 30+

// Per-app request (preferred, targets exactly this app's package):
Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION).apply {
    data = Uri.parse("package:$packageName")
}
// Fallback (some OEM/older builds lack the per-app action; opens the general "All files access" list):
Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
```
`Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION` = `"android.settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION"` (`android.provider.Settings`, API 30). No runtime dialog exists for `MANAGE_EXTERNAL_STORAGE` — the manifest permission alone grants nothing; the user must flip the toggle in the Settings screen these intents open. Manifest:
```xml
<uses-permission android:name="android.permission.MANAGE_EXTERNAL_STORAGE" tools:ignore="ScopedStorage"/>
```

### 5.2 `StorageManager` / `StorageVolume`

```kotlin
val storageManager = context.getSystemService(StorageManager::class.java)   // android.os.storage.StorageManager
fun getStorageVolumes(): List<StorageVolume>                                 // API 24+

// android.os.storage.StorageVolume
fun getDirectory(): File?              // API 30+ — mount-point File, or null if not currently mounted/accessible
fun getDescription(context: Context): String
fun isPrimary(): Boolean
fun isRemovable(): Boolean
fun getState(): String                 // one of Environment.MEDIA_* string constants below
```
`Environment.MEDIA_*` state strings (`android.os.Environment`): `MEDIA_MOUNTED = "mounted"`, `MEDIA_MOUNTED_READ_ONLY = "mounted_ro"`, `MEDIA_UNMOUNTED = "unmounted"`, `MEDIA_CHECKING = "checking"`, `MEDIA_NOFS = "nofs"`, `MEDIA_SHARED = "shared"`, `MEDIA_BAD_REMOVAL = "bad_removal"`, `MEDIA_REMOVED = "removed"`, `MEDIA_EJECTING = "ejecting"`.
```kotlin
for (volume in storageManager.storageVolumes) {
    if (volume.state == Environment.MEDIA_MOUNTED) {
        val dir = volume.getDirectory()      // API 30+, null-check required
        val label = volume.getDescription(context)
    }
}
```

### 5.3 `StatFs`

```kotlin
StatFs(path: String)                    // android.os.StatFs, constructor, API 1
fun getAvailableBytes(): Long           // API 18+
fun getTotalBytes(): Long               // API 18+
```
```kotlin
val stat = StatFs(volume.directory!!.path)
val free = stat.availableBytes
val total = stat.totalBytes
```

### 5.4 App-private directories (`android.content.Context`, all API 1 except noted)

```kotlin
fun getFilesDir(): File
fun getCacheDir(): File
fun getNoBackupFilesDir(): File   // API 21+ — excluded from Auto Backup / cloud backup
```

Sources: [Manage all files on a storage device](https://developer.android.com/training/data-storage/manage-all-files), [StorageVolume reference](https://developer.android.com/reference/android/os/storage/StorageVolume), [StorageManager reference](https://developer.android.com/reference/android/os/storage/StorageManager), [Environment reference](https://developer.android.com/reference/android/os/Environment), [StatFs reference](https://developer.android.com/reference/android/os/StatFs), [Context reference](https://developer.android.com/reference/android/content/Context). All items in this section restate/tighten facts already in `android-platform.md` §3 with exact method signatures added; no contradiction found.

---

## 6. Opening / sharing / receiving files with other apps

### 6.1 `FileProvider` — `<paths>` element for "this app opens files anywhere on shared storage"

`<paths>` child elements and what each maps to (`androidx.core.content.FileProvider`, package `androidx.core:core`):

| Element | Attribute | Real directory | Safe for this app's "browse anywhere" use case? |
|---|---|---|---|
| `<files-path>` | `path` | `Context.getFilesDir()` | N/A (app-private) |
| `<cache-path>` | `path` | `Context.getCacheDir()` | **Yes — recommended pattern** (§6.1.1 below) |
| `<external-files-path>` | `path` | `Context.getExternalFilesDir(null)` | N/A (app-private external) |
| `<external-cache-path>` | `path` | `Context.getExternalCacheDir()` | N/A (app-private external) |
| `<external-media-path>` | `path` | `Context.getExternalMediaDirs()[0]` | N/A |
| `<external-path>` | `path` | Root of shared/external storage (`Environment.getExternalStorageDirectory()`) | **Avoid** — bypasses app sandboxing for the whole declared subtree; broad declarations are a known Play/security-review red flag |
| `<root-path>` | `path` | Root of the entire filesystem (`/`) | **Never use** — exposes the whole device filesystem through the provider |

Both caveats (`root-path` never, `external-path` broad-scope risk) are stated explicitly in `android-platform.md` §4 already and confirmed again against the live training page in this pass; not a new finding, restated here because it's the crux of task item 6.

**6.1.1 Recommended alternative for "this app opens arbitrary files anywhere on shared storage/removable volumes":** rather than declaring a single broad `<external-path path="."/>` (which would statically expose every removable volume's entire root through the provider, including volumes not yet known at manifest-write time — StorageVolume roots aren't fixed paths across all devices), copy/stream the specific file the user picked into the app's own cache dir just before sharing, and expose only that narrow, app-controlled `cache-path`:
```xml
<!-- res/xml/file_paths.xml -->
<paths xmlns:android="http://schemas.android.com/apk/res/android">
    <cache-path name="share_cache" path="share/" />
</paths>
```
```kotlin
val stagedFile = File(File(context.cacheDir, "share"), originalFile.name).also { it.parentFile?.mkdirs() }
originalFile.copyTo(stagedFile, overwrite = true)
val uri = FileProvider.getUriForFile(context, "$packageName.fileprovider", stagedFile)
```
This keeps the FileProvider declaration narrow and fixed regardless of how many/which removable volumes exist on a given device, at the cost of a copy for large files (acceptable for "open/share one file" flows; not recommended for bulk operations). If direct exposure of the real path (no copy) is required for a specific narrow reason, scope `<external-path>` to the single named subdirectory actually being shared rather than declaring the volume root, and build the `path` value dynamically only if the app can enumerate it safely — this project's default should be the cache-copy pattern above.

### 6.2 Provider manifest + `getUriForFile`

```xml
<provider
    android:name="androidx.core.content.FileProvider"
    android:authorities="${applicationId}.fileprovider"
    android:exported="false"
    android:grantUriPermissions="true">
    <meta-data android:name="android.support.FILE_PROVIDER_PATHS" android:resource="@xml/file_paths" />
</provider>
```
```kotlin
fun FileProvider.getUriForFile(context: Context, authority: String, file: File): Uri
// overload with a display name for the shared URI's last path segment:
fun FileProvider.getUriForFile(context: Context, authority: String, file: File, displayName: String): Uri
```

### 6.3 Opening a file in another app

```kotlin
val uri = FileProvider.getUriForFile(this, "$packageName.fileprovider", file)
val mimeType = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase())
    ?: "application/octet-stream"
val intent = Intent(Intent.ACTION_VIEW).apply {
    setDataAndType(uri, mimeType)
    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
}
try {
    startActivity(Intent.createChooser(intent, "Open with"))
} catch (e: ActivityNotFoundException) {
    // no app registered for this MIME type
}
```
`MimeTypeMap.getSingleton(): MimeTypeMap` (`android.webkit.MimeTypeMap`, static, API 1); `fun getMimeTypeFromExtension(extension: String): String?` — extension must be lower-case and **without** a leading dot, returns `null` if unknown. `Intent.createChooser(target: Intent, title: CharSequence?): Intent` always shows the app picker even if the user set a default handler (unlike a bare `startActivity(intent)`, which can silently open a stale default). `android.content.ActivityNotFoundException` is thrown by `startActivity`/`startActivityForResult` when no activity resolves the intent — must be caught explicitly since `ACTION_VIEW` for an arbitrary MIME type has no guaranteed handler.

### 6.4 Sharing out (`ACTION_SEND` / `ACTION_SEND_MULTIPLE`)

```kotlin
// Single file
val sendIntent = Intent(Intent.ACTION_SEND).apply {
    type = mimeType
    putExtra(Intent.EXTRA_STREAM, uri)
    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    clipData = ClipData.newRawUri(null, uri)   // enables preview thumbnails in the share sheet
}
startActivity(Intent.createChooser(sendIntent, null))

// Multiple files
val sendMultipleIntent = Intent(Intent.ACTION_SEND_MULTIPLE).apply {
    type = "*/*"
    putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))
    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    clipData = ClipData(null, arrayOf("*/*"), ClipData.Item(uris.first())).apply {
        uris.drop(1).forEach { addItem(ClipData.Item(it)) }
    }
}
startActivity(Intent.createChooser(sendMultipleIntent, null))
```

### 6.5 Receiving shared files + reading their metadata

Manifest (per app's own capability — narrow the `mimeType` if the app doesn't truly handle arbitrary types; the consolidated manifest in §10 uses `*/*` because this app is a general file manager and intentionally advertises broad handling, matching the task's explicit request):
```xml
<intent-filter>
    <action android:name="android.intent.action.SEND"/>
    <category android:name="android.intent.category.DEFAULT"/>
    <data android:mimeType="*/*"/>
</intent-filter>
<intent-filter>
    <action android:name="android.intent.action.SEND_MULTIPLE"/>
    <category android:name="android.intent.category.DEFAULT"/>
    <data android:mimeType="*/*"/>
</intent-filter>
```
```kotlin
// androidx.core.content.IntentCompat — type-safe replacement for the deprecated
// Intent.getParcelableExtra(String) (unbounded-type overload deprecated API 33)
fun <T : Parcelable> IntentCompat.getParcelableExtra(intent: Intent, name: String, clazz: Class<T>): T?
fun <T : Parcelable> IntentCompat.getParcelableArrayListExtra(intent: Intent, name: String, clazz: Class<T>): ArrayList<T>?

val uri = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
uri?.let { contentResolver.openInputStream(it) }?.use { input -> /* read */ }

// File display name via OpenableColumns (android.provider.OpenableColumns)
val displayName: String? = uri?.let { u ->
    contentResolver.query(u, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
        if (cursor.moveToFirst()) cursor.getString(cursor.getColumnIndexOrThrow(OpenableColumns.DISPLAY_NAME)) else null
    }
}
```
`ContentResolver.openInputStream(uri: Uri): InputStream?` (`android.content.ContentResolver`, API 1) opens a stream on a `content://` (or `file://`) URI; returns `null` if the provider can't supply one. `OpenableColumns.DISPLAY_NAME` / `OpenableColumns.SIZE` are projection column-name string constants (`"_display_name"` / `"_size"`), not values themselves — always resolve the column index via `getColumnIndexOrThrow`/`getColumnIndex` rather than assuming column 0.

### 6.6 `singleTop` + `onNewIntent` in a `ComponentActivity`

```kotlin
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handleIncomingIntent(intent)
        addOnNewIntentListener { newIntent -> handleIncomingIntent(newIntent) }
    }
    private fun handleIncomingIntent(intent: Intent) { /* ACTION_SEND / SEND_MULTIPLE handling */ }
}
```
`ComponentActivity.addOnNewIntentListener(listener: Consumer<Intent>)` (`androidx.activity.ComponentActivity`, backed by `androidx.core.util.Consumer<T>` — a Kotlin lambda `(Intent) -> Unit` satisfies the SAM-convertible `Consumer<Intent>` interface). Companion `removeOnNewIntentListener(listener: Consumer<Intent>)` to unregister. This is the modern (non-deprecated) replacement for overriding `onNewIntent(Intent)` directly; it composes with multiple listeners (e.g. a nav-controller deep-link listener plus this app's own) without one overriding the other. Manifest activity must declare `android:launchMode="singleTop"` so a second `ACTION_SEND` while the activity is already on top re-delivers via `onNewIntent`/the listener instead of creating a second instance:
```xml
<activity
    android:name=".MainActivity"
    android:launchMode="singleTop"
    android:windowSoftInputMode="adjustResize"
    android:exported="true">
```

Sources: [FileProvider reference](https://developer.android.com/reference/androidx/core/content/FileProvider), [Setup file sharing](https://developer.android.com/training/secure-file-sharing/setup-sharing), [Send simple data to other apps](https://developer.android.com/training/sharing/send), [Receive simple data from other apps](https://developer.android.com/training/sharing/receive), [MimeTypeMap reference](https://developer.android.com/reference/android/webkit/MimeTypeMap), [OpenableColumns reference](https://developer.android.com/reference/android/provider/OpenableColumns), [ComponentActivity#addOnNewIntentListener reference](https://developer.android.com/reference/androidx/activity/ComponentActivity#addOnNewIntentListener(androidx.core.util.Consumer)), [`<activity>` manifest element](https://developer.android.com/guide/topics/manifest/activity-element).

---

## 7. APK install

```kotlin
context.packageManager.canRequestPackageInstalls(): Boolean   // android.content.pm.PackageManager, API 26+

Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES).apply { data = Uri.parse("package:$packageName") }
```
Manifest: `<uses-permission android:name="android.permission.REQUEST_INSTALL_PACKAGES"/>` (required on API 26+ to use `ACTION_INSTALL_PACKAGE`/`PackageInstaller`).

### 7.1 Simple path — `ACTION_VIEW` + FileProvider

```kotlin
val apkUri = FileProvider.getUriForFile(this, "$packageName.fileprovider", apkFile)
val installIntent = Intent(Intent.ACTION_VIEW).apply {
    setDataAndType(apkUri, "application/vnd.android.package-archive")
    flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK
}
startActivity(installIntent)
```
Delegates entirely to the system installer UI; no programmatic success/failure callback — the app only learns the outcome if the user returns to it and it re-checks (e.g. via `PackageInfo` version comparison), or not at all if the user abandons the flow.

### 7.2 Full-control path — `PackageInstaller` session

```kotlin
// android.content.pm.PackageInstaller.SessionParams
PackageInstaller.SessionParams(mode: Int)              // mode = PackageInstaller.SessionParams.MODE_FULL_INSTALL (= 1)

// android.content.pm.PackageInstaller
fun createSession(params: PackageInstaller.SessionParams): Int          // returns sessionId
fun openSession(sessionId: Int): PackageInstaller.Session

// android.content.pm.PackageInstaller.Session
fun openWrite(name: String, offsetBytes: Long, lengthBytes: Long): OutputStream
fun fsync(out: OutputStream)
fun commit(statusReceiver: IntentSender)
fun close()
```
```kotlin
val installer = packageManager.packageInstaller
val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
val sessionId = installer.createSession(params)
installer.openSession(sessionId).use { session ->
    session.openWrite("apk", 0, apkFile.length()).use { out ->
        apkFile.inputStream().use { it.copyTo(out) }
        session.fsync(out)
    }
    val statusReceiverIntent = Intent(this, InstallResultReceiver::class.java)
    val pi = PendingIntent.getBroadcast(this, sessionId, statusReceiverIntent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    session.commit(pi.intentSender)
}
```
Result receiver:
```kotlin
class InstallResultReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                val confirmIntent = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_INTENT, Intent::class.java)
                confirmIntent?.let { context.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            }
            PackageInstaller.STATUS_SUCCESS -> { /* done */ }
            else -> {
                val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
                // status is one of STATUS_FAILURE, STATUS_FAILURE_ABORTED, STATUS_FAILURE_BLOCKED,
                // STATUS_FAILURE_CONFLICT, STATUS_FAILURE_INCOMPATIBLE, STATUS_FAILURE_INVALID,
                // STATUS_FAILURE_STORAGE — named int constants on PackageInstaller; exact numeric
                // values were not confirmed against a static primary-source page in this pass
                // (see Unresolved) — always branch on the named constant, never a literal int.
            }
        }
    }
}
```
`EXTRA_STATUS: String`, `EXTRA_STATUS_MESSAGE: String`, `EXTRA_SESSION_ID: String` are extra-key constants on `PackageInstaller` used to read the broadcast `Intent`'s extras (not to be confused with `Intent.EXTRA_INTENT`, which carries the confirmation `Intent` to launch on `STATUS_PENDING_USER_ACTION`).

### 7.3 Which is simpler/more robust on API 30–36

The **`ACTION_VIEW` + FileProvider path is simpler** (a handful of lines, no session bookkeeping, no result receiver) and remains fully functional across API 30–36 without version-specific branches. The **`PackageInstaller` session path is more robust**: it gives a programmatic, in-process success/failure/pending-user-action result via the `IntentSender` callback instead of relying on the user returning to the app, and it is the mechanism required for silent/streamed updates (not applicable here without being the device owner) and for apps that want to show their own install-progress UI. For a sideloaded self-updater (this app's actual use case per `android-rust-deps.md`'s framing and the task's own §7 ask), the session path is the better fit specifically **because** it reports completion back to the app deterministically; the simple `ACTION_VIEW` path is adequate only if the app is fine with "fire and forget, re-check version on next launch."

Sources: [PackageInstaller reference](https://developer.android.com/reference/android/content/pm/PackageInstaller), [PackageInstaller.Session reference](https://developer.android.com/reference/android/content/pm/PackageInstaller.Session), [Request install packages permission](https://developer.android.com/reference/android/Manifest.permission#REQUEST_INSTALL_PACKAGES). This section restates/extends `android-platform.md` §8 with the exact session write/commit signatures it did not include; no contradiction found.

---

## 8. Network / power

### 8.1 `ConnectivityManager.NetworkCallback`

```kotlin
// android.net.ConnectivityManager.NetworkCallback — override what's needed
override fun onAvailable(network: Network)
override fun onLost(network: Network)
override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities)

// android.net.ConnectivityManager
fun registerDefaultNetworkCallback(networkCallback: ConnectivityManager.NetworkCallback)   // API 24+
fun unregisterNetworkCallback(networkCallback: ConnectivityManager.NetworkCallback)
```
`registerDefaultNetworkCallback` fires `onAvailable`/`onCapabilitiesChanged` immediately with the current default network's state (not just on the next change), and requires `ACCESS_NETWORK_STATE` (normal, manifest-only, no runtime prompt).

`android.net.NetworkCapabilities`:
```kotlin
fun hasCapability(capability: Int): Boolean
fun hasTransport(transport: Int): Boolean
```
Relevant constants: `NetworkCapabilities.NET_CAPABILITY_NOT_METERED`, `NetworkCapabilities.TRANSPORT_WIFI` (exact int values not confirmed as static text in this pass — always reference by name via `hasCapability(...)`/`hasTransport(...)`, never a literal int).

```kotlin
val callback = object : ConnectivityManager.NetworkCallback() {
    override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
        val unmetered = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        val onWifi = caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
    }
}
connectivityManager.registerDefaultNetworkCallback(callback)
```
`ConnectivityManager.isActiveNetworkMetered(): Boolean` remains a valid, non-deprecated convenience call for a one-shot metered check (a specific "deprecated since 29" claim surfaced once in this pass's research but was **not corroborated** by any other source or by the method's continued prominent documentation on the official "Monitor connectivity" training page — treated as unconfirmed/likely wrong; prefer it for simple one-shot checks and reserve the `NetworkCapabilities.hasCapability(NET_CAPABILITY_NOT_METERED)` form for use inside a live `NetworkCallback`, which is the pattern the platform docs actually push developers toward).

### 8.2 `WifiManager.MulticastLock`

```kotlin
fun WifiManager.createMulticastLock(tag: String): WifiManager.MulticastLock   // android.net.wifi.WifiManager
fun acquire()
fun release()
fun setReferenceCounted(refCounted: Boolean)
fun isHeld(): Boolean
```
Requires `<uses-permission android:name="android.permission.CHANGE_WIFI_MULTICAST_STATE"/>` (normal permission). Needed for the `mdns-sd` Rust crate to receive multicast mDNS traffic on Android, per `android-rust-deps.md` §1/§5 (already flagged there; this is the exact Kotlin-side call that must wrap the discovery window).
```kotlin
val lock = wifiManager.createMulticastLock("se-mdns").apply { setReferenceCounted(true) }
lock.acquire()
try { /* discovery window */ } finally { lock.release() }
```

### 8.3 `PowerManager`

```kotlin
fun isPowerSaveMode(): Boolean                              // android.os.PowerManager
fun isIgnoringBatteryOptimizations(packageName: String): Boolean
```
`PowerManager.ACTION_POWER_SAVE_MODE_CHANGED = "android.os.action.POWER_SAVE_MODE_CHANGED"`.
```kotlin
val filter = IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
```
`ContextCompat.registerReceiver(context: Context, receiver: BroadcastReceiver, filter: IntentFilter, flags: Int): Intent?` (`androidx.core.content.ContextCompat`) — the `flags` parameter (`ContextCompat.RECEIVER_NOT_EXPORTED` or `ContextCompat.RECEIVER_EXPORTED`) is **mandatory on this compat call regardless of `minSdk`**; ContextCompat back-fills the correct pre-33 behavior itself. `RECEIVER_NOT_EXPORTED` is correct here since `POWER_SAVE_MODE_CHANGED` is a system broadcast this app only needs to observe locally, not one other apps should be able to spoof into this receiver.

Battery-optimization exemption request (unchanged from `android-platform.md` §6, cited not repeated): `Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` + `Uri.parse("package:$packageName")`, `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` manifest permission.

Sources: [ConnectivityManager.NetworkCallback reference](https://developer.android.com/reference/android/net/ConnectivityManager.NetworkCallback), [ConnectivityManager reference](https://developer.android.com/reference/android/net/ConnectivityManager), [NetworkCapabilities reference](https://developer.android.com/reference/android/net/NetworkCapabilities), [Monitor connectivity status and connection metering](https://developer.android.com/training/monitoring-device-state/connectivity-status-type), [WifiManager.MulticastLock reference](https://developer.android.com/reference/android/net/wifi/WifiManager.MulticastLock), [PowerManager reference](https://developer.android.com/reference/android/os/PowerManager), [ContextCompat reference](https://developer.android.com/reference/androidx/core/content/ContextCompat).

---

## 9. Misc

### 9.1 `ThumbnailUtils.createImageThumbnail` (API 29+)

```kotlin
// android.media.ThumbnailUtils, static
@Throws(IOException::class)
fun createImageThumbnail(file: File, size: Size, signal: CancellationSignal?): Bitmap
```
```kotlin
val bmp: Bitmap = ThumbnailUtils.createImageThumbnail(imageFile, Size(256, 256), null)
val imageBitmap: ImageBitmap = bmp.asImageBitmap()   // androidx.compose.ui.graphics, extension fn, artifact androidx.compose.ui:ui-graphics
```
`Bitmap.asImageBitmap(): ImageBitmap` is a Compose extension function (package `androidx.compose.ui.graphics`) — listed here only as the conversion point since ThumbnailUtils itself is framework, not Compose; the reverse is `ImageBitmap.asAndroidBitmap(): Bitmap`.

### 9.2 `ClipboardManager` / `ClipData`

```kotlin
fun ClipboardManager.setPrimaryClip(clip: ClipData)               // android.content.ClipboardManager
fun ClipData.newPlainText(label: CharSequence, text: CharSequence): ClipData   // static factory, android.content.ClipData
```
```kotlin
val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
clipboard.setPrimaryClip(ClipData.newPlainText("path", fullPath))
```

### 9.3 `Application` subclass registration

```kotlin
class SmartExplorerApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        // init logging, WorkManager custom Configuration if used, etc.
    }
}
```
```xml
<application android:name=".SmartExplorerApplication" ...>
```

### 9.4 Opening a browser URL

```kotlin
try {
    startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
} catch (e: ActivityNotFoundException) {
    // no browser/handler available
}
```
Same `ActivityNotFoundException` contract as §6.3 — always wrap in try/catch since a stripped-down device could lack any `ACTION_VIEW`-for-`http(s)` handler.

### 9.5 `Build.VERSION.SDK_INT` checks

```kotlin
if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) { /* API 33+ only path */ }
```
`android.os.Build.VERSION_CODES` constants relevant to this app's `minSdk 30`/`targetSdk 36` span: `R` (30), `S` (31), `S_V2` (32), `TIRAMISU` (33), `UPSIDE_DOWN_CAKE` (34), `VANILLA_ICE_CREAM` (35), `BAKLAVA` (36).

### 9.6 `System.loadLibrary` naming rule

```kotlin
companion object { init { System.loadLibrary("smart_explorer") } }   // no "lib" prefix, no ".so" suffix
```
Matches the exact rule already documented in `rust-jni.md` §2.3 (loads `libsmart_explorer.so` as produced by `cargo ndk`); restated here only for cross-reference completeness, not re-derived.

Sources: [ThumbnailUtils reference](https://developer.android.com/reference/android/media/ThumbnailUtils), [ClipboardManager reference](https://developer.android.com/reference/android/content/ClipboardManager), [Build.VERSION_CODES reference](https://developer.android.com/reference/android/os/Build.VERSION_CODES), [Application overview](https://developer.android.com/reference/android/app/Application).

---

## 10. Consolidated `AndroidManifest.xml`

One `ComponentActivity`, `launchMode="singleTop"`, `windowSoftInputMode="adjustResize"`, plus every permission/service/receiver/provider/intent-filter from §1–§9. Package/authority placeholders use `${applicationId}` per the Gradle skeleton in `android-toolchain.md` §8.

```xml
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools">

    <!-- Foreground service (§1) -->
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE"/>
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC"/>
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE_SPECIAL_USE"/>

    <!-- Notifications (§2) -->
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>

    <!-- Boot (§4) -->
    <uses-permission android:name="android.permission.RECEIVE_BOOT_COMPLETED"/>

    <!-- Storage (§5) -->
    <uses-permission android:name="android.permission.MANAGE_EXTERNAL_STORAGE" tools:ignore="ScopedStorage"/>

    <!-- Network / power (§8) -->
    <uses-permission android:name="android.permission.INTERNET"/>
    <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE"/>
    <uses-permission android:name="android.permission.ACCESS_WIFI_STATE"/>
    <uses-permission android:name="android.permission.CHANGE_WIFI_MULTICAST_STATE"/>
    <uses-permission android:name="android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS"/>

    <!-- APK install (§7) -->
    <uses-permission android:name="android.permission.REQUEST_INSTALL_PACKAGES"/>

    <application
        android:name=".SmartExplorerApplication"
        android:label="@string/app_name"
        android:icon="@mipmap/ic_launcher"
        android:allowBackup="true">

        <activity
            android:name=".ui.MainActivity"
            android:exported="true"
            android:launchMode="singleTop"
            android:windowSoftInputMode="adjustResize">
            <intent-filter>
                <action android:name="android.intent.action.MAIN"/>
                <category android:name="android.intent.category.LAUNCHER"/>
            </intent-filter>
            <intent-filter>
                <action android:name="android.intent.action.SEND"/>
                <category android:name="android.intent.category.DEFAULT"/>
                <data android:mimeType="*/*"/>
            </intent-filter>
            <intent-filter>
                <action android:name="android.intent.action.SEND_MULTIPLE"/>
                <category android:name="android.intent.category.DEFAULT"/>
                <data android:mimeType="*/*"/>
            </intent-filter>
        </activity>

        <!-- §1: foreground sync service -->
        <service
            android:name=".sync.SyncService"
            android:foregroundServiceType="dataSync"
            android:exported="false"/>

        <!-- §1.4/§4.3: specialUse service usable from BootReceiver (dataSync is NOT boot-startable at targetSdk 36) -->
        <service
            android:name=".sync.BootSyncService"
            android:foregroundServiceType="specialUse"
            android:exported="false">
            <property
                android:name="android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE"
                android:value="Resumes user-configured background folder synchronization after device boot"/>
        </service>

        <!-- §3.8: WorkManager's own service, type merged to match the periodic sync worker -->
        <service
            android:name="androidx.work.impl.foreground.SystemForegroundService"
            android:foregroundServiceType="dataSync"
            tools:node="merge"/>

        <!-- §4: boot restart -->
        <receiver android:name=".boot.BootReceiver" android:exported="false">
            <intent-filter>
                <action android:name="android.intent.action.BOOT_COMPLETED"/>
                <action android:name="android.intent.action.MY_PACKAGE_REPLACED"/>
            </intent-filter>
        </receiver>

        <!-- §7.2: APK install session result -->
        <receiver android:name=".update.InstallResultReceiver" android:exported="false"/>

        <!-- §6: FileProvider -->
        <provider
            android:name="androidx.core.content.FileProvider"
            android:authorities="${applicationId}.fileprovider"
            android:exported="false"
            android:grantUriPermissions="true">
            <meta-data
                android:name="android.support.FILE_PROVIDER_PATHS"
                android:resource="@xml/file_paths"/>
        </provider>

    </application>
</manifest>
```

`res/xml/file_paths.xml` (§6.1.1's recommended narrow cache-copy pattern):
```xml
<?xml version="1.0" encoding="utf-8"?>
<paths xmlns:android="http://schemas.android.com/apk/res/android">
    <cache-path name="share_cache" path="share/" />
</paths>
```

---

## Contradictions / uncertain items (explicit per format requirement)

1. **`ServiceCompat.startForeground` minimum `androidx.core` version**: two fetches disagreed ("1.5.0+" vs "1.12"); §1.2 records 1.12 as the better-supported answer but flags it as not independently confirmed against `androidx.core` release notes.
2. **`ConnectivityManager.isActiveNetworkMetered()` deprecation**: one fetch claimed "deprecated since API 29 in favor of `NetworkCapabilities.hasCapability`"; not corroborated elsewhere and the method still appears prominently in the current official "Monitor connectivity" guide. §8.1 treats the deprecation claim as unconfirmed/likely wrong.
3. **`NetworkCapabilities.NET_CAPABILITY_NOT_METERED` / `TRANSPORT_WIFI` exact int values**: not obtained as static text from any primary source in this pass (the reference page only rendered as navigation JSON to the fetch tool). Always reference by name (`hasCapability`/`hasTransport`), never by literal int — recorded as a gap, not a wrong value.
4. **`PackageInstaller` `STATUS_FAILURE_*` exact integer values**: same JS-rendering limitation; only the named constants (`STATUS_FAILURE_ABORTED`, `STATUS_FAILURE_BLOCKED`, `STATUS_FAILURE_CONFLICT`, `STATUS_FAILURE_INCOMPATIBLE`, `STATUS_FAILURE_INVALID`, `STATUS_FAILURE_STORAGE`) are confirmed to exist; branch on the constants, not literal values.
5. **General fetch-tool limitation noted for transparency**: several `developer.android.com/reference/...` pages returned only the left-nav package/class index to the fetch tool rather than the class's own Javadoc body (the tool's own output repeatedly said so explicitly rather than silently guessing). Where that happened, this file's answer is either (a) cross-checked via a second targeted `WebSearch` pass whose snippets did contain the real method text (most cases — e.g. `Environment.isExternalStorageManager`, `StorageVolume`, `FileProvider` paths, `StatFs`, `Context` dirs), or (b) sourced from the Mono.Android/`learn.microsoft.com` mirror of the same AOSP field/class definitions when an exact numeric constant was needed (§1.4's `ServiceInfo` type values) — that mirror carries the same `ApiSince` annotations and links back to the matching `developer.android.com/reference/...` Java doc page for each field, so it is being used as a values-transcription aid for AOSP's own constants, not as an independent/divergent source.

## Unresolved
- Exact `androidx.core` version that introduced the `ServiceCompat.startForeground(Service, Int, Notification, Int)` overload (see Contradictions #1) — confirm against `androidx.core` release notes before pinning the Gradle catalog version.
- Exact int values of `NetworkCapabilities.NET_CAPABILITY_NOT_METERED`/`TRANSPORT_WIFI` and `PackageInstaller.STATUS_FAILURE_*` (see Contradictions #3/#4) — low practical risk since all call sites in this file branch on the named constants, but flagged in case any interop/JNI boundary ever needs the raw int.
- Whether `isActiveNetworkMetered()` carries any deprecation annotation in the exact targetSdk-36-era SDK stubs (see Contradictions #2) — worth a one-line confirmation at implementation time via Android Studio's own inline Javadoc/deprecation strikethrough, which this web-only research pass cannot check.
