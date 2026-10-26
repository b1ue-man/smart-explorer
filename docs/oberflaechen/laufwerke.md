# Oberflächen der Laufwerke

Stand: 2026-10-21 (Lauf U42, DRV-M9). Lesestand: Stand `946b9205` mit den Änderungen der Pakete U06 (`a6107a70`), U12 (`fc59b6f3`), U14 (`5267e500`), U17 (`0b11d3e6`) und U23 (`94240017`) des Plans `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, am Code nachgelesen, Belege als `Datei:Zeile`. Angelegt von U06 (C6, W4); gepflegt von U12, U14, U17 und U23 im Commit ihrer Oberflächenänderung; vervollständigt von DRV-M9 (U42). Diese Fassung ist nicht gebaut und nicht ausgeführt; das prüft die eine Stapel-Suite.

Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`, `<AD>` = Datenordner der App (`support_dirs::app_data_dir()`).

## 1 Oberflächen

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se drive runtime`, `install-runtime`, `mount`, `list`, `unmount`, `retry`, `set`, `open` (DRV-M5), `devices status\|watch\|on\|off` (DRV-M6) | `N/cli/drive.rs`, Optionen `N/cli/drive_options.rs`, `N/cli/drive_set.rs`, `N/cli/drive_open.rs`, `N/cli/drive_devices.rs`, Ausgabe `N/cli/drive_print.rs` |
| Desktop | Fenster „Remote als Laufwerk“ und „Smart-Explorer-Laufwerke“ (egui); Abschnitt „Verbundene Geräte als Ordner“ in Einstellungen → Integration (DRV-M6) | `N/app/core/mount_ui_draft.rs:130-224`, `N/app/core/mount_ui_location.rs`, `N/app/core/mount_ui.rs`, `N/app/core/mount_ui_open.rs`, `N/app/core/settings_devices_ui.rs` |
| Android | keine Laufwerke; Dokumentanbieter „Ordner für Apps bereitstellen“ (DRV-M8, Abschnitt 7): Einstellungen → „Ordner für Apps bereitstellen“ | `A/system/documents/SeDocumentsProvider.kt`, `A/ui/settings/DocumentsProviderSection.kt` |
| Bridge | `docs.roots`, `docs.document`, `docs.children`, `docs.create`, `docs.rename`, `docs.delete`, `docs.settings.get`, `docs.settings.set`, `docs.pending`, `docs.retryPending`; Ereignis `docs.changed`; JNI `docOpen`, `docRead`, `docWrite`, `docSize`, `docFsync`, `docRelease` (DRV-M8) | `N/mobile/os/shared/domains/docs.rs`, `docs_io.rs`, `native/android-bridge/src/io.rs` |
| IPC | Loopback-IPC GUI/CLI ↔ Hintergrunddienst: `StartMount`, `StopMount`, `ListMounts`, `RetryMount` (Bestand) und `Drive { token, req }` mit `DriveIpcRequest` (DRV-M3; DRV-M6: `DevicesStatus`, `DevicesEnable`, `DevicesDisable`, `DevicesWatch`, Antwort `IpcResponse::Devices`) | `N/daemon/os/shared/ipc_protocol.rs` (`IpcRequest`), `N/daemon/os/shared/ipc_protocol_drives.rs:28-36`; Bedienung `ipc_protocol_drives.rs:48-70` |
| MCP | Werkzeug `se` führt jeden `se drive`-Befehl aus | `N/cli/mcp/catalog.rs` (Beispiel `["drive","mount","@prod:/srv","--letter","M"]`) |
| Anmeldung | Anmeldeeintrag der Wiederherstellung `<Programm> --sync-daemon --drive-restore` | `N/daemon/os/shared/drive_autostart.rs`, `N/autostart/os/linux_os.rs:188-235`, `N/autostart/os/windows.rs:209-253` |
| Ablage | `<AD>/mounts/registry.json` (Form v0.5.182) und `<AD>/mounts/registry-2.json` | `N/daemon/os/shared/mount_registry.rs`, `N/daemon/os/shared/mount_registry_ext.rs` |

## 2 Laufwerk einhängen, aushängen, Status

„–“ = nicht erreichbar; „Ausnahme“ verweist auf Abschnitt 9.

| Fähigkeit | CLI (Text / JSON / Vervollständigung) | Desktop | Android | IPC | MCP |
|---|---|---|---|---|---|
| Einhängen mit Ziel, Buchstabe, Schreibzugriff, Wurzelvertrauen, Metadatentiefe, Cache, Systemlaufzeit, Name | `se drive mount <TARGET>` (`N/cli/drive.rs:120-165`, Optionen `N/cli/drive_options.rs:5-55`); Text `drive\t<id>\t<auto\|X:>\t<Modus>\t<Status>\t<Name>` (`drive.rs:425-442`); `--json` = `MountSnapshot` | „Remote als Laufwerk“ → „Einbinden“ (`N/app/core/mount_ui_draft.rs:139`, `:199`, `:226-276`) | Ausnahme | `StartMount` | `se` |
| Liste | `se drive list [--json]` (`drive.rs:167-182`); leer: `drives\t0` | „Smart-Explorer-Laufwerke“ (`N/app/core/mount_ui.rs:188`, Abfrage alle 2 s `:162-169`) | Ausnahme | `ListMounts` | `se` |
| Aushängen | `se drive unmount <SELEKTOR> [--force]` (`drive.rs:184-189`; `--force` nur unter Linux: auch belegte oder hängende Laufwerke, offene Dateien verlieren ihre Verbindung, `N/cli/drive_options.rs:78-88`); Selektor-Vervollständigung aus der Registry (`drive.rs:60-66`, `N/cli/drive_set.rs:64-80`) | „Auswerfen“, „Entfernen“ (`mount_ui.rs:263`, `:248`) | Ausnahme | `StopMount` | `se` |
| Erneut versuchen | `se drive retry <SELEKTOR>` (`drive.rs:191-197`), Vervollständigung wie oben | „Erneut versuchen“ (`mount_ui.rs:238`) | Ausnahme | `RetryMount` | `se` |
| Status | `unmounted`, `mounting`, `mounted:<X:>`, `mounted:<Pfad>` (Ordner-Laufwerke, `MountStatus::MountedAt`), `unmounting`, `runtime-unavailable:…`, `conflict:…`, `failed:…` (`N/cli/drive_print.rs:66-80`, `N/mount/core/mount_status.rs:17-21`) | Statustext je Laufwerk; `MountedAt` braucht die Folgezeile in `N/app/core/mount_ui_helpers.rs` (Abschnitt 10) | Ausnahme | `ListMounts`, `MountSnapshot` | `se` |

Plattformen: Windows über Dokany 2.3.1.1000 und Linux über FUSE. `drive_mount_supported()` ist unter Windows und Linux `true` (`N/mount/mod.rs:361-364`). Linux nutzt `fuser =0.18.0` (`native/Cargo.toml:326`), Einhängeort ist standardmäßig `~/SmartExplorer/<Name>` (`N/cli/drive_options.rs:24`).

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
| Ordner als Einhängeort (Linux) | `--mount-point PFAD` (`N/cli/drive_options.rs:24`; unter Windows Exitcode 2 mit „Einhängeorte gibt es nur unter Linux; nutze --letter“, `N/cli/drive.rs:165-167`) | Feld „Einhängeort“: „Standard (~/SmartExplorer)“ oder „Eigener Ordner“ mit Formprüfung (`mount_ui_location.rs:98-116`, `:33-56`); Anzeige in „Laufwerkdetails“ (`N/app/core/mount_ui.rs:212-214`) | Ausnahme | `StartMount` (`config.location` = `MountLocation::Directory { path }`, `N/mount/core/mount_location.rs:18-22`, Prüfung `:38-70`) | `se` |
| GTK-Lesezeichen (Linux, `$XDG_CONFIG_HOME/gtk-3.0/bookmarks`) | – (der Host legt sie beim Einhängen an und entfernt sie beim Aushängen; der Dienst entfernt sie nach einem Hostabsturz) | – | Ausnahme | – | – |
| KDE-Orte (Linux, `$XDG_DATA_HOME/user-places.xbel`) | – (wie oben) | – | Ausnahme | – | – |
| „Im Dateimanager öffnen“ | `se drive open <SELEKTOR> [--json]`: Text `opened\t<Pfad>\t<Methode>`, JSON `{"opened":"<Pfad>","method":"filemanager1\|xdg-open\|explorer"}`; Selektor wie bei `unmount` (zusätzlich das Wort `devices`), Vervollständigung wie bei `retry`; ein Laufwerk, das nicht eingehängt ist, meldet `se: Das Laufwerk ist nicht eingehängt` mit Exitcode 1 (`N/cli/drive_open.rs`) | Knopf „Im Dateimanager öffnen“ („Im Explorer öffnen“ unter Windows) neben „Auswerfen“ je eingehängter Zeile (`N/app/core/mount_ui.rs`, `mount_ui_open.rs`) und im Abschnitt „Verbundene Geräte als Ordner“ | Ausnahme | – | `se` |

Dateimanager-Einbindung unter Linux (DRV-M5, Dateien `N/mount/os/linux_os/bookmarks*.rs`, `file_manager_open.rs`):
- **Regel „GLib zeigt den Einhängeort selbst“** (`glib_shows_mount`): Liegt der kanonische Einhängepfad unter `<Home>/` und enthält der ganze Pfad die Zeichenfolge `/.` nicht, zeigen GLib, GVfs, Nautilus, Nemo und Thunar das Laufwerk von allein (Strang S2 NR N8). Dann entsteht kein GTK-Lesezeichen. Jeder andere Ort (Tresor-Laufwerke unter `$XDG_RUNTIME_DIR`, ein Ordner mit `--mount-point` außerhalb von Home oder mit einem versteckten Teil) bekommt eine Zeile `file://<Pfad> <Bezeichnung>` am Ende von `gtk-3.0/bookmarks`. Das Entfernen löscht jede Zeile mit demselben URI, auch eine vom Nutzer umbenannte, und lässt alle anderen Zeilen byte-gleich.
- **KDE-Orte:** Jedes se-Laufwerk bekommt ein `bookmark`-Element in `user-places.xbel` mit der Metadaten-Eigentümerkennung `urn:smart-explorer:places` und der Mount-ID. Se ändert und entfernt nur Elemente mit dieser Kennung; fremde Elemente, auch mit gleichem `href`, bleiben unberührt. Eine Datei, die kein wohlgeformtes XML ist, wird nicht verändert.
- **Schreiben:** Beide Dateien werden einmal gelesen und über eine Hilfsdatei im selben Ordner (`rename`) ersetzt; eine Verknüpfung auf die Datei bleibt erhalten. Weder GTK noch KDE sperren die Dateien. Ein anderes Programm, das zwischen Lesen und Ersetzen schreibt, verliert seine Änderung (bekannte Grenze).
- **Anlegen, Entfernen, Aufräumen:** Der Host legt die Einträge an, sobald die FUSE-Sitzung läuft, und entfernt sie nach dem Aushängen vor dem Sperren des Ordners. Der Dienst entfernt sie zusätzlich in `clean_after_host`, bedingungslos und wiederholbar, wenn ein Host ohne Aushängen endete. Ein Fehler dabei macht das Einhängen nie zum Fehlschlag (Host: Zeile auf stderr, Dienst: Protokoll).
- **Öffnen:** `show_folders` ruft `org.freedesktop.FileManager1.ShowFolders` auf dem Sitzungsbus (höchstens 25 s) und fällt bei jedem Fehler auf `xdg-open <Ordner>` zurück; die Oberflächen rufen es in einem Hintergrund-Thread.
- **Annahmen (nicht geprüft, kein Dolphin im Prüflauf):** A8: Dolphin zeigt `fuse.smartexplorer`-Einhängungen nicht von allein, darum bekommt auch ein Laufwerk unter Home einen KDE-Ort (bei Irrtum erscheint er doppelt). KDE akzeptiert für `ID` jede eindeutige Zeichenfolge (`se-<Mount-ID>`), und `folder-remote` ist ein vorhandenes Symbol (bei Irrtum zeigt Dolphin den Eintrag ohne Symbol oder vergibt eine eigene ID). Handtest: `docs/TODO.md` (U43).

## 5 Gerätebaum „Verbundene Geräte als Ordner“

Stand DRV-M6 (U17), am Code nachgelesen; im Quelltext committet (`0b11d3e6`, Registrierung `f83d1d94`), nicht ausgeführt. Ein Laufwerk, `se-devices`, zeigt jedes gekoppelte Gerät als Ordner: Windows als Laufwerk „Smart Explorer Geräte“, Linux unter `~/SmartExplorer/Geräte`.

| Fähigkeit | CLI (Text / JSON / Vervollständigung) | Desktop | Android | IPC | MCP |
|---|---|---|---|---|---|
| Zustand und Geräte | `se drive devices status [--json]`: eine Zeile `devices\t<on\|off>\t<state>\t<mounted\|unmounted>\t<Wurzelpfad\|->` und je Gerät `device\t<key>\t<Name>\t<direct\|room:<Raum>>\t<online\|offline>\t<Pfad\|->`; JSON das Statusobjekt (Felder unten) (`N/cli/drive_devices.rs`) | Abschnitt „Verbundene Geräte als Ordner“ in Einstellungen → Integration: Schalter, Buchstabe bzw. Einhängeort, „Zustand: …“, Knopf „Im Dateimanager öffnen“, Text zu wartenden Änderungen, Erklärung, fehlende Laufzeit; Abfrage alle 2 s, nur solange die Seite offen ist (`N/app/core/settings_devices_ui.rs`) | Ausnahme (Dokumentanbieter, DRV-M8) | `Drive { req: DevicesStatus }` → `IpcResponse::Devices { status }` | `se` |
| Auf Änderung warten | `se drive devices watch [--since N] [--json]`: genau eine Antwort nach einer Änderung der Revision oder nach 30 s | – | Ausnahme | `Drive { req: DevicesWatch { since } }` (zählt nicht als Anfrage, die einen eingeschränkten Dienst befördert; se Desktop liest sie mit 35 s Lesefrist über `ipc_peek_client.rs`) | `se` |
| Einschalten | `se drive devices on [--letter auto\|X] [--mount-point PFAD] [--json]`; `--letter` unter Linux und `--mount-point` unter Windows sind Parameterfehler (Exitcode 2); Vervollständigung von `--letter`: `auto`, `D`–`Z` (Windows) | Kontrollkästchen; Buchstabe (Windows) oder Einhängeort (Linux) gelten beim nächsten Einschalten | Ausnahme | `Drive { req: DevicesEnable { letter, mount_point } }` | `se` |
| Ausschalten | `se drive devices off [--json]`; `se drive unmount devices` (oder `se-devices`) und „Auswerfen“ im Fenster „Smart-Explorer-Laufwerke“ rufen dasselbe Ausschalten auf | Kontrollkästchen | Ausnahme | `Drive { req: DevicesDisable }` | `se` |
| Öffnen | `se drive open devices` | Knopf „Im Dateimanager öffnen“ | Ausnahme | – | `se` |

Statusobjekt (Vertrag V4): `{"enabled","state","mounted","root_path"?,"letter"?,"mount_point"?,"revision","devices":[{"key","name","kind","room"?,"online","path"?}],"pending"}`; `state` ist `off`, `mounting`, `mounted`, `unmounting`, `waiting` (Dienst ohne Share), `failed` oder `stopped`. `name` ist der Ordnername, `room` der Raumname, `path` der Ordner des Geräts, solange das Laufwerk eingehängt ist; `pending` die Zahl der Geräte mit nicht gespeicherten Änderungen. Nicht gesetzte Werte fehlen.

Aufbau und Verhalten:
- **Ebenen:** Wurzel: ein Ordner je Direktkontakt und `Räume` (nur wenn ein Raum mindestens ein Mitgliedsgerät hat); `Räume/<Raum>/<Gerät>`; im Geräteordner je Freigabe `<Freigabename>` und `Verbindungen/<Name>`. Die Wurzel „Alle freigegebenen Bereiche“ gehört nicht dazu. Die Ebenen bis zum Geräteordner und `Verbindungen` sind nur lesbar: Anlegen, Umbenennen, Löschen und Schreiben liefern `PermissionDenied`.
- **Namen:** Zeichenersatz je Dialekt, Länge 255, leer = `Gerät`; ungültige Namen = `Gerät <6 Hex>`; ein Gerät namens `Räume` heißt `Räume (Gerät)`; gleiche Namen (Windows ohne Rücksicht auf Groß-/Kleinschreibung) bekommen alle ` (<6 Hex>)` aus dem SHA-256 des Schlüssels. Reihenfolge und Namen hängen nicht von der Reihenfolge der Eingabe ab (`N/mount/core/device_tree_names.rs`).
- **Schlüssel (V4):** `direct~<contact_id>` bzw. `room~<room_id>~<device_id>`, jedes Byte außerhalb `[A-Za-z0-9._-]` als `%XX` in Großbuchstaben; dieselbe Funktion (`mount::device_key`) benutzen die Kacheln von se Desktop.
- **Rechte:** Eine Freigabe eines Direktkontakts ist schreibbar, wenn ihr Backend alle Schreibgarantien meldet und das Gerät dem Kontakt das Schreiben nicht ausdrücklich verweigert hat. Raumgeräte sind immer nur lesbar. Verschieben zwischen zwei Freigaben oder Geräten liefert `CrossesDevices` (Linux `EXDEV`, Windows `ERROR_NOT_SAME_DEVICE`).
- **Offline:** Ein Gerät bleibt sichtbar. Sein Inhalt liefert `HostUnreachable` (Linux `EHOSTUNREACH`, Windows 1232), nie einen leeren Ordner; zuvor gelesene Freigabenamen bleiben sichtbar. Der Start des Hosts verbindet sich zu keinem Gerät; eine Freigabe wird beim ersten Zugriff geöffnet.
- **Engines:** Eine `MountEngine` je (Gerät, Freigabe), Cache `<mount-cache>/<Kind-ID>`. Wird ein Gerät offline oder entfällt es, schließt der Host jede Engine ohne offene Datei und ohne Journaleintrag sofort; die übrigen schließen sich nach der letzten Datei, sobald ihr Journal leer ist. Kommt das Gerät zurück, werden die Journale ohne Zutun des Nutzers veröffentlicht. Ein Journal bleibt auch nach einem Neustart des Hosts erhalten (`<mount-cache>/se-devices/children.json`).
- **Ausschalten:** Der Host hängt aus; der Registry-Eintrag und die Entscheidung „aus“ werden in jedem Fall gespeichert; die Caches der Kinder und `children.json` bleiben. Unter „Verbundene Geräte als Ordner“ steht dann „Nicht gespeicherte Änderungen für <n> Gerät(e) warten auf eine Verbindung.“ Einschalten öffnet die Kinder mit Journal wieder.
- **Voreinstellung „ein“ (C5, V11):** Beim Start eines nicht eingeschränkten Dienstes schaltet der Dienst den Baum einmal ein, wenn keine Entscheidung vorliegt (`<AD>/mounts/devices.json`), der Baum nicht besteht, Laufwerke unterstützt sind, die Laufzeit bereit ist und mindestens ein Gerät gekoppelt ist. Es gibt keine Abfrage und keine Installation; ein Fehler schreibt keine Entscheidung. Ein Gerät, das erst nach dem Start des Dienstes gekoppelt wird, löst die Voreinstellung erst beim nächsten Start des Dienstes aus. Einmaliger Hinweis (`<AD>/mounts/devices-hint-shown`): die App nennt Laufwerk bzw. Ordner einmal; wer selbst einschaltet, bekommt ihn nicht.
- **Dateimanager:** Der Baum bekommt den KDE-Ort „Geräte“; ein GTK-Lesezeichen entfällt wegen der GLib-Regel (Abschnitt 4).

Dateien: Kern `N/mount/core/device_tree*.rs`, `peer_roots.rs`; Host `N/mount/os/shared/host_devices.rs`; Daemon `N/daemon/os/shared/devices_mount.rs`, `devices_host.rs`, `devices_ipc.rs`, `mount_host_ext_serve.rs`, `mount_client_devices.rs`, Wire `ipc_protocol_mount_host.rs` (`ChildBackend`, `DeviceWatch`).

## 6 Tresor-Laufwerke

Stand DRV-M7 und VLT-M12 (U14), am Code nachgelesen; im Quelltext committet (`ee4a5761`, `5267e500`), nicht ausgeführt.

- **Einbinden:** `se drive mount vault://<SITE> [--read-write] [--label <NAME>] [--letter <X>|--mount-point <ORDNER>]`. `<SITE>` ist die ID, ein ID-Anfang mit mindestens 4 Zeichen oder die eindeutige Bezeichnung; gespeichert wird die Site-ID (`MountSource::Vault { site_id }`, nur in `registry-2.json`). Ohne `--label` trägt das Laufwerk die Bezeichnung des Tresors. Ein Unterpfad und `vault-store://` werden abgelehnt (`N/cli/drive.rs:396`). Desktop und Android binden über dieselbe Quelle ein (S3-Kontextmenü „Als Laufwerk einhängen“, U29/U31).
- **Ort:** Windows ein Laufwerksbuchstabe (festes lokales Laufwerk, Abweichung P2: keine eigene Laufwerksart gegen Windows Search, Annahme A13); Linux `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>` auf tmpfs mit Sticky-Bit, außerhalb eines Mounts `1500`, gemountet `1700`. Ein Ordner außerhalb dieses Orts wird beim Start abgelehnt (`N/daemon/os/shared/mount_termination.rs:286`), ein Ort ohne tmpfs vom Host.
- **Kein Klartext auf der Platte:** Spool-Dateien im Cryptomator-Inhaltsformat unter dem Spool-Schlüssel des Laufwerks, Pfadfelder und Konflikttexte in `journal.jsonl` und `stages.json` versiegelt. Der Spool-Schlüssel liegt mit `spool-wrap` gewickelt in `<AD>/mount-cache/<mount-id>/spool-key.wrapped`; der Laufwerkshost erhält ihn über `Vault::SpoolKey` (Sitzungstoken des Hosts), nie den Masterkey (`N/daemon/os/shared/mount_manager_host.rs:287`). Laufwerke ohne Tresor schreiben ihre Bytes wie bisher.
- **Status:** Gesperrter Tresor beim Start: `failed:vault_locked` (JSON `status.Failed.detail` beginnt mit `vault_locked: `); mit versiegelten Änderungen `requires-unlock:<n>`, JSON `"recovery": {"requires_unlock": {"pending": <n>}}` (`N/cli/drive.rs:378`). Das Laufwerksfenster zeigt „Tresor gesperrt – wird nach dem Entsperren eingebunden“ ohne Fehlermeldung (`N/app/core/mount_ui_helpers.rs:66`, `:110`).
- **Sperren:** Der Hintergrunddienst ist Sperrteilnehmer jedes Tresor-Laufwerks (Frist: Ruhezustand 4 s, ausdrücklich und Entfernen 60 s, sonst 15 s). Das Laufwerk veröffentlicht, entfernt saubere Cache-Einträge und hängt aus. `se vault lock` ohne `--keep-unsaved` wird abgelehnt, solange Änderungen nicht veröffentlicht sind (Laufwerk bleibt eingehängt); mit `--keep-unsaved` und bei jeder automatischen Sperre bleiben sie versiegelt. Antwortet ein Laufwerk nicht bis zur Frist, bricht der Dienst es ab (Linux `fusectl`, Windows Job-Objekt) und bestätigt erst danach (`N/daemon/os/shared/mount_lock_participant.rs:173`).
- **Nach dem Entsperren:** Gesperrte Laufwerke behalten ihren Registry-Eintrag und werden bei jedem Entsperren wieder eingebunden; dabei werden ihre versiegelten Änderungen veröffentlicht (`N/daemon/os/shared/mount_termination.rs:244`). Ein beim Anmelden gesperrter Tresor stellt seine Laufwerke bis zum Entsperren zurück. `se drive unmount` vergisst das Laufwerk wie jedes andere.
- **Laufwerksende:** Aushängen, Auswerfen außerhalb von se und Absturz des Hosts werden dem Tresor gemeldet; hält danach nichts mehr die Sitzung, sperrt der Tresor. Unter Linux werden die freedesktop-Miniaturbilder von Dateien im Laufwerksordner beim Laufwerksende und beim Sperren entfernt (`N/vault/os/linux_thumbnails.rs:44`); Windows-Miniaturbilder lassen sich nur global abschalten.
- **Öffnen aus se:** `mount_path_for` liefert den Pfad eines Tresor-Orts auf einem eingehängten Laufwerk (`DriveIpcRequest::MountPathFor { location }`), `ensure_mounted_for` bindet den Tresor dafür bei Bedarf ein (`N/daemon/os/shared/mount_paths.rs:155`, `:189`).
- **MCP:** über das Werkzeug `se` mit denselben Befehlen; kein eigenes Werkzeug.

## 7 Android-Dokumentanbieter

Stand U23 (DRV-M8); im Quelltext committet (`94240017`, Registrierung `c9b0bed8`), nicht ausgeführt.

**Einschalten.** Einstellungen → „Ordner für Apps bereitstellen“: Hauptschalter „Für andere Apps bereitstellen“, darunter „Verbundene Geräte“, je gespeicherte Verbindung (SFTP, FTP, FTPS, WebDAV, SMB), „Google Drive“ (nur angemeldet) und je Tresor. Ab Werk ist nichts freigegeben. Der Hauptschalter schaltet die Komponente `SeDocumentsProvider` selbst ein oder aus (`COMPONENT_ENABLED_STATE_*`, im Manifest `android:enabled="false"`); ausgeschaltet kann keine andere App den Anbieter fragen. Ablage: `<AD>/docs-provider.json` (`{"provider","devices","connections":[Kontoschlüssel|"gdrive"],"vaults":[Site-ID]}`).

**Roots** (Systemauswahl, App „Dateien“): „Smart Explorer – Geräte“ (`devices`), je freigegebene Verbindung (`conn:<Kontoschlüssel als Hex>`), „Google Drive“ (`gdrive`), je freigegebener **entsperrter** Tresor „<Bezeichnung> (Tresor)“ (`vault:<Site-ID>`). Dokument-IDs: Root-ID, `/`, Pfad unter dem Root; Geräte über ihren Schlüssel (`devices/direct~<Kontakt>/…`, `devices/room~<Raum>~<Gerät>/…`), daher bleiben erteilte URIs nach einer Umbenennung gültig. Flags: `FLAG_SUPPORTS_IS_CHILD` immer, `FLAG_SUPPORTS_CREATE` bei schreibbarem Root, `FLAG_LOCAL_ONLY` nur für Tresore im Speicher des Telefons (Annahme A10).

**Rechte.** Nur das System (`MANAGE_DOCUMENTS`) bindet den Anbieter; andere Apps bekommen nur die URIs, die die Person auswählt. Jeder Aufruf prüft, ob der Root freigegeben ist; eine erratene ID eines anderen Roots ist „nicht gefunden“. Geschrieben wird nur, wo das Gerät es zulässt: Räume nur lesend, Kontakte nach der Freigabe dieses Telefons (Regel des Desktop-Gerätebaums), Verbindungen und Tresore nur bei sicheren Staged Writes des Backends. Ein Root, der seine Freigabe verliert, verliert sofort seine offenen Deskriptoren (`EACCES`) und seine Engine.

**Tresore.** Ein Tresor-Root besteht nur, solange der Tresor in der App entsperrt ist. Der Anbieter ist Sperrteilnehmer: Er veröffentlicht vorher, was offen ist; ein ausdrückliches Sperren ohne „Änderungen verschlüsselt aufbewahren“ stoppt, wenn danach Änderungen offen blieben („<n> Änderungen aus anderen Apps“). Sonst enden alle Deskriptoren (`EACCES`), Engine und zwischengespeicherte Listen, und der Root verschwindet sofort (`notifyChange` der Roots). Inhalte stehen nur versiegelt im Spool (Spool-Schlüssel wie DRV-M7); große Dateien liest „r“ bereichsweise über `open_read_at`.

**Ablauf.** Je Root eine Mount-Engine im App-Prozess (`<AD>/docs-cache/<docs-…>`, Cache-Lease, Posix-Namensregeln, `RangeReadPolicy::ReadOnlyHandles`). `docs.roots` braucht kein Netz. Eine kalte Liste antwortet mit `EXTRA_LOADING`, `docs.changed` folgt; ein Offline-Gerät antwortet mit `EXTRA_ERROR` „Gerät nicht erreichbar“. Öffnen: `r`, `w`, `wt`, `rw`, `rwt` („t“ kürzt), `wa`/`rwa` abgelehnt. Je Datei ein `HandlerThread`; `onGetSize` ohne Netz; `onFsync` meldet Fehler; `onRelease` blockiert nie: Hochladen im Hintergrund, dazu der WorkManager-Auftrag `DocsCommitWorker` (beschleunigt, `getForegroundInfo()` dataSync), der nach einem Prozessende `docs.retryPending` ausführt; scheitert es sechsmal, meldet eine Benachrichtigung „Hochladen fehlgeschlagen“, die Änderungen bleiben im Journal.

**Grenzen** (auch im Einstellungsabschnitt): Apps, die nur Dateipfade kennen, sehen nichts; MediaStore indiziert Anbieterinhalte nicht (Annahme A11); kein Kürzen offener Dateien; zufälliges Schreiben lädt die ganze Datei neu; Hochladen erst nach dem Schließen; Offline-Fehler.

## 8 Laufzeit

| Fähigkeit | CLI | Desktop | Android | IPC | MCP |
|---|---|---|---|---|---|
| Laufzeit prüfen | `se drive runtime [--json]` (`N/cli/drive.rs:31`, `:98-118`); Windows: Dokany, Linux: FUSE | „Dokany-Laufzeit / Hilfe“ (`N/app/core/mount_ui.rs:276`) | Ausnahme | – | `se` |
| Laufzeit installieren | `se drive install-runtime [--json]` (`drive.rs:33`, `:83-96`); unter Linux installiert se kein Paket, es nennt den Befehl der Distribution (`N/cli/drive_output_linux.rs:79-90`, `N/cli/drive.rs:106-110`) | Schaltfläche in „Dokany-Laufzeit / Hilfe“ (`N/app/core/mount_runtime_ui.rs`) | Ausnahme | – | `se` |
| FUSE-Bereitschaft, Kernel-Zeitlimit | `se drive runtime` unter Linux: `/dev/fuse` lesbar und schreibbar, ein setuid `fusermount3`, Kernel-Zeitlimit `fs.fuse.default_request_timeout` und Paketbefehl je Distribution (`N/mount/os/linux_os/fuse_runtime.rs:1-12`, `:220-234`) | keine eigene Anzeige belegt (Offene Punkte) | Ausnahme | – | `se` |

## 9 Ausnahmen mit Grund

- Android: keine Laufwerke und keine CLI (`N/lib.rs`: `cli` nur ohne `target_os = "android"`); Ordner für andere Apps stellt der Dokumentanbieter bereit (DRV-M8).
- MCP: kein eigenes Laufwerkswerkzeug; das Werkzeug `se` führt jeden `se drive`-Befehl aus.
- Anmeldeeintrag: keine eigene Bedienung in CLI, Desktop oder MCP; er folgt den Laufwerksoptionen (S2 C2), damit kein Eintrag ohne Laufwerk zurückbleibt.
- Windows: kein Ordner als Einhängeort, nur Laufwerksbuchstaben (S2 C1).
- Eingeschränkter Dienst nach der Anmeldung: Laufwerke anderer Geräte und der Gerätebaum warten auf den ersten anderen Auslöser, weil ein Dienst, der nur für die Wiederherstellung startet, dieses Gerät nicht über Share erreichbar machen darf (S2 K7, V11).
- Auswerfen vergisst das Laufwerk samt Option (Verhalten von v0.5.182); wer es nach der nächsten Anmeldung wieder haben will, lässt es eingehängt.

## 10 Offene Punkte

- Die Abnahme aller Laufwerksoberflächen läuft in der Stapel-Suite (Plan D3, Stufen des Strangs DRV-M10); diese Fassung ist nicht ausgeführt.
- Installer: `install-linux.sh --with-fuse` installiert `fuse3` aus den Paketquellen der Distribution (Funktion `install_fuse3`, `install-linux.sh`). Ohne den Schalter installiert das Skript nichts; es nennt den Paketbefehl (`fuse_package_command`) und prüft danach `fusermount3` (setuid root) und `/dev/fuse` (`check_fuse`). `se drive runtime` nennt den Befehl ebenfalls.
- Desktop-Anzeige der FUSE-Bereitschaft unter Linux: nicht belegt.
- Registrierungen der Laufwerks-Pakete stehen in den Commits `91d70957` (U06), `9456b8ad` (U12), `f83d1d94` (U17) und `c9b0bed8` (U23). Ob alle Zeilen vollständig sind, prüft die Stapel-Suite.
- Tresor-Laufwerke (U14): E2E-Prüfungen über FUSE und Dokany stehen laut Lesestand U14 aus.
- Ende-zu-Ende mit echtem Dokany und FUSE (`drive_task_auto_remount_after_daemon_restart`) läuft nur in der Stapel-Suite (Stufen `windows-core: drive-cli`, `linux-root: drive-fuse`).
- DRV-M8 (U23): Der Gerätebaum läuft auf Android mit einer Engine je Gerät über dessen ganzes Share-Backend, nicht über `DeviceTree`; `RootedBackend` ist im Dienst privat (Lesestand U23). Ein echtes Prozessende prüft die Emulatorstufe nicht; `drive_task_android_process_kill_retries_pending` prüft den Journalweg von `docs.retryPending`.
- DRV-M5, DRV-M6 (U17): Die Tests des Gerätebaums laufen über Ordner statt über echte Geräte (`ChildLink`); `drive_task_devices_*` mit `fixture=share` sind als Kerntests geschrieben, die Ende-zu-Ende-Fassung mit zwei Prozessen steht aus. Der Windows-Test `drive_task_devices_windows_drive_lists_peer` ist `#[ignore]` und braucht Dokany.
- Handtests (U43): Dolphin zeigt den KDE-Ort mit Symbol (Annahme zu `ID` und `folder-remote`) und nicht doppelt (Annahme A8); Nautilus, Nemo und Thunar zeigen das Lesezeichen außerhalb von Home; der Gerätebaum unter Windows mit mehreren Geräten und einem Raum.
- Linux zeigt die virtuellen Ordner des Gerätebaums mit den Rechten der FUSE-Attributabbildung (`fuse_ops_attr.rs`); der Plan nennt `0555`. Schreiben bleibt in jedem Fall mit `PermissionDenied` verweigert.
- Dokany: Wiedervorlage `WINFSP-AB` und `DOKANY-WATCH` stehen in `docs/TODO.md` (außerhalb dieses Dokuments; Abschnitt „Dokany bleibt“ in `README.md`).

## Laufender Gesamtkandidat: Recovery, Kapazität und Rootfreigabe

Quellstand 2026-10-26; gemeinsame Remote-Suite und Veröffentlichung stehen aus.

| Fähigkeit | Sichtbare Oberfläche | Gemeinsame Grenze |
|---|---|---|
| Cache sichern und Laufwerk entfernen | Desktop `N/app/core/mount_recovery_ui.rs`; `se drive save-remove` und `se drive recovery list/import/reactivate` (`N/cli/drive_recovery.rs:23`); MCP über `se` | Der Daemon besitzt die Offline-Transaktion. Reaktivierung ergibt zunächst ein gestopptes Laufwerk mit ausgeschaltetem Auto-Remount; `retry` bleibt ausdrücklich nötig. |
| Providerkapazität und Cachebelegung unterscheiden | Desktop `N/app/core/mount_capacity_ui.rs`; CLI `N/cli/drive_capacity.rs:14`, JSON im Mountsnapshot und Daemon-IPC | `N/mount/core/capacity.rs` und `os/shared/capacity_refresh.rs` trennen belegte Providerkapazität, unbekannte Werte, veraltete Werte und lokale Cachebelegung. Unbekannt ist nicht null. |
| Tresorroot ausdrücklich freigeben | Desktop `vault_settings_ui.rs`; CLI `vault/open_policy.rs:18`; Android `VaultOpenPolicy.kt` über `vault.openPolicy/confirmRootTrust/revokeRootTrust/providerRelease` (`N/mobile/os/shared/domains/vault_open_policy.rs:6`) | Rootvertrauen ist an Tresor, Backend, Account und Root gebunden. Klartextkopie und Android-Providerfreigabe bleiben eigene Entscheidungen. Widerruf betrifft auch bereits laufende Providerzugriffe. |

Die native Mount-Recoveryaktion betrifft Windows-/Linux-Mounts. Android bleibt
Dokumentanbieter; aus dessen Existenz wird keine zusätzliche native Mountaktion
abgeleitet. Recoverypakete halten historische Identität und unbestätigte Lücken;
frische Profildaten dürfen einen alten Bestand nicht still neu legitimieren.
Belege: `N/daemon/os/shared/mount_recovery_identity.rs`,
`N/mount/core/recovery_identity.rs` und
[Review B1](../plaene/2026-10-24-batches/review-b1.md).
