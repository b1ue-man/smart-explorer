# Smart Explorer für Android – Umsetzung

Plan-Ordner: `docs/superpowers/plans/2026-09-25-android-apk/` (spec.md · recherche.md · api.md ·
umsetzung.md · review.md). Vertrag Kotlin↔Rust: **api.md** (verbindlich). Kritik und Einarbeitung:
review.md.

## Regeln für alle Blöcke
- AGENTS.md: keine lokalen Builds/Tests/Compiler (`cargo build/check/test/fmt`, `gradle`, `rustc`,
  `java`, `kotlinc`); einzig erlaubt: `sudo -n /root/.cargo/bin/rustfmt --check --edition 2021 < datei.rs`
  (Stdin-Modus; leere Ausgabe = parsebar/formatiert) und `sudo -n /root/.cargo/bin/cargo tree|metadata
  --offline` (reine Auflösung). Keine Commits/Pushes/Installationen durch Agents, keine Sub-Agents.
- Neue/stark geänderte Rust-Dateien < 500 Zeilen und < 50 KiB; `core/` plattformfrei; Plattformcode
  unter `os/`; `Result` statt `unwrap`/`expect` in Produktionspfaden.
- Bestandsschutz: Desktop-Verhalten (Windows/Linux) bleibt unverändert – Locator-Formate,
  Papierkorb/Backup-Semantik, Link-Auslassungen, Verhalten ausgelagerter Logik. Ausgelagerte
  app-Dateien bleiben als Re-Export-Schalen (`pub(in crate::app) use crate::…`), damit
  `app/mod.rs` und alle Aufrufer unverändert bleiben; ihre Tests wandern mit.
- Kein `std::env::set_var`/`remove_var` in neuem Code; Host-Werte nur über `support_dirs::set_host`.
- Test-Präfix für neue Rust-Tests: `android_task_` (einheitlich).
- Jeder API-/Lib-Aufruf gegen die genannten Refs; fehlt etwas Kleines: offizielle Doku nachschlagen und
  die Ref ergänzen; Größeres melden.
- Schnittstellen unten sind Vertrag. Abweichung → im Bericht melden, nicht still ändern.
- Bericht: gelesene Dateien, erstellte/geänderte Dateien, Umsetzung je Abschnitt, Abweichungen mit
  Grund, Self-Review-Befunde, offene Punkte/Schnittstellenmeldungen.

## Blöcke

### B1a Rust-Portierung und Android-Adapter (Welle 1)
- Funktionen: F2; Kern von F11 (App-Papierkorb)
- Dateien (exklusiv): `native/Cargo.toml`, `native/src/lib.rs`, `native/src/support_dirs.rs`,
  `native/src/scanner/mod.rs`, `native/src/folder_index/mod.rs`, `native/src/cloud/**`,
  `native/src/syncjobs/mod.rs`, `native/src/syncjobs/os/linux_os.rs`, `native/src/updater/mod.rs`,
  `native/src/net/mod.rs`, `native/src/net/os/android/**` (neu), `native/src/share/mod.rs`,
  `native/src/share/os/android/**` (neu), `native/src/share/os/shared/identity_store.rs`,
  `native/src/bisync/os/shared/apply_delete.rs`, `native/src/apptrash/**` (neu),
  `native/src/local_access/os/linux_os.rs`, `native/src/vfs/os/linux_os/local_platform.rs`,
  `native/src/copy/os/linux_os.rs`, `native/src/agent_proto/os/linux_os/local_platform.rs`,
  `native/src/sync/**` (Papierkorb-Auslassung beim Spiegeln), `native/src/android_fs/**` (neu:
  Android-Rename-Kette), Platzhalter `native/android-bridge/{Cargo.toml,src/lib.rs}` und
  `native/src/mobile/mod.rs` (kompilierbar, leer; gehen in Welle 2 an B2),
  `native/src/mount/**` (nur falls für Android nötig) sowie jede weitere Nicht-app-Datei, in der
  `std::env::temp_dir()` auf Android erreichbar ist und die keinem anderen Welle-1-Block gehört
  (Liste per `rg` im Bericht). Nicht: `native/src/daemon/**`, `native/src/autostart/**` (B1b),
  Dateien von B4.
- Schnittstellen – bietet (verbindlich):
  ```rust
  // support_dirs.rs (alle Plattformen kompiliert; Host-Werte wirken, sobald gesetzt – der Desktop setzt sie nie)
  pub struct HostConfig { pub data_home: std::path::PathBuf, pub cache_dir: std::path::PathBuf,
      pub home_dir: std::path::PathBuf, pub device_name: String, pub boot_marker: String }
  pub fn set_host(config: HostConfig);                 // einmalig; weitere Aufrufe ignoriert
  pub fn host() -> Option<&'static HostConfig>;
  pub fn temp_dir() -> std::path::PathBuf;             // gesetzt: <cache>/tmp; sonst std::env::temp_dir()
  // apptrash (plattformneutral; wirkt, sobald Volumes gesetzt sind – der Desktop setzt keine)
  pub struct TrashEntry { pub id: String, pub name: String, pub original: std::path::PathBuf,
      pub deleted_ms: i64, pub size: u64, pub is_dir: bool }
  pub const TRASH_DIR_NAME: &str = ".SmartExplorer-Papierkorb";
  pub fn excluded_name(name: &str) -> bool;            // Volumes gesetzt: name == TRASH_DIR_NAME; sonst false
  pub fn set_volumes(volumes: Vec<std::path::PathBuf>);
  pub fn move_to_trash(path: &std::path::Path) -> std::io::Result<TrashEntry>;
  pub fn list() -> std::io::Result<Vec<TrashEntry>>;
  pub fn restore(id: &str) -> std::io::Result<std::path::PathBuf>;  // „Name (2)“ bei belegtem Ziel
  pub fn delete(id: &str) -> std::io::Result<()>;
  pub fn purge_older_than(days: u32) -> std::io::Result<usize>;
  // cloud
  pub fn set_url_opener(opener: Box<dyn Fn(&str) + Send + Sync>);  // Android: open_url → Opener
  ```
  Modul-Deklarationen (nur Deklaration; B4 bzw. B2 füllen die Dateien): in `lib.rs`
  `pub mod apptrash; pub mod transfer; #[cfg(any(target_os = "android", all(unix, test)))] pub mod mobile;
  #[cfg(any(target_os = "android", all(unix, test)))] pub(crate) mod android_fs;`; in `share/mod.rs`
  `#[path = "os/shared/profile_edits.rs"] pub(crate) mod profile_edits;`,
  `…lifecycle_view.rs` → `pub(crate) mod lifecycle_view;`, `…discovery_state.rs` → `discovery_state`,
  `…discovery_retention.rs` → `discovery_retention`, `…poll_status.rs` → `poll_status`,
  `…removal.rs` → `removal`, `…discovery_events.rs` → `discovery_events`; in
  `syncjobs/mod.rs` `#[path = "os/shared/editor.rs"] pub mod editor;`.
- Abschnitte:
  1. Cargo: `eframe`, `egui_extras`, `trash` → `[target.'cfg(not(target_os = "android"))'.dependencies]`;
     `rfd` → `cfg(all(not(windows), not(target_os = "android")))`; `libc` bleibt `not(windows)`;
     `[workspace] members = ["android-bridge"], exclude = ["explorer-command"]` (Wurzelpaket bleibt
     Standardmitglied; `explorer-command` behält eigene Profile). Kontrolle vor dem Workspace-Eintrag:
     `cargo tree --offline --locked --target aarch64-linux-android -e normal` ohne eframe/winit/rfd/trash,
     Host/Windows unverändert (Paketmenge gleich). Der Hauptagent löst den Lock nach Welle 1 statisch
     auf (Platzhalter-Brücke). Refs: `docs/refs/android-rust-deps.md` §4/§6.
  2. `lib.rs`: `app`, `icons`, `cli`, `run_gui` und `install_panic_logger`-Nutzung nur
     `cfg(not(target_os = "android"))` (Panik-Logger als eigenständige pub-Funktion erhalten, die
     die Fassade nutzen kann); Modul-Deklarationen s. o.
  3. Modulauswahlen `scanner`, `folder_index`, `syncjobs::platform`, `updater::os`, `net::platform`
     → `any(target_os = "linux", target_os = "android")`; `net::interfaces` → Android-Datei mit
     `if-addrs`; `share::platform_exec` → Android-Stub (Anbieter nicht verfügbar, `prepare` →
     `Unsupported`); `cloud::os` → Android-Datei (Opener); `local_access::request_access` →
     Android-Text. Refs: Lesungen `portability-*` §1.
  4. `rename_no_replace` (vfs, copy, agent_proto, syncjobs): Linux-Körper bytegleich unter
     `cfg(target_os = "linux")`; auf Android Aufruf von `crate::android_fs::rename_no_replace`:
     `renameat2(RENAME_NOREPLACE)` → bei `EINVAL`/`ENOSYS`/`EOPNOTSUPP` Dateien per Hardlink+Entfernen,
     sonst (und für Verzeichnisse) Existenzprüfung + `rename` (FUSE-Speicher; dokumentiertes
     Restrisiko); ausgelöste Fehlernummern werden protokolliert (Nachweis in G4).
  5. `support_dirs` Host-Werte; `identity_store.rs` nutzt auf Android `host().device_name`/`home_dir`
     statt `HOSTNAME`/`HOME`; Android-erreichbares `std::env::temp_dir()` → `support_dirs::temp_dir()`.
  6. `apptrash` (`<Volume>/.SmartExplorer-Papierkorb/<id>/<name>` + `<id>.json`, Rename auf gleichem
     Volume, nie überschreiben, Purge); `bisync::apply_delete` nutzt auf Android `apptrash`, sonst
     unverändert `trash::delete`; `sync::start_sync` lässt `excluded_name` als geschützte Auslassung
     aus (mit Meldung in `omissions`).
  7. Unit-Tests `android_task_*` (apptrash rundum, `support_dirs` Host-Werte, `excluded_name`,
     rename-Fallback-Entscheidung als reine Funktion).
- Fertig, wenn: alle Auswahlstellen aus den Lesungen haben einen Android-Arm; `cargo tree` wie oben;
  rustfmt-Stdin sauber für jede geänderte Datei.

### B1b Daemon-Einbettung (Welle 1)
- Funktionen: F17-Kern, F18-Voraussetzung (Share-Host im Prozess)
- Dateien (exklusiv): `native/src/daemon/**`, `native/src/autostart/**`.
- Nutzt: `support_dirs::{host, temp_dir}` (B1a-Vertrag).
- Schnittstellen – bietet (verbindlich, über `crate::daemon`, verfügbar für `unix`; Android-Verhalten
  hinter `os/android`, Desktop-Prozess-Daemon unverändert):
  ```rust
  pub struct HostState { pub power_save: bool, pub metered: bool }
  pub fn set_host_state(state: HostState);             // Android: battery_saver_on/on_metered_network
  pub fn ensure_embedded_daemon(timeout: std::time::Duration) -> Result<bool, String>; // Thread starten falls nötig, auf Bereitschaft warten
  pub struct CatchUpStatus { pub finished: bool, pub running_job: Option<String>, pub queued: usize, pub message: Option<String> }
  pub fn request_catch_up() -> Result<u64, String>;    // Nachhol-Lauf beim laufenden Daemon anfordern
  pub fn catch_up_status(id: u64) -> Option<CatchUpStatus>;
  pub fn cancel_catch_up(id: u64);                     // bricht laufende + wartende Jobs dieses Laufs ab
  pub fn active_job() -> Option<String>;
  pub fn drain_share_events_in_process() -> Option<Result<ShareWorkerSnapshot, String>>; // None = kein eingebetteter Daemon
  ```
  Semantik Nachhol-Lauf: Status endet (`finished`), wenn alle für diesen Lauf zugelassenen Jobs fertig
  sind; vom Supervisor abgelehnte (`AlreadyScheduled`, `RecentlyAttempted`) erscheinen mit Grund in
  `message`/einer Liste; `cancel_catch_up` bricht nur Jobs dieses Laufs ab (Abbruch je Job im
  Supervisor neu anlegen).
- Abschnitte:
  1. `daemon/mod.rs`-Auswahlen `ipc_storage`, `mount_process`, `platform` → Android-Arme (POSIX-Dateien
     wiederverwenden; Android-`platform`: Lock-Verzeichnis unter `support_dirs::sync_data_dir()`,
     `battery_saver_on`/`on_metered_network` aus `HostState`, `removable_drives` leer,
     `run_shell_command` mit `/system/bin/sh`).
  2. `run_daemon()` → `run_daemon_with(handoff: Option<Handoff>)` (eigene Datei, Grenze 500 Zeilen
     beachten); Desktop ruft weiter `run_daemon()` (liest Umgebung), Android nie Umgebung.
     `default_home`/Gerätename in `ipc_host.rs` auf Android aus `support_dirs::host()`.
  3. Einbettung: `ensure_embedded_daemon` (ein Thread je Prozess, nie wegen UI gestoppt);
     `ipc_client::ensure_worker_ready`/`restart_worker_for_client` starten auf Android den Thread statt
     eines Prozesses (kein Handoff); `autostart`-Android-Adapter: `is_enabled` = Flag-Datei
     `sync_data_dir()/android-sync-enabled`, `enable`/`disable` schreiben/entfernen sie, Spawn-
     Funktionen → `ensure_embedded_daemon`.
  4. „Beim Start“-Jobs: auf Android nur, wenn `host().boot_marker` von der gespeicherten Marke
     abweicht (dann speichern); Desktop unverändert.
  5. Nachhol-Lauf: im Supervisor-Kontext des laufenden Daemons – fällige Intervall-Jobs, Kalender-
     Termine seit `last_run` (unabhängig von `catch_up`), aktivierte Echtzeit-Jobs einmal; Pause/
     Autopause/Sync-aus beenden den Lauf sofort mit Meldung; Status und Abbruch wie Vertrag.
  6. Unit-Tests `android_task_*` (Nachhol-Auswahl als reine Funktion über Job-Listen und Zeiten,
     Boot-Marken-Entscheidung, Android-`platform`-Zustände).
- Refs: Lesungen `background-daemon-model`, `sync-facade-map` §11, `portability-daemon-sync-mount-updater`.
- Fertig, wenn: Vertrag vollständig; Desktop-Pfade (Prozess-Daemon, Handoff, Autostart) unverändert;
  rustfmt sauber.

### B4 Auslagerung egui-freier Logik aus `app/` (Welle 1)
- Funktionen: Voraussetzung für F7/F8/F9/F12/F15/F16/F18 in der Fassade
- Vorgehen: zuerst die vollständige `use`-Kette jeder Zieldatei per `rg` bestimmen und im Bericht
  auflisten (Kritiker Befund 4: u. a. `downloads.rs` → `temp.rs::{open_temp_path,cleanup_temp_copy}`
  → `platform_helpers::EditProcess`/`recovery_manifest`; `download_file.rs` → `transfer_helpers`;
  `upload_plan.rs`/`upload_stream.rs` → `upload_is_link_like`; `transfer_jobs.rs`/`app_models.rs`
  → app-`prelude` mit `eframe::egui`). Nur Logik verschieben, die ohne egui und ohne `App` auskommt;
  Rest bleibt in app/ und importiert die ausgelagerte Logik.
- Dateien (exklusiv): alle dafür berührten Dateien unter `native/src/app/**`; neu
  `native/src/transfer/**` (Lane, Transfer-Typen, Upload/Download/Remote-Kopie, Staging/Commit,
  Fortschritt, Abbruch, Temp-Pfade für Transfers mit derselben Wurzel wie bisher),
  `native/src/vfs/mod.rs` + `native/src/vfs/os/shared/remote_util.rs` (neu),
  `native/src/syncjobs/os/shared/editor.rs` (neu), `native/src/share/os/shared/{profile_edits,
  lifecycle_view,discovery_state,discovery_retention,poll_status}.rs` (neu, Tests mitnehmen),
  `native/src/connect/mod.rs` + neue Dateien unter `native/src/connect/**` (`RemovedEndpointScope`,
  `CleanupReport`, `cleanup_removed_endpoint_state`, Favoriten-/`dir_sort`-Dateifunktionen),
  `native/src/filter/mod.rs` + `native/src/filter/core/tree.rs` (neu: `result_rows`,
  `selected_files`, `clipboard_snapshot`), `native/src/share/os/shared/{removal,discovery_events}.rs`
  (neu: egui-freier Kern der Entfernen-Kaskade aus `share_removal_ui.rs` und Dispatch/Apply aus
  `share_discovery_events.rs` als freie Funktionen mit expliziten Parametern; App-Methoden rufen sie).
  `upload_is_link_like` als eigener Adapter `transfer/os/{unix,windows}.rs` mit **identischen**
  Funktionskörpern aus `app/os/*/platform.rs` (nicht `local_access`).
- Schnittstellen – bietet (Namen wie heute in app/, neue Pfade, Sichtbarkeit `pub(crate)` oder `pub`):
  `crate::transfer::{TransferLane, TransferRequest, TransferKind, TransferProgress, TransferMsg,
  FinishedTransfer, Admission, launch_transfer, MAX_ACTIVE_TRANSFERS, upload_file,
  upload_paths_progress, upload_pairs_progress, download_paths_progress, copy_remote_paths_progress,
  download_to_id, download_clipboard_snapshot, upload_reader_progress}` (neu:
  `upload_reader_progress(backend: &dyn Backend, reader: &mut dyn std::io::Read, size_hint: Option<u64>,
  dest_dir: &str, name: &str, tx: &crossbeam_channel::Sender<TransferMsg>, cancel: &AtomicBool)
  -> Result<String, String>` – Staging + `CommitMode::Create` + Namensreservierung, liefert den
  Zielnamen); Transfer-Durchläufe (`upload_plan::collect_paths`, `RemoteEntryCollector`) lassen
  `apptrash::excluded_name` als geschützte Auslassung aus; `crate::vfs::remote_util::{ep_join, conflict_rel_name,
  numbered_remote_name, read_text, write_bytes, sig_from, rjoin, find_remote_unique_name}`;
  `crate::syncjobs::editor::{JobEditor, min_to_hm, hm_to_min}` (+ `JobEditor::build_sync_job`,
  `JobEditor::from_job`); `crate::share::profile_edits::merge_user_edits`,
  `crate::share::lifecycle_view::{request_views, authorized_device_views, RequestView,
  AuthorizedDeviceView}`, `crate::share::discovery_state::{DiscoveryUiState, DiscoveryPinDraft}`,
  `crate::share::poll_status::after_successful_snapshot`, `crate::share::removal::*` und
  `crate::share::discovery_events::*` (Namen melden); `crate::connect::{RemovedEndpointScope,
  CleanupReport, cleanup_removed_endpoint_state, favorites_path, load_favorites, save_favorites,
  location_key}`; `crate::filter::tree::{result_rows, selected_files, clipboard_snapshot,
  ClipboardVirtualFile}`. Weicht ein Name ab, genau benennen.
- Abschnitte: 1. Abhängigkeitskette; 2. `transfer` + `remote_util`; 3. Job-Editor; 4. Share-Logik;
  5. Cleanup/Favoriten; 6. Baum/Auswahl; 7. app-Schalen und Tests prüfen (jede verschobene Datei
  rustfmt-sauber; jede app-Schale re-exportiert alles, was app/ vorher nutzte).
- Refs: `docs/lesungen/2026-09-25-android-files-facade-map.md`, `…-sync-facade-map.md`,
  `…-share-facade-map.md`, `…-app-feature-map.md`.
- Fertig, wenn: Vertragsnamen existieren; app/ nutzt nur noch Schalen; keine egui-Abhängigkeit in
  verschobener Logik; Bericht listet die Kette und jede Verschiebung.

### K0 Android-Projekt-Fundament (Welle 1)
- Funktionen: F1, F21-Rahmen, F23-Anzeige
- Dateien (exklusiv): `android/settings.gradle.kts`, `android/build.gradle.kts`,
  `android/gradle.properties`, `android/gradle/libs.versions.toml`,
  `android/gradle/wrapper/gradle-wrapper.properties` (JAR und `gradlew`/`gradlew.bat` liegen schon,
  Prüfsumme verifiziert – nicht überschreiben), `android/.gitignore`, `android/app/build.gradle.kts`,
  `android/app/proguard-rules.pro`, `android/app/src/main/AndroidManifest.xml`,
  `android/app/src/main/res/**`, `…/{SmartExplorerApp,MainActivity}.kt`, `…/core/**`,
  `…/ui/theme/**`, `…/ui/AppRoot.kt`, `…/ui/AppNav.kt`, `…/ui/common/**`, `…/ui/onboarding/**`,
  `…/system/{Permissions,Storage,Notifications}.kt`, `…/prefs/AppPrefs.kt`
  (Paket `app.smartexplorer.android`, Quellpfad `android/app/src/main/java/app/smartexplorer/android/`).
- Versionen (fix, AAR-Metadaten geprüft – `docs/refs/android-gradle-build.md` §8): AGP 8.13.2,
  Gradle 8.13, Kotlin 2.4.20 (+ `org.jetbrains.kotlin.plugin.compose`/`.serialization` 2.4.20),
  Compose BOM **2026.06.01** (material3 1.4.0), activity-compose 1.13.0, **lifecycle 2.10.0**,
  **core-ktx 1.18.0**, work-runtime-ktx 2.12.0, kotlinx-serialization-json 1.11.0,
  kotlinx-coroutines-android 1.11.0, material3-adaptive (BOM, 1.2.0), material-icons-core (BOM);
  Test: junit 4.13.2, androidx.test core/runner/rules 1.7.0, ext:junit 1.3.0, espresso-core/-intents
  3.7.0, uiautomator 2.4.0, work-testing 2.12.0, compose ui-test-junit4/-manifest (BOM).
  compileSdk/targetSdk 36, minSdk 30, JDK 17, `abiFilters` arm64-v8a + x86_64,
  `useLegacyPackaging = true`, `versionName`/`versionCode` aus `../native/Cargo.toml` (Ref §5.4).
  Release-Signatur nur aus `ANDROID_KEYSTORE_FILE`/`ANDROID_KEYSTORE_PASSWORD`/`ANDROID_KEY_ALIAS`/
  `ANDROID_KEY_PASSWORD`. `rustls:rustls-platform-verifier` aus dem lokalen Maven-Verzeichnis der
  Gradle-Property `rustlsVerifierMaven` (fehlt sie: klarer Build-Fehler). Nur APIs aus material3 1.4.0.
- Manifest (fix): Berechtigungen laut Sammelmanifest (`docs/refs/android-apis.md`) inkl.
  `MANAGE_EXTERNAL_STORAGE`, `POST_NOTIFICATIONS`, `FOREGROUND_SERVICE(_DATA_SYNC|_SPECIAL_USE)`,
  `RECEIVE_BOOT_COMPLETED`, `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`, `REQUEST_INSTALL_PACKAGES`,
  `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE`, `CHANGE_WIFI_MULTICAST_STATE`;
  `android:allowBackup="false"`, `android:dataExtractionRules` (alles ausgeschlossen);
  Komponenten: `.SmartExplorerApp`, `.MainActivity` (singleTop, adjustResize, SEND/SEND_MULTIPLE
  `*/*`), `.service.TaskForegroundService` (dataSync), `.service.BackgroundService` (specialUse +
  Subtyp-Property), `.system.BootReceiver` (BOOT_COMPLETED, MY_PACKAGE_REPLACED), WorkManager-
  `SystemForegroundService` (dataSync, merge), `.system.LocalFileProvider`
  (`${applicationId}.localfiles`, nicht exportiert, `grantUriPermissions`), `androidx.core.content.FileProvider`
  (`${applicationId}.files`, nur `cache-path`). Kanäle: `transfers`, `background`, `updates`, `share`.
- Schnittstellen – bietet (verbindlich, Kotlin):
  ```kotlin
  package app.smartexplorer.android.core
  object NativeBridge { external fun init(context: android.content.Context, configJson: String): String
                        external fun call(method: String, argsJson: String): String
                        external fun pollEvents(timeoutMs: Int): String }
  class CoreException(val kind: String, message: String) : Exception(message)
  sealed interface CoreEvent { data class Task(val task: TaskInfo) : CoreEvent; data object Share : CoreEvent
      data class ShareRequest(val count: Int) : CoreEvent; data object Jobs : CoreEvent; data object Edits : CoreEvent
      data class OpenUrl(val url: String) : CoreEvent; data class Error(val action: String, val message: String) : CoreEvent
      data object Volumes : CoreEvent }
  object Core {
      val json: kotlinx.serialization.json.Json              // ignoreUnknownKeys, explicitNulls = false
      val events: kotlinx.coroutines.flow.SharedFlow<CoreEvent>
      val tasks: kotlinx.coroutines.flow.StateFlow<List<TaskInfo>>
      val ready: kotlinx.coroutines.flow.StateFlow<CoreState>  // Starting | Ready | Failed(message)
      fun start(app: android.app.Application)               // kehrt sofort zurück: Bibliothek laden + init auf eigenem Thread, dann Ereignispumpe
      suspend fun call(method: String, args: kotlinx.serialization.json.JsonObject = JsonObject(emptyMap())): kotlinx.serialization.json.JsonElement
      suspend inline fun <reified T> request(method: String, args: JsonObject = JsonObject(emptyMap())): T
      suspend inline fun <reified A, reified T> request(method: String, args: A): T
  }
  // Gemeinsame DTOs (@Serializable, Felder exakt api.md §2): Entry, Crumb, Root, FilterSpec, SortSpec, TaskInfo, TaskError
  package app.smartexplorer.android.ui
  enum class MainTab { Files, Sync, Share, More }
  sealed interface NavRequest {
      data class OpenLocation(val location: String) : NavRequest
      data class SelectTab(val tab: MainTab) : NavRequest
      data object ShowTransfers : NavRequest
      data class OpenJobConflicts(val jobId: String) : NavRequest
      data object ShowShareRequests : NavRequest
      data object ShowUpdate : NavRequest
      data object ShowBackgroundSettings : NavRequest }
  object AppNav { val requests: kotlinx.coroutines.flow.SharedFlow<NavRequest>; fun send(request: NavRequest)
                  fun intentFor(context: android.content.Context, request: NavRequest): android.content.Intent }  // für PendingIntents
  package app.smartexplorer.android.prefs
  object AppPrefs { /* SharedPreferences + StateFlow je Wert: theme ("system|light|dark"), showHidden,
      dirsFirst, compact, thumbnails, bgMode ("off|periodic|persistent"), bgIntervalMin, bgWifiOnly,
      bgChargingOnly, bgBatteryNotLow, autoUpdateCheck, lastUpdateCheckMs, onboardingDone;
      fun init(context), Setter je Wert */ }
  ```
  Ruft (Namen fix, andere Blöcke implementieren): `FilesScreen()` (K1), `SyncScreen()` (K2),
  `ShareScreen()` (K3), `MoreScreen()` (K3), `TaskKeeper.start(app)` und
  `BackgroundController.onAppStart(context)`, `BackgroundController.onUiVisible(context, visible)` (K2),
  `ShareIntentHandler.handle(activity, intent): Boolean`
  (K1), `UpdateChecker.maybeCheck(context)` (K3). `MainActivity` verarbeitet `NavRequest`-Extras aus
  Benachrichtigungen und `openUrl`-Ereignisse (Browser per `startActivity`, `ActivityNotFoundException`).
  `Core.start` übergibt die Init-Konfiguration aus api.md §1 (Volumes aus `StorageManager`,
  `homeDir` = primäres Volume, `bootMarker` = `Settings.Global.BOOT_COUNT` (in android-apis.md
  belegen), `appVersion` aus `PackageInfo`; Debug-Build: Überschreibungen aus
  `filesDir/test-overrides.json`, z. B. `updateFeedUrl`, `startDaemon`). `Core.call` wartet auf
  `Core.ready`. `MainActivity.onStart/onStop` melden die Sichtbarkeit an
  `BackgroundController.onUiVisible(context, visible)` (K2).
- Abschnitte: 1. Gradle-Projekt (Ref android-gradle-build.md §5 + §8); 2. Manifest + res (Themes,
  Strings, Kanäle, `file_paths.xml` mit `cache-path` für `open/`, `share/`, `update/`,
  `data_extraction_rules.xml`, adaptives
  Launcher-Symbol als Vektor); 3. `core/` (Brücke, Core, Ereignispumpe, DTOs, Task-Zustand);
  4. Theme, AppRoot (NavigationBar < 840 dp, NavigationRail ≥ 840 dp über `currentWindowAdaptiveInfo`),
  Einrichtungsseite, gemeinsame Bausteine (Fehlerkarte, Leerzustand, Bestätigungsdialog, Größen-/
  Datumsformat, Snackbar-Host, Ladebalken); 5. Symbole (unten).
- Refs: `docs/refs/android-gradle-build.md`, `docs/refs/compose-material3.md`, `docs/refs/android-apis.md`.

### REL Release-Einbindung und Doku (Welle 1)
- Funktionen: F22-Verteilung (E9)
- Dateien (exklusiv): `.github/workflows/build.yml`, `native/publish-release-local.ps1`,
  `native/release-publication.ps1`, `native/release-version.ps1` (neu, dot-sourcebar),
  `android/build-release-apk.sh` (neu), `android/ndk-version` (neu: `29.0.14206865`), `docs/RELEASING.md`,
  `README.md` (Android-Abschnitte),
  `AGENTS.md` (nur die Liste der erwarteten Release-Assets).
- Inhalt:
  1. `Resolve-ReleasePlan`, `Get-NextPatchVersion`, `Set-NativeVersion` samt Hilfen unverändert nach
     `release-version.ps1`; Wrapper dot-sourct sie (Verhalten identisch).
  2. Job `android-release-apk` (ubuntu, vor `complete-release`, gleiche Bedingung, gleicher
     `complete_release_source_sha`): Plan bestimmen (pwsh + `release-version.ps1`); liegt ein
     committetes Feed-APK mit der Zielversion vor → durchreichen; sonst `Set-NativeVersion` (Bump oder
     Resume) auf den Checkout und `android/build-release-apk.sh` (NDK aus `android/ndk-version`,
     cargo-ndk für `native/android-bridge`, Gradle `assembleRelease`, Signatur aus Secrets); Prüfung
     `versionName`/`versionCode` (`aapt2 dump badging`), Zertifikat gegen `android/release-cert.sha256`
     (`apksigner verify --print-certs`), SHA-256; Artefakt `android-release-apk` (APK +
     `apk-metadata.json` mit `version`, `versionCode`, `sha256`, `certSha256`).
  3. `complete-release` lädt das Artefakt nach `$env:RUNNER_TEMP` und ruft den Wrapper mit
     `-AndroidApkDirectory`; Wrapper prüft (ohne Android-Werkzeuge) SHA-256 der Datei gegen die
     Metadaten, `version` gegen die eigene Version und `certSha256` gegen `android/release-cert.sha256`, stagt
     `release-native/update-feed/smart-explorer-android.apk` + `.sha256` im selben atomaren Ablauf;
     ohne Parameter (lokaler Lauf) ruft er `android/build-release-apk.sh` in WSL/Linux nach Preflight.
  4. Asset-Map (`Get-PublicationReleaseAssetMap`) und Commit-Pfade 18 → 20, alle YAML-Kopien in
     `build.yml` (Prüfblock, `allowed_release_change`, Staging, `windows-gnu-release-e2e`,
     `publish-release`, `files:`, Release-Text); `Assert-PublicationNoUntrackedBuildInputs` prüft auch
     `android`.
  5. Doku: `docs/RELEASING.md` (neuer Job, Secrets, Zertifikat, lokaler Lauf); README-Abschnitte
     „Android: Installieren und Updates“ und „Android: Umfang und Grenzen“ (aus spec.md A:
     Funktionen, Nicht-Ziele, Grenzen inkl. Drive-Anmeldung ungeprüft, Share nur bei offener
     App/Dauerbetrieb); AGENTS.md-Assetliste um Android-APK + Hash.
- Refs: `docs/lesungen/2026-09-25-android-ci-release-integration.md`, `docs/refs/android-ci.md`,
  `docs/refs/android-gradle-build.md`, `docs/refs/rust-jni-022.md` (rustls-platform-verifier-Maven-Pfad per
  `cargo metadata`).
- Fertig, wenn: alle Listen konsistent 20; Wrapper-Stufen unverändert bis auf die APK-Einbindung;
  Skripte `bash -n`-sauber (reine Syntaxprüfung).

### B2 Rust-Fassade Kern und JNI-Brücke (Welle 2)
- Funktionen: F1-Init, F3–F11, F23 (Kern)
- Dateien (exklusiv): `native/src/mobile/**` außer `native/src/mobile/os/shared/domains/**`,
  `native/android-bridge/**` (neu: `Cargo.toml` mit `crate-type = ["cdylib"]`, `jni = "=0.22.4"`,
  `ndk-context = "=0.1.1"`, `rustls-platform-verifier = "=0.7.0"` nur `cfg(target_os = "android")`,
  `smart_explorer = { path = ".." }`, `serde_json`), `native/src/scanner/**`, `native/src/rscan/**`,
  `native/src/zipfs/**`, `native/src/folder_index/**` (je nur soweit nötig, inkl.
  `apptrash::excluded_name` als geschützte Auslassung – auch in `scanner::collect_recursive`, das die
  lokale Kopier-Engine nutzt). Die Platzhalter von B1a (`native/src/mobile/mod.rs`,
  `native/android-bridge/**`) gehen in B2-Besitz über.
- Schnittstellen – bietet B3 (verbindlich):
  ```rust
  // crate::mobile (Re-Exporte in mobile/mod.rs)
  pub(crate) struct Runtime { .. }
  impl Runtime {
      pub(crate) fn get() -> Result<&'static Runtime, ApiError>;
      pub(crate) fn config(&self) -> &HostSettings;          // Init-Konfiguration (api.md §1)
      pub(crate) fn resolve(&self, location: &str) -> Result<(crate::vfs::BackendHandle, String), ApiError>; // gepoolt
      pub(crate) fn emit(&self, event: serde_json::Value);
      pub(crate) fn log_error(&self, action: &str, message: &str);
      pub(crate) fn spawn_task<F>(&self, kind: &str, title: String, work: F) -> String
          where F: FnOnce(&TaskCtx) -> Result<serde_json::Value, ApiError> + Send + 'static;
      pub(crate) fn is_app_internal(location: &str) -> bool; // zip:// und trash://
  }
  pub(crate) struct TaskCtx { .. }
  impl TaskCtx {
      pub(crate) fn cancelled(&self) -> bool;
      pub(crate) fn cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool>;
      pub(crate) fn progress(&self, done_bytes: u64, total_bytes: u64, done_items: u64, total_items: u64);
      pub(crate) fn message(&self, text: &str);
      pub(crate) fn error(&self, path: &str, message: &str);
  }
  pub(crate) struct ApiError { pub kind: &'static str, pub message: String }
  impl ApiError { pub(crate) fn new(kind: &'static str, message: impl Into<String>) -> Self; }
  impl From<std::io::Error> for ApiError { .. }             // kind aus io::ErrorKind
  // B3 besitzt native/src/mobile/os/shared/domains/mod.rs mit:
  pub(crate) fn dispatch(rt: &Runtime, method: &str, args: &serde_json::Value)
      -> Option<Result<serde_json::Value, ApiError>>;    // None = keine Domänenmethode
  pub(crate) fn on_init(rt: &'static Runtime);             // startet z. B. den Share-Poller
  ```
  B2 deklariert in `mobile/mod.rs` `#[path = "os/shared/domains/mod.rs"] mod domains;`, ruft
  `domains::on_init` am Ende von `init` und `domains::dispatch` für alle nicht eigenen Methoden.
- Abschnitte:
  1. Protokoll, Fehler, Ereignis-Warteschlange (gebündelte Task-Snapshots, höchstens 4/s), Task-
     Register, Dispatcher, `init`/`call`/`poll_events` (api.md §1–§3); `init` ruft
     `support_dirs::set_host`, `tempfile::env::override_temp_dir`, `apptrash::set_volumes`,
     `cloud::set_url_opener` (→ `openUrl`), Panik-Logger; bei `startDaemon` Anstoß von
     `daemon::ensure_embedded_daemon` auf einem Hintergrund-Thread (nicht warten);
     `apptrash::purge_older_than(30)` im Hintergrund; `init` kehrt ohne Netz-/Daemon-Warten zurück.
  2. Backend-Pool (ein Backend je Verbindung, `CachingBackend` wie Desktop, Wiederverbinden nur
     für Leseoperationen), `loc.*`, Favoriten im Desktop-Format über `crate::connect`, „Zuletzt“ in
     `<data>/mobile/recent.json`, `fs.list/stat/checkName/mkdir/newFile/rename/conflicts`.
  3. `fs.transfer` (Lane aus `crate::transfer`; lokal→lokal über `copy`; gefilterte relative Kopie
     über `crate::filter::tree`), `fs.delete` (apptrash / Backend-Papierkorb / `remove_entry_controlled`),
     `fs.properties`, `fs.import` (Dateideskriptoren), `fs.extract` (Zielordner eindeutig, kein
     Überschreiben).
  4. `scan.*` (lokal `scanner`, remote `rscan`, `FilterRetention`, Baumzeilen über
     `crate::filter::tree`, Fenster/Revision), `index.*`, `trash.*`,
     `fs.open/fetch/materialize/edits/uploadEdit/discardEdit` (Register `<data>/mobile/edits.json`,
     Ereignis `edits` bei erkannter Änderung beim nächsten `fs.edits`/App-Start).
  5. Brücke `native/android-bridge` (`jni` 0.22.4 `EnvUnowned`-Muster, `catch_unwind`,
     `ndk_context::initialize_android_context`, `rustls_platform_verifier::android::init_with_env`,
     liblog über `__android_log_write`) – Ref `docs/refs/rust-jni-022.md`.
  6. Unit-Tests `android_task_*` (Protokoll/Hülle, Ereignisbündelung, Filter-/Sortier-/Baumfenster,
     App-interne Orte abgelehnt, lokale fs-Methoden in Temp-Verzeichnissen, Edits-Register-Rundlauf).
- Refs: api.md; Lesungen files-facade-map (Rezepte 1–9), core-ops-api (D.1–D.8); rust-jni-022.md.
- Fertig, wenn: jede Methode aus api.md §4.1–§4.5 hat einen Handler; Brücke exportiert drei
  JNI-Funktionen; rustfmt sauber.

### B3 Rust-Fassade Domänen (Welle 2)
- Funktionen: F12–F20, F22 (Kern)
- Dateien (exklusiv): `native/src/mobile/os/shared/domains/**` (inkl. eigener `mod.rs`),
  `native/src/bisync/**` (Auslassung `apptrash::excluded_name` im Walk, sonst nur falls nötig),
  `native/src/analytics/**` (Auslassung in Analyse/Reclaim), `native/src/syncjobs/**`,
  `native/src/share/**`, `native/src/connect/**`, `native/src/updater/**` (nur neue
  plattformneutrale Feed-Hilfe) – jeweils nach Welle 1 (sequentiell zu B1a/B4).
- Abschnitte (Reihenfolge nach Wichtigkeit):
  1. `sync.*` + `bg.*` – Refs: sync-facade-map (Rezepte 1–8); „Jetzt“ = GUI-Pfad (`resolve_endpoint` +
     `bisync::run`, `mark_run`/`record_result`, keine Vorher/Nachher-Befehle); Konfliktkontext je
     Job im Speicher, Probelauf für `sync.checkConflicts`, Baseline-Speicherung wie
     `finish_bisync_conflicts`; Editor über `crate::syncjobs::editor::JobEditor`; `bg.catchUp` über
     B1b-Vertrag; App-interne Orte ablehnen.
  2. `conn.*` + `gdrive.*` – Refs: files-facade-map §1/§9, core-ops-api D.2; Aufräumen über
     `crate::connect::cleanup_removed_endpoint_state` + „Zuletzt“; `conn.forgetHostKey` entfernt genau den
     `host:port`-Eintrag aus `known_hosts_sftp.txt` (Sperrdatei wie `known_hosts.rs`).
  3. `share.*` (api.md §5) – Refs: share-facade-map Rezepte 1–9; Entfernen/Discovery über die von B4
     ausgelagerten `crate::share::{removal, discovery_events}`; Poller auf eigenem Thread ab `on_init`
     über `daemon::drain_share_events_in_process` (Takt 300 ms bei `share.watch`, 5 s Vordergrund,
     60 s Hintergrund), Snapshot, Ereignisse `share`/`shareRequest`.
  4. `analyze.*` + `reclaim.*` – Refs: core-ops-api D.6, app-feature-map.
  5. `update.*` – Feed aus `native/update_source.txt` (per `include_str!`) oder `updateFeedUrl`,
     `version.txt` + `smart-explorer-android.apk.sha256`, Download in den Cache, SHA-256 (`sha2`).
  6. Unit-Tests `android_task_*` (Job-JSON-Rundlauf, Editor-Validierung, Optionen, Update-Feed-
     Parsing, Analyse-Knoten, Share-Status-Abbildung aus einem `ShareWorkerSnapshot`).
- Fertig, wenn: jede Methode aus api.md §4.6–§4.10 und §5 hat einen Handler; rustfmt sauber.

### K1 Dateien-UI (Welle 2)
- Funktionen: F3–F11
- Dateien (exklusiv): `…/ui/files/**`, `…/ui/picker/**`, `…/ui/trash/**`, `…/ui/transfers/**`,
  `…/api/FilesApi.kt`, `…/system/{Opener,ShareIntentHandler,Thumbnails,LocalFileProvider}.kt`.
- Refs: `docs/refs/compose-material3.md`, `docs/refs/android-apis.md` §6 (Teilen/Empfangen, FileProvider),
  §9 (Thumbnails, Zwischenablage); `ContentProvider.openFile`/`query`/`getType` für
  `LocalFileProvider` (kleine Doku-Lücke selbst nachschlagen und in android-apis.md ergänzen); api.md §4.2–§4.5.
- Vorgaben: Öffnen/Teilen über `startActivity` mit `ActivityNotFoundException`-Behandlung (kein
  `resolveActivity`); Empfangen übergibt Deskriptoren (`detachFd`); `scan.view` in Fenstern
  (Seiten zu 300 Zeilen, Nachladen beim Scrollen, während des Scans ≤ 1/s).
- Bietet: `@Composable fun FilesScreen()`, `@Composable fun LocationPickerDialog(title: String,
  initialLocation: String?, confirmLabel: String, allowAppInternal: Boolean = false,
  onPick: (String) -> Unit, onDismiss: () -> Unit)`, `@Composable fun TransfersSheet(onDismiss: () -> Unit)`,
  `@Composable fun TrashScreen(onBack: () -> Unit)`,
  `object ShareIntentHandler { fun handle(activity: androidx.activity.ComponentActivity, intent: android.content.Intent): Boolean }`.

### K2 Sync, Hintergrund, Dienste (Welle 2)
- Funktionen: F14–F17, Dienstseite von F8/F13/F18
- Dateien (exklusiv): `…/ui/sync/**`, `…/api/SyncApi.kt`, `…/work/**`, `…/service/**`,
  `…/system/{BootReceiver,HostMonitor}.kt`, `…/ui/settings/BackgroundSettings.kt`.
- Refs: `docs/refs/android-apis.md` §1–§4, §8; `docs/refs/android-platform.md` §1–§2;
  `docs/refs/compose-material3.md`; api.md §4.7.
- Vorgaben: `BackgroundController.onAppStart` ruft `bg.ensureDaemon`, setzt WorkManager nach
  `AppPrefs.bgMode` (UPDATE-Policy) und startet/stoppt `BackgroundService` (specialUse);
  `SyncWorker` sendet zuerst `sys.hostState` (synchron gemessen), ruft dann `bg.catchUp` und wartet auf
  das Task-Ende, `setForeground` in try/catch, bei Stopp `task.cancel`; `TaskKeeper` startet
  `TaskForegroundService` nur bei sichtbarer UI, wenn Tasks (außer `kind == catchup`) laufen, und beim
  Verlassen der UI (`onUiVisible(false)`), wenn Tasks laufen oder `bg.status.activeJob` gesetzt ist –
  jeder Start in try/catch (`ForegroundServiceStartNotAllowedException`); der Dienst beendet sich, wenn
  weder Tasks noch `activeJob` laufen (Prüfung alle 5 s); `onTimeout` → `task.cancelAll` + `stopSelf`; `BootReceiver` startet nur `BackgroundService`
  (Dauerbetrieb) und plant periodische Arbeit; `HostMonitor` meldet Energiesparmodus/getaktetes Netz
  (`sys.hostState`) und hält einen Multicast-Lock, solange Share online ist; `BackgroundService`
  zeigt bei `shareRequest` eine Benachrichtigung (`AppNav.intentFor(ShowShareRequests)`).
- Bietet: `@Composable fun SyncScreen()`, `@Composable fun BackgroundSettingsSection()`,
  `object TaskKeeper { fun start(app: android.app.Application) }`,
  `object BackgroundController { fun onAppStart(context: android.content.Context); fun onUiVisible(context: android.content.Context, visible: Boolean); fun apply(context: android.content.Context) }`,
  Klassen `SyncWorker`, `BackgroundService`, `TaskForegroundService`, `BootReceiver`, `HostMonitor`.
- Nutzt: `LocationPickerDialog` (K1).

### K3 Teilen, Mehr, Verbindungen, Analyse, Einstellungen, Update (Welle 2)
- Funktionen: F12, F13, F18–F23
- Dateien (exklusiv): `…/ui/share/**`, `…/ui/more/**`, `…/ui/connections/**`, `…/ui/analytics/**`,
  `…/ui/settings/**` außer `BackgroundSettings.kt`, `…/api/{ShareApi,ConnApi,AnalyzeApi,UpdateApi}.kt`,
  `…/update/**`.
- Refs: `docs/refs/compose-material3.md` (Canvas für die Treemap), `docs/refs/android-apis.md` §7
  (APK-Installation über FileProvider auf die Cache-Datei), api.md §4.6, §4.9, §4.10, §5.
- Bietet: `@Composable fun ShareScreen()`, `@Composable fun MoreScreen()`,
  `object UpdateChecker { fun maybeCheck(context: android.content.Context) }` (höchstens 1×/Tag,
  `AppPrefs.autoUpdateCheck`).
- „Über“ nennt App-/Kernversion und verlinkt die README-Abschnitte (Umfang und Grenzen).
- Nutzt: `LocationPickerDialog`, `TransfersSheet`, `TrashScreen` (K1), `BackgroundSettingsSection` (K2).

### T Task-Suite (Welle 3, nach allen Blöcken)
- Dateien: `.github/workflows/android-task.yml`, `android/test-android-task.sh`,
  `android/app/src/androidTest/**`, `android/app/src/test/**`, `android/test-servers/**`.
- Inhalt: Gesamtablauf unten, eine Suite, ein Einstieg.

## Symbole (K0, `res/drawable/ic_*.xml`, Material Symbols, Apache-2.0)
folder, file, image, video, audio, text, archive, document, apk, storage, sd_card, usb, cloud,
drive, device, room, trash, star, history, search, filter, sort, more_vert, menu, add, close,
check, copy, cut, paste, delete, share, rename, info, sync, play, pause, settings, warning,
error, refresh, upload, download, link, lock, key, analytics, duplicate, update, tab,
arrow_back, arrow_forward, chevron_right, expand_more, expand_less, visibility, terminal.

## Agentenplan und Sicherung
- Welle 1 (parallel, 5): A = B1a · B = B1b · C = B4 · D = K0 · E = REL. Danach: Hauptagent
  prüft Berichte, rustfmt-Stdin über alle geänderten Rust-Dateien, löst den Lock statisch auf
  (`cargo metadata --offline`, Platzhalter-Brücke), Commit je Block, Push auf `main` (Repo-Praxis,
  Endmarke `[task candidate]`, kein CI-Lauf).
- Welle 2 (parallel, 5): F = B2 · G = B3 · H = K1 · I = K2 · J = K3. Danach: Hauptagent löst den
  Lock für `native/android-bridge` statisch auf (`cargo metadata --offline`), Commit je Block, Push.
- Welle 3: K = T. Danach: Graph aktualisieren (graphify), Commit, Push, eine Remote-Suite.

## Gesamtablauf
Eine Suite, ein Einstieg (`android/test-android-task.sh`, Workflow `android-task.yml`,
`workflow_dispatch` mit `candidate_sha`), ausgeführt nur remote; Abdeckungsliste mit ausdrücklichen
Ausnahmen im Testbericht.

| Ablauf | Funktionen | Wie | Erfolg, wenn |
|---|---|---|---|
| G1 Kern-Einheiten | F2, F3–F11, F15–F17, F22 | `cargo test --locked --lib android_task_` (Linux-Host) | exakte erwartete Anzahl bestanden, 0 fehlgeschlagen |
| G2 Desktop unverändert | Bestandsschutz | `cargo check --locked --lib --bins` (Linux) und `--target x86_64-pc-windows-gnu`; `cargo metadata --locked --manifest-path native/explorer-command/Cargo.toml`; bestehende Tests der ausgelagerten Logik unter neuen Pfaden (Transfer, Job-Editor, Share-Merge/Lifecycle-View/Poll-Status, Cleanup-Scope, Baumzeilen) | kompiliert; genannte Tests grün; rustfmt je geänderter Datei sauber; clippy ohne Befund in geänderten Zeilen |
| G3 Android-Build | F1, F2 | `cargo tree --target aarch64-linux-android` ohne eframe/winit/rfd/trash; `cargo ndk` (arm64-v8a, x86_64) für `native/android-bridge`; 16-KB-Ausrichtung (`llvm-readelf -l`); `./gradlew assembleDebug assembleDebugAndroidTest testDebugUnitTest` | .so je ABI, APK baut, JVM-Tests (DTO-Dekodierung aller api.md-Beispiele) grün |
| G4 Gesamtablauf Gerät | F3–F17, F19–F22 | Emulator (x86_64, SD-Karten-Image), `adb` gewährt Dateizugriff/Benachrichtigungen; instrumentierte Tests über die echte App: lokale Abläufe (neu, umbenennen, filtern, rekursiv mit Fenstern, kopieren, verschieben zwischen Volumes, Eigenschaften, Papierkorb je Volume + Wiederherstellen, ZIP), Ordnerindex, SFTP/WebDAV/FTP auf dem Runner (`10.0.2.2`), Sync lokal↔SFTP inkl. Konflikt/Prüfen/Lösen/Zusammenführen, Job auf Volume-Wurzel mit Löschung (kein Papierkorb-Upload), Spiegeln der Volume-Wurzel (Papierkorb ausgelassen), Rename-Fehlernummern je Volume protokolliert, `bg.catchUp` über `work-testing`-TestDriver (Bedingungen/Periode), Dauerbetrieb-Dienst + Benachrichtigung, Übertragung läuft nach Home-Taste weiter (UiAutomator), Boot-Broadcast (`adb root`), Öffnen/Teilen (Espresso-Intents), Empfangen (`am start -a SEND` mit Deskriptor), Analyse, Duplikate, Update gegen lokalen Test-Feed | alle Tests grün; Abdeckungsliste: jede api.md-Methode aufgerufen außer ausdrücklich ausgenommen (`gdrive.signIn`: echtes Google-Konto nötig) |
| G5 Share mit Desktop | F18 | Runner startet `se-share-server` (Server/Relay an die Nicht-Loopback-IP des Runners) + Desktop-`se`; CLI erstellt einen Raum, Telefon tritt per Code bei; beide sehen sich; Telefon listet und lädt eine Datei des Desktop-Geräts über `share://room/…`; Raum entfernen → gemeldete Favoriten/Jobs | Mitgliedschaft beidseitig, Datei inhaltsgleich, Entfernen-Kaskade wie Desktop |
| G6 Oberfläche | F1, F3–F23 (Layout) | Compose-Test startet `MainActivity`, prüft Tabs, Seitenleiste, Filterzeile, Sync-/Teilen-/Mehr-Seite; Screenshots als Artefakt | Elemente vorhanden; Screenshots zur Layout-Kontrolle |
| G7 Release-Pfad statisch | F22-Verteilung | PowerShell-Parser über geänderte `.ps1`, YAML-Parse von `build.yml`, `bash -n`, Asset-Listen konsistent (20), `release-version.ps1` von Wrapper und Job gleich eingebunden | alle Prüfungen grün |

## Status
| Block | Agent | Status | Notiz |
|---|---|---|---|
| B1a | A | fertig (Welle 1, statisch geprüft) | Bericht: welle1-berichte.md |
| B1b | B | fertig (Welle 1, statisch geprüft) | Bericht: welle1-berichte.md |
| B4 | C | fertig (Welle 1, statisch geprüft) | Bericht: welle1-berichte.md |
| K0 | D | fertig (Welle 1, statisch geprüft) | Bericht: welle1-berichte.md |
| REL | E | fertig (Welle 1, statisch geprüft) | Bericht: welle1-berichte.md |
| B2 | F | fertig (Welle 2, statisch geprüft) | Bericht: welle2-berichte.md |
| B3 | G | fertig (Welle 2, statisch geprüft) | Bericht: welle2-berichte.md |
| K1 | H | fertig (Welle 2, statisch geprüft) | Bericht: welle2-berichte.md |
| K2 | I | fertig (Welle 2, statisch geprüft) | Bericht: welle2-berichte.md |
| K3 | J | fertig (Welle 2, statisch geprüft) | Bericht: welle2-berichte.md |
| T | K | in Arbeit (Welle 3) | |
