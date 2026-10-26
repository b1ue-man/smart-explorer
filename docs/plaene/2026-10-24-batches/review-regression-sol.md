# Unabhängiger Release-Regressionsreview Sol

Stand: 2026-10-26. Statischer Review einschließlich Capture-/Rendererergänzung abgeschlossen. Elf belegte
Funktionsregressionen: acht mit Schwere Hoch, drei mit Schwere Mittel.
Dieser Bericht bescheinigt keine allgemeine Regressionsfreiheit.

## Auftrag, Grundlage und Grenzen

Quelle des Ergebnisauftrags: `/tmp/smartexplorer-final-review-user-contract.txt` und
`docs/plaene/2026-10-24-batches/regressionsreview.md`. Vergleich ausschließlich
`a814f263096fe69d3746e840b5f3fad3fd7e88a0` (veröffentlichtes v0.5.184) gegen
`ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b` (fester Quellkandidat).
Die letzte Veröffentlichung wurde vom Main am 2026-10-26 anhand der GitHub-API
belegt; dieser Reviewer übernimmt die dokumentierte Vergleichsbasis, ohne eine
eigene Aussage über den derzeit sichtbaren GitHub-Veröffentlichungszustand zu treffen.

Alleiniger Ergebnisumfang: belegte Verluste oder negative Beeinflussungen
bestehender Endnutzerfunktionen einschließlich kleiner Funktionsfehler. Der interne
Implementierungsweg darf sich ändern. Kein Fixplan, keine vollständige
Fehler-Verknüpfungskarte und keine Umsetzung. Keine Produktänderung, kein Testlauf,
kein Build, kein Commit, kein Push, kein Graphupdate und kein Release durch diesen
Reviewer. Die Berichte der beiden gleichzeitig arbeitenden Reviewer werden weder
gelesen noch mit deren Autoren abgeglichen.

Pfadkonvention der Befunde: Native Domänenpfade ohne `native/` beziehen sich
auf `native/src/`; Androidpfade ab `api/`, `screen/` oder `ui/` auf
`android/app/src/main/java/app/smartexplorer/android/`. Explizit mit
`native/` beginnende Installerpfade beziehen sich auf die Repositorywurzel.
Historische Kurzbelege `a814f263:…` verwenden dieselbe Konvention.

## Erstplan aus dem delegierten Auftrag

Bekannt aus dem Auftrag: feste Vergleichscommits, Erhalt des bestehenden
Endnutzerverhaltens und ausschließliche Schreibfläche dieses Berichts. Durch
Bestandslektüre zu klären: tatsächlich geänderte Funktionsgrenzen, unveränderte
Verbraucher, Persistenzverträglichkeit und etwaige neue Abweisungen bestehender
Nutzungspfade. Es fehlt keine nur vom Nutzer lieferbare Angabe.

Geplanter Nachweis: Quellvergleich mit konkretem Auslöser, Vorher-/Nachherbeleg,
sichtbarer Endnutzerfolge und Gegenprüfung jedes tatsächlichen Befunds. Externe
API-/Protokollaussagen benötigen eine Primärquelle. Ein statischer Review belegt
keinen erfolgreichen Lauf auf Windows, Linux oder Android.

## Zweitplan aus dem Bestand

Gelesener Einstieg: `docs/ARCHITEKTUR.md`; Zuständigkeitsvertrag:
`docs/plaene/2026-10-24-batches/regressionsreview.md`. Das vollständige Inventar
nennt 1.529 Nicht-Graph-Dateien und 34 native Domänengrenzen. Das Vendorinventar
trennt unveränderte Release-Abhängigkeiten von tatsächlichen Projektänderungen.
Die Graphabfrage zu Transfer, Reservierung, Kapazität und Bildschirmverbindungen
bestätigt die gemeinsame Backendgrenze und ihre Verbraucher; sie ersetzt keine
Quelllektüre. Graphify meldete die lokale Paket-/Skillversionsabweichung
0.9.17/0.9.11; ein Graphupdate wurde nicht ausgeführt.

Die zusammenhängenden Reviewblöcke sind:

1. Dateiübertragung, Ordnerentscheidungen, Wiederaufnahme, optische Quellen,
   lokale Handlegrenzen und alle betroffenen Backendadapter. Erwartung: bisherige
   Quellen-/Zielkombinationen, Verschieben, Abbruch und Konfliktschutz bleiben nutzbar.
2. Laufwerke, Providerkapazität, Wiederherstellung, Tresore, Cloudanmeldung und
   Suchwiederholung. Erwartung: bestehende Einstellungen und Daten bleiben zugänglich.
3. Share-Routing, Rechte und Protokolle mit vendorten Transportänderungen sowie
   neue Fernzugriffsintegrationen. Erwartung: bestehende Datei-, Sync-, Exec- und
   Bildschirmverbindungen bleiben mit ihrer bisherigen Identität verwendbar.
4. Bildschirmaufnahme, Codecs, Präsentation und Eingabe auf Desktop und Android.
   Erwartung: bestehende Einzelbildschirm-/Fensterfunktionen, Eingabefreigaben,
   Fallbacks und Sitzungsende bleiben erhalten.
5. Desktop-, CLI-, MCP- und Android-Bedienwege, Medien, Launcher und Installation.
   Erwartung: bisherige Aufrufe und gespeicherte Zustände behalten ihre Funktion.

Jeder Block wird gegen Release und Kandidat untersucht; ungeklärte Laufzeit- oder
Lesegrenzen werden ausdrücklich festgehalten. Das ist ein Reviewarbeitsplan und
kein Reparaturplan. Es werden keine Abnahmeschritte lokal ausgeführt.

## Befunde

### RSOL-01 — Hoch: Normales Ordnerkopieren auf FTP/SFTP scheitert vor der Ordneranlage

**Auslöser:** Im Desktop einen lokalen Ordner auf ein schreibbares FTP-/FTPS-Ziel
einfügen oder in Android denselben Kopierauftrag mit der normalen Ordnerregel
`keep-both` starten. Der Zielordner kann vollständig leer sein. Für SFTP gilt
derselbe Fehler beim direkten SFTP-Backend ohne die neue Agentenfunktion.

**Kandidatenbeleg:** `app/core/transfer_route.rs:192` setzt standardmäßig
`FolderConflict::KeepBoth`; `mobile/os/shared/transfer.rs:99` tut dies ebenfalls.
`transfer/os/shared/engine/roots.rs::plan_roots` ruft für den ausgewählten Ordner
`Reservations::reserve` auf. Dort verlangt Zeile 54 mit
`directory_identity(backend, &parent_path)?` einen erfolgreichen Identitätsaufruf.
`ftp/core/folder_reservation.rs:11` und `sftp/core/folder_reservation.rs:11` geben
ausnahmslos `ErrorKind::Unsupported` zurück. Die gemeinsame Grenze
`vfs/core/folder_reservation.rs::directory_identity` propagiert diesen Fehler.
Die alternative direkte MKDIR-Reservierung für `Ok(None)` wird deshalb nie erreicht.

**Releasebeleg:** `a814f263:transfer/os/shared/engine/roots.rs::plan_roots`
verwendete `claim` und `plan_exclusive`; die eigentliche exklusive Anlage in
`folders.rs::create_exclusive` rief `create_dir_exclusive` auf. Keine dieser
Stationen verlangte eine dauerhafte Ordneridentität. Die bestehenden
FTP-/SFTP-Ordnerprimitive sind im Kandidaten weiter vorhanden.

**Endnutzerfolge:** Ein bisher möglicher normaler Ordnerupload bricht mit
„Zielordner … reservieren“/Unsupported ab. Es handelt sich nicht nur um fehlende
Crashwiederaufnahme: Auch ein neuer, konfliktfreier Auftrag überträgt keine Datei.

**Gegenprüfung:** `directory_reservation_supported` meldet für FTP und SFTP
`true`; bei einem freien Namen lässt `selection.rs::folder_conflicts` daher
sogar den Auswahldialog aus. Eine manuell gesetzte Merge-Regel umgeht die
Reservierung, beseitigt den Ausfall des normalen Bedienwegs aber nicht. Lokale
Ziele mit unterstützter Identität werden von diesem konkreten Fehler nicht erfasst.
Belegt durch statische Aufrufkette; kein FTP-/SFTP-Server wurde gestartet.

### RSOL-02 — Hoch: Uploadziele ohne Downloadrecht werden neu unbenutzbar

**Auslöser:** Eine einzelne Datei in ein vorhandenes FTP-/FTPS-Verzeichnis
hochladen, dessen Konto LIST, STOR und die bisherigen Stage-/Umbenennungsbefehle
ausführen darf, aber kein RETR. Ein konkretes zulässiges Serverprofil ist
vsftpd mit `write_enable=YES`, `download_enable=NO` und erlaubter Verzeichnisliste.
Einzeldateiauswahl vermeidet den getrennten Ordnerfehler RSOL-01.

**Kandidatenbeleg:** Die normalen Desktop-/Android-/Legacy-Engineaufträge sind
`recoverable=true`. `publish.rs::publish` verlangt vor `promote_copy_stage`
`file_recovery::before`; `file_recovery.rs:352` liest die geschriebene Remotestage
mit `observe`. `file_evidence.rs:42–43` ruft dafür `open_read_regular` auf und
hasht ihren gesamten Inhalt. Ein fehlendes Leserecht wird unverändert mit `?`
weitergereicht. Der Fehlerzweig in `publish.rs` verwirft die Stage und meldet den
Upload fehlgeschlagen. Auch die frische Publikation verlangt nach dem Flush
erneutes Lesen (`file_recovery.rs:371`).

**Releasebeleg:** `a814f263:transfer/os/shared/engine/publish.rs::complete` und
`publish` schlossen den Writer ab, prüften die Quelle und veröffentlichten die
Stage. Sie lasen keine Zieldateiinhalte. FTP-Schreiben und FTP-Lesen besitzen
weiterhin getrennte Implementierungen in `ftp/core/ftp.rs`; der Kandidat macht
den bisher unabhängigen Download jetzt zur Voraussetzung des Uploads.

**Endnutzerfolge:** Eine bisher funktionierende Uploadablage mit absichtlich
entzogenem Downloadrecht nimmt über diesen Bedienweg keine Datei mehr an. Die
Sicherheitsregel des Servers ist kein fehlendes Konto und kein fehlender Testzugang.

**Gegenprüfung und Primärquelle:** Die offizielle
[vsftpd-Konfigurationsreferenz](https://security.appspot.com/vsftpd/vsftpd_conf.html)
beschreibt `download_enable=NO` als Abweisung sämtlicher Downloadanfragen und
`write_enable` getrennt für STOR, DELE, RNFR, RNTO, MKD, RMD, APPE und SITE;
geprüft am 2026-10-26. LIST bleibt über die eigene Einstellung steuerbar. Das
Problem ist deshalb ein tatsächlich darstellbares Berechtigungsprofil und kein
erfundener Providerzustand. Ein Konto mit zusätzlichem Leserecht umgeht den
Fehler; das stellt die verlorene Fähigkeit des Uploadkontos nicht wieder her.
Kein Serverlauf wurde ausgeführt.

### RSOL-03 — Hoch: Android mit weniger als 512 MiB freiem Heap kann keinen bestehenden Bildschirmhost mehr ansehen

**Auslöser:** Die aktualisierte Android-App verbindet sich mit einem weiterhin
auf v0.5.184 laufenden Desktophost; ihr gemeldetes `workingBytes` ist kleiner
als 536.870.912 Bytes. Das trifft ausdrücklich auch die kleine Gruppenvorschau.
Der konkrete Wert ist eine auslösende Bedingung und keine Behauptung über das
Telefon des Nutzers.

**Kandidatenbeleg:** `api/ScreenApi.kt::viewOpen` meldet immer
`ScreenFrameLimits.workingBytes()`. `screen/ScreenFrameLimits.kt:13–16`
berechnet den verbleibenden VM-Heap aus `maxMemory − (totalMemory − freeMemory)`.
`mobile/os/shared/domains/screen_view.rs::view_open` übernimmt diesen Wert als
Prozessgrenze; `screen/os/shared/image_memory.rs:16–18` reicht ihn an die
Sessionzulassung weiter. `screen/os/shared/session_memory.rs::handshake`
begrenzt die Lease darauf. Da v0.5.184 kein `frame_contract` sendet, beendet
`screen/core/viewer_client.rs:217–218` die Verbindung ausnahmslos mit
„legacy host requires the complete legacy memory budget“, sobald die Lease
kleiner als die festen 512 MiB ist. Die angeforderte Bildgröße und der
Vorschaumodus werden bei dieser Abweisung nicht berücksichtigt.

**Releasebeleg:** `a814f263:screen/core/viewer_client.rs::handshake` akzeptierte
denselben v1-Host ohne diese pauschale Speicherabweisung. Die etablierte
Vorschaugröße war `PREVIEW_EDGE = 320` in `screen/core/host_pipeline.rs:43`.
Der neue Client kann die vorhandene v1-Bildschirmfunktion deshalb schon vor
dem ersten Bild verlieren, obwohl nur diese kleine Vorschau angefordert ist.

**Endnutzerfolge:** Nach Aktualisierung des Handys verschwinden funktionierende
Bildschirmvorschau, Ansicht und Steuerung eines noch nicht aktualisierten
Desktopgeräts hinter einem Speicherfehler. Das Protokoll wird weiterhin als
unterstützt erkannt; es fehlt eine größenbezogene verträgliche Zulassung.

**Gegenprüfung und Grenze:** Metadatenkanäle sind ausdrücklich ausgenommen.
Ein neuer Host mit ausgehandeltem v2-Vertrag umgeht genau diese Abweisung;
ein v1-Host mit einer vollen 512-MiB-Lease ebenfalls. Der umgekehrte Pfad besitzt
eine verwandte feste Prüfung in `screen/core/host_server_start.rs:27`; seine
konkrete Androidaufnahme hängt zusätzlich vom Projektionszustand ab und wird
hier nicht als eigener Laufzeitbefund behauptet. Die offizielle
[Android-Runtime-Referenz](https://developer.android.com/reference/java/lang/Runtime#maxMemory())
definiert `maxMemory()` als VM-Speicherobergrenze; Abruf 2026-10-26. Keine
Geräteverbindung wurde ausgeführt.

### RSOL-04 — Hoch: Die erste Android-Bildschirmvorschau belegt das gesamte gemeinsame Sitzungsbudget

**Auslöser:** In Android die Bildschirm-Gruppenansicht mit mindestens zwei
erlaubten, erreichbaren neuen Hosts öffnen. Beide Hosts können mindestens das
vom Androidprozess angebotene Budget bedienen. Jede Kachel verlangt nur ihre
bestehende 320-Pixel-Vorschau.

**Kandidatenbeleg:** `ui/screen/ScreenPreviewTile.kt` öffnet je Kachel eine
eigene `ScreenSession` mit `PREVIEW_EDGE = 320`. Der Produktionspfad
`ScreenSession.kt::CoreScreenBackend.open` reicht den tatsächlichen Renderer
weiter; `api/ScreenApi.kt::viewOpen` bietet für **jede** Sitzung das gesamte
verbleibende Heapbudget `B ≤ 512 MiB` an. Der oben beschriebene mobile Pfad
setzt zugleich die prozessweite Obergrenze auf B. Androids
`screen/os/android/session_memory.rs::facts` liefert `None`.
`screen/os/shared/session_memory.rs::handshake` verlangt eine Lease über B,
ohne die angeforderten 320×320 Pixel zu berücksichtigen.

Für die erste Sitzung ergibt `frame_memory.rs::reserve_with_ceiling`
(Zeilen 104–115) somit `observed=B`, `capacity=B`, `bytes=B` und
`committed=B`. Die Lease bleibt in `ViewerSession.memory` bis zum Ende der
ersten Verbindung erhalten. Für die zweite Sitzung sind
`observed.saturating_sub(committed)` und
`capacity.saturating_sub(committed)` null; Zeile 111 meldet
„insufficient screen session memory“. Eine kleinere ausgehandelte Bildgröße
gibt den ungenutzten Teil der ersten Lease nicht frei.

**Releasebeleg:** Die Gruppenansicht mit einer eigenen Verbindung je Kachel
bestand bereits in `a814f263:.../ui/screen/ScreenPreviewTile.kt`. Die damalige
`screen/core/viewer_client.rs::handshake` besaß weder diese Prozesslease noch
die daraus folgende Nullzuteilung. Die Gruppenansicht wurde nicht zur Ablösung
beauftragt.

**Endnutzerfolge:** Die erste Kachel kann streamen; weitere erlaubte Geräte
zeigen Speicherfehler statt ihrer bisherigen gleichzeitigen Vorschau. Dies
betrifft auch vollständig aktualisierte Geräte und ist damit vom
v1-Verbindungsabbruch RSOL-03 getrennt.

**Gegenprüfung und Grenze:** Der Befund setzt voraus, dass die erste
Sitzung tatsächlich eine Lease über B erhält; wenn eine kleinere Hostgrenze
oder ein vorheriger Eigentümer die erste Lease begrenzt, lautet die konkrete
Rechnung anders. Schon das reguläre Profil „zwei neue Desktophosts und ein
sonst freier Androidpool“ erfüllt die auslösende Bedingung. Die Freigabe der
ersten Lease beim Sitzungsende ermöglicht danach wieder eine einzelne Sitzung.
Es wurde kein gleichzeitiger Lauf behauptet oder lokal ausgeführt.

### RSOL-05 — Hoch: Verschieben auf FAT32/exFAT benötigt neu Platz für eine vollständige zusätzliche Kopie

**Auslöser:** In der Desktopoberfläche eine einzelne reguläre Datei zwischen
zwei bereits vorhandenen Ordnern desselben FAT32-/exFAT-Datenträgers verschieben.
Der freie Platz reicht für die Verzeichniseinträge, aber nicht für eine zweite
Kopie der Datei. Das Beispiel wählt eine Datei unterhalb der FAT32-Dateigrößengrenze;
es benötigt weder einen Ordnerkonflikt noch einen Datenträgerwechsel.

**Kandidatenbeleg:** `transfer/os/shared/engine/local.rs::copy_file` ruft auch
für diesen normalen lokalen Move `transfer_local_observed` auf.
`copy/os/shared/direct_move.rs:25–33` erlaubt die direkte Umbenennung nur nach
`PublicationAlias::create`. Diese Funktion verlangt in Zeile 160 einen
`hard_link` und liefert bei dessen Fehler in Zeile 181 `Ok(None)`.
`safe_file.rs:113–118` übernimmt dieses `None` als Wechsel in den Stagepfad;
Zeilen 133–151 kopieren die vollständige Datei in eine neue Geschwisterdatei.
Die gesicherte Quelle wird dabei bis zur erfolgreichen Publikation behalten.
Die Windowsplattform reicht den Hardlinkaufruf an `std::fs::hard_link` weiter.

**Releasebeleg:** `a814f263:copy/os/shared/safe_file.rs:93–99` rief bei einem
Move direkt `platform::move_file` für die gesicherte Quelle auf. Nur ein
tatsächlicher Datenträgerwechsel führte zum Kopierpfad. Ein Hardlink oder
zusätzlicher Platz für den Dateiinhalt war auf demselben Datenträger nicht nötig.

**Endnutzerfolge:** Ein bisher erfolgreiches Verschieben auf einem fast vollen
Wechseldatenträger scheitert jetzt beim Schreiben der Stage wegen Platzmangels.
Auch mit genügend Platz wird die Datei vollständig neu geschrieben; der
vorherige reine Namenswechsel entfällt für diese Dateisysteme.

**Gegenprüfung und Primärquelle:** Microsoft führt Hardlinks für FAT32 und
exFAT ausdrücklich als nicht unterstützt auf:
[File System Functionality Comparison](https://learn.microsoft.com/en-us/windows/win32/fileio/filesystem-functionality-comparison),
Abschnitt „Functionality“, Abruf 2026-10-26. Der Kopierfallback erhält zwar die
Quelle bei einem Schreibfehler, stellt die verlorene Verschiebefähigkeit bei
knappem Platz aber nicht wieder her. Ein kompletter Ordner kann den getrennten
Ordner-Renamepfad benutzen; der Befund wählt deshalb ausdrücklich eine einzelne
Datei. Kein Dateisystem wurde formatiert und kein Verschiebeversuch ausgeführt.

### RSOL-06 — Mittel: Der bestehende Nur-Terminal-Updateweg verliert Plugininstallation und Konfigurationsausgabe

**Auslöser:** Eine bestehende Nur-Terminal-Installation von v0.5.184 ohne
`mcp/current.v1.json` wird mit ihrem bisherigen `se update` auf den neuen Code
aktualisiert. Danach `se plugin install codex`, `se plugin install claude`
oder `se plugin config generic` aufrufen. Die Hostprogramme und deren normale
Zugänge dürfen vorhanden sein; die Abweisung erfolgt vor ihrem Aufruf.

**Vorher-/Nachherbeleg:** Die veröffentlichte
`a814f263:updater/os/shared/terminal.rs::replace_cli` lädt und ersetzt nur den
`se`-Payload. Sie startet zur Abschlussbestätigung das neu installierte `se`
mit `update --complete-install VERSION`. Der Kandidat erhält diesen Einstieg
in `cli/update.rs::complete_install` (Zeilen 260–284), führt dort aber
ausschließlich Versionsbestätigung und Workerübergabe aus. Es entsteht weder
ein Launcher noch eine initiale MCP-Generation. Die neu ergänzte
`McpTerminalUpdate::prepare` wird nur vom bereits **neuen** `replace_cli`
aufgerufen; der laufende historische Updater kann sie nicht nachträglich ausführen.

Gleichzeitig ersetzt `cli/plugin/files.rs:13–16` die bisherige
`std::env::current_exe()`-Ermittlung durch die zwingende Forderung nach
`Installation::current()`. `mcp_launcher/os/shared/installation.rs::for_image`
liefert ohne Manifest `Ok(None)`. `cli/plugin.rs:101–105` gibt den daraus
entstehenden Fehler bei Install und Config sofort zurück:
`integration_launcher_missing: reinstall or update Smart Explorer`.
Im Release benötigten beide Funktionen nur das vorhandene `se`.

**Endnutzerfolge:** Nach einem bisher unterstützten erfolgreichen Selbstupdate
lassen sich die bestehenden Claude-/Codex-Integrationen nicht mehr installieren
und die bisherige generische MCP-Konfiguration nicht mehr ausgeben. Der Fehler
fordert ein Update an, obwohl gerade dieser reguläre Updateweg ausgeführt wurde.

**Gegenprüfung und Grenze:** Eine Neuinstallation mit dem neuen vollständigen
Installationsweg oder eine erneute Installation durch den bereits neuen
Terminalupdater kann eine Generation bereitstellen. Das behebt die fehlende
Migration beim ersten historischen Update nicht. Ein schon auf `se mcp`
zeigender Hosteintrag wird hier ausdrücklich nicht als komplett ausgefallen
behauptet: `launch_from_installed` erhält ohne Manifest den Legacy-Serverweg,
und dessen fehlgeschlagene Profilaktualisierung wird lediglich protokolliert.
Der Befund betrifft ausschließlich die drei genannten bestehenden Aufrufe.
Kein Update, kein Hostaufruf und keine Profiländerung wurde ausgeführt.

### RSOL-07 — Mittel: Unveränderte installierte v0.5.184-Plugins lassen sich nicht unmittelbar deinstallieren

**Auslöser:** Ein mit v0.5.184 erzeugtes Claude- oder Codex-Profil bleibt
unverändert erhalten; der aktualisierte `se` wird mit `plugin uninstall claude`
beziehungsweise `plugin uninstall codex` aufgerufen, bevor eine erfolgreiche
Profilmigration stattgefunden hat. Das tritt insbesondere beim historischen
Nur-Terminal-Update aus RSOL-06 auf. Die Fehlerursache hier ist die zusätzliche
Deinstallationsprüfung, nicht die fehlende Launcherdatei.

**Kandidatenbeleg:** `cli/plugin.rs` erlaubt Uninstall ausdrücklich auch dann,
wenn die Launcherauflösung fehlschlägt. Anschließend rufen
`cli/plugin/claude.rs::uninstall` und `cli/plugin/codex.rs::uninstall` jedoch
sofort `files::verify_owned` auf. `cli/plugin/files.rs:54–57` verlangt das neue
`fingerprints`-Objekt und verweigert jeden alten Marker mit
`integration_conflict: legacy marker requires migration`. Der reguläre
Release-Marker enthält nur Version und ausführbare Datei. Keine der beiden
Deinstallationsfunktionen übernimmt den bereits vorhandenen
`legacy_file_matches`-Nachweis. Auch bei exakt bekannten, unveränderten
Releasebytes kommt der Deinstallationsaufruf nicht bis zur Hostabmeldung.

**Releasebeleg und Gegenprüfung:**
`a814f263:cli/plugin/files.rs::write_marker` schrieb den alten Marker;
`remove_generated` akzeptierte diesen Marker. Die beiden alten
`uninstall`-Funktionen entfernten die entsprechende Hostregistrierung und
anschließend die markierten Dateien. Der im Kandidaten fest hinterlegte
`LEGACY_SKILL_SHA256` stimmt mit dem statisch berechneten SHA-256 der
Release-Skilldatei überein; der Auslöser benötigt also keine bearbeitete oder
fremde Datei. Install/Refresh besitzt einen Legacy-Migrationsweg, Uninstall
besitzt ihn nicht. Eine vorherige erfolgreiche Migration umgeht diesen Befund.

**Endnutzerfolge:** Ein bisher unmittelbar entfernbares, eigenes Plugin bleibt
nach dem Update registriert und installiert, bis der Nutzer es zuerst migriert
oder außerhalb des bisherigen Deinstallationsbefehls entfernt. Unveränderte
eigene Releaseprofile werden dabei wie ein Inhaltskonflikt behandelt.
Kein Plugin wurde installiert, migriert, aufgerufen oder gelöscht.

### RSOL-08 — Mittel: Windows-Deinstallation hinterlässt ausführbare MCP-Installation

**Auslöser:** Die aktualisierte Windowsinstallation regulär deinstallieren.
Für die zusätzlich sichtbare Folge bleibt ein mit dem Kandidaten registriertes
Claude-/Codex-Profil oder die generische Launcherregistrierung bestehen.
Während der Deinstallation muss keine Sitzung laufen.

**Kandidatenbeleg:** Der neue Installationsweg in `native/installer.nsi:98–118`
übergibt den Bundleaustausch an den verifizierten Helfer.
`mcp_launcher/os/shared/provision.rs:28–48` legt dafür vollständige
Programmkopien in `mcp/runtimes/<sha256>/se.exe` und
`mcp/launchers/<sha256>/se-mcp-launcher.exe` an; die bestätigte Installation
besitzt `mcp/current.v1.json`, `installation.id` und ihre Locks.
Der vollständige Uninstallabschnitt (`native/installer.nsi:309–399`) entfernt
weiterhin nur die Programme im Hauptordner sowie die ausdrücklich aufgeführten
Installationsdateien. Er entfernt oder widerruft keine MCP-Generation.
Zeile 394 führt ausschließlich `RMDir "$INSTDIR"` ohne `/r` aus und wertet
den Fehler nicht aus. Der Deinstallationseintrag wird trotzdem gelöscht.

`Installation::load` verlangt keine `se.exe` im Hauptordner. Der registrierte
Launcher prüft in `launcher.rs:23–56` sein eigenes unverändertes Image, das
zurückgelassene Manifest und die ebenfalls zurückgelassene Runtime. Danach
kann er diese Runtime wieder mit `mcp` starten. Die Zulassung in
`admission.rs::check_state`/`acquire_with` verlangt ebenfalls keine inzwischen
gelöschte Hauptordner-CLI. Die optionale Profilaktualisierung kann daran
scheitern; das stoppt den MCP-Server nicht.

**Releasebeleg:** `a814f263:native/installer.nsi` installierte und entfernte
die ausführbaren App-, Updater- und CLI-Dateien im Hauptordner. Die neuen
ausführbaren Generationen gab es nicht. Ein dort registrierter MCP-Einstieg
zeigte auf die bei der Deinstallation gelöschte CLI.

**Endnutzerfolge:** „Deinstallieren“ entfernt die neu verwalteten Programmbytes
nicht mehr. Der Installationsordner bleibt mit vollständigen CLI-Kopien zurück;
ein weiterhin registrierter Launcher kann Smart Explorer nach der vermeintlichen
Entfernung erneut als MCP-Server starten. Das ist eine Regression der bestehenden
Deinstallationsfunktion und kein Wunsch nach Entfernung von Nutzerdaten.

**Gegenprüfung und Primärquelle:** Der Uninstallabschnitt besitzt keinen
Generationscleanup und keinen Aufruf eines solchen Helpers. Prozessbeendigung
am Anfang der Deinstallation löscht diese Dateien nicht und verhindert keinen
späteren Neustart. Die offizielle
[NSIS-Referenz zu RMDir](https://nsis.sourceforge.io/Docs/Chapter4.html#rmdir)
bestätigt: Ohne `/r` werden nur vollständig leere Verzeichnisse entfernt;
Abruf 2026-10-26. Der Befund verlangt keine pauschale rekursive Löschung
unbekannter Inhalte. Es wurde weder installiert noch deinstalliert und kein
Launcher gestartet.

### RSOL-09 — Hoch: Vorübergehend verweigerter Windows-Aufnahmeneuaufbau bleibt dauerhaft ohne Bilder

**Auslöser:** Eine laufende Windows-Fensteransicht ändert ihre Größe; WGC
muss die Aufnahme neu aufbauen. Der neue Plan passt nach Freigabe der noch
lebenden alten Bilder beziehungsweise Encoderressourcen in das Prozessbudget,
bekommt im ersten Versuch aber `WouldBlock`. Derselbe verlorene Retry entsteht
beim DXGI-Neuaufbau nach Access-Lost. Der Auslöser ist ausdrücklich eine
vorübergehende Belegung, kein dauerhaft zu großer Aufnahmeplan.

**Kandidatenbeleg:** `screen/os/windows/capture_wgc.rs::restart` (123–128)
ruft zuerst `stop` auf. Damit wird `live` entfernt und die vorherige Grant
retiriert. `CaptureBudget::replace` in `capture_slots.rs:109–118` verwirft
die Grant und erhöht die Generation, bevor `approve(plan)?` erfolgreich ist.
`capture_authority.rs::fund` liefert bei der noch belegten alten Grant und
überschrittener Gesamtobergrenze gerade `WouldBlock`. Dann bleiben
`live=None` und `grant=None` bestehen.

Der normale Workerpfad behandelt diesen Fehler in
`host_worker_produce.rs::produce` als `StageFrame::Unchanged`; bei geänderter
Generation gibt er außerdem den alten Encoder frei. Das ermöglicht zwar eine
spätere Zuteilung, aber die nächste WGC-Aufnahme liefert wegen
`if self.live.is_none() { return Ok(SourceFrame::Unchanged); }` in
`capture_wgc.rs:156` keine weitere Neuaufbauanforderung. DXGI besitzt denselben
Endzustand über `capture_dxgi.rs:229–234` und die frühe Rückgabe in Zeile 188.
`WindowsSource::capture` setzt für diese intern ausgelösten Neuaufbauten
keinen `renew`-Retry. Der umgebende `AdmittedSource` besitzt bereits eine
aktive Source und bereitet sie ebenfalls nicht neu vor.

**Releasebeleg:** `a814f263:screen/os/windows/capture_wgc.rs::read` aktualisierte
bei neuer ContentSize den vorhandenen Pool mit `Recreate`; es gab keine neue
fehlbare Grantzuteilung zwischen Poolabbau und Wiederverwendung.
`a814f263:screen/os/windows/capture_dxgi.rs::reopen` ersetzte die vorhandene
Duplication erst nach erfolgreichem `duplicate`. Der neue leere Dauerzustand
nach einem als vorübergehend behandelten Zulassungsfehler existierte nicht.

**Endnutzerfolge:** Nach einer Größenänderung oder einem wiederherstellbaren
DXGI-Wechsel bleibt die bestehende Sitzung verbunden, liefert jedoch auch
nach Freigabe des Speichers dauerhaft keine Bilder mehr. Der Nutzer muss
einen neuen Aufnahme-/Sitzungsstart auslösen.

**Gegenprüfung:** Ein Neuaufbau, dessen erste Zulassung gelingt, betrifft den
Befund nicht. Ein zusätzlicher Sichtbarkeitswechsel kann über
`set_capture_active(true)` einen neuen Versuch anstoßen; normales Weiterlaufen
der sichtbaren Ansicht und Keyframeanforderungen tun dies nicht. Ein aus
SecureDesktop über `WindowsSource.renew` stammender Retry ist ein getrennter
Pfad und bleibt wiederholbar. Der Befund behauptet deshalb keinen generellen
Ausfall sämtlicher Windows-Wiederanläufe. Hergeleitet aus den vollständigen
Produktkörpern; kein Hardware- oder Fensterlauf wurde ausgeführt.

### RSOL-10 — Hoch: Android-Fortsetzung verlangt das volle Aufnahmebudget doppelt

**Auslöser:** Eine zunächst funktionierende Android-Bildschirmansicht ausblenden
und wieder einblenden oder ihre Encoderkonfiguration ändern. Bezeichne das
gemeldete Plattformbudget mit B und die vollständigen alten/neuen Aufnahmepläne
mit P und Q. Der Fehler tritt bei P ≤ B, Q ≤ B, aber P + Q > B auf. Zwei
gleichgroße Pläne über der halben Plattformkapazität erfüllen dies bereits;
die erste Grant kann wegen der nativen Aufnahmegeometrie auch größer sein
als der später angeforderte Ersatzplan. Das sind explizite Ressourcenbedingungen,
keine Behauptung über die Heapgröße eines bestimmten Nutzergeräts.

**Kandidatenbeleg:** `screen/os/android/capture_admission.rs::begin` verwendet
die erste vorhandene Grant höchstens beim allerersten `begin` wieder (60–68).
Jeder spätere Aufruf verlangt in Zeile 72 `authority().approve(plan)?`, bevor
Zeile 75 die alte Grant retiert. `capture_authority.rs::fund` addiert dabei
die vollständigen Planmaxima und weist P + Q > B als `WouldBlock` ab. Keine
Freigabe tatsächlicher Kotlin-Puffer reduziert die noch nicht retirierte
vollständige Reservierung in `State.current`.

`ScreenHostControl.kt:232–245` beendet beim Verbergen ImageReader/Encoder und
löst die Displayoberfläche ab; die Rust-Grant bleibt trotzdem vollständig
reserviert. `BridgeSource::set_capture_active` setzt den Encoder-Epoch auf
`None`. Nach dem Einblenden sendet der nächste Frame daher wieder `configure`.
`ScreenHostControl.applyConfig` ruft `ScreenCaptureBudget.open` auf (290),
welches über JNI erneut `CaptureSession::begin` verlangt. Dessen Ablehnung
wird als Encoderfehler gemeldet. Ein Wiederholungsversuch bleibt unter
derselben Budgetbedingung abgewiesen, selbst wenn kein altes Captureobjekt
mehr lebt.

**Releasebeleg:** `a814f263:screen/ScreenHostControl.kt::applyConfig` erstellte
den neuen Encoder und ersetzte die Displayoberfläche, ohne eine zweite
vollständige Sessionreservierung zu verlangen. Ein unverändertes erneutes
Ansehen benötigte diesen neuen Zulassungsschritt ebenfalls nicht.

**Endnutzerfolge:** Eine vorher funktionierende H.264-Ansicht kann nach dem
Zurückkehren dauerhaft auf Tiles zurückfallen. Falls auch P + Q_tiles > B
gilt, scheitert anschließend die gesamte Ansicht. Ein Fehler des Plattformcodecs
oder tatsächlich noch belegte alte Bildressourcen sind dafür nicht notwendig.

**Gegenprüfung:** Es wird kein pauschaler Ausfall jedes Android-Neustarts
behauptet: P + Q ≤ B funktioniert, und der erste `begin` kann die vorhandene
Grant übernehmen. Der normale Pipelinezweig fängt H.264-Encoderfehler ab und
wählt Tiles; deshalb ist Sitzungsabbruch nur unter der zusätzlich genannten
Tiles-Bedingung belegt. Bei erfolgreicher Neuzulassung reduziert `retire_grant`
die alte Reservierung tatsächlich; gerade dieser Aufruf wird im Fehlerfall
nicht erreicht. Rein statischer Nachweis; kein Telefonlauf.

### RSOL-11 — Hoch: X11-Größenwechsel kann dauerhaft auf dem alten Encoderbudget warten

**Auslöser:** Eine bestehende X11-Fensteransicht mit bereits erzeugtem
Tiles- oder Hybridreferenzbild vergrößern. Der vollständige neue Aufnahmeplan
passt allein in die Plattformkapazität B, zusammen mit den noch gehaltenen
alten Encoderpixeln aber nicht. Bei stabiler Qualitäts-/FPS-Einstellung und
unverändert sichtbar bleibender Ansicht entsteht der folgende Wartezustand.

**Kandidatenbeleg:** `screen/os/linux_os/x11_capture_memory.rs::admit_geometry`
entfernt die alten SHM-/Cachebilder und retiert die alte Grant (75–83), fordert
die neue Grant jedoch vor dem Wechsel der Aufnahmegeneration und vor
`note_geometry` an (85–89). Die alte Grant bleibt dadurch mit den tatsächlich
lebenden Encoderclaims belastet. `AdmittedTilesEncoder.previous` hält eigene
`CodecInput`-Pixel; `CapturedHybrid.previous` hält ebenfalls eine eigene
Pixelkopie. Beide werden erst mit dem betreffenden Encoder freigegeben.

`host_worker_produce.rs::produce` entfernt den Encoder nach `capture` nur bei
geänderter `source.reference_generation` (34–47). Der fehlgeschlagene
X11-Zulassungsversuch erreicht `note_geometry` gerade nicht und ändert diesen
Wert nicht. Der Worker gibt `WouldBlock` deshalb als `Unchanged` zurück und
behält seinen alten Encoder. Jeder folgende Captureversuch trifft auf
dieselben alten Encoderclaims und scheitert erneut. Eine Keyframeanforderung
führt ebenfalls zuerst wieder durch diese Capturestelle; `encoder_for` wird
erst nach erfolgreicher Aufnahme erreicht.

**Releasebeleg:** `a814f263:screen/os/linux_os/x11_capture.rs::pass_window`
übernahm die neue Fenstergröße und las deren Bild über `read`; es gab keine
vorherige Zulassungsabhängigkeit von den alten Encoderpixeln. Die bestehende
Pipeline konnte danach die geänderte Sourcegröße erkennen und den Encoder
ersetzen. Der jetzt vorhandene zyklische Freigabe-/Zulassungszwang ist neu.

**Endnutzerfolge:** Das geänderte Fenster bleibt im Betrachter dauerhaft auf
dem alten Bild stehen, obwohl die neue Aufnahme nach Abbau des alten
Encoders zulässig wäre. Erneutes Verbinden oder ein zusätzlicher Zustand,
der den Encoder ausdrücklich entfernt, ist nötig.

**Gegenprüfung:** Ein neuer Plan, der trotz alter Encoderclaims sofort passt,
läuft weiter. Ohne gehaltene Encoderpixel ist der genannte Zyklus ebenfalls
nicht vorhanden. Ein expliziter Pause-/Fortsetzungswechsel beziehungsweise
Quell-/Sitzungswechsel kann den Encoder separat entfernen; der Befund
betrifft das normale Weiterlaufen nach dem Fenstergrößenwechsel. X11 hat
hier einen anderen Fehler als RSOL-09: Es versucht die Zulassung erneut,
gibt aber deren selbst gehaltene Voraussetzung nicht frei. Rein statischer
Kontroll-/Besitznachweis; kein X-Server- oder Speicherlastlauf.

## Gelesene Dateien und geprüfter Änderungsumfang

### Ergänzungsauftrag zu den Ersatzimplementierungen

Der Main hat am 2026-10-26 die ausdrücklich verbliebene Lektürelücke für
neue Capture-/Admission-/Worker-/Codec-/Präsentationskörper zurückgegeben.
Die vorherige Abgrenzung dieser ausführbaren Ersatzpfade als bloße
Prüfgrenze war unzureichend; die Projektvorgabe verlangte bereits deren
statischen Review. Die acht bestätigten Befunde bleiben erhalten.
Die Fix-Arbeitskopie `/tmp/smartexplorer-review-fixes-ee75e91b` wird weder
gelesen noch verändert. Die Quellbasis bleibt ausschließlich der feste Kandidat.

Erstplan der Ergänzung: Die konkrete bisherige Bildschirmbedienung muss
weiterhin vom Quellstart über Ressourcenaufnahme, Workerkommunikation,
Codec und Frameübertragung bis zur Präsentationsbestätigung und Eingabe
funktionieren. Reine neue Login-/Appstart-/RDP-Fähigkeiten bleiben außerhalb
dieses Ergänzungsauftrags. Die tatsächlichen Quelländerungen bestimmen die
benötigten Dateikörper; Hardwarewirkung bleibt ohne Ausführung unbestätigt.

Bestands- und Entscheidungsplan: Das Änderungsinventar und die read-only
Graphabfrage benennen vorbereitete Quellen, Capturepläne/-grants, Broker,
Worker, Plattformaufnahmen und lokale Renderer als zusammenhängende
Ersatzkette. Statt eine unverstandene neue Implementierung als Umweltgrenze
auszuschließen, werden ihre ausführbaren Produktkörper gelesen und mit dem
entsprechenden Releaseweg verglichen. Priorität haben Zulassungsbedingungen,
Pflichtressourcen, Größenwechsel, Codec-Fallbacks, Lebensdauer, Abbruch,
Präsentationsbestätigung und Fortbestand von Vorschau, Vollansicht und Eingabe.

Konkreter Arbeitsplan: zuerst gemeinsame Capture-/Admission-/Transportmodelle,
dann Worker-/Codecgrenzen, anschließend Windows-, Linux- und Androidadapter,
zuletzt Desktop-/Android-Decodierung, Skalierung und Präsentation. Jede
tatsächliche neue Fehlerhypothese wird am Release und an alternativen
Produktpfaden gegengeprüft. Der Abschluss ergänzt das Leseregister und ersetzt
die alte Lektüregrenze durch den tatsächlich erreichten statischen Umfang.

### Abgeschlossene Ergänzung: Capture, Codecs und Präsentation

Die im ersten Abschluss ausdrücklich offen gelassene Produktlektüre wurde am
26.10.2026 geschlossen. Die nachstehende Liste enthält die 199 zusätzlich
gelesenen oder vollständig nachgelesenen Quelldateien. Ohne Bereichszusatz
wurden die ausführbaren Produktkörper gelesen; enthaltene Test-/Fixturekörper
sind damit nicht als vollständig fachlich geprüft bezeichnet. Ein Bereich
oder „geänderte Funktionen“ benennt eine gezielte Integrationslektüre.
Abgeschnittene Ausgaben wurden an der fehlenden Stelle nachgelesen.

- `android/app/src/main/java/app/smartexplorer/android/screen/`:
  `ScreenCaptureBudget.kt`, `ScreenCaptureBuffer.kt`, `ScreenCaptureOwnership.kt`,
  `ScreenCodecLatency.kt`, `ScreenConsentActivity.kt`, `ScreenControlService.kt`,
  `ScreenEncoder.kt`, `ScreenFrameLimits.kt`, `ScreenHostControl.kt`,
  `ScreenProjectionFrames.kt`, `ScreenProjectionIdentity.kt`, `ScreenStillSource.kt`,
  `ScreenSyntheticRetirement.kt`, `SyntheticSurfaceSource.kt`.
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/`:
  `ScreenDecoder.kt`, `ScreenHybridDecoder.kt`, `ScreenPreviewTile.kt`,
  `ScreenPullBuffers.kt`, `ScreenResample.kt`, `ScreenScaleWorker.kt`, `ScreenScaling.kt`,
  `ScreenSession.kt`, `ScreenTouch.kt`, `ScreenViewBars.kt (geänderte Funktionen)`,
  `ScreenViewScreen.kt`, `ScreenVisibility.kt`.
- `native/android-bridge/src/`:
  `screen_capture_jni.rs`, `screen_jni.rs`.
- `native/src/app/core/`:
  `screen_decode_run.rs`, `screen_decode_thread.rs`, `screen_image_memory.rs`,
  `screen_input_keys.rs`, `screen_input_map.rs`, `screen_launch.rs:1-170`, `screen_pool.rs`,
  `screen_presented.rs`, `screen_render_app.rs`, `screen_render_caps.rs`,
  `screen_render_cpu.rs`, `screen_render_glow.rs`, `screen_render_owner.rs`,
  `screen_render_upload.rs`, `screen_render_wgpu.rs`, `screen_render_yuv.rs`,
  `screen_scaling.rs`, `screen_session.rs`, `screen_session_backend.rs`,
  `screen_texture_cpu.rs`, `screen_texture_worker.rs`, `screen_tile_face.rs`,
  `screen_tile_grid.rs`, `screen_view_widget.rs`, `screen_view_widget_render.rs`,
  `screen_viewer_app.rs`, `screen_viewer_view.rs (geänderte Funktionen)`, `screen_viewport.rs`,
  `screen_visibility.rs`, `screen_wgpu_yuv.rs`, `screen_wgpu_yuv.wgsl`.
- `native/src/app/os/linux_os/`:
  `screen_visibility.rs`.
- `native/src/app/os/shared/`:
  `screen_visibility.rs`.
- `native/src/app/os/windows/`:
  `screen_visibility.rs`.
- `native/src/screen/core/`:
  `admitted_tiles_encoder.rs`, `capture_admission.rs`, `capture_authority.rs`,
  `capture_bytes.rs`, `capture_grant.rs`, `capture_grant_claims.rs`,
  `capture_grant_slices.rs`, `capture_image.rs`, `capture_import.rs`, `capture_plan.rs`,
  `capture_plan_cases.rs`, `capture_prepared_lifetime.rs`, `capture_records.rs`,
  `capture_session_context.rs`, `capture_source_contract.rs`, `capture_transport.rs`,
  `capture_transport_wire.rs`, `captured_external_hybrid.rs`, `captured_hybrid.rs`,
  `codec_registry.rs`, `control_sender.rs`, `decoded_frame.rs`, `frame_budget.rs`,
  `frame_memory.rs`, `frame_protocol.rs`, `frame_reference.rs`, `frame_regions.rs`,
  `frame_size_caps.rs`, `gop_stream.rs`, `host_admission.rs`, `host_backend.rs`,
  `host_backend_codecs.rs`, `host_capture.rs`, `host_capture_pause.rs`,
  `host_codec_adapters.rs`, `host_codec_phase_plan.rs`, `host_frame_send.rs`,
  `host_hybrid.rs`, `host_input_release.rs`, `host_mode.rs`, `host_pipeline.rs`,
  `host_pipeline_actor.rs`, `host_pipeline_init.rs`, `host_pipeline_produce.rs`,
  `host_quality.rs`, `host_reference.rs`, `host_server.rs:115-405`, `host_server_start.rs`,
  `host_session.rs`, `host_visibility.rs`, `host_work_control.rs`, `image_admission.rs`,
  `negotiate.rs`, `pixel_convert.rs`, `sender_budget.rs`, `tiles_codec.rs`,
  `viewer_reconstruct.rs`, `wire_binary.rs`, `worker_capture_resource_wire.rs`,
  `worker_capture_wire.rs`, `worker_frame_wire.rs`, `worker_reply_wire.rs`,
  `worker_request_wire.rs`, `worker_wire.rs`.
- `native/src/screen/os/android/`:
  `bridge_backend.rs`, `bridge_channels.rs`, `bridge_source.rs`, `capture_admission.rs`,
  `capture_arena.rs`, `capture_prepared.rs`, `session_memory.rs`, `viewer_sink.rs`.
- `native/src/screen/os/linux_os/`:
  `capture_prepared.rs`, `host_backend.rs`, `pipewire_abi.rs`, `pipewire_buffers.rs`,
  `pipewire_capture_memory.rs`, `pipewire_frame_copy.rs`, `pipewire_identity.rs`,
  `pipewire_import.rs`, `pipewire_import_control.rs`, `pipewire_import_events.rs`,
  `pipewire_memory_abi.rs`, `pipewire_pause.rs`, `pipewire_stream.rs`, `portal_input.rs`,
  `portal_session.rs`, `portal_source_binding.rs`, `session_memory.rs`, `spa_pod.rs`,
  `x11_admission_stream.rs`, `x11_capture.rs`, `x11_capture_memory.rs`,
  `x11_capture_shm.rs`, `x11_capture_target.rs`, `x11_cursor.rs`,
  `x11_input.rs (geänderte Funktionen)`.
- `native/src/screen/os/native_ffi/`:
  `openh264_capture.rs`, `openh264_codec.rs`, `openh264_encode.rs`.
- `native/src/screen/os/shared/`:
  `capture_broker.rs`, `capture_broker_imports.rs`, `capture_dispatch.rs`,
  `capture_dispatch_requests.rs`, `codec_inventory.rs`, `host_worker_backend.rs`,
  `host_worker_client.rs`, `host_worker_lifetime.rs`, `host_worker_main.rs`,
  `host_worker_prepare.rs`, `host_worker_prepared.rs`, `host_worker_produce.rs`,
  `host_worker_reader.rs`, `host_worker_run.rs`, `host_worker_source.rs`,
  `session_memory.rs`, `synthetic_backend.rs`.
- `native/src/screen/os/windows/`:
  `capture_dxgi.rs`, `capture_prepared.rs`, `capture_readback.rs`, `capture_retirement.rs`,
  `capture_slots.rs`, `capture_source.rs`, `capture_wgc.rs`, `d3d_device.rs`,
  `host_backend.rs`, `input_send.rs`, `mf_buffer_lease.rs`, `mf_capture_media.rs`,
  `mf_capture_output.rs`, `mf_common.rs`, `mf_decoder.rs`, `mf_encoder.rs`,
  `mf_encoder_sync.rs`, `mf_media.rs`, `secure_desktop.rs`, `session_memory.rs`.

Die tatsächlich bearbeiteten Ersatzketten reichen von der einmaligen W-
Zulassung über vorbereitete Quellen, Phase-/Ownerpläne, Grantretirierung,
delegierte Parent-/Workercredits und zugelassene Bild-/Recordtransporte bis
zu Aufnahme, Codecauswahl, Fallback, Rekonstruktion, Skalierung, GPU-/Canvas-
Präsentation und Eingabefreigabe. Die Windowsprüfung umfasst WGC, DXGI,
D3D-Readback, Rotation, Neuaufbau, native Retirierung und beide MF-
Encoderarten; die Linuxprüfung X11 mit SHM/Replygate/Cursor sowie Portal,
PipeWireimport, Pause und Quellbindung; die Androidprüfung Kotlin-Ressourcen,
JNI-Token und Rust-Channel einschließlich MediaProjection, Accessibility,
MediaCodec, Full/Preview und Surface-/Bitmapwechsel.

Vorher-/Nachhergegenproben betreffen insbesondere WGC-/DXGI-Neuaufbau,
X11-Größenwechsel, Android-Konfiguration und die bisherigen Vorschau-/
Vollansichts-, Tiles-/H.264-, Sitzungswechsel- und Eingabeverbraucher.
Die zusätzliche Fachlektüre lieferte RSOL-09 bis RSOL-11. Die bereits
berichteten RSOL-01 bis RSOL-08 bleiben unverändert inhaltlich erhalten.

Zusätzliche Gegenprüfungen ohne eigenen Fehlerbefund: PipeWire erhöht seine
Generation vor einer erneut versuchten Bildzulassung; dessen expliziter
`WouldBlock`-Retry ist deshalb vom X11-Zyklus abgegrenzt. Windows-
Treiberzeilenpadding wird im Readback zeilenweise übersprungen und nicht
als zusätzlicher Produktpuffer allokiert. Ein zunächst vermuteter dauerhafter
Android-Previewfehler durch einen Codec-Fallbacktext wurde verworfen:
Der gelesene Android-Vorschaueinstieg bietet ausschließlich Tiles; die
angenommene H.264-Fehlerfolge ist dort nicht belegt. Die alten
`open_source`-/Worker-Openpfade mit `capture_grant_required` wurden nicht
als Verlust gewertet, weil der produktive Hoststart die vorbereitete
Quelle aktiviert.

Die native OpenH264-Ausgabezuordnung wurde zusätzlich gegen Ciscos
[API von v2.6.0](https://raw.githubusercontent.com/cisco/openh264/v2.6.0/codec/api/wels/codec_api.h)
und [SBufferInfo von v2.6.0](https://raw.githubusercontent.com/cisco/openh264/v2.6.0/codec/api/wels/codec_def.h)
abgegrenzt: Eingabezeit und Ausgabezeit sind getrennte Felder;
der Kandidat verwendet für fertige YUV-Ausgaben das Ausgabefeld.
Abruf 2026-10-26. Die konkrete Rust-Wrapperquelle 0.9.9 war über die
abgerufene docs.rs-URL nicht verfügbar; daraus wird weder ein Fehler noch
ein Erfolgsnachweis konstruiert. Es wurde keine unveränderte Bibliothek
außerhalb der freigegebenen lokalen Fläche gelesen.


### Abgrenzung des vollständigen Vergleichs

Das bereitgestellte Nicht-Graph-Inventar wurde als vollständige
Vergleichsfläche verwendet: 1.529 Dateien, davon 1.111 unter `native/src/`,
134 unter `android/`, neun unter `native/android-bridge/`, 209 unter
`vendor/`, 47 Dokumentationsdateien und 19 Dateien für Wurzelkonfiguration,
Installation, Veröffentlichung und Workflows. Die 1.111 nativen Dateien
verteilen sich auf 34 Domänen mit 1.109 Dateien sowie `lib.rs` und
`login_entry.rs`. Die nachfolgende Tabelle ordnet **alle 34 Domänen** zu;
ihre Zahlen bezeichnen den Änderungsumfang, keine vollständig gelesenen
Dateikörper und keine erfolgreich ausgeführten Prüfungen.

| Zugeordnete native Domänen | Geänderte Dateien | Untersuchte bestehende Funktionsgrenze |
| --- | ---: | --- |
| `transfer`, `copy`, `vfs`, `local_access`, `source_media` | 97 | Datei-/Ordnerauswahl, Konflikte, Stagepublikation, Recoverynachweise, lokales Verschieben, optische Lesegrenzen und Backendidentität |
| `agent`, `agent_proto`, `ftp`, `sftp`, `smb`, `webdav`, `gdrive`, `zipfs` | 42 | Adapter für exklusive Ordneranlage, Datei-/Ordneridentität, Schreibstages, Kapazität und protokollierte Backendaufrufe |
| `cloud`, `creds` | 12 | Bestehende Google-Drive-Anmeldung, gespeicherte Clientbindung, Tokenrefresh und Abbruch |
| `vault`, `mount`, `daemon` | 135 | Bestehende Tresorauflösung, Dateizugriff, Laufwerksstart, Cache-/Recoverybesitz, Freigabewiderruf, IPC und Updateabschluss |
| `scanner`, `folder_index`, `analytics` | 14 | Scan, partielle Ergebnisse, Wiederleseindex, Größenfortschreibung und Indexaufbau |
| `screen`, `app`, `desktop` | 426 | Bildschirmzulassung, Aufnahme-/Codecpfade, Präsentation, Eingabe, Viewerlebensdauer, Picker, Einstellungen, lokale Laufwerke, Desktopaktionen und Fensterende |
| `share` | 105 | Vorhandene Datei-, Exec- und Bildschirmverbindungen, Rechte, Genehmigungen, Routensteuerung und Abbruch |
| `cli`, `mcp_launcher`, `updater`, `bin` | 124 | Bestehende CLI-Kopier-/Verschiebeaufrufe, MCP-Registrierung, Profilverwaltung, Selbstupdate, Installationsabschluss und Programmstart |
| `apps`, `login`, `rdp`, `remote_window` | 118 | Eintrittspunkte und Einbindung in vorhandene Start-, Share-, Bildschirm- und Beendigungspfade; keine vollständige Fachprüfung der erstmals eingeführten Fähigkeiten |
| `mobile` | 36 | Android-Transfer, Ordnerentscheidungen, DocumentsProvider-Zugriff, Tresorwiderruf, Cloudstatus und Bildschirmaufrufe |

Der Quellreview verfolgt die geänderten Verträge bis zu unveränderten
Verbrauchern. Er ist kein Zeile-für-Zeile-Lesen aller 1.529 Dateikörper.
Insbesondere neu eingeführte isolierte Fachfunktionen, Testkörper und
unveränderte Upstreamquellen wurden nicht pauschal als Bestandsregression
bewertet. Abgeschnittene breite Diffausgaben wurden nicht als vollständig
gelesene Dateikörper gezählt; befundtragende Ausschnitte wurden gezielt
erneut gelesen. Die konkret verbleibenden Grenzen stehen am Ende.

### Befundtragende und unmittelbar gegenprüfende Dateilektüre

Die folgenden Pfadgruppen benennen gelesene Quellen beziehungsweise gezielt
gelesene Änderungs-/Aufrufabschnitte. Ein Eintrag behauptet außerhalb der
benannten Kette keine vollständige Fachprüfung des ganzen Moduls.

- Grundlage: `/tmp/smartexplorer-final-review-user-contract.txt`,
  `/tmp/smartexplorer-regression-sol-review-scope.json`,
  `/tmp/smartexplorer-final-review-inventory.json`,
  `/tmp/smartexplorer-vendor-review-inventory.json`, `AGENTS.md`,
  `docs/ARCHITEKTUR.md`,
  `docs/plaene/2026-10-24-batches/regressionsreview.md` und die für
  Sicherheits-/Kompatibilitätsentscheidungen herangezogenen Abschnitte von
  `docs/plaene/2026-10-24-batches/vertraege.md` und
  `docs/AI_AGENT_PLUGIN.md`. Die read-only Graphabfrage diente der Navigation.
- Transfer unter `native/src/transfer/os/shared/engine/`: `mod.rs`, `roots.rs`,
  `folder_reservation.rs`, `selection.rs`, `folders.rs`, `folder_move.rs`,
  `file_recovery.rs`, `file_evidence.rs`, `source.rs`, `upload.rs`, `publish.rs`,
  `local.rs`, `download.rs`, `remote_copy.rs`, `folders_local.rs`, `legacy.rs`,
  `worker.rs`, `batch.rs`, `job_store.rs` und `record_io.rs`; relevante
  Releasefassungen insbesondere von `roots.rs`, `folders.rs`, `publish.rs`
  und `local.rs`. Die Befundketten für RSOL-01/02 wurden vollständig bis
  zur Fehlerweitergabe und Stagebehandlung verfolgt.
- Lokale Kopiergrenze unter `native/src/copy/os/shared/`: `direct_move.rs`,
  `publication.rs`, `publication_proof.rs`, `recovery.rs`, `safe_file.rs`,
  `staging.rs` und `move_guard.rs`; dazu die betroffenen Windows-/Linux-
  Dateiidentitäts- und Publikationsadapter sowie die Releasefassung von
  `safe_file.rs`. Unter `native/src/local_access/` wurden die Linux-
  ConfinedDirectory- und Windows-Regular-/Quarantänegrenzen gelesen.
  `native/src/source_media/reader.rs`, `permit.rs` und die zugehörigen
  Einstiege wurden hinsichtlich Reader-/Iteratorlebensdauer ausgewertet.
- Gemeinsame Backendgrenze: `native/src/vfs/core/folder_reservation.rs`,
  `file_identity.rs`, die lokalen Extension-/Optik-/Confined-Adapter sowie
  die betroffenen Extension- und Ordnerreservierungsdateien der Domänen
  `ftp`, `sftp`, `smb`, `webdav`, `gdrive`, `agent` und `zipfs`.
  Besonders: `native/src/ftp/core/ftp.rs` für STOR/RETR, freie Stagenamen
  und RNFR/RNTO; `native/src/ftp/core/folder_reservation.rs` und
  `native/src/sftp/core/folder_reservation.rs` für Unsupported;
  Agent-Protokolländerungen für Ordnerreservierung, Featureaushandlung und
  Backendaufrufe. Kapazitätsadapter wurden als geänderte Verbraucher gelesen.
- Cloud: `native/src/cloud/core/auth_state.rs`,
  `native/src/cloud/os/coordinator.rs`, `oauth_flow.rs`, `oauth_http.rs`,
  `session_store.rs`, die geänderten Konfigurations-/Cloudkernabschnitte,
  `native/src/creds/os/shared.rs`, die Google-Drive-Auth-/Stateänderungen
  und `native/src/mobile/os/shared/domains/gdrive.rs`.
- Tresor unter `native/src/vault/`: `core/open_policy.rs`, `core/sealed.rs`,
  `os/shared/open_policy.rs`, `backend.rs`, `backend_dirs.rs`,
  `backend_extensions.rs`, `backend_identity.rs`, `backend_io.rs`,
  `backend_listing.rs`, `backend_write.rs`, `local_store.rs`,
  `registry_store.rs` und `session_seal.rs`. Bestehender Dateizugriff und
  Zustimmungsbindung wurden mit `native/src/app/os/shared/vault_open.rs`,
  `native/src/daemon/os/shared/mount_paths.rs` und den Android-Providergrenzen
  gegengeprüft.
- Laufwerke/IPC: geänderte Abschnitte von
  `native/src/mount/core/cache_space.rs`, `spool.rs`,
  `native/src/mount/os/shared/capacity_refresh.rs`, `host_capacity.rs`,
  `host_session.rs`, den Linux-FUSE-Metadaten- und Windows-Metadaten-Callbacks;
  `native/src/daemon/os/shared/mount_manager.rs`, `mount_manager_start.rs`,
  `mount_manager_start_cache.rs`, `mount_registry.rs`, `rooted_backend.rs`,
  `rooted_backend_extensions.rs`, `rooted_backend_gate.rs`,
  `ipc_protocol_io.rs`, `runtime_hello.rs`, `exec_ipc.rs` und die jeweils
  betroffenen IPC-/Runtimeanforderungen. Neue Recovery-Export-/Importkörper
  wurden nicht vollständig fachlich geprüft.
- Scan/Index/Analyse: die geänderten Collector-/Walk-/Retryabschnitte unter
  `native/src/scanner/`, `native/src/folder_index/os/shared.rs`, `os/walk.rs`,
  `native/src/analytics/core/partial_merge.rs`,
  `native/src/analytics/os/shared/analytics.rs`, `analytics_outcome.rs`,
  `native/src/app/core/read_access.rs`, `analytics_access.rs`, `scanning.rs`,
  `drive_snapshot.rs` und die einbezogenen Laufwerksinventaradapter.
- Bildschirmprotokoll und Budget unter `native/src/screen/core/`:
  `frame_budget.rs`, `frame_memory.rs`, `viewer_client.rs`, `host_server.rs`,
  `host_server_start.rs`, `host_backend.rs`, `host_session.rs`,
  `host_capture.rs`, `host_mode.rs`, `host_pipeline.rs`, `codec_registry.rs`,
  `negotiate.rs`, `gop_stream.rs`, `wire.rs`, `wire_binary.rs`, `wire_control.rs`,
  `snapshot.rs`, die ausgegliederten Pipeline-/Qualitäts-/Referenzpfade und
  die geänderten Familienlisten-/Eingabefreigabeabschnitte. Releasefassungen
  wurden für Handshake, Vorschau, Codec-/Pipeline- und Wireverhalten verglichen.
- Plattformbildschirm: `native/src/screen/os/shared/session_memory.rs`,
  `image_memory.rs`, `host_worker_backend.rs`, die Worker-Client-/Mainänderungen,
  `native/src/screen/os/android/session_memory.rs` und Bridgeänderungen;
  Windows `capture_source.rs`, `capture_prepared.rs`, Capture-/MF-Änderungen;
  Linux `capture_prepared.rs`, `portal_source_binding.rs`, Portal-/PipeWire-
  Änderungen und die relevanten X11-/Hostbackendabschnitte;
  `native/src/screen/os/native_ffi/openh264_codec.rs` und dessen Encoderpfad.
  GPU-/Treiberwirkungen wurden damit nicht ausgeführt oder als erfolgreich bestätigt.
- Desktoppräsentation/Bedienung unter `native/src/app/`: geänderte
  `core/screen_decode_thread.rs`, `screen_session.rs`, `screen_tile_grid.rs`,
  `screen_tile_face.rs`, `screen_view_widget.rs`, `screen_viewer_app.rs`,
  `screen_input_keys.rs`, `screen_input_map.rs`, Renderer-/Sichtbarkeitsgrenzen,
  `frame_update.rs`, `picker_impl.rs`, `picker_async.rs`, `preview_core.rs`,
  `transfer_route.rs`, `transfer_jobs.rs`, `transfer_window.rs`,
  `settings_background.rs`, `settings_ai_plugin.rs`, die betroffenen
  Einstellungen-/Share-/Desktopaktionsabschnitte;
  `os/shared/sync_exit_gate.rs`, `rdp_exit_gate.rs` und `vault_open.rs`.
  Die Änderungen an Desktopkonfiguration, Dateiaufruf, Zielauflösung und
  Hintergrundbildquellen wurden über ihre jeweiligen Aufrufstellen gelesen.
- Share unter `native/src/share/core/`: `access_grant.rs`, `access_grant_wire.rs`,
  `access_approval.rs`, `route_state.rs`, `route_policy.rs`,
  `route_protocol.rs`, `node_routes.rs`,
  `node_screen_link.rs`, die betroffenen `node.rs`-/`node_accept.rs`-/
  `server.rs`-/`service.rs`- und Sessionabschnitte; außerdem geänderte
  Exec-Zulassungs-/Client-/Server-/Session- und Discovery-Abbruchpfade.
  Hinzu kommen `native/src/share/os/shared/route_adapter.rs` und
  `route_monitor.rs`.
  Die neuen App-/Login-/RDP-/Fensterfamilienmodule wurden über `mod.rs`,
  ihre Einbindung in Share/GUI und `native/src/login_entry.rs` betrachtet.
- CLI/MCP: `native/src/cli/ops_copy.rs`, `ops_move.rs`, `folder_conflict.rs`,
  `tree_spool.rs`, `tree_apply.rs`, `tree_plan.rs`, geänderte Tree-/Observer-
  Aufrufe, `plugin.rs`, `plugin/files.rs`, `claude.rs`, `codex.rs`, `managed.rs`,
  `config.rs`, `update.rs` und relevante MCP-Start-/Dispatchabschnitte;
  `native/src/mcp_launcher/os/shared/installation.rs`, `launcher.rs`,
  `admission.rs`, `generator.rs`, `provision.rs`, `installer_cli.rs` und
  die betroffenen Plattform-Start-/Sicherheitsgrenzen. Releasefassungen
  von Pluginmarker, Installation, Deinstallation und Selbstupdate wurden
  eigens verglichen. Der Legacy-Skillhash wurde aus den Releasebytes berechnet.
- Update/Installation: `native/src/updater/os/shared/terminal.rs`,
  `legacy_launcher.rs`, `feed_launcher.rs`, die geänderten Feed-/Apply-/
  CLI-Swap-/Staging- und Plattformabschnitte;
  `native/src/bin/se.rs`, `se_mcp_launcher.rs`,
  `smart_explorer_updater.rs`, `smart_explorer_updater/installer.rs` und
  dessen Argumentgrenze; `native/installer.nsi`, insbesondere der gesamte
  Uninstallabschnitt, sowie `native/src/shell_register/os/windows.rs::unregister_all`.
  `install-linux.sh` wurde für Assetauswahl, CLI-only und Bundleinstallation
  gelesen. Änderungen an Publishskripten und Buildworkflow wurden als
  Installations-/Assetintegration gelesen, ohne sie auszuführen.
- Mobile unter `native/src/mobile/os/shared/`: `transfer.rs`, `fs_edit.rs`,
  die betroffenen `domains/screen_view.rs`, `screen_view_json.rs`,
  `screen_view_run.rs`, `screen_host.rs`, `docs.rs`, `docs_engines.rs`,
  `docs_access.rs`, `docs_pending.rs`, `docs_io.rs`, `docs_vault.rs`,
  `gdrive.rs`, `vaults.rs`, `vault_open_policy.rs`, die geänderten
  Share-/Screen-/Login-/App-/RDP-/TCP-Einbindungen und Dispatchabschnitte.
  Die JNI-Änderungen unter `native/android-bridge/src/` wurden als native
  Integrationsgrenze betrachtet; die erstmals hinzugefügten Login-Bildkörper
  wurden nicht vollständig als neue Fachfunktion geprüft.
- Android unter `android/app/src/main/java/app/smartexplorer/android/`:
  `api/ScreenApi.kt`, `screen/ScreenFrameLimits.kt`, `ScreenEncoder.kt`,
  geänderte Capture-/Control-/Stillabschnitte, `ui/screen/ScreenSession.kt`,
  `ScreenPreviewTile.kt`, `ScreenVisibility.kt`, die Decoder-/Viewänderungen;
  `ui/viewer/MediaViewer.kt`, `MediaGeometry.kt`, `ImagePage.kt`,
  `MediaPlaybackState.kt`, `PlayerPage.kt`; `launcher/ui/DragLayer.kt`,
  `HomeScreen.kt`, `launcher/widgets/WidgetView.kt`,
  `ui/files/FileActions.kt`, `FileDialogs.kt`, `FilesOverlays.kt`,
  `ui/picker/LocationPickerDialog.kt`, `MainActivity.kt`, die betroffenen
  Navigation-/Share-/Settings-/Vault-/DocumentsProvider- und API-Änderungen.
  Zusätzlich Manifest-, Gradle- und APK-Packagingänderungen; keine Ausführung
  der Android-Test- oder Instrumentationdateien.

### Vendorte Abhängigkeiten

Die vier freigegebenen `.crate`-Originalarchive wurden ausschließlich
read-only im Speicher mit dem Kandidaten verglichen. Ihr SHA-256 entspricht
jeweils der im Release-Lockfile dokumentierten Prüfsumme. Ergebnis des
eigenen Bytevergleichs: Iroh 1.3.0 mit 80 identischen und vier geänderten,
Noq 1.3.0 mit 31 identischen und zwei geänderten, Noq-proto 1.3.0 mit
61 identischen und acht geänderten sowie egui_glow 0.29.1 mit neun
identischen und drei geänderten archivierten Dateien. Hinzu kommen die
im Vendorinventar einzeln genannten neuen Dateien. Es wurde nichts in den
Produktbaum extrahiert.

Gelesen wurden die tatsächlichen Änderungen in Irohs `endpoint.rs`,
`endpoint/connection.rs`, `endpoint/quic.rs`,
`socket/remote_map/remote_state.rs` und die neue
`application_path_retirement.rs`; Noqs `connection.rs` und `lib.rs`;
Noq-protos `config/transport.rs`, `connection/datagrams.rs`,
`connection/mod.rs`, `connection/packet_builder.rs`,
`connection/streams/state.rs`, `frame.rs`, `lib.rs`, `n0_nat_traversal.rs`
sowie die neuen `application_path_policy.rs` und
`application_path_revalidation.rs`; egui_glows `Cargo.toml`, `src/lib.rs`,
`src/painter.rs` und die neue `src/error_scope.rs`.
Die nativen Verbraucher der Routen-/Serialisierungsänderungen wurden
an `route_adapter.rs`, `route_protocol.rs`, `route_monitor.rs` und den
Nodeeinstiegen verfolgt. Die 181 byteidentischen Dateien wurden nicht
erneut vollständig fachlich gelesen. Ihre Identität ist belegt; daraus
folgt keine Aussage über alle möglichen Wechselwirkungen des neuen Codes.

## Erstellte oder geänderte Dateien

Ausschließlich `docs/plaene/2026-10-24-batches/review-regression-sol.md`.

## Entscheidungen und verbleibende Prüfgrenzen

### Getroffene Einordnungen

- Elf Befunde erfüllen den Nachweismaßstab: konkreter bisheriger Bedienweg,
  neue brechende Bedingung, alte funktionierende Quellkette, sichtbare Folge
  und Prüfung eines möglichen Gegenwegs. Es gibt keine Mindestbefundzahl.
  Schwere Hoch bezeichnet hier den Ausfall eines normalen bestehenden
  Übertragungs-, Verschiebe- oder Bildschirmwegs; Mittel die konkret
  eingeschränkten Installations-/Entfernungswege. Das ist keine CVSS-Wertung.
- Die neue Klartext-/Rootzustimmung bei Tresoren, die Bindung von
  Cloudtokens an Client und Konto sowie unverifizierbare Android-
  Appfensteridentität wurden gegen die beauftragten Sicherheitsverträge
  abgegrenzt. Eine bewusst verlangte Sicherheitsgrenze wurde nicht allein
  wegen ihrer Neuheit als Bestandsfehler gemeldet.
- Die Quelle des lokalen Move war bereits im Release als lesbarer Handle
  geöffnet. Daraus wurde keine zusätzliche neue Leserechtsregression
  konstruiert. Der belegte FAT32-/exFAT-Fehler betrifft die neue
  Hardlinkvoraussetzung und den dadurch erzwungenen Vollkopierpfad.
- Der Android-Metadatenkanal umgeht die prozessweite Bildsessionlease.
  Er wurde deshalb nicht fälschlich den Speicherfehlern RSOL-03/04
  zugerechnet. Der direkte Produktionsviewer besitzt diese Lease tatsächlich.
- Die optischen Reader und Directory-Iteratoren geben ihre Sperre vor
  einer anschließenden Metadatenoperation beziehungsweise vor Rückgabe
  des Iteratoritems frei. Für diese untersuchten Ketten wurde kein
  angeblicher Doppel-Lock-Deadlock gemeldet.
- Die Android-Quellauswahl verlangt bereits gleiche Backendart und
  gleichen Prefix. Der reine Pfadvergleich in der Vorschauzuordnung
  wurde daher für diesen Bedienweg nicht als bewiesene Kollision
  verschiedener Remoteidentitäten gemeldet.
- Bestehendes `se mcp` ohne Manifest besitzt einen Legacyfallback.
  RSOL-06 wurde ausdrücklich auf die drei belegten Installations- und
  Konfigurationsaufrufe begrenzt. Nicht ausgeführte Programme oder
  fehlende Livezugänge sind kein Fehlerbeleg.

### Exakt verbleibende Grenzen

1. Kein Windows-, Linux- oder Androidlauf, kein FTP-/SFTP-/WebDAV-/SMB-/
   Google-Drive-Konto, kein gleichzeitiger Bildschirmbetrieb und kein
   Live-NAT-/Relay-/Netzwechsel wurde ausgeführt. Es gibt keinen Laufzeitpass.
   Die Auslöser der elf Befunde sind statisch hergeleitet und als solche
   bezeichnet. Die Folge unter ihren jeweiligen Bedingungen ist durch die
   konkrete Quellkette belegt.
2. Keine vollständige Prüfung der erstmals eingeführten Login-, Appstart-,
   RDP-, Fensterfamilien-, Recovery-Import-/Export- und Loginservice-
   Implementierungen als eigenständige neue Funktionen. Geprüft wurden ihre
   genannten Berührungspunkte mit dem bestehenden Bestand. Insbesondere
   `native/install-login-service.ps1` und `.sh` wurden nicht vollständig
   fachlich geprüft. Aus diesem Review folgt keine Freigabe dieser neuen
   administrativen Bedienwege.
3. GPU-/Treiberwirkungen von DXGI, WGC, Media Foundation, D3D, OpenGL,
   WGPU, PipeWire, X11 und Android MediaCodec bleiben ohne Plattformlauf
   unbestätigt. Die ersetzenden ausführbaren Capture-/Admission-/Worker-/
   Codec-/Präsentationskörper und betroffenen Plattformadapter wurden im
   Ergänzungsauftrag fachlich gelesen; diese Grenze bezeichnet keine
   ausgelassene Produktlektüre. Nicht statisch bestätigt sind erfolgreiche
   dynamische Symbolauflösung, konkrete Treiberformate, Callbackreihenfolge,
   Hardwarecodec-Latenz, GPU-Ausführung und reale Druck-/Abbruchtimings auf
   den jeweiligen Geräten. Ein Beweis sämtlicher möglichen Interleavings
   oder eine vollständige Fachlektüre unveränderter Bibliotheken wurde
   nicht erbracht und wird nicht behauptet.
4. Neu hinzugefügte Testkörper, Fixtures, Suite-Manifest und Suiteimplementierung
   wurden nicht vollständig als automatischer Nachweis geprüft und nicht
   ausgeführt. Ein vorhandener Testname oder eine Erwartung im Test ist hier
   kein bestätigtes Produktverhalten. Es wurde keine Suite definiert oder
   geändert; diese Zuständigkeit liegt beim Main.
5. Nicht jede der 47 Dokumentationsänderungen und nicht jeder Körper der
   19 Konfigurations-/Installations-/Workflowdateien wurde vollständig gelesen.
   Die angegebenen Quellenabschnitte und Integrationsänderungen sind die
   tatsächliche Lektüre. Historische Dokumente wurden nicht als Beleg für
   vorhandenes Endnutzerverhalten übernommen.
6. Die Vergleichsbasis der tatsächlichen letzten Veröffentlichung stammt aus
   dem dokumentierten API-Nachweis des Main. Eigene Prüfung installierbarer
   Releasebytes, Signaturen, Installerlauf, GitHub-Release-Sichtbarkeit,
   vollständiger Assetkohärenz oder Veröffentlichung fand nicht statt.
7. Die read-only Graphabfrage ist ein Navigationshinweis aus dem vorhandenen
   Kandidatengraphen; ihre gekürzte Ausgabe ist keine vollständige
   Abhängigkeitskarte. Die Paket-/Skillversionsabweichung wurde nicht durch
   Installation oder Graphänderung behoben. Keine zusätzliche Projektlesefläche
   wurde eigenmächtig erschlossen.
8. Die unabhängige Gesamtreview und der zweite Regressionsreview wurden nicht
   gelesen und ihre Autoren nicht kontaktiert. Es wurde keine agentenübergreifende
   Fehlerliste, kein Fixplan, keine Umsetzung und keine nachgelagerte
   Anti-Regressionsfreigabe erzeugt. Diese Arbeit bleibt dem zuständigen
   Fixautor vorbehalten.

Keine ungelesene oder statisch unentscheidbare Funktion wird mit diesem
Abschluss als regressionsfrei bestätigt. Der Bericht ist das abgeschlossene
eigene Reviewresultat innerhalb der freigegebenen Fläche; die elf Befunde
bleiben im festen Kandidaten unbehandelt. Die separate Fixarbeitskopie
`/tmp/smartexplorer-review-fixes-ee75e91b` wurde nicht gelesen; über ihren
Fixstand wird keine Aussage getroffen.
