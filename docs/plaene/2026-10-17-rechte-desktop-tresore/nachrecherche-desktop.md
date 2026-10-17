# Nachrecherche S5 — se Desktop und se Launcher (DN1–DN30, Revalidierung Abschnitt C)

Stand und Abrufdatum 2026-10-17. Codestand der Lektüre: `da2646d2`. Bezug: `strang-desktop.md` Abschnitt C und G. Nichts gebaut, kompiliert oder ausgeführt (AGENTS.md); Crate-Quelltexte nur lesend aus dem lokalen Cargo-Register `~/.cargo/registry/src/index.crates.io-*/` (im Folgenden `REG/`). Abkürzungen wie im Strangplan: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

Kennzeichnung: **Befund** = in Quelle oder Code nachgelesen; **Folgerung** = Schluss aus Befunden (Planfolge); **Annahme** = ungeprüft; **Lücke** = genau benannte fehlende Angabe. Diese Datei bewertet nicht neu und entscheidet nicht; Entscheidungen trifft der Planverfasser.

## 1 Antworten DN1–DN30

Reihenfolge der Unterabschnitte nach Bearbeitung: DN2, DN10, DN21, DN22, DN24, DN26, DN30, DN1, DN3, DN4, DN5, DN6, DN7, DN23, DN27, DN8, DN9, DN11, DN12, DN13, DN28, DN14, DN15, DN16, DN17, DN18, DN19, DN20, DN25, DN29 (jede der 30 Fragen genau einmal).

### DN2 — winit 0.30.13 „always-on-bottom“, Variantenname egui 0.29.1, `with_taskbar(false)`

**Antwort.**
- Variantenname: `egui::viewport::WindowLevel::AlwaysOnBottom` existiert (Befund `REG/egui-0.29.1/src/viewport.rs:835-838`); egui-winit bildet ihn auf `winit::window::WindowLevel::AlwaysOnBottom` ab, beim Erzeugen (`REG/egui-winit-0.29.1/src/lib.rs:1621-1624`) und per `ViewportCommand::WindowLevel` (`:1443-1446`).
- winit Windows setzt die unterste Lage **nur einmal beim Wechsel des Flags**: `SetWindowPos(window, HWND_BOTTOM, 0,0,0,0, SWP_ASYNCWINDOWPOS | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)` (Befund `REG/winit-0.30.13/src/platform_impl/windows/window_state.rs:339-355`). Die Behandlung von `WM_WINDOWPOSCHANGING` in winit betrifft ausschließlich den Vollbildzustand (`if let Some(ref mut fullscreen) = window_state.fullscreen`, `…/windows/event_loop.rs:1299-1302`); das Flag `ALWAYS_ON_BOTTOM` kommt in `event_loop.rs` nicht vor. Folgerung: Bei Aktivierung (Klick) hebt Windows das Fenster nach oben; winit hält die unterste Lage **nicht**. Die im Plan als Rückfall vorgesehene Unterklasse (`SetWindowSubclass`, `hwndInsertAfter = HWND_BOTTOM` in `WM_WINDOWPOSCHANGING`) ist damit Pflicht, nicht Rückfall.
- `with_taskbar(false)` → egui-winit `with_skip_taskbar(true)` (`REG/egui-winit-0.29.1/src/lib.rs:1726-1727`, nur `cfg(target_os = "windows")`) → winit `set_skip_taskbar` ruft ausschließlich `ITaskbarList::DeleteTab(hwnd)` (`REG/winit-0.30.13/src/platform_impl/windows/window.rs:1498-1535`); `WS_EX_TOOLWINDOW` setzt winit nur für sein internes Hilfsfenster (`event_loop.rs:932-940`), nicht für Anwendungsfenster. Folgerung: `DeleteTab` entfernt nur den Taskleistenknopf; ein Alt+Tab-Eintrag bleibt, bis der Adapter `WS_EX_TOOLWINDOW` setzt (Planannahme bestätigt).
- winit setzt `WS_EX_APPWINDOW`, solange das Flag `ON_TASKBAR` gilt (`window_state.rs:273-275`). Folgerung: Setzt der Adapter `WS_EX_TOOLWINDOW` per `SetWindowLongPtrW`, muss er `WS_EX_APPWINDOW` gleichzeitig entfernen, sonst bleibt der Fenstereintrag sichtbar (Annahme zur Wirkung bei gleichzeitigem Setzen beider Stile; Microsoft-Beleg nicht gelesen).
- Zugriff auf das HWND weiterer Viewports: eframe 0.29.1 bietet `HasWindowHandle` nur für `CreationContext` und `Frame` (`REG/eframe-0.29.1/src/epi.rs:96`, `:621-626`), beides das Fenster des Wurzel-Viewports. Für zusätzliche Viewports (Plan C1 L3: „je Monitor ein Viewport“) liefert eframe 0.29.1 kein Fensterhandle (Befund: keine weitere Implementierung in `REG/eframe-0.29.1/src`). Folgerung: Der Adapter muss das HWND je Viewport selbst finden (Lücke: Verfahren, zum Beispiel `EnumThreadWindows` mit Fenstertitel, ist nicht festgelegt) oder je Monitor einen eigenen `--desktop-surface`-Prozess mit nur einem Wurzel-Viewport starten.
- `with_msg_hook` (`REG/winit-0.30.13/src/platform/windows.rs:199-226`) sieht nur gepostete Nachrichten vor `DispatchMessage`; `WM_WINDOWPOSCHANGING` wird gesendet, nicht gepostet (Folgerung aus Win32-Nachrichtenmodell, Microsoft-Seite nicht abgerufen). Folgerung: Der Hook ersetzt `SetWindowSubclass` nicht.

**Belege.** Pfade oben (Crate-Versionen exakt wie im Lock: egui 0.29.1, egui-winit 0.29.1, eframe 0.29.1, winit 0.30.13), Abruf 2026-10-17.

**Verbleibende Lücke.** Verhalten der Unterklassen-Lösung bei `Win+Tab`/Taskansicht und bei Aktivierung über Klick ungemessen.

### DN10 — Layer-Backend ohne Systembibliotheken zur Bauzeit

**Antwort.**
- SCTK 0.19.2 Features: `default = ["calloop", "xkbcommon"]`, `calloop = ["dep:calloop", "calloop-wayland-source"]`, `xkbcommon = ["dep:xkbcommon", "bytemuck", "pkg-config", "xkeysym/bytemuck"]` (Befund `REG/smithay-client-toolkit-0.19.2/Cargo.toml`, Abschnitt `[features]`). Ohne `xkbcommon` fehlt das Modul `seat::keyboard` vollständig (`REG/smithay-client-toolkit-0.19.2/src/seat/mod.rs:22-23`, `#[cfg(feature = "xkbcommon")] pub mod keyboard;`); `seat::pointer`, `seat::touch` und `shell::wlr_layer` bleiben (`src/shell/wlr_layer`, `src/seat/pointer`). Folgerung: Die Tastatur muss der Backend selbst über `wl_keyboard` (wayland-client `Dispatch`) und `xkbcommon-dl` 0.4.2 abbilden; SCTK hilft dabei nicht.
- winit 0.30.13 bindet SCTK genau mit `default-features = false, features = ["calloop"]` (`REG/winit-0.30.13/Cargo.toml:263-268`). Folgerung: Eine eigene Abhängigkeit `smithay-client-toolkit = { version = "0.19.2", default-features = false, features = ["calloop"] }` erzeugt keinen neuen Lock-Eintrag (Lock-Eintrag `native/Cargo.lock:6676-6698` enthält bereits `calloop 0.13.0`, `calloop-wayland-source 0.3.0`, `wayland-protocols-wlr`, `xkeysym`, kein `xkbcommon`).
- `dlopen`: winit-Feature `wayland-dlopen = ["wayland-backend/dlopen"]`, Teil von winit `default` (`REG/winit-0.30.13/Cargo.toml:70-117`). Aktiv im se-Bau, weil eframe 0.29.1 `egui-winit` ohne `default-features = false` einbindet (`REG/eframe-0.29.1/Cargo.toml:202-207`) und egui-winit `default` `"wayland"`, `"winit/default"`, `"x11"` enthält (`REG/egui-winit-0.29.1/Cargo.toml`, `[features]`). Lock: `wayland-sys` 0.31.11 mit `dlib` (`native/Cargo.lock:7904-7913`). Hinweis: `native/Cargo.toml:244` setzt eframe mit `default-features = false` und ohne `wayland`/`x11`; Wayland/X11 kommen nur über diesen Umweg (Befund).
- x11rb 0.13.2 wird von winit mit `dl-libxcb`, `randr`, `xkb`, `xinput`, `resource_manager`, `allow-unsafe-code` eingebunden (`REG/winit-0.30.13/Cargo.toml:299-310`). Folgerung: EWMH- und RandR-Abfragen über x11rb mit denselben Features erzeugen keinen neuen Lock-Eintrag und laden `libxcb` zur Laufzeit.
- wgpu 22.1.0: `SurfaceTargetUnsafe::RawHandle { raw_display_handle, raw_window_handle }` und `unsafe fn Instance::create_surface_unsafe` existieren (`REG/wgpu-22.1.0/src/lib.rs:714-725`, `:2579`). egui-wgpu 0.29.1 re-exportiert `wgpu` (`REG/egui-wgpu-0.29.1/src/lib.rs:21`) und bietet `Renderer::new(device, output_color_format, output_depth_format, msaa_samples, dithering)`, `update_texture`, `update_buffers`, `render(&mut wgpu::RenderPass<'static>, …)` ohne eframe (`REG/egui-wgpu-0.29.1/src/renderer.rs:210-216`, `:417-421`, `:540`, `:844`). Folgerung: Der Renderpfad ohne eframe ist mit vorhandenen Versionen möglich; `wgpu` über `egui_wgpu::wgpu` statt eigener Abhängigkeit vermeidet eine Versionsdrift.
- Lock bleibt ohne neue Pakete, wenn der Backend nur SCTK 0.19.2 (`calloop`), `wayland-client` 0.31.14, `wayland-protocols-wlr` 0.3.12, `calloop` 0.13.0, `xkbcommon-dl` 0.4.2, `x11rb` 0.13.2 und `egui_wgpu::wgpu` nutzt (Folgerung aus den Lock-Einträgen; nicht durch `cargo` geprüft, AGENTS.md).

**Verbleibende Lücke.** Ob `wayland-protocols-wlr` 0.3.12 im Lock mit Feature `client` aktiviert ist (SCTK benötigt es; Lock-Datei führt keine Features), ist ohne Cargo-Aufruf nicht belegbar; Annahme: ja, weil SCTK `wlr_layer` übersetzt.

### DN21 — GitHub-Runner für die Suite

**Antwort.**
- Windows: Die Sitzung der GitHub-Windows-Runner ist interaktiv. Mitarbeiterkommentar in `actions/runner-images` #14049 vom 2026-08-07: „the session is actually interactive on `windows-11-arm` — `UserInteractive=True`, it's on the interactive window station (`WinSta0`)“; auf dem ARM-Bild scheitert nur `SetForegroundWindow` (Befund, Abruf über GitHub-API 2026-10-17; dieselbe Quelle nutzt S4 N7 in `nachrecherche-bildschirm.md:108-121`). Der Bestand nutzt `windows-2025` (12 Jobs) und `windows-latest` (2 Jobs) (Befund `runs-on` in `.github/workflows/*.yml`).
- Ob `explorer.exe` als Shell mit `Progman`/`SHELLDLL_DefView` auf `windows-2025` läuft: **Lücke** — keine Quelle in `actions/runner-images` gefunden (GitHub-Suche nach `explorer.exe` in Issues: kein Treffer zum Thema). Die Bild-Vorlage setzt `AutoAdminLogon` nur während des Bildbaus und entfernt es danach (`images/windows/templates/build.windows-11-arm64.pkr.hcl:54-60`, `:210-216`). Folgerung: Die Suite muss das Vorhandensein von `Progman` selbst prüfen (`FindWindowW(L"Progman")`) und den Ausgang berichten, statt es vorauszusetzen.
- Ubuntu 24.04 (noble), Paketversionen laut packages.ubuntu.com (Abruf 2026-10-17): `openbox` 3.6.1-12build5, `xfwm4` 4.18.0-1build3, `xfdesktop4` 4.18.1-1build3, `sway` 1.9-1build2, `grim` 1.4.0+ds-2build2, `x11-utils` 7.7+6build2, `wmctrl` 1.07-7ubuntu3, `xdotool` 1:3.20160805.1-5build1, `labwc` 0.7.1-1build1. Bestand installiert bereits `xvfb`, `xauth`, `x11-utils` (`.github/workflows/build.yml:305-307`, `:417-418`).
- Sway 1.9 `create_output`: Befehl vorhanden (`sway/commands/create_output.c` Tag 1.9); bei Headless-Backend `wlr_headless_add_output(backend, 1920, 1080)`, sonst Fehler „Can only create outputs for Wayland, X11 or headless backends“ (Befund https://raw.githubusercontent.com/swaywm/sway/1.9/sway/commands/create_output.c, Zeilen 21-22, 45). In `sway.5.scd` und `sway-output.5.scd` von 1.9 ist der Befehl nicht dokumentiert (Befund: kein Treffer). `WLR_BACKENDS=headless` ist eine wlroots-Umgebungsvariable (Annahme ohne Abruf der wlroots-0.17-Doku `docs/env_vars.md`; Lücke: Renderer-Wahl ohne GPU, `WLR_RENDERER=pixman`, nicht belegt).

**Folgerung für den Plan.** DSK-M15 kann Windows-Fensterprüfungen in der Runner-Sitzung ausführen; Explorer-Desktop-Prüfungen brauchen eine Vorabprüfung mit berichtetem Ausgang. xfwm4 und xfdesktop4 sind über `universe` installierbar.

### DN22 — `image` 0.25.10 Features `jpeg`, `webp`, `bmp`

**Antwort.**
- `bmp = []` (keine Abhängigkeit), `jpeg = ["dep:zune-core", "dep:zune-jpeg"]`, `webp = ["dep:image-webp"]` (Befund `REG/image-0.25.10/Cargo.toml`, `[features]`); Abhängigkeitsangaben `zune-jpeg = "0.5.5"`, `zune-core = "0.5.0"`, `image-webp = "0.2.0"` (dort `:181-183`, `:226-233`).
- Lock heute: `image` 0.25.10 mit `bytemuck`, `byteorder-lite`, `moxcms`, `num-traits`, `png` (`native/Cargo.lock:3244-3254`); `image` ist keine direkte Abhängigkeit von `native/Cargo.toml` (Befund: kein Eintrag `image =`). `zune-jpeg`, `zune-core`, `image-webp`, `quick-error` fehlen im Lock (Befund).
- Neue Lock-Einträge (Folgerung aus crates.io-API, Abruf 2026-10-17): `zune-jpeg` (neueste stabile 0.5.15, Lizenz „MIT OR Apache-2.0 OR Zlib“, Abhängigkeit `zune-core ^0.5.1`), `zune-core` (0.5.3, „MIT OR Apache-2.0 OR Zlib“, optionale `log`, `serde`), `image-webp` (0.2.4, „MIT OR Apache-2.0“, Abhängigkeiten `byteorder-lite ^0.1.0` — schon im Lock —, `quick-error ^2.0.1`), `quick-error` (2.0.1, „MIT/Apache-2.0“). Genau vier neue Pakete; alle Lizenzen MIT-verträglich.

**Verbleibende Lücke.** Exakte Versionen hängen vom Auflösungszeitpunkt in CI ab (Annahme: neueste kompatible wie oben).

### DN24 — Updater-Helfer, Installer und laufende `--desktop-shell`-Prozesse

**Antwort.**
- Der Updater-Helfer **beendet keine Prozesse**. `stop_target_processes_for_update` schreibt `<appdata>/sync/daemon.stop` = `stop` (`N/bin/smart_explorer_updater/process.rs:80-84`, `:181`), wartet bis zu 30 s, dass alle Prozesse mit dem Zielabbild von selbst enden, und bricht sonst mit „Smart Explorer laeuft noch und blockiert das Update: <PID (Pfad)>“ ab (Windows `:179-207`; Linux `:110-135`). Erkennung Windows: Prozessname gleich Dateiname des Ziels und `QueryFullProcessImageNameW` gleich Zielpfad (`:217-264`), also unabhängig von Argumenten — ein `Smart Explorer.exe --desktop-shell` aus `$INSTDIR` zählt mit. Linux: `/proc/<pid>/exe` kanonisch gleich Ziel (`:137-169`). Kein `TerminateProcess` und kein `kill` mit Signal ≠ 0 im Updater (Befund `grep` über `N/bin/`, `N/updater/`; `libc::kill(pid, 0)` nur zur Existenzprüfung `process.rs:57`). Aufruf für GUI-Ziel und CLI-Ziel (`N/bin/smart_explorer_updater.rs:150-160`).
- Wer das Signal beachtet: `crate::daemon::update_stop_marker()` (`N/daemon/os/shared/handoff.rs:158-172`) wird gelesen von `N/cli/mcp/update_stop.rs`, `N/cli/shell_repl.rs:269-274`, `N/cli/shell_terminal.rs:175-368`; der Daemon über `stop_requested_*` (`handoff.rs`). Die GUI-App (`N/app/`) liest es nicht (Befund `grep`). Folgerung: Ein Wächter `--desktop-shell` und eine Fläche `--desktop-surface`, die `update_stop_marker()` nicht beachten, **blockieren jedes Desktop-Update** nach 30 s. Die Planaussage B „Der Updater-Helfer beendet alle Prozesse“ ist widerlegt (Abschnitt 2).
- NSIS-Installer: `SEC_MAIN` beendet vor dem Schreiben alle Prozesse mit `taskkill /F /T /FI "IMAGENAME eq Smart Explorer*"`, `/IM "smart_explorer.exe"`, `/IM "se.exe"` (`native/installer.nsi:88-91`, Wiederholung `:110-111`); das trifft `--desktop-shell` und `--desktop-surface`. `.onInstSuccess` startet nur bei nicht stillem Lauf und ohne Dokany-Neustartbedarf `Exec '"$INSTDIR\${EXE_NAME}"'` (`:283-288`); bei stiller Installation startet es nichts (Planannahme bestätigt). Deinstallation beendet dieselben Abbilder (`:292-294`) und ruft danach `--unregister` per `nsExec::ExecToStack` (ohne Abbruch bei Fehler, `:299`), `--lan-uplink-cleanup` und `--uninstall-cli-path` mit Abbruch (`:303-313`).
- Wiederanlauf nach Helfer-Update: Der Helfer startet die App mit `--updated` (`N/bin/smart_explorer_updater.rs:224`); ein Wächter wird nur über den im Plan vorgesehenen `desktop::resume_on_gui_start` wieder gestartet (Planannahme).

**Folgerung für den Plan.** Wächter und Fläche müssen `daemon::update_stop_marker()` mit derselben Grundlinienregel wie `N/cli/mcp/update_stop.rs` (neuer oder neu geschriebener Marker beendet; bei Start vorhandener wird ignoriert) im Abstand ≤ 1 s prüfen und sich beenden; Suite-Prüfung „Update mit laufendem se Desktop“ nötig.

### DN26 — `windows` 0.58: Feature-Namen

**Antwort** (Befund: Modulpfad in `REG/windows-0.58.0/src/Windows/Win32/…` = Feature-Name):

| Element | Modul/Feature | in `native/Cargo.toml:201-232` (Crate `windows`) |
|---|---|---|
| `IDesktopWallpaper`, CLSID `DesktopWallpaper` | `Win32_UI_Shell` (`UI/Shell/mod.rs:34522`) | ja |
| `ICustomDestinationList`, CLSID `DestinationList`, `SetCurrentProcessExplicitAppUserModelID`, `IShellLinkW`, `SHGetKnownFolderPath`, `FOLDERID_Desktop`, `FOLDERID_PublicDesktop`, `SetWindowSubclass` | `Win32_UI_Shell` | ja |
| `ICustomDestinationList::AppendCategory`, `AddUserTasks`; `IObjectCollection`, `IObjectArray`; CLSID `EnumerableObjectCollection` | `Win32_UI_Shell_Common` (Methoden mit `#[cfg(feature = "Win32_UI_Shell_Common")]`) | ja |
| `IPropertyStore` (Titel der Sprunglisten-Einträge) | `Win32_UI_Shell_PropertiesSystem` | **nein** |
| `PKEY_Title` | `Win32_Storage_EnhancedStorage` | **nein** |
| `PROPVARIANT`-Hilfen (`InitPropVariantFromStringVector`) | `Win32_System_Com_StructuredStorage` | ja |
| `EnumDisplayMonitors`, `GetMonitorInfoW` | `Win32_Graphics_Gdi` | ja |
| `GetDpiForMonitor` | `Win32_UI_HiDpi` | **nein** (nur nötig, wenn genutzt) |
| `SetWindowPos`, `RegisterWindowMessageW`, `SetWindowLongPtrW` | `Win32_UI_WindowsAndMessaging` | ja |

`AppendCategory` verlangt für jedes `IShellLink`: Ziel (PIDL oder Pfad), Argumente, Symbolort und den Anzeigenamen über `System.Title` (`PKEY_Title`) (Befund MicrosoftDocs `sdk-api` `nf-shobjidl_core-icustomdestinationlist-appendcategory.md`, Abschnitt Remarks). Folgerung: Die zwei fehlenden Features `Win32_UI_Shell_PropertiesSystem` und `Win32_Storage_EnhancedStorage` sind für L10 Pflicht.

### DN30 — `--open-location` zu einem Offline-Gerät

**Antwort.**
- Einen Startschalter `--open-location` gibt es im Bestand nicht (Befund `grep "open-location"` über `N/`: kein Treffer); er ist in DSK-M2 neu geplant (`strang-desktop.md` D, DSK-M2 „Startschalter des GUI-Binarys“). Heute öffnet das erste Argument ohne `--` nur einen vorhandenen lokalen Ordner (Strangplan B, `N/lib.rs:108-112`).
- Ablauf im Bestand für Share-Ziele: `navigate_to_location` → `PeerOpenTarget::from_endpoint` → `open_share_target_at` (`N/app/core/prefs_tabs.rs:123-128`, `N/app/core/share_navigation.rs:7-39`) → `open_share_target` (`N/app/core/share.rs:265-313`). Direktkontakte ohne `DirectAccessState::Accepted` werden ohne Verbindungsversuch auf `WaitingForAccess` gesetzt (`share.rs:277-292`). Sonst startet ein Thread `share-open`, der `daemon::open_share_backend` ausführt; die UI bleibt unblockiert und empfängt das Ergebnis über einen Kanal (`share.rs:294-312`).
- Zeitverhalten: `open_share_backend` versucht bis zu 8-mal: Verbindung zum Worker mit 2 s, Antwort-Lesefrist 3 s je Versuch, Pausen 250 ms bzw. 750 ms (`N/daemon/os/shared/ipc_client.rs:22-88`). Der Worker probiert je Anfrage bis zu 45 s (`N/daemon/os/shared/ipc_host.rs:355-383`, Schlaf 750 ms) in einem Thread je Client (`N/daemon/os/shared/ipc_listener.rs:148-165`). Folgerung: Bei einem Offline-Gerät erscheint das Fenster sofort mit Zustand „Verbindet“; der Client gibt nach höchstens etwa 8 × (2 s + 3 s + 0,75 s) ≈ 46 s mit dem letzten Fehler auf (Annahme zur oberen Schranke: Worker antwortet innerhalb 3 s nie mit Erfolg), während worker-seitig bis zu 8 Probeschleifen zu je 45 s weiterlaufen können. Kein Hängen der UI.

**Verbleibende Lücke.** Ob `probe_backend_for_target` für ein Offline-Gerät sofort mit Fehler zurückkehrt oder blockiert, ist nicht gelesen.

### DN1 — Win+D, „Desktop anzeigen“, Aero Peek bei einem untersten Fenster

**Antwort.**
- Mechanismus „Desktop anzeigen“ (Win+D): Raymond Chen (Microsoft), 2024-10-21: „the Show Desktop command Win+D first minimizes all windows that can be minimized, and then moves the desktop window to the top of the z-order.“ Ein Fenster ohne `WS_MINIMIZEBOX` wird nicht minimiert (`IsIconic` bleibt `FALSE`). Befund https://devblogs.microsoft.com/oldnewthing/20241021-00/?p=110393 (über `wp-json/wp/v2/posts/110393`, Abruf 2026-10-17); gleiche Aussage 2004-05-27 https://devblogs.microsoft.com/oldnewthing/20040527-00/?p=39153 (Desktop wird „angehoben“ und beim Rückgängigmachen wieder „abgesenkt“).
- Folgerung: Das se-Fenster ohne Minimieren-Knopf bleibt bei Win+D offen, der Explorer-Desktop (mit Symbolebene) liegt danach darüber; ein zweites Win+D senkt den Desktop wieder ab. Die Planannahme DR1/DN1 („Desktop anzeigen hebt den Explorer-Desktop über se“) ist damit für den Mechanismus bestätigt; für 24H2/25H2 gibt es keinen separaten Microsoft-Beleg, ob das „Anheben“ weiter über dasselbe Desktopfenster läuft (Lücke).
- winit setzt `WS_MINIMIZEBOX`, solange `WindowButtons::MINIMIZE` gilt (`REG/winit-0.30.13/src/platform_impl/windows/window_state.rs:267-268`, `window.rs:312`), und zwar auch ohne Dekoration (`to_window_styles` beginnt immer mit `WS_CAPTION | WS_BORDER | WS_CLIPSIBLINGS | WS_SYSMENU`, `window_state.rs:257-259`). Folgerung: „Kein Minimieren-Rahmen“ verlangt ausdrücklich `ViewportBuilder::with_minimize_button(false)` (`REG/egui-0.29.1/src/viewport.rs:519`); `with_decorations(false)` genügt nicht.
- Win+M („Minimize All“) minimiert nur Fenster mit Minimieren-Knopf (Chen 2004, oben). Folgerung: ohne `WS_MINIMIZEBOX` bleibt se bei Win+M sichtbar.
- Aero Peek (Zeigen auf die Taskleistenecke): **Lücke** — keine Microsoft-Quelle zum Verhalten bei untersten Fenstern gefunden.
- Windows 10 22H2, 11 23H2: gleicher Chen-Mechanismus (Artikel 2024 ohne Versionsvorbehalt); 24H2/25H2: Lücke wie oben. Windows 11 25H2 ist ein Funktionsupdate per Aktivierungspaket auf 24H2-Basis („Devices updating from Windows 11, version 24H2 use an enablement package. Most the files for the 25H2 update already exist on Windows 11, version 24H2 devices“, https://learn.microsoft.com/en-us/windows/whats-new/whats-new-windows-11-version-25h2, ms.date 2025-10-03, updated_at 2025-10-17). Folgerung: 25H2 teilt die 24H2-Desktophierarchie (Annahme auf Grund der gemeinsamen Dateibasis).

**Verbleibende Lücke.** Messung auf 24H2/25H2 und Aero Peek; Ort: Suite-Prüfung oder Handtest.

### DN3 — Navigationsbereich-Wurzel: vollständige Werte, Dateidialoge, fehlendes Ziel

**Antwort.** Die zwölf Schritte der Microsoft-Seite (Befund https://raw.githubusercontent.com/MicrosoftDocs/win32/docs/desktop-src/shell/integrate-cloud-storage.md, ms.date 05/31/2018, letzter Commit 2025-03-11 `f8866e4e2f`; Learn-Seite `updated_at` 2025-03-11T21:20:00Z, Abruf 2026-10-17), alle unter HKCU:

| Schritt | Schlüssel | Wert | Typ | Daten |
|---|---|---|---|---|
| 1 | `Software\Classes\CLSID\{CLSID}` | (Standard) | REG_SZ | Anzeigename |
| 2 | `…\{CLSID}\DefaultIcon` | (Standard) | REG_EXPAND_SZ | Symbolpfad, Beispiel `%SystemRoot%\system32\imageres.dll,-1043` |
| 3 | `…\{CLSID}` | `System.IsPinnedToNameSpaceTree` | REG_DWORD | `0x1` |
| 4 | `…\{CLSID}` | `SortOrderIndex` | REG_DWORD | `0x42` |
| 5 | `…\{CLSID}\InProcServer32` | (Standard) | REG_EXPAND_SZ | `%systemroot%\system32\shell32.dll` |
| 6 | `…\{CLSID}\Instance` | `CLSID` | REG_SZ | `{0E5AAE11-A475-4c5b-AB00-C66DE400274E}` |
| 7 | `…\{CLSID}\Instance\InitPropertyBag` | `Attributes` | REG_DWORD | `0x11` (`FILE_ATTRIBUTE_DIRECTORY` + `FILE_ATTRIBUTE_READONLY`) |
| 8 | `…\{CLSID}\Instance\InitPropertyBag` | `TargetFolderPath` | REG_EXPAND_SZ | Zielordner |
| 9 | `…\{CLSID}\ShellFolder` | `FolderValueFlags` | REG_DWORD | `0x28` |
| 10 | `…\{CLSID}\ShellFolder` | `Attributes` | REG_DWORD | `0xF080004D` (SFGAO-Kombination laut Seite) |
| 11 | `Software\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace\{CLSID}` | (Standard) | REG_SZ | Anzeigename |
| 12 | `Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel` | `{CLSID}` | REG_DWORD | `0x1` |

- Planabgleich (C1 L6 „Einzelheiten“): Der Plan nennt Schritte 2, 5, 6, 8, 11, 12 und „die Attributwerte“; die Werte der Schritte 3, 4, 7, 9, 10 stehen nur in obiger Tabelle (Ergänzung, keine Widerlegung).
- Dateidialoge `IFileOpenDialog`/`IFileSaveDialog`: **nicht belegbar** — die Microsoft-Seite nennt nur „the Navigation Pane of the File Explorer window“. Indiz: `NSTCFC_PINNEDITEMFILTERING` („The System.IsPinnedToNameSpaceTree property exists on this folder and filtering based on that property value is supported.“, https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-nstcfoldercapabilities) gehört zum Namensraum-Baumsteuerelement; dass der Common-Item-Dialog dieses Steuerelement mit derselben Filterung nutzt, ist nicht von Microsoft belegt (Lücke).
- Fehlender `TargetFolderPath` (S2-Laufwerk nicht eingehängt): **nicht belegbar** — die Seite beschreibt nur das vorhandene Ziel („A file system folder that contains the data to display to the user“ als Voraussetzung). Folgerung: Der Plan hält den Eintrag nur bei eingehängtem Gerätebaum (C1 L6), was diese Voraussetzung erfüllt.

### DN4 — Explorer-Desktophierarchie, Darstellung im Kindfenster, Kopplung der Eingabewarteschlangen

**Antwort.**
- Hierarchie bis Windows 11 23H2 (und Windows 10 22H2): Top-Level `Progman`; nach Nachricht `0x052C` an `Progman` ein Top-Level-`WorkerW` mit `SHELLDLL_DefView` → `SysListView32` „FolderView“ und ein weiteres Top-Level-`WorkerW` dahinter (Befund Lively `src/Lively/Lively/Core/WinDesktopCore.cs`, Zweig `core-separation`, letzter Commit der Datei 2026-09-27 `7ae2fe446a`, Zeilen 177-215; GPL-3.0, nur Faktenquelle).
- Ab 24H2 („raised desktop with layered ShellView“): `Progman` mit `WS_EX_NOREDIRECTIONBITMAP` → Kinder `SHELLDLL_DefView` (→ `SysListView32`) und `WorkerW`; Erkennung über den Stil `WS_EX_NOREDIRECTIONBITMAP` von `Progman` (Lively `:173`, `:219-230`). Die von Lively zitierte Microsoft-Mitteilung (`:155-170`, nicht auf learn.microsoft.com veröffentlicht; Lücke L-A1 bleibt): „If your application forces the "raised desktop" state, it will now need to create its own WS_EX_LAYERED child HWND that is z-ordered under the DefView window but above the WorkerW window. This window should likely be a SetLayeredWindowAttributes(bAlpha=0xFF) window so that you can do DX blt presents to it“. 25H2: siehe DN1 (gemeinsame Dateibasis, Annahme).
- Praxisbeleg für **L2 (über der Symbolebene, mit Eingaben)**: Cairo Desktop (Apache-2.0, `Cairo Desktop/CairoDesktop.DynamicDesktop/Services/DesktopManager.cs`, letzter Commit 2026-09-09) setzt `WS_CHILD` und hängt sein Desktopfenster per `SetParent` unter `WindowHelper.GetLowestDesktopChildHwnd()` (Zeilen 229-235), das ist `SHELLDLL_DefView` unter `Progman` bzw. unter dem `WorkerW`, der `SHELLDLL_DefView` enthält (ManagedShell `src/ManagedShell.Common/Helpers/WindowHelper.cs:83-102`); `AllowProgmanChild => ShellWindow == null && EnvironmentHelper.IsWindows8OrBetter; // doesn't work in win7 due to layered child window restrictions` (`DesktopManager.cs:72`); Neuaufbau bei `DwmChanged` und `TaskbarCreated` (`:88-90`, `:582-598`). Folgerung: se wäre im L2-Modus Kind von `SHELLDLL_DefView` (nicht von `Progman`); das ist ein in Produktion genutzter, aber von Microsoft nicht dokumentierter Weg.
- Kopplung der Eingabewarteschlangen: Raymond Chen, 2013-04-12, „Is it legal to have a cross-process parent/child or owner/owned window relationship?“: „Creating a cross-thread parent/child or owner/owned window relationship implicitly attaches the input queues“; prozessübergreifend „some window messages are blocked between processes“, „the consequences can be very difficult to manage“ (Befund https://devblogs.microsoft.com/oldnewthing/20130412-00/?p=4683, Abruf 2026-10-17). `AttachThreadInput`-Remarks: angehängte Threads teilen „input states (such as keyboard states and the current focus window)“; „Keyboard and mouse events received by both threads are processed in the order they were received“ (MicrosoftDocs `sdk-api` `nf-winuser-attachthreadinput.md`). Folgerung: Planannahme DR18 (ein hängender se-UI-Thread kann Eingaben des Explorer-Desktop-Threads blockieren) ist durch Microsoft-Autor belegt, nicht nur angenommen.
- DPI: `SetParent`-Remarks, Tabelle „SetParent (Cross-Proc)“: bei unterschiedlichen DPI-Bewusstseinsmodi „Forced reset (of child window's process)“ auf Windows 8.1, Windows 10 ≤ 1607 und ≥ 1703 (MicrosoftDocs `sdk-api` `nf-winuser-setparent.md`, Abschnitt Remarks). Lücke: DPI-Modus von `explorer.exe` auf 24H2/25H2 nicht belegt; winit setzt standardmäßig Per-Monitor-v2 (`with_dpi_aware`, `REG/winit-0.30.13/src/platform/windows.rs:197`, Annahme zum Standardwert).
- Darstellung wgpu-DX12 bzw. glow in prozessfremdem `WS_CHILD` mit `WS_EX_LAYERED`: **nicht belegbar** — die DXGI-Seiten (`ne-dxgi-dxgi_swap_effect.md`, `dxgi-flip-model.md`) enthalten keine Aussage zu geschichteten Kindfenstern; einziger Hinweis ist die zitierte Microsoft-Mitteilung „DX blt presents“ (Bitblt-Modell, nicht Flip-Modell). Lücke: ob wgpu 22 (DX12 nutzt Flip-Modell, Annahme) in einem `WS_EX_LAYERED`-Kindfenster präsentiert.

**Folgerung für DSK-M8 (Go/No-Go-Grundlage, keine Entscheidung):** belegt sind (a) undokumentierte, mit 24H2 geänderte Hierarchie, (b) implizite Kopplung der Eingabewarteschlangen bei prozessübergreifendem Eltern-/Kindfenster (Microsoft-Autor), (c) erzwungener DPI-Reset des Kindprozesses bei abweichendem DPI-Modus (Microsoft Learn), (d) ein produktiver Präzedenzfall (Cairo, Kind von `SHELLDLL_DefView`, Windows 8 bis 11); nicht belegt ist (e) die Darstellbarkeit des wgpu-Renderers im geschichteten Kindfenster. Details in Abschnitt 3.

### DN5 — `IDesktopWallpaper` je Monitor, Diashow, Windows-Blickpunkt

**Antwort.**
- `GetWallpaper(monitorID)`: `monitorID` aus `GetMonitorDevicePathAt`; mit `NULL` und unterschiedlichen Bildern oder laufender Diashow „returns S_FALSE and an empty string“; leerer String auch, wenn kein Bild oder eine Volltonfarbe angezeigt wird (Befund MicrosoftDocs `sdk-api` `nf-shobjidl_core-idesktopwallpaper-getwallpaper.md`). Folgerung: je Monitor mit `GetMonitorDevicePathAt`/`GetMonitorRECT` abfragen, nie mit `NULL`.
- `GetPosition(position)`: ein Wert `DESKTOP_WALLPAPER_POSITION` „how the image is being displayed on the system's monitors“ — **ohne** Monitorparameter (`nf-shobjidl_core-idesktopwallpaper-getposition.md`). Folgerung: Die Planformulierung „je Monitor … `GetPosition`“ (C1 L3 Hintergrund) ist ungenau: die Position gilt systemweit.
- `GetBackgroundColor()` ohne Monitorparameter (windows-rs 0.58 Signatur `GetBackgroundColor(&self) -> Result<COLORREF>`, `REG/windows-0.58.0/src/Windows/Win32/UI/Shell/mod.rs`, `impl IDesktopWallpaper`).
- Diashow: `GetSlideshow` liefert `IShellItemArray` (Einzelbilder oder ein Container), `GetStatus` liefert `DSS_ENABLED` (0x01), `DSS_SLIDESHOW` (0x02), `DSS_DISABLED_BY_REMOTE_SESSION` (0x04) (`…-getslideshow.md`, `…-getstatus.md`). Ob bei jedem Diashow-Wechsel `WM_SETTINGCHANGE` mit `SPI_SETDESKWALLPAPER` gesendet wird: **Lücke** (keine Microsoft-Aussage gefunden).
- Windows-Blickpunkt (Spotlight) als Hintergrund: welchen Pfad `GetWallpaper` liefert: **Lücke**.
- HDR-Hintergründe: Die Lively-zitierte Microsoft-Mitteilung nennt „HDR Backgrounds“ als Anlass der 24H2-Änderung (Lively `:155-158`). Folgerung: HDR-Bilddateien (Format JPEG XR, Annahme) sind mit `image` 0.25.10 nicht dekodierbar und fallen unter DR13.

### DN6 — Sprungliste ohne eigene AppUserModelID

**Antwort.**
- Ohne explizite AUMID weist das System per Heuristik eine interne zu; „Applications cannot retrieve a system-assigned AppUserModelID“; explizite ID „strongly recommended“ (Befund MicrosoftDocs `win32` `desktop-src/shell/appids.md:22`). Wer eine explizite AUMID nutzt, muss sie auch auf Verknüpfungen und Dateizuordnungen setzen und mit `ICustomDestinationList::SetAppID` verwenden (`appids.md:24`, `:85`); `BeginList`: „If an application has an explicit … AppUserModelID, you must call ICustomDestinationList::SetAppID before you call this method“ (`nf-shobjidl_core-icustomdestinationlist-beginlist.md:89`). Folgerung: Die Liste wirkt aus dem GUI-Prozess auch ohne eigene AUMID (Planannahme bestätigt, sofern sie im GUI-Prozess geschrieben wird).
- `DeleteList(NULL)`: Parameter = AUMID des Prozesses, dessen Taskleistenknopf die Liste zeigt; „this method is intended to be called from an uninstaller, which runs in a separate process. Because it is in a separate process, the system cannot reliably deduce the AppUserModelID.“ (`nf-shobjidl_core-icustomdestinationlist-deletelist.md:61`). Folgerung: Ruft `se.exe desktop remove` oder der Deinstallationspfad (`se.exe`, anderer Prozess und anderes Abbild als `Smart Explorer.exe`) `DeleteList(NULL)` auf, trifft er nicht sicher die Liste der GUI. Ohne explizite AUMID kann ein Fremdprozess die GUI-Liste nicht gezielt löschen; vollständiges Entfernen (K5) verlangt entweder eine explizite AUMID (dann auch auf der Startmenü-Verknüpfung des Installers) oder das Löschen aus dem GUI-Prozess selbst.
- Bestand: kein `SetCurrentProcessExplicitAppUserModelID` im Code (Befund `grep` über `N/`); der NSIS-Installer setzt keine AUMID auf Verknüpfungen (Befund `grep AppUserModel native/installer.nsi`: kein Treffer).
- Platzzahl: `BeginList` liefert `pcMinSlots` = Nutzereinstellung „Number of recent items to display in Jump Lists“, Standard 10; „More destinations can be added, but they will not be shown in the UI.“ (`beginlist.md:57-66`). Planaussage „Anzahl = gemeldete Platzzahl (Systemgrenze)“ bestätigt.
- Einträge: Jedes `IShellLink` einer eigenen Kategorie braucht Ziel, Argumente, Symbolort und den Namen über `PKEY_Title` (`appendcategory.md:116-127`); `E_ACCESSDENIED` bei Datenschutzrichtlinie → trotzdem `CommitList` (`:95`).
- Verhalten nach Update (gleicher Pfad): **Lücke** — keine Microsoft-Aussage; Folgerung: Ohne explizite AUMID hängt die Zuordnung an der Systemheuristik über das Abbild (Annahme).

### DN7 — `HideDesktopIcons\NewStartPanel`

**Antwort.**
- CLSIDs (Befund windows-rs 0.58 Metadaten aus Windows-SDK, `REG/windows-0.58.0/src/Windows/Win32/UI/Shell/mod.rs`): Papierkorb `CLSID_RecycleBin` = `{645FF040-5081-101B-9F08-00AA002F954E}` (`:24926`), Dieser PC `CLSID_MyComputer` = `{20D04FE0-3AEA-1069-A2D8-08002B30309D}` (`:24916`), Netzwerk `NetworkExplorerFolder` = `{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}` (`:35376`), Systemsteuerung `CLSID_ControlPanel` = `{21EC2020-3AEA-1069-A2DD-08002B30309D}` (`:24899`). Benutzerordner `{59031A47-3F72-44A7-89C5-5595FE6B30EE}` und Desktop-Systemsteuerung `{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}`: in den windows-rs-Metadaten nicht vorhanden; nur Sekundärquellen (Lücke).
- Bedeutung der Werte: Microsoft-Archivblog „Please Bring Back my IE“ (https://learn.microsoft.com/en-us/archive/blogs/jb/please-bring-back-my-ie): Wert von 1 auf 0 stellt das Symbol wieder her; HKLM gilt systemweit, HKCU hat Vorrang. Eine aktuelle Microsoft-Referenz zu `NewStartPanel` gibt es nicht (Lücke); Standard bei fehlendem Wert (Windows 10/11 zeigen ohne Einstellung nur den Papierkorb, Annahme) ist nicht belegt.
- Folgerung: Die Plan-Regel „‚Papierkorb‘ und ‚Dieser PC‘ nur, wenn Explorer sie zeigt“ beruht für den Fall „Wert fehlt“ auf einer Annahme.

### DN23 — `SHChangeNotify(SHCNE_ASSOCCHANGED)` nach Anlegen/Entfernen der Wurzel

**Antwort.** **Nicht belegbar.** `SHCNE_ASSOCCHANGED`: „A file type association has changed. SHCNF_IDLIST must be specified … dwItem1 and dwItem2 are not used and must be NULL“; Remarks: Anwendungen, die Handler registrieren, rufen es auf, damit die Shell Symbol- und Miniaturbild-Caches verwirft (MicrosoftDocs `sdk-api` `nf-shlobj_core-shchangenotify.md:86-88`, `:292`, ms.date 12/05/2018). Eine Aussage zur Aktualisierung des Navigationsbereichs fehlt; die Cloud-Storage-Seite (DN3) nennt keinen `SHChangeNotify`-Aufruf. Für einen geänderten Ordnerinhalt nennt die Seite `SHCNE_UPDATEDIR` (`:194-196`). Folgerung: Ein zusätzliches `SHCNE_UPDATEDIR` für den Desktop-Namensraum (PIDL des Desktops) wäre die dokumentierte Ereignisart „Inhalt eines Ordners geändert“; ob Explorer darauf den Navigationsbereich neu aufbaut, ist ebenfalls nicht belegt (Lücke, Messung nötig).

### DN27 — Re-Validierung der Microsoft-Seiten (Ref A2/A3/A4)

**Antwort** (Abruf 2026-10-17, Meta-Angaben der Learn-Seiten):

| Seite | ms.date | updated_at | Kernaussage | Ergebnis |
|---|---|---|---|---|
| „Integrate a Cloud Storage Provider“ | 2018-05-31 | 2025-03-11T21:20:00Z | 12 Schritte unverändert (DN3) | bestätigt |
| „Pin your app to the taskbar“ | 2026-07-16 | 2026-07-17T02:50:00Z | „Don't use installers to call the API.“; „limited access restriction is being removed from Windows starting with KB5074105 (OS Builds 26200.7705 and 26100.7705)“; „App is in foreground“; „has a Start menu entry“ | bestätigt |
| „Implement a widget provider in a win32 app“ | 2026-08-19 | 2026-08-19T15:41:00Z | „only packaged apps can be registered as widget providers“ | bestätigt |
| „Shell Launcher“ | 2025-03-07 | 2025-03-10T23:44:00Z | „Enterprise / Enterprise LTSC“ (Editionenliste) | bestätigt |
| „Taskbar Extensions“ | 2018-05-31 | letzter Commit 2025-03-11 (`9aa3fac4f4`) | „cannot be programmatically controlled“ (Infobereich) | bestätigt |

### DN8 — X11: `_NET_SHOWING_DESKTOP`, Fenstertyp DESKTOP, BELOW je Fenstermanager

**Antwort** (Quelltexte, Abruf 2026-10-17):

| Fenstermanager | Was „Desktop anzeigen“ ausblendet | Fenster mit `_NET_WM_STATE_BELOW` (+ `SKIP_TASKBAR`), Typ NORMAL | Fenster Typ DESKTOP | Beleg |
|---|---|---|---|---|
| xfwm4 (master) | nur Fenster der Typen NORMAL, DIALOG, UTILITY, die weder minimiert noch `SKIP_TASKBAR` sind | **bleibt sichtbar** (wegen `SKIP_TASKBAR`) und liegt dann über xfdesktop | bleibt | `src/client.c` `clientToggleShowDesktop` (`(c->type & WINDOW_REGULAR_FOCUSABLE) && !FLAG_TEST (c->flags, CLIENT_FLAG_ICONIFIED \| CLIENT_FLAG_SKIP_TASKBAR)`), `src/client.h:197-199` (https://gitlab.xfce.org/xfce/xfwm4) |
| KWin (master, X11 und Wayland gemeinsam) | jedes Fenster, das nicht unverwaltet, Dock, Desktop, „belongsToDesktop“ oder Eingabemethode ist | wird ausgeblendet | bleibt | `src/workspace.cpp:1755-1779` `breaksShowingDesktop` (https://invent.kde.org/plasma/kwin) |
| Mutter (main) | jedes Fenster, das weder selbst noch über einen Vorfahren Typ DESKTOP oder DOCK ist | wird ausgeblendet | bleibt | `src/core/window.c:892-909` (`is_desktop_or_dock_foreach`) |
| Muffin (master) | wie Mutter (`window_is_desktop_component`: DESKTOP oder DOCK) | wird ausgeblendet | bleibt | `src/core/window.c:817-848` (https://github.com/linuxmint/muffin) |
| Marco (master) | wie Mutter (DESKTOP oder DOCK) | wird ausgeblendet | bleibt | `src/core/window.c:165-173`, `:187` (https://github.com/mate-desktop/marco) |
| Openbox (master) | `client_should_show`: Fenster mit `client_normal` (alles außer DESKTOP, DOCK, SPLASH) | wird ausgeblendet | bleibt | `openbox/client.c:2803-2813`, `:2876-2880` (https://github.com/danakj/openbox) |
| i3 (Zweig `next`) | unterstützt `_NET_SHOWING_DESKTOP` nicht | – | **kein Sonderfall**: i3 kündigt `_NET_WM_WINDOW_TYPE_DESKTOP` und `_NET_WM_STATE_BELOW` nicht an | `include/i3-atoms_NET_SUPPORTED.xmacro.h` (Liste ohne `_NET_WM_WINDOW_TYPE_DESKTOP`, `_NET_WM_STATE_BELOW`, `_NET_SHOWING_DESKTOP`) |
| Fluxbox (master) | nicht erhoben (`_NET_SHOWING_DESKTOP` in `src/Ewmh.cc` nicht gefunden) | nicht erhoben | wird als `TYPE_DESKTOP` erkannt (`src/Ewmh.cc:661-662`) | https://github.com/fluxbox/fluxbox |

- Folgerung: L2 (BELOW) wird bei „Desktop anzeigen“ in KWin-X11, Mutter-X11, Muffin, Marco und Openbox ausgeblendet; dann ist der fremde Desktop (xfdesktop, plasmashell, nemo-desktop, caja, DING) zu sehen. In xfwm4 bleibt das se-Fenster sichtbar.
- Folgerung: i3 behandelt ein Fenster mit Typ DESKTOP wie ein gewöhnliches Fenster (wird gekachelt); die Planaussage C2 L1 „Fenstermanager mit dieser Eigenschaft in der Standardeinrichtung: i3, Openbox, Fluxbox“ ist für i3 widerlegt.
- Stapelung zweier DESKTOP-Fenster: EWMH 1.5 legt sie nicht fest (Befund: keine Aussage im Abschnitt `_NET_WM_WINDOW_TYPE`, Ref B1); Planbegründung bestätigt.

### DN9 — Layer-shell `on_demand`, Plasma unter `bottom`, COSMIC-Desktopsymbole

**Antwort.**
- KWin (master und `Plasma/6.4`): globale Protokollversion 5 (`src/wayland/layershell_v1.cpp:22` bzw. `:21`); `set_keyboard_interactivity` setzt `acceptsFocus = keyboard_interactivity` (`:244-246`), also jeder Wert ≠ `none` macht die Fläche fokussierbar. Ebenenabbildung `background` → `DesktopLayer`, `bottom` → `BelowLayer`, `top` → `AboveLayer`, `overlay` → `OverlayLayer` (`src/layershellv1window.cpp:216-230`); Fenster vom Typ Desktop (plasmashell-Desktop) liegen in `DesktopLayer` (`src/window.cpp`, `Window::belongsToLayer`: `if (isDesktop()) return DesktopLayer;`), Reihenfolge `DesktopLayer`, `BelowLayer`, `NormalLayer`, … (`src/effect/globals.h`, `enum Layer`). Folgerung: Eine se-Fläche in `bottom` liegt über dem Plasma-Desktop und unter Programmfenstern (Planannahme bestätigt).
- **Neu und entscheidungsrelevant (KWin):** Der Fenstertyp einer Layer-Fläche ergibt sich aus ihrem `namespace`: `"desktop"` → `WindowType::Desktop`, `"dock"`, `"critical-notification"`, `"notification"`, `"tooltip"`, `"on-screen-display"`, `"dialog"`, `"splash"`, `"utility"`, sonst `Normal` (`src/layershellv1window.cpp:22-36`, `:45`); `setSkipSwitcher(!isDesktop())` (`:49`). Zusammen mit `breaksShowingDesktop` (DN8) gilt: Eine Fläche mit anderem Namespace als `desktop` wird bei „Desktop anzeigen“ in KWin **ausgeblendet**. Folgerung: Die Planaussage C2 L3 „‚Desktop anzeigen‘ betrifft nur Fenster“ gilt in KWin nur mit `namespace = "desktop"`; mit `"desktop"` erscheint die Fläche laut `setSkipSwitcher(false)` als Kandidat im Fensterwechsler (Wirkung in der Tabbox nicht erhoben, Lücke).
- Hyprland (main): Protokollversion 5 (`src/managers/ProtocolManager.cpp:199`), `on_demand` angenommen (Prüfung `kbi > ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_ON_DEMAND` als Fehler, `src/protocols/LayerShell.cpp:110-111`).
- COSMIC (cosmic-comp master): Smithay `WlrLayerShellState::new_with_filter(dh, client_not_sandboxed)` (`src/state.rs:738`, Filter `:649-653`), Smithay erzeugt das Global mit Version 5 (`src/wayland/shell/wlr_layer/mod.rs:232-233`, Smithay master). Folgerung: COSMIC bietet v5 nur nicht abgeschotteten Clients (Flatpak-Sandbox ausgeschlossen); se als native Installation ist nicht abgeschottet (Annahme: `security_context` fehlt bei direkt gestarteten Programmen).
- niri: Smithay-basiert, wayland.app meldet 5 (Abruf 2026-10-17); niri-Quelltext nicht gelesen (Lücke für `on_demand`-Verhalten).
- COSMIC-Desktopsymbole (Ref L-B1): **ja** — `cosmic-files` (GPL-3.0, letzter Push 2026-10-13) hat einen Modus `Mode::Desktop`, der je Ausgang eine Layer-Fläche `layer: Layer::Bottom`, `keyboard_interactivity: KeyboardInteractivity::OnDemand`, Anker an allen vier Kanten, `namespace: "cosmic-files-applet"` mit dem Inhalt von `desktop_dir()` anlegt (`src/app.rs:5244-5268`, https://github.com/pop-os/cosmic-files). Folgerung: Unter COSMIC liegen bei eingeschalteten Desktopsymbolen die se-Fläche und die cosmic-files-Fläche **in derselben Ebene `bottom`**; ihre gegenseitige Reihenfolge legt das Protokoll nicht fest (Annahme: Reihenfolge der Abbildung). Der Hintergrund in COSMIC muss daher wie bei KDE undurchsichtig gezeichnet werden, wenn cosmic-files-Desktop aktiv ist; wie der Modus erkannt wird (COSMIC-Konfiguration), ist Lücke.

### DN11 — GNOME-Shell-Erweiterung

**Antwort.**
- `meta_window_set_type`: in Mutter `gnome-42`, `gnome-44`, `gnome-45`, `gnome-46`, `gnome-47`, `gnome-48` nur in `src/core/window-private.h` deklariert (gnome-45 `:761`, gnome-48 `:722`), erst ab `gnome-49` öffentlich mit `META_EXPORT` in `src/meta/window.h:443-444` (Befund Mutter-Zweige auf gitlab.gnome.org, Abruf 2026-10-17). Folgerung: `Meta.Window.set_type()` ist für Erweiterungen ab GNOME 49 aufrufbar; für GNOME 45–48 **nicht** über GObject-Introspection verfügbar (Annahme: GIR enthält nur öffentliche Header; Mutter-`meson.build` nicht gelesen). Die Planaussage C2 L4 „set_type in GNOME 45–49“ ist für 45–48 widerlegt.
- DING-Belege: DING master (`metadata.json` `"shell-version": ["50", "51"]`) ruft `this._window.set_type(Meta.WindowType.DESKTOP)` (`emulateX11WindowType.js:132`), `stick()` (`:142`), `make_above()`/`unmake_above()` (`:136-138`); DING für GNOME 45 (Tag `gnome45_47.0.5`, `"shell-version": ["45"]`) nutzt stattdessen `this._window.lower()` bei jeder Neustapelung (`:51-52`, `:191-192`, `:208-209`), `this._waylandClient.hide_from_window_list(this._window)` (`:179`) und eigenes Ausfiltern im Fensterwechsler (`:355-363`). Folgerung: Für 45–48 wäre eine zweite Variante nach diesem Muster nötig; der Plan nennt „< 200 Zeilen je Variante“ (C6), ohne die Varianten zu bestimmen.
- Fenstererkennung: DING erkennt seine Fenster ausschließlich über einen selbst gestarteten `Meta.WaylandClient` (`query_window_belongs_to(window)` in `enable()`, `emulateX11WindowType.js` Ende; `extension.js` startet das Programm selbst, `launchDesktop`). Mutter-49-API: `meta_wayland_client_new_subprocess (context, launcher, argv, error)`, `meta_wayland_client_get_subprocess`, `meta_wayland_client_owns_window`, `meta_wayland_client_get_pid` (`src/meta/meta-wayland-client.h:34-47`, Zweig gnome-49). Folgerung: Das DING-Muster verlangt, dass die Erweiterung `--desktop-surface` selbst startet (nicht der se-Wächter); die Plan-Variante „App-ID + Sitzungs-Token“ ist kein belegtes Muster und erlaubt jedem Programm mit gleicher App-ID, sich als Desktop zu setzen (Folgerung).
- D-Bus `org.gnome.Shell.Extensions` (gnome-shell main, `data/dbus-interfaces/org.gnome.Shell.Extensions.xml`): Methoden `ListExtensions`, `GetExtensionInfo`, `GetExtensionErrors`, `InstallRemoteExtension`, `UninstallExtension`, `ReloadExtension`, `EnableExtension`, `DisableExtension`, `LaunchExtensionPrefs`, `OpenExtensionPrefs`, `CheckForUpdates` (Planannahme bestätigt).
- Neuanmeldung: `enableExtension(uuid)` gibt `false` zurück, wenn die UUID nicht in `this._extensions` steht (`js/ui/extensionSystem.js:277-279`, main; gleich in gnome-46 `:275-277`); `this._extensions` wird beim Shell-Start in `_loadExtensions()` gefüllt (`:705-762`). Folgerung: Eine nach dem Shell-Start in `~/.local/share/gnome-shell/extensions/` geschriebene Erweiterung lässt sich bis zum Neustart der Shell nicht aktivieren; unter Wayland heißt das Ab- und Anmelden (Ref L-D3 bestätigt). Ausnahme nur `InstallRemoteExtension` (lädt von extensions.gnome.org, nicht für eingebettete Dateien).
- GNOME-Versionen (Abruf 2026-10-17): Ubuntu 24.04 `gnome-shell` 46.0-0ubuntu6~24.04.3; Ubuntu 26.04 („resolute“) 50.1-0ubuntu1; Debian 13 („trixie“) 48.7-0+deb13u2; Fedora 43 49.10-1.fc43; Fedora 44 50.6-1.fc44 (packages.ubuntu.com, packages.debian.org, mdapi.fedoraproject.org). Folgerung: Ubuntu 24.04 (46) und Debian 13 (48) liegen vor GNOME 49 und brauchen die zweite Variante.
- Kopfloser Start im CI: Mutter kennt die Optionen `--headless` und `--virtual-monitor` (gnome-46 `src/core/meta-context-main.c:666-671`; main `:367-372`), ab main zusätzlich `--devkit` (`:378`), in gnome-46 `--nested` (`:641`). gjs.guide: „You may need to install mutter-devkit on GNOME 49 and later“ und `dbus-run-session gnome-shell --devkit --wayland` (https://gjs.guide/extensions/development/creating.html, Abruf 2026-10-17). Lücke: ob `gnome-shell --headless --wayland --virtual-monitor 1920x1080` unter `dbus-run-session` auf einem GitHub-Ubuntu-24.04-Runner startet, ist nicht belegt.
- `get_wm_class`/`get_gtk_application_id`: nicht erhoben (durch das `WaylandClient`-Muster nicht nötig).

### DN12 — Hintergrundquellen je Desktop

**Antwort** (Schemadateien, Abruf 2026-10-17):
- GNOME `org.gnome.desktop.background`: Schlüssel `picture-options`, `picture-uri`, `picture-uri-dark`, `picture-opacity`, `primary-color`, `secondary-color`, `color-shading-type`, `show-desktop-icons` (gsettings-desktop-schemas master `schemas/org.gnome.desktop.background.gschema.xml.in`). XML-Diashows werden von gnome-desktop `libgnome-desktop/gnome-bg/gnome-bg-slide-show.c` ausgewertet (Elemente `starttime`, Befund: 6 Vorkommen); ein Rust-Parser dafür existiert im Bestand nicht (Lücke: Element-Liste `static`, `transition`, `duration`, `file`, `from`, `to` nicht vollständig gelesen).
- Cinnamon `org.cinnamon.desktop.background`: `picture-options`, `picture-uri`, `background-mode`, `picture-uri-list`, `picture-opacity`, `primary-color`, `secondary-color`, `color-shading-type`, `slideshow-enabled`, `image-source`, `delay`, `random-order`, `slideshow-paused` (cinnamon-desktop master).
- MATE `org.mate.background`: `draw-background`, `show-desktop-icons`, `background-fade`, `picture-options`, `picture-filename`, `picture-opacity`, `primary-color`, `secondary-color`, `color-shading-type` (mate-desktop master).
- KDE: Bild-Hintergrund `org.kde.image`, Gruppe `General`, Einträge `Blur`, `Color`, `Image`, `PreviewImage`, `FillMode`, `SlideshowFoldersFirst`, `SlidePaths`, `SlideInterval`, `UncheckedSlides`, `SlideshowMode`, `DynamicMode` (plasma-workspace master `wallpapers/image/imagepackage/contents/config/main.xml`). Folgerung: Pfad im Plan `[Containments][N][Wallpaper][org.kde.image][General] Image` passt zu Gruppe/Eintrag (Ablage in `plasma-org.kde.plasma.desktop-appletsrc` selbst nicht gelesen, Annahme).
- Xfce: Kanal `xfce4-desktop`, Eigenschaften `/backdrop/screen<N>/monitor<NAME>/workspace<N>/{backdrop-cycle-enable, backdrop-cycle-period, backdrop-cycle-random-order, backdrop-cycle-timer, rgba1, rgba2, color-style, image-style, last-image}` sowie `/backdrop/single-workspace-mode`, `/backdrop/single-workspace-number`; Monitore „by name where possible“ (xfdesktop `doc/README.xfconf`; Namensbildung `"/backdrop/screen%d/monitor%s/workspace%d/%s"` in `common/xfdesktop-common.c:462`, Spiegel https://github.com/xfce-mirror/xfdesktop).

### DN13 — Vertrauensregeln für `.desktop`-Starter

**Antwort.**
- GNOME/DING: Start nur, wenn `trustedDesktopFile` = gültige `.desktop`-Datei UND ausführbar (`_attributeCanExecute`) UND `metadata::trusted` = `"true"` UND Desktop-Ordner und Datei nicht für Gruppe oder übrige Nutzer schreibbar (`writableByOthers`) (DING `app/fileItem.js:350`, `:981-987`).
- KDE: `KDesktopFile::isAuthorizedDesktopFile`: erlaubt unter Anwendungsverzeichnissen und `autostart/`, bei Kiosk-Sperre `run_desktop_files` verboten, sonst „only allow if the file is executable, or if owned by root (uid == 0)“ (kconfig master `src/core/kdesktopfile.cpp:100-150`).
- Xfce: Attribut `metadata::xfce-exe-checksum` (SHA-256 des Dateiinhalts), Funktionen `xfce_g_file_set_trusted`, `xfce_g_file_is_trusted` (libxfce4util master `libxfce4util/xfce-gio-extensions.c:38`, `:106`, `:156`, `:227`).
- `gio launch DESKTOP-FILE [FILE-ARG…]`: „Launch a desktop file from any location given“ (GLib main `docs/reference/gio/gio.rst:157-163`). Einführungsversion: Lücke.
- Folgerung: Drei unvereinbare Regeln; die Plan-Zwischenlösung (öffnen über `xdg-open`, nicht selbst starten) bleibt ohne Widerspruch.

### DN28 — Re-Validierung Ref B2: `zwlr_layer_shell_v1` je Compositor

**Antwort.** wayland.app, Abruf 2026-10-17: Cage 0.3.0 ✗, COSMIC 1.0.0~beta.8 5, GameScope 3.15.14 4, Hyprland 0.52.1 5, Jay 1.14.0 5, KWin 6.7 5, Labwc 0.20.2 4, Louvre 2.14.1 5, Mir 2.26 4, Muffin 6.7.4 4, Mutter 51 ✗, niri 26.04 5, phoc 0.55 3, river 0.4.8 4, Sway 1.11 4, Treeland 0.8.0 4, Wayfire 0.9.0 4, Weston 15.0.1 ✗. Quelltexte: KWin 5 (DN9), Hyprland 5 (DN9), Smithay/COSMIC 5 (DN9); Sway `SWAY_LAYER_SHELL_VERSION` = 4 in Tag 1.9 und 5 auf `master` (`sway/server.c`); wlroots `LAYER_SHELL_VERSION` 4 in 0.17.4 und 0.18.2, 5 in 0.19.0 (`types/wlr_layer_shell_v1.c`). Ubuntu 24.04 liefert Sway 1.9 (DN21) → Version 4. phoc: nur wayland.app (Quelltext bei gitlab.gnome.org per Rohdatei nicht abrufbar, Lücke). Ergebnis: Ref B2 bestätigt; Ergänzung Sway master 5.

### DN14 — Home nach Abschalten der HOME-Komponente des Rolleninhabers; `cmd role`

**Antwort.**
- AOSP (Permission-Modul, Zweige `main` und `android16-release`): `RoleService` registriert `Intent.ACTION_PACKAGE_CHANGED` und ruft `maybeGrantDefaultRolesAsync` (`service/java/com/android/role/RoleService.java:229-247`, android16-release). `RoleControllerServiceImpl.onGrantDefaultRoles` entfernt nicht mehr qualifizierte Inhaber (`:166`) und setzt, wenn die Rolle danach leer ist, **Standardinhaber nur für neu hinzugekommene oder statische Rollen** (`if (addedRoleNames.contains(roleName) || isStaticRole) … getDefaultHoldersAsUser`), sonst den Ersatzinhaber `getFallbackHolderAsUser` (`:172-184`, `main` gleichlautend).
- `HomeRoleBehavior.getFallbackHolderAsUser`: unter allen Aktivitäten für `MAIN`+`HOME` (ohne die Settings-Ersatz-Home) gewinnt die höchste Filterpriorität; bei Gleichstand `null` (`PermissionController/role-controller/java/com/android/role/controller/behavior/HomeRoleBehavior.java:81-111`, main).
- Prioritäten: „The priority for non privileged applications is capped to '0'“ (`frameworks/base` main `services/core/java/com/android/server/pm/resolution/ComponentResolver.java:484-493`); Launcher3 und Launcher3QuickStep deklarieren für `HOME` keine Priorität (`AndroidManifest.xml:59-66`, `quickstep/AndroidManifest-launcher.xml:57-64`, main) → 0.
- Folgerung: Auf AOSP mit genau einem weiteren Launcher (System-Launcher, Priorität 0) wird dieser nach dem Abschalten von se automatisch Rolleninhaber. Sind zwei oder mehr Launcher mit Priorität 0 installiert (zum Beispiel System-Launcher und ein Drittanbieter-Launcher), bleibt die Rolle leer; was beim nächsten Home-Druck erscheint (Auswahldialog des Systems), ist im Rollencode nicht belegt (Lücke L-C3 bleibt teilweise). Pixel Launcher (Priorität im Manifest) und Samsung One UI Home: **Lücke** (keine Quelltexte).
- `cmd role`: Hilfe in `service/java/com/android/role/RoleShellCommand.java:193-203` (main): `get-role-holders [--user USER_ID] ROLE`, `add-role-holder [--user USER_ID] ROLE PACKAGE [FLAGS]`, `remove-role-holder …`, `clear-role-holders …`, `set-bypassing-role-qualification true|false`, `get-active-user-for-role …`, `set-active-user-for-role …`. Die Shell-App hält `android.permission.MANAGE_ROLE_HOLDERS` (`frameworks/base` main `packages/Shell/AndroidManifest.xml:294`). Folgerung: `adb shell cmd role add-role-holder android.app.role.HOME app.smartexplorer.android` wirkt im Emulator ohne Root (Annahme: Paketname laut Bestand; Ausführung nicht geprüft).

### DN15 — Gestennavigation mit Fremd-Launchern

**Antwort.**
- AOSP/Pixel: Quickstep nutzt bei fremdem Standard-Home die Ersatz-Übersicht: „The default home app is a different launcher. Use the fallback Overview instead.“ (Launcher3 main `quickstep/src/com/android/quickstep/OverviewComponentObserver.java:200`, Abruf 2026-10-17; Ref C2 bestätigt).
- Xiaomi HyperOS: **nicht belegbar** aus Primärquelle; Sekundärquellen (Beebom, droid.tools, Projektseite „HyperGesture“) melden die Systemmeldung „Can't use full screen gestures with third party launchers“.
- Samsung One UI: **nicht belegbar** (keine Herstellerseite gefunden).

### DN16 — Deinstallieren

**Antwort.**
- `REQUEST_DELETE_PACKAGES`: „Apps targeting APIs Build.VERSION_CODES.P or greater must hold this permission in order to use Intent.ACTION_UNINSTALL_PACKAGE or PackageInstaller.uninstall(VersionedPackage, IntentSender). Protection level: normal“ (developer.android.com `Manifest.permission`, Abruf 2026-10-17).
- `ACTION_DELETE`: AOSP `UninstallerActivity` behandelt `android.intent.action.DELETE` (`frameworks/base` main `packages/PackageInstaller/AndroidManifest.xml:105-111`) und bricht für Aufrufer mit `targetSdk >= Build.VERSION_CODES.P` ohne `REQUEST_DELETE_PACKAGES` ab (`src/com/android/packageinstaller/UninstallerActivity.java:138-149`). Folgerung: Planannahme bestätigt (auch für `ACTION_DELETE`).
- `PackageInstaller.uninstall(String, IntentSender)`: API 21 (Referenz); gleiche Berechtigung laut obiger Beschreibung (für die `VersionedPackage`-Variante ausdrücklich).

### DN17 — Ruhemodus, Privater Bereich, Profile im Emulator

**Antwort.**
- `requestQuietModeEnabled(boolean, UserHandle)` API 28: „The caller must either be the foreground default launcher or have one of these permissions: MANAGE_USERS or MODIFY_QUIET_MODE.“ (developer.android.com `UserManager`, Abruf 2026-10-17); Variante mit `flags` API 30. Planannahme bestätigt.
- Pflichten laut „Behavior changes: all apps“ Android 15, Abschnitt „Launcher apps“ (https://developer.android.com/about/versions/15/behavior-changes-all, Abruf 2026-10-17): Standard-Launcher (`ROLE_HOME`) und `ACCESS_HIDDEN_PROFILES` im Manifest; eigener Container für den privaten Bereich (`getLauncherUserInfo()`); Nutzer kann den Container aus- und einblenden; Nutzer kann ihn sperren und entsperren über `requestQuietModeEnabled()` (true sperrt, false entsperrt); im gesperrten Zustand keine App des Bereichs sichtbar oder auffindbar, auch nicht über Suche; Empfänger für `ACTION_PROFILE_AVAILABLE`/`ACTION_PROFILE_UNAVAILABLE` (mit `EXTRA_USER`), Zustand über `isQuietModeEnabled()`.
- `ACCESS_HIDDEN_PROFILES`: „Protection level: normal“ (`Manifest.permission`).
- `getPrivateSpaceSettingsIntent()`: „Added in API level 36“; „Caller should have RoleManager.ROLE_HOME and either of the permissions required“ (`LauncherApps`, Abruf 2026-10-17). `getLauncherUserInfo` API 35, `getPreInstalledSystemPackages` API 35.
- Emulator: `pm create-user [--profileOf USER_ID] [--managed] [--restricted] [--guest] [--user-type USER_TYPE] [--ephemeral] [--for-testing] [--pre-create-only] USER_NAME` (`frameworks/base` main `services/core/java/com/android/server/pm/PackageManagerShellCommand.java:4968-4973`); Typen `android.os.usertype.profile.MANAGED` und `android.os.usertype.profile.PRIVATE` (`core/java/android/os/UserManager.java:176`, `:194`). Folgerung: Arbeitsprofil `pm create-user --profileOf 0 --managed <Name>`; privates Profil `pm create-user --profileOf 0 --user-type android.os.usertype.profile.PRIVATE <Name>` — ob das System-Image des Emulators den privaten Profiltyp erlaubt (Funktionsschalter), ist **Lücke**.

### DN18 — Widget-Host

**Antwort.**
- `deleteHost()`: „Remove all records about this host from the AppWidget manager … release all resources associated with your host. Any future calls about this host will cause the records to be re-allocated.“; `deleteAllHosts()` (statisch): alle Hosts des Pakets; `deleteAppWidgetId(int)`: „Stop listening to changes for this AppWidget.“ (developer.android.com `AppWidgetHost`). AOSP `deleteHostLocked` löscht jedes Widget des Hosts über `deleteAppWidgetLocked` und entfernt den Host (`frameworks/base` main `services/appwidget/java/com/android/server/appwidget/AppWidgetServiceImpl.java:2683-2696`). Planannahme (gibt alle Widget-IDs frei) bestätigt.
- „Immer erlauben“: gespeichert als Paar (Nutzer-ID, Paketname) in `mPackagesWithBindWidgetPermission` (`AppWidgetServiceImpl.java:305`, Prüfung `:1355-1363`), persistent im Zustand (`"b"`-Eintrag, `:4377-4383`); `deleteHost` berührt die Freigabe nicht (Befund `deleteHostLocked`). Folgerung: Nach Reaktivierung bindet `bindAppWidgetIdIfAllowed` ohne neuen Dialog, solange das Paket installiert ist; die Freigabe gilt für das ganze se-Paket.
- `adb shell appwidget grantbind --package <PACKAGE> …` (`frameworks/base` main `cmds/appwidget/src/com/android/commands/appwidget/AppWidget.java:37`, `:55`); die Shell-App hält `BIND_APPWIDGET` (`packages/Shell/AndroidManifest.xml:250`).
- Generierte Vorschauen: `AppWidgetManager.getWidgetPreview(ComponentName provider, UserHandle profile, int widgetCategory)` API 35; `AppWidgetProviderInfo.generatedPreviewCategories` („Flags indicating the widget categories for which generated previews are available“) (developer.android.com, Abruf 2026-10-17). Folgerung: Die Widget-Auswahl des Plans (C3 „Vorschau aus Bild bzw. Vorschau-Layout“) kennt die generierte Vorschau ab API 35 nicht (Ergänzung).

### DN19 — WorkManager und HOME-Aktivität im Nebenprozess

**Antwort.**
- `<provider android:process>`: „Normally, all components of an application run in the default process … each component can override the default with its own process attribute“ (developer.android.com `provider-element`). WorkManager initialisiert sich über den App-Startup-Provider (Bestand nutzt `androidx.work:work-runtime-ktx`, `android/gradle/libs.versions.toml:27`; Verwendung in `A/work/SyncWorker.kt`, `A/service/BackgroundController.kt`, `A/system/SyncScheduleAlarm.kt`, `A/ui/settings/BackgroundSettings.kt`, `A/service/BackgroundText.kt`). Folgerung: Ohne `android:process` am Provider läuft die automatische Initialisierung nur im Standardprozess; im Prozess `:launcher` ist WorkManager nicht initialisiert (Planannahme bestätigt, sofern der Launcher WorkManager nicht aufruft). Der Text der App-Startup-Seite zur Prozessfrage wurde nicht gefunden (Lücke).
- Plattformgrenzen für eine HOME-Aktivität in einem Nebenprozess: **nicht belegbar** — keine developer.android.com-Aussage; Quickstep entscheidet nach Paket/Komponente (`OverviewComponentObserver`), nicht nach Prozess (Folgerung aus DN15-Beleg, Prozessprüfung im Quelltext nicht gelesen).

### DN20 — Gebundener Dienst und Aktivitätsstart

**Antwort.**
- `<service android:exported="false">`: „only components of the same application or applications with the same user ID can start the service or bind to it“ (developer.android.com `service-element`). Folgerung: Bindung aus `:launcher` an einen nicht exportierten Dienst des Hauptprozesses ist zulässig.
- Hintergrund-Aktivitätsstart, Ausnahmen (https://developer.android.com/guide/components/activities/background-starts, Abruf 2026-10-17): „The app has a visible window, such as an activity in the foreground.“, „The activity is started from a PendingIntent that was sent by the system“, „The app has the SYSTEM_ALERT_WINDOW permission“, „The app has been granted the START_ACTIVITIES_FROM_BACKGROUND permission“, „The app is bound by a service that has been granted permission to start background activities“, „The app is the current Input Method Editor (IME).“, „The launch is initiated by the device's launcher“ (gelesen bis zu diesem Punkt; die auf der Seite folgenden Ausnahmen wurden nicht gelesen, Lücke). Folgerung: Die Ausnahme gilt je App; ob AOSP „App“ als UID (beide Prozesse) prüft, ist im Quelltext `BackgroundActivityStartController` nicht gelesen (Annahme: UID).

### DN25 — „In Dateien öffnen“ über `ACTION_VIEW`

**Antwort.**
- DocumentsUI (AOSP main `AndroidManifest.xml:119-135`): `.files.FilesActivity` hat `VIEW`+`DEFAULT` mit `vnd.android.document/root` und getrennt mit `vnd.android.document/directory`.
- se beansprucht `VIEW` mit `vnd.android.document/directory`, `resource/folder`, `inode/directory` (`android/app/src/main/AndroidManifest.xml:80-89`), **nicht** `vnd.android.document/root`.
- Folgerung: `ACTION_VIEW` mit `DocumentsContract.buildRootUri(…)` und Typ `vnd.android.document/root` löst auf AOSP nur DocumentsUI auf, ohne se; mit `buildDocumentUri`/Typ `…/directory` konkurrieren DocumentsUI und se (Auswahldialog). Ob Pixel (Paket der Google-DocumentsUI) und Samsung „Eigene Dateien“ den `root`-Typ annehmen: **Lücke**.

### DN29 — Re-Validierung Ref C1/C3

**Antwort.**
- `LauncherApps.getProfiles()` (API 26): „To get hidden profile UserManager.USER_TYPE_PROFILE_PRIVATE, caller should have normal Manifest.permission.ACCESS_HIDDEN_PROFILES permission and the RoleManager.ROLE_HOME role“ (Referenz, Abruf 2026-10-17) — bestätigt.
- `RoleControllerServiceImpl.onGrantDefaultRoles` auf `android16-release`: gleicher Ablauf wie `main` (DN14) — bestätigt.
- Neu in der Referenz: `Manifest.permission.ACCESS_LAUNCHER_DATA` (Version 36.1): „protects a content provider within home/launcher applications, enabling management of home screen metadata such as shortcut placement, launch intents, and labels“; `LauncherApps.setArchiveCompatibility` und `getAppMarketActivityIntent` (API 35). Folgerung: Die Pflichtliste C3 enthält weder archivierte Apps (API 35) noch `ACCESS_LAUNCHER_DATA`; ob diese für einen Launcher Pflicht sind, belegt die Referenz nicht (Lücke).

## 2 Revalidierung Abschnitt C

Jede Zeile: handlungsbestimmende Tatsachenaussage im Strangplan mit Stelle; Ergebnis; Beleg (Verweis auf Abschnitt 1 oder Quelle); richtige Angabe bei Widerlegung.

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe |
|---|---|---|---|---|
| R1 | Win32-Fensterhierarchie: ab 24H2 `SHELLDLL_DefView` unter `Progman`, vorher unter `WorkerW` (C1 L1, L2) | bestätigt | DN4 (Lively `WinDesktopCore.cs:173-230`) | – |
| R2 | L1: keine Eingaben, solange `SHELLDLL_DefView` darüber liegt (C1 L1) | bestätigt (indirekt) | Lively empfängt Eingaben über ein eigenes `RawInputMsgWindow` (`WinDesktopCore.cs:70-82`) statt über das Hintergrundfenster | – |
| R3 | L2: prozessübergreifendes Elternfenster koppelt Eingabewarteschlangen (C1 L2, DR18, „Annahme“) | bestätigt | DN4 (Raymond Chen 2013-04-12) | Aussage ist belegt, nicht nur Annahme |
| R4 | L2: z-geordnet über `SHELLDLL_DefView` als Kind der Explorer-Desktophierarchie (C1 L2) | bestätigt mit Präzisierung | DN4 (Cairo `DesktopManager.cs:229-235`, ManagedShell `WindowHelper.cs:83-102`) | Praxisweg ist Kind **von** `SHELLDLL_DefView`, nicht Geschwister darüber |
| R5 | L3: egui 0.29.1 `with_window_level` „always-on-bottom“, Variantenname Annahme (B, C1 L3) | bestätigt | DN2 (`WindowLevel::AlwaysOnBottom`, `viewport.rs:835-838`) | – |
| R6 | L3: winit behandelt `WM_WINDOWPOSCHANGING` für „always-on-bottom“ (Annahme; „falls nicht, Unterklasse“) (C1 L3 Einzelheiten) | widerlegt | DN2 (`event_loop.rs:1299-1302` nur Vollbild; `window_state.rs:339-355` einmaliges `HWND_BOTTOM`) | winit hält die unterste Lage nicht; `SetWindowSubclass` mit `HWND_BOTTOM` ist Pflicht |
| R7 | `with_taskbar(false)` allein entfernt den Alt+Tab-Eintrag nicht (Annahme, C1 L3) | bestätigt | DN2 (`set_skip_taskbar` = nur `ITaskbarList::DeleteTab`, `window.rs:1498-1535`) | zusätzlich `WS_EX_APPWINDOW` entfernen (winit setzt es, `window_state.rs:273-275`) |
| R8 | „Kein Minimieren-Rahmen, damit Win+M es nicht minimiert“ (C1 L3) | bestätigt mit Bedingung | DN1 (winit setzt `WS_MINIMIZEBOX` auch ohne Dekoration, `window_state.rs:257-268`) | nur mit `with_minimize_button(false)` (`viewport.rs:519`) |
| R9 | Je Monitor ein Viewport im Desktop-Prozess (C1 L3) | nicht belegbar als umsetzbar ohne Zusatz | DN2 (eframe 0.29.1 liefert HWND nur für den Wurzel-Viewport, `epi.rs:96`, `:621-626`) | HWND-Ermittlung je Viewport ist offen; Alternative: ein Prozess je Monitor |
| R10 | „Desktop anzeigen“/Win+D hebt den Explorer-Desktop über se (Annahme DN1, C1 L3, C1 Antwort, DR1) | bestätigt (Mechanismus) | DN1 (Chen 2024-10-21) | gilt für Fenster ohne `WS_MINIMIZEBOX`; 24H2/25H2 und Aero Peek ungemessen |
| R11 | L4: Winlogon-Shell nur als HKLM-Wert dokumentiert (C1 L4) | bestätigt | https://learn.microsoft.com/en-us/windows/win32/secauthn/msgina-dll-features (ms.date 2018-05-31): „The Shell key value can contain a comma-separated list of programs…“ unter `HKEY_LOCAL_MACHINE\Software\Microsoft\Windows NT\CurrentVersion\Winlogon` | – |
| R12 | L5: Shell Launcher nur Enterprise/Education/IoT (C1 L5) | bestätigt | DN27 | – |
| R13 | L6: Instance-CLSID `{0E5AAE11-A475-4c5b-AB00-C66DE400274E}`, `shell32.dll`, `TargetFolderPath`, nur HKCU (C1 L6) | bestätigt | DN3 | Werte der Schritte 3, 4, 7, 9, 10 ergänzen (DN3-Tabelle) |
| R14 | L6: auch in Standard-Dateidialogen (Annahme DN3) | nicht belegbar | DN3 | Lücke: keine Microsoft-Aussage |
| R15 | L6: `SHChangeNotify(SHCNE_ASSOCCHANGED)` genügt (Annahme DN23) | nicht belegbar | DN23 | Lücke; dokumentiert ist nur Wirkung auf Symbol-/Miniaturbild-Caches |
| R16 | L7: DLL bleibt bis Explorer-Neustart geladen (Annahme) | nicht belegbar | nicht neu erhoben (verworfene Lösung) | – |
| R17 | L10: Sprungliste über `ICustomDestinationList`, `DeleteList` (C1 L10, C5 Spuren) | bestätigt mit Einschränkung | DN6 | `DeleteList(NULL)` aus anderem Prozess (Deinstallation, `se.exe`) trifft ohne explizite AUMID nicht sicher die GUI-Liste |
| R18 | L10: wirksam ohne eigene AUMID (Annahme DN6) | bestätigt (für Schreiben aus dem GUI-Prozess) | DN6 (`appids.md:22`) | Microsoft empfiehlt explizite AUMID „strongly“ |
| R19 | L10: Anzahl = gemeldete Platzzahl (C1 L10) | bestätigt | DN6 (`beginlist.md:57-66`, Standard 10) | – |
| R20 | L11: Widgets-Board nur für verpackte Apps (C1 L11) | bestätigt | DN27 (ms.date 2026-08-19) | – |
| R21 | L12: Anheften nur nutzerbestätigt, nicht aus Installern; LAF-Token auf älteren Builds (C1 L12) | bestätigt | DN27 (ms.date 2026-07-16; KB5074105) | – |
| R22 | L14: Umleitung des Standard-Dateimanagers wirkt unter Windows 11 nicht (C1 L14) | bestätigt (Bestandskommentar) | `N/shell_register/os/windows.rs:154-155` „The default-manager override doesn't work on Win11“ | – |
| R23 | Infobereichssymbol „cannot be programmatically controlled“ (C1 Antwort) | bestätigt | DN27 (Taskbar Extensions) | – |
| R24 | Windows-11-Kontextmenü ohne signiertes Sparse-Paket nicht übernehmbar (`docs/WIN11_CONTEXT_MENU.md:3`) (C1 Antwort) | bestätigt (Bestand) | `docs/WIN11_CONTEXT_MENU.md:3` „blocked only on code signing“ | – |
| R25 | Hintergrund je Monitor über `IDesktopWallpaper::GetWallpaper`, `GetPosition`, `GetBackgroundColor` (C1 L3) | teilweise widerlegt | DN5 | nur `GetWallpaper`/`GetMonitorRECT` je Monitor; `GetPosition` und `GetBackgroundColor` systemweit |
| R26 | Papierkorb/Dieser PC „nur, wenn Explorer sie zeigt“ über `HideDesktopIcons\NewStartPanel` (C1 L3) | bestätigt (CLSIDs), Standardfall nicht belegbar | DN7 | Standard bei fehlendem Wert ist Lücke |
| R27 | Neuberechnung bei `WM_DPICHANGED` (C1 L3) | nicht belegbar für `WS_EX_TOOLWINDOW` unter Windows 11 | Sekundärbeleg dotnet/wpf #10422 (offen, 2025-02-07): „ToolWindows that are per-monitor DPI aware do not react to DPI changes in Windows 11“; ManagedShell setzt deshalb `WS_EX_TOOLWINDOW` auf das Besitzerfenster (`WindowHelper.cs:104-125`) | Lücke: ob ein winit-Fenster mit `WS_EX_TOOLWINDOW` `WM_DPICHANGED` erhält |
| R28 | C2 L1: i3, Openbox, Fluxbox behandeln `_NET_WM_WINDOW_TYPE_DESKTOP` als Desktop („unten, alle Arbeitsflächen, kein Taskleisteneintrag“) | widerlegt für i3; bestätigt für Openbox, Fluxbox (Typ erkannt) | DN8 | i3 kündigt weder `_NET_WM_WINDOW_TYPE_DESKTOP` noch `_NET_WM_STATE_BELOW` an |
| R29 | C2 L1: egui `with_window_type(X11WindowType::Desktop)` (B, Ref B1) | bestätigt | Ref B1 (egui-winit 0.29.1 `lib.rs:1698-1708`); bei DN2 gesehen: Abbildung `egui::X11WindowType::Dnd => WindowType::Dnd` im selben Block (`lib.rs:1715`) | – |
| R30 | C2 L2: „Desktop anzeigen“ blendet BELOW-Fenster je Fenstermanager unterschiedlich aus (Annahme DN8) | bestätigt, jetzt je WM bekannt | DN8 | xfwm4 behält se (wegen `SKIP_TASKBAR`); KWin, Mutter, Muffin, Marco, Openbox blenden aus |
| R31 | C2 L3: Compositor-Liste mit layer-shell (KWin, wlroots/Sway, Hyprland, COSMIC, niri, labwc, Wayfire, river, Jay, Louvre, Treeland, Mir, Muffin, GameScope, phoc) | bestätigt | DN28 | – |
| R32 | C2 L3: Tastatur `on_demand` ab Protokollversion 4; Version 3 nur `none`/`exclusive` (C2 Tabelle) | bestätigt | Protokoll-XML `wayland-protocols-wlr` 0.3.12 `wlr-layer-shell-unstable-v1.xml:215-246` (`on_demand` `since="4"`) | – |
| R33 | C2 L3: „‚Desktop anzeigen‘ betrifft nur Fenster“ (C2 L3) | widerlegt für KWin | DN9 (`layershellv1window.cpp:22-36`, `workspace.cpp:1755-1757`) | in KWin nur, wenn der Layer-Namespace `"desktop"` ist; sonst wird die Fläche ausgeblendet |
| R34 | C2 L3: winit #2582 offen, eframe ohne layer-shell (C2 L3) | bestätigt | GitHub-API: `open`, zuletzt aktualisiert 2025-12-22 | – |
| R35 | C2 L4: GNOME-Erweiterung setzt `Meta.Window.set_type(DESKTOP)`; API für GNOME 45–49 (C2 L4, DN11) | widerlegt für 45–48 | DN11 (Mutter: `META_EXPORT` erst ab gnome-49) | `set_type` ab GNOME 49; 45–48 nur Nachbildung (`lower()`, `stick()`, `hide_from_window_list`) wie DING-Tag `gnome45_47.0.5` |
| R36 | C2 L4: Erweiterung wirkt „nur auf Fenster mit se-App-ID und Sitzungs-Token“ (C2 L4) | nicht belegbar als sicheres Muster | DN11 (DING erkennt nur Fenster eines selbst gestarteten `Meta.WaylandClient`) | belegtes Muster: Erweiterung startet die Fläche selbst über `Meta.WaylandClient` |
| R37 | C2 Tabelle: erste Aktivierung der Erweiterung wirkt erst nach erneuter Anmeldung (Annahme) | bestätigt | DN11 (`extensionSystem.js:277-279`, `:705-762`) | – |
| R38 | C2 Tabelle: D-Bus `org.gnome.Shell.Extensions` `DisableExtension`/`UninstallExtension` (C5 Spuren) | bestätigt | DN11 | – |
| R39 | C2 L5: DING GPL-3.0-only (C2 L5, C6) | bestätigt GPL-3.0; Zusatz „only“ nicht belegbar | `COPYING` = „GNU GENERAL PUBLIC LICENSE Version 3, 29 June 2007“ (gitlab.com/rastersoft/desktop-icons-ng, Abruf 2026-10-17); letzte Aktivität 2026-10-14 | Lücke: „-only“ vs. „-or-later“ |
| R40 | C2 L6: KDE Folder View `url` (C2 L6) | nicht neu geprüft | Ref B2 | verworfene Lösung; nicht handlungsbestimmend |
| R41 | C2 L10: Sway kennt keine Below-Ebene für XWayland-Fenster (C2 L10) | nicht belegbar | keine Quelle gelesen (verworfene Lösung) | – |
| R42 | C2 Hintergrund: KDE, Xfce, Cinnamon, MATE, GNOME mit DING zeichnen Symbole unter se → undurchsichtig; wlroots, Hyprland, niri, labwc, Wayfire, river → durchsichtig; COSMIC nach DN9 | bestätigt mit Ergänzung | DN9 | COSMIC: cosmic-files-Desktop zeichnet in Ebene `bottom` (gleiche Ebene wie se) → undurchsichtig, wenn aktiv |
| R43 | C2 Hintergrundquellen (Schlüssel DN12) | bestätigt | DN12 | – |
| R44 | C2 `.desktop`-Starter nicht selbst starten bis DN13 | bestätigt (Regeln unvereinbar) | DN13 | – |
| R45 | C3 L1: Launcher3 als App nur ohne Quickstep, `privileged: true` (C3 L1, L7) | bestätigt | Launcher3 main `Android.bp:416-442` (`name: "Launcher3"`, `privileged: true`, `system_ext_specific: true`, `overrides` mit `"Home"`), `:554-569` (`Launcher3QuickStep`, `privileged: true`), `platform_apis: true` (`:317`) | – |
| R46 | C3 L2: Lawnchair Apache-2.0, `targetSdk = 37` (C3 L2) | bestätigt | `LICENSE.txt:5` „Licensed under the Apache License, Version 2.0“; `build.gradle:32-33` (Zweig `16-dev`) | – |
| R47 | C3 L3/C6: Kvaesitso, Neo Launcher, Fossify, KISS, Olauncher GPL-3.0 | bestätigt | GitHub-API `license.spdx_id` = `GPL-3.0` je Repository (Abruf 2026-10-17) | – |
| R48 | C3 Prozessschnitt: `Application.getProcessName()` API 28 | bestätigt | developer.android.com `Application` („Added in API level 28“) | – |
| R49 | C3 Prozessschnitt: WorkManager initialisiert sich nur im Hauptprozess (Annahme DN19) | bestätigt (Folgerung aus Provider-Prozessregel) | DN19 | – |
| R50 | C3 Pflicht „Standardwahl“: `ROLE_HOME` (29), `ACTION_HOME_SETTINGS` (21) | bestätigt | API-Level aus Referenz (Abruf 2026-10-17) | – |
| R51 | C3 „App-Liste“: `getProfiles` (26), `LauncherApps.Callback` | bestätigt | DN29 | – |
| R52 | C3 „App-Verknüpfungen“: `getShortcuts` (25), `hasShortcutHostPermission` | bestätigt | `LauncherApps` („Added in API level 25“; Zugriff „The current launcher …“) | – |
| R53 | C3 „Anheftanfragen“: `ACTION_CONFIRM_PIN_*` (26), `getPinItemRequest` | bestätigt | `LauncherApps` (API 26) | – |
| R54 | C3 „Widgets binden“: `bindAppWidgetIdIfAllowed` (16) | bestätigt | DN18 | – |
| R55 | C3 „Widget-Auswahl“: `getInstalledProvidersForProfile` (26), `previewLayout` (31) | widerlegt (API-Level) | `AppWidgetManager` „getInstalledProvidersForProfile Added in API level 21“; `previewLayout` API 31 | API 21; Ergänzung `getWidgetPreview` (API 35, DN18) |
| R56 | C3 „Widget-Konfiguration“: `startAppWidgetConfigureActivityForResult` (21), `WIDGET_FEATURE_RECONFIGURABLE` (28), `WIDGET_FEATURE_CONFIGURATION_OPTIONAL` (31) | bestätigt | Referenz (API-Level wie im Plan) | – |
| R57 | C3 „Widget-Größe“: `updateAppWidgetSize(Bundle, List<SizeF>)` (31), Vier-Werte-Variante auf API 30, `maxResizeWidth/Height` (31) | bestätigt | `AppWidgetHostView`: Vier-Werte-Variante API 16, „Deprecated in API level 31“; Listenvariante API 31; `maxResizeWidth` API 31 | – |
| R58 | C3 „Widget-Darstellung“: `setColorResources` (31), `getWallpaperColors` (27) | bestätigt | Referenz | – |
| R59 | C3 „Benachrichtigungspunkte“: `ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS` (30), `Ranking.canShowBadge()` | bestätigt | Referenz (API 30; `canShowBadge` API 26) | – |
| R60 | C3 „Arbeitsprofil“: `requestQuietModeEnabled` (28), Berechtigung des Standard-Launchers (Annahme DN17) | bestätigt | DN17 | Bedingung „foreground default launcher“ |
| R61 | C3 „Privater Bereich“: `getLauncherUserInfo` (35), `getPrivateSpaceSettingsIntent` (35), `ACCESS_HIDDEN_PROFILES` normal | teilweise widerlegt | DN17 | `getPrivateSpaceSettingsIntent` API 36; Pflichten zusätzlich: ein-/ausblenden, sperren/entsperren über `requestQuietModeEnabled`, gesperrt nicht über Suche auffindbar, `ACTION_PROFILE_AVAILABLE`/`UNAVAILABLE` |
| R62 | C3 „App-Info“: `startAppDetailsActivity` (21) | bestätigt | `LauncherApps` (API 21; für privates Profil `ACCESS_HIDDEN_PROFILES` + `ROLE_HOME`) | – |
| R63 | C3 „Deinstallieren“: `ACTION_DELETE` braucht `REQUEST_DELETE_PACKAGES` ab API 28 (Annahme DN16) | bestätigt | DN16 | – |
| R64 | C3 „Gesten“: Übersicht/Recents zeichnet das System (Quickstep-Rückfall) | bestätigt (AOSP) | DN15 | Hersteller: Lücke |
| R65 | C3 „Gesten“: Bedienungshilfe-Aktionen `GLOBAL_ACTION_NOTIFICATIONS`, `GLOBAL_ACTION_LOCK_SCREEN` | nicht neu geprüft | – | Lücke: API-Level `GLOBAL_ACTION_LOCK_SCREEN` (Annahme 28) nicht abgerufen |
| R66 | C3 „Nicht übernehmbar“: fremde Widgets an fremden Host gebunden | bestätigt (Folgerung) | DN18 (Widgets gehören zu Host-Einträgen in `AppWidgetServiceImpl`) | – |
| R67 | C5 Android: ausgeschaltete Komponente → Rolle fällt weg (Ref C3) | bestätigt | DN14 | Ersatzinhaber nur bei eindeutiger höchster Priorität |
| R68 | C5 „vorheriger Launcher“: keine API zum Setzen fremder Rolleninhaber | bestätigt (für Apps) | DN14 (`cmd role` nur über Shell mit `MANAGE_ROLE_HOLDERS`) | – |
| R69 | C5 Widgets: `deleteHost()` gibt alle Widget-IDs frei | bestätigt | DN18 | – |
| R70 | B/C5: Der Updater-Helfer beendet alle Prozesse mit dem Abbild der App (B „Update und Release“, C5 Wiederanlauf) | widerlegt | DN24 | Helfer schreibt `daemon.stop`, wartet 30 s und bricht ab; Desktop-Prozesse müssen `update_stop_marker()` beachten |
| R71 | C5: `.onInstSuccess` startet bei stillem Upgrade nichts (DN24) | bestätigt | DN24 (`installer.nsi:283-288`) | – |
| R72 | C5 Linux-Autostart `TryExec` ignoriert Eintrag bei fehlendem Programm | nicht neu geprüft | – | Lücke: Desktop-Entry-Spezifikation nicht abgerufen |
| R73 | C6 Lizenzen: egui/eframe, egui-wgpu, wgpu, winit „MIT/Apache“ | teilweise widerlegt | Register-`Cargo.toml`: egui `MIT OR Apache-2.0`, wgpu `MIT OR Apache-2.0`, **winit `Apache-2.0`** | winit nur Apache-2.0 (MIT-Projekt-verträglich mit NOTICE-Pflichten) |
| R74 | C6 Lizenzen: SCTK, wayland-client, wayland-protocols-wlr, calloop, xkbcommon-dl MIT; x11rb MIT/Apache; zbus MIT | bestätigt | Register-`Cargo.toml` (`license`) | – |
| R75 | C6: `image` + Features, Unterpakete (DN22) | bestätigt | DN22 | vier neue Lock-Pakete (`zune-jpeg`, `zune-core`, `image-webp`, `quick-error`) |
| R76 | C6: Cairo Apache-2.0, Lively GPL-3.0, PowerToys MIT | bestätigt | GitHub-API `license.spdx_id` (Abruf 2026-10-17) | – |
| R77 | C8: `windows`-Features `Win32_UI_Shell`, `Win32_Graphics_Gdi`, `Win32_UI_WindowsAndMessaging` genügen (Namen DN26) | widerlegt (unvollständig) | DN26 | zusätzlich `Win32_UI_Shell_PropertiesSystem` und `Win32_Storage_EnhancedStorage` (Sprunglisten-Titel); `Win32_UI_HiDpi` nur bei `GetDpiForMonitor` |
| R78 | C8 CI: Layer-Backend lädt `libwayland-client` und `libxkbcommon` zur Laufzeit; keine neuen Systembibliotheken | bestätigt | DN10 | – |
| R79 | C4: Offline-Gerät öffnet ohne Hängen (DN30) | bestätigt | DN30 | Startschalter `--open-location` ist neu (DSK-M2), nicht Bestand |
| R80 | C4: „In Dateien öffnen“ nur bei verlässlichem Weg (DN25) | bestätigt für AOSP über Root-URI | DN25 | Pixel/Samsung: Lücke |

## 3 Folgen für Entscheidungen

Nur Entscheidungen, deren Tatsachengrundlage sich durch Abschnitt 1/2 geändert hat. Keine Neubewertung; die Entscheidung trifft der Planverfasser.

### 3.1 DSK-M8 (Einbetten in den Explorer-Desktop) — Go/No-Go-Grundlage nach DN4

- Für **No-Go** sprechende Befunde: (1) Kopplung der Eingabewarteschlangen bei prozessübergreifendem Eltern-/Kindfenster ist durch Microsoft-Autor belegt (DN4, R3); der Wächter mit 10-s-Lebenszeichen begrenzt einen Hänger auf bis zu 10 s Eingabeblockade des Explorer-Desktops, verhindert ihn aber nicht. (2) Erzwungener DPI-Reset des Kindprozesses bei abweichendem DPI-Modus (Microsoft Learn `SetParent`, DN4). (3) Undokumentierte, mit 24H2 geänderte Hierarchie; Microsoft-Mitteilung nur über Lively-Zitat. (4) Darstellbarkeit von wgpu (DX12/Vulkan) bzw. glow in einem `WS_EX_LAYERED`-Kindfenster nicht belegt (DN4).
- Für **Go** sprechende Befunde: (5) Cairo Desktop nutzt den Weg produktiv (Kind von `SHELLDLL_DefView`, Windows 8 bis 11, Neuaufbau bei `DwmChanged`/`TaskbarCreated`; DN4, R4).
- Offene Entscheidungsgrundlage: (4) und das DPI-Verhalten von `explorer.exe` lassen sich nur durch Messung schließen (Windows-Runner, DN21: interaktive Sitzung belegt, Explorer-Shell nicht belegt).

### 3.2 C1 L3 (unterstes Werkzeugfenster) — Umsetzungsgrundlage geändert

- R6: Unterklasse mit `HWND_BOTTOM` in `WM_WINDOWPOSCHANGING` ist Pflichtbestandteil, nicht Rückfall.
- R8: `with_minimize_button(false)` ist nötig, damit Win+D/Win+M das Fenster nicht minimieren und der Chen-Mechanismus (R10) wie angenommen greift.
- R7: `WS_EX_TOOLWINDOW` setzen **und** `WS_EX_APPWINDOW` entfernen.
- R9: „je Monitor ein Viewport“ hat keinen eframe-Weg zum HWND der Nicht-Wurzel-Viewports.
- R25: `GetPosition`/`GetBackgroundColor` sind systemweit.
- R27: `WM_DPICHANGED` kommt bei `WS_EX_TOOLWINDOW` unter Windows 11 möglicherweise nicht an (Sekundärbeleg); die Neuberechnung bei `WM_DISPLAYCHANGE`/`WM_SETTINGCHANGE` trägt dann allein.

### 3.3 C1 L10 und C5 „vollständig entfernen“ (Sprungliste)

- R17/DN6: `DeleteList(NULL)` aus `se.exe` oder dem Deinstallationspfad trifft ohne explizite AUMID nicht sicher die Liste der GUI. Entscheidungsgrundlage für „hinterlässt nichts“ (K5) geändert: entweder explizite AUMID für GUI-Prozess und Startmenü-Verknüpfung (NSIS setzt heute keine) oder Löschen aus dem GUI-Prozess.
- R77: zwei fehlende `windows`-Features.

### 3.4 C5 Wiederanlauf nach Update und C8 Installer

- R70/DN24: Der Helfer beendet nichts; ein laufender Wächter/eine Fläche ohne Beachtung von `daemon::update_stop_marker()` lässt jedes Desktop-Update nach 30 s scheitern. Der Wiederanlauf über `resume_on_gui_start` setzt voraus, dass beide Prozesse sich auf das Signal hin selbst beenden.

### 3.5 C2 Linux-Mechanismen

- R28: i3 unterstützt weder Typ DESKTOP noch BELOW; die Sitzungstabelle C2 („X11 ohne fremdes Desktopfenster → L1“) liefert unter i3 ein gekacheltes Normalfenster.
- R30: Verhalten bei „Desktop anzeigen“ je X11-Fenstermanager ist jetzt bekannt (DN8) und für die Doku (DR1-Gegenstück Linux) verwendbar.
- R33: KWin blendet Layer-Flächen ohne Namespace `"desktop"` bei „Desktop anzeigen“ aus.
- R42/DN9: COSMIC kann mit cosmic-files-Desktop eine zweite Fläche in derselben Ebene `bottom` haben; Durchsichtigkeitsregel und Stapelung dort offen.
- R35/R36/DN11: GNOME-Weg geändert: `set_type` erst ab GNOME 49 (Ubuntu 24.04 = 46, Debian 13 = 48 brauchen eine Nachbildungsvariante); belegtes Erkennungsmuster ist „Erweiterung startet die Fläche über `Meta.WaylandClient`“, was den Prozessstart unter GNOME vom Wächter zur Erweiterung verschiebt (Folgerung); Neuanmeldung nach erster Installation bestätigt.

### 3.6 C3 Android-Launcher

- R61/DN17: Pflichten des privaten Bereichs sind umfangreicher als in der Pflichtliste (Sperren/Entsperren, Ein-/Ausblenden, Suchausschluss, Profil-Broadcasts); `getPrivateSpaceSettingsIntent` erst ab API 36.
- R55/DN18: Widget-Auswahl: API-Level 21; generierte Vorschauen ab API 35 nicht vorgesehen.
- DN14/R67: Nach dem Abschalten wird der System-Launcher nur dann automatisch Inhaber, wenn er als einziger die höchste Priorität hat; sonst bleibt die Rolle leer. DR9-Gegenmaßnahme (Führung über `ACTION_HOME_SETTINGS`) bleibt nötig; der Testweg `cmd role add-role-holder` ist im Emulator ohne Root belegt.
- DN25/R80: „In Dateien öffnen“ hat auf AOSP einen Weg ohne se-Selbstauflösung (Root-URI, Typ `vnd.android.document/root`); die Bedingung im Plan (C4 „nur wenn DN25 einen verlässlichen Weg belegt“) ist für AOSP erfüllt, für Pixel/Samsung offen.

### 3.7 C8 Abhängigkeiten

- DN22/R75: vier neue Lock-Pakete durch `image`-Features (Lizenzen verträglich).
- DN10/R78: Layer-Backend ohne neue Lock-Pakete möglich, Tastatur vollständig selbst über `wl_keyboard` + `xkbcommon-dl` (SCTK-Tastaturmodul entfällt ohne Feature `xkbcommon`).
