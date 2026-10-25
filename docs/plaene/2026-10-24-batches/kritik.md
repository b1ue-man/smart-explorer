# Kritik und Abschluss des Detailplans

Stand: 2026-10-25. Gegenstand ist ausschließlich der Plan in
[batches.md](batches.md), nicht eine bereits umgesetzte Produktänderung.

## Auftrag und Modellbeleg

Der Nutzer verlangt Planung mit GPT-6-Astra/max und Kritik mit Opus 5.5/xhigh.
Der Detailplanungsagent wurde über den Host ausdrücklich mit
`model=gpt-6-astra`, `reasoning_effort=max` gestartet. Der Host stellt für diesen
Agenten keine Antwortmodell-Metadaten bereit; belegt ist die Startkonfiguration,
nicht eine zusätzlich überprüfte Modellkennung aus dessen Antworten.

Die unabhängige Kritik wurde mit Claude Code 2.1.293 abgeschlossen, mit den ausdrücklichen
Startparametern `--model claude-opus-5-5 --effort xhigh` und ohne Fallbackmodell.
Initialisierung, Antwortmetadaten und abschließendes `modelUsage` melden tatsächlich
`claude-opus-5-5`; der Prozess endete erfolgreich. `xhigh` ist als Startparameter
belegt, nicht als zusätzliches Antwortmetadatenfeld. Sein isolierter Snapshot enthält Anforderungen, Plan,
Recherche und einzeln freigegebene Codeanker. Verfügbare Dateiwerkzeuge sind
ausschließlich Read, Grep und Glob; Arbeitsweise-MCP darf den delegierten
Auftrag protokollieren. Produktcode, Tests, Dienste und Releases werden nicht
ausgeführt. Private Betriebsnotizen gehören nicht zum Lesebestand.

Der erste Start scheiterte vor der Planlektüre an der CLI-Planmodus-Sperre des
Arbeitsweise-Metadatenaufrufs. Der korrigierte Leselauf verwendet eine explizite
Werkzeugfreigabe für genau diese Metadaten und weiterhin keine Dateiänderung
oder Codeausführung. Der fehlgeschlagene erste Start ist kein zusätzlicher
Kritikbericht; maßgeblich ist ausschließlich der danach abgeschlossene Leselauf.

Geprüfter Ausgangsplan, SHA-256:
`d916fa306597a4b42d6d8273e34c3c568275cc9d0a2f40db38b29e8627d25bf6`.
Zugehöriger Recherche-Snapshot, SHA-256:
`d51cd3b2fbd95e78bb85056be17982227eb6f23ba226fd65764eb0bbafca4268`.
Spätere begründete Änderungen werden im folgenden Abschluss festgehalten.

## Autorprüfung während der Kritik

Der Hauptagent hat E1 zusätzlich mit dem aktuellen SendSAS-Vertrag und den
vorhandenen Releaseskripten abgeglichen. Eine eigene Authenticode-Signierung
ist darin nicht belegt. Der Dienstweg verlangt deshalb releasegebundene,
unmittelbar vor Start hashgeprüfte Helfer mit administrativ geschütztem Pfad
und Erwartungshash; ein `uiAccess`-Weg wird nicht vorausgesetzt.
Beleg und Abgrenzung stehen in [Recherche G7](recherche.md#g7-ergänzender-abgleich-während-der-unabhängigen-kritik-dienstsignatur).

P11 entfernte im ersten Entwurf automatische Indexierung optischer Medien.
Das widerspräche dem Erhalt bestehender Fähigkeiten. Der Plan lässt diese
Indexierung deshalb an derselben Leserlaubnis warten und danach fortsetzen.
P22 koppelte das Austragen zu streng an vollständige Remote-Sicherung, obwohl
die Anforderung gerade einen nicht mehr vorhandenen Laufwerkszugang beschreibt.
Der Plan trennt nun aktive Registrierung, inaktiven Recoverybesitz und
Cachelöschung. Lokaler Zustand bleibt auch bei fehlender Gegenstelle erhalten;
fehlende Remoteinhalte werden nicht als gesichert ausgegeben.

## Befunde und Bearbeitung

Der Kritiker bewertete den Ausgangsentwurf als **noch nicht umsetzungsreif**.
Er bestätigte den vollständigen Umfang, fand aber fehlende Entscheidungen an
Sicherheits-, Prozess- und Integrationsgrenzen. Die folgenden 28 Kennungen
stammen aus seinem Bericht; sie bezeichnen **Befunde**, nicht die acht
fachlichen Pakete B1–B8. Sämtliche Korrekturen sind Planänderungen. Die
Prioritätsgruppen entsprechen dem Bericht: B1–B10 Priorität 1, B11–B20
Priorität 2, B21–B28 Priorität 3.

| Befund | Lücke und mögliche Auswirkung | Entscheidung / konkreter Planbeleg |
|---|---|---|
| B1 | P1 besitzt vor Benutzerlogin keine laufende Netzwerkidentität; Loginumfang könnte still auf Sperre/UAC verengt werden. | V1 plant einen unprivilegierten Boot-Netzdienst mit eigener Maschinenidentität und administrativen Grants; vier Sitzungszustände ausdrücklich enthalten. |
| B2 | Benutzer-SID als Brokergrenze und benutzerschreibbarer Helfer können lokale Rechteausweitung ermöglichen. | V1: Dienst-SID/UID, unabhängige signierte Brokeranforderungen, geschütztes Manifest/Installationsverzeichnis, erneute Hash-/Handleprüfung und administrativer Updateweg. |
| B3 | Grobe QUIC-Sperre lässt Handshake-/0-RTT-/Neupfadwege und Steuerframes offen. | V3 klassifiziert sämtliche Framevarianten des Pins, sperrt STREAM/DATAGRAM ab Pfadanlage und verlangt Prüfung vor jeder Serialisierung einschließlich Retransmits. |
| B4 | Kein endliches Ende der Direktphase; Anwendungs-Handshake und Deadlines können sich blockieren. | V3 bestimmt Snapshot, Sammelfenster, Probeerschöpfung, letzte Antwort-/Validierungsfrist, Spätkandidaten, Appverkehr und Timeoutfortschreibung. Kein nicht vorhandenes NAT-Finished-Ereignis vorausgesetzt. |
| B5 | Private Runtimekopie ohne geprüften stabilen Einstieg; unberührtes Profil bleibt alt; Stop-/IPC-Vertrag fehlt. | Agentenvertrag D1–D4: stabiler prüfender Starter, Currentmanifest, Generationen, Herkunft, kompatible Hashliste, Operationslocks, Transaktionsstop und Altprofilmigration. |
| B6 | Offlineprovider verhindert bei Vollständigkeitsforderung dauerhaft das Entfernen; Reimport fehlt. | P22 trennt aktiven Mount, inaktiven Recoverybesitz und Cachelöschung. Originalzustand und Datenlücken bleiben dauerhaft erhalten; versionierter Import/Reaktivierung ist enthalten. |
| B7 | Unknown-Quota als Fehlerstatus könnte Speichern in bestehenden Programmen brechen. | P23/V4 ersetzt den Fehlerfallback durch gekennzeichnetes reales Cachebudget bei weiterhin unbekannter Remotequote. Echte I/O-Fehler bleiben erhalten. |
| B8 | P10 erfasst nur vier statt sämtlicher Laufwerksverbraucher. | P10b/V5 nennt zehn Verbraucher einschließlich Startseite, Aufräumen, Suche, Exporte, Indexwurzeln und Watcher; Diff steuert deren Lebenszyklus. |
| B9 | „Wirksames Recht“ verschleiert verbleibenden Raumzugang nach Direktentzug. | P18/V2 zeigt Beziehungseffekt und erreichbaren Gesamtzugang samt Bedingungen. Bearbeitung und Sitzungsautorisierung bleiben quellgebunden, ohne erfundene Grantunion. |
| B10 | P4 lässt Menüs/Popups/Modalität weg und behauptet indirekt Fensterisolation. | V6 umfasst die Fensterfamilie und benennt SendInput/XTest als sitzungsweite Kontrolle. Nicht sicher zugeordnete Dialoge werden nicht heimlich erfasst. |
| B11 | TCP fehlt in MCP; lokale Portgrenze und RDP-Listenerzugang sind unklar. | V2 definiert denselben Grantweg über MCP, lokalen Mitbenutzerzugriff, Bestätigung bei Diensten ohne eigene Authentifizierung sowie den bestehenden lokalen RDP-Listener ohne automatische Umkonfiguration. |
| B12 | Neue Rechte haben keinen Speicher-/Einengungs-/Downgradevertrag. | Neuer P18d/V2: Version 1 für unveränderte Altgrants, Version 2 für neue eingeschränkte Grants, ausgeschaltetes Legacyfeld, Entzug bei Einengung und ausdrücklich benannte Altlesergrenze. |
| B13 | Share-Serverrollout und quantitative Leistungsabnahme fehlen. | V7: alte/neue Client-/Serverkombinationen, regulärer separat autorisierter Dienstrollout und konkrete Rampen-/Durchsatz-/Latenzkriterien. Payloadrelease ist keine heimliche Produktionsumstellung. |
| B14 | P7/P30/P16 besitzen zu schwache Ergebnismerkmale. | V7: Capture-Referenz und SSIM/PSNR, pixelgleicher Stillstand, native statische Quellauflösung trotz niedriger Rate sowie Formel für doppelte Listenhöhe. |
| B15 | Unklarer Runnerbestand könnte Sicherheitsverträge als fehlende Hardwarefälle überspringen. | V9 inventarisiert die konfigurierte Runnerbasis; isolierte Sicherheits-/Vertragsfälle sind verbindlich, zusätzliche reale Geräte-/Kontofälle dürfen ehrlich `not_run` sein. |
| B16 | P26 setzt Accounttrennung voraus, ohne Persistenzgranularität zu prüfen. | Recherche G8 belegt providerweite Ablage; V4 plant Authidentität, eindeutige Legacymigration, Kontowechselkonflikt und getrennte Account-/Clientfixtures. |
| B17 | Tresorvertrauen ist beim ersten Öffnen nicht erreichbar; Android und Migration fehlen. | P21/V4 stellt die Wahl in den ersten Öffnungsdialog, bindet bestehende Einträge an die bestätigte Identität und benennt die gelesene Android-Spool-/Descriptor-/Rootkette. |
| B18 | Pauschal „API-Lücken geschlossen“ behauptet mehr als die Recherche trägt. | Recherche G8 und Agentenvertrag schließen gewählte Verträge: Windows-Nachrichtenfenster, tatsächlicher Codex-Prozessstart, Portal-Koordinaten, OpenLocation, Androidquoten. Private Mutter-API, TermService-Neubindung und Borderless-WGC sind keine Planvoraussetzungen. |
| B19 | Aktuelle Modellvorgabe fehlt; spätere Reichweite unklar. | V8 und Modellabschnitt nennen Planung/Kritik, Start-/Antwortbeleg und spätere Auswahlzeit. Die nachfolgende voreilige Codex-Einschränkung wurde vom Nutzer ausdrücklich zurückgenommen. Die Auslegung zur Kritikmodellpräferenz ist getrennt als Auslegung benannt. |
| B20 | B4 setzt P28-Status aus B5 voraus. | P19c Routeanzeige liegt jetzt in B5 mit P28d; B4 enthält P19a/b. Abhängigkeitsgraph ergänzt B4→B5. |
| B21 | P23 lässt lokale/UNC-/zipfs-Quellen und virtuelle Aggregation offen. | V4 enthält OS-/SMB-Werte, schreibgeschütztes Archiv ohne freie Quote und keine Addition unabhängiger/doppelter Kapazitäten. |
| B22 | Exklusive lokale Zielanlage und Alias-Persistenz erscheinen fälschlich als Bestand. | P14a/V5 kennzeichnet beide als neue Arbeit, einschließlich Crash-/Jobidentität und Namensrace. |
| B23 | SVG-Themes ergeben ohne Rasterung fehlende Linuxicons. | P3/V6 plant begrenzte hostseitige Rasterung über resvg/usvg und überträgt Rasterbytes; externe Ressourcen werden abgewiesen. |
| B24 | Neuer optischer Reader könnte CopyFile2-Metadaten verlieren. | P11/V5 erhält Zeiten, Attribute, Streams, Zielrechte und Staging. Nicht erhaltbare Metadatenfähigkeiten verwenden den serialisierten bisherigen Copyweg. |
| B25 | LANklassifikation und dauerhafte Gerätepaarmesswerte fehlen. | V3 definiert physisches On-link-Interface, Overlay/VPN, CGNAT/ULA/Linklocal und private netzgenerationsgebundene Messhistorie. |
| B26 | MCP-PIN-Eingang unentschieden. | Agentenvertrag D5/P29c verwendet vorhandenes `stdin` mit `--pin-stdin`, sichere Ausgabe und vollständige Fehlerbehandlung. Client-Toolinput bleibt ausdrücklich kein geheimer Kanal gegenüber dem Client. |
| B27 | E14 wird einer Nachricht vor seiner Entstehung zugerechnet. | C trennt E14 vom 2026-10-24 von der älteren Vollmacht; die allgemeine Entscheidungserlaubnis ist als eigene Ableitung bezeichnet. |
| B28 | Tag aus Actions könnte zweiten Publikationsrun auslösen oder gar keinen. | Recherche G8/V9 belegt die bestehende Wrappererkennung und den expliziten kandidatengebundenen Dispatch mit Run-ID im GITHUB_TOKEN-Kontext. Kein neuer manueller Trigger. |

## Umfang und Grenzen der Prüfung

Der Kritiker las vollständig die Projektanweisungen, Architektur, Richtung,
Oberflächenindex, Projektübergabe, Ausgangsplan, Recherche und beide
Machbarkeitsdokumente. Seine konkreten Codeanker waren Share-Session und
AccessGrant, RootedBackend, Recoveryjournal, Drivebackend, Tokenpersistenz,
Windows-Updateradapter, MCP-Stop, Pixelkonvertierung, Ratenregelung, Pacing,
Ordnertransfer, Dokany-/FUSE-Metadaten, Relayqueue und Share-Serverrelay. Ein
separater Auszug enthielt die zuvor gelesenen Iroh-/Noq-Verträge. Er änderte
keine Datei und führte keine Produktprozesse aus. Die fehlenden Codeanker zu
Accountablage, Rechtewireformat, Installationsort, ALPN-Verbindungen und
Wrapperdispatch wurden danach vom Hauptagenten in Recherche G8 geschlossen.

Die ergänzten Verträge wurden **nicht erneut von Opus bewertet**. Die
Zusammenführung, Quellenkontrolle und Widerspruchsprüfung verantwortet der
Hauptagent. Der P13/P29-Nachtrag stammt vom erneut beauftragten Detailplanungsagenten
mit dessen ursprünglicher GPT-6-Astra/max-Startkonfiguration. Eine Quellenlektüre
und ein Planentscheid beweisen noch keine funktionierende Implementierung.

**Abschlussmaßstab:** Alle 27 aktiven Punkte plus P31, ihre ausdrücklich
ausgeschlossenen Umfänge, Entscheidungen, Abhängigkeiten, Kompatibilitätsregeln
und spätere Abnahmesignale bleiben enthalten. Die statische Dokumentkontrolle von Verweisen, Codeblöcken, Umfang,
Befundzuordnung und Whitespace ist abgeschlossen. Das Planungsergebnis ist
erreicht; dieser Auftrag endet damit. Umsetzung, Produktprüfungen, lokale
Builds, Versionswechsel und Release sind weiterhin nicht begonnen.
