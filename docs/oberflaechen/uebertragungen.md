# Übertragungsentscheidungen und gezieltes Lesen

Quellstand: 2026-10-26, laufender B2-Kandidat; ergänzt 2026-10-31 um die Dateiregel
„Neuere ersetzen ältere“ (Paket D4 des Plans `docs/plaene/2026-10-31-sync-abgleich-schnell/`,
nicht gebaut, nicht getestet). Gemeinsame Remote-Suite und
Veröffentlichung stehen aus. Abkürzungen: `N/` = `native/src/`, `A/` =
`android/app/src/main/java/app/smartexplorer/android/`.

| Fähigkeit | Desktop und se Desktop | Android und Bridge | CLI und MCP | Gemeinsamer Vertrag |
|---|---|---|---|---|
| Obersten ausgewählten Ordnerkonflikt entscheiden | `N/app/core/folder_conflict_ui.rs:118` und `transfer_route.rs`; Vorschau mit einzelnen Entscheidungen und Anwendung auf verbleibende Ordner | `A/ui/files/FileDialogs.kt:224`, `FileActions.kt:211`; `FilesApi.transfer` übergibt `folderConflict` und `folderConflicts` (`A/api/FilesApi.kt:265`) | `--folder-conflict` bei Kopieren/Verschieben (`N/cli/mod.rs`); MCP-Transferadapter verwendet dieselbe Policy | `transfer/core/folder_alias.rs` und `engine/folder_alias_store.rs`: exklusiv reservierter Alias bleibt durch Kinder, Fortschritt und Wiederanlauf stabil. Ordner- und Dateikonflikte bleiben getrennt. |
| Dateiregel „Neuere ersetzen ältere“ (fehlende einfügen, neuere ersetzen ältere nach Änderungszeit, gleiche überspringen, neueres Ziel behalten, gleiche Zeit und andere Größe: beide behalten) | Einfügen/Ziehen/„Herunterladen nach…“ setzen sie für die Dateien zusammengeführter Ordner als Vorgabe (`N/app/core/transfer_route.rs:127`, Hinweis im Dialog `folder_conflict_ui.rs:157`); Kopierdialog: Wahl „neuere ersetzen ältere“ (`copy_dialog.rs:68`); Vorschau „Zielordner entscheiden“ zählt neu, neuer, gleich, Ziel neuer, Konflikt nur, wenn der Auftrag die Regel anwendet (`merge_update` oder `Update`; sonst „neue und gleichnamige Dateien“, `folder_conflict_ui.rs` `count_text`); Übertragungsfenster nennt ersetzt, gleich übersprungen, Ziel neuer behalten, Konflikt doppelt behalten (`transfer_rows.rs:159`, `:211`) | Dialog „Name vorhanden“/„Zielordner“: Hinweis „Dateien in zusammengeführten Ordnern: neuere ersetzen ältere“ und die fünf Zählwerte nur bei `rule` der Vorschau, sonst „neue und gleichnamige Dateien“ (`A/ui/files/FileDialogs.kt`), Wahl „Neuere ersetzen ältere“ für Namen oberster Ebene bei jeder Seitenkombination, „Ersetzen“ und „Überspringen“ nur lokal nach lokal; `FileActions.kt:238` sendet `mergeFiles: "update"`, die Schlussmeldung nennt die Entscheidungen (`:245`); Bridge: `fs.transfer` `conflict: "update"` für jede Seitenkombination und `mergeFiles` (`N/mobile/os/shared/transfer.rs:89`, `:111`), Vorschau `newerFiles`, `equalFiles`, `targetNewerFiles`, `conflictFiles` (`fs_edit.rs:149`), Ergebnis `replaced`, `skippedEqual`, `keptNewer`, `keptBoth` (`drive.rs:45`) | `se cp|get|put|mv --conflict update` (nicht mit `--force`, `--resume-job` oder einer anderen `--folder-conflict` als merge; `mv` nur lokal; `N/cli/mod.rs:228`, `cli/update_copy.rs:49`); Zeile `se: update: …` auf stderr; Shell-Vervollständigung aus der Option; MCP `copy`/`move` mit `file_conflict: "update"` und Ergebnisfeld `update` (`cli/mcp/catalog.rs:228`, `transfer_tools.rs:37`) | `Conflict::Update` (`N/types/core/types.rs:133`), reine Regel `transfer/core/update_rule.rs`, Entscheidung je Datei vor jedem Kopierweg (`engine/update.rs:166`); `FolderConflictPolicy::merge_update`; gespeicherte Aufträge: `JobRecord::file_update` bei `file_policy` 0, mit `merge_update` `file_policy` 0 und die Wahl der obersten Ebene in `file_choice` (ältere Versionen behalten beide, setzen nie als Überschreiben fort); Worker-Dienst: Code 3 und Fähigkeit `transfer-update-v1`, sonst läuft der Auftrag im eigenen Prozess (`daemon/os/shared/transfer_job_wire.rs:26`); Beschreibung und Grenzen: `docs/TRANSFER_ENGINE.md`, Abschnitt „Dateiregel“. |
| Gespeicherte Übertragung wieder aufnehmen | Übertragungsfenster mit Quellen-/Zielbindung (`N/app/core/transfer_recovery_ui.rs:12`), einschließlich gespeicherter CLI-Jobs (`:69`) | Mobile Jobs verwenden denselben recoveryfähigen Enginevertrag (`N/mobile/os/shared/transfer.rs:189`); dieser Bestand belegt keinen zusätzlichen allgemeinen mobilen Recoverykatalog | `--resume-job` bindet Quelle, Ziel und Entscheidungen an die gespeicherte Job-ID (`N/cli/folder_conflict.rs:127`, `transfer_recovery.rs:33`); MCP erreicht den CLI-Vertrag über `se` | Bestätigte Publikation, Inkarnation und Cleanup bleiben getrennte Belege; unsicherer Ausgang wird nicht als erfolgreicher Dateiaustausch fortgeschrieben. |
| Physisches optisches Medium lesen | Kopieren, Browser, Vorschau, Suche und Index verwenden die gemeinsame Quelle | Kein eigener Disc-Hostadapter für Android; normale Remoteübertragung bewahrt die Quellidentität | Bestehende Dateioperationen, keine neue Rawdevice-Schnittstelle | `N/source_media/` und Copy-/VFS-Adapter serialisieren Leser desselben tatsächlich identifizierten optischen Geräts; unabhängige Geräte bleiben unabhängig. |
| Nach Rechteerteilung gesperrte Teilbäume neu lesen | Windows-Zustimmungsweg startet begrenzte Teilscans; Explorer und Analyse übernehmen nur Ergebnisse ihrer aktuellen Generation (`N/app/core/partial_scan.rs:26`) | Mobile Scans melden Fehler/Unvollständigkeit (`N/mobile/os/shared/scan.rs:138`); die Windows-Zustimmungsaktion wird dort nicht angeboten | Kein neuer Windows-Zustimmungsdialog im Terminal; ausdrücklicher normaler Scan bleibt vorhanden | `scanner/os/denied_roots.rs`, `scanner/os/retry.rs`, `analytics/core/partial_merge.rs`: unberührte Einträge und geschützte Auslassungen bleiben erhalten. |

Desktop und Android bieten für ausgewählte Ordner „Beide behalten“ als Vorgabe,
„Zusammenführen“ und „Überspringen“. Wählt die Person „Zusammenführen“, gilt für die
Dateien in diesem Ordner die Dateiregel „Neuere ersetzen ältere“; gleichnamige Dateien auf
oberster Ebene der Einfügung behalten ihre bisherige Regel (beide behalten, im Kopierdialog
die gewählte). Unterschied nach Ziel: lokale Ziele ersetzen über die bisherige Überschreib-
Veröffentlichung; entfernte Ziele ersetzen über eine dauerhafte Stufe und
`promote_staged_to_id`, und wo ein Ziel das nicht kann, bleiben beide Dateien erhalten.
Damit die Regel nach einer Kopie stimmt, trägt jede Datei, die die Engine kopiert oder ersetzt, die
Änderungszeit ihrer Quelle (lokal Linux/Android auf der Stufe, Download auf der Teildatei; entfernte
Ziele im Schreibschritt der Stufe (WebDAV, Drive, SFTP, SMB), im Paket oder sonst über `finish_stage`;
Windows lokal wie bisher über `CopyFile2`). Pakete tragen die Zeit je Datei (`BatchPut::mtime_ms`,
Fähigkeit `batch_put_times`: Share `batch_put_mtime_v1`, Agent `batch-mtime-v1`, SFTP und SMB immer);
nur Gegenstellen ohne die Fähigkeit (ältere Share-Hosts und Agenten) bekommen Dateien der Regel einzeln,
und andere Pakete behalten dort die Zeit der Gegenstelle. Wo nur eine eigene Anfrage die Zeit setzen
kann, steht das als Grenze in `docs/TRANSFER_ENGINE.md`. Vor jedem Ersetzen wird das Ziel noch
einmal mit dem Stand der Entscheidung verglichen (Größe, Zeit); weicht es ab, bleiben beide Dateien.
Die CLI bewahrt dagegen den ausdrücklich
getippten Zielordner als Standard. Ein Backend ohne belegte exklusive
Ordnerreservierung meldet die Einschränkung; ein unsicherer Keep-both-Ausgang
wird nicht als erfolgreiche Reservierung ausgegeben.

Die Engine nutzt dieselben Backend-/Verbindungsidentitäten über VFS, Agent,
Share und Daemon. Ein gleichlautender relativer Pfad auf zwei Gegenstellen
bezeichnet weiterhin zwei verschiedene Orte. Sync und „Fehlende übertragen“
erhalten ihre bestehenden Zielnamen. Abbruch, Lesefehler und geschützte Links
werden nicht zu Lösch- oder Vollständigkeitsbelegen umgedeutet.

„Fehlende übertragen“ bleibt nach Kopien und lokalen Moves verfügbar,
auch nach einem frühen Fehler ohne Ergebnisroots. Die Wiederholung behält
Verbindungen, Auswahl, Zielnamen und die Journal-ID. Bereits vorhandene
Dateien gleicher Größe bleiben unangetastet; abweichende Dateien werden
gemeldet und nicht ersetzt. Eine bestätigte aktuelle Übertragungsantwort
verlangt dafür keine neue persistente Providerordner-ID. Unklare Anlage- und
Publikationsantworten bleiben im selben Journal geschützt, auch bei mehreren
Klicks. Movequellen und Quarantäne werden nur mit ihren erforderlichen
Belegen ausgelassen oder bereinigt. Die ausdrücklich gewählte Wiederherstellung
eines gespeicherten Jobs behält die strenge Objekt-, Quellen-, Ziel-, Rechte-
und Journalprüfung.

FTP/FTPS und SFTP ohne se-Agent können ausgewählte Ordner weiterhin über die
exklusive MKD/MKDIR-Anlage kopieren. Eine fehlende stabile Objektidentität
bleibt unbekannt; nach einer verlorenen Anlageantwort wird kein fremder oder
unbestätigter Zielordner übernommen. Gewöhnliches WebDAV-Ordnerumbenennen nutzt
weiterhin `MOVE` mit `Overwrite: F`. Die strikte Reservierungsveröffentlichung
verlangt gesondert atomare Providerunterstützung; eine ausdrücklich ohne Mutation
abgelehnte REBIND-Veröffentlichung kann im bestätigten Erstlauf auf die exklusive
Anlage im endgültigen Namen wechseln. Ein unklarer Ausgang erlaubt diesen
Fallback nicht.

Ein Uploadkonto ohne Inhaltsleserecht kann seine erlaubten Uploads abschließen.
Fehlende Rücklese- oder Inkarnationsbelege werden dabei nicht als bestätigte
Wiederherstellbarkeit ausgegeben. Stagetyp, bekannte übertragene Größe,
Quellprüfung, Providerabschluss und Konfliktschutz bleiben erforderlich.
Gleichvolumiges Verschieben einer regulären lokalen Datei verlangt keinen
Hardlink und keine vollständige zusätzliche Inhaltskopie, sofern der gehaltene
Quellhandle den direkten Move erlaubt. Ohne Hardlink bleibt ein unbestätigter
bereits umbenannter Inhalt am Ziel erhalten; er wird nicht zur Fehlerbereinigung
gelöscht. Dauerhaftigkeit und persistentes Eigentum bleiben getrennte Belege.

Diese Kompatibilitätskorrekturen sind im Quellkandidaten enthalten; ihre
Remote-Laufzeitprüfung und Veröffentlichung stehen aus.

Statische Befunde: [Review B2](../plaene/2026-10-24-batches/review-b2.md).
Offene Produktarbeit: [TODO](../TODO.md).
