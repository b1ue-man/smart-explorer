# Oberflächen der Laufwerke

Stand: 2026-10-17. Lesestand: Commit `f45d9580` mit den Änderungen des Pakets U06 (DRV-M3) des Plans `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, am Code nachgelesen, Belege als `Datei:Zeile`. Angelegt von U06 (C6, W4); gepflegt von U12, U14, U17 und U23 im Commit ihrer Oberflächenänderung; vervollständigt von DRV-M9 (U42). Nichts davon ist gebaut oder ausgeführt; das prüft die eine Stapel-Suite.

Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`, `<AD>` = Datenordner der App (`support_dirs::app_data_dir()`).

## 1 Oberflächen

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se drive runtime`, `install-runtime`, `mount`, `list`, `unmount`, `retry`, `set` | `N/cli/drive.rs:28-44`, Optionen `N/cli/drive_options.rs`, `N/cli/drive_set.rs` |
| Desktop | Fenster „Remote als Laufwerk“ und „Smart-Explorer-Laufwerke“ (egui) | `N/app/core/mount_ui_draft.rs:130-224`, `N/app/core/mount_ui_location.rs`, `N/app/core/mount_ui.rs:180-299` |
| Android | keine Laufwerke; der Dokumentanbieter folgt mit DRV-M8 (Abschnitt 7) | – |
| Bridge | – (DRV-M8: `docs.*`) | – |
| IPC | Loopback-IPC GUI/CLI ↔ Hintergrunddienst: `StartMount`, `StopMount`, `ListMounts`, `RetryMount` (Bestand) und `Drive { token, req }` mit `DriveIpcRequest` (DRV-M3) | `N/daemon/os/shared/ipc_protocol.rs` (`IpcRequest`), `N/daemon/os/shared/ipc_protocol_drives.rs:28-36`; Bedienung `ipc_protocol_drives.rs:48-70` |
| MCP | Werkzeug `se` führt jeden `se drive`-Befehl aus | `N/cli/mcp/catalog.rs` (Beispiel `["drive","mount","@prod:/srv","--letter","M"]`) |
| Anmeldung | Anmeldeeintrag der Wiederherstellung `<Programm> --sync-daemon --drive-restore` | `N/daemon/os/shared/drive_autostart.rs`, `N/autostart/os/linux_os.rs:188-235`, `N/autostart/os/windows.rs:209-253` |
| Ablage | `<AD>/mounts/registry.json` (Form v0.5.182) und `<AD>/mounts/registry-2.json` | `N/daemon/os/shared/mount_registry.rs`, `N/daemon/os/shared/mount_registry_ext.rs` |

## 2 Laufwerk einhängen, aushängen, Status

„–“ = nicht erreichbar; „Ausnahme“ verweist auf Abschnitt 9.

| Fähigkeit | CLI (Text / JSON / Vervollständigung) | Desktop | Android | IPC | MCP |
|---|---|---|---|---|---|
| Einhängen mit Ziel, Buchstabe, Schreibzugriff, Wurzelvertrauen, Metadatentiefe, Cache, Systemlaufzeit, Name | `se drive mount <TARGET>` (`N/cli/drive.rs:120-165`, Optionen `N/cli/drive_options.rs:5-55`); Text `drive\t<id>\t<auto\|X:>\t<Modus>\t<Status>\t<Name>` (`drive.rs:425-442`); `--json` = `MountSnapshot` | „Remote als Laufwerk“ → „Einbinden“ (`N/app/core/mount_ui_draft.rs:139`, `:199`, `:226-276`) | Ausnahme | `StartMount` | `se` |
| Liste | `se drive list [--json]` (`drive.rs:167-182`); leer: `drives\t0` | „Smart-Explorer-Laufwerke“ (`N/app/core/mount_ui.rs:188`, Abfrage alle 2 s `:162-169`) | Ausnahme | `ListMounts` | `se` |
| Aushängen | `se drive unmount <SELEKTOR>` (`drive.rs:184-189`); Selektor-Vervollständigung aus der Registry (`drive.rs:60-66`, `N/cli/drive_set.rs:64-80`) | „Auswerfen“, „Entfernen“ (`mount_ui.rs:263`, `:248`) | Ausnahme | `StopMount` | `se` |
| Erneut versuchen | `se drive retry <SELEKTOR>` (`drive.rs:191-197`), Vervollständigung wie oben | „Erneut versuchen“ (`mount_ui.rs:238`) | Ausnahme | `RetryMount` | `se` |
| Status | `unmounted`, `mounting`, `mounted:<X:>`, `mounted:<Pfad>` (Ordner-Laufwerke, `MountStatus::MountedAt`), `unmounting`, `runtime-unavailable:…`, `conflict:…`, `failed:…` (`drive.rs:444-458`, `N/mount/core/mount_status.rs:17-21`); Exitcodes unverändert (`drive.rs:461-472`) | Statustext je Laufwerk; `MountedAt` braucht die Folgezeile in `N/app/core/mount_ui_helpers.rs` (Abschnitt 10) | Ausnahme | `ListMounts`, `MountSnapshot` | `se` |

Plattformen: Windows über Dokany (Bestand). Linux bindet ab DRV-M4 (U12) ein; bis dahin meldet `drive_mount_supported()` unter Linux `false` (`N/mount/mod.rs:313-315`).

## 3 Optionen und „Beim Anmelden automatisch einbinden“

| Fähigkeit | CLI (Text / JSON / Vervollständigung) | Desktop | Android | IPC | MCP |
|---|---|---|---|---|---|
| Option beim Einhängen setzen | `se drive mount … --auto-remount` (`N/cli/drive_options.rs:48-52`, übernommen `N/cli/drive.rs:159`) | Kontrollkästchen „Beim Anmelden automatisch einbinden“ im Fenster „Remote als Laufwerk“ (`N/app/core/mount_ui_location.rs:60-71`, übernommen `N/app/core/mount_ui_draft.rs:262`) | Ausnahme | `StartMount` mit `config.auto_remount` | `se` |
| Option eines verwalteten Laufwerks ändern | `se drive set <SELEKTOR> --auto-remount on\|off [--json]` (`N/cli/drive_set.rs:14-54`): Text `drive\t<id>\tauto-remount=<on\|off>`, JSON `MountSnapshot`; Vervollständigung von Selektor und `on`/`off` (`drive_set.rs:16-27`, `:64-80`) | Kontrollkästchen je Laufwerk im Fenster „Smart-Explorer-Laufwerke“ (`N/app/core/mount_ui.rs:217-231`, Aktion `:284-287`, Rückmeldung `:128-142`) | Ausnahme | `Drive { req: SetOptions { id, auto_remount } }` (`ipc_protocol_drives.rs:32`, Bedienung `:55-60`; Daemon `N/daemon/os/shared/mount_restore.rs:146-173`) | `se` |
| Option anzeigen | `se drive list --json`: `config.auto_remount` nur, wenn eingeschaltet; Text unverändert | Zustand des Kontrollkästchens | Ausnahme | `ListMounts` | `se` |
| Wiederherstellung nach Anmeldung und Neustart des Dienstes | – (Anmeldeeintrag statt Befehl, Ausnahme) | – (folgt der Option, Ausnahme) | Ausnahme | Anmeldeeintrag → `Drive { req: Restore }` an einen schon laufenden Dienst (`ipc_protocol_drives.rs:94-114`) | – |

Wiederherstellung:
- Anmeldeeintrag: unter Linux `~/.config/autostart/smart-explorer-drives.desktop` (`N/autostart/os/linux_os.rs:6`, `:213-235`), unter Windows der Run-Wert `SmartExplorerDrives` (`N/autostart/os/windows.rs:3`, `:32-37`, `:220-249`). Beide starten `<Programm> --sync-daemon --drive-restore`; unter Windows ist das Programm die GUI-Datei neben `se.exe`.
- Der Eintrag besteht genau dann, wenn ein Laufwerk `auto_remount` hat oder der Gerätebaum besteht (`N/daemon/os/shared/drive_autostart.rs:16-19`, `N/mount/core/types.rs:317-323`). Abgleich nach jeder Registry-Änderung (`N/daemon/os/shared/mount_registry.rs:59`, `:65`) und bei jedem Start des Dienstes (`N/daemon/os/shared/run_start_reason.rs:122-134`). Ein vom Desktop abgeschalteter Eintrag bleibt abgeschaltet (`linux_os.rs:224-235`; Windows hält den Schalter getrennt in `StartupApproved\Run`).
- Jeder Start des Dienstes bindet diese Laufwerke nach der Cache-Prüfung ein (`N/daemon/os/shared/mount_manager.rs:77-109`, `mount_restore.rs:62-129`), über den Startpfad von „Erneut versuchen“, ohne die Registry neu zu schreiben.
- Ein Dienst, der nur für die Wiederherstellung startet, startet weder Share-Host noch Exec-Host und plant keine Sync-Läufe, solange der Autostart des Hintergrund-Syncs aus oder vom Desktop abgeschaltet ist (Entscheidung S2 K7; `run_start_reason.rs:1-19`, `:68-118`). Laufwerke anderer Geräte warten dann mit dem Status „Wartet auf Smart Explorer Share: …“ (`mount_restore.rs:27`) bis zur ersten Anfrage der App, von `se` oder `se mcp` oder bis der Sync-Autostart eingeschaltet wird.
- Ein Stopp des Dienstes und ein Update-Stopp (Exit 75, V8) behalten die Registry-Einträge dieser Laufwerke (`mount_restore.rs:31-60`, `mount_manager.rs:367-384`, `N/daemon/os/shared/mount_manager_state.rs:9-43`); Auswerfen und Entfernen vergessen jedes Laufwerk wie bisher.

Ablage (Strang S2 C4):
- `registry.json` behält die Form von v0.5.182; `location` und `auto_remount` stehen nur in Einträgen, die sie gesetzt haben (`N/mount/core/types.rs:212-219`), daher bleiben unberührte Einträge byte-gleich und v0.5.182 liest die Datei weiter.
- `MountSource::Devices` und `MountSource::Vault { site_id }` stehen nur in `registry-2.json` (`N/mount/core/mount_source.rs:84-92`, `N/daemon/os/shared/mount_registry_ext.rs:1-11`). Einträge einer Art, die erst eine spätere Version kennt, bleiben dort unverändert erhalten (`mount_registry_ext.rs:46-106`).

## 4 Einhängeort und Dateimanager-Einbindung

| Fähigkeit | CLI | Desktop | Android | IPC | MCP |
|---|---|---|---|---|---|
| Laufwerksbuchstabe (Windows) | `--letter auto\|X` (`N/cli/drive_options.rs:9-15`) | Auswahl „Laufwerksbuchstabe“ `D`–`Z` (`N/app/core/mount_ui_location.rs:73-96`, unverändert aus `mount_ui_draft.rs` ausgegliedert) | Ausnahme | `StartMount` (`config.drive`) | `se` |
| Ordner als Einhängeort (Linux) | `--mount-point` folgt mit DRV-M4 (U12) | Feld „Einhängeort“: „Standard (~/SmartExplorer)“ oder „Eigener Ordner“ mit Formprüfung (`mount_ui_location.rs:98-116`, `:33-56`); Anzeige in „Laufwerkdetails“ (`N/app/core/mount_ui.rs:212-214`); erreichbar, sobald DRV-M4 Linux-Laufwerke einhängt | Ausnahme | `StartMount` (`config.location` = `MountLocation::Directory { path }`, `N/mount/core/mount_location.rs:18-22`, Prüfung `:38-70`) | `se` |
| GTK-Lesezeichen, KDE-Orte, „Im Dateimanager öffnen“, `se drive open` | folgt mit DRV-M5 (U17) | folgt mit DRV-M5 (U17) | Ausnahme | – | – |

## 5 Gerätebaum „Verbundene Geräte als Ordner“

Folgt mit DRV-M6 (U17): `se drive devices …`, Einstellungen → Integration, IPC-Wache. Vorbereitet in DRV-M3: die Quelle `MountSource::Devices` (`N/mount/core/mount_source.rs:49-50`), gespeichert nur in `registry-2.json`; der Gerätebaum zählt für den Anmeldeeintrag (`N/mount/core/types.rs:317-323`) und wartet im eingeschränkten Dienst auf Share (`mount_source.rs:98-104`).

## 6 Tresor-Laufwerke

Folgt mit DRV-M7 (U14): `se drive mount vault://<SITE>`, `requires-unlock`, Sperren. Vorbereitet in DRV-M3: die Quelle `MountSource::Vault { site_id }` (`N/mount/core/mount_source.rs:51-55`), gespeichert nur in `registry-2.json`, und `MountStatus::MountedAt { path }` für die Einhängepfade in `list_mounts` (V2).

## 7 Android-Dokumentanbieter

Folgt mit DRV-M8 (U23): `docs.*`, Einstellungen „Ordner für Apps bereitstellen“.

## 8 Laufzeit

| Fähigkeit | CLI | Desktop | Android | IPC | MCP |
|---|---|---|---|---|---|
| Laufzeit prüfen | `se drive runtime [--json]` (`N/cli/drive.rs:31`, `:98-118`), Windows/Dokany | „Dokany-Laufzeit / Hilfe“ (`N/app/core/mount_ui.rs:276`) | Ausnahme | – | `se` |
| Laufzeit installieren | `se drive install-runtime [--json]` (`drive.rs:33`, `:83-96`) | Schaltfläche in „Dokany-Laufzeit / Hilfe“ (`N/app/core/mount_runtime_ui.rs`) | Ausnahme | – | `se` |
| FUSE-Laufzeit, Kernel-Zeitlimit | folgt mit DRV-M4 (U12) | folgt mit DRV-M4 (U12) | Ausnahme | – | – |

## 9 Ausnahmen mit Grund

- Android: keine Laufwerke und keine CLI (`N/lib.rs`: `cli` nur ohne `target_os = "android"`); Ordner für andere Apps stellt der Dokumentanbieter bereit (DRV-M8).
- MCP: kein eigenes Laufwerkswerkzeug; das Werkzeug `se` führt jeden `se drive`-Befehl aus.
- Anmeldeeintrag: keine eigene Bedienung in CLI, Desktop oder MCP; er folgt den Laufwerksoptionen (S2 C2), damit kein Eintrag ohne Laufwerk zurückbleibt.
- Windows: kein Ordner als Einhängeort, nur Laufwerksbuchstaben (S2 C1).
- Eingeschränkter Dienst nach der Anmeldung: Laufwerke anderer Geräte und der Gerätebaum warten auf den ersten anderen Auslöser, weil ein Dienst, der nur für die Wiederherstellung startet, dieses Gerät nicht über Share erreichbar machen darf (S2 K7, V11).
- Auswerfen vergisst das Laufwerk samt Option (Verhalten von v0.5.182); wer es nach der nächsten Anmeldung wieder haben will, lässt es eingehängt.

## 10 Offene Punkte

- Zum Kompilieren nötig, außerhalb von U06: neue `match`-Zweige für `MountStatus::MountedAt` in `N/app/core/mount_ui_helpers.rs` (`status_label`) und `N/daemon/os/shared/mount_manager_host.rs` (`host_transition_allowed`, `validate_host_status`), für `MountSource::Devices`/`Vault` in `N/daemon/os/shared/mount_source.rs` (`resolve`) und `N/app/core/analytics_mounts.rs` (`open`).
- Linux-Einhängen, `--mount-point`, `mounted:<Pfad>` in der Praxis: DRV-M4 (U12). Dateimanager: DRV-M5 (U17). Gerätebaum: DRV-M6 (U17). Tresore: DRV-M7 (U14). Android: DRV-M8 (U23).
- Ende-zu-Ende mit echtem Dokany und FUSE (`drive_task_auto_remount_after_daemon_restart`) läuft nur in der Stapel-Suite (Stufen `windows-core: drive-cli`, `linux-root: drive-fuse`).
