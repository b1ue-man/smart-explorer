# Planungsbelege zum Übergabeauftrag

Stand: 2026-10-25. Produktbestand bei Beginn: Commit `aec62a40`.
Diese Datei enthält ausschließlich Projektwissen. Private Betriebsschritte,
Gerätekennungen, Zugangsdaten und Kontingente sind kein Bestandteil.

## Auftrag und Erstplan

Nutzer am 2026-10-25: „du stoppst wenn der plan steht, du setzt ihn noch nicht um“.
Ergebnis: recherchierter, kritisch geprüfter Umsetzungsplan; keine Umsetzung,
Builds, Tests, Versionierung oder Veröffentlichung von Programmdateien.
Annahme zum Umfang: alle 27 Punkte aus `batches.md` plus die Ergänzungen P31,
mit den dort bereits gesetzten Ausschlüssen. Das Ziel umfasst Planung und Kritik,
nicht das Starten eines Entwicklungslaufs. Der bisherige Fünferzuschnitt wird
erst nach dem Detailplan neu bewertet.

Erstplan: Übergabe und Bestandsverträge prüfen; Lösungswege anhand von
Primärquellen vergleichen; Meilensteine mit Abhängigkeiten und erwarteten
Ergebnissen formulieren; verbliebene Planlücken recherchieren; Kritik einarbeiten;
nach dem abgeschlossenen Plan stoppen.

## Bestandsgrundlage und Zweitplan

- `uebergabe.md` und die beiden Machbarkeitsnotizen vom 2026-10-22 sind die
  Herkunft der Anforderungen; sie beschreiben noch keine Implementierung.
- `docs/TODO.md` ist das einzige offene Arbeitsbrett. Der Batchvorschlag führt
  27 Punkte; P8 ist ein Hinweis, P11 Stufen 2/3 und P12 sowie P25 sind ausgeschlossen.
- `docs/ARCHITEKTUR.md` und `docs/oberflaechen/INDEX.md` bestimmen die
  acht Vertragsoberflächen: CLI, Desktop, Android, Android-Brücke, Daemon-IPC,
  MCP, Netzprotokoll und Dokumentation.
- `native/Cargo.toml` und `release-native/update-feed/version.txt` nennen beide
  `0.5.184`. Dies belegt die Versionsdateien, nicht für sich eine vollständig
  geprüfte Veröffentlichung. Die Planung reserviert keine folgenden Versionsnummern.
- Der vorhandene Graph hat 55.776 Knoten. Die erste Abfrage mit `screen desktop
  mount vault oauth discovery permission relay widget icon launcher bitrate`
  liefert insbesondere `screen/core/wire_control.rs`, `pixel_convert.rs`,
  `negotiate.rs`, `share/core/discovery_signal_types.rs`, `transfer/core/job.rs`,
  `daemon/os/shared/backend_server.rs`. Der Graph ist Navigation; Laufzeitbehauptungen
  werden am aktuellen Quelltext geprüft. Das Werkzeug meldet Skill 0.9.11 bei
  Paket 0.9.17; die Abfrage wurde erfolgreich abgeschlossen, kein Graph neu aufgebaut.
- `native/src/share/core/session.rs:307`: `endpoint_addr` fügt IP- und
  Relay-Adressen zusammen; `transport_label` unterscheidet derzeit `direct`,
  `relay`, `unknown`. Eine eigene LAN-/WAN-/Fallback-Zulassung für Nutzdaten fehlt hier.
- `native/src/daemon/os/shared/rooted_backend.rs:66`: `Enforced` wird bei
  unbewiesener Wurzeleingrenzung abgewiesen. P21 darf diesen Schutz nicht abschalten.

Folgerung für den Zweitplan: Planungstiefe 4 wegen neuer Schichtgrenzen für
Transportwahl, Rechteoberflächen, privilegierte Bildschirmhelfer, TCP-Tunnel
und native Fernfenster. Bestehende Standortidentitäten, Rechte, Updateintegrität,
Abbruchverhalten und Recovery bleiben ausdrückliche Abnahmebedingungen.

## Externe Recherche, Durchgang 1

### R1: RDP und RemoteApp sind verschiedene Fähigkeiten

Quelle, abgerufen 2026-10-25: [Microsoft: unterstützte Konfigurationen](https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/remotepc/remote-desktop-supported-config).
Die Seite nennt Windows Pro/Enterprise für RDP-Verbindungen und Windows Server
für RemoteApp. Folgerung: P31 darf RemoteApp nicht aus der bloßen RDP-Fähigkeit
eines Windows-Pro-Hosts ableiten. E14 erhält getrennte Fähigkeiten und den eigenen
Fensterstrom als allgemeinen Weg. Nicht dokumentierte Registry-Freischaltungen
sind keine Implementierungsgrundlage.

### R2: Android-Projektion verlangt eine Sitzung mit Zustimmung

Quelle, abgerufen 2026-10-25: [Android MediaProjection](https://developer.android.com/media/grow/media-projection).
Ab Android 14 ist die Freigabe einer ausgewählten App vorgesehen. Das ist keine
Zusicherung beliebig vieler gleichzeitig fernbedienbarer App-Fenster. E3 muss
einen privilegierten Zusatzweg ausdrücklich von der normalen App-Fähigkeit trennen.

### R3: Platzangaben sind typisierte Providerdaten

Quellen, abgerufen 2026-10-25: [RFC 4331](https://www.rfc-editor.org/rfc/rfc4331.html),
[Drive v3 About](https://developers.google.com/workspace/drive/api/reference/rest/v3/about).
WebDAV bietet `quota-available-bytes` und `quota-used-bytes`; Drive bietet
`storageQuota` mit Limit und Nutzung. Folgerung für P23: Remote-Kapazität,
Kontingent, lokale Spool-Grenze und unbekannter Wert müssen getrennt bleiben.
Die lokale Festplattengröße darf nicht als bewiesene Remote-Gesamtgröße erscheinen.

### R4: Loginhelfer brauchen eine eigene Vertrauensgrenze

Quellen, abgerufen 2026-10-25: [DXGI DuplicateOutput](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput),
[SendSAS](https://learn.microsoft.com/en-us/windows/win32/api/sas/nf-sas-sendsas),
[SetThreadDesktop](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setthreaddesktop).
DXGI verlangt für den sicheren Desktop LocalSystem; die SAS-Erzeugung hängt
zusätzlich von der Systemrichtlinie ab. Ein Desktopwechsel eines Threads mit
Fenstern oder Hooks kann scheitern. Folgerung für E1/E5: kleiner lokaler Broker,
sitzungsgebundener Helfer und ausdrückliches Anmelderecht; Netzwerkparser und
Videodecoder gehören nicht in den privilegierten Prozess. Kein Abschalten von
UAC, kein automatisches Ändern einer globalen SAS-Richtlinie und keine Zusage,
dass gewöhnliche WGC-Erfassung den sicheren Desktop unterstützt.

### R5: Laufende MCP-Verbindungen und Dateien sind getrennte Updateverträge

Quellen, abgerufen 2026-10-25: [Claude Code MCP](https://code.claude.com/docs/en/mcp),
[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp),
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew),
[MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).
Claude Code beschreibt manuelles Neuverbinden interaktiver stdio-Server und
automatischen Neustart eines beendeten Servers beim nächsten Toolaufruf in
`claude -p`/Agent SDK. Das widerlegt die pauschale Annahme, jeder stdio-Abbruch
verlange immer einen neuen Clientprozess. Die Codex-Dokumentation belegt keinen
universellen Hotswap-Vertrag. `codex mcp --help` wurde lokal nur gelesen.

Windows-Dateifreigaben begrenzen Rename/Delete laufend geöffneter Dateien.
`updater/os/windows.rs::cli_self_replacement` weist den Tausch des laufenden
CLI-Abbilds gegenwärtig ab. `cli/mcp/update_stop.rs` beendet auf Updateanforderung
den MCP-Server. E9 wählt deshalb A+B: verwaltete Einbindungen atomar aktualisieren,
kompatible Sitzungen an ihre hashgeprüfte Version binden und notwendiges
Neuverbinden ausdrücklich melden. C, transparenter Protokoll-Hotswap, entfällt.
Die Historie eines Clients wird nicht durch `tools/list_changed` übertragen.

### R6: Android und Wayland benötigen Fähigkeits- und Sitzungsbezüge

Quellen, abgerufen 2026-10-25: [Android Activity-Starts](https://developer.android.com/guide/components/activities/secure-bal),
[RemoteDesktop-Portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html),
[ScreenCast-Portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).
Android erlaubt Hintergrundstarts nur unter dokumentierten Bedingungen;
eine Remoteanfrage allein ist keine Startberechtigung. Bei fehlender Fähigkeit
wird eine lokale Bestätigung benötigt. Die bestehende App zielt auf SDK 36.
MediaProjection kann bei Sperren, neuer Projektion oder Prozessende gestoppt
werden; die Sitzung muss dann aufgeräumt werden.

Bei gemeinsamem ScreenCast-/RemoteDesktop-Portal gehören `persist_mode` und
`restore_token` zu `SelectDevices`, nicht zusätzlich zu `SelectSources`.
Wiederherstellungstoken sind keine beliebig wiederverwendbaren Dauerrechte.
ScreenCast v6 bietet `pipewire-serial` gegen wiederverwendete Node-IDs;
ältere Portalversionen brauchen weiterhin einen sicheren unterstützten Weg.
`ConnectToEIS` und absolute Zeigerkoordinaten sind an die gewährte Sitzung
und Streamgeometrie gebunden. Folgerung: keine allgemeinen Wayland-Fensterlisten
oder frei steuerbaren Hintergrundfenster aus einem gewöhnlichen Portal ableiten.

### R7: Laufwerksereignisse und Bildmetadaten haben Plattformgrenzen

Quellen, abgerufen 2026-10-25: [Windows Medienwechsel](https://learn.microsoft.com/en-us/windows/win32/devio/detecting-media-insertion-or-removal),
[UDisks2 Filesystem](https://storaged.org/doc/udisks2-api/latest/gdbus-org.freedesktop.UDisks2.Filesystem.html),
[Desktop Duplication](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api),
[Android VideoCapabilities](https://developer.android.com/reference/android/media/MediaCodecInfo.VideoCapabilities).
Windows-Medienereignisse erreichen bei optischen Medien nicht jede RDP-Sitzung;
P10 braucht deshalb zusätzlich den erneuten Abgleich beim Fokuswechsel.
UDisks2 veröffentlicht tatsächliche MountPoints; ihre Anzeige ist kein Auftrag,
ein ungemountetes Gerät automatisch einzuhängen.

DXGI beschreibt Bewegungs- und Änderungsrechtecke. Bewegungen werden vor den
neuen Bildteilen angewandt. Diese Metadaten sind nicht automatisch bei WGC,
X11, Wayland oder Android vorhanden. P31 benötigt dort Bildvergleich und nach
Referenzverlust eine vollständige Wiederherstellung. Android-Codecfähigkeiten
müssen für die tatsächliche Größe und Rate abgefragt werden. Die derzeitigen
Grenzen in `screen/core/pixel_convert.rs` schützen auch Allokationen;
`codec_inventory.rs` führt konkrete OpenH264-Grenzen. P30 ersetzt diese nicht
durch ungeprüft unbegrenzte Maße.

## Zweiter Recherchegang: verbleibende Vertragslücken

### G1: Iroh 1.3.0 kann den Datenpfad wählen, aber `none()` sperrt nichts

Untersucht wurde die tatsächlich gebundene Version 1.3.0 aus `native/Cargo.lock`;
deren Cargo-Quellcache nennt Git-Commit
`0072d7d84b233f9e7185eb676f049beaf557ac03`.
Die versionierten Quellen sind [endpoint.rs](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint.rs),
[connection.rs](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint/connection.rs),
[biased_rtt_path_selector.rs](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/biased_rtt_path_selector.rs),
[remote_state.rs](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket/remote_map/remote_state.rs).
Die lokalen Kopien wurden gelesen, kein Build gestartet.

Der Standardselektor behandelt IP als Primary und Relay als Backup und nutzt
danach RTT, IPv6-Bias und Hysterese. Das beweist keine LAN-Präferenz innerhalb
der IP-Pfade. `Builder::path_selector` ist hinter
`unstable-custom-transports` vorhanden; die Schnittstelle besitzt keine stabile
Semver-Zusage. `PathSelector::select` steuert den Nutzdatenpfad.
`PathSelection::none()` behält jedoch die vorherige Auswahl; es blockiert keine
Nutzdaten. `paths_stream` beginnt mit dem aktuellen Snapshot und liefert
Pfad-/Auswahländerungen, ist aber selbst keine Durchsetzung.

P28 verwendet deshalb einen gekapselten, versionsgebundenen Selektor plus
eine ausdrückliche Nutzdatenzulassung. Signalisierung über Relay bleibt möglich,
während die Direktversuche laufen. Ein Labelwechsel ohne gemessenen tatsächlichen
Pfad erfüllt P28 nicht. Auswahl, Wiederwahl und Rückfall gelten für Datei,
Mount, Sync, Befehle, Terminal, Bildströme und spätere TCP-Tunnel gemeinsam.

Nachgeschlossene Lücke, 2026-10-25: Iroh `apply_selected_path` setzt nicht gewählte
Pfade nur auf `Backup`. Das gebundene noq-proto 1.3.0, Commit
`c1f411562e6078852749b8bcf1190523096a107f`, definiert in
[paths.rs](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/paths.rs)
Backup ausdrücklich als benutzbar, sobald kein Available-Pfad besteht.
[connection/mod.rs](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs)
berechnet `may_send_data` entsprechend. Beide lokalen Versionsquellen wurden
gelesen. Deshalb kann ein Gate nur vor neuen App-Schreibaufrufen bereits
gepufferte QUIC-Daten bei Pfadverlust nicht sicher zurückhalten.
Die Durchsetzung gehört vor die Ausgabe von Anwendungsdaten im QUIC-Scheduler,
mit getrennter Behandlung von Handshake, ACKs und Pfadprüfung. Falls die
öffentliche Schnittstelle diese Zulassung nicht ausdrücken kann, ist ein kleiner
gepinnt gepflegter Iroh-/noq-Patch Teil von P28. Ein Filter nach Verschlüsselung,
der Nutzdaten erraten müsste, ist kein gleichwertiger Weg. Die Wartung dieser
Abweichung und ihre Regressionen gehören ausdrücklich zum Paketumfang.

### G2: Unbekannte Kapazität darf Dokanys erfundenen Standard nicht auslösen

Bestand: Windows `mount/os/windows/callbacks_metadata.rs::get_disk_free_space`
und Linux `mount/os/linux_os/fuse_ops_meta.rs::statfs` fragen derzeit den lokalen
Cache ab. `native/dokany-private/recipe.json` pinnt Dokany auf
`f1d5de68ff459af94e309cfdd171e4b8ca2af4dd`.
[Dessen volume.c](https://github.com/dokan-dev/dokany/blob/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd/dokan/volume.c)
wurde am 2026-10-25 über den GitHub-Connector vollständig gelesen:
`STATUS_NOT_IMPLEMENTED` ersetzt Dokany durch 1 GiB Gesamt- und 512 MiB freien
Speicher. Dieser Status ist deshalb kein zulässiger Unknown-Fallback.

Die anfängliche Planwahl `STATUS_NOT_SUPPORTED`/`ENOSYS` ist nach Kritik B7
verworfen: Ein formal möglicher Fehlerstatus beweist keine Verträglichkeit mit
bestehenden Speicherprogrammen. Gewählt ist der kompatible Statistikweg mit
realem Cachebudget, eigener Herkunftskennung und sichtbarer Trennung von der
Remotequote nach [V4](vertraege.md#v4--kapazität-accountbindung-und-vaultöffnung).
Es entsteht keine erfundene Remotegröße und keine neue Fehlerklasse bei
unbekannter Quote. Echte Provider-/Schreibfehler bleiben erhalten. Android
kann unbekannte oder unbegrenzte Rootkapazität ausdrücklich als `null` liefern.

Drive `storageQuota.usage` umfasst Dienste über Drive hinaus; ein vorhandenes
Limit wird nicht gegen `usageInDrive` allein gerechnet. Fehlendes Limit bedeutet
unbegrenzt und ist kein numerischer Nullwert. Für SFTP ist die beworbene
OpenSSH-Erweiterung maßgeblich: [PROTOCOL](https://raw.githubusercontent.com/openssh/openssh-portable/master/PROTOCOL),
am 2026-10-25 gelesen, beschreibt `statvfs@openssh.com` Version 2 mit
`f_frsize`, `f_blocks`, `f_bavail` und Fehlerantwort. Eine allgemeine
SFTP-v6-Draftannahme wäre für den vorhandenen OpenSSH-Weg unzutreffend.
Der vorhandene `sftp/core/extensions.rs::target_error` nutzt bereits
`generation.sftp().fs_info(...)` für Nur-Lesen-/Speichervoll-Diagnosen;
`native/Cargo.lock` bindet russh-sftp 2.3.0. P23 kann diese bestehende
generationsgebundene Abfrage wiederverwenden.

### G3: Vault-Vertrauen und Recovery brauchen die tatsächliche Identität

`gdrive/core/backend.rs::root_confinement` liefert bereits `Enforced`.
Eine pauschale Diagnose „Google Drive begrenzt die Wurzel nicht“ ist deshalb
widerlegt. `vault/os/shared/backend.rs` delegiert die Eigenschaft; untersucht
werden muss die tatsächlich gewählte Auflösungskette. P21 behält den sicheren
Standard und bindet explizites Vertrauen an Vault, Backend, Konto und Root.

`mount/core/journal_record.rs` enthält Dateiänderungen, ausstehende Löschungen,
Quarantänebezug und Namespacekonflikte. Die Löschphasen `LocalOnly`, `Unresolved`
und `Moved` bezeichnen unterschiedliche Wiederaufnahmezustände. P22 exportiert
deshalb ein vollständiges, persistiertes Recovery-Paket vor dem Austragen.
Ein Verzeichnis mit sichtbaren bearbeiteten Dateien wäre kein vollständiges Backup.
Die Anforderung P22 betrifft auch eine dauerhaft fehlende Gegenstelle. Deshalb
trennt der Plan das Austragen aus aktiver Mount-/Autoremountregistrierung von
der Aufbewahrung inaktiver Recovery und vom Löschen lokaler Originaldaten.
Der Eigentumsübergang muss dauerhaft sein; fehlende Remoteinhalte bleiben
ausdrücklich ungesichert. Unerreichbarkeit allein erzwingt keine aktive
Wiedereinhängeschleife und keine Löschung des vorhandenen Journals.

### G4: Die wiederholte Drive-Abmeldung ist noch keine bekannte Ursache

`cloud/core/token_persistence.rs` bewahrt bei einem erfolgreichen Refresh ohne
neuen Refresh-Token den alten Token und persistiert einen rotierten Token vor
dem Erfolg. `cloud/core/cloud.rs` formatiert OAuth-Fehler bisher als Text.
P26 ergänzt strukturierte Fehler, koordinierte Aktualisierung und atomare
Persistenz; es entfernt keine Tokens nach bloß transienten Fehlern.

[Googles OAuth-Dokumentation](https://developers.google.cn/identity/protocols/oauth2?hl=en),
am 2026-10-25 über den offiziellen Spiegel gelesen, nennt sieben Tage für
externe Apps im Testing-Status mit relevanten Scopes sowie weitere Ursachen
für ungültige Refresh-Tokens. Der konkrete Projektstatus ist nicht belegt.
Die Regel wird als Diagnosemöglichkeit behandelt, nicht als erwiesene Ursache.
Live-Zugangsdaten sind weder Planvoraussetzung noch spätere Releasevoraussetzung.

### G5: Relayspeicher ist von Encoderbitrate und Empfangslimit getrennt

`share-server/Cargo.toml` verwendet den gepatchten
`vendor/iroh-relay-1.0.0`, nicht die Clientbibliothek 1.3.0.
`vendor/iroh-relay-1.0.0/src/server/queue_budget.rs` begrenzt ausgehende
Paketdaten pro Empfänger auf 1 MiB und insgesamt auf 64 MiB.
`client.rs::send_packet` verwirft bei fehlender Reservierung; die Reservierung
lebt bis zum Schreibversuch. `share-server/src/relay.rs` begrenzt separat
eingehenden Verkehr auf 64 MiB/s mit 8 MiB Burst.
P27 untersucht Encoder-Pacing, QUIC, Paketverluste und diese konkreten Queues
zusammen. Ein größeres Queuebudget allein beweist keinen Durchsatzgewinn;
Speicherschutz wird durch begründete Budgets ersetzt, nicht entfernt.

### G6: Die vorhandene Remote-Veröffentlichung bleibt der einzige Abschlussweg

Gelesen wurden `.github/workflows/build.yml`,
`.github/workflows/rechte-desktop-tresore-task.yml`,
`native/publish-release-local.ps1` und `docs/RELEASING.md`.
Der spätere komplette Release startet über `build.yml` auf `main` mit
`complete_release_source_sha` gleich dem vollständigen Kandidaten-Commit;
`verify_release_candidate` und `publish_release` bleiben beim Wrapperstart aus.
Der Wrapper übernimmt Build, Version, Artefakte, Commit, Tag und seine interne
Publikation. Es wird kein konkurrierender Tag-/Dispatchweg ergänzt.
Der konfigurierte Releasejob besitzt 360 Minuten und der Wrapper 330 Minuten.
Die Task-Suite erhält genau einen Einstieg pro gemeinsam freigegebenem
Releaseumfang, ein exaktes Kandidatensignal und mindestens 30 Minuten.
Planungspakete sind keine automatische Aufteilung in mehrere Veröffentlichungen.
Nicht ausführbare
Realgerätfälle werden `not_run`, nie `passed`; sie halten die Veröffentlichung
nicht zurück. Jetzt wird ausschließlich der Plan bearbeitet.

### G7: Ergänzender Abgleich während der unabhängigen Kritik: Dienstsignatur

Der erneute Abgleich von [SendSAS](https://learn.microsoft.com/en-us/windows/win32/api/sas/nf-sas-sendsas)
am 2026-10-25 trennt zwei Wege: Dienst oder Anwendung mit `uiAccess`.
Die dokumentierte Authenticode-Pflicht bezieht sich auf `uiAccess`.
Der Plan wählt den Dienstweg und darf daraus keine neue zwingende
Authenticode-Beschaffung ableiten. In den gelesenen Release-/Publishskripten
und `build.yml` ist kein `signtool`-/`Set-AuthenticodeSignature`-Schritt für
eigene Windows-EXE belegt. Die Signaturprüfung in `docs/RELEASING.md:89`
betrifft das fremde Dokany-MSI; die Android-Signatur besitzt einen eigenen Vertrag.
E1 benötigt deshalb einen releasegebundenen, unmittelbar vor Start erneut
hashgeprüften Helfer in einem administrativ geschützten Pfad samt geschütztem
Erwartungshash. Ein konkreter `uiAccess`-Zusatzweg wäre ein neuer ausdrücklich
zu belegender Signaturvertrag und gehört nicht still in diesen Plan.

### G8: Nachrecherche zur unabhängigen Kritik

Alle folgenden Quellen wurden am 2026-10-25 gelesen. Die Entscheidungen stehen
in [V1–V9](vertraege.md); MCP-Laufzeit und PIN-Eingabe samt eigener Quellenliste
in [agentenvertrag.md](agentenvertrag.md). Das sind Ergänzungen zum Plan,
keine Ausführung der beschriebenen Plattformfunktionen.

| Frage | Tatsächlicher Befund | Konsequenz |
|---|---|---|
| Vor-Login-Identität und lokaler Angreifer | `native/installer.nsi` verwendet per-user Installation; [SERVICE_SID_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-service_sid_info) beschreibt Dienst-SIDs und eingeschränkte Tokens, [Dienstkonten](https://learn.microsoft.com/en-us/windows/win32/services/service-user-accounts) den getrennten Sicherheitskontext. `share/core/crypto.rs` besitzt bereits Iroh-Signaturprüfung. | V1 ergänzt Boot-Netzdienst, Maschinenidentität, administrativen Grantbesitz, unabhängige Brokersignaturen und geschützte Installation. Same-user-ACL allein genügt nicht. |
| Exakter QUIC-Inhalt und NAT-Ende | Noq 1.3.0 `src/frame.rs::Frame` enthält die in V3 vollständig aufgezählten Varianten. `src/n0_nat_traversal.rs` nennt maximal neun Probes, `retry_delay(initial_rtt)` und ausdrücklich das noch fehlende Abschlussereignis. Standard `initial_rtt` ist 333 ms; Retryintervalle sind auf 2 s begrenzt. | Neue beobachtbare Probe-/Antwort-/Validierungsfakten im gepinnten Adapter; keine erfundene bereits vorhandene Finished-Nachricht. Anfangs gesperrte STREAM/DATAGRAM auch in Handshake-/0-RTT-Pfaden. |
| Fristmaßstab | [RFC 9000 §8.2.4](https://www.rfc-editor.org/rfc/rfc9000.html#section-8.2.4) empfiehlt zur Pfadvalidierung drei größere PTO-Intervalle aus bestehendem und neuem Pfad. | V3 verwendet diesen Maßstab für endliche initiale Kandidatensammlung und Validierung. Die Anwendung auf das Sammelfenster ist ausdrücklich eine eigene Planentscheidung. |
| Gemeinsamer Endpoint versus Verbindungen | `share/core/node.rs` registriert die ALPNs; `node.rs:379` verbindet Exec, `node_sessions.rs:371` Datei-/Sharesitzungen, `node_screen_link.rs:82` Screen jeweils mit dem passenden ALPN. | Die Routepolitik gilt für jede dieser QUIC-Verbindungen; eine Freigabe eines ersten Screen- oder Dateikanals deckt die übrigen nicht still ab. |
| Neue Rechte beim Downgrade | `access_grant_wire.rs` akzeptiert Version 1, bewahrt unbekannte Schlüssel/Felder und behandelt unbekannte Versionen als ausgeschaltet. `exec_policy_wire.rs` schreibt bei Accessdatensatz `enabled=false`. | V2 plant neue eingeschränkte Grants als Version 2, unveränderte Altgrants weiterhin als Version 1 und dokumentiert die sichere vollständige Abschaltung eines V2-Grants im Altleser. |
| Accountgranularität | `cloud/os/shared.rs::keyring_account` verwendet `cloud:<provider>`; Clientkonfiguration und CLI sind ebenfalls providerbezogen. | V4 macht Authidentität, Legacyzuordnung, Kontowechselkonflikt und Zwei-Account-/Clientfixtures zur konkreten P26-Arbeit; kein ungefragtes Mehrkonten-UI. |
| Laufwerksverbraucher | `landing.rs`, `omni_accel.rs`, `share_exports_ui.rs`, `scanning.rs` und `app/os/windows/watchers.rs::start_watcher` konsumieren den Laufwerksbestand zusätzlich zu Sidebar, Picker, Analyse, Aufräumen und Gerätebaum. | P10 umfasst die zehn in V5 vollständig benannten Verbraucher, einschließlich Watcher-Abmeldung und Indexgenerationen. |
| Windows-Ereigniszustellung | [WM_DEVICECHANGE](https://learn.microsoft.com/en-us/windows/win32/devio/wm-devicechange) wird über WindowProc behandelt; [GetMessage](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmessage) dispatcht auch gesendete Nachrichten und kann -1 liefern. | Eigene versteckte Top-Level-Nachrichtenschleife plus Fokusabgleich; CM_Register_Notification ist keine zusätzliche Voraussetzung des gewählten Wegs. |
| OpenLocation-Vertrag | `prefs_tabs.rs::navigate_to_location` behandelt Share, Vault, gespeicherte Remoteverbindungen und lokale Pfade. Sein GDrive-Fallback öffnet bisher nur den Root. | P15 kann den vorhandenen Einstieg benutzen; konkrete Drive-Unterorte dürfen dabei nicht still ihren Pfad verlieren. |
| Android-Vaultöffnung und Quote | `docs_vault.rs::source` bindet entsperrten Vault und versiegelten Spool; `withdraw` entzieht Deskriptoren. `DocumentsCursors.kt` bietet bisher keine Kapazitätsspalten. [DocumentsContract.Root](https://developer.android.com/reference/android/provider/DocumentsContract.Root#COLUMN_AVAILABLE_BYTES) erlaubt optionales `null` für unbekannt/unbegrenzt; Gesamtbytes ab API 24, verfügbare Bytes ab API 19. | P21 erhält diese reale Androidkette; P23 ergänzt DocsRoot/Bridge/Cursor statt einer nur hypothetischen Anbieteroberfläche. |
| WGC und Fensterfamilien | [IncludeSecondaryWindows](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.includesecondarywindows?view=winrt-26100) ist ab Windows 11 24H2 dokumentiert, standardmäßig aus und beschneidet sekundäre Fenster an der Hauptfenstergrenze. [IsBorderRequired](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired?view=winrt-26100) bindet Borderless-Capture an Zustimmung und Paketfähigkeit. | V6 erfasst eigenständige Dialogflächen und behält den Host-Captureindikator. Eine randlose lokale Darstellung behauptet keinen randlosen Hostcapture. |
| SVG-Icons | `native/Cargo.toml` enthält Rasterdecoder, keinen SVG-Renderer. [resvg 0.49.0](https://docs.rs/resvg/0.49.0/resvg/) rendert einen usvg-Baum in eine Pixmap; [usvg Options](https://docs.rs/usvg/0.49.0/usvg/struct.Options.html) bietet einen eigenen Image-Href-Resolver. | Hostseitiger begrenzter Rasteradapter nach V6; keine externen Datei-/Netzreferenzen. MSRV-/Zielprüfung gehört vor die spätere Dependencyaufnahme. |
| Updater-Prozessgrenze | `bin/smart_explorer_updater.rs::apply_update` prüft Hashes, wartet auf App/CLI, ersetzt eine Transaktion und bestätigt den neuen Appstart. `process.rs` wartet 30 s ohne Kill; unter Windows wird der normalisierte volle Imagepfad verglichen. | Der P13-Vertrag muss dort seine Generationen-/Operationssperre integrieren; eine private Runtime darf nicht ihren eigenen unveränderlichen Bestand per `se update` ersetzen. |
| Gehostete Publikation | `publish-release-local.ps1:747–821` erkennt Actions, findet einen bestehenden exakten Publikationsrun oder dispatcht mit `publish_release=true` und zurückgegebener Run-ID. `docs/RELEASING.md` erläutert den GITHUB_TOKEN-Kontext. | Der Agent startet weiterhin nur den bestehenden kompletten Wrapperweg. Der Wrapper koordiniert seinen einen Publikationsconsumer; kein zweiter manueller Trigger. |

Die bereits geprüften Portalverträge R3/G6 erlauben logische streambezogene
Eingabekoordinaten. Private Mutter-RecordWindow-Aufrufe sind nicht Teil des
gewählten Ansatzes. TermService wird nicht neu an Loopback gebunden: der
Tunnel benutzt einen tatsächlich erreichbaren lokalen Listener; fehlt er,
meldet die App den Einrichtungszustand. Diese Entscheidungen beseitigen
unnötige Plattformvoraussetzungen, ohne sie als getestete Fähigkeiten auszugeben.

Die Remote-Runnerkonfiguration ist in V9 inventarisiert. Dies ist kein
Nachweis erfolgreicher neuer Tests. Isolierte Sicherheits-/Vertragsfälle sind
von optionalen physischen Geräten und Liveproviderfällen getrennt.

## Entscheidungsschleife und Abschlusskriterien

Ziel der Schleife ist der ausführbare Plan aus dem Auftrag vom 2026-10-25.
Kriterien sind vollständige P-Zuordnung, belegte API-Grenzen, erhaltene
Kompatibilität, konkrete Ergebnismerkmale, genau ein späterer Suite-/Releaseweg
je gemeinsam beauftragtem Releaseumfang und die angeforderte unabhängige Kritik.
Verglichen werden der ursprüngliche Fünferzuschnitt und eine Trennung an den
eigenständigen Update-/Transport-/Dateisicherheitsverträgen. Die zusätzliche
Releasearbeit ist ein ausdrücklicher Nachteil kleinerer Batches.

Aktiver Stand: Erst- und Zweitplan, beide Recherchegänge und die Nachrecherche
zu den 28 Kritikbefunden sind dokumentiert. Die ergänzten Verträge sind mit
dem Hauptplan zusammengeführt; die Abschlussbewertung steht in kritik.md.
Schleifenstatus: erreicht für das Planungsergebnis. Die statische Kontrolle
von Dokumentverweisen, Codeblöcken, vollständiger P-Zuordnung und sämtlichen
Kritikkennungen ergab keinen Befund; git diff --check ebenfalls nicht.
Es werden dafür keine Produktfunktionen, Tests oder Veröffentlichungen gestartet.
