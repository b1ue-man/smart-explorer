# Desktop-Integration, Installation, Release und Update-Feed: Bestand

Stand 2026-10-17, Lesestand da2646d2

## 1. Ziel und Fragen

Ziel: Bestand für den Strang S5 „se Desktop (Windows, Linux) und se Launcher (Android)“ erheben. Grundlage sind `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md` (Abschnitt A, Ergebnisse E4, E6, E7) und `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-desktop.md`. Die Android-Seite wird in einem separaten Auftrag gelesen. Diese Datei beschreibt nur den Bestand; sie bewertet nichts und entscheidet nichts.

Konventionen:
- Pfade sind relativ zum Repository-Wurzelverzeichnis.
- `Pfad:Zeile` bezeichnet die Zeile der Quelle im Lesestand da2646d2. `Pfad:a-b` bezeichnet die Zeilen a bis b.
- „Präsenz“ bedeutet: eine gezielte `grep`-Suche hat die Stelle gefunden. Die Umgebung der Stelle wurde nicht gelesen. Jede Präsenzprüfung nennt ihren Suchbereich.
- „Annahme:“ kennzeichnet eine nicht belegte Vorannahme. „Ableitung:“ kennzeichnet eine Folgerung aus den genannten Zeilen.
- Versionsstand: `native/Cargo.toml:3` nennt `0.5.182`. `release-native/update-feed/version.txt` enthält `0.5.182`.

Fragen aus dem Leseauftrag:
- F1: Welche Binärdateien und Startmodi hat die Desktop-App? Wie sind Fenster, Seitenleiste und Orte aufgebaut? Wo erscheinen Geräte (Share-Kontakte, Räume) und Laufwerke in der GUI?
- F2: Welche Windows-Shell-Integration existiert (klassisches Kontextmenü, Windows-11-Menü, COM-Registrierung pro Nutzer oder Maschine, Explorer-Befehl-DLL, Autostart)? Wie wird sie registriert und vollständig entfernt (Installer, Laufzeit, Deinstallation)?
- F3: Welche Linux-Integration existiert (.desktop-Datei, MIME, Autostart, Installationsskript, Pfade)?
- F4: Wie ist der Installer aufgebaut? Welche Payloads umfassen ein Release und der Update-Feed (vollständige Namensliste)? Wie lädt und ersetzt der Updater Dateien?
- F5: Wie ist eine bestehende Task-Suite aufgebaut (Workflow-Eingaben, Jobs, Stufen, Timeouts) als Muster für die spätere Suite?

## 2. Befunde je Frage

### F1 Binärdateien, Startmodi, Fenster, Seitenleiste, Orte, Geräte, Laufwerke

**Binärdateien.** Vollständig für die Quellen `native/src/main.rs`, `native/src/bin/` und den Build-Aufruf `native/publish-feed.sh:188` (`--bin smart_explorer --bin smart_explorer_updater --bin se`). Nicht gelesen: die `[[bin]]`-Einträge in `native/Cargo.toml`. Außerhalb des Desktop-Bereichs liegen committed die Agent-Binärdateien `native/agent-bin/se-agent-x86_64-linux-musl` und `native/agent-bin/se-agent-aarch64-linux-musl` (`.github/workflows/build.yml:648-649`); ihr Zweck ist in dieser Lesung nicht geprüft.

| Binärdatei | Quelle | Befund | Im Feed-Build |
|---|---|---|---|
| `smart_explorer` (GUI) | `native/src/main.rs:1-9` | ruft zuerst `smart_explorer::mount::run_host_if_requested` (`:5`), sonst `smart_explorer::run_gui()` (`:8`) | ja, `native/publish-feed.sh:188` |
| `se` (Terminal-CLI) | `native/src/bin/se.rs:1-36` | Dispatch, siehe Startmodi unten | ja, `native/publish-feed.sh:188` |
| `smart_explorer_updater` (Update-Helfer) | `native/src/bin/smart_explorer_updater.rs:42-58` | ohne Schalter `--apply` sofort Ende (`:44-46`) | ja, `native/publish-feed.sh:188` |
| `bench` (Leistungswerkzeug) | `native/src/bin/bench.rs:1-3` | Nutzung laut Kopfkommentar: `cargo run --release --bin bench -- <path>` | nein; `publish-feed.sh:188` nennt nur die drei oben |
| `se-share-server` (eigenes Crate `share-server/`) | `native/publish-feed.sh:195-198` | Windows-Build mit `--bin se-share-server` (`:198`) | eigener Build; die Linux-Variante `se-share-server-linux` ist in dieser Lesung nicht nachverfolgt |

Keine Binärdatei, aber eigenes Artefakt: `native/explorer-command/` ist das cdylib-Crate `smart_explorer_command` (`native/explorer-command/Cargo.toml:10-12`). `native/src/bin/se_path_windows/mod.rs` ist ein Windows-Modul von `se.rs` (`native/src/bin/se.rs:1-2`), kein eigenes Binary.

**Startmodi der GUI** (`native/src/lib.rs:64-135`, Funktion `run_gui`), in Prüfreihenfolge:
1. Lesehelfer ohne Oberfläche: `local_access::run_helper_if_requested` (`:68-74`).
2. Exec-Supervisor: `share::run_exec_supervisor_if_requested` (`:75-78`).
3. Genau ein Argument `--sync-guardian`: `daemon::run_guardian()` (`:80-83`).
4. Irgendein Argument `--sync-daemon`: `daemon::run_daemon()` (`:86-89`).
5. Argument `--updated`: `just_updated` (`:91`), weitergegeben an `App::new` (`native/src/app/core/init.rs:5-6`).
6. Ohne ausstehende Startbestätigung: `updater::cleanup_old_binaries()` und `updater::archive_current_version()` (`:92-97`).
7. Nur Windows, Argument `--unregister`: `shell_register::unregister_all()`, dann Ende (`:99-103`).
8. Nur Windows, sonst: `shell_register::cleanup_stale_default_manager()` (`:105-106`).
9. Erstes Argument ohne `--`-Präfix ist der Startpfad (`:108-112`).
10. Fenster: Titel „Smart Explorer“, Größe 1400×900, Mindestgröße 900×600, Icon aus `native/assets/smart-explorer-icon-256.rgba` (`:114-121`, `:138-144`).
11. Nur Windows: `shell_menu::init_com()` (`:123-124`).

**Startmodi der CLI** (`native/src/bin/se.rs:4-36`), in Prüfreihenfolge: Exec-Supervisor (`:6`), Mount-Host (`:9`), LAN-Uplink-Helfer `net::run_uplink_helper_if_requested` (`:12`), nur Debug-Build `--share-exec-platform-self-test` (`:15-18`), `--sync-guardian` (`:19-22`), `--sync-daemon` (`:23-26`), nur Windows `--install-cli-path` (`:27-30`) und `--uninstall-cli-path` (`:31-34`), sonst `smart_explorer::cli::run()` (`:35`). Die Schalter der CLI verlangen exakt ein Argument (`is_internal_invocation`, `:79-81`). Die GUI prüft `--sync-daemon` dagegen mit `any` (`lib.rs:86`). Ableitung: Beide Einstiege unterscheiden sich in der Argumentprüfung von `--sync-daemon`.

**Updater-Helfer** (`native/src/bin/smart_explorer_updater.rs:42-59`): Ohne `--apply` sofort Ende (`:44-46`). Die Protokollvariante `ApplyRequest::Current` oder `ApplyRequest::Legacy` wird gewählt (`:56-59`).

**Fenster und Panels.**
- Ein Hauptfenster. Präsenz in `native/src/app`: kein Aufruf von `show_viewport`, `ViewportBuilder` oder `ViewportId`. Ableitung: Die GUI nutzt keine zusätzlichen Betriebssystem-Fenster.
- Panels, Datei `native/src/app/core/frame_layout.rs`: oben `tabbar` (`:18`), `toolbar` (`:21`), `filterbar` (`:24`); unten `status` (`:43`); links `sidebar` (`:47`); rechts `summary` (`:56`). Zentralfläche: `egui::CentralPanel::default()` in `native/src/app/core/central_tabs.rs:10`.
- Schwebende Fenster `egui::Window::new`: 26 Aufrufstellen. Suchbereich: `native/src/app`, ohne Testdateien. Pfade in der Tabelle sind relativ zu `native/src/app/`. Titel mit `format!` oder aus einer Variablen sind so angegeben, wie der Aufruf sie bildet.

| Stelle | Titel | Art |
|---|---|---|
| `core/bisync_ui.rs:11` | „🔍 Vergleich (Vorschau)“ | fest |
| `core/reclaim_ui.rs:66` | „📊 Speicher-Analyse“ | fest |
| `core/host_key_ui.rs:71` | „SSH-Hostschlüssel prüfen“ | fest |
| `core/merge_ui.rs:14` | `format!("Zeilenvergleich: {}", m.rel)` | format |
| `core/mount_ui.rs:167` | „Smart-Explorer-Laufwerke“ | fest |
| `core/sync_job_log_ui.rs:102` | `title` | Variable |
| `core/error_log_ui.rs:16` | `format!("Fehler-Protokoll ({})", …)` | format |
| `core/sync_versions_ui.rs:129` | `format!("Versionen · {name}")` | format |
| `core/mount_ui_draft.rs:136` | „Remote als Laufwerk“ | fest |
| `core/settings_ui.rs:31` | „Einstellungen“ | fest |
| `core/share_window_ui.rs:9` | „Geräte & Freigaben“ | fest |
| `core/analytics_ui.rs:103` | „📊 Speicher-Analyse“ | fest |
| `core/copy_dialog.rs:16` | `title` | Variable |
| `core/dialogs.rs:12` | `format!("Umbenennen: {}", title)` | format |
| `core/dialogs.rs:61` | „Update bereit“ | fest |
| `core/dialogs.rs:146` | „Hinweis & Haftungsausschluss“ | fest |
| `core/dialogs.rs:185` | „Tastenkürzel“ | fest |
| `core/transfer_window.rs:67` | „⇅ Übertragungen“ | fest |
| `core/connect_dialog_ui.rs:54` | `title` | Variable |
| `core/menus_sync_jobs.rs:12` | „📜 Sync-Protokoll“ | fest |
| `core/menus_sync_jobs.rs:59` | „⚙ Sync-Setups“ | fest |
| `core/bisync_conflict_ui.rs:31` | `format!("⚠ Sync-Konflikte ({conflict_count})")` | format |
| `os/shared/sync_exit_gate.rs:64` | „Synchronisierung abschließen“ | fest |
| `core/picker_impl.rs:134` | `title` | Variable |
| `core/job_editor_ui.rs:39` | `title` | Variable |
| `core/sync_job_state_ui.rs:276` | „Sicherheitsstopp bestätigen“ | fest |

- Das Share-Fenster „Geräte & Freigaben“ (`native/src/app/core/share_window_ui.rs:9`) erscheint nur, wenn `show_share` gesetzt ist (`native/src/app/core/frame_layout.rs:112-113`).

**Seitenleiste** (`native/src/app/core/sidebar.rs` und `native/src/app/core/sidebar_locations.rs`). Die Seitenleiste ruft zuerst `ui_sidebar_locations` auf (`sidebar.rs:6`), danach `ui_sidebar_connections`.
- `sidebar_locations.rs:6`: Eintrag „Startseite“.
- `sidebar_locations.rs:12`: Abschnitt „FAVORITEN“, nur wenn Favoriten vorhanden sind.
- `sidebar_locations.rs:29`: Abschnitt „SCHNELLZUGRIFF“ mit sieben Zeilen (Liste `:32-34`): „Persönlicher Ordner“ (Persönlicher Ordner), „Desktop“ (Unterordner `Desktop` im Persönlichen Ordner), „Dokumente“ (`Documents`), „Downloads“ (`Downloads`), „Bilder“ (`Pictures`), „Musik“ (`Music`), „Videos“ (`Videos`). Die Pfadbildung steht in `:36`. Ableitung: Der Eintrag „Desktop“ ist ein Ordnername im Persönlichen Ordner und kein Bestandteil eines se-Desktops.
- `sidebar_locations.rs:41-42`: Abschnitt „LAUFWERKE“, nur wenn `drive_info` nicht leer ist.
- `sidebar_locations.rs:56-60`: einklappbarer Abschnitt „ZULETZT“ mit höchstens fünf Einträgen.
- `sidebar.rs:17`: Abschnitt „VERBINDUNGEN“ (Zeilen `:16-131`: aktive Verbindung, gespeicherte Verbindungen, Schaltfläche „+ Neue Verbindung“).

**Geräte in der GUI** (Share-Kontakte):
- Abschnitt „Geräte“ in `sidebar.rs:132-170`. Er zeigt `share_profiles.direct_contacts` (`:132`), höchstens fünf Einträge (`:139`, `take(5)`). Ein Klick öffnet das Gerät: Zuweisung von `open_share_target` mit einem `PeerOpenTarget::Direct`-Ziel (`:153`).
- Menüeintrag „Als Laufwerk einbinden…“ je Gerät (`:157`), nur bei `mount_supported` (`:157`).
- Schaltfläche „Alle Geräte verwalten…“ (`:165`) setzt `show_share = true` und `share_tab = 0` (`:168-169`).

**Räume in der GUI:**
- Abschnitt „Räume“ in `sidebar.rs:172-212`: einklappbarer Kopf „Räume“ (`:173`, Kennung `sidebar_rooms_v2`, `:174`), Menüeintrag „Als Laufwerk einbinden…“ je Raum (`:193-194`, mit `mount_supported` in `:193`).
- Schaltfläche „Räume verwalten…“ (`:210`) setzt `show_share = true` und `share_tab = 1` (`:211-212`).
- Das Share-Fenster hat die Tabs in dieser Reihenfolge (`native/src/app/core/share_pages_ui.rs:19-29`): Geräte, Anfragen, Räume, Freigaben, Befehle, Dieses Gerät, Netzwerk, Diagnose, Papierkorb. Beschriftungen in `:31-43`. Der Tab Papierkorb erscheint nur, wenn `crate::host_trash::available()` wahr ist (`native/src/app/core/share_window_ui.rs:23`).

**Laufwerke in der GUI:**
- Windows: `list_drives()` liefert alle Laufwerksbuchstaben über `GetLogicalDrives` (`native/src/app/os/windows/platform.rs:41-48`). `drive_info_list` liefert freien und gesamten Speicher je Laufwerk (`platform.rs:50` ff.).
- Linux: `list_drives()` liefert `["/"]` (`native/src/app/os/linux_os.rs:21-23`). `drive_info_list` liefert eine leere Liste (`linux_os.rs:25-27`). Ableitung: Der Abschnitt „LAUFWERKE“ entfällt unter Linux (`sidebar_locations.rs:41`).
- Einbindung als Laufwerk: `drive_mount_supported()` ist `cfg!(windows)` (`native/src/mount/mod.rs:279-281`). Die Menüs „Als Laufwerk einbinden…“ prüfen diesen Wert in `sidebar.rs:32` (Wertzuweisung) und in den Bedingungen `:82`, `:100`, `:157` und `:193`. Ableitung: Unter Linux erscheint in der GUI kein Menüpunkt „Als Laufwerk einbinden…“.
- Annahme: Die Verwaltung der Laufwerke über „Smart-Explorer-Laufwerke“ (`native/src/app/core/mount_ui.rs:167`) und „Remote als Laufwerk“ (`native/src/app/core/mount_ui_draft.rs:136`) ist Teil des Laufwerksstrangs S2 und wird hier nur benannt.

**Präsenz ohne Befund** (Suchbereich `native/src`): keine Desktop-Ebene (`Progman`, `WorkerW`, `SetParent`); keine Linux-MIME-Einträge (`mimeapps`, `x-scheme-handler`, `MimeType=`); keine Zugriffe auf `HKEY_LOCAL_MACHINE`, `HKLM`, `HKEY_CLASSES_ROOT` oder `HKCR` außer einem Kommentar (`native/src/shell_register/os/windows.rs:7`).

### F2 Windows-Shell-Integration

**2.1 Klassisches Kontextmenü „In Smart Explorer öffnen“ (nur HKCU).** Modul `shell_register`, nur Windows (`native/src/lib.rs:47-49`), Datei `native/src/shell_register/os/windows.rs`.
- Verb `OpenInSmartExplorer`, Beschriftung „In Smart Explorer öffnen“ (`:28-29`).
- Schlüssel unter `HKCU\Software\Classes` (`:6-7`): `Directory\shell\OpenInSmartExplorer` mit `%1` (`:95`), `Drive\shell\OpenInSmartExplorer` mit `%1` (`:96`), `Directory\Background\shell\OpenInSmartExplorer` mit `%V` (`:97`). Jeder Verbschlüssel trägt `MUIVerb`, `Icon` und den Unterschlüssel `command` (`register_verb`, `:62-74`).
- Entfernen: `unregister_verb` löscht den Verbschlüssel rekursiv (`:76-83`). `set_context_menu_for(false, …)` ruft das für alle drei Schlüssel auf (`:98-102`). Nach jeder Änderung `SHChangeNotify(SHCNE_ASSOCCHANGED)` (`:103`, Funktion `:247-257`).
- Prüfung: `context_menu_enabled()` liest `Directory\shell\OpenInSmartExplorer\command` (`:107-114`).
- Installer: `native/installer.nsi:161-169` schreibt dieselben drei Verbschlüssel; der Befehl nennt `"$INSTDIR\Smart Explorer.exe"` (`:163`, `:166`, `:169`).
- GUI-Schalter: Der Abschnitt „Dateimanager“ erscheint nur bei `shell_integration_available()` (`native/src/app/core/menus_settings.rs:130`). Diese Funktion liefert unter Windows `true` (`native/src/app/os/windows/platform.rs:185-187`). Der Schalter mit der Beschriftung „In Smart Explorer öffnen“ im Rechtsklick (`native/src/app/core/menus_settings.rs:140-141`) ruft `set_context_menu_enabled(on)` (`:148`). Diese Funktion ruft `shell_register::set_context_menu(on)` auf (`native/src/app/os/windows/platform.rs:199-200`).
- CLI: `se integration context-menu` mit `on` oder `off` und optional `--json`. Die Argumentfolgen stehen in den Testfällen (`native/src/cli/integration.rs:109-111`). Die Ausführung beginnt in `native/src/cli/integration.rs:49` und schreibt über `native/src/cli/integration.rs:73`. Die Windows-Implementierung ruft `shell_register::set_context_menu_for` mit dem Pfad der App auf (`native/src/cli/os/windows.rs:82`).
- Entfernen bei der Deinstallation: `native/installer.nsi:299` ruft `"$INSTDIR\Smart Explorer.exe" --unregister` auf. Der Aufruf endet in `native/src/lib.rs:99-103` mit `shell_register::unregister_all()` (`native/src/shell_register/os/windows.rs:232-235`), das `set_context_menu(false)` und `disable_default_manager()` aufruft. Zusätzlich löscht der Installer die drei Verbschlüssel direkt (`native/installer.nsi:317-319`).

**2.2 Standard-Dateimanager (Opt-in, im Produktionscode nicht aufgerufen).**
- Mechanismus laut Kommentar: Überschreiben des Verbs `open` unter `HKCU\Software\Classes\Directory` und `Drive` (`native/src/shell_register/os/windows.rs:9-14`).
- `enable_default_manager` (`:157-178`) trägt `#[cfg_attr(not(test), allow(dead_code))]` (`:156`). Der Kommentar lautet: „The default-manager override doesn't work on Win11 (kept only to exercise the reversibility test and the cleanup path).“ (`:154-155`). Präsenz in `native/src`: Aufrufstellen sind nur die Definition (`:157`) und ein Test (`:282`). Ableitung: Im Produktionscode wird der Standard-Dateimanager nicht aktiviert.
- Sicherungsdatei `default_manager_backup.txt` unter `%APPDATA%\smart_explorer\` (`:47-54`). Sie wird nur von `capture_backup` (`:123-140`) geschrieben, und `capture_backup` wird nur aus `enable_default_manager` aufgerufen (`:158`).
- Entfernen: `disable_default_manager` (`:180-213`) löscht den Schlüssel `open` oder stellt einen gesicherten Befehl wieder her und löscht die Sicherungsdatei (`:210`).
- Bereinigung bei jedem GUI-Start unter Windows: `native/src/lib.rs:105-106` ruft `cleanup_stale_default_manager()` auf (`native/src/shell_register/os/windows.rs:241-245`). Die Funktion ruft `disable_default_manager()` auf, wenn `default_manager_enabled()` (`:216-227`) wahr ist oder die Sicherungsdatei existiert.

**2.3 Windows-11-Hauptmenü (COM-Handler und Paketmanifest; nicht registriert).**
- `native/explorer-command/src/lib.rs`: CLSID `{7F3B1E20-9C4A-4D8E-A1B2-3C4D5E6F7081}` (`:34-36`). `OpenCommand` implementiert `IExplorerCommand` (`:68-69`, `:71-118`) mit dem Titel „In Smart Explorer öffnen“ (`:72-74`). `Invoke` startet `Smart Explorer.exe` aus demselben Verzeichnis wie die DLL (`:55-66`, `:87-111`) per `ShellExecuteW` je Element aus `items` (`:90-109`). Ist `items` nicht gesetzt, geschieht nichts; die Funktion gibt `Ok(())` zurück (`:110`).
- Exportierte Funktionen: `DllMain` (`:43-44`), `DllGetClassObject` (`:151-168`), `DllCanUnloadNow` (`:170-177`). Eine Funktion `DllRegisterServer` gibt es nicht (vollständig gelesen: die Datei enthält genau drei `#[no_mangle]`-Funktionen, `:43`, `:151`, `:170`).
- `native/explorer-command/AppxManifest.xml`: Herausgeber-Platzhalter `CN=REPLACE_WITH_CERT_SUBJECT` (`:29`), Anzeigename des Herausgebers gesetzt (`:35`), Anwendungspfad `%LOCALAPPDATA%\Programs\Smart Explorer\Smart Explorer.exe` (`:56`), COM-Server-Erweiterung mit der CLSID (`:65-73`), Verben `OpenInSmartExplorer` für `Directory` (`:78-81`), `Directory\Background` (`:82-85`) und `Drive` (`:86-89`).
- Status laut Dokument: „implementation done + compile-verified; blocked only on code signing“ (`docs/WIN11_CONTEXT_MENU.md:3`). Die Signierung wird als Hindernis beschrieben (`:50-62`).
- Ausgelieferung: `smart_explorer_command.dll` wird gebaut (`native/publish-feed.sh:208`), geprüft (`:257`) und als Release-Asset veröffentlicht (`native/release-publication.ps1:152`). Die DLL liegt committed unter `release-native/smart_explorer_command.dll` (`.github/workflows/build.yml:654`).
- Keine Registrierung: `native/installer.nsi` enthält die Zeichenkette `smart_explorer_command` nicht (Präsenz). In `native/*.ps1`, `native/*.sh`, `native/*.nsi` und `install-linux.sh` gibt es für `Add-AppxPackage`, `makeappx`, `AppxManifest`, `signtool` und `Remove-AppxPackage` keine Treffer (Präsenz). Ableitung: Kein Installer und kein Skript des Repositorys registriert das Sparse-Paket; die DLL wird nur ausgeliefert.

**2.4 Registrierungsebene.** Die klassische Integration schreibt ausschließlich nach `HKCU`. Der Kopfkommentar lautet „Per-user Windows shell integration (no admin)“ und „We never touch HKLM/HKCR or the Folder class.“ (`native/src/shell_register/os/windows.rs:1-7`). Präsenz in `native/src`: `HKEY_LOCAL_MACHINE`, `HKLM`, `HKEY_CLASSES_ROOT`, `HKCR` nur im Kommentar `:7`. Die DLL hat keinen Registrierungscode (Abschnitt 2.3). Ableitung: Alle Shell-Registrierungen des Bestands gelten pro Benutzer; eine Maschinen-Registrierung ist im Bestand nicht vorhanden.

**2.5 Autostart (Windows).**
- Schlüssel `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, Wertname `SmartExplorerSync` (`native/src/autostart/os/windows.rs:1-2`).
- Befehl `"<exe>" --sync-daemon` (`:23-28`). `<exe>` ist `smart_explorer.exe` oder `Smart Explorer.exe` im Verzeichnis von `se.exe`, wenn der laufende Prozess `se.exe` ist (`:12-21`). Sonst ist es die laufende Datei. Fehlt die GUI-Datei neben `se.exe`, schlägt das Schreiben mit `ErrorKind::Unsupported` fehl (`:30-42`).
- Funktionen: `is_enabled` (`:141`), `disabled_by_system` über `Explorer\StartupApproved\Run` (`:153`), `enable` (`:175`), `disable` (`:181`, löscht nur den Wert `SmartExplorerSync`).
- Schalter: GUI `native/src/app/core/settings_background.rs:20` (`enable`) und `:41` (`disable`); CLI `native/src/cli/background.rs:135` (`enable`) und `:99` (`disable`).
- Installer und Deinstallation: `native/installer.nsi` enthält die Zeichenkette `SmartExplorerSync` nicht und greift nicht auf `CurrentVersion\Run` zu (Präsenz). `--unregister` (`native/src/lib.rs:99-103`) ruft nur `shell_register::unregister_all()` auf. Aufrufe von `autostart::disable` gibt es in `native/src/app/core/settings_background.rs`, `native/src/cli/background.rs` und `native/src/mobile/os/shared/domains/background.rs` (Präsenz). Ableitung: Die Deinstallation entfernt den Run-Wert nicht. Ob danach ein Wert verbleibt, ist nicht geprüft (Lücke L-01).

**2.6 PATH-Eintrag für `se.exe`.**
- Der Installer ruft `"$INSTDIR\se.exe" --install-cli-path` auf (`native/installer.nsi:128`). Bei Fehler erscheint eine Meldung (`:130-131`).
- `native/src/bin/se_path_windows/mod.rs`: Wert `Path` unter `HKCU\Environment` (`:10`, Lesen und Schreiben `:105-128`). Eigentumsmarker unter `HKCU\Software\SmartExplorer\Installer` mit den Werten `CliPath` und `CliPathOwned` (`:11-13`). `register()` (`:15-50`) hängt genau einen Eintrag an, wenn er fehlt, und setzt die Marker. Ein Semikolon im Installationspfad verbietet die Registrierung (`:21-26`). `unregister()` (`:52-79`) entfernt den Eintrag nur, wenn der Eigentumsmarker gesetzt und der Pfad identisch ist (`:58-73`), und löscht danach die Marker-Werte (`:74-75`).
- Umgebungsbroadcast `WM_SETTINGCHANGE` (`:184-206`).
- Deinstallation: `native/installer.nsi:309` ruft `"$INSTDIR\se.exe" --uninstall-cli-path` auf. Bei Fehler bricht die Deinstallation vor dem Löschen von `se.exe` ab (`:310-313`).

**2.7 Weitere Registrierungen des Installers.**
- Windows-Firewall-Regel `Smart Explorer Share Peer Listener` für `Smart Explorer.exe` (`native/installer.nsi:146-147`), Entfernung in der Deinstallation (`:320`). Der Regelname steht zusätzlich als Konstante `SHARE_FIREWALL_RULE` in `native/src/updater/os/windows.rs:9`.
- Eintrag für die Liste der installierten Programme `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartExplorer` (`native/installer.nsi:52`, `:177-185`), Entfernung in `:340`.

**2.8 Bestehende Explorer-Kopplungen** (Präsenz, nicht nach Tiefe gelesen).
- Windows-Kontextmenü des Explorers im App-Fenster; laut Kommentar einschließlich Drittanbieter-Erweiterungen (`native/src/shell_menu/os/windows.rs:1-5`). Funktionen `show_for_paths` (`:209`) und `show_background_menu` (`:292`). Aufrufe aus `native/src/app/os/windows/shell_menus.rs:73` und `:122`.
- Ziehen aus der App in den Explorer über OLE `DoDragDrop` mit CF_HDROP (`native/src/dragout/os/windows.rs:2-3`, `:23`).
- Zwischenablage: CF_HDROP lesen (`native/src/shell_clipboard/os/windows/read.rs:18-22`); eigene Formate über `RegisterClipboardFormatW` (`native/src/shell_clipboard/os/windows.rs:35`).
- „Im Explorer anzeigen“: `explorer.exe /select,<pfad>` (`native/src/app/os/windows/platform.rs:67-70`).

### F3 Linux-Integration

**3.1 Installationsskript `install-linux.sh`** (469 Zeilen; gelesen: `:1-64`, `:228-303`, `:350-469`). Die Zeilen `:65-227` und `:304-349` sind nicht gelesen (Lücke L-14). Die Präsenzprüfungen zu `uninstall`, `rm`, `sudo` und den Paketmanagern umfassen die ganze Datei.
- Standardwerte: Repository `b1ue-man/smart-explorer` (`:4`); Referenz `main` (`:5`); Release-Tag `latest` (`:6`); Installationsordner `$HOME/.local/opt/smart-explorer` (`:8`); Binärordner `$HOME/.local/bin` (`:9`); `DATA_HOME` = `$XDG_DATA_HOME` oder `$HOME/.local/share` (`:10`); `APP_DIR` = `$DATA_HOME/applications` (`:11`); `ICON_DIR` = `$DATA_HOME/icons/hicolor/256x256/apps` (`:12`).
- Argumente: `--dry-run` und `--cli-only`. Jedes Argument außer diesen beiden bricht mit einem Hilfetext ab (`:56-63`).
- Plattform: nur Linux (`:230-233`) und nur `x86_64` (`:235-238`).
- Release-Assets für den Desktop-Pfad (`:265-270`): `smart_explorer`, `smart_explorer.sha256`, `smart_explorer_updater`, `smart_explorer_updater.sha256`, `se`, `se.sha256`. Bei `--cli-only` (`:261-262`) nur `se` und `se.sha256`.
- Prüfung (`:278-284`): jede Datei gegen ihr SHA-256-Sidecar (`verify_payload_sha256`, `:287`).
- Installation, Funktion `install_files` (`:353-409`):
  - Binärdateien `smart_explorer` (`:359`), `smart_explorer_updater` (`:360`) und `se` (`:362`), jeweils atomar über `install_executable_atomic` (`:411-424`: temporäre Datei, `install -m 755`, `mv -f`).
  - Datei `update_source.txt` mit dem Feed (`:367`). Bei `--cli-only` wird sie nur angelegt, wenn sie fehlt (`:370-377`).
  - Symlinks: `$BIN_DIR/smart_explorer` auf `APP_BIN` (`:369`) und `$BIN_DIR/se` auf `CLI_BIN` (`:379`).
  - Icon: Download von `native/assets/smart-explorer-logo-256.png` aus dem Repository nach `$ICON_DIR/smart-explorer.png` (`:384-386`). Fehler werden mit `|| true` ignoriert (`:386`).
  - `.desktop`-Datei `$APP_DIR/smart-explorer.desktop` (`:392-403`) mit `Type=Application`, `Name=Smart Explorer`, `Exec=$APP_BIN`, `Icon=smart-explorer`, `Terminal=false`, `Categories=Utility;FileManager;`, `StartupNotify=true`.
  - `update-desktop-database` (`:406-408`), nur wenn der Befehl vorhanden ist.
- Quellbau-Fallback (`:426-442`): Fehlen die Release-Assets, werden Systempakete installiert (`:439`) und `cargo build --release` ausgeführt (`:345` für `se`; `:348` für `smart_explorer`, `smart_explorer_updater` und `se`). `CARGO_BUILD_JOBS` ist standardmäßig `1` (`:345`, `:348`).
- Paketmanager-Erkennung: `apt-get` (`:163-165`), `dnf` (`:171`), `pacman` (`:176-177`), `zypper` (`:181-182`). Rechteerhöhung über `sudo` (`:105-106`). Ohne `sudo` oder `doas` und ohne Root bricht das Skript ab (`:112`).
- PATH-Hinweise: Warnung, wenn `$BIN_DIR` nicht im PATH steht (`:457-463`); Warnung, wenn `se` auf einen Pfad auflöst, der nicht `$BIN_DIR/se` ist (`:464-468`).
- Deinstallation: kein Pfad im Skript. Präsenz: das Wort `uninstall` kommt in `install-linux.sh` nicht vor. `rm` steht nur an `:67` (temporäre Installationsdatei) und `:70` (temporäres Verzeichnis).

**3.2 Autostart unter Linux** (Laufzeit, vom Installer nicht angelegt). Datei `native/src/autostart/os/linux_os.rs`.
- Dateiname `smart-explorer-sync-daemon.desktop` (`:4`) im Verzeichnis `$XDG_CONFIG_HOME/autostart`, sonst `$HOME/.config/autostart` (`:6-16`; Dateipfad `:18-19`).
- Inhalt (`:103-112`): `Type=Application`, `Name=Smart Explorer Sync Daemon`, `Exec=<exe> --sync-daemon` (`:104`), `Terminal=false`, `X-GNOME-Autostart-enabled=true`. Ableitung: Der Linux-Autostart startet den Sync-Daemon und nicht die GUI.
- Funktionen: `is_enabled` (`:64`), `disabled_by_system` (`:70`, mit `system_disabled` in `:75`), `enable` (`:98`), `disable` (`:174`).
- Schalter: dieselben Aufrufe wie unter Windows (Abschnitt 2.5), weil `autostart::enable()` und `autostart::disable()` plattformübergreifend als Funktionen des Moduls `autostart` existieren.

**3.3 GUI unter Linux.**
- `shell_integration_available()` ist `false` (`native/src/app/os/linux_os.rs:74-76`). Folge: Der Abschnitt „Dateimanager“ entfällt (`native/src/app/core/menus_settings.rs:130`).
- `set_context_menu_enabled` ist ein No-op, der `Ok(())` liefert (`native/src/app/os/linux_os.rs:87-89`). `initial_context_menu_enabled()` liefert `false` (`:186-188`).
- Anzeigen und Öffnen über `xdg-open`: Anzeigen in `native/src/app/os/linux_os.rs:29-35`, Öffnen in `:37-39`.
- Laufwerke: siehe F1. Keine Einbindung aus der GUI, weil `drive_mount_supported()` `cfg!(windows)` ist (`native/src/mount/mod.rs:279-281`).
- Dateityp-Zuordnung (MIME): keine. Präsenz in `install-linux.sh` und in `native/src`: keine Zeichenketten `MimeType`, `mimeapps`, `x-scheme-handler`.

**3.4 Pfade unter Linux.**
- Installation: `$HOME/.local/opt/smart-explorer/` (`install-linux.sh:8`), `$HOME/.local/bin/` (`:9`), `$DATA_HOME/applications/` (`:11`), `$DATA_HOME/icons/hicolor/256x256/apps/` (`:12`).
- Datenordner der App: `app_data_dir()` ist `data_home()/smart_explorer` (`native/src/support_dirs.rs:196-199`). `platform_data_home` nutzt unter Linux `XDG_DATA_HOME` (`native/src/support_dirs.rs:179-182`). Unter Windows ist der Datenordner `APPDATA` (`native/src/support_dirs.rs:171-177`). Der Zweig ohne `XDG_DATA_HOME` ist nicht gelesen. README: `$XDG_DATA_HOME/smart_explorer/` bzw. `~/.local/share/smart_explorer/` (`README.md:1359-1363`).

**3.5 Dateien für Linux im Repository.** `native/assets/` enthält keine `.desktop`-Datei. Vollständiger Inhalt (`find native/assets -maxdepth 2`): `smart-explorer-icon.ico`, `smart-explorer-logo-256.png`, `smart-explorer-logo.png`, `smart-explorer-logo-64.png`, `smart-explorer-icon-256.rgba` und das Verzeichnis `dokany-private/` mit `corresponding-source.zip`, `manifest.json` und `dokan2.dll`. Die `.desktop`-Datei entsteht erst beim Installationslauf (`install-linux.sh:392-402`).

### F4 Installer, Payloads, Feed und Updater

**4.1 NSIS-Installer `native/installer.nsi`** (341 Zeilen, vollständig gelesen).
- Kopf: Installation pro Benutzer; der Dokany-Abschnitt fordert UAC separat (`:1-2`). Ausgabedatei `../release-native/Smart Explorer Setup ${VERSION}.exe` (`:36`). `RequestExecutionLevel user` (`:59`). `InstallDir` ist `$LOCALAPPDATA\Programs\Smart Explorer` (`:60`).
- Seiten in dieser Reihenfolge: Lizenzseite mit `DISCLAIMER.txt` (`:70`, `:73`), Komponenten (`:74`), Verzeichnis (`:75`), Installation (`:76`).
- Abschnitte: `SEC_MAIN` „Smart Explorer (erforderlich)“ (`:78-186`; `SectionIn RO` in `:79`); `SEC_DOKANY` „Dokany … (UAC erforderlich)“ (`:188-246`, optional); `Uninstall` (`:290-341`).
- Installierte Dateien im Abschnitt `SEC_MAIN`:
  - `Smart Explorer.exe` aus `EXE_SRC` (`:102-117`, mit Wiederholungen bei gesperrter Datei),
  - `Smart Explorer Updater.exe` aus `UPDATER_SRC` (`:119-120`),
  - `se.exe` aus `CLI_SRC` (`:122-123`),
  - `LICENSE` (`:134`),
  - `licenses\Dokany\NOTICE.txt`, `licenses\Dokany\LICENSE-GPL-3.0.txt`, `licenses\Dokany\LICENSE-LGPL-3.0.txt`, `licenses\Dokany\LICENSE-MIT.txt` (`:135-139`),
  - `update_source.txt`, nur wenn noch keine Datei vorhanden ist (`:152-153`),
  - `Uninstall.exe` (`:155`).
- Registrierungen und Befehle im Abschnitt `SEC_MAIN`: `taskkill` (`:88-90`, `:110-111`), `se.exe --install-cli-path` (`:128`), Firewall-Regel (`:146-147`), Verbschlüssel (`:161-169`), Verknüpfungen in Startmenü und Desktop (`:172-174`), Eintrag für Programme (`:177-185`).
- Optionaler Abschnitt `SEC_DOKANY` (`:188-246`): MSI nach `$PLUGINSDIR` (`:194-196`); Aufruf `se.exe drive install-runtime --msi <MSI>` (`:201`); Rückgabecodes `3010`, `1641`, `1223` und `1602` (`:204-208`); Prüfung über `se.exe drive runtime` (`:251`).
- `.onInit` (`:259-280`): Der Dokany-Abschnitt ist abgewählt, außer bei `/INSTALLDOKANY=1` (`:267-274`). Ist die Laufzeit schon passend, ist er abgewählt und sein Text geändert (`:276-278`).
- `.onInstSuccess` (`:283-288`): startet `Smart Explorer.exe` nur bei interaktiver Installation und ohne ausstehenden Dokany-Neustart.
- Deinstallation (`:290-341`) in dieser Reihenfolge: Prozesse beenden (`:292-294`); `Smart Explorer.exe --unregister` (`:299`); `se.exe --lan-uplink-cleanup` mit Abbruch bei Fehler (`:303-307`); `se.exe --uninstall-cli-path` mit Abbruch bei Fehler (`:309-313`); Löschen der drei Verbschlüssel (`:317-319`); Löschen der Firewall-Regel (`:320`); Löschen der Dateien (`:322-335`): `Smart Explorer.exe`, `Smart Explorer Updater.exe`, `se.exe`, `LICENSE`, die vier Dateien unter `licenses\Dokany`, `Smart Explorer_old.exe`, `Smart Explorer_update_pending.exe`, `update_source.txt`, `Uninstall.exe`; Entfernen der Verzeichnisse `licenses\Dokany` und `licenses` (`:330-331`) sowie `$INSTDIR` (`:336`); Löschen von Startmenü- und Desktop-Verknüpfung (`:337-339`); Löschen des Eintrags `HKCU\...\Uninstall\SmartExplorer` (`:340`).
- Im Abschnitt `Uninstall` nicht genannt: der Run-Wert `SmartExplorerSync` (Abschnitt 2.5), der Datenordner `%APPDATA%\smart_explorer\`, die Dokany-Laufzeit. Zur Dokany-Laufzeit steht in `README.md:935-989` der Satz „Smart Explorer entfernt eine installierte Dokany-Runtime beim Uninstall nicht.“

**4.2 Release-Payloads.** Die vollständige Liste der 20 Assets des GitHub Release und der committed Dateien unter `release-native/` steht in Abschnitt 3.

**4.3 Feed `release-native/update-feed/`: Erzeugung und Prüfung.**
- `native/publish-feed.sh`: Windows-Feed `smart_explorer.exe`, `smart_explorer_updater.exe`, `se.exe` (`:211-213`). Die Linux-Payloads `smart_explorer`, `smart_explorer_updater` und `se` erzeugt das Skript `native/publish-linux-feed-wsl.sh` (Aufruf `native/publish-feed.sh:192`; Kopfkommentar: „Build the Linux release payloads from Linux/WSL and stage them into the in-repo update feed“). `native/publish-feed.sh:223-224` listet alle sechs Payloads; für jeden wird ein `sha256sum`-Sidecar geschrieben (`:225-234`, Befehl `:233`). Manifest `windows-build.manifest` mit den drei Windows-Payloads (`:240-243`); Prüfung der Manifestzeilen (`:283-289`); `version.txt` wird als letzter Schritt verschoben (`:375`).
- `native/publish-release-local.ps1`: APK-Name `smart-explorer-android.apk` und Build-Skript `android/build-release-apk.sh` (`:46-47`); Sidecar der APK in den Feed (`:183-186`).
- Freigegebene Pfade im Release-Commit (`native/publish-release-local.ps1:897-902`): `release-native/share-server/`, `release-native/smart_explorer_command.dll` und der Präfix `release-native/Smart Explorer Setup `.
- Teil-Build `native/publish-update.ps1`: nur Windows; verlangt `-AllowPartialFeed` (`:46`); schreibt die drei Windows-Payloads und `windows-build.manifest` (`:124-128`).

**4.4 Updater (`native/src/updater/`).**
- Quelle: `update_source_str()` (`native/src/updater/os/shared/config.rs:42-63`) liest zuerst `%APPDATA%\smart_explorer\update_source.txt` über `override_path()` (`:7-8`; Datenordner `appdata_dir()` in `:3-5`), danach `update_source.txt` neben der laufenden Datei (`:57`). Setzen über `set_update_source` (`:66`).
- Transport: `classify_feed` (`native/src/updater/os/shared/feed.rs:161-168`): `http://` und `https://` ergeben HTTP (Präfixvergleich ohne Beachtung der Groß- und Kleinschreibung); jede Zeichenkette ohne dieses Präfix ergibt einen lokalen Pfad. Die Kurzform eines GitHub-Repositorys normalisiert `normalize_http_feed` (`:172`).
- Versionsabfrage: `read_version` liest die erste Zeile von `version.txt` (`feed.rs:38-47`). `check_feed()` vergleicht mit `CARGO_PKG_VERSION` (`native/src/updater/os/shared/terminal.rs:48-61`).
- Payload-Spezifikation `PayloadSpec` (`feed.rs:14-18`) mit den Feldern `local_names`, `http_names` und `hash_name`:
  - Windows: App `smart_explorer.exe` (lokal alternativ `Smart Explorer.exe`) mit `smart_explorer.exe.sha256` (`native/src/updater/os/windows.rs:57-63`); Updater `smart_explorer_updater.exe` (lokal alternativ `Smart Explorer Updater.exe`) mit `smart_explorer_updater.exe.sha256` (`:65-74`); CLI `se.exe` mit `se.exe.sha256` (`:76-80`).
  - Linux: App `smart_explorer` (lokal alternativ `Smart Explorer`) mit `smart_explorer.sha256` (`native/src/updater/os/linux_os.rs:53-59`); Updater `smart_explorer_updater` (lokal alternativ `Smart Explorer Updater`) mit `smart_explorer_updater.sha256` (`:61-67`); CLI `se` mit `se.sha256` (`:69-74`).
- Laden: `fetch_exe`, `fetch_updater_exe` und `fetch_cli_exe` (`feed.rs:49`, `:60`, `:71`) rufen `fetch_payload` (`:82`) auf. Die Prüfsumme kommt zuerst aus dem Sidecar; sie ist Pflicht (`feed.rs:90`). Das Zielschema ist `staged_payload_path(prefix, version, sha256)` (`native/src/updater/core/core.rs:8`), die Präfixe sind `update_download_`, `updater_download_` und `cli_download_` (`native/src/updater/os/shared/staging.rs:9`).
- Installationsarten: `Installation::TerminalOnly { cli }` und `Installation::Desktop { cli, app }` (`native/src/updater/os/shared/terminal.rs:30-36`). `installation_in` (`:72-80`) wählt `Desktop`, wenn eine App aus `installed_app_names()` neben `se` liegt, sonst `TerminalOnly` (`:79`). Namen unter Windows: `Smart Explorer.exe` und `smart_explorer.exe` (`native/src/updater/os/windows.rs:109-111`). Namen unter Linux: `smart_explorer` (`native/src/updater/os/linux_os.rs:102-104`).
  - Laufende `se`-Datei: unter Windows wie gestartet (`native/src/updater/os/windows.rs:118-120`), unter Linux nach Auflösung von Symlinks (`native/src/updater/os/linux_os.rs:112-116`).
  - Ableitung: Eine Ordnerkopie mit `Smart Explorer.exe` und `se.exe` gilt nach dem Dateinamen als `Desktop`. Ob dieser Fall vollständig geprüft ist, ist nicht belegt (Lücke L-09).
  - Präsenz: Das Wort „portable“ kommt in `native/src/updater` nicht vor.
- Ersetzen über den Helfer:
  - `apply_staged_update` (`native/src/updater/os/shared/apply.rs:13-21`) startet den Helfer über `launch_helper` (`:38`).
  - Helfer `native/src/bin/smart_explorer_updater.rs:132-241` (`apply_update`). Ablauf: Prüfsummen von Helfer, App und `se` (`:144-146`); Warten auf das Ende des Elternprozesses bis 300 Sekunden (`:148`); Instanzsperre (`:149`); Pfadprüfung (`:150`, Funktion `validate_modern_paths` in `:329`); Beenden der Zielprozesse (`:154-155`); Archiv der laufenden App mit Prüfsumme (`:163`); Ersetzungstransaktion mit Wiederholungen (`:193`); Prüfung der Prüfsumme des Ziels (`:201`); Start mit `--updated` und Bestätigung (`:224`); Buchführung (`:241`).
  - Fehlerpfad: `restore_failed_update` (`:252-274`) und `verified_previous_candidate` (`:277-294`). Der Wiederherstellungsstart nutzt den Schalter `--update-rollback` (`:67`).
  - Präsenz: Der Schalter `--update-rollback` hat im Bestand (`native/src`) genau einen Treffer, den Aufrufer `native/src/bin/smart_explorer_updater.rs:67`. Ein Empfänger ist nicht gefunden (Lücke L-05).
  - Protokoll aktuell (`native/src/bin/smart_explorer_updater/args.rs:133-160`): `--target`, `--target-sha256`, `--staged`, `--staged-sha256`, `--helper-target`, `--helper-sha256`, `--cli-staged`, `--cli-target`, `--cli-sha256`, `--archive`, `--parent-pid`, `--version`, `--last-applied`, `--error-file`, `--manifest`, `--pin-file`. Zusätzlich der Schalter `--elevated` (`:135`) und der Schalter `--apply`.
  - Protokoll alt (`args.rs:165-185`): `--target`, `--staged`, `--staged-sha256`, `--parent-pid`, `--version`, `--last-applied`, `--error-file`; optional `--helper-sha256` (`:167`).
  - Ersetzungsverfahren laut Dokumentation: `ReplaceFileW` unter Windows, gesichertes Backup mit atomarem Umbenennen unter Linux (`docs/RELEASING.md:978-981`).
  - Windows: keine Selbstersetzung der terminalen `se.exe` (`docs/RELEASING.md:899-900`). Im Quelltext steht dafür die Konstante `NO_CLI_SELF_REPLACEMENT` (`native/src/updater/os/windows.rs:138-141`).
  - Linux ohne grafische Sitzung: `desktop_session()` gibt im Quelltext den Fehler `NO_GRAPHICAL_SESSION` zurück (`native/src/updater/os/linux_os.rs:118-131`, Rückgabe `:124`). Die Bedingung in `:118-123` ist nicht gelesen.
- Zustandsdateien im Datenordner (`native/src/updater/os/shared/config.rs:11-20`): `last_applied_update.txt`, `last_updater_error.txt`, `staged_update.json`.
- Start-Bestätigung: Umgebungsvariablen `SMART_EXPLORER_UPDATE_ACK_PATH`, `SMART_EXPLORER_UPDATE_ACK_TOKEN`, `SMART_EXPLORER_UPDATE_ACK_PAYLOAD` und `SMART_EXPLORER_UPDATE_ACK_SIGNAL` (`native/src/updater/os/shared/startup_ack.rs:5-8`); Dateipräfix `update_start_ack_` (`:9`); Abschlussschema `SMART-EXPLORER-LAUNCH-COMPLETE-V1` (`:10`).

**4.5 Prüfungen vor der Veröffentlichung im CI** (`.github/workflows/build.yml:566-633`, Job `complete-release`): `version.txt` gegen `native/Cargo.toml` (`:566-568`); Existenz aller Feed-Dateien und der Release-Dateien (`:571-593`); Vergleich von Feed-Binärdateien mit den Release-Binärdateien (`:599-609`); APK-Prüfsumme (`:620`); Bindung im Manifest (`:633`).

### F5 Muster einer bestehenden Task-Suite (`luecken-task`)

Gelesen: `.github/workflows/luecken-task.yml` (274 Zeilen; `:1-107` vollständig; `:152-158` und `:243-252` als Ausschnitt; in `:108-268` nur die Zeilen mit den genannten Schlüsselwörtern, per Grep) und `native/test-luecken-task.py` (1479 Zeilen; `:101`, `:181-188`, `:1429-1479` als Ausschnitt; Definitionen per Grep). Alle Zeilen beider Dateien außerhalb der genannten Bereiche sind nicht gelesen.

- Workflow-Name `luecken-task` (`:1`).
- Auslöser: nur `workflow_dispatch` mit der Eingabe `candidate_sha` (Typ `string`, erforderlich; `:27-33`).
- Berechtigungen: `contents: read` (`:35-36`).
- Parallelität: Gruppe `luecken-task-${{ github.repository }}`, `cancel-in-progress: false` (`:38-40`).
- Job `suite` (`:43-106`):
  - Matrix `os: [ubuntu-latest, windows-2025]`, `fail-fast: false` (`:44-47`); `runs-on: ${{ matrix.os }}` (`:48`).
  - Job-Timeout `timeout-minutes: 360` (`:49`).
  - Umgebung (`:50-56`): `CANDIDATE_SHA`, `CARGO_BUILD_JOBS: 2`, `CARGO_INCREMENTAL: 1`, `CARGO_PROFILE_DEV_DEBUG: 0`, `CARGO_PROFILE_TEST_DEBUG: 0`, `CARGO_TERM_COLOR: never`.
  - Schritte in dieser Reihenfolge (`:57-106`): Checkout mit `actions/checkout@v7`, `ref: ${{ github.sha }}`, `fetch-depth: 0`, `persist-credentials: false`; `Bind workflow and checkout to the requested candidate` (`:66-72`): prüft, dass `CANDIDATE_SHA` ein 40-stelliger Hex-Wert ist und mit `GITHUB_SHA` und `HEAD` übereinstimmt; `dtolnay/rust-toolchain@stable` mit `rustfmt`; `actions/setup-python@v6` mit Python `3.12`; `Swatinem/rust-cache@v2` für `native` und `share-server` mit `shared-key: search-recursive-access`; `Run the one task entrypoint` (`:92-98`, Schritt-Timeout `:94`, Befehl `python native/test-luecken-task.py --log-root "$RUNNER_TEMP/luecken-task"` in `:95-97`); `Preserve acceptance and failure evidence` (`:99-106`): `if: always()`, `actions/upload-artifact@v4`, `retention-days: 14`.
- Job `android` (ab `:108`): `runs-on: ubuntu-24.04` (`:109`), Job-Timeout `timeout-minutes: 150` (`:110`). Schritt `Run the one task entrypoint (Android build stage)` (`:152`), Schritt-Timeout `timeout-minutes: 140` (`:154`), Befehl `python native/test-luecken-task.py --job android --log-root "$RUNNER_TEMP/luecken-task-android"` (`:156-157`).
- Job `android-device` (ab `:168`): `runs-on: ubuntu-24.04` (`:169`), Job-Timeout `timeout-minutes: 330` (`:171`). Schritte: Signaturvorbereitung (`:215`); Schritt `Build the candidate APKs and fetch the published APK` (`:225`) mit Timeout `140` (`:227`) und Befehl `python native/test-luecken-task.py --job android-device --phase build --apks "$RUNNER_TEMP/luecken-apks"` (`:234-235`); Entfernen der Signaturdatei (`:238-241`); Schritt `Enable KVM for the dedicated emulator` (`:243-252`) mit Timeout `120` (`:252`); Gerätephase über die `script:`-Eingabe des Emulator-Schritts mit `--job android-device --phase device --apks "$RUNNER_TEMP/luecken-apks" --log-root "$RUNNER_TEMP/luecken-task-android-device/device"` (`:265`).
- Python-Einstieg `native/test-luecken-task.py`:
  - Helfermodule: `helper(name)` lädt `native/luecken-task-<name>.py` (`:181-187`). Im Ordner `native/` liegen die Helfer `luecken-task-android.py`, `luecken-task-server.py`, `luecken-task-settings.py`, `luecken-task-surfaces.py` und `luecken-task-update.py`.
  - `main()` (`:1429-1479`): Argumente `--log-root` (erforderlich, `:1431`), `--job` mit den Werten `host`, `android`, `android-device` (Standard `host`, `:1432`), `--phase` mit den Werten `build`, `device` (Standard `device`, `:1433-1434`), `--apks`. Bei `--job android-device` ist `--apks` Pflicht. Die Ergebnisse sammelt `ai.Results(log_root)`; die Suite endet mit `results.finish()`.
  - Ablauf des Jobs `host` (`:1448-1479`): `ai.stage_fmt` (`:1448`), `ai.stage_build` (`:1449`), `stage_unit` (`:1450`). Nur auf Nicht-Windows: Share-Server-Binary, `stage_hostkeys`, `stage_ftps` (`:1451-1454`). Der Pfad des Entwicklungs-`se` ist `native/target/debug/se` bzw. `se.exe` (`:1455`). Fehlt er oder ist der Build fehlgeschlagen, wird ein Fehlschlag aufgezeichnet und die Suite endet (`:1456-1458`). Danach folgt eine temporäre Arbeitsmappe (Präfix `se-luecken-`), die Stufen `stage_surfaces` und `stage_settings`, auf Windows `stage_windows_secrets`, auf Nicht-Windows `stage_ssh`, `stage_ftp`, `stage_exec` und `helper("update").stage_update`, danach das Aufräumen (`:1456-1478`).
  - Ableitung: Die Suite hat genau einen Einstiegspunkt (`native/test-luecken-task.py`) und verzweigt über `--job` und `--phase`. Sie baut das Entwicklungs-`se` über `ai.stage_build` (`:1449`) und nutzt danach die Datei unter `native/target/debug/`.

## 3. Vollständige Asset- und Payload-Liste

### 3.1 GitHub Release: genau 20 Assets

Quelle: `native/release-publication.ps1:121-156` (Funktion `Get-PublicationReleaseAssetMap`). Die Zählung auf 20 steht in `:158`; dieselbe Zahl prüfen `:1512` und `:1540` gegen das veröffentlichte Release. Der CI-Job `publish-release` ordnet dieselben 20 Dateien zu (`.github/workflows/build.yml:913-933`).

| Nr. | Veröffentlichter Name | Lokale Quelle (Repository-relativ) | Beleg |
|---|---|---|---|
| 1 | `Smart.Explorer.Setup.<V>.exe` | `release-native/Smart Explorer Setup <V>.exe` | `:134-135` |
| 2 | `smart_explorer.exe` | `release-native/update-feed/smart_explorer.exe` | `:137` |
| 3 | `smart_explorer_updater.exe` | `release-native/update-feed/smart_explorer_updater.exe` | `:138` |
| 4 | `se.exe` | `release-native/update-feed/se.exe` | `:139` |
| 5 | `smart_explorer.exe.sha256` | `release-native/update-feed/smart_explorer.exe.sha256` | `:140` |
| 6 | `smart_explorer_updater.exe.sha256` | `release-native/update-feed/smart_explorer_updater.exe.sha256` | `:141` |
| 7 | `se.exe.sha256` | `release-native/update-feed/se.exe.sha256` | `:142` |
| 8 | `smart_explorer` | `release-native/update-feed/smart_explorer` | `:143` |
| 9 | `smart_explorer_updater` | `release-native/update-feed/smart_explorer_updater` | `:144` |
| 10 | `se` | `release-native/update-feed/se` | `:145` |
| 11 | `smart_explorer.sha256` | `release-native/update-feed/smart_explorer.sha256` | `:146` |
| 12 | `smart_explorer_updater.sha256` | `release-native/update-feed/smart_explorer_updater.sha256` | `:147` |
| 13 | `se.sha256` | `release-native/update-feed/se.sha256` | `:148` |
| 14 | `smart-explorer-android.apk` | `release-native/update-feed/smart-explorer-android.apk` | `:149` |
| 15 | `smart-explorer-android.apk.sha256` | `release-native/update-feed/smart-explorer-android.apk.sha256` | `:150` |
| 16 | `install-linux.sh` | `install-linux.sh` | `:151` |
| 17 | `smart_explorer_command.dll` | `release-native/smart_explorer_command.dll` | `:152` |
| 18 | `se-share-server.exe` | `release-native/share-server/se-share-server.exe` | `:153` |
| 19 | `se-share-server-linux` | `release-native/share-server/se-share-server-linux` | `:154` |
| 20 | `version.txt` | `release-native/update-feed/version.txt` | `:155` |

Zuordnung der Nummern 2 bis 13 zu den Zeilen `:137-148` nach Reihenfolge im Quelltext. Die Angabe `<V>` steht für die Version des Release.

### 3.2 Committed Dateien unter `release-native/`, nicht im GitHub Release

Die Menge umfasst 130 getrackte Dateien (`git ls-files release-native`): 108 Installer, 3 portable Dateien, 1 DLL, 2 Share-Server-Dateien und 16 Feed-Dateien (Summe 130). Im Einzelnen:
- `release-native/Smart Explorer.exe` (portable GUI, Erzeugung `native/publish-feed.sh:217`).
- `release-native/Smart Explorer Updater.exe` (portabler Updater, `native/publish-feed.sh:218`).
- `release-native/se.exe` (portable CLI, `native/publish-feed.sh:219`, Veröffentlichung `:350`).
- `release-native/Smart Explorer Setup <V>.exe`: 108 Dateien, je Version eine. Die Verzeichnisliste zeigt die Versionen 0.5.75 bis 0.5.182; die Zählung 108 entspricht dieser lückenlosen Folge. Erzeugung `native/publish-feed.sh:209`, Veröffentlichung `:352`.
- `release-native/smart_explorer_command.dll` (`native/publish-feed.sh:351`).
- `release-native/share-server/se-share-server.exe` und `release-native/share-server/se-share-server-linux` (`native/publish-feed.sh:353-354`).
- `release-native/update-feed/windows-build.manifest` (in 3.3).

### 3.3 Feed `release-native/update-feed/`: 16 Dateien, getrackt

Liste aus `ls` und `git ls-files release-native/update-feed`. Im Verzeichnis liegt außerdem ein leeres Verzeichnis `versions/`, das nicht getrackt ist (Lücke L-12).

| Datei | Rolle | Erzeugung |
|---|---|---|
| `smart_explorer.exe` | Windows-App für den Updater | `native/publish-feed.sh:211` |
| `smart_explorer_updater.exe` | Windows-Updater | `native/publish-feed.sh:212` |
| `se.exe` | Windows-CLI | `native/publish-feed.sh:213` |
| `smart_explorer.exe.sha256` | Prüfsumme der Windows-App | `native/publish-feed.sh:225-234` (Befehl `:233`) |
| `smart_explorer_updater.exe.sha256` | Prüfsumme des Windows-Updaters | `native/publish-feed.sh:225-234` |
| `se.exe.sha256` | Prüfsumme der Windows-CLI | `native/publish-feed.sh:225-234` |
| `smart_explorer` | Linux-App | `native/publish-linux-feed-wsl.sh` (aufgerufen aus `native/publish-feed.sh:192`) |
| `smart_explorer_updater` | Linux-Updater | `native/publish-linux-feed-wsl.sh` |
| `se` | Linux-CLI | `native/publish-linux-feed-wsl.sh` |
| `smart_explorer.sha256` | Prüfsumme der Linux-App | `native/publish-feed.sh:225-234` |
| `smart_explorer_updater.sha256` | Prüfsumme des Linux-Updaters | `native/publish-feed.sh:225-234` |
| `se.sha256` | Prüfsumme der Linux-CLI | `native/publish-feed.sh:225-234` |
| `smart-explorer-android.apk` | Android-APK | `native/publish-release-local.ps1:46` (Name), Veröffentlichung im Feed `:183-186` |
| `smart-explorer-android.apk.sha256` | Prüfsumme der APK | `native/publish-release-local.ps1:183-186` |
| `windows-build.manifest` | bindet Version und Quellcommit an die Windows-Payloads: die Prüfung verlangt genau fünf nichtleere Zeilen, `version=`, `source_commit=` und je eine Zeile für `smart_explorer.exe`, `smart_explorer_updater.exe` und `se.exe` (Format der Payload-Zeilen nicht gelesen) | `native/publish-feed.sh:240-243`, Prüfung `:283-289` |
| `version.txt` | erste Zeile ist die Version; wird als letzter Schritt verschoben | `native/publish-feed.sh:375` |

Die README-Strukturtabelle nennt für den Feed nur `version.txt`, `smart_explorer.exe` / `smart_explorer`, `se.exe` / `se` und `smart-explorer-android.apk` (`README.md:1244`). Lücke L-07.

### 3.4 Dateien im Windows-Installer (`SEC_MAIN`)

`Smart Explorer.exe`, `Smart Explorer Updater.exe`, `se.exe`, `LICENSE`, `licenses\Dokany\NOTICE.txt`, `licenses\Dokany\LICENSE-GPL-3.0.txt`, `licenses\Dokany\LICENSE-LGPL-3.0.txt`, `licenses\Dokany\LICENSE-MIT.txt`, `update_source.txt` (nur wenn nicht vorhanden) und `Uninstall.exe` (`native/installer.nsi:102-155`). Der optionale Abschnitt `SEC_DOKANY` installiert keine Datei dauerhaft; er legt nur die Dokany-MSI temporär in `$PLUGINSDIR` ab (`:194-196`).

Nicht im Windows-Installer enthalten (vollständig gelesen: `native/installer.nsi`): `smart_explorer_command.dll`, `se-share-server.exe`, `se-share-server-linux`, die Android-APK, `windows-build.manifest` und `version.txt`.

### 3.5 Dateien der Linux-Installation (`install-linux.sh`)

Aus dem Feed (Desktop-Pfad, `:265-270`): `smart_explorer`, `smart_explorer_updater`, `se` mit den Prüfsummendateien `smart_explorer.sha256`, `smart_explorer_updater.sha256`, `se.sha256` (`:265-270`); bei `--cli-only` nur `se` und `se.sha256` (`:261-262`).

Installierte Pfade: `$INSTALL_DIR/smart_explorer` (`:359`), `$INSTALL_DIR/smart_explorer_updater` (`:360`), `$INSTALL_DIR/se` (`:362`), `$INSTALL_DIR/update_source.txt` (`:367`), `$BIN_DIR/smart_explorer` als Symlink (`:369`), `$BIN_DIR/se` als Symlink (`:379`), `$ICON_DIR/smart-explorer.png` (`:386`), `$APP_DIR/smart-explorer.desktop` (`:392-403`). Die Variablen sind in `install-linux.sh:8-12` definiert.

## 4. Wiederverwendbare Bausteine

Bestandsaufnahme. Ob ein Baustein für S5 geeignet ist, entscheidet der Strangplan, nicht diese Lesung.

- B-01 Verb-Registrierung unter HKCU mit Löschen nur der eigenen Schlüssel: `native/src/shell_register/os/windows.rs:62-83` (`register_verb`, `unregister_verb`).
- B-02 Eigentumsmarker für einen Umgebungswert: `native/src/bin/se_path_windows/mod.rs:10-79` (`CliPath`, `CliPathOwned`; Entfernen nur bei eigenem Eintrag).
- B-03 Autostart-Eintrag: Windows `native/src/autostart/os/windows.rs:23-28` und `:175-196`; Linux `native/src/autostart/os/linux_os.rs:103-112`, `:98`, `:174`.
- B-04 Erkennung der Installationsart: `native/src/updater/os/shared/terminal.rs:30-80`.
- B-05 Feed-Transport und Payload-Beschreibung: `native/src/updater/os/shared/feed.rs:14-18` (`PayloadSpec`), `:38-47` (Version), `:49-90` (Laden), `:161-172` (Transport); Windows `native/src/updater/os/windows.rs:57-80`; Linux `native/src/updater/os/linux_os.rs:53-74`.
- B-06 Hash-gebundener Helfer mit Wiederherstellung: `native/src/bin/smart_explorer_updater.rs:132-294` (`apply_update`, `restore_failed_update`); Argumentprotokoll mit Pflicht-Prüfsummen `native/src/bin/smart_explorer_updater/args.rs:133-185`.
- B-07 Prüfsumme und atomare Installation unter Linux: `install-linux.sh:287-297`, `:411-424`.
- B-08 Optionaler NSIS-Abschnitt mit stillem Modus: `native/installer.nsi:188-246`, `:259-280`.
- B-09 Zuordnung der Release-Assets mit Zählprüfung und Dublettenprüfung: `native/release-publication.ps1:121-158`, `:638-642`, `:1506-1540`.
- B-10 Suite-Einstieg mit `--job` und `--phase` und Helfer-Lader: `native/test-luecken-task.py:181-187`, `:1429-1479`; Workflow `.github/workflows/luecken-task.yml:27-106`.
- B-11 Seitenleisten-Abschnitt mit Menüs je Zeile (Geräte, Räume): `native/src/app/core/sidebar.rs:132-212`; Schwebefenster `native/src/app/core/share_window_ui.rs:4-13`; Tab-Liste `native/src/app/core/share_pages_ui.rs:19-43`.
- B-12 Bestehende Explorer-Kopplungen: Shell-Kontextmenü `native/src/shell_menu/os/windows.rs:1-5`, `:209`, `:292`; CF_HDROP-Zwischenablage `native/src/shell_clipboard/os/windows/read.rs:18-22`; OLE-Ziehen `native/src/dragout/os/windows.rs:2-3`, `:23`.
- B-13 Dispatch der `se`-Binärdatei mit exakten internen Schaltern: `native/src/bin/se.rs:4-36`, `:79-81`.
- B-14 Panel-Gerüst der Hauptansicht: `native/src/app/core/frame_layout.rs:18-56`.

## 5. Betroffene Verträge

### 5.1 Registrierungen und Dateien

| Nr. | Ort | Gesetzt durch | Entfernt durch |
|---|---|---|---|
| R1 | `HKCU\Software\Classes\Directory\shell\OpenInSmartExplorer` mit Unterschlüssel `command` | `native/installer.nsi:161-163`; Laufzeit `native/src/shell_register/os/windows.rs:95` | `native/installer.nsi:317`; `--unregister` über `native/src/shell_register/os/windows.rs:76-83` und `native/src/lib.rs:99-103` |
| R2 | `HKCU\Software\Classes\Drive\shell\OpenInSmartExplorer` | `native/installer.nsi:164-166`; `native/src/shell_register/os/windows.rs:96` | `native/installer.nsi:318`; `native/src/shell_register/os/windows.rs:100` |
| R3 | `HKCU\Software\Classes\Directory\Background\shell\OpenInSmartExplorer` | `native/installer.nsi:167-169`; `native/src/shell_register/os/windows.rs:97` | `native/installer.nsi:319`; `native/src/shell_register/os/windows.rs:101` |
| R4 | `HKCU\Software\Classes\Directory\shell\open` und `…\Drive\shell\open` (nur bei der nicht produktiven Aktivierung) | `native/src/shell_register/os/windows.rs:157-178` (Aufruf nur im Test `:282`) | `native/src/shell_register/os/windows.rs:180-213`; Start `native/src/lib.rs:105-106` |
| R5 | `HKCU\Environment\Path` (ein Eintrag) | `native/installer.nsi:128`; `native/src/bin/se_path_windows/mod.rs:15-50` | `native/installer.nsi:309`; `native/src/bin/se_path_windows/mod.rs:52-79` |
| R6 | `HKCU\Software\SmartExplorer\Installer` mit Werten `CliPath` und `CliPathOwned` | `native/src/bin/se_path_windows/mod.rs:45-46` | `native/src/bin/se_path_windows/mod.rs:74-75` (nur die Werte) |
| R7 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, Wert `SmartExplorerSync` | `native/src/autostart/os/windows.rs:175-177` (über `:30-42`); GUI `native/src/app/core/settings_background.rs:20` | `native/src/autostart/os/windows.rs:181-196`; GUI `settings_background.rs:41`; CLI `native/src/cli/background.rs:99`; nicht im Deinstaller (Lücke L-01) |
| R8 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartExplorer` | `native/installer.nsi:177-185` | `native/installer.nsi:340` |
| R9 | Windows-Firewall-Regel `Smart Explorer Share Peer Listener` | `native/installer.nsi:146-147` | `native/installer.nsi:320` |
| R10 | Ordner `%LOCALAPPDATA%\Programs\Smart Explorer` mit den Dateien aus 3.4 | `native/installer.nsi:60`, `:102-155` | `native/installer.nsi:322-336` |
| R11 | Verknüpfungen `$SMPROGRAMS\Smart Explorer\Smart Explorer.lnk` und `$DESKTOP\Smart Explorer.lnk` | `native/installer.nsi:172-174` | `native/installer.nsi:337-339` |
| R12 | Datenordner `%APPDATA%\smart_explorer\` (Laufzeit) | `native/src/support_dirs.rs:171-177`, `:196-199` | kein Löschschritt im Deinstaller (Lücke L-10) |
| R13 | Datei `%APPDATA%\smart_explorer\default_manager_backup.txt` | `native/src/shell_register/os/windows.rs:123-140` (nur über `enable_default_manager`) | `native/src/shell_register/os/windows.rs:210` |
| R14 | Sparse-Paket der Kontextmenü-DLL | nicht registriert (Abschnitt 2.3) | entfällt |
| R15 | Linux-Autostart `$XDG_CONFIG_HOME/autostart/smart-explorer-sync-daemon.desktop` oder `$HOME/.config/autostart/…` | `native/src/autostart/os/linux_os.rs:98-112` | `native/src/autostart/os/linux_os.rs:174` (nur über GUI oder CLI); kein Installer-Schritt (Lücke L-02) |
| R16 | Linux: `$INSTALL_DIR`, `$BIN_DIR/smart_explorer`, `$BIN_DIR/se`, `$APP_DIR/smart-explorer.desktop`, `$ICON_DIR/smart-explorer.png` | `install-linux.sh:359-392` | kein Entfernungspfad im Skript (Lücke L-02) |

### 5.2 Schalter, die der Installer, Skripte oder der Updater aufrufen

| Schalter | Aufrufer | Empfänger im Bestand |
|---|---|---|
| `se.exe --install-cli-path` | `native/installer.nsi:128` | `native/src/bin/se.rs:27-30` |
| `se.exe --uninstall-cli-path` | `native/installer.nsi:309` | `native/src/bin/se.rs:31-34` |
| `Smart Explorer.exe --unregister` | `native/installer.nsi:299` | `native/src/lib.rs:99-103` |
| `se.exe --lan-uplink-cleanup` | `native/installer.nsi:303` | Linux-Zweig `native/src/net/mod.rs:92`; Windows-Zweig `native/src/net/os/windows/uplink_adapter.rs:139` (Präsenz; Windows-Pfad nicht gelesen, Lücke L-08) |
| `se.exe drive install-runtime --msi <Datei>` | `native/installer.nsi:201` | `native/src/cli/drive.rs:29-30`, `:74` |
| `se.exe drive runtime` | `native/installer.nsi:251` | `native/src/cli/drive.rs:27-28` |
| `smart_explorer --sync-daemon` (Autostart) | `native/src/autostart/os/windows.rs:23-28`; `native/src/autostart/os/linux_os.rs:104` | GUI `native/src/lib.rs:86` (mit `any`, Lücke L-16); CLI `native/src/bin/se.rs:23` |
| `smart_explorer_updater --apply …` | Start des Helfers `native/src/updater/os/shared/apply.rs:38` | `native/src/bin/smart_explorer_updater.rs:44` |
| `smart_explorer --updated` | `native/src/app/os/linux_os.rs:63-67` (`spawn_updated_app`); `native/src/bin/smart_explorer_updater.rs:224` | `native/src/lib.rs:91` |
| `smart_explorer --update-rollback` | `native/src/bin/smart_explorer_updater.rs:67` | kein Empfänger im Bestand (Lücke L-05) |

### 5.3 Feed-Vertrag

- Die 16 Dateinamen aus 3.3 sind Teil des Vertrags mit dem Updater: `PayloadSpec` nennt `local_names`, `http_names` und `hash_name` (`native/src/updater/os/windows.rs:57-80`, `native/src/updater/os/linux_os.rs:53-74`). Eine Namensänderung an einem Feed-Eintrag wirkt auf diese Stellen.
- `version.txt`: erste Zeile, getrimmt (`native/src/updater/os/shared/feed.rs:38-47`). Leere Datei: Fehler `version.txt im Feed … ist leer` (`native/src/updater/os/shared/flow.rs:76`).
- Sidecar `<name>.sha256`: gelesen mit Größenlimit `MAX_SIDECAR_BYTES` = 64 KiB (`native/src/updater/os/shared/feed_files.rs:16`, Lesefunktion `:47-60`).
- Zeitlimits: `CONNECT_TIMEOUT` 25 s und `READ_TIMEOUT` 60 s (`native/src/updater/os/shared/feed_files.rs:13-15`); `HTTP_TIMEOUT` 25 s (`native/src/updater/os/shared/feed.rs:11`). Benutzeragent `smart-explorer-updater` (`feed.rs:12`).
- Manifest-Bindung: `native/publish-feed.sh:283-289`; CI-Prüfung `.github/workflows/build.yml:566-633`.
- Android-Anschluss: Die App liest `version.txt`, lädt `smart-explorer-android.apk` und prüft ihre `.sha256` (`docs/RELEASING.md:825-827`).

### 5.4 Updater-Vertrag

- Protokollargumente des Helfers: Liste in F4, Unterpunkt „Ersetzen über den Helfer“ (`native/src/bin/smart_explorer_updater/args.rs:133-185`).
- Zustandsdateien im Datenordner: `last_applied_update.txt`, `last_updater_error.txt`, `staged_update.json` (`native/src/updater/os/shared/config.rs:11-20`).
- Staging-Präfixe: `update_download_`, `updater_download_`, `cli_download_` (`native/src/updater/os/shared/staging.rs:9`).
- Start-Bestätigung: Umgebungsvariablen `SMART_EXPLORER_UPDATE_ACK_PATH`, `_TOKEN`, `_PAYLOAD`, `_SIGNAL` (`native/src/updater/os/shared/startup_ack.rs:5-8`); Dateipräfix `update_start_ack_` (`:9`); Schema `SMART-EXPLORER-LAUNCH-COMPLETE-V1` (`:10`).
- Quelle `update_source.txt`: Lesen `native/src/updater/os/shared/config.rs:7-8`, `:57`; Schreiben `:66`; Installer `native/installer.nsi:152-153`; Linux-Installer `install-linux.sh:367` und `:377`.

### 5.5 CI-Vertrag

- Job `complete-release` (`.github/workflows/build.yml:181-189`): nur bei `workflow_dispatch` mit gesetzter Eingabe `complete_release_source_sha`; Parallelitätsgruppe `smart-explorer-complete-release`; Job-Timeout `360` (`:187`); Schritt-Timeout `330` (`:333`).
- Erlaubte Änderungen im Release-Commit (`.github/workflows/build.yml:636-672`): `native/Cargo.toml`, `native/Cargo.lock`, `native/agent-bin/se-agent-x86_64-linux-musl`, `native/agent-bin/se-agent-aarch64-linux-musl` (`:648-649`), die Release-Dateien aus 3.2 und die Feed-Dateien aus 3.3.
- Veröffentlichungsjob `publish-release` (`.github/workflows/build.yml:886`): 20 Zuordnungen von Release-Datei zu Artefakt (`:913-933`).
- Suite-Muster `luecken-task`: Abschnitt F5.

### 5.6 Dokumentationsvertrag

- `README.md:935-989` (Installation): Installer, portable Variante, Linux-Einzeiler (`:940`, `:950`), Dokany-Hinweis.
- `README.md:990-1050` (Updates): Update-Quelle; Anleitung zum Textfeld (`:1001`).
- `README.md:1226-1247` (Struktur): Feed-Zeile `:1244`.
- `README.md:1359-1363` (Datenpfade).
- `docs/RELEASING.md:829-851` (Feed-Liste), `:876-900` (`se update` im Terminal), `:966-995` (Selbstaktualisierung und Freigabe-Kriterien), `:978-981` (Ersetzungsverfahren).
- `docs/WIN11_CONTEXT_MENU.md:3` (Status) und `:50-62` (Signierung).

## 6. Lücken und offene Punkte

Jede Lücke nennt die genaue fehlende Tatsache. Keine Lücke ist bewertet.

- L-01 Autostart nach der Deinstallation: Der Deinstaller (`native/installer.nsi:290-341`) und `--unregister` (`native/src/lib.rs:99-103`) entfernen den Run-Wert `SmartExplorerSync` nicht. Offen ist, ob danach ein Wert verbleibt. Das ist nicht getestet.
- L-02 Linux ohne Entfernungspfad: `install-linux.sh` enthält keinen Entfernungspfad (Präsenz: `uninstall` fehlt). Ein separates Deinstallationsskript ist nicht geprüft. Der Linux-Autostart (`native/src/autostart/os/linux_os.rs:4`) wird vom Skript nicht angelegt und vom Skript nicht entfernt.
- L-03 Windows-11-Menü: Offen ist, ob die Shell beim Verb `Directory\Background` (`%V`) den Parameter `items` übergibt. Ohne `items` führt `Invoke` nichts aus (`native/explorer-command/src/lib.rs:90-110`). Offen sind Signierung und Herausgeber (`native/explorer-command/AppxManifest.xml:29`, `docs/WIN11_CONTEXT_MENU.md:50-62`). Das Paket ist in keinem Skript und Installer registriert (Abschnitt 2.3).
- L-04 Standard-Dateimanager: Die Funktion `enable_default_manager` (`native/src/shell_register/os/windows.rs:157-178`) hat im Produktionscode keinen Aufrufer. Abschalten und Bereinigen bleiben produktiv (`:180-213`, `native/src/lib.rs:105-106`). Ob die Aktivierung entfernt oder zurückgebracht wird, ist im Bestand nicht festgelegt.
- L-05 Schalter `--update-rollback`: Der Aufrufer `native/src/bin/smart_explorer_updater.rs:67` sendet ihn. Im Bestand (Suchbereich `native/src`) gibt es keinen Empfänger. Die Wirkung eines Starts mit diesem Schalter ist ungeklärt.
- L-06 Abweichung zwischen Anleitung und Oberfläche: `README.md:1001` nennt „App → linke Sidebar → Abschnitt `UPDATE`“. Im Suchbereich `native/src/app` gibt es keine Beschriftung „UPDATE“ (Präsenz). Das Feld „Updatequelle ändern“ steht in `native/src/app/core/settings_updates.rs:40-57`. Ob dieser Bereich im Fenster „Einstellungen“ (`native/src/app/core/settings_ui.rs:31`) liegt, ist nicht geprüft.
- L-07 README-Strukturtabelle: Die Zeile zu `release-native/update-feed/` nennt nur `version.txt`, `smart_explorer.exe` / `smart_explorer`, `se.exe` / `se` und `smart-explorer-android.apk` (`README.md:1244`). Die vollständige Liste der 16 Feed-Dateien steht in Abschnitt 3.3, nicht in der README.
- L-08 Windows-LAN-Uplink: `native/src/net/os/windows/uplink_install.rs` (240 Zeilen) ist nicht gelesen. Bekannt sind nur die Funktionsnamen `task_name` (`:108`), `ready` (`:137`), `elevated` (`:150`), `install` (`:173`), `cleanup` (`:186`). Der Deinstallationsschalter `--lan-uplink-cleanup` (`native/installer.nsi:303`) bricht bei Fehler die Deinstallation ab. Welche Systemobjekte (Aufgabe, Firewall, Dienst) der Windows-Pfad anlegt und entfernt, ist nicht belegt.
- L-09 Portable Kopie als `Desktop`: Eine Ordnerkopie mit `Smart Explorer.exe` und `se.exe` wird nach dem Dateinamen als `Desktop` erkannt (Ableitung aus `native/src/updater/os/shared/terminal.rs:72-80` und `native/src/updater/os/windows.rs:109-111`). Ob der Aktualisierungsablauf für diesen Fall geprüft ist, ist nicht belegt. Die README beschreibt die portable Variante mit einmaliger Quelleinstellung (`README.md:935-989`, Abschnitt Installieren, Punkt „Portable“).
- L-10 Datenordner nach der Deinstallation: Der Deinstaller löscht `%APPDATA%\smart_explorer\` nicht (`native/installer.nsi:290-341`; Ableitung aus fehlenden Löschbefehlen). Die README nennt die Datenpfade (`README.md:1359-1363`), und beschreibt das Verhalten bei der Deinstallation nicht (gelesener Abschnitt).
- L-11 Share-Server-Payloads: Der Windows-Installer (`native/installer.nsi`) und `install-linux.sh` installieren `se-share-server.exe` und `se-share-server-linux` nicht (Präsenz). Wo der Share-Server zur Laufzeit gestartet wird, ist nicht gelesen.
- L-12 Verzeichnis `release-native/update-feed/versions/`: leer (`ls`), nicht getrackt (`git ls-files` zeigt keine Datei darin). Der Zweck ist nicht geprüft.
- L-13 Cargo-Binärdateien und Agent-Artefakte: Die `[[bin]]`-Einträge in `native/Cargo.toml` sind nicht gelesen. Der Zweck der committed Dateien `native/agent-bin/se-agent-x86_64-linux-musl` und `native/agent-bin/se-agent-aarch64-linux-musl` (`.github/workflows/build.yml:648-649`) ist nicht gelesen.
- L-14 Teile von `install-linux.sh`: nicht gelesen sind `:65-227` (Hilfsfunktionen, Paketinstallation ab `:132` und `:157`, Abruf ab `:204`) und `:304-349` (Ende der Prüfsummenfunktion und Quellbau-Vorbereitung).
- L-15 Fenster mit variablem Titel: Fünf Aufrufe von `egui::Window::new` verwenden eine Variable `title`: `native/src/app/core/sync_job_log_ui.rs:102`, `copy_dialog.rs:16`, `connect_dialog_ui.rs:54`, `picker_impl.rs:134` und `job_editor_ui.rs:39`. Die tatsächlichen Titel sind nicht aufgelöst.
- L-16 Unterschiedliche Prüfung von `--sync-daemon`: Die GUI prüft das Argument mit `any` (`native/src/lib.rs:86`), die CLI nur als einziges Argument (`native/src/bin/se.rs:23`, `:79-81`). Die Auswirkung dieses Unterschieds ist nicht geprüft.
- L-17 Release-Assets ohne Manifest: `windows-build.manifest` liegt im Feed (Abschnitt 3.3) und wird im CI geprüft (`.github/workflows/build.yml:585`). Es gehört nicht zu den 20 GitHub-Release-Assets (`native/release-publication.ps1:121-156`). Die Liste der Pflicht-Assets in `docs/RELEASING.md:994-995` nennt es ebenfalls nicht.
- L-18 Bedingung ohne Grafik: Die Bedingung der Funktion `desktop_session()` (`native/src/updater/os/linux_os.rs:118-123`) ist nicht gelesen. Die README beschreibt sie als leere Variablen `DISPLAY` und `WAYLAND_DISPLAY` (`README.md:990-1050`, Abschnitt Updates). Ob der Quelltext das so prüft, ist nicht belegt.

## 7. Grenzen der Lektüre

**Gelesen (Zeilenbereiche in Klammern; „vollständig“ bedeutet die ganze Datei):**
- Vollständig: `native/src/main.rs`, `native/src/lib.rs`, `native/src/bin/se.rs`, `native/src/bin/se_path_windows/mod.rs`, `native/src/shell_register/mod.rs`, `native/src/shell_menu/mod.rs`, `native/explorer-command/src/lib.rs`, `native/explorer-command/Cargo.toml`, `native/explorer-command/build-dll.sh`, `docs/WIN11_CONTEXT_MENU.md`, `native/installer.nsi`, `native/src/app/os/windows.rs`, `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-desktop.md`.
- Ausschnitte: `native/src/app/core/sidebar_locations.rs` (`:1-68`; `:69-87` nicht gelesen); `native/src/app/core/frame_update.rs` (`:1-40`); `native/src/bin/smart_explorer_updater/args.rs` (Schlüsselnamen und Funktionsnamen per Grep, Zeilen aus den Grep-Treffern); `native/src/shell_register/os/windows.rs` (`:1-260`; das Testmodul `:259-326` ist nicht gelesen); `native/src/bin/smart_explorer_updater.rs` (`:1-70`); `native/src/updater/os/linux_os.rs` (`:1-117`); `native/src/updater/os/windows.rs` (`:1-16`, `:40-120`); `native/src/updater/os/shared/terminal.rs` (`:25-75`); `native/src/updater/os/shared/feed.rs` (`:14-100`, `:158-172`, `:368-416`); `native/src/updater/os/shared/config.rs` (`:1-22`, `:40-66`); `install-linux.sh` (`:1-64`, `:228-303`, `:350-469`); `native/src/app/core/init.rs` (`:1-40`, `:94-100`, `:378-384`); `native/src/autostart/os/windows.rs` (`:1-45`, `:174-197`); `native/src/autostart/os/linux_os.rs` (`:56-64`, `:103-113`); `native/src/app/os/linux_os.rs` (`:18-30`, `:60-92`, `:184-192`); `native/src/app/os/windows/platform.rs` (`:36-72`, `:196-202`); `native/src/app/os/windows/shell_menus.rs` (`:1-12`); `native/src/app/core/sidebar.rs` (`:1-60`, `:118-165`, `:164-171`, `:191-195`, `:208-214`); `native/src/app/core/mount_peer_roots.rs` (`:1-35`); `native/src/app/core/frame_layout.rs` (`:104-116`); `native/src/app/core/share_window_ui.rs` (`:4-20`); `native/src/app/core/settings_updates.rs` (`:28-46`); `native/src/app/core/menus_settings.rs` (`:128-134`); `native/src/shell_menu/os/windows.rs` (`:1-27`, Kopfkommentar); `native/src/windows_test_namespace.rs` (`:1-12`); `native/src/support_dirs.rs` (`:171-182`, `:196-214`); `native/src/net/mod.rs` (`:90-94`); `native/release-publication.ps1` (`:120-160`); `native/publish-feed.sh` (`:218-236`); `native/publish-linux-feed-wsl.sh` (Kopfkommentar `:1-12`); `.github/workflows/build.yml` (`:17-40`, `:181-200`, `:636-690`, `:886-945`); `.github/workflows/luecken-task.yml` (`:27-107`, `:152-158`, `:243-252`); `native/test-luecken-task.py` (`:181-188`, `:1429-1479`); `docs/RELEASING.md` (`:876-900`, `:966-1000`); `README.md` (`:935-1050`, `:1226-1248`, `:1359-1364`); `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md` (`:1-61`).
- Zeilenausschnitte aus Dateien, die nicht im Leseauftrag standen, und die Begründung: `native/explorer-command/*` (Verzeichnis des Auftrags); `native/release-publication.ps1` (Liste der GitHub-Release-Assets, im Auftrag als „vollständige Asset-Liste“ verlangt); `native/src/app/core/settings_updates.rs`, `frame_layout.rs`, `share_window_ui.rs`, `menus_settings.rs` (Fensterbezug, Auftrag „Fenster/Seitenleiste“); `native/src/support_dirs.rs` (Datenpfad, README-Abgleich); `native/src/net/mod.rs` (nur der Schalter `--lan-uplink-cleanup`); `native/src/windows_test_namespace.rs` (Kopf, Zuordnung zur Shell-Integration geprüft).
- Die Feed-Dateinamen stammen aus `ls` und `git ls-files` der Verzeichnisse `release-native/update-feed` und `release-native`; die Anzahlen (16, 108, 130) stammen aus `git ls-files`.

**Präsenzprüfungen (`grep`, ohne Lektüre der Umgebung):** Suchbereiche und Muster:
- `native/src/app`: `show_viewport`, `ViewportBuilder`, `ViewportId`, `egui::Window::new(`, `"UPDATE"`.
- `native/src` (ganzer Baum): `HKEY_LOCAL_MACHINE`, `HKLM`, `HKEY_CLASSES_ROOT`, `HKCR`, `IShellFolder`, `IContextMenu`, `IExplorerCommand`, `DelegateExecute`, `SHChangeNotify`, `Winlogon`, `ProgID`, `Progman`, `WorkerW`, `SetParent`, `mimeapps`, `x-scheme-handler`, `MimeType=`, `[Desktop Entry]`, `explorer.exe`, `update-rollback`, `lan-uplink-cleanup`, `install-runtime`. `portable` nur im Verzeichnis `native/src/updater`.
- `native/*.ps1`, `native/*.sh`, `native/*.nsi`, `install-linux.sh`: `Add-AppxPackage`, `makeappx`, `AppxManifest`, `signtool`, `Remove-AppxPackage` (keine Treffer); `smart_explorer_command` (Treffer in `native/publish-release-local.ps1`, `native/publish-update.ps1`, `native/release-publication.ps1`, `native/publish-feed.sh`); `uninstall` (nur in `install-linux.sh`, keine Treffer).
- `native/installer.nsi`: `smart_explorer_command`, `share-server`, `apk`, `SmartExplorerSync`, `CurrentVersion\Run`, `autostart` (keine Treffer).
- `native/src/app/core/settings_background.rs` (Aufrufstellen per Grep: `:20`, `:41`).
- `native/src/autostart`, `native/src/cli`, `native/src/mount/mod.rs`, `native/src/net`, `native/src/dragout`, `native/src/shell_clipboard`, `native/src/updater`: Aufrufstellen und Funktionsnamen (`autostart::enable`, `autostart::disable`, `set_context_menu`, `set_context_menu_for`, `drive_mount_supported`, `task_name`).
- `.github/workflows/build.yml`: Zeilen für Timeouts, Job-Namen und Asset-Zuordnungen (`timeout-minutes`, `asset_mappings`, `allowed_release_change`).
- Graph: `graphify query` mit der Frage „Windows shell context menu registration HKCU install uninstall“ einmal, nur lesend (Aufruf über den im Auftrag genannten Werkzeugpfad). Die Trefferliste (206 Knoten im BFS) enthielt mehrere Dateien ohne Bezug zur Registrierung. Aus ihr übernommen ist nur der Hinweis auf `native/src/net/os/windows/uplink_install.rs`, den eine Grep-Prüfung bestätigt hat (L-08). Keine weitere Aussage stammt aus dem Graphen.

**Nicht gelesen**, nach Auftragsbereichen gegliedert:
- `native/src/app/core/state.rs` (nur Grep), `native/src/app/core/init.rs` (nur die Zeilen, die F1 nennt), `native/src/app/core/mount_ui.rs`, `native/src/app/core/mount_ui_draft.rs`, `native/src/app/core/dialogs.rs`, `native/src/app/core/central_tabs.rs` (nur Grep), alle Dateien unter `native/src/app/core/`, soweit sie nicht in den Ausschnitten dieses Abschnitts oder in F1 genannt sind.
- `native/src/app/os/shared/*`, `native/src/app/os/windows/{media_launch,watchers}.rs`, `native/src/app/os/windows/shell_menus.rs` (ab `:13`).
- `native/src/mount/*` außer `mod.rs:279-281`; `native/src/daemon/*`; `native/src/share/*`; `native/src/cli/*` außer Grep-Treffern (`cli/integration.rs`, `cli/background.rs`, `cli/drive.rs`, `cli/os/windows.rs`); `native/src/net/os/windows/uplink_install.rs` (nur Funktionsliste); `native/src/notify_desktop/*`; `native/src/mobile/*`.
- `native/src/updater/core/*`; `native/src/updater/os/shared/{apply,archive,cli_swap,staging,flow,startup_ack,feed_files}.rs` außer den genannten Zeilen und Grep-Treffern; `native/src/bin/smart_explorer_updater/*` außer `args.rs` (Grep) und dem Hauptmodul (`:1-70`, `:132-241`, Grep); `native/src/bin/bench.rs` außer dem Kopfkommentar.
- `native/publish-release-local.ps1` und `native/publish-update.ps1` außer den genannten Zeilen und Grep-Treffern; `native/publish-linux-feed-wsl.sh` außer dem Kopfkommentar; `native/Cargo.toml`; `native/build-agent-bundles.sh`.
- `.github/workflows/build.yml` außer den genannten Zeilen; `.github/workflows/luecken-task.yml`: alle Zeilen außer den in F5 genannten; `native/test-luecken-task.py` außer den genannten Zeilen und Grep-Treffern; die Module `native/luecken-task-android.py`, `native/luecken-task-server.py`, `native/luecken-task-settings.py`, `native/luecken-task-surfaces.py` und `native/luecken-task-update.py` (Inhalt nicht gelesen).
- `docs/RELEASING.md` und `README.md` außer den genannten Abschnitten.
- `android/*`, `native/android-bridge/*`, `native/src/app/*` außerhalb der Fensterbezüge (Die Android-Seite wird in einem separaten Auftrag gelesen).
- `share-server/*` außer dem Crate-Listing (`git ls-files share-server`).

**Annahmen (ausdrücklich):**
- A-1: Im Plan bezeichnet „se Desktop“ (Abschnitt A, E4 und E6) die Desktop-Anwendung Smart Explorer. Die Annahme stützt sich auf die Nutzerformulierung „via this se desktop“ (`docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md:18`); der Bestand legt diesen Begriff nicht fest.
- A-2: Die Namen in `PayloadSpec` (`local_names`, `http_names`, `hash_name`) stimmen mit den Dateinamen des Feeds überein. Der Vergleich der Liste aus Abschnitt 3.3 mit `native/src/updater/os/windows.rs:57-80` und `native/src/updater/os/linux_os.rs:53-74` ergibt Übereinstimmung für alle zwölf Namen (sechs Payloads und sechs Prüfsummendateien). Das ist keine Laufzeitprüfung.
- A-3: Das Fehlen eines Aufrufs `autostart::disable` im Deinstaller (L-01) ist eine Ableitung aus dem gelesenen Quelltext. Eine Laufzeitprüfung der Deinstallation fand nicht statt.
- A-4: Die Angabe „108 Installer“ stammt aus `git ls-files`. Die Versionsfolge 0.5.75 bis 0.5.182 stammt aus der Verzeichnisliste. Eine Prüfung der Dateiinhalte fand nicht statt.

**Keine Daten der Geräte des Nutzers** sind in diese Datei aufgenommen. Angegeben sind nur Repository-Pfade, Dateinamen, Schlüsselnamen und Platzhalter wie `%APPDATA%`.
