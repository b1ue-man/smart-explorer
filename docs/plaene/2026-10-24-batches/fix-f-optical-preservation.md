# M08: zulässiger Indexbestand und physisch gebundener optischer Reader

Stand: 2026-10-26. Eigener statischer Autorzyklus F01/F43 abgeschlossen.
Arbeitsbaum `/tmp/smartexplorer-review-fixes-ee75e91b`, Branch
`codex/review-fixes-ee75e91b`, Dokumentationsgate `515e8286`.
Rohfehlerkandidat: `1ffbf4b0bc90b2479947680abcd0a5941cce00f8`.
Dieser Bericht enthält keinen lokalen Compiler-, Test- oder Laufzeitpass.

## 1. Auftrag und belegte Fehlerfunktionen

Quelle ist der vollständige eigene M08-Ergebnisauftrag von Main vom
2026-10-26 mit dem unveränderten wörtlichen Nutzervertrag aus
`/tmp/smartexplorer-final-review-user-contract.txt`. Die aktuelle ausschließliche
Fläche steht in `/tmp/smartexplorer-f-optical_preservation-scope.json`.
Fehlerliste, vollständige betroffene Verknüpfungskarte, konkreter Plan vor
dem Fix, Umsetzung und eigene anschließende Anti-Regressionsreview stammen
von demselben Autor. Main besitzt Integration, Manifest, Git, Rootgraph,
denselben Remote-Nachweis und Release; Windows-Rootadapter schreibt allein
M01. Keine Index-/Tresorfilter oder Rootpins werden für eine Fixture geöffnet.

Erstplan aus dem Prompt: F01/F43 aus der Rohquelle gegen ihre tatsächlichen
Fixture- und Produktfunktionen verfolgen; vollständige Indexübernahme von
Persistenzfiltern und Rootbewegung von physischer Leserautorität trennen;
Priorität, Medienidentität und Abbruch bis zu ihren Adaptergrenzen erhalten.
Nach verbundenem Bestand einen konkreten Erhaltungsplan und dessen Quellen
festhalten, dann die erlaubten Änderungen und die eigene Nachreview liefern.
Die Produkt-/Fixtureursache war zu diesem Zeitpunkt ausdrücklich offen;
eine notwendige nur vom Nutzer lieferbare Angabe war nicht benannt.

`native-failures.json` wurde für beide zugewiesenen Befunde vollständig
ausgewertet und mit `rv1-funktionsbefunde.json` abgeglichen:

| Befund | Fehlerhafte Fixturefunktion | Beobachtung und eigene Ursache |
|---|---|---|
| F01, Linux und Windows | `batches_task_optical_index_requires_proof_reuses_and_invalidates` in `app/os/shared/optical_index.rs`, Rohzeile 379 | Die In-Memory-Assertion für den nichtoptischen Eintrag passiert; erst Laden meldet angeblich partiellen Gesamtindex. Die Fixture benutzt `tempfile::tempdir()` mit `.tmp`-Präfix. `load` lässt Pfade mit verstecktem Segment aus; Windows-Temp kann zusätzlich unter dem ausgeschlossenen AppData liegen. Die Fixture erfüllt nicht den Vertrag eines zulässigen Indexpfads. |
| F43, Windows | `batches_task_optical_confined_reader_uses_held_root_and_cancel` in `source_media/tests.rs`, Rohzeile 257 | `fs::rename(root,moved).unwrap()` scheitert mit OS-Code 32. `ConfinedDirectory::open_root → DirectoryHandle::open_root` hält Root und physische Vorfahren ohne FILE_SHARE_DELETE. Diese gewollte Bindung verhindert unter Windows die von der Fixture vorausgesetzte Umbenennung. |

Die eigene Kettenanalyse belegt für dieses Befundpaar keine fehlerhafte
Produktfunktion: `build_scoped` klont beim optischen Startup den vorherigen
Gesamtindex und entfernt nur den gerade geprüften optischen Teilbaum. Der
Writer schreibt den nichtoptischen Eintrag, sofern er kein Tresorpfad ist.
`FolderIndex::load` entfernt anschließend absichtlich versteckte, AppData-,
generische und weitere ausgeschlossene Segmente sowie Tresororte. Der
Windows-Rootpin setzt seinen bestehenden Sicherheitsvertrag durch.
Beide Fixtures sind zu korrigieren; ihre eigentlichen Verhaltensabnahmen
werden erhalten und präzisiert, keine Fehlermeldung bloß wegdefiniert.

## 2. Vollständige direkte und indirekte Verknüpfungskarte

Die Karte verfolgt die durch die beiden Fixtures berührten Verträge innerhalb
der freigegebenen Grenze. Sie endet an den tatsächlichen App-/VFS-APIs und
OS-Geräte-/Dateioperationen; sie behauptet keine erneute Vollreview der
unveränderten Copy-, Scanner-, Vault- oder Kernelimplementierung.

| Direkte/indirekte Kante | Konkrete Quellen und Funktionen | Erhaltener Vertrag |
|---|---|---|
| Appregistrierung und Worker | `app/drive_modules.rs → optical_index`; `build_async`, `verify_loaded_async → spawn → build/build_scoped` | Hintergrundarbeit mit unverändertem Complete/Failed/Canceled; keine Geräteprobe oder Persistenzarbeit wird in einen GUIpfad verschoben. |
| Optischer Startup / Gesamtbestand | `build_scoped`: `previous.clone()` bei `optical_only`; `probe`, `root_key`, `beneath`, `root_identity`, `paths_hash` | Nichtoptische Einträge bleiben vollständig erhalten. Optische Wurzeln werden nur mit Geräte-/Generations-/Rootobjekt-/Pfadmengenbeleg wiederverwendet; sonst neu begangen. |
| Scan und Merge | `FolderIndex::build_async_excluding → run_builder/build_message → walk::build_index_excluding → model::try_insert`; Merge in `build_scoped` | Zertifizierte Teilbäume werden nur gezielt ausgelassen und zurückgeführt. Enumeration, Größen-/Tiefegrenzen, Linkauslassung, Fehler und Abbruch bleiben vollständig. |
| Persistenz und Beweis | `build_scoped → save_with_cancel → save_cancellable → write_and_replace → platform::replace_file`; erneutes `validate_root`, privates Zertifikat Version 2 | Kein Complete vor fertigem Index; fehlgeschlagene/gecancelte Speicherung ersetzt keinen alten Index. Zertifikat ist an dieselbe vollständige Pfadmenge und aktuelle Rootidentität gebunden. |
| Bewusste Ladefilter | `FolderIndex::load → path_has_skipped_segment → should_skip/is_generic_id`; `VaultPlaces::now/shows_vault`; `folder_index/{mod,core/filters,os/persistence}.rs` | Versteckte Segmente, AppData, benannte System-/Cachepfade und generische IDs bleiben ausgelassen. Tresororte und publizierte Tresor-Laufwerke bleiben geschützt; unbekannte Werte werden nicht als zulässiger Bestand erzwungen. |
| Modell und Suche | `core/model.rs::{FolderIndex,validate_path}`, `core/search.rs`; `os/rank.rs` | HashSet erhält eindeutige Pfade, Iterationsreihenfolge ist unbestimmt; die neue Abnahme muss Mengen vergleichen. Such-/Ranglogik und Ressourcenlimits werden nicht geändert. |
| Hintergrundleser beim Index | `walk::media_guard`, `GuardedIterator::{new,next} → acquire/acquire_source/acquire_checked` | Tatsächlich optische Quelle, Hintergrundpriorität, erneute Medienprüfung je Enumeration und Abbruch bleiben erforderlich; Fehler werden nicht als vollständiger Index gespeichert. |
| Gebundener Backendroot | `LocalBackend::confined_local_root`-Vertragsgrenze → `ConfinedLocalBackend::open`, `root`, `parts`, `parent`, `file`; `ConfinedDirectory::open_root` | Der übergebene absolute VFS-Pfad wird relativ zur gehaltenen Rootautorität aufgelöst; ein gleichlautender neuer Pfad ersetzt diese Autorität nicht. |
| Gebundener Read | `confined_local_backend.rs::open_read/open_read_at → file → guard_file_reader → acquire_file_current → acquire_checked` | Reale Filehandles bestimmen optisches Gerät und Medium. Derselbe gehaltene Root und Fileclone bleiben über den Reader erhalten; kein Umweg über einen neuen lokalen Pfad. |
| Readerlebensdauer | `source_media/reader.rs::Reader::read`, `Guard::{validate_file,set_mode,drop}`, `probe_file` | Medienidentität vor jedem Read, begrenzte Blockgröße, Sequenzmodus, EOF-/Dropfreigabe und Wiedererwerb bleiben bestehen. |
| Priorität und Abbruch | `permit.rs::{with_cancellation,context_canceled,acquire_checked}`; Vordergrund-/Hintergrundintent, `Waiting::drop`; `status::{update,snapshots}` | Eine wartende Vordergrundanforderung kann nicht vom Index überholt werden. Derselbe Jobcancel wird während des Wartens geprüft; Zähler und Guardfreigabe bleiben erhalten. |
| Windows-Rootgrenze | `local_access/mod.rs` wählt Windows; `os/windows/confined.rs::open_root` hält `DirectoryHandle`; `directory_handle.rs::open_root` pinnt gewähltes Objekt und Vorfahren | Rootrename wird mangels Delete-Sharing verweigert. M08 verändert weder Sharingflags, Zugriffsrechte, Identitätsvergleich noch den gleichzeitig von M01 besessenen Adapter. |
| Linux-Rootgrenze | `local_access/mod.rs → os/linux_os.rs → os/linux/{confined,directory_handle}.rs`; `ConfinedDirectory::open_root/file/child`, `DirectoryHandle::open_root/directory_file/open_child` | Gehaltene Arc/File-Deskriptoren bleiben über zulässiges Rename erhalten; Kindzugriffe bleiben deskriptorrelativ. Die bestehende Linuxfixture behält Umbenennung, neuen logischen Ersatzroot und Lesen der ursprünglichen Bytes. |
| Gerätebeleg / OSsperre | `source_media/mod.rs → os/{windows,linux,unsupported}.rs`; `probe/probe_file`, `try_lock/unlock` | Windows nutzt tatsächliches Volume-/Handleobjekt, Linux st_dev/sysfs und gehaltenen FD. Testprofile existieren nur im Testbinary; ein Pfadname allein wird kein produktiver Gerätebeleg. |
| Testprofile und bisherige Integrationen | `test_profiles::{register,replace,probe,enter,leave,maximum,acquisitions}`; bestehende `source_media/tests.rs` | Aliase desselben Geräts teilen Besitz; unabhängige Geräte bleiben unabhängig. Medienwechsel, Wartestatus, Prozesssperre, Copy-/Scanintegration und Abbruchassertions werden nicht entfernt. |

Die lesende Graphabfrage lautete
`graphify query 'optical_index FolderIndex load build_async confined_reader acquire_file_current' --budget 900`.
Sie erreichte relevante Index-/Readerknoten und wurde wegen allgemeiner Namen
gekürzt; die tatsächliche Karte wurde an Quellen nachgelesen. Aufruf beendet;
CLIwarnung Skill 0.9.11/Paket 0.9.17, keine Installation und kein Graphupdate.

## 3. Konkreter vorheriger Fixplan und Erhaltungsvertrag

Zweitplan aus Bestand: F01 ist ein Vergleich eines vom tatsächlichen
Ladefilter ausgeschlossenen Fixturepfads mit dem zulässigen Gesamtindex.
F43 setzt eine POSIX-artige Rootbewegung auch unter Windows voraus. Der
gemeinsame physische Warte-/Abbruch-/Readteil ist richtig und soll künftig
auf Windows erreicht werden. Die zusammenhängende Planung betrifft mehrere
gekoppelte Verträge, keine neue Produktfunktion und keinen Strukturumbau.

### Quellen und Entscheidungsplan

Cargo.lock enthält tempfile **3.27.0**; Cargo.toml bindet auf Windows
windows-sys **0.59** mit vorhandenem Win32_Foundation. Die offizielle
[tempfile-Builder-Dokumentation 3.27.0](https://docs.rs/tempfile/3.27.0/tempfile/struct.Builder.html)
belegt Standardpräfix `.tmp`, frei gewähltes Präfix und `tempdir_in` mit
TempDir-Lebensdauer. Nur das Präfix zu ändern reicht auf Windows unter
AppData nicht. Gewählt wird daher eine eigene temporäre Fixture unter dem
ersten durch den unveränderten echten Pfadfilter akzeptierten Vorfahren
des aktuellen Arbeitsverzeichnisses. Der gesamte erzeugte Pfad wird nochmals
mit demselben Filter geprüft. Erstellungs-/Unicodefehler bleiben Fehler;
kein Fall wird deswegen übersprungen und kein Filter dupliziert.

Die [CreateFileW-Dokumentation](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
belegt Delete-Sharing als Voraussetzung für parallelen Delete-/Renamezugriff.
Die [Microsoft-Fehlercodetabelle](https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-)
ordnet Code 32 der Sharing-Verletzung zu. Die beobachtete Fehlermeldung und
die konkreten gehaltenen Rootflags stimmen damit überein. Windows soll diese
gezielte Verweigerung nachweisen; Linux behält seine bestehende erfolgreiche
Rootumbenennung. Beide Wege müssen anschließend denselben wartenden
physischen Reader abbrechen und nach Freigabe die Originalbytes lesen.

Der nur unter `cfg(test)` eingehängte Helfer
`source_media/fixture_root.rs` wird am äußeren Modulniveau von
`app/os/shared/optical_index.rs` über
`#[path = "../../../source_media/fixture_root.rs"]` geladen. Die
[Rust-Modulreferenz](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute)
bestätigt die Auflösung relativ zur Quelldatei an diesem Modulniveau.
Es entsteht kein neuer produktiver Public-API- oder Plattformpfad.
Primärquellen geprüft am 2026-10-26.

Verworfene Wege: Filter zu lockern, gefilterte Pfade als erwarteten
Gesamtbestand zu erzwingen oder Windows-Delete-Sharing zu erweitern würde
bestehende Endnutzer-/Sicherheitsverträge verändern. Den Windowsfall oder
seinen gemeinsamen Abbruchteil auszublenden würde seine eigentliche
Verhaltensabnahme verlieren. Ein fester Hosttemp-/Homepfad wäre weder nötig
noch selbständig entdeckt. Nur den bisherigen Einzelpfad zu prüfen reicht
nicht für den ausdrücklich verlangten vollständigen zulässigen Gesamtindex.

### Arbeitspunkte vor jeder Produkt-/Fixtureänderung

| Punkt | Eigene Dateien | Konkretes erwartetes Ergebnis |
|---|---|---|
| Gültige Indexfixture | Neuer test-only `source_media/fixture_root.rs`; cfg(test)-Registrierung und Aufruf in `app/os/shared/optical_index.rs` | Eigener TempDir mit lesbarem nicht gefiltertem Gesamtpfad; produktive Indexworker bleiben vollständig unverändert. |
| Vollständige Indexabnahme | Bestehender F01-Fall in `optical_index.rs` | Alte Beweis-/Reuse-/Invalidierungs-/Retained-/Cancelassertions bleiben. Zusätzlich vollständige Gleichheit der gespeicherten/geladenen und verifizierten Pfadmengen statt Annahme über HashSet-Reihenfolge. |
| Physische Plattformabnahme | Bestehender F43-Fall in `source_media/tests.rs` | Windows erwartet genau Sharing-Verletzung und unveränderten Root; Linux behält Rename und Ersatzroot. Gemeinsamer Erwerb am effektiven physischen Root, Wartestatus auf ursprünglichem Gerät, Interrupted-Cancel und Originalread bleiben auf beiden Plattformen. |
| Filter-/Tresorerhalt | Bestehender `vault_task_folder_index_skips_vault` in `folder_index/os/persistence.rs` | Im alten Index zusätzlich versteckten und AppData-Pfad anbieten; die bestehenden strengen Ergebnisassertions müssen beide weiterhin auslassen. Keine Produktfilteränderung. |
| Eigene anschließende Review | Fertiger Diff, eigene obige Karte und Primärquellen | Keine Produktkörperänderung, keine entfernte Assertion, kein umbenannter/ignorierter Fall; normale Rustformatierung unter 500 Zeilen/50 KiB. Nur statische Quellen-/Diffkontrolle. |
| Übergabe an Main | Gemeinsames Manifest nur lesbar | Bestehende optische Fälle bleiben über `batches_task_optical_` erfasst; zusätzlich den bereits vorhandenen Vault-/Filterfall im selben Remote-Einstieg registrieren. Keine neue Suite und kein lokaler Lauf. |

Zweiter Lückenabgleich vor Umsetzung: Pfadnormalisierung ist im Walk dieselbe
Slashnormalisierung wie im Filter-/Fixtureplan; die Modelliteration ist
ungeordnet. Die neue Prüfung muss deshalb vollständige Mengen vergleichen.
Windowspins und der beizubehaltende Linux-Deskriptorvertrag sind belegt.
Beim anschließenden Abgleich der Linux-Reexporte wurden zusätzlich die
beiden tatsächlichen Definitionsdateien bei Main angefragt, lesend
freigegeben und vor ihrer Nachreview gelesen. Dieser Befund ändert keinen
Fixturearbeitspunkt und keine Schreibfläche. Kein notwendiger Nutzerentscheid
ist offen.

## 4. Umsetzung und eigene anschließende Anti-Regressionsreview

Der vorstehende Plan ist umgesetzt. Es wurden ausschließlich Testcode,
ein test-only Helfer und dieser Bericht geändert; kein produktiver
Indexworker, Filter, Reader, Rootadapter oder Rechtetyp wurde verändert.

### Eigene Nachreview der fertigen Änderung

Der Autor las nach dem Fix den vollständigen neuen Helfer, beide fertigen
F01/F43-Passagen, die ergänzte Altindexfixture und den fertigen Diff erneut.
Die anschließende statische Review folgte der vorstehenden eigenen Karte:

| Erhaltungspunkt | Eigener Befund nach Umsetzung |
|---|---|
| Zulässige Fixture | Der Helfer wählt dynamisch einen Unicode-fähigen, vom tatsächlichen Pfadfilter akzeptierten Arbeitsverzeichnisvorfahren und ein nicht verstecktes Präfix. Er validiert den erzeugten Gesamtpfad erneut mit demselben Filter. Ein nicht erzeugbarer oder ausgeschlossener Pfad liefert einen Fehler; kein Skip und keine stille Lockerung. Der eigene TempDir behält seine normale Lebensdauer. |
| Vollständiger zulässiger Index | Alle bisherigen F01-Assertions bleiben inhaltlich erhalten: unbelegtes Inventar wird neu gelesen, gültiger Beleg wird wiederverwendet, geänderte Generation invalidiert, nichtoptischer Eintrag bleibt, Cancel erhält den vorherigen Dateistand. Zusätzlich muss die gesamte geladene Pfadmenge exakt der verifizierten Menge entsprechen. BTreeSet vergleicht Inhalte, unabhängig von HashSet-Iterationsreihenfolge. |
| Filter und Tresore | Der gesamte produktive Teil von `persistence.rs` und die vollständige `core/filters.rs` stimmen mit `515e8286` überein. Die bestehende Vaultfixture enthält zusätzlich einen versteckten und einen AppData-Altpfad. Ihre bisherigen exakten Ergebnisse erlauben weiterhin nur den normalen Pfad, nach Rücknahme der Mountpublikation zusätzlich den ehemaligen Mountpfad. Neue verbotene Pfade würden diese Assertions scheitern lassen. |
| Windows-Rootbindung | Die F43-Fixture erwartet genau ERROR_SHARING_VIOLATION, kein beliebiges Err. Sie prüft zusätzlich fehlendes Ziel und unveränderte Originalbytes. Es wird kein Delete-Sharing ergänzt, Rootpin vorzeitig geschlossen oder Test für Windows ignoriert. Die gemeinsame Warte-/Cancel-/Readabnahme folgt danach tatsächlich. |
| Linux-Rootbindung | Der bisherige Rename-/Ersatzroot-/Profilwechsel bleibt vollständig im Nicht-Windows-Zweig. Der zurückgegebene Profilguard lebt bis nach dem Read. Die konkret nachgelesenen Linuxdefinitionen halten den geöffneten Rootfiledeskriptor; Kindzugriffe nutzen ihn und nicht den inzwischen ersetzten logischen Pfad. |
| Effektiver optischer Leser | Der vorhandene Guard wird auf Windows am noch gehaltenen Originalroot, auf Linux am umbenannten physischen Originalroot erworben. Die unveränderte gemeinsame Passage wartet auf genau einen Vordergrundleser des ursprünglichen Geräts, fordert Cancel an, verlangt Interrupted und liest anschließend Originalbytes über den gebundenen Backendpfad. |
| Priorität und benachbarte Funktionen | Sämtliche optischen Fixtures vor F43 sind textgleich zu `515e8286`: Copy-/Scan-/Indexteilung, Vordergrundvorrang, unabhängige Geräte, Medienwechsel und Prozesssperre bleiben erhalten. Die vollständigen Dateien `permit.rs`, `reader.rs`, `status.rs`, `test_profiles.rs` und `source_media/mod.rs` sind unverändert. |
| Modul-/Typgrenze | Der einzige neue Modulpfad ist cfg(test), privat und relativ zur äußeren optical_index-Quelldatei. Statische Pfadauflösung erreicht genau die zugewiesene `source_media/fixture_root.rs`. Kein neuer produktiver Reexport, keine Abhängigkeit und keine Windows-Adapteränderung durch M08. |

Der Inhaltsvergleich bestätigt: Die vollständigen Dateipräfixe vor dem
ersten `cfg(test)` in `optical_index.rs` und `persistence.rs` sind unverändert.
Der komplette gemeinsame Warte-/Cancel-/Readkörper von F43 stimmt weiter
mit dem Ausgangsstand überein. Kein bestehender Fallname wurde geändert,
kein Fall entfernt und keine bestehende Assertion abgeschwächt. Die
Whitespacekontrolle der fünf eigenen Schreibpfade meldete keinen Befund.
Die vier Rustdateien bleiben unter 500 Zeilen und 50 KiB in normalem Layout;
kein lokaler Formatter wurde verwendet.

Die verwendete Windowskonstante wurde zusätzlich in der offiziellen
[windows-sys-0.59.0-Quelle](https://raw.githubusercontent.com/microsoft/windows-rs/0.59.0/crates/libs/sys/src/Windows/Win32/Foundation/mod.rs)
als WIN32_ERROR mit Wert 32 nachgelesen. Die entsprechende docs.rs-Seite
war nicht erreichbar; die versionierte Herstellerquelle schloss diese
Aufruflücke. Das vorhandene Manifestfeature Win32_Foundation genügt.

Die eigene statische Review findet keinen verbleibenden belegten Verlust
des betroffenen zulässigen Index-/Reader-/Abbruchvertrags. Das ist kein
Compiler- oder Funktionspass. Die gemeinsame Windowsintegration mit M01
und die tatsächliche Ausführung bleiben bei Main.

## 5. Gelesene Dateien

Vollständiges Verzeichnis der tatsächlich verwendeten Dateien; die Angaben
unterscheiden komplette Lektüre und gezielte Abschnitte:

- Auftrag/Evidenz: `/tmp/smartexplorer-f-optical_preservation-scope.json`,
  wiederverwendeter unveränderter Nutzervertrag
  `/tmp/smartexplorer-final-review-user-contract.txt`, F01/F43 vollständig
  aus `/tmp/smartexplorer-rv1-40406246032/native-failures.json` und
  `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`.
- Dokumente: F01/F43-/M08- und Lückenabschnitte von
  `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`, die relevante
  aktuelle Architekturzuordnung von `docs/ARCHITEKTUR.md` (Desktop,
  optische Quellen, Plattformgrenzen) und dieser vollständige eigene
  Bericht. Die vom Nutzer gelieferten AGENTS-Anweisungen und bereits
  geladenen Skills gelten weiter; hier wird keine zusätzliche aktuelle
  Volllektüre ihrer Dateien behauptet.
- Unter `native/src/app/`: `drive_modules.rs` vollständig;
  `os/shared/optical_index.rs` (Worker, Merge-/Speicher-/Zertifikatpfad,
  Root-/Hashhelfer und vollständiger Fall).
- Unter `native/src/folder_index/`: `mod.rs`, `core/filters.rs`,
  `core/tests.rs`, `os/persistence.rs`, `os/shared.rs`, `os/linux_os.rs`
  und `os/windows.rs` vollständig; `core/model.rs` (Typen, Grenzen,
  Insert/Iteration/Validierung), `os/walk.rs` (gesamter Build-/Filter-/
  Guard-/Cancel-/Progressweg). Such-/Rangmodule wurden als unveränderte
  Verbraucherverträge identifiziert, nicht als neu vollständig reviewte
  Implementierungen ausgegeben.
- Unter `native/src/source_media/`: vollständige `reader.rs`, `status.rs`,
  `test_profiles.rs`, neue `fixture_root.rs`; relevante Registrierungs-/
  Source-/Probeabschnitte von `mod.rs`, Guard-/Prioritäts-/Cancel-/Warte-
  und Fileerwerb von `permit.rs`; `tests.rs` (Copy-/Indexintegration,
  vollständiger F43-Fall und statischer Vergleich sämtlicher unveränderter
  vorangehender Fixtures); `os/linux.rs` und `os/unsupported.rs`,
  `os/windows.rs` (Probe, tatsächlicher Filehandlebezug, Lock/Unlock).
- VFS: `native/src/vfs/os/shared/local_optical.rs` vollständig;
  `confined_local.rs` (Root, Pfadteile, Parent-/Fileautorität),
  `confined_local_backend.rs` (Metadata, Read, Fileclone und Guard),
  `confined_local_extensions.rs` (gezielte optische Erwerb-/Readverweise).
- Lokale Adapter: `native/src/local_access/mod.rs` (OSauswahl/Reexporte),
  `os/windows/directory_handle.rs` (physischer Pin und Rooterwerb),
  `os/windows/confined.rs` (Root-/Childbindung), `os/linux_os.rs`
  (tatsächliche Reexporte), zusätzlich ausdrücklich durch Main freigegeben
  `os/linux/confined.rs` und `os/linux/directory_handle.rs` (Rooterwerb,
  Filebesitz und deskriptorrelative Kinder).
- Version/Auswahl: `native/Cargo.lock` (tempfile/windows-sys),
  `native/Cargo.toml` (Windowsabhängigkeit und Foundationfeature),
  `native/review-task-manifest.json` (optisches Präfix, Prozesshelper-
  Ausschluss und fehlende Vaultintegration); `graphify-out/graph.json`
  über die oben dokumentierte ausschließlich lesende Abfrage.
- Offizielle Primärquellen: die fünf erfolgreichen Rust-/tempfile-/Microsoft-
  und windows-sys-Seiten im vorstehenden Plan und Review, Abrufstand
  2026-10-26. Die zusätzliche nicht erreichbare docs.rs-Konstantenseite
  wird nicht als gelesener Versionsbeleg benutzt.

Dokumentationsgate: auf eigene Schreibpfade begrenztes
`git status --short`, `git log -1 --oneline`, gezielte F01/F43/M08-Suche und
aktueller Quellen-/Rohfehlerabgleich. Quellen und konkrete Funktionen sind
in der vorstehenden Karte bezeichnet; keine breite Repositoryerkundung.

## 6. Erstellte und geänderte Dateien

Vollständige eigene Schreibdateiliste:

| Datei | Eigene Änderung | Aktuelle Größe |
|---|---|---|
| `native/src/source_media/fixture_root.rs` | Neuer rein testbezogener TempDir-Helfer mit echter Pfadfilterprüfung | 27 Zeilen / 1.002 Byte |
| `native/src/app/os/shared/optical_index.rs` | Nur cfg(test)-Helferregistrierung und F01-Fixture-/Mengenabnahme | 400 Zeilen / 13.933 Byte |
| `native/src/source_media/tests.rs` | Nur F43-Plattformvoraussetzung; gemeinsamer physischer Cancel-/Readteil erhalten | 308 Zeilen / 11.439 Byte |
| `native/src/folder_index/os/persistence.rs` | Nur zwei zusätzliche geschützte Legacy-Indexpfade im bestehenden Vaultfall | 232 Zeilen / 8.338 Byte |
| `docs/plaene/2026-10-24-batches/fix-f-optical-preservation.md` | Dieser neue vollständige eigene Autorbericht | Keine native Größenregel |

Keine Windows-Root-/VFS-/Filterdatei, kein Manifest und kein fremder Bericht
wurde durch diesen Autor geändert. Die gleichzeitig von M01 besessenen
Windowsadapter sind ausdrücklich außerhalb der eigenen Schreibfläche.

## 7. Entscheidungen, Registrierung und offene Laufzeitfragen

Main wurde über den eigenen Ursachenbefund und den exakten zusätzlichen
bestehenden Fall `vault_task_folder_index_skips_vault` informiert; dieser
fehlte beim eigenen Manifestabgleich. Kein Schreibmehrbedarf und keine
eigenmächtige Scopeerweiterung. Die tatsächliche Windows-/Linuxabnahme des korrigierten
Index-/Root-/Abbruchwegs bleibt im selben Remote-Einstieg bei Main.

Der Fall liegt in `folder_index::os::persistence::vault_tests`. Main hat ihn
für die gemeinsame Manifestanpassung vorgemerkt und bestätigt die tatsächliche
Auswahl nach sämtlichen Quellfixes; eine bereits erfolgte Registrierung wird
hier nicht behauptet. Es wurde kein neuer Testname angelegt.

Die bestehenden Hauptfälle bleiben unter `batches_task_optical_` erhalten:

- `batches_task_optical_index_requires_proof_reuses_and_invalidates` (F01)
- `batches_task_optical_confined_reader_uses_held_root_and_cancel` (F43)
- `batches_task_optical_two_copy_jobs_and_scan_share_device`
- `batches_task_optical_wait_cancel_foreground_priority_and_independent_drive`
- `batches_task_optical_media_change_rejects_open_reader_and_queued_index`
- `batches_task_optical_process_lock_excludes_second_process`

`batches_task_optical_process_child` bleibt absichtlich aus direkter Auswahl
ausgeschlossen; der Prozesssperrenfall ruft ihn selbst gebunden auf. Diese
Auswahl und sämtliche Fallnamen wurden nicht verändert.

Offen sind der tatsächliche Windows-/Linuxnachweis des vollständigen
zulässigen Indexroundtrips, der unveränderten Zertifikats-/Cancelpfade,
der plattformgemäßen Rootvorbedingung und des anschließend gemeinsamen
physischen Reader-/Abbruchwegs. Der neue TempDir-Helfer setzt einen
schreibbaren zulässigen Arbeitsverzeichnisvorfahren voraus; fehlende
Erstellungsrechte werden als Fehler sichtbar. Es wird kein erfolgreiches
Verhalten unter einer noch nicht ausgeführten Remoteumgebung behauptet.

Die zwei zusätzlich angefragten Linuxdefinitionsdateien wurden von Main
vor ihrer Lektüre ausschließlich lesend freigegeben. Keine notwendige
Scopeabhängigkeit und keine Nutzerentscheidung bleiben offen. Der eigene
vollständige statische Autorzyklus ist abgeschlossen. Dieser Autor führte
keine lokalen Builds, Compiler, Tests, Formatter, Server, Installationen,
Unteragenten, Gitmutationen, Graphupdates oder Releases aus. Integration,
bestehende Registrierung und derselbe Remote-Nachweis gehören Main.
