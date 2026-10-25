# B2: statischer kritischer Review

Stand: 2026-10-25. Gegenstand: P11 Stufe 1, P14 und P24 aus [batches.md, D2](batches.md#d2--kopieren-und-gezieltes-erneutes-lesen) sowie der Copy-/Aliasvertrag aus [vertraege.md](vertraege.md). Vergleichsbasis des ursprünglichen Reviews: `bea312ea`; damals abgelesener HEAD: `3c24152a`; HEAD der ersten drei Nachreviews: `7442afc0`; zuletzt abgelesener HEAD beim engen N11-Abschluss: `126f9325`. Die Prüfung betrifft jeweils den gemeinsamen Arbeitsbaum einschließlich noch nicht eingecheckter Integration. Die während des ursprünglichen Reviews vom Hauptagenten vorgenommene Korrektur ist bei B2-R1 ausdrücklich vermerkt.

**Aktueller Nachreviewstand, 2026-10-25, 23:05 UTC:** B2-R1 bis B2-R3 und B2-N1 bis B2-N11 sind im jeweils nachgelesenen Umfang statisch geschlossen. Der enge N11-Abschluss bestätigt die gebundene Mount-/Volumenprüfung vor Dispatch, die dauerhafte negative Autorisierung und den wieder erreichbaren Stagefallback. Im geprüften Umfang bleibt kein bestätigter Defekt offen. Fehlende Plattformfähigkeiten und unklare Mutationsantworten bleiben konservativ begrenzt; daraus folgt keine allgemeine Recoveryunterstützung jedes Dateisystems. Maßgeblich ist der letzte Abschnitt „Vierter Nachreview“. Die ursprünglichen P24-, Ordner- und Wallpapernachweise bleiben erhalten. Keine Laufzeit- oder Compilerabnahme.

## Ursprünglicher Review: Auftrag, Prüffragen und Entscheidungen

Die folgenden ursprünglichen Abschnitte bis zum datierten Nachreview bewahren die damaligen Belege. Ihre Zustandsaussagen sind durch den Nachreview nur dort ersetzt, wo dies ausdrücklich vermerkt ist.

Nutzervorgaben vom 2026-10-25 laut delegiertem Auftrag: „aufgrund des umfangs der aufgaben, ohne dass das weiteres vorgehen blockt, will ich dass du danach n astra xhigh die sachen kritisch reviewn lässt. per Batch“; Präzisierung: „also rein codeseitig und gegen quellen reviewen lässt“.

Der Review beantwortet drei Fragen: Teilt jeder belegte optische In-App-Leseweg die Geräteerlaubnis? Bleiben Ordnerentscheidung, Dateientscheidung, Aliasbesitz und tatsächlicher Dateifortschritt bei Neustart getrennt und korrekt? Ersetzt ein Rechte-Retry ausschließlich die tatsächlich erneut gelesenen Teilbäume unter der passenden Ansichtsgeneration? Die freigegebenen Planstellen und vorhandenen Aufrufer bestimmen diese Fragen; eine vorgegebene Zahl an Befunden wurde nicht verwendet.

Die Arbeitsweise wurde geladen. Ihre MCP-Kontextwerte waren kein Nachweis des ausführenden Modells; unbekannte Anbieter-/Modellkennungen wurden auf unbekannt gesetzt. Dieser Bericht behauptet deshalb keinen selbst überprüften Hostnachweis für Astra/xhigh. Die tatsächliche Ausführungskennung muss der Hauptagent anhand seines Agententranskripts belegen.

Ausgeführt wurden ausschließlich Dateilektüre, eingegrenzte Text-/Diffsuche, Git-Metadatenlektüre, TOML-Parsing für gebundene Versionen und Abrufe von Primärquellen. Keine Produktionsdatei wurde durch diesen Reviewer geändert. Keine Builds, Compiler, Tests, Formatläufe, Server, Installationen, Commits, Pushes oder Releaseaufrufe. Kein Unteragent. Der Graph wurde nicht gelesen oder neu gebaut, weil seine Dateien außerhalb der expliziten Lesefläche liegen.

## Belegte Defekte

### B2-R1 — Reguläre Dateien gerieten in die Ordner-Move-Reservierung

**Schwere:** hoch. **Stand:** statisch geschlossen durch den Hauptagenten; keine Ausführung behauptet.

Fundstellen: `native/src/transfer/os/shared/engine/local.rs::move_whole_roots`, nun Zeilen 269–278; `engine/roots.rs::plan_roots`; `engine/folder_move.rs::move_root`; `vfs/os/shared/local_extensions.rs::local_directory_identity`.

Der ursprüngliche Kontrollfluss prüfte eine vom Standard abweichende Ordnerpolicy vor `plain_dir(&source)`. Desktop- und Androidjobs verwenden standardmäßig `FolderConflict::KeepBoth`. Auch ausgewählte reguläre Dateien verbleiben zunächst in `walk_roots`. Dadurch erreichte ein regulärer Dateipfad `Reservations::move_root`, obwohl dessen Eigentumsbeleg eine Verzeichnisidentität verlangt. Der lokale Adapter liefert für eine reguläre Datei keine solche Identität. Der Fehler „Quellordneridentität für Move ist unbekannt“ führte über `retain_mut` zum Entfernen der Datei aus dem nachfolgenden Walk. Ein gewöhnlicher Datei-Move konnte damit bereits vor dem vorhandenen Einzeldateipfad scheitern.

Die Korrektur setzt `if !plain_dir(&source) { return true; }` unmittelbar hinter die Pfadumwandlung und vor die Ordnerpolicy. Statisch bleibt die reguläre Datei dadurch für den bestehenden Dateitransfer erhalten. Die Ordnerreserve wird weiterhin für tatsächliche Verzeichnisse verwendet. Diese konkrete Kontrollflussänderung wurde im Arbeitsbaum nachgelesen.

### B2-R2 — Persistierte Wiederaufnahme führt den Dateifortschritt nicht weiter

**Schwere:** hoch. **Stand:** im geprüften Belegumfang statisch geschlossen; Abschluss der letzten Grenze B2-N11 siehe vierten Nachreview.

Ursprüngliche Fundstellen: `native/src/transfer/core/job_record.rs::JobRecord::bind`, Zeile 88; `transfer/os/shared/engine/mod.rs::Engine::new`, Zeile 286; `engine/local.rs::copy_file`, Zeilen 29–40 und 65–76; `engine/job_store.rs::verify_sources`, Zeilen 110–123, und `confirmed_move`, Zeilen 128–146. Derselbe getrennte Zustand erscheint in `engine/legacy.rs::run_with_recovery` und `copy/os/shared/bridge.rs::run`.

Der Jobdatensatz stellt beim Wiederöffnen `recovery: RecoveryMode::Resume`, zugleich aber `resume: None` her. `Engine::new` leitet seinen Dateiwiederaufnahmemodus ausschließlich aus `view.resume.is_some()` ab. Der Ordnerreservierungsweg berücksichtigt dagegen `RecoveryMode::Resume` und öffnet den gespeicherten Alias. Somit ist der Zielordner derselbe, während seine bereits erfolgreich veröffentlichten Dateien wie bei einem neuen Auftrag behandelt werden.

Konkreter Kopierablauf: Ein Auftrag mit `KeepBoth` reserviert `Fotos (2)` und veröffentlicht darin `a.jpg`. Nach Abbruch vor `b.jpg` wird derselbe Job wiedergeöffnet. Bei unveränderten Quellen und gültigem Alias geht `a.jpg` erneut durch die ursprüngliche Dateipolicy `Conflict::Rename`; der vorhandene erfolgreiche Inhalt führt zu `a (2).jpg`. Der Ordneralias verhindert `Fotos (3)`, verhindert aber keine Doppelkopien seiner Kinder. Bei `Overwrite` kann derselbe fehlende Fortschrittsbeleg unnötige erneute Ersetzungen auslösen; daraus wird hier kein bereits nachgewiesener Datenverlust abgeleitet.

Konkreter Moveablauf: Ein Auftrag enthält zwei ausgewählte reguläre Dateien. Die erste wurde erfolgreich verschoben; die zweite scheitert. Beim Wiederöffnen ist der erste Quellpfad erwartungsgemäß nicht mehr vorhanden. `verify_sources` akzeptiert diesen Zustand nur, wenn `confirmed_move` einen abgeschlossenen Move nachweist. Diese Funktion erkennt bestätigte Ganzordner-Moves über `move_source`/Verzeichnisidentität beziehungsweise den lokalen Merge-Ordnerfall; sie hat keinen passenden Beleg für die erfolgreich verschobene reguläre Datei. Der Auftrag wird deshalb bereits vor den noch ausstehenden Dateien mit „Quellidentität … geändert oder unbestätigt“ abgewiesen.

**Erforderliche Korrektur:** Den tatsächlich veröffentlichten Dateizielnamen und einen belastbaren Abschluss-/Inhalts- und Eigentumsbeleg dauerhaft an den Job binden; diesen Beleg vor Dateiveröffentlichung/Quelllöschung und beim Wiederöffnen konsistent führen. Erfolgreiche Dateien dürfen nicht erneut umbenannt werden. Ein abgeschlossener Datei-Move muss trotz erwartbar fehlendem Quellpfad nachweisbar bleiben. Ein fremd ersetztes Ziel oder ein unbelegt fehlender Quellpfad muss weiterhin zum sicheren Abbruch führen. Nur `engine.resume` auf `true` zu setzen oder vorhandene Ziele allein nach Länge zu überspringen schließt diese Beleglücke nicht. Der gesonderte CLI-Baumpfad mit `resume_existing` und Inhaltsvergleich in `tree_apply.rs`/`tree_spool.rs` ist kein Nachweis für den Enginepfad.

### B2-R3 — Der Desktop-Hintergrundbildreader umgeht die optische Geräteerlaubnis

**Schwere:** mittel. **Stand:** ursprünglicher Befund; im Nachreview vom 2026-10-25 statisch geschlossen, siehe unten.

Ursprüngliche Fundstellen: `native/src/desktop/os/shared/wallpaper_image.rs::spec_for`, Zeilen 100–105, und `decode`, Zeilen 277–287; `native/src/app/core/desktop_surface_wallpaper_ui.rs::start`, Zeilen 147–169.

Der konfigurierte Bildpfad wird unverändert zum `WallpaperSpec::Image`. `start` erzeugt einen Thread, der `wallpaper_image::load` aufruft. `decode` öffnet die Datei unmittelbar mit `std::fs::File::open`, verpackt sie in `BufReader` und lässt `image::ImageReader` daraus lesen und dekodieren. Keiner dieser Aufrufe erwirbt die `source_media`-Geräteerlaubnis oder übergibt einen Abbruchzustand an diesen Leseweg.

Ein konfiguriertes Hintergrundbild auf demselben optischen Gerät kann dadurch parallel zu einem bereits erlaubten Kopier-, Browse- oder Indexleser auf die Disc zugreifen. Ein erneutes Laden verwirft lediglich den bisherigen Empfänger; ein bereits laufender Decoderthread wird nicht abgebrochen. Der Planvertrag „ein aktiver Leser pro physischem Gerät“ ist damit innerhalb der Anwendung nicht vollständig umgesetzt. Der Unterschied zu externen Playern ist relevant: Hier handelt es sich um einen konkret vorhandenen eigenen Dateireader.

**Erforderliche Korrektur:** Den Hintergrundbild-Lesezugriff über dieselbe Geräteerlaubnis mit Hintergrundpriorität, Medienbindung und wirksamem Abbruch führen. Der Decoder benötigt weiterhin seine `BufRead`-/`Seek`-Fähigkeit; der vorhandene `Box<dyn Read + Send>`-Wrapper allein erfüllt diesen Decodervertrag nicht. Hintergrund-I/O muss zwischen begrenzten Operationen zugunsten ausdrücklicher Leser zurückstehen. Das ist aus der API von `image` 0.25.10 und dem bestehenden Leseweg ableitbar, keine gemessene Behauptung über physische Suchzeiten. [ImageReader, beim Abruf Version 0.25.10](https://docs.rs/image/latest/image/struct.ImageReader.html)

## Nachgelesene Verträge und erhaltene Grenzen

| Bereich | Statischer Beleg und Entscheidung |
|---|---|
| Physisches optisches Gerät | Windows verwendet Laufwerkstyp, Volume-/Storagegeräteinformation und Medienzähler; Linux verwendet `st_dev`, sysfs-SCSI-Typ, Boot-ID und `diskseq`. Ein bloßes UDF-Dateisystem genügt dem Code nicht. Bind-/Partitionspfade werden auf den Geräteträger bezogen. |
| Ein Leser und Priorität | `source_media/permit.rs` verwendet getrennte Absichts- und Lesersperren. Vordergrund hält eine gemeinsame Absichtssperre; Hintergrund benötigt eine exklusive Absichtssperre und gibt sie nach Zulassung wieder frei. Geräte sind getrennt. Iteratoren lösen die Erlaubnis zwischen Öffnen/`next`; Dateistreams halten sie bis EOF/Drop. Statusanzeigen sind ausdrücklich pro Prozess beschriftet, kein globaler Jobbesitznachweis. B2-R3 war hier die ursprüngliche Integrationslücke; ihre spätere Schließung steht im Nachreview. |
| Windows-Copy | Der kontrollierte Reader arbeitet mit einer Blockgröße von 1 MiB, einem Vielfachen von 32 KiB. Bei Metadatenfähigkeiten außerhalb dieses Pfads wird der bisherige Copyweg unter derselben Erlaubnis serialisiert. Stageveröffentlichung, bestehende Backup-/Movegrenzen und Cancellation bleiben in der gelesenen Copykette vorhanden. Daraus folgt kein gemessener Durchsatz und keine Bestätigung jedes Dateisystempaars. |
| Ordner- gegenüber Dateikonflikt | `FolderConflictPolicy` bindet Entscheidungen an genaue ausgewählte Quellroots. CLI/MCP behalten Merge als Standard; Desktop/Android fragen mit KeepBoth-Vorgabe. Merge, Skip und KeepBoth bleiben von der Dateipolicy getrennt. Gerüstordner werden nicht als ausgewählter Quellroot neu nummeriert. `selection.rs::folder_conflicts`, Zeilen 36–37 und 52, nimmt bei fehlender Exklusivfähigkeit auch kollisionsfreie Ordner in die Entscheidungsliste auf. Damit bleibt die ausdrücklich auswählbare Merge-/Skipbehandlung im Admissioncode erreichbar. |
| Alias- und Jobbesitz | Alias-Scope enthält Job, Quell-/Zielnamespace, Zielroot und Vaultbezüge. Private Stage, bestätigte Identität, atomare Veröffentlichung und Journalsperre verhindern eine bloße Übernahme aufgrund von Existenz/Leerheit. Unklare erfolgreiche Remotemutationen werden nicht beliebig wiederholt. Job-/Alias-Kontexte prüfen Vaultbindung und Verfügbarkeit. Das ersetzt keinen Dateifortschrittsbeleg, siehe B2-R2. |
| Locator und Verbindung | Im gelesenen Endpoint-/Job-/Mobile-/CLIpfad bleiben lokale/Remoteart, Namespace und Quellenpfad getrennt; mobile Einzelentscheidungen werden gegen die tatsächlich ausgewählten Locators geprüft. Gleiche relative Namen reichen nicht für gleiche Verbindung. Keine pauschale Aussage über Resolver außerhalb der Lesefläche. |
| Links und Abbruch | Die gelesenen Transfer- und Scannergrenzen behandeln Linkauslassungen getrennt vom Dateierfolg; Cancellation wird durch TLS-Kontext beziehungsweise explizite Flags bis zu wartenden optischen Lesern geführt. B2-R3 hatte im ursprünglichen Stand diesen Kontext nicht; der Nachreview aktualisiert den Readerbefund. Unvollständige Teilscans entfernen keine ausgelassenen Nachfahren. |
| P24-Wurzelindex | `DeniedRoots` speichert Arbeitswurzeln getrennt von begrenzten Meldungstexten, wechselt zu einer privaten Spilldatei, hält bei Schreibfehlern den ungeschriebenen Rest im Speicher und fasst geschachtelte Wurzeln zusammen. Größen-/Besuchsmessungen sind getrennte Datensätze. |
| P24-Teilscanner | `open_retry_root` und `read_confined` halten den bereits aufgelösten Wurzelhandle. `partial_scan.rs` liest nur die ausgewählten verweigerten Roots; Sender bleiben abbrechbar. Vollständigkeit verlangt erfolgreiches Ende ohne Fehler/Trunkierung. Die Rootzeile wird vom Scanner selbst emittiert; ein vermuteter Rootverlust durch `merge_entries` wurde deshalb verworfen. |
| P24-Merge | `read_access.rs`/`analytics_access.rs` prüfen Root und Generation sowie den laufenden Ansichts-/Scanstatus. Vollständiger Explorerersatz ist auf den Teilbaum begrenzt; unvollständiger Ersatz erhält bekannte Nachfahren. Die Analyse zieht belegte alte Teilgrößen ab und addiert neue; fehlende Belege für bereits aggregierte Teilbäume werden nicht als Null behandelt. Keine behauptete UI-Laufzeitabnahme für Auswahl, Sortierung, Filter oder offene Baumzweige. |

## APIstände und Primärquellen

Abrufdatum sämtlicher nachstehender Quellen: 2026-10-25. Die Versionen stammen aus den gelesenen `native/Cargo.toml`-/`Cargo.lock`-Einträgen, nicht aus einem ausgeführten Compiler: direkte Windowsbindungen `windows-sys` 0.59.0 und `windows` 0.58.0; `smb2` 0.26.0; `suppaftp` 10.0.2; `ureq` 2.12.1; direktes `roxmltree` 0.20.0; `image` 0.25.10. Der Lock enthält zusätzlich Windows-/XML-Versionen transitiver Abhängigkeiten; ihnen wird hier kein direkter APIaufruf zugeschrieben. Die installierte Betriebssystem-/Treiber-/Serversoftware wurde nicht erhoben.

| Primärquelle / Stand | Für die Bewertung verwendeter Vertrag |
|---|---|
| [GetFinalPathNameByHandleW, Win32](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew) | GUID-/Opened-Pfadflags; keine allgemeine GUIDverfügbarkeit für Netzwerkfreigaben. Die Probe darf fehlende Gerätebelege nicht zu optischer Gewissheit machen. |
| [GetVolumeInformationByHandleW, Win32](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getvolumeinformationbyhandlew) | Volumeattribute für ACLs, Objekt-IDs, Verschlüsselung, benannte Streams und erweiterte Attribute; Grundlage der Auswahl des metadatenerhaltenden Copywegs. |
| [IOCTL_STORAGE_CHECK_VERIFY, WDK](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddstor/ni-ntddstor-ioctl_storage_check_verify) | Optionaler Medienwechselzähler; Statusverhalten bei gemeldetem Wechsel. Das dokumentiert den APIvertrag, nicht die Zuverlässigkeit eines nicht untersuchten physischen Laufwerks. |
| [Linux v6.12, block/disk-events.c](https://github.com/torvalds/linux/blob/v6.12/block/disk-events.c) und [sr.c](https://github.com/torvalds/linux/blob/v6.12/drivers/scsi/sr.c) | `disk_check_events` erhöht `diskseq` bei `DISK_EVENT_MEDIA_CHANGE`; der SCSI-CD-Öffnungspfad prüft Medienänderung. Die Hypothese, `diskseq` bezeichne ausschließlich das Anschließen des Laufwerks, ist für diesen geprüften Kernelpfad widerlegt. |
| [RFC 3659, §7.5.2, März 2007](https://www.rfc-editor.org/rfc/rfc3659.html#section-7.5.2) | MLST-`Unique` ist mindestens für die Lebensdauer der Kontrollverbindung stabil. Das genügt nicht als allgemeine Garantie über Neustart/Reconnect; der FTPadapter behauptet daraus keinen dauerhaften Ordnerbesitz. |
| [SFTP v3, draft-ietf-secsh-filexfer-02, Oktober 2001, §5](https://datatracker.ietf.org/doc/html/draft-ietf-secsh-filexfer-02#section-5) | Die v3-ATTRS enthalten keinen standardisierten dauerhaften Objektbeleg. Dies ist der historische Protokollentwurf dieses Versionsstands, kein behaupteter moderner Internetstandard. |
| [Google Drive API v3, Files](https://developers.google.com/workspace/drive/api/reference/rest/v3/files) | Namen sind innerhalb eines Ordners nicht zwingend eindeutig. Der Adapter erfindet keine exklusive Anlagegarantie aus der Suche nach einem freien Namen. |
| [RFC 5842, April 2010, §2.5/§6](https://www.rfc-editor.org/rfc/rfc5842.html) | `DAV:resource-id` und atomare REBIND-Semantik; REBIND mit `Overwrite: F` ist eine konkrete Zusatzfähigkeit. Normales WebDAV-MOVE ist kein automatisch gleichwertiger Beleg. RFCstatus: Experimental. |
| [MS-FSCC FileIdInformation, §2.4.26, Seitenstand 2025-06-10](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/e4185a8a-ed8d-4f98-ab55-ca34dc8916e6) | Acht Byte VolumeSerialNumber plus 16 Byte FileId. Nicht unterstützte 128-Bit-IDs sind Null und müssen ignoriert werden; der SMBadapter weist solche unbrauchbaren Werte ab. |
| [ImageReader, gerenderter Cratestand 0.25.10](https://docs.rs/image/latest/image/struct.ImageReader.html) | Der übergebene Reader liefert die Daten und muss die benötigten `BufRead`-/`Seek`-Eigenschaften erhalten; eine Optiksperre entsteht nicht durch den Decoder. Der direkte versionsbezogene docs.rs-Pfad war beim Abruf nicht erreichbar; die geöffnete Hersteller-Cratedokumentation zeigte ausdrücklich 0.25.10. |

Die ersten Raw-GitHub-Abrufe des Linuxquelltexts lieferten unvollständig durchsuchbare Ergebnisse; weitere Rawabrufe wurden mit HTTP 429 abgewiesen. Die relevante Entscheidung wurde deshalb gegen die zugänglichen, auf `v6.12` gebundenen GitHub-Dateiseiten getroffen. Kein Fehler wurde aus einem leeren Suchergebnis abgeleitet.

## Verbleibende Grenzen

- Keine physische CD/DVD, kein Windowslaufwerk und kein live angebundener FTP-, SFTP-, SMB-, WebDAV-, Google-Drive- oder Sharepeer wurde verwendet. Durchsatz, Treiber-Medienwechsel, Netzwerkabbrüche und tatsächliche Serversupportstände bleiben ungemessen.
- Geräteerlaubnisse koordinieren die hier angebundenen SmartExplorer-Prozesse desselben Benutzerkontexts. Externe Desktop-Medienplayer besitzen diese Erlaubnis nicht. `docs/oberflaechen/medien.md` beschreibt die Desktop-Dateimedien ausdrücklich als extern geöffnet; daraus wird kein eigener Desktop-Previewreader erfunden.
- FTP/SFTP besitzen im gelesenen Adapter keinen dauerhaften Verzeichnisbeleg; Google Drive keinen belegten exklusiven Geschwisternamen. WebDAV benötigt die tatsächlich angebotenen Bindingfähigkeiten; SMB braucht verwertbare Dateiidentitäten; Agent/Share benötigen die ausgehandelte Erweiterung. Sichere Ablehnung dieser Fähigkeiten ist eine APIgrenze, kein erfolgreicher Kompatibilitätsnachweis.
- Die ergänzend bestätigte Fundstelle `engine/selection.rs::FolderPreviewScan` wurde vollständig nachgelesen. Die vorübergehend offene Frage zur Vorschau kollisionsfreier Ordner ohne Exklusivfähigkeit ist durch `folder_conflicts` geschlossen; daraus wird kein zusätzlicher Defekt abgeleitet. Die Primärliste wird vollständig gesetzt, bevor `ready` erscheint; anschließende Zählung aktualisiert bestehende Indizes und erweitert die Entscheidungsliste nicht.
- Die bestehenden lokalen Handle-/Rechteadapter, vollständigen Remoteprotokollklassen, UI-Neuberechnung sowie Watcheraufrufer außerhalb des Inventars wurden nicht zusätzlich geöffnet. Aussagen dieses Reviews gelten für die beschriebenen Übergänge in der freigegebenen Fläche, nicht für eine vollständige Repositoryabnahme.
- Im ursprünglichen Review benötigten B2-R2 und B2-R3 Produktionskorrekturen. Der datierte Nachreview unten aktualisiert diese Bewertung. Fehlende Hardware-/Providerabnahme bleibt getrennt von den belegten Codefehlern.

## Vollständiges Lese- und Schreibinventar

Die folgenden Pfade wurden als vollständiger Inhalt, B2-bezogener Auszug, gezielter Symboltreffer oder freigegebener Diff gegen `bea312ea` gelesen. Die Liste behauptet keine Einzelzeilenabnahme jeder nicht betroffenen Funktion. Cargo-Dateien wurden auf Versions-/Featureeinträge begrenzt; umfangreiche CLI-, Copy- und Testdateien auf den betroffenen Kontrollfluss beziehungsweise die geänderten Stellen. Testquellen wurden ausschließlich gelesen.

Ursprüngliche Freigabe: genau die 104 Dateien aus `/tmp/smartexplorer-review-b2-scope.json`; diese Scope-Datei wurde selbst gelesen:

- `AGENTS.md`
- `android/app/src/main/java/app/smartexplorer/android/api/FilesApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/files/FileActions.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/files/FileDialogs.kt`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/batches.md`
- `docs/plaene/2026-10-24-batches/recherche.md`
- `docs/plaene/2026-10-24-batches/vertraege.md`
- `native/Cargo.lock`
- `native/Cargo.toml`
- `native/src/agent/core/folder_reservation.rs`
- `native/src/agent_proto/os/linux_os/folder_reservation.rs`
- `native/src/agent_proto/os/windows/folder_reservation.rs`
- `native/src/app/core/drive_snapshot.rs`
- `native/src/app/core/folder_conflict_ui.rs`
- `native/src/app/core/optical_status.rs`
- `native/src/app/core/preview_core.rs`
- `native/src/app/core/scanning.rs`
- `native/src/app/core/transfer_route.rs`
- `native/src/app/drive_modules.rs`
- `native/src/app/os/shared/optical_index.rs`
- `native/src/cli/engine_copy.rs`
- `native/src/cli/folder_conflict.rs`
- `native/src/cli/mcp/transfer_tools.rs`
- `native/src/cli/mod.rs`
- `native/src/cli/ops.rs`
- `native/src/cli/ops_copy.rs`
- `native/src/cli/ops_move.rs`
- `native/src/cli/optical_status.rs`
- `native/src/cli/transfer.rs`
- `native/src/cli/tree_apply.rs`
- `native/src/cli/tree_observer.rs`
- `native/src/cli/tree_ops.rs`
- `native/src/cli/tree_plan.rs`
- `native/src/cli/tree_remove.rs`
- `native/src/cli/tree_spool.rs`
- `native/src/copy/mod.rs`
- `native/src/copy/os/shared/bridge.rs`
- `native/src/copy/os/shared/copy.rs`
- `native/src/copy/os/shared/pairs.rs`
- `native/src/copy/os/shared/server_copy.rs`
- `native/src/copy/os/shared/staging.rs`
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/windows_optical.rs`
- `native/src/desktop/os/shared/desktop_ops.rs`
- `native/src/folder_index/os/shared.rs`
- `native/src/folder_index/os/walk.rs`
- `native/src/ftp/core/folder_reservation.rs`
- `native/src/gdrive/core/folder_reservation.rs`
- `native/src/mobile/os/shared/dispatch.rs`
- `native/src/mobile/os/shared/domains/mod.rs`
- `native/src/mobile/os/shared/drive.rs`
- `native/src/mobile/os/shared/fs_edit.rs`
- `native/src/mobile/os/shared/transfer.rs`
- `native/src/sftp/core/folder_reservation.rs`
- `native/src/share/core/folder_reservation.rs`
- `native/src/smb/core/folder_reservation.rs`
- `native/src/source_media/mod.rs`
- `native/src/source_media/os/linux.rs`
- `native/src/source_media/os/unsupported.rs`
- `native/src/source_media/os/windows.rs`
- `native/src/source_media/permit.rs`
- `native/src/source_media/reader.rs`
- `native/src/source_media/status.rs`
- `native/src/source_media/test_profiles.rs`
- `native/src/source_media/tests.rs`
- `native/src/transfer/core/folder_alias.rs`
- `native/src/transfer/core/folder_conflict.rs`
- `native/src/transfer/core/job.rs`
- `native/src/transfer/core/job_record.rs`
- `native/src/transfer/mod.rs`
- `native/src/transfer/os/shared/engine/folder_alias_store.rs`
- `native/src/transfer/os/shared/engine/folder_move.rs`
- `native/src/transfer/os/shared/engine/folder_reservation.rs`
- `native/src/transfer/os/shared/engine/folders.rs`
- `native/src/transfer/os/shared/engine/folders_local.rs`
- `native/src/transfer/os/shared/engine/job_store.rs`
- `native/src/transfer/os/shared/engine/legacy.rs`
- `native/src/transfer/os/shared/engine/local.rs`
- `native/src/transfer/os/shared/engine/mod.rs`
- `native/src/transfer/os/shared/engine/ops.rs`
- `native/src/transfer/os/shared/engine/record_io.rs`
- `native/src/transfer/os/shared/engine/roots.rs`
- `native/src/transfer/os/shared/engine/segments_tests.rs`
- `native/src/transfer/os/shared/engine/segments_upload.rs`
- `native/src/transfer/os/shared/engine/selection.rs`
- `native/src/transfer/os/shared/engine/source.rs`
- `native/src/transfer/os/shared/engine/test_run.rs`
- `native/src/transfer/os/shared/engine/tests_flow.rs`
- `native/src/transfer/os/shared/engine/view.rs`
- `native/src/transfer/os/shared/engine/worker.rs`
- `native/src/transfer/os/shared/walk.rs`
- `native/src/transfer/os/shared/walk_listers.rs`
- `native/src/vault/os/shared/transfer_blob.rs`
- `native/src/vault/os/shared/transfer_blob_codec.rs`
- `native/src/vault/os/shared/transfer_blob_context.rs`
- `native/src/vfs/core/folder_reservation.rs`
- `native/src/vfs/mod.rs`
- `native/src/vfs/os/shared/confined_local_backend.rs`
- `native/src/vfs/os/shared/confined_local_extensions.rs`
- `native/src/vfs/os/shared/local.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/shared/local_optical.rs`
- `native/src/webdav/core/folder_reservation.rs`

Zusätzlich ausdrücklich freigegebene und gelesene Dateien:

- `docs/oberflaechen/INDEX.md`
- `native/src/app/core/read_access.rs`
- `native/src/app/core/analytics_access.rs`
- `native/src/app/core/partial_scan.rs`
- `native/src/scanner/os/collect.rs`
- `native/src/analytics/os/shared/analytics_outcome.rs`
- `native/src/analytics/core/partial_merge.rs`
- `native/src/scanner/os/denied_roots.rs`
- `native/src/scanner/os/retry.rs`
- `native/src/scanner/os/shared.rs`
- `native/src/scanner/os/walk.rs`
- `native/src/desktop/os/shared/wallpaper_image.rs`
- `native/src/app/core/desktop_surface_wallpaper_ui.rs`
- `native/src/app/os/shared/remote_helpers/temp.rs`
- `native/src/vault/os/linux_thumbnails.rs`
- `docs/oberflaechen/medien.md`

Zusätzlich geladene Regelquellen: `/root/.agents/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/graphify/SKILL.md` sowie die über `arbeitsweise.skill_message`/`skill_read` gelieferten zentralen Texte, Revision 337. Das Lesen der Graphify-Regeln hat keinen Graphzugriff ausgelöst. Gitmetadaten: letzter Commit und auf die Berichtdatei begrenzter Arbeitsbaumstatus. Externe Lektüre: die oben verlinkten Primärquellen; erfolglose Abrufe sind ausdrücklich beschrieben.

**Erstellt/geändert durch diesen Reviewer:** ausschließlich `docs/plaene/2026-10-24-batches/review-b2.md`. **Produktionsänderungen durch diesen Reviewer:** keine. **Vom Hauptagenten während des Reviews nachgelesene Produktionskorrektur:** `native/src/transfer/os/shared/engine/local.rs` für B2-R1.


## Nachreview der integrierten B2-Korrekturen — 2026-10-25, 21:37 UTC

**Historischer Befundstand:** Die folgenden sechs Offenmeldungen dokumentieren den damaligen Code. Ihr aktueller Status wird im zweiten Nachreview ausdrücklich neu bewertet; die damaligen Belege bleiben erhalten.

**Basis:** abgelesener HEAD `7442afc0` plus gemeinsamer Arbeitsbaum. Die folgenden Zeilenangaben beziehen sich auf den bis 21:37 UTC festgehaltenen gelesenen Stand. Während dieses Reviews wurden sämtliche sechs nachstehenden Befunde an den Hauptagenten übergeben. Anschließend begonnene Produktionsänderungen sind ohne ausdrücklichen Schließungsvermerk noch nicht nachgeprüft. Der Reviewer hat ausschließlich diese Berichtdatei geändert.

**Erstplan aus dem Ergebnisauftrag:** Die alten Copy-/Move-Wiederaufnahmefälle und den Hintergrundbildreader gegen deren konkrete Übergänge prüfen; Quellenabruf und Berichtaktualisierung sind erlaubt, Produktionsänderungen und Ausführung von Builds/Tests sind ausgeschlossen. Unbekannt waren zunächst die Absturzfenster, die Weitergabe der neuen Objektidentität und die tatsächliche Permit-Lebensdauer.

**Detaillierter Reviewplan nach Bestandslektüre:** Erstens den gespeicherten Zielnamen vom Stagebeleg durch `before`/`after`, Job-Reopen und Rootplanung verfolgen; zweitens Quarantäne, vorhandenes Ziel, Journalindex und verworfene Belege an jedem dauerhaften Übergang unterscheiden; drittens die neuen Dateiidentitäten durch Local, Agent, Daemon, Share, Google Drive und Vault bis zu deren Rechte-/Rootgrenzen verfolgen; viertens Hintergrundpriorität, Medienbindung und Abbruch des Readers gegen `image` und die bestehenden Sperradapter abgleichen. Die zweite Quellenprüfung betraf besonders `flock`/`LockFileEx`, Verzeichnis-Fsync, Inodelebensdauer und den Google-Exportvertrag. Erwartete Nachweise sind jeweils unten benannt; kein Prüfszenario wurde ausgeführt.

### Bereits geschlossene ursprüngliche Fälle

- **B2-R1:** `local.rs::move_whole_roots` prüft weiterhin `plain_dir` vor der Ordnerreservierung. Der Dateipfad bleibt erhalten.
- **B2-R2, ursprüngliche normale Kopierfolge:** `file_recovery::Receipt` bindet Quelle, beide gespeicherten Endpunkte, Job, Zielroot, tatsächlichen Zielnamen, Objektbeleg und SHA-256. `ops::transfer` fragt den Beleg vor der ursprünglichen Dateipolicy ab. Ein erfolgreich veröffentlichtes `a (2).jpg` geht damit bei gültigen Belegen nicht nochmals durch die Namensvergabe. Das ist eine statische Kontrollflussaussage mit den verbleibenden Grenzen N1–N6.
- **B2-R2, ursprüngliche abgeschlossene Einzeldatei-Moves:** `verify_saved` liefert bestätigte Datei-Roots; `job_store::verify_sources` akzeptiert den erwartbar fehlenden ausgewählten Quellpfad nur mit passendem gespeicherten Quellbeleg und Zielbeleg. `roots::plan` nimmt diesen Root aus dem erneuten Walk. Explizite Paare verwenden einen Schlüssel aus Quellpfad und ursprünglichem Relativpfad.
- **B2-R3:** `wallpaper_image.rs::decode` verwendet `BufReader<WallpaperReader>`. `wallpaper_source.rs::open` erwirbt Hintergrundpriorität vor dem Öffnen, prüft den geöffneten Handle gegen das Medium und gibt die Erlaubnis wieder frei. `access` erwirbt sie für jeden `Read`-/`Seek`-Schritt erneut und prüft anschließend die Medienbindung. `Read` begrenzt den Aufruf auf `OPTICAL_BLOCK_BYTES`. `Wallpaper::reload`, `start` und `Drop` setzen den bisherigen Abbruchzustand. `ConnectionAborted` verhindert, dass die explizite Cancellation als gewöhnliches `Interrupted` endlos wiederholt wird. `BufRead`/`Seek` bleiben durch die Zusammensetzung erhalten; dies entspricht dem dokumentierten Readervertrag. Eine bereits laufende Betriebssystem-Leseoperation und reine Decoder-/Resize-CPUarbeit werden nicht präemptiv unterbrochen; die Begrenzung gilt an den nachgelesenen I/O- und Abschlussgrenzen. [ImageReader 0.25.10](https://docs.rs/image/latest/image/struct.ImageReader.html)

### B2-N1 — Optische Dateibelege erwerben den eigenen Gerätelock erneut

**Schwere:** hoch. **Stand im gelesenen Code:** offen; Hauptagent hat Korrektur beauftragt.

**Fundstellen:** `engine/file_recovery.rs::hash`, Zeilen 79–100; `::before`, Zeilen 302–307; `engine/upload.rs::upload`, Zeilen 20–65; `engine/source.rs::LocalSource`; `engine/job_store.rs::snapshot`; `vfs/os/shared/local.rs::stat`, Zeilen 194–196; `vfs/os/shared/local_extensions.rs::open_read_regular`, Zeilen 51–54; `source_media/reader.rs::guard_reader`; `source_media/permit.rs::acquire_checked`.

Zwei auslösende Folgen sind belegt:

1. Die Wiederaufnahme einer erfolgreich kopierten Datei von CD/DVD ruft `verify_source` und dann `hash` auf. `hash` hält eine eigene Vordergrunderlaubnis und ruft `vfs::open_read_regular` auf. Der lokale Adapter erwirbt durch `guard_reader` dieselbe exklusive Gerätesperre über einen weiteren geöffneten Lockhandle. Die äußere Erlaubnis wird erst nach Rückkehr der inneren Anfrage freigegeben. Der Aufruf kommt deshalb ohne Cancellation nicht weiter.
2. Ein normaler journalisierter CD/DVD-Upload hält `LocalSource.optical` bis zum Ende von `upload`. Nach dem Datenstream erreicht `publish::publish` den neuen `file_recovery::before`-Hook. Dessen `snapshot` ruft `LocalBackend::stat` auf, das seinerseits die optische Erlaubnis fordert. Die Datei bleibt bereits vor Veröffentlichung stehen. Das Ausschalten des Fresh-Pfads für optische Quellen beseitigt diese zweite Reentranz nicht.

Linux `flock` behandelt getrennte Opens desselben Lockfiles unabhängig und kann durch den eigenen bereits gehaltenen Lock abgewiesen werden. Auch Windows schließt überlappende exklusive Locks aus. Das sind dokumentierte Sperrverträge, kein gemessener Stillstand. [Linux flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html), [Microsoft LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex)

**Erforderlicher Nachweis:** Genau eine Erlaubnis je tatsächlich aktivem optischen Reader; Quellmetadaten und Publikationsbelege nach Freigabe oder über den bereits autorisierten Handle. Die spätere gemeinsame Suite muss normalen optischen Upload und Reopen mit bereits veröffentlichtem Discinhalt berücksichtigen.

### B2-N2 — Beleg und Journalzähler besitzen keinen wiederanlaufbaren gemeinsamen Übergang

**Schwere:** mittel. **Stand im gelesenen Code:** offen.

**Fundstellen:** `engine/file_recovery.rs::JobRun::start_file`, Zeilen 267–283, insbesondere 278–281; `::visit_saved`, Zeilen 140–158; `engine/record_io.rs::write_record`.

Ein neuer Schlüssel wird zuerst als eigene JSON-Datei geschrieben; erst danach wird `index.json.count` erhöht und separat geschrieben. Ein Prozessende oder Schreibfehler zwischen diesen beiden Aufrufen hinterlässt einen gültigen vorbereiteten Dateibeleg mit altem Index. Der Hook hat noch nicht erfolgreich zurückgegeben, die eigentliche Veröffentlichung muss also noch gar nicht stattgefunden haben. `visit_saved` prüft jedoch zwingend die Anzahl gegen den alten Index und sperrt jede Wiederaufnahme. Die Atomarität der beiden Einzeldateien macht ihren Übergang nicht zu einer gemeinsamen Transaktion.

**Erforderlicher Nachweis:** Wiederanlauf nach jedem der beiden dauerhaften Schreibschritte und nach einem Fehler des zweiten Schritts, bei weiterhin sicherer Erkennung eines tatsächlich fehlenden Abschlussbelegs. Der vorhandene Test zur entfernten Receiptdatei deckt diesen eigenen Anlagetransient nicht ab. Für die Reihenfolge ist der Projektcode der unmittelbare Beleg; ein externer Dienstvertrag ist nicht beteiligt.

### B2-N3 — Vor Veröffentlichung existierende Ziele werden als fremd ersetzter Erfolg behandelt

**Schwere:** mittel. **Stand im gelesenen Code:** offen.

**Fundstellen:** `engine/file_recovery.rs::start_file`, Zeilen 273–276; `::verify_saved`, Zeilen 175–202; `copy/os/shared/safe_file.rs::transfer_local_inner`; `copy/os/shared/direct_move.rs::try_move`.

Konkreter Fall: Ein Move mit Overwrite hat die Quelle bereits in die belegte Quarantäne verschoben; der neue Stageinhalt ist vollständig. `before_publish` schreibt dessen Objektbeleg bereits mit dem endgültigen Zielpfad. Der Prozess endet vor `commit_staged`. Am Ziel liegt deshalb noch die legitime alte Datei. Beim Reopen bewertet `verify_saved` den ungleichen Objektbeleg sofort als Fehler; nur `NotFound` darf bei unbestätigter Veröffentlichung „noch unveröffentlicht“ bedeuten. Die vorhandene eigene Stage und die unveränderte Quarantäne werden nicht mehr zur Entscheidung erreicht. Der Quellname bleibt in dieser Konstellation verschwunden, der Job gesperrt. Auch ein Renamekonflikt vor dessen nächstem nummerierten Versuch besitzt dieses Fenster.

**Erforderlicher Nachweis:** Ein noch vorhandenes Altziel plus eigene unpublizierte Stage muss sicher vom nachträglichen Austausch eines bereits veröffentlichten Ziels unterscheidbar sein. Dabei muss die Stage wirklich der konsumierte Publikationsname sein: Der gelesene Direct-Move-Pfad meldet `source_path` (Quarantäne) als Stage, benennt aber `rename_source` (Hardlinkalias) um. Der Hauptagent hat die Anpassung dieses Hooks angekündigt; ohne nachgelesenen Folgecode ist sie hier noch keine Schließung. Nach tatsächlich erfolgter Veröffentlichung darf eine fremde Zieldatei weiterhin weder übernommen noch ersetzt werden.

### B2-N4 — Exportierte Google-Dokumente geraten beim Reopen in den Regular-File-Reader

**Schwere:** mittel. **Stand im gelesenen Code:** offen.

**Fundstellen:** `engine/file_recovery.rs::verify_source`, `::observe`, `::hash`; `engine/download.rs::download`/`open_reader`; `engine/source.rs::RemoteCheck`; `gdrive/core/extensions.rs::open_read_regular`, Zeilen 63–76.

Der bestehende Downloadpfad unterstützt Exporte mit unbekannter Ausgabelänge über `read_size` und `open_read_id`; `RemoteCheck` unterscheidet diese Ausgaben ausdrücklich von gewöhnlichen gelisteten Dateibytes. Die neue Wiederaufnahme schickt dagegen jeden noch vorhandenen Quellpfad durch `vfs::open_read_regular`. Der Google-Drive-Adapter lehnt dort `application/vnd.google-apps.*` ausdrücklich ab. Ein erfolgreich exportiertes Dokument kann dadurch die Wiederaufnahme seiner ausstehenden Geschwister verhindern. Google beschreibt den Export als eigene Byteausgabe in einem angeforderten MIMEformat. Ein beliebiger erneuter regulärer Datei-Read ist kein Beleg dieses Exports. [Google Drive v3 files.export, Seitenstand 2025-08-26](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/export)

**Erforderlicher Nachweis:** Der Quellenbeleg muss den vorhandenen Exportvertrag erhalten und Quellobjekt/-generation von der konkreten exportierten Ausgabe unterscheiden. Bloßes Zulassen von Ordnern oder Shortcuts im Regular-Reader wäre keine passende Korrektur.

### B2-N5 — Recovery löscht die Movequarantäne vor dauerhafter Bestätigung des Zielnamens

**Schwere:** hoch. **Stand im gelesenen Code:** offen.

**Fundstellen:** `engine/file_recovery.rs::verify_saved`, Zeilen 175–218; `copy/os/shared/recovery.rs::recover_transfer_quarantine`; `copy/os/linux_recovery.rs::recover_transfer_child`; zum erhaltenen regulären Vertrag `copy/os/shared/durability.rs::finish_staged_commit`.

Konkretes Fenster: Ein geräteübergreifender Move veröffentlicht seine bereits synchronisierte Stage; der Prozess endet nach dem Rename, aber vor `finish_staged_commit` und dessen `sync_parent(target)`. Bei einem Prozessneustart ohne Betriebssystemneustart ist der Zielname aus dem Dateisystemcache weiterhin sichtbar. `verify_saved` liest und hasht ihn, löscht anschließend die Quellquarantäne und synchronisiert deren Elternverzeichnis. Eine vorherige dauerhafte Bestätigung des Zielnamens findet auf diesem Recoverypfad nicht statt. Ein anschließender System-/Stromausfall kann deshalb den nicht bestätigten Zielnamen verlieren, während die letzte dauerhafte Quelle bereits entfernt ist. Das ist ein durch die Reihenfolge fehlender Haltbarkeitsvertrag, keine Behauptung eines beobachteten Datenverlusts.

Der reguläre `finish_staged_commit` bestätigt das Zielverzeichnis ausdrücklich vor `remove_quarantine`; die neue Recovery überspringt genau diese Voraussetzung. Dateilesen und selbst Datei-Fsync bestätigen den Verzeichniseintrag nicht automatisch. [Linux fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html)

**Erforderlicher Nachweis:** Vor jeder Recoverylöschung der Quellquarantäne müssen Zielinhalt und Zielnamespace über dieselbe kontrollierte Objekt-/Parentgrenze dauerhaft bestätigt sein; bei Bestätigungsfehler bleibt die Quarantäne bestehen. Der nachgelesene reguläre Direct-Move-Pfad ruft `remove_quarantine` außerdem vor `finish_direct_move` auf; dessen Ziel-Parentfehler darf ebenfalls nicht als erfolgreich gesicherter Abschluss gewertet werden.

### B2-N6 — Linux dev/ino ist kein dauerhafter Beleg gegen Objektwiederverwendung

**Schwere:** mittel. **Stand im gelesenen Code:** offen.

**Fundstellen:** `copy/os/linux_os.rs::stable_path_identity`/`stable_handle_identity`, Zeilen 11–18; `engine/file_recovery.rs::verify`, Zeilen 118–125; `engine/file_recovery_tests.rs::batches_task_p14_reopen_refuses_same_sized_target_mutation_and_equal_foreign_objects`. Der standalone Agent bildet seinen Beleg ebenfalls aus `device` und `object` in `agent_proto/os/shared/file_identity.rs`; für dessen plattformspezifische Objektzahl wurde keine zusätzliche Implementierung außerhalb der Freigabe gelesen.

Der lokale Linuxstring enthält ausschließlich `st_dev` und `st_ino`. Nach Löschung und Freigabe des alten Zielobjekts kann das Dateisystem dieselbe Inodenummer für eine neu angelegte fremde Datei vergeben. Bei gleichen Bytes bestehen dann sämtliche in `verify` verglichenen Werte: `object`, lokale `id=None`, Größe und SHA-256. Die gespeicherte `mtime_ms` wird dort absichtlich nicht verglichen. Damit ist die geforderte Ablehnung eines fremden gleichinhaltlichen Zielobjekts nicht allgemein gewährleistet. Der vorhandene Test verschiebt die alte Datei ausdrücklich nach `retained-owned-object`; dadurch bleibt deren Inode belegt und gerade die Wiederverwendung ausgeschlossen.

Die Kernel-Dokumentation trennt Inodenummer, Generation und Erstellzeit; ein Upstream-Hostfs-Fix beschreibt explizit die Wiederverwendung freier ext4-Inodenummern. Daraus folgt keine Aussage über die Häufigkeit auf einer ungemessenen lokalen Partition. [Linux ext4-Inodes](https://docs.kernel.org/filesystems/ext4/inodes.html), [Kernel-Upstreamfix 0bc754d1e31f40f4a343b692096d9e092ccc0370, veröffentlichter Stable-Patch](https://lore-kernel.gnuweeb.org/stable/20250408104857.550090435%40linuxfoundation.org/)

**Erforderlicher Nachweis:** Ein dauerhafter Beleg muss die Lebensdauer des Objekts unterscheiden oder seine Wiederverwendung durch eine erhaltene eigene Referenz ausschließen. Ein Größen-, Zeit- oder Hashvergleich allein ersetzt diese Objektgrenze nicht. Für Dateisysteme ohne geeigneten Beleg ist die Grenze ausdrücklich unbekannt zu behandeln, ohne daraus eine unbestätigte Eigentumsbehauptung zu machen.

### Nachgelesene Integration und ausdrücklich begrenzte Aussagen

- `batch::members` nimmt `RecoveryMode::Resume` vom Paketpfad aus. Der zunächst vermutete Wiedersende-Bypass bereits abgeschlossener Paketmitglieder ist deshalb kein weiterer Befund. Paketvorbereitung hält unbekannte Antworten konservativ offen.
- `copy::bridge` und `engine::legacy` führen ihren bisherigen öffentlichen Vertrag durch denselben `run_view` mit Job-ID und Recoverymodus; sie umgehen die neuen zentralen Belege nicht.
- Der VFS-Default liefert keine generische Metadata-ID als Eigentumsbeleg. Local und ConfinedLocal verwenden gehaltene lokale Autorität; Cache leitet an den tatsächlichen Provider weiter. Agent und Share handeln eine eigene Fähigkeit aus; Daemon-, Mount- und Rootwrapper wenden ihre bestehenden Zugangsgates an. Die gelesene Share-Erweiterung verlangt für private Stagenamen ein vorhandenes Creatorticket und für deren Hostauflösung Schreibautorität; ein gewöhnlicher Identitätsread verleiht kein Schreibrecht. Vault erhält die Sitzungsprüfung und mappt auf den zugehörigen Inhalt.
- Fehlende Dateiidentität lässt den normalen Transferpfad teilweise zu, aber die spätere Belegübernahme bleibt bewusst abgewiesen. Daraus wird keine vollständige Wiederaufnahmeunterstützung für FTP, SFTP, SMB, WebDAV oder ältere Agent-/Sharepeers behauptet.
- Kein zusätzlicher P24-Code wurde in diesem Nachreview geöffnet. Die ursprünglichen geschlossenen P24-Feststellungen werden erhalten, nicht als erneut ausgeführt ausgegeben.
- Es wurden keine Compiler, Builds, Tests, Formatter, Server, Installationen, Commits, Pushes, Graphrebuilds oder Releases ausgeführt. Die vorhandenen Testquellen sind statische Orakel, keine bestandenen Tests. Physische optische Medien, Windows-Treiber und Liveprovider wurden nicht benutzt.
- Die freigegebene Graphdatei wurde einmal mit `graphify query` abgefragt. Der Graph lieferte hauptsächlich vorhandene Namensnachbarschaften und keinen vollständigen Nachweis der neuen Recoveryimplementierung. Außerhalb der freigegebenen Quelldateien wurden keine angezeigten Graphpfade geöffnet. Die Versionswarnung des Graphify-Skills führte zu keiner Installation.

**Quellenabruf:** 2026-10-25. `image/latest` zeigte ausdrücklich 0.25.10; der direkte versionsgebundene docs.rs-Abruf war nicht zugänglich. Die Open-Group-Seiten für `fsync` und `rename` antworteten mit HTTP 403; die Bewertung verwendet stattdessen Linux-man-pages und Rusts Standardbibliotheksdokumentation. Die direkten GitHub-/kernel.org-Abrufe des genannten Hostfs-Commits waren nicht zugänglich; belegt ist der veröffentlichte Stable-Patch mit Autor und Upstream-Commit-ID. Kein Abruffehler wurde als APIverhalten gewertet.

### Lese- und Schreibinventar dieses Nachreviews

Das Manifest `/tmp/smartexplorer-review-b2-fixes-scope.json` wurde gelesen. Die folgende Liste umfasst vollständige Lektüre, begrenzte Symbol-/Zeilenlektüre und durch den manifestgebundenen Suchlauf tatsächlich eingelesene Dateien; sie behauptet keine Abnahme aller darin enthaltenen Funktionen. Die zusätzlichen Freigaben betreffen `remote_copy.rs`, `durability.rs`, `staging_names.rs`, `linux_os.rs` und `cache_extensions.rs`. `replace.rs` wurde nicht geöffnet: Die gelesene Engine registriert kein solches Modul, ihre Veröffentlichungswege sind oben konkret benannt.

- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/batches.md`
- `docs/plaene/2026-10-24-batches/recherche.md`
- `docs/plaene/2026-10-24-batches/review-b2.md`
- `docs/plaene/2026-10-24-batches/vertraege.md`
- `graphify-out/graph.json`
- `native/src/agent/core/extensions.rs`
- `native/src/agent/core/folder_reservation.rs`
- `native/src/agent/core/route.rs`
- `native/src/agent_proto/core/features.rs`
- `native/src/agent_proto/core/folder_frames.rs`
- `native/src/agent_proto/core/server.rs`
- `native/src/agent_proto/core/types.rs`
- `native/src/agent_proto/mod.rs`
- `native/src/agent_proto/os/linux_os/folder_reservation.rs`
- `native/src/agent_proto/os/shared/file_identity.rs`
- `native/src/agent_proto/os/windows/folder_reservation.rs`
- `native/src/app/core/desktop_surface_wallpaper_ui.rs`
- `native/src/copy/mod.rs`
- `native/src/copy/os/linux_os.rs`
- `native/src/copy/os/linux_recovery.rs`
- `native/src/copy/os/shared/bridge.rs`
- `native/src/copy/os/shared/copy.rs`
- `native/src/copy/os/shared/direct_move.rs`
- `native/src/copy/os/shared/durability.rs`
- `native/src/copy/os/shared/move_guard.rs`
- `native/src/copy/os/shared/pairs.rs`
- `native/src/copy/os/shared/publication.rs`
- `native/src/copy/os/shared/recovery.rs`
- `native/src/copy/os/shared/safe_file.rs`
- `native/src/copy/os/shared/server_copy.rs`
- `native/src/copy/os/shared/staging.rs`
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/windows_identity.rs`
- `native/src/copy/os/windows_recovery.rs`
- `native/src/daemon/os/shared/backend_ops.rs`
- `native/src/daemon/os/shared/backend_server.rs`
- `native/src/daemon/os/shared/ipc_backend_extensions.rs`
- `native/src/daemon/os/shared/mount_proxy_extensions.rs`
- `native/src/daemon/os/shared/rooted_backend_extensions.rs`
- `native/src/desktop/os/shared/wallpaper_image.rs`
- `native/src/desktop/os/shared/wallpaper_source.rs`
- `native/src/gdrive/core/extensions.rs`
- `native/src/gdrive/core/folder_reservation.rs`
- `native/src/local_access/mod.rs`
- `native/src/local_access/os/windows/confined.rs`
- `native/src/share/core/file_identity_rpc.rs`
- `native/src/share/core/fs_access.rs`
- `native/src/share/core/fs_guard_extensions.rs`
- `native/src/share/core/fs_policy.rs`
- `native/src/share/core/fs_request.rs`
- `native/src/share/core/fs_response.rs`
- `native/src/share/core/mount_lease.rs`
- `native/src/share/core/peer_extensions.rs`
- `native/src/share/core/peer_fs_logging.rs`
- `native/src/share/core/peer_request_policy.rs`
- `native/src/share/core/peer_stages.rs`
- `native/src/share/core/peer_transfer.rs`
- `native/src/share/core/server_fs.rs`
- `native/src/share/core/wire_capabilities.rs`
- `native/src/source_media/mod.rs`
- `native/src/source_media/os/linux.rs`
- `native/src/source_media/os/windows.rs`
- `native/src/source_media/permit.rs`
- `native/src/source_media/reader.rs`
- `native/src/transfer/core/job_record.rs`
- `native/src/transfer/os/shared/engine/batch.rs`
- `native/src/transfer/os/shared/engine/batch_put.rs`
- `native/src/transfer/os/shared/engine/download.rs`
- `native/src/transfer/os/shared/engine/file_recovery.rs`
- `native/src/transfer/os/shared/engine/file_recovery_tests.rs`
- `native/src/transfer/os/shared/engine/job_store.rs`
- `native/src/transfer/os/shared/engine/legacy.rs`
- `native/src/transfer/os/shared/engine/local.rs`
- `native/src/transfer/os/shared/engine/mod.rs`
- `native/src/transfer/os/shared/engine/ops.rs`
- `native/src/transfer/os/shared/engine/publish.rs`
- `native/src/transfer/os/shared/engine/record_io.rs`
- `native/src/transfer/os/shared/engine/remote_copy.rs`
- `native/src/transfer/os/shared/engine/roots.rs`
- `native/src/transfer/os/shared/engine/source.rs`
- `native/src/transfer/os/shared/engine/upload.rs`
- `native/src/transfer/os/shared/engine/worker.rs`
- `native/src/vault/os/shared/backend_extensions.rs`
- `native/src/vfs/core/cache_extensions.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/file_identity.rs`
- `native/src/vfs/core/staging_names.rs`
- `native/src/vfs/mod.rs`
- `native/src/vfs/os/shared/confined_local_backend.rs`
- `native/src/vfs/os/shared/confined_local_extensions.rs`
- `native/src/vfs/os/shared/local.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/shared/local_file_identity.rs`
- `native/src/vfs/os/shared/local_optical.rs`

Zusätzliche Regelquellen: `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.agents/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/graphify/SKILL.md` und zentrale MCP-Regeln Revision 337. Gitmetadaten: HEAD und Status ausschließlich der Berichtdatei. **Geändert:** ausschließlich `docs/plaene/2026-10-24-batches/review-b2.md`. **Produktionsänderungen dieses Reviewers:** keine. **Offene Arbeit des Hauptagenten:** N1–N6 beheben beziehungsweise mit konkretem Gegenbeleg auflösen und die tatsächlichen Folgeänderungen vor einer Schließung statisch nachprüfen lassen. Der Hauptagent bestätigte zum Abschluss dieses Reviews, dass N1–N5 sowie die N5-API und N6 noch in Bearbeitung sind; dafür folgt ein gesondert abgegrenzter Nachreview.

## Zweiter Nachreview der Korrekturen B2-N1–N6 — 2026-10-25, 22:25 UTC

**Historischer Befundstand:** Die Offenmeldungen dieses Abschnitts gelten für den damaligen Code. Der dritte Nachreview bewertet N1 und N7–N10 erneut; ihre damaligen Belege bleiben erhalten.

**Basis und Umfang:** HEAD `7442afc0` mit dem von den Autoren abgeschlossenen gemeinsamen Arbeitsbaum. Der Hauptagent hielt die betroffene Copy-/Enginefläche bis zum Reviewabschluss unverändert. Lesefreigabe: das aktualisierte Manifest mit 179 exakten Dateien, die bereits ausdrücklich freigegebene `engine/remote_copy.rs` und zusätzlich `engine/discovery.rs`, `local_access/os/windows/confined_native.rs` sowie `local_access/os/windows/directory_handle.rs`. Diese gemeinsame Fläche war für die Kette Quelle → Stage → dauerhafter Beleg → Publikation → Quarantänenlöschung erforderlich. Der Bericht bewertet den gelesenen Code, nicht nachfolgend angekündigte Korrekturen.

**Plan und Entscheidungen:** Aus dem Ergebnisauftrag folgte zunächst die erneute Prüfung jedes N1–N6-Auslösers. Nach der Bestandslektüre wurden die unmittelbar berührten normalen Copy-/Movepfade einbezogen: fehlende persistierbare Identität, verweigerte Veröffentlichung, fehlgeschlagene Journalvorbereitung, gehaltene Windows-Handles und gemischte optische Pakete. Die zweite Quellenprüfung klärte Exporthandleformate, Datei-/Parentflush und Windows-Sharing. Das Abnahmekriterium dieses Auftrags ist ein belegtes statisches Urteil je altem Befund samt konkreten verbleibenden Zustandsfolgen; ein Build oder ausgeführtes Testorakel gehört ausdrücklich nicht dazu.

### Urteil je ursprünglichem Befund

| Befund | Urteil im gelesenen Stand | Beleg und Grenze |
|---|---|---|
| B2-N1 | Teilweise geschlossen; weiterhin **offen, hoch** | Die beiden ursprünglichen Einzelfile-Reentranzen sind entfernt. Ein nichtoptisches erstes Paketmitglied kann weiterhin optische Mitglieder aufnehmen; deren gleichzeitig gehaltene Reader erzeugen denselben Lockkonflikt. Konkrete Folge unten. |
| B2-N2 | Ursprünglicher Fehler **statisch geschlossen** | Ein dauerhaftes `Index.pending` identifiziert genau den neu anzulegenden Beleg vor dessen Anlage. Die Reparatur darf ausschließlich diesen Eintrag als nicht autorisiert ergänzen; zuvor gezählte Belege müssen vollständig vorhanden sein. |
| B2-N3 | Ursprüngliches Vorpublikationsfenster **statisch geschlossen** | Der Beleg speichert Altziel und tatsächliche Stage. Nur eine noch vorhandene eigene Stage bei unverändertem Altziel erlaubt die Einstufung als unveröffentlicht. Der Direct-Move-Hook bindet nun den tatsächlich umbenannten Hardlinkalias. Die gesonderte Fehlerfolge N8 bleibt offen. |
| B2-N4 | Ursprünglicher Regular-Reader-Fehler **statisch geschlossen** | Quellen mit `read_size=None` werden mit der gespeicherten und erneut bestätigten Objekt-ID über `open_read_id` gelesen. Der Regular-Reader für Google-Dokumente bleibt beschränkt. |
| B2-N5 | Ursprüngliche Reihenfolge **statisch geschlossen** | Recovery und normaler Stage-/Direct-Move bestätigen Zielobjekt, Inhalt, Datei und Parent vor der Quarantänenlöschung. Fehlende Flushrechte liefern einen Fehler bei erhaltener Quarantäne. Das neue Hashen besitzt jedoch die Abbruchlücke N10. |
| B2-N6 | Ursprünglicher bloßer dev/ino-Beleg **statisch geschlossen** | Linux verlangt Dateisystem-UUID und qualifiziertes Exporthandle mit Generation; Windows beschränkt den persistenten Beleg auf lokales NTFS mit vollständiger Objektkennung. Fehlende Fähigkeit bleibt unbekannt. Die konkrete Btrfs-Qualifizierung ist falsch, siehe N9; die normale Bereinigung bei fehlender Fähigkeit ist fehlerhaft, siehe N7. |

**N1, geschlossene Einzelfile-Folgen:** `file_evidence.rs::observe`, Zeilen 28–52, hält keinen äußeren Gerätepermit mehr. `hash` konsumiert den Reader, sodass dessen Erlaubnis vor dem folgenden `stat`/Identitätsread freigegeben ist. `source.rs::LocalSource::publication_check`, Zeilen 75–80, konsumiert den gehaltenen Reader vor der Veröffentlichung; `LocalCheck::verify`, Zeilen 184–192, prüft Beobachtung, Medium und gegebenenfalls Handleidentität erneut. `upload.rs`, Zeilen 64–68, verwendet diesen Abschluss. Die Vorbereitung in `run_view` und die Dateioperation im Worker liegen im Cancellationkontext. `vfs/os/shared/local_file_identity.rs` hält eine kurze optische Erlaubnis für seinen eigenen Metadatenhandle, ohne zusätzlich einen `guard_reader` darunter zu öffnen. Damit sind die beiden unter 21:37 UTC beschriebenen Folgen statisch geschlossen.

**N2, Journalübergang:** `file_recovery.rs::start_file`, Zeilen 303–327, hält die Jobdaten-Sperre und schreibt für einen neuen Schlüssel zuerst den vollständigen Pendingintent, danach den Receipt, danach den erhöhten Index ohne Pendingintent. `repair_new_entry`, Zeilen 76–98, akzeptiert am vorgesehenen Kandidatenpfad nur exakt den Intent oder seine bereits nicht autorisierte Form. Die Zahl aller bereits vorhandenen übrigen Belege muss dem alten Zähler entsprechen. Die Reparatur kann daher ein eigenes unterbrochenes Anlegen abschließen, ohne den Verlust eines zuvor bestätigten Belegs wegzuzählen. Der wiederholte Reparaturschritt bleibt anhand derselben beiden zulässigen Kandidatenzustände möglich. Die neuen Orakel für einen fehlenden Kandidaten und einen fehlenden bereits gezählten Beleg wurden gelesen, nicht ausgeführt.

**N3, Vorpublikation und Vorbereitungsfehler:** `file_recovery.rs::publication_state`, Zeilen 127–151, trennt dauerhafte negative Autorisierung, nachgewiesene Publikation und unbestätigte Vorpublikation. Das Altziel wird über `previous_matches` geprüft; die eigene Stage muss weiterhin vorliegen. Ein nachträglich ersetztes Ziel bei bereits konsumierter Stage bleibt abgewiesen. `direct_move.rs`, Zeile 45, meldet `rename_source` als Stage. `preparation_failed`, Zeilen 417–427, repariert gegebenenfalls den Pendingintent und schreibt danach `publication_authorized=false` dauerhaft. `safe_file.rs`, Zeilen 180–184, und `direct_move.rs`, Zeilen 45–53, geben Stage/Alias und Quarantäne erst nach erfolgreicher negativer Bestätigung auf. Ein `Unsupported` des Observerdefaults liefert diese Bestätigung nicht. Die gelesenen Download-/Remote-Publish-Hooks erhalten ihre Stage bei fehlgeschlagener Revokation ebenfalls. Dieser Nachweis gilt für fehlgeschlagene Vorbereitung; die spätere Commitfehlerfolge ist N8.

**N4, Exportbeleg:** `file_evidence.rs` unterscheidet Quellenbeobachtung und veröffentlichte Dateibytes. Bei unbekannter Exportlänge muss die ausgewählte gespeicherte ID nichtleer sein und mit der aktuellen Metadaten-ID übereinstimmen, bevor `open_read_id` aufgerufen wird. `file_recovery.rs::verify_source`, Zeilen 275–283, prüft Quell-ID, Objektbeleg, gelistete Größe, Änderungszeit und den Hash der Exportausgabe gegen den gespeicherten Inhalt. Die Quellgröße wird nicht als Exportlänge verwendet. `gdrive/core/extensions.rs` behält seine Regular-File-Abweisung für native Google-Dokumente. Der gelesene Fake-Provider verlangt den ID-Reader ausdrücklich; daraus folgt keine Liveabnahme und keine Garantie, dass ein späterer erneuter Google-Export bytegleich ausfällt. Die getrennte Exportoperation mit Datei-ID und MIMEformat entspricht dem APIvertrag. [Google Drive v3 files.export](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/export)

**N5, dauerhafte Veröffentlichung:** `file_recovery.rs::verify_saved`, Zeilen 203–239, prüft alle Belege vor den Recoverymutationen. Bei publiziertem Moveziel folgt in Zeilen 246–254 zuerst `confirm_transfer_publication`, danach erneute Inhalts-/Objektprüfung und Zielrootprüfung, erst danach die Quarantänenoperation. `publication_proof.rs` hält den aufgelösten Parent und die tatsächliche Datei, vergleicht ID/Hash, synchronisiert Datei und Parent und prüft danach erneut Inhalt und Namensbindung. Linux benötigt den Parentflush zusätzlich zum Dateiflush. [Linux fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html)

Der normale Stagepfad hält den Writer bis `after_publish_handle`; der Direct-Move-Pfad hält unabhängig von einem Observer einen privaten Hardlinkalias und die ursprüngliche Quarantäne. Fehlende Hardlink-/Flushrechte führen vor Veröffentlichung zum Stagefallback. `safe_file.rs`, Zeilen 226–241, und `direct_move.rs`, Zeilen 66–83, bestätigen Datei und Namespace vor der eigenen Quarantänenlöschung. Der temporäre Directwriter wird vor dem Windows-DELETE-Open freigegeben. Die Livebereinigung bindet die Mutation an gehaltenen Parent und Datei; fehlende persistierbare IDs führen dort nicht zu einem ungebundenen `remove_file(path)`.

Unter Windows öffnet `local_access/os/windows/publication.rs` das vorhandene Ziel relativ zum gehaltenen Parent mit explizit angeforderten Rechten und `FILE_OPEN`, ohne Dateiattribute zur Rechtebeschaffung zu ändern. Für den Parent verwendet es `ReOpenFile` und `NtFlushBuffersFileEx` mit Flag 0. Die zusätzlich gelesenen `confined_native.rs` und `directory_handle.rs` zeigen die gehaltene Parentauflösung und Read-/Write-Sharing. Daraus ergibt sich kein zusätzlicher eigener Sharingwiderspruch. Die APIs prüfen die angeforderten Rechte und die Kompatibilität mit bestehenden Opens; fehlende Rechte bleiben ein Fehler. Ein vorheriger Metadataread beweist oder verleiht diese Schreibrechte nicht. Die konkrete Windows-Dateisystem-/Treiberreaktion wurde nicht ausgeführt. [ReOpenFile](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-reopenfile), [NtFlushBuffersFileEx](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntflushbuffersfileex)

**N6, persistierte Objektlebensdauer:** `copy/os/linux_file_proof.rs` ersetzt den gespeicherten dev/ino-String durch eine vom geöffneten Objekt gelesene UUID und das qualifizierte `name_to_handle_at(AT_EMPTY_PATH)`-Exporthandle. Es gibt keinen Fallback auf eine bloße Inodenummer. Ext- und XFS-Formate enthalten eine Generation; ein für ext/Btrfs als Wildcard wirkender Nullwert wird abgewiesen. Die Agentadapter verwenden denselben plattformspezifischen Beleg. Der Vergleich von dev/ino bei zwei gleichzeitig gehaltenen Linuxhandles dient weiterhin dem Liveobjektvergleich und wird nicht als Wiederanlaufidentität ausgegeben. [name_to_handle_at(2)](https://man7.org/linux/man-pages/man2/name_to_handle_at.2.html), [Linux generic_encode_ino32_fh](https://linux.googlesource.com/linux/kernel/git/torvalds/linux/+/6c790212c588fddeb0d852f2790840753bb604b1/fs/libfs.c), [XFS-Exportformat, Linux v6.12](https://raw.githubusercontent.com/torvalds/linux/v6.12/fs/xfs/xfs_export.h)

`copy/os/windows_file_proof.rs` qualifiziert ausschließlich lokales NTFS, prüft VolumeSerialNumber und die vollständige `FILE_ID_INFO`-Objektkennung einschließlich MFT-Sequenz und weist Netzwerk-/unqualifizierte Dateisysteme als nicht unterstützt ab. Die Aussage ist ausdrücklich auf die Plattforminkarnation begrenzt: NTFS dokumentiert einen zirkulären Sequenzzähler, keine mathematisch unbegrenzte Nichtwiederverwendung; geklonte Volumes und ein vollständiger Generationenüberlauf wurden nicht untersucht. [Microsoft MFT_SEGMENT_REFERENCE](https://learn.microsoft.com/en-us/windows/win32/devnotes/mft-segment-reference)

### Verbleibender B2-N1 — Optische Mitglieder eines gemischten Pakets umgehen den Einzelfile-Ausschluss

**Schwere:** hoch. **Stand:** offen, an den Hauptagenten übergeben.

**Fundstellen:** `engine/worker.rs`, Zeilen 36–48; `engine/batch.rs`, Zeilen 21–41; `engine/queue.rs::take_small`, Zeilen 222–234; `engine/discovery.rs`, Zeilen 120–125; `engine/batch_put.rs`, Zeilen 28–31, 70–75 und 103; `engine/file_recovery.rs::prepare_direct`, Zeilen 387–392.

Konkrete Folge: Ein Upload zu einem paketfähigen Agent-/Shareziel enthält kleine lokale Dateien von einer nichtoptischen Quelle und einem optischen Gerät. Der Worker zieht zuerst die nichtoptische Datei. Nur diese erste Datei wird auf optische Herkunft geprüft. `take_small` prüft bei den weiteren Mitgliedern Größe, Retry und `alone`, aber nicht das Medium; die Discovery setzt `alone` nicht für optische Dateien. `batch_put::upload` öffnet daraufhin alle Mitglieder vor dem Stream und hält ihre `LocalSource`-Reader gleichzeitig.

Mit zwei Dateien desselben optischen Geräts wartet bereits das Öffnen der zweiten auf den noch gehaltenen Permit der ersten. Bei einem journalisierten Paket genügt ein optisches Mitglied neben dem nichtoptischen ersten Mitglied: `prepare_batch` ruft dessen Quellensnapshot auf, während der Reader bis Zeile 103 gehalten wird; der optische `stat` verlangt dieselbe Erlaubnis erneut. Der Ausschluss von `RecoveryMode::Resume` in `batch::members` beseitigt diesen normalen neuen Uploadpfad nicht. Cancellation kann das Warten auflösen; ohne Abbruch kann dieser Kontrollfluss nicht fortschreiten. Das folgt aus der Locklebensdauer und den dokumentierten unabhängigen Opens, nicht aus einem ausgeführten Hängertest. [Linux flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html), [Microsoft LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex)

**Erforderliche Schließung:** Die Paketmitgliedschaft muss für jedes Mitglied die optische Einzelfile-Grenze erhalten, oder der Paketreader muss die optischen Handles ohne gleichzeitigen eigenen Permit und ohne Snapshot-Reentranz führen. Die spätere gemeinsame Suite benötigt den gemischten Auswahlfall; ein ausschließlich optisches erstes Mitglied deckt ihn nicht ab.

### B2-N7 — Normales Stage-Discard verliert bei unbekannter persistenter Identität die Bereinigung

**Schwere:** mittel. **Stand:** offen, an den Hauptagenten übergeben.

**Fundstellen:** `copy/os/shared/staging.rs::discard`, Zeilen 337–342; Abbruch-/Fehleraufrufe in `stage_copy`, Zeilen 125–130, und `safe_file.rs`, Zeilen 149–151; Fähigkeitsgrenzen in `copy/os/linux_file_proof.rs` und `windows_file_proof.rs`.

`discard` fragt die neue persistierbare Handleidentität ab, schließt den noch gehaltenen Writer und versucht die Entfernung nur bei `Ok(identity)`. Bei `Unsupported` erfolgt weder Bereinigung noch Fehlermeldung. Konkreter Fall: Eine gewöhnliche Kopie auf FAT/exFAT, ein nicht qualifiziertes Linux-Dateisystem oder einen Kernel ohne verfügbaren UUID-ioctl wird während des Streams abgebrochen. Die vorhandene eigene `.part`-Datei bleibt liegen, obwohl der normale Transfer eine fehlende Wiederanlaufidentität weiterhin zulässt. Wiederholte Abbrüche können dadurch den Zielplatz mit privaten Stages belegen. Ein entsprechender Umfang wurde nicht gemessen.

Die bereits vorhandene gehaltene Liveobjekt-/Parentgrenze kann eigene temporäre Dateien ohne persistierbaren Wiederanlaufbeleg unterscheiden. Das Schließen dieses Handles vor der Entscheidung verschenkt genau diesen Beleg. **Erforderliche Schließung:** Abbruch-/Fehlercleanup an den gehaltenen eigenen Stagehandle und Parent binden; unbekannte Persistenzfähigkeit darf die normale Bereinigung nicht still ausschalten. Quelle dieses Befunds ist der vollständige lokale Kontrollfluss; es wird kein nicht dokumentierter Providervertrag vorausgesetzt.

### B2-N8 — Commitfehler verwirft die eigene Stage ohne dauerhafte negative Autorisierung

**Schwere:** mittel. **Stand:** offen, an den Hauptagenten übergeben.

**Fundstellen:** `copy/os/shared/safe_file.rs`, Zeilen 186–221; `copy/os/shared/direct_move.rs`, Zeilen 55–113; `transfer/os/shared/engine/local.rs`, Zeile 83; `engine/file_recovery.rs::publication_state`, Zeilen 142–149.

Der neue Revokationshook wird bei einem Fehler von `before_publish` aufgerufen. Gibt `before_publish` erfolgreich zurück und scheitert anschließend der eigentliche Commit, verwerfen die genannten Fehlerzweige dagegen die Stage beziehungsweise den Alias und stellen die Quarantäne zurück, ohne den weiterhin autorisierten Receipt dauerhaft zu widerrufen. `LocalFailure::Publish` ruft im Engineadapter kein `unpublished` auf.

Konkrete Folge auf einem qualifizierten lokalen Linux-Dateisystem: Ein journalisierter Overwrite-Move schreibt seine eigene Stage in ein beschreibbares Verzeichnis mit Sticky-Bit. Die vorhandene fremde Zieldatei darf der aufrufende Benutzer nicht ersetzen. Die Veröffentlichung wird verweigert, während die eigene Stage entfernt werden darf. Der Code entfernt diese Stage und stellt die Quelle zurück; am Ziel bleibt die alte Datei. Beim Wiederöffnen ist der Receipt weiterhin `publication_authorized=true`, das Ziel passt nicht zum neuen Beleg und die gespeicherte eigene Stage fehlt. `publication_state` lehnt den Job deshalb dauerhaft als unbestätigte Publikationsphase ab. Die Beseitigung des ursprünglichen Rechtehindernisses allein macht diesen Job nicht wiederanlaufbar. [Linux rename(2), Sticky-Bit-Fehler](https://man7.org/linux/man-pages/man2/rename.2.html)

**Erforderliche Schließung:** Vor dem Verwerfen von Stage/Alias und Quarantänenrestauration muss eine nachgewiesene Nichtveröffentlichung dauerhaft gesichert werden. Bei unklarem Publikationsergebnis bleiben die Belege erhalten. Ein pauschales „Rename meldete Fehler, also unveröffentlicht“ wäre falsch: Die Linux-Dokumentation nennt für NFS ausdrücklich fehlgeschlagene Aufrufe nach bereits erfolgtem Rename. Ebenso darf ein Fehler nach einem erfolgreichen Commit nicht zur negativen Autorisierung werden. [Linux rename(2), NFS-Grenze](https://man7.org/linux/man-pages/man2/rename.2.html)

### B2-N9 — Btrfs-Exporthandle wird mit dem XFS-Typwert verglichen

**Schwere:** mittel. **Stand:** offen, an den Hauptagenten übergeben.

**Fundstelle:** `copy/os/linux_file_proof.rs::qualify_export`, Zeile 101: `(0x9123_683e, 0x81, 20)`.

Der Btrfs-Encoder ohne angeforderten Parent liefert `FILEID_BTRFS_WITHOUT_PARENT`, dessen Wert im Kernel `0x4d` ist. Der gepackte Inhalt aus `objectid`, `root_objectid` und `gen` hat die erwarteten 20 Byte. `0x81` ist hier somit keine passende Btrfs-Qualifikation. Ein tatsächliches Btrfs-Handle wird auch bei vorhandener UUID und Generation als `Unsupported` verworfen. Dadurch bleibt Dateiwiederaufnahme auf diesem ausdrücklich vorgesehenen Dateisystem unbelegt; zugleich erreicht normale Stage-Bereinigung dort den Fehler N7. [Linux exportfs.h, Commit c9049984f0e470af865c497c7f785fe895e5da9c](https://code.googlesource.com/linux/torvalds/linux/+/c9049984f0e470af865c497c7f785fe895e5da9c/include/linux/exportfs.h), [Btrfs-Encoder](https://raw.githubusercontent.com/torvalds/linux/master/fs/btrfs/export.c), [Btrfs-Handlelayout](https://raw.githubusercontent.com/torvalds/linux/master/fs/btrfs/export.h)

**Erforderliche Schließung:** Den tatsächlich dokumentierten Btrfs-Typ zusammen mit Länge und Generation qualifizieren; unbekannte Exportformate weiterhin ablehnen. Ein allgemeines Entfernen der Formatprüfung würde den N6-Vertrag schwächen.

### B2-N10 — Publikationshash liest ganze Dateien ohne Cancellationprüfung

**Schwere:** mittel. **Stand:** offen, an den Hauptagenten übergeben.

**Fundstellen:** `copy/os/shared/publication_proof.rs::require_digest`, Zeilen 132–151; seine Aufrufe in `confirm_transfer_publication`, Zeilen 9–29, und `confirm_transfer_publication_handle`, Zeilen 35–53; `engine/file_recovery.rs`, Zeilen 249 und 365–368.

Die neue Digestfunktion liest in einer Schleife mit 256-KiB-Puffer bis EOF. Ihre API erhält keinen Abbruchzustand; die Schleife fragt keinen solchen Zustand ab. `Interrupted` wird ohne eigene Abbruchprüfung wiederholt. Beide Bestätigungsfunktionen rufen die vollständige Lektüre zweimal auf. Damit wechselt der journalisierte lokale Move-/Overwriteabschluss vom abbrechbaren Enginebeleg in zwei nicht abbrechbare Volllektüren. Dasselbe gilt für die neue Bestätigung im Recoverypfad.

Konkreter Fall: Der Benutzer bricht während der ersten zusätzlichen Ziellesephase eines großen Moveziels ab. Die Bestätigung liest trotzdem bis zu ihrem Ende weiter und beginnt bei unverändertem Inhalt auch die zweite Lektüre, bevor ein nachgelagerter Enginecheck den Abbruch sehen kann. Diese Grenze ist eine vom Programm selbst fortgesetzte Mehrblockschleife, keine lediglich bereits laufende einzelne Betriebssystemoperation. Es wird weder eine gemessene Dauer noch eine garantierte Unterbrechbarkeit blockierender Systemaufrufe behauptet.

**Erforderliche Schließung:** Den vorhandenen Abbruchzustand bis zur Bestätigungsfunktion führen und zwischen den gelesenen Blöcken berücksichtigen. Ein Abbruch vor vollständiger Bestätigung muss die Quarantäne erhalten. Maßstab ist der explizite Cancellationvertrag des Projekts und die in `file_evidence.rs` erhaltene Prüfung zwischen Hashblöcken; ein externer Dienstvertrag ist nicht beteiligt.

### Erhaltene Grenzen und noch fehlende Nachweise

- Die normalen Copy-, Bridge- und Legacy-Einstiege wurden an ihren betroffenen Belegübergängen nachgelesen. Fehlende persistente Identität bleibt an den VFS-/Agentgrenzen optional; daraus folgt keine Wiederaufnahmeautorisierung. Die unter N7 dokumentierte normale Bereinigung erfüllt diesen Kompatibilitätsanspruch noch nicht.
- Die getroffenen Aussagen betreffen Belegkette, Rechte, Quarantäne, Abbruch und optische Readerlebensdauer. Es gab keinen erneuten P24- oder Wallpaper-Codeeingriff; die bereits geschlossenen Nachweise oben bleiben erhalten.
- Die vorhandenen Orakel `file_recovery_tests.rs` und `file_recovery_fix_tests.rs` wurden als Quelltext geprüft. Kein Orakel wurde ausgeführt. Journalabsturzfenster, Wechselmedien, Windows-Sharing/Flush, Dateisysteminkarnationen und Live-Google-Exporte besitzen in diesem Review keinen Laufzeitnachweis.
- Vor dem Bericht wurden Gitstatus, HEAD und gezielte Status-/Quellensuchen gelesen. Die Graphabfrage zeigte hauptsächlich bestehende Namensnachbarschaften und lieferte keinen vollständigen Graphnachweis der neuen Module. Es gab keinen Graphrebuild und keine Installation trotz Versionswarnung.
- Alle in diesem zweiten Nachreview verwendeten Primärquellen wurden am 2026-10-25 abgerufen. Die Raw-Kerneldateien mit `master` bezeichnen den beim Abruf sichtbaren Stand, keine Behauptung über den eingesetzten Kernel. Nicht erreichbare Raw-/Kernel-Seiten wurden durch die oben verlinkten erreichbaren Primärquellen ersetzt; aus Abruffehlern wurde kein APIverhalten abgeleitet.
- Der Hauptagent hat die fünf offenen Grenzen zur Korrektur übernommen. Diese Ankündigung ist kein Codebeleg. Der Reviewer endet nach dieser Berichtaktualisierung; Produktionsänderungen, Remote-Suite und Release bleiben beim Hauptagenten.

### Lese- und Schreibinventar des zweiten Nachreviews

Gelesen wurden das Scope-Manifest, die zentralen Arbeitsweise-/Graphify-Anweisungen und die folgenden Projektdateien. Die Liste umfasst vollständige Lektüre, begrenzte Zeilen-/Symbollektüre und gezielte Suchen; sie behauptet ausdrücklich keine vollständige Funktionsabnahme jeder aufgelisteten Datei. Die drei zusätzlichen Dateien sind im Umfangsabsatz benannt.

- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/batches.md`
- `docs/plaene/2026-10-24-batches/recherche.md`
- `docs/plaene/2026-10-24-batches/review-b2.md`
- `docs/plaene/2026-10-24-batches/vertraege.md`
- `graphify-out/graph.json`
- `native/Cargo.toml`
- `native/src/agent_proto/os/linux_os/local_platform.rs`
- `native/src/agent_proto/os/shared/file_identity.rs`
- `native/src/agent_proto/os/windows/local_platform.rs`
- `native/src/copy/mod.rs`
- `native/src/copy/os/linux_file_proof.rs`
- `native/src/copy/os/linux_os.rs`
- `native/src/copy/os/linux_recovery.rs`
- `native/src/copy/os/shared/bridge.rs`
- `native/src/copy/os/shared/direct_move.rs`
- `native/src/copy/os/shared/durability.rs`
- `native/src/copy/os/shared/move_guard.rs`
- `native/src/copy/os/shared/publication.rs`
- `native/src/copy/os/shared/publication_proof.rs`
- `native/src/copy/os/shared/recovery.rs`
- `native/src/copy/os/shared/safe_file.rs`
- `native/src/copy/os/shared/staging.rs`
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/windows_file_proof.rs`
- `native/src/copy/os/windows_recovery.rs`
- `native/src/gdrive/core/extensions.rs`
- `native/src/gdrive/core/folder_reservation.rs`
- `native/src/local_access/os/linux/quarantine.rs`
- `native/src/local_access/os/windows/confined.rs`
- `native/src/local_access/os/windows/confined_native.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/publication.rs`
- `native/src/local_access/os/windows/read.rs`
- `native/src/local_access/os/windows/regular.rs`
- `native/src/source_media/reader.rs`
- `native/src/transfer/os/shared/engine/batch.rs`
- `native/src/transfer/os/shared/engine/batch_put.rs`
- `native/src/transfer/os/shared/engine/discovery.rs`
- `native/src/transfer/os/shared/engine/download.rs`
- `native/src/transfer/os/shared/engine/file_evidence.rs`
- `native/src/transfer/os/shared/engine/file_recovery.rs`
- `native/src/transfer/os/shared/engine/file_recovery_fix_tests.rs`
- `native/src/transfer/os/shared/engine/file_recovery_tests.rs`
- `native/src/transfer/os/shared/engine/job_store.rs`
- `native/src/transfer/os/shared/engine/legacy.rs`
- `native/src/transfer/os/shared/engine/local.rs`
- `native/src/transfer/os/shared/engine/mod.rs`
- `native/src/transfer/os/shared/engine/ops.rs`
- `native/src/transfer/os/shared/engine/publish.rs`
- `native/src/transfer/os/shared/engine/queue.rs`
- `native/src/transfer/os/shared/engine/record_io.rs`
- `native/src/transfer/os/shared/engine/remote_copy.rs`
- `native/src/transfer/os/shared/engine/roots.rs`
- `native/src/transfer/os/shared/engine/segments_upload.rs`
- `native/src/transfer/os/shared/engine/selection.rs`
- `native/src/transfer/os/shared/engine/source.rs`
- `native/src/transfer/os/shared/engine/upload.rs`
- `native/src/transfer/os/shared/engine/worker.rs`
- `native/src/transfer/os/shared/walk.rs`
- `native/src/transfer/os/shared/walk_listers.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/shared/local_file_identity.rs`

**Geändert:** ausschließlich `docs/plaene/2026-10-24-batches/review-b2.md`. Keine Produktionsdatei erstellt oder geändert. Keine lokalen oder entfernten Builds, Compiler, Tests, Formatter, Server, Installationen, Commits, Pushes oder Releaseaufrufe durch diesen Reviewer. Keine Unteragenten.

## Dritter Nachreview der Korrekturen N1/N7/N8/N9/N10 — 2026-10-25, 22:57 UTC

**Historischer Befundstand:** N11 war in diesem Stand offen. Der vierte Nachreview dokumentiert die nachgelesene Schließung; die damalige Zustandsfolge bleibt als Beleg erhalten.

**Basis:** HEAD `7442afc0` plus der zur Prüfung als abgeschlossen gemeldete gemeinsame Arbeitsbaum. Auftrag: die fünf konkreten Auslöser und ihre direkt betroffene Kompatibilität rein statisch gegen Code und Primärquellen bewerten. Der Hauptagent hat den nachstehend bestätigten N11-Befund an den Autor zurückgegeben und den Abschluss dieses Berichts ohne Warten auf die Folgekorrektur verlangt. Eine anschließend geänderte Produktionsdatei ist hier nicht als nachgeprüft ausgegeben.

**Plan und Entscheidungen:** Zunächst wurden Permit-Lebensdauer, gehaltenes Stage-Discard, Commitfehler/Revokation, Btrfs-Qualifikation und beide Cancellationcaller verfolgt. Der aus dem Bestand abgeleitete zweite Prüfschritt bezog NoReplace-Konflikte vor Dispatch, unklare Fehler nach Dispatch und den bestehenden Cross-Device-Fallback ein. Die Quellenprüfung trennte dafür Linux-VFS-Mountprüfung, tatsächlichen Provideraufruf und Windows-Volumequalifikation. Ein fehlender lokaler Compilerlauf ist keine offene Reviewhandlung: Ausführung war ausdrücklich verboten. Die verbleibende Produktkorrektur ist N11.

### Geschlossene Auslöser und erhaltene Grenzen

| Befund | Statisches Ergebnis | Nachgelesene Zustandsfolge |
|---|---|---|
| B2-N1 | **Geschlossen** | `engine/batch.rs`, Zeilen 21–64, prüft das erste File und jeden nachgezogenen Kandidaten. Optische Quellen und Probefehler kommen als `alone=true` zurück in die Queue. `batch_put.rs`, Zeilen 28–40, prüft zusätzlich den tatsächlich geöffneten Reader: Bei optischer Zulassung wird er vor `run_file` gedroppt und nie in `opened` gehalten. Somit hält auch ein nach der Queueprobe optisch gewordenes Mitglied keinen später benötigten eigenen Permit während Einzeltransfer oder Paketsnapshot. Nichtoptische Mitglieder behalten den Paketpfad. |
| B2-N7 | **Geschlossen** | `staging.rs::discard`, Zeilen 335–359, hält den Writer, öffnet Parent und Metadatenhandle, prüft Regularität und Liveobjektgleichheit und schließt erst danach den Writer. Der Metadatenhandle bleibt durch `discard_transfer_stage_live` und die bedingte Mutation gehalten. Keine persistierbare ID wird für dieses Cleanup verlangt. Linux bindet die Löschung an private Capture und gehaltenen Inode; Windows an den identitätsgeprüften DELETE-Handle. Fehler werden mit dem Stagepfad zurückgegeben. Die Abbruch-/Fehleraufrufer behandeln jetzt das `Result`. |
| B2-N8 | Ursprüngliche fehlende Revokation **geschlossen** | `safe_file.rs`, Zeilen 186–226, und `direct_move.rs`, Zeilen 55–63, rufen bei Commitfehler zuerst `publication::commit_failed` auf. Diese Funktion verlangt Nichtpublikationsbeleg und danach die dauerhafte Observer-Revokation. Erst danach folgen Skip, nummerierter Retry, Stagefallback, Discard oder Quarantänenrestauration. Misslingt eine Voraussetzung, bleiben Stage/Alias und Quarantäne erhalten. Der gesonderte Cross-Device-Kompatibilitätsfehler dieses Gates ist N11. |
| B2-N9 | **Geschlossen** | `linux_file_proof.rs`, Zeile 101, qualifiziert Btrfs nun als `(0x9123_683e, 0x4d, 20)` und prüft die Generation in Bytes 16–20. Unbekannte Formate und die Wildcardgeneration bleiben abgewiesen. |
| B2-N10 | **Geschlossen** | `publication_proof.rs::require_digest`, Zeilen 196–219, prüft das übergebene `AtomicBool` vor und nach jedem Block; ein `Interrupted` kehrt über den nächsten Schleifenanfang zur Abbruchprüfung zurück. Beide Volllektüren und die Flush-/Abschlussgrenzen verwenden denselben Zustand. `file_recovery.rs`, Zeilen 249 und 367, übergibt jeweils `cancel` beziehungsweise `engine.stop_flag()`. Fehler verlassen Recovery/Move vor der Quarantänenlöschung. |

Die N1-Schließung beruht auf der jetzt sichtbaren Dropreihenfolge und demselben dokumentierten Lockvertrag wie der ursprüngliche Befund; sie behauptet keinen ausgeführten optischen Paralleltest. Ein bereits laufender Read-/Flush-Systemaufruf wird auch durch N10 nicht präemptiv unterbrochen. [Linux flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html), [Microsoft LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex)

Der korrigierte Btrfs-Typ entspricht `FILEID_BTRFS_WITHOUT_PARENT`; der Encoder und das gepackte Layout bleiben die konkreten Primärbelege. Die neue Typkonstante ersetzt keine Prüfung der verfügbaren UUID-/Exportfähigkeit des tatsächlich eingesetzten Dateisystems. [Linux exportfs.h, Commit c9049984f0e470af865c497c7f785fe895e5da9c](https://code.googlesource.com/linux/torvalds/linux/+/c9049984f0e470af865c497c7f785fe895e5da9c/include/linux/exportfs.h), [Btrfs-Encoder](https://raw.githubusercontent.com/torvalds/linux/master/fs/btrfs/export.c), [Btrfs-Handlelayout](https://raw.githubusercontent.com/torvalds/linux/master/fs/btrfs/export.h)

**N8, dauerhafte Bindung:** `file_recovery.rs::revoke`, Zeilen 421–439, prüft bei einem Commitfehler den gespeicherten Stage-/Zielpfad sowie `confirmed` und `discarded`, bevor es `publication_authorized=false` schreibt. Ein fehlender Receipt wird abgewiesen. Die vorhandene Pendingreparatur bleibt vorgeschaltet. Der Default von `PublicationObserver::publication_failed` liefert `Unsupported`; er erfindet keine dauerhafte Bestätigung. Ein erfolgreicher Receiptwrite ist Voraussetzung für die nachfolgenden Freigaben.

**NoReplace vor Dispatch:** Beide Copy-OSadapter rufen `preflight_no_replace` unmittelbar vor ihrer NoReplace-Mutation auf. Die Funktion markiert nur einen bereits beobachteten Zielkonflikt oder einen Fehler dieser reinen Vorabprüfung mit dem vorhandenen `vfs::Unpublished`-Typ. Bei fehlendem Ziel geht es ohne Marker weiter zur ursprünglichen Mutation. `confirm_unpublished`, Zeilen 80–116, akzeptiert den expliziten Marker nur bei weiterhin passender eigener Live-Stage und unverändertem Parent. Damit bleibt der nummerierte Retry für einen schon vor Dispatch erkannten Konflikt auch auf FUSE/Overlay erreichbar. Das ist eine bewusst begrenzte Aussage: Ein später zwischen Probe und Mutation entstandener Konflikt besitzt diesen Marker nicht.

**Fehler nach Dispatch:** Ohne Marker fordert `confirm_unpublished` qualifizierte lokale Beobachtung beider Eltern, weiterhin vorhandene eigene Stage und ein Ziel, das nicht dasselbe Objekt benennt. FUSE/Overlay und Netzwerkprovider werden nicht allein wegen eines Fehlercodes zu „unveröffentlicht“ erklärt. Die vorhandene Linux-/Android-Renameleiter gibt ihre nach Dispatch entstandenen Fehler unverändert weiter; ihr bereits dokumentierter Checked-Rename-Restkonflikt wird durch diesen Review nicht als behoben ausgegeben. Die VFS-Ersetzungswege werden ebenfalls nicht pauschal mit einer negativen Publikationsbestätigung versehen. Die dokumentierte NFS-Grenze verbietet eine solche generische Ableitung. [Linux rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html)

**Windows-Qualifikation:** `windows_publication_failure.rs` ist als eigenes Modul registriert. Es liest `FileFsDeviceInformation` über den gehaltenen Parent, behandelt `STATUS_PENDING` mit gehaltenen Puffer-/Handlereferenzen und wertet ausschließlich eine vollständige erfolgreiche Antwort aus. Remote- und WebDAV-Merkmale werden abgewiesen; die zusätzliche Dateisystemliste umfasst NTFS, ReFS, FAT, FAT32 und exFAT. Das qualifiziert den betrachteten Fehlerbeobachtungspfad, nicht die davon getrennte persistente Recoveryidentität. Der Query fordert keine Schreibrechte an und verleiht keine Ersetzungsrechte. Die verwendeten Antwortlängen und Merkmalsbits entsprechen den Microsoft-Definitionen. [NtQueryVolumeInformationFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryvolumeinformationfile), [MS-FSCC FileFsDeviceInformation](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/616b66d5-b335-4e1c-8f87-b4a55e8d3e4a)

### B2-N11 — Nichtpublikationsgate verhindert Cross-Device-Stagefallback bei unqualifiziertem Ziel-FS

**Schwere:** mittel. **Stand:** einzig offener bestätigter Befund dieses Reviewstands; an den Hauptagenten übergeben, Korrektur beauftragt und hier noch nicht nachgeprüft.

**Fundstellen:** `copy/os/shared/direct_move.rs`, Zeilen 55–63 und 98; `copy/os/shared/publication.rs::commit_failed`; `copy/os/shared/publication_proof.rs::confirm_unpublished`, Zeilen 101–103; `copy/os/linux_os.rs::require_local_publication_observation`, Zeilen 74–86, und `::move_file`, Zeilen 186–191; `android_fs/os/rename.rs::rename_no_replace`.

Konkrete Folge: Ein normaler Datei-Move besitzt eine Hardlink-fähige lokale Quelle, beispielsweise auf ext4, und ein Ziel auf einem getrennten FUSE- oder Overlay-Mount. Die Quelle wird in die gehaltene Quarantäne verschoben; der Direct-Move erzeugt seinen eigenen Hardlinkalias. Das Ziel ist frei, die NoReplace-Vorabprüfung setzt daher keinen `Unpublished`-Marker. Der Rename liefert wegen der getrennten Mounts `EXDEV`. Vor dem bisher vorhandenen `is_cross_device`-Zweig muss der Aufruf jetzt jedoch durch `commit_failed`. Dessen Prüfung lehnt das FUSE-/Overlay-Elternverzeichnis als unqualifiziert ab. Die Funktion endet mit `LocalFailure::Publish`; der Stagefallback in Zeile 98 wird nicht erreicht. Quarantäne und Alias bleiben erhalten, der ursprüngliche Quellname ist weiterhin weg. Auch ein normaler Transfer ohne Journalobserver wird auf diesem Weg gestoppt.

Dies ist keine bloß unklare Providerantwort: Linux `filename_renameat2` vergleicht die aufgelösten Mounts und gibt bei Ungleichheit `EXDEV` zurück, bevor es `mnt_want_write` und später `vfs_rename` aufruft. Die gelesene Kernelquelle zeigt diesen Zweig in Zeilen 5926–5928 gegenüber dem Provideraufruf in Zeile 5971; `rename` und `renameat` benutzen dieselbe Funktion. Der Copyadapter verliert diese konkret belegte Grenze und behandelt sie anschließend wie eine möglicherweise bereits ausgeführte Mutation. [Linux fs/namei.c, beim Abruf sichtbarer Stand](https://raw.githubusercontent.com/torvalds/linux/master/fs/namei.c), [Linux rename(2), EXDEV](https://man7.org/linux/man-pages/man2/rename.2.html)

**Erforderliche Schließung:** Der normale Cross-Device-Transfer muss seinen Stagefallback erreichen, ohne für eine tatsächlich versandte unqualifizierte Mutation eine negative Bestätigung zu erfinden. Dazu muss der Code die konkrete Nichtdispatch-Grenze bewahren oder die fehlende Direct-Move-Eignung bereits vor Publikationsautorisierung/Mutation erkennen. Ein bloßes Vorziehen des alten Fallbacks unter Umgehung der dauerhaften Revokation würde N8 wieder öffnen. Ein allgemeines Freigeben aller FUSE-/Overlay-Fehler wäre ebenfalls kein ausreichender Nachweis.

### Dateien, Entscheidungen und offene Nachweise dieses dritten Nachreviews

Gelesen wurden `/tmp/smartexplorer-review-b2-fixes-scope.json`, die gelieferten zentralen Arbeitsweise-Regeln und die folgenden Projektdateien als vollständiger Inhalt oder auf die genannten Funktionen begrenzte Zeilen-/Symbollektüre:

- `docs/plaene/2026-10-24-batches/review-b2.md`
- `native/src/android_fs/mod.rs`
- `native/src/android_fs/os/rename.rs`
- `native/src/copy/os/linux_file_proof.rs`
- `native/src/copy/os/linux_os.rs`
- `native/src/copy/os/linux_recovery.rs`
- `native/src/copy/os/shared/direct_move.rs`
- `native/src/copy/os/shared/publication.rs`
- `native/src/copy/os/shared/publication_proof.rs`
- `native/src/copy/os/shared/recovery.rs`
- `native/src/copy/os/shared/safe_file.rs`
- `native/src/copy/os/shared/staging.rs`
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/windows_publication_failure.rs`
- `native/src/copy/os/windows_recovery.rs`
- `native/src/local_access/os/windows/confined.rs`
- `native/src/transfer/os/shared/engine/batch.rs`
- `native/src/transfer/os/shared/engine/batch_put.rs`
- `native/src/transfer/os/shared/engine/file_recovery.rs`
- `native/src/transfer/os/shared/engine/file_recovery_fix_tests.rs`
- `native/src/transfer/os/shared/engine/file_recovery_tests.rs`
- `native/src/transfer/os/shared/engine/source.rs`
- `native/src/transfer/os/shared/engine/worker.rs`
- `native/src/vfs/core/unpublished.rs`
- `native/src/vfs/os/linux_os/local_platform.rs`
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/vfs/os/windows/local_writes.rs`

**Geändert:** ausschließlich `docs/plaene/2026-10-24-batches/review-b2.md`; die historischen Nachweise bleiben erhalten. Gitstatus/HEAD und gezielte Statussuchen wurden vor der Berichtänderung gelesen. Keine Produktionsdatei geändert, kein Graphlauf, kein Unteragent, keine Builds, Compiler, Tests, Server, Installationen, Commits, Pushes oder Releases.

**Quellen und offene Nachweise:** Neu abgerufen am 2026-10-25 wurden Linux `rename(2)`/`fs/namei.c` sowie die oben verlinkten Microsoft-Volumeinformationen; die an diesem Datum bereits geprüften Lock- und Btrfs-Primärquellen wurden weiterverwendet. Die zuerst versuchte Microsoft-URL unter `ntddk` war nicht erreichbar; die Entscheidung verwendet die erreichbare offizielle MS-FSCC-Seite. Keine physische Disc, kein realer FUSE-/Overlay-Move, kein Windows-Flush/-Sharinglauf und kein Hashabbruch wurde ausgeführt. Die statische Schließung N1/N7/N8/N9/N10 ist keine Compiler- oder Laufzeitfreigabe. N11 bleibt bis zum engen Nachreview des integrierten Fixes offen; dieser Reviewer stoppt nach der Berichtaktualisierung.

## Vierter Nachreview: enger Abschluss B2-N11 — 2026-10-25, 23:05 UTC

**Basis und Ergebnis:** Zuletzt abgelesener HEAD `126f9325` plus gemeinsamer Arbeitsbaum mit dem als abgeschlossen gemeldeten N11-Fix. Ausschließlicher Gegenstand sind der Cross-Device-Stagefallback und seine unmittelbare Nichtpublikations-/Journalgrenze. **B2-N11 ist statisch geschlossen.** In dieser engen Nachprüfung wurde kein weiterer Defekt bestätigt. Die Schließungen der vorangegangenen Reviewabschnitte werden erhalten, nicht als vollständig neu durchgeprüft ausgegeben.

**Prüfentscheidung:** Aus dem Auftrag wurden zuerst Vorab-Mountbeleg, Unterbleiben des Renameaufrufs und Erreichen des Fallbacks verfolgt. Die anschließende Quellenprüfung kontrollierte die statx-UAPI und den Linux-/Windows-Renamevertrag; danach wurde die vorhandene dauerhafte Revokation bis zum Stageaufruf nachgelesen. Die Aussage betrifft ausschließlich belegte Vorabentscheidungen. Ein fehlgeschlagener, bereits versandter Provideraufruf wird weiterhin gesondert behandelt.

### Nachgelesene Zustandsfolge

1. `copy/os/shared/direct_move.rs`, Zeilen 34–43, hält einen zur privaten Aliasdatei gebundenen Filehandle. `publication_proof.rs::preflight_direct_move`, Zeilen 80–91, öffnet den Zielparent als `ConfinedDirectory` und vergleicht beide gehaltenen Objekte über den OSadapter. Die neue Vorprüfung steht in `direct_move.rs`, Zeilen 55–56, vor `platform::move_file` in einer `and_then`-Kette. Bei ihrem Fehler wird diese Mutation nicht aufgerufen.
2. Linux `different_rename_boundary`, Zeilen 77–102, erkennt unterschiedliche `st_dev`-Werte direkt. Bei gleicher Gerätezahl wird zusätzlich die Mount-ID beider Handles mit `statx(AT_EMPTY_PATH)` abgefragt und nur bei gesetztem zurückgegebenem `STATX_MNT_ID`-Bit verwendet. Damit wird auch eine getrennte Bind-Mountgrenze erkannt, sofern der Kernel diese Angabe liefert. Der statx-Puffer ist 256 Byte groß und auf acht Byte ausgerichtet; Maske `0x1000` und Offset `0x90` stimmen mit der gelesenen UAPI überein. [Linux v6.12 stat.h](https://raw.githubusercontent.com/torvalds/linux/v6.12/include/uapi/linux/stat.h), [statx(2)](https://man7.org/linux/man-pages/man2/statx.2.html)
3. Windows `different_rename_boundary`, Zeilen 103–108, vergleicht die über beide Handles gelesenen Volumeserien nur bei zwei von null verschiedenen Werten. Unterschiedliche bekannte Serien wählen den Stagepfad. `dwVolumeSerialNumber` bezeichnet das enthaltende Volume; der vorhandene Direct-Move verwendet kein `MOVEFILE_COPY_ALLOWED`. Damit überlässt er einen geräteübergreifenden Copy-/Deleteablauf weiterhin dem eigenen Stagepfad. [BY_HANDLE_FILE_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information), [MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
4. Ein beobachteter Unterschied erzeugt vor Dispatch `ErrorKind::CrossesDevices` mit dem `Unpublished`-Marker. `publication::commit_failed`, Zeilen 100–114, prüft weiterhin die gebundene Live-Stage und ruft anschließend den Observer auf. `file_recovery.rs::revoke`, Zeilen 421–439, bestätigt den passenden Stage-/Zielreceipt und schreibt die negative Autorisierung dauerhaft. Scheitert dieser Schritt, bleibt der Direct-Move mit erhaltenem Alias und erhaltener Quarantäne stehen. Der Fallback umgeht diesen Schutz nicht.
5. Nach erfolgreicher Revokation erreicht `direct_move.rs`, Zeile 99, `Ok(None)`. Der private Alias wird über seine gehaltene Bereinigungsgrenze verworfen; die ursprüngliche Quarantäne bleibt erhalten. `safe_file.rs`, Zeilen 113–147, setzt mit `stage_copy` aus genau diesem Quarantänenpfad fort. Die anschließende Stageveröffentlichung erhält ihre eigene Vorbereitung und die bereits geprüfte dauerhafte Zielbestätigung. Auch ohne Observer bleibt die Liveobjektprüfung vor dem Fallback wirksam.

Für den ursprünglichen Auslöser ext4-Quelle → separater FUSE-/Overlay-Zielmount wird somit schon vor dem Rename die unterschiedliche Geräte-/Mountgrenze erkannt. Die nachträgliche FUSE-/Overlay-Beobachtungsqualifikation blockiert diesen nachweislich nicht ausgeführten Direct-Move nicht mehr. Das passt zur Linux-VFS-Grenze: In der erneut geöffneten v6.12-Quelle prüft `do_renameat2` die Mountungleichheit vor `mnt_want_write` und `vfs_rename`. Die in diesem Abruf sichtbaren Quellzeilen sind 4793–4795, 4806 und 4872; die vom Autor genannten abweichenden Zeilennummern wurden nicht ungeprüft übernommen. [Linux v6.12 fs/namei.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/fs/namei.c)

### Konservativ erhaltene Grenzen

- Ein erst von `platform::move_file` zurückgegebenes EXDEV erhält keinen neuen `Unpublished`-Marker. Es muss weiterhin durch `confirm_unpublished` mit der nach Dispatch erforderlichen Qualifikation. Insbesondere wird eine unklare FUSE-/Overlay-/Netzwerkantwort nicht pauschal widerrufen. N8 wird durch den neuen Fallback nicht wieder geöffnet.
- Fehlende statx-Unterstützung, verweigerte statx-Abfrage oder ein nicht geliefertes Mount-ID-Bit werden bei gleicher `st_dev`-Zahl nicht als Mountunterschied interpretiert. Ebenso beweisen null oder gleiche Windows-Volumeserien keinen Unterschied. Dann bleibt der normale Renameversuch möglich; ein anschließend unqualifiziert fehlgeschlagener Versuch bleibt konservativ unbestätigt. Der Review behauptet deshalb keinen garantierten automatischen Fallback für jede unbekannte Bind-Mount-/Providerkonstellation.
- Der Vorabvergleich verleiht weder Quelllöschungs- noch Zielersetzungsrechte. Quarantäne, Liveobjektbindung, dauerhafte Revokation und spätere Stagebestätigung bleiben eigenständige Voraussetzungen. Ein Abbruch oder Fehler während des Stagekopierens benutzt weiterhin die vorhandene Bereinigung und Quarantänenrestauration.

### Lese-/Schreibinventar und Abschlussgrenze

Die bereits genehmigte Lesefläche wurde nicht erweitert. Für diesen engen Abschluss wurden die folgenden Dateien an den genannten Symbolen beziehungsweise Zeilen erneut gelesen:

- `docs/plaene/2026-10-24-batches/review-b2.md`
- `native/src/copy/os/linux_os.rs`
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/shared/direct_move.rs`
- `native/src/copy/os/shared/publication_proof.rs`
- `native/src/copy/os/shared/publication.rs`
- `native/src/copy/os/shared/move_guard.rs`
- `native/src/copy/os/shared/recovery.rs`
- `native/src/copy/os/shared/safe_file.rs`
- `native/src/copy/os/shared/staging.rs`
- `native/src/transfer/os/shared/engine/file_recovery.rs`

**Geändert:** ausschließlich `docs/plaene/2026-10-24-batches/review-b2.md`. Vor der Änderung wurden Gitstatus, HEAD und die vorhandenen Befundstatus gelesen. Nach der Änderung wurden der aktuelle Status und Whitespace rein textuell geprüft. Die oben genannten Primärquellen wurden am 2026-10-25 erneut beziehungsweise erstmals für diesen engen Abschluss abgerufen.

**Offener Nachweisumfang:** Kein Cross-Device-Move, kein Bind-Mountfall, kein Windows-Volumenwechsel, kein Journalfehler und kein Abbruch wurde ausgeführt. Keine Compiler-, Build- oder Laufzeitfreigabe. Keine Produktionsänderung, kein Test, Server, Installations-, Graph-, Commit-, Push- oder Releaseaufruf; keine Unteragenten. Mit dem dokumentierten statischen Abschluss von N11 endet dieser Reviewauftrag.
