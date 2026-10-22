# Laufwerke, Medien, Agenten, Kopieren und se Desktop: Machbarkeit (2026-10-22)

## Auftrag

Nachträge vom 2026-10-22, getrennt von der Rückmeldung zu Fernzugriff und se Desktop
(`docs/plaene/2026-10-22-fernzugriff-desktop/machbarkeit.md`, P1 bis P9). Der Auftrag:
- P10 und P11 aufschreiben; sie sollen „beim nächsten Lauf mitgemacht werden“;
- P12 kam mit P10 und P11, bleibt aber vorerst draußen (Nutzer, 2026-10-22);
- P13 als Ergänzung dokumentieren. Annahme: P13 gehört in denselben nächsten Lauf;
- P14 und P15 kamen danach ohne Zeitangabe. Zu P15 sagte der Nutzer „noch nicht umsetzen“. Annahme: beide gehören in den nächsten Lauf und sind bis dahin nur dokumentiert;
- noch nichts umsetzen. Wie bei P1 bis P9: einschätzen, ob und wie jeder Punkt umsetzbar ist, ohne konkrete Implementierungsplanung und ohne Fehlersuche.

Die Punkte stehen als Zeilen in `docs/TODO.md`:
- `LAUFWERKE-LIVE`
- `OPTISCH-LESEN`
- `OPTISCH-DATENTRAEGER` (zurückgestellt)
- `AGENT-PLUGIN-UPDATE`
- `KOPIE-ORDNER-ZUSATZ`
- `DESKTOP-ORDNER-EXPLORER`

Kennzeichnung:
- **Befund** = im Code oder in der Doku dieses Repositorys nachgelesen, Stand 0.5.184 (Commit `66cf09db`), mit `Datei:Zeile`.
- **Quelle** = externe Quelle, selbst abgerufen am 2026-10-22.
- **Zusammenfassung des Abrufwerkzeugs** = Quelltextdatei abgerufen; die Aussage stammt aus der Zusammenfassung des Abrufwerkzeugs, nicht aus einem Vergleich mit dem Rohtext.
- **laut Suchergebnis** = nur in Suchergebnissen gesehen, Seite nicht geladen.
- **Annahme** = ungeprüft, ausdrücklich so benannt.
- **Lücke** = genau benannte fehlende Angabe.

Pfadkürzel: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

## Die Punkte

| Nr. | Rückmeldung |
|---|---|
| P10 | Laufwerke, USB-Datenträger und Ähnliches einstecken oder ausstecken: Sie tauchen erst nach Schließen und erneutem Öffnen auf |
| P11 | CDs und DVDs besser, effizienter und schneller lesen |
| P12 | Kleiner Zusatz: Funktionen wie IsoBuster, also die Dateiinhalte einer Disc wirklich lesen. Nachtrag: vorerst draußen; Umsetzung potentiell wie bei IsoBuster, oder Übernahme von dessen Komponenten |
| P13 | Die MCP-Einbindung in den Agenten soll sich mit Updates möglichst automatisch aktualisieren, ebenso die Plugin-Fassung |
| P14 | Beim Kopieren eines Ordners, dessen Name im Ziel schon existiert, sollen die Dateien nicht in den gleichnamigen Ordner hineingemischt werden. Stattdessen bekommt der oberste doppelte Ordner den Zusatz, und zwar dort, wohin kopiert wird. Weil Zusammenführen manchmal gewollt ist, fragt ein Fenster, wie vorgegangen wird, wenn möglich mit Vorschau |
| P15 | Im se Desktop Ordner automatisch im se Explorer öffnen |

Die Frage „Werden Remotes noch über Dokany eingebunden?“ ist kein Arbeitspunkt. Die Antwort steht in `README.md:239-250` („Dokany bleibt“) und in `docs/oberflaechen/laufwerke.md:32`.

## Übersicht

| Punkt | Urteil | Hauptweg | Größte Hürde |
|---|---|---|---|
| P10 | Windows umsetzbar, klein. Linux umsetzbar, mittel. Android laut Code schon live | Windows: Geräteereignisse sammeln und die Laufwerke neu lesen, dazu Neulesen beim Fensterwechsel. Linux: Datenträgerliste über UDisks2 | Netzlaufwerke und `subst` melden sich nicht über Geräteereignisse |
| P11 | umsetzbar; vorher messen | ein Leser je optischem Laufwerk und große Anfragen. Danach Lesen nach Lage auf der Disc und eine Fehlerkarte für beschädigte Discs | Die Lage einer Datei gibt es ohne Sonderrechte nur über ein eigenes Lesen des Dateisystems (Linux, CDFS) |
| P12 | zurückgestellt | Umsetzung wie bei IsoBuster, oder Übernahme von dessen Komponenten | Eine Übernahme braucht eine Vereinbarung mit dem Hersteller (EULA) |
| P13 | umsetzbar; Grundmechanik vorhanden | Stufe A: beim Update auffrischen, auch bei Pfadwechsel. Stufe B: laufende Server nicht beenden. Stufe C: Server ohne Neuverbinden tauschen | Claude Code und Codex verbinden beendete stdio-Server nicht neu |
| P14 | umsetzbar, mittel | lokale Ziele wie Remote-Ziele: oberster Ordner „Name (2)“. Abfrage mit Vorschau, Option im Terminal | Vorschau großer oder entfernter Bäume. Voreinstellung im Terminal (E10) |
| P15 | umsetzbar, klein | Ordner der Fläche als Fenster des se Explorers öffnen, wie heute schon Tresore und Geräte. Einstellung „Ordner öffnen mit“ für den Systemweg | keine bekannt |

## P10 Laufwerke erscheinen erst nach Neustart

### Bestand

- **Desktop unter Windows** (Befund): Die Laufwerksliste wird einmal beim Start gelesen (`N/app/core/init.rs:106-107`):
  - `list_drives()` über `GetLogicalDrives` (`N/app/os/windows/platform.rs:41-48`);
  - `drive_info_list()` mit freiem und gesamtem Platz (`:50`).

  Danach schreibt kein Code mehr in `drives` oder `drive_info`. Eine Suche nach Zuweisungen, `push` und `retain` in `N/app/` findet keine Stelle.
- **Was den Startstand nutzt** (Befund):
  - Seitenleiste (`N/app/core/sidebar_locations.rs:41`);
  - Startseite (`N/app/core/landing.rs:78`);
  - Ordnerauswahl (`N/app/core/picker_impl.rs:120`);
  - Speicheranalyse (`N/app/core/analytics_core.rs:470`, `N/app/core/analytics_controls_ui.rs:39`);
  - Aufräumen (`N/app/core/reclaim_ui.rs:12`);
  - Schnellsuche (`N/app/core/omni_accel.rs:221`);
  - Freigaben (`N/app/core/share_exports_ui.rs:179-184`);
  - Wurzeln des Ordnerindex (`N/app/core/scanning.rs:250`);
  - unter Windows dessen rekursive Überwachung jeder Wurzel (`N/app/os/windows/watchers.rs:73-88`), die erst startet, wenn ein Index besteht (`N/app/os/windows/platform.rs:343-345`).
- **Desktop unter Linux** (Befund): `list_drives()` liefert nur `/`, `drive_info_list()` eine leere Liste (`N/app/os/linux_os.rs:20-26`). Den Laufwerksabschnitt zeigt die Seitenleiste nur bei nicht leerer Liste (`sidebar_locations.rs:41`).
  - Eingesteckte Datenträger erscheinen dort weder vor noch nach einem Neustart als Laufwerk. Das folgt aus dem Code und ist am Gerät nicht geprüft.
  - Die beschriebene Wirkung „erst nach Schließen und Öffnen“ passt daher zum Windows-Desktop. Annahme: Die Rückmeldung betrifft Windows; eine Plattform wurde nicht genannt.
- **Android** (Befund): `Storage.watchVolumes` meldet einen `StorageManager.StorageVolumeCallback` an (`A/system/Storage.kt:32-41`). Der Kern schickt bei jeder Änderung die Volumeliste neu (`A/core/Core.kt:163`). Laut Code aktualisiert Android schon live; am Gerät nicht geprüft.
- **Hintergrunddienst** (Befund):
  - Unter Windows meldet `CM_Register_Notification` neue Volumes (`N/daemon/os/windows/volume_monitor.rs:5-33`). Das ist der Auslöser „beim Anstecken synchronisieren“.
  - Unter Linux liest der Dienst `/proc/self/mountinfo` (`N/daemon/os/linux_os/drives.rs:22`).
  - Der Baustein für Windows ist also vorhanden; die Desktop-App nutzt ihn nicht.
- **Terminal `se` und MCP** (Befund): Sie halten keine Laufwerksliste. `GetLogicalDrives` steht nur in `N/app/os/windows/platform.rs:43` und `N/mount/os/windows/host.rs:423,430`. P10 betrifft sie nicht.
- **Eigene Dokany-Laufwerke** (Annahme): Ein über die App eingehängtes Laufwerk taucht in der Laufwerksliste der App erst nach einem Neustart auf. Das folgt aus dem Startstand und ist am Gerät nicht geprüft.

### Wege je Plattform

**Windows:**
- **Fensternachricht `WM_DEVICECHANGE`:** Windows schickt sie bei neuen oder entfernten Laufwerken und Medien an alle Top-Level-Fenster, ohne Anmeldung [Quelle: https://learn.microsoft.com/en-us/windows/win32/devio/detecting-media-insertion-or-removal, abgerufen 2026-10-22].
  - Message-only-Fenster bekommen keine Broadcasts [Quelle: https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features, abgerufen 2026-10-22].
  - Eine eingelegte Disc trägt `DBTF_MEDIA` [Quelle: https://learn.microsoft.com/en-us/windows/win32/api/dbt/ns-dbt-dev_broadcast_volume, abgerufen 2026-10-22].
  - Der Hook `with_msg_hook` von winit 0.30.13 sieht nur Nachrichten aus der Warteschlange; gesendete Nachrichten verteilt `PeekMessage` selbst [Quelle: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-peekmessagew, abgerufen 2026-10-22].
  - Ob Windows diese Nachricht sendet oder postet, ist nicht dokumentiert (Lücke). Ein eigenes, unsichtbares Top-Level-Fenster in einem eigenen Thread hängt davon nicht ab.
- **`CM_Register_Notification`:** Sie braucht kein Fenster [Quelle: https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_register_notification, abgerufen 2026-10-22] und ist im Dienst schon im Einsatz.
  - Lücke: ob sie beim Einlegen einer Disc meldet.
- **Shell-Benachrichtigung:** `SHChangeNotifyRegister` mit `SHCNE_DRIVEADD`, `SHCNE_DRIVEREMOVED`, `SHCNE_MEDIAINSERTED` und `SHCNE_MEDIAREMOVED` [Quelle: https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shchangenotify, abgerufen 2026-10-22].
- **Dokany-Laufwerke:** Dokany sendet beim Einhängen eines Buchstabens sowohl `SHCNE_DRIVEADD` als auch `WM_DEVICECHANGE` mit `DBT_DEVICEARRIVAL`, beim Aushängen die Gegenstücke [Quelle: https://raw.githubusercontent.com/dokan-dev/dokany/master/dokan/mount.c, abgerufen 2026-10-22; Zusammenfassung des Abrufwerkzeugs, kein Rohtextvergleich].
- **Lücken der Ereignisse:**
  - `subst` löst nur `WM_SETTINGCHANGE` aus [Quelle: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-definedosdevicew, abgerufen 2026-10-22].
  - Für verbundene Netzlaufwerke ist kein Broadcast belegt (Lücke).
- **Weg:** Die Ereignisse sammeln, entprellen und bei jedem Ereignis `GetLogicalDrives` und den Platz neu lesen. Zusätzlich neu lesen, wenn das Fenster aktiv wird; das deckt `subst` und Netzlaufwerke ab. Annahme: Das Neulesen ist billig genug für jeden Fensterwechsel.

**Linux:**
- Die Desktop-App bekommt erst eine Datenträgerliste. Grundlage ist UDisks2 über D-Bus; `zbus` ist schon Abhängigkeit (`native/Cargo.toml:330-333`).
  - Laufwerke kommen über `InterfacesAdded` und `InterfacesRemoved`, Änderungen über `PropertiesChanged` [Quelle: https://dbus.freedesktop.org/doc/dbus-specification.html, abgerufen 2026-10-22].
  - Die Eigenschaften `MediaAvailable` und `Optical` [Quelle: https://storaged.org/doc/udisks2-api/latest/gdbus-org.freedesktop.UDisks2.Drive.html, abgerufen 2026-10-22] und `MountPoints` [Quelle: https://storaged.org/doc/udisks2-api/latest/gdbus-org.freedesktop.UDisks2.Filesystem.html, abgerufen 2026-10-22] zeigen Medium und Einhängeort.
  - Einhängen geht über `Filesystem.Mount()` mit polkit-Nachfrage (gleiche Quelle).
- Rückfall ohne D-Bus: `/proc/self/mounts` ist pollbar, Ein- und Aushängen setzt `POLLPRI` [Quelle: https://man7.org/linux/man-pages/man5/proc_pid_mounts.5.html, abgerufen 2026-10-22]. Annahme: `mountinfo` verhält sich gleich.

**Android:** Bleibt wie heute. Am Gerät prüfen, ob die Liste live folgt.

### Urteil P10

- **Windows:** umsetzbar, klein. Die Bausteine sind vorhanden: `windows-sys` und die Benachrichtigung aus dem Dienst. Alle Anzeigen aus dem Bestand lesen danach den neuen Stand.
- **Linux:** umsetzbar, mittel. Die Datenträgerliste ist neu, die Live-Erkennung kommt aus UDisks2 dazu.
- **Android:** laut Code schon live; nur prüfen.

## P11 CDs und DVDs schneller lesen

### Bestand

- **Keine Sonderbehandlung optischer Laufwerke** (Befund): In `N/` und `A/` gibt es keinen Treffer für `cdrom`, `optical`, `iso9660`, `IOCTL_CDROM`, `IOCTL_SCSI_PASS_THROUGH` oder `DRIVE_CDROM`. Nur die Dateisystemnamen kommen vor: `CDFS` mit Zeitauflösung in Sekunden (`N/vfs/core/fs_profile.rs:82`) und `udf` in der Liste für gesammeltes Flushen (`:54`).
- **Lokales Kopieren** (Befund):
  - Unter Windows kopiert ein einziger `CopyFile2`-Aufruf jede Datei, nie in Bereichen (`N/transfer/os/shared/engine/local.rs:107-110`).
  - Die Zahl gleichzeitiger Operationen regelt ein adaptiver Regler (`N/transfer/core/flow_control.rs:1-23`):
    - Er startet bei 2 (`START_LIMIT`, `:33`).
    - Er verdoppelt, solange das mindestens 10 % mehr Durchsatz bringt.
    - Danach probt er regelmäßig einen Schritt nach oben oder unten.
  - Unter Linux und Android kopiert die Engine eine große Datei erst nach einer Messung je Gerätepaar in parallelen Bereichen (`N/transfer/os/shared/engine/local_plan.rs:1-5`).
- **Folge** (Annahme, nicht gemessen): Eine Kopie von einer Disc beginnt mit zwei gleichzeitigen Dateilesungen auf demselben Laufwerk.

### Was optisches Lesen bremst

- **Suchen:** Microsoft rechnet mit 150 ms mittlerer Suchzeit. 20 schlecht liegende Dateien kosten damit 4 s reines Suchen. Während des Suchens liest das Laufwerk nichts [Quelle: https://learn.microsoft.com/windows/desktop/DxTechArts/optimizing-dvd-performance-for-windows-games, abgerufen 2026-10-22].
- **Mehrere gleichzeitige Ströme senken den Durchsatz** (gleiche Quelle). Ein Nutzerbericht von 2007: Etwa 4 GB von einer DVD dauerten allein etwa 10 Minuten, mit einer zweiten gleichzeitigen Kopie fast eine Stunde [Quelle: https://bugzilla.gnome.org/show_bug.cgi?id=413748, abgerufen 2026-10-22].
- **Anfragegröße:** Ein DVD-Fehlerkorrekturblock umfasst 32 KB. Kleinere Anfragen verschwenden den Rest; Anfragen sollen ein Vielfaches davon sein [Quelle: Microsoft-Seite wie oben].
- **Beschädigte Stellen:**
  - Unter Linux wartet `sr` je Befehl bis 30 s und wiederholt bis zu dreimal [Quelle: https://gbmc.googlesource.com/linux/+/refs/heads/master/drivers/scsi/sr.h, abgerufen 2026-10-22; Zusammenfassung des Abrufwerkzeugs].
  - Für Windows fehlen die Werte von `cdrom.sys` (Lücke).
  - Laut Suchergebnis braucht ein Laufwerk auf beschädigten Medien 30 s oder mehr je Block [laut Suchergebnis: https://www.isobuster.com/help/data_recovery].

### Vorbilder

- **GNU ddrescue** liest zuerst in großen Stücken und überspringt Fehler. Danach liest es die Ränder und den Rest sektorweise. Eine Karte merkt sich den Stand jedes Bereichs [Quelle: https://www.gnu.org/software/ddrescue/manual/ddrescue_manual.html, abgerufen 2026-10-22].
- **dvdisaster** steuert Leseversuche je defektem Sektor selbst und rät zu 0 oder 1 Wiederholungen im Laufwerk [Quelle: https://linux.die.net/man/1/dvdisaster, abgerufen 2026-10-22].
- **IsoBuster** stellt Wiederholungen, Wartezeiten und Rückwärtslesen ein [Quelle: https://www.isobuster.com/help/communication_read_settings, abgerufen 2026-10-22].

### Weg

- **Messen zuerst:** wie bei P7 der vorigen Notiz. Eine DVD mit vielen Dateien einmal wie heute kopieren und einmal mit einem Leser.
- **Stufe 1, ein Leser je optischem Laufwerk:**
  - Für eine optische Quelle gilt eine feste Gleichzeitigkeit von 1 statt des Reglers, und die Anfragen sind große Vielfache von 32 KB.
  - Erkennen lässt sich die optische Quelle unter Windows über den Laufwerkstyp. Unter Linux geht es über das Dateisystem `iso9660` bzw. `udf` in `mountinfo` oder über ein `sr`-Gerät (Annahme).
  - Für eine optische Quelle löst ein eigener Leseweg `CopyFile2` ab. Annahme: `CopyFile2` lässt Reihenfolge und Anfragegröße nicht steuern.
  - Ordnerindex und Überwachung lassen optische Laufwerke aus. Annahme: Auf einem schreibgeschützten Medium ändern sie nichts, kosten aber Drehen und Suchen.
- **Stufe 2, Reihenfolge nach Lage auf der Disc:**
  - Unter Windows liefert `FSCTL_GET_RETRIEVAL_POINTERS` die Lage für UDF; CDFS steht nicht in der Liste [Quelle: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_get_retrieval_pointers, abgerufen 2026-10-22].
  - Unter Linux kennen `isofs` und `udf` kein FIEMAP. FIBMAP braucht `CAP_SYS_RAWIO` [Quelle: https://gbmc.googlesource.com/linux/+/refs/heads/master/fs/ioctl.c, abgerufen 2026-10-22; Zusammenfassung des Abrufwerkzeugs].
  - Ohne Sonderrechte bleibt das eigene Lesen des Dateisystems vom Rohgerät. Das gehört zu P12 und ruht mit ihm.
- **Stufe 3, beschädigte Discs:**
  - Ein erster Durchgang mit wenigen Wiederholungen und kurzer Frist.
  - Defekte Bereiche in einer Karte merken, wie bei ddrescue.
  - Erst nach den guten Dateien nachlesen, auf Wunsch bei gedrosseltem Laufwerk.
  - Drosseln geht unter Windows über `IOCTL_CDROM_SET_SPEED` [Quelle: https://learn.microsoft.com/en-us/previous-versions/windows/drivers/storage/cd-rom-set-speed, abgerufen 2026-10-22]. Unter Linux geht es über `SET CD SPEED` per SG_IO, nur bei zum Schreiben geöffnetem Gerät [Quelle: https://gbmc.googlesource.com/linux/+/refs/heads/master/drivers/scsi/scsi_ioctl.c, abgerufen 2026-10-22; Zusammenfassung des Abrufwerkzeugs].
- **Android:** `vold` bindet ein USB-CD/DVD-Laufwerk nicht ein, denn die Blockgruppe `sr` (Major 11) fehlt in seiner Liste [Quelle: https://android.googlesource.com/platform/system/vold/+/refs/heads/main/model/Disk.cpp, abgerufen 2026-10-22]. P11 betrifft daher Windows und Linux.

### Urteil P11

- **Gesamt:** umsetzbar; vorher messen.
- **Stufe 1:** klein bis mittel. Betroffen sind Regler, Leseweg und Index. Sie ist der Teil von P11 für den nächsten Lauf.
- **Stufe 2 und 3:** mittel. Sie brauchen Dateisystemleser und Rohzugriff aus P12 und ruhen deshalb mit P12.

## P12 Funktionen wie IsoBuster

Zurückgestellt: P12 bleibt vorerst draußen und gehört nicht zum nächsten Lauf (Nutzer, 2026-10-22).

- **Bestand** (Befund): Es gibt keinen eigenen Disc-Zugriff, keinen Leser für ISO 9660 oder UDF und kein Abbildformat (Suche wie unter P11).
- **Weg** (Vorgabe des Nutzers, bewusst einfach gehalten): Umsetzung potentiell wie bei IsoBuster, oder Übernahme von dessen Komponenten.
- **Hinweis zur Übernahme:** Die EULA von IsoBuster verbietet, Teile der Software zu nutzen („You may not modify, use parts, reverse engineer, decompile, or disassemble the Software“) [Quelle: https://www.isobuster.com/eula.php, abgerufen 2026-10-22]. Eine Übernahme von Komponenten braucht daher eine Vereinbarung mit dem Hersteller. Lücke: ob der Hersteller Komponenten lizenziert.

## P13 Agenten-Einbindung mit Updates aktualisieren

### Bestand

- **Claude Code** (Befund): `se plugin install` schreibt einen lokalen Marketplace nach `<App-Daten>/smart_explorer/ai-plugins/claude/` (`N/cli/plugin/files.rs:23-27`). Darin stehen:
  - `plugin.json` mit der Version dieses `se` (`:54`);
  - `.mcp.json` mit dem absoluten Pfad dieses `se` und dem Argument `mcp` (`:66-67`).

  Danach ruft `se` die Befehle `claude plugin marketplace add` und `claude plugin install … -y` auf (`N/cli/plugin.rs:19-31`, `docs/AI_AGENT_PLUGIN.md:39-47`).
- **Codex** (Befund): `codex mcp add smart-explorer -- <se> mcp` mit absolutem Pfad, dazu der Skill unter `~/.agents/skills/smart-explorer` (`docs/AI_AGENT_PLUGIN.md:48-53`, `N/cli/plugin/codex.rs:1-4`).
- **Selbstauffrischung** (Befund):
  - Jeder erzeugte Ordner trägt einen Marker mit Version und Programmpfad (`N/cli/plugin/files.rs:113-121`).
  - `se mcp` schreibt veraltete Ordner beim Start neu (`:146-163`, Aufruf `N/cli/mcp/server.rs:174`).
- **Was dabei nicht abgedeckt ist** (Befund, wo nicht anders markiert):
  1. Die Auffrischung läuft erst, wenn ein Agent `se mcp` startet. Annahme: Der Agent liest Skill und Plugin-Dateien beim Sitzungsstart, bevor er den Server startet. Dann nutzt die erste Sitzung nach einem Update noch den alten Skill.
  2. Der Updater frischt nichts auf. `N/updater/` erwähnt das Plugin nur im Dokukommentar `N/updater/mod.rs:47-48`.
  3. Den MCP-Eintrag in der Codex-Konfiguration schreibt nur die Installation. Die Auffrischung erneuert nur den Skill (`files.rs:155-161`).
     - Ändert sich der Pfad von `se`, behält der Eintrag den alten Pfad.
     - Dasselbe gilt für `.mcp.json` bei Claude Code: Startet unter dem alten Pfad kein `se` mehr, läuft auch keine Auffrischung.
  4. Laufende Agenten: Der Desktop-Updater beendet `se mcp` vor dem Ersetzen. Danach muss man im Agenten den Server neu verbinden oder eine neue Sitzung starten (`docs/AI_AGENT_PLUGIN.md:308-315`).
  5. Dass Claude Code ein Plugin eines lokalen Marketplace bei jedem Sitzungsstart direkt aus dem Ordner lädt, ist ein Stand vom 2026-10-12 (`docs/refs/claude-code-plugins-mcp-2026-10-12.md:38-42`). Die Nachprüfung steht unten.

- **Updatewege und laufende Server** (Befund):
  - Der Desktop-Updater ersetzt `se` erst, wenn kein Prozess des Programms mehr läuft. Dazu schreibt er `daemon.stop` = `stop` und wartet höchstens 30 s. Jeder langlebige Prozess beendet sich auf diese Anforderung, auch `se mcp` (`N/daemon/os/shared/update_stop.rs:1-18`, `N/cli/mcp/update_stop.rs:1-9`).
  - Das Terminal-Update `se update` stoppt keine anderen `se`-Prozesse. Unter Linux benennt es die neue Datei über die alte (`N/updater/os/shared/cli_swap.rs:212-227`). Unter Windows kann sich `se.exe` nicht selbst ersetzen (`N/updater/os/windows.rs:134-141`).

### Nachprüfung beim Agenten (Claude Code)

- **Lokales Plugin:** Die Aussage vom 2026-10-12 gilt weiter. Wörtlich [Quelle: https://code.claude.com/docs/en/plugins/loading.md, abgerufen 2026-10-22]:

  > „Relative-path plugins in a marketplace you added from a local path: the plugin loads in place from its path inside the marketplace folder. Your edits to the source directory take effect at the next session start or `/reload-plugins`, and you don't need to increase the version.“

  Ein `claude plugin update` oder ein eingeschaltetes Auto-Update braucht `se` dafür nicht. Auto-Update ist für lokale Marketplaces ohnehin voreingestellt aus [Quelle: https://code.claude.com/docs/en/plugins/install.md, abgerufen 2026-10-22].
- **Kein automatisches Neuverbinden:** stdio-Server verbindet Claude Code nicht von selbst neu. Wörtlich [Quelle: https://code.claude.com/docs/en/mcp.md, abgerufen 2026-10-22]:

  > „Stdio servers are local processes, and Claude Code doesn't reconnect them automatically.“

  Von Hand geht es über `/mcp`, oder ab Version 2.1.284 über `/mcp reconnect all`.
- **Kein Auffrischen von außen:** Laufende Sitzungen lassen sich nicht von außen neu laden. `/reload-plugins` ist ein Befehl innerhalb der Sitzung [Quelle: https://code.claude.com/docs/en/plugins/cli-reference.md, abgerufen 2026-10-22].

- **`list_changed` in Claude Code:** In einer interaktiven Terminalsitzung holt Claude Code die neue Werkzeugliste ohne Neuverbinden. Wörtlich [Quelle: https://code.claude.com/docs/en/mcp.md, Abschnitt „Dynamic tool updates“, abgerufen 2026-10-22]:

  > „In an interactive terminal session, Claude Code fetches the updated list from that server, so you don't need to reconnect it.“

  Zwei Issues melden, dass die Meldung mitten in der Sitzung nicht beachtet wird [laut Suchergebnis: https://github.com/anthropics/claude-code/issues/62058]. Lücke: ob das in der aktuellen Version behoben ist.

### Nachprüfung beim Agenten (Codex)

- **MCP-Eintrag:** Er liegt in `~/.codex/config.toml` unter `[mcp_servers.<name>]`, `command` ist Pflicht [Quelle: https://learn.chatgpt.com/docs/extend/mcp?surface=cli, abgerufen 2026-10-22]. `developers.openai.com/codex` leitet dorthin weiter.
  - Lücke: Die Doku sagt nicht, wann Codex den Eintrag liest und ob es einen beendeten Server neu verbindet.
  - Das Issue zum fehlenden Neuverbinden ist offen. Als Ausweg nennt es nur manuelles Neuladen oder einen Neustart des Prozesses [Quelle: https://github.com/openai/codex/issues/11489, abgerufen 2026-10-22].
- **Eintrag ändern:** Ein erneutes `codex mcp add` mit demselben Namen überschreibt den Eintrag und bricht nicht ab: `run_add` setzt den Eintrag mit `servers.insert(name.clone(), new_entry)` und schreibt die Tabelle zurück [Quelle: https://github.com/openai/codex/blob/main/codex-rs/cli/src/mcp_cmd.rs, abgerufen 2026-10-22].
  - Die Doku sagt das nicht (Lücke). Annahme: Der Rückgabewert ist dabei 0.
  - Für den Pfadwechsel in Stufe A reicht damit ein erneutes `codex mcp add`.
- **Skills:** Sie liegen unter `$HOME/.agents/skills` [Quelle: https://learn.chatgpt.com/docs/build-skills, abgerufen 2026-10-22]. Wörtlich [Quelle: gleiche Seite]:

  > „Codex detects skill changes automatically. If an update doesn't appear, restart Codex.“
- **Codex-Plugins:**
  - Es gibt sie mit Marketplace. Lokale Marketplaces holt Codex nicht automatisch neu [Quelle: https://github.com/openai/codex/pull/17425, abgerufen 2026-10-22].
  - Eine Doku-Regel für absolute `command`-Pfade in einem Plugin gibt es nicht (Lücke).
  - `codex mcp add` mit absolutem Pfad bleibt der belegte Weg [Quelle: https://github.com/openai/codex/issues/19582, abgerufen 2026-10-22].
- **`list_changed` in Codex:** Standardmäßig nicht. Das Issue ist offen, Zitat: „Codex receives the notification but does not act on it“ [Quelle: https://github.com/openai/codex/issues/37417, abgerufen 2026-10-22].
  - Ein am 2026-10-16 gemergter Pull Request fügt den Schalter `mcp_tool_list_changed` hinzu, voreingestellt aus [Quelle: https://github.com/openai/codex/pull/54656, abgerufen 2026-10-22].
  - Lücke: der Konfigurationsweg und die erste Version mit diesem Schalter.

### Weg

Drei aufeinander aufbauende Stufen:
- **Stufe A, beim Update auffrischen.** Updater und `se update` rufen nach dem Ersetzen dieselbe Auffrischung auf wie `se mcp` beim Start, und zwar im Kontext der Person, deren Agenten eingerichtet sind.
  - Hat sich der Pfad von `se` geändert, schreibt die Auffrischung auch den MCP-Eintrag neu: für Claude Code in `.mcp.json`, für Codex über dessen Befehl (siehe Nachprüfung Codex).
  - Die Auffrischung beim Start von `se mcp` bleibt als Rückfall.
  - Wirkung: Schon die erste neue Agentensitzung nach dem Update hat Skill und Plugin der neuen Version.
  - Grenze: Ein `se update` als root frischt nur die App-Daten von root auf (Annahme aus dem Ort `N/cli/plugin/files.rs:23-27`).
- **Stufe B, laufende Server nicht beenden.** Das Update ersetzt `se`, ohne laufende `se mcp` zu beenden. Laufende Agentensitzungen behalten dann bis zu ihrem Ende den alten Server, statt ihn zu verlieren.
  - Unter Linux leistet das Umbenennen das schon.
  - Unter Windows bräuchte es ein Umbenennen der laufenden `se.exe` zur Seite. Annahme: Windows erlaubt das Umbenennen einer laufenden Programmdatei, nicht ihr Löschen; nicht geprüft.
  - Die Regel V8 „jeder Prozess endet vor dem Update“ gälte dann für `se mcp` nicht mehr.
- **Stufe C, Server ohne Neuverbinden tauschen.** `se mcp` wird ein schlanker, stabiler Vermittler. Er hält die stdio-Verbindung und startet den eigentlichen Server als Kindprozess.
  - Nach einem Update startet er den neuen Server.
  - Dem neuen Server spielt er den `initialize`-Austausch erneut vor.
  - Dem Agenten meldet er `notifications/tools/list_changed`.
  - Laufende Sitzungen bekommen so die neue Version ohne Neuverbinden. Voraussetzung: Der Agent übernimmt `list_changed` zur Laufzeit (siehe Nachprüfung).

### Urteil P13

- **Grundmechanik vorhanden:** Claude Code liest das lokale Plugin bei jedem Sitzungsstart aus dem Ordner. Codex erkennt geänderte Skills selbst. `se mcp` frischt beide Ordner nach einem Update auf. Die automatische Aktualisierung besteht also schon, kommt aber eine Agentensitzung zu spät und lässt Pfadwechsel und laufende Sitzungen aus.
- **Stufe A:** umsetzbar, klein. Wirkung ab der ersten neuen Sitzung nach dem Update, für Claude Code und Codex.
- **Stufe B:** unter Linux umsetzbar, denn das Ersetzen per Umbenennen gibt es schon. Unter Windows hängt sie an der Annahme zum Umbenennen einer laufenden `se.exe` und an einer Änderung der Regel V8 für `se mcp`. Umfang mittel.
- **Stufe C:** umsetzbar, mit Grenzen.
  - Bleibt die Werkzeugliste zwischen zwei Versionen gleich, merkt keiner der beiden Agenten den Tausch.
  - Ändert sie sich, übernimmt Claude Code im interaktiven Terminal die neue Liste (laut Doku; widersprechende Issues).
  - Codex übernimmt sie erst nach einem Neustart, außer der Schalter `mcp_tool_list_changed` ist an.
  - Umfang mittel bis groß; braucht Stufe B.

## P14 Ordner kopieren: Zusatz statt Zusammenführen

### Bestand

- **Desktop:** Einfügen, Ablegen und „Herunterladen nach…“ laufen über `paste_job` (`N/app/core/transfer_route.rs:122-138`) mit der Regel: „occupied names become "Name (2)", local folders merge and remote folders get a new name; nothing is replaced“.
- **Engine:** Neue oberste Ordner legt sie an Remote-Zielen exklusiv an. Ist der Name belegt, bekommen sie eine Nummer; lokale Ordner führt sie zusammen (`N/transfer/os/shared/engine/folders.rs:1-11`, Konfliktregel `N/transfer/core/job.rs:132-134`).
  - Remote-Ziele haben die gewünschte Wirkung also schon.
  - Lokale Ziele führen zusammen und nummerieren gleichnamige Dateien im Inneren. Das ist das „Reinsplicen“ aus der Rückmeldung.
- **„Kopieren/Verschieben nach…“:** Der Dialog fragt „Bei Konflikt: umbenennen / überschreiben / überspringen“ für Dateien (`N/app/core/copy_dialog.rs:59-67`). Annahme nach `folders.rs`: Ordner werden auch hier zusammengeführt.
- **se Desktop:** Einfügen läuft über das Kopiermodul mit `Conflict::Rename` (`N/desktop/os/shared/desktop_ops.rs:137-143`), also über dieselbe Engine.
- **Android:**
  - Vor dem Übertragen prüft Android die obersten Namen im Ziel (`N/mobile/os/shared/fs_edit.rs:112-132`).
  - Lokal → lokal fragt ein Dialog „Überspringen / Ersetzen / Beide behalten“; Remote-Ziele nummerieren (`A/ui/files/FileActions.kt:211-223`).
  - „Beide behalten“ wird zu `Conflict::Rename` (`N/mobile/os/shared/transfer.rs:87`). Annahme nach der Engine-Regel: Bei Ordnern wirkt es auf die Dateien im Inneren, nicht auf den obersten Ordner. Am Gerät nicht geprüft.
- **Terminal `se cp` und MCP-Werkzeug `copy`:**
  - Das Ziel trägt genau den eingegebenen Namen (`N/cli/engine_copy.rs:8-10`).
  - Ist das Ziel eines Ordners ein vorhandener Ordner, schreibt `se cp` hinein (`N/cli/tree_guard.rs:24-26`).
  - Eine vorhandene Datei bricht ohne `--force` ab (`:33`).

### Weg

- **Engine:** Lokale Ziele bekommen dieselbe Regel wie Remote-Ziele heute. Ein oberster Ordner wird exklusiv angelegt und heißt bei belegtem Namen „Name (2)“. Den Baustein dafür gibt es in `folders.rs`. Zusammenführen bleibt als ausdrückliche Wahl.
- **Abfrage auf Desktop, se Desktop und Android:** Vor dem Start werden die obersten Namen im Ziel geprüft, so wie es Android schon tut. Ist ein Ordnername belegt, fragt ein Fenster:
  - „Beide behalten“ mit Zusatz, als Vorgabe;
  - „Zusammenführen“;
  - „Überspringen“.

  Bei mehreren belegten Namen gibt es eine Wahl je Ordner und „für alle übernehmen“. Beim Zusammenführen gilt für gleichnamige Dateien die heutige Regel (nummerieren). Ersetzen bleibt dort, wo es heute angeboten wird: Android lokal → lokal und „Kopieren nach…“.
- **Vorschau:**
  - Für „Beide behalten“ zeigt sie den neuen Namen.
  - Für „Zusammenführen“ zeigt sie die Zahl neuer und gleichnamiger Dateien und die ersten gleichnamigen Pfade.
  - Sie entsteht aus dem Auflisten beider Bäume. Annahme: Bei großen oder entfernten Bäumen kostet das spürbar Zeit, deshalb erscheint sie fortlaufend, während das Fenster schon bedienbar ist.
- **Verschieben:** Für einen Ordner auf einen gleichnamigen gilt dieselbe Abfrage.
- **Terminal und MCP:** Sie haben kein Fenster und bekommen deshalb eine Option für einen vorhandenen Zielordner: zusammenführen, Zusatz oder abbrechen. Die JSON-Ausgabe nennt den tatsächlichen Zielnamen. Die Voreinstellung ist Entscheidung E10.
- **Unverändert:**
  - Sync-Jobs, denn sie sind keine Kopien.
  - „Fehlende übertragen“: Es nutzt die Ziele des früheren Laufs (`N/transfer/core/types.rs:133-142`) und legt keine neuen nummerierten Kopien an.
- **Lücke:** Bei „Ordnerstruktur erhalten“ in „Kopieren nach…“ legt die Kopie Zwischenordner an. Welche Ebene dort den Zusatz bekommt, ist nicht festgelegt. Vorschlag: die erste Ebene unter dem Ziel.

### Urteil P14

Umsetzbar, mittel. Den Engine-Baustein gibt es für Remote-Ziele schon. Neu sind die lokale Regel, die Abfrage mit Vorschau auf Desktop, se Desktop und Android sowie die Option für Terminal und MCP.

## P15 se Desktop: Ordner im se Explorer öffnen

### Bestand

- **Doppelklick und Enter** auf ein Element der Fläche (Befund): Sie laufen über `activate` (`N/app/core/desktop_surface_menu_ui.rs:59-80`).
  - Eine Datei oder ein Ordner der Fläche wird zu `OpenRequest::File` (`N/desktop/os/shared/open_target.rs:155-161`).
  - `launch` gibt Dateien und Ordner an das System (`:180-197`). Unter Linux geht das über das Portal, `gio launch` oder `xdg-open` (`N/app/core/desktop_surface_actions.rs:116-130`), sonst an die Standardanwendung.
  - Ein Ordner öffnet damit im Dateimanager des Systems.
- **Kontextmenü** einer Datei oder eines Ordners (Befund): „Öffnen“ und „Öffnen mit …“, beide über das System (`N/app/core/desktop_surface_menu_ui.rs:165-170`).
- **Tresore und Geräte** öffnen dagegen im eigenen Fenster, über den Startauftrag `StartRequest::OpenLocation` (Tresore: `open_target.rs:125-129`).
  - Für Geräte wählt die Einstellung „Geräte öffnen mit“ den Weg (`open_devices_with`, Vorgabe `App`, `N/desktop/core/config.rs:146,164`). Im Terminal geht das über `se desktop config` (`docs/oberflaechen/desktop-launcher.md`, Zeile O4).
- **„Papierkorb“ und „Dieser PC“** gehen als Shell-Ordner an das System (`open_target.rs:150-153`).
- **Android** (Befund): Der se Launcher kennt die Elementarten `app`, `shortcut`, `folder`, `widget`, `seDevice`, `seRoom`, `seVault`, `seScreens`, `seAddDevice` und `seJoinRoom` (`A/launcher/model/LayoutModel.kt:21-31`). Dateien und Dateiordner gibt es dort nicht. Annahme: `folder` ist ein Launcher-Ordner für Apps.

### Weg

- **Ordner öffnen im se Explorer:** Ein Ordner der Fläche öffnet als Fenster des se Explorers, über den Startauftrag „Ort öffnen“ mit dem Ordnerpfad, wie schon Tresore und Geräte. Annahme: Der Startauftrag nimmt einen lokalen Pfad an.
- **Unverändert:** Dateien öffnen weiter mit ihrer Standardanwendung. „Papierkorb“ und „Dieser PC“ bleiben beim System.
- **Systemweg bleibt erreichbar:**
  - Im Kontextmenü über „Öffnen mit …“ und über einen Eintrag „Im Explorer öffnen“ bzw. „Im Dateimanager öffnen“. Die Beschriftung gibt es schon (`desktop_surface_menu_ui.rs:51-57`).
  - Über eine Einstellung „Ordner öffnen mit“ neben „Geräte öffnen mit“: Smart Explorer als Vorgabe, oder der Dateimanager des Systems. Sie steht in den Einstellungen, in `se desktop config` und über MCP mit dem Werkzeug `se`.
- **Android:** entfällt.

### Urteil P15

Umsetzbar, klein.

## Plattformprüfung

| Punkt | Desktop Windows | Desktop Linux | Android | Terminal und MCP |
|---|---|---|---|---|
| P10 | Ereignisse und Neulesen | neue Datenträgerliste über UDisks2 | laut Code live; am Gerät prüfen | nicht betroffen (keine Laufwerksliste) |
| P11 Stufe 1 | ein Leser, eigener Leseweg statt `CopyFile2` | ein Leser | entfällt, `vold` bindet `sr` nicht ein | Annahme: `se cp` nutzt dieselbe Engine und damit dieselbe Regel |
| P13 | Updater frischt auf (A). Laufende `se.exe` umbenennen (B, Annahme) | Updater und `se update` frischen auf (A). Ersetzen per Umbenennen gibt es schon (B) | entfällt, Android hat kein `se mcp` (`docs/AI_AGENT_PLUGIN.md`, Abschnitt Grenzen) | betrifft `se mcp` selbst |
| P14 | Abfrage mit Vorschau, auch im se Desktop | Abfrage mit Vorschau, auch im se Desktop | bestehender Dialog, erweitert um Ordner | Option statt Abfrage (E10) |
| P15 | Doppelklick öffnet den se Explorer | Doppelklick öffnet den se Explorer | entfällt, der se Launcher zeigt keine Dateien | Einstellung über `se desktop config` |

## Entscheidungen vor einer Umsetzung

Die Nummern setzen E1 bis E6 der Notiz zu Fernzugriff und Desktop fort. E7 und E8 (Quellenweg und Umfang von P12) entfallen, weil P12 zurückgestellt ist.

- **E9, Stufen von P13:** A; A und B; oder A, B und C. Vorschlag: A und B.
- **E10, Terminal und MCP bei P14.** Zwei Möglichkeiten:
  - Das heutige Zusammenführen in einen ausdrücklich genannten Zielordner bleibt Voreinstellung, und die Option kommt dazu. Das ist der Vorschlag, weil der Zielname dort getippt wird.
  - Der Zusatz wird neue Voreinstellung, wie in den Fenstern.
- **E11, Reihenfolge im nächsten Lauf.** Vorschlag:
  1. P10, P13, P14 und P15;
  2. P11 Stufe 1 nach der Messung.

  P12 und die Stufen 2 und 3 von P11 bleiben draußen.

## Abhängigkeiten

- P11 Stufe 2 und Stufe 3 brauchen den Dateisystemleser und den Rohzugriff aus P12 und ruhen deshalb mit P12.
- P13 Stufe C braucht Stufe B.

## Lücken

1. **P10:**
   - Sendet Windows `WM_DEVICECHANGE` für Volumes, oder postet es sie?
   - Meldet `CM_Register_Notification` das Einlegen einer Disc?
   - Gibt es Broadcasts für verbundene Netzlaufwerke?
2. **Android am Gerät** ungeprüft: die Live-Liste aus P10 und „Beide behalten“ bei Ordnern aus P14.
3. **P11:**
   - Hochlaufzeit, Cache und Read-Ahead gängiger Laufwerke;
   - Cache von CDFS und UDFS unter Windows;
   - Wiederholungen und Zeitlimit von `cdrom.sys`;
   - ob `cdfs.sys` `FSCTL_GET_RETRIEVAL_POINTERS` kennt.
4. **P12:** ob der Hersteller von IsoBuster Komponenten lizenziert.
5. **P13:**
   - wann Codex seine MCP-Konfiguration liest;
   - Konfigurationsweg und erste Version des Schalters `mcp_tool_list_changed`;
   - ob Claude Code `list_changed` in der aktuellen Version zuverlässig übernimmt;
   - ob Windows das Umbenennen einer laufenden `se.exe` erlaubt.
6. **P14:** welche Ebene bei „Ordnerstruktur erhalten“ den Zusatz bekommt.
7. **P15:** ob der Startauftrag „Ort öffnen“ einen lokalen Ordnerpfad annimmt.
