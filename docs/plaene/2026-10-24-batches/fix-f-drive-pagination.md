# M10 / F17: vollständige Drive-Scanfixture

Stand: 2026-10-27; Quellenrecherche vom 2026-10-26. Fixautor und eigene anschließende Review: derselbe delegierte
Agent. Arbeitskopie `/tmp/smartexplorer-review-fixes-ee75e91b`, Branch
`codex/review-fixes-ee75e91b`. Dies ist ein Quellen- und Fixturefix innerhalb des
laufenden Gesamtbatches; eine erfolgreiche Laufzeitabnahme wird nicht behauptet.

## A — Auftrag und Erstplan vor der Quelllektüre

Quelle: delegierter Ergebnisauftrag M10/F17; ausdrücklicher Reviewvertrag in
`/tmp/smartexplorer-final-review-user-contract.txt`; Umfang in
`/tmp/smartexplorer-f-drive_pagination-scope.json`. Ziel ist, leere Zwischenseiten,
leere Kindordner, wörtliche Titel und genaue IDs durch den vorhandenen realen
Drive-/Scanpfad abzunehmen. Die Fixture muss dessen erforderliche API-Aufrufe
vollständig liefern. Der Autor muss selbst Fehlerfunktion, verkettete Stellen,
Fixplan, Fix und Anti-Regressionsreview liefern.

Aus dem Auftrag bekannte Tatsachen: F17 ist auf Linux und Windows im beendeten
Lauf `40406246032` am Kandidaten
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8` fehlgeschlagen. Der bisherige gemeinsame
Funktionsplan enthält bereits zwei Recherchepässe. Die genaue Ursache der
HTTP-400-Antwort war vor eigener Quelllektüre unbekannt.

Erstplan: beobachteten Fehler gegen den freigegebenen Scan-/Providerpfad verfolgen,
die echte Requestfolge und bestehende Erhaltungsverträge bestimmen, den
Fixture-/Produktfehler unterscheiden, vor Änderung den konkreten Fix planen,
diesen umsetzen und anschließend selbst statisch gegen die vollständige Kette
und Primärquellen prüfen. Planungstiefe 2 wegen mehrerer gekoppelter bestehender
Verträge. Keine neue API, Bibliothek, Produktoberfläche oder Persistenzform.

Grenzen: ausschließlich ausdrücklich freigegebene Pfade; keine lokalen Builds,
Compiler, Formatter, Tests, Server, Installationen, lang laufenden Prozesse,
Unteragenten, Commits, Pushes, Graphänderungen oder Releasebefehle. Main besitzt
Manifestregistrierung, gemeinsame Remote-Suite, Integration und Veröffentlichung.

## B — Belegter Bestand und Fehlerfunktion

Die F17-Artefakte nennen auf beiden Plattformen denselben Fehler:
`gui_design_task_drive_empty_intermediate_pages_continue_and_empty_children_scan`
meldet an der Wurzel `list_dir: HTTP 400: {}`. Die Artefakte zeigen keine
fehlende Seite und keinen Fehler des echten Google-Dienstes.

Belegte Ursache der Fixture: `GDriveBackend::from_setup` beginnt mit
`root_id = None`. `list_dir_entries` ruft `resolve` auf; `resolve` beginnt mit
`actual_parent_id("root")`. `identity.rs::actual_parent_id` fordert deshalb
zuerst `GET /drive/v3/files/root?fields=id` an. Die alte F17-Fixture erwartet
bereits `GET /drive/v3/files`. `gui_task_http.rs::Fixture::new` beantwortet den
abweichenden Pfad selbst mit HTTP 400. Dieselbe Rootauflösung ist auch am
gescheiterten Kandidaten vorhanden, bestätigt durch `git show` des freigegebenen
`identity.rs`. Das ist ein belegter Protokollfehler der Fixture.

Die zweite fehlende Voraussetzung betrifft das Kind: Nach der vollständigen
Rootliste validiert die bestehende Ordnerauflösung die konkrete `empty-id`
durch `files.get`; die Projektion nach dem Kindlisting löst denselben Ordner
erneut auf. Die alte Fixture bietet keine dieser zwei exakten ID-Proben an.
Alle betreffenden Antwortobjekte müssen außerdem die tatsächlich ermittelte
Root-ID in `parents` tragen. Der Alias `root` ersetzt keine reale Parent-ID.

Kein belegter Produktpagerfehler: `listing_query.rs::collect_attempt` wertet
`FileListPage::next_token` unabhängig von der Anzahl der Dateien aus. Fehlendes
`files`, `files: null` und `files: []` sind im bestehenden Parser leere Listen;
ein weiterer Token wird weiterverfolgt. Wiederholte/verweigerte Token und
`incompleteSearch` führen zu begrenztem Neustart mit verworfenem Teilergebnis;
ungültige Typen bleiben Fehler. Diese Produktlogik wird nicht abgeschwächt.

## Direkte und indirekte Verknüpfungskarte

Die Karte erfasst die durch F17 tatsächlich berührte Grenze, keine
repositoryweite Funktionsbehauptung. Die bestehende Rootgraphabfrage wurde nur
auf die freigegebenen Quelldateien beschränkt. Aus den dort vorhandenen Begriffen
wurden `drive`, `empty`, `pages`, `scan`, `listing` und `token` gewählt. Der Graph
belegt Scan-/Fixtureeinstiege und enthaltene Aufrufer; die folgende genauere
Requestfolge wurde aus dem gelesenen Quelltext hergeleitet.

| Station | Direktes Verhalten und verkettete Grenze |
|---|---|
| `gui_task_tests.rs` → `gui_task_http::{Fixture, step}` | Der bestehende F17-Fall startet einen echten Backendscan gegen den isolierten HTTP-Server. Keine vorbefüllte Rootidentität. |
| `state.rs::{test_backend, from_setup}` | Testtoken läuft nicht ab; Account-/Pfadcache und Bindings bleiben im Speicher. Die Rootidentität beginnt unbekannt. |
| `rscan.rs::{start_scan_backend, run}` → `parallel.rs::{run, serial_fallback}` | Rootstat, parallele Browserauflistung, deterministische Ausgabe, Abbruch und bestehende serielle Rückfallebene. |
| `backend.rs::list_dir` → `listing.rs::{projected_listing, list_dir_entries}` | Vollständige Rohauflistung vor Browsernamen, Ordnerprojektion, Cachepublikation und Ergebnisausgabe. |
| `resolution.rs::{resolve, valid_cached_id, validate_cached_id}` → `identity.rs::{actual_parent_id, object_json, in_parent}` | Rootalias wird in eine echte ID aufgelöst; Kindordner werden gegen genaue ID, ursprünglichen Titel, Parent und Papierkorbstatus geprüft. |
| `listing_query.rs::{collect_files, collect_attempt}` → `file_list.rs::FileListPage::parse` | Query mit tatsächlichem Parent, vollständige Tokenfolge, ID-Deduplizierung, frische Auflösung widersprüchlicher Beobachtungen, keine Teilkollektion bei Fehler. |
| `auth.rs::{get_json, metadata_text}` → `api.rs::{send_retry, parse_json}` → `http.rs` | Authentisierter GET mit vorhandenem Requesttimeout und begrenztem Wiederholungsvertrag; echte Providerfehler bleiben erhalten. |
| `sync_projection.rs::{project_folders, resolve_bound_folder, bind_folder_path}` → `sync_projection_names.rs::allocate_folders` → `sync_bindings.rs` → `os/shared/binding_store.rs` | Bestehende Parent-/Accountbindung, unveränderliche Ordneridentität, Literalname/Locatortrennung und Reservierung bleiben erhalten. F17 verwendet den Speicherzweig des Stores. |
| `duplicates.rs` + `names.rs` → `cache.rs::remember_path`/`cache_store.rs` | Wörtliche Titel erhalten reversible Browsersegmente; identische Titel behalten unterscheidbare IDs; Erfolg veröffentlicht erst das vollständige Listing. |
| `walk_state.rs::{process_listing, validate_listing, listing_failed, finish}` → `scanner::ScanMessage` | Sichere eindeutige Childnamen, genaue Eintrags-IDs, Budget-/Abbruchgrenze, vollständige Fehlpfade und terminaler Fortschritt. |
| `resolution.rs::find_child` und `promotion_api.rs::named_objects` → `collect_files` | Namensauflösung und mutierende Namensbeobachtung teilen denselben vollständigen Pager und dieselbe tatsächliche Parent-ID. |

Die drei zunächst fehlenden Lesepfade `auth.rs`, `promotion_api.rs` und
`os/shared/binding_store.rs` hat Main ausdrücklich read-only ergänzt. Es besteht
keine benötigte offene Scopeerweiterung. Die tatsächliche Google-Anmeldung und
die persistenten OS-Dateiprimitiven sind unveränderte Außengrenzen, die die
speicherbasierte F17-Fixture nicht ausübt.

## Recherchepass 1 und Entscheidungsplan C

Primärquellen am 2026-10-26 abgerufen:

- [Drive v3 files.list](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list), Abschnitte `pageToken`, `files`, `nextPageToken` und `incompleteSearch`: Fortsetzung bis zum fehlenden Token; verworfener Token verlangt frische Sammlung; unvollständige Suche darf nicht als vollständige Abwesenheit gelten.
- [Drive v3 Ordner](https://developers.google.com/workspace/drive/api/guides/folder), Einleitung: `root` ist ein Alias für eine Datei-ID.
- [Drive v3 files.get](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/get), HTTP-/Pathabschnitt: Metadaten werden über die genaue Datei-ID gelesen.
- [ProtoJSON](https://protobuf.dev/programming-guides/json/), `Presence and default-values` und `Null values`: leere wiederholte Felder können fehlen; `null` wird als nicht gesetzt geparst. Dies stützt den vorhandenen toleranten Parservertrag, ist keine Behauptung, Google müsse `null` senden.

Entscheidung: die erwarteten Requests ausdrücklich um Rootauflösung und
Ordnerproben ergänzen, eine Root-ID ungleich `root` verwenden und jede Listquery
an Parent und Token binden. Der bestehende Fallname bleibt erhalten. Die
Scanfixture wird aus der bereits 488 Zeilen langen GUI-Testdatei in die
freigegebene `scan_task_tests.rs` ausgelagert. Der gemeinsame Server erhält keine
automatischen erfolgreichen Antworten und kein vorbefülltes Identitätswissen.

Verworfene Optionen: Rootcache mit `root` vorbelegen würde die reale
Identitätsgrenze auslassen; Parentprüfung lockern würde falsche Objekte
zulassen; leere Seite als Ende behandeln würde Inhalte verlieren; HTTP 400
pauschal ignorieren würde echte Providerfehler maskieren. Keine Option erfüllt
den Erhaltungsvertrag.

## D — Endgültiger Meilensteinplan vor Quelländerung

| Teil | Dateien | Abhängigkeit | Konkretes erwartetes Ergebnis |
|---|---|---|---|
| D1 Explizite vollständige Scanfixture | `gui_task_tests.rs`, neue `scan_task_tests.rs`, eng begrenzte Queryerwartungen in `gui_task_http.rs` | Belegte Root-/Child-Aufrufkette | Der vorhandene F17-Fall verfolgt fehlende, `null`- und leere Zwischenlisten bis zum Folder, validiert die genaue ID und scannt dessen leere Kinder ohne Fehler, Auslassung oder Trunkierung. Root und beide Kind-ID-Proben werden wirklich gesendet. |
| D2 Namen, IDs und echte Fehler | dieselben Testdateien | D1 | Ein paginierter Scan erhält reversible Literalnamen und unterscheidbare gleichnamige IDs; abgelehnte Root-/Folgeanfrage und malformed Folgeantwort bleiben Fehlpfade und veröffentlichen keine vorläufigen Dateien. `find_child` und `named_objects` folgen ebenfalls einer leeren Seite. |
| D3 Autorenreview und Übergabe | dieser Bericht; Main registriert neue Fallnamen | D1 und D2 | Eigene statische Prüfung gegen alle oben genannten Verträge, unveränderter Produktpager, vollständige Lese-/Änderungsliste und präzise offene Remote-Fragen. Kein lokaler Laufzeitpass. |

## Recherchepass 2: geschlossene Planlücken

Die zusätzliche Quelllektüre bestätigt: `promotion_api::named_objects` löst
`root` ebenfalls tatsächlich auf; `BindingStore` verwendet bei `root=None`
validierte Speichertransaktionen; `auth::get_json` sendet keinen zusätzlichen
Accountrequest bei nicht verwalteten, nicht abgelaufenen Testtoken. Der Titel
`TITLE` enthält `/` und bleibt deshalb im Browser über `names::encode`
adressierbar; beim Betreten entsteht die bestehende exakte Bindung.

Die geplante Erfolgsfolge ist damit vollständig: Root-ID-GET, drei leere
Rootseiten mit jeweils frischem Folgetoken, terminale Rootseite mit Ordner,
exakter Kind-ID-GET, leere Kindliste, erneuter exakter Kind-ID-GET. Der Server
muss falsche Parentqueries, falsche Tokens, fehlende ID-Proben und zusätzliche
Requests als Fehler behandeln. Für Fehlerfälle wird ein terminaler
403-Permissionsfehler verwendet; der vorhandene Congestion-/Retryvertrag wird
nicht durch künstlich zu kurze Antwortskripte umgangen.

## Erweiterung D4 vor deren Quelländerung

Main hat am 2026-10-26 zusätzlich fünf im selben Quellumfang konkret belegte
Root-/Parent-/ID-Fixturefehler zur Behebung freigegeben. Die folgenden
vollständigen bestehenden Fallnamen bleiben erhalten:

- `gui_design_task_drive_recursive_scan_stat_read_and_trash_use_exact_ids`
- `gui_design_task_drive_uncached_paged_markers_and_reloaded_cache`
- `gui_design_task_drive_repeated_or_incomplete_pages_fail_closed`
- `gui_design_task_drive_folder_and_file_creation_decode_metadata_once`
- `gui_design_task_drive_optional_empty_pages_agree_at_read_and_mutation_boundaries`

Ihre belegten fehlenden ersten Aufrufe: Der rekursive Scan und die
Leerantwortprüfung erreichen `list_dir` und damit `resolve`; die
Cache-/Markerprüfung ruft `resolve` direkt; die Wiederholungsprüfung ruft
`find_child` und damit `resolve_bound_folder`; die Ordneranlage erreicht
`ensure_dir("")`. Jeder Pfad verlangt zuerst den bereits oben belegten
Root-ID-GET, den sein vorhandenes Skript nicht liefert.

Erweiterter Vorplan: die vollständigen Aufrufketten für die vorhandene
Ordner-/Dateianlage und das spätere Lesen/Trashen gezielt ergänzen; die
Token-/Incomplete-Fälle gegen alle tatsächlich vorhandenen begrenzten
Neustartversuche skripten; bei ungültiger Abwesenheitsantwort jede
Create-/Uploadmutation weiterhin verbieten. Die bestehenden Schutzorakel und
vollständigen Fallnamen bleiben erhalten. Es gibt keine automatische
Rootantwort im Server und keine Änderung der Produktlogik. Die drei
zusammengehörigen Paging-/Leerantwortfälle werden nach ausdrücklicher
Scopefreigabe nach `paging_boundary_task_tests.rs` ausgelagert; die GUI-Datei
behält ihre benannten Testwrapper.

Die bereits gültigen APIquellen und D1–D3 blieben Grundlage. Vor der folgenden
D4-Lückenrunde waren die genauen Create-/Upload-/Trash-Aufrufe und die
zugehörigen Primärquellen noch unbekannt. Main wurde vor Nutzung über jeden zusätzlich erforderlichen
Lesepfad und die konkrete neue Auslagerungsdatei informiert.

Zweiter Lückenpass D4 vor Änderung: Die sieben ergänzten Quelldateien und die
neue Auslagerungsdatei wurden ausdrücklich freigegeben. `transfer::upload_spooled`
erhält den Parent aus `ensure_dir`, verwendet die frische Namensabfrage und
reservierte IDs; `resumable` bestätigt die Antwort-ID der PUT-Antwort.
`trash::validate_trash_path` löst den Parent erneut auf und verlangt vor dem
PATCH ein `exact_named_object`. Dadurch braucht der vorhandene
Scan-/Stat-/Read-/Trashfall neben seinem Rootlisting wiederholte exakte
Ordnerproben für jeden Auflösungsschritt. Die Fixture erhält diese Proben
ausdrücklich, statt sie im Produkt zu überspringen.

Zusätzliche Primärquellen am 2026-10-26:
[files.generateIds](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/generateIds)
bindet die Anzahl der erzeugten IDs an `count`;
[resumable Upload](https://developers.google.com/workspace/drive/api/guides/manage-uploads)
liefert die Sitzungs-URI über `Location` nach erfolgreicher Initiierung;
[files.update](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/update)
trennt Metadaten-PATCH und Upload-PATCH. Die neue Anlagefixture liefert einen
vollständigen Zehnerpool passend zum aktuellen ersten Request und erhält die
ID-Wiederverwendung zwischen Ordner und Datei. Der Inhalt bleibt `abc`, die
Titel bleiben nach genau einer Dekodierung `I/O` und `Daten%2F.csv`.

Endgültige D4-Abnahme: jeder der fünf benannten Fälle erreicht seine bisherigen
Schutzorakel mit tatsächlichen Parentmetadaten. Wiederholte Tokens erhalten
sechs vollständige Versuche und enden am echten `InvalidData`-Pagerfehler;
unvollständige Suche erhält sechs Versuche mit fünf expliziten Corpusproben.
Die Testkonfiguration von `overload::pause` überspringt bereits die Wartezeit;
der Produktretryvertrag und die Versuchsgrenze werden nicht geändert.
Leer-/Invalidantworten bleiben an Browser-, Auflösungs- und Mutationseinstieg
gleich interpretiert; fehlgeschlagener Writer darf weder ID noch Upload
erzeugen. Die Aufrufkarte erweitert sich ausschließlich um
`metadata::ensure_dir` → `id_pool::take_generated_id` →
`folder_create_journal` → `submit_reserved_folder` sowie
`transfer::upload_spooled` → `resumable`/`resumable_session` und
`trash::{trash, validate_trash_path, trash_id_once}` → `promotion_api`.

Ein sechster direkt belegter Rootfixturefehler wird im selben Umfang
korrigiert: `gui_design_task_drive_content_replacement_never_renames_an_alias`.
`DriveWriter::flush` erreicht `upload_spooled` und damit `ensure_dir("")` vor
dem ursprünglichen PATCH. Die Änderung ergänzt nur diesen echten Rootrequest;
das bestehende Orakel, dass die Uploadmetadaten leer bleiben und den Alias
nicht umbenennen, bleibt identisch. Main wurde über diesen direkten Befund
informiert.

Ergänzung D5 vor Änderung: Die ausdrücklich freigegebene Lektüre von
`promotion.rs` und dem tatsächlich in `gdrive/mod.rs` eingebundenen
`os/shared/copy_writer.rs` belegt dieselbe fehlende Rootauflösung in
`gui_design_task_drive_rename_and_copy_promotion_preserve_original_titles`.
`rename_serialized` löst zuerst den Quellparent auf; `CopyWriter::commit`
ermittelt zuerst seinen Parent über `ensure_dir`. Der ergänzte Erhaltungsplan
liefert jeweils den Root-ID-GET, tatsächliche Parentmetadaten und danach die
bestehenden vollständigen Namens-/ID-/Mtime-/Stagingprüfungen. Die ursprünglichen
Quell-/Zieltitel und der erzeugte Stagingbesitz bleiben unverändert. Die
Copyfixture erhält einen realen Zehner-ID-Pool; der letzte davon verwendete ID
bleibt `item-id`. Nachprüfungen bleiben vor der Erfolgsaussage bestehen.
Die Kettenkarte erweitert sich um `promotion::{rename_serialized,
promote_staged_file_with_mode, rename_absent_locked}` → `promotion_api` sowie
`os/shared/copy_writer::{commit, finish_verified, verify_stage}` →
`named_objects`, `resumable` und die bestehende Staging-/Unique-ID-Prüfung.

## Umsetzung, Autorenreview und Übergabe

D1–D5 sind als explizite Requestfolgen umgesetzt. Der Produktpager, die
Rootauflösung, die Identitätsvalidierung, die Mutation, die Persistenz und der
Scanner selbst sind unverändert. `gui_task_http::Step::query` ergänzt nur
ausdrückliche Erwartungen an vorhandene Parameter einschließlich Abwesenheit
und Eindeutigkeit. Schritte ohne Queryerwartung behalten ihre bestehende
Methoden-/Pfadprüfung. Ein falscher Request bleibt HTTP 400 und ein
Fixturefehler; seine Antwort benennt jetzt erwarteten Schritt und tatsächlich
empfangene Methode/URL, damit die Ursache nicht hinter `{}` verschwindet.
Es gibt keine implizite erfolgreiche Root-, ID- oder Listenanfrage.

Die ursprünglichen Testfunktionen bleiben in `gui_task_tests.rs`; drei
Paging-/Leerantwortkörper liegen nun in `paging_boundary_task_tests.rs`, die
Scanorakel und gemeinsamen expliziten Requestkonstruktoren in
`scan_task_tests.rs`. Die [Rust-Referenz zum path-Attribut](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute),
am 2026-10-26 gelesen, bestätigt die Dateiauflösung relativ zum enthaltenden
Quellverzeichnis. Es entsteht kein zusätzlicher Testeinstieg.

### Eigene anschließende Anti-Regressionsreview

Diese Review erfolgte nach den Quelländerungen durch denselben Fixautor.
Gegenstand waren der vollständige begrenzte Diff, die neuen Hilfsdateien,
sämtliche oben kartierten aktuellen Aufrufer und die benannten Primärquellen.
Die tatsächlich zu bestätigenden Requestfolgen sind aus diesen Quellen
hergeleitet; keine davon wird als bereits aufgezeichnete erfolgreiche
Netzwerkkommunikation ausgegeben.

| Erhaltungsvertrag | Konkreter Abgleich nach Änderung | Grenze des Belegs |
|---|---|---|
| Echte Root-/Parentidentität | Jede berührte Fixture fordert ausdrücklich `files/root?fields=id` an und liefert `scan-root-id`. Alle nachfolgenden Objekte und Listqueries verwenden die tatsächlichen Eltern. Kinddatei im Scan hat Parent `folder-id`. | Statische Request-/Antwortkette; Remoteausführung ausstehend. |
| Vollständige Seitenfolge | F17 enthält fehlendes `files`, `null`, `[]` und anschließend den echten Ordner; jeder Folgetoken sowie das Fehlen des ersten Tokens ist fest gebunden. Auch `find_child` und `named_objects` werden weiter durch denselben Pager geprüft. | Keine Behauptung über ein angemeldetes Google-Konto. |
| Begrenzter Neustart und Fehlerwahrheit | Zyklusfall liefert alle sechs vollständigen Leseversuche; Incompletefall liefert auch die fünf Corpusproben. Erwartet werden die genauen `InvalidData`-Pagerfehler, kein beliebiger Fehler. Fehlerfall für Root-403, Folge-403 und falschen `files`-Typ verlangt einen Fehlpfad und darf vorläufige Einträge nicht ausgeben. | Keine lokale Ausführung; kein geänderter Retry-/Backoffvertrag. |
| Wörtliche Namen und genaue IDs | Scanorakel bindet Pfad, Name und ID; zusätzliche paginierte Dateiliste erhält Slash, Backslash, wörtliches `%2F`, markerähnlichen Literalnamen, Unicode und beide gleichnamigen Datei-IDs. Browserencoding wird reversibel geprüft. | Bestehende `names`-/`duplicates`-Implementierung unverändert. |
| Exakte Auflösung vor Stat/Read/Trash | Rekursivfall liefert zunächst zwei Ordnerproben während des Scans, danach drei für Stat, zwei vor dem Read und drei vor dem Trash. Vor Trash steht außerdem die frische exakte Datei-/Namensprobe; PATCH enthält weiterhin ausschließlich `trashed: true`. | Die statisch hergeleitete vollständige Folge muss remote bestätigt werden. |
| Anlage und Inhaltstausch | Frische Namensabfrage bleibt trotz `listed`-Hint erhalten; zehn generierte IDs liefern zuerst `folder-id`, danach dieselbe reservierte `item-id` für die Datei. Ordner-/Uploadmetadaten behalten Titel, Parent und ID. Inhaltstausch bleibt PATCH derselben ID mit leerem Metadatenobjekt. Ein ungültiger Abwesenheitsbeleg erlaubt keine ID-/Uploadmutation. | Keine Anmeldung, Quota- oder persistente Crashabnahme. |
| Rename und Copy-Promotion | Frische Quell-/Zielabfragen, genaue ID-/Mtimebestätigung und eindeutige Zielnachprobe bleiben im Skript. `CopyWriter::verify_stage` behält Parent-, Namens-, Größen-, MD5-, MIME- und Eindeutigkeitsprüfung. `promotion_checks::{require_one, require_absent, validate_staging_object, verify_unique_id}` bleiben unverändert und werden tatsächlich durchlaufen. | Keine Behauptung einer atomaren Namensreservierung; Drive darf weiterhin gleichnamige Objekte haben. |
| Abbruch, Budgets und Ressourcenabschluss | Scanner-/Backendimplementierung und HTTP-Deadlines/Dropabschluss unverändert. Erfolgsorakel verlangt keine Trunkierung, keinen vorzeitigen Cancel, keine verlorenen Fehlpfade und keine beschädigte DeniedRoots-Liste. Alle Serverinstanzen bleiben dynamisch gebundene Loopbackfixtures mit Speichertoken und Speichercache. | Es wurde kein lokaler Server gestartet. |

Die eigene Review fand keinen notwendigen Produktcodefix. Sie bestätigte als
weitere echte Fixturefehler die ausdrücklich aufgenommenen alten Rootskripte;
deren Schutzorakel wurden erhalten und die Requestfolgen ergänzt. Annahmen
über erfolgreiche Ausführung wurden nicht als Beleg verwendet.

Statische Prüfung: Tree-sitter-Rust meldet für jede der vier geänderten
Rustdateien weder Fehlerknoten noch fehlende Syntaxknoten. Die Dateigrößen
liegen jeweils unter 500 Zeilen und 50 KiB. `git diff --check` der zugewiesenen
getrackten Änderungen liefert keine Leerzeichenfehler. Diese Aussagen sind
Parsing-/Textbelege, keine Compiler-, Typ-, Test- oder Releasebestätigung.

### Geänderte und neu angelegte Dateien — vollständiger Umfang

- Geändert: `native/src/gdrive/core/gui_task_http.rs`.
- Geändert: `native/src/gdrive/core/gui_task_tests.rs`.
- Neu: `native/src/gdrive/core/scan_task_tests.rs`.
- Neu: `native/src/gdrive/core/paging_boundary_task_tests.rs`.
- Neu: `docs/plaene/2026-10-24-batches/fix-f-drive-pagination.md`.

### Gelesene Dateien — vollständige Liste

Die folgende Liste nennt alle gelesenen Dateien; der Inhalt wurde je nach
Auftragsbezug vollständig oder in gezielten Abschnitten gelesen. Graphkanten
ersetzen keinen behaupteten Quelltextbeleg.

Regeln, Scope und Artefakte:

- `/root/.codex/AGENTS.md`
- `/root/.codex/skills/arbeitsweise/SKILL.md`
- `/root/.agents/skills/arbeitsweise/SKILL.md`
- `/root/.codex/skills/graphify/SKILL.md`
- `/root/.codex/skills/graphify/references/query.md`
- `/tmp/smartexplorer-final-review-user-contract.txt`
- `/tmp/smartexplorer-f-drive_pagination-scope.json`, einschließlich der ausdrücklich ergänzten Pfade
- `/tmp/smartexplorer-rv1-40406246032/native-failures.json`, F17
- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`, F17
- `docs/plaene/2026-10-24-batches/fix-f-drive-pagination.md`
- `graphify-out/graph.json`, nur abgegrenzte Abfrage und Metadaten der angefragten Abhängigkeiten
- `native/review-task-manifest.json`, Auswahl und R20-Registrierung

Providerquellen:

- `native/src/gdrive/mod.rs`
- `native/src/gdrive/core/api.rs`
- `native/src/gdrive/core/auth.rs`
- `native/src/gdrive/core/backend.rs`
- `native/src/gdrive/core/cache.rs`
- `native/src/gdrive/core/cache_store.rs`
- `native/src/gdrive/core/core.rs`
- `native/src/gdrive/core/duplicates.rs`
- `native/src/gdrive/core/file_list.rs`
- `native/src/gdrive/core/folder_create_journal.rs`
- `native/src/gdrive/core/folder_reservation.rs`
- `native/src/gdrive/core/gui_task_fixture.rs`
- `native/src/gdrive/core/gui_task_http.rs`
- `native/src/gdrive/core/gui_task_tests.rs`
- `native/src/gdrive/core/http.rs`
- `native/src/gdrive/core/identity.rs`
- `native/src/gdrive/core/id_pool.rs`
- `native/src/gdrive/core/listing.rs`
- `native/src/gdrive/core/listing_query.rs`
- `native/src/gdrive/core/metadata.rs`
- `native/src/gdrive/core/names.rs`
- `native/src/gdrive/core/overload.rs`
- `native/src/gdrive/core/paging_boundary_task_tests.rs`
- `native/src/gdrive/core/promotion.rs`
- `native/src/gdrive/core/promotion_api.rs`
- `native/src/gdrive/core/promotion_checks.rs`
- `native/src/gdrive/core/resolution.rs`
- `native/src/gdrive/core/resumable.rs`
- `native/src/gdrive/core/resumable_session.rs`
- `native/src/gdrive/core/scan_task_tests.rs`
- `native/src/gdrive/core/state.rs`
- `native/src/gdrive/core/sync_bindings.rs`
- `native/src/gdrive/core/sync_projection.rs`
- `native/src/gdrive/core/sync_projection_names.rs`
- `native/src/gdrive/core/sync_reliability_task_fixture.rs`
- `native/src/gdrive/core/sync_reliability_task_paging_tests.rs`
- `native/src/gdrive/core/transfer.rs`
- `native/src/gdrive/core/transfer_ops.rs`
- `native/src/gdrive/core/trash.rs`
- `native/src/gdrive/os/shared/binding_store.rs`
- `native/src/gdrive/os/shared/copy_writer.rs`

Direkt betroffene Scan-/VFS-Verträge:

- `native/src/rscan/os/shared/rscan.rs`
- `native/src/rscan/os/shared/parallel.rs`
- `native/src/rscan/os/shared/walk_state.rs`
- `native/src/scanner/os/shared.rs`
- `native/src/vfs/core/core.rs`

Lesebelege aus Git beschränkten sich auf freigegebene Providerpfade am
gescheiterten Kandidaten und auf `listing.rs` am Releasebaseline
`a814f263096fe69d3746e840b5f3fad3fd7e88a0`. Der Abruf von
`graphify-out/.graphify_python` scheiterte mit `No such file or directory`;
deshalb wurde der erlaubte bestehende Rootgraph direkt im Speicher auf den
freigegebenen Scope gefiltert und abgefragt. Es wurde keine Graphdatei
angelegt, geändert oder neu aufgebaut. Der zunächst fälschlich freigegebene
Core-CopyWriterpfad wurde nicht gelesen; Main korrigierte ihn vor dem Zugriff
auf die tatsächlich eingebundene OS-Datei.

### Konkrete Übergabe an dieselbe Remote-Suite

Main registriert diese erhaltenen bestehenden Namen und die zwei neuen Namen
gezielt für Linux und Windows im selben R20-/Remote-Einstieg:

- `gui_design_task_drive_empty_intermediate_pages_continue_and_empty_children_scan`
- `gui_design_task_drive_recursive_scan_stat_read_and_trash_use_exact_ids`
- `gui_design_task_drive_uncached_paged_markers_and_reloaded_cache`
- `gui_design_task_drive_repeated_or_incomplete_pages_fail_closed`
- `gui_design_task_drive_folder_and_file_creation_decode_metadata_once`
- `gui_design_task_drive_optional_empty_pages_agree_at_read_and_mutation_boundaries`
- `gui_design_task_drive_content_replacement_never_renames_an_alias`
- `gui_design_task_drive_rename_and_copy_promotion_preserve_original_titles`
- Neu: `gui_design_task_drive_paginated_scan_preserves_literal_file_names_and_ids`
- Neu: `gui_design_task_drive_paginated_scan_reports_provider_and_schema_failures`

Offene Laufzeitfrage: Bestätigen die Kandidatenbinärdateien auf beiden
Plattformen exakt die aus dem aktuellen Quelltext hergeleiteten vollständigen
Requests, Literalnamen, IDs und Fehlpfade? Dieser Autor hat keinen lokalen
Build, Compiler, Formatter, Test oder Server ausgeführt und keinen Remotejob
ausgelöst. Ein reales Google-Konto wurde nicht verwendet; die bestehende
Anmelde-/Providerabnahme wird weder als bestanden noch als Releasevoraussetzung
ausgegeben. Es besteht keine offene benötigte Scopeabhängigkeit. Main besitzt
die unveränderte gemeinsame Manifest-/Suite-/Graph-/Git-/Releaseintegration.
