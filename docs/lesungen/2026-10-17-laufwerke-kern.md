# Lesung: Laufwerke, Einhängen und lokale Ablage (Kern)

Stand 2026-10-17, Lesestand da2646d2

Strang S2 „Laufwerke und Einhängen je Betriebssystem“ aus `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md` (Abschnitt A). Grundlage für E10, E4 und E8.

Diese Lesung enthält Befunde mit Belegen. Sie bewertet, empfiehlt und entscheidet nichts. Der Lesestand ist der Commit `da2646d2`. `git status` zeigte zum Lesezeitpunkt keine Änderung an getrackten Dateien, nur unverfolgte Dateien. Alle Pfade sind relativ zum Repository-Wurzelverzeichnis. `Pfad:Zeile` bezeichnet eine Zeile im Lesestand. In `docs/oberflaechen/einstellungen.md` steht `N/` für `native/src/`; hier sind die Pfade ausgeschrieben.

## 1. Ziel und Fragen

Ziel: Den Bestand des Laufwerks- und Einhängesystems belegen. Erfasst werden der Windows-Host mit Dokany, der plattformneutrale Kern unter `native/src/mount/core/`, CLI, Desktop-Oberfläche und die Android-Berührungspunkte.

- F1: Welche Teile von `mount/core` sind plattformneutral und welche Windows-spezifisch? Welche Schnittstelle trennt das Dateisystem-Frontend (Dokany-Callbacks) von Engine, Spool und Metadaten? Typen, Traits und Funktionen mit Signatur.
- F2: MountSource-Arten, Persistenz der Laufwerksdefinitionen (Datei, Format, Felder), Wiederherstellung beim Start, Laufwerksbuchstaben und Ordnerpfade.
- F3: Lokale Ablage (Spool, Cache, Stages, Journale, Recovery): Ort, Format, Löschzeitpunkt; Ablauf des Speicherns (`file_commit.rs`, `file_commit_stage.rs`, `stage_ledger.rs`).
- F4: Definitionsort von `drive_mount_supported`, Ergebnis je Plattform, fehlende Codepfade für Linux und Android.
- F5: Alle Oberflächen der Laufwerke: CLI `se drive` mit Optionen und JSON, Desktop-Dialoge, Android, IPC, MCP.
- F6: Bekannte Zuverlässigkeitsprobleme und offene Punkte aus den gelesenen Dokumenten und `docs/TODO.md`, wörtlich zitiert.
- F7: Gibt es in der Android-App einen DocumentsProvider oder einen ContentProvider neben dem FileProvider?

## 2. Befunde je Frage

### F1: Plattformneutral und Windows-spezifisch

Kern ohne OS-Code. In den 51 Nicht-Test-Dateien von `native/src/mount/core/` (vollständige Liste in Abschnitt 7) gibt es kein `#[cfg(windows)]`, kein `#[cfg(target_os …)]`, kein `std::os::…` und keinen Import eines Windows-Crates. Die Suche `cfg\(|std::os|windows|target_os|CommandExt` ergab in diesen Dateien nur `#[cfg(test)]`-Attribute, Kommentare und Funktionsnamen mit `windows`.

Windows-Regeln im Kern als Funktionen ohne OS-Aufruf. Nur `windows_access.rs` nennt in seiner Modul-Doku die Testbarkeit auf allen Plattformen (`:12-13`):
- `native/src/mount/core/windows_access.rs:1-13` (Doku: „The helpers here are pure so the policy is testable on every platform“). Funktionen: `requests_maximum_allowed(desired_access: u32) -> bool` (`:28`), `maximum_allowed_read_grant(desired_access: u32) -> u32` (`:34`), `maximum_allowed_full_grant(desired_access: u32) -> u32` (`:46`).
- `native/src/mount/core/windows_case.rs:12` `windows_ordinal_key(value: &str) -> String`; `:32` `validate_windows_case_component(_component: &str) -> io::Result<()>`.
- `native/src/mount/core/path.rs:90` `validate_windows_component(component: &str) -> io::Result<()>`; `:113` `is_dos_device_name` (privat).
- `native/src/mount/core/peer_names.rs:1-2` (Windows-Sicht auf den Share-Namensraum); `:16` `project_peer_listing(mut entries: Vec<VfsMeta>) -> io::Result<Vec<VfsMeta>>`.

Windows-Bezüge im Kern nur als Kommentar oder Fehlertext, ohne Verzweigung: `native/src/mount/core/file_io.rs:112` („Windows callers treat a short count as end-of-file“), `metadata.rs:27` („Metadata-only lookup for Dokany query callbacks“), `metadata_loading.rs:24` („after Dokany has made the drive available“), `types.rs:474-475` („Windows must not replay it; the host stops and remounts after surfacing the detail.“).

Windows-Bezug in `native/src/mount/mod.rs` ohne `cfg`-Gate (nicht im Kern; ob das Modul `mount` auf allen Zielen eingebunden ist, ist nicht geprüft): Dokany-Konstanten `DOKANY_LIBRARY_API_VERSION: u32 = 231` (`:149`) und `DOKANY_DRIVER_PROTOCOL_VERSION: u32 = 0x0000_0190` (`:150`); MSI-Exitcode-Zuordnung `DriveRuntimeInstallOutcome::from_msi_exit_code` (`:211-222`); deutsche Meldungen mit „Dokany 2.3.1“ (`:239-263`).

Windows-spezifisch außerhalb des Kerns (Dateien nicht gelesen):
- `native/src/mount/os/mod.rs:1-2`: `#[cfg(windows)] pub(crate) mod windows;` Das ist das einzige OS-Modul. Das Verzeichnis `native/src/mount/os/windows/` existiert; sein Inhalt ist nicht gelesen.
- Aufrufe aus `native/src/mount/mod.rs`: `os::windows::preflight_runtime` (`:286`), `os::windows::install_runtime` (`:304`), `os::windows::run_mount_host` (`:324`).

Grenze Kern zu Backend-Schicht: Der Kern importiert `crate::vfs` (außerhalb des Lesebereichs). Belegt sind `crate::vfs::BackendHandle`, `crate::vfs::VfsMeta`, `crate::vfs::Backend`, `crate::vfs::unique_staging_path`, `crate::vfs::remote_util` und `crate::vfs::DeleteDisposition` (`native/src/mount/core/delete.rs:4`).

Grenze Dateisystem-Frontend zu Engine: Die Dokany-Callbacks liegen in `native/src/mount/os/windows/`, das nicht gelesen ist. Welcher Callback welche `MountEngine`-Methode aufruft, ist deshalb nicht belegt (Lücke L1). Belegt ist die öffentliche Engine-Schnittstelle, vollständig in Abschnitt 3 aufgelistet. Die Oberflächen-Dateien greifen nicht direkt auf `MountEngine` zu. `native/src/app/core/mount_*.rs` und `native/src/cli/drive.rs` nennen den Typ `MountEngine` nicht. Sie verwenden Typen aus `crate::mount` und Funktionen aus `crate::daemon` (Abschnitt 3, Ebene A).

### F2: Quellarten, Persistenz, Start, Buchstaben, Pfade

MountSource (`native/src/mount/core/types.rs:134-148`) hat genau drei Varianten:
- `SavedRemote { account: String, root: BackendRoot }`: gespeicherte Verbindung.
- `GoogleDrive { account: String, root: BackendRoot }`: Google Drive. `validate` verlangt `account == "cloud:gdrive"` (`types.rs:164-166`).
- `Peer { target: PeerMountTarget, root: BackendRoot }`: Share-Peer.

PeerMountTarget (`types.rs:116-120`): `Direct { contact_id: String }` und `RoomDevice { room_id: String, device_id: String }`. Kennungen dürfen höchstens 256 Zeichen lang sein (`types.rs:8`) und keine NUL- oder Steuerzeichen enthalten (`types.rs:485-495`).

Erzeugung: CLI `native/src/cli/drive.rs:264-271` (Peer), `:274-281` (GoogleDrive aus `gdrive://`), `:322-327` (SavedRemote). Desktop `native/src/app/core/mount_ui_draft.rs:34-40` (SavedRemote), `:48-54` (GoogleDrive mit Wurzel `/`), `:104-124` (Peer aus der Discovery).

Laufwerksdefinition `MountConfig` (`types.rs:237-252`), Felder in dieser Reihenfolge:
- `id: MountId`
- `source: MountSource`
- `drive: DriveSelection`
- `mode: MountMode`
- `root_security: MountRootSecurity` (`#[serde(default)]`, Vorgabe `Enforced`, `types.rs:229-235`)
- `metadata: MountMetadataPolicy` (`#[serde(default)]`)
- `cache: MountCachePolicy` (`#[serde(default)]`)
- `runtime_preference: MountRuntimePreference` (`#[serde(default)]`)
- `label: String`

Die Typen sind mit `Serialize, Deserialize` abgeleitet (`types.rs:237`). `MountConfig::validate` (`types.rs:277-290`) prüft Quelle, Metadaten-Policy, Cache-Policy und die Bezeichnung (höchstens 128 Zeichen, keine NUL- und Steuerzeichen, `:281-288`).

Validierte Kennungen und Pfade:
- MountId (`types.rs:27-37`): 1 bis 64 ASCII-Zeichen aus Buchstaben, Ziffern, `-` und `_`. Neu erzeugt werden 32 Hex-Zeichen aus 16 Zufallsbytes (`types.rs:15-25`).
- BackendRoot (`types.rs:66-99`): absoluter Pfad mit Vorwärtsschrägstrichen, höchstens 32 767 UTF-16-Einheiten, ohne `\`, NUL, `.`, `..` und leere Komponenten. Eine UNC-Wurzel braucht Server und Freigabe (`:94-96`).

Persistenz: Ort, Dateiname und Format der gespeicherten Laufwerksliste sind in den gelesenen Dateien nicht belegt (Lücke L2). Der Kern liefert nur die serde-Ableitung der Typen. Ein Graph-Einstieg, nicht am Quelltext geprüft: `native/src/daemon/os/shared/mount_manager.rs` (Knoten `MountEntry`, Zeile 35).

Wiederherstellung beim Start (aus dem Kern belegt):
1. `MountRecovery` hat die Werte `Clean`, `Required` und `Unknown`; Vorgabe ist `Unknown` (`native/src/mount/core/recovery_state.rs:8-14`). Der Kommentar sagt, `Unknown` bleibt, bis der isolierte Host das Journal „under its exclusive cache lease“ prüft (`:5-7`).
2. Host ohne Remote-Zugriff: `MountEngine::open_host_cache` prüft nur den lokalen Cache (`native/src/mount/core/startup.rs:10-24`).
3. `prepare_host_remote` (`startup.rs:26-31`): prüft die Wurzel als einfaches Verzeichnis (`:34-46`), ruft `recover_pending_deletes` auf (`native/src/mount/core/delete.rs:303`) und `collect_orphan_stages` (`native/src/mount/core/file_commit_stage.rs:24-31`).
4. `open_at_root` (`native/src/mount/core/engine_recovery.rs:14-21`): öffnet den Spool (`:30`), übernimmt die Einträge (`:35-60`), die Löschtransaktionen (`:76-96`) und die Namenskonflikte (`:102-119`).
5. `retry_pending_changes` (`native/src/mount/core/recovery.rs:9-30`): wiederholt zuerst die Löschtransaktionen und Namensintents unter der Schreibsperre des Namensraums (`:11-13`), danach die Flushes der Einträge im Zustand `Dirty` (`:16-28`).

Wer die Schritte 4 und 5 beim Start auslöst, ist in den gelesenen Dateien nicht belegt (Lücke L9).

Laufwerksbuchstaben:
- `DriveSelection` ist `Automatic` oder `Letter(DriveLetter)` (`types.rs:214-218`).
- `DriveLetter::parse` akzeptiert jeden ASCII-Buchstaben und speichert ihn in Großschreibung (`types.rs:176-189`). Anzeige: `X:` (`:191-195`).
- CLI: `auto` oder genau ein ASCII-Buchstabe (`native/src/cli/drive.rs:228-244`, Fehlertext `:239`).
- Desktop: nur `Automatisch` und `D` bis `Z` (`native/src/app/core/mount_ui_draft.rs:309-331`, Bereich `:321`).
- Unterschied: Auf Parse-Ebene akzeptiert die CLI `A` bis `C`, die Desktop-Auswahl nicht. Ob der Daemon `A` bis `C` annimmt, ist nicht gelesen (Lücke L2).
- Welcher freie Buchstabe bei `Automatisch` gewählt wird, bestimmt der nicht gelesene Windows-Host (Lücke L11).
- Status mit Buchstabe: `MountStatus::Mounted { drive: DriveLetter }` (`types.rs:365-371`).

Ordnerpfade:
- `BackendRoot` ist die Wurzel innerhalb eines bereits autorisierten Backends (`types.rs:60-63`).
- Eine Einhängung in einen Ordner (Mountpoint, Junction oder Reparsepunkt) ist in den gelesenen Dateien nicht vorhanden. Die Suche `mount point|mountpoint|folder mount|reparse|junction` in den Mount-, App- und CLI-Dateien ergab keine Einhängungslogik; der einzige Treffer im Kern ist eine Fehlermeldung in `metadata_point_cache.rs:203`.
- Annahme: Die Einhängeform ist ausschließlich der Laufwerksbuchstabe (`DriveSelection`).

### F3: Lokale Ablage und Speichern

Spool-Wurzel: `prepare_spool_root(path)` (`native/src/mount/core/spool.rs:29-48`) verlangt einen absoluten Pfad, prüft die Vorfahren auf Links (`:50-77`) und kanonisiert den Pfad.

Ablageort je Laufwerk (`spool.rs:80-86`), Pfad `<spool-root>/<mount-id>/`:
- `files/` mit Dateien `<32 Hex>.spool`. Erzeugung in `spool.rs:127-146`, Namensprüfung in `:307-310` (38 Zeichen).
- `journal.jsonl` (`spool.rs:87`).
- `stages.json` (`native/src/mount/core/stage_ledger.rs:15`; Öffnung über `StageLedger::open(&root)` in `spool.rs:100`).

Journal (`native/src/mount/core/journal.rs`):
- Format: JSON-Zeilen mit `Record { version: u8, action: Action }` (`:56-60`). `JOURNAL_VERSION = 1` (`:8`).
- Aktionen (`:62-80`): `UpsertEntry`, `MoveEntry`, `ForgetEntry`, `UpsertDelete`, `UpsertNamespaceConflict`, `ForgetNamespaceConflict`, `ForgetDelete`.
- Verdichtung beim Öffnen, wenn die Datei mindestens 1 MiB groß ist (`:9`, `:103-111`). Konstanten `MAX_JOURNAL_BYTES` = 64 MiB (`:10`) und `MAX_RECORD_BYTES` = 1 MiB (`:11`); ihre Verwendung ist nicht gelesen.
- Ein unvollständiges Ende wird beim Öffnen abgeschnitten (`TornTail::Truncate`, `:13-17`, `:93`).
- Rotationsdateien: `journal.jsonl.compact-old` und `journal.jsonl.compact-new` (`native/src/mount/core/spool_recovery.rs:7`).
- Eintrag `PersistedEntry` (`journal.rs:19-27`): `remote_path`, `spool_name`, `baseline`, `condition`, `delete_token`.
- Löschung `PersistedDelete` (`:39-47`): `token`, `original_path`, `quarantine_path`, `id`, `is_directory`, `phase`.
- `DeletePhase` (`:29-37`): `LocalOnly`, `Unresolved`, `Prepared` (Altwert, wie `Unresolved` behandelt, `:34-35`), `Moved`.

Stage-Ledger (`native/src/mount/core/stage_ledger.rs`):
- Format: JSON mit `Recorded { stages: BTreeMap<String, u64> }` (`:17-21`). Schlüssel ist der Stage-Pfad, Wert die größte Größe, die der Upload bekommen hat.
- Schreiben über `stages.json.new` und anschließendes Umbenennen (`:109-117`).
- Der Kommentar lautet: „Losing the ledger loses only this cleanup, never user data: the spool keeps every unsaved change.“ (`:6-7`).

Stage-Namensmuster: `<name>.se-mount-<16 hex>` (`file_commit_stage.rs:1`, `stage_ledger.rs:1`). Die Erzeugung leistet `crate::vfs::unique_staging_path` (außerhalb des Lesebereichs).

Löschzeitpunkte:
- Unreferenzierte Spool-Dateien werden beim Öffnen entfernt (`spool.rs:119`, `remove_orphan_clean_files` in `:225-244`) und im `Drop` (`:262-269`). Der `Drop` läuft nur nach erfolgreicher Prüfung. Kommentar `:117`: „A failed audit must never authorize cleanup from Drop.“
- Nach erfolgreichem Commit: `ForgetEntry` im Journal (`file_commit.rs:178-190`), danach Zustand `Clean` (`:191-192`) und `schedule_retirement` (`:194`).
- Stage bei Erfolg: `resolve` (`file_commit.rs:104-106`). Stage bei Fehler vor jeder Wirkung: `discard_own_stage` (`:110-116`), nur wenn es eine reguläre Datei mit einer Größe höchstens gleich der gemerkten Größe ist (`file_commit_stage.rs:35-51`, Prüfung in `:38`).
- Stage-Waisen werden beim nächsten Speichern (`file_commit.rs:35`, `file_commit_stage.rs:24-31`) und beim Mount (`startup.rs:29`) entfernt.
- Saubere, nicht geöffnete Inhalte (`CleanCache`, `clean_cache.rs:112`): `MAX_IDLE_RECORDS = 10_000` (`:10`), `MAX_CONTENT_AGE = 300 s` (`:11`). Entfernt wird über `evict_path` (`:155`), `evict_oldest` (`:166`), `trim(limit)` (`:170`) und `invalidate` (`:141`). Welche Stelle `trim` mit welchem Limit aufruft, ist nicht gelesen (Lücke L6).
- Budget je Laufwerk (`cache_policy.rs`): Vorgabe 500 MiB (`:6`), Obergrenze 65 536 MiB (`:7`). Offene und ungespeicherte Daten sind ausgenommen (`:9-11`). Null bedeutet keine Aufbewahrung nach dem Schließen (`:25`). Umrechnung `retained_bytes` in `:46-48`.
- Platzreserve: `CACHE_RESERVE_BYTES = 512 MiB` (`cache_space.rs:9`). `reserve_growth` (`:33`) und `maintain_space` (`:60`); die Aufrufer sind nicht gelesen.
- Metadaten (im Speicher): `MAX_CACHED_ENTRIES = 50_000` (`metadata_cache.rs:47`), `MAX_CACHED_BYTES = 128 MiB` (`:48`), `DIRECTORY_TTL = 20 s` (`:51`), `SNAPSHOT_RETRY_DELAY = 5 min` (`:50`). Punktcache: `MAX_POINT_BYTES = 16 MiB`, `POINT_STAT_TTL = 5 s`, `POINT_MISSING_TTL = 1 s` (`metadata_point_cache.rs:14-16`). Der Dialogtext nennt die Begrenzung: „Metadaten werden begrenzt im Speicher gehalten“ (`mount_ui_draft.rs:349`).

Ablauf des Speicherns (`native/src/mount/core/file_commit.rs:9-196`, Funktion `flush_entry`):
1. Zustand prüfen. Gelöschter Handle: nur Sync des Spools (`:11-17`). Löschmarke: nur Sync (`:18-21`). `Clean` ergibt `NoChanges` (`:23`), `Conflict` wird zurückgegeben (`:24-26`).
2. Konfliktprüfung gegen die Baseline (`detect_conflict`, `:29`; Definition `:198-221`). Bei Konflikt wird der Zustand im Journal gespeichert (`:30-31`).
3. Stage-Name über `unique_staging_path(…, "mount")` (`:36`). Caches werden invalidiert (`:37-39`).
4. Spool-Inhalt synchronisieren und Länge messen (`:40-42`).
5. Stage vor dem exklusiven Anlegen im Ledger erfassen (`:43-45`). Dann `open_write_new` (`:49`). Bei `AlreadyExists` wird nur der Ledger-Eintrag gelöst; das fremde Objekt bleibt unberührt (`:46-48`, `:52-54`).
6. Hochladen per `io::copy`, dann Flush und Schließen (`:58-63`). Bei Fehler wird die eigene Stage entfernt; der Spool behält den Inhalt (`:64-69`).
7. Zweite Konfliktprüfung unmittelbar vor der Umbenennung (`:70-73`). Bei Konflikt entsteht eine Konfliktkopie über `publish_conflict_copy` (`:78`; `file_commit_stage.rs:57-73`, Namensschema `<name> (Konflikt <Zeit>)[ (n)].<ext>`, Doku `:53-56`), und der Zustand wird `Conflict` (`:84-87`).
8. Umbenennung: Baseline `Missing` führt zu `promote_staged_no_replace` (`:98-100`), sonst `promote_staged` (`:101`).
9. Fehlerpfad (`:107-155`):
   - Sind Ziel und Stage unverändert, gibt es einen Fehler, und der Spool behält den Inhalt (`:110-116`).
   - Bei `Missing` und `AlreadyExists` mit einer Stage als Datei entsteht eine Konfliktkopie (`:117-140`).
   - Sonst `CommittedPendingVerification` mit dem Text „remote save may already be committed after an ambiguous promotion response“ (`:141-154`, Text in `:142`).
10. Nachprüfung per `stat` (`:156-176`), Journal-Löschung (`:178-190`), dann Zustand `Clean` (`:191-192`) und `Committed` (`:195`).

Die deutschen Texte „die gespeicherte Fassung liegt als …“ stehen im Kern (`file_commit.rs:80-81`, `:132-133`, `:150`).

### F4: `drive_mount_supported` und die Plattformen

Definition: `native/src/mount/mod.rs:279-281`, `pub const fn drive_mount_supported() -> bool { cfg!(windows) }`. Das Ergebnis ist zur Übersetzungszeit festgelegt: `true` nur bei Windows-Builds.

Aufrufer in den gelesenen Dateien: genau einer, `native/src/app/core/mount_ui.rs:142`, das Gate für das Status-Polling. `native/src/cli/drive.rs` prüft nicht. Die Funktion `mount()` (`:111-151`) ruft direkt `crate::daemon::start_mount` (`:147`) auf.

Ergebnis je Plattform:
- Windows: `true`.
- Linux: `false`.
- Android: `false`. Annahme: Das Android-Ziel erfüllt `cfg(windows)` nicht. Ein Android-Build wurde nicht geprüft.

Nicht-Windows-Pfade in `native/src/mount/mod.rs`:
- `drive_runtime_info` (`:283-298`): `#[cfg(not(windows))]`, Ergebnis `Err("virtual drive hosts are supported only on Windows")` (`:296-297`).
- `install_drive_runtime` (`:300-310`): Ergebnis `Err("Dokany kann nur unter Windows installiert werden")` (`:307-308`).
- `run_host_if_requested` (`:315-327`): bei `--mount-host <id>` auf Nicht-Windows `Some(Err("virtual drive hosts are supported only on Windows"))` (`:325-326`).

Fehlende Codepfade für Linux (Fakten aus den gelesenen Dateien):
- Kein OS-Modul für Linux: `native/src/mount/os/mod.rs:1-2`.
- Keine Laufzeit-, Installations- oder Hostfunktion. Die drei Funktionen aus `mount/mod.rs` liefern nur Fehler (`:296-297`, `:307-308`, `:325-326`).
- Keine Einhängung in einen Linux-Pfad.
- Das Status-Polling ist abgeschaltet (`mount_ui.rs:142`).
- Vorhanden ist der plattformneutrale Kern. Core-Tests laufen auf Nicht-Windows: vier Testmodule mit `cfg(all(test, not(windows)))` (`native/src/mount/mod.rs:85-99`).

Fehlende Codepfade für Android (Fakten aus den gelesenen Dateien):
- Keine Mount-Methode in der Android-Brücke. `native/src/mobile/os/shared/domains/mod.rs` hat die Methoden `conn` (`:82`) und `gdrive.status`, `gdrive.configure`, `gdrive.signIn`, `gdrive.signOut` (`:161-164`). Eine Methode `mount.*` oder `drive.*` gibt es nicht.
- Die Suche `mount|drive` in `native/src/mobile/os/shared/domains/` ergab nur Kommentare, Google-Drive-Code und den Sync-Filter `crossMounts` (vollständige Trefferliste in Abschnitt 5, Bereich D).
- Ob `crate::mount` in den Android-Build kompiliert wird, ist nicht belegt (Lücke L5).

### F5: Oberflächen der Laufwerke (vollständig)

A. CLI `se drive` (`native/src/cli/drive.rs`, `native/src/cli/drive_options.rs`)
- Unterbefehle (`drive.rs:26-38`): `runtime` (prüft Laufzeit und Treiber), `install-runtime` (installiert die fest gepinnte offizielle Dokany-Laufzeit), `mount`, `list`, `unmount`, `retry`.
- `runtime [--json]` (`drive.rs:19-53`, `OutputArgs`).
- `install-runtime [--msi PFAD] [--json]` (`drive.rs:41-47`). `--msi` ist versteckt (`:43`).
- `mount <TARGET> [--letter auto|BUCHSTABE] [--read-write] [--trust-remote-root] [--metadata-depth 0..=4] [--cache-mib 0..=65536] [--system-runtime] [--label TEXT] [--json]` (`drive_options.rs:5-46`).
  - Vorgaben: `--letter auto` (`:9-15`); Lesemodus ohne `--read-write` (`:16-17`); `--metadata-depth` 2 (`:23-29`); `--cache-mib` 500 (`:30-37`); `--system-runtime` aus (`:38-42`); `--label` mit der Vorgabe „Smart Explorer“ (`drive.rs:121`).
- `list [--json]` (`drive.rs:33-34`, `:55-61`).
- `unmount <SELEKTOR> [--json]` und `retry <SELEKTOR> [--json]` (`drive.rs:35-38`). Selektor: Mount-ID oder Buchstabe wie `M:` (`:57`); Auflösung in `:205-226`.
- TARGET-Formen (`drive_options.rs:7`, `drive.rs:246-289`): gespeicherter Eintrag `@label:/pfad`, Remote-URL, `gdrive://`-Pfad, `share://`-Endpunkt, UNC-Pfad. Sonst Fehler: „only saved remotes, Google Drive, and Smart Explorer Share peers can be mounted“ (`drive.rs:288`).
- Der Anzeigename des Endpunkts wird verworfen (`drive.rs:112`, Variable `_display_label`). Der Kommentar in `:119-120` begründet das: Anzeigetext von Endpunkt und Konto braucht eine ausdrückliche Option `--label`.
- Textausgabe (Tabulatoren):
  - `dokany\tready\trequired=…\tlibrary=…\tdriver=…\trequired_library_api=…\tlibrary_api=…\trequired_driver_protocol=…\tdriver_protocol=…` (`drive.rs:97-106`).
  - `dokany\t<Meldung>`; bei Fehler auf stderr `dokany\tfailed\t<Meldung>` (`:82-84`).
  - `drives\t0` (`:161`); je Laufwerk `drive\t<id>\t<auto|BUCHSTABE>\t<read-only|read-write>\t<status>\t<label>` (`:420-427`).
  - Statuswerte (`:430-444`): `unmounted`, `mounting`, `mounted:<X:>`, `unmounting`, `runtime-unavailable:<detail>`, `conflict:<pfad>:<detail>`, `failed:<detail>`.
- JSON:
  - `runtime --json`: `DriveRuntimeInfo` (`native/src/mount/mod.rs:183-194`).
  - `install-runtime --json`: `DriveRuntimeInstallOutcome` mit dem Feld `status` (`mod.rs:196-208`).
  - `mount`, `unmount`, `retry` mit `--json`: `MountSnapshot` mit `config`, `status`, `recovery`, `recovery_required` (`native/src/mount/core/recovery_state.rs:35-45`).
  - `list --json`: Liste von `MountSnapshot` (`drive.rs:153-159`).
- Exitcodes:
  - `mount`, `unmount`, `retry`: 1 bei `Conflict`, `Failed` oder `RuntimeUnavailable`, sonst 0 (`drive.rs:446-457`).
  - `install-runtime`: `outcome.exit_code()` (`drive.rs:86`; Zuordnung `mod.rs:224-235`).
  - `runtime`: 0 bei Erfolg (`drive.rs:108`). Ein Fehler von `drive_runtime_info` wird als Fehlertext zurückgegeben.
- Wartezeit: `START_TIMEOUT` 120 s (`drive.rs:17`), Abfrage alle 150 ms (`:185-203`), Fehler bei Zeitüberschreitung (`:195-201`).
- Vervollständigung: Die Inventur behauptet `clap_complete` (`docs/oberflaechen/einstellungen.md:11`). `native/src/cli/mod.rs` ist nicht gelesen; die Aussage ist nicht geprüft.

B. Desktop-Oberfläche (egui), `native/src/app/core/`
- Fenster „Smart-Explorer-Laufwerke“ (`mount_ui.rs:167`):
  - Leerzustand: „Keine Laufwerke in dieser Smart-Explorer-Sitzung.“ (`:180`).
  - Je Laufwerk: Name, Status, Ausklappbereich „Laufwerkdetails“ mit „ID: …“ und Recovery-Text (`:188-191`).
  - „Erneut versuchen“ (`:199`, bei `Failed` und `RuntimeUnavailable`) ruft `retry_mount` auf (`:247-249`).
  - „Entfernen“ (`:209`, nur wenn der Recovery-Zustand `clean` ist; Tooltip `:212`) und „Auswerfen“ (`:224`) erzeugen beide `action = (false, id)` und damit `stop_mount` (`:250-253`).
  - „Dokany-Laufzeit / Hilfe“ (`:237`) mit der Schaltfläche „Dokany 2.3.1 installieren“ (`native/src/app/core/mount_runtime_ui.rs:11`) ruft `install_drive_runtime(None)` auf (`mount_ui.rs:242-243`).
- Fenster „Remote als Laufwerk“ (`mount_ui_draft.rs:136`):
  - Remote-Wurzel; bei Peer eine Auswahlliste (`:275-307`).
  - „Laufwerksbuchstabe:“ (`:311`); „Ordnerstruktur vorladen:“ (`:335`) mit den Stufen „nur bei Bedarf“, „Wurzel“ und „<n> Ebenen“ (`:343-345`).
  - Cache-Bereich aus `native/src/app/core/mount_ui_cache_settings.rs`.
  - „Name:“ (`:152`); „Schreibzugriff erlauben“ (`:161`).
  - Vertrauens-Checkbox „Dieser ausgewaehlten Remote-Wurzel ausdruecklich vertrauen“ (`:174`).
  - Hinweis „Voraussetzung: offizielle Dokany-2.3.1-Laufzeit (kein Entwicklermodus).“ (`:182`).
  - „Manueller Download“ (`:185`), „Einbinden“ (`:196`, ruft `start_mount` auf, `:263-265`), „Abbrechen“ (`:201`).
- Cache-Bereich (`mount_ui_cache_settings.rs:5-24`): „Datei-Cache pro Laufwerk:“ mit einem Feld von 0 bis 65 536 „MiB“ (`:11-14`); drei Hinweise (`:19-21`); Checkbox „Kompatibilitaetsmodus: offizielle System-Dokany-DLL“ (`:22-23`).
- Einstiegsfunktionen: `offer_mount_saved` (`mount_ui_draft.rs:26`), `offer_mount_gdrive` (`:43`), `offer_mount_peer` (`:57`). Die Aufrufer liegen außerhalb des Lesebereichs (Lücke L10).
- Peer-Prüfung (`native/src/app/core/mount_peer_roots.rs`):
  - Discovery über `open_share_backend` (`:40`).
  - Wurzeln: `/Verbindungen/<name>` als „Verbindung: <name>“ (`:181-199`, Text `:193`); Top-Level-Ordner; `/` als „Alle freigegebenen Bereiche (nur Lesen)“ (`:201-205`, Text `:204`).
  - Fähigkeitsprüfung über `probe_share_mount_capabilities` (`:76`, `:96`). Es läuft höchstens eine Prüfung gleichzeitig (`:99-117`, `PEER_CAPABILITY_PROBE_ACTIVE`).
- Meldungen: Alarme (`mount_ui_helpers.rs:19-47`), Statusbezeichnungen (`:90-104`), Recovery-Bezeichnungen (`:75-81`).
- Polling alle 2 s (`mount_ui.rs:8`), nur wenn `drive_mount_supported()` wahr ist (`:142-149`).

C. Android-Oberfläche: In den gelesenen Dateien gibt es keinen Laufwerksbildschirm. Das Manifest hat einen Ordner-Öffnen-Filter: `MainActivity` mit `VIEW` für `vnd.android.document/directory`, `resource/folder` und `inode/directory` (`android/app/src/main/AndroidManifest.xml:83-89`). Das ist kein Laufwerk. Die übrigen Android-App-Dateien sind nicht durchsucht (Lücke L5).

D. Android-Brücke (`native/src/mobile/os/shared/domains/`), Suche `mount|drive` vollständig:
- `gdrive.rs`: 13 Treffer (Google Drive).
- `sync_jobs.rs:56`: Kommentar „Device-arrival triggers never fire on Android (no removable-drive events).“
- `locations.rs:22-23`: `gdrive://`-Route; `:55`: Kommentar zu Mounts beim Entfernen eines Endpunkts.
- `mod.rs:2`, `:17-18`, `:82`, `:161-164` (Methoden `gdrive.*`).
- `connections.rs:219` und `:225`: Kommentare zu Laufwerken und Mounts am alten Endpunkt.
- `job_json.rs:5`, `:71` (`crossMounts`), `:176` (`flag("crossMounts", …)`): Sync-Filter „Eingehängte Dateisysteme“.
- `tests.rs:422-423`: Google-Drive-Pfade.
- Die Brücke `native/android-bridge/` ist nicht gelesen.

E. Daemon-IPC: alle Aufrufe in den gelesenen Dateien:
- `crate::daemon::start_mount`: `native/src/cli/drive.rs:147`, `native/src/app/core/mount_ui_draft.rs:264`.
- `crate::daemon::list_mounts`: `drive.rs:154`, `:190`, `:213`; `mount_ui.rs:147`.
- `crate::daemon::stop_mount`: `drive.rs:172`; `mount_ui.rs:252`.
- `crate::daemon::retry_mount`: `drive.rs:179`; `mount_ui.rs:248`.
- `crate::daemon::open_share_backend`: `mount_peer_roots.rs:40`.
- `crate::daemon::probe_share_mount_capabilities`: `mount_peer_roots.rs:76`, `:96`.
- Host-Start: `native/src/mount/mod.rs:315-318`, genau zwei Argumente `--mount-host <id>`. Laut Kommentar `:312-314` braucht die Authentisierung die einmalige Umgebung, die der Daemon erzeugt.
- Wire-Protokoll: nicht gelesen. Graph-Einstiege, nicht am Quelltext geprüft: `native/src/daemon/os/shared/ipc_protocol.rs` (`MountHostConfig` Zeile 157, `IpcRequest` 251, `IpcResponse` 378), `native/src/daemon/os/shared/mount_client.rs` (`MountHostSession` 60, `connect_backend` 171), `native/src/daemon/os/shared/ipc_client.rs` (`open_share_backend` 22).

F. MCP: In den gelesenen Dateien ist kein MCP-Werkzeug für Laufwerke belegt. graphify lieferte nur Knoten in `native/src/cli/mcp/` (`jobs.rs`, `job_view.rs`, `exec_session_job.rs`, `exec_share_job.rs`), darunter `drive()` in `native/src/cli/mcp/exec_share_job.rs:61`. Dessen Inhalt ist nicht gelesen (Lücke L7). Die Stichwortsuche in `docs/oberflaechen/einstellungen.md` (laufwerk, mount, dokany, drive) fand keine MCP-Laufwerkszeile; Zeile 126 betrifft Google Drive.

G. Netzprotokoll (Share): `crate::share::PeerOpenTarget` mit `from_endpoint` und `endpoint_prefix` (`native/src/cli/drive.rs:254`, `:255`) ist nicht gelesen.

H. Dokumentation:
- `docs/oberflaechen/einstellungen.md:111`: Zeile „Laufwerke: Laufzeit, Einbinden mit Buchstabe, Schreiben, Wurzel vertrauen, Metadaten-Tiefe, Cache, Systemlaufzeit, Name; Liste, Trennen, Wiederholen“, Belege `N/cli/drive.rs:25-38`, `N/cli/drive_options.rs:5-46`, `N/app/core/mount_ui_draft.rs:127-260`, `mount_ui.rs:155-276`.
- `docs/oberflaechen/einstellungen.md:26`: Ausnahme „Laufwerke als Windows-Laufwerksbuchstaben“. Die Zelle endet mit dem Wort „ohne“ (Lücke L12).

### F6: Zuverlässigkeit und offene Punkte (wörtlich)

Mit Status laut `docs/TODO.md` (im Original als Zeichen, hier als Wort):
- `docs/TODO.md:95` (D1, Status: in Arbeit, offen): „Remaining live scope includes editor save/atomic-replace and conflict/retry, direct↔relay changes, physical reconnect, policy-change fail-closed behavior and eject.“ Und: „Neither synthetic mounted fixtures nor Obsidian-style operations certify an actual Obsidian installation or every remote.“
- `docs/TODO.md:63` (MOUNT-META-TESTS, Status: offen): „Drei Tests des Laufwerks-Metadaten-Caches scheitern unabhängig von aktuellen Änderungen“. Zu Test (2): „warum der Test unter Windows besteht, ist nicht geklärt“. Zu Test (3): „ob Test oder Cache falsch ist, ist nicht bestimmt.“
- `docs/TODO.md:124` (D2, Status: erledigt): „no measured speedup, actual Obsidian certification or catch-all live-hang recovery claim“.
- `docs/TODO.md:125` (D3, Status: erledigt): „actual Obsidian/all-transport certification remains separately under D1“.
- `docs/TODO.md:72` (MC1, Status: erledigt): „The user's original 450-GB directory/backend were unavailable for direct attribution.“
- `docs/TODO.md:62` (SHAREWRITE, Status: erledigt): Laufwerks-Speichern ohne `.se-mount-…`-Namen ist als erledigt markiert. Kein offener Punkt.

Aus den Dokumenten `docs/MOUNT_*.md` und `docs/REMOTE_FS_TRADEOFFS.md`:
- `docs/REMOTE_FS_TRADEOFFS.md:148-149`: „Plain SFTP, Local/UNC, WebDAV and FTP remain unverified against an external path-component swap.“
- `docs/REMOTE_FS_TRADEOFFS.md:154-156`: „exporting Local/UNC/plain SFTP does not remove its TOCTOU window and remains `Unverified`.“
- `docs/REMOTE_FS_TRADEOFFS.md:203-205`: „Google Drive can safely stage a create/update but cannot promise one atomic pathname replacement for editor temp-rename; WebDAV `MOVE Overwrite:T` and the current FTP/FTPS surface likewise do not supply the full contract.“
- `docs/REMOTE_FS_TRADEOFFS.md:274-277`: „free-space reporting reflects the local spool lower bound, while a remote quota can fail only at commit“ und „uncached files need a live backend, and a first open downloads the full file.“
- `docs/REMOTE_FS_TRADEOFFS.md:278-283`: „The remaining acceptance gap is a real Windows machine running the official Dokany 2.3.1.1000 runtime with DLL API 231 and driver protocol 0x190/400, Explorer and Obsidian.“ Und: „cannot be inferred from a GNU cross-build.“
- `docs/MOUNT_OBSIDIAN_STARTUP.md:55`: „A Linux run cannot certify Windows/Dokany watcher behavior.“
- `docs/MOUNT_OBSIDIAN_STARTUP.md:81`: „Repeated timed-out waves can exhaust the server's 16 live workers“.
- `docs/MOUNT_OBSIDIAN_STARTUP.md:176-181`: „The remaining gap is the user's actual stalled phase/request, so milestone 2's implementation is intentionally not selected yet.“ Und: „Do not release this guard alone as an Obsidian fix.“
- `docs/MOUNT_VAULT_METADATA.md:252-253`: „These are synthetic loopback acceptance timings, not a user-network or actual Obsidian benchmark.“
- `docs/MOUNT_BULK_ACCESS.md:278-279`: „Queue progress is not backend cancellation. A genuinely blocked synchronous provider operation cannot be safely killed by forgetting its client request.“
- `docs/MOUNT_OPTIMIZATION.md:150`: „likewise does not guarantee timely Storage Sense reclamation of a custom cache.“ Und `:222`: „Space queries cannot reserve capacity against unrelated apps.“
- `docs/MOUNT_RECOVERY_CACHE_REPAIR.md:3-4`: „gemeldeter wiederkehrender Startfehler in 0.5.167“ (behoben laut `docs/TODO.md:72`, Version 0.5.168). `:5-7`: „eine Zuordnung einzelner Dateien zu diesen 450 GB ist deshalb noch kein gesicherter Befund.“

Historische Aussagen (als solche zu lesen):
- `docs/MOUNT_BATCHING.md:5-6` beschreibt den Stand 0.5.150: „The current cache/private-DLL optimization is a separate, unverified candidate in the same project“. Laut `docs/TODO.md:124` (D2, Status: erledigt, Version 0.5.151) ist die Optimierung inzwischen veröffentlicht.
- `docs/MOUNT_OPTIMIZATION.md:3`: „This is historical v0.5.151 task-scoped design/evidence, not a second live work board.“

Code-Ebene (Kommentare im Kern):
- `native/src/mount/core/journal.rs:32`: „A dispatched quarantine move has no restart-safe commit/rollback inference.“ Variante `DeletePhase::Unresolved` (`:31-33`).
- `native/src/mount/core/engine.rs:106`: „remote delete transaction remains unresolved; preserve the mount cache and retry or resolve the original and quarantine paths“.
- `native/src/mount/core/file_commit.rs:141-142`: „remote save may already be committed after an ambiguous promotion response“.
- `native/src/mount/core/types.rs:474-475`: „Windows must not replay it; the host stops and remounts after surfacing the detail.“

### F7: Android-Provider

Das Manifest `android/app/src/main/AndroidManifest.xml` (vollständig gelesen, 154 Zeilen) enthält genau zwei `<provider>`-Elemente:
1. `.system.LocalFileProvider`, `android:authorities="${applicationId}.localfiles"`, `exported="false"`, `grantUriPermissions="true"` (`:137-141`).
2. `androidx.core.content.FileProvider`, `android:authorities="${applicationId}.files"`, `exported="false"`, `grantUriPermissions="true"`, Metadaten `android.support.FILE_PROVIDER_PATHS` mit `@xml/file_paths` (`:144-152`). Kommentar: „Cache copies only (remote files to open/share, downloaded update APK)“ (`:143`).

Ergebnis: Ein DocumentsProvider fehlt. Es gibt kein `<provider>` mit einem DocumentsProvider-Intent-Filter. Die Berechtigungsliste (`:6-46`) enthält kein `MANAGE_DOCUMENTS`. In den gelesenen Dateien leitet keine Klasse von `DocumentsProvider` ab. Die übrigen Klassen der App sind nicht gelesen (Lücke L5).

`android/app/src/main/java/app/smartexplorer/android/system/LocalFileProvider.kt` (vollständig gelesen, 130 Zeilen):
- `class LocalFileProvider : ContentProvider()` (`:26`). `attachInfo` verweigert eine exportierte oder nicht grant-fähige Instanz (`:29-34`).
- URI-Form `content://<package>.localfiles/<Pfadsegmente>` (`:21`, `:96-104`).
- Nur kanonische Pfade unterhalb gemeldeter Volumes (`:84-90`, `:113-128`). Die Volumes kommen aus `Storage.volumes(context)` (`:119`); `Storage` ist nicht gelesen.
- `query` liefert `DISPLAY_NAME` und `SIZE` (`:38-62`). `openFile` wandelt `"w"` in `"wt"` um (`:64-74`, `:67-69`). Der Kommentar in `:67-68` begründet das: Ein kürzerer Inhalt darf keinen Rest des alten Inhalts im Original hinterlassen.
- `insert`, `delete` und `update` wirken nicht (`:77-81`). Der Kommentar: „the provider never creates or deletes.“ (`:76`).
- `getType` über die Dateiendung (`:36`, `:107-111`).

Berechtigung `MANAGE_EXTERNAL_STORAGE` mit Kommentar „All-files access: the core works on real file paths of every mounted volume“ (`:17-20`).

## 3. Frontend-Grenze: Liste der Schnittstellen

Ebene A, Oberfläche zu Daemon (vollständig, siehe F5 E): `crate::daemon::start_mount`, `list_mounts`, `stop_mount`, `retry_mount`, `open_share_backend`, `probe_share_mount_capabilities`. Keine dieser Funktionen ist im Lesebereich definiert (Lücke L2).

Ebene B, Daemon zu Host:
- `native/src/mount/mod.rs:315`: `pub fn run_host_if_requested(arguments: &[std::ffi::OsString]) -> Option<Result<(), String>>`. Erkennt `--mount-host <id>`.
- `native/src/mount/os/windows` (`os::windows::run_mount_host`, `mod.rs:324`): nicht gelesen.
- Konfiguration für den Host (`MountRuntimeConfig`): „Sanitized configuration passed to a filesystem host. The host receives an already-rooted Backend and therefore never needs account or peer identity.“ (`types.rs:323-324`).

Ebene C, öffentliche Funktionen auf Modulebene (`native/src/mount/mod.rs`):
- `pub const fn drive_mount_supported() -> bool` (`:279`)
- `pub fn drive_runtime_info() -> Result<DriveRuntimeInfo, String>` (`:283`)
- `pub fn install_drive_runtime(local_msi: Option<&std::path::Path>) -> Result<DriveRuntimeInstallOutcome, String>` (`:300-302`)
- `pub fn run_host_if_requested(…)` (`:315`)

Ebene D, Engine-Schnittstelle. `MountEngine` ist über `pub use engine::MountEngine` (`mod.rs:126`) erreichbar. Vollständige Liste der Methoden in `impl MountEngine` und der zugehörigen Funktionen, die in `mount/core` als `pub` oder `pub(crate)` markiert sind. Die Signaturen sind Kopfzeilen aus dem Lesestand; wo mehrzeilig gelesen, sind alle Parameter genannt.

| Eintrag | Sichtbarkeit | Signatur | Beleg |
|---|---|---|---|
| `MountEngine::open` | pub | `(config: MountConfig, backend: BackendHandle, spool_root: impl AsRef<Path>) -> io::Result<Self>` | `core/engine.rs:61-65` |
| `MountEngine::open_host_cache` | pub | `(config: MountRuntimeConfig, backend: BackendHandle, spool_root: impl AsRef<Path>) -> io::Result<Self>` | `core/startup.rs:11-15` |
| `prepare_host_remote` | pub | `(&self) -> io::Result<()>` | `core/startup.rs:26` |
| `config` | pub | `(&self) -> &MountRuntimeConfig` | `core/engine.rs:78` |
| `dirty_entries` | pub | `(&self) -> io::Result<Vec<(String, EntryCondition)>>` | `core/engine.rs:82` |
| `case_sensitive_paths` | pub | `(&self) -> bool` | `core/case_semantics.rs:31` |
| `with_cache_space_probe` | pub | `(mut self, probe: Arc<dyn CacheSpaceProbe>) -> Self` | `core/cache_space.rs:28` |
| `open_file` | pub | `(&self, callback_path: &str, options: OpenFileOptions) -> io::Result<HandleId>` | `core/file_io.rs:10` |
| `read` | pub | `(&self, handle: HandleId, offset: u64, output: &mut [u8]) -> io::Result<usize>` | `core/file_io.rs:103` |
| `write` | pub | `(&self, handle: HandleId, offset: u64, input: &[u8]) -> io::Result<usize>` | `core/file_io.rs:127` |
| `append` | pub | `(&self, handle: HandleId, input: &[u8]) -> io::Result<usize>` | `core/file_io.rs:150` |
| `len` | pub | `(&self, handle: HandleId) -> io::Result<u64>` | `core/file_io.rs:169` |
| `truncate` | pub | `(&self, handle: HandleId, length: u64) -> io::Result<()>` | `core/file_io.rs:185` |
| `flush` | pub | `(&self, handle: HandleId) -> io::Result<FlushOutcome>` | `core/file_io.rs:198` |
| `close` | pub | `(&self, handle: HandleId) -> io::Result<()>` | `core/file_io.rs:207` |
| `open_metadata_file` | pub(crate) | `(&self, callback_path: &str, metadata: VfsMeta, writable: bool) -> io::Result<HandleId>` | `core/file_io.rs:47-52` |
| `stat` | pub | `(&self, callback_path: &str) -> io::Result<VfsMeta>` | `core/metadata.rs:16` |
| `stat_cached` | pub(crate) | `(&self, callback_path: &str) -> io::Result<VfsMeta>` | `core/metadata.rs:30` |
| `stat_handle` | pub | `(&self, handle: HandleId) -> io::Result<VfsMeta>` | `core/metadata.rs:56` |
| `list_dir` | pub | `(&self, callback_path: &str) -> io::Result<Vec<VfsMeta>>` | `core/metadata.rs:67` |
| `list_dir_cached` | pub(crate) | `(&self, callback_path: &str) -> io::Result<Arc<[VfsMeta]>>` | `core/metadata.rs:72` |
| `mkdir` | pub | `(&self, callback_path: &str) -> io::Result<NamespaceOutcome>` | `core/mutations.rs:10` |
| `rename` | pub | `(&self, source_callback_path: &str, destination_callback_path: &str, replace_existing: bool) -> io::Result<RenameOutcome>` | `core/mutations.rs:96-101` |
| `rename_with_shared_destination` | pub(crate) | Kopf `(…)`; Parameter ab Zeile 110 nicht gelesen | `core/mutations.rs:110` |
| `flush_path` | pub | `(&self, callback_path: &str) -> io::Result<FlushOutcome>` | `core/mutations.rs:339` |
| `begin_delete` | pub | `(&self, callback_path: &str, is_directory: bool) -> io::Result<DeleteToken>` | `core/delete.rs:110` |
| `cancel_delete` | pub | `(&self, token: DeleteToken) -> io::Result<()>` | `core/delete.rs:192` |
| `commit_delete` | pub | `(&self, token: DeleteToken) -> io::Result<()>` | `core/delete.rs:235` |
| `delete` | pub | `(&self, callback_path: &str, is_directory: bool) -> io::Result<()>` | `core/delete.rs:294` |
| `pending_deletes` | pub | `(&self) -> io::Result<Vec<DeleteToken>>` | `core/delete.rs:299` |
| `retry_pending_changes` | pub | `(&self) -> io::Result<()>` | `core/recovery.rs:9` |
| `maintain_cache` | pub | `(&self) -> io::Result<()>` | `core/entry_lifecycle.rs:119` |
| `evict_clean` | pub | `(&self, callback_path: &str) -> io::Result<bool>` | `core/entry_lifecycle.rs:170` |
| `preload_metadata` | pub(crate) | `(&self) -> io::Result<()>` | `core/metadata_loading.rs:25` |
| `preload_metadata_batch` | pub(crate) | `(&self) -> io::Result<usize>` | `core/metadata_loading.rs:36` |
| `preload_metadata_batch_while` | pub(crate) | Parameter nicht gelesen | `core/metadata_loading.rs:40` |
| `preload_metadata_progress_while` | pub(crate) | Parameter nicht gelesen | `core/metadata_loading.rs:47` |
| `refresh_metadata` | pub(crate) | `(&self) -> io::Result<()>` | `core/metadata_loading.rs:73` |
| `refresh_metadata_while` | pub(crate) | Parameter nicht gelesen | `core/metadata_loading.rs:77` |
| `drain_metadata_changes` | pub(crate) | `(&self, limit: usize) -> io::Result<Vec<MetadataChange>>` | `core/metadata_loading.rs:96` |
| `PathProjector::new` | pub | `(root: BackendRoot) -> Self` | `core/path.rs:29` |
| `PathProjector::root` | pub | `(&self) -> &BackendRoot` | `core/path.rs:33` |
| `PathProjector::project` | pub | `(&self, callback_path: &str) -> io::Result<ProjectedPath>` | `core/path.rs:39` |
| `ProjectedPath::relative` | pub | `(&self) -> &str` | `core/path.rs:14` |
| `ProjectedPath::backend` | pub | `(&self) -> &str` | `core/path.rs:18` |
| `prepare_spool_root` | pub | `(path: &Path) -> io::Result<PathBuf>` | `core/spool.rs:29` |
| Trait `CacheSpaceProbe` | pub | `pub trait CacheSpaceProbe: Send + Sync`; Methoden nicht gelesen | `core/cache_space.rs:5` |
| Enum `MetadataChange` | pub, Modul privat | Varianten nicht gelesen | `core/metadata_changes.rs:15` |

Weitere Typen, die über `pub use` in `native/src/mount/mod.rs:126-142` erreichbar sind: `MountCachePolicy`, `MountRuntimePreference`, `DEFAULT_MOUNT_CACHE_MIB`, `MAX_MOUNT_CACHE_MIB`, `MountMetadataPolicy`, `DEFAULT_METADATA_PRELOAD_DEPTH`, `MAX_METADATA_PRELOAD_DEPTH`, `MountRecovery`, `MountSnapshot`, `BackendRoot`, `Baseline`, `DeleteToken`, `DriveLetter`, `DriveSelection`, `EntryCondition`, `FlushOutcome`, `HandleId`, `MountConfig`, `MountConflict`, `MountId`, `MountMode`, `MountRootSecurity`, `MountRuntimeConfig`, `MountSource`, `MountStatus`, `NamespaceOutcome`, `OpenDisposition`, `OpenFileOptions`, `PeerMountTarget`, `RenameOutcome`.

Ebene E, Dokany-Callbacks zu Ebene D: nicht gelesen (Lücke L1).

## 4. Wiederverwendbare Bausteine (mit Beleg)

Die Liste nennt belegte Bausteine. Sie bewertet nicht, ob sie für E4, E8 oder Linux passen.

- Validierte Typen: `MountId` (`types.rs:27-37`, Laden über `parse` in `:50-58`), `BackendRoot` (`types.rs:66-99`, Laden in `:106-114`), `DriveLetter` (`types.rs:179-183`, Laden in `:197-212`). `MountCachePolicy` prüft beim Laden (`cache_policy.rs:51-60`). Für `MountMetadataPolicy` ist die Prüfung beim Laden nicht gelesen (`metadata_policy.rs`). `MountSource` (`types.rs:159-171`) und `PeerMountTarget` (`types.rs:123-131`) haben eine `validate`-Funktion. `DriveSelection` (`types.rs:214-218`) hat keine eigene Prüfung.
- Pfadprojektion: `PathProjector` und `ProjectedPath` (`path.rs:8-75`), mit Windows-Komponentenregeln (`path.rs:90-127`).
- Journal: JSON-Zeilen mit Version, abgeschnittenem Ende beim Öffnen, Verdichtung und Rotation (`journal.rs:8-146`, `spool_recovery.rs:7`).
- Spool: `WholeFileSpool` mit Journal-Bezug und Waisenbereinigung (`spool.rs:20-27`, `:79-121`, `:225-244`).
- Stage-Ledger: Muster „erst erfassen, dann anlegen, dann auflösen“ (`stage_ledger.rs:30-118`).
- Konfliktkopie: `publish_conflict_copy` ohne Ersetzen (`file_commit_stage.rs:57-73`).
- Baseline-Vergleich: `Baseline::same_remote_state`; fehlende Fakten gelten als unbekannt, nicht als verschieden (`baseline_match.rs:1-6`, `:10`).
- Konflikt- und Wiederholungslogik: `retry_pending_changes` (`recovery.rs:9-30`).
- Bereinigung des Inhalts-Caches: `CleanCache` mit Alters- und Anzahlgrenze (`clean_cache.rs:10-11`, `:112-190`).
- Metadaten-Batch: `run_metadata_batch` mit Abhängigkeiten nur zwischen ausgewählten Vorfahren (`metadata_batch.rs:1-2`, `:31`).
- Warteschlange ohne Duplikate: `KeyQueue` (`metadata_refresh_order.rs:7-66`).
- Share-Namensprojektion: `project_peer_listing` und `resolve_peer_child` (`peer_names.rs:16-63`).
- Tests auf Nicht-Windows: vier Testmodule (`native/src/mount/mod.rs:85-99`).
- Android-URI-Freigabe: `LocalFileProvider.uriFor` (`LocalFileProvider.kt:99-104`), `allowedFile` (`:113-128`), `mimeTypeOf` (`:107-111`).

## 5. Betroffene Verträge

Persistenz:
- `MountConfig` mit den Feldnamen aus `types.rs:237-252`; `#[serde(default)]` auf `root_security`, `metadata`, `cache`, `runtime_preference` (`:243-250`). Änderungen an Feldnamen oder Varianten betreffen die gespeicherte Form.
- `MountSource` und `PeerMountTarget` ohne serde-Attribut (`types.rs:134`, `:116`). Annahme: serde verwendet dann die Standarddarstellung mit Variantennamen als Schlüssel.
- `MountSnapshot`-Wire-Kompatibilität: Feld `recovery_required` (`recovery_state.rs:43-44`, `:56-73`).
- `DriveRuntimeInfo`: die Felder `required_api` und `driver_api` sind als Legacy-Namen markiert (`mod.rs:185-193`).
- Journal: `version = 1` (`journal.rs:8`), Aktionsnamen (`:62-80`), Legacy-Phase `Prepared` (`:34-35`).
- `stages.json` mit `stages` als Schlüssel (`stage_ledger.rs:15`, `:17-21`). Verlust betrifft nur die Bereinigung (`:6-7`).
- Spool-Dateinamen `<32 Hex>.spool` (`spool.rs:307-310`).
- Stage-Muster `<name>.se-mount-<16 hex>` (`file_commit_stage.rs:1`, `stage_ledger.rs:1`).
- Bezeichnung höchstens 128 Zeichen (`types.rs:281-288`).

IPC (Daemon):
- Aufrufe in Abschnitt F5 E. Der Host-Aufruf `--mount-host <id>` (`mount/mod.rs:315-318`).
- `MountSnapshot`-JSON für CLI und Daemon (`recovery_state.rs:35-45`).

Bridge (Android):
- Keine Mount-Methode in `native/src/mobile/os/shared/domains/` (Abschnitt F5 D).
- Vollständige Trefferliste der Suche `mount|drive` in dieser Datei-Gruppe: `gdrive.rs` (13 Treffer), `sync_jobs.rs:56`, `locations.rs:22-23`, `locations.rs:55`, `mod.rs:2`, `mod.rs:17-18`, `mod.rs:82`, `mod.rs:161-164`, `connections.rs:219`, `connections.rs:225`, `job_json.rs:5`, `job_json.rs:71`, `job_json.rs:176`, `tests.rs:422-423`. Zählung nach Datei: gdrive.rs 13, sync_jobs.rs 1, locations.rs 3, mod.rs 8, connections.rs 2, job_json.rs 3, tests.rs 2.

CLI:
- Optionsnamen und Vorgaben (`drive_options.rs:5-46`), Unterbefehle (`drive.rs:26-38`).
- Textzeilen mit Tabulatoren (`drive.rs:82-84`, `:97-106`, `:161`, `:420-427`), Statuswörter (`:430-444`), Exitcodes (`:446-457`; `mod.rs:224-235`).
- Fehlertexte, die Skripte sehen können: „drive letter must be `auto` or one ASCII letter“ (`drive.rs:239`), „only saved remotes, Google Drive, and Smart Explorer Share peers can be mounted“ (`:288`), „no saved connection authorizes that remote path; save the remote first“ (`:303`), „no saved network-share connection authorizes that UNC path; save it first“ (`:315`), „ambiguous saved {kind}: {accounts}“ (`:350`), „no managed drive is mounted at {wanted}“ (`:223`).

GUI:
- Vorgaben beim Öffnen: Name „Smart Explorer“, Automatisch, Standardtiefe, Standard-Cache, Systemlaufzeit aus, Lesemodus, Vertrauen aus (`mount_ui_draft.rs:84-90`).
- Auswahl der Buchstaben `D` bis `Z` (`mount_ui_draft.rs:321`).
- Deutsche Statustexte (`mount_ui_helpers.rs:75-104`) und Alarmtexte (`:28-44`).

Android-URI:
- Authority `${applicationId}.localfiles` (`AndroidManifest.xml:139`), `${applicationId}.files` (`:146`).
- URI-Form `content://<package>.localfiles/<Pfad>` (`LocalFileProvider.kt:21`, `:96-104`).

## 6. Lücken und offene Punkte

- L1: Dokany-Frontend nicht gelesen. Fehlt: Zuordnung Callback zu Engine-Methode, Wahl des Buchstabens bei `Automatisch`, Handle-Tabelle, Cache-Lease. Einträge aus dem Verzeichnis `native/src/mount/os/windows/` (nur Namen, Inhalt ungelesen; die Ausgabe war nach dem Eintrag `handle_state` abgeschnitten): `cache_lease.rs`, `cache_space.rs`, `callback_context.rs`, `callback_reporter.rs`, `callback_status.rs`, `callback_timeout/` (Verzeichnis), `callback_timeout.rs`, `callbacks.rs`, `callbacks_io.rs`, `callbacks_metadata.rs`, `callbacks_mutation.rs`, `callbacks_open.rs`, `callbacks_unsupported.rs`, `dokany_abi.rs`, `handle_access.rs`, `handle_reservation.rs`, `handle_state/` (Verzeichnis). Ob weitere Einträge existieren, ist nicht geprüft.
- L2: Daemon (`native/src/daemon/`) nicht gelesen. Fehlt: Ort und Format der gespeicherten Laufwerksliste; Semantik von `start_mount`, `stop_mount`, `retry_mount`, `list_mounts`; Trigger für Schritt 4 und 5 aus F2; Capability-Umgebung.
- L3: `crate::vfs` nicht gelesen. Fehlt: `Backend`, `BackendHandle`, `VfsMeta`, `MountPathCapabilities`, `StagedWriteCapabilities`, `RootConfinement`, `unique_staging_path`, `remote_util`, `DeleteDisposition`.
- L4: `crate::share` (`PeerOpenTarget`), `crate::creds`, `crate::connect`, `crate::net`, `crate::cli::target` (`saved_shorthand`) nicht gelesen.
- L5: Android. Ungeprüft: ob `crate::mount` in den Android-Build kompiliert wird (`native/src/lib.rs` und Cargo nicht gelesen); `native/android-bridge/` nicht gelesen; Android-App außer `AndroidManifest.xml` und `LocalFileProvider.kt` nicht durchsucht; `Storage.volumes` nicht gelesen.
- L6: Auslöser für `trim`, `evict_oldest`, `evict_path`, `maintain_cache`, `maintain_space`, `reserve_growth` nicht gelesen. Funktionen sind deklariert, ihr Aufrufweg ist nicht belegt.
- L7: MCP (`native/src/cli/mcp/`) nicht gelesen. Ein Laufwerks-Werkzeug ist nicht belegt.
- L8: `se drive mount`, `runtime`, `install-runtime` auf Linux und Android haben kein Gate in `native/src/cli/drive.rs` außer dem Verhalten der Funktionen aus `mount/mod.rs`. Das Verhalten von `crate::daemon::start_mount` auf diesen Plattformen ist nicht gelesen.
- L9: Aufrufer von `open_at_root`, `retry_pending_changes`, `recover_pending_deletes` und `prepare_host_remote` beim Start sind nicht gelesen.
- L10: Aufrufer von `offer_mount_saved`, `offer_mount_gdrive`, `offer_mount_peer` außerhalb von `mount_ui_draft.rs` sind nicht gelesen. Daher ist nicht belegt, ob Menüeinträge unter Linux oder Android erscheinen.
- L11: Auswahl des freien Buchstabens bei `DriveSelection::Automatic` ist nicht belegt (Windows-Host nicht gelesen).
- L12: `docs/oberflaechen/einstellungen.md:26` endet mit dem Wort „ohne“; die Zelle ist im Dokument unvollständig.
- L13: Im Lesebereich gibt es keine Einhängung in einen Ordner (Abschnitt F2). Diese Lesung bewertet nicht, ob sie gewünscht ist.
- L14: Zuverlässigkeit: Die drei Tests in `docs/TODO.md:63` sind als offen markiert (Status: offen); die Ursache für Test (2) ist ungeklärt.
- L15: Zuverlässigkeit: D1 (`docs/TODO.md:95`, Status: in Arbeit) ist offen. Laut Text fehlen Editor-Atomarersatz, Konflikt und Wiederholung, Trennung und Wiederverbindung, Richtlinienwechsel mit Fail-Closed-Verhalten sowie Auswurf.
- L16: `DeletePhase::Unresolved` hat laut `journal.rs:32` keine Regel für Wiederanlauf mit Commit oder Rollback.
- L17: Das Ergebnis `CommittedPendingVerification` (`types.rs:455-458`, `:474-475`; `file_commit.rs:141-154`) verlangt laut Kommentar `types.rs:455-457`, dass der Aufrufer Erfolg meldet und den Wiederherstellungszustand anzeigt. Wie es in der Oberfläche erscheint, ist nicht belegt; die Zuordnung liegt außerhalb des Lesebereichs. Wie oft der Fall auftritt, ist nicht belegt.
- L18: `docs/ARCHITEKTUR.md` (in `AGENTS.md` als erste Lesequelle genannt) ist nicht gelesen, weil es nicht im Lesebereich lag.

## 7. Grenzen der Lektüre

Gelesen, vollständig (Nicht-Test-Teile):
- `native/src/mount/mod.rs` (327 Zeilen), `native/src/mount/os/mod.rs` (2 Zeilen).
- `native/src/mount/core/`: `types.rs`, `startup.rs`, `spool.rs`, `stage_ledger.rs` (Zeilen 1–119), `file_commit.rs`, `file_commit_stage.rs` (Zeilen 1–83), `recovery.rs`, `recovery_state.rs`, `cache_policy.rs`.
- `native/src/app/core/mount_ui.rs`, `mount_ui_helpers.rs`, `mount_ui_draft.rs`, `mount_ui_cache_settings.rs`, `mount_runtime_ui.rs`, `mount_peer_roots.rs`.
- `native/src/cli/drive.rs` (461 Zeilen), `native/src/cli/drive_options.rs` (78 Zeilen).
- `android/app/src/main/AndroidManifest.xml` (154 Zeilen), `android/app/src/main/java/app/smartexplorer/android/system/LocalFileProvider.kt` (130 Zeilen).

Gelesen, teilweise:
- `native/src/mount/core/engine.rs`: Zeilen 1–120 gelesen; Rest nur Deklarationen.
- `native/src/mount/core/journal.rs`: Zeilen 1–150 gelesen; Rest nur Deklarationen.
- `native/src/mount/core/delete.rs`: Zeilen 76–84 gelesen; Rest nur Deklarationen.
- `native/src/mount/core/engine_recovery.rs`: Signatur `open_at_root` (`:14-21`) gelesen; Stichwortsuche über die Datei (Ausgabe auf 40 Treffer begrenzt); Rest nur Deklarationen.
- `native/src/mount/core/mutations.rs`: Signatur `rename` (`:96-101`) gelesen; Rest nur Deklarationen.
- `native/src/mount/core/file_io.rs`: Signatur `open_metadata_file` (`:47-52`); Rest nur Deklarationen.
- `native/src/mount/core/windows_access.rs`: Modul-Doku `:1-13`; Rest nur Deklarationen.

Nur Deklarationen und Modul-Doku (`rg` auf Deklarations- und `//!`-Zeilen). Diese 35 Dateien sind nicht im Detail gelesen:
`baseline_match.rs`, `cache_space.rs`, `case_semantics.rs`, `clean_cache.rs`, `commit.rs`, `delete_recovery.rs`, `entry_lifecycle.rs`, `entry_table.rs`, `materialization.rs`, `metadata.rs`, `metadata_batch.rs`, `metadata_cache.rs`, `metadata_cache_load.rs`, `metadata_cache_order.rs`, `metadata_cache_support.rs`, `metadata_changes.rs`, `metadata_loading.rs`, `metadata_point_cache.rs`, `metadata_point_order.rs`, `metadata_policy.rs`, `metadata_preload.rs`, `metadata_preload_records.rs`, `metadata_refresh_order.rs`, `metadata_schedule.rs`, `metadata_snapshot.rs`, `namespace_recovery.rs`, `open_handle.rs`, `optimization_fixture.rs`, `path.rs`, `peer_names.rs`, `range_read.rs`, `replace.rs`, `retirement_queue.rs`, `spool_recovery.rs`, `windows_case.rs`.

Alle 51 Nicht-Test-Dateien von `native/src/mount/core/` mit Lesestatus:
- vollständig (9): `cache_policy.rs`, `file_commit.rs`, `file_commit_stage.rs`, `recovery.rs`, `recovery_state.rs`, `spool.rs`, `stage_ledger.rs`, `startup.rs`, `types.rs`.
- teilweise (7): `delete.rs`, `engine.rs`, `engine_recovery.rs`, `file_io.rs`, `journal.rs`, `mutations.rs`, `windows_access.rs`.
- nur Deklarationen (35): siehe Liste oben.

Ausgeschlossen als Testdateien (Name enthält `tests`; nicht gelesen, 18 Dateien): `bulk_metadata_task_tests.rs`, `entry_table_task_tests.rs`, `metadata_cache_task_tests.rs`, `metadata_preload_task_tests.rs`, `metadata_snapshot_task_tests.rs`, `mount_save_task_tests.rs`, `navigation_cache_task_tests.rs`, `optimization_cache_tests.rs`, `optimization_metadata_tests.rs`, `optimization_policy_tests.rs`, `peer_names_task_tests.rs`, `range_read_task_tests.rs`, `recovery_cache_task_tests.rs`, `remote_drive_task_tests.rs`, `retirement_queue_task_tests.rs`, `vault_metadata_flight_tests.rs`, `vault_metadata_task_tests.rs`, `vault_scheduler_task_tests.rs`.

Dokumente, nur Gliederung, Stichwortzeilen und Zitatstellen gelesen (nicht vollständig):
- `docs/MOUNT_BATCHING.md`, `docs/MOUNT_BULK_ACCESS.md`, `docs/MOUNT_OBSIDIAN_STARTUP.md`, `docs/MOUNT_OPTIMIZATION.md`, `docs/MOUNT_RECOVERY_CACHE_REPAIR.md`, `docs/MOUNT_VAULT_METADATA.md`, `docs/REMOTE_FS_TRADEOFFS.md`.
- `docs/TODO.md`: alle 64 Zeilen mit den Stichwörtern `mount|laufwerk|dokany|drive` per Suche; Zeilen 9, 12, 62, 63, 72, 95, 124–127 vollständig oder gekürzt.
- `docs/oberflaechen/einstellungen.md`: Gliederung, Stichwortzeilen, Zeilen 26, 111, 113–114, 181–184.

Außerhalb des Lesebereichs (nicht gelesen):
- `native/src/mount/os/windows/` (Inhalt; nur Namen, siehe L1).
- `native/src/daemon/`, `native/src/vfs/`, `native/src/share/`, `native/src/cli/mcp/`, `native/android-bridge/`, die Android-App außer den zwei Dateien, `native/src/cli/mod.rs`, `native/src/lib.rs`.
- `docs/ARCHITEKTUR.md`, `docs/lesungen/` (außer dem Namen des Verzeichnisinhalts).
- Eine weitere Lesung desselben Tages im Verzeichnis (`docs/lesungen/2026-10-17-laufwerke-windows-dokany.md`) ist nicht gelesen und nicht verändert.

Graphify (schreibgeschützt, Einstiegspunkte, keine Belege): drei Abfragen mit `graphify query`. Die Abfragen ergaben die Einstiege, die in den Abschnitten F2, F5 E und F5 F als unverifiziert gekennzeichnet sind.

Annahmen (ausdrücklich):
- Annahme: Die Einhängeform ist ausschließlich der Laufwerksbuchstabe (F2).
- Annahme: Das Android-Ziel erfüllt `cfg(windows)` nicht (F4).
- Annahme: serde-Standarddarstellung für `MountSource` und `PeerMountTarget` (Abschnitt 5).

Verzeichnislisten außerhalb des Lesebereichs: `native/src/mount/os/windows` (`ls -la`, Ausgabe nach dem Eintrag `handle_state` abgeschnitten) und `docs/lesungen` (`ls`, nur Namen). Beide Listungen dienten der Orientierung.

Keine Daten der Geräte des Nutzers sind in dieser Lesung enthalten.
