# Laufwerke, optische Medien und Agenten-Einbindung: Machbarkeit (2026-10-22)

## Auftrag

Nachträge vom 2026-10-22, getrennt von der Rückmeldung zu Fernzugriff und se Desktop
(`docs/plaene/2026-10-22-fernzugriff-desktop/machbarkeit.md`, P1 bis P9). Der Auftrag:
- P10 bis P12 aufschreiben; sie sollen „beim nächsten Lauf mitgemacht werden“;
- P13 als Ergänzung dokumentieren. Annahme: P13 gehört in denselben nächsten Lauf;
- P14 kam danach ohne Zeitangabe. Annahme: ebenfalls nächster Lauf, bis dahin nur dokumentiert;
- noch nichts umsetzen. Wie bei P1 bis P9: einschätzen, ob und wie jeder Punkt umsetzbar ist, ohne konkrete Implementierungsplanung und ohne Fehlersuche.

Die Punkte stehen als offene Zeilen in `docs/TODO.md`:
- `LAUFWERKE-LIVE`
- `OPTISCH-LESEN`
- `OPTISCH-DATENTRAEGER`
- `AGENT-PLUGIN-UPDATE`
- `KOPIE-ORDNER-ZUSATZ`

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
| P12 | Kleiner Zusatz: Funktionen wie IsoBuster, also die Dateiinhalte einer Disc wirklich lesen. Für den Code ist wählbar: Inhalte übernehmen oder, bei Lizenzproblemen, Cleanroom-Engineering. Dabei liefert ein Agent eine exakte Spezifikation ohne Code, und der Implementierer setzt nur nach dieser Spezifikation um |
| P13 | Die MCP-Einbindung in den Agenten soll sich mit Updates möglichst automatisch aktualisieren, ebenso die Plugin-Fassung |
| P14 | Beim Kopieren eines Ordners, dessen Name im Ziel schon existiert, sollen die Dateien nicht in den gleichnamigen Ordner hineingemischt werden. Stattdessen bekommt der oberste doppelte Ordner den Zusatz, und zwar dort, wohin kopiert wird. Weil Zusammenführen manchmal gewollt ist, fragt ein Fenster, wie vorgegangen wird, wenn möglich mit Vorschau |

Die Frage „Werden Remotes noch über Dokany eingebunden?“ ist kein Arbeitspunkt. Die Antwort steht in `README.md:239-250` („Dokany bleibt“) und in `docs/oberflaechen/laufwerke.md:32`.

## Übersicht

| Punkt | Urteil | Hauptweg | Größte Hürde |
|---|---|---|---|
| P10 | Windows umsetzbar, klein. Linux umsetzbar, mittel. Android laut Code schon live | Windows: Geräteereignisse sammeln und die Laufwerke neu lesen, dazu Neulesen beim Fensterwechsel. Linux: Datenträgerliste über UDisks2 | Netzlaufwerke und `subst` melden sich nicht über Geräteereignisse |
| P11 | umsetzbar; vorher messen | ein Leser je optischem Laufwerk und große Anfragen. Danach Lesen nach Lage auf der Disc und eine Fehlerkarte für beschädigte Discs | Die Lage einer Datei gibt es ohne Sonderrechte nur über ein eigenes Lesen des Dateisystems (Linux, CDFS) |
| P12 | umsetzbar unter Windows und Linux, großer eigener Strang. Android nur Abbilder | eigene Leser nach freien Standards, permissive Crates. Cleanroom nur für Formate ohne öffentliche Beschreibung | Rechte für Rohzugriff unter Windows (Lücke). Rechtslage des KI-Cleanrooms offen. Entschlüsseln ausgeschlossen |
| P13 | umsetzbar; Grundmechanik vorhanden | Stufe A: beim Update auffrischen, auch bei Pfadwechsel. Stufe B: laufende Server nicht beenden. Stufe C: Server ohne Neuverbinden tauschen | Claude Code und Codex verbinden beendete stdio-Server nicht neu |
| P14 | umsetzbar, mittel | lokale Ziele wie Remote-Ziele: oberster Ordner „Name (2)“. Abfrage mit Vorschau, Option im Terminal | Vorschau großer oder entfernter Bäume. Voreinstellung im Terminal (E10) |

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
  - Ohne Sonderrechte bleibt das eigene Lesen des Dateisystems vom Rohgerät. Das ist derselbe Baustein wie P12.
- **Stufe 3, beschädigte Discs:**
  - Ein erster Durchgang mit wenigen Wiederholungen und kurzer Frist.
  - Defekte Bereiche in einer Karte merken, wie bei ddrescue.
  - Erst nach den guten Dateien nachlesen, auf Wunsch bei gedrosseltem Laufwerk.
  - Drosseln geht unter Windows über `IOCTL_CDROM_SET_SPEED` [Quelle: https://learn.microsoft.com/en-us/previous-versions/windows/drivers/storage/cd-rom-set-speed, abgerufen 2026-10-22]. Unter Linux geht es über `SET CD SPEED` per SG_IO, nur bei zum Schreiben geöffnetem Gerät [Quelle: https://gbmc.googlesource.com/linux/+/refs/heads/master/drivers/scsi/scsi_ioctl.c, abgerufen 2026-10-22; Zusammenfassung des Abrufwerkzeugs].
- **Android:** `vold` bindet ein USB-CD/DVD-Laufwerk nicht ein, denn die Blockgruppe `sr` (Major 11) fehlt in seiner Liste [Quelle: https://android.googlesource.com/platform/system/vold/+/refs/heads/main/model/Disk.cpp, abgerufen 2026-10-22]. P11 betrifft daher Windows und Linux.

### Urteil P11

- **Gesamt:** umsetzbar; vorher messen.
- **Stufe 1:** klein bis mittel. Betroffen sind Regler, Leseweg und Index.
- **Stufe 2:** mittel. Unter Linux braucht sie den Dateisystemleser aus P12.
- **Stufe 3:** mittel. Unter Windows hängt sie an den Rechten für Rohzugriff, siehe P12.

## P12 Funktionen wie IsoBuster

### Bestand

- **Kein eigener Disc-Zugriff** (Befund): Es gibt keinen Leser für ISO 9660 oder UDF, kein Abbildformat und keinen Rohzugriff auf das Laufwerk (Suche wie unter P11). Das einzige Archiv-Crate ist `zip` (`native/Cargo.toml:151`).
- **Lizenz** (Befund): Das Projekt steht unter MIT (`native/Cargo.toml:7`, `LICENSE`) und verteilt Binärdateien.

### Was IsoBuster kann

Belegt aus den Seiten `isobuster.php`, `help/file_systems`, `help/track_and_sector_modes`, `help/recovery` und `license-models.php` unter https://www.isobuster.com/, jeweils [Quelle, abgerufen 2026-10-22]:

- **Plattform und Medien:** Version 5.8, nur Windows. Medien: CD, DVD, Blu-ray, HD DVD, GD-ROM, UDO, M-Disc. IsoBuster greift selbst auf das Gerät zu und deutet die Daten selbst, nicht über Windows.
- **Sessions und Tracks:** alte und offene Sessions, alle CD-Sektormodi (Mode 1 und Mode 2 Form 1 mit 2048 Byte, Mode 2 Form 2 mit 2324 Byte, Audio mit 2352 Byte), Einzelsektoren, Audio-Tracks als WAV und CD-Text.
- **Dateisysteme:** ISO 9660, Joliet, Rock Ridge, Romeo, CD-i, El Torito, UDF 1.02 bis 2.6, HFS und HFS+. Dazu kommen Datenträger-Dateisysteme, die für Discs nicht nötig sind.
- **Beschädigte Discs:**
  - mehrere Wiederholverfahren mit einstellbarer Pause;
  - Oberflächenscan;
  - Liste der Dateien mit Lesefehlern;
  - ein „Managed Image“, das Fehlerblöcke markiert und später nachliest [laut Suchergebnis: https://www.isobuster.com/help/create_and_complete_a_managed_image_file].
- **Verlorene Dateien:** verlorene ISO- und Joliet-Sessions, UDF-Reste, IFO/VOB-Verweise und Signatursuche.
- **Kostenpflichtig:** Frei sind ISO 9660, Joliet, Rock Ridge und El Torito. Das Extrahieren aus UDF und HFS(+) kostet.
- **EULA:** Sie verbietet Reverse Engineering [Quelle: https://www.isobuster.com/eula.php, abgerufen 2026-10-22]. Nach § 69g Abs. 2 UrhG ist eine Klausel nichtig, soweit sie § 69d Abs. 3 oder § 69e widerspricht [Quelle: https://www.gesetze-im-internet.de/urhg/__69g.html, abgerufen 2026-10-22].

### Lizenzen der vorhandenen Bausteine

- **GPL scheidet aus:** libcdio (GPL-3.0-or-later), redumper, dvdisaster, cdrdao und GNU ddrescue stehen unter der GPL. Gelinkt ergäben sie mit dem MIT-Programm ein GPL-Gesamtwerk [Quelle: https://www.gnu.org/licenses/gpl-faq.html, abgerufen 2026-10-22].
- **LGPL-2.1 ist unbequem:** Betroffen sind pycdlib, libudfread und 7-Zip. Statisch gelinkt verlangt die LGPL-2.1 Objektdateien oder Quellen zum Neu-Linken, dynamisch geladen nicht [Quelle: https://spdx.org/licenses/LGPL-2.1-only.html, abgerufen 2026-10-22]. Annahme: Rust linkt hier statisch.
- **Permissiv und übernehmbar** [Quelle: https://crates.io/api/v1/crates/, abgerufen 2026-10-22]:
  - `hadris-iso` (MIT, 2.5.0; ISO 9660 mit Joliet, Rock Ridge, El Torito);
  - `hadris-udf` (MIT, nur Vorabversion 3.0.0-rc.2);
  - `cdfs` (MIT oder Apache-2.0);
  - `udf-core` (Apache-2.0, 0.1.0);
  - `lamfold-udf` (MIT oder Apache-2.0, UDF bis 2.01);
  - `hfsplus` (MIT);
  - `chd` (BSD-3-Clause);
  - `cue-rw` (Apache-2.0 oder MIT);
  - die SPTI-Strukturen in `windows-sys`;
  - libarchive (BSD-2-Clause, ISO 9660 ohne UDF) [Quelle: https://raw.githubusercontent.com/libarchive/libarchive/master/COPYING, abgerufen 2026-10-22].
- **Ohne Fundstelle:** Für NRG, MDS/MDF, CCD, CDI und klassisches HFS fand die Suche kein permissives Crate.

### Öffentliche Spezifikationen

- **Frei:** [Quelle: https://ecma-international.org/publications-and-standards/, abgerufen 2026-10-22]
  - ECMA-119 (ISO 9660, 6. Ausgabe Dezember 2025);
  - ECMA-130 (CD-ROM);
  - ECMA-167 (Volume- und Dateistruktur);
  - ECMA-168 (Mehrsession-CD);
  - ECMA-267 (DVD-ROM);
  - alle UDF-Fassungen 1.02 bis 2.60 als ECMA TR/112 (Dezember 2023);
  - HFS+ als Apple TN1150 [Quelle: https://developer.apple.com/library/archive/technotes/tn/tn1150.html, abgerufen 2026-10-22].
- **Nicht frei:**
  - IEC 60908, das Red Book für Audio-CDs [laut Suchergebnis: https://webstore.iec.ch/publication/3885];
  - die MMC-6-Entwürfe, die laut T10 nur für Mitglieder sind [Quelle: https://www.t10.org/drafts.htm, abgerufen 2026-10-22];
  - die BD-ROM-Teile, die lizenzpflichtig sind [laut Suchergebnis: https://blu-raydisc.info/authoring-faq.php].
- **Ohne öffentliche Herstellerbeschreibung:** NRG, MDS/MDF, CCD, CDI und IBP/IBQ (Lücke).

### Cleanroom: was es leistet und was nicht

- **Das Verfahren:** Ein Team schreibt die Spezifikation, ein zweites setzt nur danach um, wie beim Phoenix-BIOS. Es schützt gegen den Vorwurf, Ausdruck kopiert zu haben, nicht gegen Patente [Quelle: https://en.wikipedia.org/wiki/Clean-room_design, abgerufen 2026-10-22]. Annahme: Gegen Vertragsbindung, Geschäftsgeheimnisse und das Umgehungsverbot schützt es ebenfalls nicht.
- **Datenformate sind frei:** Funktionalität, Programmiersprache und Dateiformat sind nicht urheberrechtlich geschützt (EuGH C-406/10, 2012-05-02) [Quelle: https://www.lto.de/recht/nachrichten/n/urheberrechtsfaehigkeit-von-computerprogrammen, abgerufen 2026-10-22].
  - Beobachten und Testen eines Programms bleibt erlaubt [Quelle: https://www.gesetze-im-internet.de/urhg/__69d.html, abgerufen 2026-10-22].
  - Dekompilieren ist nur für Interoperabilität erlaubt und nicht für ein „weitgehend ähnlich aufgebautes Programm“ [Quelle: https://gesetze.legal/eu/rl_2009_24_eg/6, abgerufen 2026-10-22].
  - Annahme: IsoBuster als Schwarzkasten mit Test-Discs beobachten ist erlaubt, es zu disassemblieren nicht.
- **Cleanroom mit KI-Agenten ist rechtlich offen.**
  - Es gibt kein Urteil.
  - Bei chardet 7.0 (März 2026) räumte der Maintainer ein, dass die Trennung „did not exist here“. Das Modell kannte das Original sehr wahrscheinlich aus dem Training [Quelle: https://simonwillison.net/2026/Mar/5/chardet/, abgerufen 2026-10-22].
  - Das LG München I hat das Speichern von Liedtexten in Modellparametern als Vervielfältigung gewertet (Urteil vom 2025-11-11, Az. 42 O 14139/24, nicht rechtskräftig) [Quelle: https://www.lto.de/recht/nachrichten/n/42o1413924-lg-muenchen-i-gema-openai-chatgpt-songtexte-ki, abgerufen 2026-10-22].
  - Annahme, Gegenmittel:
    - Die Spezifikation enthält keinen Code und nicht die Bezeichner und die Reihenfolge des Originals.
    - Der Implementierer sieht nur die Spezifikation.
    - Eine Ähnlichkeitsprüfung gegen das Original läuft danach.
    - Der Cleanroom gilt nur für Formate ohne öffentlichen Standard.
- **Kopierschutz ist ausgeschlossen:** CSS, AACS, BD+ und CPRM zu entschlüsseln oder entsprechende Werkzeuge zu verbreiten, verbieten § 95a UrhG und Art. 6 der Richtlinie 2001/29/EG [Quelle: https://www.gesetze-im-internet.de/urhg/__95a.html, abgerufen 2026-10-22]. Ein Cleanroom ändert daran nichts. Verschlüsselte VOB-Dateien roh zu kopieren, ohne zu entschlüsseln, ist nach Annahme zulässig. Eine Entscheidung dazu fand die Recherche nicht (Lücke).

### Quellenweg je Funktion

Wege:
- (a) eigene Umsetzung nach öffentlichem Standard;
- (b) Übernahme permissiven Codes;
- (c) Cleanroom;
- (d) ausgeschlossen.

Die Zuordnung ist ein Vorschlag aus den Belegen oben.

| Funktion | Weg | Grund |
|---|---|---|
| Laufwerkszugriff: SPTI unter Windows, SG_IO unter Linux | (b) für die Strukturen, (a) für die MMC-Befehle | `windows-sys` ist MIT oder Apache-2.0 |
| TOC, Tracks, Sessions, Sektormodi, Rohsektoren | (a) | ECMA-130 und ECMA-168 sind frei |
| ISO 9660, Joliet, Rock Ridge, El Torito | (a), mit `hadris-iso` als Vergleich | Annahme: Für beschädigte Discs braucht es einen fehlertoleranten Leser |
| UDF 1.02 bis 2.60 | (a) | ECMA-167 und ECMA TR/112 sind frei; die Crates decken nicht alle Fassungen ab |
| Audio-Tracks als WAV, CD-Text | (a) | READ CD und Unterkanal nach ECMA-130. Abstimmung über Mehrfachlesen statt cdparanoia-Code (GPL/LGPL) |
| Abbilder ISO, BIN/CUE | (a) oder (b) mit `cue-rw` | einfache Formate |
| CHD, HFS+ | (b) | `chd` (BSD-3-Clause), `hfsplus` (MIT) |
| Eigenes Rettungsabbild mit Fehlerkarte | (a) | Die Fehlerkarte ist eine Idee, kein Ausdruck (§ 69a Abs. 2 UrhG); kein ddrescue-Code |
| Verlorene Dateien, Signatursuche | (a) | Heuristiken auf den freien Strukturen von ECMA-119 und ECMA-167 |
| NRG, MDS/MDF, CCD, CDI, klassisches HFS, IBP/IBQ, Mount Rainier | (c) | ohne öffentliche Beschreibung; IBP/IBQ nur aus Beispieldateien, nie aus IsoBuster selbst |
| Entschlüsseln von CSS, AACS, BD+, CPRM | (d) | § 95a UrhG |

### Rohzugriff je Plattform

- **Windows:**
  - Ein SCSI-Durchgriff braucht laut Microsoft keine Administratorrechte, wohl aber Lese- und Schreibzugriff auf das Gerät [Quelle: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddscsi/ni-ntddscsi-ioctl_scsi_pass_through, abgerufen 2026-10-22].
  - Ein Volume direkt zu öffnen verlangt Administratorrechte [Quelle: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew, abgerufen 2026-10-22].
  - Lücke: ob das auch für ein optisches Laufwerk gilt und welche Rechte ein Standardnutzer auf `\\.\CdRom0` hat.
- **Linux:**
  - systemd gibt der angemeldeten Person über `uaccess` Zugriff auf `/dev/sr0` und das zugehörige sg-Gerät [Quelle: https://third-party-mirror.googlesource.com/systemd/+/refs/heads/master/rules.d/70-uaccess.rules.in, abgerufen 2026-10-22].
  - Lesebefehle wie READ CD und READ TOC sind über SG_IO auch ohne root erlaubt [Quelle: https://gbmc.googlesource.com/linux/+/refs/heads/master/drivers/scsi/scsi_ioctl.c, abgerufen 2026-10-22; Zusammenfassung des Abrufwerkzeugs].
- **Android:** `vold` bindet optische Laufwerke nicht ein (siehe P11).
  - Ginge es über die USB-Host-API, müsste das Programm Transport und MMC-Befehle selbst sprechen. Dafür braucht es eine Zustimmung je Gerät [Quelle: https://developer.android.com/develop/connectivity/usb/host, abgerufen 2026-10-22].
  - Die Bibliothek libaums kann nur FAT32 [Quelle: https://github.com/magnusja/libaums, abgerufen 2026-10-22].
  - Abbilddateien (ISO, BIN/CUE, CHD) dagegen ließen sich auch auf Android öffnen. Annahme: Die Leser sind plattformneutral.

### Urteil P12

- **Gesamt:** umsetzbar unter Windows und Linux, als eigener Strang mit großem Umfang. Die meisten Teile gehen nach öffentlichen Standards (Weg a). Ein Cleanroom ist nur für die Formate ohne öffentliche Beschreibung nötig, Entschlüsseln ist ausgeschlossen.
- **Stufe 1:** Discs und Abbilder (ISO, BIN/CUE) als Orte im Explorer. Alle Sessions und Tracks sowie Dateien, die das System nicht zeigt, etwa aus alten Sessions.
- **Stufe 2:** Rettung beschädigter Discs mit Fehlerkarte und Fortsetzen, verlorene Dateien, Signatursuche.
- **Stufe 3:** Audio-Tracks und die weiteren Abbildformate (CHD, HFS+ über (b), die übrigen über (c)).
- **Android:** Abbilder ja, Laufwerke nein.

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

## Plattformprüfung

| Punkt | Desktop Windows | Desktop Linux | Android | Terminal und MCP |
|---|---|---|---|---|
| P10 | Ereignisse und Neulesen | neue Datenträgerliste über UDisks2 | laut Code live; am Gerät prüfen | nicht betroffen (keine Laufwerksliste) |
| P11 | ein Leser, eigener Leseweg statt `CopyFile2` | ein Leser; Lage über eigenes Lesen des Dateisystems | entfällt, `vold` bindet `sr` nicht ein | Annahme: `se cp` nutzt dieselbe Engine und damit dieselbe Regel |
| P12 | Laufwerke und Abbilder | Laufwerke und Abbilder | nur Abbilder | Befehle und Werkzeuge für Disc und Abbild (Annahme: neue Unterbefehle) |
| P13 | Updater frischt auf (A). Laufende `se.exe` umbenennen (B, Annahme) | Updater und `se update` frischen auf (A). Ersetzen per Umbenennen gibt es schon (B) | entfällt, Android hat kein `se mcp` (`docs/AI_AGENT_PLUGIN.md`, Abschnitt Grenzen) | betrifft `se mcp` selbst |
| P14 | Abfrage mit Vorschau, auch im se Desktop | Abfrage mit Vorschau, auch im se Desktop | bestehender Dialog, erweitert um Ordner | Option statt Abfrage (E10) |

## Entscheidungen vor einer Umsetzung

Die Nummern setzen E1 bis E6 der Notiz zu Fernzugriff und Desktop fort.

- **E7, Quellenweg für P12.** Vorschlag:
  - (a) vor (b) vor (c);
  - kein GPL- und kein LGPL-Code;
  - Cleanroom nur für NRG, MDS/MDF, CCD, CDI, klassisches HFS, IBP/IBQ und Mount Rainier, mit getrennten Agenten, einer Spezifikation ohne Code und Bezeichner des Originals, und einer Ähnlichkeitsprüfung;
  - Entschlüsseln ausgeschlossen.
- **E8, Umfang von P12 im nächsten Lauf:** Stufe 1; Stufe 1 und 2; oder alle drei Stufen.
- **E9, Stufen von P13:** A; A und B; oder A, B und C. Vorschlag: A und B.
- **E10, Terminal und MCP bei P14.** Zwei Möglichkeiten:
  - Das heutige Zusammenführen in einen ausdrücklich genannten Zielordner bleibt Voreinstellung, und die Option kommt dazu. Das ist der Vorschlag, weil der Zielname dort getippt wird.
  - Der Zusatz wird neue Voreinstellung, wie in den Fenstern.
- **E11, Reihenfolge im nächsten Lauf.** Vorschlag:
  1. P10, P13 und P14;
  2. P11 Stufe 1 nach der Messung;
  3. P12 als eigener Strang, mit den Stufen 2 und 3 von P11.

## Abhängigkeiten

- P11 Stufe 2 und Stufe 3 brauchen den Dateisystemleser und den Rohzugriff aus P12. Das gilt für die Lage unter Linux und unter CDFS sowie für die Fehlerkarte.
- P13 Stufe C braucht Stufe B.
- P12 zeigt Discs als Orte. Annahme: Unter Linux braucht das die Datenträgerliste aus P10.

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
4. **P12:**
   - Rechte eines Standardnutzers auf `\\.\CdRom0` und auf `\\.\D:` eines optischen Laufwerks;
   - der Text von MMC-6, den T10 nur für Mitglieder bereithält;
   - Beschreibungen von NRG, MDS/MDF, CCD, CDI, IBP/IBQ und Mount Rainier;
   - Rechtsprechung zum rohen Lesen verschlüsselter Sektoren und zum KI-Cleanroom.
5. **P13:**
   - wann Codex seine MCP-Konfiguration liest;
   - Konfigurationsweg und erste Version des Schalters `mcp_tool_list_changed`;
   - ob Claude Code `list_changed` in der aktuellen Version zuverlässig übernimmt;
   - ob Windows das Umbenennen einer laufenden `se.exe` erlaubt.
6. **P14:** welche Ebene bei „Ordnerstruktur erhalten“ den Zusatz bekommt.
