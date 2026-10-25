# P13/E9 und P29: Vertrag für verwaltete Agenteneinbindungen

**Aktualisierung 2026-10-25:** Die Umsetzung wurde nach Planabschluss ausdrücklich
autorisiert; siehe [Ausführung](umsetzung.md). Die ursprüngliche Begrenzung auf
Planung unten beschreibt den damaligen Auftrag, keine aktuelle Ausführungssperre.

Stand: 2026-10-25. **Planungsnachtrag, keine Umsetzung.** Bezug: [Gesamtplan](batches.md), [Recherche](recherche.md), [Kritik](kritik.md), Opus-Befunde B5, B18 nur Codex und B26. Der Quellstand bei der Lektüre ist `aec62a40`; die Planungsdateien werden parallel überarbeitet. Dieser Nachtrag ersetzt für diese drei Befunde die bislang zu offenen Aussagen des Gesamtplans. Er ist keine zweite Aufgabenliste und reserviert keine Version.

## A — Auftrag und Grenzen

Die Nutzervorgabe vom 2026-10-25 lautet: „für planung und kritik möchte ich dass du im gpt 6 astra max für planung und opus 5.5 xhigh für kritik verwendest. du stoppst wenn der plan steht, du setzt ihn noch nicht um.“ P13 verlangt eine möglichst automatische Aktualisierung verwalteter MCP-/Plugin-Einbindungen. E9 bleibt **A+B**: neue Starts verwenden die neue Installation; kompatible laufende Server behalten ihre bisherige Generation. **C**, ein Austausch des Servers innerhalb einer laufenden MCP-Verbindung, ist ausgeschlossen. P29 ergänzt den konkret benutzbaren PIN-Eingang über MCP.

Ergebnisse dieses Nachtrags sind der stabile Einstieg mit Herkunfts- und Startprüfung, ein begrenzter IPC-/Speichervertrag, das Stopprotokoll, die Migration und objektive spätere Abnahmen. Er ändert ausschließlich dieses Dokument. Produktdateien, Agentenprofile, Dienste, Tests, CI und Releases sind in diesem Auftrag nicht zur Änderung oder Ausführung freigegeben.

Aus dem Auftrag zunächst bekannt waren die drei Kritikpunkte und die Einschränkung auf Planung. Durch Bestandslektüre zu klären waren der bisherige Startpfad, die Updatebeendigung, das tatsächliche stdin-Angebot und die belegten Clientgrenzen. Eine zusätzliche Nutzerentscheidung ist für den hier gewählten Vertrag nicht nötig. Planungstiefe **4** folgt aus der neuen Grenze zwischen Installation, Starter, Laufzeit, Benutzerprofil und Updater; sie bezeichnet den geplanten Strukturumbau, keine bereits ausgeführte Änderung.

## B — Verifizierter Bestand und Lücken

| Beleg | Vorhandenes Verhalten | Konsequenz |
|---|---|---|
| `native/src/cli/plugin/files.rs::{write_claude,write_codex_skill,refresh}`; `plugin/codex.rs::install`; `plugin/config.rs::render` | Claude und Codex starten den absoluten Pfad des erzeugenden `se` mit `mcp`. Der Marker enthält Version und Executable. Die Auffrischung richtet sich nach dem gerade laufenden `current_exe`. | Eine alte private Kopie kennt die neue Installation nicht automatisch. Ein Marker allein beweist nicht, dass ein später manuell bearbeiteter Eintrag noch unverändert verwaltet ist. |
| `native/src/cli/mcp/server.rs::{serve,get_info,list_tools}` | Der Server frischt Dateien nach seinem Start auf. Werkzeugkatalog und Initialisierungsanweisungen werden aus diesem Prozess bereitgestellt; der Katalog bleibt in einem `Arc<Vec<Tool>>`. | Ein nachträglicher Dateirefresh belegt keine vorherige Übernahme durch den Host. Eine feste Werkzeugliste je Servergeneration passt bereits zum Bestand. |
| `native/src/cli/mcp/session.rs::{open,close}` | Private Sitzung mit gehaltenem Dateilock; Kind-CLI wird über das `current_exe` dieser Sitzung gestartet. Beim Ende werden Jobs beendet und Sitzungsdateien entfernt. | Die Generation muss auch für Kindprozesse gelten. Eine bloße PID-Liste reicht weder zur Bereinigung noch zur Kompatibilitätsentscheidung. |
| `native/src/daemon/os/shared/update_stop.rs`; `native/src/cli/mcp/update_stop.rs`; `server.rs::serve` | Ein neuer `daemon.stop` mit Inhalt `stop` beendet auch MCP. Pollintervall 1 s; MCP-Abschluss wartet bis zu 10 s auf abgebrochene Jobs. Der gemeinsame Helfer dokumentiert höchstens 30 s auf Prozessende und keinen erzwungenen Kill. `handoff:<generation>` ist kein Stop. | Die bisherige Regel kennt keine kompatible Laufzeit. Eine Ausnahme braucht einen eigenen, an die Update-Transaktion gebundenen Vertrag. |
| `native/src/updater/os/windows.rs::{cli_self_replacement,spawn_update_helper}`; `linux_os.rs`; `shared/cli_swap.rs::verify_installed` | Windows verweigert CLI-Selbstaustausch; Linux kann den installierten Pfad ersetzen. Helper und ersetzte CLI werden gegen erwartete SHA-256 geprüft. | Die neue MCP-Lösung verwendet auf beiden Plattformen unveränderliche Generationen; sie stützt Windows nicht auf das Umbenennen einer laufenden EXE. Vorhandene Hash-/Rollbackmittel werden wiederverwendet. |
| `native/src/daemon/os/shared/ipc_protocol.rs` | Tokengebundene Requests, konkrete variantenspezifische Antworten; `Pong` kennt Version, Generation und Initialisierung, aber keinen ausgehandelten Semantikvertrag. Beispielsweise unterscheiden sich `ShareSnapshot` und `ShareSnapshotPeek` im Verhalten. | Eine gleiche Versionszeichenfolge oder deserialisierbare JSON-Antwort beweist keine IPC-Kompatibilität. |
| `native/src/cli/mcp/cli_tool.rs::{start,capture}`; `args.rs::opt_string` | Das Tool `se` nimmt ein separates `stdin`-Feld an, startet einen Kindprozess mit gepiptem stdin und schreibt dessen Bytes hinein. Sein Jobtitel enthält nur die Argumentliste. stdout/stderr werden gesammelt. Schreibfehler am stdin werden derzeit verworfen. | P29 kann diesen Eingang verwenden. Der neue Pairing-Befehl muss Ausgabegeheimhaltung und vollständige Eingabe sicherstellen; die MCP-Eingabe bleibt dem Client zugänglich. |

Die folgenden Detailabschnitte schließen die verbliebenen Fragen zu Manifest, Start, Stilllegung, Codex-Übernahme und PIN. Herstelleraussagen sind in Abschnitt F mit Abrufdatum belegt; vorgeschlagene Produktverträge sind ausdrücklich Planung.

## C — Gewählte Architektur und ausgeschlossene Alternativen

**Entscheidung:** Ein kleiner, fest adressierter Starter wählt vor jedem tatsächlichen MCP-Serverstart genau eine geprüfte Laufzeitgeneration aus. Er bleibt als Prozesswächter bis zu deren Ende bestehen. Laufende Verbindungen wechseln ihre Generation nicht. Benutzerprofile enthalten den stabilen Einstieg und eine versionsunabhängige Basisskill-Anweisung; die maßgeblichen generationseigenen Anweisungen kommen mit der MCP-Initialisierung.

| Möglichkeit | Entscheidung und Grund |
|---|---|
| Den installierten `se`-Pfad direkt weiterverwenden | Als öffentlicher CLI-Weg erhalten und bei `se mcp` auf den Starter weiterleiten; für die verwaltete Dauerregistrierung ungeeignet, weil Windows dieses Image beim Update ersetzen muss. |
| Einen vollständigen Laufzeitpfad im Agentenprofil festschreiben | Verworfen: ohne erneute Profiländerung bliebe ein unberührtes Profil auf der alten Generation. |
| Den stabilen Starter beim normalen Update überschreiben | Verworfen: er kann selbst noch laufen. Eine notwendige Startererneuerung erhält einen neuen Hashpfad und einen eigenen Migrationsschritt; die Protokollkompatibilität wird getrennt geprüft. |
| Neuer kleiner Starter plus generationenbezogene Laufzeiten | Gewählt: enger Verantwortungsbereich, Prüfung vor Codeausführung, festes Installationsziel und kein Überschreiben laufender Dateien. Das zusätzliche Starterartefakt gehört später in die bestehende Release-/Installertransaktion. |
| Server während einer bestehenden stdio-Verbindung austauschen | Verworfen gemäß E9-C: offene Requests, Cancellation, Shells, Sitzungsspeicher und Hostkatalog werden nicht durch eine Werkzeuglistenbenachrichtigung übertragen. |
| Clientneustart oder Konfigurationsreload von außen erzwingen | Kein Bestandteil des automatischen Updatewegs. Der belegte Hostweg wird angezeigt; Smart Explorer schreibt nicht in eine laufende Eingabeoberfläche. |

## D — Konkrete Verträge und spätere Umsetzung

Die folgenden Teilverträge werden vor einer Umsetzung gemeinsam in P13/P29 integriert. Die konkrete Ausführung gehört zu einem späteren ausdrücklichen Umsetzungsauftrag; die Meilensteine sind keine Freigabe für Tests, Builds oder Releases in diesem Planungsauftrag.

### D1 — Installationsidentität, Manifest und Starter

**Großer Meilenstein P13b.1:** Unter dem tatsächlichen Installationsstamm liegen `mcp/launchers/<Starter-SHA-256>/se-mcp-launcher.exe` auf Windows beziehungsweise `se-mcp-launcher` auf Linux, `mcp/current.v1.json` sowie `mcp/runtimes/<Runtime-SHA-256>/se.exe` beziehungsweise `se`. Dies sind **geplante Artefaktpfade**. Der einmal registrierte Starterpfad bleibt bei normalen Runtimeupdates unverändert; seine Datei wird niemals in-place aktualisiert. Der Starter leitet den Installationsstamm aus seinem eigenen geschützten Imagepfad ab; weder Arbeitsverzeichnis, `PATH`, Pluginmarker noch ein frei gesetzter Umgebungswert bestimmen das Manifest. Die Profilregistrierung speichert seinen absoluten Pfad und die zugehörige Installations-ID. Parallel installierte Benutzer- und Maschineninstallationen bleiben getrennte Ziele; es gibt keine automatische Umschaltung auf die höchste irgendwo gefundene Version.

`current.v1.json` enthält genau den Schemainterpreter, die Installations-ID, die bestätigte Transaktions-ID, Produktversion und Generation, Runtime-SHA-256, die feste relative Runtime-Datei, die benötigte Starterprotokollversion, den aktuellen Starter-SHA-256 und dessen Produkt-/Buildversion für neue Registrierungen, die endliche Liste weiterhin zulässiger Starter-Hashes, den IPC-Vertragsbezeichner, den Bezeichner des gemeinsam verwendeten Speicherformats sowie eine endliche Liste ausdrücklich weiter unterstützter Runtime-Hashes mit diesen Verträgen. Jeder Starter prüft vor dem Runtime-Start auch seine eigene Zugehörigkeit zur zulässigen Starterliste; damit kann eine fehlerhafte Starterfassung unabhängig von ihrer Protokollhauptversion zurückgenommen werden. Diese Selbstzuordnung ersetzt nicht den Schutz seines bereits vom Client gestarteten Images durch die Installation. Anzeigeversion und Kompatibilität sind getrennte Felder. Freie Befehle, Downloader-URLs, Benutzer-Credentials, absolute Fremdpfade und Argumentlisten sind nicht zulässig. Pfadtraversal, Links/Reparse-Punkte im verwalteten Teilbaum, falsche Installation, doppelte Generationen und unbekannte Pflichtschemata führen zum Startfehler.

**Herkunft und Schutz:** Der vorhandene Installer beziehungsweise Updater erzeugt das Manifest aus dem überprüften Releaseartefakt und dessen erwartetem Hash. Erst werden Generation und Dateirechte vollständig angelegt und überprüft, dann das Manifest über eine gleichverzeichnisige Stagedatei dauerhaft und atomar veröffentlicht. Der erfolgreiche bestehende Updateabschluss ist der Commitpunkt; davor bleibt der alte Zeiger gültig. Ein abgebrochener Versuch macht niemals eine Teilgeneration aktiv. Der Starter erzeugt keine eigenen Vertrauenshashes aus einer beliebig gefundenen EXE. Eine neben einer EXE liegende, vom selben Angreifer veränderbare `.sha256` ist kein Herkunftsnachweis. Es wird **keine vorhandene Authenticode- oder Manifestsignatur behauptet**: Vertrauen beruht hier auf dem bestehenden überprüften Installations-/Updateweg und dem geschützten Installationsstamm. [Q1, Q2]

Bei einer **administrativen Installation** besitzen ausschließlich Administrator/SYSTEM auf Windows beziehungsweise root auf Linux Schreib-, Ersetzungs- und Löschrechte an Starter, Manifest, Generationen und deren übergeordneten Verzeichnissen; berechtigte Nutzer erhalten Lese-/Ausführungsrechte. Bei einer **Benutzerinstallation** besitzt ausschließlich der installierende Nutzer diese Änderungsrechte, zusätzlich gelten die systemeigenen Administratorrechte. Die Benutzerinstallation ist keine Sicherheitsgrenze gegen absichtlichen Code desselben Betriebssystemkontos. Der Starter erhöht keine Rechte und übernimmt keine Tokens eines Updaters. Ein administrativer Updateprozess startet den MCP-Server niemals als SYSTEM/root für ein fremdes Profil.

**Kleiner Meilenstein P13b.2 — Hash direkt vor Start:** Der Starter hält zunächst die Lebensdauersperre der gewählten Generation. Windows öffnet die EXE mit `CreateFileW`, Lesezugriff und ausschließlich `FILE_SHARE_READ`, validiert den endgültigen Installationsbezug und hasht über dieses Handle. Die Handlebindung und geschützte Verzeichnisrechte bleiben bis nach `CreateProcessW` bestehen. `lpApplicationName` ist der vollständige geprüfte EXE-Pfad; es gibt weder Shell- noch PATH-Auflösung. Es werden nur stdio und der private Lebenszykluskanal über eine explizite Handleliste vererbt. Scheitern Öffnen, Hashvergleich oder Prozessstart, wird kein Ersatzprogramm gesucht. [Q1–Q3]

Linux öffnet die reguläre native Runtime lesend, prüft Eigentümer/Rechte und Hash und startet im Kind über `fexecve` **diesen Deskriptor**. Der Schutz gegen Inhaltsänderung stammt zusätzlich aus den beschriebenen Schreibrechten und der Regel, veröffentlichte Generationen niemals zu überschreiben. Ein Scriptwrapper ist keine zulässige Runtime; `ENOENT`, `ENOSYS`, fehlende Ausführungsrechte und Hashabweichung werden konkret gemeldet. Ein Rückfall auf ungeprüftes `execve(path)` ist ausgeschlossen. [Q5]

Der Starter interpretiert keine MCP-Nachrichten. Er übergibt die Standardkanäle an genau ein Runtimekind, wartet auf dieses Kind und reicht dessen Ende weiter. Ein privater, nicht an CLI-Enkel vererbter Lebenszykluskanal lässt das Runtimekind einen unerwarteten Startertod erkennen und den vorhandenen Sitzungsabschluss auslösen. Ein Runtimeabsturz startet innerhalb derselben Verbindung keinen Ersatzserver. Diagnose erscheint ausschließlich auf stderr; stdout bleibt Protokollkanal. Die Runtime trägt ihre Generation im vorhandenen Initialisierungs-/Statuspfad. Ihre `se`-Kindprozesse benutzen dieselbe Generation und ebenfalls die Startprüfung, keine frisch installierte fremde Version.

**Begrenzte Wartungsausnahme:** `se update` und schreibende `se plugin`-Verwaltungsbefehle aus einer beibehaltenen Runtime richten sich an die registrierte Installation. Sie übergeben einen typisierten Wartungsauftrag an deren aktuell geprüften Updater beziehungsweise Integrationsgenerator. Sie ersetzen niemals `Session.executable` im Generationenordner und schreiben keine alten Pluginvorlagen zurück. Normale Datei-, Transfer-, Exec- und Pairingbefehle bleiben generationengebunden. Der Wartungshandoff quittiert seine Übernahme, bevor der aufrufende Job seinen Operationslock freigibt; erst danach darf der Updatekoordinator exklusiven Operationszugang verlangen. Damit wartet ein aus MCP gestartetes Update nicht auf seine eigene dauerhaft gehaltene Jobsperre.

**Starterwartung:** Ein geänderter Starter erhält einen neuen Hashpfad; ein normaler Runtimewechsel setzt keine Profiländerung voraus, solange der bisherige Starter den Manifestvertrag erfüllt. Eine notwendige inkompatible oder sicherheitsbedingte Starterkorrektur stellt die eigenen Profilregistrierungen auf den neuen Hashpfad um und entzieht dem alten Starter bei Bedarf die Manifestkompatibilität. Unberührte alte Registrierungen erhalten dann einen konkreten Migrationsfehler; eine beliebige spätere Starterlücke lässt sich nicht durch die Behauptung „für immer stabil“ beheben. Alte Starter werden erst entfernt, wenn ihre Registrierungen bewusst migriert wurden und ihre Prozesse beendet sind; ohne diesen Nachweis bleiben sie als veralteter Einstieg gesperrt erhalten.

**Distribution, verbindlicher Teil von P13b.1:** Das neue kleine Binary wird auf dem Remote-Runner als eigenes Windows-/Linux-Ziel gebaut und über **dieselbe** vollständige Releasepipeline wie App, Updater und `se` verteilt. Die zusätzlichen Feed-/Releaseassets heißen `se-mcp-launcher.exe`, `se-mcp-launcher.exe.sha256`, `se-mcp-launcher` und `se-mcp-launcher.sha256`. Die vom Hauptagenten belegte bisherige Liste von genau 20 Assets in `release-publication.ps1` wird um diese vier auf 24 erweitert. Feedlayout, Download-/Staginglogik, `native/installer.nsi`, Linux-Installer, Hash-/Artefakt-/Versionsprüfungen und `README.md`/`docs/RELEASING.md` werden gemeinsam angepasst. Der veröffentlichte Starter nennt seine tatsächliche Produkt-/Buildversion und separat seine Starterprotokollversion; das Releasemanifest bindet seine Bytes an diese Version und den erwarteten Hash. Bereits laufende ältere Starter behalten ihren ausdrücklich unterstützten Protokollvertrag und werden nicht fälschlich als aktuelle Produktversion gemeldet. Installer/Updater prüfen das erwartete Starterartefakt vor seiner Ablage im Hashpfad. Eine zufällig lokal gefundene Launcherdatei wird niemals als Runtimekopie verwendet; es gibt keine unabhängige Downloadquelle oder zusätzliche Veröffentlichungspipeline.

**Spätere Abnahme:** Manipuliertes Manifest, veränderte Runtimebytes, fremde Installations-ID, vertauschter Pfad zwischen Prüfen und Start sowie fehlende Berechtigung starten keinen MCP-Code. Eine laufende Windows-Generation bleibt beim Update unverändert geöffnet; ein neuer Starteraufruf liefert die bestätigte neue Generation. Client-EOF, Runtimefehler und Startertod hinterlassen keine weiterarbeitende MCP-Sitzung. Linux startet bei einem nachträglichen Pfadtausch ausschließlich den bereits geprüften Dateideskriptor oder verweigert den Start.

### D2 — Profile, Codex-Vertrag und Migration

**Kleiner Meilenstein P13a.1:** Verwaltete Claude-`.mcp.json` und Codex-`mcp_servers.smart-explorer` zeigen auf den stabilen Starter. Der bisherige öffentliche Befehl `se mcp` bleibt verfügbar. Exporte durch `plugin config` erhalten denselben Einstieg; für Generic-, VS-Code- und OpenCode-Exporte wird keine automatische Konfigurationsänderung zugesagt. Die bisherigen Toolnamen, Claude-Namensräume, Benutzerrechte und das Ausführungsumfeld bleiben erhalten. Keine neue Codex-Plugininstallation ersetzt stillschweigend die vorhandene Codex-MCP-plus-Skill-Integration.

Der verwaltete Marker erhält Installations-ID, Starterpfad, Schemageneration und Fingerprints **der tatsächlich von Smart Explorer geschriebenen Einträge und Dateien**. Ein Refresh verändert nur diese Felder, wenn ihr aktueller Inhalt noch dem gespeicherten eigenen Stand entspricht. Benutzerergänzungen zu Freigaben, Toolfiltern und Umgebungsvariablen bleiben erhalten. Eine Abweichung führt zu `integration_conflict` mit konkretem Dateipfad/Feld, nicht zum Überschreiben. Alle Änderungen sind transaktional vorbereitete eigene Dateischreibvorgänge mit Vergleich vor Ersatz und Rücknahme des eigenen Diffs bei Scheitern; ein bestehender MCP-Eintrag wird nicht zunächst gelöscht. Der heutige `codex mcp remove`-vor-`add`-Weg wird deshalb ersetzt. Codex dokumentiert die TOML-Konfiguration ausdrücklich; der Adapter aktualisiert nur die eigene Tabelle mit einem formatbewahrenden Parser und prüft die resultierende Registrierung über den vorhandenen Hostadapter. Bei zeitgleicher Benutzeränderung bricht der Vergleich vor Ersatz ab. [Q7]

Der Dateiersatz garantiert vollständige Dateien; der Dokumentationsbefund beweist keinen prozessübergreifenden Codex-Konfigurationslock. Der Adapter serialisiert seine eigenen Refresh-/Installaufrufe und prüft den Dateiinhalt erneut vor und nach dem Ersatz. Wird eine konkurrierende Änderung erkannt, erfolgt keine unbedingte Rückschreibung einer alten Gesamtdatei: Der Konflikt bleibt sichtbar und nur ein noch eindeutig eigenes Delta darf zurückgenommen werden. Unkoordinierte Editoränderungen genau während des Ersetzens sind keine behauptete atomare Mehrschreibertransaktion. Normale Runtimeupdates schreiben die Codex-Tabelle nach der einmaligen Startermigration gar nicht mehr; dieser Konfliktbereich betrifft Installation, Migration oder einen ausdrücklich geänderten Starterpfad.

**Kleiner Meilenstein P13a.2 — unberührtes Profil:** Nach erfolgreichem Update wird die verwaltete Einbindung des ursprünglichen Nutzers durch denselben idempotenten Refresh aktualisiert. Der administrative Updateweg schreibt keine Konfiguration in sämtliche Profile. Beim nächsten tatsächlichen MCP-Prozessstart liest auch ein zwischenzeitlich unbenutztes Profil über seinen unveränderten Starterpfad das zentrale Manifest und startet sofort dessen neue Runtime. Diese Runtime kann danach ihre eigenen markierten Profildateien nachziehen. Es wird nicht behauptet, der Host hätte einen erst dann geänderten Skill schon zuvor gelesen.

Deshalb enthält der dauerhaft geladene Basisskill nur generationsstabile Aufgabenregeln: Smart Explorer zuerst verwenden, Rechte respektieren, aktuelle Serveranweisungen und vorhandene Werkzeughilfe beachten. Versionsspezifische Werkzeuglisten, Pfade und Featureversprechen gehören in die Initialisierungsanweisungen und Toolbeschreibungen der tatsächlich gestarteten Runtime. Diese Grenze vermeidet die vorausgesetzte Reihenfolge „erst Profilrefresh, dann Host-Skillcache“. Codex dokumentiert die Verwendung der Initialisierungsanweisungen. [Q7]

**Codex, präzise Grenze zu B18:** Die geprüfte Dokumentation beschreibt die gemeinsame lokale TOML-Konfiguration, einen ausdrücklichen Serverneustart in der Desktopoberfläche, einen Extension-Neustart in der IDE und `config/mcpServer/reload` im Appserver. Die Appserver-Methode lädt Konfiguration und stellt eine Auffrischung geladener Threads ein; sie ist kein belegter extern zugänglicher Steuerkanal für ein beliebiges `se` und kein Nachweis, dass jede neu eröffnete Unterhaltung einen Serverprozess startet. Der Plan benötigt sie nicht. „Neue Sitzung“ wird in P13 daher technisch als **neuer MCP-Serverprozess mit neuer Initialisierung** präzisiert. [Q7, Q8]

Der am 2026-10-22 in `machbarkeit.md` erwähnte Schalter `mcp_tool_list_changed` ist in der am 2026-10-25 abgefragten aktuellen Konfigurationsreferenz nicht aufgeführt. Daraus wird weder seine Entfernung noch seine allgemeine Verfügbarkeit abgeleitet. P13 setzt ihn nicht, verlässt sich nicht auf seinen Default und sendet keinen Katalogwechsel zur Laufzeitmigration. Die Werkzeugliste bleibt bis zum Prozessende gleich; eine bestehende Sitzung erhält neue Werkzeuge erst über den dokumentierten Neustart ihres Hosts. Damit ist die ungesicherte Schalterannahme aus dem kritisierten Vertrag entfernt. [Q9]

**Claude:** Der vorhandene lokale Marketplace bleibt erhalten. Relative Plugins eines lokal registrierten Marketplace werden aus ihrem Quellordner geladen; Änderungen gelten beim nächsten Sitzungsstart oder `/reload-plugins`. Bei beendetem stdio-Server verlangt die interaktive Sitzung ein Reconnect; `claude -p` und Agent SDK starten einen **mitten in der Sitzung beendeten** Server beim nächsten Toolaufruf neu. Ein Fehler schon beim Erststart hat davon abweichende Regeln. Diese Fähigkeiten sind Clientverhalten, kein Smart-Explorer-Hotswap. [Q10, Q11]

**Migration:**

1. Der erste P13-Release legt Starter, Runtime, Manifest und Sperrdateien neben der bestehenden Installation an. Vorhandene unversionierte MCP-Prozesse behalten für diesen Übergang den bisherigen Stopvertrag; ein einmaliges Neuverbinden wird angezeigt.
2. Ein Refresh im ursprünglichen Nutzerkontext migriert ausschließlich nachweislich eigene Einträge. Der alte Marker wird nur dann auf das stärkere Schema angehoben, wenn Executable, `mcp`-Argument und generierte Dateien dem belegten alten Format entsprechen; widersprüchliche Einträge bleiben erhalten.
3. Ein unberührtes **noch nicht migriertes** Profil kann über seinen weiterhin vorhandenen installierten `se mcp`-Pfad die Migration beim nächsten Start ausführen. Auf Windows kann dieser einmalige Elternprozess noch das installierte `se.exe` blockieren; für diese Sitzung wird keine neue Updateausnahme behauptet. Der folgende Serverstart verwendet die neue Registrierung. Ist ein früher registrierter fremder/verschobener Pfad nicht mehr vorhanden, kann dessen Code keine Migration ausführen: Status meldet den exakten defekten Einstieg und benötigt die einmalige erneute Registrierung im betroffenen Profil. Dieser Sonderfall wird nicht als automatisch repariert ausgegeben.
4. Ein bereits migriertes, seitdem unberührtes zweites Profil braucht nach normalen Runtimeupdates keine Registrierung mehr. Sein Starter liest die gemeinsame neue Generation selbst. Bei getrennten Benutzerinstallationen bezieht sich „neu“ ausschließlich auf die registrierte Installation.

**Spätere Abnahme:** Profil A aktualisiert eine gemeinsame Installation; Profil B bleibt währenddessen unangetastet und startet danach nachweislich die neue Runtime. Profil B mit altem unversioniertem Eintrag durchläuft genau den beschriebenen einmaligen Übergang. Eine manuell bearbeitete Codex-Tabelle bleibt einschließlich ihrer Filter erhalten; ein vor dem Ersatz erkannter Schreibkonflikt verhindert die Änderung. Konkurrierende Schreibabläufe werden zusätzlich an der beschriebenen begrenzten Transaktionsgarantie geprüft und nicht als atomare Mehrschreiberfähigkeit ausgegeben. Eine laufende Codex-Verbindung behält Katalog und Generation; ein belegter neuer Prozess übernimmt beide neu. Tests dürfen hierfür keine erfundene automatische Hostneuverbindung voraussetzen.

### D3 — Begrenzte IPC- und Speicherkombinationen

**Großer Meilenstein P13b.3:** Eine weiterlaufende Runtime ist nur kompatibel, wenn **alle** folgenden Bedingungen erfüllt sind: Sie ist eine durch D1 identifizierte Generation derselben Installation; ihr Hash steht in der neuen Manifestliste unterstützter Bestandsruntimes; ihr IPC-Vertrag und ihr Vertrag für gemeinsam verwendete lokale Zustände stimmen mit einem ausdrücklich angebotenen Vertrag des neuen Daemons überein; sie implementiert das neue Updateprotokoll und dessen Sperren. Keine Bedingung wird aus der Versionsnummer, einem erfolgreichen `Ping` oder einem gültigen JSON-Parser abgeleitet.

Der neue `RuntimeHello`-Request verwendet die vorhandene IPC-Authentifizierung und übermittelt `installation_id`, `runtime_sha256`, `ipc_contract_id` und `shared_state_contract_id`. Die Antwort bindet Annahme/Ablehnung an die tatsächliche Daemon-Startgeneration und nennt genau den akzeptierten Vertrag. Vor dem ersten daemonabhängigen Auftrag und nach einem Daemonwechsel muss dieser Handshake erfolgreich sein. Der Stand ohne Hello ist **legacy** und erhält keine Weiterlaufausnahme. Existierende Requests behalten für diese Migration ihre Bedeutung; ein unbekanntes Hello bedeutet `reconnect_required`, keine automatische Wiederholung einer Mutation gegen eine neue API.

Der erste Vertragsbezeichner friert die vom MCP-Server und seinen CLI-Kindern verwendeten IPC-Request-/Responsevarianten **einschließlich** Tokenprüfung, Root-/Credentialbezug, Fehlerbedeutung, Cancellation und Ergebnisabschluss ein. Der Speicherbezeichner umfasst die tatsächlich parallel verwendeten Konfigurations-/Credentialformate und deren Schreib-/Lockregeln. Ein Schemafingerprint dient der Änderungsdetektion, ersetzt aber nicht die semantische Prüfung. Änderungen an einem dieser Verträge verlangen entweder einen ausdrücklich implementierten Adapter für den alten Bezeichner oder den geordneten Stopp dieser Generation. Es gibt keine pauschale Zusage „alle alten Versionen“ oder „die letzten zwei Versionen“.

Das Kompatibilitätsfenster ist die **konkrete endliche Liste geprüfter alter Runtime-Hashes im eingehenden Release**. Die spätere auftragsbezogene Suite prüft jede dort freigegebene Kombination gegen den neuen Daemon und die gemeinsamen Zustände. Ein Hash ohne diesen Nachweis wird nicht eingetragen. Die heute noch nicht veröffentlichten Hashwerte werden nicht erfunden. Ein alter Runtimeprozess darf neue Profildateien nicht mit seinen alten Generatorvorlagen zurückschreiben; D2-Refresh wird ausschließlich von der gegenwärtig aktiven Installation ausgeführt, niemals aus einer beibehaltenen Generation.

**Spätere Abnahme:** Freigegebene Runtime/Daemon-Kombinationen erhalten identische Rechte-, Fehler-, Abbruch- und Schreibsemantik. Fehlender Listeneintrag, anderer Speicherbezeichner oder Daemonneustart ohne passenden Hello sperrt neue Arbeit nachvollziehbar; eine fehlgeschlagene schreibende Anfrage wird nicht automatisch nochmals ausgeführt. Lokale Credentials werden nie zwischen Benutzerkonten kopiert.

### D4 — Operationssperre und Stopprotokoll

**Großer Meilenstein P13b.4:** Die Installation erstellt feste Sperrdateien, die während ihrer Nutzung **nicht ersetzt** werden: `mcp/update-owner.lock` für den Updatebesitzer, `mcp/control.lock` für konsistente Status-/Stopmarker, `mcp/operations.lock` für aktive Arbeit und `mcp/runtimes/<Runtime-SHA-256>/lifetime.lock` je Runtime. Der zugehörige geschützte Transaktionszustand liegt in `mcp/update.v1.json`. Diese Dateien enthalten keine Credentials. Normale Nutzer müssen die Lockdateien öffnen und sperren können, erhalten bei administrativer Installation aber keine Inhalts-, Lösch- oder Verzeichnisschreibrechte. Windows verwendet `LockFileEx` mit Lesehandles auf genau einem Byte ab Offset 0; Linux verwendet `flock` auf lokalen regulären Dateien. Shared-/Exclusive-Locks sind hier ein Kooperationsvertrag, kein Schutz gegen einen absichtlich blockierenden lokalen Nutzer. Ein blockierter Lock führt zum Updateabbruch, nie zur Rechteausweitung. Netzlaufwerksinstallationen erhalten diese Weiterlaufausnahme nicht, solange ihr Lockvertrag nicht gesondert unterstützt ist; der bisherige geordnete Stopweg bleibt dort erhalten. [Q4, Q6]

Jede neue Runtime hält ihren Lebensdauerlock vom zugelassenen Start bis zum Ende. Jeder Auftrag, der Dateien, Netzwerk, Daemon oder gemeinsam genutzten Zustand berührt, hält einen Shared-Operationslock bis zum tatsächlichen Ende; Hintergrundjobs und persistente Shells halten ihn für ihre aktive Lebensdauer. Nur das Lesen bereits vorhandener lokaler Job-/Updatezustände und das Auslösen der bestehenden Cancellation dürfen während `prepared` ohne neuen Arbeitsauftrag fortfahren. Ein Updater verlangt exklusiven Operationszugang. Neue Aufträge lesen den Kontrollzustand vor **und nach** ihrer Shared-Locknahme und geben den Lock sofort zurück, wenn inzwischen `prepared` gilt. Der Updater wartet nie auf die Operationssperre, während er die kurze Kontrollsperre hält.

Auch ein `se`-Kindprozess erwirbt vor seiner ersten Arbeit einen eigenen Operations- und Lebensdauerlock und bestätigt dies dem Elternprozess über den internen Startkanal. Der Elternprozess hält seine Sperre bis zu dieser Bestätigung, damit kein ungeschütztes Zeitfenster entsteht. Auf Windows wird keine Lockvererbung behauptet: ein geerbtes Dateihandle überträgt laut Microsoft nicht die gehaltene Sperre an das Kind. Linux-`flock` kann über die offene Dateibeschreibung erhalten bleiben; ein bewusstes Unlock im Elternprozess darf die Kindarbeit deshalb nicht entsperren. Bevorzugt bleibt auch dort die explizite eigene Kindzulassung. [Q4, Q6]

**Verbindliche Zustandsfolge:**

1. **Prepare:** Der Updater hält den bestehenden Update-Ausschluss und die neue Eigentümersperre. Er verifiziert die neue Generation und schreibt unter der Kontrollsperre `prepared` mit Installations-ID, eindeutiger Transaktions-ID, Zielmanifest-Hash, seinen Prozessidentitätsdaten und der Kompatibilitätsliste. Neue Starts und neue Arbeit werden vorübergehend mit `update_in_progress` abgewiesen; bestehende Aufträge dürfen zu Ende gehen. Status und Abbruch bleiben erreichbar.
2. **Drain:** Der Updater verlangt exklusiven Operationszugang innerhalb des vorhandenen 30-s-Wartefensters. Läuft ein Transfer oder eine Shell danach weiter, endet dieser Updateversuch mit `update_deferred_busy`; das bestätigte Manifest bleibt unverändert, die Arbeitszulassung wird wieder geöffnet. Es gibt keinen automatischen Kill oder heimlichen Abbruch des Auftrags. Ein Nutzerabbruch wirkt ausschließlich über den schon vorhandenen Jobabbruchweg.
3. **Stop:** Erst nach erfolgreichem Drain veröffentlicht der Updater den zu dieser Transaktion gehörenden Stopauftrag. Für Legacyprozesse schreibt er weiterhin den exakten Inhalt `stop` nach `daemon.stop`. Die Kontrollinformation bindet die Ausnahme an die konkrete frisch erzeugte Stopdatei mit Dateiidentität und Änderungsstand; Veröffentlichung und Lesen des Paars werden durch die kurze Kontrollsperre atomar beobachtbar. Ein einfacher Stop ohne passendes lebendes Update, eine erneuerte Stopdatei oder eine fremde Transaktion bleibt ein normaler Beendigungsauftrag. Die bisherige Startbaseline für alte liegengebliebene Marker bleibt erhalten.
4. **Retain or exit:** Nur eine Runtime nach D3 bestätigt die passende Transaktion intern, bleibt mit unveränderter stdio-Sitzung bestehen und hält daemonabhängige Arbeit gesperrt. GUI, Daemon, unversionierte MCP-Prozesse und inkompatible Generationen beenden sich geordnet. MCP benutzt dafür weiterhin `Session::close` mit seinem bisherigen 10-s-Abschlussfenster. Der Updater prüft reale Imagepfade und Prozessidentitäten; ein Prozessname oder eine PID allein begründet keine Ausnahme. Nicht kooperierende Altprozesse verhindern den Ersatz nach dem bestehenden Timeout. Der neue Kooperationslock allein darf sie nicht als beendet ausweisen.
5. **Commit/rollback:** Der vorhandene Updater ersetzt ausschließlich seine regulären Installationsziele, startet/verifiziert den neuen Daemon und veröffentlicht erst nach erfolgreichem Abschluss das neue Manifest. Beibehaltene Runtimes verhandeln D3 erneut und nehmen danach Arbeit an. Bei Rücknahme gilt wieder das alte Manifest und dessen Daemonvertrag. Kein gerade unterbrochener mutierender Call wird automatisch wiederholt. Der zugeordnete Stopmarker wird als verarbeitet übernommen; eine spätere neue Stopanforderung wird weiterhin erkannt.
6. **Crash:** Eine hinterlassene `prepared`-Datei ist kein fortdauernder Freibrief. Eigentümersperre und Prozessidentität entscheiden, ob der Updater noch läuft. Ist die Transaktion verwaist, bleibt `update_recovery_required` sichtbar und neue Arbeit gesperrt, bis der reguläre Updater seine belegte Rücknahme oder Fertigstellung ausgeführt hat. Keine Bereinigung allein anhand einer fehlenden PID.
7. **Bereinigung:** Eine alte Generation wird erst entfernt, wenn sie weder aktuelle noch für Rollback benötigte Generation ist, ihr exklusiver Lebensdauerlock verfügbar ist und keine zugehörigen Runtime-/CLI-Prozesse mehr bestehen. Lockfehler, fehlende Rechte oder unklarer Prozessstatus erhalten die Dateien. Private Sitzungsspooldaten bleiben im jeweiligen Benutzerprofil; der Installer bereinigt keine fremden Sitzungsinhalte.

Bei administrativer Installation gilt die Operationssperre installationsweit über die nutzenden Konten. Der Updater muss daher keine fremde Agentenkonfiguration öffnen, um kooperierende aktive Arbeit zu erkennen. Nicht kooperierende Programme sind durch diese Sperre nicht erfasst; für sie bleiben die vorhandenen Prozess-/Stopprüfungen maßgeblich. Bei Benutzerinstallationen sind Installations-ID, Locks und Manifest ausschließlich dieser Installation zugeordnet. Ein Update der Benutzerinstallation A sperrt nicht die getrennte Benutzerinstallation B.

**Spätere Abnahme:** Idle-MCP in Profil B behält seine Verbindung während des Updates aus Profil A. Ein aktiver langer Auftrag führt zu einem zurückgestellten Update und bleibt bedienbar. Ein früher beendeter Auftrag lässt genau einen Ersatz zu. Windows-Kindprozess mit beendetem Elternprozess hält seine eigene Sperre bis zu seinem Ende; seine Generation wird nicht gelöscht. Ein unversionierter Altprozess wird niemals wegen fehlender neuer Locks übersprungen. Falsche Stop-Transaktion, Marker aus einem abgestürzten Update, PID-Wiederverwendung und abweichende ACLs ergeben die definierten Stop-/Fehlerzustände. Die Linux-Abnahme umfasst gemeinsam geöffnete und separat geöffnete Lockdeskriptoren.

### D5 — P29: PIN über den vorhandenen MCP-stdin-Kanal

**Kleiner Meilenstein P29c:** Der bestehende MCP-Katalog beschreibt `se.stdin` bereits ausdrücklich als Eingang für `--pin-stdin`. Der vorgesehene Aufruf lautet strukturell:

```json
{
  "args": ["share", "discover", "pair", "<Angebots-ID>", "--pin-stdin", "--json"],
  "stdin": "<PIN exakt als empfangene Zeichenfolge>"
}
```

Die beiden Winkelklammerwerte sind Platzhalter dieses Planbeispiels, keine auszuführenden Werte. Der PIN steht ausschließlich im `stdin`-Feld; das Tool schreibt dessen UTF-8-Bytes ohne selbst ergänzten Zeilenabschluss und schließt die Pipe. Die neue CLI-Option liest bis EOF und reicht die Zeichenfolge ohne eigenes Trimmen, Groß-/Kleinschreibung oder Unicode-Normalisierung an den gemeinsamen Discovery-Vertrag weiter. Die dort geltende Validierung bleibt maßgeblich; MCP erfindet keine zweite PIN-Grammatik. Fehlt `stdin`, ist die Eingabe leer oder verletzt sie diesen Vertrag, scheitert der Aufruf ohne Netz-Kopplungsversuch und ohne Wiedergabe der Eingabe.

**Tatsächlich notwendige Ergänzungen:** Der P29-CLI-Unterbefehl und dessen `--pin-stdin` werden ergänzt; der bestehende MCP-Kanal selbst muss nicht erfunden werden. `cli_tool::capture` muss einen Schreibfehler und den vollständigen Abschluss des stdin-Schreibens an den Job zurückgeben, statt den Fehler zu verwerfen. Die Pairing-Implementierung gibt den PIN weder auf stdout/stderr noch im Fehlertext aus; `Debug`, Tracing, IPC-Diagnosen, Joblabel, Sitzungsübersicht und Spool enthalten keine PIN. Ergebnisfelder beschränken sich auf Angebots-/Austauschkennung, bestätigte Geräteidentität, Status und einen nicht geheimen Fehlercode. Ein `--pin`-Argument mit Klartextwert wird für diesen Weg nicht angeboten. `--pin-prompt` ist im MCP-Pipekontext nicht verfügbar und muss sofort einen nicht interaktiven Eingabefehler liefern, statt auf eine nicht vorhandene Konsole zu warten.

Das Pairing muss seinen Austausch bis zum beobachteten Abschluss oder Abbruch einem besitzenden Job zuordnen. Weil der vorhandene Windows-`se`-Job sein Kind beim Abbruch unmittelbar beenden kann, genügt ein CLI-Destruktor nicht als zugesagte Daemonbereinigung: Die Daemonseite bindet den Austausch an die besitzende Sitzung/Jobverbindung und beendet ihn bei deren Ende; ein expliziter Cancel bleibt zusätzlich möglich. Ein bereits unabhängig bestehender Austausch wird dadurch nicht beendet. Diese P29-Integration liegt beim gemeinsamen Discovery-/CLI-Arbeitsplan des Hauptagenten.

**Geheimhaltungsgrenze:** Der vorhandene MCP-Client und der Agent sehen die Tool-Eingabe. Smart Explorer kann nicht zusagen, dass ein fremder Client sie nicht in seiner Unterhaltung oder seinem eigenen Debuglog speichert. Der zugesagte Schutz umfasst Prozessargumente, Smart-Explorer-Standardlogs, dessen Spool/Status und Toolergebnisse. Für einen PIN, den der Client nicht erhalten soll, wird kein geheimer MCP-Eingabedialog behauptet; die Kopplung erfolgt dann über die lokale GUI oder den mit P29 geplanten Terminal-Prompt außerhalb des MCP-PIN-Aufrufs, und MCP fragt anschließend nur den Kopplungsstatus ab. Das ist ein ausdrücklich benannter Bedienweg, keine still angenommene neue Clientfähigkeit.

**Spätere Abnahme:** Ein ausschließlich zu Prüfzwecken erzeugter Sentinel-PIN gelangt bytegetreu in den gemeinsamen Pairing-Eingang. Er erscheint weder in Prozessargumenten, Smart-Explorer-Logs, Joblisten, stdout/stderr-Spool noch in irgendeiner Toolantwort; auch Ablehnung, ungültige Eingabe, Broken Pipe, Abbruch und Daemonfehler geben ihn nicht wieder. Der MCP-Request selbst bleibt von dieser Behauptung ausdrücklich ausgenommen. Falscher PIN, abgelaufenes Angebot, bestehender Austausch und Offlinezustand behalten die bestehenden Hostrechte. Abbruch entfernt ausschließlich den eigenen noch laufenden Austausch.

### D6 — Umsetzungsschnitt und Oberflächen

| Meilenstein | Konkrete spätere Dateigrenze | Abhängigkeit |
|---|---|---|
| P13b.1/b.2 Starter, Manifest, sichere Starts | Neues eigenständiges Starter-Binary und schmale Kern-/OS-Module; geplante Zuordnung `native/src/mcp_launcher/core/` für Manifest-/Policytypen, `native/src/mcp_launcher/os/windows.rs` und `linux_os.rs` für Handles/Start/Locks, ein dünner Einstieg unter `native/src/bin/`. Bestehende Hashfunktionen aus `native/src/updater/` wiederverwenden. Launcher-Assets und gemeinsame Auslieferungsänderung sind in D1 verbindlich festgelegt; genaue vorhandene Buildskriptpfade ergänzt der Hauptplan aus seinem Quellbefund. | Bestehender Release-/Installationsweg; kein neuer unabhängiger Releaseworkflow. |
| P13a.1/a.2 Profilrefresh und Migration | `native/src/cli/plugin.rs`, `plugin/{files,config,codex,claude,host_cli}.rs`; neue eng begrenzte Module für Eigentumsfingerprints und Transaktion. Die eingebettete Basisskill-Datei ist eine zusätzliche spätere Integrationsfläche. | D1; dokumentierte Hostkonfiguration Q7–Q11. |
| P13b.3 IPC-/Speichervertrag | `native/src/daemon/os/shared/ipc_protocol.rs` mit ausgelagertem Runtime-Vertragsmodul; Daemon-Handler/-Client und tatsächlich gemeinsame Persistenzadapter als Integrationsflächen des Hauptplans; `native/src/cli/mcp/{server,session,cli_tool}.rs`. | D1; explizite Liste freigegebener Hash-/Vertragskombinationen. |
| P13b.4 Stop und Lebensdauer | `native/src/daemon/os/shared/update_stop.rs`, `native/src/cli/mcp/update_stop.rs`, `session.rs`, `server.rs`, `cli_tool.rs`, `exec_tool.rs`, `exec_session_job.rs`; Updater-Plattformadapter sowie `native/src/bin/smart_explorer_updater.rs::apply_update` und `native/src/bin/smart_explorer_updater/process.rs::stop_target_processes_for_update`. | D1/D3; alle Kindprozesse und persistente Shells eingeschlossen. |
| P13c Status | Bestehende Pluginstatus-/MCP-Statuspfade, CLI Text/JSON und Desktop-Agenteneinstellungen; `docs/AI_AGENT_PLUGIN.md` sowie kanonische Installations-/Releasedokumentation. | D2/D4. Status unterscheidet installierte, aktive und nächste Generation, Profilkonflikt, Busy, Recovery und erforderliches Reconnect. |
| P29c MCP-PIN | `native/src/cli/mcp/{catalog,cli_tool,args}.rs` und P29-Discovery-CLI/Daemon-Jobbesitz im Hauptplan. | D5 und gemeinsamer Discoveryvertrag, keine neue Kopplungsimplementierung. |

Windows und Linux sind betroffen. Android besitzt keinen lokalen `se mcp`-Host und erhält daraus keinen neuen Runtime- oder Plugininstaller; sein bestehender Kopplungszustand bleibt mit P29 gemeinsam. Ein neuer dynamischer MCP-Toolkatalog, Clienthooks, HTTP-MCP-Proxy, Secret-Elicitation und remote verwaltete Agentenprofile sind nicht Bestandteil dieses Vertrags.

Der Bedienaufwand für ein normales Runtimeupdate sinkt geplant auf den vorhandenen Updateaufruf: eine bereits migrierte Einbindung braucht keinen zusätzlichen Registrierungs- oder Reloadschritt. Eine laufende kompatible Sitzung behält ihre alten Fähigkeiten; neue Fähigkeiten sind beim nächsten tatsächlichen Serverstart verfügbar. Der seltene Legacyübergang, eine inkompatible Runtime oder ein Profilkonflikt zeigt genau den betroffenen Einstieg und den dokumentierten Reparatur-/Neustartweg. Es wird keine automatische Aktion in der Hostoberfläche simuliert. Fehlende Dauerangaben erscheinen als Wartezustand, nicht als erfundener Fortschrittsprozentsatz.

## E — Abnahmezuordnung und noch nicht ausgeführte Nachweise

| Kritikbefund | Konkrete Schließung im Plan | Objektives späteres Signal |
|---|---|---|
| B5 a: Hashprüfung vor Runtime-Start fehlt | D1: geschütztes Installationsmanifest, Windows-Handlebindung, Linux-Deskriptorstart | Keine ausgeführte Runtime bei falschem Hash oder manipuliertem Ziel; gestarteter Imagehash entspricht dem ausgewählten Manifest. |
| B5 b: alte private Kopie beziehungsweise zweites Profil bleibt alt | D1/D2: stabiler installierter Einstieg, zentrale Generation, Basisskill; expliziter Legacyübergang | Unberührtes migriertes Profil B startet nach Update durch A die neue Generation ohne Profilmutation durch A. |
| B5 c: globale Stopregel und IPC-Ausnahme offen | D3/D4: konkrete Hash-/Vertragsliste, Hello, installationsweite Arbeitssperre, transaktionsgebundene Ausnahme | Kompatible Idle-Sitzung bleibt; lange aktive Arbeit verschiebt das Update; Legacy-/inkompatibler Prozess erhält keine Ausnahme. |
| B18, Codex | D2 und Q7–Q9: belegte Reloadgrenzen, keine Abhängigkeit vom unbestätigten Featureflag, Prozessstart als Übernahmegrenze | Bestehender Katalog bleibt stabil; neuer Prozess übernimmt neue Konfiguration/Generation; kein angeblicher Reload ohne Hostaktion. |
| B26 | D5: bereits vorhandenes `se.stdin`, explizite nötige CLI-/Fehler-/Jobbesitzergänzungen und Client-Sichtbarkeitsgrenze | Sentinel-PIN gelangt exakt zum Pairing, fehlt in argv/SE-Logs/Spool/Antwort; Cancel betrifft ausschließlich eigenen Austausch. |

Diese Signale gehören später in **eine** automatisierte auftragsbezogene Remote-Suite zusammen mit dem übrigen ausdrücklich für denselben Release beauftragten Umfang. Die Suite erfasst Windows/Linux, zwei Benutzerprofile einer administrativen Installation, getrennte Benutzerinstallationen, unterstützte Altgenerationen, ACL-/Hashabweichung, Crash, Updatebusy, Cancellation und PIN-Geheimhaltung. Externe Hostprüfungen werden mit den tatsächlich eingesetzten Codex-/Claude-Versionen dokumentiert; fehlender realer Hostnachweis wird nicht als bestanden ausgegeben und ist nach Projektregel kein eigenständiger Releaseblocker. In diesem Auftrag wurden weder diese Abnahmen noch Tests oder Builds ausgeführt.

**Exakte Integrationsgrenzen:** Der begrenzte Nachtrag hat den Updater-Hauptprozess, Installer/Feedlayout, Daemon-Handler/-Client, sämtliche Credential-/Persistenzadapter und den eingebetteten Skillinhalt nicht neu gelesen. Der Hauptplan ergänzt deren tatsächliche Dateizuordnung; die oben festgelegten Verträge sind dadurch nicht optional. Vom Hauptagenten liegt inzwischen folgender konkreter Quellbeleg vor: `native/src/bin/smart_explorer_updater.rs::apply_update` (143–242) prüft Helper-/App-/CLI-Hashes, stoppt `args.target` und `args.cli_target`, ersetzt die Transaktion, wartet auf `startup_ack` und finalisiert erst danach. `native/src/bin/smart_explorer_updater/process.rs::stop_target_processes_for_update` wartet 30 s ohne Kill; Windows vergleicht volle normalisierte Imagepfade, Linux verwendet `linux_target_pids`. Diese beiden Dateien, `native/installer.nsi` und die vom Hauptplan konkret zugeordneten vorhandenen Packaging-/Feedskripte gehören zur Starterauslieferung und Stopintegration. Dies ist **übernommener Hauptagentenbeleg**, keine eigene zusätzliche Lektüre. Die konkrete Liste künftig kompatibler Runtime-Hashes wird erst aus den späteren Kandidaten und der einen Remote-Suite erzeugt. Ein noch unbekannter Hash ist kein offener Architekturentscheid.

## F — Aktuelle Primärquellen und zweite Lückenrecherche

Alle folgenden Quellen wurden am **2026-10-25** abgerufen. Die Links belegen externe API-/Clientaussagen; die gewählten Produktverträge in D sind daraus abgeleitete Planung.

| Kennung | Quelle und tatsächlich benötigter Befund |
|---|---|
| Q1 | [Microsoft CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew): Lesezugriff, Share-Modi, Lebensdauer der Handlefreigabe; ohne `FILE_SHARE_DELETE` kein damit kollidierender Lösch-/Renamezugriff. |
| Q2 | [Microsoft CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw): vollständiges `lpApplicationName`, beschreibbare Kommandozeile, gültige Standardhandles, Aufrufer-Sicherheitskontext, Prozesshandles/Startfehler. Ein erfolgreicher Aufruf allein beweist noch keine abgeschlossene Kindinitialisierung. |
| Q3 | [Microsoft Prozesse erzeugen](https://learn.microsoft.com/en-us/windows/win32/procthread/creating-processes) und [UpdateProcThreadAttribute](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute): explizite `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` statt unbeschränkter Handlevererbung. |
| Q4 | [Microsoft LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex): shared/exclusive und nicht blockierendes Sperren auf Lesehandles möglich; Kindprozesse erhalten durch Handlevererbung nicht die Sperre; Freigabe bei Prozessende/Handleende, verzögerte Freigabe möglich. |
| Q5 | [Linux fexecve(3)](https://man7.org/linux/man-pages/man3/fexecve.3.html), man-pages 6.19, und [execve(2)](https://man7.org/linux/man-pages/man2/execve.2.html): Start über geöffneten Deskriptor, Fehlerfälle und notwendiger Schutz gegen Inhaltsänderung. Die Prüfen-dann-Pfadstart-Lücke wird ausdrücklich beschrieben. |
| Q6 | [Linux flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html): kooperative shared/exclusive Locks, offene Dateibeschreibung, Verhalten bei dup/fork/exec und Einschränkungen für NFS/SMB. |
| Q7 | [Codex MCP](https://learn.chatgpt.com/docs/extend/mcp): lokale STDIO-Konfiguration, MCP-Initialisierungsanweisungen, TOML und unterschiedliche Neustartaktionen in App/CLI/IDE. |
| Q8 | [Codex Appserver: API overview](https://learn.chatgpt.com/docs/app-server#api-overview): `config/mcpServer/reload` lädt Konfiguration von Disk und stellt Threadrefresh ein; `mcpServerStatus/list` kann eine Threadverbindung benutzen. |
| Q9 | [Codex Configuration Reference](https://learn.chatgpt.com/docs/config-file/config-reference): vollständig abgerufene Referenz maschinell auf `mcp_tool_list_changed`/`list_changed` durchsucht; kein Treffer. Das belegt ausschließlich die fehlende Dokumentation dieses Schalters in dieser Referenz. |
| Q10 | [Claude Code MCP](https://code.claude.com/docs/en/mcp), Abschnitte „Dynamic tool updates“ und „Stdio servers that exit or fail to start“: Laufzeitkatalogrefresh und Reconnect unterscheiden sich nach Clientmodus und Fehlerzeitpunkt. |
| Q11 | [Claude Plugin loading](https://code.claude.com/docs/en/plugins/loading), „In-place and copied plugins“: lokal hinzugefügte relative Marketplaceplugins laden aus der Quelle; Übernahme bei neuem Sitzungsstart oder `/reload-plugins`. |

Die zweite Lückenrecherche änderte den Entwurf an vier konkreten Stellen: `LockFileEx`-Sperren werden nicht als vererbt behandelt; Linux nutzt `fexecve` statt eines nur vorab geprüften Pfadstarts; Codex-Appserverreload wird von einer universellen externen Reloadfähigkeit getrennt; P29 verwendet den bereits im Katalog angebotenen stdin-Eingang. Die zunächst angefragte Datei `native/src/cli/mcp/private_fs.rs` existiert nicht; `session.rs` verweist tatsächlich auf `support_dirs::{ensure_private_dir,open_private_lock}`. Die ergänzend erlaubte Lektüre von `native/src/support_dirs.rs` bestätigt die Delegation an `creds::private_storage`; sie ist kein Beleg, dass dessen private Profilsperre unverändert als lesbare installationsweite Sperre verwendbar wäre.

## G — Lese- und Änderungsnachweis dieses Planungsagents

**Dieser Nachtrag:** Tatsächlich gelesene Texte beziehungsweise gezielt ausgewertete Suchtreffer stehen einzeln in der folgenden Liste. „Auszug/Suche“ behauptet keine vollständige Dateilektüre. Die Projektanweisungen aus `AGENTS.md` wurden im Nutzerkontext vollständig übergeben; diese Übergabe wird nicht als eigener Dateiaufruf ausgegeben.

- `docs/ARCHITEKTUR.md` — gezielte Architekturtreffer; die vorherige Planlektüre bleibt gültig.
- `docs/AI_AGENT_PLUGIN.md` — gezielte MCP-/Installations-/Update-/stdio-Stellen.
- `docs/oberflaechen/INDEX.md` — gezielte Oberflächensuche; vorherige Lektüre vorhanden.
- `docs/plaene/2026-10-24-batches/batches.md` — P13/E9/P29 sowie Vertrags- und Oberflächenzuordnung.
- `docs/plaene/2026-10-24-batches/recherche.md` — Quellbelege und R5.
- `docs/plaene/2026-10-24-batches/kritik.md` — Kritikrahmen und Modellnachweise.
- `docs/plaene/2026-10-22-laufwerke-medien-agenten/machbarkeit.md` — P13/E9 und damalige Codex-/Claude-Befunde.
- `/tmp/smartexplorer-planreview-glth_z45/result.txt` — Abschnitte zu B5, B18 und B26.
- `native/src/cli/plugin.rs` — gezielte Einstiegs-/Registrierungssuche.
- `native/src/cli/plugin/files.rs` — Generierung, Marker, Refresh und atomarer Dateiersatz.
- `native/src/cli/plugin/config.rs` — Exportformate und direkte Executable-Einträge.
- `native/src/cli/plugin/codex.rs` — Install, Status, Remove/Add und Umgebungsübernahme.
- `native/src/cli/plugin/claude.rs` — gezielte Registrierungs-/Statusstellen.
- `native/src/cli/plugin/host_cli.rs` — gezielt durchsucht; keine zusätzliche Implementierungsbehauptung daraus.
- `native/src/cli/mcp.rs` — Modul-/Hilfezuordnung durch gezielte Suche.
- `native/src/cli/mcp/server.rs` — Initialisierung, Werkzeugdispatch und Stop-/Sitzungsabschluss.
- `native/src/cli/mcp/cli_tool.rs` — Childstart, stdin, Spool und Cancellation.
- `native/src/cli/mcp/args.rs` — Argumenttypen und Stringentnahme.
- `native/src/cli/mcp/catalog.rs` — `se`, `stdin`, Jobstatus, Jobinput und Cancellation.
- `native/src/cli/mcp/session.rs` — private Sitzung, Runtimepfad, Lock und Bereinigung.
- `native/src/updater/os/windows.rs` — Auszug bis Zeile 240: Helperhash, CLI-/Installationspfad und fehlender Selbstaustausch.
- `native/src/updater/os/linux_os.rs` — Auszug bis Zeile 190: Installationspfad, Helperstart und CLI-Austausch.
- `native/src/updater/os/shared/cli_swap.rs` — Auszug bis Zeile 200: Lock, Rollback und erneute Hashprüfung.
- `native/src/daemon/os/shared/update_stop.rs` — gesamter gemeinsamer Stopvertrag.
- `native/src/daemon/os/shared/ipc_protocol.rs` — Auszüge zu Requesttypen, Token und Pong.
- `native/src/support_dirs.rs` — Zeilen 1–42, nach ausdrücklicher Erweiterung der Lesefläche.
- `/root/.codex/skills/.system/openai-docs/SKILL.md` — einschlägige OpenAI-Dokumentationsanweisung.

`native/src/cli/mcp/private_fs.rs` und der Ordner `native/src/support_dirs/` wurden unter ausdrücklich erweitertem Leserecht angefragt, existieren aber nicht. Sie sind keine gelesenen Dateien. Die aktuellen Arbeitsweiseanweisungen kamen über das MCP und sind ebenfalls keine Repository-Dateilektüre. Die externen gelesenen Primärquellen sind Q1–Q11. `plugins-reference` und `plugin-marketplaces` von Claude wurden zusätzlich als Hersteller-Navigation zur erfolgreich erreichten Loading-Referenz gelesen. Fehlgeschlagene Direktabrufe dieser Referenz wurden nicht als Quelle verwendet.

**Frühere Gesamtplanlektüre, soweit im erhaltenen Verlauf belegt:** Die folgende Einzelliste rekonstruiert nur belegte Dateiaufrufe und Quelltexttreffer. Vor der Kontextkompaktierung übergebene Toolausgaben waren teilweise gekürzt; deshalb ist dies keine Behauptung eines lückenlosen nachträglich ausgelesenen Hostjournals. Verzeichnis-/Dateinamenslisten sind nicht als vollständige Quelltextlektüre gezählt. Vom Hauptagenten übernommene Iroh-/Noq-/Dokany-/Relaybefunde bleiben dessen Lektüre und sind nicht nachträglich als eigene Reads verbucht.

- `/root/.codex/skills/arbeitsweise/SKILL.md`.
- `/root/.agents/skills/arbeitsweise/SKILL.md`.
- `/root/.codex/skills/graphify/SKILL.md`.
- `/root/.codex/skills/graphify/references/query.md`.
- `docs/ARCHITEKTUR.md` — Architekturlektüre; ursprüngliche kombinierte Ausgabe teilweise gekürzt.
- `docs/RICHTUNG.md`.
- `docs/TODO.md` — gezielte Treffer.
- `docs/RELEASING.md` — Zeilen 450–500.
- `docs/oberflaechen/INDEX.md`.
- `docs/plaene/2026-10-24-batches/batches.md` — vorherige Fassung und eigene Planprüfung.
- `docs/plaene/2026-10-24-batches/recherche.md` — ursprünglicher und ergänzter Quellenstand.
- `docs/plaene/2026-10-24-batches/uebergabe.md` — damalige Übergabe; im jetzigen Nachtrag nicht erneut gelesen.
- `docs/plaene/2026-10-22-fernzugriff-desktop/machbarkeit.md` — Auszüge bis Zeile 600.
- `docs/plaene/2026-10-22-laufwerke-medien-agenten/machbarkeit.md` — Auszüge bis Zeile 860.
- `graphify-out/.vocab.txt` — gezielte Treffer.
- `graphify-out/graph.json` — bestehender Graph über Query, kein Neuaufbau.
- `native/Cargo.toml` — gezielte Versions-/Abhängigkeitstreffer.
- `native/Cargo.lock` — gezielte Iroh-/Noq-/Abhängigkeitstreffer.
- `native/src/daemon/os/shared/rooted_backend.rs` — Auszüge bis Zeile 140.
- `native/src/daemon/os/shared/mount_paths.rs` — Zeilen 180–255.
- `native/src/vault/os/shared/backend.rs` — Zeilen 250–305.
- `native/src/share/core/access_grant.rs` — Auszug bis Zeile 180; kombinierte Ausgabe teilweise gekürzt.
- `native/src/share/core/session.rs` — Zeilen 300–365.
- `native/src/mount/core/types.rs` — gezielte Typ-/Signaturtreffer.
- `native/src/mount/core/spool.rs` — gezielte Treffer.
- `native/src/mount/core/journal.rs` — gezielte Treffer.
- `native/src/mount/os/windows/callbacks_metadata.rs` — gezielte Treffer.
- `native/src/mount/os/linux_os/fuse_ops_meta.rs` — gezielte Treffer.
- `native/src/mount/core/journal_record.rs` — Zeilen 1–100.
- `native/src/cli/mcp/update_stop.rs` — Stopadapter.
- `native/src/cli/plugin/files.rs` — Zeilen 1–130.
- `native/src/transfer/os/shared/engine/folders.rs` — Zeilen 1–120; Ausgabe teilweise gekürzt.
- `native/src/cli/engine_copy.rs` — Zeilen 1–100.
- `native/src/app/core/read_access.rs` — Zeilen 1–150.
- `native/src/app/core/analytics_access.rs` — Zeilen 100–170.
- `native/src/scanner/os/collect.rs` — Zeilen 1–90.
- `native/src/cloud/core/token_persistence.rs` — Zeilen 1–130.
- `native/src/cloud/core/cloud.rs` — Zeilen 150–265.
- `native/src/screen/core/window_list.rs` — Zeilen 1–150.
- `native/src/screen/core/pixel_convert.rs` — Zeilen 1–115.
- `native/src/screen/core/pacing.rs` — Zeilen 1–105.
- `native/src/share/core/discovery_signal_types.rs` — Zeilen 145–180.
- `native/src/desktop/os/shared/open_target.rs` — Zeilen 120–205.
- `native/src/app/core/share_pages_ui.rs` — Zeilen 1–70.
- `native/src/gdrive/core/backend.rs` — Zeilen 305–335.
- `native/src/app/core/theme.rs` — Zeilen 60–95.
- `native/src/app/core/settings_updates.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/share_diagnostics_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/share_exec_log_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/share_screen_codec_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_actions_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/share_window_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_check_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_compare_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_compare_detail_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/picker_impl.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_compare_list_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_unlock_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_add_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_sync_editor_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/vault_create_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/dialogs.rs` — Dialog-/Scrolltreffer.
- `native/src/app/core/connect_dialog_ui.rs` — Dialog-/Scrolltreffer.
- `native/src/app/os/shared/sync_exit_gate.rs` — Dialog-/Scrolltreffer.

**Geänderte Dateien:** Im ursprünglichen delegierten Planauftrag ausschließlich `docs/plaene/2026-10-24-batches/batches.md`; im jetzigen Nachtrag ausschließlich die neu angelegte Datei `docs/plaene/2026-10-24-batches/agentenvertrag.md`. Zwischenzeitliche Änderungen des Hauptagenten an Gesamtplan, Recherche, Kritik, TODO und Übergabe sind nicht als eigene Änderungen gezählt.

**Modellnachweis:** Der Hauptagent hat für diesen Planungsagenten ausdrücklich die Startkonfiguration `gpt-6-astra` mit `max` bestätigt. Der hier verfügbare Antwort-/Toolverlauf enthält keine unabhängigen Antwortmodell-Metadaten. Der belegte Startparameter wird deshalb nicht als gesondert nachgewiesene Modellkennung jeder Antwort ausgegeben. Es wurden in diesem Nachtrag keine Umsetzungsagenten ausgewählt oder gestartet.
