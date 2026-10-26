# M02: Transferquittungen, Missing-only und Share-Schreibschutz

Stand: 2026-10-26. Autor: derselbe Fixautor für Bestandsaufnahme, Plan,
Implementierung und anschließende eigene Anti-Regressionsreview. Auftrag von
Main vom 2026-10-26; alleinige Dateifreigabe:
`/tmp/smartexplorer-f-transfer_receipts-scope.json`.

## Erstplan aus dem Auftrag

Beauftragt sind F16-Linux, F41, F45, F46, F47 und F50. Die Arbeitskopie steht
auf `33257f82`; Main erklärt ihre Quellen gegenüber dem Fehlerkandidaten
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8` als identisch. Der eingelesene
Fehlerbeleg ist `/tmp/smartexplorer-rv1-40406246032/native-failures.json`.
Ein beobachteter fehlgeschlagener Prüffall ist noch kein eigenständig
nachgewiesener Produktfehler.

Erwartetes Ergebnis: Ordnerkopien und „Fehlende übertragen“ verwenden weiterhin
wörtliche Backendnamen und die tatsächliche Remoteidentität. Quittungen
verhindern fremde oder manipulierte Zielbindungen; Wiederholung und Konflikte
bleiben sicher. Schreibgeschützte Sharequellen bleiben unverändert geschützt
und melden den vertraglich zutreffenden Fehler.

Vor der Quelländerung werden die fehlerhaften Funktionen und ihre direkten
und indirekten Produzenten, Verbraucher und Speichergrenzen ermittelt.
Danach werden ein belegter Erhaltungsplan und die konkreten erwarteten
Suiteergebnisse festgehalten. Erst anschließend folgen Implementierung,
eigene statische Anti-Regressionsreview und vollständige Dateiliste.

Noch durch Bestandslektüre zu klären sind die Ursache der Zielpfadablehnung,
die Darstellung beobachteter gegenüber gespeicherten Pfaden, die zulässigen
Backendnamen sowie der tatsächliche Schreibschutzfehler. Es fehlt derzeit
keine ausschließlich vom Nutzer zu beantwortende Angabe.

Grenzen: keine lokalen Builds, Compiler, Tests, Formatter, Server,
Installationen, Unteragenten, Gitmutationen, Graphupdates oder Releases.
Windows-Publikationsadapter gehören ausschließlich zum parallelen Auftrag
`fix_windows_publication`. Main integriert und bestätigt ausschließlich im
bestehenden gemeinsamen Remote-Einstieg. Die lokale Abgabe bestätigt keine
Kompilierung und keine Laufzeit.

## Bestandsbefund und fehlerhafte Funktionen

| Beobachtung | Belegte Fehlerfunktion und Ursache | Erhaltener Bedienvertrag |
|---|---|---|
| F16-Linux | Desktopkopieren eines ausgewählten Ordners erreicht `Reservations::reserve`. `engine_names::join_rel("", "Ordner")` erzeugt `/Ordner`, obwohl hier ein relativer Alias verlangt wird. Der Walker übernimmt ihn; `FolderRegister::path_of` erzeugt unter dem Root ein leeres Segment. `receipts::validate` lehnt den Dateibeleg zu Recht ab. | Ordnerstruktur, leere Ordner, Dateikonflikte und erhaltene Kopierquellen. Windows-F16 hat zusätzlich eine getrennt bearbeitete Publikationsursache. |
| F45 | Derselbe relative Joinfehler betrifft den Erstlauf mit Provider-ID, aber ohne reserviertes atomisches Rename. Auch der zulässige direkte Anlagefallback übernimmt den falschen Alias. | ID allein wird nicht zur Behauptung einer REBIND-Fähigkeit; gewöhnliche Erstkopie bleibt möglich. |
| F46 | Derselbe Joinfehler betrifft die direkt und exklusiv angelegte nummerierte Ordnerwurzel ohne Provider-ID. | Vorhandener Fremdordner bleibt unverändert; Nutzdaten landen ausschließlich in der reservierten Nummer. |
| F47 | Der Erstlauf scheitert bereits an dieser Pfadursache. Derselbe fehlerhafte Alias wäre auch nach Persistierung ungültig; eine Lockerung der Dateiquittung würde den Wiederaufnahmefehler nicht beheben. | Missing-only behält bestätigte Rootnamen, überspringt gleiche Größe und ersetzt abweichende vorhandene Dateien nicht. |
| F50 | Wiederholung nach frühem Listingfehler erreicht anschließend die erstmalige Ordnerreservierung und denselben Joinfehler. | Leere Ergebnisroots nach einem belegbar noch nicht mutierten Lauf verhindern den Wiederholweg nicht. |
| F41 | Fixturefehler: Die produktive `TargetPolicy::write` liefert für einen lesbaren ReadOnly-Export ausdrücklich `ReadOnlyFilesystem`. Der Prüffall erwartete stattdessen `PermissionDenied`. Nicht freigegebene beziehungsweise private Pfade haben bereits einen getrennten PermissionDenied-Vertrag. | Der Schreibschutz, die vorgelagerte Pfad-/Sitzungsfreigabe und die getrennten gewöhnlichen beziehungsweise reservierten Rename-Operationen bleiben wirksam. |

Der Pfadfehler betrifft zwei Erzeugerstellen in `folder_reservation.rs`:
`relative = join_rel(parent, &name)` und die entsprechende Stageverknüpfung.
Bei `parent == ""` werden beide Werte absolut. Die globale `join_rel`-Funktion
wird zugleich für absolute Backendpfade verwendet; ihre Semantik wird nicht
verändert. Eine allgemeine entfernte Pfadnormalisierung wäre kein sicherer Fix.

## Direkte und indirekte Verknüpfungskarte

- Bedien-/Jobgrenze: Desktopfall `desktop_task_desktop_ops_rename_trash_copy`
  → `desktop_ops::{transfer, transfer_job, run_transfer_job}`
  → `TransferJob`/`JobView` → `run_view`. Die getrennten Kopierdialog-/Clipboard-
  und Drag-and-drop-Einstiege in `copy/os/shared/copy.rs` verwenden dagegen
  `copy/os/shared/bridge.rs::{spawn, spawn_with_recovery, run, translate}`
  und danach denselben `run_view`-Vertrag einschließlich Abbruch und Fehlern.
  Der eingelesene eigene Vorbericht beschreibt die bereits abgeglichenen App-,
  Android-, CLI- und MCP-Einstiege. Die jetzige Änderung fügt keinen Einstieg
  hinzu und ändert deren Endpunkt- oder Jobmodell nicht.
- Vorbereitung: `prepare` → `job_store::{preflight, begin, prepare_apply}`
  → `job_store_validation::{endpoint, snapshot, verify_target, verify_sources}`
  → `roots::{plan, plan_roots, plan_pairs}`. `StoredEndpoint` bindet lokale
  Bedeutung, Namespace und Stateidentität; `JobRecord::bind` erhält diese
  Bindung bei gespeichertem Wiederanlauf. Die Job-ID und vollständigen
  Quellen-/Ziel-/Policyvergleiche bleiben unverändert.
- Ordnerbesitz: `roots::plan_roots` → `Reservations::{open, reserve,
  confirm_resume, publish}` → `AliasStore::{open, save}` und
  `validate_aliases` → `FolderAliasScope`/`FolderAliases`/`FolderAlias`.
  Job, beide Namespaces, Zielroot, native Zielbedeutung und Vaultorte gehören
  zum gespeicherten Scope. Die Korrektur betrifft ausschließlich die zwei
  neu erzeugten relativen Zeichenfolgen. Vorhandene ungültige absolute
  Aliase werden nicht still umgedeutet oder übernommen.
- Ordnerveröffentlichung: exklusive Anlage → tatsächliche Directory-ID
  → reserviertes NoReplace-Rename → IDnachprüfung und Namespacebestätigung.
  Ohne ID bleibt die bestätigte direkte Anlage ein Livebeleg; bei ausdrücklich
  Unsupported bleibt der vorhandene Erstlauffallback erhalten. Unbekannte
  Anlage-/Renameantworten erlauben weder Adoption noch Neunummerierung.
- Nutzdaten: korrigierter Rootalias → `walk_roots::root` → `walk::child`
  → `discovery::on_event` → `FileWork`/`WorkQueue` → `worker::parent_ready`
  und `ops::transfer`. `FolderRegister::{actual_rel, path_of, ensure}`
  verbindet den Alias mit dem tatsächlichen Zielroot und erstellt Eltern
  einmalig. Link-/Spezialdateiauslassung und begrenzte Queue bleiben erhalten.
- Einzeldateipublikation: `local::copy_file` → `LocalObserver`; Upload und
  Remote-Stream/Serverkopie → `publish`; Download → `publish_local`.
  Segmentupload/-kopie enden ebenfalls in `publish::publish`, Segmentdownload
  im Downloadabschluss. Paketupload verwendet `prepare_batch` und `after`,
  Paketdownload `publish_local`. Alle betroffenen Wege behalten denselben
  `file_recovery`-Quittungsvertrag.
- Beweise/Persistenz: `file_evidence::{observe, observe_available,
  observe_source, verify}` → `file_recovery::{before, prepare_direct,
  after_checked, revoke, unpublished}` → `JobRun::{start_file, read_file,
  write_file}` und `receipts::{validate, repair_new_entry, visit_saved}`
  → `record_io::{write_record, read_record}`. Inhaltsdigest, Objekt-ID,
  erlaubte fehlende Remoteleserechte, vollständiger Index, Pendingintent,
  atomarer privater Datensatz und Vaultversiegelung bleiben getrennte Grenzen.
- Wiederholung: `missing_retry::{live, unstarted, target, acknowledged,
  resume}` und `verification::{verify_saved, publication_state,
  verify_source, previous_matches}` → gespeicherte Dateiroots → Rootplanung.
  Acknowledged bleibt von dauerhaftem Eigentum getrennt. Erst werden alle
  Belege geprüft; erst anschließend dürfen nachgewiesene eigene Stages und
  Movequarantänen behandelt werden. Quellenverlust, falsche Objekt-ID,
  fehlender Index und unbekannte Veröffentlichung bleiben Fehler.
- Share: `GuardedBackend::{read, write, destructive}` →
  `TargetPolicy::{read, write, destructive}` → innere Provideroperation.
  `fs_guard_extensions::rename_reserved_directory` delegiert ausschließlich
  an die strenge VFS-Operation; gewöhnliches `rename_no_replace` bleibt
  getrennt. `FsAccess::{resolve_write, rename_reserved_directory}` und
  `folder_reservation::serve` erhalten Export-/Lease-/Principalbindung und
  die vorhandene Antwort. Dieser Fix ändert nur die zu enge Fixtureannahme.

Der gelesene AST-Graph zeigt `Receipt` mit `StoredEndpoint`, `SourceGuard`,
`Index`, `validate` und Journalzugriff. Er ersetzt keine Quelllektüre und
liefert keinen vollständigen dynamischen Aufrufnachweis. Die Karte oben
stammt aus den persönlich gelesenen Definitionen und Aufrufern.

## Entscheidungsplan und zweiter Quellenabgleich vor der Umsetzung

1. Gewählt: Die zwei relativen Ordnernamen mit einem kleinen privaten
   Relativjoin erzeugen: leerer Elternpfad ergibt unverändert den Literalnamen;
   vorhandener Elternpfad erhält genau seine bestehende Slashverknüpfung.
   Kein URL-Decoding, keine Groß-/Kleinschreibung, kein Trimmen von Literalnamen.
   Absolute Backendjoins bleiben unangetastet.
2. Verworfen: Führenden Slash beziehungsweise leere Segmente in gespeicherten
   Quittungen akzeptieren. Das verschleiert den Erzeugerfehler, rettet die
   Aliasvalidierung nicht und schwächt eine vorhandene Schutzgrenze.
3. Verworfen: `join_rel` global ändern. Diese Funktion hat reale absolute
   Rootverbraucher. Ein neues globales Verhalten wäre eine unnötige
   Vertragsänderung außerhalb der belegten Ursache.
4. Gewählt: Pfadzerlegung und Pfadvalidierung der Dateiquittung als kohärente
   private Verantwortung in die freigegebene `file_recovery_paths.rs` ziehen;
   die bisherigen Bedingungen bleiben vollständig erhalten. Gezielte
   Pfadorakel prüfen dort wörtliche Zeichen, Rootcontainment, manipulierte
   Relative-/Proofpfade und unveränderte Endpunktbindung.
5. Gewählt: F41 erwartet den tatsächlichen ReadOnly-Fehler, belegt weiter
   die verweigerte Operation und ergänzt die verweigerte gewöhnliche
   Renameoperation samt unveränderten Providercountern. Der produktive
   Guard wird nicht gelockert.

Primärquellen, abgerufen am 2026-10-26: Rust `std` 1.99.1,
[`Path`](https://doc.rust-lang.org/std/path/struct.Path.html) und
[`std::path`](https://doc.rust-lang.org/std/path/), dokumentieren lokale
Plattformsyntax und die Normalisierung wiederholter Trennzeichen. Daraus
folgt gerade keine Normalisierung beliebiger Backendliterale. Die
[`ErrorKind`-Dokumentation](https://doc.rust-lang.org/std/io/enum.ErrorKind.html)
trennt unzureichende Rechte von einem Schreibversuch auf schreibgeschütztem
Speicher. Die zusätzliche Unterscheidung lesbarer Exporte und verweigerter
Pfade ist konkret im gelesenen `TargetPolicy`-Code belegt.

## Konkreter Arbeits- und Erhaltungsplan vor der Quelländerung

| Schritt | Dateien / Abhängigkeit | Erwartetes Ergebnis im selben Remote-Einstieg |
|---|---|---|
| M02.1 | `folder_reservation.rs`; bestehende Alias-/Root-/FolderRegisterkette | Top-Level- und verschachtelte relative Aliase/Stages haben keinen künstlichen führenden Slash. ID-basierte und direkte Erstanlage, nummerierte Kollision und reserviertes Unsupported-Verhalten bleiben erhalten. |
| M02.2 | `file_recovery.rs`, `file_recovery_receipts.rs`, neue `file_recovery_paths.rs` | Semantisch unveränderte strenge Pfad-/Proofbindung ist direkt prüfbar. Wörtliche Backendnamen bleiben identisch; Ausbrüche und fremde Endpunkte bleiben verweigert. |
| M02.3 | `batches_backend_compatibility_task_tests.rs`, `batches_missing_retry_compatibility_task_tests.rs` | Bestehende F45/F46/F47/F50-Orakel bleiben vollständig; Literalnamen, verschachtelte Wurzeln und mehrmalige Wiederholung werden explizit erhalten. Keine neuen Ordnernummern, kein Überschreiben fremder Dateien. |
| M02.4 | `fs_guard_extensions.rs` | Gewöhnliches und reserviertes Rename bleiben getrennt; ReadOnly verweigert beide vor jedem Provideraufruf. |
| M02.5 | Eigene statische Nachreview und dieser Bericht | Jede geänderte Kette wird gegen den Erhaltungsvertrag gelesen; Modulregistrierung, Rustsyntax, Whitespace und Größen werden ausschließlich statisch geprüft. Main erhält genaue Fallnamen und verbleibende Laufzeitgrenzen. |

Noch vor Umsetzung wird die delegierte lokale `native_path_text`-Darstellung
an ihren tatsächlichen Adaptern abgeglichen. Eine Windows-Normalisierungsursache
wird aus der Delegation allein nicht behauptet. Der belegte relative Joinfehler
und der F41-Fixturefehler benötigen keine neue Nutzerangabe.

Der zweite Adapterabgleich ist nun gelesen: `copy/mod.rs` registriert Linux
beziehungsweise Windows; `copy/os/linux_os.rs::path_text` erhält gültiges
Unicode unverändert, `copy/os/windows.rs::path_text` verwendet die vorhandene
lokale Scanroot-/Displaykonvertierung und Slashdarstellung. Remoteziele gehen
in `destination_parts` weiterhin durch keine dieser nativen APIs. Daraus ist
keine zusätzliche beobachtete Windowsursache dieses M02 abgeleitet. Der Plan
ändert diese Darstellung, `after_checked` und die Windows-Adapter nicht.

Präzisierung M02.3: Die vorhandenen Backendorakel bleiben unverändert und
werden wiederverwendet; sie stehen bereits an der Modulgrenze. Die ergänzten
Mehrfachwiederholungs-/Literal-/Aliasorakel kommen zusammenhängend in die
freigegebene Missing-Retry-Fallgruppe. Der unter 500 Zeilen liegende vorhandene
Backendfallbestand wird dadurch nicht künstlich verdichtet oder überfüllt.

Der vor der zugehörigen Änderung gelesene Moveabgleich zeigt dieselbe
Ursache in `folder_move.rs::move_root`: der nicht wiederaufgenommene Zweig
verwendet ebenfalls `join_rel(parent, name)` für einen relativen Alias.
Main hat genau diese Aufrufstelle nachgemeldet und schreibfreigegeben.
M02.1 umfasst deshalb auch den Ersatz dieses Aufrufs durch denselben privaten
Relativhelfer. Objektidentitätsprüfung, Moveintent, NoReplace, bestehender
Alias und CrossesDevices-Rückfall bleiben unverändert. Das erwartete Ergebnis
ist ein gültiger gespeicherter Movealias mit funktionierender Wiederholung
für Top-Level- und Scaffoldwurzeln.

Die persönlich gelesenen tatsächlichen `reserve_named_folder`-Verbraucher
sind `cli/ops_copy.rs::copy_observed_in_job` und
`cli/ops_move.rs::move_observed_in_job`; Main hat eine vollständige Suche
nach genau diesem Symbol in `native/src` bestätigt. Beide halten die
zurückgegebene Reservierung über Kopie beziehungsweise Moveende. Zielplanung,
Quell-/Zielprüfung, CommandContext, Abbruch, Baumquittung und verifizierte
Quellentfernung bleiben erhalten. Der lokale ganze Ordnermove verwendet
`reserve_named_move`/`move_root`; ein CrossesDevices-Ergebnis fällt auf die
korrigierte exklusive Kopierreservierung zurück. Die Korrektur gilt damit
auch für die vorhandenen CLI-/MCP-Aufrufer dieser gemeinsamen Funktionen.

M01-Integrationsgrenze: Ein konkreter erfolgreicher Windows-Move darf einen
Liveabschlussbeleg besitzen. Ein späterer Wiederanlauf besitzt diesen
erfolgreichen APIaufruf nicht. M02 ändert weder `acknowledged`, `confirmed`,
die Pending-/Indexpersistenz noch `verify_saved`/`publication_state`.
Ein allgemeiner unbekannter Directoryflush wird durch diesen Fix nicht zum
Erfolgsbeleg. Es ist keine Quittungsänderung allein aus dem M01-Hinweis geplant.

Die letzte konkrete Verbraucherlücke ist geschlossen: `desktop_ops.rs` ist
persönlich gelesen; F16 verwendet direkt `TransferJob` und `run_view`, nicht
die getrennte alte CopyMsg-Brücke. Ebenso ist `cli/folder_conflict.rs` mit
dem einzigen externen `confirmed_named_move`-Aufrufer gelesen. Dort verlangt
`CommandContext::finish_confirmed_move` vor der Aliasbestätigung fehlende
Quelle, passenden vollständigen Quellen-/Ziel-/Policyvertrag, unveränderte
Zielparent-ID und gespeicherte Quellobjekt-ID. Erst danach wird `complete`
geschrieben. Dieser Ablauf bleibt unangetastet; der korrigierte Movealias
stellt seine gültige relative Eingabe wieder her.

Vor der zusätzlichen Fallimplementierung wird M02.3 um einen direkten
`reserve_named_folder`-Verbrauchernachweis ergänzt: Ein wörtlicher expliziter
Zielname mit Kollision erhält genau den nummerierten absoluten Zielpfad;
nach Freigabe der ersten Reservierung bindet gespeicherter Wiederanlauf
denselben Alias mit Objekt-ID, ohne eine weitere Anlage oder Umbenennung.
Der Fall gehört als Pfad-/Persistenzorakel in `file_recovery_paths.rs`.

## Umsetzungsergebnis

- `folder_reservation.rs::relative_child` erhält einen Literalnamen bei
  leerem Scaffold und verwendet bei vorhandenem Scaffold die bisherige
  Verknüpfung. `reserve` verwendet den Helfer genau für relativen Alias
  und relative Stage. Die absoluten Provideraufrufe verwenden weiter
  `engine_names::join_rel(root, relative)`.
- `folder_move.rs::move_root` verwendet denselben Helfer ausschließlich
  im neu erzeugten Aliaszweig. Ein vorhandener gespeicherter Movealias
  wird weiterhin überprüft und unverändert verwendet.
- `file_recovery_paths.rs` enthält die vorherige Zielzerlegung und die
  vollständigen vorherigen Pfadbedingungen. `receipts::validate` prüft
  weiterhin zuerst Version, Job, beide vollständigen Endpunkte und Root;
  danach ruft es die ausgelagerte Pfadprüfung auf. `file_recovery.rs`
  registriert das private Modul und bezieht die Zielzerlegung von dort.
- Der bestehende F41-Fall erwartet `ReadOnlyFilesystem`, belegt lesbaren
  Stat-Zugriff und verweigert jetzt ausdrücklich beide Renameoperationen.
  Beide Providercounter müssen nach den verweigerten Operationen unverändert
  bleiben. Kein produktiver Share-Guard wurde geändert.
- Die ergänzten Fallkörper erhalten Literalnamen einschließlich Punkt,
  `#`, `%`, `%2F` und Unicode, verschachtelte Scaffolds, nummerierte
  Fremdordner, gespeicherte gültige Aliase, ID- und No-ID-Erstlauf sowie
  den Unsupported-Fallback. Mehrere Missing-only-Aufrufe übertragen
  ausschließlich fehlende Dateien; abweichender vorhandener Inhalt,
  ursprüngliche Quellen und Fremdordner bleiben erhalten. Ein Wechsel
  des Zielkontos bei gleichen Pfaden bleibt abgewiesen. Ein vollständiger
  lokaler Ordnermove bleibt mit demselben relativen Alias wiederaufnehmbar.

Es gibt keine Änderung der Quittungsfelder, Serdeattribute, Versionswerte,
Digest-/Dateischlüssel, Pending-/Indexzählung, Job-/Alias-Scopes,
`acknowledged`-/`confirmed`-/Autorisierungszustände oder Missing-Retry-Logik.
Die vorhandenen Backend-/Publikations-/Recoveryfälle wurden nicht entfernt
oder abgeschwächt. Die einzige geänderte alte Prüferwartung ist F41; ihre
produktive Klassifikation ist oben belegt, die verweigerte Wirkung ist weiter
und zusätzlich an beiden Providerzählern geprüft.

## Eigene anschließende Anti-Regressionsreview

Prüfgegenstand waren der fertige M02-Patch, seine konkreten Erzeuger,
persistierten Modelle, Verbraucher, Fehler- und Abbruchgrenzen. Geprüft wurde
gegen `33257f82`, die gelesene Fehlerquelle, den unveränderten Releasejoin
aus `a814f263096fe69d3746e840b5f3fad3fd7e88a0` und die oben verlinkten
Primärquellen. Diese Prüfung ist ausschließlich statisch.

| Vertrag | Eigener Quellabgleich nach der Änderung | Ergebnis / Grenze |
|---|---|---|
| Relative gegenüber absoluten Namen | Alle drei neuen Alias-/Stageverwendungen und beide CLI-Reservierungsverbraucher gelesen. Der globale `join_rel`-Funktionskörper entspricht weiter dem Releasebestand. | Der erzeugte führende Slash entfällt ausschließlich bei relativem leerem Parent; absolute Roots und vorhandene Scaffoldnamen behalten ihre Bedeutung. |
| Quittungspfad und Rootcontainment | Rust-ASTvergleich der alten und neuen Zielzerlegung sowie sämtlicher Pfadbedingungen. Ausschließlich ausdrucksgleiche Closureblockhüllen werden im Tokenvergleich normalisiert. | Operanden, Operatoren, Literalnamen, Backslashgrenze und Fehlermeldung sind erhalten. Die Prüfung erlaubt weiterhin weder absoluten Relativpfad, leeres Segment, Punkt-/Elternsegment, NUL noch abweichenden Ziel-/Proofpfad. |
| Persistenz und Remoteidentität | `Receipt`, `Index` und `PreviousTarget` mit ihren Attributen per AST gegen den Kandidaten verglichen; Job-/Alias-/CommandContextbindung persönlich gelesen. | Modelle sind identisch. Namespace, Stateidentität, Lokalflag, Job-ID und Zielroot bleiben Teil der Übereinstimmung. Gleichlautende Pfade auf einem fremden Konto werden nicht übernommen. |
| Erstkopie ohne fehlende Providerfähigkeit | Direkte Anlage, ID-basierte Stage, strenges Unsupported und Erstlauffallback in `reserve`/`publish` nachgelesen. | Kein neuer Capability-, Download- oder Agentbedarf. Die Korrektur verändert ausschließlich die relativen Zeichenfolgen. |
| Wiederholung und Konflikte | Rootplan, Aliaswiederaufnahme, `missing_retry`, vorhandene F47/F50-Fälle und neue Mehrfachwiederholungsfallkörper abgeglichen. | Ein verlorener oder unklarer mutierter Ausgang wird nicht zum leeren Journal. Bestätigte Namen bleiben; vorhandene andere Größe wird nicht ersetzt. Laufzeitergebnis noch offen. |
| Move und Quarantäne | `move_root`, `confirmed_named_move`, `CommandContext::finish_confirmed_move`, `verification::verify_saved` sowie die unveränderten Observer-/Revokefunktionen abgeglichen. | Gespeicherter fremder Alias/Objektwechsel bleibt gesperrt. Quarantäne- und Stagebehandlung folgen weiter erst auf vollständige Belegprüfung. Keine zusätzliche vollständige Nutzdatenkopie wird eingeführt. |
| M01-Schnittstelle | Die inzwischen vorhandene Änderung von M01 in `verify_saved` lesend abgeglichen: Der zusätzliche Bestätigungsanschluss wird nur bei `confirmed && acknowledged && publication_authorized` gewählt. Fehlende positive Quittung verwendet weiter die frische Bestätigung. | M02 verändert keinen dieser Zustände und erhält die vorgeschaltete Pfad-/Endpunktbindung. M01-Datei und Copy-Re-Export werden nicht von M02 geschrieben. Gemeinsamer Laufzeitnachweis bleibt bei Main. |
| Abbruch, Quellen und Backup | `active`, `before`, `after_checked`, `revoke`, `unpublished`, `LocalObserver`, `LocalSource`, Publikations-/Download-/Paket-/Segmentabschluss nachgelesen. Die Funktionskörper in `file_recovery.rs` sind per AST unverändert. | Quellenprüfung, fehlende Readbackrechte, kontrollierte Nichtpublikation, unbekannte Antwort, bestehender Backup-/Overwriteweg und Abbruch bleiben erhalten. Kein Cleanup wird neu autorisiert. |
| Share-Schreibschutz | Produktive Guardkörper sind im Diff unverändert. F41 prüft strengen und gewöhnlichen Renamepfad sowie unveränderte innere Aufrufzähler. | ReadOnly bleibt ReadOnlyFilesystem; nicht freigegebene Pfade behalten den vorgelagerten PermissionDenied-Vertrag. Keine Umdeutung in einen Pfadformatfehler. |
| Ressourcen und Oberfläche | Relativhelfer ersetzt eine bestehende Stringallokation; keine neue Datenkopie, kein zusätzlicher Provideraufruf, kein neuer Worker und kein neues UI-Feld. Desktop-/CopyMsg-/CLI-Ausgabegrenzen gelesen. | Fortschritt, terminales Ergebnis, Quellbehalt und vorhandene Oberflächenanschlüsse bleiben unverändert. Kein Oberflächen- oder Leistungsnachweis wurde lokal ausgeführt. |

Statische Befunde: Tree-sitter-Rust konnte die fertigen eigenen Rustdateien
ohne Syntaxfehler parsen. Der ASTvergleich bestätigt unveränderte
Zielzerlegung, vollständige strenge Pfadbedingungen, Quittungsmodelle und
sämtliche vorhandenen Fallnamen. `git diff --check` meldete für die eigenen
Dateien keinen Whitespacebefund. Keine dieser Prüfungen kompiliert oder führt
Produktcode aus. Eine tatsächliche Rustformatierung und Typ-/Laufzeitprüfung
bleiben Bestandteil des Remotewegs; sie wurden lokal nicht behauptet.

## Exakte Fälle für denselben Remote-Einstieg

Vorhandene Fehlerfälle bleiben unter ihren bisherigen vollständigen Namen:

```text
desktop::os::desktop_ops::tests::desktop_task_desktop_ops_rename_trash_copy
share::fs_guard_backend::extensions::reservation_compatibility_tests::batches_task_transfer_compat_share_guard_preserves_strict_dispatch_and_read_only_denial
transfer::engine::batches_backend_compatibility_task_tests::batches_task_transfer_compat_folder_identity_does_not_require_binding_rename_for_first_copy
transfer::engine::batches_backend_compatibility_task_tests::batches_task_transfer_compat_folder_without_identity_copies_and_numbers
transfer::engine::batches_backend_compatibility_task_tests::batches_task_transfer_compat_missing_only_preserves_resolved_folder_and_skip_contract
transfer::engine::file_recovery::missing_retry::tests::batches_task_transfer_compat_missing_only_retries_empty_roots_after_no_id_listing_failure
```

Neue explizite Registrierungsnamen; Main besitzt die Manifeständerung:

```text
transfer::engine::file_recovery::paths::tests::batches_task_transfer_compat_receipt_paths_preserve_backend_literals
transfer::engine::file_recovery::paths::tests::batches_task_transfer_compat_receipt_paths_reject_traversal_and_unbound_proofs
transfer::engine::file_recovery::paths::tests::batches_task_transfer_compat_receipt_paths_bind_endpoints_and_job
transfer::engine::file_recovery::paths::tests::batches_task_transfer_compat_receipt_paths_keep_native_backslash_boundary
transfer::engine::file_recovery::paths::tests::batches_task_transfer_compat_named_folder_receipt_reuses_literal_destination
transfer::engine::file_recovery::missing_retry::tests::batches_task_transfer_compat_missing_only_literal_folder_aliases_survive_repeated_clicks
transfer::engine::file_recovery::missing_retry::tests::batches_task_transfer_compat_missing_only_whole_folder_move_keeps_relative_alias
```

Die vorhandenen `batches_task_transfer_compat_`- und `batches_task_p14_`-
Auswahlen enthalten weiter die Fälle für verlorene Anlage-/Publikationsantwort,
fehlenden Index, Upload-only, fremde gleichgroße Zielobjekte, Quelländerung,
Pendingreparatur, Movequarantäne und optischen Abbruch. Es wurde weder ein
weiterer Suiteeinstieg angelegt noch das gemeinsame Manifest geändert.
Die neuen Fälle benötigen dieselben Linux-/Windows-Entwicklungsbinärdateien,
Tempverzeichnisse und vorhandenen kontrollierten Backends der gemeinsamen
Suite; keine zusätzlichen Anmeldedaten und keine lokalen Vorläufe.

## Vollständige geänderte und erstellte Dateien

Alle Pfade sind relativ zur ausschließlich bearbeiteten Arbeitskopie
`/tmp/smartexplorer-review-fixes-ee75e91b`.

| Datei | Änderung | Zeilen / Bytes am statisch gelesenen Endstand |
|---|---|---|
| `native/src/transfer/os/shared/engine/folder_reservation.rs` | Relativhelfer und zwei erlaubte Erzeugerstellen | 432 / 17789 |
| `native/src/transfer/os/shared/engine/folder_move.rs` | Ein erlaubter relativer Movealiasaufruf | 228 / 8808 |
| `native/src/transfer/os/shared/engine/file_recovery.rs` | Privates Pfadmodul registrieren und importieren | 262 / 8783 |
| `native/src/transfer/os/shared/engine/file_recovery_receipts.rs` | Unveränderte Pfadverantwortung auslagern; übrige Validierung erhalten | 382 / 12107 |
| `native/src/transfer/os/shared/engine/file_recovery_paths.rs` | Neu: Zielzerlegung, strenge Pfadbindung und konkrete Pfad-/Reservierungsorakel | 268 / 9932 |
| `native/src/transfer/os/shared/engine/batches_missing_retry_compatibility_task_tests.rs` | Bestehende Fälle unverändert; Literal-/Mehrfachretry und ganzer Ordnermove ergänzen | 414 / 16809 |
| `native/src/share/core/fs_guard_extensions.rs` | Ausschließlich bestehender Fixturekörper: korrekter ReadOnly-Vertrag und beide verweigerten Dispatchwege | 310 / 10934 |
| `docs/plaene/2026-10-24-batches/fix-f-transfer-receipts.md` | Neu: eigener vollständiger Fixzyklus und Nachreview | Dokument |

## Vollständige gelesene beziehungsweise durchsuchte Dateien

Quellpfade relativ zur Arbeitskopie. „Funktionslektüre“ umfasst die tatsächlich benötigten Definitionen, Aufrufer oder Abschnitte; sie behauptet keine vollständige Review jedes Dateiinhalts. „Symbolsuche“ bezeichnet ausschließlich die gezielte Suche innerhalb der freigegebenen Datei. Die neue eigene Pfaddatei wurde nach ihrer Erstellung gelesen.

| Quellpfad | Tatsächlicher Zugriff |
|---|---|
| `native/src/cli/folder_conflict.rs` | Funktionslektüre und Symbolsuche |
| `native/src/cli/ops_copy.rs` | Funktionslektüre und Symbolsuche |
| `native/src/cli/ops_move.rs` | Funktionslektüre und Symbolsuche |
| `native/src/copy/mod.rs` | Funktionslektüre und Symbolsuche |
| `native/src/copy/os/linux_os.rs` | Funktionslektüre und Symbolsuche |
| `native/src/copy/os/shared/bridge.rs` | Funktionslektüre und Symbolsuche |
| `native/src/copy/os/shared/copy.rs` | Funktionslektüre und Symbolsuche |
| `native/src/copy/os/windows.rs` | Funktionslektüre und Symbolsuche |
| `native/src/desktop/os/shared/desktop_ops.rs` | Funktionslektüre und Symbolsuche |
| `native/src/desktop/os/shared/desktop_ops_tests.rs` | Funktionslektüre und Symbolsuche |
| `native/src/share/core/folder_reservation.rs` | Funktionslektüre und Symbolsuche |
| `native/src/share/core/fs_access.rs` | Funktionslektüre und Symbolsuche |
| `native/src/share/core/fs_guard_backend.rs` | Funktionslektüre und Symbolsuche |
| `native/src/share/core/fs_guard_extensions.rs` | Funktionslektüre und Symbolsuche |
| `native/src/share/core/server_fs.rs` | Funktionslektüre und Symbolsuche |
| `native/src/share/os/shared/fs_host_policy.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/core/engine_names.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/core/folder_alias.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/core/job.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/core/job_record.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/core/types.rs` | Symbolsuche |
| `native/src/transfer/mod.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/batch_get.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/batch_put.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/batches_backend_compatibility_task_tests.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/batches_missing_retry_compatibility_task_tests.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/batches_publication_compatibility_task_tests.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/discovery.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/download.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_evidence.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_recovery.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_recovery_fix_tests.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_recovery_fixture.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_recovery_observer.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_recovery_paths.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_recovery_receipts.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_recovery_tests.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/file_recovery_verify.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/folder_alias_store.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/folder_move.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/folder_reservation.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/folders.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/folders_local.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/folders_tests.rs` | Symbolsuche |
| `native/src/transfer/os/shared/engine/job_store.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/job_store_catalog.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/job_store_validation.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/local.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/local_extras_tests.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/local_plan.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/missing_retry.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/mod.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/ops.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/prepare.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/publish.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/queue.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/record_io.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/remote_copy.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/roots.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/segments_copy.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/segments_download.rs` | Symbolsuche |
| `native/src/transfer/os/shared/engine/segments_upload.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/source.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/test_backend.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/test_backend_streams.rs` | Symbolsuche |
| `native/src/transfer/os/shared/engine/test_faults.rs` | Symbolsuche |
| `native/src/transfer/os/shared/engine/test_run.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/upload.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/view.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/engine/worker.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/walk.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/walk_listers.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/shared/walk_roots.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/unix.rs` | Funktionslektüre und Symbolsuche |
| `native/src/transfer/os/windows.rs` | Funktionslektüre und Symbolsuche |
| `native/src/vfs/core/core.rs` | Symbolsuche |
| `native/src/vfs/core/extensions.rs` | Funktionslektüre und Symbolsuche |
| `native/src/vfs/core/folder_reservation.rs` | Funktionslektüre und Symbolsuche |
| `native/src/vfs/mod.rs` | Funktionslektüre und Symbolsuche |

Zusätzliche tatsächlich gelesene Regeln, Dokumente und Belege:

- `AGENTS.md` der Arbeitskopie und `/root/.codex/AGENTS.md`.
- `docs/ARCHITEKTUR.md`: Architekturübersicht und Transfer-/VFS-/Share-Verträge.
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`: M02-Befund-, Recherche-, Erhaltungs- und Meilensteinabschnitte.
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`: vollständige Einträge F16, F41, F45, F46, F47 und F50.
- `docs/plaene/2026-10-24-batches/fix-regression-transfer.md`: gezielte frühere Quittungs-, Literal-, Missing-Retry- und Strukturbelege desselben Autors.
- `docs/plaene/2026-10-24-batches/fix-f-transfer-receipts.md`: eigener inkrementeller Vorplan und fertige Nachreview.
- `native/Cargo.toml`: Paket-/Edition-/Modulkontext; keine Versions- oder Releasebehauptung.
- `native/review-task-manifest.json`: vorhandene Filter und genaue Transfer-/Desktop-Fallregistrierungen; ausschließlich gelesen.
- `graphify-out/graph.json`: deterministische Vokabular- und Beziehungstraversierung, ohne Graphänderung oder Modellaufruf zur Extraktion.
- `/tmp/smartexplorer-rv1-40406246032/native-failures.json`: Schema sowie vollständige Diagnosen der sechs zugewiesenen Beobachtungen.
- `/tmp/smartexplorer-final-review-user-contract.txt` und `/tmp/smartexplorer-f-transfer_receipts-scope.json`: vollständiger Vertrag und aktualisierte Grenzen.
- `/root/.agents/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/graphify/SKILL.md` und `/root/.codex/skills/graphify/references/query.md`.

Die zusätzlich erlaubten Dateien `native/Cargo.lock` und `graphify-out/.graphify_python` wurden für diesen Fix nicht benötigt und nicht als Quellbeleg gelesen. Die vollständigen absoluten Quellpfade ergeben sich jeweils durch Voranstellen der oben genannten Arbeitskopie; außerhalb dieser Arbeitskopie wurde ausschließlich auf die hier ausdrücklich genannten Regel-/Scope-/Belegdateien lesend zugegriffen.

## Entscheidungen und offene Grenzen

Die angefragten Scopeabhängigkeiten sind vollständig von Main freigegeben
und gelesen. Es bleibt keine bekannte notwendige Quelländerung außerhalb
der freigegebenen M02-Fläche. Beschädigte absolute Altaliaswerte aus dem
fehlgeschlagenen unveröffentlichten Kandidaten werden weiterhin verweigert;
dieser Fix nimmt keine stille Migration oder fremde Übernahme vor.

Die Laufzeitfragen sind noch unbestätigt: die vollständigen genannten
Linux-/Windows-Ordnerkopier- und Wiederholungsfälle, die Typauflösung des
ausgelagerten Moduls und der gemeinsame M01-/M02-Publikationsnachweis.
Kontrollierte Backends belegen noch keinen echten FTP-/FTPS-/SFTP-/WebDAV-/
Google-Drive-/Shareaccount und kein reales FAT32-/exFAT-Medium. Fehlende
Livekonten sind kein zusätzlicher Releasehalt. Main integriert die Änderungen,
registriert die benannten Fälle, aktualisiert den Graph und bestätigt im
bereits vorgesehenen Remote-Einstieg. Commit, Push und terminaler Release
liegen ausschließlich bei Main.

Modellbeleg aus der erhaltenen Orchestrierungsnachricht: Dieser Autor wurde
mit `model="gpt-6.1-sol"`, `reasoning_effort="max"`, `fork_turns="none"`
gestartet. Ein tatsächliches Antwortmodell stellt der Host nicht unabhängig
als Metadatum bereit; ein zusätzlicher Verifikationsbeleg wird nicht behauptet.

Der lokale M02-Autorzyklus endet mit dieser statischen Abgabe. Es wurden
keine lokalen Builds, Compiler, Tests, Formatter, Server, Installationen,
Unteragenten, Gitmutationen, Graphupdates oder Releasebefehle ausgeführt.
