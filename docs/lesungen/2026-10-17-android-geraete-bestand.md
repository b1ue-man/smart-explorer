# Android-App und Gerätemodell: Bestand (Lesung)

Stand 2026-10-17, Lesestand da2646d2

**Auftrag.** Abschnitt A des Plans `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md` (Ergebnisse E4, E6, E7), Strang S5 „se Desktop (Windows, Linux) und se Launcher (Android)“ laut `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-desktop.md`, Teil Android-App und Gerätemodell. Die Desktop-Seite ist nicht Teil dieses Auftrags.

**Art der Lektüre.** Reine Bestandslektüre. Keine Änderung am Code, keine Builds, Tests, Paketinstallationen, Server, Commits oder Pushes. Keine Empfehlung, keine Bewertung, keine Entscheidung.

**Geltung.** Alle Aussagen gelten für den Quellstand bei Commit `da2646d2`. Zeilenangaben `Pfad:Zeile` beziehen sich auf die Zeilen der jeweiligen Datei. Repository-Pfade sind relativ zur Wurzel `<Repository-Wurzel>`.

**Kurzformen.** Kotlin-Kurzformen wie `system/Opener.kt`, `ui/AppNav.kt` oder `service/BackgroundController` bezeichnen Dateien unter `android/app/src/main/java/app/smartexplorer/android/`. Rust-Dateinamen ohne Verzeichnis bezeichnen diese Dateien:
- `share_status.rs`, `share_peers.rs`, `share_state.rs`, `locations.rs`: `native/src/mobile/os/shared/domains/`;
- `profiles.rs`, `profile_settings.rs`, `direct_relation.rs`, `room_relation.rs`, `room_relation_members.rs`, `relation_rights.rs`: `native/src/share/core/`;
- `api_exports.rs`: `native/src/share/`;
- `mount_peer_roots.rs`, `sidebar_locations.rs`: `native/src/app/core/`;
- `ipc_client.rs`, `ipc.rs`, `ipc_host.rs`, `ipc_host_relation_events.rs`, `ipc_host_legacy_events.rs`, `mount_source.rs`: `native/src/daemon/os/shared/`.

**Kennzeichnung.**
- *Befund*: aus einer gelesenen Quelle belegt, mit `Pfad:Zeile`.
- *Annahme*: nicht belegt; ausdrücklich als Annahme gekennzeichnet.
- *Lücke*: die fehlende Tatsache ist benannt, mit der Datei, die sie enthält.
- *Einstieg*: Trefferhinweis aus graphify; kein Beleg.

## 1. Ziel und Fragen

**Ziel.** Den Bestand der Android-App (Module, SDK-Stände, Abhängigkeiten, Komponenten, Auslieferung), die Dateiöffnung der App, das Gerätemodell der Share-Schicht und die Stellen, an denen Geräte und Räume heute als Orte erscheinen, mit Belegen zusammentragen.

**Fragen (Wortlaut aus dem Auftrag):**
- **F1.** Android-App: Module, SDK-Stände, Abhängigkeiten (Compose-Version), Activities und ihre Intent-Filter (gibt es schon einen HOME/LAUNCHER-Filter außer der Startaktivität?), Dienste, Provider, Berechtigungen; wie werden Komponenten heute ein-/ausgeschaltet; wie wird die APK gebaut, signiert und ausgeliefert; welche Stufen hat die Android-Task-Suite?
- **F2.** Wie öffnet die App heute eine Datei (lokal und entfernt), welche Opener/Intents/Provider werden genutzt?
- **F3.** Gerätemodell: Typen und Felder für Direkt-Kontakt, Raum, Mitglied, Freigabe (Export), Präsenz/online, automatisch verbinden/öffnen; wie wird ein Gerät mit seinen Freigaben als Ort/Backend geöffnet (`open_share_backend`, Locator-Form); wie erfährt die GUI (Desktop/Android) von online/offline?
- **F4.** Wo erscheinen Geräte und Räume heute als Orte (Desktop-Seitenleiste, Android PlacesPanel), und welche Daten liefert die Bridge dafür (Methoden, JSON-Felder)?

**Leseumfang.** Die im Auftrag genannten Dateien (vollständige Liste in Abschnitt 6). Zusätzlich die Daemon-Stellen, die der Auftrag für `open_share_backend` und für Präsenz/Status erlaubt.

## 2. Befunde je Frage

### 2.1 F1: Android-App

#### 2.1.1 Module, SDK-Stände, Build-Varianten

- Ein Gradle-Modul: `include(":app")` (`android/settings.gradle.kts:54`); `rootProject.name = "SmartExplorer"` (`android/settings.gradle.kts:53`).
- Root-Build: die Plugins `android.application`, `kotlin.android`, `kotlin.compose`, `kotlin.serialization` mit `apply false` (`android/build.gradle.kts:2-5`). Das Modul `:app` nutzt dieselben vier Plugins (`android/app/build.gradle.kts:4-7`).
- Paket: `namespace` und `applicationId` = `app.smartexplorer.android` (`android/app/build.gradle.kts:83`, `:88`).
- SDK: `compileSdk = 36` (`:84`), `minSdk = 30` (`:89`), `targetSdk = 36` (`:90`).
- Java und Kotlin: `sourceCompatibility` und `targetCompatibility` = `VERSION_17` (`:140-141`); `kotlin { jvmToolchain(17) }` (`:154`); `jvmTarget` = `JVM_17` (`:156`).
- NDK: `ndkVersion` aus der Datei `android/ndk-version` (`:77-80`, `:85`). Der Wert ist Lücke (Datei nicht gelesen).
- ABI-Filter: `arm64-v8a`, `x86_64` (`:95`).
- Build-Varianten: genau `debug` und `release` (`:110-123`). Keine `productFlavors`: die Suche nach `flavor`, `dimension` und `variant` in `android/app/build.gradle.kts` ergab keinen Treffer.
- `release`: `isMinifyEnabled = false` (`:117`); ProGuard-Dateien `proguard-android-optimize.txt` und `proguard-rules.pro` (`:118`).
- `debug`: die Release-Signatur gilt nur, wenn die Gradle-Eigenschaft `syncReliabilitySigning` den Wert `"true"` hat (`:74`, `:111-113`); dann verlangt die Build-Datei alle vier Signatur-Variablen (`:75`).
- Version: `versionName` aus `native/Cargo.toml`, Abschnitt `[package]` (`:19-40`, `:60`, `:92`). `versionCode = major * 1_000_000 + minor * 1_000 + patch` (`:61-62`, `:91`); Minor und Patch müssen unter 1000 liegen (`:36-38`).
- Weitere Einstellungen: `buildFeatures` mit `compose = true` und `buildConfig = true` (`:134-137`); `packaging.jniLibs.useLegacyPackaging = true` (`:125-128`); Ausschluss `META-INF/{AL2.0,LGPL2.1}` (`:129-131`); `lint { abortOnError = false }` (`:148-150`); `testOptions { animationsDisabled = true }` (`:144-146`).

#### 2.1.2 Abhängigkeiten und Compose-Version

Direkte Abhängigkeiten, vollständig (`android/app/build.gradle.kts:160-193`):

| Konfiguration | Eintrag | Zeile |
|---|---|---|
| implementation | `platform(libs.compose.bom)` | 161 |
| implementation | `libs.androidx.core.ktx` | 164 |
| implementation | `libs.androidx.activity.compose` | 165 |
| implementation | `libs.androidx.lifecycle.runtime.compose` | 166 |
| implementation | `libs.androidx.lifecycle.viewmodel.compose` | 167 |
| implementation | `libs.androidx.work.runtime.ktx` | 168 |
| implementation | `libs.androidx.material3.adaptive` | 169 |
| implementation | `libs.kotlinx.serialization.json` | 170 |
| implementation | `libs.kotlinx.coroutines.android` | 171 |
| implementation | `rustls:rustls-platform-verifier:$rustlsVerifierVersion` | 172 |
| implementation | `libs.compose.ui` | 174 |
| implementation | `libs.compose.ui.graphics` | 175 |
| implementation | `libs.compose.ui.tooling.preview` | 176 |
| implementation | `libs.compose.material3` | 177 |
| implementation | `libs.compose.material.icons.core` | 178 |
| debugImplementation | `libs.compose.ui.tooling` | 179 |
| debugImplementation | `libs.compose.ui.test.manifest` | 180 |
| testImplementation | `libs.junit4` | 182 |
| androidTestImplementation | `platform(libs.compose.bom)` | 162 |
| androidTestImplementation | `libs.androidx.test.ext.junit` | 184 |
| androidTestImplementation | `libs.androidx.test.espresso.core` | 185 |
| androidTestImplementation | `libs.androidx.test.espresso.intents` | 186 |
| androidTestImplementation | `libs.androidx.test.runner` | 187 |
| androidTestImplementation | `libs.androidx.test.rules` | 188 |
| androidTestImplementation | `libs.androidx.test.core` | 189 |
| androidTestImplementation | `libs.androidx.test.uiautomator` | 190 |
| androidTestImplementation | `libs.androidx.work.testing` | 191 |
| androidTestImplementation | `libs.compose.ui.test.junit4` | 192 |

- `rustlsVerifierVersion` stammt aus `native/Cargo.lock`, Crate `rustls-platform-verifier-android` (`android/app/build.gradle.kts:43-47`, `:63`). Das Maven-Verzeichnis kommt aus der Gradle-Eigenschaft `rustlsVerifierMaven` und wird über das Repository `rustlsPlatformVerifier` für die Gruppe `rustls` eingebunden (`android/settings.gradle.kts:9-32`, `:39-49`).
- **Compose-Version: Lücke.** Die Versionen der `libs.*`-Einträge stehen in `android/gradle/libs.versions.toml`. Diese Datei steht nicht im Leseauftrag. Die Compose-BOM der gebauten App ist damit nicht belegt.
- Die Dokumentation `docs/refs/android-gradle-build.md` nennt zwei verschiedene Stände, von denen sich keiner als gebaut belegen lässt:
  - §2 (Tabelle, `:150-176`) und die Vorlage §5.3 (`:329-373`): `androidx.compose:compose-bom` 2026.09.00; `androidx.core:core-ktx` 1.19.1; `androidx.lifecycle` 2.11.0; `androidx.compose.material3.adaptive` 1.3.0.
  - §8 „Korrektur 2026-09-25“ (`:662-684`), die ihre Stände als „Verbindliche Pins für AGP 8.13.2 / compileSdk 36“ bezeichnet: `compose-bom` 2026.06.01; `core-ktx` 1.18.0; `lifecycle-*-compose` 2.10.0; `material3.adaptive` 1.2.0. §8 begründet das damit, dass die Stände aus §2 `minCompileSdk=37` und `minAndroidGradlePluginVersion=9.1.0` verlangen. §5.3 ist nach §8 nicht nachgezogen.
- Toolchain laut Dokumentkopf (`docs/refs/android-gradle-build.md:1`): AGP 8.13.2, Gradle 8.13, Kotlin 2.4.20. **Lücke:** die tatsächlichen Werte stehen in `android/gradle/libs.versions.toml` (AGP, Kotlin) und `android/gradle/wrapper/gradle-wrapper.properties` (Gradle). Beide Dateien sind nicht gelesen.

#### 2.1.3 Activities, Intent-Filter, Dienste, Empfänger, Provider, Berechtigungen

Manifest `android/app/src/main/AndroidManifest.xml`, vollständig:

- Application (`:50-59`): `android:name=".SmartExplorerApp"`, `allowBackup="false"`, `dataExtractionRules="@xml/data_extraction_rules"`, `enableOnBackInvokedCallback="true"`, `icon`, `label`, `roundIcon`, `supportsRtl="true"`, `theme="@style/Theme.SmartExplorer"`.
- Activities: genau eine, `.MainActivity` (`:61-65`) mit `exported="true"`, `launchMode="singleTop"`, `windowSoftInputMode="adjustResize"`.
- Intent-Filter von `.MainActivity`, genau vier:
  1. Aktion `android.intent.action.MAIN` mit Kategorie `android.intent.category.LAUNCHER` (`:66-69`).
  2. Aktion `android.intent.action.SEND`, Kategorie `DEFAULT`, Daten `mimeType="*/*"` (`:70-74`).
  3. Aktion `android.intent.action.SEND_MULTIPLE`, Kategorie `DEFAULT`, Daten `mimeType="*/*"` (`:75-79`).
  4. Aktion `android.intent.action.VIEW`, Kategorie `DEFAULT`, Daten `vnd.android.document/directory`, `resource/folder`, `inode/directory` (`:83-89`). Kommentar `:80-82`: Dateien werden bewusst nicht beansprucht.
- **HOME-Filter: keiner.** Die Zeichenfolge `HOME` kommt im Manifest nicht vor. Der einzige LAUNCHER-Filter ist der der Startaktivität (`:68`).
- **Widgets: keine.** Im Manifest und in allen gelisteten Kotlin-Dateien ergab die Suche nach `widget` (ohne Unterscheidung von Groß- und Kleinschreibung) keinen Treffer.
- Dienste, genau drei:

| Klasse | Attribute | Zeilen | Kommentar im Manifest |
|---|---|---|---|
| `.service.TaskForegroundService` | `exported="false"`, `foregroundServiceType="dataSync"` | 93-96 | `:92`: Übertragungen, Läufe „Jetzt ausführen“, Google-Anmeldung |
| `.service.BackgroundService` | `exported="false"`, `foregroundServiceType="specialUse"`, Property `android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE` | 100-107 | `:98-99`: Dauerbetrieb oder „Share im Hintergrund erreichbar“, beides nach Nutzer-Opt-in; bootfähiger Typ |
| `androidx.work.impl.foreground.SystemForegroundService` | `foregroundServiceType="dataSync"`, `tools:node="merge"` | 110-113 | `:109`: WorkManager-Dienst |

- Empfänger, genau vier:

| Klasse | Attribute und Filter | Zeilen |
|---|---|---|
| `.service.ExecHostStopReceiver` | `exported="false"`, kein Intent-Filter | 116-118 |
| `.system.KeepAliveReceiver` | `exported="false"`, kein Intent-Filter | 121-123 |
| `.system.SyncScheduleReceiver` | `exported="false"`, kein Intent-Filter | 124 |
| `.system.BootReceiver` | `exported="false"`; Aktionen `BOOT_COMPLETED`, `MY_PACKAGE_REPLACED`, `SCHEDULE_EXACT_ALARM_PERMISSION_STATE_CHANGED` | 126-134 |

- Provider, genau zwei:

| Klasse | Authority und Attribute | Zeilen |
|---|---|---|
| `.system.LocalFileProvider` | `${applicationId}.localfiles`; `exported="false"`; `grantUriPermissions="true"` | 136-141 |
| `androidx.core.content.FileProvider` | `${applicationId}.files`; `exported="false"`; `grantUriPermissions="true"`; Meta-Daten `android.support.FILE_PROVIDER_PATHS` = `@xml/file_paths` | 143-152 |

- Berechtigungen, genau 16 (`uses-permission`):
  1. `FOREGROUND_SERVICE` (`:6`)
  2. `FOREGROUND_SERVICE_DATA_SYNC` (`:7`)
  3. `FOREGROUND_SERVICE_SPECIAL_USE` (`:8`)
  4. `POST_NOTIFICATIONS` (`:11`)
  5. `RECEIVE_BOOT_COMPLETED` (`:14`)
  6. `SCHEDULE_EXACT_ALARM` (`:15`)
  7. `MANAGE_EXTERNAL_STORAGE` (`:18-20`; Kommentar `:17`: Zugriff auf alle Dateien)
  8. `INTERNET` (`:23`)
  9. `ACCESS_NETWORK_STATE` (`:24`)
  10. `ACCESS_WIFI_STATE` (`:25`)
  11. `CHANGE_WIFI_MULTICAST_STATE` (`:26`)
  12. `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` (`:27`)
  13. `WAKE_LOCK` (`:30`)
  14. `PACKAGE_USAGE_STATS` (`:34-36`; Kommentar `:32-33`: optional, vom Nutzer in den Einstellungen erteilt, nie zur Laufzeit angefragt)
  15. `QUERY_ALL_PACKAGES` (`:41-43`)
  16. `REQUEST_INSTALL_PACKAGES` (`:46`)
- Komponenten-Aktivierung im Manifest: kein Element trägt das Attribut `enabled` (Suche im Manifest ohne Treffer).

#### 2.1.4 Ein- und Ausschalten von Komponenten

- Befund: In den gelisteten Dateien (Manifest, `android/app/build.gradle.kts`, die Kotlin-Dateien des Bereichs A) gibt es keinen Aufruf von `setComponentEnabledSetting`, keine Nutzung von `PackageManager` und kein `enabled`-Attribut. Die Komponenten sind statisch deklariert.
- Befund: Einstellbar ist das Verhalten der Hintergrunddienste über Werte in `android/app/src/main/java/app/smartexplorer/android/prefs/AppPrefs.kt`:
  - `bgMode` mit den Werten `off|periodic|persistent`, Standard `periodic` (`:19`, `:30`, `:56`, `:120-124`);
  - `shareReachable`, Standard `true` (`:38`, `:69`, `:148`);
  - `shareRunning`, gespeicherter Stand „Share eingerichtet“ (`:39`, `:72-75`, `:150`).
- Befund: `SmartExplorerApp.onCreate` startet beim Prozessstart `AppPrefs.init`, `Notifications.ensureChannels`, `WakeKeeper.init`, `Core.start`, `TaskKeeper.start`, `ExecHostNotifier.start` und `BackgroundController.onAppStart`, in dieser Reihenfolge (`android/app/src/main/java/app/smartexplorer/android/SmartExplorerApp.kt:18-26`).
- Befund: In den gelisteten Dateien gibt es keinen Schalter, der eine Launcher-Rolle oder eine Komponente an- oder abschaltet.
- **Lücke:** welche Klassen Dienste tatsächlich starten oder beenden. `service/BackgroundController` (aufgerufen in `SmartExplorerApp.kt:26`) und `system/BootReceiver` (Manifest `:126-134`) liegen außerhalb des Leseauftrags.
- **Annahme (Plattformwissen, nicht aus lokalen Quellen belegt):** eine Komponente ohne `android:enabled` ist im installierten Paket aktiv.

#### 2.1.5 Bau, Signatur, Auslieferung

Bauskript `android/build-release-apk.sh` (398 Zeilen). Aufrufmodi laut Usage (`:11-28`): `--check-env` (prüft die Umgebung; installiert laut Usage-Text nur den gepinnten NDK aus `android/ndk-version` und zwei Rust-Ziele); Bau mit `--out DIR`; `--verify APK --out DIR` (Prüfung einer vorhandenen APK); `--expect-version X.Y.Z`.

Ablauf des Bauzweigs (Hauptablauf `:368-398`):

1. `check_build_env` (`:218-230`): prüft die Werkzeuge `cargo`, `rustup`, `cargo-ndk`, `java`, `python3`, `sha256sum`, `find`, `sort`; ferner `android/gradlew`, das Zertifikat (`expected_cert`), die Cargo-Version, die Versionscode-Formel, SDK, NDK, Build-Tools, Rust-Ziele und `require_signing`.
2. `build_apk` (`:261-290`): `cargo ndk` für `arm64-v8a` und `x86_64` auf Plattform `30`, mit `build --release --locked -p smart_explorer_android`; Ausgabe nach `android/app/src/main/jniLibs/<abi>/libsmart_explorer_android.so` (Variablen `:38-46`). Danach `assert_16k_aligned` (`:250-259`) und `sh ./gradlew --no-daemon --console=plain :app:assembleRelease "-PrustlsVerifierMaven=<Verzeichnis>"`. Verlangt wird genau eine APK in `android/app/build/outputs/apk/release`; eine Datei `*-unsigned.apk` wird abgelehnt.
3. `publish_verified` (`:336-366`): kopiert die APK als `.partial`, prüft sie mit `verify_apk`, berechnet die SHA-256-Summe, benennt sie in `smart-explorer-android.apk` um (`:38`) und schreibt `apk-metadata.json` (`:39`) mit den Feldern `version`, `versionCode`, `sha256`, `certSha256`.

Prüfung `verify_apk` (`:292-332`):
- `aapt2 dump badging`: `versionName` muss der Cargo-Version entsprechen, `versionCode` dem berechneten Code.
- `apksigner verify --print-certs`: genau ein Signierer; dessen SHA-256 muss dem Wert aus `android/release-cert.sha256` entsprechen.
- Python-Prüfung: `lib/<abi>/libsmart_explorer_android.so` ist für jede ABI in der APK enthalten.

Signatur:
- Release-Signatur entsteht nur, wenn alle vier Umgebungsvariablen gesetzt sind: `ANDROID_KEYSTORE_FILE`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` (`android/app/build.gradle.kts:65-72`, `:99-108`, `:119-121`; `android/build-release-apk.sh:208-216`). Ohne sie ergibt `assembleRelease` eine unsignierte APK (Kommentar `android/app/build.gradle.kts:65-66`).
- Zertifikatsbindung: die erste Zeile von `android/release-cert.sha256` muss ein 64-stelliger SHA-256-Fingerabdruck sein (`android/build-release-apk.sh:135-142`). Der Fingerabdruck selbst ist Lücke (Datei nicht gelesen).

Versionscode: `version_code_for` (`android/build-release-apk.sh:119-133`) rechnet `major * 1000000 + minor * 1000 + patch`, verlangt Minor und Patch unter 1000, Major mit höchstens vier Stellen und ein Ergebnis zwischen 1 und 2100000000. Die Formel ist in `android/app/build.gradle.kts:61-62` dieselbe. Der Kommentar in `:119-120` nennt zusätzlich `Get-PublicationAndroidVersionCode` in `native/release-publication.ps1` (nicht gelesen).

Auslieferung:
- Der Kopfkommentar nennt als Aufrufer den Job `android-release-apk` in `build.yml` und das Skript `native/publish-release-local.ps1` (`android/build-release-apk.sh:5-7`). Diese Dateien sind nicht gelesen. Die Angabe ist eine Kommentarangabe, keine geprüfte Verdrahtung.
- `AGENTS.md` (Abschnitt „mandatory release“) verlangt im GitHub Release die APK `smart-explorer-android.apk` und ihren Hash.
- **Lücke:** `.github/workflows/build.yml` und `native/publish-release-local.ps1` (tatsächliche Aufrufwege) sind nicht gelesen.

#### 2.1.6 Stufen der Android-Task-Suite

Datei `android/test-android-task.sh` (1101 Zeilen). Der Kopf (`:1-25`) beschreibt einen Einstieg; laut Kopf ruft jeder Job von `.github/workflows/android-task.yml` genau einen Unterbefehl auf (`:4`). Die Workflow-Datei ist nicht gelesen.

Unterbefehle laut Dispatcher (`:1066-1101`), vollständig:
1. `host` ohne Argument und `host --without-batch-gates`;
2. `desktop-bins --out DIR`;
3. `android-build --out DIR`;
4. `review-build --out DIR`;
5. `sync-reliability-build --out DIR` (im Kopf `:1-25` nicht genannt);
6. `emulator-run` ohne Argument;
7. `-h` und `--help` (Usage).

Stufen je Unterbefehl:
- **`host`** (`cmd_host`, `:421-436`):
  - G1: Unit-Tests mit dem Namenspräfix `android_task_` auf dem Linux-Host (`run_g1`, `:274-280`);
  - G2, Desktop unverändert: verschobene Desktop-Tests unter den neuen Pfaden (`run_g2_moved_tests`, `:282-297`); rustfmt im stdin-Modus für die Rust-Dateien des Stapels (`run_g2_rustfmt`, `:303-318`); `cargo check` und `clippy --lib --bins` für Linux und Windows-GNU (`run_g2_check_and_clippy`, `:323-355`; Kopf `:6-9`); Befehlsmanifest des Explorers (`run_g2_explorer_command`, `:357-360`);
  - G7: statischer Release-Pfad (`run_g7`, `:373-390`).
  - `--without-batch-gates` lässt die rustfmt- und clippy-Gates auf den Zeilen des Stapels aus (Kopf `:10-14`; Zweig `:434`).
- **`desktop-bins`** (`cmd_desktop_bins`, `:439-451`): G5-Voraussetzungen, nur die Linux-Binärdateien `se` und `se-share-server` im Dev-Profil.
- **`android-build`** (`cmd_android_build`, `:489-555`): G3:
  - Android-Abhängigkeitsbaum ohne Desktop-GUI-, Papierkorb- und D-Bus-Crates (`:501`);
  - `cargo ndk check` für arm64-v8a (`:511`);
  - `cargo ndk build` für x86_64 im Dev-Profil für den Emulator (`:514`);
  - Maven-Verzeichnis von rustls-platform-verifier über `cargo metadata` und `jq` (`:520`);
  - Gradle: Debug-APK, Test-APK und JVM-Unit-Tests (`:527`).
- **`review-build`** (`cmd_review_build`, `:558-618`): RV1, nur x86_64-JNI und APKs sowie direkt betroffene JVM-Klassen (Kopf `:19-20`; Aufruf über `native/test-review-task.sh`).
- **`emulator-run`** (`cmd_emulator_run`, `:944-1063`): G4, G5 und G6 auf dem gebooteten Emulator; verlangt `SE_TASK_ARTIFACTS` mit den Unterordnern `apk/` und `bin/` (Kopf `:22-23`). Schritte in dieser Reihenfolge:
  - Gerätevorbereitung (`:962`);
  - Instrumentierungen `main` (`:1011`), `rename-internal` (`:1013`), `rename-sdcard` (`:1014`);
  - G5 „Share with the desktop CLI“ (`:1016`): `share-join` (`:1022`), `share-exec` mit `share_exec_host` (`:1025`, Schritttext `:712`), `reach` (`:1026`) mit `reach_check` (`:1027`) und `reach_boot_check` (`:1028`), `share-cleanup` (`:1035`);
  - `boot-prep` (`:1041`) mit der G4-Prüfung `boot_check` (`:1042`, Schritttext `:729`: „BOOT_COMPLETED starts only the specialUse background service“);
  - G6: „api.md coverage and evidence“ (`:1044`, Prüfung `:1047`).
  - Jede Instrumentierung läuft mit `timeout 5400` (`:686`).

### 2.2 F2: Dateien öffnen

#### 2.2.1 Lokale Datei im Dateien-Tab

- Medienbetrachter (`android/app/src/main/java/app/smartexplorer/android/ui/files/FilesScreen.kt:135-148`): `MediaViewer` erhält `onOpenExternal`. Ist eine geladene Kopie vorhanden (`loaded`), geht die Datei direkt als `FilesEffect.Open(loaded.path, loaded.mime, chooser)` an eine externe App (`ACTION_VIEW`-Empfänger, `:139-142`). Sonst ruft die Routine `vm.openFile(entry, media.local, chooser)` auf (`:143-145`).
- Aktion `EntryAction.OpenWith`: `openFile(it, local, chooser = true)` für genau einen Eintrag (`FilesScreen.kt:398-400`).
- **Lücke:** die Implementierung von `FilesViewModel.openFile`, `openLocation`, `openRoot` und `emit` liegt außerhalb des Leseauftrags. Ob ein Antippen einer Datei den Medienbetrachter öffnet, ist in den gelisteten Dateien nicht belegt. `AppPrefs.mediaViewer` (Standard `true`) beschreibt die Wahl als „Medien im eigenen Betrachter öffnen“ (`prefs/AppPrefs.kt:49-53`).
- Effekte: `perform(context, effect)` (`FilesScreen.kt:431-451`) behandelt genau drei Effektarten: `Open`, `Share`, `CopyText`. Effekte laufen nur bei gestartetem Lebenszyklus (`FilesScreen.kt:102-109`).
- `FilesEffect.Open` → `Opener.open(context, localPath, mime, chooser)` (`system/Opener.kt:33-41`):
  - Intent `Intent.ACTION_VIEW` mit `setDataAndType(uri, type)`;
  - Flags `FLAG_GRANT_READ_URI_PERMISSION` und `FLAG_GRANT_WRITE_URI_PERMISSION`;
  - Typ: `mime`, sonst aus der Dateiendung über `LocalFileProvider.mimeTypeOf` (`:35`);
  - bei `chooser = true`: `Intent.createChooser(view, "Öffnen mit")` (`:40`);
  - Ergebnisse `Opener.Outcome`: `Started`, `NoApp`, `NotShareable` (`:27`); `ActivityNotFoundException` ergibt `NoApp` (`:109-117`); ohne Activity-Kontext wird `FLAG_ACTIVITY_NEW_TASK` gesetzt (`:110`).
- URI-Wahl `Opener.uriFor` (`system/Opener.kt:79-101`):
  - der kanonische Pfad unterhalb von `context.cacheDir` ergibt `FileProvider.getUriForFile(context, "<applicationId>.files", file)` (Cache-Kopien; bei Pfaden außerhalb der deklarierten Cache-Ordner wird `null`, `:91-98`);
  - Pfade außerhalb von `context.cacheDir` ergeben `LocalFileProvider.uriFor(context, canonical.path)` (Originale, `:100`).
- Meldungen (`FilesScreen.kt:433-437`): `NoApp` → „Keine App für diesen Dateityp“; `NotShareable` → „Diese Datei kann nicht an andere Apps übergeben werden.“
- `LocalFileProvider` (`android/app/src/main/java/app/smartexplorer/android/system/LocalFileProvider.kt`):
  - URI-Form `content://<applicationId>.localfiles/<Pfadsegmente des absoluten Pfads>` (`:21`, `:96-104`);
  - nicht exportiert, nur über URI-Freigaben erreichbar (`:23-24`; Manifest `:137-141`); `attachInfo` verweigert eine exportierte Instanz und eine Instanz ohne `grantUriPermissions` (`:29-34`);
  - `openFile(uri, mode)` öffnet mit dem Modus; `"w"` wird zu `"wt"` und schneidet die Datei ab (`:64-74`);
  - `query` liefert genau die Spalten `DISPLAY_NAME` und `SIZE` (`:38-62`);
  - `insert` liefert `null`, `delete` und `update` liefern `0` (`:77-81`);
  - ein Pfad wird nur akzeptiert, wenn der kanonische Pfad unterhalb eines Volumes aus `Storage.volumes(context)` liegt (`:113-128`; `Storage` nicht gelesen).

#### 2.2.2 Teilen und Kopieren (ausgehend)

- `EntryAction.Share` → `share(entries)` (`FilesScreen.kt:401`) → `FilesEffect.Share` → `Opener.share(context, paths)` (`:438-442`).
- `Opener.share` (`system/Opener.kt:44-69`): eine Datei → `Intent.ACTION_SEND` mit `EXTRA_STREAM` und `clipData`; mehrere → `Intent.ACTION_SEND_MULTIPLE` mit `EXTRA_STREAM` als Liste; `FLAG_GRANT_READ_URI_PERMISSION`, also nur Lesen (`:63`); `Intent.createChooser(send, null)` mit `EXTRA_EXCLUDE_COMPONENTS` = die eigene `MainActivity` (`:64-67`).
- Ordner werden ausgelassen; bei Erfolg erscheint „Ordner ausgelassen: N“, wenn `skippedFolders > 0` (`FilesScreen.kt:439`). Ohne App: „Keine App zum Teilen gefunden“; nicht teilbar: „Diese Dateien können nicht geteilt werden.“ (`:440-441`).
- Kopieren: `FilesEffect.CopyText` → `Opener.copyText(context, label, text)` mit `ClipboardManager` (`system/Opener.kt:72-76`); Meldungen „<label> kopiert“ bzw. „Zwischenablage nicht verfügbar“ (`FilesScreen.kt:443-449`). Pfad-Kopie über `EntryAction.CopyPath` (`:410`) und `FolderAction.CopyPath` (`:425`).

#### 2.2.3 Entfernte Datei

- In den gelisteten Kotlin-Dateien gibt es keinen eigenen Öffnungsweg für entfernte Dateien. Der Weg läuft über eine Kopie:
  - Manifest `AndroidManifest.xml:143`: „Cache copies only (remote files to open/share, downloaded update APK)“;
  - `system/Opener.kt:19-21`: „cache copies (`cache/open`, `cache/share`) through the androidx FileProvider“;
  - `FilesScreen.kt:140`: „An already loaded copy goes straight to the other app; otherwise as "Öffnen"“.
- **Lücke:** wie die Kopie entsteht (Herunterladen, Ziel `cache/open`), liegt in `FilesViewModel` und im Rust-Kern; nicht gelesen. Die Deklaration der Cache-Ordner in `android/app/src/main/res/xml/file_paths.xml` ist nicht gelesen. Der Kommentar `system/Opener.kt:95` nennt die deklarierten Cache-Ordner `open/`, `share/` und `update/`.
- **Annahme:** entfernte Dateien ohne geladene Kopie gehen über `vm.openFile(entry, media.local, chooser)` (`FilesScreen.kt:144`); die Ausführung dieser Funktion liegt nicht im Leseauftrag.

#### 2.2.4 Eingehend: eine externe App öffnet einen Ordner oder eine Datei in Smart Explorer

- `MainActivity.handleIntent` (`android/app/src/main/java/app/smartexplorer/android/MainActivity.kt:75-87`), in dieser Reihenfolge:
  1. `AppNav.fromIntent(this, intent)`: nur Intents mit der Aktion `app.smartexplorer.android.action.NAVIGATE` und dem Token dieser Installation (`ui/AppNav.kt:97-111`).
  2. `OpenWithIntent.outcome(this, intent)` (`MainActivity.kt:82`):
     - `Open` → `AppNav.send(opened.request)`;
     - `Refused` → „Dieser Ort lässt sich in Smart Explorer nicht öffnen.“ (`:84`);
     - `null` → `ShareIntentHandler.handle(this, intent)` (`:85`; `ShareIntentHandler` nicht gelesen).
- `OpenWithIntent.outcome` (`system/OpenWithIntent.kt:25-41`): nur `ACTION_VIEW` mit Daten. Schema `file` → Pfad aus `uri.path` (`:29`). Schema `content` → `documentPath` (`:30`), das nur bei der Authority `com.android.externalstorage.documents` arbeitet (`:43-55`; Dokument-ID über `DocumentsContract.isDocumentUri` bzw. `getTreeDocumentId`, dann `OpenWithTarget.pathForDocumentId`; `OpenWithTarget` nicht gelesen). Alle Schemata außer `file` und `content` ergeben `null` (`:31`). Der Pfad muss `OpenWithTarget.acceptedPath(path, Storage.volumes(context))` bestehen (`:33`). Verzeichnis → der Ordner selbst; Datei → ihr Elternordner; sonst `Refused` (`:35-39`).
- Der Manifest-Filter (`AndroidManifest.xml:83-89`) nimmt nur die drei Verzeichnis-MIME-Typen an; Dateien werden bewusst nicht beansprucht (Kommentar `:80-82`).

#### 2.2.5 Öffnen einer URL

- `MainActivity.openUrl(url)` (`MainActivity.kt:96-102`): `Intent(Intent.ACTION_VIEW, Uri.parse(url))`; ohne Browser erscheint „Kein Browser gefunden, um die Anmeldung zu öffnen.“. Auslöser ist das Ereignis `CoreEvent.OpenUrl` (`MainActivity.kt:42-47`).

#### 2.2.6 Vollständige Liste der Intents, Provider und Navigationskanäle (gelistete Dateien)

- Intents mit Aktion: `ACTION_VIEW` (Datei öffnen, URL öffnen, eingehend Ordner öffnen); `ACTION_SEND` und `ACTION_SEND_MULTIPLE` (Teilen ausgehend, siehe 2.2.2; eingehend über `ShareIntentHandler`); `Intent.createChooser` (Titel „Öffnen mit“ beim Öffnen, kein Titel beim Teilen).
- Kein Intent: `ClipboardManager` (Kopieren).
- Provider: `LocalFileProvider` (Authority `<applicationId>.localfiles`) und `androidx.core.content.FileProvider` (Authority `<applicationId>.files`).
- Eigener Navigationskanal: Aktion `app.smartexplorer.android.action.NAVIGATE` mit den Extras `app.smartexplorer.android.extra.NAV_KIND`, `…NAV_VALUE` und `…NAV_TOKEN` (`ui/AppNav.kt:41-44`). Die Navigationsarten sind genau: `openLocation`, `selectTab`, `showTransfers`, `openJobConflicts`, `showShareRequests`, `showUpdate`, `showBackgroundSettings` (`ui/AppNav.kt:101-109`).

### 2.3 F3: Gerätemodell

#### 2.3.1 Persistenz-Wurzel

- `ShareProfiles` (`native/src/share/core/profiles.rs:38-69`), 13 Felder in dieser Reihenfolge: `storage_revision` (nicht serialisiert, `:40-41`); `schema_version` (`:42-43`; aktuelle Version `SHARE_PROFILE_VERSION = 8`, `:17`); `auto_connect` (Standard `true`, `:44-45`); `default_direct_exports` (`:46-47`); `auto_home_migrations` (`:48-50`); `direct_contacts` (`:51-52`); `direct_grants` (`:53-54`); `direct_requests` (`:55-56`); `direct_request_tombstones` (`:57-58`); `legacy_direct_requests` (`:59-60`); `legacy_direct_request_tombstones` (`:61-62`); `removed_direct_peers` (`:63-66`); `rooms` (`:67-68`).
- Laden: `ShareProfiles::load_checked(default_home())` (`native/src/mobile/os/shared/domains/share_peers.rs:17-18`); `default_home()` ist `crate::support_dirs::host()` (`native/src/mobile/os/shared/domains/share_state.rs:399-401`). **Lücke:** Dateiname und Ort des gespeicherten Profils (`support_dirs` nicht gelesen).
- Änderungen laufen über `ShareProfiles::mutate_persisted(default_home(), …)` (`share_peers.rs:223`, `:244`). `native/src/share/core/profile_settings.rs:1-4` beschreibt diese Transaktion als gemeinsamen Weg aller Oberflächen (CLI, Desktop, Android).

#### 2.3.2 Direkt-Kontakt (ausgehend: Zugriff dieses Geräts auf ein Gegengerät)

- `DirectContact` (`native/src/share/core/direct_relation.rs:101-135`), 21 Felder: `id`, `display_name`, `lookup_id`, `expected_fingerprint`, `expected_node_id`, `remote_device_id`, `remote_public_key`, `auto_connect`, `auto_open`, `last_seen`, `status` (`ShareStatus`), `last_error`, `presence` (`PeerPresence`), `access_state` (`DirectAccessState`), `request_sent_at`, `accepted_at`, `accepted_public_key`, `lan_candidates`, `lan_seen_at`, `lan_uplink`, `relation` (`DirectRelationFlags`).
- `DirectRelationFlags` (`direct_relation.rs:139-150`): `share_back` („Auch meine Freigaben für dieses Gerät öffnen“) und `signed_presence` (B03).
- `DirectAccessState` (`:14-19`), vollständig: `Pending`, `Accepted`, `Ignored`, `IdentityConflict`. Bezeichnungen (`label`, `:22-29`): „Warte auf Freigabe“, „Freigegeben“, „Ignoriert“, „Identitaetskonflikt“.
- `DirectContactRuntime` (`:165-170`): `contact_id`, `status`, `last_seen`, `last_error`, `presence`.

#### 2.3.3 Direkt-Freigabe an dieses Gerät (eingehend: Zugriff eines Geräts auf dieses)

- `DirectGrant` (`direct_relation.rs:64-80`), 9 Felder: `device_id`, `device_name`, `public_key`, `fingerprint`, `node_id`, `state` (`DirectGrantState`), `updated_at`, `exec` (`ExecGrant`), `write` („Darf schreiben“).
- `DirectGrantState` (`:49-61`), vollständig: `Accepted`, `Ignored`, `Reconfirm`.
- `DirectRequestPolicy` (`:155-160`), vollständig: `Ask` (Standard), `AutoAccept`.
- `DirectGrant::authorizes_session` (`:87-97`): Freigabe ist `Accepted` und stimmt in Gerät, Schlüssel und Node überein.

#### 2.3.4 Raum und Mitglied

- `RoomProfile`, `RoomMember`, `PeerPresence` und `ShareStatus` sind in `native/src/share/core/types.rs` definiert (Import in `native/src/share/core/room_relation_members.rs:7` und `native/src/share/core/direct_relation.rs:11`). Diese Datei ist nicht gelesen. Einstieg graphify (kein Beleg): `ShareStatus` Zeile 47, `PeerOpenTarget` Zeile 182, `ShareEvent` Zeile 365.
- Raum, Felder aus den gelesenen Stellen: `id` (Profil-ID, JSON `profileId`), `room_id` (JSON `roomId`), `name`, `status`, `auto_join` (JSON `autoJoin`), `members`, `policy` (`RoomPolicy`), `exports` (`relation_rights.rs:37`; `share_status.rs:288-292`). **Lücke:** die vollständige Felderliste von `RoomProfile`.
- `RoomPolicy` (`native/src/share/core/room_relation_members.rs:13-25`): `members_may_write` („Darf schreiben“; neue Räume `false`, `:29-34`; vor V5 gespeicherte Räume `true`, `:37-41`); `confirm_new_members` („Neue Mitglieder bestätigen“, B15).
- `RoomMemberAdmission` (`:45-53`), vollständig: `Admitted` (Standard), `Pending`.
- `RoomMemberFlags` (`:55-64`): `admission`, `signed_presence`.
- Mitglied, Felder aus den gelesenen Stellen: `device_id`, `device_name`, `public_key`, `node_id`, `fingerprint`, `status`, `blocked`, `relation.admission` (`native/src/mobile/os/shared/domains/share_status.rs:123-132`), `presence` und `status` (`profile_settings.rs:112-114`), `exec` mit `enabled` und `changed_at` (`room_relation_members.rs:124-146`). **Lücke:** die vollständige Felderliste von `RoomMember`.
- `RoomMember::is_admitted` (`room_relation_members.rs:69-71`): nicht gesperrt und `Admitted`. `RoomMember::authorizes_session` (`:75-85`): zugelassen und Gerät, Schlüssel und Node stimmen überein.
- `RoomProfile::set_member_blocked` (`:101-133`): Sperren schaltet das Exec des Mitglieds ab und setzt `confirm_new_members` auf `true`; Entsperren lässt ein wartendes Mitglied zu. `RoomProfile::admit_member` (`:136-149`) lässt ein wartendes Mitglied zu.

#### 2.3.5 Freigabe (Export)

- `ShareExportConfig` ist in `native/src/share/core/export_config.rs` definiert (Re-Export in `native/src/share/api_exports.rs:66-68` zusammen mit `ExportAccess`, `SharedConnection`, `SharedRoot`, `DIRECT_EXPORT_SCOPE`). Die Datei ist nicht gelesen.
- Gelesene Felder: `roots` (Liste); je Wurzel `label`, `path`, `access`, `allow_system_writes` (`share_status.rs:186-193`; `share_peers.rs:225-232`; `profile_settings.rs:129-131`). **Lücke:** die vollständige Variantenliste von `ExportAccess`. Belegt ist nur `ReadWrite` (Kommentar `relation_rights.rs:22-23`).
- Bereiche (Scope): der Wert `"direct"` bezeichnet `default_direct_exports` (`share_peers.rs:186-188`); eine Raum-Profil-ID bezeichnet `room.exports` (`share_peers.rs:189-193`). Die Konstante `DIRECT_EXPORT_SCOPE` (`api_exports.rs:67`) hat laut Name denselben Zweck; ihr Wert ist Lücke (Definition in `export_config.rs`, nicht gelesen).
- `add_export` (`share_peers.rs:206-239`): Argumente `scope` (Pflicht), `path` (Pflicht), `label` (optional). Der Pfad wird über `canonical_directory` kanonisiert und muss ein vorhandener Ordner sein (`:197-204`); `reject_app_internal(raw)` (`:209`). Ohne `label` ist die Bezeichnung der letzte Pfadabschnitt oder `Freigabe` (`:211-222`). Doppelte Wurzel: „Dieser Ordner ist bereits freigegeben.“ (`:225-227`).
- `remove_export` (`share_peers.rs:241-257`): Argumente `scope`, `path`; nicht gefunden: „Freigabe nicht gefunden.“ (`:249`).
- Reihenfolge der Freigaben: `move_export_root(scope, path, offset)` (`profile_settings.rs:118-141`) verschiebt eine Wurzel innerhalb von `scope` um `offset` Plätze. Die Zielposition wird auf den Bereich begrenzt; entspricht sie der Ausgangsposition, gibt die Funktion `false` zurück (`:133-137`).
- `connectionExports` und `writeGrants` entstehen aus `policy_status::connections(profiles)` und `policy_status::grants(profiles)` (`share_status.rs:327-328`). Das Modul `share_policy_status.rs` ist nicht gelesen.

#### 2.3.6 Präsenz und online

- `PeerPresence`: Typ in `native/src/share/core/types.rs` (nicht gelesen). Felder, die in den Daemon-Stellen vorkommen: `device_id`, `public_key`, `node_id`, `nonce` (`native/src/daemon/os/shared/ipc_host_legacy_events.rs:159-162`). **Lücke:** die vollständige Felderliste.
- `DirectContact.presence` (`direct_relation.rs:117-118`) wird gesetzt durch `apply_verified_presence` (Definition nicht gelesen; Aufruf `native/src/daemon/os/shared/ipc_host_relation_events.rs:59`). Gelöscht wird es durch `reset_contact_trust` (`profile_settings.rs:56`) und durch `DirectOffline` (`ipc_host_relation_events.rs:69`).
- `ShareStatus` (Typ in `core/types.rs`, nicht gelesen). Die Zustände, die `status_code` unterscheidet (`native/src/mobile/os/shared/domains/share_status.rs:58-70`), vollständig: `Offline` → `offline`; `Waiting` → `waiting`; `WaitingForAccess` → `waitingForAccess`; `Available` → `available`; `Connecting` → `connecting`; `Connected` → `connected`; `ConnectedDirect` → `connectedDirect`; `ConnectedRelay` → `connectedRelay`; `Failed(_)` → `failed`; `IdentityConflict` → `identityConflict`. Die Anzeigebezeichnungen (`label()`) sind nicht gelesen.
- Erreichbar (`reachable`, `share_status.rs:73-81`): `Available`, `Connected`, `ConnectedDirect`, `ConnectedRelay`.
- LAN-Sicht (`seen_on_lan`, `share_status.rs:83-88`): `lan_seen_at` liegt innerhalb von `LAN_PRESENCE_TTL_SECS` und `lan_candidates` ist nicht leer. **Lücke:** der Wert von `LAN_PRESENCE_TTL_SECS` (Re-Export `api_exports.rs:72`; Definition in `native/src/share/core/lan_presence_match.rs`, nicht gelesen).
- Gerät im JSON: `online` = erreichbar oder LAN-Sicht (`share_status.rs:100`); `status` = Code, `statusText` = Bezeichnung (`:98-99`).
- Präsenz-Ereignisse im Daemon (`native/src/daemon/os/shared/ipc_host_relation_events.rs`, per Grep; acht `ShareEvent`-Varianten werden dort behandelt):
  - `DirectAvailable` (`:51-61`): verifizierte Präsenz über `apply_verified_presence`; Zuordnung über `lookup_id`.
  - `DirectOffline` (`:62-72`): Status `Offline`, Präsenz `None`.
  - `DirectAccessAccepted` (`:73`): Wirkung in dieser Lektüre nicht vollständig gelesen.
  - `RoomRoster` (`:98`): Raum-Status `Available` (`:109`).
  - `RoomJoined` (`:118`): Wirkung in dieser Lektüre nicht gelesen.
  - `RoomLeft` (`:131`): Mitglied-Status `Offline` (`:142`).
  - `LanPeerSeen` (`:149`): Status `Available`, wenn der Kontakt vorher `Offline` oder `Waiting` war (`:163-166`).
  - `LanPeerLost` (`:171`): Status `Offline`, wenn der Kontakt vorher `Available` war und keine Server-Präsenz vorliegt (`:184-185`).

#### 2.3.7 Automatisch verbinden und automatisch öffnen

- Kontakt: `auto_connect` und `auto_open` (`direct_relation.rs:110-111`). Setzen: `ShareProfiles::set_contact_options(contact_id, auto_connect, auto_open)` (`profile_settings.rs:11-30`); `None` lässt den Wert unverändert; die Rückgabe ist `true`, wenn sich ein Wert ändert.
- Global: `ShareProfiles.auto_connect` (`profiles.rs:44-45`, Standard `true`).
- Raum: `auto_join` (JSON `autoJoin`, `share_status.rs:141`). Setzen über `set_room_auto_join(profile_id, auto_join)` (`profile_settings.rs:78-94`); Verlassen setzt den Raum auf `Offline` (`:90-92`).
- JSON des Geräts: `autoConnect` und `autoOpen` (`share_status.rs:105-106`).
- Android-Aktion „Verbindung…“ (`android/app/src/main/java/app/smartexplorer/android/ui/share/SharePeers.kt:95`; Kommentar `:47`: „connect/open automatically, reset trust“) ruft `actions.connection(device)` auf. **Lücke:** die Bridge-Funktion dieser Aktion ist in `share_peers.rs` nicht enthalten.

#### 2.3.8 Rechte-Sicht je Beziehung (Restriktionen)

- `SessionAuthorization` (`native/src/share/core/relation_rights.rs:15-41`): `exports` und `may_write`. `may_write` ist bei Direktbeziehungen `DirectGrant.write`, bei Räumen `RoomPolicy.members_may_write` (`:21-23`, `:35-39`).
- `RelationScope` (`:43-51`), vollständig: `Direct`, `Room { room_id }`.
- `PrincipalKey` (`:56-67`): `public_key` und `node_id`; passt bei Gleichheit eines der beiden. Leere Werte passen nie (`:62-66`).
- `RestrictionReason` (`:69-90`), vollständig: `Removed`, `Blocked`, `Reconfirm`, `IdentityChanged`, `WriteRevoked`, `ExecRevoked`, `ExportsNarrowed`, `RelationInactive`, `Unattributed`.
- `RightsRestriction` (`:92-119`): `relation`, `principal` (optional), `reason`; `affects(relation_kind, relation_id, public_key, node_id)` (`:101-118`).
- `RestrictionSet` (`:121-128`): `everything` (alles schließen) und `items`.

#### 2.3.9 Gerätekarten und Raumzeilen (Android-Darstellung)

- `SharePeers.kt` (`android/app/src/main/java/app/smartexplorer/android/ui/share/SharePeers.kt`): `PeerTarget(name, location)` (`:31-32`); `PeerActions` (`:34-52`) mit 15 Callbacks: `open`, `sendFiles`, `exec`, `requestAccess`, `removeDevice`, `showRoomCode`, `leaveRoom`, `removeRoom`, `shareBack`, `roomPolicy`, `roomMember`, `connection`, `renameRoom`, `rejoinRoom`, `resetMemberTrust`.
- Gerätezeile `DeviceRow` (`:66-102`): Name (Fallback „Gerät“); Untertitel aus `statusText` oder `status`, dazu „LAN“, „Rückfreigabe an“ oder „Keine neue Rückfreigabe“, „Schreibrecht“ oder „Nur lesen“, „Verbindet nicht automatisch“ (bei `autoConnect` falsch) und „Öffnet automatisch“ (bei `autoOpen` wahr) (`:69-76`). Antippen öffnet `device.location`, wenn er nicht leer ist (`:100`). Menü (`:88-98`), vollständig: „Dateien öffnen“, „Datei senden“, „Befehl ausführen“, „Zugriff anfragen“, „Eigene Freigaben…“, „Verbindung…“, „Entfernen“ (sieben Einträge).
- `RoomsSection` (`:106-113`) und `RoomBlock` (`:116-165`): Antippen öffnet `room.location`, wenn er nicht `null` ist; sonst klappt die Mitgliederliste auf (`:152-158`). Untertitel: Zustand, Anzahl Mitglieder, Zahl der wartenden Mitglieder („warten auf Zustimmung“), Rechte (`:121-128`). Menü (`:141-148`), vollständig: „Code anzeigen“, „Rechte und Bestätigung…“, „Umbenennen…“, „Verlassen“ oder „Wieder beitreten“, „Entfernen“.
- `MemberRow` (`:168-194`): Antippen öffnet `member.location`, wenn das Mitglied nutzbar ist (`:172`, `:192`). Menü (`:178-190`): „Dateien öffnen“, „Datei senden“ und „Befehl ausführen“ nur bei nutzbarem Mitglied; je nach Zustand „Zulassen…“, „Wieder zulassen…“ oder „Sperren…“; immer „Vertrauen zurücksetzen“.
- `ShareDeviceCard.kt` (`android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDeviceCard.kt`): `ThisDeviceCard` (`:47-129`) zeigt das eigene Gerät. Eine Öffnen-Aktion gibt es dort nicht (Suche nach `open`, `Öffnen` und `location` ohne Treffer). Die Online-Zeile stammt aus `onlineText` (`:132-138`): „Offline“, „Online – nur LAN“, „Online über Relay“, „Online“, „Offline – Server nicht erreichbar“. Farbe: `onlineColor` (`:141-146`).

#### 2.3.10 Statusdaten der Bridge (JSON)

- `device_json` (`share_status.rs:90-108`), 11 Schlüssel: `contactId`, `name`, `status`, `statusText`, `online`, `location`, `lan`, `shareBack`, `write`, `autoConnect`, `autoOpen`.
- `rooms_json` (`share_status.rs:110-151`): je Raum 8 Schlüssel: `profileId`, `roomId`, `name`, `status`, `autoJoin`, `location` (immer `null`), `members`, `policy` (mit `membersMayWrite` und `confirmNewMembers`). Je Mitglied 9 Schlüssel: `deviceId`, `name`, `publicKey`, `nodeId`, `fingerprint`, `status`, `location`, `blocked`, `admission`.
- `status_json` (`share_status.rs:262-334`), 20 Schlüssel: `running`, `connected`, `relayUrl`, `lastError`, `server`, `lanPresence` (Wert `"aus"`, wenn leer; `:303-307`), `identity` (mit `deviceId`, `deviceName`, `fingerprint`, `directCode`), `devices`, `execProvider`, `execTargets`, `rooms`, `incoming`, `outgoing`, `exports` (mit `direct` und `rooms`), `connectionExports`, `writeGrants`, `autoHomeMigrations`, `discovery`, `removedDevices` (je Eintrag `deviceId`, `name`), `notices`.
- `devices` in `status_json` ist die Liste aller `direct_contacts`, abgebildet über `device_json` (`share_status.rs:274-278`).
- Antworten der Funktionen in `share_peers.rs` (vollständig, soweit gelesen):
  - `add_direct` → `contactId`, `shareBack` (`:86`);
  - `create_room` → `profileId`, `code` (`:120`);
  - `join_room` → `profileId` (`:131`);
  - `room_code` → `code` (`:143`);
  - `remove_device` und `remove_room` → `removedFavorites`, `orphanedJobs` (über `finish_removal`, `:44-47`; Aufrufe `:101`, `:179`);
  - `readmit`, `leave_room`, `add_export`, `remove_export` → `{}` (`:109`, `:168`, `:238`, `:256`).
- Fehlerarten, die `share_peers.rs` über `ApiError::new` setzt: `internal` (`:14`), `invalid` (`:61`, `:128`), `not_found` (`:97`, `:141`, `:156`, `:175`, `:253`), `network` (`:167`).
- **Lücke:** die Bridge-Methodennamen (Dispatch-Tabelle, nicht gelesen). Einziger Beleg für die Schreibweise: die Fehlermeldung in `add_direct` nennt `share.setShareBack` (`share_peers.rs:81`).

#### 2.3.11 Ort eines Geräts und Locator-Form

- Der Ort eines Geräts ist der Wert `location` im Status-JSON. Für einen Direktkontakt ist es `PeerOpenTarget::Direct { contact_id }.endpoint_prefix()` (`share_status.rs:92-101`); für ein Raummitglied `PeerOpenTarget::RoomDevice { room_id, device_id }.endpoint_prefix()` (`share_status.rs:119-130`). Ein Raum hat keinen Ort (`location` ist `null`, `share_status.rs:142`).
- `PeerOpenTarget` hat genau zwei Varianten: `Direct { contact_id }` und `RoomDevice { room_id, device_id }` (`native/src/app/core/mount_peer_roots.rs:228-242`).
- **Lücke:** das Präfix selbst (Wert von `endpoint_prefix()` und Syntax von `from_endpoint()`). Die Definition liegt laut Re-Export (`native/src/share/api_exports.rs:101-106`) im Typmodul `types`; der graphify-Einstieg nennt `native/src/share/core/types.rs:182`. Nicht gelesen.
- Zusammensetzung eines Pfads am Gerät (`native/src/mobile/os/shared/domains/locations.rs:12-37`, `location_for(base, path)`), vollständig:
  1. Basis ohne `://` (`is_local`, `:8-10`): der Pfad selbst (`:14-16`);
  2. Basis `gdrive://`: `crate::connect::gdrive_endpoint(&path)` (`:22-24`);
  3. Basis ist ein Peer-Endpunkt (`PeerOpenTarget::from_endpoint(base)`): `endpoint_prefix()` plus Pfad (`:25-27`);
  4. sonst `proto://user@host:port`, unverändert, mit angehängtem Pfad (`:30-35`).
- Peer-Wurzeln am Desktop (`native/src/app/core/mount_peer_roots.rs`): `PEER_CONNECTIONS_ROOT = "/Verbindungen"` (`:4`). Die Wurzeln sind `/<Name>` für jeden Ordner der obersten Ebene des Peers, der kein Symlink ist, außer dem Ordner `Verbindungen`; dessen Unterordner ergeben `/Verbindungen/<Verbindung>` mit der Bezeichnung „Verbindung: <Verbindung>“. Dazu kommt `/` mit „Alle freigegebenen Bereiche (nur Lesen)“ (`:144-207`). Namen müssen `safe_virtual_segment` bestehen (`:219-226`).

#### 2.3.12 Öffnen als Backend (`open_share_backend`)

- Desktop-Aufrufkette, belegt:
  1. `discover_peer_mount(open_target, label)` (`native/src/app/core/mount_peer_roots.rs:35-58`) ruft `crate::daemon::open_share_backend(open_target.clone())` auf (`:40`).
  2. `open_share_backend(target: PeerOpenTarget) -> Result<(String, BackendHandle, ShareStatus), String>` (`native/src/daemon/os/shared/ipc_client.rs:22-24`): stellt den Worker bereit (`ensure_worker_ready`), liest das Token, verbindet sich per TCP mit 2 s Zeitlimit, sendet `IpcRequest::OpenShare { token, target }` (`:36-39`), erwartet `IpcResponse::OpenOk { label, status }` (`:54`) und kapselt den Strom in `AgentBackend::from_streams` mit `share_backend_identity(label, target)` (`:57-61`).
  3. Bis zu acht Versuche (`:30`); Neustarts nach Agent-Fehler oder fehlendem IPC (Flags `:28-29`).
  4. Host-Seite: `IpcRequest::OpenShare { target, .. }` ruft `host.open_share(target)` auf (`native/src/daemon/os/shared/ipc.rs:85-89`; Definition `native/src/daemon/os/shared/ipc_host.rs:355`, per Grep).
- Ohne IPC-Umweg im selben Prozess: `native/src/daemon/os/shared/mount_source.rs:47-49` ruft `host.open_share(target)` direkt auf. Der Kommentar `:47` nennt `open_share_backend` als den Weg, der eine Loopback-IPC-Verbindung zurück in denselben Prozess erzeugen würde (Grep-Treffer).
- Wurzelauswahl: `peer_roots` (`mount_peer_roots.rs:144-174`) listet `/`; voreingestellt ist die erste Wurzel, die nicht `/` ist, sonst Index 0 (`:42-45`). Danach `probe_peer_safety` (`:49`; Funktion `:91`).
- Mount-Ziel: `crate::mount::PeerMountTarget` mit denselben zwei Varianten (`mount_peer_roots.rs:228-242`; Typ nicht gelesen).
- Android: Die Öffnen-Aktion übergibt `device.location` als Zeichenkette (`SharePeers.kt:90`, `:100`). Die Auflösung dieser Zeichenkette zu einem Backend liegt im Rust-Kern hinter `FilesApi.list` (`ui/files/BrowserTab.kt:199`; Gegenstück nicht gelesen). **Lücke:** der Weg von `location` zu einem Backend auf Android ist in den gelisteten Dateien nicht belegt.

#### 2.3.13 Wie die GUI von online und offline erfährt

- Android, Rust-Seite (`native/src/mobile/os/shared/domains/share_state.rs`):
  - Abfrageschleife `poll_once` (`:341-390`): liest die Ereignisse des Worker-Prozesses über `crate::daemon::drain_share_events_in_process()` (`:344`); wendet den Schnappschuss an (`state.apply`, `:354`); berechnet das Status-JSON neu (`status_value`, `:369`).
  - Ohne aktives Beobachten (`cadence().watch` falsch) entfernt `share_power::strip_volatile` flüchtige Felder vor dem Vergleich (`:370-372`). **Lücke:** welche Felder das sind (Modul `share_power` nicht gelesen).
  - Bei Änderung des Status-JSON oder der Ausführungsaktivität: `rt.emit({"type": "share"})` (`:374-386`). Bei neuen offenen Anfragen: `rt.emit({"type": "shareRequest", "count": N})` (`:387-389`).
  - Abfrageintervall (`poll_interval`, `:285-295`): `watch` oder `pairing` → `WATCH_INTERVAL`; Aktivitätsfenster (`activity_until`) → `ACTIVITY_INTERVAL`; `foreground` → `FOREGROUND_INTERVAL`; sonst `BACKGROUND_INTERVAL`.
  - Konstanten (`:28-35`): `WATCH_INTERVAL` 300 ms (`:28`), `FOREGROUND_INTERVAL` 5 s (`:29`), `BACKGROUND_INTERVAL` 60 s (`:30`), `ACTIVITY_INTERVAL` 1 s (`:34`), `ACTIVITY_FOLLOW_UP` 3 s (`:35`).
  - `set_watch` (`:263-268`) und `set_foreground` (`:270-273`) steuern das Intervall; `wake` und `wake_for_activity` (`:258-283`) erzwingen eine Abfrage.
  - Auf Anforderung liefert `status()` (`:392-395`) das Status-JSON.
- Android, Kotlin-Seite: Das Ereignis `"share"` wird in den gelisteten Dateien nicht verarbeitet. Sichtbar sind in `ui/AppRoot.kt:116-122` die Zweige `CoreEvent.Error` (`:117-118`) und `CoreEvent.ShareRequest` mit `count` (`:119-121`); zusätzlich `CoreEvent.OpenUrl` in `MainActivity.kt:42-47`. **Lücke:** die Definition von `CoreEvent` und der Verbraucher des Ereignisses `"share"`.
- Desktop: Die Ereignisse des Worker-Prozesses werden im Daemon auf Relationen angewandt (`native/src/daemon/os/shared/ipc_host_relation_events.rs:45-185`, Grep-Treffer). Exportiert sind `drain_share_worker_events` und `share_worker_snapshot` (`native/src/daemon/mod.rs:207-212`). **Lücke:** der Verbraucher dieser Schnittstellen in der Desktop-GUI ist nicht gelesen.

### 2.4 F4: Orte (Geräte und Räume als Ordner)

#### 2.4.1 Desktop-Seitenleiste

Datei `native/src/app/core/sidebar_locations.rs` (87 Zeilen). Die Methode `ui_sidebar_locations` (`:5-66`) zeigt in dieser Reihenfolge:

1. „Startseite“ (`:6-9`), ohne Abschnittsüberschrift; öffnet die Startseite.
2. Abschnitt „FAVORITEN“ (`:11-27`), nur bei vorhandenen Favoriten; je Favorit eine Zeile und ein „×“ zum Entfernen (`:21-22`).
3. Abschnitt „SCHNELLZUGRIFF“ (`:29-39`) mit genau sieben Zeilen: „Persönlicher Ordner“, „Desktop“, „Dokumente“, „Downloads“, „Bilder“, „Musik“, „Videos“ (`:31-35`).
4. Abschnitt „LAUFWERKE“ (`:41-52`), nur bei nicht leerer `drive_info`; je Laufwerk eine Zeile mit Fortschrittsbalken (frei von gesamt).
5. Abschnitt „ZULETZT“ (`:54-65`), nur bei nicht leerer `recent`; einklappbar; höchstens fünf Einträge (`:58`, `take(5)`).

- In dieser Datei gibt es keine Zeile für Geräte oder Räume. Die Suche nach `share`, `peer`, `Gerät`, `Geräte`, `Raum`, `device`, `verbind` und `connection` ergab keinen Treffer.
- Die Wurzeln eines Peers entstehen in `native/src/app/core/mount_peer_roots.rs` (siehe 2.3.11).
- **Lücke:** an welcher Stelle der Desktop-App diese Peer-Wurzeln angeboten werden und ob eingebundene Peers in der Seitenleiste erscheinen, liegt außerhalb der gelisteten Dateien.

#### 2.4.2 Android: PlacesPanel

Datei `android/app/src/main/java/app/smartexplorer/android/ui/files/PlacesPanel.kt`, Funktion `PlacesPanel` (`:55-92`). Die Liste ist eine einzige `LazyColumn` mit dem Testtag `places-list`. Abschnitte in dieser Reihenfolge, vollständig (`:57-90`):

1. Eintrag „Ordner suchen“ (`:57-59`), ohne Überschrift;
2. Fehlerkarte „Orte nicht geladen“ mit „Erneut“ (`:60-64`), nur bei Fehler;
3. „Speicher“ (`roots.storage`, `:66`);
4. „Favoriten“ (`roots.favorites`, `:67-70`); langes Antippen öffnet „Entfernen“ (`:117-138`);
5. „Zuletzt“ (`roots.recent`, höchstens `MAX_RECENT` = 10, `:71`, `:34-35`);
6. „Verbindungen“ (`roots.connections`, `:72-73`) mit „Verbindung hinzufügen“ (`:74-76`);
7. „Google Drive“ (`roots.gdrive`, `:77-85`); ohne Eintrag der Platzhalter „Nicht eingerichtet – unter Mehr einrichten“;
8. „Share“ (`roots.devices + roots.rooms`, `:86`);
9. Trennlinie und „Papierkorb“ (`roots.trash`, `:87-90`).

- Geräte und Räume erscheinen im Abschnitt „Share“ als eine gemeinsame Liste (`:86`). Eine eigene Überschrift „Räume“ gibt es in `PlacesPanel` nicht.
- Jede Zeile ist ein `Root` mit den Feldern `id`, `label`, `subtitle` und `location` (`RootItem`, `:112-114`). Ausgewählt ist eine Zeile, wenn `root.location == current` gilt (`:113`, `:124`); `current` ist `vm.activeTab?.listing?.location` (`FilesScreen.kt:176`).
- Antippen: `callbacks.onOpen(root)` ruft `vm.openRoot(root)` auf und schließt die Schublade (`FilesScreen.kt:156-160`).
- Datentypen: `Roots` (importiert aus `app.smartexplorer.android.api`, `PlacesPanel.kt:28`) und `Root` (aus `app.smartexplorer.android.core`, `:29`). **Lücke:** Felder, JSON-Schlüssel und Erzeuger beider Typen liegen außerhalb der gelisteten Dateien.
- **Annahme:** `Roots.devices` und `Roots.rooms` entstehen aus den Geräte- und Raumdaten des Status-JSON (2.3.10). Belegt ist nur die Verwendung der Namen in `PlacesPanel.kt:86`.

#### 2.4.3 Daten der Bridge für Orte und Teilen

- Für den Abschnitt „Share“ liegen in den gelisteten Dateien nur die Status-Daten vor (Abschnitt 2.3.10): `devices` (Liste aus `device_json`, 11 Schlüssel je Gerät), `rooms` (`rooms_json`, 8 Schlüssel je Raum, 9 Schlüssel je Mitglied) und `exports` (`direct`, `rooms`).
- Der Wert `location` je Gerät und je Mitglied ist der Locator aus 2.3.11. Der Wert `location` eines Raums ist `null`.
- Der Reiter „Teilen“ zeigt Geräte und Räume in den Abschnitten „Geräte“ und „Räume“ (`android/app/src/main/java/app/smartexplorer/android/ui/share/SharePeers.kt:54-63`, `:104-113`). **Annahme:** die Listen stammen aus dem Status-JSON. Die Datei `ui/share/ShareScreen.kt`, die sie übergibt, ist nicht gelesen.
- Die Abschnitte Speicher, Favoriten, Zuletzt, Verbindungen, Google Drive und Papierkorb sowie ihre Datenquelle sind in den gelisteten Dateien nicht belegt. Die Abschnitte Ordner suchen und Fehlerkarte werden nicht aus `roots` gespeist (`PlacesPanel.kt:57-64`).
- **Lücke:** Erzeuger und JSON-Schlüssel von `Roots` und `Root` (Bridge-Funktion und Felder). Die Dateien dafür liegen im Paket `android/app/src/main/java/app/smartexplorer/android/api/` und in der Rust-Bridge-Dispatch-Stelle; keine davon ist gelesen.
- Desktop: Die Seitenleiste bezieht keine Bridge-Daten für Geräte und Räume (2.4.1). Der Desktop-Zugriff auf Peers läuft über den Daemon (2.3.12).

## 3. Vorhandene Bausteine (Bestand, ohne Bewertung der Wiederverwendung)

Android (Kotlin, `android/app/src/main/java/app/smartexplorer/android/`):
- `system/LocalFileProvider.kt`: Inhalts-Provider für lokale Dateien unterhalb der gemeldeten Volumes, nur über URI-Freigaben (2.2.1).
- `system/Opener.kt`: Öffnen, Teilen und Kopieren mit den Ergebnisarten `Started`, `NoApp`, `NotShareable` (2.2.1, 2.2.2).
- `system/OpenWithIntent.kt`: Auswertung eingehender Ordner-Anfragen per `ACTION_VIEW` (2.2.4).
- `ui/AppNav.kt`: Navigationsanfragen mit einem Token je Installation; Enum `MainTab` mit den Werten `Files`, `Sync`, `Share`, `More` (`:14`).
- `ui/AppRoot.kt`: Startzustand (Einrichtung, Startfehler, Start) und Hauptgerüst mit Navigationsleiste unter 840 dp und Navigationsschiene ab 840 dp (`:62-80`, `:99-100`, `:147-158`; die Breite 840 dp steht im Kommentar `:99`).
- `ui/files/PlacesPanel.kt`: Abschnittsliste der Orte mit `PlacesCallbacks` (`:40-48`).
- `ui/files/FilesScreen.kt`: Effektverarbeitung `perform` (`:431-451`), Anbindung des Medienbetrachters (`:135-148`), Ordnersuche (`:126-130`).
- `ui/share/SharePeers.kt`: Geräte-, Raum- und Mitgliederzeilen mit Aktionsmenüs; `PeerActions` (`:34-52`).
- `ui/share/ShareDeviceCard.kt`: Karte des eigenen Geräts mit Online-Text (`onlineText`, `:132-138`).
- `prefs/AppPrefs.kt`: Einstellungen als `StateFlow` über `SharedPreferences` mit dem Dateinamen `app_prefs` (`:15`).

Rust (`native/src/`):
- `mobile/os/shared/domains/share_status.rs`: Projektion des Status als JSON, nach eigenem Kommentar ohne I/O (`:1-2`; 2.3.10).
- `mobile/os/shared/domains/share_peers.rs`: Share-Funktionen der Android-Bridge: `add_direct`, `remove_device`, `readmit`, `create_room`, `join_room`, `room_code`, `leave_room`, `remove_room`, `add_export`, `remove_export` (2.3.10).
- `mobile/os/shared/domains/share_state.rs`: Abfrageschleife, Intervallsteuerung und Ereignisse (2.3.13).
- `mobile/os/shared/domains/locations.rs`: Zusammensetzung von Orts-Zeichenketten (2.3.11).
- `share/core/profile_settings.rs`: Einstellungsänderungen über die gemeinsame Profiltransaktion (`:1-4`). Funktionen: `set_contact_options` (`:11`), `reset_contact_trust` (`:35`), `rename_room` (`:64`), `set_room_auto_join` (`:78`), `reset_member_trust` (`:98`), `move_export_root` (`:121`).
- `share/core/room_relation_members.rs`: Raumregeln, Mitgliederzulassung und Sperren (2.3.4).
- `share/core/relation_rights.rs`: Rechte-Sicht und Restriktionen je Beziehung (2.3.8).
- `share/core/direct_relation.rs`: Typen für Direktkontakt und Direkt-Freigabe (2.3.2, 2.3.3).
- `share/api_exports.rs`: öffentliche Fassade mit Re-Exports (Liste in Abschnitt 4.7).
- `app/core/mount_peer_roots.rs`: Peer-Wurzelauswahl und Sicherheitsprüfung der Einbindung (2.3.11, 2.3.12).
- `app/core/sidebar_locations.rs`: Zeilen der Desktop-Seitenleiste (2.4.1).
- `daemon/os/shared/ipc_client.rs` (`open_share_backend`), `daemon/os/shared/ipc.rs` (`OpenShare`), `daemon/os/shared/ipc_host_relation_events.rs` (Präsenz-Ereignisse), `daemon/os/shared/mount_source.rs` (Quelle im selben Prozess) (2.3.12, 2.3.13).

Build und Suite (nur Inventar; nicht ausgeführt):
- `android/build-release-apk.sh`: Bauen, Signieren und Prüfen der APK (2.1.5).
- `android/test-android-task.sh`: Stufen der Android-Task-Suite (2.1.6).
- `docs/refs/android-gradle-build.md`: Versionsvorlage mit widersprüchlichen Ständen (2.1.2).

## 4. Betroffene Verträge

### 4.1 Manifest (Android-Komponenten)

- Die einzige Komponente, die das Manifest mit `exported="true"` deklariert, ist `.MainActivity` (`AndroidManifest.xml:61-65`) mit den Intent-Filtern MAIN/LAUNCHER, SEND, SEND_MULTIPLE und VIEW (2.1.3). Der WorkManager-Dienst (`:110-113`) trägt kein `exported`-Attribut; sein Wert ergibt sich aus der eingebundenen Bibliothek und ist nicht gelesen.
- Der Navigationsintent wird nur mit dem Token dieser Installation angenommen (`ui/AppNav.kt:93-100`; Begründung im Kommentar `:93-96`).
- Provider: `${applicationId}.localfiles` (nicht exportiert, `grantUriPermissions`) und `${applicationId}.files` (FileProvider mit `@xml/file_paths`).
- Dienste (3), Empfänger (4) und Berechtigungen (16) sind vollständig in 2.1.3 aufgeführt.
- Heute fehlt: ein HOME-Filter; ein Schalter für Komponenten; ein Widget-Eintrag (2.1.3, 2.1.4).

### 4.2 Bridge (Share, Android)

- Schreibweise der Methodennamen: `share.<Methode>` (Beleg: Fehlermeldung in `share_peers.rs:81`: `share.setShareBack`). Die vollständige Zuordnung ist Lücke (Dispatch-Tabelle nicht gelesen).
- Antwort-JSON: `device_json` (11 Schlüssel), `rooms_json` (8 Raum-Schlüssel, 9 Mitglied-Schlüssel), `status_json` (20 Schlüssel), Antworten der Funktionen (vollständig in 2.3.10).
- Fehlerarten: `internal`, `invalid`, `not_found`, `network` (2.3.10).
- Ereignisse an die Oberfläche: `{"type": "share"}` und `{"type": "shareRequest", "count": N}` (`share_state.rs:385`, `:388`).

### 4.3 Locator (Orts-Zeichenkette)

- Lokal: Pfad ohne `://` (`locations.rs:8-10`).
- Google Drive: `gdrive://…` über `crate::connect::gdrive_endpoint` (`locations.rs:22-24`).
- Peer-Endpunkt: Präfix aus `PeerOpenTarget::endpoint_prefix()` plus Pfad (`locations.rs:25-27`). Das Präfix ist Lücke.
- Übrige Verbindungen: `proto://user@host:port` plus Pfad (`locations.rs:30-35`).
- Raum: `location` ist `null` (`share_status.rs:142`).
- Desktop-Peer-Wurzeln: `/`, `/<Name>`, `/Verbindungen/<Verbindung>` (`mount_peer_roots.rs:181-207`).
- Vergleich der Auswahl: `root.location == current` mit `current = vm.activeTab?.listing?.location` (`PlacesPanel.kt:113`, `:124`; `FilesScreen.kt:176`). Der Vergleich setzt voraus, dass beide Seiten dieselbe Form haben (belegt durch die Verwendung, nicht durch einen Test).
- Ein Direktgerät und ein Raummitglied haben je einen Locator. Beide Formen entstehen aus `PeerOpenTarget`, und zwar aus `Direct { contact_id }` oder aus `RoomDevice { room_id, device_id }` (`mount_peer_roots.rs:228-242`). Ein Raumprofil hat keinen Locator (`null`).

### 4.4 Persistenz

- `ShareProfiles` mit `schema_version` 8 (`profiles.rs:17`) und den 12 gespeicherten Feldern sowie `storage_revision` (nicht gespeichert) (2.3.1). Änderungen über `mutate_persisted` (`share_peers.rs:223`, `:244`).
- Android-Einstellungen, `SharedPreferences` „app_prefs“, 17 Schlüssel (`AppPrefs.kt:167-183`): `theme`, `show_hidden`, `dirs_first`, `compact`, `thumbnails`, `media_viewer`, `bg_mode`, `bg_interval_min`, `bg_wifi_only`, `bg_charging_only`, `bg_battery_not_low`, `auto_update_check`, `last_update_check_ms`, `onboarding_done`, `share_reachable`, `share_running`, `bg_start_refused_ms`.
- Navigationstoken: `SharedPreferences` „nav“, Schlüssel „token“ (`ui/AppNav.kt:45-46`, `:133-138`).
- Share-Serverdatei: Dateiname `share_server.txt` (`share_state.rs:37`), Pfad über `server_path()` (`share_state.rs:412`).

### 4.5 GUI

- Android: `CoreEvent.Error`, `CoreEvent.ShareRequest` (mit `count`) und `CoreEvent.OpenUrl` (`ui/AppRoot.kt:116-122`, `MainActivity.kt:42-47`). Definition von `CoreEvent`: Lücke.
- Android: Status-Abfrage über `status()` (`share_state.rs:392-395`); Änderungsereignis `"share"` (2.3.13).
- Desktop: Daemon-Schnittstellen `drain_share_worker_events` und `share_worker_snapshot` (`daemon/mod.rs:207-212`); Schnappschuss-Felder laut Verwendung `running`, `connected`, `relay_url`, `last_error`, `lan.presence` (`share_status.rs:31-37`).

### 4.6 IPC (Desktop zu Worker)

- Anfrage `IpcRequest::OpenShare { token, target }` (`daemon/os/shared/ipc_client.rs:36-39`).
- Antwort `IpcResponse::OpenOk { label, status }` (`ipc_client.rs:54`).
- Host: `IpcRequest::OpenShare { target, .. }` ruft `host.open_share(target)` auf (`daemon/os/shared/ipc.rs:85-89`; `ipc_host.rs:355`).

### 4.7 Öffentliche Fassade (`native/src/share/api_exports.rs`)

- Vom Bericht genannte Typen, die die Fassade exportiert: `DirectContact`, `DirectGrant`, `DirectGrantState`, `DirectRelationFlags`, `DirectRequestPolicy`, `DirectAccessState`, `PeerOpenTarget`, `PeerPresence`, `RoomMember`, `RoomMemberAdmission`, `RoomMemberFlags`, `RoomPolicy`, `RoomProfile`, `ShareEvent`, `ShareStatus` (`:101-106`); `ShareExportConfig`, `SharedRoot`, `SharedConnection`, `ExportAccess`, `DIRECT_EXPORT_SCOPE` (`:66-68`); `ShareProfiles` (`:94`).
- Funktionen in den Zeilen `:108-146`, vollständig: `core_now_secs` (`:108`), `exec_provider_status` (`:112`), `exec_audit_tail` (`:117`), `exec_audit_entries` (`:125`), `exec_run_as_supported` (`:131`), `run_exec_supervisor_if_requested` (`:137`), `run_exec_platform_self_test` (nur mit `debug_assertions`, `:143-146`). Die übrigen Re-Exports in den Zeilen `:2-106` sind in diesem Bericht nicht aufgeführt.

## 5. Lücken, offene Punkte und Annahmen

### 5.1 Lücken (Fakten, die diese Lesung nicht belegt)

1. Versionen der `libs.*`-Einträge (Compose-BOM, AndroidX-Bibliotheken, Kotlin-Plugins, AGP): `android/gradle/libs.versions.toml` nicht gelesen. Die Dokumentation widerspricht sich bei Compose-BOM, core-ktx, lifecycle und material3.adaptive (2.1.2).
2. Gradle-Version des Wrappers: `android/gradle/wrapper/gradle-wrapper.properties` nicht gelesen.
3. NDK-Version: `android/ndk-version` nicht gelesen.
4. Zertifikat-Fingerabdruck: `android/release-cert.sha256` nicht gelesen.
5. Android-Ressourcen: `android/app/src/main/res/xml/file_paths.xml`, `android/app/src/main/res/xml/data_extraction_rules.xml`, `android/app/proguard-rules.pro` nicht gelesen.
6. CI-Verdrahtung: `.github/workflows/build.yml` (Job `android-release-apk`), `.github/workflows/android-task.yml` (Jobs der Suite) und `native/test-review-task.sh` nicht gelesen.
7. Release-Skripte: `native/publish-release-local.ps1` und `native/release-publication.ps1` nicht gelesen.
8. Dienststart und Dienstende: nicht gelesen sind die Klassen, die Dienste starten oder beenden. Genannt sind: `service/BackgroundController` (aufgerufen in `SmartExplorerApp.kt:26`), `system/BootReceiver`, `system/KeepAliveReceiver` und `system/SyncScheduleReceiver` (Manifest `:121-134`).
9. Kotlin-Vertragstypen nicht gelesen: Paket `app.smartexplorer.android.api` (`Roots`, `Listing`, `FilesApi`, `ShareDevice`, `ShareRoom`, `ShareMember`, `ShareStatus`, `ShareServerInfo`), Paket `app.smartexplorer.android.core` (`Core`, `CoreEvent`, `CoreException`, `CoreState`, `Entry`, `FilterSpec`, `Root`, `TaskInfo`), `ui/files/FilesViewModel`, `ui/share/ShareScreen`, `ui/common` (`rootIcon`, `SeIcon`, `Snackbars`). Die Paketzuordnung ist belegt durch Importe in `ui/share/SharePeers.kt:24-26`, `ui/files/BrowserTab.kt:9-13`, `ui/files/PlacesPanel.kt:28-29` und `ui/share/ShareDeviceCard.kt:21-22`.
10. Kotlin-Helfer: `system/ShareIntentHandler` (Aufruf in `MainActivity.kt:85`), `system/OpenWithTarget` (`OpenWithIntent.kt:33`, `:54`), `system/Storage` (Volumes; `OpenWithIntent.kt:33`, `LocalFileProvider.kt:119`), `update/UpdateChecker` nicht gelesen.
11. Bridge-Dispatch: die Zuordnung von Methodennamen zu den Funktionen in `share_peers.rs` und `share_state.rs` nicht gelesen.
12. Typdefinitionen in `native/src/share/core/types.rs`: `RoomProfile`, `RoomMember`, `PeerPresence`, `ShareStatus` (einschließlich `label()`), `PeerOpenTarget` (einschließlich `endpoint_prefix()` und `from_endpoint()`), `ShareEvent` nicht gelesen.
13. `native/src/share/core/export_config.rs`: Felder von `ShareExportConfig` und `SharedRoot`, Varianten von `ExportAccess`, Wert von `DIRECT_EXPORT_SCOPE` nicht gelesen.
14. `native/src/share/core/lan_presence_match.rs`: Wert von `LAN_PRESENCE_TTL_SECS` nicht gelesen.
15. `native/src/mobile/os/shared/domains/share_policy_status.rs` (`contact_write`, `connections`, `grants`), das Modul `share_power` (`strip_volatile`, welche Felder), `crate::support_dirs` (Dateiname des Profils) nicht gelesen.
16. Daemon: Definition von `apply_verified_presence`, `drain_share_events_in_process`, `ShareHost::open_share` (Rumpf, `ipc_host.rs:355`), `AgentBackend::from_streams`, und der Verbraucher von `drain_share_worker_events` und `share_worker_snapshot` in der Desktop-GUI nicht gelesen.
17. Desktop-Oberfläche: die Stelle, an der Peer-Wurzeln (`mount_peer_roots.rs`) angeboten werden, und ob eingebundene Peers in der Seitenleiste erscheinen, nicht gelesen.
18. `native/src/mount/` (`PeerMountTarget`, `BackendRoot::parse`) nicht gelesen.
19. Android-Bridge-Funktion der Aktion „Verbindung…“ (`SharePeers.kt:95`) nicht in `share_peers.rs` enthalten; Zielfunktion nicht gelesen.
20. Android-Verhalten, das über den Quellstand hinausgeht (Startaktivität als Launcher, Widgets, Komponenten-Standardzustand): nicht aus lokalen Quellen belegt. Diese Lesung enthält dazu keine Aussage außer den Befunden in 2.1.3 und 2.1.4.

### 5.2 Annahmen (nicht belegt, ausdrücklich gekennzeichnet)

- A1 (2.1.4): eine Komponente ohne `android:enabled` ist im installierten Paket aktiv (Plattformwissen).
- A2 (2.2.3): entfernte Dateien ohne geladene Kopie gehen über `vm.openFile(entry, media.local, chooser)` (`FilesScreen.kt:144`).
- A3 (2.3.9): die Listen „Geräte“ und „Räume“ im Reiter „Teilen“ stammen aus dem Status-JSON (`ShareScreen.kt` nicht gelesen).
- A4 (2.4.2): `Roots.devices` und `Roots.rooms` werden aus den Geräte- und Raumdaten des Status-JSON gebildet.
- A5 (4.5): die Kotlin-Modelle lesen die JSON-Schlüssel aus 2.3.10 unverändert. Belegt sind nur die Feldnamen, die `SharePeers.kt` und `ShareDeviceCard.kt` verwenden (`SharePeers.kt:66-102`, `:168-194`; `ShareDeviceCard.kt:47-146`).
- A6 (4.3): der Kotlin-Vergleich `root.location == current` setzt gleiche Formen beider Seiten voraus; belegt ist nur die Verwendung.

### 5.3 Bezug zu den Annahmen des Plans

- Die Annahme aus `plan.md` A, „in the groups“ meine die Share-Räume, hat im Bestand einen Gegenstand: Räume mit Mitgliedern und Zulassungsregeln (2.3.4) und eine eigene Anzeige (2.4.2, 2.4.3).
- Die Hypothese M-Android (a) in `strang-desktop.md` nennt „Startaktivität standardmäßig ausgeschaltet“. Im Quellstand gibt es dafür keinen Schalter (2.1.4); ein Schalter ist nicht vorhanden.
- Die Annahme aus `plan.md` A zu „donkey“ (Dokany) liegt außerhalb dieses Auftrags (Desktop-Laufwerke).

## 6. Grenzen der Lektüre

### 6.1 Gelesene Dateien

Bereich A (Android), vollständig gelesen, sofern nicht anders angegeben:
1. `android/settings.gradle.kts`
2. `android/build.gradle.kts`
3. `android/app/build.gradle.kts`
4. `android/app/src/main/AndroidManifest.xml`
5. `android/app/src/main/java/app/smartexplorer/android/MainActivity.kt`
6. `android/app/src/main/java/app/smartexplorer/android/SmartExplorerApp.kt`
7. `android/app/src/main/java/app/smartexplorer/android/ui/AppRoot.kt`
8. `android/app/src/main/java/app/smartexplorer/android/ui/AppNav.kt`
9. `android/app/src/main/java/app/smartexplorer/android/ui/files/PlacesPanel.kt`
10. `android/app/src/main/java/app/smartexplorer/android/ui/files/FilesScreen.kt`: Zeilen 77-200 und 398-451 gelesen, übrige Zeilen nur per Grep auf Stellen (Aufruf-Stellen, Klassen- und Funktionsdeklarationen).
11. `android/app/src/main/java/app/smartexplorer/android/ui/files/BrowserTab.kt`: Deklarationen per Grep, Klasse `BrowserTab` (Zeile 23); kein Volltext.
12. `android/app/src/main/java/app/smartexplorer/android/system/Opener.kt`
13. `android/app/src/main/java/app/smartexplorer/android/system/OpenWithIntent.kt`
14. `android/app/src/main/java/app/smartexplorer/android/system/LocalFileProvider.kt`
15. `android/app/src/main/java/app/smartexplorer/android/prefs/AppPrefs.kt`
16. `android/build-release-apk.sh`: Zeilen 1-30, 35-52 (nur die Variablenzeilen), 103-143, 208-231 und 261-398 gelesen. Nicht gelesen: Zeilen 31-34, 53-102, 144-207 (`resolve_sdk`, `ensure_ndk`, `resolve_build_tools`, `ensure_rust_targets`) und 232-260 (`rustls_maven_dir`, `assert_16k_aligned`).
17. `android/test-android-task.sh`: Kopf `:1-27`, Funktionsgliederung und Schrittnamen per Grep, Dispatcher `:1066-1101`, Stellen der Emulator-Stufe per Grep und `sed`-Auszügen (`:686`, `:712`, `:729`, `:1016-1044`). Keine Volltext-Lektüre.
18. `docs/refs/android-gradle-build.md`: `:1-12`, `:150-207`, `:329-373`, `:662-684`.

Bereich B (Gerätemodell), gelesen:
19. `native/src/share/core/profiles.rs`: `:1-75` gelesen, übrige Zeilen nicht.
20. `native/src/share/core/profile_settings.rs`: vollständig.
21. `native/src/share/core/direct_relation.rs`: `:1-170`.
22. `native/src/share/core/room_relation.rs`: `:1-175`.
23. `native/src/share/core/room_relation_members.rs`: `:1-160`; übrige Zeilen nicht gelesen.
24. `native/src/share/core/relation_rights.rs`: `:1-130`.
25. `native/src/share/api_exports.rs`: vollständig.
26. `native/src/app/core/mount_peer_roots.rs`: `:1-60`, `:140-245` gelesen; `:61-139` nur per Grep.
27. `native/src/app/core/sidebar_locations.rs`: vollständig.
28. `native/src/mobile/os/shared/domains/share_peers.rs`: `:13-90` und `:182-257` gelesen; `:91-181` nur per Grep auf Funktionen und Antwortzeilen.
29. `native/src/mobile/os/shared/domains/share_state.rs`: `:233-300`, `:341-400` gelesen; übrige Zeilen per Grep.
30. `native/src/mobile/os/shared/domains/share_status.rs`: `:1-103`, `:110-152`, `:262-336` gelesen; übrige Zeilen per Grep.
31. `native/src/mobile/os/shared/domains/locations.rs`: vollständig.
32. `android/app/src/main/java/app/smartexplorer/android/ui/share/SharePeers.kt`: `:28-197` gelesen; übrige Zeilen per Grep.
33. `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDeviceCard.kt`: vollständig.

Daemon, nur die Stellen nach Auftrag (`open_share_backend`, Präsenz/Status):
34. `native/src/daemon/os/shared/ipc_client.rs`: `:15-62`.
35. `native/src/daemon/mod.rs`: `:205-212`.
36. `native/src/daemon/os/shared/ipc_host_relation_events.rs`: `:45-72` und Grep-Treffer auf `ShareEvent::`, `ShareStatus::`.
37. Grep-Treffer ohne Lektüre des umgebenden Codes: `native/src/daemon/os/shared/ipc.rs`, `ipc_host.rs`, `mount_source.rs`, `ipc_host_legacy_events.rs`.

Plan und Strang:
38. `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`: `:1-61`.
39. `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-desktop.md`: vollständig.

Einstieg, kein Beleg:
40. graphify-Abfrage „PeerOpenTarget endpoint_prefix definition“ im Repository-Wurzelverzeichnis. Der Graph `graphify-out/graph.json` trägt laut `ls -la` das Änderungsdatum „Oct 16 16:08“ (Jahr nicht angezeigt) und kann älter als der Lesestand sein.

Projektvorgaben: `AGENTS.md` (Abschnitt „mandatory release“ und Dokumentationsregeln) wurde aus dem Projektkontext übernommen und nicht als Datei neu gelesen.

### 6.2 Methode

- Zeilen über `cat -n`, `sed -n`, `awk` mit `FNR` (Zeilennummer je Datei) und `grep -n` auf die gelisteten Dateien.
- Keine Datei außerhalb der Liste wurde inhaltlich gelesen. Außerhalb der Liste liegen nur Verzeichnislisten (`docs/plaene/2026-10-17-rechte-desktop-tresore/`, `docs/lesungen/`) und die Metadaten (Änderungsdatum) von `graphify-out/graph.json`. Der graphify-Aufruf lieferte nur Einstiegspunkte.
- Keine Builds, Tests, Paketinstallationen, Server, Commits, Pushes und keine Netzrecherche. Keine Agenten oder Subagenten.
- Es wurden keine Plattformregeln außerhalb des Quellstands geprüft (zum Beispiel Launcher-Rolle, Widgets, Android-Standardverhalten von Komponenten).

### 6.3 Enthaltene Daten

- Das Dokument enthält keine Gerätenamen, Kennungen, Schlüssel, Fingerabdruckwerte, Pfade oder Zugangsdaten aus dem Umfeld des Nutzers.
- Zeilenangaben in den Zitaten sind Zeilen des Quellstands `da2646d2`.

