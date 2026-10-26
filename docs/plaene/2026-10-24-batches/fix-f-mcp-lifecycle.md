# M03: MCP-Lebenszyklus und Codex-Kommentare

Stand: 2026-10-26. Fixautor: delegierter M03-Agent. Quellbasis beim Auftrag
`33257f82`; Remote-Fehlerevidenz gehört zum quellidentischen Kandidaten
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8`, Lauf `40406246032`.

## Erstplan aus dem delegierten Auftrag

Beauftragtes Ergebnis: Die eigene MCP-Installation bleibt mit tatsächlichen
Windows-ACLs und Linux-Prozesssicht installierbar, updatefähig und entfernbar.
Fremde Prozesse und Dateien sowie nicht verwaltete Codex-Konfiguration und
Nutzerkommentare bleiben erhalten. Quellen: delegierter M03-Auftrag und
`/tmp/smartexplorer-final-review-user-contract.txt`, Präzisierung vom 2026-10-26.

Die eigene Lieferung umfasst die belegte Fehlerfunktionsliste für F09 und
F18–F35, ihre direkte und indirekte Aufrufkarte, einen konkreten Fixplan mit
Erhaltungsvertrag, den Fix und die anschließende eigene statische
Anti-Regressionsreview. Die vollständige Dateigrenze steht in
`/tmp/smartexplorer-f-mcp_lifecycle-scope.json`.

Explizite Grenzen: keine unzugewiesenen Dateien, lokalen Builds, Compiler,
Tests, Formatter, Server, Paketinstallationen, Unteragenten, Commits, Pushes,
Graphupdates oder Releases. Main besitzt Manifestregistrierung, Remote-Suite,
Integration und Release. Die lokale Lieferung behauptet keinen Laufzeitpass.

Promptwissen: Der Remote-Lauf beobachtete verlorenen Codex-Kopfkommentar,
Windows-Vorfahren-ACL-Abbruch und unlesbare Linux-Prozessbilder. Ob jeder
abgebrochene Fall einen eigenständigen Produktdefekt enthält, ist noch unbekannt.
Bestandslektüre muss die tatsächliche Ursache, alle verbundenen Aufrufer und
die Schutzgrenze zwischen fremden und eigenen unbekannten Prozessen klären.
Keine nur durch den Nutzer klärbare Angabe verhindert die Untersuchung.

Vorläufiger Ansatz: die drei zusammenhängenden Verhaltensgrenzen getrennt
herleiten, ihre APIs und Plattformverträge aus Primärquellen abgleichen,
Meilensteine vor der Quelländerung konkretisieren und anschließend alle
geänderten Aufrufer/Persistenzgrenzen statisch gegen den Erhaltungsvertrag
prüfen. Das Installations-, Update- und Entfernungsergebnis bleibt über die
bestehenden CLI-Einstiege erreichbar.

## Zweitplan aus dem belegten Bestand

Planungstiefe 2: vorhandene, gekoppelte Installations-, Prozess- und
Konfigurationsverträge; keine neue öffentliche Funktion. Die ursprünglichen
Lifecycle- und Entfernungsfälle werden vollständig weiterverwendet.

| Belegter Fehler | Ursache im gelesenen Kandidaten | Abgrenzung |
|---|---|---|
| F09: Codex-Deinstallation verliert den Kopfkommentar | `codex::uninstall` und der entsprechende Fall entfernen das vollständige `smart-explorer`-Item. `toml_edit` hängt den Kommentar vor dem ersten Tabellenheader an dessen Dekoration; die Dekoration verschwindet mit dem Item. | Produktfehler, den der bestehende Fall direkt ausübt. Verwaltete Umgebungswerte und fremde Einstellungen sind getrennte unveränderte Verträge. |
| F18, F21, F25–F27 auf Linux: beliebige unlesbare fremde Prozesse blockieren die Entfernung inaktiver Programme | `linux_os::processes_for` versucht für jeden PID `process_identity` und bricht ohne Bezug zum Zielobjekt mit `runtime_cross_user_process_status_unknown` ab. | Produktfehler; UID und Prozessname allein sind kein negativer Ausführungsbeleg. |
| F19–F35 auf Windows: Fixtures brechen bereits bei `Installation::load` ab | `security::check_ancestor` stuft `GENERIC_WRITE` pauschal als Ersetzungsrecht ein und akzeptiert den aktuellen Tokenbenutzer nur, falls er zugleich Besitzer des Vorfahren ist. Beide Klassifikationen sind für reale Windows-ACLs zu eng. | Die tatsächlich abgelehnte SID/ACE des früheren Temp-Verzeichnisses ist unbekannt: vorhandenes Artefakt enthält keine DACL-Ausgabe, der damalige gehostete Runner ist nicht mehr für Diagnose verfügbar. Keine Behauptung über eine konkrete ACE. |

Keine dieser Beobachtungen beweist einen eigenständigen Defekt jedes vorzeitig
abgebrochenen Lifecycle-Falls. Es wird kein Fall entfernt und keine erwartete
Sicherheitsgrenze geändert, nur um den beobachteten Abbruch zu umgehen.

Erster Recherchepass, 2026-10-26:

- [Linux `/proc/pid/exe`](https://man7.org/linux/man-pages/man5/proc_pid_exe.5.html):
  Der Zugriff hängt von Ptrace-Berechtigungen ab. Eine fehlende Einsicht sagt
  nichts über die Nutzung einer konkreten Installation aus.
- [Linux `open`](https://man7.org/linux/man-pages/man2/open.2.html): Schreibendes
  Öffnen eines aktuell ausgeführten Programmobjekts liefert `ETXTBSY`.
  Ohne Create/Truncate und ohne Write bleibt der Dateiinhalt erhalten.
- [Windows Dateirechte](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights)
  und [Rechtekonstanten](https://learn.microsoft.com/en-us/windows/win32/fileio/file-access-rights-constants):
  generisches Schreiben enthält Datei-/Attributschreiben und Lesen des
  Deskriptors; Löschen und Ändern von ACL/Besitzer sind separate Rechte.
  Erzeugungsrechte eines Vorfahren sind vom Schreiben einer verwalteten Datei
  getrennt zu behandeln.
- Die gepinnte lokale Primärquelle `toml_edit 0.25.11+spec-1.1.0`, `table.rs`,
  `repr.rs`, `document.rs`, `encode.rs`, belegt Eigentum der Dekoration am
  Tabellen-/Schlüsselobjekt sowie Dokumentpositionen. Die versionsgebundene
  docs.rs-Seite ließ sich nicht abrufen; der lokale Quelltext ist der Beleg.

Historischer Entscheidungsstand vor der Scopefreigabe: Ein aufrufendes
installiertes Linux-CLI führt selbst das
Zielobjekt aus. Ein Kernelbeleg für ein inaktives Runtimeobjekt allein löst
diese Grenze nicht. Der vorgeschlagene, geprüfte Exec-Übergang auf eine private
identische Helferdatei benötigt ausdrücklich erweiterten Dateiscope und eine
zweite Vertragsrecherche. Bis zur Klärung erfolgen daran keine Quelländerungen.

## Direkte und indirekte Verknüpfungskarte

Die Karte gilt für die freigegebene verbundene Quellgrenze; der Rootgraph wurde
mit `graphify query` abgefragt, die handlungsbestimmenden Beziehungen sind
anschließend an den konkreten Quellstellen bestätigt.

| Einstieg / Besitzer | Direkte Kette | Indirekt betroffene Verträge |
|---|---|---|
| `bin/se_mcp_launcher` → `run_launcher` | `Installation::for_image/load` → `validate_tree` → Vorfahren-ACL, Besitzer, Knoten-ACL; anschließend `open_verified` → SHA-256 → `launch` → Runtime-Lifetime → `spawn_verified` | Registrierter Starterpfad, zugelassene Starterhashes, frische Laufzeitgeneration, Steuerlock, privater Kindkanal und Lebensdauer bleiben gebunden. |
| `provision` / `stage_generation` / `InitialGenerationLease` | Verzeichnisse einzeln validieren/erzeugen → unveränderliche Nutzdateien kopieren → `open_verified` → Contractprobe → Manifest | `compatible_runtimes` und `compatible_launchers` nutzen dieselbe ACL-/Dateigrenze; eingehender Katalog bleibt alleinige Zulassung. Keine Änderung an Vertragspaar, Generation oder Pointerbedeutung. |
| `UpdateTransaction::{prepare,drain,publish_stop,commit,rollback}`, `recover_update` | `Installation::load/lock/state` → Besitzerlock plus `process_matches` → Ziel-/Markerhashbeweise | Aktive Arbeit wird vor Stop entwässert, fehlgeschlagene Wiederherstellung bleibt blockiert, exakte PID-Startzeit und Programmobjekt bleiben erforderlich. `process_matches` wird nicht gelockert. |
| `admit_cli_process`, `operation`, `operation_for_contract` | Runtime entdecken → Lifetime/Operations/Control → `check_state`/`check_admission` → `owner_alive` | Uninstallrecord sperrt neue Starts; Fortsetzungen behalten vorbestehende Besitzer. Profilwartung wird aus dem laufenden Generationprozess an die installierte CLI übergeben. |
| `plugin::run(RetireInstallation)` | `retire_current_installation` → eigene Bild-/Installationsbindung → Plattform-Idleprüfung → `load_removal`/`retire_mcp` | Inventar erfasst ausschließlich feste MCP-Dateinamen, gültige Generationsnamen und Originalhashes. Owner/Control/Operation/Lifetime bleiben bis zum belegten Löschen gehalten. Teilentfernung nutzt denselben dauerhaften Record. Fremde Dateien und Kindlinks werden nicht rekursiv entfernt. |
| `garbage_collect` | exklusive Owner/Control/Lifetime → `processes_for` → atomare Quarantäne → Entfernung | Eine unbekannte relevante Aktivität verhindert Sammlung. Der neue negative Prozessbeleg gilt ausschließlich für das exakt per Gerät/Inode gebundene Programmobjekt. Der vorhandene GC-Entfernungsalgorithmus wird nicht erweitert. |
| `plugin::run(Install/Uninstall/Refresh/Status)` → Codex | `files::{executable,installation_for_registration}` → installierter zugelassener Starter oder geprüfte Legacy-CLI → `codex::{configure,uninstall,status}` → `managed::{lock,read,replace}` | `configure` erhält fremde Felder und Werte, Ownershiphash enthält command/args, Legacy-Umgebung wird plattformgerecht portiert. `uninstall` prüft Besitzer und fremde Optionen vor der Dokumentänderung, anschließend entfernt `files::remove_generated` ausschließlich eigene geprüfte Profildateien. |
| `files::refresh` und `refresh_installed_profile` | aktuelle Installation/Starter prüfen → `codex::refresh` → command-Dekoration erhalten → compare-before-replace und Rollback | Neue Kommentarbehandlung betrifft ausschließlich das nach Eigentumsprüfung entfernte eigene Serveritem; Claude-Konfiguration, Skills, Host-CLI-Protokolle und Concurrent-Editor-Schutz bleiben unverändert. |

Zusätzliche notwendige Grenze am 2026-10-26 ausdrücklich freigegeben:
`linux_os/retirement.rs`, kleine Wiederverwendung der bestehenden fexecve-Bindung
in `linux_os/spawn.rs`, interne Adapter in `windows/mod.rs` und `os/android.rs`.
Der Exec-Übergang leitet den Root aus dem tatsächlichen Bild und einem
gehaltenen Root-Deskriptor ab. Ein bloßer Umgebungswert oder ein Argument darf
keine fremde Installation auswählen.

## Entscheidungsplan und endgültige Meilensteine vor Quelländerung

Alternativen und Entscheidung:

1. Fremde Linux-PIDs anhand von UID/Prozessname ignorieren: verworfen, weil
   fremde Nutzer dieselbe Installation ausführen können und der Name veränderbar
   ist. Ein fehlender `/proc`-Zugriff bleibt kein Idlebeleg.
2. Nicht truncierendes schreibendes Öffnen des exakten Programmobjekts:
   gewählt. Gerät/Inode müssen vor/nach dem Open übereinstimmen. Der gehaltene
   Deskriptor verhindert zugleich erneutes Exec bis zum Löschschritt. Fehlende
   Schreibrechte und `ETXTBSY` bleiben konkrete Fehler, kein künstlicher Pass.
3. Nur inaktive Runtime-/Starterdateien behandeln: verworfen; das laufende
   installierte Deinstallations-CLI wäre selbst ein dauerhafter Gegenbeleg.
   Gewählt ist ein Exec derselben frisch geprüften Bytes im selben PID aus
   einem privaten Stagingverzeichnis innerhalb des tatsächlich laufenden
   Installationsroots. Dadurch wird kein ausführbares Systemtemp benötigt.
   Rootdeskriptor, tatsächlicher Stagepfad, Besitz, Originalbild, laufendes Bild
   und aktueller Manifest-/Removalbezug werden erneut zusammen geprüft.
4. Windows-Vorfahren oder DACLs ungeprüft zulassen: verworfen. Gewählt sind
   tatsächliches generisches Datei-Mapping und Vertrauen ausschließlich in
   SYSTEM, Administratoren, TrustedInstaller und den aktuellen Tokenbenutzer.
   Die strengere Mutationsprüfung innerhalb der Installation bleibt getrennt.
5. Codex-Konfiguration neu serialisieren oder Kommentare per `#`-Suche aus
   Nutzwerten rekonstruieren: verworfen. Gewählt ist die Übernahme ausschließlich
   der parsergeprüften Dekorationen des entfernten eigenen Items als Kommentare.
   Strings mit `#` bleiben Werte. Alle verbleibenden Items bleiben unverändert.

Zweiter Lückenabgleich, 2026-10-26:

- [`fexecve`](https://man7.org/linux/man-pages/man3/fexecve.3.html) führt den
  tatsächlich geöffneten Deskriptor aus. Der bestehende `FrozenExec` bereitet
  argv/env vor und führt im Übergang nur die gebundenen Syscalls aus.
- [`execve`](https://man7.org/linux/man-pages/man2/execve.2.html) erhält den PID
  und verweigert Exec auf ein schreibend geöffnetes Programmobjekt. Daraus
  folgt die Besitzdauer des Kernelbelegs bis nach der Entfernung.
- [Rust `CommandExt`](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html)
  dokumentiert `exec` als Prozessersatz. FD-CLOEXEC und Fehlerbereinigung werden
  im vorhandenen Linux-Adapter geführt; kein OS-Import wandert in `core` oder
  `os/shared`.
- [`MapGenericMask`](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-mapgenericmask)
  bildet generische Rechte vor der Prüfung auf standardisierte Datei-Bits ab.
  Erzeugungsrechte eines Vorfahren erlauben nicht automatisch Löschen eines
  schon vorhandenen geschützten Kindes.
- Gepinnte `toml_edit`-Quelle: `DocumentMut` löst Parse-Spans in eigene
  Rohdekorationen auf; `TableLike::get_key_value` und `Key::leaf_decor`
  erlauben Zugriff auf Kommentare, die vor dem Tabellenheader liegen.

| Meilenstein | Dateien / Abhängigkeit | Erhaltungsvertrag und konkretes Abnahmesignal |
|---|---|---|
| M03.1 Windows-ACL | `windows/security.rs`, `security_tests.rs` | Generisches Schreiben eines Vorfahren wird korrekt gemappt; echter fremder Delete/ACL/Owner-Zugriff, fehlende DACL und Reparsepunkte bleiben abgelehnt. Aktueller Benutzer wird unabhängig vom administrativen Vorfahrenbesitzer erkannt. Bestehende Windows-Fälle erreichen ihre unveränderten Körper. |
| M03.2 Codex-Kommentare | `codex.rs`, `codex_comments.rs`, `batches_codex_task_tests.rs` | Das nach Besitzprüfung entfernte Item verliert seine Felder, seine Nutzerkommentare bleiben einmalig erhalten; fremde Server/Umgebung/Filter/Timeouts und Concurrent-Editor-Rollback behalten ihre Verträge. F09 und gezielte Dekorationsfälle prüfen das gemeinsam. |
| M03.3 Linux-Prozessbeleg | `linux_os/mod.rs`, `process_inventory.rs`; hängt am vorhandenen Dateiobjektvertrag | Inaktive eigene Binärdateien sind trotz unlesbarer sachfremder `/proc`-Einträge nachweisbar; laufende oder nicht eindeutig gebundene Bilder bleiben gesperrt. Keine fremden Signale, keine PID-/Namensheuristik. |
| M03.4 Gebundene Deinstallation | `installer_cli_removal.rs`, `installation.rs`, `linux_os/retirement.rs`, kleine `spawn.rs`-Ergänzung und Windows-/Androidadapter; hängt an M03.3 | Installiertes Linux-CLI wechselt ohne Prozessverlust auf eigene hashgeprüfte Stagebytes. Kein Temp-Ausführungsrecht und kein Schreibrecht in fremden Verzeichnissen erforderlich. Root-/Hash-/Besitzabweichung verweigert Entfernung; eigene Stagebereinigung verwendet kein rekursives Löschen. Bildausschluss bleibt während aller MCP-Löschschritte gehalten. |
| M03.5 Eigene Autorenreview / Remote-Fallbindung | Bericht und zugewiesene bestehende Fallmodule | Jeder F09/F18–F35-Fall bleibt im selben Einstieg. Neue gezielte Fälle belegen ACL-, Kommentar-, unbekannte PID-, aktive Bild- und Übergabegrenzen; Main registriert konkrete Namen. Die Autorenreview gleicht alle Ketten und Primärverträge ab. Lokale statische Prüfung ersetzt keinen Remote-Lauf. |

Prüffragen der eigenen abschließenden Review: Kann eine fremde Installation
über Parameter/Umgebung ausgewählt werden? Kann ein unlesbarer aktiver Besitzer
ignoriert werden? Kann ein Stage-/Dateipfad getauscht werden? Bleiben Hash,
besitzgebundene Sperren, Wiederaufnahme, Benutzerkonfiguration und Kindlinks
erhalten? Bleiben Windows- und Androidpolitik außerhalb des internen
Retirementadapters unverändert? Die Antworten werden nach der Umsetzung anhand
des tatsächlichen Diffs dokumentiert.

### Ergänzung aus der eigenen Review vor dem GC-Fix

Belegter direkt betroffener Erhaltungsfehler: Der bisherige
`transaction::garbage_collect` quarantänisiert ein Generationsverzeichnis und
ruft anschließend `remove_dir_all` auf. Damit verschwinden auch nicht
verwaltete Kinddateien. Der reparierte Linux-Idlebeleg würde diesen zuvor durch
fremde `/proc`-Einträge blockierten Pfad wieder erreichbar machen. Main hat die
Korrektur und die Extraktion in `os/shared/garbage_collection.rs` ausdrücklich
freigegeben; `transaction.rs` bleibt bei Registrierung und Reexport.

Zusätzlicher Meilenstein M03.6: Frische Sammlung überspringt Verzeichnisse mit
unbekannten Kindern, damit deren Originalpfade erhalten bleiben. Eigene CLI
wird gegen den aus dem Generationsnamen gebundenen Hash verifiziert, der
Lifetime-Lock muss regulär, leer und exklusiv gehalten sein. Nach Quarantäne
werden ausschließlich diese bewiesenen Dateien nicht rekursiv entfernt.
Bereits vorhandene gültige Quarantänen werden anhand derselben Beweise erneut
bearbeitet; fehlende bereits entfernte Dateien sind zulässig. Fremde Reste
bleiben, und Fehler löschen weder fremde Inhalte noch erzeugen sie einen Pass.
Abnahmesignal: aktueller Runtimebestand bleibt erhalten; saubere unbenutzte
Altgeneration verschwindet; fremde Kinddateien/Links behalten ihren Pfad;
Quarantänereste sind wiederholbar bearbeitbar. Diese Fälle gehören in denselben
Remote-Einstieg.

Ergänzung aus der statischen Übergangsreview vor der Korrektur: Der Root- und
Bildbeleg allein bindet einen während Exec ausgetauschten Manifest-/Removalstand
nicht an die ursprüngliche Entscheidung. Deshalb umfasst der Übergabevertrag
zusätzlich den SHA-256 des vollständig validierten Manifest-/Removal-Snapshots.
Der neue Prozess berechnet denselben Wert aus seinem frisch geladenen Bestand
und verweigert jede Abweichung vor MCP-Löschungen. Der Root wird weiterhin allein
aus tatsächlichem Stagebild plus Rootdeskriptor abgeleitet. Der Snapshot ist
keine Rootauswahl und führt keinen neuen persistenten Dateityp ein.

## Umsetzung und anschließende eigene Anti-Regressionsreview

M03.1–M03.6 sind quellseitig umgesetzt. Diese Angabe bezeichnet die eigene
Änderung und statische Autorenreview; die gemeinsame Remote-Bestätigung und
der terminale Release liegen bei Main und sind hier nicht behauptet.

| Geprüfter Vertrag | Befund nach der eigenen Änderung |
|---|---|
| Windows-Vorfahrenrechte gegenüber verwalteten Dateirechten | `MapGenericMask` verwendet die vier gepinnten Datei-Mappings. `check_ancestor` prüft danach Ersetzen/Löschen/ACL/Besitzer; `check` prüft weiterhin zusätzlich Daten-/EA-/Attributschreiben. Ein generisches Schreibrecht eines Vorfahren wird nicht zu einem Schreibrecht auf verwaltete Nutzdaten. Die tatsächlich früher abgelehnte ACE bleibt unbekannt; künftige Ablehnung nennt konkrete Trustee-SID sowie ursprüngliche und gemappte Maske. |
| Vorfahrenbesitz | Die eigene Review hat zusätzlich die implizite DACL-Änderungsbefugnis des Besitzers berücksichtigt. `check_ancestor` verwirft einen untrusted owner auch ohne explizite mutierende Allow-ACE. Zugelassen bleiben ausschließlich aktueller Tokenbenutzer, SYSTEM, Administratoren und aufgelöster TrustedInstaller. Die strengere identische Besitzerbindung innerhalb des Installationsbaums bleibt bestehen. |
| Reparsegrenze | Jeder tatsächlich durchlaufene Vorfahr enthält den geschützten Installationsroot. Ein neuer Reparsepunkt erfordert einen leeren Ordner; ohne fremdes Delete/DELETE_CHILD/WRITE_DAC/WRITE_OWNER kann der fremde Schreiber diesen Vorfahren nicht zunächst leeren. Vorhandene Reparsepunkte bleiben abgelehnt. Primärbeleg: [Microsoft FSCTL_SET_REPARSE_POINT](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4aeefef8-92c3-4abc-af7a-a610caf8a165), Abruf 2026-10-26. Besitzerbefugnis: [Microsoft Access Mask](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/access-mask), gleicher Abruf. |
| Linux-Negativbeleg und aktive Prozesse | `removal_image_guard` verwendet ausschließlich Open ohne Create/Truncate/Write. Es bindet Gerät/Inode vor, am gehaltenen Deskriptor und nach dem Öffnen. `ETXTBSY`, fehlende Schreibberechtigung, Links und Objektwechsel bleiben Fehler. `processes_for` erhält nur bei diesem positiven Kernelbeleg eine leere Liste; ohne ihn bleibt der bisherige konservative PID-Bestand einschließlich unbekannter eigener/fremder Besitzer wirksam. `process_identity` und `process_matches` werden nicht gelockert. |
| Besitzdauer | Die Deinstallation hält die gewöhnlichen Programmguards während der MCP-Entfernung; `retire_mcp` hält zusätzliche Generationsguards bis nach Entfernung sämtlicher Nicht-Lock-Dateien. GC hält seinen Guard über Quarantäne und Entfernen des Programmobjekts. Kein Signal beendet einen fremden Prozess. |
| Linux-Selbstübergang | Nur das tatsächlich laufende installierte `se` kann den Übergang vorbereiten. Die private Stage liegt im selben Installationsroot; der bestehende `FrozenExec` führt den unmittelbar zuvor erneut SHA-256-geprüften read-only Deskriptor aus. Das Stage-Schreibhandle ist vorher geschlossen. Der PID bleibt erhalten; ein Systemtemp und ein fremdes Schreibverzeichnis sind nicht erforderlich. |
| Übergabebindung und Fehler | Der neue Prozess leitet den Root aus seinem tatsächlichen Stagebild ab, vergleicht den geerbten Rootdeskriptor über Gerät/Inode, prüft private Stagebesitz-/Modusdaten sowie laufendes Bild und installierte CLI. Der shared Aufrufer vergleicht den vollständigen validierten Manifest-/Removal-Snapshot. Eine Umgebungsangabe wählt keinen Root. Bei Abweichung werden keine MCP-Dateien entfernt. Schreibrecht auf dem betroffenen Installationsroot und eigener Programmdatei bleibt für den Kernelbeleg erforderlich; Fehler nennen diese Grenze. |
| Eigene temporäre Dateien | Bereinigung prüft die eigenen ursprünglichen Geräte-/Inodepaare und entfernt ausschließlich die eigene Stage-Datei plus den anschließend leeren Stageordner. Ersetzte Dateien und hinzugefügte fremde Kinder bleiben erhalten. Fehler vor Exec lassen die ursprüngliche CLI und MCP-Dateien unverändert. Ein harter Prozessabbruch vor erfolgreichem Exec kann eine eigene private Stage zurücklassen; es gibt bewusst keinen unbelegten rekursiven Aufräumer. |
| Deinstallationsinventar, Wiederaufnahme und Sperren | Zulässige Pfade, Originalhashes, Pfadtraversalverbot, Recordbudget, Besitzer-/Control-/Operation-/Lifetime-Sperren und Recoveryzustand sind erhalten. Fehlende Locks autorisieren weiterhin kein Entfernen vorhandener Programme. Teilentfernung wird gegen denselben gespeicherten Record wiederholt. Fremde Notizen, Ordner und Links werden nicht rekursiv entfernt. |
| GC-Nutzerbestand und Wiederholung | Frische Generationsordner mit unbekannten Kindern werden vollständig behalten, einschließlich der bisherigen Nutzerpfade. Saubere Ordner werden nur nach unverändertem CLI-Hash, leerem Lifetime-Lock und Inaktivitätsbeleg quarantänisiert. Vorhandene gültige Quarantänen werden erneut bearbeitet; entfernte bewiesene Dateien fehlen danach zulässigerweise, fremde Reste verbleiben. Keine `remove_dir_all`-Operation ist im neuen GC vorhanden. |
| Codex-Kommentare und nicht verwaltete Konfiguration | Eigentums-/Fremdfeldprüfung erfolgt vor `comments::remove_server`. Der Helfer liest nur geparste Key-/Table-/Value-/Array-Dekorationen; `#` in Strings wird nicht kopiert. Eine verlorene Dokumentüberschrift bleibt vorne, verbleibende entfernte Hinweise werden als selbstständige Kommentare am Dokumentende erhalten. Überlebende Items, Werte, Dekorationen und Reihenfolge werden nicht neu erzeugt. Legacy-Umgebung, geänderte command/args, Filter, Timeouts, Benutzerumgebung und Compare-before-replace/Rollback behalten ihre bisherigen Prüfungen. |
| Linux- und Windows-Profile, Android | Der neue Schreibbedarf betrifft ausschließlich den versteckten installationsbezogenen Entfernungseinstieg. `plugin install/uninstall/refresh` für Profile gewinnt keinen neuen Installationsschreibbedarf. Windows behält seine Prozess-/Sharingpolitik und führt keinen Selbstübergang aus. Android verweigert dieselben Desktop-Lebenszyklusoperationen weiterhin ausdrücklich. |
| Modul-/APIgrenzen | Neue Funktionen bleiben in MCP-OS-/Shared-Verantwortung; kein OS-Import ist in `core` oder neuem shared Produktionscode hinzugekommen. Die GC-Extraktion hält `transaction.rs` eng. Alle geänderten/neu angelegten Rust-Dateien bleiben unter 500 Zeilen und 50 KiB. Die gepinnten `windows-sys 0.59.0`- und `toml_edit 0.25.11+spec-1.1.0`-Signaturen sind lokal abgeglichen. |

Durchgeführter lokaler Nachweis: Quellen-/Diffreview, gezielter API-Abgleich,
Dateigrößen-/Zeilenzählung und `git diff --check` ohne Befund. Es wurde kein
Compiler, Formatter, Build, Test, Produktprozess, Server, Installer, Release,
Commit, Push oder Graphupdate lokal ausgeführt. Der von Main benannte vorhandene
Interpreter `/root/.local/share/uv/tools/graphifyy/bin/python` wurde zusätzlich
für einen rein statischen Parse sämtlicher geänderter/neuer Rust-Dateien mit
`tree_sitter`/`tree_sitter_rust` verwendet: kein ERROR- oder fehlender Knoten.
Das bestätigt Parserakzeptanz, keine Rust-Typprüfung, API-Verfügbarkeit oder
Laufzeitwirkung. Die funktionalen Aussagen oben beschreiben Quellverträge und
die zu bestätigenden Orakel. Main hat währenddessen den Integrationsstand auf
`92cf56a4` fortgeschrieben und laut eigener Meldung keine M03-Quelle verändert.

## Verbleibender Nachweis und Fallbindung für Main

Der bestehende Runner setzt `SE_SCREEN_TASK_WORKER` aus seiner mittels
Cargo-Artefaktmetadaten ermittelten Kandidaten-CLI vor den nativen Fällen
(`native/review-task-native.py:480`). Die neuen tatsächlichen CLI-Fälle
verwenden exakt diese Bindung und scheitern ausdrücklich bei fehlendem Binary;
es gibt keine Annahme über `target/debug`, keinen stillen Skip und keinen
zusätzlichen Testeinstieg. Die realen Prozessaufrufe stehen ausschließlich in
den für denselben Remote-Lauf vorgesehenen Fällen, sie wurden lokal nicht
ausgeführt.

Noch unbestätigt sind die tatsächliche Windows-Runner-DACL-Akzeptanz, die
plattformseitigen Kernel-/Exec-/Dateisperreffekte, die genaue
`toml_edit`-Dekorationsausgabe und die Quarantänewiederholung unter realen
Fehlern. Die vorhandenen F09/F18–F35-Fälle bleiben unverändert ausgewählt. Die
ergänzten Fälle sind am Präfix `batches_p13_` angebunden und müssen von Main
zusätzlich im bestehenden Manifest als konkrete erforderliche Fälle geführt
werden. Die vollständigen Namen und Plattformen folgen in der Dateiliste.

Offene Scopeabhängigkeit: keine. Die erforderlichen Ergänzungen für lokalen
Primärquellenzugriff, Runnerlektüre und GC-/Retirementmodule wurden vor ihrer
Verwendung durch Main freigegeben. Offene Laufzeitfrage: tatsächlicher Erfolg
der gemeinsam geänderten Grenzen auf demselben Remote-Kandidaten. Ein
fehlender oder fehlgeschlagener Remote-Lauf wäre kein Pass.

## Vollständige Datei- und Fallliste

Dateizugriffe sind vollständig enumeriert; bei Quellabgleichen wurden teils
nur die fachlich benannten Abschnitte oder gezielte `rg`-Treffer gelesen.
Die Auflistung behauptet keine vollständige Inhaltslektüre jedes Moduls.

Projektdateien mit Lesezugriff:

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/fix-f-mcp-lifecycle.md`
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`
- `graphify-out/graph.json`
- `native/Cargo.lock`
- `native/Cargo.toml`
- `native/review-task-manifest.json`
- `native/review-task-native.py`
- `native/src/bin/se_mcp_launcher.rs`
- `native/src/cli/plugin.rs`
- `native/src/cli/plugin/assets/SKILL.md`
- `native/src/cli/plugin/batches_codex_task_tests.rs`
- `native/src/cli/plugin/batches_legacy_lifecycle_task_tests.rs`
- `native/src/cli/plugin/batches_managed_task_tests.rs`
- `native/src/cli/plugin/claude.rs`
- `native/src/cli/plugin/codex.rs`
- `native/src/cli/plugin/codex_comments.rs`
- `native/src/cli/plugin/config.rs`
- `native/src/cli/plugin/files.rs`
- `native/src/cli/plugin/host_cli.rs`
- `native/src/cli/plugin/managed.rs`
- `native/src/mcp_launcher/core/batches_manifest_task_tests.rs`
- `native/src/mcp_launcher/core/capability.rs`
- `native/src/mcp_launcher/core/catalog.rs`
- `native/src/mcp_launcher/core/lifecycle.rs`
- `native/src/mcp_launcher/core/manifest.rs`
- `native/src/mcp_launcher/core/mod.rs`
- `native/src/mcp_launcher/mod.rs`
- `native/src/mcp_launcher/os/android.rs`
- `native/src/mcp_launcher/os/linux_os/mod.rs`
- `native/src/mcp_launcher/os/linux_os/process_inventory.rs`
- `native/src/mcp_launcher/os/linux_os/retirement.rs`
- `native/src/mcp_launcher/os/linux_os/spawn.rs`
- `native/src/mcp_launcher/os/mod.rs`
- `native/src/mcp_launcher/os/shared/admission.rs`
- `native/src/mcp_launcher/os/shared/batches_lifecycle_task_tests.rs`
- `native/src/mcp_launcher/os/shared/batches_uninstall_task_tests.rs`
- `native/src/mcp_launcher/os/shared/child.rs`
- `native/src/mcp_launcher/os/shared/compatibility.rs`
- `native/src/mcp_launcher/os/shared/contract_probe.rs`
- `native/src/mcp_launcher/os/shared/garbage_collection.rs`
- `native/src/mcp_launcher/os/shared/generator.rs`
- `native/src/mcp_launcher/os/shared/installation.rs`
- `native/src/mcp_launcher/os/shared/installer_cli.rs`
- `native/src/mcp_launcher/os/shared/installer_cli_inventory.rs`
- `native/src/mcp_launcher/os/shared/installer_cli_removal.rs`
- `native/src/mcp_launcher/os/shared/launcher.rs`
- `native/src/mcp_launcher/os/shared/provision.rs`
- `native/src/mcp_launcher/os/shared/transaction.rs`
- `native/src/mcp_launcher/os/windows/environment.rs`
- `native/src/mcp_launcher/os/windows/mod.rs`
- `native/src/mcp_launcher/os/windows/persistence.rs`
- `native/src/mcp_launcher/os/windows/security.rs`
- `native/src/mcp_launcher/os/windows/security_tests.rs`
- `native/src/mcp_launcher/os/windows/spawn.rs`

Zusätzliche gelesene lokale Regel-/Beleg-/Primärdateien:

- `/root/.agents/skills/arbeitsweise/SKILL.md`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toml_edit-0.25.11+spec-1.1.0/src/array.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toml_edit-0.25.11+spec-1.1.0/src/document.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toml_edit-0.25.11+spec-1.1.0/src/encode.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toml_edit-0.25.11+spec-1.1.0/src/item.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toml_edit-0.25.11+spec-1.1.0/src/key.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toml_edit-0.25.11+spec-1.1.0/src/raw_string.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toml_edit-0.25.11+spec-1.1.0/src/repr.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toml_edit-0.25.11+spec-1.1.0/src/table.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/Security/Authorization/mod.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/Security/mod.rs`
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/Storage/FileSystem/mod.rs`
- `/root/.codex/AGENTS.md`
- `/root/.codex/skills/arbeitsweise/SKILL.md`
- `/root/.codex/skills/graphify/SKILL.md`
- `/root/.codex/skills/graphify/references/query.md`
- `/tmp/smartexplorer-f-mcp_lifecycle-scope.json`
- `/tmp/smartexplorer-final-review-user-contract.txt`
- `/tmp/smartexplorer-rv1-40406246032/native-failures.json`

Der abgefragte Pfad `graphify-out/.graphify_python` existiert in dieser
Arbeitskopie nicht. Die vorhandene Graph-CLI-Abfrage funktionierte; es wurde
keine Installation und kein Graphupdate vorgenommen. Die zentrale Arbeitsweise
wurde über das lokale MCP geladen (Revision 337), nicht aus einem behaupteten
alten lokalen Zentralstand abgeleitet.

Neu erstellt oder geändert (vollständig):

- `docs/plaene/2026-10-24-batches/fix-f-mcp-lifecycle.md`
- `native/src/cli/plugin/batches_codex_task_tests.rs`
- `native/src/cli/plugin/codex.rs`
- `native/src/cli/plugin/codex_comments.rs`
- `native/src/mcp_launcher/os/android.rs`
- `native/src/mcp_launcher/os/linux_os/mod.rs`
- `native/src/mcp_launcher/os/linux_os/process_inventory.rs`
- `native/src/mcp_launcher/os/linux_os/retirement.rs`
- `native/src/mcp_launcher/os/linux_os/spawn.rs`
- `native/src/mcp_launcher/os/shared/batches_lifecycle_task_tests.rs`
- `native/src/mcp_launcher/os/shared/batches_uninstall_task_tests.rs`
- `native/src/mcp_launcher/os/shared/garbage_collection.rs`
- `native/src/mcp_launcher/os/shared/installation.rs`
- `native/src/mcp_launcher/os/shared/installer_cli_removal.rs`
- `native/src/mcp_launcher/os/shared/transaction.rs`
- `native/src/mcp_launcher/os/windows/mod.rs`
- `native/src/mcp_launcher/os/windows/security.rs`
- `native/src/mcp_launcher/os/windows/security_tests.rs`

Neue konkrete Fälle zur Registrierung im bestehenden Manifest:

| Fallname | Plattform | Betroffener Nachweis |
|---|---|---|
| `batches_p13_codex_removal_preserves_owned_table_comments_without_copying_string_hashes` | Linux und Windows | Nutzerkommentare und verbleibendes Dokument |
| `batches_p13_codex_inline_and_dotted_removal_keeps_user_document_parseable` | Linux und Windows | Nutzerkommentare und verbleibendes Dokument |
| `batches_p13_linux_idle_image_proof_preserves_bytes_despite_unknown_processes` | Linux | Unbekannte Prozesse, Kernelbesitz und Links |
| `batches_p13_linux_running_image_and_unknown_owners_remain_blocked` | Linux | Unbekannte Prozesse, Kernelbesitz und Links |
| `batches_p13_linux_image_exclusion_never_follows_a_foreign_link` | Linux | Unbekannte Prozesse, Kernelbesitz und Links |
| `batches_p13_linux_retirement_cleanup_preserves_foreign_stage_entries` | Linux | Eigene Stage/Root-/Snapshotbindung und Bereinigung |
| `batches_p13_linux_retirement_handoff_cannot_select_a_foreign_installation` | Linux | Eigene Stage/Root-/Snapshotbindung und Bereinigung |
| `batches_p13_linux_installed_cli_retirement_reexecs_only_with_its_bound_root` | Linux | Eigene Stage/Root-/Snapshotbindung und Bereinigung |
| `batches_p13_gc_preserves_foreign_children_at_their_original_paths` | Linux und Windows | Keine fremde Rekursion; Quarantänewiederholung |
| `batches_p13_gc_collects_only_proved_idle_images_and_retries_quarantines` | Linux | Keine fremde Rekursion; Quarantänewiederholung |
| `batches_p13_gc_changed_images_or_locks_never_authorize_deletion` | Linux und Windows | Keine fremde Rekursion; Quarantänewiederholung |
| `batches_p13_windows_ancestor_creation_rights_do_not_authorize_owned_file_mutation` | Windows | Vorfahren-/Dateimutationsgrenze und tatsächlicher Tokenbenutzer |
| `batches_p13_windows_ancestor_foreign_replacement_rights_remain_rejected` | Windows | Vorfahren-/Dateimutationsgrenze und tatsächlicher Tokenbenutzer |
| `batches_p13_windows_ancestor_trust_follows_token_user_not_object_owner` | Windows | Vorfahren-/Dateimutationsgrenze und tatsächlicher Tokenbenutzer |

Diese Fälle sind geschrieben, aber nicht lokal ausgeführt. Ihre Existenz ist
kein bestandener Nachweis. Die ergänzten Schutzfälle beantworten ausschließlich
die im M03-Plan genannten Grenzen; sie bilden keinen separaten Testlauf.
