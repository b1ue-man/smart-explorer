# Smart Explorer für Android – Recherche und Entscheidungen

Stand: 2026-09-25. Grundlage: `docs/lesungen/2026-09-25-android-*.md`, `docs/refs/*.md`
(Index: `docs/refs/INDEX.md`, `docs/lesungen/INDEX.md`). Statisch nachgeprüft (ohne Kompilieren):
`cargo tree --offline --locked --target aarch64-linux-android` und die Quellen im Cargo-Registry-Cache.

## E1 Oberfläche: Kotlin + Jetpack Compose statt egui auf Android
Entscheidung: native Android-App (Kotlin, Compose Material 3) über dem Rust-Kern.
Grund: Hintergrunddienste, WorkManager, Benachrichtigungen, Teilen/Empfangen, FileProvider,
Berechtigungsdialoge und Soft-Keyboard sind Android-Framework-APIs (Java/Kotlin); egui/winit auf
Android (NativeActivity) bietet davon nichts und ist für Touch nicht entworfen. Die Desktop-GUI ist
ein 263-Felder-Immediate-Mode-Zustand (`app/core/state.rs:35`) – ein Neuaufbau der Bedienung ist
ohnehin nötig.
Verworfen: eframe `android-native-activity` (keine Dienste, schwache Texteingabe, Desktop-Layout);
Flutter/React Native (zweite Sprache/Runtime ohne Vorteil gegenüber Kotlin).

## E2 Kern-Anbindung: JSON-Befehle über zwei JNI-Funktionen, `jni` 0.22.4
Entscheidung: `NativeBridge.call(method, argsJson): String` (synchron, von Kotlin auf
`Dispatchers.IO` aufgerufen) und `NativeBridge.pollEvents(timeoutMs): String` (blockiert bis
Ereignisse vorliegen oder Timeout). Lang laufende Vorgänge liefern eine Task-ID; Fortschritt/Ende
kommen als Ereignisse. `jni` 0.22.4, weil `rustls-platform-verifier` 0.7.0 auf Android genau diese
Version (`Env`-API) verlangt und die Version schon im `Cargo.lock` steht (`docs/refs/rust-jni-022.md`).
Grund: kleinste FFI-Fläche, keine Codegenerierung, keine JNA-Laufzeitkosten; `serde_json` ist schon
Abhängigkeit; die Fassade ist reines Rust und auf dem Linux-Host testbar.
Verworfen: UniFFI (JNA pro Aufruf, Codegen-Schritt im Build, pre-1.0); JNI-Rückrufe aus Rust-Threads
(Thread-Attach, Lebensdauer von Globalreferenzen) – Polling ist einfacher und robust.

## E3 Kern auf Android kompilierbar machen (statt Kern-Neubau)
Befund: sechs+ Modulauswahlen kennen nur `windows`/`target_os = "linux"` (`scanner`, `folder_index`,
`cloud::os`, `daemon::{ipc_storage,mount_process,platform}`, `syncjobs::platform`, `updater::os`,
`autostart::platform`, `net::{interfaces,platform}`, `share::platform_exec`); `trash` 5.2.5 und `rfd`
0.15.4 haben kein Android-Backend (Quelle geprüft); `eframe`/`egui_extras` gehören nur zum Desktop.
`libc` 0.2.186 exportiert `SYS_renameat2`/`RENAME_NOREPLACE` für Android (geprüft:
`libc-0.2.186/src/unix/linux_like/android/`), die POSIX-Adapter (`linux_os`) sind also übernehmbar.
Entscheidung:
- `Cargo.toml`: `eframe`, `egui_extras`, `trash` nach `[target.'cfg(not(target_os = "android"))']`,
  `rfd` nach `cfg(all(not(windows), not(target_os = "android")))`. Der Lockfile ändert sich dadurch
  nicht (gleiche Paketmenge). Neue Android-Abhängigkeiten nur aus dem Lockfile:
  `jni` 0.22.4, `ndk-context` 0.1.1, `rustls-platform-verifier` 0.7.0 (alle bereits gelockt).
- `lib.rs`: `app`, `icons`, `cli`, `run_gui` unter `cfg(not(target_os = "android"))`.
- Modulauswahlen: `any(target_os = "linux", target_os = "android")`, wo der POSIX-Adapter passt;
  eigene Android-Adapter, wo Linux-Verhalten falsch wäre: `support_dirs` (vom Host gesetzte
  Verzeichnisse), `daemon::platform` (Lock-Verzeichnis im App-Speicher; Energiesparen/getaktetes Netz
  aus Host-Zuständen), `autostart` (kein Prozess-Spawn; Host-Rückruf startet den eingebetteten
  Daemon), `cloud::open_url` (Host-Rückruf → Kotlin öffnet Browser), `net::interfaces` (`if-addrs`),
  `share::platform_exec` (Exec-Host nicht verfügbar), `bisync` Papierkorb (App-Papierkorb, E7).
Grund: 208k Zeilen erprobter Kern; der Anpassungsbedarf ist mechanisch und an Adaptergrenzen.
Verworfen: Kern-Teilkopie in eigenes Crate (Drift, doppelte Wartung); Feature-Flag `desktop-gui`
(ändert Desktop-Build-Kommandos; Zielplattform-cfg reicht und ist unsichtbar für den Desktop).

## E4 Pflicht-Initialisierung auf Android
- `ndk_context::initialize_android_context(vm, context)` vor jedem DNS-Zugriff: `hickory-resolver`
  0.26.1 liest DNS-Server auf Android per JNI und paniert ohne Kontext (hickory-dns#3625).
- `rustls_platform_verifier::android::init_with_env(env, context)` vor jeder TLS-Verbindung über
  `reqwest` (iroh): sonst Panik „Expect rustls-platform-verifier to be initialized“; dazu das
  Kotlin-AAR `rustls:rustls-platform-verifier` aus dem Maven-Verzeichnis des Crates
  `rustls-platform-verifier-android` 0.1.1 (Pfad per `cargo metadata`).
- `support_dirs::set_host` (Daten-, Cache-, Home-Verzeichnis, Gerätename, Boot-Marke) und
  `tempfile::env::override_temp_dir(cache)` vor jedem Pfadzugriff; kein `set_var` im
  Mehrthread-Prozess (Kritiker Befund 3). Nicht-app-Code, der `std::env::temp_dir()` nutzt und auf
  Android erreichbar ist, wechselt auf `support_dirs::temp_dir()`.
- Panik-Schutz: jede JNI-Funktion fängt Paniken (`catch_unwind`), Profil `panic = "unwind"`.

## E5 Hintergrund: ein eingebetteter Desktop-Daemon je Prozess + WorkManager-Nachholen
Befund: `daemon::run_daemon()` nutzt Datei-Steuerung (`daemon.stop`, Pause, Kadenz) und Loopback-TCP-
IPC; die Instanzsperre ist ein `flock`. Stoppen und Neustarten im selben Prozess ist unsicher
(`stop_daemon` beendet den Share-Dienst nicht, `cancel_and_join` bricht laufende Jobs ab, der Neustart
versucht den Lock nur einmal) – Kritiker Befund 1.
Entscheidung:
- Genau **ein** Daemon-Thread je App-Prozess, beim ersten Bedarf gestartet (App-`init`, Dienst,
  Worker, Share-Aufruf über `ensure_worker_ready`), **nie** wegen UI-Sichtbarkeit gestoppt. Android
  friert oder beendet den Prozess selbst; das ersetzt den Stopp. „Aus“ wirkt über das Sync-Flag
  (Android-Adapter von `autostart::is_enabled`) und `auto_connect`.
- `run_daemon()` wird in `run_daemon_with(handoff: Option<Handoff>)` geteilt (Desktop ruft weiter
  `run_daemon()` mit Umgebung, Android ohne); `ensure_worker_ready` startet auf Android den Thread
  direkt statt einen Prozess.
- „Beim Start“-Jobs auf Android einmal je Gerätestart (Boot-Marke vom Host, gespeichert), am
  Desktop unverändert je Daemon-Start.
- Dauerbetrieb: Vordergrunddienst `specialUse` hält den Prozess wach (Sideload-tauglich, aus
  BOOT_COMPLETED startbar; `dataSync` ist ab targetSdk 35 aus BOOT_COMPLETED verboten).
- Periodisch: `CoroutineWorker` (eindeutige periodische Arbeit, Bedingungen) ruft `bg.catchUp`:
  Der Daemon plant einen Nachhol-Lauf mit **seinem** Supervisor (Intervall fällig, Kalender seit
  letztem Lauf, Echtzeit einmal) und meldet Leerlauf; kein zweiter Scheduler. `setForeground(dataSync)`
  während des Laufs; Fehler beim Start des Vordergrunds (Hintergrund ohne Akku-Ausnahme) wird
  abgefangen, dann gilt das 10-Minuten-Limit; `onStopped` → Abbruch des Nachhol-Laufs.
- Übertragungen/„Jetzt ausführen“/Anmeldung: Vordergrunddienst `dataSync`, solange Tasks laufen;
  `onTimeout` (Android 15, 6 h/24 h) bricht Tasks ab.
- Auto-Pause: Kotlin liefert Energiesparmodus und getaktetes Netz (`sys.hostState`) an den Kern.
Verworfen: Daemon beim Verlassen stoppen (Befund 1); eigener „fällige Jobs“-Scheduler (zweite
Implementierung, verfehlt Echtzeit/Kalender); Dauerbetrieb über `dataSync` (6-h-Grenze, kein Boot-Start).

## E6 Speicherzugriff: „Zugriff auf alle Dateien“, minSdk 30
Entscheidung: `MANAGE_EXTERNAL_STORAGE` (Einrichtungsseite, Systemeinstellung), Orte aus
`StorageManager.getStorageVolumes()`/`StorageVolume.getDirectory()` (API 30). Kern arbeitet mit
Pfaden (`LocalBackend`). Öffnen lokaler Dateien in anderen Apps über einen eigenen schmalen
`LocalFileProvider` (nur Pfade unter gemeldeten Volumes, Schreiben aufs Original); der
`androidx`-FileProvider nur mit `cache-path` für Remote-Kopien, Teilen-Kopien und APK-Download
(Kritiker Runde 1 Befund 6, Runde 2 Befund 16).
Verworfen: SAF-Dokumentbäume (Kern kennt keine `content://`-Pfade; jeder Zugriff müsste über JNI).

## E7 Papierkorb auf Android
Befund: `trash` 5.2.5 schließt Android aus; Android hat keinen System-Papierkorb für beliebige
Dateien (MediaStore-Papierkorb nur für Medien, mit Bestätigungsdialog).
Entscheidung: App-Papierkorb je Speichervolume `<Volume>/.SmartExplorer-Papierkorb/<id>/<name>` +
`<id>.json` (ursprünglicher Pfad, Zeit); Verschieben per Umbenennen (gleiches Volume, atomar);
Wiederherstellen ohne Überschreiben; automatische Bereinigung nach 30 Tagen. Dieselbe Funktion
bedient `bisync` „Papierkorb“-Löschregel auf Android; schlägt das Verschieben fehl, wird wie am
Desktop nicht gelöscht (`apply_delete` liefert Fehler).
Verworfen: endgültiges Löschen statt Papierkorb (bricht Desktop-Semantik „Recycle“ und AGENTS.md
„destructive changes must not proceed when the backup step fails“).

## E8 Google Drive
Entscheidung: gleicher Ablauf wie Desktop (Nutzer-Client-ID Typ Desktop, PKCE, Loopback-Listener
im Kern); `open_url` öffnet über den Host den Systembrowser; der Anmelde-Task hält wie jeder
laufende Task den `dataSync`-Vordergrunddienst aktiv, bis die Rückmeldung kommt (≤ 3 min). Android-Client-Typ (SHA-1-gebunden, Google
Identity Services) verworfen: erfordert eine vom Veröffentlicher registrierte Client-ID je
Signaturschlüssel und eine Play-Services-Abhängigkeit; widerspricht dem Desktop-Modell „eigene
Client-ID“. Risiko offen dokumentiert (Spec, Annahmen).

## E9 Verteilung, Signatur, Update
- APK-Asset `smart-explorer-android.apk` (+ `.sha256`) im Update-Feed `release-native/update-feed/`
  und im GitHub-Release; `versionName` = `native/Cargo.toml`-Version, `versionCode` =
  `major·1 000 000 + minor·1 000 + patch`.
- Stabiler Release-Schlüssel (PKCS12, lokal mit `openssl` erzeugt, Sicherung unter
  `~/.config/smart-explorer-android/` des Arbeitskontos, Rechte 700/600), als GitHub-Secrets
  hinterlegt (`gh secret list` bestätigt); SHA-256-Fingerabdruck des Zertifikats im Repo
  (`android/release-cert.sha256`) für die Prüfung in Job und Wrapper.
- App-Update: `version.txt` aus dem Feed, Download, SHA-256-Prüfung, System-Installer
  (`ACTION_VIEW` + FileProvider auf die Cache-Kopie; `REQUEST_INSTALL_PACKAGES`).
- Release-Einbindung (Kritiker Befund 2): Brücke als Workspace-Mitglied `native/android-bridge`
  (ein `Cargo.lock`, den `Set-NativeVersion` schon pflegt). `Resolve-ReleasePlan`,
  `Get-NextPatchVersion` und `Set-NativeVersion` wandern unverändert in `native/release-version.ps1`,
  die Wrapper und APK-Job (pwsh auf Ubuntu) gemeinsam dot-sourcen. Der Job `android-release-apk`
  läuft vor `complete-release`: bei `Bump` wendet er denselben Bump auf seinen Checkout an und baut
  die signierte APK; bei `Tagged`/`Resume` baut er nicht, sondern reicht das committete APK durch.
  Er prüft `versionName`/`versionCode` (`aapt2 dump badging`) und das Zertifikat
  (`apksigner verify --print-certs`) und lädt APK + Metadaten als Artefakt hoch. Der Wrapper
  (`-AndroidApkDirectory`) prüft dieselben Metadaten und die SHA-256 und stagt die APK im selben
  atomaren Ablauf; Asset-Map und alle YAML-Kopien wachsen gemeinsam von 18 auf 20.
Verworfen: APK-Build im WSL1 des Windows-Runners (kein JDK/SDK/NDK, WSL1-Risiken, +30–60 min);
eigener Lock für die Brücke (veraltet nach jedem Bump); zweite Implementierung der Planlogik im Job;
unverifizierter Upload nach der Veröffentlichung.

## E10 Build-Werkzeuge (AAR-Metadaten geprüft, Kritiker Befund 16)
- AGP 8.13.2, Gradle 8.13 (Wrapper-JAR = offizielle Prüfsumme `81a82aae…e45f`), Kotlin 2.4.20
  (klassisches `kotlin-android` + compose/serialization-Plugins gleicher Version),
  compileSdk/targetSdk 36, minSdk 30, JDK 17.
- Die neuesten AndroidX-Stände (core 1.19, lifecycle 2.11, Compose 1.12/BOM 2026.08+, adaptive 1.3)
  verlangen laut `aar-metadata.properties` compileSdk 37 und AGP ≥ 9.1 – mit AGP 8.13.2/API 36 nicht
  baubar. Gewählt (Metadaten einzeln geladen und geprüft: minCompileSdk ≤ 36, minAGP ≤ 8.13.2):
  Compose BOM **2026.06.01** (ui/foundation 1.11.4, material3 1.4.0, adaptive 1.2.0,
  material-icons-core 1.7.8), **core-ktx 1.18.0**, **lifecycle-runtime/viewmodel-compose 2.10.0**,
  activity-compose 1.13.0, work-runtime-ktx 2.12.0, kotlinx-serialization-json 1.11.0,
  kotlinx-coroutines-android 1.11.0; transitive Auflösung geprüft (siehe unten).
- NDK fest gepinnt (Runner-Image wechselt am 2026-10-01 die NDK-Auswahl; r28+ liefert 16-KB-ELF-
  Ausrichtung), `cargo-ndk` 4.1.2, `--platform 30`, Ziele `aarch64-linux-android`, `x86_64-linux-android`.
- Brücken-Crate `native/android-bridge` (cdylib `smart_explorer_android`, Workspace-Mitglied, Lock
  per `cargo metadata --offline` statisch aufgelöst), Fassade `native/src/mobile/` (plattformneutral,
  auf dem Linux-Host testbar).

## E11 Prüfung
Eine Task-Suite `.github/workflows/android-task.yml` + `android/test-android-task.sh`:
Host-Tests der Fassade und der Android-Adapter (`android_task_`-Präfix), Desktop-Regressionschecks der
geänderten Module (Linux-Host + `x86_64-pc-windows-gnu`), Cross-Build der Android-Bibliothek, Gradle-
Build + Unit-Tests, Emulator-Gesamtablauf (instrumentierte Tests gegen die echte App mit SFTP-,
WebDAV- und FTP-Testservern auf dem Runner, erreichbar über `10.0.2.2`), Screenshots zur
Layout-Kontrolle.
