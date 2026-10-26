# Transferkompatibilität: Fixautorbericht

Stand: 2026-10-26. Arbeitskopie `/tmp/smartexplorer-review-fixes-ee75e91b`,
Ausgangskandidat `ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b`.
Dieser Bericht und alle zugehörigen Änderungen stammen vom selben Fixautor.
Es wurde kein lokaler Build, Formatter, Compiler oder Test gestartet.

## A — Auftrag und Grenzen

Quelle ist der delegierte Ergebnisauftrag mit dem wörtlichen Nutzervertrag
in `/tmp/smartexplorer-final-review-user-contract.txt`.
Vollständig zu liefern sind Funktionsauflistung, direkte und indirekte
Verknüpfungskarte, Fixplan, Implementierung und eigene anschließende
Anti-Regressionsreview für RA-02, RA-03/RSOL-01, RA-04, RA-09/RSOL-02 und RSOL-05.
Die sieben IDs beschreiben fünf eigenständige Fehlerursachen.
Die Schreibgrenze steht in `/tmp/smartexplorer-fix-transfer-compat-scope.json`.
Der Main-Arbeitsbaum bleibt unverändert. Suite-Orchestrierung, Ausführung,
Commits, Pushes, Graphrefresh und terminaler Release gehören dem Hauptagenten.

Grundfunktion: zusätzliche Wiederherstellungsbelege dürfen bestehende
Erstübertragungen, bewusst neue Missing-only-Ausführungen und gewöhnliches
Umbenennen nicht von unbelegten Providerfähigkeiten abhängig machen.
Gespeicherte Wiederherstellung bleibt an echte Objektidentität gebunden.
Keine Annahme über verfügbare Livekonten oder lokale Testresultate wird gemacht.
Planungstiefe 2: zusammenhängende Korrektur vorhandener Verträge.

## B — Bestand, Fehlerfunktionen und Verknüpfungskarte

| Befund | Fehlerhafte Funktion und Auswirkung | Direkt betroffene Kette |
|---|---|---|
| RA-02 | `WebdavBackend::rename_no_replace` verlangt für jeden Ordner Resource-ID und REBIND; Desktop und Android verlieren gewöhnliches Umbenennen. | `app/os/shared/file_actions.rs::confirm_rename` und `mobile/os/shared/fs_edit.rs::rename` → `Backend::rename_no_replace` → `webdav/core/webdav.rs` → `folder_reservation::rename`. CLI-Move erreicht denselben Vertrag. Die strikte Engine-Reservierung verwendet bisher dieselbe Schnittstelle. |
| RA-03/RSOL-01 | FTP-/SFTP-`directory_identity` gibt Unsupported statt unbekannter Identität zurück; Erstkopie eines Ordners scheitert vor dem ersten Kind. | Desktop `transfer_route.rs` und Android `transfer.rs` → `TransferJob`/Lane → `engine::prepare` → `roots::plan_roots` → `Reservations::reserve` → `vfs::directory_identity` → FTP-/SFTP-Extension. `selection.rs` prüft die separate exklusive MKD/MKDIR-Fähigkeit. |
| RA-04 | `resume_job` bindet „Fehlende übertragen“ an `RecoveryMode::Resume`; unbekannter Zielroot sperrt den bisherigen Missing-only-Bedienweg. | `transfer_window.rs`/`TransferCenter::take_resume` → `resume_job` → `job_store::preflight/begin/verify_target` → `roots`/AliasStore → Datei-Resume/Größenvergleich. Persistente Recovery verwendet dagegen `transfer_recovery_ui.rs`, `cli/transfer_recovery.rs` und `run_legacy_with_identity`. |
| RA-09/RSOL-02 | `file_recovery::before/after_checked` liest die Remote-Datei zur Belegerzeugung; ein erlaubter Upload scheitert bei verweigertem Download. | Upload, Remote-Copy, Server-Copy, Segmente, Pakete → `publish` beziehungsweise `prepare_direct/after` → `observe` → `open_read_regular`. Writerabschluss, Quellprüfung, NoReplace und Stagebesitz sind davon getrennte bestehende Verträge. |
| RSOL-05 | `direct_move::try_move` setzt einen Hardlink voraus; FAT32/exFAT fallen trotz gleichem Volume auf vollständige Stagekopie zurück. | Desktop lokaler Move → `engine/local.rs::copy_file` → `transfer_local_observed` → Quarantäne → `PublicationAlias::create` → Hardlink → bisheriger Stagefallback. PublicationObserver und `publication_proof` halten Datei, Inhalt und Zielparentbeleg; `move_guard` besitzt Restore/Cleanup. |

Indirekte Grenzen der fünf Ketten:

- VFS-Vertrag: `Backend`, `BackendExtensions`, `CachingBackend`, Local-/ConfinedLocal-,
  FTP-, SFTP-, WebDAV-, Drive-, SMB-, Agent- und Shareadapter. Cacheinvalidierung
  und Share-Freigaben müssen die getrennte strikte Renameoperation weiterreichen.
- Job- und Aliasvertrag: `JobView`, `JobRecord`, `StoredEndpoint`, `SourceGuard`,
  `FolderAliasScope`, `FolderAliases`, `record_io` und Filejournalindex. Namespace,
  State, Root, Auswahl, Policy, Vaultkontext und Lock dürfen nicht gelockert werden.
- Dateioperationen: `worker` → `ops` → `local`, `upload`, `download`, `remote_copy`,
  `segments_copy`, `segments_upload`, `segments_download`, `batch_put`, `batch_get`.
  Die gemeinsame Publikationsgrenze entscheidet Quellbehalt, Fehlerzuordnung,
  Abbruch, Stagebereinigung, Skip, Nummerierung und endgültige Fortschrittsmeldung.
- Normale CLI-/MCP-Zielauflösung bleibt über `cli/transfer.rs`, `engine_copy.rs`,
  `folder_conflict.rs` und `tree_ops.rs` ausdrücklich zielnamengebunden.
  Sync nutzt unveränderte Backendpfade und erhält keine Missing-only-Abkürzung.
- Lokal: `safe_file`, `direct_move`, `move_guard`, `durability`, `publication`,
  `publication_proof`, `recovery`, OS-Moveadapter und gehaltene
  `local_access::ConfinedDirectory` bestimmen Fremdobjektschutz und Cleanup.

## C — Lösungsentscheidung und Quellenrecherche

Erste Recherche: bestehende Erstlaufpfade und die Unterscheidung zwischen
aktueller Ausführung und persistenter Recovery wiederverwenden. Bloßes globales
Abschalten von Journaling, erfundene Provideridentitäten, Probe-plus-Mkdir als
Reservierung sowie MOVE als angeblich atomarer Recoverybeleg werden verworfen.

Zweite Recherche: gewöhnliches WebDAV-MOVE und striktes REBIND brauchen eine
separate typisierte Schnittstelle; Downloadverweigerung darf nur die optionale
Remote-Belegerzeugung begrenzen, niemals lokale Fehler oder echten I/O-Verlust
verschlucken. Ohne Hardlink muss der gehaltene Quarantäneinhalt unmittelbar
umbenannt werden; eine misslungene Nachbestätigung darf ihn nicht löschen.

Primärquellen, erneut geprüft 2026-10-26:

- [RFC 4918 §9.9](https://www.rfc-editor.org/rfc/rfc4918#section-9.9):
  Collection-MOVE und `Overwrite: F`; keine allgemeine Atomaritätsgarantie.
- [RFC 5842 §2.5 und §6](https://www.rfc-editor.org/rfc/rfc5842#section-6):
  REBIND ist der gesonderte atomare Bindingvertrag.
- [RFC 3659 §7.5.2](https://www.rfc-editor.org/rfc/rfc3659#section-7.5.2):
  FTP-Unique ist kein belegter verbindungsübergreifender Wiederherstellungsnachweis.
- [vsftpd-Konfiguration](https://security.appspot.com/vsftpd/vsftpd_conf.html):
  `download_enable` und `write_enable` begrenzen Downloads und Uploadmutationen getrennt.
- [Microsoft Dateisystemvergleich](https://learn.microsoft.com/en-us/windows/win32/fileio/filesystem-functionality-comparison):
  FAT32 und exFAT unterstützen keine Hardlinks.
- [MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw):
  Rename desselben Volumes benötigt keine inhaltsgleiche Zusatzkopie.

## D — Konkrete Meilensteine vor der Implementierung

1. **M1 Provider-Erstkopie:** FTP/SFTP melden unbekannte stabile Ordneridentität
   mit `Ok(None)`; die exklusive MKD/MKDIR-Fähigkeit bleibt separat wahr.
   Erwartung: freier und kollidierender Ordnername funktionieren; verlorene
   Antwort wird ohne Eigentumsbeleg nicht als erfolgreiche Recovery ausgegeben.
2. **M2 WebDAV-Umbenennen:** normalen MOVE-Vertrag wiederherstellen, strikte
   Reservierungsveröffentlichung an eine eigene VFS-Operation binden. Cache
   und Share-Rechtewrapper reichen dieselbe strikte Operation weiter.
   Erwartung: normaler Server ohne Bindings kann Ordner umbenennen; strikte
   Wiederherstellungsreservierung behauptet dort keine Atomarität.
3. **M3 Missing-only, nach Abschnitt J korrigiert:** Copy und lokale Moves
   behalten ihre Job-ID, Endpunkte, Auswahl, Policy und aufgelösten Zielnamen.
   Die aktuelle Wiederholung unterscheidet bestätigte Ausführungsantworten von
   dauerhaftem Eigentum; unbekannte Intents bleiben gesperrt. Frühe Fehler ohne
   tatsächlichen Mutationseintrag bleiben auch ohne Ergebnisroots wiederholbar.
   Gespeicherte Recovery bleibt streng. Zuvor übersprungene Ordner bleiben
   ausgelassen. Erwartung: passende Dateien werden übersprungen, fehlende
   übertragen, abweichende weder ersetzt noch neu nummeriert.
4. **M4 Uploadrechte:** Remote-Belegerzeugung behandelt ausschließlich PermissionDenied
   oder Unsupported als fehlenden optionalen Zielbeleg. Stagetyp und bekannte
   Übertragungsgröße werden weiterhin über Metadaten geprüft. Vor dem Rename wird ein
   unbestätigter Intent gespeichert; danach bleibt die fehlende Wiederherstellbarkeit
   ehrlich. Erwartung: zulässiger Upload ist erfolgreich; unklarer Ausgang wird
   weder automatisch wiederholt noch als bestätigtes Eigentum interpretiert.
5. **M5 Move ohne Hardlink:** direkten Same-volume-Rename mit gehaltenem Handle
   zulassen, wenn der Hardlink nicht verfügbar ist. Quellquarantäne, NoReplace,
   vorbereiteter Intent, tatsächliche Flush-/Namespaceprüfung und Fehlerbehalt
   bleiben aktiv. Erwartung: keine Stagekopie für diesen Fall; Konflikt, Abbruch,
   EXDEV und unbestätigte Veröffentlichung bleiben sicher.
6. **M6 Eigene Anti-Regressionsreview und Fallvorbereitung:** dieselben direkten
   und indirekten Ketten nach Implementierung erneut lesen; eng gebundene Fälle
   für die eine spätere Remote-Suite deklarieren und erforderliche Registrierungen
   dem Hauptagenten übergeben. Keine lokale Ausführung.

Status vor der Integrationsnachfrage: M1–M6 waren quellseitig umgesetzt und
statisch geprüft. Die anschließende RA-04-Quellenklärung widerlegt jedoch die
Copy-/Nichtleere-Roots-Einschränkung des ersten M3-Fixes. Abschnitt J führt
denselben Autorzyklus dafür fort; seine Ergebnisse ersetzen die früheren
M3-Aussagen. Laufzeit-, Compiler- und Veröffentlichungsbeleg stehen bei Main aus.

## E — Implementierungsergebnis und ergänzte vollständige Wirkungsketten

**M1:** `ftp/core/folder_reservation.rs::identity` und
`sftp/core/folder_reservation.rs::identity` liefern `Ok(None)`.
`directory_reservation_supported` bleibt die vorhandene Aussage über die
exklusive einzelne MKD/MKDIR-Anlage. Die Engine erreicht damit ihren bereits
vorhandenen direkten Erstlaufpfad. Weder ein Verzeichnisname noch FTP-Unique
werden als persistente Eigentumsidentität gespeichert.

**M2:** Die neue VFS-Operation `rename_reserved_directory` trennt die strikte
Veröffentlichung vom gewöhnlichen `Backend::rename_no_replace`.
Ihre vollständige direkte/indirekte Weitergabe lautet:

1. `engine/folder_reservation.rs::Reservations::publish` →
   `vfs/core/folder_reservation.rs::rename_reserved_directory` →
   `BackendExtensions::rename_reserved_directory`.
2. Direkter WebDAV-Backend → `webdav/core/extensions.rs` → bestehendes
   `webdav/core/folder_reservation.rs::rename` mit Resource-ID, REBIND und
   `Overwrite: F`. Gewöhnliches `WebdavBackend::rename_no_replace` verwendet
   stattdessen wieder `move_stage_time(..., false)`.
3. `CachingBackend` delegiert die strikte Operation an seinen echten Backend
   und invalidiert Quelle und Ziel auch bei einem fehlgeschlagenen Ergebnis.
4. `GuardedBackend` prüft beide Pfade mit seinen bestehenden `destructive`-
   Freigaben, delegiert die strikte Operation und bestätigt anschließend die
   Ziel-Schreibfreigabe. Es reicht keinen inneren Backend ungeprüft heraus.
5. `AgentBackend` verwendet für gewöhnliches Umbenennen weiterhin den alten
   `RenameNoReplace`-Frame. Die strikte Extension verlangt `folder_reservation`
   und sendet einmal den bestehenden `RenameDirectoryNoReplace`-Frame.
   `daemon/backend_ops::folder_operation` delegiert diesen Frame an die strikte
   VFS-Operation. `backend_server` behält für den gewöhnlichen Frame seinen
   bisherigen Backendaufruf. Der eigenständige se-Agent verwendet für den
   strikten Frame weiterhin seinen vorhandenen lokalen atomaren OS-Adapter.
6. `PeerBackend` → `peer_extensions::rename_reserved_directory` →
   `folder_reservation::rename` verlangt das ausgehandelte
   `folder_reservation_v1` für beide Orte. `call_once` sendet den neuen
   `FsRequest::RenameReservedDirectory` ohne die Retrylogik von `Attempts`.
   `FsRequest::effect` klassifiziert ihn als Schreiboperation;
   `peer_request_policy` erlaubt nur die Unit-Erfolgsantwort und keinen Readretry;
   `peer_fs_logging` benennt ihn getrennt. `server_fs` →
   `folder_reservation::serve` hält die bestehende autorisierte Mount-Lease →
   `FsAccess::rename_reserved_directory` löst beide Schreiborte auf, verlangt
   denselben Backend/Mount und delegiert an die strikte VFS-Operation.

Das Merkmal `folder_reservation_v1` existiert im letzten Releasebaum
`a814f263` noch nicht; dies wurde am Versionsdiff von `wire_capabilities.rs`
nachgeprüft. Die Erweiterung ergänzt denselben noch unveröffentlichten
Featurevertrag. Ein Altpeer erhält keinen neuen strikten Request. Sein
gewöhnlicher Renamevertrag bekommt keine neue Featurevoraussetzung.

Ein Provider mit Resource-ID, aber ohne atomare Binding-Veröffentlichung wird
beim bestätigten Unsupported-Ergebnis nicht pauschal unbenutzbar: Die Engine
prüft erneut ihre leere Stageidentität, entfernt diese Stage und reserviert den
endgültigen Namen durch die exklusive Verzeichnisanlage. Erst deren bestätigte
Antwort und nachfolgende Identitätsabfrage können einen neuen Eigentumsbeleg
erzeugen. Transportfehler, fremde Stage, fehlende Stage und verlorene
Publikationsantwort erlauben diesen Fallback nicht. Der neue strikte
Extensionvertrag legt ausdrücklich fest, dass Unsupported keine Mutation
ausgeführt hat; nachgelagerte Fehler werden zusätzlich durch die noch vorhandene
eigene Stage abgegrenzt.

**M3:** `app/core/transfer_route.rs::resume_job` klont den Auftrag für Copy
und lokale Moves, behält dieselbe Job-ID und setzt `RecoveryMode::Resume`.
Die Ergebnisroots ergänzen die bereits vorhandene Zuordnung; ein fehlgeschlagener
Folgeversuch mit leeren Roots löscht vorher aufgelöste Namen nicht. Vor der
ersten Rootplanung bleibt `resume = Some([])` möglich. Die neue Operation
`missing_retry::live` trennt diesen bestehenden aktuellen Bedienweg von
explizit gespeicherter Recovery mit `resume == None`. `job_store` bindet weiter
Endpunkte, Auswahl, Filter, Policy, Vault und Joblock. Bestätigte aktuelle
Publikationsantworten dürfen ohne persistente Objekt-ID den bisherigen
Größen-/Nichtüberschreibenpfad erneut erreichen. Unbekannte Intents werden
weiter streng geprüft, bevor ein neuer Intent geschrieben werden darf.
Aliasse ohne bestätigte Anlageantwort werden weder übernommen noch neu
nummeriert. Frühe Fehler vor dem Jobjournal oder vor tatsächlicher Mutation
und noch nicht geplante Wurzeln erhalten einen sicheren neuen Planungsversuch.
Vollständig bewegte ausgewählte Dateien werden nur über bestätigte Ergebnisse
und geprüfte Zielbelege ausgelassen; Quarantänebereinigung bleibt an die
vorhandenen starken Eigentumsbelege gebunden. Die vollständige korrigierte
Bedienkette, die Implementierung und die eigene Nachreview stehen in Abschnitt J.

**M4:** `file_evidence::observe_available` behandelt ausschließlich
PermissionDenied/Unsupported eines optionalen Remote-Zielbelegs als fehlenden
Beleg. `before` speichert trotzdem den Publikationsintent; `after_checked`
bestätigt ohne vollständigen Beleg kein persistentes Dateieigentum.
`validate_copy_target`/`validate_target_shape` verlangen eine vorhandene
reguläre Datei und die bekannte übertragene Größe. Google-native Exportquellen
behalten ihren vorhandenen `read_size == None`-Vertrag; unbekannte Exportlänge
wird nicht fälschlich zu Null.

Die tatsächliche FTP-Kette ist `FtpConnection::open_reader_from` →
`retr_as_stream` → RETR-Antwortklassifizierung → `FtpBackend::open_read`.
Ausschließlich eine RETR-550-Ablehnung bekommt PermissionDenied mit erhaltenem
Servertext. SIZE, MLST, LIST, MKD, RNFR und RNTO behalten ihre Fehlerabbildung;
550 wird nirgends zu einem Abwesenheitsbeweis umgedeutet. Der vorhandene
Agent-Textdecoder erkennt PermissionDenied, der Share-Fehlerkanal transportiert
die vorhandene typisierte Klasse. Quell-Lesefehler, lokale Fehler, Abbruch,
Timeout, ungültige Daten und verkürzte Streams werden nicht unterdrückt.

Die indirekten Verbraucher von `before`, `prepare_direct`, `prepare_batch`
und `after` sind weiterhin Upload, Remote-Stream, lokales FTP-Bridging,
Server-Copy, segmentierter Upload/Remote-Copy sowie Paketpublikation.
Jeder Verbraucher behält seinen Writerabschluss, Quellcheck, Größencheck,
Zielkonflikt und seinen konservativen Umgang mit unklarer Publikation.
`publication_state`, `verify_source`, Aliasvalidierung, Indexzähler und
Reparatur eines Pending-Intents behalten ihre starken Eigentumsbedingungen.
`verify_saved` und `verify_target` unterscheiden jetzt die aktuelle ausdrücklich
angeforderte Missing-only-Ausführung; ihre gespeicherte Recovery bleibt streng.
Kein fehlender Rücklesebeleg bestätigt dabei persistentes Eigentum.

**M5:** `direct_move::try_move` verwendet einen Hardlink weiterhin dort, wo
dieser tatsächlich verfügbar ist. Ohne Hardlink benennt es die bereits
gesicherte Quarantäne selbst um. Es hält den flushfähigen Quellhandle über
Preflight, Rename, Zielprüfung, tatsächlichen Datei-/Parentflush und
Publikationsbeleg. Die Zielprüfung bindet den erreichten Pfad an dasselbe
aktuelle Liveobjekt. Eine vor dem Rename festgehaltene FAT-Dateinummer wird
nicht als über Rename hinweg stabile Inkarnation behauptet.

Vor dem Commit wird Abbruch nach einer erfolgreichen Vorbereitung nochmals
behandelt; zuerst wird eine mögliche Publikationserlaubnis widerrufen, dann
die Quarantäne ohne Ersetzen zurückgestellt. Bei unbekanntem Renameergebnis
oder fehlgeschlagener Nachbestätigung wird kein bereits umbenannter Inhalt
gelöscht. Fehlermeldungen nennen für den Pfad ohne Hardlink keine vermeintlich
noch vorhandene Quarantänedatei. NoReplace-Konflikte, Rename-Nummerierung,
Skip, Overwrite, EXDEV-Stagefallback und der bisherige Pfad bei fehlenden
Flushrechten bleiben über dieselben vorhandenen Grenzen erhalten.

Zusatzquelle zur zweiten Recherche, geprüft 2026-10-26:
[NtFlushBuffersFileEx](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntflushbuffersfileex)
führt NTFS, ReFS, FAT und exFAT für Flags 0 auf. Das belegt die APIgrundlage,
keinen tatsächlich ausgeführten FAT-/exFAT-Lauf dieses Kandidaten.

## F — Eigene anschließende Anti-Regressionsreview

Prüfgegenstand war nach vollständiger Implementierung die gesamte oben
abgebildete Wirkungskette einschließlich ihrer unveränderten Verbraucher.
Methode: erneutes Lesen der Änderungen und Gegenstellen, Verfolgen von
Typ-/Fehler-/Besitzübergängen und Persistenz, Vergleich mit dem Releasevertrag,
anschließend Rust-AST-Parsing und Whitespaceprüfung. Der Reviewautor ist
derselbe Autor wie bei Fehlerauflistung, Plan und Fix.

| Vertrag | Eigener Befund nach der Umsetzung |
|---|---|
| FTP/FTPS/SFTP-Erstkopie | Unbekannte Objektidentität erreicht die direkte exklusive Anlage. MKD/MKDIR und eindeutige Kollisionsbehandlung bleiben erforderlich. SSH-Anmeldung, Hostschlüssel und SSH-Authentifizierungsagent werden nicht verändert. |
| WebDAV-Umbenennen | Desktop `confirm_rename`, Android `fs_edit::rename` und CLI `try_rename_fast` erreichen wieder den gewöhnlichen serverseitigen No-overwrite-MOVE. Die neue stärkere Ordnergarantie bleibt über alle beschriebenen Wrapper ausdrücklich getrennt. |
| Identität und Literale | Endpunkt-/Verbindungsidentitäten, Root- und Paarbedeutung sowie Literalnamen werden nicht neu kodiert. Die aktuelle Missing-only-Wiederholung behält dieselben Backend-Arcs; zwei Gegenstellen mit gleichlautendem relativem Pfad werden nicht gleichgesetzt. |
| Fehlende übertragen | Copy und lokale Moves bleiben über denselben sichtbaren Button erreichbar. Passende Dateien bleiben unangetastet; fehlende Dateien benutzen die bestätigten Ergebnisnamen. Frühe Fehler ohne Roots sind erneut planbar. Abweichende Dateien bleiben erhalten. Eine frühere Skip-Entscheidung wird nicht zu Merge; eine unbestätigte Rootanlage bekommt keinen geratenen Alias. Derselbe Journalbestand schützt jeden erneuten Klick. |
| Recovery | Gespeicherte Recovery verlangt weiter Quellen-/Ziel-/Objekt-/Policy-/Vaultbindung und vollständige Journale. Ein fehlender Rücklese-/Inkarnationsbeleg wird nicht durch Größe oder Metadaten als Eigentum ersetzt. Moves behalten die ursprüngliche Jobidentität. |
| Uploadrechte | Erlaubtes Schreiben und verweigertes Inhaltslesen bleiben getrennt. Nach fehlendem Rücklesen werden Stagetyp und bekannte Größe erneut verlangt. Fehlende, zu Ordnern ersetzte und in der Größe veränderte Stages können nicht als vollständiger Upload durchlaufen. |
| Fehler, Abbruch und Retry | Nur die benannten optionalen Belegklassen werden begrenzt. Transport-/Datenfehler und Abbruch bleiben Fehler. Ein unbekannter Publish-/Renameausgang erteilt weder automatische Wiederholerlaubnis noch Cleanupfreigabe. Neue Sharemutationen umgehen die Request-Retrylogik vollständig. |
| Rechte und Cache | Cachedelegation behält Liveprüfung und Invalidierung. Share prüft beide destruktiven Pfade und dieselbe Mount-Lease/Backendgrenze; Read-only-Exporte gelangen nicht zur Provider-Mutation. |
| Lokales Move/Backup | Hardlinkfähige Dateisysteme behalten den bisherigen Retentionspfad. Ohne Hardlink ist kein vollständiger Inhaltskopierpfad zwingend. Vor erfolgreicher Renameoperation bleiben Original/Quarantäne wiederherstellbar. Nach tatsächlichem Commit bleibt der Inhalt am Ziel; bei unklarem Ausgang wird keine vermeintlich entbehrliche Quelle bereinigt. Ersetzungs-/Backupprimitive wurden nicht verändert. |
| Sync, Versionen und Konfliktkopien | Die zusätzlich selbst gelesenen produktiven Renameverbraucher sind `bisync::version_save::{archive,rollback}`, `merge_recovery`-Originalwiederherstellung, `replacement_recovery::recover` und `Staged::publish_sibling`. Sie verarbeiten reguläre Archiv-/Stage-/Konfliktdateien, keine Collection-Veröffentlichung. Inhalt-/Signaturprüfung, Backupvoraussetzung, Nichtüberschreiben und getrennte Namespacebestätigung bleiben unverändert. Die zugehörigen tatsächlichen Fixturedelegationen wurden nachgelesen. |
| Oberflächen | Desktop und Android behalten ihre vorhandenen Aktionen und Ergebnisorte. CLI/MCP behalten ausdrücklich eingegebene Zielnamen und ihren bestehenden Merge-Standard. Es wurde keine neue Konten-, Dialog-, Berechtigungs- oder Featurevoraussetzung für gewöhnliches Umbenennen eingeführt. |

Während dieser eigenen Review wurden der Schutz zuvor übersprungener Ordner,
der Umgang mit fehlender Ergebniszuordnung, die Typ-/Größenprüfung ohne
Rückleserecht und die korrekte Fehlermeldung nach einem Move ohne Hardlink
noch im selben Fixzyklus ergänzt. Diese Korrekturen sind in den oben genannten
Ergebnissen enthalten. Es gibt nach dieser statischen Nachprüfung keinen
weiteren festgestellten unbehandelten Fehler innerhalb der ermittelten
Wirkungskette. Das ist kein ausgeführter Kompilier- oder Laufzeitnachweis.

## G — Vorbereitete Fälle und Registrierung durch Main

Alle nachfolgenden Namen sind vorhandene Rust-Testfunktionen dieses Fixstands.
Keine dieser Funktionen wurde lokal ausgeführt. Die neuen Testmodule sind
bereits in den zugehörigen Rust-Modulen eingebunden. Für die gemeinsame Suite
selektiert das bestehende Präfix `batches_` sie bereits grundsätzlich; Main muss
zusätzlich `batches_task_transfer_compat_` als erforderliches Präfix und die
Meilensteinzuordnung mit genau diesen `native_cases` im gemeinsamen Manifest
registrieren. Dieses Manifest und die CI-Orchestrierung wurden hier nicht
geändert. Die Fälle sind für dieselbe Linux-/Windows-Remote-Suite vorgesehen.

M1 — freie/kollidierende Ordner, fehlende Identität, unklare Anlageantwort:

```text
batches_task_transfer_compat_folder_without_identity_copies_and_numbers
batches_task_transfer_compat_folder_lost_response_is_not_recovered_without_identity
```

M2 — WebDAV-MOVE, striktes REBIND/Unsupported, Cache, Share und Daemon:

```text
batches_task_transfer_compat_folder_identity_does_not_require_binding_rename_for_first_copy
batches_task_transfer_compat_webdav_ordinary_folder_move_needs_no_binding_extension
batches_task_transfer_compat_webdav_reserved_move_keeps_strict_rebind_boundary_through_cache
batches_task_transfer_compat_reserved_share_rename_is_a_single_write_with_distinct_wire_operation
batches_task_transfer_compat_reserved_share_rename_accepts_only_unit_reply_and_never_read_retries
batches_task_transfer_compat_share_guard_preserves_strict_dispatch_and_read_only_denial
batches_task_transfer_compat_daemon_reserved_frame_keeps_strict_provider_boundary
```

M3 — tatsächlicher Button für Copy/Move, leere Roots vor/nach Jobanlage,
bestätigte Zielnamen, abweichende Dateien, früherer Ordner-Skip, Remote-Paare,
literale Namen, Uploadrechte, aufeinanderfolgende verlorene Antworten,
Journalverlust und geschützter Quellbesitz:

```text
batches_task_transfer_compat_missing_button_keeps_copy_move_and_empty_root_contract
batches_task_transfer_compat_missing_only_preserves_resolved_folder_and_skip_contract
batches_task_transfer_compat_missing_only_preserves_skipped_selected_folder
batches_task_transfer_compat_missing_only_remote_pairs_keep_literal_names_and_endpoints
batches_task_transfer_compat_missing_only_retries_before_journal_for_copy_and_move
batches_task_transfer_compat_missing_only_retries_empty_roots_after_no_id_listing_failure
batches_task_transfer_compat_missing_only_local_move_keeps_completed_pairs_and_conflicts
batches_task_transfer_compat_missing_only_lost_directory_reply_stays_protected_with_empty_roots
batches_task_transfer_compat_missing_only_lost_file_reply_survives_successive_clicks
batches_task_transfer_compat_missing_only_acknowledged_upload_needs_no_read_or_object_id
batches_task_transfer_compat_missing_only_keeps_stored_recovery_and_journal_loss_strict
batches_task_transfer_compat_missing_only_never_discards_unknown_move_quarantine
```

M4 — Uploadrechte, Kollisionsschutz, fehlende Recoverybelege, verlorene Antwort,
fehlende/ersetzte/veränderte Stages, tatsächliche Lesefehler und lokale Ablehnung:

```text
batches_task_transfer_compat_upload_only_target_completes_without_claiming_recovery
batches_task_transfer_compat_upload_only_lost_answer_never_republishes_or_removes_source
batches_task_transfer_compat_refused_readback_does_not_publish_missing_directory_or_resized_stage
batches_task_transfer_compat_readback_io_failure_is_not_permission_fallback
batches_task_transfer_compat_optional_remote_readback_never_hides_local_permission_error
```

M5 — Rename ohne Hardlink/Inhaltskopie, Belegfehler, unbekannte Commitantwort,
widerrufene Vorbereitung, Abbruch und vollständige Dateikonfliktpolitik:

```text
batches_task_transfer_compat_move_without_hardlinks_does_not_copy_bytes
batches_task_transfer_compat_move_without_hardlinks_retains_committed_target_on_receipt_failure
batches_task_transfer_compat_move_without_hardlinks_keeps_unknown_committed_result
batches_task_transfer_compat_move_without_hardlinks_restores_failed_preparation_and_cancel
batches_task_transfer_compat_move_without_hardlinks_preserves_conflicts
```

Grenzen dieser Fallvorbereitung: Die neuen Backend-Doubles üben produktive
Engine-/VFS-/Copy-/Guard-/Daemonpfade mit kontrollierten Fähigkeiten und Fehlern
aus. Die WebDAV-Fälle benutzen auf dem späteren Runner einen eigenen
Loopback-HTTP-Server. Die Movefälle unterdrücken Hardlinkverfügbarkeit gezielt;
sie erzeugen kein tatsächliches FAT32-/exFAT-Volume. Sie ersetzen keinen
ausgeführten FTP-/FTPS-/SFTP-Serverlauf, keinen vollständigen Share-/Agent-
Netzwerklauf und keinen Android-UI-Lauf. Bestehende gemeinsame Integrationen
für Backupfehler, Source-/Destinationdrift, Baseline-Retry und geschützte
Auslassungen bleiben im Manifest unverändert und gehören zu Mains einer Suite.


## H — Gelesene, geänderte und erstellte Dateien

Die folgenden Listen beziehen sich auf diese isolierte Arbeitskopie.
„Gezielt gelesen“ umfasst vollständige Dateien oder die entscheidungsrelevanten
Funktions-/Aufruferausschnitte; es behauptet keine zeilenweise Komplettlektüre
jeder Datei. Die AST-only-Liste bezeichnet ausschließlich Syntaxlektüre.

Erstellt:

- `docs/plaene/2026-10-24-batches/fix-regression-transfer.md`
- `native/src/copy/os/shared/batches_direct_move_compatibility_task_tests.rs`
- `native/src/transfer/os/shared/engine/batches_backend_compatibility_task_tests.rs`
- `native/src/transfer/os/shared/engine/batches_publication_compatibility_task_tests.rs`
- `native/src/transfer/os/shared/engine/missing_retry.rs`
- `native/src/transfer/os/shared/engine/batches_missing_retry_compatibility_task_tests.rs`

Geändert:

- `docs/oberflaechen/uebertragungen.md`
- `native/src/agent/core/backend.rs`
- `native/src/agent/core/extensions.rs`
- `native/src/app/core/transfer_route.rs`
- `native/src/copy/os/shared/direct_move.rs`
- `native/src/daemon/os/shared/backend_ops.rs`
- `native/src/ftp/core/folder_reservation.rs`
- `native/src/ftp/core/streams.rs`
- `native/src/sftp/core/folder_reservation.rs`
- `native/src/share/core/folder_reservation.rs`
- `native/src/share/core/fs_access.rs`
- `native/src/share/core/fs_guard_extensions.rs`
- `native/src/share/core/fs_request.rs`
- `native/src/share/core/peer_extensions.rs`
- `native/src/share/core/peer_fs_logging.rs`
- `native/src/share/core/peer_request_policy.rs`
- `native/src/share/core/server_fs.rs`
- `native/src/share/core/wire_capabilities.rs`
- `native/src/transfer/os/shared/engine/file_evidence.rs`
- `native/src/transfer/os/shared/engine/file_recovery.rs`
- `native/src/transfer/os/shared/engine/folder_reservation.rs`
- `native/src/transfer/os/shared/engine/mod.rs`
- `native/src/transfer/os/shared/engine/job_store.rs`
- `native/src/transfer/os/shared/engine/roots.rs`
- `native/src/vfs/core/cache_extensions.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/folder_reservation.rs`
- `native/src/vfs/mod.rs`
- `native/src/webdav/core/extensions.rs`
- `native/src/webdav/core/webdav.rs`

Zusätzlich zu sämtlichen geänderten/erstellten Dateien gezielt gelesen:

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/batches.md`
- `docs/plaene/2026-10-24-batches/regressionsreview.md`
- `docs/plaene/2026-10-24-batches/vertraege.md`
- `graphify-out/graph.json`
- `native/Cargo.lock`
- `native/review-task-manifest.json`
- `native/src/agent/core/agent_error.rs`
- `native/src/agent/core/folder_reservation.rs`
- `native/src/agent_proto/core/folder_frames.rs`
- `native/src/agent_proto/core/server.rs`
- `native/src/agent_proto/core/transport_error.rs`
- `native/src/agent_proto/core/types.rs`
- `native/src/app/core/transfer_center.rs`
- `native/src/app/core/transfer_center_tests.rs`
- `native/src/app/core/transfer_route_tests.rs`
- `native/src/app/core/transfer_window.rs`
- `native/src/app/os/shared/file_actions.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/bisync/os/shared/apply_transfer_tests.rs`
- `native/src/bisync/os/shared/engine_provider_fixture.rs`
- `native/src/bisync/os/shared/merge_recovery.rs`
- `native/src/bisync/os/shared/merge_task_fixture.rs`
- `native/src/bisync/os/shared/replacement_recovery.rs`
- `native/src/bisync/os/shared/sync_reliability_task_backend_fixture.rs`
- `native/src/bisync/os/shared/sync_reliability_task_spelling_migration_tests.rs`
- `native/src/bisync/os/shared/test_remote.rs`
- `native/src/bisync/os/shared/tests/dedupe.rs`
- `native/src/bisync/os/shared/tests/hash_walk.rs`
- `native/src/bisync/os/shared/tests/incremental.rs`
- `native/src/bisync/os/shared/tests/incremental_safety_fixture.rs`
- `native/src/bisync/os/shared/tests/move_retry.rs`
- `native/src/bisync/os/shared/tests/safety_backends.rs`
- `native/src/bisync/os/shared/version_save.rs`
- `native/src/cli/engine_copy.rs`
- `native/src/cli/folder_conflict.rs`
- `native/src/cli/transfer.rs`
- `native/src/cli/transfer_recovery.rs`
- `native/src/cli/tree_ops.rs`
- `native/src/copy/os/linux_file_proof.rs`
- `native/src/copy/os/linux_os.rs`
- `native/src/copy/os/shared/durability.rs`
- `native/src/copy/os/shared/move_guard.rs`
- `native/src/copy/os/shared/publication.rs`
- `native/src/copy/os/shared/publication_proof.rs`
- `native/src/copy/os/shared/recovery.rs`
- `native/src/copy/os/shared/safe_file.rs`
- `native/src/copy/os/shared/safe_file_tests.rs`
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/windows_file_proof.rs`
- `native/src/copy/os/windows_identity.rs`
- `native/src/copy/os/windows_publication_failure.rs`
- `native/src/daemon/os/shared/backend_server.rs`
- `native/src/ftp/core/errors.rs`
- `native/src/ftp/core/extensions.rs`
- `native/src/ftp/core/ftp.rs`
- `native/src/ftp/core/io_adapters.rs`
- `native/src/ftp/core/staging.rs`
- `native/src/ftp/core/transfer_engine_task_tests.rs`
- `native/src/ftp/core/transfer_fixture.rs`
- `native/src/ftp/mod.rs`
- `native/src/local_access/os/windows/confined.rs`
- `native/src/local_access/os/windows/publication.rs`
- `native/src/mobile/os/shared/fs_edit.rs`
- `native/src/mobile/os/shared/transfer.rs`
- `native/src/sftp/core/backend.rs`
- `native/src/sftp/core/extensions.rs`
- `native/src/share/core/backend.rs`
- `native/src/share/core/export_config.rs`
- `native/src/share/core/fs.rs`
- `native/src/share/core/fs_error.rs`
- `native/src/share/core/fs_guard_backend.rs`
- `native/src/share/core/fs_guard_backend_task_tests.rs`
- `native/src/share/core/fs_guard_names.rs`
- `native/src/share/core/fs_guard_stream.rs`
- `native/src/share/core/fs_policy.rs`
- `native/src/share/core/peer_request.rs`
- `native/src/share/core/peer_reversible_replace.rs`
- `native/src/share/core/peer_stream.rs`
- `native/src/share/core/peer_telemetry.rs`
- `native/src/share/core/wire.rs`
- `native/src/share/mod.rs`
- `native/src/share/os/shared/fs_host_policy.rs`
- `native/src/transfer/core/folder_conflict.rs`
- `native/src/transfer/core/job.rs`
- `native/src/transfer/core/job_record.rs`
- `native/src/transfer/os/shared/engine/batch_put.rs`
- `native/src/transfer/os/shared/engine/discovery.rs`
- `native/src/transfer/os/shared/engine/file_recovery_fix_tests.rs`
- `native/src/transfer/os/shared/engine/file_recovery_tests.rs`
- `native/src/transfer/os/shared/engine/folder_alias_store.rs`
- `native/src/transfer/os/shared/engine/folder_move.rs`
- `native/src/transfer/os/shared/engine/folders.rs`
- `native/src/transfer/os/shared/engine/folders_local.rs`
- `native/src/transfer/os/shared/engine/folders_tests.rs`
- `native/src/transfer/os/shared/engine/job_store.rs`
- `native/src/transfer/os/shared/engine/legacy.rs`
- `native/src/transfer/os/shared/engine/local.rs`
- `native/src/transfer/os/shared/engine/ops.rs`
- `native/src/transfer/os/shared/engine/publish.rs`
- `native/src/transfer/os/shared/engine/remote_copy.rs`
- `native/src/transfer/os/shared/engine/selection.rs`
- `native/src/transfer/os/shared/engine/source.rs`
- `native/src/transfer/os/shared/engine/test_backend.rs`
- `native/src/transfer/os/shared/engine/test_run.rs`
- `native/src/transfer/os/shared/engine/tests_batch.rs`
- `native/src/transfer/os/shared/engine/tests_remote.rs`
- `native/src/transfer/os/shared/engine/tests_review_packets.rs`
- `native/src/transfer/os/shared/engine/upload.rs`
- `native/src/transfer/os/shared/engine/worker.rs`
- `native/src/vfs/core/backend_defaults.rs`
- `native/src/vfs/core/cache.rs`
- `native/src/vfs/core/cache_load.rs`
- `native/src/vfs/core/cache_state.rs`
- `native/src/vfs/core/cache_support.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/meta.rs`
- `native/src/vfs/core/promotion.rs`
- `native/src/vfs/core/scheme.rs`
- `native/src/vfs/os/linux_os/local_platform.rs`
- `native/src/vfs/os/shared/confined_local_backend.rs`
- `native/src/vfs/os/shared/confined_local_extensions.rs`
- `native/src/vfs/os/shared/local.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/shared/local_file_identity.rs`
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/webdav/core/copy_writer_task_tests.rs`
- `native/src/webdav/core/folder_reservation.rs`
- `native/src/webdav/core/stage_move.rs`
- `native/src/webdav/core/transfer_engine_task_tests.rs`
- `native/src/webdav/mod.rs`

Zusätzlich ausschließlich als Rust-AST gelesen:

- `native/src/transfer/os/shared/engine/batch.rs`
- `native/src/transfer/os/shared/engine/batch_get.rs`
- `native/src/transfer/os/shared/engine/bottleneck.rs`
- `native/src/transfer/os/shared/engine/dispatch.rs`
- `native/src/transfer/os/shared/engine/download.rs`
- `native/src/transfer/os/shared/engine/issues.rs`
- `native/src/transfer/os/shared/engine/issues_vault_tests.rs`
- `native/src/transfer/os/shared/engine/limiter_tests.rs`
- `native/src/transfer/os/shared/engine/local_extras_tests.rs`
- `native/src/transfer/os/shared/engine/local_plan.rs`
- `native/src/transfer/os/shared/engine/queue.rs`
- `native/src/transfer/os/shared/engine/record_io.rs`
- `native/src/transfer/os/shared/engine/segments.rs`
- `native/src/transfer/os/shared/engine/segments_copy.rs`
- `native/src/transfer/os/shared/engine/segments_copy_tests.rs`
- `native/src/transfer/os/shared/engine/segments_download.rs`
- `native/src/transfer/os/shared/engine/segments_tests.rs`
- `native/src/transfer/os/shared/engine/segments_upload.rs`
- `native/src/transfer/os/shared/engine/segments_upload_tests.rs`
- `native/src/transfer/os/shared/engine/stats.rs`
- `native/src/transfer/os/shared/engine/test_faults.rs`
- `native/src/transfer/os/shared/engine/tests_flow.rs`
- `native/src/transfer/os/shared/engine/tests_local.rs`
- `native/src/transfer/os/shared/engine/tests_review.rs`
- `native/src/transfer/os/shared/engine/util.rs`
- `native/src/transfer/os/shared/engine/view.rs`

Außerhalb der Arbeitskopie ausschließlich gelesen: `/root/.codex/AGENTS.md`,
`/root/.codex/skills/graphify/SKILL.md`,
`/tmp/smartexplorer-final-review-user-contract.txt`,
`/tmp/smartexplorer-fix-transfer-compat-scope.json` und die jeweils zugewiesenen
Befundabschnitte in
`/home/CodeProjects/smartexplorer/smart-explorer/docs/plaene/2026-10-24-batches/review-regression-astra.md`
sowie
`/home/CodeProjects/smartexplorer/smart-explorer/docs/plaene/2026-10-24-batches/review-regression-sol.md`.
Die oben verlinkten externen Primärquellen wurden über das Webwerkzeug gelesen.

## I — Nachweisgrenzen und Übergabe

- Die eigenen Änderungen liegen ausschließlich auf
  `codex/review-fixes-ee75e91b` in der zugewiesenen Arbeitskopie.
  Dieser Autor hat keinen Commit, Push, Build, Formatter, Compiler, Test,
  Server, Paketinstaller, Graphrefresh oder Release ausgeführt.
- Die statische Endkontrolle verwendete den bereits installierten
  Tree-sitter-Rust-Parser aus der Graphify-Pythonumgebung. In den eigenen
  geänderten Rustdateien zeigte der Parser keine Syntaxfehler;
  `git diff --check` zeigte keinen Whitespacebefund. Das beweist weder
  Rust-Typprüfung noch bestandene Testfunktionen. Sämtliche eigenen neuen
  oder geänderten Rustdateien liegen unter 500 Zeilen und 50 KiB.
- Der Quellgraph wurde als Navigationshilfe abgefragt. Die CLI meldete
  Skillversion 0.9.11 gegenüber Paketversion 0.9.17. Dynamische Traitaufrufer
  wurden deshalb und wegen der AST-Grenzen anhand des tatsächlichen Codes
  nachverfolgt. Der vollständige Rootgraphrefresh nach Integration bleibt Main
  zugewiesen; dieser Autor hat keinen Teilgraph erzeugt.
- Main hat den tatsächlichen Startaufruf mit
  `model="gpt-6.1-sol"`, `reasoning_effort="max"`, `fork_turns="none"`
  bestätigt. Unabhängige Antwortmodell-Metadaten oder zusätzliche
  Transkript-Modelleinträge stellt der Host nach Mains Auskunft nicht bereit.
  Belegt ist somit der Start mit diesen Parametern, keine unabhängige
  Verifikation des tatsächlichen Antwortmodells.
- Ein tatsächlicher FTP-/FTPS-/SFTP-/WebDAV-Providerlauf, ein tatsächliches
  FAT32-/exFAT-Volume, eine vollständige Share-/Agent-Netzwerksitzung und die
  sichtbaren Desktop-/Android-Bedienwege wurden hier nicht ausgeführt.
  Fehlende Livezugänge werden nicht als bestandene Abnahme dargestellt und
  sind nach Projektvorgabe keine Voraussetzung für den terminalen Release.
- Es besteht keine noch ungeklärte erforderliche Quelländerung außerhalb der
  zuletzt erweiterten exklusiven Schreibfläche. Main muss die beschriebenen
  Fälle im gemeinsamen Manifest zuordnen, den integrierten Kandidaten
  committen/pushen, die eine Remote-Suite auswerten, den Rootgraph aktualisieren
  und den einen terminalen vollständigen Remote-Release ausführen.

## J — RA-04: fortgesetzter eigener Autorzyklus nach Integrationsnachfrage

**A — Auftrag und widerlegte Grundlage:** Main meldet vor dem Commit, dass
der Release `a814f263` auch lokale Moves und leere Ergebnisroots über
„Fehlende übertragen“ wiederholen konnte. Die erste M3-Einschränkung war eine
zu enge eigene Folgerung, keine Nutzergrenze. Der vollständige bisherige
Bedienweg bleibt beauftragt; unbekannte Publikationen und Quellbesitz bleiben
geschützt. Schreib-/Ausführungsgrenzen bleiben unverändert.

**B — tatsächlicher Bestand:** `FinishedEntry::can_resume` prüft nur
`job.is_some()`, Unvollständigkeit und `TransferPurpose::Copy`. Dieser Zweck
umfasst lokale Paste-/Dialog-Moves; `build_job` schließt ausschließlich
Remote-Moves aus. `take_resume` führt beide lokalen CopyModes in `resume_job`.
Im Release klont diese Funktion ohne Modus- oder Roots-Bedingung. Auch ein
Enginefehler vor erfolgreicher Rootplanung liefert `Done` mit leeren Roots und
erhält den Auftrag in der Lane. Die neue Journalpipeline prüft dagegen den
Zielroot bereits vor dem erneuten Planen auf stabile Objektidentität.

**C — Entscheidungsplan:** Einen neuen Job ohne Prüfung des alten Journals
zu starten wird verworfen: Eine verlorene Publishantwort würde dadurch ihre
Sperre verlieren. CopyMode oder eine leere Roots-Liste allein beweisen weder
eine sichere noch eine unklare Ausführung. Gewählt wird derselbe Job mit einer
getrennten aktuellen Missing-only-Prüfung (`resume.is_some()`) und unveränderter
strikter gespeicherter Recovery (`resume == None`). Eine zusätzlich gespeicherte
bestätigte Ausführungsantwort bleibt von dauerhaftem Objekteigentum getrennt.
Unbestätigte Intents werden weiterhin über die vorhandenen starken Belege
aufgelöst oder gesperrt; ein neuer Intent überschreibt niemals diese Prüfung.

**D — konkrete Ergänzung M3:**

1. `resume_job` behält für Copy und lokale Moves dieselbe Job-ID und markiert
   die aktuelle Missing-only-Ausführung mit den Ergebnisroots, auch bei leerer
   Liste. Endpunkte, Auswahl, Filter, Konflikte und Zielnamen bleiben erhalten.
2. Job-/Dateijournal unterscheiden eine bestätigte aktuelle Publishantwort
   von einer restartfest bestätigten Objektidentität. Ohne Objekt-ID dürfen
   bestätigte Copy-Ergebnisse erneut nach den bestehenden Größen-/Skipregeln
   geprüft werden. Unklare Intents behalten ihre bisherige Recoveryprüfung.
3. Frühe Fehler ohne Mutation dürfen Root- und Quellbelege neu erheben; fehlende
   alte Jobdaten werden nur für die aktuelle leere Wiederholung ohne verbliebene
   Datei-/Aliasjournale als nie gestartete Ausführung behandelt. Gespeicherte
   Recovery verlangt weiter ihren Datensatz.
4. Rootplanung unterscheidet tatsächlich aufgelöste Aliasse von noch nicht
   geplanten Wurzeln. Bestätigte No-ID-Aliasse dürfen im aktuellen Bedienweg
   unter exakt demselben Namen weiterlaufen; unbestätigte Anlageantworten
   werden weder zu Merge noch zu einer neuen Nummer. Lokale Movequellen
   dürfen nur mit bestätigtem Ergebnis ausgelassen oder mit ihren bisherigen
   starken Belegen bereinigt werden.
5. Eigene Nachreview verfolgt Button, Lane, Record, Alias, Dateipublikation,
   Quarantäne und Konflikte erneut. Genau vorbereitete Fälle für Copy/Move,
   leere Roots, fehlende IDs, verlorene Antworten und wiederholten Abbruch
   werden derselben Remote-Suite zugeordnet; hier wird nichts ausgeführt.

**Implementierungsergebnis:** Die Ergänzung ist im selben Fixzyklus umgesetzt.
`resume_job` setzt keine neue Job-ID mehr. Die aktuelle Wiederholung bleibt in
`JobView`, Lane und jedem nachfolgenden Klick auf demselben Journal. Der neue
Rootabgleich behält bei einem fehlgeschlagenen Folgeversuch die vorherigen
Ergebnisnamen, statt sie durch eine leere Ergebnisliste zu ersetzen. Der neue
Helper `file_recovery::missing_retry` bündelt die Abgrenzung. Das additive
Receiptfeld `acknowledged` wird ausschließlich nach einer erfolgreichen
`after_checked`-Bestätigung geschrieben, bei jedem neuen Intent auf false
initialisiert und bei Widerruf ebenfalls zurückgesetzt. Es ist unabhängig von
`confirmed`, das weiterhin einen beobachteten Objektbeleg verlangt.
[Serde-Feldattribute](https://serde.rs/field-attrs.html), geprüft 2026-10-26,
belegen `#[serde(default)]`: ältere Datensätze erhalten für das neue Boolfeld
false; vorhandene echte `confirmed`-Belege bleiben verwendbar.

`verify_saved` validiert weiterhin sämtliche Einträge und Indexzähler. Nur für
diese aktuelle Wiederholung dürfen bestätigte Copyantworten ohne destructive
Recoveryarbeiten den normalen Missing-only-Dateipfad erreichen. Lokale Moves
mit noch vorhandener Quarantäne gehen weiterhin durch die strikte Beleg- und
Bereinigungsprüfung. Ohne Quarantäne wird eine fehlende ausgewählte Movequelle
nur nach bestätigter Antwort und geprüftem Zielbeleg ausgelassen. Dateien mit
unklarer Publishantwort behalten unabhängig von der Roots-Liste ihre Sperre.
Ein wiederholter Klick kann weder die ursprüngliche Journal-ID wechseln noch
einen unbestätigten Eintrag überschreiben.

`job_store` darf vor tatsächlicher Mutation frische Quell-/Zielbelege erheben.
Ein vorher unbestätigter Quellpfad wird erst nach Prüfung sämtlicher Intents
anhand eines jetzt vorhandenen aktuellen Objekts bestätigt. Eine schon bekannte
Objektbindung wird dabei nicht überschrieben. Ein fehlender Jobdatensatz wird
nur im aktuellen leeren Bedienweg ohne verbliebenes Datei- oder Aliasjournal
als Fehler vor `begin` behandelt. Ein fehlender Index nach Mutation bleibt ein
Fehler; gespeicherte Recovery erhält keine Abwesenheitsausnahme.

`roots::plan_roots` unterscheidet Ergebnis-/gespeicherte Aliasse von noch nicht
geplanten Wurzeln. Die erste Gruppe behält ihre exakten Namen; ein bestätigter
No-ID-Ordner verlangt keine erfundene Identität. Nicht bestätigte MKD/MKCOL-
Antworten benutzen dagegen die bestehende starke Auflösung oder bleiben
unbestätigt. Noch nicht geplante Verzeichnisse behalten ihre exklusive
Ordnerkonfliktentscheidung. Unaufgelöste reguläre Dateien/Pairs benutzen im
Missing-only-Durchgang den bisherigen Namen und bekommen keine neue Nummer.

**Eigene anschließende Nachreview:** Erneut selbst verfolgt wurde die Kette
`transfer_window::finished_row` → `FinishedEntry::can_resume` →
`take_resume` → `resume_job` → `transfer_jobs::submit_job/submit_job_for` →
`folder_conflict_ui::requires_preview` → `TransferCenter::submit` → Lane →
`run_job/run_view/prepare` → Job-/Dateijournal → Alias-/Rootplanung → lokaler,
Upload- und Remote-Copy-Worker → Publikationsantwort → `Done`/Lane/FinishedEntry.
`requires_preview` behält bei `resume.is_some()` die vorhandene Entscheidung;
es wird beim Klick keine neue Policy eingeschoben. Die Lane hält den geklonten
Auftrag einschließlich derselben Job-ID für den nächsten Klick fest.

| Erneut geprüfter Vertrag | Eigener statischer Befund |
|---|---|
| Sichtbarer Move-Button | `TransferPurpose::Copy` schließt `CopyMode::Move` nicht aus. Der ergänzte Appfall prüft genau `FinishedEntry::can_resume` für einen unvollständigen lokalen Move. Remote-Moves bleiben durch den vorhandenen Jobvalidator abgelehnt. |
| Früher Fehler vor Jobanlage | Ein als Datei blockierter lokaler Zielordner scheitert vor `begin`; nach Beseitigung des Hindernisses darf Copy oder Move mit leerer Ergebnisliste starten. Vorhandene Datei-/Aliasjournale schließen diesen Sonderfall aus. |
| Früher Fehler nach Jobanlage | Ein verweigertes Remote-Listing liegt vor `prepare_apply`, daher existiert kein Mutationseintrag. Beim erneuten Versuch werden keine stabilen Provider-IDs vorausgesetzt. |
| Teilweise lokaler Move | Fehlende erfolgreich bewegte ausgewählte Dateien brauchen bestätigten Zielinhalt. Noch ausstehende Dateien bleiben in der Auswahl. Ein vorhandenes abweichendes Ziel wird gemeldet; die betreffende Quelle bleibt liegen. |
| Verlorene MKD-/Publishantwort | Erneute Klicks mit zunächst vorhandenen und danach leeren Roots halten dieselbe Journal-ID. Nicht bestätigte Ordner-/Dateipublikationen werden weder zu Skip noch zu Renummerierung oder neuer Publikation. |
| Quarantäne und Ersatz | Eine noch vorhandene Quarantäne umgeht die strikte Recovery nie. Ein nach verlorener Antwort durch ein gleich großes gleichinhaltiges Fremdobjekt ersetztes Ziel gibt keine Quellbereinigung frei. |
| Rechte und fehlende IDs | Ein bestätigter erlaubter Upload bleibt nach verweigertem Rücklesen aktuell wiederholbar. Die Kopie kann fehlende Dateien ergänzen, bestätigt aber weiterhin kein dauerhaftes Eigentum. |
| Gespeicherte Recovery und Journalverlust | `resume == None` erreicht keine Missing-only-Ausnahme. Zielobjekt-, Quellen-, Policy-, Vault-, Lock-, Index-/Pending- und Eigentumsprüfungen bleiben dort wirksam. Ein fehlender Index wird nicht neu auf Null gesetzt. |
| Unveränderte fünf Fixketten | Erstkopie, gewöhnliches/striktes Rename, RETR-Rechtegrenze und direkter Move verwenden weiterhin ihre beschriebenen Pfade. Das neue Boolfeld allein erlaubt keine Mutation und keine Cleanupoperation. |

Die genaue Fallliste ist in Abschnitt G ergänzt; der bisherige Appfall wurde
für den tatsächlichen Copy-/Move-/Leerroots-Vertrag umbenannt. Alle neuen
Module sind quellseitig eingebunden. Diese Nachreview ist eine statische
Autorprüfung, keine ausgeführte Suite.

Abschluss der Ergänzung: Eigene Rust-AST-Prüfung, Dateigrenzenkontrolle,
Whitespaceprüfung und der Abgleich deklarierter Fallnamen mit vorhandenen
Funktionen zeigten keine Befunde. Kompilierung und Ausführung stehen weiterhin
bei Main aus; im fortgesetzten Autorzyklus wurde kein lokaler Build, Compiler,
Formatter, Test oder Server gestartet.


## K — Fachliche Modulaufteilung nach Remoteformatierung

**A — fortgesetzter Auftrag und Nachweis:** Die Remote-Vorbereitung
`40332971455` auf Kandidat `34f3361de13104e386eba26bf16d6a0160475481` hat
nach tatsächlicher Rustformatierung Größenverletzungen gemeldet. Laut Main
wurde dabei keine Kompilierung gestartet. Der gebundene Format-/Abhängigkeitspatch
liegt als Commit `10988167` in derselben Arbeitskopie. Die frühere reine
Zeilenzählung vor Rustformatierung bewies die normale Modulgröße nicht; diese
Grenze wird hier korrigiert. Maßgeblich bleiben dieselben Transferverträge und
die aktualisierte exklusive Scope-Datei. Lokaler Formatter, Build, Compiler,
Test, Server, Unteragent, Gitänderung und Graphrefresh bleiben verboten.

**B — belegter Bestand und vollständige Strukturgrenzen:** Die zehn Ziele
umfassen Agent-Extensions (516 Zeilen), Picker (558), CLI-Zielauflösung (517),
Dateistaging (508), Share-Dateiserver (502), Dateirecovery (1.025), deren
Testfixture/-fälle (524), Jobstore (724), Engine-Einstieg (523) und
Backend-Testdouble (501). Quelle sind der aktuelle formatierte Quellstand,
`/tmp/smartexplorer-rv1-40332971455/format.json` und das von Main bereitgestellte
AST-Inventar `/tmp/smartexplorer-rv1-size-inventory.json`. Es wird keine
Größenausnahme und keine künstliche Verdichtung verwendet.

**C — gewählte Aufteilung:** Unveränderte fachliche Blöcke ziehen in private
Kindmodule. Bestehende Aufrufer behalten ihre Namen über eng begrenzte
Weitergabe; zusätzliche Sichtbarkeit reicht nur bis zum benötigten bisherigen
Elternmodul. Testmodule behalten ihre Modulnamen und Funktionssuffixe. Ein
gemeinsames unspezifisches Hilfsmodul oder eine verkürzte Formatierung wird
verworfen, weil beides die Verantwortung nicht trennt. Neue Protokolle,
Persistenzfelder, Featuregates, Copy- oder Wiederholentscheidungen entstehen
bei dieser Aufteilung nicht.

**D — konkreter Strukturplan und erwarteter Vertragserhalt:**

| Verantwortung | Aufteilung und Verknüpfung | Erforderliches Ergebnis |
|---|---|---|
| Agent-Änderungssignale | `extensions.rs` delegiert Watch-Modus und Abonnement an `extensions_watch.rs`; Queue-Zustellung und Drop-Guard bleiben dort zusammen. | Featurefallback, Cancel-Poll, Abschlussmeldung und Threadjoin bleiben unverändert; normale und reservierte Rename-Frames bleiben in der Extension. |
| Picker-Navigation | `picker_impl.rs` behält die Dialogdarstellung; `picker_navigation.rs` trägt Öffnen, lokale Orte, Parent- und Wertbildung sowie den Childpfadhelper. | Lokale/Remote-Backends, Präfix, Verbindungsidentität und alle Pickerzwecke bleiben erhalten. |
| CLI-Zielauflösung | Inline-Testmodule ziehen bei gleichem Modulnamen nach `target_tests.rs`. | Zielresolver, Vaultbindung, gespeicherte Verbindungen und literal erhaltene Pfade ändern sich nicht; Fallnamen bleiben gleich. |
| Dateistaging | Inline-Testmodul zieht nach `staging_tests.rs`. | Quellhandle, Stagename, Löschgrenze, Datenkopie, Abbruch und Rückgabefehler bleiben im produktiven Staging unverändert. |
| Share-Streaming | `server_fs.rs` behält Autorisierung und Requestdispatch; `server_fs_io.rs` bündelt die vorhandenen Lese-/Schreibstreams. | Mount-Lease, guarded backend, Wireantwort, Schreibmodus und I/O-Besitz bleiben unverändert. |
| Dateirecovery | `file_recovery_receipts.rs` enthält Modelle/Index/Journalzugriff; `file_recovery_verify.rs` die gespeicherte Prüfung; `file_recovery_observer.rs` den lokalen Quarantäne-/Publishobserver. | Intent-/Acknowledged-/Confirmed-Grenzen, Pending-Reparatur, Indexvollständigkeit, Backup-/Quellbesitz und Live-Missing-Retry bleiben gleich. |
| Recovery-Testfixture | `file_recovery_fixture.rs` enthält nur die bestehenden Fixturehelfer. | Produktive Prüfpunkte, die bisherigen Testmodule und deren genaue Fallnamen bleiben gleich. |
| Jobvertrag | `job_store_validation.rs` bündelt Endpunkt-/Quellsnapshot und Wiederaufnahmevalidierung; `job_store_catalog.rs` Katalog und Bindung gespeicherter Jobs. | Vertragsvergleich, Vaultkontext, Lock, Mutation, live Wiederholung und strikte gespeicherte Recovery bleiben gleich. |
| Engine-Vorbereitung | `prepare.rs` übernimmt die vorhandene Vorbereitung; `mod.rs` bleibt Einstieg und Laufzeitbesitz. | Reihenfolge von Preflight, Rootauflösung, Joblock, Aliasplanung und Quellenbestätigung bleibt gleich. |
| Backend-Teststreams | `test_backend_streams.rs` bündelt Guard, Reader und Writer; das Testbackend behält Fähigkeiten und Backenddispatch. | Fehlerzeitpunkte, Zähler, Chunk-/Lesefehler und RA-04-/Publikationsorakel bleiben gleich. |

Eigene anschließende Anti-Regressionsprüfung: Modulpfade und tatsächlich
benötigte Sichtbarkeit nachlesen; die verschobenen Funktionskörper statisch
gegen `10988167` vergleichen; exakte Fallnamen, sämtliche neuen/berührten
Dateigrößen und Rust-AST-Syntax kontrollieren. Reale Rustformatierung,
Kompilierung und Ausführung erfolgen ausschließlich später durch Main im
selben Remote-Suiteeinstieg. Fortschritt: Strukturplan vor Umsetzung gesichert.


**Implementierungsergebnis und eigene Nachreview:** Die zehn in D genannten
Verantwortlichkeiten sind aufgeteilt. `picker_entries.rs` wird nicht benötigt:
Die komplette Dialogdarstellung bleibt nach Auslagerung der Navigation ein
zusammenhängender Dialog unterhalb der Dateigrenze. Alle verwendeten neuen
Dateien sind über private Module eingebunden; die produktiven und die bisherigen
Test-Einstiegspunkte bleiben über ihre bisherigen Namen erreichbar.
`prepare` bleibt im Engine-Elternmodul auch als bisheriger Funktionsname erhalten.

Die verwendeten Sprachgrenzen wurden anhand der offiziellen Rustreferenz
geprüft (Abruf 2026-10-26):
[Module und Pfadattribute](https://doc.rust-lang.org/reference/items/modules.html),
[Sichtbarkeit und Privatsphäre](https://doc.rust-lang.org/reference/visibility-and-privacy.html),
[mehrere inhärente Implementierungsblöcke](https://doc.rust-lang.org/reference/items/implementations.html).
Die neuen Watch-Helfer sind ein inhärenter `AgentBackend`-Block; die einzige
`BackendExtensions`-Implementierung bleibt bestehen. Die gemeinsame Belegklasse
und ihre Felder sind ausschließlich im bisherigen File-Recovery-Bereich
sichtbar. Dessen neue `JobRun`-Methoden erreichen denselben bisherigen Bereich;
sie erteilen keinen Zugriff außerhalb dieser Modulkette.

Der statische Körpervergleich verwendet die tatsächlichen Rust-AST-Blätter
gegen `10988167`, erhält Operatoren und Literale und löst relative Modulpfade
für den Vergleich auf. Sämtliche bisherigen Funktionskörper bleiben vorhanden.
Zwei gezielte Typbezüge bilden die erklärten Ausnahmen: der Alias-Typ in
`confirmed_move` und die `FolderAliasScope`-Konstruktion in `roots::plan_roots`.
Alle Testfunktionssuffixe und ihre logischen Rustmodulpfade sind gegenüber
dem gebundenen Stand unverändert. Es wurden für die Aufteilung keine neuen
Testfälle erfunden und keine vorhandenen Fälle entfernt.

**Konkret gefundene Typabhängigkeit:** `transfer/mod.rs` bindet `engine`
unmittelbar unter `transfer` ein; dessen `job_store`, `folder_alias_store`,
`folder_reservation` und `roots` sind unmittelbare Kinder von `engine`.
Drei alte Typimporte zeigten mit dreifachem `super` zur Cratewurzel. Ebenso
zeigten der Typ in `confirmed_move` und die Rootkonstruktion dorthin.
Korrigiert sind ausschließlich diese bestätigten Bezüge auf
`crate::transfer::job_record` beziehungsweise `crate::transfer::folder_alias`.
Main hat dafür `folder_alias_store.rs`, `folder_reservation.rs` und `roots.rs`
ausdrücklich nur für diese Typstellen freigegeben. Die unterschiedlichen
Modultiefen werden nicht pauschal ersetzt; `folders_local.rs` liegt unter
`engine::folders::local` und wird hier nicht verändert.

| Erneut selbst verfolgte Kette | Eigener statischer Befund nach der Aufteilung |
|---|---|
| Erstkopie, Live-Missing-Retry und gespeicherte Recovery | `run_view` → gleicher Vorbereitungsaufruf → Jobpreflight/-bindung → gleich geordnete Rootplanung → Quellenbestätigung bleibt erhalten. `missing_retry` sieht dieselben bestätigten Antwort-/Eigentumsflags und dieselbe Journal-ID. |
| Receipt, Index und Publikation | Feldnamen, Serdeattribute, Digest-/Dateischlüssel, Pending-Reparatur, Zähler und Schreibreihenfolge ziehen vollständig in `receipts`; Verifizierung und Journalzugriff bleiben vor jeder Cleanupentscheidung gebunden. Keine Wire- oder Persistenzdarstellung wird geändert. |
| Movequarantäne, Veröffentlichung und Backup | Lokale Beobachter behalten Quellsnapshot, Vorbereitung, Widerruf, Ziel-/Handleprüfung und Callbackreihenfolge. Die gleiche gespeicherte Prüfung entscheidet über Quellen- und Stagebereinigung. Ersetzungs- und Backupprimitive werden nicht geändert. |
| Agent und Share | Watch-Queue, Cancel-Poll und Drop-Join bleiben zusammen; gewöhnliche und reservierte Rename-Frames bleiben getrennt. Share behält Requestautorisierung und Mount-Lease im Dispatch; die Streams halten dieselben Slots und I/O-Werte. |
| Picker und CLI-Orte | Backend-Handle, Connection-Präfix, lokale/Remote-Bedeutung, literaler Pfad, ausgewählter Zweck und Ergebniskonsument bleiben unverändert. CLI-Produktivfunktionen werden nicht verschoben; ihre Testmodule behalten ihre bisherigen logischen Orte. |
| Testorakel | Der Fixtureaufbau behält echte Produktivprüfungen, Locks und Aufrufpunkte. Reader-/Writerfehler und ihre Guardzähler haben dieselben Körper und dieselbe Feldreihenfolge. Alle Testnamen und vollständigen logischen Testpfade bleiben erhalten. |

Eigene statische Kontrolle: Tree-sitter-Rust zeigt keine Syntaxfehler in den
berührten Dateien; `git diff --check` zeigt keinen Whitespacebefund. Die größte
berührte Rustdatei ist `roots.rs` mit 485 Zeilen; unter den zehn aufgeteilten
Verantwortlichkeiten ist `staging.rs` mit 473 Zeilen die größte. Die bereits
remote formatierten Körper werden in ihrer normalen mehrzeiligen Form
übernommen. Neue Deklarationen und Sichtbarkeiten haben ausreichend Abstand
zur Grenze; die tatsächlich erneut durch Rustformatierung bestätigte Größe
bleibt dennoch Mains späterem Remoteeinstieg vorbehalten. Diese Kontrolle
beweist keine Typprüfung oder ausgeführten Testfall.

**Dateiinventar dieses fortgesetzten Strukturzyklus:** Die bisherigen
Dateilisten in H bleiben der Nachweis des vorangegangenen funktionalen Fixes.
Hier wurden die zehn Ziele in D, ihre nachfolgend genannten Extraktionsdateien,
`transfer/mod.rs`, die konkreten Alias-/Root-Typstellen sowie die eigenen
betroffenen Recovery-/Retry-Testverbraucher gezielt als Text oder AST gelesen.
Die ursprünglichen Funktionskörper wurden dabei mit `git show 10988167:<Pfad>`
nachgelesen. Zusätzlich gelesen wurden die aktualisierte Scope-Datei,
`/tmp/smartexplorer-rv1-40332971455/format.json` und
`/tmp/smartexplorer-rv1-size-inventory.json` sowie die oben verlinkten
Rust-Primärquellen. Eine AST-Lektüre ist keine Behauptung einer vollständigen
zeilenweisen Textlektüre sämtlicher unveränderter Verbraucher.

Erstellt:

- `native/src/agent/core/extensions_watch.rs` — 110 Zeilen, 3807 Bytes.
- `native/src/app/core/picker_navigation.rs` — 107 Zeilen, 3575 Bytes.
- `native/src/cli/target_tests.rs` — 108 Zeilen, 3643 Bytes.
- `native/src/copy/os/shared/staging_tests.rs` — 35 Zeilen, 1381 Bytes.
- `native/src/share/core/server_fs_io.rs` — 47 Zeilen, 1454 Bytes.
- `native/src/transfer/os/shared/engine/file_recovery_fixture.rs` — 87 Zeilen, 2843 Bytes.
- `native/src/transfer/os/shared/engine/file_recovery_observer.rs` — 92 Zeilen, 3198 Bytes.
- `native/src/transfer/os/shared/engine/file_recovery_receipts.rs` — 400 Zeilen, 13458 Bytes.
- `native/src/transfer/os/shared/engine/file_recovery_verify.rs` — 322 Zeilen, 12532 Bytes.
- `native/src/transfer/os/shared/engine/job_store_catalog.rs` — 172 Zeilen, 5792 Bytes.
- `native/src/transfer/os/shared/engine/job_store_validation.rs` — 225 Zeilen, 8307 Bytes.
- `native/src/transfer/os/shared/engine/prepare.rs` — 57 Zeilen, 2017 Bytes.
- `native/src/transfer/os/shared/engine/test_backend_streams.rs` — 100 Zeilen, 3007 Bytes.

Geändert:

- `docs/plaene/2026-10-24-batches/fix-regression-transfer.md`
- `native/src/agent/core/extensions.rs` — 432 Zeilen, 15336 Bytes.
- `native/src/app/core/picker_impl.rs` — 461 Zeilen, 22366 Bytes.
- `native/src/cli/target.rs` — 409 Zeilen, 13842 Bytes.
- `native/src/copy/os/shared/staging.rs` — 473 Zeilen, 17329 Bytes.
- `native/src/share/core/server_fs.rs` — 468 Zeilen, 16823 Bytes.
- `native/src/transfer/os/shared/engine/file_recovery.rs` — 260 Zeilen, 8730 Bytes.
- `native/src/transfer/os/shared/engine/file_recovery_tests.rs` — 446 Zeilen, 16872 Bytes.
- `native/src/transfer/os/shared/engine/folder_alias_store.rs` — 143 Zeilen, 5305 Bytes.
- `native/src/transfer/os/shared/engine/folder_reservation.rs` — 422 Zeilen, 17469 Bytes.
- `native/src/transfer/os/shared/engine/job_store.rs` — 351 Zeilen, 12171 Bytes.
- `native/src/transfer/os/shared/engine/mod.rs` — 470 Zeilen, 15202 Bytes.
- `native/src/transfer/os/shared/engine/roots.rs` — 485 Zeilen, 18383 Bytes.
- `native/src/transfer/os/shared/engine/test_backend.rs` — 412 Zeilen, 13397 Bytes.

Keine erforderliche Quellabhängigkeit außerhalb der zuletzt freigegebenen
Fläche bleibt offen. Die gemeinsame Fallregistrierung aus G ändert sich durch
diese Strukturkorrektur nicht. Main übernimmt Integration, Git, Graphrefresh
und die Wiederholung desselben Remote-Suiteeinstiegs. Dieser Autor hat keinen
Formatter, Compiler, Build, Test, Server oder Release ausgeführt und keinen
Commit oder Push erstellt. Nach dieser vollständigen eigenen Nachreview stoppt
dieser Strukturzyklus.

## L — Konkrete Schnittstellenfehler aus Remotelauf 40354605251

**A — Auftrag und Grundlage:** Derselbe Fixautor setzt den Zyklus auf
`d95dc404e834edfe7be0e1c96be162b36865efa9` fort. Maßgeblich sind die vollständigen
Diagnosen in `/tmp/smartexplorer-rv1-40354605251/transfer-compat-errors.json` und
die erneut ersetzte exklusive Scope-Datei. Der Remotelauf ist laut Main beendet.
Beauftragt sind die konkret belegten Schnittstellenfehler mit ihrem Daten-,
Fehler-, Abschluss- und Schutzvertrag; keine breite Bereinigung, entfernte
Prüfung oder umgangene Schutzgrenze. Lokale Nativeausführung bleibt verboten.

**B — fehlerhafte Funktionen und vollständige betroffene Karte:**

| IDs | Fehlerfunktion | Direkte und indirekte Kette und zu erhaltender Vertrag |
|---|---|---|
| C18/C20/C21 | Windows-Adapter exportiert `require_local_publication_observation` weiter, obwohl die private Helper-Sichtbarkeit nur bis zum Adapter reicht. | `copy::publication_proof::confirm_unpublished` prüft Stage- und Zielparent über `copy::platform`; Windows delegiert an `publication_failure`, Linux hat dieselbe Adapterfunktion. Der Fehler ist kein Nichtpublikationsbeweis. Datei-/Parentidentität, lokales Dateisystem, echte NT-Abschlüsse, unvermutete Statuswerte und zurückgehaltene unsichere Handles bleiben geprüft. |
| C40 | `SourceSpool::collect_inner` gibt `io::Error` an `TreeObserver::explain(String)` weiter. | CLI/MCP-Treeplanung → privater verschlüsselter Quellspool → begrenzter `ObservedRead` → `EncryptingWriter` → genauer Längencheck und `finish`/`sync_all` → erneute Quellen-/Baumprüfung → Zielpublikation. `se mv` darf seine Quelle erst nach bestätigter Publikation und erforderlicher erneuter Inhaltsprüfung entfernen. Cancel behält seinen stabilen Fehlertext. |
| A02 | `local_file_identity` referenziert das nicht registrierte VFS-Wurzelmodul `local_optical`. | `vfs/mod.rs` registriert Dateiidentität und lokalen Backend; dessen optische Hilfe liegt tatsächlich unter `local::optical`. Die Funktion muss denselben `source_media::acquire_current(..., Foreground)`-Guard vor dem autorisierten Parent-/Dateizugriff halten. `ConfinedDirectory::selected_entry`, Klassifikation regulärer Dateien und Unsupported→None bleiben erhalten. Keine Android-Sonderausnahme darf den Guard überspringen. |
| C54/A18 | `propfind_properties` verspricht `String`, während der begrenzte Reader einen reservierungsgebundenen `listing_body::Body` liefert. | Metadatentransport → erfolgreicher 207-Status → begrenztes UTF-8-Lesen mit RAM-Reservierung → `folder_reservation::identity/parse_identity` beziehungsweise `capacity::query_inner` → XML-Prüfung. Main hat genau diese zwei Aufrufer repoübergreifend lokalisiert; beide sind vom selben Autor nachgelesen. Depth-1-Listing und Depth-0-Stat verwenden den Body bereits. Resource-ID-/Href-/Quota- und Fehlerprüfungen bleiben erhalten. |
| C57 | Der bestehende GDrive-Scanfall behandelt das neue `ScanMessage::DeniedRoots` nicht. | GDrive-Pagination und leere Kinder → `rscan::start_scan_backend` → Scanereignisse → vorhandener GUI-Taskfall. `DeniedRoots` ist ein verlustfreier, gegebenenfalls gespoolter Fehler-/Retrybeleg mit Speicherwarnung und fehlbarem Lesen. Ein Root oder eine Warnung darf nicht als vollständiger erfolgreicher Scan verschwinden. Der vorhandene Fallname und sämtliche bisherigen Aussagen bleiben bestehen. |

**C — Entscheidung und zweite Lückenprüfung:**
Ein öffentlicher Export der privaten Windows-Hilfe wird verworfen; der Adapter
erhält eine schmale direkte Weitergabefunktion mit dem bisherigen Namen.
Die CLI konvertiert den zurückgegebenen I/O-Fehler vor `explain` explizit in
seinen Text; die eigentliche Kopie und ihre Abschlüsse ändern sich nicht.
Die Android-Dateiidentität ruft denselben vorhandenen optischen Guard direkt
auf; ein bedingtes Weglassen oder eine zweite Modulregistrierung wird verworfen.
WebDAV reicht den vorhandenen `Body` wie `propfind` weiter: eine Stringkopie
würde den Speicherbeleg zu früh abgeben. Beide echten XML-Konsumenten borgen
den Body nur und halten ihn bis nach der Verarbeitung. Der GDrive-Fall behält
DeniedRoots-Handles bis zum Done-Ereignis und prüft anschließend deren vollständige
Roots und Speicherwarnungen. Die konkrete Backendscan-Erzeugerstelle wird vor
Abschluss dieses letzten Punkts zusätzlich im freigegebenen Bereich abgeglichen.

**D — konkreter Erhaltungsplan vor Umsetzung:**

1. Windows: direkten Adapterwrapper einsetzen; beide Parentprüfungen und die
   unveränderte strikte Helperfunktion erneut lesen. Erwartung: derselbe private
   Schutzvertrag ist auf Windows aufrufbar, ohne eine neue öffentliche API.
2. CLI: ausschließlich die fehlerhafte Error→String-Grenze korrigieren.
   Erwartung: I/O-Fehler bleiben Fehler, Cancel bleibt stabil, Verschlüsselung,
   genaue Länge, Writerabschluss und Quellbehalt bleiben unverändert.
3. Android: den vorhandenen optischen Lease-Aufruf an der richtigen
   Definition benutzen. Erwartung: weiterhin echte Autorität und reguläre
   Dateiidentität, keine erfundene ID und kein fehlender Guard.
4. WebDAV: Rückgabetyp der Eigenschaftsabfrage an `Body` angleichen.
   Erwartung: kein Kopieren oder vorzeitiges Abgeben der Speicherreservierung;
   beide XML-Konsumenten behalten ihre Prüfungen und Fehler.
5. GDrive: den bestehenden Scanfall erschöpfend ergänzen; DeniedRoots nach
   Done auf lesbare, warnungsfreie Leerheit prüfen. errors==0 bleibt bestehen;
   verweigerte Zugriffe und Budgetabbruch dürfen keinen Erfolg vortäuschen.
6. Eigene Nachreview: gesamte Karte erneut gegen die Änderungen lesen,
   AST-/Dateigrenzen-/Whitespaceprüfung, vorhandene Fallnamen erhalten und
   konkreten Registrierungsbedarf an Main melden. Keine lokale Ausführung.

Fortschritt: Diagnosen, Funktionskarte, Entscheidungsbegründung und konkreter
Erhaltungsplan wurden vor der Umsetzung gesichert. Umsetzung und eigene
anschließende Anti-Regressionsreview sind abgeschlossen.


**Implementierungsergebnis:**

- Windows behält den privaten Helper unverändert. Der bisherige Adaptername
  ist eine direkte `pub(super)`-Weitergabe mit identischer Signatur und
  identischem Ergebnis; die Helper-Sichtbarkeit wird nicht öffentlich erweitert.
  `confirm_unpublished` verlangt weiterhin beide echten Parentprüfungen.
- Der CLI-Spool konvertiert ausschließlich den zurückgegebenen I/O-Fehler mit
  `to_string()` für `TreeObserver::explain`. `io::copy`, Größenbegrenzung,
  Zähler, verschlüsselter Writer, `finish`, `sync_all` und nachfolgende
  Quellvalidierung bleiben unverändert.
- Die Dateiidentität verwendet direkt denselben gelesenen Aufruf aus
  `local::optical::guard`: `source_media::acquire_current` mit Foreground-Priorität.
  Der `_permit` bleibt bis nach Parent-/Dateihandle- und Klassifikationsprüfung
  im Scope. Weder `cfg`-Weglassen noch eine zweite Moduldefinition wird ergänzt.
- `propfind_properties` gibt den vorhandenen `listing_body::Body` zurück.
  Beide tatsächlichen Aufrufer halten ihn lokal und borgen ihn per `&str` für
  die XML-Verarbeitung. Es gibt keine Stringkopie und keine frühere Freigabe
  der Budgetreservierung. Die offizielle
  [Deref-Dokumentation](https://doc.rust-lang.org/std/ops/trait.Deref.html#deref-coercion),
  geprüft 2026-10-26, belegt diese geliehene Typanpassung; die vorhandene
  `Body`-Implementierung liefert dafür unverändert `&self.text`.
- Der bestehende GDrive-Fall sammelt `DeniedRoots` bis zum Abschluss. Danach
  sind eine Speicherwarnung, ein Lesefehler oder eine nichtleere Rootliste
  jeweils ein Testfehler. Bisherige Fehler-, Ergebnis- und Paginationaussagen
  bleiben bestehen; Permission-Denied-Zähler und Truncation müssen ebenfalls
  leer beziehungsweise false sein. Kein Wildcardarm versteckt weitere Varianten.

**Eigene anschließende Anti-Regressionsreview:**

| Grenze | Selbst nachgelesener und abgeglichener Erhalt |
|---|---|
| Windows-Publikation | `copy/mod.rs` wählt den echten OS-Adapter. Der Wrapper ruft genau den bisherigen privaten NT-/Volumenschutz auf. Die NT-Status-/Completion-/Geräte-/Dateisystemprüfungen und der konservative Umgang mit laufenden I/O-Puffern bleiben identisch; Fehler werden weder ignoriert noch in Erfolg oder sichere Nichtpublikation umgedeutet. |
| Fehler oder Abbruch im Quellspool | `ObservedRead` stoppt Cancel mit `Other`, damit `io::copy` keine Interrupted-Schleife bildet. Die Umwandlung erfolgt erst nach dem zurückgegebenen Kopierfehler und erhält den vorgesehenen Canceltext. Ein Fehler beendet die private Spoolphase vor `apply_transfer`; Writerabschluss und erfolgreiche Quellbestätigung werden nicht vorweggenommen. |
| Move, Backup und Quellbehalt | `tree_ops::copy_entry_with` erstellt den Receipt erst nach Spooling und erfolgreichem Apply. Die nachfolgende Movebereinigung behält ihre bisherigen Ziel-, Quellen- und Inhaltsprüfungen. Die Änderungen greifen weder in Quarantäne- noch Ersetzungs-/Backupoperationen ein. |
| Android-Dateiidentität | Die tatsächliche Modulregistrierung erklärt den Fehler: optische Arbitration ist unter `local::optical`, nicht an der VFS-Wurzel definiert. Der direkte identische Aufruf erhält Priorität, Lease-Lebensdauer und Fehlerweitergabe. Nur reguläre, nicht umleitende Dateien erhalten weiterhin eine echte Handleidentität; Unsupported bleibt unbekannt. |
| WebDAV-Speicher und Fehler | Die Bodyklasse hält ihre Reservierungen durch UTF-8-Decodierung und XML-Parsing. Die unveränderten zwei Konsumenten prüfen weiter Status, eindeutige Resourceantwort, Href-/Collectionbindung, Resource-ID beziehungsweise Quota-Propstats und Zahlen. Allgemeines Listing, Namespaceidentität, gewöhnliches MOVE und striktes REBIND bekommen keine neue Voraussetzung. |
| GDrive-Scanabschluss | Die von Main ergänzten tatsächlichen Quellen `rscan::start_scan_backend` und `walk_state::finish` sind selbst gelesen. Dieser Erzeuger sendet ausstehende Einträge, gegebenenfalls `FailedPaths`, dann `Done`; derzeit sendet er keine `DeniedRoots`. Der gemeinsame Ereignistyp wird trotzdem erschöpfend behandelt. Erhaltene DeniedRoots-Handles werden nach Done verlustfrei geprüft, sodass ein späterer solcher Erzeugerpfad keine stillen Auslassungen erlaubt. Der aktuelle Fall injiziert keine verweigerte Wurzel; eine tatsächlich ausgeführte DeniedRoots-Fehlerreproduktion wird hier nicht behauptet. |

Der endgültige Diff enthält ausschließlich diese vier produktiven
Schnittstellenkorrekturen, die Erweiterung des bestehenden GDrive-Falls und
diesen Bericht. Die fünf betroffenen Rustdateien sind statisch parsebar,
unter 500 Zeilen/50 KiB und ohne Whitespacebefund. Die Funktionsnamen sind
gegen `d95dc404` erhalten; der GDrive-Fall wird nicht umbenannt. Das ersetzt
keine erfolgreiche Rust-Typprüfung und keine Remoteausführung.

**Registrierungsbedarf:** Main ist über den exakten vorhandenen Fall
`gui_design_task_drive_empty_intermediate_pages_continue_and_empty_children_scan`
informiert. Er steht im gelesenen Manifest noch nicht als Fall beziehungsweise
`gui_design_task_`-Präfix; Main hat seine konkrete Registrierung in derselben
abschließenden Remote-Suite vorgemerkt. Es werden keine weiteren Fälle oder
Präfixe durch diesen Autor angelegt. Die bisherigen Transferfälle aus G bleiben
in derselben Suite erhalten; deren Registrierung wird hier nicht geändert.

**Gelesene Dateien dieses ergänzten Zyklus:** Gezielte Textlektüre der
entscheidungsrelevanten Definitionen und Aufrufer, einschließlich der unten
geänderten Dateien; keine behauptete zeilenweise Komplettlektüre aller
unveränderten Dateiteile:

- `native/src/cli/tree_observer.rs`
- `native/src/cli/tree_ops.rs`
- `native/src/cli/tree_spool.rs`
- `native/src/copy/mod.rs`
- `native/src/copy/os/linux_os.rs`
- `native/src/copy/os/shared/publication_proof.rs`
- `native/src/copy/os/windows.rs`
- `native/src/copy/os/windows_publication_failure.rs`
- `native/src/gdrive/core/gui_task_tests.rs`
- `native/src/scanner/mod.rs`
- `native/src/scanner/os/shared.rs`
- `native/src/scanner/os/denied_roots.rs`
- `native/src/rscan/os/shared/rscan.rs`
- `native/src/rscan/os/shared/walk_state.rs`
- `native/src/vfs/mod.rs`
- `native/src/vfs/os/shared/local.rs`
- `native/src/vfs/os/shared/local_file_identity.rs`
- `native/src/vfs/os/shared/local_optical.rs`
- `native/src/webdav/mod.rs`
- `native/src/webdav/core/webdav.rs`
- `native/src/webdav/core/listing_body.rs`
- `native/src/webdav/core/capacity.rs`
- `native/src/webdav/core/folder_reservation.rs`
- `native/review-task-manifest.json`
- `docs/plaene/2026-10-24-batches/fix-regression-transfer.md`

Zusätzlich gelesen: die vollständige bereitgestellte Diagnosedatei
`/tmp/smartexplorer-rv1-40354605251/transfer-compat-errors.json`, die aktualisierte
`/tmp/smartexplorer-fix-transfer-compat-scope.json` und die oben verlinkte
Rust-Primärquelle. Der wörtliche Nutzervertrag und die zuvor geladenen
Projektgrenzen gelten in diesem fortgesetzten Autorzyklus weiter.

**Geänderte Dateien:**

- `docs/plaene/2026-10-24-batches/fix-regression-transfer.md`
- `native/src/cli/tree_spool.rs` — 227 Zeilen, 8332 Bytes.
- `native/src/copy/os/windows.rs` — 467 Zeilen, 17063 Bytes.
- `native/src/gdrive/core/gui_task_tests.rs` — 488 Zeilen, 17095 Bytes.
- `native/src/vfs/os/shared/local_file_identity.rs` — 32 Zeilen, 1213 Bytes.
- `native/src/webdav/core/webdav.rs` — 470 Zeilen, 16834 Bytes.

Keine neue Datei wurde angelegt. Alle erforderlichen Definitionen und
Verbraucher sind im zuletzt freigegebenen Bereich geklärt; es bleibt kein
ungelöster Quell-Scopebedarf. Offen sind ausschließlich Integration und
Bestätigung im selben Remote-Suiteeinstieg durch Main. Dieser Autor hat keinen
Build, Compiler, Test, Formatter, Server, Paketinstaller, Gitmutation,
Graphrefresh oder Unteragenten ausgeführt. Nach dieser eigenen abschließenden
Review stoppt dieser ergänzte Fixzyklus.
