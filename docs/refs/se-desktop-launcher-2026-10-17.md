# se Desktop (Windows, Linux) und se Launcher (Android) — Faktenlage

Stand und Abrufdatum 2026-10-17. Alle Quellen wurden am 2026-10-17 abgerufen. Recherche für Strang S5 (`docs/plaene/2026-10-17-rechte-desktop-tresore/strang-desktop.md`, Ergebnisse E4, E6, E7). Nichts hiervon ist gebaut oder getestet. „Folgerung:“ markiert eigene Schlüsse, „Annahme:“ ungeprüfte Voraussetzungen; jede nicht so markierte Aussage ist mit der genannten Quelle belegt. „ms.date“ ist das Dokumentdatum der Microsoft-Learn-Seite, „updated_at“ der letzte Seitenstand.

Bezug zum Bestand (nur aus den erlaubten Dateien gelesen): `android/app/build.gradle.kts` setzt `compileSdk = 36`, `minSdk = 30`, `targetSdk = 36`, Compose über `platform(libs.compose.bom)` (BOM-Version steht im Versionskatalog, nicht in dieser Datei; nicht gelesen). `docs/WIN11_CONTEXT_MENU.md`: Das moderne Windows-11-Kontextmenü verlangt einen `IExplorerCommand`-COM-Handler mit Paketidentität (Sparse-Paket); dieser ist im Repo als `native/explorer-command/` gebaut und blockiert nur an der Codesignatur. `docs/refs/windows-virtual-files.md` behandelt das virtuelle `IDataObject` für Zwischenablage/Ziehen, nicht den Navigationsbereich.

---

## A. Windows-Tiefe

### A1. Aufbau des Windows-Desktops (Fensterhierarchie)

- Der Desktop mit Symbolen ist kein eigener Prozess, sondern Teil von `explorer.exe`. Fensterhierarchie laut Spy++-Ausgabe im Quelltext von Lively Wallpaper (`src/Lively/Lively/Core/WinDesktopCore.cs`, Zweig `core-separation`, Repository `lively-community/lively`, GPL-3.0, letzter Push 2026-09-30):
  - Bis Windows 11 23H2 („klassisch“): Top-Level `Progman` („Program Manager“); nach der undokumentierten Nachricht `0x052C` an `Progman` erzeugt Explorer ein Top-Level-`WorkerW`, das `SHELLDLL_DefView` (mit Kind `SysListView32` „FolderView“ = Symbolansicht) enthält, und ein weiteres Top-Level-`WorkerW` dahinter, in das Hintergrundprogramme ihr Fenster per `SetParent` hängen.
  - Ab Windows 11 24H2 („raised desktop with layered ShellView“): `Progman` (mit `WS_EX_NOREDIRECTIONBITMAP`) → Kinder `SHELLDLL_DefView` (Kind `SysListView32` „FolderView“) und `WorkerW`. Lively zitiert im Code eine Microsoft-Mitteilung (nicht öffentlich auf learn.microsoft.com gefunden): „the top-level "Progman" window is now created with WS_EX_NOREDIRECTIONBITMAP … and the shell DefView child window is a WS_EX_LAYERED child window. When the desktop is raised, we create a child WorkerW window that is z-ordered under the DefView that will render the wallpaper … If your application forces the "raised desktop" state, it will now need to create its own WS_EX_LAYERED child HWND that is z-ordered under the DefView window but above the WorkerW window.“ Lively erkennt den neuen Aufbau am Stil `WS_EX_NOREDIRECTIONBITMAP` von `Progman` und hängt sein Fenster dann als `WS_CHILD` mit `WS_EX_LAYERED` (Alpha 255) unter `Progman`, z-geordnet direkt unter `SHELLDLL_DefView`.
  - Quelle: https://raw.githubusercontent.com/lively-community/lively/core-separation/src/Lively/Lively/Core/WinDesktopCore.cs (Zeilen 155–240, 1036–1070), Abruf 2026-10-17.
- Microsoft dokumentiert weder `WorkerW`, `0x052C` noch die Hierarchie öffentlich (Lücke L-A1: keine Microsoft-Learn-Seite gefunden). Microsoft hatte für 24H2 eine Upgrade-Sperre („safeguard hold“) für Hintergrund-Anpassungsprogramme; laut Sekundärquelle The Register (2025-04-14, https://www.theregister.com/2025/04/14/windows_11_24h2_hold/) wurde sie ab April 2025 schrittweise aufgehoben, sobald Entwickler Updates lieferten. Die Release-Health-Seite selbst wurde nicht gelesen (Lücke L-A2).
- Cairo Desktop (Repository `cairoshell/cairoshell`, Apache-2.0, C#, letzter Push 2026-10-15; README: „Windows 7 SP1, Windows 8.1, Windows 10, or Windows 11“) hängt sein Desktopfenster ebenfalls als Kind von `Progman`, wenn es nicht selbst Shell ist (`AllowProgmanChild => ShellWindow == null && EnvironmentHelper.IsWindows8OrBetter`, Datei `Cairo Desktop/CairoDesktop.DynamicDesktop/Services/DesktopManager.cs`), und baut das Fenster nach einem DWM-Neustart neu auf (`DwmChanged` → `DestroyDesktopWindow(); CreateDesktopWindow();`) sowie nach `TaskbarCreated` (Explorer-Neustart).

Folgerung: Ein Fenster „im Desktop“ unter laufendem Explorer ist nur über undokumentierte Fensterbeziehungen möglich; es muss auf Explorer-Neustart (`TaskbarCreated`), DWM-Neustart und Wechsel zwischen alter und 24H2-Hierarchie reagieren. Ein Windows-Funktionsupdate kann das jederzeit brechen (belegt durch die 24H2-Änderung).

### A2. Wege, eigene Inhalte auf oder statt des Desktops zu zeigen

| Weg | Was Explorer behält | Rechte/Edition | Rückbau | Beleg |
|---|---|---|---|---|
| W1 Ebene hinter den Symbolen (Lively-Weg: Kind von `WorkerW` bzw. ab 24H2 Kind von `Progman` unter `SHELLDLL_DefView`) | Taskleiste, Start, Desktopsymbole, Kontextmenü des Desktops; eigenes Fenster bekommt keine Maus-/Tastatureingaben, solange `SHELLDLL_DefView` darüber liegt (Folgerung aus der Z-Ordnung) | Standardnutzer; alle Editionen | Fenster schließen; Explorer zeichnet Hintergrund neu | Lively-Quelltext oben |
| W2 Ersatz der Symbolebene (eigenes Fenster über `SHELLDLL_DefView` bzw. Explorer-Desktopsymbole ausblenden) | Taskleiste, Start | Standardnutzer | Fenster schließen, Symbole wieder einblenden | Cairo `DesktopManager.cs` (Kind von `Progman`); kein Microsoft-Beleg für ein Ausblenden-API (Lücke L-A3) |
| W3 Vollbild-Fenster ganz unten (`HWND_BOTTOM`, ohne Taskleisteneintrag) | alles; Explorer-Desktop liegt darunter verdeckt | Standardnutzer | Fenster schließen | Folgerung aus Win32-Fensterverwaltung; „Win+D“/„Desktop anzeigen“ minimiert solche Fenster (Annahme, nicht belegt, Lücke L-A4) |
| W4 Shell-Ersatz über `Winlogon\Shell` | nichts: Explorer startet nicht als Shell; Taskleiste, Start, Benachrichtigungsbereich, Desktop muss der Ersatz selbst liefern | HKLM verlangt Admin; HKCU-Wert pro Nutzer wird von Cairo/ManagedShell geschrieben (`Registry.CurrentUser…\WinLogon`, Wert `Shell`) | Wert `Shell` unter HKCU löschen, ab- und anmelden | Microsoft: „The **Shell** key value can contain a comma-separated list of programs to be executed. Windows Explorer is the default shell program and will be executed if the **Shell** key value is null or not present.“ (nur HKLM-Pfad dokumentiert) — https://learn.microsoft.com/en-us/windows/win32/secauthn/msgina-dll-features, ms.date 2018-05-31; HKCU-Praxis: https://raw.githubusercontent.com/cairoshell/ManagedShell/master/src/ManagedShell.Common/Helpers/EnvironmentHelper.cs (Z. 185–245, ManagedShell Apache-2.0) |
| W5 Shell Launcher (v1 `Eshell.exe`, v2 `CustomShellHost.exe`) | nichts (Kiosk/Spezialgerät) | nur „Enterprise / Enterprise LTSC, Education, IoT Enterprise / IoT Enterprise LTSC“ (nicht Home, nicht Pro); Konfiguration über CSP, GPO, AppLocker | Feature-Konfiguration entfernen | https://learn.microsoft.com/en-us/windows/configuration/shell-launcher/, ms.date 2025-03-07 |

Weitere Belege zu W5 (gleiche Quelle): v2 ab Windows 10 1809 erlaubt Win32- oder UWP-Shell; „Shell Launcher doesn't support a custom shell with an application that launches a different process and exits“; die Shell läuft mit den Rechten des angemeldeten Kontos.

Folgerung: W4 ist der einzige Vollersatz ohne Enterprise-Edition, setzt aber voraus, dass se Taskleiste, Start, Infobereich und Fensterumschaltung selbst ersetzt (Cairo tut das) — das widerspricht „kollisionsfrei“ und „übernimmt, was übernommen werden kann“. W1–W3 lassen Explorer laufen.

### A3. „Gerät als Ordner“ in Explorer

| Integration | Ort in Explorer | Registrierung ohne Admin | Implementierungsaufwand | Entfernen | Beleg |
|---|---|---|---|---|---|
| I1 Navigationsbereich-Wurzel als Shell-Instanzobjekt (`shell32.dll` + Instance-CLSID `{0E5AAE11-A475-4c5b-AB00-C66DE400274E}`, `TargetFolderPath`) | Wurzelebene des Navigationsbereichs (neben OneDrive, Dieser PC, Netzwerk) | ja, alle 12 Schritte unter `HKCU\Software\Classes\CLSID\{…}` und `HKCU\…\Explorer\Desktop\NameSpace\{…}`; Schritt 12 `HKCU\…\Explorer\HideDesktopIcons\NewStartPanel` blendet es auf dem Desktop aus | gering: kein eigener COM-Code; zeigt einen **existierenden Dateisystemordner** (Ziel z. B. ein Laufwerk oder Mount-Pfad aus S2) | die angelegten HKCU-Schlüssel löschen (Folgerung: keine weiteren Spuren, da kein Code registriert) | https://learn.microsoft.com/en-us/windows/win32/shell/integrate-cloud-storage, ms.date 2018-05-31, updated_at 2025-03-11 |
| I2 Virtuelle Abzweigung (junction point) einer eigenen Namensraumerweiterung | unter `Desktop`, `MyComputer` („Dieser PC“), `ControlPanel`, `NetworkNeighborhood`, `NetworkNeighborhood\EntireNetwork`, `RemoteComputer`, `UsersFiles`; alternativ Ordner `Name.{CLSID}` oder `desktop.ini` mit `CLSID=` | ja: „HKEY_LOCAL_MACHINE or HKEY_CURRENT_USER … Explorer\<Virtual Folder Name>\NameSpace\{Extension CLSID}“ | hoch: eigener In-Process-COM-Server mit mindestens `IShellFolder` und `IPersistFolder` (Folgerung aus der Erwähnung von `IPersistFolder::Initialize` auf der Seite; vollständige Schnittstellenliste nicht gelesen, Lücke L-A5) | Schlüssel und COM-Registrierung unter HKCU löschen; DLL kann bis zum Explorer-Neustart geladen bleiben (Annahme) | https://learn.microsoft.com/en-us/windows/win32/shell/nse-junction, ms.date 2018-05-31 |
| I3 Cloud-Files-Sync-Root (cfapi, `cldflt.sys`) | eigener Knoten im Navigationsbereich mit Symbol und Namen, Platzhalterdateien, Zustandssymbole | Microsoft: „Sync engines using the cloud files APIs are designed to use the Desktop Bridge as an implementation requirement“; die ältere Seite beschreibt Registrierung unter `HKLM\…\SyncRootManager\…` | hoch (Hydrierung, Platzhalter, nur NTFS: „Cldflt.sys currently only supports NTFS volumes“) | `Unregister`/Deinstallation des Pakets; bei Absturz bleibt die Registrierung bestehen („If the sample crashes, it’s possible that the sync root will remain registered“) | https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine, ms.date 2025-05-12, updated_at 2025-11-18; Vertiefung siehe `docs/refs/windows-dateisystem-frontends-2026-10-17.md` (nicht neu erhoben) |
| I4 Laufwerk (Dokany im Bestand) | „Dieser PC“ als Laufwerksbuchstabe | Treiber verlangt einmalig Admin (Bestand S2) | vorhanden | Laufwerk aushängen | Strang S2, nicht neu erhoben |

Weitere Fakten:
- Explorer öffnet eine Namensraumerweiterung direkt mit `%SystemRoot%\Explorer.exe /e,::{CLSID}` bzw. `…::{CLSID},objectname` (nse-junction, oben).
- Folgerung: Weil `Desktop\NameSpace` derselbe Schlüssel ist, über den I1 in den Navigationsbereich kommt, erscheint ein Eintrag ohne Schritt 12 zusätzlich als Symbol auf dem Explorer-Desktop. Microsoft rät davon ab: „you should not use this method to create a Desktop shortcut“ (integrate-cloud-storage). Eine gewöhnliche Verknüpfung (`.lnk`) im Desktop-Ordner des Nutzers ist der dokumentationskonforme Weg zu einem Desktopsymbol (Folgerung).
- `StorageProviderSyncRootManager.Register` sagt nur: „Multiple sync roots cannot be registered with the same path“; eine Aussage zur Paketidentität fehlt auf der Referenzseite (https://learn.microsoft.com/en-us/uwp/api/windows.storage.provider.storageprovidersyncrootmanager.register, ms.date 2017-02-08, updated_at 2025-11-21; Lücke L-A6: ob ein unverpacktes Programm ohne Sparse-Paket registrieren darf, ist dort nicht belegt).

### A4. Windows-11-Widgets, Anheften, Sprunglisten

| Funktion | Unverpacktes Win32-Programm (heutiger se-Installer) möglich? | Pflichten | Beleg |
|---|---|---|---|
| Widgets-Board-Anbieter | nein: „In the current release, only packaged apps can be registered as widget providers.“ | MSIX-Paket (Windows Application Packaging Project), Microsoft.WindowsAppSDK ≥ 2.3.1, `com:ComServer` und `uap3:AppExtension Name="com.microsoft.windows.widgets"` im Manifest; Inhalt als Adaptive Cards (JSON) | https://learn.microsoft.com/en-us/windows/apps/develop/widgets/implement-widget-provider-win32, ms.date 2026-08-19; https://learn.microsoft.com/en-us/windows/apps/develop/widgets/widget-providers, ms.date 2026-07-05 |
| An Taskleiste anheften | nur mit Nutzerbestätigung über `TaskbarManager.RequestPinCurrentAppAsync`; Win32 nur, wenn `ITaskbarManagerDesktopAppSupportStatics` vorhanden; Bedingungen „App is in foreground“, „App has a Start menu entry“; „Don't use installers to call the API.“ | auf manchen Versionen Limited-Access-Feature-Token; „The limited access restriction is being removed from Windows starting with KB5074105 (OS Builds 26200.7705 and 26100.7705)“ | https://learn.microsoft.com/en-us/windows/apps/develop/windows-integration/pin-to-taskbar, ms.date 2026-07-16 |
| Sprungliste (Jump List: Aufgaben, eigene Kategorien, zuletzt verwendet) | ja (Folgerung: API `ICustomDestinationList`/`SHAddToRecentDocs` hängt an der AppUserModelID, die Seite nennt keine Paketpflicht) | AppUserModelID der Fenster/Verknüpfung | https://learn.microsoft.com/en-us/windows/win32/shell/taskbar-extensions, ms.date 2018-05-31 |
| Infobereichssymbol sichtbar halten | nein: „All notification icons are now hidden by default and that visibility cannot be programmatically controlled.“ | — | gleiche Seite |

Folgerung: Ohne MSIX-/Sparse-Paket kann se an Windows-11-Oberflächen nur Sprunglisten, `.lnk`-Verknüpfungen, den Navigationsbereich (I1/I2) und ein Laufwerk (I4) übernehmen. Das bereits vorbereitete Sparse-Paket (`native/explorer-command/`, Codesignatur offen) wäre auch Träger für Widgets nur, wenn es die Widgets-Erweiterung deklariert; ob ein Sparse-Paket (Paket mit externem Ort) als Widget-Anbieter zulässig ist, belegt die Seite nicht (Lücke L-A7).

### A5. Vergleichstabelle Abschnitt A (Desktop-Wege)

| Kriterium | W1 hinter Symbolen | W2 statt Symbolebene | W3 unterstes Vollbild | W4 Winlogon-Shell HKCU | W5 Shell Launcher | I1 Navigationsbereich |
|---|---|---|---|---|---|---|
| Explorer-Taskleiste/Start bleiben | ja | ja | ja | nein | nein | ja |
| Admin nötig | nein | nein | nein | nein (HKCU-Praxis, nicht Microsoft-dokumentiert) | ja (Edition + Verwaltung) | nein |
| Windows Home/Pro | ja | ja | ja | ja | nein | ja |
| Dokumentiert durch Microsoft | nein | nein | teilweise (Fenster-API) | nur HKLM-Wert | ja | ja |
| Eingaben auf eigenem Inhalt | nein (DefView liegt darüber, Folgerung) | ja | ja | ja | ja | entfällt (Explorer zeigt Ordner) |
| Bruchrisiko durch Funktionsupdate | belegt (24H2) | belegt (gleiche Hierarchie) | gering (Folgerung) | gering (Folgerung) | gering | gering |
| Vollständiger Rückbau | Fenster schließen | Fenster schließen, Symbole einblenden | Fenster schließen | Wert löschen + Neuanmeldung | Konfiguration entfernen | HKCU-Schlüssel löschen |
| Quelloffenes Vorbild | Lively (GPL-3.0) | Cairo (Apache-2.0) | — | Cairo/ManagedShell (Apache-2.0) | — | Microsoft-Beispiel |

---

## B. Linux-Desktop

### B1. X11: Fenstertyp Desktop (EWMH)

- EWMH 1.5 („Publication Date: 29 November 2011, Version: 1.5“): „_NET_WM_WINDOW_TYPE_DESKTOP indicates a desktop feature. This can include a single window containing desktop icons with the same dimensions as the screen, allowing the desktop environment to have full control of the desktop, without the need for proxying root window clicks.“ — https://specifications.freedesktop.org/wm/latest/ar01s05.html (alte Adresse `…/wm-spec/latest/` leitet dorthin um), Abruf 2026-10-17.
- egui 0.29.1 (im Repo verwendete eframe-Linie 0.29) bietet `ViewportBuilder::with_window_type(X11WindowType)` mit Variante `X11WindowType::Desktop`; egui-winit 0.29.1 setzt das in `window_attributes.with_x11_window_type(vec![WindowType::Desktop])` um. Quelle: Quelltext `egui-0.29.1/src/viewport.rs` Z. 311, 607, 844–852 und `egui-winit-0.29.1/src/lib.rs` Z. 1698–1708 aus https://static.crates.io/crates/egui/egui-0.29.1.crate bzw. `…/egui-winit/egui-winit-0.29.1.crate`, Abruf 2026-10-17. Die Option wirkt nur unter X11 (Name der API; unter Wayland gibt es keinen solchen Hinweis).

### B2. Wayland: wlr-layer-shell je Compositor

Quelle: https://wayland.app/protocols/wlr-layer-shell-unstable-v1, Abruf 2026-10-17 (Protokoll `zwlr_layer_shell_v1`, Version 5; Ebenen „background, bottom, top, overlay“; Tastaturfokus `none` (Standard), `exclusive`, `on_demand` ab Version 4).

| Compositor/Desktop | layer-shell laut wayland.app (Version) | Desktopsymbole des Systems | Ablösbar durch eigenes Desktopfenster |
|---|---|---|---|
| KDE Plasma (KWin) | ja (KWin 6.7, Protokollversion 5) | Plasma-Containment `org.kde.desktopcontainment` (Folder View), GPL-2.0+, Konfigurationseintrag `url` „URL of the file system location being shown“, Standard `desktop:/` — https://invent.kde.org/plasma/plasma-desktop/-/raw/master/containments/desktop/package/contents/config/main.xml | Folgerung: ja, als layer-shell-Fläche in Ebene `background`/`bottom`; Plasma zeichnet seine eigene Desktopfläche weiter, solange das Containment aktiv ist |
| GNOME (Mutter) | nein | keine im Standard; Erweiterung „Desktop Icons NG“ (DING, GPL-3.0-only, `shell-version` 50 und 51, letzte Aktivität 2026-10-14) | nur über eine GNOME-Shell-Erweiterung (siehe B3) |
| wlroots/Sway | ja (Version 4) | keine | ja (layer-shell) |
| Hyprland | ja (Version 5) | keine | ja (layer-shell) |
| COSMIC | ja (Version 5) | Lücke L-B1: nicht erhoben, ob COSMIC Desktopsymbole zeichnet | ja (layer-shell) |
| Xfce (X11; xfdesktop) | entfällt (X11) | xfdesktop „draws the background … and file icons on the desktop using Thunar libraries“, neueste Version 4.21.1 (2026-10-11) — https://docs.xfce.org/xfce/xfdesktop/start (Seite zuletzt geändert 2024-12-11) | Folgerung: ja, als `_NET_WM_WINDOW_TYPE_DESKTOP`-Fenster; xfdesktop muss dann beendet oder abgewählt werden, wie, belegt die Seite nicht (Lücke L-B2) |

Weitere Compositoren laut gleicher Tabelle mit Unterstützung: Jay, Louvre, niri (5); GameScope, Labwc, Mir, Muffin, river, Treeland, Wayfire (4); phoc (3). Ohne Unterstützung: Cage, Mutter, Weston.

### B3. GNOME: wie DING ohne layer-shell einen Desktop zeigt

- DING besteht aus einer GNOME-Shell-Erweiterung (`extension.js`, `emulateX11WindowType.js`) und einem getrennt gestarteten GTK-Programm (`app/`). Die Erweiterung erkennt die Fenster dieses Programms und setzt in Mutter `this._window.set_type(Meta.WindowType.DESKTOP)`, wenn der Titel mit „Desktop Icons “ beginnt; Titelendungen `@!H`, `@!HTD` steuern „T : put this window at the top of the window stack“ und „D : show this window in all desktops“ (`stick()`). Quelle: https://gitlab.com/rastersoft/desktop-icons-ng/-/raw/master/emulateX11WindowType.js (Z. 24–150), Abruf 2026-10-17.
- Folgerung: Unter GNOME Wayland braucht ein se-Desktopfenster eine eigene GNOME-Shell-Erweiterung (JavaScript, an GNOME-Shell-Versionen gebunden), die das se-Fenster per `Meta.Window.set_type(DESKTOP)` herabsetzt; ein unverändertes egui/winit-Fenster kann das nicht selbst.

### B4. egui/winit und layer-shell

- winit: Feature-Wunsch „Support for Wayland's wlr-layer-shell protocol“ (#2582) ist offen (erstellt 2022-12-03, zuletzt aktualisiert 2025-12-22) — https://github.com/rust-windowing/winit/issues/2582, Abruf über GitHub-API 2026-10-17. eframe 0.29 erbt damit keine layer-shell-Unterstützung (Folgerung).
- smithay-client-toolkit (SCTK) 0.21.1 (MIT) enthält `shell::wlr_layer::LayerSurface` — https://docs.rs/smithay-client-toolkit/0.21.1/smithay_client_toolkit/shell/index.html.
- gtk-layer-shell (GTK3) ist „complete and in maintenance mode“, verweist für neue Projekte auf gtk4-layer-shell; „Layer shell __is not supported__ on: Gnome-on-Wayland“ — https://raw.githubusercontent.com/wmww/gtk-layer-shell/master/README.md.
- Auf crates.io wurde bei der Suche „egui layer shell“ kein passendes Paket gefunden (Abfrage über die crates.io-API, 2026-10-17; Lücke L-B3: eine gezielte Prüfung von Paketen wie Wrappern um SCTK mit egui-wgpu/egui-glow fand nicht statt).
- Folgerung: Für KWin/wlroots/Hyprland/COSMIC braucht das se-Desktopfenster einen eigenen Wayland-Pfad (SCTK-Layer-Surface + egui-Renderer ohne eframe-Fensterverwaltung) oder einen X11-Pfad über XWayland mit `X11WindowType::Desktop`; ob XWayland-Fenster vom Typ Desktop in KWin/Mutter als Desktop behandelt werden, ist nicht belegt (Lücke L-B4).

### B5. Eigene Sitzung als stärkste Form

- SDDM liest Sitzungsdateien standardmäßig aus „/usr/local/share/xsessions,/usr/share/xsessions“ (X11) und „/usr/local/share/wayland-sessions,/usr/share/wayland-sessions“ (Wayland) — https://raw.githubusercontent.com/sddm/sddm/develop/data/man/sddm.conf.rst.in Z. 127–129, 175–177. Für GDM wurde die Liste nicht belegt (Lücke L-B5).
- Folgerung: Eine se-Sitzung verlangt Schreibrechte unter `/usr/share` oder `/usr/local/share` (root) und einen vollständigen Compositor/Fenstermanager; das ist kein „nahtloses“ Zusatzmodul, sondern eine eigene Desktop-Umgebung.

### B6. Dateimanager-Einbindung

Lesezeichen/Orte und FUSE-Laufwerke sind in `docs/refs/linux-fuse-laufwerke-2026-10-17.md` erhoben (hier nicht neu erhoben). Ergänzend: KDE Folder View kann über `url` jeden KIO-Ort zeigen (siehe B2); KDE Userbase: „You can set Folder View to show any folder, local or remote, you have access to“ (https://userbase.kde.org/Translations:Plasma/16/en, Versionsstand der Seite nicht angegeben).

### B7. Vergleichstabelle Abschnitt B

| Weg | X11 (Xfce, KDE-X11, übrige EWMH-WMs) | KDE Wayland | GNOME Wayland | Sway/Hyprland/COSMIC | Rechte | Rückbau | Umsetzungsmittel |
|---|---|---|---|---|---|---|---|
| L1 egui-Fenster Typ Desktop | ja (EWMH) | nein (Wayland) | nein | nein | Nutzer | Programm beenden | egui 0.29 `with_window_type` vorhanden |
| L2 layer-shell-Fläche `background`/`bottom` | entfällt | ja | nein | ja | Nutzer | Programm beenden | SCTK 0.21 + eigener egui-Renderpfad (neu zu bauen) |
| L3 GNOME-Shell-Erweiterung setzt `Meta.WindowType.DESKTOP` | entfällt | entfällt | ja | entfällt | Nutzer (Erweiterung unter `~/.local/share/gnome-shell/extensions`, Annahme) | Erweiterung deaktivieren/entfernen | JS-Erweiterung je GNOME-Version, DING als Vorbild (GPL-3.0-only) |
| L4 Desktopsymbole des Systems auf se-Ordner richten (Folder View `url`, `.desktop`-Starter im Desktop-Ordner) | ja (xfdesktop zeigt Dateien des Desktop-Ordners) | ja (KIO-URL) | nur mit DING | nein | Nutzer | Einträge löschen | kein neuer Fensterpfad |
| L5 eigene Sitzung | ja | ja | ja | ja | root | Sitzungsdatei löschen | vollständige Desktop-Umgebung |
| L6 Dateimanager-Orte/FUSE | ja | ja | ja | ja | Nutzer | siehe Laufwerks-Ref | S2 |

---

## C. Android-Launcher

Referenzseiten (developer.android.com, Stand der Referenz am Abrufdatum 2026-10-17; die Seiten führen bereits Einträge „Added in version 37.2“): RoleManager, LauncherApps, AppWidgetManager, AppWidgetHost, AppWidgetHostView, AppWidgetProviderInfo, PackageManager, Settings, Intent, WallpaperManager, WindowManager.LayoutParams, NotificationListenerService — jeweils `https://developer.android.com/reference/android/<paket>/<Klasse>`.

### C1. Pflichten eines „normalen Launchers“ und die zugehörigen öffentlichen APIs

| Pflicht | Öffentliche API (API-Level) | Bedingung/Einschränkung | Beleg (Zitat) |
|---|---|---|---|
| Startbildschirm sein | Aktivität mit `Intent.CATEGORY_HOME` (API 1) | Nutzer wählt Standard | „This is the home activity, that is the first activity that is displayed when the device boots.“ |
| Standard-Launcher-Wahl | `RoleManager.ROLE_HOME` = `"android.app.role.HOME"` (API 29), `createRequestRoleIntent(String)`, `isRoleHeld`, `isRoleAvailable`; `Settings.ACTION_HOME_SETTINGS` (API 21) | Nutzer bestätigt; Rollenanfrage öffnet Systemdialog | ACTION_HOME_SETTINGS: „Show Home selection settings. If there are multiple activities that can satisfy the Intent.CATEGORY_HOME intent, this screen allows you to pick your preferred activity.“ |
| App-Liste aller Profile | `LauncherApps.getProfiles()` (API 26), `getActivityList(String, UserHandle)`, `LauncherApps.Callback` | Privater Bereich nur mit Berechtigung | getProfiles: „To get hidden profile UserManager.USER_TYPE_PROFILE_PRIVATE, caller should have normal Manifest.permission.ACCESS_HIDDEN_PROFILES permission and the RoleManager.ROLE_HOME role.“ |
| Arbeitsprofil-Kennzeichnung/Private Space | `LauncherApps.getLauncherUserInfo(UserHandle)` (API 35), `getPreInstalledSystemPackages`, `getPrivateSpaceSettingsIntent()` | wie oben: ACCESS_HIDDEN_PROFILES + ROLE_HOME für privaten Bereich | getLauncherUserInfo: „Returns information related to a user which is useful for displaying UI elements to distinguish it from other users (eg, badges).“ |
| App-Verknüpfungen (Shortcuts) anzeigen/starten | `LauncherApps.getShortcuts(ShortcutQuery, UserHandle)` (API 25), `pinShortcuts`, `startShortcut` | nur mit `hasShortcutHostPermission()` | „Access is currently available to: The current launcher (or default launcher if there is no set current launcher). The currently active voice interaction service.“ |
| Angeheftete Shortcuts/Widgets annehmen | `LauncherApps.ACTION_CONFIRM_PIN_SHORTCUT`, `ACTION_CONFIRM_PIN_APPWIDGET` (API 26), `getPinItemRequest(Intent)`, `PinItemRequest.accept(Bundle)` | nur Standard-Launcher | „For the default launcher to show the confirmation dialog to create a pinned shortcut.“; `AppWidgetManager.isRequestPinAppWidgetSupported()`: „Returns true if the default launcher supports requestPinAppWidget…“ |
| Widgets binden | `AppWidgetHost`, `AppWidgetManager.bindAppWidgetIdIfAllowed(int, ComponentName)` (API 16), sonst `ACTION_APPWIDGET_BIND` | „You need the BIND_APPWIDGET permission or the user must have enabled binding widgets always for your component.“ (BIND_APPWIDGET ist eine Systemberechtigung, Annahme aus Schutzstufe, nicht geprüft) | AppWidgetManager |
| Widget-Konfiguration | `ACTION_APPWIDGET_CONFIGURE` (API 3), `AppWidgetHost.startAppWidgetConfigureActivityForResult(…)` (API 21) für Widget-Anbieter in einem anderen Nutzerprofil; `WIDGET_FEATURE_RECONFIGURABLE` (API 28), `WIDGET_FEATURE_CONFIGURATION_OPTIONAL` (API 31) | — | „Use this method if the provider is in another profile as you are not allowed to start an activity in another profile.“; CONFIGURATION_OPTIONAL: „The host may choose not to launch the provided configuration activity.“ |
| Widget-Größenänderung | `AppWidgetHostView.updateAppWidgetSize(Bundle, List<SizeF>)` (API 31) | — | „Provide guidance about the size of this widget … The sizes should correspond to the full area the AppWidgetHostView is given.“ |
| Dynamische Widget-Farben | `AppWidgetHostView.setColorResources`/`resetColorResources` (API 31) | — | AppWidgetHostView |
| Hintergrundbild | Fensterflag `FLAG_SHOW_WALLPAPER` (API 5) bzw. Theme mit Hintergrund; `WallpaperManager.setWallpaperOffsets(IBinder, float, float)`; Setzen über `setBitmap` | Lesen des Bilds als Drawable: „only apps with Manifest.permission.MANAGE_EXTERNAL_STORAGE should use this method“ (`getDrawable(int)`); Anzeigen über Flag braucht keine Leseberechtigung (Folgerung) | „Window flag: ask that the system wallpaper be shown behind your window. The window surface must be translucent …“ |
| Benachrichtigungspunkte | `NotificationListenerService` mit `BIND_NOTIFICATION_LISTENER_SERVICE` | Nutzer muss Zugriff gewähren über `Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS` (API 22) bzw. `ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS` (API 30) | „Users can grant and deny access to notifications for a ComponentName from here.“ |
| App-Info | `LauncherApps.startAppDetailsActivity(ComponentName, UserHandle, Rect, Bundle)` (API 21) | — | LauncherApps |
| Deinstallieren | `Intent.ACTION_UNINSTALL_PACKAGE` „deprecated in API level 29. Use android.content.pm.PackageInstaller.uninstall(String,IntentSender) instead“; `ACTION_DELETE` | Berechtigungsbedarf für `PackageInstaller.uninstall` nicht gelesen (Lücke L-C1) | Intent |
| Ziehen und Ablegen, Ordner, Raster, Suche | keine Launcher-spezifische System-API; Launcher-eigene Logik (Folgerung) | — | — |
| Übersicht/Recents und Gestennavigation | keine öffentliche API für Drittanbieter | siehe C2 | — |

### C2. Gestennavigation/Recents mit Drittanbieter-Launcher

- AOSP Launcher3 `quickstep/src/com/android/quickstep/OverviewComponentObserver.java` (Zweig `main`): Ist der Standard-Home nicht der eigene Launcher, nutzt Quickstep `RecentsActivity` als Ersatz-Übersicht: „The default home app is a different launcher. Use the fallback Overview instead.“ (Z. 190–208). Quelle: https://android.googlesource.com/platform/packages/apps/Launcher3/+/refs/heads/main/quickstep/src/com/android/quickstep/OverviewComponentObserver.java, Abruf 2026-10-17.
- Folgerung: Mit se als Standard-Launcher zeichnet auf AOSP-nahen Geräten die System-Quickstep die Übersicht und die Wischgesten-Animationen; se kann Recents nicht ersetzen. Herstellerlaunchern (Samsung One UI Home, Pixel Launcher) wurde das nicht nachgeprüft (Lücke L-C2). Lawnchair nennt Recents-Integration nur mit Root: „QuickSwitch support for Android Recents integration on Android 15-16 (root required).“ (README Zweig `16-dev`).

### C3. Was beim Deaktivieren eines aktiven Home-Launchers passiert

- `PackageManager.setComponentEnabledSetting(ComponentName, int, int)` (API 1): „Set the enabled setting for a package component (activity, receiver, service, provider). This setting will override any enabled state which may have been set by the component in its manifest.“ Flag `DONT_KILL_APP` verhindert das Beenden der App.
- AOSP RoleService registriert `Intent.ACTION_PACKAGE_CHANGED` und ruft dann `maybeGrantDefaultRolesAsync` (https://android.googlesource.com/platform/packages/modules/Permission/+/refs/heads/main/service/java/com/android/role/RoleService.java, Z. 238–256). `RoleControllerServiceImpl.onGrantDefaultRoles` entfernt Rolleninhaber, die nicht mehr qualifiziert sind („Removing package that no longer qualifies for the role“), und setzt danach Standard- oder Ersatzinhaber („If there is no holder for a role now … we need to add default or fallback holders, if any.“), Z. 149–208 (…/role-controller/java/com/android/role/controller/service/RoleControllerServiceImpl.java).
- `HomeRoleBehavior.getFallbackHolderAsUser` wählt unter allen HOME-Aktivitäten die mit der höchsten Intent-Filter-Priorität, überspringt die Settings-Ersatz-Home und liefert `null` bei Gleichstand (…/role-controller/java/com/android/role/controller/behavior/HomeRoleBehavior.java Z. 76–110).
- Folgerung: Schaltet se seine HOME-Aktivität per `setComponentEnabledSetting(…DISABLED…)` ab, verliert se die Rolle; Android setzt den Inhaber mit höchster Priorität (auf AOSP-nahen Geräten den System-Launcher, Annahme: dessen Filter hat höhere Priorität) oder lässt die Rolle leer, worauf beim nächsten Home-Druck die Auswahl erscheint (Annahme, nicht im Quelltext nachgelesen, Lücke L-C3). Ein „Zurück zum vorherigen Launcher“ (falls das ein Drittanbieter-Launcher war) speichert Android nicht; se muss den vorherigen Rolleninhaber selbst merken und den Nutzer über `ACTION_HOME_SETTINGS` dorthin führen (Folgerung; eine API zum Setzen der Rolle für fremde Pakete ist nur `RoleManager.addRoleHolderAsUser` mit Systemberechtigung, nicht geprüft, Lücke L-C4).

### C4. Launcher3 eigenständig bauen und „Standard-Launcher bearbeiten“

- AOSP `Android.bp` (Zweig `main`): Modul `Launcher3` (ohne Quickstep) baut mit `sdk_version: "current"`, `min_launcher3_sdk_version = "31"`, aber `privileged: true`, `system_ext_specific: true`, `overrides: ["Home", "Launcher2"]`; Modul `Launcher3QuickStep` setzt `platform_apis: true` und `privileged: true` — https://android.googlesource.com/platform/packages/apps/Launcher3/+/refs/heads/main/Android.bp (Z. 20, 414–445, ~562–573). Eine Gradle-Datei `build.gradle` gibt es auf `main` nicht (Abruf liefert „NOT_FOUND“).
- Lawnchair 16 (`LawnchairLauncher/lawnchair`, Zweig `16-dev`, Lizenz Apache-2.0 laut `LICENSE.txt`, letzter Push 2026-10-17): „Taking Launcher3—Android’s default home app—as a starting point“; baut mit Gradle gegen vorkompilierte Plattform-JARs `prebuilts/libs/framework-10.jar` … `framework-14.jar` und weitere, `SystemUI-core-16.jar`, `WindowManager-Shell-16.jar`, `QuickstepResLib.jar` als `compileOnly`, `minSdk = 26`, `targetSdk = 37`.
- Neo Launcher (`NeoApplications/Neo-Launcher`, GPL-3.0, Java, letzter Push 2026-09-26): „The modern and feature-rich AOSP-based Launcher!“; Namespace `com.android.launcher3`, `minSdk = 26/30`, `targetSdk = 37`, nutzt Compose-BOM (`platform(libs.compose.bom)`); README: „The original AOSP Launcher3 code is licensed under the Apache License, Version 2.0“.
- Folgerung: Den vorinstallierten Standard-Launcher eines Geräts zu „bearbeiten“ geht als App nicht (privilegierte System-App, Systemsignatur). Ein Launcher3-Ableger als normale App verliert Quickstep/Recents (C2) und braucht für Plattform-APIs vorkompilierte Framework-JARs je Android-Version (Lawnchair-Bauweise). Das ist nach der Nutzerregel „if you must do substantial changes, build new“ eine substanzielle Änderung.

### C5. Quelloffene Launcher: Lizenz und Bauweise

Quelle: GitHub-API `https://api.github.com/repos/<repo>` (Felder `license.spdx_id`, `language`, `pushed_at`), Abruf 2026-10-17, plus jeweilige Build-Dateien.

| Launcher | Repository | Lizenz | Sprache/UI | Basis | Widgets | Letzter Push | minSdk/targetSdk |
|---|---|---|---|---|---|---|---|
| AOSP Launcher3 | android.googlesource.com/platform/packages/apps/Launcher3 | Apache-2.0 (Kopf in Lawnchairs `LICENSE.txt`: „Copyright (c) 2005-2008, The Android Open Source Project“) | Java/Kotlin, Views | — | ja | laufend | 31 / current |
| Lawnchair | LawnchairLauncher/lawnchair | Apache-2.0 (`LICENSE.txt`; GitHub meldet NOASSERTION) | Java/Kotlin, Views + Compose-Modul | Launcher3 (Android 16) | ja | 2026-10-17 | 26 / 37 |
| Neo Launcher | NeoApplications/Neo-Launcher | GPL-3.0 | Java/Kotlin, Compose-BOM eingebunden | Launcher3 | ja (README: „A unique custom-widget.“; AOSP-Widgets folgen aus der Basis, Folgerung) | 2026-09-26 | 26–30 / 37 |
| Kvaesitso | MM2-0/Kvaesitso | GPL-3.0 | Kotlin, Compose (`androidx-compose = "1.13.0-beta01"` im Versionskatalog) | Neubau, „search focused“ | ja: `app/ui/…/widgets/external/AppWidgetHost.kt` bettet `AppWidgetHostView` per `AndroidView` in Compose ein und nutzt `SizeF` | 2026-10-16 | 26 / 37 |
| KISS | Neamar/KISS | GPL-3.0 | Java, Views | Neubau (Suchlauncher) | nicht erhoben (Lücke L-C5) | 2026-10-17 | nicht ausgelesen |
| Olauncher | tanujnotes/Olauncher | GPL-3.0 | Kotlin, Views (13 Layout-XML-Dateien) | Neubau (minimal) | keine Widget-Dateien im Baum gefunden (Folgerung: keine Widgets) | 2026-10-17 | 24 / 36 |
| Fossify Launcher | FossifyOrg/Launcher | GPL-3.0 | Kotlin, Views (Detekt-Compose-Plugin eingebunden) | Neubau (Simple-Mobile-Tools-Linie, Annahme) | ja: „COMPLETE WIDGET SUPPORT … fully resizable widgets“ | 2026-10-15 | aus Versionskatalog, nicht ausgelesen |

Lizenzfolgerung (keine Rechtsberatung): se ist MIT-lizenziert. Apache-2.0-Code (Launcher3, Lawnchair) darf mit Hinweis-/NOTICE-Pflichten in MIT-Projekte; GPL-3.0-Code (Neo, Kvaesitso, KISS, Olauncher, Fossify) würde die ausgelieferte Android-App unter GPL-3.0 stellen, Lesen als Vorbild ohne Übernahme von Code ist davon nicht betroffen.

### C6. Compose-Eignung

- Belegtes Beispiel eines vollständigen Compose-Launchers mit fremden Widgets: Kvaesitso (oben). Neo Launcher und Lawnchair binden Compose zusätzlich zu Launcher3-Views ein.
- Leistungsmessungen von Compose-Launchern: keine Primärquelle gefunden (Lücke L-C6).
- Bestand se: `minSdk = 30`, `targetSdk = 36`, `compileSdk = 36`, Compose aktiv. Folgerung: Alle Widget-APIs in C1 bis API 31 sind ab minSdk 30 verfügbar außer `updateAppWidgetSize(Bundle, List<SizeF>)`, `setColorResources` und `WIDGET_FEATURE_CONFIGURATION_OPTIONAL` (API 31) sowie `getLauncherUserInfo`/Private Space (API 35); diese brauchen Versionsweichen.

### C7. Getrennt aktivierbar: eigene APK oder Komponente in der bestehenden APK

| Kriterium | Komponente in se-APK (HOME-Aktivität per `setComponentEnabledSetting`) | eigene Launcher-APK |
|---|---|---|
| „Hinzufügen“ | ab Installation vorhanden, standardmäßig `android:enabled="false"` (Folgerung) | separater Download/Installation (Sideload braucht Installationsfreigabe; Annahme) |
| Aktivieren | Komponente einschalten + Rollenanfrage `ROLE_HOME` | Installation + Rollenanfrage |
| Deaktivieren | Komponente ausschalten → Rolle fällt weg (C3); Widget-IDs, `NotificationListenerService`-Freigabe und gespeicherte Rasterdaten bleiben in der App, bis se sie löscht (Folgerung) | APK deinstallieren entfernt Daten, Widget-Host-IDs, Freigaben (Folgerung) |
| Prozessgemeinschaft mit se-Kern | ja: direkter Zugriff auf Rust-Bridge, Geräte, Laufwerke | nein: IPC zur se-App nötig |
| Speicher/Absturzisolierung | Launcher-Absturz trifft se-Prozess, sofern gleicher Prozess (Folgerung; `android:process` trennbar) | getrennt |
| Rückkehr zum vorherigen Launcher | se merkt sich vorherigen Inhaber, führt per `ACTION_HOME_SETTINGS` zurück | gleich |

---

## D. Modulare Auslieferung und „vollständig deaktivieren“

### D1. PowerToys (Microsoft, MIT, `microsoft/PowerToys`, letzter Push 2026-10-17)

- Modulmodell: Jedes Modul ist eine DLL mit Fabrikfunktion `powertoy_create()`; Schnittstelle `PowertoyModuleIface` mit `get_name`, `get_events`, `get_config`, `set_config`, `call_custom_action`, `enable`, `disable`, `is_enabled`, `destroy`. „The returned PowerToy should be in the disabled state. The runner will call the enable() method to start the PowerToy.“ Beim Beenden: `disable()`, `destroy()`, DLL entladen. Quelle: https://raw.githubusercontent.com/microsoft/PowerToys/main/doc/devdocs/modules/interface.md.
- Registrierungen beim Ein-/Ausschalten: Explorer-Erweiterungen (Vorschau- und Miniaturhandler) sind als `registry::ChangeSet` beschrieben (`getSvgPreviewHandlerChangeSet`, `getMdPreviewHandlerChangeSet`, `getMonacoPreviewHandlerChangeSet`, `getPdfPreviewHandlerChangeSet`, `getGcodePreviewHandlerChangeSet`, `getBgcodePreviewHandlerChangeSet`, `getQoiPreviewHandlerChangeSet`, `getSvgThumbnailHandlerChangeSet` sowie weitere in derselben Datei ab Z. 176, nicht einzeln gelesen; jeweils mit Parametern `installationDir`, `perUser`) mit `apply()`, `unApply()`, `isApplied()` (`src/common/utils/modulesRegistry.h`, `src/common/utils/registry.h`); `PowerPreviewModule::disable()` ruft `registryChanges.unApply()` für jedes Explorer-Modul, Umschalten ruft je nach Zustand `apply()` oder `unApply()` (`src/modules/previewpane/powerpreview/powerpreview.cpp` Z. 170–269).
- Installation: „Per User“ unter `%userprofile%\AppData\Local\Programs` oder „Machine wide“ unter `%ProgramFiles%`; Module werden in den Einstellungen einzeln ein- und ausgeschaltet; Bereinigungsskripte für fehlgeschlagene Deinstallationen unter `tools/CleanUp_tool` — https://learn.microsoft.com/en-us/windows/powertoys/install, ms.date 2025-08-20, updated_at 2026-07-14.
- Folgerung: PowerToys liefert alle Module mit und schaltet sie zur Laufzeit; „aus“ heißt dort: Modul läuft nicht und seine Registrierungen sind zurückgenommen, Dateien bleiben installiert.

### D2. NSIS-Komponenten (Bestand nutzt NSIS-Installer laut AGENTS.md)

- „If a section's name is 'Uninstall' or is prefixed with 'un.', it's an uninstaller section.“ `Section [/o] [([!]|[-])section_name] [section_index_output]`: „If section_name is empty, omitted, or begins with a -, then it is a hidden section … If the /o switch is specified, the section will be unselected by default.“ `SectionInstType … RO` macht eine Sektion schreibgeschützt; Komponentenseite erscheint, wenn mehr als eine sichtbare Sektion existiert. Quelle: https://nsis.sourceforge.io/Docs/Chapter4.html (Abschnitte 4.5, 4.6.1), Abruf 2026-10-17; NSIS-Version der Doku nicht ausgelesen (Lücke L-D1).
- Folgerung: Ein optionaler se-Desktop kann eine `Section /o "se Desktop"` sein; die Installer-Auswahl allein deckt aber kein späteres Ein-/Ausschalten aus der App ab — dazu braucht se eine Laufzeitschaltung wie D1.

### D3. Nachinstallation aus dem Update-Feed

- Bestand laut AGENTS.md (nicht neu erhoben): Der Update-Feed `release-native/update-feed/` führt je Plattform Nutzlasten mit `.sha256`-Dateien; „For staged updates or elevated helper flows, bind every staged executable to an expected SHA-256 and revalidate it immediately before replacement or relaunch.“
- Folgerung: Ein Desktop-Modul als eigene Feed-Datei (z. B. eigene DLL/Programmdatei) muss in Release-Wrapper, Feed, Hash-Prüfung, README und `docs/RELEASING.md` gleichzeitig aufgenommen werden (AGENTS.md: „Any new CI/release guard added during a fix must be reflected in docs and scripts together“). Ein Modul, das im Hauptprogramm einkompiliert und nur zur Laufzeit geschaltet wird, braucht keine Feed-Änderung.

### D4. Was „vollständig deaktivieren“ je Plattform technisch umfasst

| Spur | Windows | Linux | Android |
|---|---|---|---|
| Autostart | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (bzw. `RunOnce`) — Wert löschen; „The data value for a key is a command line no longer than 260 characters.“ (https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys, ms.date 2024-07-19); Verknüpfung im Startup-Ordner löschen | XDG-Autostart-Datei unter `$XDG_CONFIG_HOME/autostart` löschen oder `Hidden=true`: „When the .desktop file has the Hidden key set to true, the .desktop file MUST be ignored.“ (Autostart-Spec Version 0.5, https://specifications.freedesktop.org/autostart-spec/latest/); systemd-Nutzereinheiten deaktivieren (nicht neu belegt) | HOME-Komponente aus (C3); `NotificationListenerService`-Komponente aus |
| Shell-/Desktop-Registrierungen | HKCU-Schlüssel aus A3 (I1: 12 Schlüssel/Werte; I2: COM-Registrierung + `NameSpace`-Schlüssel); `Winlogon\Shell` unter HKCU löschen (W4); Sparse-Paket abmelden (Kontextmenü, Bestand) | GNOME-Erweiterung: `gnome-extensions disable <uuid>`, Ordner unter `~/.local/share/gnome-shell/extensions` löschen (https://gjs.guide/extensions/development/creating.html); KDE: Folder-View-`url` zurücksetzen bzw. Plasmoid entfernen; `.desktop`-Starter im Desktop-Ordner löschen; Sitzungsdatei in `/usr/share/wayland-sessions` bzw. `xsessions` löschen (root) | Rolle `ROLE_HOME` geht beim Deaktivieren verloren; Widget-IDs über `AppWidgetHost.deleteAppWidgetId`/`deleteHost` freigeben (Methodennamen aus AppWidgetHost-Referenz, Einzelzitat nicht ausgelesen, Lücke L-D2) |
| Laufende Prozesse/Fenster | Desktopfenster schließen; Explorer zeichnet neu (W1–W3); bei W4 Ab- und Anmeldung | Desktopfenster/Layer-Fläche schließen; GNOME-Shell-Erweiterungen unter Wayland laden ohne Neuanmeldung nicht neu (Annahme aus dem Hinweis auf verschachtelte Sitzungen in der GJS-Anleitung, Lücke L-D3) | Launcher-Aktivität beenden; Rollenwechsel durch System |
| In Explorer geladene DLLs | bleiben bis Explorer-Neustart geladen (Annahme für I2/Kontextmenü-DLL, nicht belegt, Lücke L-D4) | entfällt | entfällt |
| Dienste/Treiber | Dokany-Treiber gehört zu S2, nicht zum Desktop-Modul | FUSE gehört zu S2 | entfällt |
| Dateien | Moduldatei(en) unter `%LOCALAPPDATA%\Programs\Smart Explorer` entfernen, falls als Feed-Datei nachinstalliert | entsprechend | eigene APK deinstallieren oder Komponentendaten löschen |
| Gespeicherter Zustand | Desktop-Layout/Konfiguration löschen oder für Reaktivierung behalten (Entscheidung offen, nicht Recherchefrage) | gleich | Raster, Ordner, Widget-Zuordnungen |

### D5. Vergleichstabelle Abschnitt D

| Modell | Getrennt hinzufügbar | Laufzeit-Ein/Aus | Rückbau der Registrierungen | Dateien beim Aus entfernt | Beispiel |
|---|---|---|---|---|---|
| M1 einkompiliert + Laufzeitschalter | nein (immer installiert) | ja | ja, durch Modulcode (`unApply`) | nein | PowerToys |
| M2 Installer-Komponente | ja (bei Installation) | nein ohne M1 | Uninstaller-Sektion `un.` | ja bei Deinstallation | NSIS `Section /o` |
| M3 Nachinstallation aus Feed + M1 | ja | ja | ja | ja, wenn Modul selbst gelöscht wird | Bestand-Feed mit SHA-256 (Folgerung) |
| M4 Android-Komponente in se-APK | nein (in APK) | ja (`setComponentEnabledSetting`) | Rolle fällt automatisch (C3) | nein | — |
| M5 eigene Android-APK | ja | ja (Installation/Deinstallation) | ja (Deinstallation) | ja | Lawnchair, Kvaesitso (eigenständige Apps) |

---

## Widersprüchliche Angaben

1. Lawnchair-Lizenz: GitHub-API meldet `NOASSERTION`, die Datei `LICENSE.txt` im Zweig `16-dev` ist Apache License 2.0. Diese Datei gilt hier als maßgeblich.
2. Cloud-Files-Registrierung: Die ältere Seite „Integrate a Cloud Storage Provider“ (ms.date 2018-05-31) beschreibt Sync-Root-Schlüssel unter `HKLM\…\SyncRootManager\…` (Admin), die neuere Seite „Build a Cloud Sync Engine…“ (ms.date 2025-05-12) nennt die Desktop Bridge als „implementation requirement“ und `StorageProviderSyncRootManager.Register`; die Referenzseite dazu schweigt zur Paketidentität. Was davon für ein unverpacktes Programm gilt, ist offen (L-A6).
3. Winlogon-Shell pro Nutzer: Microsoft dokumentiert nur `HKLM\…\Winlogon\Shell`; Cairo/ManagedShell schreibt und liest `HKCU\…\WinLogon\Shell` und nutzt es als Nutzer-Shell. Der Microsoft-Beleg für die Wirksamkeit des HKCU-Werts fehlt.
4. Taskleisten-Anheften: Die Seite „Taskbar Extensions“ (ms.date 2018-05-31) sagt „programmatic pinning by an application is not permitted“; die neuere Seite „Pin your app to the taskbar“ (ms.date 2026-07-16) erlaubt eine vom Nutzer bestätigte Anfrage über `TaskbarManager`. Die neuere Seite gilt (Nutzerbestätigung bleibt Pflicht).
5. 24H2-Desktophierarchie: Die Microsoft-Beschreibung ist nur als Zitat im Lively-Quelltext vorhanden, nicht auf einer Microsoft-Seite gefunden.

## Offene Lücken

- L-A1: Keine Microsoft-Dokumentation zu `Progman`/`WorkerW`/`0x052C`.
- L-A2: Microsoft-Release-Health-Eintrag zur 24H2-Sperre für Hintergrund-Apps nicht direkt gelesen (nur The Register).
- L-A3: Kein belegtes API, um Explorer-Desktopsymbole programmgesteuert auszublenden.
- L-A4: Verhalten eines `HWND_BOTTOM`-Vollbildfensters bei „Desktop anzeigen“/Win+D nicht belegt.
- L-A5: Vollständige Liste der Pflichtschnittstellen einer eigenen Namensraumerweiterung nicht gelesen (Seite `nse-implementing` liefert 404).
- L-A6: Ob ein unverpacktes Programm ohne Sparse-Paket einen Cloud-Files-Sync-Root registrieren darf.
- L-A7: Ob ein Sparse-Paket (Paket mit externem Ort) als Widget-Anbieter zulässig ist.
- L-B1: Ob COSMIC Desktopsymbole zeichnet.
- L-B2: Wie xfdesktop abgeschaltet/ersetzt wird.
- L-B3: Ob ein egui-Paket für layer-shell existiert (keine gezielte Prüfung über die Suche hinaus).
- L-B4: Ob XWayland-Fenster vom Typ `_NET_WM_WINDOW_TYPE_DESKTOP` in KWin bzw. Mutter als Desktop behandelt werden.
- L-B5: Sitzungsverzeichnisse von GDM.
- L-C1: Berechtigungsbedarf von `PackageInstaller.uninstall` für Launcher.
- L-C2: Verhalten der Gestennavigation mit Drittanbieter-Launcher auf Samsung One UI und Pixel.
- L-C3: Was genau bei leerer HOME-Rolle angezeigt wird (Auswahldialog oder Settings-Ersatz-Home).
- L-C4: Ob eine App die HOME-Rolle an einen zweiten, fremden Launcher zurückgeben kann (nur Systemberechtigung vermutet).
- L-C5: Widget-Unterstützung von KISS.
- L-C6: Leistungsmessungen von Compose-Launchern.
- L-D1: NSIS-Doku-Version.
- L-D2: Wörtliche Belege zu `AppWidgetHost.deleteAppWidgetId`/`deleteHost`.
- L-D3: Ob GNOME-Shell-Erweiterungen unter Wayland ohne Neuanmeldung aktiviert/deaktiviert werden.
- L-D4: Ob Explorer eine abgemeldete Shell-Erweiterungs-DLL bis zum Neustart geladen hält.
