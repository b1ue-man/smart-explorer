# Smart Explorer – Architektur

Stand: 2026-09-25. Kurzüberblick als erster Einstieg; Details liefert der Code-Graph
(`graphify query "…"`, siehe AGENTS.md) und die Lesungen unter `docs/lesungen/`.

## Zweck
Schneller Datei-Explorer mit tiefem Filtern, Remote-Backends, Sync, Peer-to-Peer-Share und
Speicheranalyse – als Desktop-App (Windows, Linux; Rust + egui) und als Android-App
(Kotlin/Compose über demselben Rust-Kern).

## Einstiege
| Aufgabe / Frage | Anfangen bei |
|---|---|
| Desktop-Start, CLI-Flags | `native/src/main.rs`, `native/src/lib.rs::run_gui`, `native/src/bin/se.rs` (CLI) |
| Desktop-Oberfläche | `native/src/app/` (egui, `App` in `app/core/state.rs`, Frame-Schleife `app/core/frame_update.rs`) |
| Android-App | `android/app/src/main/java/app/smartexplorer/android/` (`SmartExplorerApp`, `MainActivity`, `ui/AppRoot.kt`) |
| Kotlin↔Rust-Vertrag | `docs/superpowers/plans/2026-09-25-android-apk/api.md`; Kotlin `core/Core.kt` + `core/NativeBridge.kt`; Rust `native/android-bridge/src/lib.rs` → `native/src/mobile/` |
| Dateisystem-/Remote-Zugriff | `native/src/vfs/` (`Backend`-Trait), Backends `sftp/`, `ftp/`, `webdav/`, `gdrive/`, `zipfs/`, Share über `daemon::open_share_backend` |
| Orte/Endpunkt-Strings | `native/src/connect/core/location.rs` (`EndpointSpec`), `connect/os/shared/resolution.rs` (`resolve_endpoint`) |
| Filter und Scan | `native/src/filter/`, `native/src/scanner/`, `native/src/rscan/`, Baumzeilen `filter/core/tree.rs` |
| Kopieren/Übertragen | lokal `native/src/copy/`, remote `native/src/transfer/` (Lane, Upload/Download/Remote-Kopie) |
| Sync | Jobs `native/src/syncjobs/`, Zwei-Wege `native/src/bisync/`, Einweg-Spiegeln `native/src/sync/` |
| Hintergrund-Daemon | `native/src/daemon/` (`run_daemon`, eingebettet `ensure_embedded_daemon`, Nachhol-Lauf `request_catch_up`), `native/src/autostart/` |
| Share/P2P | `native/src/share/` (Iroh/QUIC, Profile, Discovery, Räume), Share-Server `share-server/` |
| Speicheranalyse/Duplikate | `native/src/analytics/` |
| Updates | Desktop `native/src/updater/`, Android `update.*` in `native/src/mobile/os/shared/domains/` |
| Release | `native/publish-release-local.ps1` (einziger Einstieg), `docs/RELEASING.md` |

## Module und Ordner
- `native/src/*/core/` plattformneutrale Logik; `*/os/{windows,linux_os,android,shared}` Adapter,
  per `#[cfg]`/`#[path]` in `mod.rs` gewählt (Android hat eigene Arme; `target_os = "android"` ist nicht `linux`).
- `native/src/app/` nur Desktop (egui); egui-freie Logik liegt in Kernmodulen, `app/` hält Re-Export-Schalen.
- `native/src/mobile/` nur Android (und Linux-Tests): JSON-Fassade, Laufzeit, Tasks/Ereignisse,
  Domänen unter `os/shared/domains/`.
- `native/src/apptrash/` App-Papierkorb je Speichervolume (Android), `native/src/android_fs/` Rename-Kette für FUSE-Speicher.
- `native/android-bridge/` cdylib `libsmart_explorer_android.so` (Workspace-Mitglied; `explorer-command` ist ausgeschlossen).
- `android/` Gradle-Projekt (Modul `:app`): `core/` Brücke/DTOs, `ui/{files,sync,share,more,…}`, `service/`, `work/`, `system/`.

## Datenwege
- Android-Aufruf: Compose-Bildschirm → `Core.call(method, args)` (IO-Thread) → JNI `NativeBridge.call` →
  `mobile::call` → Dispatcher → Kernmodul (z. B. `vfs::Backend::list_dir`) → JSON-Antwort.
  Lange Vorgänge: Task-ID, Fortschritt über `pollEvents` (Ereignispumpe) → `Core.tasks`.
- Hintergrund Android: WorkManager `SyncWorker` → `bg.catchUp` → eingebetteter Daemon-Supervisor →
  `job::run_one` → `bisync::run`; Dauerbetrieb hält den Prozess mit `BackgroundService` (specialUse).
- Desktop: GUI ↔ Daemon-Prozess über Loopback-TCP-IPC (`daemon/os/shared/ipc*.rs`).
- Persistenz: App-Daten unter `support_dirs::app_data_dir()` (Android: `<filesDir>/smart_explorer`),
  Sync-Jobs `sync/jobs/*.conf`, Zugangsdaten `secrets-v1/` (Datei-Store), Share-Profile/Identität.

## Externe Abhängigkeiten
Rust-Crates und ihre Android-Tauglichkeit: `docs/refs/android-rust-deps.md`; JNI: `docs/refs/rust-jni-022.md`;
Gradle/AndroidX-Stände (AAR-Metadaten geprüft): `docs/refs/android-gradle-build.md` §8;
Android-APIs: `docs/refs/android-apis.md`, `docs/refs/android-platform.md`; CI: `docs/refs/android-ci.md`.

## Bauen, Prüfen, Konventionen
- Keine lokalen Builds/Tests (AGENTS.md); Prüfung ausschließlich über die eine Remote-Task-Suite je Batch
  (Android: `.github/workflows/android-task.yml` → `android/test-android-task.sh`).
- Android-Bibliothek: `cargo ndk -t arm64-v8a -t x86_64 --platform 30 -o android/app/src/main/jniLibs build -p smart_explorer_android`
  (im Verzeichnis `native/`), NDK aus `android/ndk-version`; Gradle braucht `-PrustlsVerifierMaven=<Pfad>`.
- Release-APK: `android/build-release-apk.sh` (Job `android-release-apk` in `build.yml`), Signatur aus Repo-Secrets.
- Neue Rust-Dateien < 500 Zeilen; Test-Präfix je Batch (Android: `android_task_`).
