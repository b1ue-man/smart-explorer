# Android-Speicheranalyse: StorageStats, FUSE-Walk, Hashing

Quellen (primär, AOSP-Stand `main`/`android1x-release`, soweit nicht als „sekundär“ markiert):
- developer.android.com/reference: `android/app/usage/StorageStatsManager`, `.../StorageStats`, `.../ExternalStorageStats`, `android/app/AppOpsManager`, `android/os/storage/StorageManager`, `android/os/PowerManager`, `android/provider/MediaStore*`, `android/content/pm/PackageManager`, `android/R.attr`, `android/Manifest.permission` (Seitenstand 2026-08)
- developer.android.com/training/data-storage/manage-all-files, /about/versions/11/privacy/storage, /develop/background-work/services/fgs/timeout, /games/optimize/adpf/thermal
- source.android.com/docs/core/storage/scoped, /fuse-passthrough, /docs/compatibility/{15,16,17}/android-N-cdd (§9.8), /docs/security/features/encryption/adiantum
- android.googlesource.com: `platform/frameworks/base` (`services/usage/.../StorageStatsService.java`, `core/jni/com_android_internal_os_Zygote.cpp`, `services/core/.../StorageManagerService.java`, `.../am/ProcessList.java`, `core/java/android/os/storage/StorageManager.java`), `platform/frameworks/native` (`cmds/installd/InstalldNativeService.cpp`, `utils.cpp`, `QuotaUtils.cpp`), `platform/packages/providers/MediaProvider` (`jni/FuseDaemon.cpp`, `jni/MediaProviderWrapper.cpp`, `src/.../MediaProvider.java`, `scan/ModernMediaScanner.java`, `util/FileUtils.java`), `platform/system/vold`, `platform/system/bpfprogs/fuseMedia.c`, `platform/external/libfuse`, `platform/packages/modules/Permission` (`EnhancedConfirmationService`), `platform/packages/apps/Settings`, `kernel/common` (`fs/fuse/readdir.c`, `gki_defconfig`)
- Sekundär: github.com/BLAKE3-team/BLAKE3 (PR #319, Kommentare), github.com/minio/sha256-simd README, lkml.iu.edu (Patch „arm64/sha256-ce finup_mb“), RustCrypto `sha2`/`cpufeatures`, github.com/termux/termux-app/issues/2174, androidauthority.com (Restricted Settings Android 15)

Abgerufen: 2026-10-01

Hinweis: „abgeleitet“ = aus Quellcode gelesen, nicht auf einem Gerät verifiziert. Die Plattform steht inzwischen bei API 37 (Android 17). Verwandt: `android-data-obb-access.md` (Shizuku etc.).

## 1. StorageStatsManager (API 26)

### 1.1 Semantik (Javadoc, wörtlich wo tragend)

> „no permissions are required when calling these APIs for your own package or UID. However, requesting details for any other package requires the `PACKAGE_USAGE_STATS` permission, which is a system-level permission that will not be granted to normal apps.“

Alle Methoden: „This method may take several seconds to complete, so it should only be called from a worker thread.“ `storageUuid`: `StorageManager.UUID_DEFAULT` oder `StorageManager.getUuidForPath(File)`.

| Methode (alle API 26) | Rechte (Code `StorageStatsService`) |
|---|---|
| `StorageStats queryStatsForPackage(UUID, String packageName, UserHandle)` | eigenes Paket frei; fremd: Usage access. Wirft `NameNotFoundException`, `IOException` |
| `StorageStats queryStatsForUid(UUID, int uid)` | eigene UID frei; fremd: Usage access |
| `StorageStats queryStatsForUser(UUID, UserHandle)` | immer Usage access („Always require permission to see user-level stats“) |
| `ExternalStorageStats queryExternalStatsForUser(UUID, UserHandle)` | immer Usage access |
| `long getTotalBytes(UUID)` / `getFreeBytes(UUID)` | keine („NOTE: No permissions required“) |

Anderer `UserHandle` als der eigene: zusätzlich `INTERACT_ACROSS_USERS`. `queryStatsForPackage` bei `sharedUserId` > 1 Paket: „slower manual calculation path“; sonst Fast-Path über `queryStatsForUid`.

### 1.2 StorageStats-Felder

- `getAppBytes()` (26): „This includes APK files, optimized compiler output, and unpacked native libraries. If the primary external/shared storage is hosted on this storage device, then this includes files stored under `Context.getObbDir()`.“ Code: `codeBytes = codeSize + externalCodeSize`; `externalCodeSize` = `calculate_tree_size(/data/media/<user>/Android/obb/<pkg>)`.
- `getDataBytes()` (26): intern `getDataDir/getCacheDir/getCodeCacheDir`, „then this includes“ `getExternalFilesDir`, `getExternalCacheDir`, `getExternalMediaDirs`. Code (Quota-Pfad): externer Anteil = Project-Quota `PROJECT_ID_EXT_DATA_START + (uid-10000)` plus `..._EXT_CACHE_START`; `Android/media/<pkg>` fließt im Quota-Pfad NICHT ein (nur im manuellen Pfad) – abgeleitet.
- `getCacheBytes()` (26): intern + `getExternalCacheDir`. `getExternalCacheBytes()` (31): nur externer Cache.
- `getAppBytesByDataType(int)` (35): Konstanten `APP_DATA_TYPE_FILE_TYPE_DEXOPT_ARTIFACT=0, REFERENCE_PROFILE=1, CURRENT_PROFILE=2, APK=3, DM=4, APP_DATA_TYPE_LIB=5`; für `queryStatsForUser` 0.
- Der externe Anteil je Paket (`Android/data/<pkg>` ohne Cache) wird NICHT separat geliefert; nur Summe in `getDataBytes` (zusammen mit internem Data).

### 1.3 ExternalStorageStats (öffentlich nur diese 5, alle API 26)

`getTotalBytes()`, `getAudioBytes()`, `getVideoBytes()`, `getImageBytes()`, `getAppBytes()`. Es gibt KEIN `getAppCacheBytes`/`getDocumentsBytes`. `getObbBytes()` ist `{@hide}` (nicht verwenden).

Berechnung (installd `getExternalSizesForUserWithQuota`, Nicht-sdcardfs):
- `totalBytes` = Default-Projekt (alle Nicht-Medien-Dateien ohne App-Verzeichnisse) + Audio + Video + Image + Σ aller Apps (`Android/data` + `Android/data/*/cache`); Kommentar im Code: „excludes OBBs (Android/obb), but includes app data + cache“. Javadoc: „Any OBB data shared between users is not accounted in this value.“
- `appBytes` = Σ über alle `appIds` von ext-data + ext-cache = der gesamte `Android/data`-Platz ALLER Apps (inkl. eigener App). Javadoc: „already accounted against individual apps as returned through StorageStats“.
- `appIds` stammen aus `getInstalledApplicationsAsUser(MATCH_UNINSTALLED_PACKAGES)`, schließen also deinstallierte Apps mit behaltenen Daten ein.

### 1.4 Welche Felder enthalten fremdes `Android/data` / `Android/obb` (primärer Speicher)

| Wert | Android/data fremder Apps | Android/obb fremder Apps |
|---|---|---|
| `ExternalStorageStats.getAppBytes()` | ja (Summe, inkl. Cache) | nein |
| `ExternalStorageStats.getTotalBytes()` | ja (zusammen mit Rest des Speichers) | nein |
| `queryStatsForPackage/Uid(...).getDataBytes()` | ja, je App (gemischt mit internem Data) | nein |
| `queryStatsForPackage/Uid(...).getAppBytes()` | nein | ja, je App (gemischt mit APK/Dex/Libs; trennbar nur grob über `getAppBytesByDataType` ab API 35 bzw. APK-Größen via `ApplicationInfo.sourceDir`) |
| `queryStatsForUser(...).getAppBytes()` | nein | ja (Summe, gemischt mit /data/app) |
| `queryStatsForUser(...).getDataBytes()` | ja (gesamter freigegebener Speicher außer OBB + interne App-Daten) | nein |

Folgerung (abgeleitet): `unseen_Android_data ≈ ExternalStorageStats.getAppBytes() − (vom Walker gesehene eigene Android/data/<eigenes Paket>)`. Quota liefert belegten Plattenplatz (`dqb_curspace`, Blöcke), daher Walker-Summe mit `st_blocks*512` bilden, nicht `st_size`. OBB-Gesamtsumme ist öffentlich nicht isolierbar (Rest/Näherung).

### 1.5 Kosten

Quota-basiert: `quotactl(Q_GETQUOTA, PRJQUOTA/GRPQUOTA/USRQUOTA)` je ID, O(1); `IsQuotaSupported` nur wenn /data-Blockgerät Quota liefert, sonst `fts`-Vollwalk als system (langsam). `queryExternalStatsForUser` ≈ wenige Hundert `quotactl` (je App 3–5). Keine Messung gefunden; Javadoc warnt trotzdem „several seconds“ → Worker-Thread, nur einmal je Analyse.

### 1.6 Usage access: prüfen und anfordern

- Manifest: `<uses-permission android:name="android.permission.PACKAGE_USAGE_STATS" tools:ignore="ProtectedPermissions"/>`; Schutzstufe `signature|privileged|development|appop|retailDemo`. Ohne Deklaration fehlt die App in der Settings-Liste (`permissionDeclared`).
- Prüfung spiegelt `StorageStatsService.checkStatsPermission`: `AppOpsManager.OPSTR_GET_USAGE_STATS` (`"android:get_usage_stats"`, API 21); `unsafeCheckOpNoThrow(op, uid, pkg)` (API 29, in API 36 deprecated → `checkOpNoThrow(String,int,String)` bzw. 4-Arg mit `attributionTag`, ab API 36). Modus `MODE_ALLOWED` ⇒ ok; `MODE_DEFAULT` ⇒ `checkCallingOrSelfPermission(PACKAGE_USAGE_STATS)==GRANTED`; sonst verweigert.
- Anfordern: `Settings.ACTION_USAGE_ACCESS_SETTINGS` (`android.settings.USAGE_ACCESS_SETTINGS`, API 21; „a matching Activity may not exist“). AOSP-Settings hat zusätzlich einen Filter mit `<data android:scheme="package"/>` ⇒ `Intent(ACTION_USAGE_ACCESS_SETTINGS, Uri.parse("package:"+pkg))` öffnet die App-Seite (nicht öffentlich dokumentiert; Fallback allgemein).
- Änderung beobachten: `AppOpsManager.startWatchingMode(op, pkg, listener)` (API 19; nur eigene UID) oder in `onResume` neu prüfen.

### 1.7 „Restricted settings“ für sideloaded Apps

- Android 13–14 (AOSP-Code): `UsageAccessDetails` ohne ECM-Prüfung ⇒ Usage access NICHT eingeschränkt (Restricted Settings dort nur Accessibility, Notification listener; sekundär bestätigt).
- Ab Android 15 (`android15-release`, 16, `main`; CDD 15/16/17 §9.8 „[9.8/H-0-1] MUST implement Restricted Settings … Usage access (`AppOpsManager.OPSTR_GET_USAGE_STATS`)“): Liste Accessibility, Notification listener, Device admin (`BIND_DEVICE_ADMIN`), Display over other apps, Usage access; Rollen Dialer, SMS; Runtime SMS. Settings: `mSwitchPref.checkEcmRestrictionAndSetDisabled(OPSTR_GET_USAGE_STATS, pkg)`; `EnhancedConfirmationService.PROTECTED_SETTINGS` enthält `OPSTR_GET_USAGE_STATS` und `OPSTR_LOADER_USAGE_STATS`.
- „Guarded“ (`isPackageEcmGuarded`): nicht vorinstalliert/allowlisted UND (`InstallSourceInfo.getPackageSource()` = `PACKAGE_SOURCE_LOCAL_FILE` oder `PACKAGE_SOURCE_DOWNLOADED_FILE`, oder Installer nicht vorinstalliert/allowlisted, falls die Allowlist nicht leer ist). Aufhebung nur durch den Nutzer: App-Info → ⋮ → „Eingeschränkte Einstellungen zulassen“ (Menü `ACCESS_RESTRICTED_SETTINGS`), danach erst Usage access schaltbar. `EnhancedConfirmationManager` ist SystemApi (nicht aufrufbar); Hinweis-UI nötig: Settings öffnen, Nutzer führen. Prüfhilfe: `PackageManager.getInstallSourceInfo(pkg)` (API 30, `getPackageSource()` API 33). Offen: ob Selbst-Update per PackageInstaller-Session den Status ändert.
- Zusätzlich: `MANAGE_EXTERNAL_STORAGE` und `REQUEST_INSTALL_PACKAGES` stehen in CDD-15-Liste der Special permissions (STRONGLY RECOMMENDED für ECM-Anbindung).

### 1.8 Volumes

`StorageManager.getUuidForPath(File)` (26): `UUID_DEFAULT` für Pfade unter `Environment.getDataDirectory()`, sonst Volume ohne `TYPE_PUBLIC`/`TYPE_STUB`; „Throws IOException when … it doesn't have a valid UUID“. Primär-emuliert ⇒ `UUID_DEFAULT` (`41217664-9172-527a-b3d5-edabb50a7d69`). Portable SD-Karte (public, exFAT/FAT): `getUuidForPath` wirft `FileNotFoundException`; `StorageVolume.getStorageUuid()` liefert kodierte FAT-UUID; `getTotalBytes` geht (`vol.disk.size` gerundet), `queryExternalStatsForUser` scheitert abgeleitet (kein Quota, kein `/mnt/expand/<uuid>/media/<user>`). Adoptierte SD: Private Volume mit UUID, Stats möglich. Für SD gilt also nur Rest = `statvfs`-belegt − gescannt.

### 1.9 Pakete aufzählen

`QUERY_ALL_PACKAGES` (API 30, „Allows query of any normal app on the device, regardless of manifest declarations. Protection level: normal“) – bei Sideload kein Review nötig (Play-Policy irrelevant). `PackageManager.getInstalledApplications(PackageManager.ApplicationInfoFlags)` (API 33; `int`-Variante bis 32), `getInstalledPackages(PackageInfoFlags)` (33). `MATCH_UNINSTALLED_PACKAGES` (API 24, braucht `QUERY_ALL_PACKAGES`) liefert deinstallierte Apps mit Daten, `MATCH_ARCHIVED_PACKAGES` (35) archivierte. `ApplicationInfo.storageUuid` (26) als `storageUuid`-Argument. Label/Icon: `PackageManager.getApplicationLabel(ApplicationInfo)`, `getApplicationIcon(ApplicationInfo)` (Drawable ≤ 2048×2048). `ApplicationInfo.category` (26; `CATEGORY_GAME` …). Ab API 31 mit `MANAGE_EXTERNAL_STORAGE`+`QUERY_ALL_PACKAGES`: `StorageManager.getManageSpaceActivityIntent(pkg, requestCode)` zum „Speicher verwalten“ der Fremd-App (manage-all-files-Doku).

## 2. Total/Free und Settings-Kategorien

- `getTotalBytes(UUID_DEFAULT)`: Marketinggröße (`getPrimaryStorageSize()` gerundet, bei > 512 GB Blockgerät mit leichtem Runden) – nicht die /data-Größe. Javadoc: „Apps making logical decisions about disk space should always use `File.getTotalSpace()`“.
- `getFreeBytes`: Code: `path.getUsableSpace() + cacheClearable` (Quota-Volumes). Javadoc: „designed to reflect both unused space and cached space that could be reclaimed“.
- `statvfs`/`File.getTotalSpace()` auf `/storage/emulated/0`: `pf_statfs` im Daemon ruft `statvfs()` auf dem Lower-FS-Root ⇒ Werte der /data-Partition (abgeleitet). „Belegt auf /data“ = `f_blocks−f_bfree`.
- Settings (`StorageAsyncLoader`, `StorageItemPreferenceController`): Apps/Spiele = je installierter App `queryStatsForPackage` (`data + code`, Cache auf Quota gedeckelt, Doppel-Code abgezogen); Bilder/Videos/Audio/Dokumente/Sonstige/Papierkorb = `MediaStore`-Abfragen `sum(_size)` (`VOLUME_EXTERNAL_PRIMARY`, `MEDIA_TYPE`, `QUERY_ARG_MATCH_TRASHED`); System = `getTotalBytes(UUID_DEFAULT) − Environment.getDataDirectory().getTotalSpace()`; „Temporäre Dateien“ = `max(1 GiB, used − attributed)`; used = `getTotalBytes − getFreeBytes`.

## 3. MediaProvider-FUSE und Android/data, Android/obb (App mit MANAGE_EXTERNAL_STORAGE)

Doku: „Apps that are granted this permission still can't access the app-specific directories that belong to other apps, because these directories appear as subdirectories of `Android/data/`“ (manage-all-files); Android 11: „apps can no longer access files in any other app's dedicated, app-specific directory within external storage“.

Mechanismus (Code):
1. Mountmodus: `StorageManagerService.getMountModeInternal` vergibt `MOUNT_MODE_EXTERNAL_DEFAULT` an normale Apps – auch mit MANAGE_EXTERNAL_STORAGE; `PASS_THROUGH` nur MediaProvider-UID, `ANDROID_WRITABLE` nur DownloadProvider/ExternalStorageProvider/MTP-Plattform, `INSTALLER` bei `INSTALL_PACKAGES` oder erlaubtem Op `REQUEST_INSTALL_PACKAGES` (Zugriff nur auf `Android/obb`, laut `isMountModeAllowedPrivatePathAccess`; im Daemon Teilpfad mit Vorbehalt: `is_app_accessible_path` lehnt ohne fuse-bpf Zugriffe auf `/storage/emulated/.../Android/(data|obb)/<pkg>` ab – Verhalten für Installer-Apps nicht verifiziert, **offen**).
2. Zygote `BindMountStorageDirs`: „Mount tmpfs on Android/data and Android/obb, then bind mount all app visible package directories“. Folge für DEFAULT-Apps (abgeleitet): `readdir(Android/data)` und `readdir(Android/obb)` zeigen nur das eigene Paket (falls Verzeichnis existiert), `lstat/opendir` fremder `Android/data/<pkg>` ⇒ `ENOENT`. Aktiv wenn `persist.sys.vold_app_data_isolation_enabled`; Code-Default false, aber CTS-Commit df20d24: „As the feature is already enabled by default, we no longer need to check this.“
3. FUSE-Rückfall (`is_app_accessible_path`, Pfad passt auf `^/storage/[^/]+/(?:[0-9]+/)?Android/(?:data|obb)/([^/]+)(/?.*)?`): nicht erlaubt ⇒ `ENOENT` bei lookup (Parent-Prüfung), getattr, opendir, readdir; `.nomedia` ausgenommen. `onFileOpenForFuse`: „Can't open a file in another app's external directory!“ ⇒ `ENOENT`. `EACCES` kommt nur bei Cross-User-Lookups (`is_user_accessible_path`), Verzeichnisfehlern oder Schreib-/Erzeugungsprüfungen. Das Verzeichnis `Android/data` selbst passt nicht auf das Muster; `getFilesInDirectoryForFuse` liefert `{""}` (Fehler) für Nicht-Manager, `{"/"}` (Lower-FS-Listing) bei `shouldBypassFuseRestrictions` (Manager) – wegen Punkt 2 greift das für DEFAULT-Apps praktisch nicht.
4. `Android/media/<pkg>` ist nicht privat (Regex ohne `media`) und für MANAGE_EXTERNAL_STORAGE sichtbar/scannbar.
5. Lower-FS unzugänglich: `init.rc`: `mkdir /data/media 0550 media_rw media_rw`, `/mnt/pass_through 0700 root root`, `/mnt/pass_through/0 0710 root media_rw`. SELinux erlaubt `untrusted_app` `media_rw_data_file`, die Sperre ist DAC/Mount-Namespace.
6. MediaStore indexiert `Android/data`, `Android/obb`, `Android/sandbox` nie (`PATTERN_INVISIBLE`, `ModernMediaScanner.shouldScanDirectory`).

Praktisch: Android/data/obb fremder Apps nur über StorageStats (Abschnitt 1) bilanzieren; Walker darf `ENOENT` dort als „nicht vorhanden/unsichtbar“, nicht als Fehler melden.

## 4. FUSE-Walk: Kosten, d_type, Parallelität

- Dokumentiert (scoped): „FUSE … introduces performance regressions … especially visible in mid and low-end devices“ (Kontextwechsel Kernel→Daemon); Maßnahmen u. a. „Optimizations for apps with All Files access to make bulk operations faster“ und „Caching permissions to reduce IPCs“; Bulk-Apps „listing or removing a directory with a 1000 files … may be impacted“.
- Passthrough (Android 12, `persist.sys.fuse.passthrough.enable`; Kernel `android12-5.4/5.10`; Upgrade-Geräte von 11 ohne): „read/write … directly forwarded to the lower file system“ – nur Datei-Lesen/Schreiben nach `open`, nicht lookup/getattr/readdir. Daemon öffnet bei Passthrough immer über FUSE; der `open` bleibt ein Upcall (JNI `onFileOpenForFuse`).
- fuse-bpf: GKI `CONFIG_FUSE_BPF=y` ab `android13-5.15`; Daemon aktiviert es, wenn `/sys/fs/fuse/features/fuse_bpf`=„supported“ (Properties `ro.fuse.bpf.is_running`/`persist.sys.fuse.bpf.override`/`ro.fuse.bpf.enabled` überschreiben). Kommentar `FuseDaemon.cpp`: „Currently FUSE BPF is limited to the Android/data and Android/obb directories.“ (`PATTERN_BPF_BACKING_PATH`; Programm `fuseMedia.c`: Standard `FUSE_BPF_BACKING`, Postfilter für lookup/readdir). Für den Walk über freigegebenen Speicher: **kein** Vorteil.
- Per Verzeichnis: lookup (gecacht), `pf_opendir` ⇒ JNI `IsOpendirAllowed`, erster readdir ⇒ JNI `getFilesInDirectoryForFuse`; für Manager `{"/"}` ⇒ Listing aus Lower-FS (`addDirectoryEntriesFromLowerFs`), kein DB-Zugriff. Viele kleine Verzeichnisse sind daher teurer als wenige große.
- readdirplus immer: `pf_init` löscht `FUSE_CAP_READDIRPLUS_AUTO` („We don't want a getattr request with every read request“; Code: „readdir_plus enabled without adaptive readdir_plus“). Jede Zeile ⇒ `do_lookup` ⇒ `lstat` im Daemon + Node-Anlage; Attribute werden mitgeliefert (`attr_timeout`/`entry_timeout` = unendlich, außer private Pfade/Uncached-Pfade/externally-managed Volumes) ⇒ anschließendes `fstatat(AT_SYMLINK_NOFOLLOW)` auf Kinder wird aus dem Kernel-Cache bedient (abgeleitet), kein Upcall.
- Puffer: Kernel `fuse_readdir_uncached` fordert pro Anfrage `PAGE_SIZE` (1 Seite; identisch in android12-5.10 … android16-6.12; Daemon `READDIR_BUF 32768`, begrenzt aber auf die angeforderte Größe; kein `FOPEN_CACHE_DIR`). Ein `fuse_direntplus` ≈ 152 B + Name ⇒ ≈ 24 Einträge je Upcall bei 4 KiB-Seiten, ≈ 97 bei 16 KiB (abgeleitet). 1 Mio. Einträge ≈ 40 000 readdirplus-Upcalls.
- `d_type`: gefüllt (libfuse `fuse_add_direntry_plus` setzt den Typ aus `st_mode` des Lookups); Verzeichnisse aus Lower-FS-`d_type`. Walker kann `d_type` nutzen und nur für Dateigrößen `fstatat` aufrufen.
- Parallelität: `fuse_session_loop_mt` mit `clone_fd=1`, `max_idle_threads=10`. libfuse je Branch: android12 3.8.0, android13–15 3.10.5 (kein `max_threads`, Threads nach Bedarf), android16 3.17.0 (`FUSE_LOOP_MT_DEF_MAX_THREADS 10` ⇒ max. 10 Worker je Volume-Daemon; Modul-Update kann abweichen). Alle Node-Baum-Operationen (`Create`, `LookupChildByName`, `BuildPath`, `Release`) laufen unter EINEM `std::recursive_mutex` je Volume ⇒ Skalierung begrenzt (abgeleitet). Empfehlung: Walker-Threads an gemessene Dateien/s adaptiv anpassen (Start 2–4, nicht > 8), Rest offen.
- `max_read` = `256 * getpagesize()` = 1 MiB (4 KiB-Seiten) bzw. 4 MiB (16 KiB-Seiten); `FUSE_CAP_ASYNC_READ`, Splice-Read an; `FAdviser` ruft `POSIX_FADV_DONTNEED` auf Lower-FS-Handles (Doppel-Caching vermeiden).
- Messungen: keine belastbare veröffentlichte Messung für readdir/stat-Walks mit MANAGE_EXTERNAL_STORAGE gefunden. Sekundär: Termux-Issue #2174 (Pixel 4a, Android 11, Legacy-App ohne Manager): `rm -rv` von 5787 Dateien 5m22s vs 1,6 s intern – betrifft Schreib-/DB-Pfad, nicht Lesen.
- `requestRawExternalStorageAccess` (R.attr, API 31): „all file path access on external storage will bypass database operations that update MediaStore“; Default true nur bei MANAGE_EXTERNAL_STORAGE und targetSdk ≤ 30 – betrifft Schreibvorgänge (Löschen von Duplikaten), nicht den reinen Walk; bei true danach `MediaScannerConnection.scanFile` nötig.

## 5. MediaStore als Größenindex (kurz)

`MediaStore.Files.getContentUri(VOLUME_EXTERNAL_PRIMARY)` (API 11/29) mit MANAGE_EXTERNAL_STORAGE: „Access to the contents of the MediaStore.Files table“. Spalten `_data` (read-only ab R, aber lesbar), `_size`, `relative_path`, `date_modified`, `volume_name`, `is_pending`, `is_trashed` (30); Standard filtert Pending/Trashed weg (`QUERY_ARG_MATCH_PENDING/TRASHED`, 30). Enthält auch Nicht-Medien und Verzeichnisse (mime_type NULL, so nutzt Settings `MIME_TYPE IS NOT NULL`), NICHT `Android/data|obb|sandbox` (siehe 3.6). Aktualität: Scanner/Event-getrieben, Manager-Schreibzugriffe mit Raw-Access landen nicht sofort in der DB; Änderungserkennung `MediaStore.getVersion(ctx, volume)` und `getGeneration(ctx, volume)` + `generation_modified` (30). Kosten: `config_cursorWindowSize` = 2048 KB je CursorWindow ⇒ bei ~100 B/Zeile ≈ 20 000 Zeilen je Binder-Rundlauf (abgeleitet); keine Messung für 1 Mio. Zeilen. Fazit: nur als Beschleuniger/Plausibilitätsprüfung (Summen je `media_type`), nicht als vollständige Wahrheit; Walk bleibt maßgeblich.

## 6. Hashing und Wärme/Akku

- SHA-256 hardware: ARMv8 Crypto Extension (optional); Erkennung `getauxval(AT_HWCAP) & HWCAP_SHA2` (RustCrypto `cpufeatures` unterstützt Android; `sha2`-Crate: „use `aarch64-sha2` … when the required target features are detected at runtime; otherwise fall back to `soft`“). Nicht jedes Gerät hat sie (Android-Doku Adiantum: „devices … whose CPUs lack AES instructions“) ⇒ Laufzeiterkennung nötig.
- BLAKE3: Crate 1.8.7, `build.rs` aktiviert NEON (C-Intrinsics, C-Compiler nötig) automatisch auf little-endian aarch64; kein Laufzeit-Detect.
- Zahlen (sekundär, indirekt, gleiche Taktklasse): Cortex-A53 @1,2 GHz SHA-256 mit SHA2-Extension 638 MB/s (minio/sha256-simd, Pine64) vs. BLAKE3-NEON auf RPi3 (A53 @1,2 GHz) 203→267 MB/s, RPi4 aarch64 348→383 MB/s, M1 Performance-Kern ≈ 1,5 GB/s, M1 Effizienzkern ≈ 0,4 GB/s (BLAKE3 PR #319). Daraus: MIT Crypto Extension ist SHA-256 einthreadig ≥ BLAKE3-NEON (≈ 2,4× auf A53); OHNE sie gewinnt BLAKE3 deutlich. Kernel-Patch (LKML 2025-02): 2-Nachrichten-Interleave steigert SHA-256-CE-Durchsatz auf A76 +65 %, X3 +68 %, A55 +8 % ⇒ große Kerne latenzgebunden. Empfehlung (abgeleitet): `sha2` bei HWCAP_SHA2, sonst `blake3`; Hash-Sicherheit für Duplikate: erst Größe, dann Teil-Hash (Kopf/Mitte/Ende), dann Voll-Hash.
- Lese-I/O: Daemon `max_read` 1 MiB; Kernel-Readahead-Fenster standardmäßig klein (gerätespezifisch, nicht ermittelt) ⇒ Puffer 256 KiB–1 MiB, sequenziell, kein `mmap`; mit Passthrough direkt Lower-FS-Lesen. Wert nicht gemessen.
- Thermik: `PowerManager.addThermalStatusListener(Executor, OnThermalStatusChangedListener)` und `getCurrentThermalStatus()` (API 29; `THERMAL_STATUS_NONE=0, LIGHT, MODERATE, SEVERE=3, CRITICAL, EMERGENCY, SHUTDOWN=6`), `getThermalHeadroom(int forecastSeconds)` (API 30; 0–60 s; 1.0 = SEVERE; „no benefit to calling this function more frequently than about once per second“, ADPF: NaN wenn öfter als alle 10 s oder nicht unterstützt), `getThermalHeadroomThresholds()` (35, kann `UnsupportedOperationException`), `addThermalHeadroomListener` (36). ADPF: Statuswert allein nicht verlassen, mit Headroom validieren.
- Akku/Hintergrund: `isPowerSaveMode()` (21, `ACTION_POWER_SAVE_MODE_CHANGED`: „applications should reduce their functionality“), `isDeviceIdleMode()` (23), `isIgnoringBatteryOptimizations(pkg)` (23). Lange Arbeit: Vordergrunddienst; `dataSync`/`mediaProcessing` bei targetSdk ≥ 35: 6 h je 24 h, danach `Service.onTimeout(int,int)` und `stopSelf()` nötig, sonst `RemoteServiceException`; Timer wird zurückgesetzt, wenn der Nutzer die App öffnet (FGS-Timeout-Doku). Android 16: JobScheduler-Quota-Optimierungen (Jobs parallel zu FGS unterliegen Job-Laufzeitquota; behavior-changes-16). Details FGS-Typen: `android-platform.md`.
- Vorschlag Drosselung (abgeleitet): Threads reduzieren bei `THERMAL_STATUS_LIGHT`/Headroom > 0,8, pausieren ab `MODERATE`/`isPowerSaveMode()`; Hash-Phase nur bei Ladegerät optional anbieten.

## Offene Punkte

1. Installer-Mountmodus (`REQUEST_INSTALL_PACKAGES`-Op erlaubt): Sichtbarkeit von `Android/obb` fremder Apps im Walk auf echtem Gerät prüfen (Code uneindeutig, tmpfs-Overlay entfällt laut `ProcessList.needsStorageDataIsolation`).
2. Auf Gerät verifizieren: `readdir/lstat` von `Android/data` und `Android/data/<fremd>` für Manager-App (erwartet nur eigenes Paket bzw. `ENOENT`), je Android 11–17 und Hersteller (Isolation-Property).
3. Laufzeit von `queryExternalStatsForUser`/`queryStatsForPackage` (~200 Apps) messen; Quota-Unterstützung (`isQuotaSupported` ist `StorageStatsManager`-intern, öffentlich nur indirekt erkennbar).
4. Walk-Durchsatz (Einträge/s, Threads 1/2/4/8, 4K vs 16K-Seiten, mit/ohne Passthrough) messen; die `max_threads`-Grenze 10 gilt nur bei libfuse ≥ 3.12 (MediaProvider-Modulstand unbekannt).
5. Ob Selbst-Update per PackageInstaller den „guarded“-Status (Restricted Settings) aufhebt; OEM-Abweichungen (z. B. fehlendes `package:`-Intent).
6. `getObbBytes` (hidden) nicht nutzbar; OBB-Summe nur als Rest/Näherung (`getAppBytes` − APK/Lib/Dexopt, API 35 `getAppBytesByDataType`).
7. Portable SD-Karten: keine Stats-API; Rest-Rechnung per `statvfs`. Verhalten von `Android/data` auf SD (FUSE ohne Quota) nicht geprüft.
8. BLAKE3 vs. SHA-256 auf echten Phone-SoCs (A55/A76/X-Kerne) nicht direkt veröffentlicht; HWCAP-Verbreitung auf Low-End-SoCs unbekannt.
9. Optimale Lesepuffergröße/Readahead je Gerät nicht gemessen.
10. MediaStore-Vollabfrage (~1 Mio. Zeilen) ungemessen; Scanner-Verzögerung nicht quantifiziert.
