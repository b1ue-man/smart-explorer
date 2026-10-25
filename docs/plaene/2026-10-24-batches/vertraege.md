# Ergänzende Implementierungsverträge nach der Plankritik

Stand: 2026-10-25, Quellbasis `aec62a40`. Dieses Dokument konkretisiert den
[Detailplan](batches.md). Es beschreibt geplante Änderungen, keine vorhandenen
Fähigkeiten. Der separate [Agentenvertrag](agentenvertrag.md) behandelt P13 und
die PIN-Eingabe von P29. Die vollständige Befundzuordnung steht in
[kritik.md](kritik.md).

## V1 — Anmeldung, Dienstidentität und privilegierte Eingaben

**Entscheidung für P1/E1/E5:** Die bestehende Benutzerinstallation ist kein
Vertrauensanker für SYSTEM/root. `native/installer.nsi` installiert bisher mit
`RequestExecutionLevel user` unter dem Benutzerprofil. Die neue optionale
Anmeldefähigkeit erhält deshalb eine gesondert administrativ eingerichtete
Dienstkomponente. Ein gewöhnliches Appupdate aktiviert sie nicht.

| Prozess / Zustand | Identität, Besitz und zulässige Aufgabe |
|---|---|
| Normale App und Benutzerdaemon | Bestehende Benutzeridentität und bestehende Gerätebeziehungen; kein direkter Aufruf privilegierter Capture-/Inputoperationen. |
| Netzwerkdienst vor der Anmeldung | Eigene, nicht administrative Dienstidentität: unter Windows ein eigener LocalService-Prozess mit eingeschränkter Dienst-SID; unter Linux ein eigener Systembenutzer. Start beim Boot, eigener Iroh-Schlüssel und administrativ geschützte Zuordnung zum bekannten Gerät. Netzwerk, Protokollparser und Codecs laufen hier. Der Benutzerprofilschlüssel wird nicht kopiert. |
| Lokaler Broker | SYSTEM beziehungsweise root, kein Netzwerklistener. IPC nur für die konkrete Dienst-SID/UID, keine Freigabe an alle Prozesse des angemeldeten Benutzers. Prüft zusätzlich die Gegenstellenautorisierung unabhängig vom Netzwerkdienst. |
| Capture-/Inputhelfer | Nur vom Broker aus geschütztem Installationsbestand gestartet; pro aktivem Sitzungs-/Desktopzustand. Kein beliebiges Kommando, Pfadargument oder frei gewählter Benutzerkontext. |
| Geschützte Installation | Windows: administrativ geschützter Program-Files-Unterordner und Policy unter ProgramData; Linux: root-eigener libexec-Bestand und Policy unter `/var/lib`. Konkrete Installationspfade werden aus dem Installervertrag abgeleitet, nicht aus privaten Rechnerpfaden. |

Die erstmalige Einrichtung bestätigt mit administrativer Autorität genau den
fernen Geräteschlüssel, die neue Maschinenidentität, erlaubte Zielbenutzer und
Anmelde-/Sperr-/UAC-Rechte. Vorhandenes `screen_control` und Raumrechte erzeugen
diese Freigabe nicht. Die Geräteoberfläche kann Benutzer- und Maschinenendpunkt
gemeinsam darstellen, muss ihren unterschiedlichen Rechtebesitz aber zeigen.
Der maschinenweite Grant ist für gewöhnliche Benutzerprozesse nicht schreibbar.

Der Broker erzeugt einen frischen Challenge und prüft signierte,
domänenseparierte Anforderungen mit Maschinen-/Peer-ID, Bootgeneration,
Sitzungs-/Desktopgeneration, Policyrevision, fortlaufender Sequenz und exakter
Operation samt Nutzdaten. Die vorhandenen Iroh-Signaturfunktionen in
`share/core/crypto.rs` werden wiederverwendet. Auch Eingabebündel sind signiert;
ein Netzwerkdienst kann nicht nach einer einmaligen Anmeldung beliebige
unsignierte Eingaben einschieben. Wiederholte Sequenzen, alte Challenges und
Signaturen für eine andere Operation werden abgewiesen. Es entsteht kein neuer
Kryptoalgorithmus. Ein kompromittierter Codecprozess besitzt keine Broker-
oder Policydateien. Administrator-/Rootkompromittierung liegt außerhalb dieses
Schutzversprechens; zugelassene Bildschirmdaten sind im Netzwerkdienst sichtbar.

Der erwartete SHA-256 stammt aus dem administrativ übernommenen Releasebestand
und liegt im geschützten Manifest, nicht neben einer benutzerschreibbaren EXE.
Vor Start und Ersatz werden Datei, Parentpfad, Besitzer/ACL und Hash erneut
geprüft; gehaltene Handles verhindern Austausch zwischen Prüfung und Start.
Eine Aktualisierung dieser Komponenten benötigt denselben administrativen
Installationsweg und eine atomare Manifest-/Dateigeneration mit Rollback.
Der bisherige per-user Updater darf dort keine ungeprüften Dateien einschleusen.
Es wird kein `uiAccess` vorausgesetzt und keine fehlende Authenticode-Signierung
behauptet. Dienst-/Installer-/Updateränderungen gehören zu P1b/P1c und werden
mit README und RELEASING dokumentiert.

Vier Zustände werden ausdrücklich umgesetzt: Sperre des berechtigten Benutzers,
sein UAC-Desktop, Boot/Logout vor einer Benutzeranmeldung und Sitzung eines
abweichenden Benutzers. Vor der Anmeldung funktioniert nur der eingerichtete
Maschinenendpunkt. Nach Anmeldung wird die tatsächliche SID/UID erneut geprüft;
ein nicht freigegebener Benutzer beendet Capture und Eingabe. Zugriff auf mehrere
Benutzer verlangt eine ausdrücklich administrativ gespeicherte Liste. Bei
Sitzungswechsel werden gehaltene Tasten gelöst und die alte Generation beendet.
Linux übernimmt dieselbe Identitätsgrenze für erkannte X11-Greeter; für
Waylandgreeter wird kein nicht belegter Capture-/Inputweg erfunden.

**Verbindliche spätere Abnahme:** Neben den vier Zuständen laufen negative
IPC-/Policyfälle mit gewöhnlichem Prozess derselben Benutzer-SID, fremder SID,
gefälschtem Client, manipuliertem Manifest, vertauschtem Helfer, alter Signatur,
falscher Sitzung und Entzug. Diese isoliert prüfbaren Sicherheitsverträge dürfen
nicht als bloß fehlender Hardwaretest übersprungen werden. Reales Windows Hello
und konkrete Displaymanager sind zusätzliche Laufzeitbelege nach V9.

Quellen: [Dienst-SID und eingeschränktes Token](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-service_sid_info),
[Dienstkonten](https://learn.microsoft.com/en-us/windows/win32/services/service-user-accounts)
und Recherche G7; geprüft am 2026-10-25. Die Dienstarchitektur ist eine daraus
abgeleitete Planentscheidung, keine Microsoft-Zusage für Smart Explorer.

## V2 — TCP-Tunnel, lokale Portgrenze und neue Rechte

P1e gilt für Desktop, Android, CLI Text/JSON, MCP, Daemon-IPC und Netzvertrag.
MCP benutzt denselben authentifizierten CLI-/Daemonweg wie die Geräteaktion;
ein Toolaufruf ersetzt keinen Grant. Die Regel enthält Initiator, Zielgerät,
Richtung, Zielhost/IP, Portmenge, Ablauf und Rückfrage. DNS-Ergebnisse werden vor
jedem neuen Socket gegen die gespeicherte Zielregel geprüft und für diesen
Socket gebunden. Loopback des Zielhosts ist der Standard, ein LANziel wird
ausdrücklich separat erlaubt. TermService muss nicht auf Loopback umkonfiguriert
werden: verbunden wird der tatsächlich erreichbare lokale Listener; deaktivierter
Dienst, fehlende Edition oder nicht erreichbarer Port erzeugen einen konkreten
Einrichtungszustand. Keine automatische RDP-/Firewall-/NLA-Umstellung.

**Lokale Grenze:** Ein TCP-Loopbackport ist keine Benutzerautorisierung. Ein
gleichzeitig lokal laufender Prozess kann ihn grundsätzlich erreichen. Das
wird beim allgemeinen Tunnel als Wirkung der Freigabe benannt; die Schaltfläche
bestätigt die lokale Bereitstellung des Dienstes. Der RDP-Weg bleibt zusätzlich
durch RDP-TLS/NLA und sichere Zertifikatsprüfung geschützt. Eine OS-Portreservierung
oder ein kurzlebiger Port wird nicht als Geheimnis behandelt. Dienste ohne eigene
Authentifizierung verlangen eine ausdrückliche Bestätigung dieses lokalen
Zugangsrisikos; sie werden nicht unbemerkt über den RDP-Standard freigegeben.
Abnahme enthält einen zweiten lokalen Prozess sowie unzulässige Ziele,
DNSwechsel, Half-close, EOF und Entzug während laufender Datenübertragung.

**P18d als zusätzliche große Integrationsarbeit:** `app_launch`, `login_access`
und `tcp_forward` erhalten typisierte Einschränkungen, hostseitige Auflösung,
versionierte Persistenz und Fähigkeiten-Aushandlung. Unveränderte Altgrants
bleiben im bisherigen Accessformat Version 1. Ein Grant mit neuen eingeschränkten
Fähigkeiten wird als Accessversion 2 geschrieben, mit `enabled=false` im
Legacyfeld. Der vorhandene Altleser bewahrt unbekannte Versionen und behandelt
sie als ausgeschaltet (`access_grant_wire.rs`, `exec_policy_wire.rs`). Ein
Downgrade darf deshalb diesen Grant vollständig deaktivieren; UI und Migration
benennen diesen Kompatibilitätsverlust. Er darf niemals die neuen Einschränkungen
ignorieren oder eine vor dem Entzug gesicherte alte Policy automatisch aktivieren.
Neue Leser verstehen beide Versionen; unbekannte Felder bleiben erhalten.
Alte Peers bekommen keine neuen Fähigkeiten zugesagt.

Jede Verschärfung wird wie Entzug ausgewertet: Allow→Ask/Off, verkleinerte
App-/Zielhost-/Port-/Benutzerliste, verkürzte Gültigkeit und Kontowechsel. Nicht
mehr gedeckte laufende Kanäle schließen; Eingaben werden gelöst, mutierende
Jobs abbrechbar und wiederaufnehmbar beendet. Baselines verzeichnen keine
fehlgeschlagenen Schritte als Erfolg. Die Abnahme enthält Migration,
Altclient/Neuhost, Neuclient/Althost, Persistenzfehler und Downgrade.

Die Geräteseite zeigt Rechte **je Beziehung** und zusätzlich den **erreichbaren
Gesamtzugang über alle Beziehungen**. Ein Direktkontakt kann etwa Off sein,
während ein Raum noch Allow gewährt. Das ist sichtbar und wird beim Entzug
genannt. Die Zusammenfassung vereinigt keine Grants für eine Sitzung; der Host
prüft weiterhin die konkret verwendete Beziehung und deren Einschränkungen.
Bei bedingten Rechten werden die erlaubten App-/Zielbereiche angezeigt, nicht
ein irreführendes einziges Maximum ohne Bedingungen.

## V3 — Nutzdatenzulassung und begrenzte Direktphase

Die folgenden Verträge ergänzen P28a–d. Grundlage sind Iroh 1.3.0 und Noq 1.3.0
mit den in Recherche G1 angegebenen Commits. Die Noq-Quellen besitzen noch kein
verlässliches Ereignis „NAT-Runde vollständig beendet“; `n0_nat_traversal.rs`
benennt diese Lücke selbst. Der Vendoradapter muss daher Probeerschöpfung,
letzte Antwortfrist und laufende Pfadvalidierungen als neue beobachtbare Fakten
ausgeben. Warten auf ein erfundenes vorhandenes Ereignis ist ausgeschlossen.

**Vollständige Frameklassifikation für diesen Pin:** Nutzdaten sind `Stream`
und `Datagram`, auch während 0-RTT, serverseitiger früher Daten, Wiederholung und
neuer Pfadanlage. Beide sind standardmäßig gesperrt, bis die aktuelle
Pfad-/Policygeneration zugelassen ist. Zulässige Transportsteuerung umfasst
`Padding`, `Ping`, `Ack`, `PathAck`, `ResetStream`, `StopSending`, `Crypto`,
`NewToken`, `MaxData`, `MaxStreamData`, `MaxStreams`, `DataBlocked`,
`StreamDataBlocked`, `StreamsBlocked`, `NewConnectionId`, `RetireConnectionId`,
`PathChallenge`, `PathResponse`, `Close`, `AckFrequency`, `ImmediateAck`,
`HandshakeDone`, `ObservedAddr`, `PathAbandon`, `PathStatusAvailable`,
`PathStatusBackup`, `MaxPathId`, `PathsBlocked`, `PathCidsBlocked`, `AddAddress`,
`ReachOut` und `RemoveAddress`. Vor Zulassung enthalten Fehler-/Close-Texte keine
Anwendungsdaten. Ein neuer Framevariant verlangt eine ausdrückliche Zuordnung;
kein allgemeiner Durchlass für unbekannte Varianten.

Geprüft wird bei der Frameserialisierung vor Verschlüsselung auf jedem
Sendepfad, einschließlich Handshake und Retransmits. Weder die Wahl eines
`SpaceKind` noch der bestehende Handshakezweig darf die Prüfung umgehen.
Pfadanlage beginnt mit `data_allowed=false`; eine alte Available-Einstellung
ist kein Grant. Eine Änderung der Policygeneration invalidiert gepufferte
Paketpläne. Appwrite-Gate und Backpressure verhindern zusätzlich unbegrenztes
Ansammeln gesperrter Nutzdaten.

`SessionHello`, HMAC-Anmeldung, Datei-/Exec-/Terminal-/Screen-/Tunnelprotokoll
und darin geführte Statusabfragen sind Anwendungsdaten; sie warten auf dieselbe
Zulassung. Für den Transport selbst nötiges Relay-Signaling, Presence und mDNS
dürfen Routing-/Identitätsmetadaten austauschen. Sie tragen keine Datei-,
Kommando-, Bild- oder Tastatureingaben. Die Pfadpolitik ist lokal schon vor
einer Anwendungs-Aushandlung aktiv; dadurch entsteht kein Deadlock zwischen
SessionHello und erstmaliger Routezulassung.

**Endliche Entscheidungsregel:** Jeder Verbindungsversuch besitzt eine monotone
Netzgeneration und einen Kandidatensnapshot. Nach dem Transporthandshake wird
ein begrenztes Sammelfenster von `3 × max(PTO_bestand, PTO_initial)` geschlossen.
Dies ist eine eigene Planentscheidung aus dem QUIC-Validierungsmaßstab, keine
Iroh-Discoverygarantie. Es folgt genau eine initiale Proberunde für die bis dahin
bekannten LAN- und Direktkandidaten. Steuerprobes dürfen parallel laufen;
Anwendungsdaten bevorzugen den validierten LANpfad. Ein validierter weiterer
Direktpfad wird erst nach Abschluss der bekannten LANversuche freigegeben.

Das Rundenbudget berechnet der Adapter aus den tatsächlichen Noq-Retryintervallen
für höchstens `MAX_NAT_PROBE_ATTEMPTS=9`, anschließend der letzten Antwortfrist
und der Pathvalidierung von `3 × max(PTO_bestand, PTO_neu)`. Grundlage ist die
implementierte `retry_delay(initial_rtt)` mit Standard-RTT 333 ms und maximal
2 s je Intervall; die verkürzte Quelltextbeschreibung „up to 4s“ wird nicht als
vollständiger Fristbeweis benutzt. Alle Werte und die errechnete Deadline sind
im Diagnoseergebnis sichtbar. Ein endgültiger Netzfehler kann einen einzelnen
Kandidaten früher beenden. Fristablauf bedeutet „im aktuellen Versuch nicht
erreicht“, niemals weltweit oder dauerhaft unerreichbar.

Später eintreffende Adressen und Irohs weitere Upgraderunden verlängern diese
Deadline nicht. Sie werden nach Freigabe des begründeten Relayrückfalls weiter
geprüft und ermöglichen die Rückkehr zu Direkt/LAN. Bei einem Netzwechsel
beginnt eine neue endliche Generation. Die UI zeigt bis dahin „Verbindung wird
ermittelt“, „LAN wird geprüft“, „Direktverbindung wird geprüft“, „pausiert wegen
Pfadwechsel“ oder „offline“; ein noch unentschiedener Pfad heißt nicht Relay.

App-Handshaketimeouts beginnen erst nach Routezulassung; ein Benutzerabbruch
wirkt trotzdem sofort. Laufende Mutationen werden bei Pfadverlust angehalten
und nicht erneut ausgeführt. Bildschirmframes aus der Unterbrechung werden
verworfen; nach Wiederkehr entsteht eine neue vollständige Referenz. Aktive
Frame-/GOP-Deadlines dürfen die Direktphase nicht als Codecfehler behandeln.
Explizite vom Aufrufer gesetzte Gesamtfristen werden nicht still verlängert:
bei Ablauf lautet der Fehler Verbindungsaufbau/Pfadwechsel.

LANklassifikation verlangt eine validierte Route über ein erkanntes physisches
lokales Interface und einen tatsächlich on-link liegenden Kandidaten.
RFC1918, IPv6-ULA und CGNAT allein sind kein LANbeweis; IPv6-Linklocal benötigt
die richtige Scope-ID. VPN-/Overlayinterfaces zählen zur zweiten Direktklasse;
unbekannte Interfacetypen ebenfalls. Messwerte werden privat pro gepinntem
Gerätepaar, Netzgeneration und Pfadklasse gespeichert, mit Zeitstempel und
begrenzter Historie; sie verleihen keine Rechte und autorisieren keinen noch
nicht erneut validierten Pfad.

**Abnahme:** Fake-clock-/Framefälle decken sämtliche Varianten, neue Pfade,
0-RTT, frühen Serverversand, NAT-Steuerung und Retransmits ab. Isolierte
Netzfälle belegen tatsächliche Nutzbytes je Richtung, endlichen Fallback,
keine Deadlineverlängerung durch ständig neue Adressen und Rückkehr zu LAN.
Der RFC-Maßstab stammt aus [RFC 9000 §8.2.4](https://www.rfc-editor.org/rfc/rfc9000.html#section-8.2.4),
geprüft am 2026-10-25. Er beweist keine universelle Internet-Erreichbarkeit.

## V4 — Kapazität, Accountbindung und Vaultöffnung

**P23:** Bei bekannter Providerkapazität bleibt die Remotequelle maßgeblich,
zusätzlich begrenzt durch das konkrete lokale Schreib-/Spoolbudget. Bei
unbekannter/unbegrenzter oder momentan nicht abrufbarer Remotequote erhalten
Dokany/FUSE eine erfolgreiche, konsistente Statistik über das reale
**Cachebudget** mit eigenem `CapacitySource::CacheCompatibility`. Die App zeigt
„Remote unbekannt“ beziehungsweise „Remote unbegrenzt“ getrennt von „OS-Anzeige:
Cachebudget“; auch die Volumebezeichnung kennzeichnet den Cachebezug innerhalb
der Plattformlänge. Weder die ganze lokale Platte noch ein erfundener 1-GiB-Wert
wird als Remotevolumen ausgegeben. Providerfehler und Erhebungszeit bleiben
sichtbar. Ein fehlender Quotawert darf keinen bestehenden Schreibweg sperren.

Dies ersetzt die frühere Planwahl `STATUS_NOT_SUPPORTED`/`ENOSYS`: Die
Verträglichkeit solcher Fehler mit sämtlichen Speicherprogrammen ist nicht
belegt. Der konservative Zahlenweg vermeidet diese neue Fehlerklasse. Echte
Schreibfehler und `ENOSPC` bleiben unverändert. Cachebudget bedeutet hier den
tatsächlich konfigurierten und aktuell verfügbaren Stagingraum, keine
zusätzliche willkürliche Produktobergrenze. Bei nicht ermittelbarem lokalen
Speicherzustand bleibt ein echter lokaler I/O-Fehler erkennbar.

Die vollständige Backendzuordnung umfasst zusätzlich zu den in P23a genannten
Netzbackends lokale Volumes und UNC/gemappte Laufwerke über ihre OS-/SMB-Abfrage,
`zipfs` als schreibgeschütztes Archiv ohne behaupteten freien Speicher sowie
virtuelle Gerätebäume ohne Addition unabhängiger Volumes oder doppelt
referenzierter Konten. Archiv-Inhaltsgröße ist keine Kapazitätsquote.
Android `DocumentsContract.Root` erlaubt für verfügbare/gesamte Bytes ausdrücklich
`null` bei unbekannt oder unbegrenzt; diese Spalten werden im DocsRoot-/Bridge-
und `DocumentsCursors.kt`-Vertrag ergänzt. Android benötigt keinen erfundenen
Cachewert als Remotezahl.

**P26:** Die bestehende Ablage `cloud:<provider>` und Clientkonfiguration pro
Provider sind noch keine Account-/Clientgrenze. Der neue Persistenzvertrag
verwendet eine interne Authidentität aus Provider, Client-ID und bestätigter
Accountidentität sowie eine explizite Zuordnung der bisherigen Providerinstanz.
Der bisherige einzelne konfigurierte Provider bleibt bedienbar; ein neues
Mehrkonten-UI gehört nicht automatisch dazu. Refreshlock und atomare Rotation
sind an diese Identität gebunden. Legacydaten werden nur bei eindeutiger
Zuordnung übernommen. Unklare Zuordnung verlangt erneute Anmeldung statt
Ausprobieren desselben Tokens für mehrere Konten. Ein Kontowechsel darf gespeicherte
Drive-Root-IDs nicht still neu interpretieren; widersprüchliche Bindungen werden
als Identitätskonflikt angehalten. Die Suite enthält zwei getrennte Account-/
Clientfixtures, Clientwechsel und eine konkurrierende alte Refreshgeneration.

**P21:** Die Vertrauenswahl steht bereits im fehlgeschlagenen ersten Öffnungsdialog
am konkreten Tresor; sie ist kein nur nachträglich auffindbares Einstellungsfeld.
Vorhandene vertrauensbasierte Mountkonfigurationen werden nur bei eindeutiger
Vault-/Backend-/Konto-/Rootzuordnung atomar übernommen. Andernfalls bleiben sie
erhalten und verlangen die konkrete Bestätigung. Eine strengere veraltete
Konfiguration darf die gültig bestätigte Wahl nicht wieder überschreiben.
Android ist belegt über `docs_vault.rs::source`, den versiegelten Spool und
`withdraw`: nur entsperrte und ausdrücklich für Apps freigegebene Vaultroots,
Descriptorentzug beim Sperren, keine Klartextdatei auf Platte. Diese Kette und
ihre Rootprüfung gehören ausdrücklich zur Abnahme; eine Desktop-Mountänderung
allein deckt Android nicht ab.

## V5 — Vollständige Laufwerks- und Kopierintegration

P10b umfasst **zehn Verbraucher**: Sidebar, Ordnerpicker, Analysequellwahl,
Gerätebaum, Startseite, Aufräumquellwahl, Schnellsuche, Share-Exportauswahl,
Index-/Scanwurzeln und Windows-Dateiwatcher. Belegte Einstiegstellen ergänzen
`landing.rs`, `omni_accel.rs`, `share_exports_ui.rs`, `scanning.rs` und
`app/os/windows/watchers.rs` zur bisherigen Liste. Ein gemeinsamer Snapshotdiff
entfernt alte Watcherwurzeln, meldet neue an und invalidiert nur die betroffenen
Indexgenerationen. Der Nutzerroot und eine entfernte Remoteverbindung werden
nicht durch einen zufällig gleichlautenden Laufwerksnamen ersetzt.

Windows erhält ein eigenes verstecktes **Top-Level-Fenster** mit regulärer
Nachrichtenschleife für `WM_DEVICECHANGE`; kein allein auf Message-Queue-Polling
oder Message-only-Fenster gestützter Empfang. `GetMessage` dispatcht auch
gesendete Nachrichten; Rückgabe -1 wird als Fehler behandelt. Fokusabgleich
bleibt für Medienereignisse ohne Broadcast erhalten. Eine zusätzliche
CM-Notification-Schicht ist für diesen gewählten Weg keine Planvoraussetzung.

P14a ist ausdrücklich **neue** Arbeit: exklusives Erzeugen/Reservieren des
obersten Zielverzeichnisses auch lokal und ein dauerhaft gespeicherter
Quellroot→Zielalias. `exists` plus `create_dir_all` ist keine Reservierung.
Beide Enginewege und Recovery konsumieren dieselbe Zuordnung; nach Crash wird
ein bereits angelegter Alias durch Jobidentität bestätigt, nicht neu gezählt.

P11a erweitert neben dem Transfer die Copy-OS-Grenze
`copy/os/windows.rs`/`copy/os/shared/staging.rs`. Der sequentielle Reader muss
die bisher tatsächlich erhaltenen Zeitstempel, Dateiattribute, alternativen
Streams und Zielberechtigungen des jeweiligen Quell-/Zielformatpaars erhalten.
Für nicht unterstützte Metadaten gelten dieselben ausdrücklich sichtbaren
Grenzen wie im bisherigen Copyvertrag. Neue Metadatenverluste werden nicht
als optische Optimierung akzeptiert. Stageveröffentlichung, Hash, Cancellation,
Reparse-Grenze und Quelllöschung bei Move bleiben identisch. Eine Quelle mit
über diesen Reader nicht erhaltbarer Metadatenfähigkeit verwendet den unter
derselben Ein-Leser-Erlaubnis serialisierten bisherigen Copyweg.

P15 verwendet `StartRequest::OpenLocation` und `prefs_tabs.rs::navigate_to_location`.
Dieser verarbeitet Share, Vault und gespeicherte Remoteverbindungen bereits
getrennt; der Drive-Fallback öffnet heute nur den Root. Soweit P15 einen
konkreten Drive-Unterort öffnet, muss dessen unveränderter Locator über die
gemeinsame Auflösung geführt werden. Ein stiller Rootwechsel erfüllt K1 nicht.

## V6 — Fensterfamilien, Eingabe und Iconformate

P4 umfasst die sichtbare **Fensterfamilie**: Hauptfenster, zugeordnete Menüs,
Tooltips, Popups und modale Dialoge. Jedes Mitglied besitzt eigene Generation,
Geometrie, Z-Reihenfolge und Besitzerbezug. Ein Speichern-Dialog nach Close
muss sichtbar und bedienbar bleiben. Windows nutzt die verifizierte
Owner-/Prozesszuordnung; X11 `WM_TRANSIENT_FOR` plus überprüfte Besitzer-/
Sitzungsdaten. Nicht sicher zuordenbare Fenster werden nicht heimlich mitgefilmt.
Der Betrachter meldet dann die notwendige lokale Dialogbedienung oder bietet
den ausdrücklich gewählten vollständigen Desktop an.

WGC `IncludeSecondaryWindows` ist erst ab Windows 11 24H2 dokumentiert und
schneidet Inhalte außerhalb der Hauptfenstergrenze ab. Es ist eine optionale
Optimierung, kein vollständiger Dialogvertrag. Zugeordnete selbständige
Dialogfenster erhalten daher eigene Captureflächen. Der Windows-Captureindikator
bleibt eingeschaltet; ein randloses **Betrachterfenster** verlangt kein
`graphicsCaptureWithoutBorder`-Paketrecht am Host. Wayland benutzt ausschließlich
die standardisierten Portale mit tatsächlich gewählter Quelle, keine private
Mutter-RecordWindow-Schnittstelle. Portalpointerkoordinaten beziehen sich auf
die logische Streamgröße und den gewählten Stream, nicht unbesehen auf
PipeWire-Pixel oder globale Monitorkoordinaten.

**Eingabegrenze:** SendInput und XTest sind sitzungsweite Eingabe. Fokusprüfung
verhindert viele Fehlleitungen, ist aber keine Sicherheitsisolation eines
Fensters. `screen_control` bleibt deshalb ausdrücklich ein Recht für die
interaktive Sitzung; die Oberfläche behauptet keine technische Beschränkung
auf eine einzelne App. Nach Fokusabweichung stoppt der Sender Eingaben, löst
Tasten und verlangt bestätigte Neuzuordnung. Wer ausschließlich eine begrenzte
Appfreigabe benötigt, erhält dadurch kein unterschobenes Sitzungssteuerrecht.
Der zusätzliche RemoteAppweg hat seinen eigenen serverseitigen Appvertrag.

P3 überträgt ein einheitliches Rastericon mit Hash. Linux-SVG-Themes werden
am Host in Rasterbilder umgewandelt, nicht als SVG an alle Betrachter geschickt.
Geplanter Adapter ist resvg/usvg 0.49.0, vor Aufnahme gegen Projekt-MSRV und
Zielplattformen abzugleichen; ausschließlich der Iconadapter erhält ihn.
`image_href_resolver` weist externe Datei-/Netzreferenzen zurück; eingebettete
Ressourcen, Eingabegröße, Geometrie und dekodierte Pixel werden begrenzt.
Rasterformate nutzen den vorhandenen `image`-/PNG-Weg. Render-/Decoderfehler
ergeben das reale neutrale Fallbackicon, keine neue Voraussetzung für die
Fensterliste. Abnahme enthält SVG, PNG, Themewechsel, Popups außerhalb des
Hauptfensters, modale Dialoge, Fokusentzug und Besitzerwechsel.

## V7 — Messbare Darstellungs- und Leistungsziele

Die folgenden Werte sind **geplante Abnahmeregeln**, keine gemessenen Ergebnisse
und keine garantierten Produktionsraten. Alle Vergleichsfälle gehören in die
eine spätere Task-Suite.

| Punkt | Konkretes Ergebnis und Auswertung |
|---|---|
| P16 Listenhöhe | Bei gleicher Skalierung und ausreichend hoher Arbeitsfläche: anfängliche nutzbare Listenhöhe `min(2 × H_alt, H_arbeitsfläche − H_dialogchrome)`. Bei gespeicherter Nutzergröße wird diese erhalten und begrenzt. Sichtbare Zeilen ergeben sich aus tatsächlicher Zeilenhöhe; Dialogchrome/Buttons werden nicht zur Listenhöhe gezählt. |
| P7 Referenzverlust | Nach Verlust keine abhängigen Frames als korrekt präsentieren. Die erste bestätigte decodierbare Keyframegeneration beendet die Beschädigung. SSIM und PSNR werden gegen dieselben Captureframes vor/nach Änderung ausgewertet; verlustfreier Stillstand hat nach P31 exakt null abweichende Pixel. Codecqualität und Transportschäden werden getrennt ausgewiesen. |
| P30 statische Schrift | Der heutige Pixelhaushalt aus Bitrate und mindestens 10 fps darf eine unbewegte Quelle nicht zur Verkleinerung zwingen. Änderungsgetriebene Stillstandsübertragung behält innerhalb der ausgehandelten Codec-/Texturgrenzen die Quellpixel. Das deterministische Beispiel 3840×2160 bei 2 Mbit/s endet nach Übertragung der vollständigen Korrektur mit exakt passender Auflösung und pixelgleicher Schriftreferenz. Die zulässige Aufbauzeit folgt tatsächlichen komprimierten Bytes/Rate plus gemessener Pipelinezeit, keinem erfundenen Sofortversprechen. |
| P27 Rampenregel | Bei hinreichendem Encodeangebot wächst das Ziel pro unverstopftem Regelintervall um den Faktor 1,25 bis zur gemessenen Delivery-/Encodekapazität; kein permanenter 2-Mbit/s-Deckel. Geplante Regeltaktung: `max(gemessene RTT, Frameintervall)`. Nach Stau wird nicht durch größere Queues kaschiert. Faktor 1,25 ist eine ausdrücklich gewählte Regelungsannahme, im gemeinsamen Suitevergleich gegen Latenz/Verlust zu bewerten. |
| P27 Leistungsabnahme | Isolierter 10-Mbit/s-Pfad, 20-ms-RTT und saturierbare synthetische Frames; Produktionslimit entsteht daraus nicht. Die Relayreferenz ohne Encoder misst nutzbare Rate `R_ref` unter demselben Profil. Nach höchstens `ceil(log(R_ref/R_start)/log(1,25)) + 2` Regelintervallen muss das **Ziel** `R_ref` erreichen können. Tatsächlicher stationärer Nutzdurchsatz mindestens 90 % der Referenz; Differenzbudget 10 % ist ein offenes, hier festgelegtes Abnahmekriterium für Frame-/Regeloverhead. Steuerlatenz P95 darf gegenüber Referenz um höchstens zwei Frameintervalle wachsen; Queuealter darf im stationären Fenster nicht fortlaufend steigen. Niedrigrate und mehrere Fenster prüfen dieselbe Budgetteilung. |

Falls die gewählte Regelungsannahme die Kriterien verfehlt, wird sie innerhalb
derselben Implementierungs-/Suite-Fixschleife korrigiert. Ein fehlender
physischer Androidcodec ist kein bestandener Qualitätsvergleich. Deterministische
Decoder-/Referenz- und Ratecontrollerfälle bleiben ohne diesen Codec prüfbar.

P27 kann den getrennten Share-Server betreffen. Der Vergleich umfasst alten
Client/neuen Server, neuen Client/alten Server und neuen Client/neuen Server.
Ein neuer Client darf auf einem alten Server die Verbesserung erkennen, aber
nicht voraussetzen. Neue Serverpayloads laufen durch den vorhandenen kompletten
Releaseweg. Ein produktiver Share-Server wird nur nach tatsächlich erteiltem
Betriebsauftrag über seinen regulären Deploymentweg aktualisiert; Veröffentlichung
eines Payloads bedeutet noch keine Serverumstellung. Bis dahin nennt die
Diagnose die tatsächlich aktive Serverfähigkeit. Kein automatischer Eingriff
in einen Dienst aus der privaten Betriebsübergabe.

## V8 — Paketgrenzen und Modellzeitpunkt

P19b wird geteilt: verständliche Codec-/Statistikanordnung in B4;
**P19c Routezustand** gemeinsam mit P28d in B5. B4 übernimmt bis dahin nur
tatsächlich vorhandene Zustände und zeigt keine noch nicht ermittelte Route
als Relay. Damit hängt B4 nicht rückwärts von B5 ab. P18d ist die Grundlage
der neuen Rechte in B7/B8 und gehört zur B4-Integration.

Die ausdrückliche Vorgabe vom 2026-10-25 gilt für die aktuelle Planung
(GPT-6-Astra/max) und Kritik (Opus 5.5/xhigh). Der Nutzer hat seine zwischenzeitliche
voreilige Einschränkung „komplett über codex … sol 6.1 oder astra 6“ ausdrücklich
zurückgenommen, weil sie auf einer Fehlinterpretation des Dateinamens beruhte.
Die Umsetzungsmodelle werden erst nach abgeschlossener Planung zum von der
Arbeitsweise vorgesehenen Zeitpunkt ausgewählt. Dieser Plan startet keinen
Umsetzungsagenten und schreibt keine Auswahl aus der zurückgenommenen Nachricht
fest. Auslegung der ursprünglichen Kritikvorgabe: Opus 5.5/xhigh bleibt für Kritik maßgeblich, soweit der Nutzer dies beim später vorgesehenen Auswahlzeitpunkt nicht ändert. Die zurückgenommene Codex-Vorgabe ersetzt diese Wahl nicht. Die spätere Gesamtreview vor Tests bleibt ein eigener tatsächlicher
Arbeitsschritt; sie wird nicht durch diese Plankritik ersetzt.

## V9 — Remote-Nachweise und Veröffentlichung

Die vorhandene `.github/workflows/rechte-desktop-tresore-task.yml` belegt
**Konfiguration**, keinen erfolgreichen Lauf dieses Plans: Ubuntu 24.04 mit
`sch_netem`, Root-/FUSEfällen, Xvfb/Openbox/Xfwm4, Sway/Portal sowie einem
GNOMEjob; Windows 2025 für Core/Screen/Desktop; Androidemulator API 36 mit
API-35-Fallback und gesondertem API-30-Weg. Der GNOMEjob kann bei nicht startender
Shell ausdrücklich `not_run` liefern. Hardwareencoder, echte sichere Windows-
Desktops und produktive Providerkonten sind dadurch nicht nachgewiesen.

Die neue eine Task-Suite übernimmt nur benötigte Runner-/Artefaktmuster. Ihr
Manifest trennt **verbindliche isolierte Verträge** (Frametypen, Deadlines,
Rechte/Migration, Broker-IPC, Updategenerationen, Crash-Recovery, Quotaadapter,
Rootgrenzen) von **zusätzlichen Realfällen** (konkreter Greeter, Windows Hello,
physischer Codec, DVD und Liveprovider). Ein fehlender isolierter Vertragsfall
ist ein Suite-/Implementierungsdefekt und wird behoben; `not_run` ersetzt hier
keinen Erfolgsnachweis. Fehlende Realvoraussetzungen sind ehrlich dokumentiert
und niemals eine Releasevoraussetzung. Keine Zusatzkampagne und kein lokaler
Build/Test entstehen daraus.

Der bestehende Releasewrapper behandelt GitHub-Tokenrekursion bereits:
`native/publish-release-local.ps1::Test-GitHubActionsCompleteRelease`,
`Get-GitHubActionsPublicationRun` und `Invoke-GitHubActionsPublicationDispatch`
erkennen den gehosteten Dispatchkontext, verwenden den unveränderten Kandidaten
und erhalten eine konkrete Workflow-Run-ID. So wird die Publikation ausgelöst,
obwohl der mit `GITHUB_TOKEN` gepushte Tag keinen neuen Pushworkflow startet.
Das ist interner Bestandteil des einen kompletten Releasewegs, kein zusätzlicher
manueller Agenttrigger. Dokumentation: `docs/RELEASING.md`, Abschnitt um Zeile 501.
Dieser Plan benötigt weder einen Wrapperlauf noch einen Release.
