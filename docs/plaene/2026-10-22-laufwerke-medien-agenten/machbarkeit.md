# Laufwerke, Medien, Agenten, Kopieren und se Desktop: Machbarkeit (2026-10-22)

## Auftrag

Nachträge vom 2026-10-22, getrennt von der Rückmeldung zu Fernzugriff und se Desktop
(`docs/plaene/2026-10-22-fernzugriff-desktop/machbarkeit.md`, P1 bis P9). Der Auftrag:
- P10 und P11 aufschreiben; sie sollen „beim nächsten Lauf mitgemacht werden“;
- P12 kam mit P10 und P11, bleibt aber vorerst draußen (Nutzer, 2026-10-22);
- P13 als Ergänzung dokumentieren. Annahme: P13 gehört in denselben nächsten Lauf;
- P14 und P15 kamen danach ohne Zeitangabe. Zu P15 sagte der Nutzer „noch nicht umsetzen“. Annahme: beide gehören in den nächsten Lauf und sind bis dahin nur dokumentiert;
- P16 bis P20 kamen danach als Zusätze, ebenfalls ohne Zeitangabe. Annahme: nächster Lauf, bis dahin nur dokumentiert;
- P21 ist ein Fehlerbild mit Frage: Gibt es schon eine Lösung? Wenn nicht, ist es ein weiterer Punkt, mit der Vorgabe A (ohne Abfrage) vor B (Häkchen „immer“);
- P22 bis P24 sind weitere Fehlerbilder und Wünsche ohne Zeitangabe. Annahme: nächster Lauf. Für P22 bekam der Nutzer zusätzlich eine Anleitung für das eigene Gerät; sie steht nicht in diesem Repository;
- noch nichts umsetzen. Wie bei P1 bis P9: einschätzen, ob und wie jeder Punkt umsetzbar ist, ohne konkrete Implementierungsplanung und ohne Fehlersuche.

Die Punkte stehen als Zeilen in `docs/TODO.md`:
- `LAUFWERKE-LIVE`
- `OPTISCH-LESEN`
- `OPTISCH-DATENTRAEGER` (zurückgestellt)
- `AGENT-PLUGIN-UPDATE`
- `KOPIE-ORDNER-ZUSATZ`
- `DESKTOP-ORDNER-EXPLORER`
- `AUSWAHL-HOEHE`
- `FENSTER-GROESSE`
- `EINSTELLUNGEN-GERAETE`
- `EINSTELLUNGEN-TECHNIK`
- `EINSTELLUNGEN-AUSRICHTUNG`
- `TRESOR-OEFFNEN-WURZEL`
- `LAUFWERK-RESTEINTRAG`
- `LAUFWERK-PLATZ`
- `LESERECHTE-NUR-NEU`

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
| P16 | Das Auswahlfenster, das aufpoppt, öffnet in der Höhe immer sehr eng; es kann etwa doppelt so groß sein |
| P17 | Fenster allgemein in der Größe veränderbar machen |
| P18 | Die Einstellungen für die Freigaben sind schwer zu finden, weil sie über Untermenüs laufen. Einstellungen sinnvoller gruppieren: eigenes Gerät, Geräte und von dort aus deren Zugriffe. Die Rechte zentral je Gerät einstellen, nicht je Funktion an vielen Stellen |
| P19 | IDs, Exec-IDs, Kodierungsart und Ähnliches gehören nicht in die Einstellungsfenster, dafür sind Logs da; außer eine Angabe hat einen praktischen Nutzen |
| P20 | In den Einstellungsfenstern tabellarisch ausrichten, statt die Bedienelemente je nach Länge der Beschriftung versetzt zu haben. Bei mehreren gleichen Auswahlmöglichkeiten untereinander nicht jedes Mal den Optionsnamen wiederholen. Sinnvolles, lesbares, intuitives Design |
| P21 | Eine Datei lässt sich nicht im Tresor-Laufwerk öffnen: „Laufwerk-Wurzel absichern: backend does not technically enforce the selected mount root; explicit trusted-root mode is required“. Dass jedes Mal ein Fenster fragt, ist inakzeptabel. A: so umsetzen, dass keine Abfrage nötig ist. B: wenn nicht anders möglich, ein Häkchen, das immer bestätigt |
| P22 | Bei jedem Start meldet ein Fehler „Laufwerk nach Daemon-Neustart zur Wiederherstellung bereit; bitte erneut versuchen“ für einen Laufwerkseintrag mit offenen Änderungen, hinter dem kein Laufwerk mehr steht |
| P23 | Ein eingebundenes Laufwerk zeigt unter „Dieser PC“ den freien und den gesamten Platz der eigenen Festplatte |
| P24 | Nach dem Anfragen von Leserechten für Admin-Bereiche sollen nur diese Bereiche neu gelesen werden |

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
| P16 | umsetzbar, klein | Liste der Ordnerauswahl füllt das Fenster; Startgröße nach der Bildschirmhöhe | welches Fenster gemeint ist (Lücke) |
| P17 | umsetzbar, klein bis mittel | alle Dialoge veränderbar, Listen wachsen mit, Größe wird gemerkt | 14 gesperrte Dialoge und 18 Listen mit fester Höhe |
| P18 | umsetzbar, groß | ein Fenster „Einstellungen“; je Gerät eine Seite mit allen Rechten; Funktionsreiter nur als Übersicht | beide Apps; Gliederung (E12) |
| P19 | umsetzbar, klein bis mittel | IDs, Revisionen und Kodierungsnamen aus den Fenstern in Protokolle und Diagnose | den Nutzen einzelner Angaben abwägen |
| P20 | umsetzbar, mittel | gemeinsamer Formularbaustein (Raster), Tabellen mit Kopfzeile für gleichartige Zeilen | viele Seiten |
| P21 | B umsetzbar, klein; A je nach Speicherart | A: Wurzelprüfung erfüllen. B: Vertrauen je Tresor merken | Sicherheitsprüfung je Speicherart für A |
| P22 | umsetzbar, klein bis mittel | Aktion „Änderungen sichern und entfernen“ | offene Änderungen dürfen nicht verloren gehen |
| P23 | umsetzbar, mittel | Platz der Gegenseite melden, begrenzt durch den Zwischenspeicher | je Speicherart eine Abfrage; FTP ohne Standard |
| P24 | umsetzbar, mittel | gesperrte Teilbäume vollständig merken, nach der Zustimmung nur sie neu lesen und einfügen | heutige Obergrenzen der Fehlerlisten (64, 100, 500) |

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

## P16 Auswahlfenster öffnet zu niedrig

### Bestand

Welches Auswahlfenster gemeint ist, sagt die Rückmeldung nicht (Lücke). Kandidaten (Befund):

- **Ordnerauswahl** mit den Titeln „Quelle wählen“, „Ziel wählen“, „Ordner öffnen“, „Ordner für Analyse“, „Ordner fuer Aufraeumen“, „Ziel zum Spiegeln“, „Ziel für 2-Wege-Sync“, „Zielordner wählen“ und „Speichern unter…“ (`N/app/core/picker_types.rs:30-41`):
  - Das Fenster ist veränderbar und startet mit 760 × 560 (`N/app/core/picker_impl.rs:134-140`).
  - Seine Ordnerliste begrenzt die Höhe auf die verfügbare Höhe minus 60, mindestens 80 px (`:246-249`). Sie steht in einer waagrechten Zeile (`:146`).
  - Annahme, nicht gemessen: In einer waagrechten Zeile startet egui mit der Höhe eines Bedienelements. Dann bliebe die Liste bei 80 px, gleich wie groß das Fenster ist. Das passte zu „immer echt eng“.
- **Aufklapplisten:** Sie sind auf 280 px Höhe gesetzt (`N/app/core/theme.rs:73`).
- **Dateidialog des Systems unter Linux:** Er läuft über das Portal; seine Größe bestimmt das Portal (Annahme).

### Weg

- Die Liste der Ordnerauswahl füllt die Fensterhöhe, statt sie aus der Zeile abzuleiten.
- Die Startgröße richtet sich nach der Bildschirmhöhe, etwa doppelt so hoch wie heute, wo der Bildschirm das hergibt.
- Für Aufklapplisten mit vielen Einträgen gilt eine größere Höhe.

### Urteil P16

Umsetzbar, klein. Lücke: welches Fenster gemeint ist. Annahme: die Ordnerauswahl.

## P17 Fenster allgemein in der Größe veränderbar

### Bestand (Befund)

- In `N/app/` gibt es 45 Aufrufe `Window::new` in 36 Dateien. Annahme nach der Voreinstellung von egui: Fenster ohne eigene Angabe sind veränderbar.
- 14 Fenster in 11 Dateien sperren die Größe mit `.resizable(false)`:
  - „Synchronisierung abschließen“ (`N/app/os/shared/sync_exit_gate.rs:64-66`);
  - „SSH-Hostschlüssel prüfen“ (`N/app/core/host_key_ui.rs:71-73`);
  - der Tresor-Prüfdialog (`N/app/core/vault_check_ui.rs:183-186`);
  - „Synchronisieren – <Ort>“ (`N/app/core/vault_sync_editor_ui.rs:127-130`);
  - „Tresor hinzufügen“ (`N/app/core/vault_add_ui.rs:102-105`);
  - der Tresor-Vergleich (`N/app/core/vault_compare_ui.rs:365-368`);
  - „Tresor austragen?“ (`N/app/core/vault_actions_ui.rs:156-159`);
  - „Neuer Tresor“ (`N/app/core/vault_create_ui.rs:231-234`);
  - der Verbindungsdialog (`N/app/core/connect_dialog_ui.rs:54-57`);
  - „Umbenennen: <Name>“ (`N/app/core/dialogs.rs:12-15`);
  - „Update bereit“ (`:61-63`);
  - „Hinweis & Haftungsausschluss“, dazu mit fester Breite 560 (`:146-150`);
  - „Tresor sperren?“ (`N/app/core/vault_unlock_ui.rs:163-166`);
  - „Tresor entsperren“ (`:261-264`).
- 18 Listen in 14 Dateien haben eine feste Höchsthöhe und wachsen deshalb nicht mit dem Fenster. Die Werte in px, alle unter `N/app/core/`:
  - `share_diagnostics_ui.rs:75` (420);
  - `shell_commands.rs:45` (360) und `:55` (420);
  - `settings_updates.rs:154` (160) und `:200` (140);
  - `bisync_ui.rs:83` (140);
  - `merge_ui.rs:44` (420);
  - `transfer_window.rs:315` (160);
  - `sync_versions_ui.rs:142` (310);
  - `vault_compare_detail_ui.rs:108` (260), `:149` (200) und `:196` (160);
  - `dialogs.rs:154` (420);
  - `analytics_access.rs:77` (180);
  - `analytics_ui.rs:30` (160);
  - `analytics_accessibility.rs:32` (180);
  - `share_screen_codec_ui.rs:250` (380);
  - `share_exec_log_ui.rs:115` (360).
- Eigene Fenster des Systems: Kein Aufruf `with_resizable(false)` in `N/`.

### Weg

- Alle Dialoge werden veränderbar und bekommen eine Mindestgröße.
- Listen füllen die verfügbare Höhe, statt eine feste Höchsthöhe zu haben.
- Fenster merken sich ihre Größe. Lücke: ob die Desktop-App die Fenstergrößen von egui heute über einen Neustart hinweg speichert.
- Android: entfällt. Annahme: Dialoge auf dem Telefon passen sich dem Bildschirm an.

### Urteil P17

Umsetzbar, klein bis mittel. Viele Dialoge sind betroffen, und je Dialog muss der Inhalt mitwachsen.

## P18 Einstellungen gruppieren, Rechte zentral je Gerät

### Bestand (Befund aus einer Inventur vom 2026-10-22, Stichproben am Code bestätigt)

- **Zwei getrennte Fenster am Desktop:**
  - „Einstellungen“ hat die Seiten Darstellung, Verbindungen (Share-Server, Gerätename, Google Drive), Updates, Suche & Daten und Integration (KI-Agenten, Hintergrund-Sync, verbundene Geräte als Ordner, se Desktop) (`N/app/core/settings_ui.rs:31`, `:39-47`).
  - „Geräte & Freigaben“ hat die Reiter Geräte, Anfragen, Räume, Freigaben, Befehle, Bildschirm, Dieses Gerät, Netzwerk, Diagnose und Papierkorb (`N/app/core/share_window_ui.rs:9`, `:21-40`; `N/app/core/share_pages_ui.rs:26-52`).
  - Lücke: der Weg vom Hauptfenster zu „Geräte & Freigaben“.
- **Rechte eines Geräts an mehreren Stellen (Desktop):**
  - **Geräteseite** (Geräte → Zeile → „Details“):
    - Rückfreigabe (`N/app/core/share_device_page_ui.rs:181-186`);
    - automatisch verbinden und öffnen (`:251-256`);
    - Vertrauen zurücksetzen (`:269`);
    - Autorisierung, „Darf schreiben“ und Zugriff (`:357-395`).
  - **Reiter Befehle:** dieselbe Zugriffseinstellung je Gerät über „Ändern…“ (`N/app/core/share_exec_ui.rs:76`, `:150`). Der Dialog „Zugriff für …“ (`N/app/core/share_access_dialog_ui.rs:354`, Zeilen `N/app/core/share_exec_programs_ui.rs:270-423`) umfasst:
    - Befehle und Terminal;
    - Bildschirm ansehen und steuern (Zeilen aus dem Katalog, `:358`);
    - Konto, Programmliste, Gültigkeit und Rückfrage.
  - **Reiter Freigaben:** „Darf schreiben“ je Gerät ein zweites Mal (`N/app/core/share_policy_ui.rs:52`, `:81`). Beide Stellen schreiben denselben Wert.
  - **Reiter Anfragen:** die Anfrageregel (`N/app/core/share_policy_ui.rs:8-45`).
  - **Reiter Bildschirm:** nur Rückfragen und laufende Sitzungen (`N/app/core/share_screen_tab_ui.rs:107-148`). Die Rechte stehen im Zugriffsdialog.
  - **Raumseite:** Raumrechte und Zugriff je Mitglied (`N/app/core/share_room_page_ui.rs:137-144`, `:233-248`).
- **Android:**
  - Ein Bildschirm „Teilen“ mit 14 Blöcken (`A/ui/share/ShareScreen.kt:203-217`).
  - Die Rechte eines Kontakts liegen an diesen Stellen:
    - Befehlszeile mit „Erlauben…“ und „Entziehen“ (`A/ui/share/ExecHostSection.kt:198-202`);
    - Rechte-Block (`A/ui/share/SharePolicySections.kt:35-37`);
    - Verbindungsdialog (`A/ui/share/ShareDeviceSettingsDialogs.kt:25-46`);
    - Dialog zur Rückfreigabe (`A/ui/share/ShareRightsDialogs.kt:98-107`);
    - Zugriffsdialog (`A/ui/share/AccessDialog.kt:296-420`).
  - Die Einstellungsliste hat die Einträge Darstellung, Dateiliste, Hintergrund, Share-Server, Google Drive, Ordner für Apps, Speicherzugriff, Startbildschirm, Updates, Fehlerprotokoll und Über (`A/ui/settings/SettingsScreen.kt:52`, `:84`).
- **Vorhandene Grundlage:** Der Zugriffsdialog bündelt auf beiden Plattformen schon alle Zugriffsrechte eines Geräts.

### Weg

- **Ein Fenster „Einstellungen“ mit Bereichen.** Vorschlag:
  1. **Dieses Gerät:** Name, Direkt-Code, Fingerabdruck, Erreichbarkeit, Share-Server, eigene Ordnerfreigaben, Konten dieses Geräts und die Anfrageregel.
  2. **Geräte:** eine Liste, und je Gerät eine Seite mit allen Rechten an einer Stelle. Dazu gehören:
     - Dateien sehen, schreiben und Rückfreigabe;
     - Befehle und Terminal;
     - Bildschirm ansehen und steuern;
     - Konto, Gültigkeit und Rückfrage;
     - die Verbindung, die offenen Anfragen dieses Geräts und „Entfernen“.
  3. **Räume:** je Raum die Mitglieder und Raumrechte. Die Geräteseite nennt, welche Rechte aus einem Raum kommen. Annahme: Raumrechte gelten zusätzlich zu den Geräterechten.
  4. **Die übrigen Bereiche:** Verbindungen (Remotes, Google Drive), Sync, Tresore, Laufwerke, Integration (se Desktop, KI-Agenten), Darstellung, Updates und Hintergrund.
  5. **Diagnose und Protokolle** als letzter Bereich. Dorthin kommt, was P19 aus den übrigen Seiten nimmt.
- **Funktionsreiter werden Übersichten:** „Befehle“, „Bildschirm“ und „Freigaben“ zeigen nur noch, wer was darf. Ihre Zeilen führen auf die Geräteseite; geändert wird nur dort. Offene Rückfragen bleiben oben sichtbar, weil sie eine Antwort brauchen.
- **Android und Terminal:** Android bekommt dieselbe Gliederung. Im Terminal kommt ein Befehl dazu, der alle Rechte eines Geräts zeigt und setzt; die heutigen Einzelbefehle bleiben (Annahme).
- **Werte bleiben:** Die vorhandenen Einstellungen werden nur verschoben. Rechte, ihre Werte und ihre Speicherorte ändern sich nicht.

### Urteil P18

Umsetzbar, groß. Betroffen sind beide Apps, der Kern bleibt unverändert. Die Gliederung ist Entscheidung E12.

## P19 Technische Angaben aus den Einstellungsfenstern

### Bestand (Befund aus derselben Inventur)

**Ohne praktischen Nutzen in Einstellungs- und Freigabefenstern, Desktop:**
- die Geräte-ID im Zugriff-Abschnitt (`N/app/core/share_exec_ui.rs:118`) und in „Darf schreiben“ je Gerät (`N/app/core/share_policy_ui.rs:73`);
- die Revision des Zugriffs (`share_exec_ui.rs:142`);
- auf den Job-Karten:
  - Exec-ID, Command-Digest und Policy-Revision (`N/app/core/share_exec_jobs_ui.rs:146-149`);
  - die Geräte-ID der Gegenstelle, wenn der Name fehlt (`:296-298`);
  - ein Unix-Zeitstempel als Rückfall (`:324`);
- Dateinamen und Größen in der Beschreibung des Exec-Protokolls (`N/app/core/share_exec_log_ui.rs:51`);
- die Diagnosezeilen auf Geräte- und Raumseite (`N/app/core/share_device_page_ui.rs:246`, `:292`; `N/app/core/share_room_page_ui.rs:263-266`);
- die Kodierung: Codec-Schlüssel mit Profilen (`N/app/core/share_screen_codec_ui.rs:85-93`) und die OpenH264-Version (`:170-176`);
- in der Sitzungszeile die Quelle und der Zustandsschlüssel (`N/app/core/share_screen_tab_ui.rs:50-65`);
- die Platzhalter „SE-D3-...“ und „SE-R3-...“ (`N/app/core/share_direct_ui.rs:24`, `N/app/core/share_rooms_ui.rs:76`);
- die Version des Kontodiensts (`N/app/core/share_access_accounts_ui.rs:227`);
- der Wayland-Hinweis mit Umgebungsvariable (`N/app/core/share_screen_portal_ui.rs:22-23`, `:102`);
- englische Warnbegriffe in deutscher Oberfläche (`N/app/core/share_exec_programs_ui.rs:184-192`).

**Ohne praktischen Nutzen, Android:**
- im Schreibrecht-Dialog Geräte-ID, Knoten-ID und der vollständige öffentliche Schlüssel (`A/ui/share/ShareRightsDialogs.kt:86`);
- im Raum-Dialog Profil- und Raum-ID (`:134`);
- „LAN: <Rohwert>“ (`A/ui/share/ShareDeviceCard.kt:82`);
- der Share-Server-Hinweis mit Umgebungsvariablen und Befehlsflags (`A/ui/settings/ShareServerSettings.kt:77-79`).

**Mit praktischem Nutzen; bleibt:**
- Fingerabdrücke zum Abgleich beim Koppeln (`N/app/core/share_device_page_ui.rs:229-231`, `N/app/core/share_this_device_ui.rs:87-89`, `A/ui/share/ShareDeviceCard.kt:121`);
- der eigene Direkt-Code zum Weitergeben (`N/app/core/share_this_device_ui.rs:81-83`);
- Programm und Argumente einer Rückfrage als Entscheidungsgrundlage (`N/app/core/share_access_approvals_ui.rs:130-141`);
- der Exitcode eines Befehls (`A/ui/share/ExecDialog.kt:255`).

**Nutzen unklar,** beim Umbau zu entscheiden: die Adresse des Share-Servers, das Kennzeichen Hardware/Software der Kodierung, Bilder/s und kbit/s einer Sitzung.

### Weg

- Angaben ohne Nutzen kommen aus den Fenstern heraus. Sie bleiben in den Protokollen und im Bereich „Diagnose“ (P18).
- Angaben mit Nutzen bleiben, gekürzt und mit Kopierknopf, wo man sie weitergibt.
- Englische Begriffe werden übersetzt. Annahme: Die Oberfläche soll durchgehend deutsch sein.
- Terminal und MCP sind nicht betroffen, denn dort spricht man Einträge über IDs an (Annahme).

### Urteil P19

Umsetzbar, klein bis mittel. Sinnvoll zusammen mit P18 und P20.

## P20 Ausrichtung und lesbares Design der Einstellungsfenster

### Bestand (Befund)

- **Keine Raster in den Einstellungen:** In 13 Dateien der Einstellungs- und Freigabefenster stehen Beschriftung und Bedienelement in 51 waagrechten Zeilen (`ui.horizontal`), in keiner davon ein Raster (`Grid`).
  - Die Dateien unter `N/app/core/`: `settings_ui.rs`, `settings_background.rs`, `settings_updates.rs`, `settings_cloud.rs`, `settings_ai_plugin.rs`, `settings_devices_ui.rs`, `share_device_page_ui.rs`, `share_exports_ui.rs`, `share_exec_ui.rs`, `share_access_dialog_ui.rs`, `share_this_device_ui.rs`, `share_lan_ui.rs` und `share_policy_ui.rs`.
  - In einer solchen Zeile beginnt das Bedienelement dort, wo seine Beschriftung endet. Daher kommt der versetzte Eindruck.
- **Bausteine sind vorhanden:**
  - `egui::Grid` wird 14-mal in 10 anderen Dateien unter `N/app/` genutzt.
  - `egui_extras` 0.29 ist schon Abhängigkeit (`native/Cargo.toml:317`); seine Tabelle `TableBuilder` wird 2-mal genutzt.

### Weg

- **Formularbaustein:** Einstellungsseiten nutzen einen gemeinsamen Baustein, ein zweispaltiges Raster mit Beschriftungsspalte und Bedienspalte. Dazu kommen Abschnittsüberschriften und einheitliche Abstände.
- **Gleichartige Zeilen als Tabelle:** Zeilen untereinander, etwa je Gerät oder je Freigabe, werden eine Tabelle mit einmaliger Spaltenüberschrift. Die Optionsbezeichnung steht dann nicht mehr in jeder Zeile. Baustein: `TableBuilder`, oder ein Raster mit Kopfzeile.
- **Gilt für alle Einstellungs- und Freigabefenster.** Sinnvoll zusammen mit P18 und P19, weil dieselben Seiten neu entstehen.
- **Android:** Die Einstellungen bestehen aus Compose-Zeilen und Listenelementen (`Row` und `ListItem` in `A/ui/settings/` und `A/ui/share/`).
  - Lücke: ob dort dasselbe Problem besteht.
  - Annahme: Die Rückmeldung betrifft die Fenster der Desktop-App.

### Urteil P20

Umsetzbar, mittel. Viele Seiten sind betroffen; ein gemeinsamer Baustein hält sie einheitlich.

## P21 Tresor-Datei öffnen: Wurzelprüfung und Abfrage bei jedem Öffnen

### Bestand (Befund)

- **Ablauf beim Öffnen:** Eine Tresor-Datei öffnet im Tresor-Laufwerk. Ist kein Laufwerk eingehängt und „beim Öffnen einhängen“ an (Voreinstellung an, `N/app/core/vault_ui_settings.rs:23,29`), hängt `ensure_mounted_for` den Tresor ein (`N/daemon/os/shared/mount_paths.rs:189-235`).
  - Scheitert das, fragt die App „Klartextkopie öffnen?“ (`N/app/os/shared/vault_open.rs:35-64`, Aufruf `:261`).
  - Die Frage hat keine Möglichkeit „immer“ und kommt bei jedem Öffnen wieder. Bei „Ja“ bleibt eine unverschlüsselte Kopie bis zum Beenden in den App-Daten.
- **Warum das Einhängen scheitert:**
  - Das Einhängen beim Öffnen nutzt `MountConfig::new` mit der Voreinstellung `MountRootSecurity::Enforced` (`N/mount/core/types.rs:192-196`, `:236`). Ebenso tut es „Als Laufwerk einhängen“ in den Tresor-Aktionen (`N/app/core/vault_actions_ui.rs:62-90`).
  - Im strengen Modus verlangt der Laufwerk-Host, dass der Speicher die Wurzel technisch eingrenzt. Sonst kommt genau die gemeldete Meldung (`N/daemon/os/shared/rooted_backend.rs:66-70`, Vorsatz „Laufwerk-Wurzel absichern“ aus `N/daemon/os/shared/mount_manager_start.rs:217`).
  - Der Tresor reicht die Eingrenzung seines Speichers durch (`N/vault/os/shared/backend.rs:276-282`). Ein Speicher ohne eigene Zusicherung meldet „Unverified“ (`N/vfs/core/core.rs:315-319`).
  - SFTP im strengen Modus verlangt einen eingegrenzten Gegenagenten (`N/connect/os/shared/connector.rs:30-40`).
  - Annahme: Betroffen sind Tresore auf WebDAV, FTP und Google Drive, sowie auf SFTP ohne eingegrenzten Gegenagenten.
- **Vorhandene Lösung, nur für normale Remote-Laufwerke:**
  - das Häkchen „Dieser ausgewaehlten Remote-Wurzel ausdruecklich vertrauen“ im Dialog „Remote als Laufwerk“ (`N/app/core/mount_ui_draft.rs:177`, `:238-241`);
  - im Terminal `se drive mount … --trust-remote-root` (`N/cli/drive.rs:194-198`).

  Der Vertrauensmodus heißt: Ein anderer Schreiber am Remote kann die Wurzel während eines Pfadzugriffs verändern (`N/app/core/mount_ui_draft.rs:406`).
- **Für Tresor-Laufwerke bietet die App diesen Modus nicht an.** Das Terminal nimmt `vault://<SITE>` als Laufwerksquelle an (`N/cli/drive.rs:362-379`), und `--trust-remote-root` gilt dort für jede Quelle.
  - Laut Code hängt `se drive mount vault://<SITE> --trust-remote-root` einen Tresor also im Vertrauensmodus ein.
  - Nicht erprobt: ob „Öffnen“ danach dieses Laufwerk nutzt. `ensure_mounted_for` nimmt das erste Laufwerk des Tresors aus der Liste; ein älterer, strenger Eintrag könnte zuerst kommen.
- **Klarstellung des Nutzers:** Beim Tresor erscheint kein Vertrauensdialog und kein Häkchen. Das passt zum Befund oben.
  - Die Klartextkopie-Frage ist unter Windows ein Meldungsfenster mit „Ja“ und „Nein“ (`N/app/os/windows/platform.rs:6-20`), ohne „immer“.

### Weg

Die Rangfolge hat der Nutzer vorgegeben: A vor B.

- **A, ohne Abfrage.** Tresor-Laufwerke so einhängen, dass die Wurzelprüfung erfüllt ist:
  - Wo der Speicher eingrenzbar ist, die eingegrenzte Fassung nutzen: SFTP mit eingegrenztem Gegenagenten, lokaler Speicher.
  - Prüfen, ob die Tresorschicht selbst eine Eingrenzung zusichern kann. Sie greift nur auf Pfade unter dem Tresorordner zu, die sie aus verschlüsselten Namen bildet.
  - Lücke: ob ein fremder Schreiber am Speicher die Wurzel trotzdem verlassen kann, etwa mit einem symbolischen Link an Stelle eines Tresorordners. Das braucht eine Sicherheitsprüfung je Speicherart.
- **B, wo A nicht geht.** Ein Häkchen am Tresor „Diesem Speicherort immer vertrauen“ merkt den Vertrauensmodus für das Tresor-Laufwerk, je Tresor und in den Tresor-Einstellungen widerrufbar.
  - Dateien öffnen dann im Laufwerk, ohne Klartextkopie und ohne Abfrage.
  - Nur wo gar kein Laufwerk möglich ist, etwa ohne Dokany- oder FUSE-Laufzeit, bekommt die Klartextkopie-Frage ein „Immer für diesen Tresor“.
- **Android:** hat keine Laufwerke. Lücke: wie Android Tresor-Dateien öffnet und ob dort eine ähnliche Abfrage kommt.

### Urteil P21

- **A:** umsetzbar für eingrenzbare Speicher. Für die übrigen Speicher hängt es an der Sicherheitsprüfung.
- **B:** umsetzbar, klein. Der Vertrauensmodus und seine Speicherung im Laufwerkseintrag gibt es schon. Neu sind das Häkchen am Tresor und das Merken je Tresor.

## P22 Laufwerkseintrag mit offenen Änderungen lässt sich nicht entfernen

### Bestand (Befund)

- **Die Rückmeldung:** Bei jedem Start von Smart Explorer meldet ein Fehler: „Laufwerk nach Daemon-Neustart zur Wiederherstellung bereit; bitte erneut versuchen“.
  - Der Eintrag zeigt „Recovery: Aenderungen oder Konflikte offen“ und „Beim Anmelden automatisch einbinden“.
  - Laut Nutzer steht kein Laufwerk mehr dahinter.
- **Der Text** ist der Zustand eines registrierten Laufwerks nach einem Start des Hintergrunddienstes, bis zur Wiederherstellung oder bis „Erneut versuchen“ (`N/daemon/os/shared/mount_manager.rs:15-18`).
- **„Aenderungen oder Konflikte offen“** heißt: Im Zwischenspeicher des Laufwerks liegen Journal-Einträge, die noch nicht zurückgeschrieben sind, nämlich Änderungen, Löschungen und Namenskonflikte (`N/mount/core/spool.rs:268-283`, Anzeige `N/app/core/mount_ui_helpers.rs:95`).
  - Der Zwischenspeicher liegt unter `<AD>/mount-cache/<Laufwerks-ID>` (`N/mount/core/spool.rs:94`, `N/daemon/os/shared/mount_manager_start_cache.rs:86`).
  - `<AD>` ist unter Windows `%APPDATA%\smart_explorer` (`docs/oberflaechen/laufwerke.md:7`).
- **Entfernen ist mit offenen Änderungen absichtlich gesperrt:**
  - „Entfernen“ zeigt die App nur bei sauberem Zustand (`N/app/core/mount_ui.rs:256-266`).
  - `se drive unmount` lässt einen solchen Eintrag stehen (`N/daemon/os/shared/mount_manager.rs:173-175`).
  - `se drive set <ID> --auto-remount off` schaltet nur das Einbinden beim Anmelden ab (`N/cli/drive_set.rs:21-27`).
  - In App und Terminal fehlt ein Weg, die offenen Änderungen zu sichern und den Eintrag danach zu entfernen.
- **Die Einträge** stehen in `<AD>/mounts/registry.json` (gespeicherte Verbindungen, Google Drive, Share-Geräte) und `<AD>/mounts/registry-2.json` (Gerätebaum, Tresore) (`N/daemon/os/shared/mount_registry.rs:1-11`, `:24`; `N/daemon/os/shared/mount_registry_ext.rs:21`).

### Weg

- **Neue Aktion „Änderungen sichern und entfernen“:** am Eintrag in der App, im Terminal und über MCP mit dem Werkzeug `se`.
  - Sie schreibt die offenen Dateien aus dem Zwischenspeicher in einen gewählten Ordner. Bei Tresor-Laufwerken geht das nur entsperrt, weil der Zwischenspeicher sonst versiegelt ist (Annahme nach `N/mount/core/spool.rs:279-281`).
  - Sie zeigt Löschungen und Konflikte als Liste.
  - Danach entfernt sie Eintrag, Zwischenspeicher und Anmeldeeintrag.
- **Mit vorhandener Quelle** bleibt „Erneut versuchen“ der Weg ohne Verlust.
- **Die Meldung beim Start** wird ein Hinweis am Eintrag statt eines Fehlers bei jedem Start. Lücke: wo die App die Meldung beim Start anzeigt.

### Urteil P22

Umsetzbar, klein bis mittel.

## P23 Eingebundenes Laufwerk zeigt den Platz der eigenen Festplatte

### Bestand (Befund)

- **Windows:** Das Laufwerk meldet Windows den freien und den gesamten Platz des Datenträgers, auf dem der lokale Zwischenspeicher liegt (`N/mount/os/windows/callbacks_metadata.rs:82-110`). Der Code begründet das so: „The local whole-file spool is a hard lower capacity boundary. Remote quotas are backend-specific and still surface on commit.“
- **Linux:** FUSE meldet ebenso den Platz des Zwischenspeichers (`N/mount/os/linux_os/fuse_ops_meta.rs:186-200`).
- **Speicherkontingente:** Kein Speicher-Backend fragt heute ein Kontingent oder den freien Platz der Gegenseite ab. Eine Suche nach Abfragen für SFTP, WebDAV, Google Drive und SMB in `N/` findet keine.

### Weg

- **Den Platz der Gegenseite melden, wo der Speicher ihn kennt.** Annahme, nicht nachgeprüft:
  - SFTP: die OpenSSH-Erweiterung `statvfs@openssh.com`;
  - WebDAV: die Eigenschaften `quota-available-bytes` und `quota-used-bytes` nach RFC 4331;
  - Google Drive: `storageQuota` der About-Abfrage;
  - SMB: die Größenabfrage des Dateisystems;
  - Share-Geräte: eine neue Angabe des freigebenden Geräts;
  - FTP: ohne Standard.
- **Frei** ist das Kleinere aus freiem Platz der Gegenseite und freiem Platz des Zwischenspeichers, weil der Zwischenspeicher eine harte Grenze bleibt.
- **Wo der Speicher nichts liefert,** bleibt die heutige Anzeige. Lücke: welche Anzeige dann gewünscht ist.

### Urteil P23

Umsetzbar, mittel. Je Speicherart kommt eine Abfrage dazu.

## P24 Nach Leserechten nur die gesperrten Bereiche neu lesen

### Bestand (Befund)

- **Explorer-Ansicht** (Scan und Suche): Nach erfolgreicher Rechteanfrage („Leserechte anfordern …“, mit Windows-Zustimmung) liest `rescan()` die ganze Wurzel neu (`N/app/core/read_access.rs:43-48`, `N/app/core/scanning.rs:142`).
- **Speicheranalyse:** Nach erfolgreicher Anfrage startet `start_analytics_scan(root)` die Analyse der ganzen Wurzel neu. Das alte Ergebnis bleibt bis dahin sichtbar (`N/app/core/analytics_access.rs:139-146`).
- **Was über gesperrte Stellen bekannt ist:** ihre Zahl (`permission_denied`) und eine begrenzte Liste. Darüber hinaus wird nur gezählt.
  - Die Analyse merkt sich höchstens 64 Leseprobleme (`N/analytics/os/shared/analytics_outcome.rs:10`).
  - Der Scanner merkt sich höchstens 100 (`N/scanner/os/collect.rs:11`).
  - Der Explorer merkt sich höchstens 500 Fehlerpfade (`N/app/os/shared/platform_helpers.rs:42-43`).
- **Linux und Android:** Die App kann dort keine Rechte anfragen; das System gewährt sie (`N/local_access/os/linux_os.rs:73`, `:77`).

### Weg

- **Gesperrte Wurzeln merken:** Der Scan merkt sich die Wurzeln aller gesperrten Teilbäume, vollständig und ohne Obergrenze. Geschachtelte fallen unter die oberste.
- **Nur diese Teilbäume neu lesen:** Nach der Zustimmung liest die App nur diese Teilbäume mit den neuen Rechten und fügt sie in das vorhandene Ergebnis ein. In der Analyse wachsen die Summen der Elternordner mit.
- **Änderungen außerhalb:** Was sich außerhalb der gesperrten Bereiche geändert hat, holt weiter das ausdrückliche Aktualisieren (Annahme: F5 wie bisher).

### Urteil P24

Umsetzbar, mittel.

## Plattformprüfung

| Punkt | Desktop Windows | Desktop Linux | Android | Terminal und MCP |
|---|---|---|---|---|
| P10 | Ereignisse und Neulesen | neue Datenträgerliste über UDisks2 | laut Code live; am Gerät prüfen | nicht betroffen (keine Laufwerksliste) |
| P11 Stufe 1 | ein Leser, eigener Leseweg statt `CopyFile2` | ein Leser | entfällt, `vold` bindet `sr` nicht ein | Annahme: `se cp` nutzt dieselbe Engine und damit dieselbe Regel |
| P13 | Updater frischt auf (A). Laufende `se.exe` umbenennen (B, Annahme) | Updater und `se update` frischen auf (A). Ersetzen per Umbenennen gibt es schon (B) | entfällt, Android hat kein `se mcp` (`docs/AI_AGENT_PLUGIN.md`, Abschnitt Grenzen) | betrifft `se mcp` selbst |
| P14 | Abfrage mit Vorschau, auch im se Desktop | Abfrage mit Vorschau, auch im se Desktop | bestehender Dialog, erweitert um Ordner | Option statt Abfrage (E10) |
| P15 | Doppelklick öffnet den se Explorer | Doppelklick öffnet den se Explorer | entfällt, der se Launcher zeigt keine Dateien | Einstellung über `se desktop config` |
| P16 | Ordnerauswahl | Ordnerauswahl | Lücke: ob eine Android-Auswahl gemeint ist | entfällt |
| P17 | 14 Dialoge, 18 Listen | dieselbe Desktop-App | entfällt (Annahme) | entfällt |
| P18 | ein Einstellungsfenster, Geräteseiten | dasselbe | dieselbe Gliederung | Befehl für alle Rechte eines Geräts (Annahme) |
| P19 | Angaben entfernen | dasselbe | Angaben entfernen | entfällt, IDs werden dort gebraucht (Annahme) |
| P20 | Formularbaustein | dasselbe | Lücke | entfällt |
| P21 | Tresor-Laufwerk über Dokany | Tresor-Laufwerk über FUSE | Lücke: Öffnen von Tresor-Dateien | `se drive mount vault://<SITE>` nimmt `--trust-remote-root` schon an |
| P22 | Aktion am Laufwerkseintrag | dasselbe | entfällt, keine Laufwerke | neuer Befehl |
| P23 | Platz für Dokany | Platz für FUSE | Lücke: Platzangabe des Dokumentanbieters | entfällt |
| P24 | Explorer-Ansicht und Speicheranalyse | entfällt, keine Anfrage in der App | entfällt, keine Anfrage in der App | entfällt |

## Entscheidungen vor einer Umsetzung

Die Nummern setzen E1 bis E6 der Notiz zu Fernzugriff und Desktop fort. E7 und E8 (Quellenweg und Umfang von P12) entfallen, weil P12 zurückgestellt ist.

- **E9, Stufen von P13:** A; A und B; oder A, B und C. Vorschlag: A und B.
- **E10, Terminal und MCP bei P14.** Zwei Möglichkeiten:
  - Das heutige Zusammenführen in einen ausdrücklich genannten Zielordner bleibt Voreinstellung, und die Option kommt dazu. Das ist der Vorschlag, weil der Zielname dort getippt wird.
  - Der Zusatz wird neue Voreinstellung, wie in den Fenstern.
- **E11, Reihenfolge im nächsten Lauf.** Vorschlag:
  1. P10, P15, P16, P17, P21 und P22;
  2. P13 und P14;
  3. P18 bis P20 als ein Strang „Einstellungen“;
  4. P11 Stufe 1 nach der Messung, P23 und P24.

  P12 und die Stufen 2 und 3 von P11 bleiben draußen.
- **E12, Gliederung der Einstellungen (P18).** Vorschlag aus P18: Dieses Gerät, Geräte (je Gerät alle Rechte), Räume, die übrigen Bereiche, zuletzt Diagnose und Protokolle.

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
8. **P16:** welches Auswahlfenster gemeint ist.
9. **P17:** ob die Desktop-App Fenstergrößen über einen Neustart hinweg speichert.
10. **P18:**
    - der Weg vom Hauptfenster zu „Geräte & Freigaben“;
    - wie Raumrechte und Geräterechte zusammenwirken.
11. **P20:** ob die Android-Einstellungen dasselbe Problem haben.
12. **P21:**
    - die Sicherheitsprüfung je Speicherart für Weg A;
    - wie Android Tresor-Dateien öffnet.
13. **P22:** wo die App die Meldung beim Start anzeigt.
14. **P23:**
    - welche Anzeige gewünscht ist, wenn der Speicher keinen Platz meldet;
    - die Belege für die Abfragen je Speicherart;
    - die Platzangabe des Android-Dokumentanbieters.
