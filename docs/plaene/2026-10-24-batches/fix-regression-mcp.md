# MCP-Lebenszyklus: RA-06, RA-08, RSOL-06, RSOL-07 und RSOL-08

Stand: 2026-10-26. Autorauftrag in der isolierten Arbeitskopie
`/tmp/smartexplorer-review-fixes-ee75e91b`, Ausgang `ee75e91b`.
Dieser Bericht enthält keine Release- oder Laufzeitfreigabe.

## Auftrag und Erstplan aus dem delegierten Ergebnisauftrag

Der Nutzer verlangt denselben Fixautor für Funktionsauflistung, direkte und
indirekte Kettenkarte, Fixplan, Umsetzung und anschließende Anti-Regressionsreview.
Der zugewiesene Umfang sind die vier ursprünglichen Fehler und der von Main
als deckungsgleich ergänzte RSOL-08. Main besitzt Integration,
die einzige spätere Remote-Suite, Git und den terminalen Release. Produktive
Profile, Claude-/Codex-Hostaufrufe und lokale Builds/Tests bleiben unberührt.

Geplante Ergebnisse: unveränderte v0.5.184-Profile direkt und nach Migration
deinstallieren; eigene XDG-/DBus-Werte übernehmen und fremde Zusätze schützen;
Plugininstallation und generische Konfiguration nach historischem Nur-Terminal-
Update erhalten; eigene MCP-Programme bei Windows-Deinstallation gezielt entfernen.
Offene Bestandsfragen waren Eigentumsmarkerformat, historische Updatebestätigung,
Generationsbesitzer und die reguläre NSIS-Deinstallationskette. Eine nur vom
Nutzer beantwortbare Angabe ist für diesen Umfang nicht erforderlich.

## Bestandsbefund und Funktionsauflistung

| Befund | Fehlerhafte Funktion und konkrete Folge |
|---|---|
| RA-06 | `plugin/codex.rs::configure` übernimmt beim Legacy-Übergang vorhandenes `env`, zeichnet aber `{}` als `managed_env` auf. `uninstall` verweigert dadurch eigene unveränderte XDG-/DBus-Werte. |
| RSOL-07 | `plugin/files.rs::verify_owned` verlangt Fingerprints und verweigert den veröffentlichten Marker `{version, executable}`. Beide Hostdeinstallationen erreichen ihre bisherige Entfernung nicht. |
| RSOL-06 | Der historische `se update` tauscht ausschließlich die CLI. `update::complete_install` bestätigt Version/Worker, erzeugt kein Manifest. `plugin::run` verlangt für Installation/Konfiguration dennoch zwingend einen Launcher. |
| RA-08 / RSOL-08 | `installer.nsi` entfernt Hauptprogramme, aber weder `mcp/runtimes/<hash>/se.exe` noch Launcher, Manifest oder Identität. Ein zurückgelassener Launcher kann weiter starten. |

Belege: die vier Abschnitte der beiden Regressionreviews im unveränderten
Main-Kandidaten; zusätzlich `git show a814f263:native/src/cli/plugin/{files,codex}.rs`
für tatsächlich veröffentlichte Marker, Skill und die drei `--env`-Schlüssel.
Graphabfrage nach `legacy_matches`, `verify_owned`, `complete_install` und MCP-
Generation am 2026-10-26: vorhandener AST-Graph, anschließend gezielte Quelllektüre.
Die Graphify-CLI meldete Skill 0.9.11 gegenüber Paket 0.9.17; keine Graphänderung.

## Direkte und indirekte Kettenkarte

1. CLI `plugin install/uninstall/status/config/refresh` → `plugin.rs::run` →
   Generatorzulassung (`generator::capture_entry/identity`,
   `admission::generator_operation`) → Launcherauflösung
   (`files::executable`, `Installation::current/for_image/checked_launcher_path`).
   Desktop benutzt dieselbe installierte CLI; MCP-Start nutzt
   `refresh_generated_files` → `refresh_entry_installation` beziehungsweise
   `refresh_current_profile` → authentifizierte Wartungsübergabe an die installierte
   CLI. Diese gemeinsamen Einstiege dürfen keine alte Generatorversion zulassen.
2. Claude-Installation → `write_claude` → `managed::write_set/replace` →
   Marketplace, Pluginmetadaten, MCP-JSON, Skill, Eigentumsmarker → Host-CLI
   Marketplace/Plugin hinzufügen. Deinstallation → `verify_owned` →
   `marketplace_is_ours` → Hostabmeldung → `remove_generated`. Fremde MCP-Server,
   zusätzliche Felder, geänderte eigene Bytes und fremde Marketplacequelle bleiben
   Sperrgründe. Hostbefehle laufen ausschließlich über `host_cli`.
3. Codex-Installation/Refresh → `configure` → Skillpreflight → Integrationslock →
   `document/fields/owner_matches/legacy_matches` → ausschließlich Command/Args
   ändern → `managed_env` und `entry_sha256` persistieren → Skill schreiben →
   optionale Hoststatusabfrage. Schreibfehler benutzen `restore_owned`; gleichzeitige
   Editorbytes dürfen nicht zurückgerollt werden. Deinstallation → Skillpreflight →
   dieselbe Registrierungsidentität → Schutz zusätzlicher Schlüssel und Envwerte →
   nur eigenen Server entfernen → eigene Sidecardatei und bekannten Skill entfernen.
   Kommentar, Tabellenreihenfolge und unbeteiligte Server bleiben `toml_edit`-Bestand.
4. Historisches `se update` → `replace_cli/install_cli_in_place` (alte Binärdatei)
   → neues `update --complete-install VERSION` → Workerübergabe → alter
   `ReplacedCli::commit` oder Rollback. Neuer Terminalupdater → Feedpayloads →
   `McpTerminalUpdate::prepare` → Generation/Initiallease oder Updatetransaktion →
   CLI-Swap → Abschlussbestätigung → Manifestcommit → Profilrefresh. Desktop/
   NSIS → verifizierter Bundlehelper `mcp::prepare/commit/rollback` → dieselbe
   Provisionierungs-/Transaktionsgrenze. Die lokale Legacyregistrierung darf keinen unbekannten Manifestcommit in
   den alten Rollbackvertrag einschleusen und keinen neuen Feedabruf verlangen.
5. Provisionierung → `stage_generation` → Root-/ACL-/Linkprüfung → SHA-256-
   geprüfte unveränderliche Runtime-/Launcherkopie → native Vertragsprobe →
   `InitialGenerationLease`/`UpdateTransaction` → `current.v1.json`. Launcher →
   Kontrolllock/Zustandszulassung → Runtimelebenszeit → hashgebundener Spawn →
   Runtimezulassung → Operationslock/Daemon-Hello. Updateowner benötigt Lock und
   tatsächliche Prozessidentität; Lebenszeitlocks gehören jedem Laufzeitbesitzer.
   Deinstallation muss diese Grenzen respektieren und vor Hauptdateilöschung
   scheitern, wenn aktive Besitzer oder ein offener Recoveryzustand vorliegen.
6. NSIS-Deinstallation → bisher Desktopremove, Shellunregister, Uplinkcleanup,
   PATH-Entfernung, optionaler Kontodienst, bekannte Programm-/Lizenzdateien,
   Shortcuts und Uninstallregistrierung. Die neue MCP-Entfernung muss davor
   ausführbar sein; unbekannte Installationsdateien und Benutzerprofile dürfen
   keine rekursive Löschfreigabe erhalten.

## Zweistufiger Plan und zweiter Recherchegang vor Änderung

Erstplan nach Bestand: vorhandene Legacy-Skillprüfung für direkte Entfernung
wiederverwenden; Envbesitz beim nachgewiesenen Übergang erfassen; historischen
Terminalbestand an einem rollbackverträglichen Zeitpunkt vervollständigen;
NSIS um gezielte, sperrengeschützte MCP-Entfernung ergänzen.

| Meilenstein | Betroffene Grenze | Erwartetes Ergebnis und Regressionserhalt |
|---|---|---|
| M1 | Pluginmarker und beide Deinstallationen | Exakt bekannte Legacydateien sind ohne vorherige Installation entfernbar; veränderte Dateien und zusätzliche MCP-Felder werden erhalten. |
| M2 | Codex-Registrierung/Migration | Die drei historisch gesetzten Envschlüssel werden aus dem belegten Eintrag übernommen; unbekannte Schlüssel und Benutzeroptionen sperren die vollständige Entfernung. Neue Ownerrecords und Compare-before-replace-Rollback bleiben gültig. |
| M3 | Lokaler Terminalbestand und Pluginbefehl | Install/Refresh/Config verwenden ohne Managed-Manifest den lokal gegen das am Prozessstart erfasste Image geprüften absoluten CLI-Pfad. Kein Feedabruf und kein Manifestcommit während alter Updatebestätigung. Ein späterer regulärer Managed-Updateweg migriert dieselben Fingerprintprofile auf den Launcher; beschädigter Managed-Zustand ist keine Legacyfreigabe. |
| M4 | MCP-Installation und NSIS | Nur eigene hashgeprüfte Programme und geprüfte Lifecycledateien werden entfernt; aktive Besitzer, offene Recoveryzustände und unprüfbare Inhalte verhindern Verlust und erlauben Wiederholung. Keine rekursive Entfernung fremder Nutzerdaten. |
| M5 | isolierte deklarierte Fixtures und Autorreview | Genau benannte Fälle werden für Mains gemeinsame Remote-Suite vorbereitet; keine lokale Ausführung. Die gesamte oben ermittelte Kette wird nach dem Fix durch denselben Autor erneut abgeglichen. |

Zweiter Recherchegang: Der veröffentlichte Codexinstaller übergab ausschließlich
`XDG_DATA_HOME`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, jeweils nichtleer,
auf Nicht-Windows. Der Marker speicherte keine damaligen Envwerte. Die Zuordnung
beim Legacyübergang kann deshalb nur bekannte erzeugte Schlüssel anhand des
unveränderten Skill-/Commandnachweises übernehmen; nachträglich vom Benutzer auf
denselben historischen Schlüssel geschriebene Werte sind ohne früheren Beleg
nicht von Altbestandswerten unterscheidbar. Zusätzliche Schlüssel bleiben fremd.

Primärquellen, Abruf 2026-10-26: [NSIS RMDir](https://nsis.sourceforge.io/Reference/RMDir)
bestätigt reine Leerordnerentfernung ohne `/r` und warnt vor pauschalem `/r $INSTDIR`;
[LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex)
bestätigt unabhängige Handlelocks und Fail-immediately;
[MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
bestätigt, dass eine Verzeichnisverschiebung auf demselben Volume bleibt;
[Rust remove_dir_all](https://doc.rust-lang.org/std/fs/fn.remove_dir_all.html)
bestätigt die rekursive Wirkung. Daher selektive Dateientfernung, keine rekursive
Freigabe unbekannter Inhalte und keine Besitzentscheidung allein anhand einer PID.

## Entscheidungen und Umsetzung

M1: `verify_marker_owned` und `legacy_owned_files` akzeptieren ausschließlich den
alten Zweifeldmarker und die exakt bekannten Releaseinhalte. Claude benötigt die
vollständige veröffentlichte Vierdateimenge; Codex den veröffentlichten Skill.
Die Entfernung erhält unbekannte Dateien und erlaubt nur die fünf fest benannten
verwalteten relativen Dateipfade: `SKILL.md`, `.claude-plugin/marketplace.json`,
`smart-explorer/.claude-plugin/plugin.json`, `smart-explorer/.mcp.json` und
`smart-explorer/skills/smart-explorer/SKILL.md`. Ein Zusatzobjekt mit Löschpfaden am Legacy-
Marker wird abgewiesen. Neue Fingerprintmarker behalten ihren bisherigen Schutz
gegen geänderte Skill-/Metadatenbytes und fremde MCP-Felder.

M2: `legacy_entry_matches` bindet Codex-Command/Args an den alten Marker und den
bekannten Release-Skill. Die Migration erfasst die vorhandenen Werte der drei
historischen Unixschlüssel als `managed_env`, bevor der Skillmarker ersetzt wird.
Windows übernimmt diese Unixwerte ausdrücklich nicht, da der veröffentlichte
Windowsinstaller sie nicht gesetzt hat. Uninstall unterstützt denselben Legacy-
Nachweis direkt und benötigt keine vorherige Migration. Unbekannte Envschlüssel,
Toolfilter, geänderte eigene Befehlsfelder, entfernte oder geänderte aufgezeichnete
Envwerte verhindern die vollständige Registrierungslöschung. TOML-Kommentare,
unbeteiligte Tabellen und `restore_owned` bleiben erhalten. Ein ungültiger neuer
Ownerrecord wird vor Änderungen als Konflikt behandelt. Ohne Codex auf PATH
bleibt ein vorhandener alter Skillmarker stehen, damit ein späterer regulärer
Installationsversuch seinen Registrierungsnachweis weiterhin benutzen kann.

M3: Die erste Lösungsüberlegung, eine Generation aus dem Feed nachzuladen, wurde
vor Abschluss verworfen: Main belegte die Offlinegrenze des bisherigen lokalen
Konfigurationsbefehls. Die finale Umsetzung benutzt `legacy_entry_path` und
`verified_legacy_entry` ausschließlich mit lokalen Dateien. Das aktuelle Image
wird beim CLI-Einstieg erfasst und der installierte Dateipfad vor Verwendung
gegen diesen SHA-256 geprüft. `marker` schreibt im Legacyfall `mode: legacy_cli`
mit Fingerprints und ohne erfundene Installations-ID; Codex zeichnet dabei
`installation_id: null` auf. `installation_for_registration` prüft beim späteren
Managed-Übergang den zugelassenen Launcher. Legacyprofil → Managedprofil ist
über denselben Fingerprintpfad möglich. Ein bereits ersetzter alter Generator,
eine laufende Entfernung und vorhandene verwaiste Managed-Identität/-Historie
geben keinen Legacy-Schreibweg frei. Ein Legacy-Start von `se mcp` behält seinen
automatischen lokalen Profilrefresh; Managed-Starts benutzen unverändert den
authentifizierten Wartungshandoff. `update.rs::complete_install`, CLI-Swap,
Updatecommit und Rollback behalten ihren bisherigen Ablauf. Die schmale neue
Fassade `LegacyIntegrationGuard` in `updater/os/shared/terminal.rs` hält eine
rein lesende Imagelease vom erneuten Imagecheck bis zum Ende ihrer
Vorlagenerzeugung. Der Linux-Terminalupdater beachtet diese Lease zusätzlich
zu seinem unveränderten Namespaceausschluss. Unter Windows verhindert der
Lesehandle Schreib-/Löschfreigaben; dort existiert kein Terminal-Selbstaustausch.
Historische Lockdateien werden vom Pluginweg nur gelesen. So bleiben nur
lesbare Installationsverzeichnisse ohne Elevation nutzbar. Die vollständige
Folgeplanung und eigene Review stehen im letzten Abschnitt. Config erhält
keinen Lock und keinen Schreibzugriff.

M4: NSIS ruft vor Hauptdateilöschung `se plugin retire-installation` auf. Der
Einstieg leitet den Stamm ausschließlich aus dem tatsächlich laufenden
installierten `se` ab und prüft dessen Image gegen die bestätigte Runtime. Er
prüft die tatsächlichen Images dieser Installation statt Prozesse nach Basename
zu beenden. Aktive Programme, ein Updateowner, Operationslock, Runtimelebenszeit
oder Launcherprozess verhindern die Entfernung ohne Kill und ohne Profilzugriff.
Der Nutzer schließt diese Besitzer regulär und wiederholt die Deinstallation.
Ein offener Update-/Retirement-Recoveryzustand bleibt erhalten.

`inventory` erfasst ausschließlich SHA-256-geprüfte Runtime-/Launcherprogramme,
bekannte Metadaten, die eigene Installationsidentität, die festen leeren Locks,
eigene validierte Manifeste und den exakten Stopmarker. Alte Hashgenerationen
und eigene GC-Quarantänen gehören dazu. Die durch die äußere Installerlektüre
belegte `install-last-applied.txt` wird nur im bekannten reinen Versionsformat
entfernt; zweckentfremdete Inhalte bleiben erhalten. `uninstall.v1.json` speichert die genaue
Menge mit ursprünglichen Hashes vor der ersten Löschung. CLI-Zulassung,
Manifestauflösung, Generator, Initialprovisionierung und Updatetransaktion
verweigern währenddessen neue Starts/Arbeit. Nach einem Teilfehler prüft die
Wiederholung dieselben Originalhashes; neue Bytes werden erhalten. Fehlende
Locks werden nur im aufgezeichneten Endzustand ohne verbliebene Programme
akzeptiert. Es gibt kein `remove_dir_all` und kein rekursives NSIS-RMDir.
Unbekannte Dateien, Ordner und Kindlinks bleiben an ihren Orten; nur vollständig
leere erzeugte Ordner werden entfernt. NSIS prüft anschließend auch die
Hauptprogrammlöschung und bewahrt bei deren Fehlern den Wiederholungsweg und
den Deinstallationseintrag. Desktopremove, Shellunregister, Uplink, PATH,
Kontodienst, Autostart, Lizenzen und Shortcuts behalten ihre bestehenden Aufrufe.

Im unmittelbar betroffenen Claude-Fehlerpfad bleiben die Eigentumsdateien jetzt
erhalten, wenn ein Host-Deinstallationsbefehl fehlschlägt; es wird kein Erfolg
mit anschließend verlorenem Wiederholungsnachweis gemeldet.
Selektive Entfernung darf leere Profilordner und fremde Notizen zurücklassen.
`generated_files_absent` unterscheidet deshalb das Fehlen aller sechs bekannten
Bestandteile (Marker und die oben benannten fünf Dateipfade) von einem
vorhandenen, aber unprüfbaren Profil. Eine erneute Deinstallation und „alle“ bei
nur einem installierten Host bleiben ohne Zugriffe auf unbewiesene Hosteinträge
möglich. Ungültige oder noch vorhandene bekannte Dateien bleiben Konflikte.

## Eigene anschließende Anti-Regressionsreview

Prüfgegenstand war nach Fertigstellung die vollständige oben beschriebene
Funktionskette; Methode: Diff gegen `ee75e91b`, Abgleich der veröffentlichten
Marker-/Envbytes gegen `a814f263`, Verfolgung der tatsächlichen Aufrufer und
Persistenzverträge sowie statischer Rust-AST-Parse. Ein Compiler oder Produktcode
wurde dabei nicht ausgeführt.

- Direkte Legacyentfernung: beide Hostdeinstallationen erreichen dieselbe
  bekannte-Dateien-Prüfung. Weder ein neuer Launcher noch ein Hostaufruf zum
  Migrieren ist Voraussetzung. Zusätzliche MCP-Felder und veränderte bekannte
  Dateien bleiben geschützt.
- Migration: Envbesitz wird vor dem Skillmarkerwechsel gelesen; die spätere
  Entfernung prüft den aufgezeichneten Wert statt aktuelle Prozessumgebung.
  Windows besitzt keinen historischen Unixenv-Freibrief. Die Prüfung hat
  außerdem den fehlenden-Host-Fall korrigiert, der sonst den Legacybeleg
  vorzeitig ersetzt hätte.
- Offlinekonfiguration und Installation: die Aufrufkette bis
  `config::render`, `write_claude` und `codex::configure` benötigt ohne Manifest
  keinen Feed und keine zusätzliche Datei aus dem Netz. Der ausgewählte CLI-Pfad
  bleibt absolut; die fünf Formate Claude, Codex, Generic, VS Code und OpenCode behalten ihre bestehenden
  Command-/Argsformate. Hostinstallation selbst benötigt weiterhin den
  tatsächlichen Host und dessen bisherige Registrierungsmittel.
- Generatoridentität: ein vorhandenes Managed-Manifest verwendet weiterhin den
  vorherigen Generator-/Operationslock. Der Legacyfall prüft den am Start
  erfassten Imagehash; ein zwischenzeitlich erschienener Managed-Zustand erlaubt
  keine nachträgliche Behauptung, der CLI-Pfad sei ein Launcher. Die bekannten
  Fehler-/Rollbackpfade bleiben ohne automatische Mutationswiederholung.
- Deinstallation: aktuelle Generation, alte Generationen und Launcher benötigen
  alle verfügbare Besitz-/Hashbelege. Die Runtime-Lebenszeit und die tatsächliche
  Prozessprüfung sind unabhängige Voraussetzungen. Neue Starts sehen den
  dauerhaften Entfernenzustand auch dann, wenn das Currentmanifest bereits
  gelöscht ist. Ein neuer CLI-Start darf dadurch nicht in einen Legacyserver
  zurückfallen. Benutzerprofile und Sitzungsspooldaten werden nicht geöffnet.
- Wiederholung und fremde Daten: die Entfernung ist dateiselektiv; geänderte
  Bytes und unbekannte Kinder bleiben bestehen. Ein fehlender Zeiger nach
  Teilentfernung ist durch den validierten Entfernungsrecord wiederaufnehmbar;
  bloße fremde Reste nach erfolgreichem MCP-Cleanup blockieren einen späteren
  NSIS-Wiederholungsversuch nicht. Eine fehlende PID autorisiert keine Entfernung
  von MCP-Programmen. Die Legacy-CLI-Lockdatei behält ihren bereits vorhandenen
  PID-basierten Wiederaufnahmevertrag; sie ersetzt keinen Runtimebesitzbeleg.

Bei der statischen AST-Prüfung fiel eine fehlende schließende Funktionsklammer
im neu ergänzten Linuxadapter auf. Sie wurde vor dieser Abschlussreview
korrigiert. Die abschließenden statischen Prüfergebnisse stehen im Abschnitt
„Abschlussbelege und echte Grenzen“.

## Deklarierte Fälle für Mains gemeinsame Remote-Suite

Alle neuen Fälle tragen den bereits im gemeinsamen Manifest registrierten
Präfix `batches_p13_`; das gemeinsame Manifest und die CI-Orchestrierung wurden
nicht bearbeitet. Die beiden neuen Testdateien sind im jeweiligen Produktions-
modul per `#[cfg(test)]` eingebunden. Kein Fall wurde lokal ausgeführt.
Die Plattformmenge bezeichnet die unterstützten Desktopziele Linux und Windows
für Mains Remote-Suite; Android wird für keinen dieser Fälle als geprüftes
MCP-Produktziel behauptet. Der Quellfilter nennt zusätzlich den tatsächlichen
OS-Filter am Fall. Ohne eigenen Filter bedeutet die Plattformmenge deshalb
keine behauptete Sperre der Rust-Deklaration auf diese beiden Targets.

| Fallname | Plattformmenge | Quellfilter | Aussage |
|---|---|---|---|
| `batches_p13_legacy_release_profiles_uninstall_directly_and_keep_unowned_files` | Linux und Windows | kein eigener OS-Filter | Exakte veröffentlichte Claude-/Codex-Dateien direkt entfernen; fremde Notizen behalten. |
| `batches_p13_legacy_uninstall_preserves_modified_skill_and_shared_mcp_fields` | Linux und Windows | kein eigener OS-Filter | Geänderte Skills, Envzusatz, zusätzlicher Server und geänderte Command/Args bleiben erhalten. |
| `batches_p13_legacy_marker_cannot_add_deletion_paths` | Linux und Windows | kein eigener OS-Filter | Zusätzliche Löschpfade in einem Legacy-Marker autorisieren keine Entfernung. |
| `batches_p13_legacy_codex_environment_is_owned_before_and_after_migration` | Linux | `not(windows)` | Linux: alle drei früher gesetzten Werte vor und nach Commandmigration entfernbar, unbeteiligter Server und Kommentare bleiben. |
| `batches_p13_legacy_windows_codex_does_not_adopt_user_unix_environment` | Windows | `windows` | Windows: manuell gesetztes XDG gehört dem Nutzer. |
| `batches_p13_codex_uninstall_retains_user_env_filters_and_changed_owned_values` | Linux und Windows | kein eigener OS-Filter | Fremde Env-/Filterfelder, fehlender Nachweis und geänderte eigene Felder verhindern Entfernung. |
| `batches_p13_missing_codex_host_keeps_legacy_registration_evidence` | Linux und Windows | kein eigener OS-Filter | Ohne Codex-CLI bleiben alter Skill und Marker als späterer Registrierungsnachweis erhalten. |
| `batches_p13_historical_terminal_entry_works_offline_and_rejects_stale_image` | Linux und Windows | kein eigener OS-Filter | Kein Manifest/Feed/Launcher nötig; ersetzte lokale Bytes werden abgewiesen. |
| `batches_p13_historical_terminal_entry_does_not_downgrade_existing_managed_state` | Linux und Windows | kein eigener OS-Filter | Verwaiste Identität, Current-, Legacy- und Updaterecords erlauben keinen Legacyrückfall. |
| `batches_p13_historical_generic_config_uses_only_the_local_cli` | Linux und Windows | kein eigener OS-Filter | Generische JSON-Ausgabe enthält den vorhandenen absoluten CLI-Pfad und `mcp`. |
| `batches_p13_legacy_profile_generation_serializes_with_terminal_updates` | Linux | `target_os="linux"` | Linux: unabhängige Shared-Reader verhindern den aktuellen CLI-Swap bis zum letzten Reader; der aktive Namespacebesitzer sperrt umgekehrt die Profile. |
| `batches_p13_legacy_profiles_need_no_installation_writes` | Linux und Windows | `any(target_os="linux", windows)` | Linux/Windows: readonly CLI und readonly Ordner, zwei Reader, identische Bytes und ausschließlich der ursprüngliche Dateieintrag; keine Sidecarerzeugung. |
| `batches_p13_legacy_profiles_read_historical_update_state_without_mutation` | Linux und Windows | `any(target_os="linux", windows)` | Linux/Windows: aktiver, ungültiger und Null-PID-Lock werden ohne Änderung abgewiesen; ein nach dem Öffnen erschienener aktiver Lock sperrt die erneute Zulassung. |
| `batches_p13_legacy_profiles_preserve_stale_update_lock_in_readonly_installation` | Linux | `target_os="linux"` | Linux: verwaiste PID-Datei bleibt im readonly Ordner erhalten; erst der spätere Updater bereinigt sie nach seinem bisherigen Vertrag. |
| `batches_p13_legacy_profiles_reject_same_byte_image_replacement` | Linux | `target_os="linux"` | Linux: ein gleichbyteiger unkooperativer Rename wird anhand des Dateiobjekts erkannt; ein neuer Reader kann anschließend die neue Datei öffnen. |
| `batches_p13_legacy_windows_profile_pins_cli_without_sidecar` | Windows | `windows` | Windows: die rein lesende Lease verhindert Write-Open und Rename ohne neue Lockdatei und gibt den Pfad nach ihrem Ende frei. |
| `batches_p13_profile_image_lease_preserves_cli_swap_commit_and_rollback` | Linux | `target_os="linux"` | Linux: Imagekonflikt hinterlässt unveränderte Bytes und keinen Namespace-Lock; Swap, Hashbestätigung, Rollback, Commit und Hashfehler behalten ihre Wirkung. |
| `batches_p13_uninstall_removes_all_owned_mcp_programs_and_manifest` | Linux und Windows | kein eigener OS-Filter | Aktuelle und supersedierte eigene Runtime sowie Launcher/Metadaten werden entfernt. |
| `batches_p13_uninstall_preserves_foreign_files_and_is_repeatable` | Linux und Windows | kein eigener OS-Filter | Fremde Top-Level-/Generations-/Unterordnerdateien bleiben; späterer NSIS-Retry erkennt nur fremde Reste. |
| `batches_p13_uninstall_respects_independent_owner_operation_and_lifetime_locks` | Linux und Windows | kein eigener OS-Filter | Updateowner, Kontrolle, Arbeit und Runtimelebenszeit verhindern Entfernung unabhängig voneinander. |
| `batches_p13_uninstall_rejects_changed_program_metadata_and_owner_before_deletion` | Linux und Windows | kein eigener OS-Filter | Geänderte Programme, Identität und Launchermetadaten bleiben vor erster Löschung erhalten. |
| `batches_p13_uninstall_preserves_open_update_recovery_and_blocks_new_admission` | Linux und Windows | kein eigener OS-Filter | Offene Recovery bleibt; Entfernungszustand sperrt Admission und Legacyrückfall. |
| `batches_p13_uninstall_resumes_exact_record_after_partial_deletion_and_keeps_new_bytes` | Linux und Windows | kein eigener OS-Filter | Teilentfernung bleibt gegen Originalhashes wiederholbar; neue Bytes werden erhalten. |
| `batches_p13_uninstall_record_never_authorizes_path_traversal` | Linux und Windows | kein eigener OS-Filter | Ein Entfernungsrecord kann keinen Pfad außerhalb der festen Menge freigeben. |
| `batches_p13_uninstall_missing_lock_never_releases_remaining_program_files` | Linux und Windows | kein eigener OS-Filter | Fehlende Locks erlauben keine Löschung verbleibender Programme; der aufgezeichnete Endzustand ohne Programme bleibt wiederholbar. |
| `batches_p13_uninstall_oversized_inventory_is_rejected_before_publication` | Linux und Windows | kein eigener OS-Filter | Das vorhandene Recordbudget wird vor Zustandsveröffentlichung und Dateientfernung geprüft. |
| `batches_p13_uninstall_does_not_follow_foreign_child_links` | Linux | `target_os="linux"` | Linux-Kindlink bleibt erhalten und sein fremdes Ziel unverändert. |

Main muss diese Fälle in der einen bereits vorgesehenen Remote-Ausführung
auswerten. Die drei bestehenden Codexfelder-/Rollbackfälle und die bisherigen
Managedprofil-/Lifecyclefälle bleiben für ihre direkt berührten Ketten gültig.
Ein echter Windows-Installerlauf mit offenen Prozessen, ein echter Claude-/Codex-
Host und produktive Profile wurden nicht ausgeführt und werden nicht als bestanden
ausgegeben. Die vorbereiteten Fälle prüfen reale temporäre Dateien und Locks,
starten aber keine Runtime und keine Host-CLI.

## Gelesene und geänderte Dateien

Jede Datei der nachfolgenden eigenen Änderungsmenge wurde durch denselben Autor
gelesen beziehungsweise nach ihrer Erstellung statisch geprüft. Es gab keine
Unteragenten und keinen zweiten Fixautor für einen Teil dieses Zyklus.

Eigene Änderungen:

- `docs/AI_AGENT_PLUGIN.md`.
- `docs/plaene/2026-10-24-batches/fix-regression-mcp.md` — neu.
- `native/installer.nsi`.
- `native/src/cli/plugin.rs`.
- `native/src/cli/plugin/batches_codex_task_tests.rs`.
- `native/src/cli/plugin/batches_legacy_lifecycle_task_tests.rs` — neu.
- `native/src/cli/plugin/claude.rs`.
- `native/src/cli/plugin/codex.rs`.
- `native/src/cli/plugin/config.rs`.
- `native/src/cli/plugin/files.rs`.
- `native/src/mcp_launcher/mod.rs`.
- `native/src/mcp_launcher/os/android.rs`.
- `native/src/mcp_launcher/os/linux_os/mod.rs`.
- `native/src/mcp_launcher/os/windows/mod.rs`.
- `native/src/mcp_launcher/os/shared/admission.rs`.
- `native/src/mcp_launcher/os/shared/batches_lifecycle_task_tests.rs`.
- `native/src/mcp_launcher/os/shared/batches_uninstall_task_tests.rs` — neu.
- `native/src/mcp_launcher/os/shared/generator.rs`.
- `native/src/mcp_launcher/os/shared/installation.rs`.
- `native/src/mcp_launcher/os/shared/installer_cli.rs`.
- `native/src/mcp_launcher/os/shared/installer_cli_removal.rs` — neu im Strukturfolgefix.
- `native/src/mcp_launcher/os/shared/installer_cli_inventory.rs` — neu im Strukturfolgefix.
- `native/src/mcp_launcher/os/shared/provision.rs`.
- `native/src/mcp_launcher/os/shared/transaction.rs`.
- `native/src/updater/mod.rs` — ausschließlich interner Guard-Re-Export.
- `native/src/updater/os/shared/terminal.rs` — lesende Guard-Fassade und deklarierte Lease-/Readonlyfälle.
- `native/src/updater/os/shared/cli_swap.rs` — zusätzliche Imagelease, rein lesende Zustandsprüfung und deklarierter Swap-/Rollbackfall.

Zusätzliche unveränderte Lektüre; „Auszug“ bezeichnet keine vollständige
semantische Dateireview:

- `AGENTS.md` und `/root/.codex/AGENTS.md` — Projekt-/Nutzervorgaben, zusätzlich vollständig im Auftrag übergeben.
- `/tmp/smartexplorer-fix-mcp-lifecycle-scope.json` und `/tmp/smartexplorer-final-review-user-contract.txt`.
- `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.agents/skills/arbeitsweise/SKILL.md` und `/root/.codex/skills/graphify/SKILL.md`.
- `docs/ARCHITEKTUR.md` — Architektureinstiege und MCP-/Pluginzuordnung.
- `docs/plaene/2026-10-24-batches/batches.md` — P13/D3 und gezielte Vertragstreffer.
- `docs/plaene/2026-10-24-batches/vertraege.md`, `umsetzung.md` und `regressionsreview.md` — thematische Auszüge/Suchen im selben Planordner.
- `docs/plaene/2026-10-24-batches/agentenvertrag.md` — D1 bis D4 und Oberflächengrenzen.
- `docs/oberflaechen/INDEX.md` — gezielte P13-/MCP-Suche.
- `README.md` — MCP-/Offline-/Deinstallationsabschnitt, durch Main geändert.
- `docs/RELEASING.md` — entsprechende kanonische Abschnitte, durch Main geändert.
- `install-linux.sh` — frei konfigurierbare Installations-/Binverzeichnisse, CLI-only-Einstieg, verifizierte Payload-/Helferaufrufe sowie Modus 755; dieselben Funktionen aus `a814f263` über Git gelesen.
- `graphify-out/graph.json` — abgefragter vorhandener AST-Teilgraph, kein Neuaufbau.
- `native/Cargo.toml` — Abhängigkeits-/Featuretreffer; keine eigene Änderung.
- `native/review-task-manifest.json` — bestehende P13-Präfixregistrierung, keine eigene Änderung.
- `native/src/bin/se.rs` — Zulassung vor internem Modus und CLI-Dispatch.
- `native/src/bin/smart_explorer_updater.rs` — Hashgrenzen, Prepare/Commit/Rollback, Refresh und Bookkeeping, Auszüge.
- `native/src/bin/smart_explorer_updater/mcp.rs`.
- `native/src/bin/smart_explorer_updater/installer.rs` — Bundleargs und Refresh nach Commit, Auszüge.
- `native/src/bin/smart_explorer_updater/process.rs` — bestehende PID-/Installationspfadprüfung und reguläres Warten vor dem Austausch, Auszüge.
- `native/src/bin/smart_explorer_updater/bookkeeping.rs` — tatsächliches Installerstatusformat.
- `native/src/bin/smart_explorer_updater/replace.rs` — gezielte Commit-/MCP-Suche ohne zusätzlichen MCP-Aufruferbefund.
- `native/src/app/core/settings_ai_plugin.rs` — eigene vollständige Lektüre des GUI-Verbrauchers.
- `native/src/cli/mod.rs` — Plugin-/MCP-Dispatch, Auszug.
- `native/src/cli/update.rs` — historische Bestätigung, Versionsvergleich, Workerübergabe, Commit/Rollback, Auszüge; final unverändert.
- `native/src/cli/plugin/managed.rs`.
- `native/src/cli/plugin/host_cli.rs`.
- `native/src/cli/plugin/batches_managed_task_tests.rs`.
- `native/src/cli/plugin/assets/SKILL.md` — aktueller Basisskill und über Git gelesene v0.5.184-Bytes.
- `native/src/cli/mcp.rs`, `native/src/cli/mcp/server.rs` und `native/src/cli/mcp/status_tools.rs` — Start-/Refresh-/Statusaufrufer, Auszüge.
- `native/src/updater/os/shared/feed.rs`, `native/src/updater/os/shared/feed_launcher.rs`, `native/src/updater/os/shared/legacy_launcher.rs` und `native/src/updater/os/shared/feed_files.rs` — verworfene Feedlösung und erhaltene Desktopbrücke; Feed teilweise als Auszug.
- `native/src/updater/os/linux_os.rs` und `native/src/updater/os/windows.rs` — Payloadnamen und OS-Grenzen, Auszüge.
- `native/src/mcp_launcher/core/manifest.rs` und `native/src/mcp_launcher/core/lifecycle.rs`.
- `native/src/mcp_launcher/os/mod.rs` und `native/src/mcp_launcher/os/shared/launcher.rs`.
- `native/src/mcp_launcher/os/shared/child.rs`.
- `native/src/mcp_launcher/os/windows/security.rs` und `native/src/mcp_launcher/os/windows/persistence.rs`.
- `native/src/mcp_launcher/os/windows/spawn.rs` und `native/src/mcp_launcher/os/linux_os/spawn.rs` — Spawn-/Stopgrenze, Auszüge.
- Die beiden Regressionreviews `review-regression-astra.md` und `review-regression-sol.md` unter `/home/CodeProjects/smartexplorer/smart-explorer/docs/plaene/2026-10-24-batches/` — ausschließlich die zugewiesenen Fehlerabschnitte und ihre Gegenprüfungen.

Die tatsächlich verwendeten Prüfwerkzeuge waren Git-Textvergleich, gezielte
`rg`-Suchen, der vorhandene Graphify-Query sowie ein reiner Tree-sitter-Rust-Parse
aus der vorhandenen Graphify-Werkzeugumgebung. Es wurde nichts installiert.
Der angefragte Pfad `native/src/updater/os/mod.rs` existiert nicht; er ist keine
gelesene Quelle. Zusätzliche Zeilenzählungen wurden nicht als semantische
Review ganzer Dateien gewertet.

## Abschlussbelege und echte Grenzen

Der statische Fixzyklus dieses Autors ist abgeschlossen. Der abschließende
AST-Parse der geänderten Rustquellen enthält keine Syntaxfehler; die Quellen
bleiben unter 500 Zeilen und 50 KiB je Datei. `git diff --check` meldet keine
Whitespacefehler in der eigenen Änderungsmenge. Der aus dem v0.5.184-Skill
übernommene Text besitzt statisch exakt den Produktionshash
`6d23270eb3dff8c2a3845e06106c75db3808bee4bd72bdc43638eb9f3bf92193`.
Das ist kein ausgeführter Test und kein Typ-/Compilerbeleg.

Die eigenen Dateien liegen ausschließlich im vereinbarten Arbeitsbaum auf
`codex/review-fixes-ee75e91b`. Main hat währenddessen getrennte Review- und
Dokumentationscommits hinzugefügt; diese fremden Änderungen wurden nicht
bearbeitet oder zurückgesetzt. Der feste Reviewbaum unter
`/home/CodeProjects/smartexplorer/smart-explorer` wurde von diesem Autor nicht
verändert. Es wurden keine Commits, Pushes, Graphupdates, Builds, Compiler,
Formatter, Testausführungen, Paketierungen, Releases oder Host-CLI-Aufrufe
durchgeführt. Keine produktiven Profile wurden installiert, migriert, gestartet
oder entfernt.

Die historische Envgrenze bleibt ausdrücklich: Der Zweifeldmarker speicherte
keine damaligen Werte. Eine vor der ersten Migration auf demselben bereits
historisch erzeugten Schlüssel vorgenommene Benutzeränderung ist ohne früheren
Nachweis nicht vom unveränderten Altbestand unterscheidbar. Die gewählte
Migration übernimmt nur die belegte historische Schlüsselmenge; zusätzliche
Schlüssel bleiben fremd. Nach dem neuen Wertnachweis werden Änderungen an diesen
Werten erkannt. Die vier konkret belegten unveränderten Regressionauslöser
benötigen keine Annahme über eine solche nachträgliche Benutzeränderung.

Ein erfolgreicher echter Hostdeinstallationsablauf und dessen Verhalten nach
Teilerfolgen hängen weiterhin von den tatsächlich eingesetzten Claude-/Codex-
Versionen ab. Diese Hostausführung ist nicht geprüft. Bei einem beobachteten
Claude-Befehlsfehler bleiben die Eigentumsdateien zur Diagnose erhalten.
Unbekannter Prozessstatus, Hash-/ACL-Abweichung und offene Recovery führen bei
der MCP-Programmdeinstallation zum Erhalt der Dateien, nicht zu einer behaupteten
erfolgreichen Entfernung. Die NSIS-Hauptprogramm-/Lizenzlöschung besitzt keine
neue allgemeine Inhaltsfingerprintgarantie; der neue Inhaltsschutz betrifft den
MCP-Baum.

Der Entfernungsrecord verwendet das bestehende 256-KiB-Recordbudget. Ein größeres
Inventar wird vor Veröffentlichung des Records und vor jeder Löschung
abgewiesen; damit entsteht kein Entfernungszustand, den derselbe Leser bei einer
Wiederholung nicht mehr öffnen könnte. Dafür ist der oben genannte isolierte
Ressourcenfall vorbereitet; er wurde nicht ausgeführt.

Außerhalb dieser Fläche verbleiben ausschließlich Mains Integration, die eine
Remote-Suite einschließlich Typ-/Compilerprüfung, der gemeinsame Graphrefresh,
Commit/Push und der terminale Release. README und RELEASING wurden von Main
bereits mit der lokalen Offlinegrenze und dem ausdrücklich auf den MCP-Baum
begrenzten Entfernungsschutz angeglichen und von diesem Autor gelesen. Es gibt
keinen weiteren benötigten Quellzugriff und keine zusätzliche Produktabhängigkeit
für den hier abgeschlossenen statischen Ergebnisauftrag.

## Fortsetzung: nur lesbare Legacy-Installation

Main meldete nach der Integration eine durch denselben Autor verursachte
Rechteregression: `LegacyIntegrationGuard` erzeugte über `UpdateLock::acquire`
eine neue `<se>.update-lock` neben der CLI. Das machte Plugininstallation und
automatischen Profilrefresh von Schreibrechten im Installationsverzeichnis
abhängig. Der vorherige Abschluss gilt für diesen Teil deshalb erst nach der
hier dokumentierten Korrektur erneut.

### Eigene Bestandsprüfung und ergänzte Funktionskette

Der Fall ist unterstützt. `install-linux.sh` erlaubt seit der Releasebaseline
`a814f263` `SMART_EXPLORER_INSTALL_DIR` und `SMART_EXPLORER_BIN_DIR` ohne
Beschränkung auf das Benutzerheim. Der veröffentlichte
`install_executable_atomic` verwendet `install -m 755` und atomisches `mv`.
Eine administrativ erzeugte CLI mit Root-Eigentümer ist dadurch für normale
Nutzer les- und ausführbar, ohne dass diese Nutzer den Installationsordner
beschreiben können. Der aktuelle Installer erhält die frei konfigurierbaren
Pfade und setzt seine verifizierten Payloads ebenfalls auf 755.
Der alte `plugin/files.rs::executable` verlangte nur `current_exe`, keinen
Schreibzugriff auf das Installationsverzeichnis. Die normale Windows-NSIS-
Installation ist benutzerbezogen; README dokumentiert zusätzlich die portable
`se.exe`. Auch deren ursprünglicher Pluginweg fordert keine Installations-
Schreibrechte.

Ergänzte Kette: `plugin install` und Legacy-`se mcp`-Start →
`files::refresh`/`legacy_profile_guard` → `lock_cli_for_integration` → bisher
schreibendes `UpdateLock::acquire`. Die Gegenrichtung ist
`replace_cli` → `UpdateLock::acquire` → Payloadprüfung/Stage →
`install_cli_in_place` → Completion → `ReplacedCli::commit/rollback`.
Der Bundlehelper stoppt beziehungsweise wartet die tatsächlichen installierten
CLI-Prozesse; Windows verweigert bereits den Terminal-Selbstaustausch vor
`UpdateLock`. CLI-Symlinks werden unter Linux vor dieser Grenze kanonisiert.

### Revidierter konkreter Plan vor dem Folgefix

- RO1: Die Integrationslease öffnet ausschließlich das vorhandene CLI-Image
  lesend. Linux hält einen Shared-`flock`; Windows hält den vorhandenen
  Read-share-Pin ohne Write-/Delete-Freigabe. Erwartetes Ergebnis: kein neuer
  Verzeichniseintrag, kein neuer Schreibzugriff und keine Elevation bei nur
  lesbarem Installationsordner.
- RO2: Der Linux-Terminalupdater übernimmt zusätzlich eine exklusive Lease am
  vorhandenen CLI-Inode, während er seinen bisherigen Namespace-UpdateLock
  weiter hält. Profilreader lesen den bestehenden UpdateLock nur. Erwartetes
  Ergebnis: laufende Profilgenerierung verhindert Swap vor jeder Mutation;
  laufender oder historischer Updatebesitzer verhindert neue Profilzulassung.
  Namespace-Lock, Sicherung, Ctrl+C, Commit und Rollback bleiben erhalten.
- RO3: Imagehash und tatsächliche Dateiidentität werden nach der Lease erneut
  gegen den Namenspfad geprüft. Erwartetes Ergebnis: auch ein gleichbyteiger
  Austausch auf einen anderen Inode wird nicht als geschützte alte Lease
  übernommen; ein stales Image schreibt keine Profile.
- RO4: Die bestehenden isolierten Legacy-/Swapfälle um Readonlydatei/-ordner,
  fehlende Sidecarerzeugung, unabhängige Reader, historischen UpdateLock,
  Austausch zwischen Öffnen und Prüfung sowie unveränderten Rollback ergänzen.
  Erwartetes Ergebnis: Mains eine Remote-Suite kann die Gegenrichtungen gemeinsam
  auswerten. Es erfolgt keine lokale Ausführung.

Die zweite Quellenprüfung verwendet die tatsächlichen OS-Verträge:
[flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html) bindet Locks an
offene Dateibeschreibungen und erlaubt sie auf lokalen Dateien unabhängig vom
Öffnungsmodus; NFS besitzt ausdrücklich abweichende Bedingungen.
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
belegt den lesenden Handle-/Share-Vertrag; die bestehende Windows-
`running_image`-Implementierung benutzt diesen Vertrag bereits.
[Rust File](https://doc.rust-lang.org/std/fs/struct.File.html)
beschreibt die zugehörigen Funktionen und die plattformabhängigen Grenzen.
Abrufdatum der erneuten Recherche: 2026-10-26. Die Lösung benutzt deshalb die
vorhandenen OS-Adapter und erfindet keinen plattformneutralen Schreiblock auf
einem nur lesbaren File.

### Tatsächliche Folgeumsetzung und eigene Anti-Regressionsreview

RO1 ist umgesetzt: `LegacyIntegrationGuard` hält ausschließlich das bereits
vorhandene CLI-Image. Sein Linuxadapter öffnet mit `read(true)`, `O_NOFOLLOW`
und `O_CLOEXEC` und nimmt einen nichtblockierenden Shared-`flock`. Windows
öffnet mit `read(true)`, `FILE_SHARE_READ` und Reparse-Prüfung. Kein Adapter
legt eine Datei an oder verändert Rechte. `plugin::run` hält diese Lease über
die vollständige Claude-/Codex-Installation; `files::refresh` hält sie über
beide Profilauffrischungen einschließlich des automatischen Legacy-MCP-Starts.
Die bestehende Host-/Profileigentumsprüfung bleibt hinter dieser Zulassung
unverändert zuständig. Mehrere getrennte Profilreader dürfen gleichzeitig
zugelassen sein; die eigentlichen Profilwrites bleiben durch ihre bisherigen
Profil- und Integrationslocks geordnet. Config liest weiterhin nur lokal.

RO2 ist umgesetzt: `UpdateLock::acquire` erzeugt seinen bisherigen Namespace-
Lock und übernimmt anschließend unter Linux eine exklusive Lease am
vorhandenen CLI-Image. Ein Imagekonflikt gibt den gerade selbst erzeugten
Namespace-Lock über RAII frei. Bis dahin wurde weder `remove_orphans` noch
Feedfetch, Stage, Backup oder Swap ausgeführt. Ein fehlgeschlagenes Schreiben
der PID gibt ebenfalls den eigenen Lock frei, statt einen neuen leeren Lock
zurückzulassen. `ReplacedCli` hält die Namespace- und Imagelease gemeinsam über
Hashbestätigung, Completion und Commit beziehungsweise Rollback. Kopierpfad,
Permissionsübernahme, Pendingdatei, SHA-Prüfungen, Backupprüfung, Ctrl+C-Undo,
Manifestcommit und Manifestrollback wurden dabei nicht umgeschrieben.
Windows übernimmt keine neue exklusive Bytebereichssperre: Sein bestehender
Terminal-Selbstaustausch scheitert bereits vor Lock-/Payloadzugriff; seine
vorhandenen Swaphelper behalten ihren bisherigen Namespacevertrag.

Die gegenläufige Zulassung liest eine historische `.update-lock` vor und nach
dem Öffnen des Images sowie nach dem abschließenden Imagehashcheck. Aktive,
unlesbare und syntaktisch unklare Lockbesitzer verhindern die Generierung.
Ein anhand des bisherigen Linux-PID-Vertrags verwaister Lock wird vom
Profilreader akzeptiert, aber weder geändert noch entfernt. Ausschließlich
der spätere Updater besitzt weiterhin die bestehende Stalelockbereinigung.
Die vorherigen Selfupdatefehler, Versionsbestätigung und Workerübergabe in
`cli/update.rs` sind quellseitig unverändert. Der Bundlehelper behält seine
Installationspfad-/Prozessprüfung und das reguläre Warten vor dem Austausch;
dieser Folgefix ersetzt diesen bestehenden Helfervertrag nicht.

RO3 ist umgesetzt: Nach der lesenden Lease wird der gegen ENTRY erfasste
Imagehash erneut geprüft. `verify_current` vergleicht zusätzlich das gehaltene
Dateiobjekt mit dem benannten Pfad: Linux Device/Inode, Windows Volume/Fileindex.
Die Linux-Namensprüfung verwendet `symlink_metadata`, damit ein nachträglicher
Kindlink kein reguläres Image vortäuscht. Gleichbyteige Ersatzdateien übernehmen
die alte Lease nicht. Diese erneute Prüfung liegt vor der Profilerzeugung.
Vorhandener Managed-Zustand und ein offener Entfernenzustand verhindern
weiterhin einen Legacyrückfall. Die RA-/RSOL-Entfernungs- und Envfixes bleiben
in ihrer vorher dokumentierten Benutzerprofilgrenze.

RO4 ist im Quellbestand deklariert. Die exakten neuen Namen stehen vollständig
in der gemeinsamen Falltabelle dieses Berichts; keiner wurde lokal ausgeführt.
Readonlyfixture bedeutet hier konkret: entfernte Schreibbits unter Linux und
das Readonlyattribut unter Windows, zwei unabhängig geöffnete Reader sowie
eine Verzeichnisliste, die auch während der Leases ausschließlich die
ursprüngliche CLI enthält. Der stale-PID-Fall setzt zusätzlich Ordner, Image
und vorhandene Lockdatei readonly und gibt die Rechte vor dem anschließenden
Updaterwiederholungsfall kontrolliert zurück. Die Fixture verändert ausschließlich
ihre eigenen temporären Pfade. Ein Windows-Readonlyattribut ist kein
NTFS-ACL-Deny; ein realer administrativer Besitzerwechsel, ein ACL-Deny und
ein Prozesslauf mit fremder UID wurden hier nicht ausgeführt. Die Quellen-
prüfung zeigt für diese Benutzerkonstellationen ausschließlich lesende
Dateiöffnungen ohne Verzeichnismutation und ohne Elevationsaufruf. Sie ersetzt
keinen ausgeführten Hostnachweis.

Die eigene Folgeprüfung hat die Aufrufer, Rückgabepfade, Leasedauer und
Gegenrichtung bis zum ursprünglichen Commit/Rollback erneut gelesen. Der
statische Tree-sitter-Parse der freigegebenen Rustfläche enthält nach dem
Folgefix keine Syntaxfehler, und die neuen beziehungsweise wesentlich
geänderten Rustdateien bleiben unter 500 Zeilen und 50 KiB. Die abschließende
Whitespaceprüfung der eigenen getrackten Änderungen ist sauber. Auch die
zusätzlich neu angelegten Dateien enthalten keine nachgestellten Leerzeichen;
alle neu deklarierten P13-Funktionen stehen in diesem Bericht. Der historische
Skillhash stimmt weiterhin mit den Releasebytes überein, und `cli/update.rs`
ist unverändert. Ein Laufzeitpass wird daraus nicht abgeleitet.

### Präzise Folgegrenzen

`flock` ist unter Linux kooperativ. Der aktuelle Terminalupdater beachtet die
neue Imagelease. Bereits veröffentlichte Updater werden anhand ihrer
vorhandenen Namespace-Lockdatei erkannt. Eine beliebige manuelle Root-
Umbenennung oder eine alte Programmkopie, die erst nach der letzten
Zulassungsprüfung ohne Imagelease tauscht, lässt sich durch einen rein
lesenden Pluginprozess nicht universell verhindern. Ein bereits geschehener
Austausch wird vor Zulassung zusätzlich am Dateiobjekt erkannt. Der Bericht
behauptet keinen Schutz gegen nachträglich unkooperative Programme.

Die Imagelease setzt den dokumentierten lokalen Linux-`flock`-Vertrag voraus.
NFS benötigt für exklusive flock-Sperren ausdrücklich einen Schreibhandle,
SMB besitzt eigene Bytebereichssemantik. Der bestehende Managed-
Installationsweg weist NFS, SMB/CIFS und FUSE bereits an seiner
Dateisystemzulassung ab. Dieser Folgefix erklärt solche Dateisysteme nicht
nachträglich zu unterstützten Managed-Installationsorten. Eine universelle
Zusicherung für historische Nur-Terminal-Installationen auf Netzlaufwerken
wird mangels ausgeführtem Hostnachweis nicht behauptet.

Der Folgeauftrag hat keine neue Produktabhängigkeit außerhalb der freigegebenen
Fläche benötigt. Main registriert die sechs neuen konkreten Fallnamen und den
um die Lesermehrzahl erweiterten bisherigen Sperrfall in seiner gemeinsamen
Remote-Suite. Main besitzt weiterhin Integration, Typ-/Compilerbeleg, Git,
Graphrefresh und den einzigen terminalen Release.

## Fortsetzung nach Remote-Strukturbefund 40332971455

Der übernommene Stand ist `10988167`, einschließlich der kandidatengebundenen
Remoteformatierung. Die Remote-Vorbereitung meldet `installer_cli.rs` mit
541 normal formatierten Zeilen. Compiler- und Gerätejobs wurden nach diesem
Vorbereitungsfehler übersprungen; daraus folgt kein Laufzeit- oder Typbefund.
Der frühere lokale Größenbeleg dieses Autors zählte die damaligen kompakten
Quellzeilen und genügte damit nicht zum Nachweis der Projektgrenze in normaler
Rustformatierung. Diese Strukturabweichung wird im selben Autorzyklus behoben.

### Funktionsbefund und erweiterte Kettenkarte vor dem Strukturfix

Das fehlerhafte Modul vereint drei Verantwortlichkeiten: `provision` verarbeitet
den expliziten Installeraufruf; `retire_current_installation` und `retire_mcp`
führen die selektive Entfernung aus; `RemovalFile`, `RemovalRecord`,
`load_removal`, `write_removal_record`, `inventory`, `remaining_managed_files`,
`generation_hash`, `removal_top_file`, `removal_directory_allowed` und
`removal_file_allowed` bestimmen und persistieren den Entfernungsnachweis.
`exists`, `verify_removal_file`, `remove_proved_file` und `remove_empty` bilden
die vorhandenen nichtrekursiven Dateioperationen dieser beiden letzten Ketten.
Die Funktionen müssen fachlich getrennt werden; Zeilenverdichtung und eine
Gateausnahme sind keine Lösung.

Erweiterte vollständige betroffene Kette:

1. `se-mcp-launcher --provision` → `launcher::run_launcher` →
   `installer_cli::provision` →
   Argumentprüfung → vorhandene Identität oder neue Identität → idempotente
   Produkt-/Runtimeprüfung beziehungsweise `stage_generation` →
   `commit_initial_generation`. Die bestehende Entfernenzustandssperre bleibt
   vor jeder Provisionierung erhalten.
2. NSIS → `plugin retire-installation` → unveränderter Re-Export in
   `mcp_launcher/mod.rs` → `installer_cli::retire_current_installation` →
   laufendes installiertes CLI-Image, Stamm und Prozessbesitzer prüfen →
   Entfernungsrecord laden oder vorhandenes Manifest prüfen →
   `retire_mcp` → Owner-/Control-/Operationslocks und Recoveryprüfung →
   vorhandener Record oder neues Inventar → Hash-/Prozess-/Lifetimenachweise →
   dauerhaften Record schreiben → bewiesene Dateien entfernen → Leases
   freigeben → Lockdateien entfernen → ausschließlich leere Ordner entfernen.
3. Inventar → feste Top-Level-Dateien und zugelassene Hashgenerationen
   einschließlich eigener Runtimequarantänen → OS-Pfad-/ACL-/Linkprüfung →
   Imagehash und Metadaten-/Identitätsprüfung → unverändertes schema-1-Record
   mit ursprünglichen Dateihashes. Laden prüft dieselbe Pfadmenge, eindeutige
   Dateinamen und Pflichtbelege. Wiederaufnahme prüft diese Originalhashes;
   fehlende Locks bleiben ausschließlich nach dem bereits belegten Ende der
   Programmentfernung zulässig. Fremde Kinder und zweckentfremdetes
   `install-last-applied.txt` bleiben erhalten.
4. Der unveränderte Testmodulpfad
   `mcp_launcher::installer_cli::batches_uninstall_task_tests` benötigt private
   Zugänge zu Ausführung, Inventar, Recordkonstruktion und den bestehenden
   Fixturehelfern. Eine Extraktion darf diesen Pfad und keinen Fallnamen
   verändern. Admission, Installation, Generator und Updatetransaktion lesen
   weiterhin denselben `mcp/uninstall.v1.json`-Zustand; dessen Schema und
   Veröffentlichungspunkt werden durch die Strukturänderung nicht berührt.

Quellen dieses Folgeblocks: `installer_cli.rs` und der Testmodulverbraucher im
Stand `10988167`, die Aufrufer in `mcp_launcher/mod.rs`, `launcher.rs`, `admission.rs` und
`cli/plugin.rs`, `/tmp/smartexplorer-rv1-40332971455/format.json` sowie der
zugewiesene Moduleintrag in `/tmp/smartexplorer-rv1-size-inventory.json`.
Die erneute nur lesende Graphabfrage findet die bisherigen Funktionsknoten;
ihre Zeilenpositionen stammen noch aus dem Stand vor der Remoteformatierung.
Der Graph wurde nicht verändert.

### Konkreter Strukturplan und zweiter Rechercheabgleich vor Änderung

| Schritt | Zuständigkeit und Dateien | Erhaltungsvertrag und erwartetes Ergebnis |
|---|---|---|
| ST1 | `installer_cli.rs` bleibt Fassade und Provisionierung. | Der vorhandene `provision`-Funktionskörper bleibt identisch. Der bisherige `retire_current_installation`-Name bleibt als crate-interner Re-Export erreichbar. |
| ST2 | Neues `installer_cli_removal.rs` übernimmt tatsächlichen CLI-Einstieg, Sperren, Löschreihenfolge, unmittelbare Hashprüfung und Leerordnerentfernung. | Sämtliche bisherigen Funktionskörper, Fehlertexte und Dropzeitpunkte bleiben identisch; keine rekursive oder neue Dateientfernung. |
| ST3 | Neues `installer_cli_inventory.rs` übernimmt Recordtypen, Laden/Schreiben, Inventar, verbleibende eigene Dateien und feste Pfadregeln. | Serdeattribute, Feldreihenfolge, Schema, Recordbudget, Pfadmengen, Pflichtdateien und Retrybelege bleiben identisch. Gemeinsame interne Zugriffe werden auf `pub(super)` begrenzt. |
| ST4 | Bestehendes Testmodul bleibt in `installer_cli.rs`; eng benannte Importe unter seinem bestehenden Test-/Desktopfilter versorgen es. | Vollständige Fallpfade und Fallnamen bleiben erhalten; es entsteht keine neue Fallregistrierung. Normale Rustformatierung bleibt erhalten und jede der drei Dateien bleibt mit deutlichem Abstand unter 500 Zeilen und 50 KiB. |
| ST5 | Eigene anschließende statische Kettenreview und Quellvergleich gegen `10988167`. | AST-Parse, Modulauflösung, private Sichtbarkeit, identische Funktionskörper/Recorddefinitionen, unveränderte Testquelle und Whitespace werden nachvollziehbar geprüft. Keine lokale Ausführung; Main bestätigt über denselben Remote-Suiteeinstieg. |

Entscheidung: Zwei fachliche Extraktionen erhalten die kleine vorhandene
Installerfassade. Eine bloße Aufteilung nach Zeilenbereichen oder die
Verschiebung aller Inhalte in ein neues Sammelmodul würde die Verantwortungen
nicht trennen. Neue Persistenztypen, zusätzliche Abhängigkeiten und geänderte
OS-Adapter sind für diesen Strukturfix nicht erforderlich.

Der zweite Rechercheabgleich betrifft ausschließlich Modulpfade und
Sichtbarkeit. Die [Rust-Modulreferenz](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute)
bestätigt, dass `#[path]` außerhalb eines Inlineblocks relativ zum Verzeichnis
der Quelldatei auflöst. Die [Rust-Sichtbarkeitsreferenz](https://doc.rust-lang.org/reference/visibility-and-privacy.html#pubin-path-pubcrate-pubsuper-and-pubself)
belegt die Begrenzung auf das Elternmodul durch `pub(super)` und die
Weitergabe eines bestehenden Einstiegs durch Re-Export. Abruf: 2026-10-26.
Diese vorhandenen Sprachmittel genügen; OS-, Serde- und Dateisystemverträge
bleiben unverändert und benötigen keine neue Ausführungsabhängigkeit.

### Umgesetzte Modulgrenzen

`installer_cli.rs` enthält weiterhin den unveränderten Provisionierungskörper.
Es bindet die beiden fachlichen Kindmodule über die freigegebenen relativen
Pfade ein und reicht ausschließlich den bisherigen crate-internen
`retire_current_installation`-Einstieg weiter. Das bestehende Testmodul bleibt
mit identischem Desktop-/Testfilter an derselben Stelle im Modulbaum. Seine
benannten Hilfsimporte gelten ausschließlich unter demselben Filter.

`installer_cli_removal.rs` enthält den tatsächlichen CLI-Einstieg,
`retire_mcp`, die unmittelbare Dateihashprüfung, die bewiesene Dateientfernung
und die Leerordnerentfernung. Die Reihenfolge der vorhandenen Prüfung,
Lockübernahme, Recordveröffentlichung, Entfernung und expliziten Drops bleibt
im Funktionskörper identisch zum übernommenen Stand.

`installer_cli_inventory.rs` enthält die beiden Recordtypen, Lesen und
Schreiben des Entfernungsnachweises, die Inventarbildung, die Prüfung eigener
Restdateien und die feste Pfadmenge. `exists` ist die bereits vorhandene
nichtmutierende Existenzprüfung beider Entfernungsketten. Benötigte Typen,
Felder und Hilfsfunktionen sind ausschließlich für das Elternmodul sichtbar;
die Pfadprädikate und das Schemafeld bleiben innerhalb des Inventarmoduls
privat. Es gibt keinen neuen öffentlichen Produktvertrag und keine neue
Ausführungsabhängigkeit.

| Datei | Vorliegende normal gegliederte Zeilen | Dateigröße |
|---|---:|---:|
| `installer_cli.rs` | 102 | 3.973 Byte |
| `installer_cli_removal.rs` | 170 | 6.316 Byte |
| `installer_cli_inventory.rs` | 304 | 11.197 Byte |

Die Funktionskörper stammen unverändert aus der übernommenen Remoteformatierung.
Attribute, Imports, Moduldeklarationen und Recordfelder stehen auf üblichen
getrennten Zeilen. Es wurde kein Formatter ausgeführt, keine Zeile zur
Grenzumgehung verdichtet und keine Größenregel geändert.

### Eigene anschließende Anti-Regressionsreview des Strukturfixes

Prüfbasis ist der nach der Extraktion vorliegende Quellbestand gegen
`10988167`. Derselbe Autor hat sämtliche ursprünglichen Funktionen ihrer
neuen Verantwortung zugeordnet und danach folgende Integrationen erneut
statisch abgeglichen:

- Provisionierung: Der vollständige `provision`-Körper ist bytegleich.
  Argumentmenge, Missing-/Duplicatefehler, Entfernenzustandssperre,
  Idempotenzprüfung, native Vertragsprobe, Stage und Initialcommit behalten
  denselben Aufrufer und dieselbe Reihenfolge.
- Selektive Entfernung: Alle ursprünglichen Funktionskörper sind als Menge
  genau einmal vorhanden und bytegleich. Das umfasst laufendes CLI-Image,
  installierten Stamm, tatsächliche Prozessbesitzer, Owner-/Control-/Operations-
  und Lifetimebelege, Recoveryfehler, unmittelbare Hashprüfung und ausschließlich
  nichtrekursive Datei-/Leerordnerentfernung. Die Modulverschiebung erzeugt
  keine zusätzliche Operation und verschiebt keinen Dropzeitpunkt.
- Persistenz und Retry: Recordfeldnamen, Feldreihenfolge, Feldtypen,
  Derive-/Serdeattribute, Schemawert, erlaubte Pfade, Pflichtbelege und
  Recordbudget bleiben erhalten. Das Schemafeld ist weiterhin privat.
  Die Wiederaufnahme benutzt dieselben Originalhashes, dieselben Regeln für
  fehlende Locks und denselben abschließenden Recordentfernungspunkt.
  Zusätzliche Kinddateien und Benutzerprofile werden durch die Aufteilung
  weder gelesen noch zur Löschung freigegeben.
- Modulzugriffe: Die drei relativen `#[path]`-Ziele einschließlich des
  unveränderten Testmoduls existieren an den erwarteten Orten. Die
  crate-interne Einstiegskette in `mcp_launcher/mod.rs`, der Launcheraufruf
  und der Pluginaufruf bleiben unverändert. Die neuen Kindmodule benutzen
  die vorhandene ausgewählte `os`-Grenze und keine direkten OS-Imports.
  Die erforderlichen internen Namen und Recordfelder sind innerhalb von
  `installer_cli` erreichbar; dessen Persistenztypen werden nicht aus dem
  Modul exportiert.
- Bestehende Fallnamen: `batches_uninstall_task_tests.rs` ist vollständig
  bytegleich zu `10988167`. Sein vollständiger Modulpfad und sein bestehender
  Desktop-/Testfilter sind erhalten. Damit bleiben sämtliche in der
  vorstehenden Falltabelle genannten Deinstallationsfälle an derselben
  Registrierungsgrenze. Es wurden keine neuen Fälle angelegt und keine
  bestehenden Fälle entfernt oder umbenannt.
- Syntax und Struktur: Ein reiner Tree-sitter-Parse der drei vorliegenden
  Dateien meldet keinen Syntaxfehler. Der AST-Abgleich bestätigt die
  vollständige Funktionsmenge mit identischen Körperbytes sowie identische
  Recordfeldnamen, Feldreihenfolge und Feldtypen. Die eigene getrackte und
  neue Dateimenge enthält keine Whitespacefehler. Das ist kein Typ-/Compiler-
  oder Laufzeitnachweis.

### Dateiumfang und verbleibender Nachweis

Für diesen Folgefix zusätzlich gelesen: aktualisierte Scope-JSON;
Remoteformatierungsbericht und zugewiesener Größeninventareintrag;
`installer_cli.rs` vollständig; `batches_uninstall_task_tests.rs` vollständig;
die drei entstandenen Module vollständig; `mcp_launcher/mod.rs`;
`launcher.rs` am Provisionierungseinstieg; `cli/plugin.rs` am
Entfernungseinstieg; `admission.rs`, `installation.rs`, `generator.rs`,
`provision.rs` und `transaction.rs` an ihren bestehenden
Entfernenzustandssperren; vorhandener Graph per nur lesender Abfrage;
die beiden oben verlinkten Rust-Referenzen. Die bisherige Funktionskarte und
die vorher dokumentierten Readonly-/Host-/Envgrenzen bleiben erhalten.

Geändert in diesem Folgefix: ausschließlich
`native/src/mcp_launcher/os/shared/installer_cli.rs` und dieser Bericht.
Erstellt: ausschließlich
`native/src/mcp_launcher/os/shared/installer_cli_removal.rs` und
`native/src/mcp_launcher/os/shared/installer_cli_inventory.rs`.
Die Testquelle, das Suite-Manifest, die CI-Orchestrierung, der Hauptarbeitsbaum
und der Graph wurden von diesem Autor nicht verändert. Es gab keine Änderung
des Gitindex, keinen Commit und keinen Push durch diesen Autor.

Offener Nachweis ist ausschließlich Mains Wiederholung desselben Remote-
Suiteeinstiegs einschließlich dessen normaler Format-/Größenprüfung und
Compiler-/Laufzeitstufen. Die angegebene Dateigröße und der Quellvergleich
stammen aus statischem Lesen, nicht aus einem ausgeführten Formatter oder
einer Kompilierung. Dieser Autor hat keine Builds, Compiler, Tests, Formatter,
Unteragenten, Hostaufrufe, produktiven Profileingriffe oder Remotejobs
ausgeführt. Es besteht kein zusätzlicher Produkt- oder Zugriffbedarf
außerhalb der freigegebenen vier Schreibpfade.
