# Oberflächen von se Desktop und se Launcher

Stand: 2026-10-17, angelegt von Paket U15 (DSK-M3, Modulrahmen Desktop) des Stapels „Rechte, se Desktop, Tresore“ (Plan: `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, Strang S5: `strang-desktop.md`). Lesestand: Arbeitsstand nach `2e3d07b1` mit den Dateien von U15; nichts davon ist gebaut oder getestet (AGENTS.md: keine lokalen Builds), die Abnahme läuft in der Stapel-Suite. Belege als `Datei:Zeile`. Pflege: jedes Paket, das eine Oberfläche von se Desktop oder se Launcher ändert, aktualisiert diese Datei im selben Commit (U30, U32, U34, U37, U40); DSK-M14 (U43) prüft und vervollständigt sie (`plan.md` C6, W4).

Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`, `<AD>` = App-Datenordner (`support_dirs::app_data_dir()`: Windows `%APPDATA%\smart_explorer`, Linux `$XDG_DATA_HOME/smart_explorer`). „–“ = nicht erreichbar. „R5“ = Registrierungspaket der Phase P5, das die Hub-Dateien verdrahtet.

## 1 Oberflächen

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se desktop status\|enable\|disable\|remove\|part\|config\|restart`, jeweils mit `--json` | `N/cli/desktop.rs:47-93` (Befehle), `:202-221` (Ausführung), Ausgabe `N/cli/desktop/output.rs:19-167`; Einhängen in `se` über R5 (`N/cli/mod.rs`); Vervollständigung über clap-`ValueEnum` und das bestehende `clap_complete::env` |
| Desktop | Einstellungen → Integration → „se Desktop“ (letzter Abschnitt der Seite, V14) | `N/app/core/settings_ui.rs:72`, Abschnitt `N/app/core/desktop_settings_ui.rs:179-414` |
| Desktop-Prozesse | Wächter `--desktop-shell`, Fläche `--desktop-surface --monitor <SCHLÜSSEL>\|--gnome-monitor <INDEX>\|--layer`, `--desktop-jumplist-clear`, Fenster `--open-location <ORT>` und `--open-page <SEITE>` | Startschalter-Parser V9 `N/app/core/start_request.rs`, Verteilung `N/desktop/os/mod.rs:129-151` (Aufruf in `run_gui` über R5), Wächter `N/desktop/os/shared/supervisor.rs:41`, Fläche `N/app/core/desktop_surface_app.rs:45`, Fensterstart `N/app/core/desktop_settings_ui.rs:333-349` |
| Android | se Launcher (Einstellungen → Startbildschirm) | folgt mit DSK-M9 bis DSK-M13 (U35, U38, U40) |
| Bridge | keine neue Methode (C5.8) | – |
| IPC | keine neue Anfrage; Lesen ohne Dienststart `share_worker_peek()` | `N/daemon/os/shared/ipc_peek_client.rs:31-67` |
| MCP | Werkzeug `se` mit der CLI-Spalte; `status` enthält `desktop` | über R5 (`N/cli/mcp/catalog.rs`, `N/cli/mcp/status_tools.rs`); Inhalt `N/desktop/os/mod.rs:154` |
| Doku | README, `docs/RELEASING.md`, `docs/GOTCHAS.md`, `docs/ARCHITEKTUR.md` | Doku-Pakete U42, U43 |

## 2 Fähigkeiten O1–O19

Tabelle aus `strang-desktop.md` C7; Spalte „Stand“ nennt, was nach U15 im Code steht.

| Nr | Fähigkeit | CLI (Text/JSON/Vervollständigung) | Desktop-GUI | Android-GUI | Bridge | IPC | MCP | Stand |
|---|---|---|---|---|---|---|---|---|
| O1 | Zustand Desktop-Modul | `se desktop status [--json]` (`N/cli/desktop.rs:204-207`, `N/desktop/os/shared/status.rs:102-179`, `:232-313`) | Integration → se Desktop (`N/app/core/desktop_settings_ui.rs:228-299`) | – (Plattform) | – | – | `se`, `status` (R5) | umgesetzt (U15) |
| O2 | Aktivieren, Deaktivieren, Entfernen | `se desktop enable\|disable\|remove [--json]` (`N/cli/desktop.rs:208-214`, `N/desktop/os/shared/control.rs:88`, `:147`, `:180`) | Knöpfe „Aktivieren“, „Deaktivieren“, „Entfernen …“ mit Bestätigung (`N/app/core/desktop_settings_ui.rs:228-299`) | – | – | – | `se`; Änderungen nur auf ausdrücklichen Nutzerwunsch (Katalogregel, R5) | umgesetzt (U15) |
| O3 | Teil schalten | `se desktop part <surface\|nav-pane\|jump-list> on\|off [--json]` (`N/cli/desktop.rs:215-217`, `N/desktop/os/shared/control.rs:248`) | Kontrollkästchen je Teil mit Grund bei „nicht verfügbar“ (`N/app/core/desktop_settings_ui.rs:352-382`) | – | – | – | `se` | umgesetzt (U15) |
| O4 | Darstellung, Gruppen, Systemelemente, „Geräte öffnen mit“ | `se desktop config [Optionen] [--json]` (`N/cli/desktop.rs:236-256`, `N/desktop/os/shared/control.rs:289`) | Integration → se Desktop → „Darstellung“ | – | – | – | `se` | CLI umgesetzt (U15); GUI-Abschnitt „Darstellung“ folgt mit DSK-M4 (U30) |
| O5 | GNOME-Desktopsymbole übernehmen/zurückgeben | `se desktop config --foreign-desktop-icons keep\|take-over` (nur Linux) | Knopf „Übernehmen“ | – | – | – | `se` (nur auf ausdrücklichen Nutzerwunsch) | Einstellung umgesetzt (U15); Übernahme und Rückgabe folgen mit DSK-M8 (U34); Wiedereinschalten aus dem Ledger: `N/desktop/os/linux_os/integration.rs:112-130` |
| O6 | Neustart, Anhalten aufheben | `se desktop restart [--json]` (`N/cli/desktop.rs:260-269`, `N/desktop/os/shared/control.rs:313`) | Knopf „Neu starten“ (`N/app/core/desktop_settings_ui.rs:228-299`) | – | – | – | `se` | umgesetzt (U15) |
| O7 | Verbinden (Live-Zustand) | – (jeder `se share`-Befehl startet den Dienst bereits) | Knopf „Verbinden“ auf der Fläche | Kachel „Verbinden“ | – | – | – | Kanalnachricht `connect` → `ensure_worker_ready` umgesetzt (`N/desktop/os/shared/model_feed.rs:104-123`); Knopf folgt mit DSK-M4 (U30), Launcher mit DSK-M13 (U40) |
| O8 | Gerätekacheln und Zustände | `se drive devices status` (S2), `se share device list` (Bestand) | Fläche | Launcher, Widget | `share.status` (Bestand) | `ShareSnapshotPeek` über `share_worker_peek` | `se` | Modell im Wächter umgesetzt (`N/desktop/os/shared/model_feed.rs:132-176`); Ansicht folgt mit DSK-M4 (U30) |
| O9 | Gerät öffnen | – (Fensterhandlung) | Doppelklick, Sprungliste; Fenster `--open-location` (`N/app/core/desktop_settings_ui.rs:333-349`) | Tippen | – | – | – | Fensterstart umgesetzt (U15); Kachel folgt mit DSK-M4 (U30), Sprungliste mit DSK-M5 (U32) |
| O10 | Im Explorer/Dateimanager öffnen | `se drive devices status --json` liefert `path` (S2) | Kontextmenü | „In Dateien öffnen“ | – | – | – | folgt mit DSK-M4 (U30) und DRV-M6 (U17) |
| O11 | Bildschirm ansehen/steuern, Alle Bildschirme | `se screen view`/`grid` (S4) | Kontextmenü, Kachel | Menü, Kachel | – | – | Ausnahme S4 | folgt mit DSK-M4 (U30), S4 SCR-M7 (U25) |
| O12 | Tresor öffnen | `se vault` (S3) | Doppelklick | Tippen | `vault.list` (S3) | `Vault::Status` (S3) | S3 | folgt mit DSK-M4 (U30), S3 VLT-M4 (U11) |
| O13 | Navigationsbereich-Eintrag | `se desktop part nav-pane on\|off` | Teil-Kontrollkästchen | – (Plattform) | – | – | `se` | Schalter und vollständiges Entfernen umgesetzt (`N/desktop/os/windows/integration.rs:170-199`); Anlegen folgt mit DSK-M5 (U32) |
| O14 | Sprungliste | `se desktop part jump-list on\|off` | Teil-Kontrollkästchen | – (Plattform) | – | – | `se` | Schalter und Löschen umgesetzt (`N/desktop/os/windows/integration.rs:58-84`, `:215-234`); Schreiben folgt mit DSK-M5 (U32) |
| O15 | Launcher-Zustand, Aktivieren, Standard, Deaktivieren, Entfernen | – (keine Android-CLI) | – | Einstellungen → Startbildschirm | – | – | – (kein MCP auf Android) | folgt mit DSK-M9 (U35) |
| O16 | Geräte-Widget für fremde Launcher | – | – | Einstellungen → Startbildschirm → „Geräte-Widget“ | – | – | – | folgt mit DSK-M13 (U40) |
| O17 | Launcher-Einstellungen | – | – | Startbildschirm-Einstellungen | – | – | – | folgt mit DSK-M10 bis DSK-M12 (U35, U38) |
| O18 | Privaten Bereich sperren/entsperren, Arbeitsprofil pausieren | – | – | App-Liste | – | – | – | folgt mit DSK-M12 (U38) |
| O19 | Update beendet Desktop-Prozesse und GUI-Fenster, Wiederanlauf | Bestand `se update` | Bestand Update-Dialog | APK-Update (System) | – | – | Bestand | Wächter und Flächen: Exitcode 75 (`N/desktop/os/shared/supervisor.rs:469-479`, `N/app/core/desktop_surface_app.rs:45-55`); Wiederanlauf `resume_on_gui_start` (`N/desktop/os/mod.rs:176`, `N/desktop/os/shared/launch.rs:62-82`, Aufruf in `run_gui` über R5); GUI-Fenster: DSK-M2 (U03) |

Statusausgabe (Vertrag DSK-M3): Text tabulatorgetrennt in der Reihenfolge `state`, je Teil `part <teil> <on|off> <active|inactive|unavailable:<code>>`, `mechanism`, `surfaces <laufend>/<Monitore>`, `share_state <live|profiles:<Grund>>`, `autostart`, `halted <no|crash_loop:<Version>>`, `foreign_desktop`, `residue <Anzahl>` (`N/cli/desktop/output.rs:38-89`); JSON mit den Schlüsseln `platform`, `state`, `state_error`, `parts`, `mechanism`, `mechanism_reason`, `extension_state`, `processes`, `share_state`, `share_state_reason`, `daemon_autostart`, `autostart`, `halted`, `foreign_desktop`, `registrations`, `residue`, `devices_folder`, `layout_saved`, `config` (`N/desktop/os/shared/status.rs:232-313`). `state_error` ergänzt den Vertrag: ein unlesbares `state.json` erscheint dort, der Zustand dann als `disabled`. Jeder ändernde Befehl gibt mit `--json` dasselbe Statusobjekt aus; `restart` gibt `{"restarted","state","reason"}` aus; `config` ohne Option zeigt `{"config": …}`. Exitcodes: 0 Erfolg, 1 Fehler mit Text (auch bei Restspuren oder gescheitertem Start), 2 Aufruffehler.

## 3 Plattformgrenzen

| Plattform | Stand nach U15 | Grenze mit Grund |
|---|---|---|
| Windows | Teile `surface`, `nav_pane`, `jump_list`; Mechanismus „unterstes Werkzeugfenster“ (C1.2); eine Fläche je Monitor aus `EnumDisplayMonitors` (`N/desktop/os/windows/mod.rs:69-117`) | Monitorschlüssel vorläufig aus dem GDI-Gerätenamen; DSK-M5 (U32) nimmt den Gerätepfad aus `IDesktopWallpaper` (C1.2). Lage ganz unten, Stile, Hintergrund, Desktop-Dateien, Navigationsbereich und Sprunglisten-Inhalt folgen mit DSK-M5. Einbetten in den Explorer-Desktop: No-Go (C1.3) |
| Linux X11 | Teil `surface` | Ohne die Sitzungsprüfungen von DSK-M6 (U34: `_NET_SUPPORTED`, RandR) meldet se „nicht verfügbar“ mit dem Grund der Kerntabelle (`N/desktop/os/linux_os/mod.rs:46-68`); Teile `nav_pane`/`jump_list`: „nur unter Windows (Explorer)“ |
| Linux Wayland (Layer-Protokoll) | Teil `surface` durch das eigene Layer-Backend (DSK-M7, U37): **ein** Prozess `--desktop-surface --layer` für alle Ausgänge der Sitzung, Ebene `bottom` (`zwlr_layer_surface_v1`), Anker an allen vier Kanten, Größe 0×0, `exclusive_zone 0`, Namensraum `desktop` unter KDE (`XDG_CURRENT_DESKTOP` enthält `KDE`, damit „Desktop anzeigen“ die Fläche behält), sonst `smart-explorer-desktop`; Tastatur `on_demand` ab Layer-Shell-Version 4, sonst keine (Status `mechanism` `layer_shell`); Bedienung mit Maus, Berührung und Tastatur; Kopieren, Ausschneiden und Einfügen von Dateien über `wl_data_device` (`text/uri-list`, `x-special/gnome-copied-files`); Skalierung ganzzahlig oder als Bruch (`wp_fractional_scale_v1` mit `wp_viewporter`); Ziehen einer Kachel von einem Ausgang auf einen anderen (auch bei verschiedener Skalierung) | Die Compositor-Größe der Fläche ist die Arbeitsfläche des Ausgangs (Leisten mit Platzanspruch verkleinern sie), die Ansicht nutzt die ganze Fläche. `Monitor.rect` ist unter Wayland **logisch** (Koordinaten des Compositors), nur in der Layer-Sitzung. Ohne Ausgänge zeichnet der Prozess nichts und wartet; ändern sich Zahl, Schlüssel oder Lage der Ausgänge, startet der Wächter alle Flächen neu. Ohne `zwlr_layer_shell_v1` ab Version 3, `wl_compositor` ab Version 4 oder `wl_seat` startet das Backend nicht (Exitcode 1, Zeile `se Desktop: Layer-Backend: <Global> fehlt`). Ohne `libxkbcommon.so.0` läuft es ohne Tastatur. Sonst siehe Abschnitt 7 |
| Linux Wayland (GNOME) | Teil `surface` | GNOME-Erweiterung (DSK-M8, U34); vorher „nicht verfügbar“ |
| Android | – | se Launcher als eigenes Modul in der APK (C5.2), Pakete U35, U38, U40 |

## 4 Feste Namen und Pfade

| Name | Wert | Beleg |
|---|---|---|
| Autostart Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, Wert `SmartExplorerDesktop` = `"<GUI>" --desktop-shell`; Abschaltung durch den Nutzer unter `Explorer\StartupApproved\Run` wird gemeldet, nie geschrieben | `N/desktop/core/ledger.rs:13`, `N/desktop/os/windows/autostart.rs:20-81` |
| Autostart Linux | `$XDG_CONFIG_HOME/autostart/smart-explorer-desktop.desktop` mit `Exec=<GUI> --desktop-shell`, `TryExec=<GUI>`, `X-GNOME-Autostart-enabled=true`; `Hidden=true` des Nutzers bleibt Byte für Byte | `N/desktop/core/ledger.rs:18`, `N/desktop/os/linux_os/autostart.rs:54-71` |
| Navigationsbereich | `{DESKTOP_NAV_CLSID}` = `{174877FD-193E-42A6-936E-60CCB4C8F1C7}` unter `HKCU\Software\Classes\CLSID`, `…\Explorer\Desktop\NameSpace` und als Wert unter `…\Explorer\HideDesktopIcons\NewStartPanel` | `N/desktop/core/ledger.rs:16`, `N/desktop/os/windows/integration.rs:22-26` |
| Sprungliste | die Liste des GUI-Abbilds `Smart Explorer.exe`, gelöscht mit `DeleteList(NULL)` nur aus diesem Abbild | `N/desktop/core/ledger.rs:22`, `N/desktop/os/windows/integration.rs:215-234` |
| GNOME-Erweiterung | UUID `se-desktop@smartexplorer.app`, Ordner `$XDG_DATA_HOME/gnome-shell/extensions/se-desktop@smartexplorer.app/`, Eintrag in `org.gnome.shell enabled-extensions` | `N/desktop/core/ledger.rs:20`, `N/desktop/os/linux_os/gnome_settings.rs` |
| Einzelinstanz | Windows benannter Mutex `Local\SmartExplorerDesktopShell` (Debug-Bau mit Testnamensraum), Linux `flock` auf `<AD>/desktop/shell.lock` | `N/desktop/os/windows/instance.rs:20`, `N/desktop/os/linux_os/instance.rs:17-53` |
| Zustand | `<AD>/desktop/state.json` (Schema 1 mit Ledger), `config.json` (Schema 1), `layout/<Monitorschlüssel>.json` (DSK-M4), `state.lock` (Schreibsperre), `shell.json` (laufender Wächter: PIDs, Monitore, Share-Zustand), `shell.log` (Wächterprotokoll, Zeilen `<unix-s.ms>\t<Ereignis>\t<Angabe>`), `shell.stop` (Neustartwunsch) | `N/desktop/os/shared/store.rs:54-254` |
| Laufzeitdateien Linux | `$XDG_RUNTIME_DIR/smart-explorer/desktop.sock`, `desktop.token`, `desktop-gnome.json`, `desktop-extension.json` (GNOME-Kanal, DSK-M8); der Ordner selbst gehört auch S2 und bleibt | `N/desktop/os/linux_os/integration.rs:20-25` |
| Exitcodes | 75 Update-Stopp (V8), 76 Kanalende einer Fläche, beim Layer-Backend auch der Verlust der Compositor-Verbindung und das Schließen der letzten Layer-Fläche; alle drei zählen nie als Absturz. Exitcode 1 des Layer-Backends: fehlendes Pflicht-Global oder keine Grafikbeschleunigung | `N/daemon/os/shared/update_stop.rs:29`, `N/desktop/core/protocol.rs:22` |
| Neustartregel | Wartezeiten 1 s, 5 s, 30 s; drei Abstürze in zehn Minuten halten an (`halted` mit Version, Desktop-Benachrichtigung); ohne `alive` für 10 s endet eine Fläche | `N/desktop/os/shared/crash_rule.rs:10-16`, `N/desktop/os/shared/supervisor.rs:27` |
| Launcher-Komponenten | folgen mit DSK-M9 (U35) | – |

## 5 Modularität

Zustände `absent` (Ordner `<AD>/desktop/` fehlt) → `enabled` ⇄ `disabled` → `absent`; jeder Übergang ist idempotent (`N/desktop/core/state.rs`). Aktivieren legt den Ordner, `config.json`, den Autostart (solange ein Teil an ist) mit Ledger-Eintrag an und startet den Wächter getrennt vom Aufrufer (`N/desktop/os/shared/control.rs:88-143`). Deaktivieren schreibt `disabled`; der Wächter endet binnen einer Sekunde und beendet seine Flächen, sonst endet er nach sieben Sekunden per PID nach Abbildprüfung (`N/desktop/os/shared/launch.rs:96-131`); danach entfernt se jede feste Spur und jeden Ledger-Eintrag, auch ohne Ledger (`N/desktop/os/shared/control.rs:201-245`). Entfernen löscht zusätzlich den Ordner, außer eine Spur ließ sich nicht entfernen: dann bleibt das Ledger für einen neuen Versuch. Fremde Werte unter denselben Schlüsseln bleiben unberührt; ein Ledger-Eintrag mit fremdem Namen wird abgelehnt.

`residue` im Status listet nach `disable` und `remove` jede noch gefundene Spur (Registrierungen und Laufzeitdateien); die Sprungliste ist nicht auslesbar und gilt als entfernt, wenn `DeleteList` Erfolg meldet (Annahme A6). Die Deinstallation ruft `se desktop remove` und danach `--unregister`, das jede feste Spur noch einmal entfernt und ein noch aktiviertes Modul deaktiviert hinterlässt (`N/desktop/os/shared/control.rs:379-396`, `N/shell_register/os/windows.rs:232-238`). Der S2-Gerätebaum bleibt bei Deaktivieren und Entfernen unberührt.

## 6 Erreichbarkeit ab Anmeldung (V11)

se Desktop startet den Hintergrunddienst nie selbst. Der Wächter liest den Share-Zustand nur über `share_worker_peek()` (Probe mit `Ping`, kein `ensure_worker_ready`, keine UI-Ereignisse; `N/daemon/os/shared/ipc_peek_client.rs:31-67`) im Takt von 900 ms; ohne Dienst zeigt er die gespeicherten Profile mit `share_state = profiles` (`N/desktop/os/shared/model_feed.rs:132-176`). Erst die Handlung „Verbinden“ auf einer Fläche startet den Dienst wie das se-Fenster. Die Einstellungen nennen dazu den Satz aus C10 mit dem bestehenden Autostart-Schalter des Hintergrunddienstes (`N/app/core/desktop_settings_ui.rs:23-24`, `:302-328`).

## 7 Ausnahmen mit Grund

| Gegenstand | Nur in | Grund |
|---|---|---|
| se Desktop | Windows, Linux | Android bekommt den se Launcher (eigenes Modul, C5.2) |
| `nav_pane`, `jump_list` | Windows | Explorer-Funktionen; unter Linux `supported: false`, Grund „nur unter Windows (Explorer)“ |
| `--recycle-bin`, `--this-pc` | Windows | „Papierkorb“ und „Dieser PC“ gibt es nur auf dem Windows-Desktop; sonst Exitcode 1 mit Grund |
| `--foreign-desktop-icons` | Linux | GNOME-Desktopsymbole (DING) gibt es nur unter GNOME; sonst Exitcode 1 mit Grund |
| Verbinden (O7) ohne CLI-Befehl | Fläche, Launcher | jeder `se share`-Befehl startet den Dienst bereits (Bestand) |
| Gerät öffnen (O9) ohne CLI-Befehl | GUI | ein Fenster zu öffnen ist eine GUI-Handlung; die Dateibefehle von `se` bleiben unverändert |
| Ablegen von Dateien anderer Programme auf die Fläche, Herausziehen in andere Programme (`wl_data_device`-Ziehen) | Windows, Linux X11 (je nach DSK-M6) | Unter dem Layer-Backend nicht umgesetzt: ein Ziehen verlangt Zeigerverfolgung und die `drag_and_drop`-Ereignisse, die das Backend nicht bedient. Kopieren, Ausschneiden und Einfügen von Dateien über die Zwischenablage gibt es. `docs/TODO.md`: `DESKTOP-WAYLAND-DND` |
| Eingabemethoden (IME) | Windows, Linux X11 | Das Layer-Backend liest die Tasten selbst über `wl_keyboard` und xkbcommon; `text-input` wird nicht gesprochen. Tote Tasten und Compose-Folgen kommen unzusammengesetzt an. `docs/TODO.md`: `DESKTOP-LAYER-IME` |
| Explorer-Aktionen („Weitere Optionen anzeigen“, Herausziehen über `DoDragDrop`) | Windows | gibt es nur dort; auf Wayland meldet der Host „Explorer-Aktionen gibt es nur unter Windows“ |
| Software-Ersatz für die Grafik | – | Das Layer-Backend zeichnet mit wgpu (Vulkan oder GL) und hat keinen CPU-Rasterisierer; ohne Adapter meldet es `se Desktop: Layer-Backend: keine Grafikbeschleunigung (<Grund>)` und endet mit Exitcode 1, die Fläche bleibt aus. `docs/TODO.md`: `DESKTOP-LAYER-SOFTWARE` (später) |
| `Monitor.rect` logisch | Layer-Sitzung | Unter Wayland sind die Rechtecke der Monitore logische Koordinaten des Compositors (nicht physische Pixel wie unter Windows und X11); nur der Wächter der Layer-Sitzung und die Ansicht rechnen damit |
| Startmenü-Anheften, Widgets-Board, Ordner-Öffnen umleiten, Einbetten in den Explorer-Desktop | – | nicht möglich oder No-Go (C1.1, C1.3, C1.6) |

## 8 Offene Punkte

- `--open-page share-device:<kontakt>` und `share-room:<raum>` öffnen in U15 das Fenster „Geräte & Freigaben“ auf dem Reiter „Geräte“ bzw. „Räume“, nicht die Detailseite: Die Seitenwahl liegt im Modul `N/app/core/share_pages_ui.rs` (privat im Share-Modul) und braucht dort eine Funktion für eine Seite von außen.
- Monitorschlüssel unter Windows vorläufig aus dem GDI-Gerätenamen (siehe 3).
- Linux-Sitzungsprüfungen (U34: `layer_shell_version` in `session_facts`, `wayland_monitors()`-Aufruf ist da), Flächen je RandR-Monitor und GNOME-Erweiterung folgen mit DSK-M6 und DSK-M8. Das Layer-Backend (DSK-M7) ist geschrieben und wird erst wirksam, wenn die Sitzungsprüfung `layer_shell_version` liefert.
- Layer-Backend: `DESKTOP-LAYER-SOFTWARE` (kein CPU-Rasterisierer), `DESKTOP-LAYER-IME`, `DESKTOP-WAYLAND-DND` (siehe Abschnitt 7); der Systemhintergrund (`platform::system_wallpaper`) und `take_wallpaper_change` kommen von U34 (KDE, GNOME), bis dahin zeichnet die Fläche ihre Hintergrundfarbe; die Zwischenablage schreibt und liest über den Sitz der letzten Eingabe auf der Fläche (sonst über den ersten Sitz); ohne bisherige Eingabe auf der Fläche (keine Serie) gibt es kein Kopieren.
- Annahme A41 (die Desktop-Ansicht `ShareSnapshotPeek` startet im Startgrund `drive-restore` keinen Share-Host; Statusbefehle mit `ShareSnapshot` heben die Einschränkung auf) prüfen `desktop_task_peek_does_not_start_share_host` und `drive_task_status_command_promotes_restricted_daemon`.
