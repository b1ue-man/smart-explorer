# B3/P13: kritischer statischer Integrationsreview

Stand: 2026-10-25. Auftrag: unabhängiger Review der MCP-/CLI-Laufzeitgenerationen und ihrer Installations-, Update-, Profil- und Daemonanschlüsse. Ausgangsbaseline `b27d0549`; während der Prüfung bewegter gemeinsamer Worktree, zuletzt gelesener HEAD `a1b44770`. Die Fundstellen beziehen sich auf die unten benannten Funktionen; Zeilennummern können durch parallel integrierte Korrekturen wandern.

Die Prüfung fand sieben konkrete Defektgruppen. Drei davon wurden während des Reviews im Produktionscode durch den zuständigen Implementierungsagenten korrigiert und anschließend punktuell erneut gelesen. Diese Quellnachprüfung ist kein Laufzeitnachweis. Für die vier verbleibenden Gruppen lag bei Niederschrift noch keine vollständig nachgeprüfte Korrektur vor. Es wurden keine Builds, Compiler, Tests, Server, Installationen, Commits, Pushes oder Releasebefehle gestartet. Nur diese Reviewdatei wurde durch den Reviewagenten angelegt.

## Prüfauftrag und Methode

Maßstab waren insbesondere D1–D4 und D6 aus [agentenvertrag.md](agentenvertrag.md), die P13-Ergebnisse aus [vertraege.md](vertraege.md), der Batchzuschnitt und die betroffenen bestehenden Installations-/CLI-Wege. Vor dem Befund wurde jeweils der ausführende Besitzer gesucht: ein Manifestfeld, ein Statuslabel oder ein `Arc`-Typ allein gilt nicht als Beleg für eine tatsächliche Sperr- oder Lebensdauerwirkung.

Die erste Prüfplanung ordnete die verbundenen Grenzen in fünf Prüfschritte: Start/SHA/Identität; Prepare/Drain/Stop/Retain/Commit/Recovery/GC; Kindprozesse und daemonseitige Exec-/Discovery-/Mountbesitzer; Profile und Clientübernahme; Installer/Feed/Release. Die zweite Recherche prüfte die konkreten Linux-Prozessnamen nach Austausch, Windows-Persistenzflags und die TOML-Implementierung gegen ihre Primärquellen. Die erforderlichen späteren Verhaltenssignale gehören ausschließlich in die vom Hauptagenten verantwortete gemeinsame Remote-Task-Suite; dieser Review eröffnet keine eigene Testkampagne.

## Priorisierte Befunde

### B3-R1 — P1: Mountstart verliert Operationsbesitz vor Ende seines Backendstreams

**Stand:** offen im zuletzt gelesenen Produktionscode.

**Fundstellen:** `native/src/daemon/os/shared/mount_manager_state.rs::clear_host_runtime` (233–240), `finish_host_exit` (30–49), `finish_clean` (116–124); `mount_manager_host.rs::BackendStreamLease` (20–28), `take_backend` (158–188), `backend_stream_finished`; `ipc.rs::MountHostBackend` (203–210); `mount_manager_start_cache.rs::insert` (42–76).

**Auslöser:** Ein generationsgebundener Mountstart hat seinen Backendstream geöffnet, aber noch kein authentifiziertes `Mounted`/`MountedAt` gemeldet. Der Hostprozess endet, während `serve_backend` noch nicht zurückgekehrt ist, beispielsweise während einer ausstehenden Remoteantwort. Die bestehende Buchhaltung führt diesen Zustand ausdrücklich mit `backend_stream_active = true` weiter.

**Tatsächlicher Kontrollfluss und Wirkung:** Der Startretainer liegt allein in `MountEntry.startup_lease`. Jeder beobachtete Hostexit ruft `clear_host_runtime` auf und setzt dieses Feld auf `None`. Gleichzeitig erhält `clear_backend_runtime` einen aktiven Backendstream ausdrücklich. Dessen `BackendStreamLease` enthält ausschließlich `MountManager` und Mount-ID; er hält keine Referenz auf den Operationsretainer. Die installationsweite Operationssperre kann deshalb schon frei werden, obwohl der tatsächliche Backendbesitzer noch arbeitet. Drain kann einen Stop freigeben, der diesen noch offenen Aufruf unterbricht. Das ist eine echte Besitzlücke, keine bloß ungenaue Statusanzeige. Ein bereits erfolgreich abgeschlossener Mountstart darf seinen Startretainer regulär abgeben; der Befund fordert keinen lebenslangen Operationslock für jedes gemountete Laufwerk.

**Primärbeleg:** Die genannten Besitzer-/Dropstellen und D4 des vereinbarten Vertrags. Die Rust-Primärdokumentation beschreibt die Freigabe des inneren Werts beim letzten starken `Arc`-Besitzer; ein bloßer Managerclone erhält einen bereits aus dessen Eintrag entfernten Retainer nicht. [Rust `Arc`, abgerufene Dokumentationsversion 1.99.1](https://doc.rust-lang.org/stable/std/sync/struct.Arc.html).

**Nötige Korrektur:** Die noch laufende Startoperation bis zum wirklichen Ende des aktiven Backendstreams behalten, auch wenn der Host zuvor stirbt. Die Übergänge `Mounted`, Hostexit ohne Stream und Ende des letzten Backendbesitzers müssen dabei getrennt bleiben. Main wurde mit den konkreten Besitzstellen informiert.

### B3-R2 — P1: Historische Runtime-/Starterhashes werden automatisch weiter freigegeben

**Stand:** offen im zuletzt gelesenen Produktionscode.

**Fundstellen:** `native/src/mcp_launcher/os/shared/compatibility.rs::compatible_runtimes`; `core/manifest.rs::implements_contract_pair`; `native/src/updater/os/shared/terminal.rs::McpTerminalUpdate::prepare` (144–147); `native/src/bin/smart_explorer_updater/mcp.rs::McpUpdate::prepare` (55–58); `mcp_launcher/os/shared/provision.rs::stage_generation` (29–34).

**Auslöser:** Eine Installation besitzt eine alte aktuelle Runtime oder einen historischen Eintrag in `supported_runtimes`. Der eingehende Build verwendet weiterhin dieselben beiden Vertragskennungen. Oder ein alter Starter muss unabhängig von seiner Protokollhauptversion zurückgenommen werden.

**Tatsächlicher Kontrollfluss und Wirkung:** Der Selektor erzeugt seine Kandidaten ausschließlich aus dem lokalen bisherigen Manifest. Gleiche fest codierte IPC-/Speicherkennungen und ein erneut passender Datei-Hash reichen für die neue Freigabe. Ein eingehender, releasegebundener Katalog ausdrücklich freigegebener Alt-Hashes wird nicht konsumiert. Beide Updateaufrufer übernehmen zusätzlich sämtliche bisherigen `allowed_launcher_hashes`; die Provisionierung fügt den neuen Hash hinzu. Damit kann der normale Updateweg weder eine einzelne schlechte Alt-Runtime mit unverändertem Vertrag noch einen einzelnen alten Starter abwählen. Lokal historisch entstandene Freigaben werden transitiv weitergetragen. Der Review behauptet ausdrücklich nicht, bereits eine konkrete semantisch inkompatible Altdatei ausgeführt oder gefunden zu haben.

**Primärbeleg:** D3 verlangt die konkrete Liste geprüfter Alt-Hashes im eingehenden Release; D1 verlangt einen unabhängigen Starterentzug. `GenerationInput.supported_runtimes` kommentiert selbst, dass ausschließlich Hashes mit tatsächlichem Kompatibilitätsnachweis hineingehören. Die gelesenen Feed-/Staging-/Manifest-/Releaseanschlüsse liefern diesen Selektor nicht. Eine erfolgreiche Typprüfung oder die bloße Gleichheit der Kennungen ersetzt den vereinbarten Hashnachweis nicht. [Vereinbarter Vertrag D1/D3](agentenvertrag.md).

**Nötige Korrektur:** Eine versionsgebundene Freigabe-/Widerrufspolitik aus dem eingehenden Kandidaten anwenden und mit lokal vorhandenen, erneut verifizierten Generationen schneiden. Ungeprüfte Hashes nicht automatisch zulassen. Ein konkreter Starterhash muss unabhängig von der Protokollnummer entziehbar sein; erforderliche Profilmigration muss diesen Entzug begleiten.

### B3-R3 — P1: Linux-Selbstaustausch ließ den lebenden Updatebesitzer verwaist erscheinen

**Stand:** ursprünglicher Defekt statisch bestätigt; eingegangene Korrektur punktuell nachgelesen, nicht ausgeführt.

**Fundstellen des Vorbefunds:** `native/src/updater/os/shared/terminal.rs::replace_cli`; `cli_swap.rs::install_cli_in_place`; `native/src/mcp_launcher/os/shared/transaction.rs::prepare_internal` und `retained_update_transaction`; `installation.rs::owner_alive`; `os/linux_os/mod.rs::process_identity`/`process_matches`; `native/src/cli/mcp/update_stop.rs`.

**Auslöser:** Die installierte Linux-CLI führt `se update` für eine Nur-Terminal-Installation aus. Prepare speichert ihre Prozessidentität, anschließend ersetzt der vorhandene CLI-Swap den installierten Namen durch die neue Datei. Die alte ausführende Datei hat in diesem Pfad nur eine kopierte Sicherung; ihr ursprünglicher Inode verliert den Namen.

**Tatsächlicher Kontrollfluss und Wirkung vor Korrektur:** Der gespeicherte Imagepfad lautet vor dem Austausch `.../se`; `/proc/<pid>/exe` liefert danach `.../se (deleted)`. Der frühere vollständige Vergleich von `ProcessIdentity` lieferte bei erfolgreich lesbarem Prozess deshalb `false`, obwohl PID, Startzeit und gehaltene Eigentümersperre gleich blieben. Der Startzeitfallback griff nur bei einem Lesefehler. Während des Fensters zwischen Austausch und Commit verweigerte `owner_alive` die Retain-Ausnahme; der sekündliche MCP-Stoppoll konnte eine eigentlich kompatible Idle-Sitzung schließen. Der Defekt verlangt kein Prozess-PID-Recycling und keinen Angreifer.

**Primärbeleg:** Linux man-pages 6.19 dokumentiert den Zusatz ` (deleted)` bei entferntem Namen und das Öffnen des tatsächlichen Executables über `/proc/pid/exe`; `rename(2)` beschreibt den Ersatz des bestehenden Zielnamens und den Fortbestand offener Deskriptoren. [proc_pid_exe(5)](https://man7.org/linux/man-pages/man5/proc_pid_exe.5.html), [rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html).

**Nachbefund:** `ProcessIdentity` enthält nun `ExecutableObject { device, inode }`. Linux öffnet `/proc/<pid>/exe`, liest dessen Objektidentität und kontrolliert die Startzeit erneut. `process_matches` vergleicht PID, Startzeit und Objektidentität statt des veränderlichen Imagepfads. Dies schließt den belegten erfolgreichen-Lesepfad statisch. Die bestehende Ausweichentscheidung bei nicht lesbarem Executable sowie das plattformübergreifende Prozessverhalten wurden nicht ausgeführt; daraus wird kein bestandener Laufzeitnachweis abgeleitet.

### B3-R4 — P1: Windows-Publikation behauptete eine von tempfile nicht gesetzte Dauerhaftigkeitsoption

**Stand:** ursprünglicher Defekt statisch bestätigt; eingegangene OS-Korrektur punktuell nachgelesen, nicht ausgeführt.

**Fundstellen des Vorbefunds:** `native/src/mcp_launcher/os/shared/installation.rs::atomic_bytes`; `os/windows/mod.rs::sync_directory`; `os/shared/provision.rs::create_once`/`copy_immutable`; `native/Cargo.lock` (`tempfile` 3.27.0).

**Auslöser und Wirkung:** Nach einer scheinbar erfolgreichen Veröffentlichung von Prepare, Stopidentität oder Commit fällt das Betriebssystem beziehungsweise die Stromversorgung aus. Der Vorzustand spülte die Stagedatei, verwendete danach `NamedTempFile::persist` und behandelte Windows-Verzeichnissynchronisation als erfolgreich, weil ein Kommentar `MOVEFILE_WRITE_THROUGH` in tempfile voraussetzte. Die exakt gepinnte Implementierung setzt jedoch ausschließlich `0` oder `MOVEFILE_REPLACE_EXISTING`. Damit fehlte der behauptete Nachweis einer dauerhaften Namensveröffentlichung; die Recoveryentscheidung konnte auf einem nach Neustart fehlenden oder alten Zustandsnamen beruhen. Dies ist ein aus API und Code abgeleitetes Crashrisiko, kein beobachteter Stromausfallversuch.

**Primärbeleg:** Vollständig gelesene relevante Windows-Persistenzimplementierung `tempfile-3.27.0/src/file/imp/windows.rs`, insbesondere Zeilen 92–109. Die Bibliotheksdokumentation grenzt `persist` von Datei-/Verzeichnissynchronisation ab; Microsoft dokumentiert `MOVEFILE_WRITE_THROUGH` separat. [tempfile 3.27.0 `persist`](https://docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html#method.persist), [Microsoft `MoveFileExW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).

**Nachbefund:** `atomic_bytes`, `create_once` und `copy_immutable` delegieren nun an `os::publish_staged`. Das neue `os/windows/persistence.rs` spült und schließt den Dateihandle und ruft selbst `MoveFileExW` mit `MOVEFILE_WRITE_THROUGH` sowie bei Ersatz `MOVEFILE_REPLACE_EXISTING` auf. Verzeichnispublikation, Retirement und GC-Quarantäne verwenden denselben expliziten Movepfad. Linux behält Persistenz plus Parent-fsync. Die falsche tempfile-Annahme ist damit im gelesenen Pfad entfernt. Keine Aussage über tatsächlich getestete Stromausfallbeständigkeit oder Dateisystem-/Treiberverhalten.

### B3-R5 — P2: Erfolgreicher Updateabschluss aktualisierte das ursprüngliche Profil nicht zuverlässig

**Stand:** ursprünglicher Defekt statisch bestätigt; eingegangener Postcommit-Handoff punktuell nachgelesen. Erhöhter Installationskontext bleibt ausdrücklich verzögert.

**Fundstellen des Vorbefunds:** `native/src/cli/update.rs::install_terminal_only` (187–208), `complete_install`; `native/src/cli/plugin.rs::refresh_generated_files`; `plugin/files.rs::refresh`/`executable`; `native/src/bin/smart_explorer_updater.rs::apply_update`; `smart_explorer_updater/installer.rs::verify_completion`.

**Auslöser und Wirkung vor Korrektur:** Der Linux-Selbstaustausch rief den Refresh nach Commit im alten `se update`-Prozess auf. Dieser ermittelte die Installation erneut über `current_exe` und kanonischen Pfad. Bei `.../se (deleted)` scheiterte dies; `files::refresh` verwarf den Fehler und kehrte zurück. Im Desktop-/NoGUI-Helperpfad fehlte ein Postcommit-Refresh vollständig. Die verwaltete Einbindung blieb bis zu einem späteren MCP-Start oder einem manuellen Verwaltungsaufruf auf ihrem vorherigen Profilstand. Main bestätigte zusätzlich mit einer eigenen repositoryweiten Callersuche, dass kein separater Desktopstartup-Refresh vorhanden war; diese breite Suche ist übernommener Main-Beleg, keine eigene Erweiterung der Lesefläche.

**Primärbeleg:** D2/P13a.2 verlangt den Refresh nach erfolgreichem Update im ursprünglichen Nutzerkontext. Für den Linux-Pfad gilt derselbe belegte `/proc/pid/exe`-Mechanismus wie B3-R3. Hostneustart und Konfigurationsübernahme bleiben getrennte Vorgänge; eine spätere Serverinitialisierung ersetzt diesen fehlenden Profilaufruf nicht. [Vertrag D2](agentenvertrag.md), [proc_pid_exe(5)](https://man7.org/linux/man-pages/man5/proc_pid_exe.5.html), [offizielle Codex-MCP-Konfiguration](https://learn.chatgpt.com/docs/extend/mcp).

**Nachbefund:** Das neue `mcp_launcher/os/shared/generator.rs` erfasst den tatsächlichen laufenden Imagehash am Eintritt, bindet schreibende Generatoren an die bestätigte aktuelle Installation und startet nach Commit deren neu gehashte CLI mit `plugin refresh`. Der normale Helperabschluss und der erfolgreiche Recovery-Frühausstieg rufen nun `installer::refresh_after_commit` auf. `refresh_generated_files` delegiert aus dem ersetzten CLI-Prozess anhand der zuvor erfassten Installationswurzel. Fehler werden nun zurückgegeben beziehungsweise protokolliert. Der erhöhte Installer startet diesen Profilprozess bewusst nicht; er protokolliert Aufschub bis zum nächsten MCP-Start des ursprünglichen Nutzers. Eine sofortige Profilübernahme im ursprünglichen Nutzerkontext bei erhöhtem Installieren ist damit weiterhin nicht belegt. Es wurde kein Clientreload ausgeführt oder behauptet.

### B3-R6 — P2: Gewöhnlicher Refresh migriert auch weiterhin zulässige Starterpfade

**Stand:** offen im zuletzt gelesenen Produktionscode.

**Fundstellen:** `native/src/cli/plugin/files.rs::executable`, `refresh`, `write_claude`; `plugin/codex.rs::configure` (61); `mcp_launcher/os/shared/provision.rs::stage_generation`; `mcp_launcher/core/capability.rs::LauncherProbe::current`.

**Auslöser:** Ein bereits migriertes Profil verweist auf Starterhash A. Ein gewöhnlicher Produktrelease bringt Starterhash B mit; A bleibt in `allowed_launcher_hashes`. Der Starter enthält seine tatsächliche Produktversion, daher kann auch ein Runtimeupdate ohne geänderten Startervertrag neue Starterbytes liefern.

**Tatsächlicher Kontrollfluss und Wirkung:** `files::executable` wählt immer den neuesten Manifeststarter. `configure` ersetzt daraufhin den Codex-`command`; `write_claude` ersetzt dessen `.mcp.json`-`command`. Es fehlt eine Entscheidung, den vorhandenen nachweislich eigenen und weiterhin zulässigen Starter zu behalten. So wird die Codex-Tabelle bei gewöhnlichen Updates erneut geschrieben, obwohl der alte Einstieg die neue Runtime bereits aus dem Manifest auswählen könnte. Der zusätzliche Konfigurationsschreib-/Konfliktbereich widerspricht dem ausdrücklich zugesagten migrationsfreien normalen Runtimeupdate.

**Primärbeleg:** D1 und D2 legen den dauerhaft registrierten Einstieg sowie das Ausbleiben einer Codex-Tabellenänderung bei gewöhnlichen Runtimeupdates fest. Die offizielle Codex-Dokumentation bestätigt `command`/`args` als ausgeführte stdio-Konfiguration, bietet aber keine hier nutzbare automatische Mehrschreibertransaktion. [Vertrag D1/D2](agentenvertrag.md), [Codex MCP](https://learn.chatgpt.com/docs/extend/mcp).

**Nötige Korrektur:** Den bisherigen registrierten Starter nach Installations-ID, Hashpfad, SHA und aktueller Zulassung prüfen und beibehalten. Auf den neuen Starter nur bei Legacyübergang, expliziter Startermigration oder Widerruf umstellen. Neue Registrierungen dürfen den aktuellen Starter wählen.

### B3-R7 — P2: Claude-Refresh übernimmt Benutzerfelder in den Löschfingerprint

**Stand:** offen im zuletzt gelesenen Produktionscode.

**Fundstellen:** `native/src/cli/plugin/files.rs::write_claude` (28–34), `verify_owned` (54–62), `remove_generated` (65–78); `plugin/managed.rs::write_set` (Prüfung `own_fields_match`, anschließend `hash(after)`); `plugin/claude.rs::uninstall` (64–82).

**Auslöser:** Der Nutzer ergänzt in der verwalteten Claude-`.mcp.json` ein `env`-Feld, einen Toolfilter oder einen zusätzlichen `mcpServers`-Eintrag, lässt Smart Explorers `command`/`args` unverändert und führt danach einen Refresh sowie später `se plugin uninstall claude` aus.

**Tatsächlicher Kontrollfluss und Wirkung:** `write_claude` erhält die Ergänzung. `write_set` akzeptiert die geänderte Datei über den Vergleich der eigenen Befehlsfelder, speichert danach jedoch den Hash der gesamten resultierenden Datei als eigenen Fingerprint. `verify_owned` und `remove_generated` interpretieren diesen Gesamtfingerprint später als vollständigen Dateibesitz und löschen die gesamte Datei. Die zuvor erhaltenen Benutzerfelder oder der zusätzliche Server werden dadurch mitgelöscht. Ein unkoordinierter Editor im exakten Renamefenster ist für diesen Verlust nicht erforderlich.

**Primärbeleg:** Die vollständig gelesene Folge `write_claude` → `write_set` → `verify_owned` → `remove_generated` und D2, das Fingerprints tatsächlich eigener Felder sowie den Erhalt von Umgebungsvariablen und Filtern verlangt. Die allgemeine Projektvorgabe verlangt außerdem den Erhalt bestehender Nutzerrechte und Konfiguration. [Vertrag D2](agentenvertrag.md).

**Nötige Korrektur:** Feldbesitz und Gesamtdateiinhalt getrennt speichern. Uninstall darf nur das eigene Delta entfernen oder muss bei verbleibenden Nutzerfeldern die Datei mit konkretem Konflikt erhalten. Ein erfolgreicher Refresh darf die Eigentumsgrenze nicht auf mitgelesene Nutzerinhalte ausweiten.

## Verworfene Vermutungen und bewusste Entscheidungen

- Die zunächst gemeldete pauschale Beanstandung des wartenden installierten `se mcp`-Elternprozesses wurde zurückgenommen. D2/Migration 3 erlaubt ausdrücklich, dass dieser einmalige Windows-Elternprozess den Ersatz noch blockiert. Daraus folgt kein Auftrag, eine gemappte EXE umzubenennen oder den Prozess ungeprüft auszunehmen. Main und Implementierungsagent erhielten die Rücknahme. Die zuletzt gelesene Quelle gibt diesen Migrationszustand zusätzlich auf stderr aus.
- Beim Linux-Profilrefresh wurde die anfängliche Vermutung „alter Generator schreibt alte Vorlagen“ präzisiert: Der belegte konkrete Selbstaustauschpfad scheiterte bereits beim Kanonisieren des entfernten Executablenamens und kehrte still zurück. Die alte Generatorgrenze war trotzdem falsch und wurde durch den neuen Handoff ersetzt.
- Eine mögliche TOML-Inline-Table-Regressionsvermutung wurde nicht als Defekt übernommen. `toml_edit` 0.25.11+spec-1.1.0 verarbeitet `Item::Table` innerhalb einer InlineTable in `append_values`/`append_all_values`. Die zunächst vermutete Auslassung beim Serialisieren war durch die tatsächliche Bibliotheksimplementierung widerlegt. Es wurde kein Parserprogramm ausgeführt.
- Der Exec-Retainer wird im gelesenen `spawn_connected_retained` im äußeren Tokio-Worker gehalten; Discovery besitzt einen eigenen Lease-State für pending/active Exchangebesitzer. Das sind konkrete Besitzanschlüsse. Daraus folgt keine allgemeine Freigabe aller Exec-/Discovery-Abbruchabläufe. Der Routeagent verändert die Aufbau-/Fristlogik parallel; seine letzten Änderungen nach der Quelllektüre sind nicht pauschal mitgeprüft.
- SHA-Prüfung, Windows-Handlebindung, Linux-fexecve, private Lebenszykluskanäle, installationsweite Operationslocks, Legacyretirement und GC sind im Code tatsächlich angeschlossen. Diese positive Strukturbeobachtung schließt die oben beschriebenen Fehler nicht aus und ist kein Funktionsnachweis.

## Quellenstand und verbleibende Prüfgrenzen

Alle externen Abrufe in diesem Review erfolgten am 2026-10-25. Versionsgebunden gelesen wurden `tempfile` 3.27.0 und `toml_edit` 0.25.11+spec-1.1.0 aus der lokalen Registry passend zu Cargo.lock; Linux-Manpages weisen Version 6.19 aus. Cargo.lock nennt libc 0.2.186; die betriebssystemseitigen Semantikaussagen stammen aus den Linux-Primärmanpages, nicht aus einem ausgeführten libc-Aufruf. Die Windows-Bindings und Win32-Dokumentation wurden als APIgrenze behandelt, nicht als Nachweis eines bestimmten Windows-Builds. Die abgerufene Rust-Arc-Dokumentation weist 1.99.1 aus; eine tatsächlich eingesetzte Compilerfassung wurde nicht ermittelt. `fs2` war in der gezielten Cargo.lock-Suche nicht vorhanden; die Quellen verwenden native flock-/Windows-Locks beziehungsweise `std::fs::File`-Locks.

Die offiziellen Codex-MCP-Dokumente wurden für Befehlsfelder und Übernahmegrenzen konsultiert. Ein erneuter Abruf der Claude-Seite `plugins/loading` schlug zuletzt fehl; daraus wird kein zusätzlicher aktueller Claude-Clientbeleg gewonnen. Es wurden keine installierte Codex-/Claude-Version, kein Hostcache und keine reale MCP-Verbindung geprüft.

Die vorhandene Graphifyabfrage wurde verwendet. Sie zeigte vorwiegend vorhandene AST-Beziehungen und ersetzt keine Prüfung der neu angelegten Module; ein Graphrebuild wurde nicht ausgeführt. Main meldete erfolgreiche statische Parses ausgewählter Quellen, aber diese Meldung ist weder eine Kompilierung noch eine durch diesen Review ausgeführte Verifikation.

Nicht erfasst sind eine Vollprüfung sämtlicher Credentialformate, der nicht freigegebenen tieferen VFS-Backendimplementierungen, sämtlicher historischen Releasebinarys und sämtlicher Windows-/Linux-Dateisystemvarianten. `native/src/daemon/os/shared/ipc_storage.rs` aus dem Brief existierte am abgefragten Pfad nicht; Main wurde um den konkreten Ersatzpfad gebeten. Die vorhandenen privaten Speicheradapter wurden gelesen, ihre nicht freigegebenen `local_access`-Implementierungen nicht. Der Releasepfad wurde an Launcher-/Hash-/Manifest-/Installer-/Assetanschlüssen geprüft; es wurde keine existierende Releaseveröffentlichung oder installierbare Artefaktdatei als Ergebnis dieses Batches bestätigt.

Die Versionierung der Freigabeliste, tatsächliche Retention über einen laufenden Updatewechsel, der Mountfehlerablauf, Windows-Crashpersistenz, Firstinstall/Legacyretirement/Recovery unter Fehlern und Clientkonfigurationsübernahme bleiben ohne ausgeführten Nachweis. Diese Grenzen gehören in den Ergebnisbericht der gemeinsamen Remote-Suite; aus ihrer Nichtausführung wird kein Erfolg abgeleitet.

## Tatsächlich gelesene Fläche

Die folgende Liste bezeichnet vollständige Modultexte oder gezielt gelesene zusammenhängende Funktionsabschnitte/Suchtreffer. Sie behauptet keine Volllektüre jeder genannten großen Datei. Pfadgruppen mit geschweiften Klammern enumerieren ausschließlich die aufgeführten Dateien.

- Anweisungen und Plan: `AGENTS.md` im bereitgestellten Nutzerkontext; `docs/ARCHITEKTUR.md`; `docs/AI_AGENT_PLUGIN.md`; Plugin-/Install-/Update-/Releaseabschnitte von `README.md` und `docs/RELEASING.md`; `docs/plaene/2026-10-24-batches/{batches.md,vertraege.md,agentenvertrag.md,recherche.md,umsetzung.md}`; `native/Cargo.toml`, `native/Cargo.lock`.
- Launcher: `native/src/mcp_launcher/{mod.rs,core/mod.rs,core/manifest.rs,core/lifecycle.rs,core/capability.rs,os/mod.rs,os/android.rs,os/linux_os/mod.rs,os/linux_os/spawn.rs,os/windows/mod.rs,os/windows/security.rs,os/windows/spawn.rs,os/windows/environment.rs,os/windows/persistence.rs,os/shared/installation.rs,os/shared/admission.rs,os/shared/transaction.rs,os/shared/provision.rs,os/shared/launcher.rs,os/shared/child.rs,os/shared/compatibility.rs,os/shared/contract_probe.rs,os/shared/installer_cli.rs,os/shared/generator.rs}`.
- CLI/Integration: `native/src/bin/{se.rs,se_mcp_launcher.rs,smart_explorer_updater.rs}`; `native/src/cli/{mcp.rs,plugin.rs,update.rs}`; `native/src/cli/plugin/{managed.rs,codex.rs,files.rs,config.rs,claude.rs,host_cli.rs,assets/SKILL.md}`; `native/src/cli/mcp/{server.rs,session.rs,update_stop.rs,cli_tool.rs,jobs.rs,shell_registry.rs,file_write.rs,transfer_tools.rs,status_tools.rs,catalog.rs}`; `native/src/app/core/settings_ai_plugin.rs`.
- Updater: `native/src/updater/{core/types.rs,os/linux_os.rs,os/windows.rs,os/shared/terminal.rs,os/shared/cli_swap.rs,os/shared/feed_launcher.rs,os/shared/staging.rs,os/shared/apply.rs,os/shared/archive.rs,os/shared/flow.rs}`; `native/src/bin/smart_explorer_updater/{mcp.rs,process.rs,installer.rs,args.rs,replace.rs,replace_platform.rs,path_safety.rs}`. Aufgezählte Dateien wurden an ihren oben beschriebenen Grenzen gelesen; nicht aufgezählte Hilfsmodule wurden nicht stillschweigend als vollständig geprüft behandelt.
- Daemon: `native/src/daemon/os/shared/{runtime_hello.rs,ipc.rs,ipc_client.rs,ipc_host.rs,ipc_host_ext.rs,ipc_host_service.rs,ipc_protocol.rs,ipc_protocol_vault.rs,ipc_protocol_screen.rs,update_stop.rs,exec_ipc.rs,mount_manager.rs,mount_manager_start.rs,mount_manager_start_cache.rs,mount_manager_state.rs,mount_manager_host.rs,mount_host_process.rs,discovery_pairing_ipc.rs}`.
- Tatsächliche asynchrone Besitzer: `native/src/share/core/{exec_client.rs,exec_client_active.rs,exec_session.rs,service.rs,node.rs,discovery_exchange_owner.rs,discovery_signal_commands.rs,discovery_signal_state.rs,discovery_signal_types.rs,discovery_signal_maintenance.rs,discovery_signal_dispatch.rs}`.
- Speichergrenze: `native/src/support_dirs.rs`, `native/src/creds/mod.rs`, `native/src/creds/os/{private_storage_windows.rs,private_storage_unix.rs}`.
- Distribution, gezielte Launcher-/Artefakt-/Hash-/Manifestabschnitte: `native/{publish-update.ps1,publish-linux-feed-wsl.sh,publish-release-local.ps1,release-publication.ps1,installer.nsi}`, `install-linux.sh`, `.github/workflows/build.yml`, `.gitignore`.
- Bibliotheksquellen: lokale Registry `tempfile-3.27.0/src/file/imp/windows.rs`; `toml_edit-0.25.11+spec-1.1.0/src/{index.rs,encode.rs,inline_table.rs}`. Externe APIquellen zusätzlich: Microsoft `CreateFileW`, `CreateProcessW`, `MoveFileExW`; Linux `fexecve(3)`, `flock(2)`, `proc_pid_exe(5)`, `rename(2)`; Rust `Arc`; tempfile `NamedTempFile::persist`; offizielle Codex-MCP-Konfiguration.

**Änderungsfläche dieses Reviewagents:** ausschließlich diese Datei. **Entscheidung:** B3 mit den benannten offenen Defekten an Main zurückgegeben; keine Implementierung, keine neue Testkampagne, keine Publikationsentscheidung durch den Reviewer.
