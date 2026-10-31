# Gesamtreview der Umsetzung „Sync-Abgleich schnell“ (2026-10-31)

Gegenstand: `git diff 29310ace..ad100297 -- native/ android/ docs/ README.md` (Commits 1f004fd9,
df715327, bc18587b, 6f3ae219, ad100297). Maßstab: `plan.md` (C1–C8, D), `bestand-datenwege.md`
> Nachtrag 2026-10-31: Die Commits wurden danach auf den Release-Commit von v0.5.189 umgesetzt (Rebase); die hier genannten Kennungen entsprechen jetzt bef433c4 (`se sync compare`), db93d3a6 (`se ls -R`) und den folgenden Commits gleichen Titels auf main.

(Abschnitt 11), `referenzen-abgleich.md` (M1–M6), `docs/oberflaechen/{INDEX,uebertragungen,einstellungen}.md`,
`AGENTS.md`.

Methode: ausschließlich Lesen. Nichts wurde kompiliert, gebaut oder getestet (Projektregel). Jede Aussage
zur Kompilierbarkeit ist eine Leseprüfung. Zeilenangaben beziehen sich auf den Stand `ad100297`;
`N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

Zählung: **2 blockierend, 8 wesentlich, 17 gering.**

## Kompilierbarkeit nach Lesen (kein Befund)

Geprüft und ohne Befund:
- Alle Struct-Literale der neuen Felder: `HashWalkRequest::meta`, `HashWalkEntry::meta`,
  `FsHashEntry::meta`, `FsHashWalk::meta`, `WireHashEntry::meta`, `vfs::BatchPut::mtime_ms`,
  `agent_proto::BatchEntry::mtime_ms`, `JobRecord::file_update`, `FileWork::{conflict, replacing}`
  (Skript über alle Fundstellen in `N/`; die einzigen Treffer ohne Feld sind `Frame::BatchPut` und ein
  bereits vollständiges `FsHashEntry` in `N/share/os/shared/host_hash_file.rs:125`).
- Jedes erschöpfende `match` über `types::Conflict` behandelt `Conflict::Update`
  (`N/app/core/folder_conflict_ui.rs:163`, `N/daemon/os/shared/transfer_job_wire.rs:165`,
  `N/transfer/os/shared/engine/job_store.rs:65`, `N/copy/os/shared/local_publish.rs:141`, `:206`,
  `N/copy/os/shared/direct_move.rs:173`, die drei Testdateien mit `unreachable!`).
- Sichtbarkeiten: `WalkState::emit` jetzt `pub(super)`, `ext_of`/`join` aus dem Elternmodul,
  `ext_ops::md5_file` `pub(super)`, `ext_ops::set_mtime` `pub(super)`, `frame_ops::{code, counted,
  put_omission, get_omission, omission_len}` `pub(super)`, `vfs::literal_child_path` als Re-Export von
  `default_sync_child_path`, `vfs::LocalMountBoundaries` mit privatem Feld eines `pub(crate)`-Typs.
- Send/Sync der parallelen Durchläufe: `DirectoryHandle` (Linux `Arc<File>`, Windows `Arc<PinnedDirectory>`
  aus `File`/`PathBuf`), `agent_proto::Sink` (`Arc<Mutex<Box<dyn Write + Send>>>`), `host_hash_walk::Shared`
  (nur Referenzen auf `Sync`-Typen).
- clap: `conflicts_with_all = ["force", "resume_job"]` verweist auf vorhandene Argument-IDs
  (`N/cli/mod.rs:243`, `:275`); `From<FolderPolicy> for FolderConflict` existiert
  (`N/cli/folder_conflict.rs:23`).
- `Option::is_none_or` wird im Repository schon vorher verwendet (`N/sftp/core/stage_finish.rs:71`).

## Blockierend

### B1 — Ersetzen prüft den Zielstand nicht nach: ein inzwischen neueres Ziel wird überschrieben (Datenverlust)

- Stellen: `N/transfer/os/shared/engine/update.rs:188-197` (Entscheidung `SourceNewer` aus der
  Auflistung), `:313-345` (Auflistung je Zielordner wird für den ganzen Auftrag gehalten),
  `N/transfer/os/shared/engine/publish.rs:304`, `:336-356` (`promote_staged_to_id`),
  `N/vfs/core/promotion.rs:21-27` (Standard: vergleicht nur `stat(...).id`), lokal
  `N/transfer/os/shared/engine/download.rs:317-327` und `N/transfer/os/shared/engine/local.rs:55`
  (Überschreib-Veröffentlichung ohne erneuten Vergleich).
- Fehlerfall entfernt: Ziel SFTP, SMB, FTP, WebDAV, Direct Share oder SSH-Agent (alle ohne
  Objekt-ID, `id = None`). Einfügen mit „Zusammenführen“ in einen Zielordner mit 2 000 Dateien. Die
  Engine listet den Zielordner beim ersten Regelfall einmal (t0) und entscheidet für `bericht.docx`
  „Quelle neuer“. Während der Auftrag die anderen Dateien überträgt, speichert jemand `bericht.docx`
  im Ziel neu (t1, jetzt neuer als die Quelle). Erreicht die Engine die Datei (t2), vergleicht
  `promote_to_id` nur `None == None`, ersetzt per `promote_staged` und die Änderung von t1 ist ohne
  Sicherung verloren.
- Fehlerfall lokal: Download oder lokale Kopie einer großen Datei; `look_local` (`update.rs:265-286`)
  liest das Ziel vor der Übertragung, die Veröffentlichung (`replace_file_atomic`) liest es nicht
  noch einmal. Ein während der Übertragung neu gespeichertes Ziel wird ersetzt.
- Abweichung: C7 verlangt „Ziel um mehr als das Fenster neuer → Ziel behalten“; M5 verlangt den Vergleich
  mit dem tatsächlichen Ziel. Der Code-Kommentar `publish.rs:300-303` („bound to the object it saw; a
  target that … changed, keeps both“) und `docs/TRANSFER_ENGINE.md:545-550` („hat sich die Datei
  geändert (`InvalidData`) … bleibt beides“) behaupten einen Schutz, der nur für Backends mit Objekt-ID
  (Drive, Tresor) besteht. AGENTS.md „preserve existing behavior“ (Sicherungen/Konflikte nicht schwächen).
- Bedarf: unmittelbar vor der Veröffentlichung Größe und Änderungszeit des Ziels gegen den Stempel der
  Entscheidung vergleichen (der `stat` in `promote_to_id` liegt schon vor; lokal ein `lstat` vor
  `replace_file_atomic`), bei Abweichung `InvalidData` → beide behalten.

### B2 — Ältere Version setzt einen gespeicherten Android-Auftrag „Ersetzen + Zusammenführen“ als Überschreiben fort

- Stellen: `N/transfer/os/shared/engine/job_store.rs:65-70` (`file_policy` 1 bei `Overwrite`,
  `merge_update` steht nur in `folders`), `N/transfer/core/folder_conflict.rs:26-28`
  (`merge_update` mit `serde(default)`, ältere Struktur ignoriert unbekannte Felder; vor 29310ace kein
  `deny_unknown_fields`), `N/mobile/os/shared/transfer.rs:85`, `:113`, `A/ui/files/FileActions.kt:238`
  (Android sendet immer `mergeFiles: "update"`), `docs/TRANSFER_ENGINE.md:566`.
- Fehlerfall: Android, lokal → lokal, Namenskonflikt oberster Ebene „Ersetzen“ gewählt, ein Ordner
  „Zusammenführen“. Gespeicherter Auftrag: `file_policy = 1`, `folders.merge_update = true`. Der
  Auftrag wird unterbrochen; eine Version vor diesem Batch nimmt ihn wieder auf. Sie liest
  `merge_update` nicht und wendet `Overwrite` auf alle Dateien der zusammengeführten Ordner an: auch
  Ziele, die neuer sind, werden überschrieben.
- Abweichung: Auftrag „gebrochene Abwärtsverträglichkeit gespeicherter Aufträge“; die Doku-Aussage
  „eine ältere Version öffnet den Auftrag als ‚beide behalten‘ und ersetzt nie etwas“
  (`TRANSFER_ENGINE.md:566`) gilt nur für `file_update`, nicht für `merge_update` mit `file_policy` 1
  oder 2. Bedarf: bei `merge_update` den gespeicherten `file_policy` wie bei `file_update` auf 0 setzen
  und die Oberflächenwahl in einem eigenen Feld halten, oder die Datensatzversion erhöhen, damit ältere
  Versionen den Datensatz ablehnen.

## Wesentlich

### W1 — Unbekannte Zeit und gleiche Größe gilt als „gleich“ und wird übersprungen

- Stelle: `N/transfer/core/update_rule.rs:51-55`; ebenso Vorschau `N/transfer/os/shared/engine/selection_count.rs:381-391`.
- Fehlerfall: eine Seite meldet `mtime_ms = 0` (Backend ohne Zeitangabe, `modified()` schlägt fehl,
  `look_local` `update.rs:273-276`). Quelle geändert, gleiche Größe → `Case::Equal` → übersprungen; die
  neuere Fassung kommt nie ans Ziel.
- Abweichung: M1 („Größe gleich **und** Zeit im Fenster“), M6 („Überspringen nur über M1“) aus
  `referenzen-abgleich.md:653`, `:658`; Nutzervorgabe N2 nennt „gleiche“, nicht „gleich große“. Ohne
  Zeit ist Gleichheit nicht belegt; „beide behalten“ (Konflikt) zerstört nichts.

### W2 — Vorschau „Zielordner entscheiden“ beschriftet Zählwerte mit der Regel, auch wenn der Auftrag sie nicht anwendet

- Stellen: `N/app/core/folder_conflict_ui.rs:98`, `:171-180` (immer „neuer (ersetzen) … Ziel neuer
  (bleibt) … Konflikt (beide behalten)“), Zählung `N/transfer/os/shared/engine/selection_count.rs:103-172`
  ohne Blick auf `merge_update`/`conflict`; Kopierdialog-Auftrag `N/app/core/transfer_route.rs:147-160`
  setzt kein `merge_update`.
- Fehlerfall: Kopierdialog „Kopieren/Verschieben nach…“ mit „überschreiben“, Ordnerentscheidung
  „Zusammenführen“. Die Vorschau zeigt „12 Ziel neuer (bleibt)“, der Auftrag überschreibt diese 12
  (Zeile darüber nennt zwar „überschreiben“, die Zählzeile widerspricht ihr). Mit „beide behalten“ zeigt
  sie „gleich (übersprungen)“, der Auftrag legt `Name (2)` an.
- Abweichung: falsche Vorschau als Entscheidungsgrundlage; C7 beschreibt die fünf Zählwerte nur für die
  Regel. Bedarf: Zählzeile nur bei `merge_update` oder `Conflict::Update`, sonst „neu/gleichnamig“ wie bisher.

### W3 — Nicht tolerante Zielauflistung: ein nicht auflistbarer Eintrag lässt alle Regeldateien des Ordners scheitern

- Stellen: `N/transfer/os/shared/engine/update.rs:358` (`target.list_dir(dir)`),
  `N/transfer/os/shared/engine/selection_count.rs:243`; Share `N/share/core/backend.rs:171-178`
  (`list_dir` schlägt bei jeder Auslassung fehl, bestand V4).
- Fehlerfall: Ziel Direct Share; der Zielordner enthält einen Eintrag, den der Host als Auslassung meldet
  (nicht lesbar, nicht darstellbarer Name). `read_listing` liefert einen Fehler, `settle` gibt ihn bei jeder
  Datei dieses Ordners zurück (auch bei fehlenden, die nur eingefügt werden müssten), im Einzelweg
  `ops.rs:291` wird daraus ein Dateifehler mit Wiederholungen. Die Vorschau zählt alle Dateien des Ordners
  als „nicht lesbar“. Vorher (Rename ohne Regel, Vorschau mit `stat` je Datei) lief derselbe Ordner durch.
- Abweichung: K1 „gleiches Ergebnis“, AGENTS.md „preserve existing behavior“. Bedarf: `vfs::list_dir_tolerant`;
  ausgelassene Namen als `Found::Other` (beide behalten).

### W4 — Die Regel nimmt Ziele ohne Zeit in Paketen aus der Paketbildung: jede neue Datei einzeln

- Stellen: `N/transfer/os/shared/engine/update.rs:182-185`, `:221`, `:230-235`; `upload.rs:38-43`;
  `batch_put_times` implementieren nur Share-Peer, Agent und Cache (`N/share/core/peer_extensions.rs:209`,
  `N/agent/core/extensions.rs:162`, `N/vfs/core/cache_extensions.rs:208`); `put_batch` haben auch
  SFTP (`N/sftp/core/extensions.rs`) und SMB (`N/smb/core/backend.rs`).
- Fehlerfall: Einfügen mit „Zusammenführen“ eines Ordners mit 20 000 kleinen neuen Dateien auf ein SFTP-
  oder SMB-Ziel (oder einen Share-Host/Agenten älterer Version). Jede Datei bekommt `alone = true`, keine
  Pakete, kein Ein-Datei-Paket; der gleiche Vorgang lief vor dem Batch in Paketen. Das betrifft auch
  Dateien in Unterordnern, die der Auftrag selbst anlegt (`look` → `Found::Absent`).
- Abweichung: Nutzervorgabe 2026-10-29 („BEST und SCHNELLST“), N1 („senden nur was muss“), K6. Die Regel
  wird damit für genau die Wege langsamer, die die Nutzerin beschleunigt haben will. Bedarf: Zeit nach dem
  Paket setzen (eine gestapelte Setz-Anfrage) oder SFTP/SMB-Pakete die Zeit tragen lassen.

### W5 — Jede über eine Stufe veröffentlichte Datei bekommt eine zusätzliche Anfrage und verliert ihren Stufenbeleg, auch ohne Regel

- Stellen: `N/transfer/os/shared/engine/publish.rs:294-301` (`publish_time::carry` für jede Datei mit
  `mtime_ms > 0`), `:310` (`take_stage_evidence(...).filter(|_| !timed)`), `publish_time.rs:39-62`.
- Fehlerfall: gewöhnliches Hochladen (kein „Zusammenführen“, keine Regel) vieler einzelner Dateien über
  SFTP/FTP/WebDAV-ohne-Zeit-beim-Hochladen/Share-Einzelweg: je Datei ein `finish_stage` (eine Rundreise);
  weil die Zeit gesetzt wurde, wird der Stufenbeleg des Peers verworfen und der Beleg mit eigener Anfrage
  gelesen (Kommentar `publish.rs:306-309`). Mindestens eine, beim Share-Peer mit `evidence_v1` zwei
  Rundreisen mehr je Datei gegenüber 29310ace.
- Abweichung: N1 („senden nur was muss“); C7 verlangt die Quellzeit nur für die Regel, der Plan nennt keine
  Änderung aller Uploads. Bedarf: Zeit im Stufenschreiber oder in der Promotion mitgeben (wie WebDAV/Drive
  `at_upload`), Beleg nach dem Setzen aus der Antwort des Setzens übernehmen.

### W6 — Share-Host-Durchlauf meldet nicht öffnbare Unterordner nur als Auslassung: rekursive Ansicht verliert die Ordnerzeile

- Stellen: `N/share/os/shared/host_hash_walk_local.rs:102-121` (Ordnereintrag erst nach erfolgreichem
  `open_child` und `ensure_local_share_handle_allowed`), Auswertung `N/rscan/os/shared/hash_walk_view.rs:163-167`;
  der Ordnerweg zeigt die Zeile aus der Elternauflistung (`walk_state.rs:144-176`) und meldet danach den
  Lesefehler.
- Fehlerfall: rekursive Ansicht (Strg+R) oder `se ls -R` auf eine Direct-Share-Freigabe mit einem
  Unterordner ohne Leserecht des Host-Prozesses: die Zeile des Ordners fehlt, nur der Fehler steht im
  Bericht. Der Agent-Durchlauf sendet die Ordnerzeile vor dem Lesen (`walk_hashed.rs:265-273`), der
  Share-Host nicht. Im Sync fehlt derselbe Ordner in `Snapshot::dirs` (Auslassung nur über
  `record_listing_omissions`), im Ordnerweg steht er dort (`snapshot_dir.rs:194-199` vor dem Lesefehler).
- Abweichung: D3 „gleiche Zeilen“, K1. Bedarf: Ordnereintrag vor dem Öffnen senden, Öffnungsfehler als
  Auslassung desselben Pfads.

### W7 — Lokale rekursive Ansicht: nur die GUI-Abholung ist geändert, der lokale Scanner nicht

- Stellen: unverändert `N/scanner/os/walk.rs:89-96` (Medienwächter je Ordner, bestand L6) und die
  Fadenobergrenze 4 des lokalen Scanners (bestand B10, L5, L8); geändert nur
  `N/app/os/shared/platform_helpers.rs:12-16`, `:57-104`, `N/app/core/frame_update.rs:147-163`.
- Fehlerfall: lokale rekursive Ansicht über zwei Laufwerke mit vielen kleinen Ordnern (Nutzervorgabe N3
  nennt genau das): Erfassung weiter mit höchstens 4 Fäden und einem Medienwächter je Ordner; die
  Speicheranalyse bleibt auf demselben Baum günstiger je Eintrag (bestand L5/L6). Nicht gemessen (O20).
- Abweichung: N3 („auch lokal ist recursive … davon betroffen“), Nutzervorgabe 2026-10-29; C6 deckt den
  lokalen Scanner nicht ab, ohne Begründung in C8.

### W8 — Gegenseiten-Durchlauf im Sync: Auswertung erst nach dem ganzen Durchlauf (kein Fortschritt, späte Frühkopien, doppelter Speicher)

- Stellen: `N/bisync/os/shared/snapshot_agent.rs:37-93`, `N/bisync/os/shared/snapshot_host_walk.rs:28-37`,
  `:176-224` (`Collector` hält alle `VfsMeta` aller Ordner, `replay` prüft danach seriell und meldet
  `ctx.listed` erst dann).
- Fehlerfall: Standard-Job (`cross_mounts = false`) gegen Direct Share oder SSH-Agent, jetzt immer über den
  Gegenseiten-Durchlauf (C1). (a) Die Fortschrittszahlen der Seite bleiben bis zum Ende des Netz-Durchlaufs
  ohne Dateizahl (`scan_listing` läuft erst in `replay`). (b) Frühkopien (`early_copy::FolderListed`, G4
  MS14), die vorher je fertig gelesenem Ordner starten konnten, starten erst nach dem gesamten Durchlauf.
  (c) Der ganze Baum liegt zweimal im Speicher (Ordnerlisten plus `Tree`), begrenzt nur durch
  `walk_entries`. (d) Die Prüfung aller Ordner läuft danach auf einem Faden, nicht überlappend mit dem Netz.
- Abweichung: N1 („gerne asynchron“), K6. Bedarf: Ordner prüfen, sobald der nächste Ordner gleicher oder
  geringerer Tiefe beginnt bzw. ein Teil vollständig ist (die Gegenseite sendet jeden Ordner vor seinen
  Kindern, aber nicht zusammenhängend), mindestens Fortschritt im `Collector` zählen.

## Gering

- **G1** `N/rscan/os/shared/hash_walk_view.rs:81-83`, `:186-189`: Sendet die Gegenseite einen Eintrag ohne
  Metadaten, setzt `stop_early` den Abbruch-Merker des Scans selbst (`cancel` ist `state.cancel_handle()`),
  dann liefert die Funktion `false`; der folgende Ordnerweg (`rscan.rs:144`) sieht `stopped()` und listet
  nichts. Ergebnis: Ansicht nur mit der Wurzel statt Rückfall. Heute durch die Prüfungen in
  `peer_hash_walk.rs:61` und `agent/core/hash_walk.rs:108` kaum erreichbar; kein Test deckt den Fall.
- **G2** `N/bisync/os/shared/snapshot_host_walk.rs:79-84`, `:132-138`: Der `Collector` zählt Einträge
  unterhalb von Ordnern, die `scan_listing` später schützt (eigene App-Ordner, Systemordner der
  Volumenwurzel), gegen `walk_entries`; der Ordnerweg betritt diese nie. Ein Baum knapp unter der Grenze
  scheitert auf dem neuen Weg („agent walk exceeds its collection budget“) statt zurückzufallen.
- **G3** `N/vfs/os/linux_os/mount_boundaries.rs:104`, `:114-138`: (a) `reload` setzt `stale = false` ohne
  den `polling`-Mutex; meldet ein anderer Faden zwischen Lesen der Tabelle und diesem Rücksetzen eine neue
  Änderung, beantworten weitere Fäden bis zu dessen eigenem Neuladen aus der alten Tabelle (Mount während
  des Durchlaufs kurz unerkannt). (b) Je Ordner bleiben ein `poll` und ein Mutex je Durchlauf; C5 sah den
  Gerätewechsel am Eintrag ohne Systemaufruf vor. Die Abweichung ist im Modulkopf begründet (Bind-Mounts),
  aber nicht im Plan nachgetragen.
- **G4** `N/bisync/os/shared/snapshot_policy.rs:125`: `apptrash::hidden_app_folders_in(parent)` läuft
  weiter je Eintrag (bestand B8/O7); `LocalFacts` hält nur `own_path`, Volumenwurzel, Mounts.
- **G5** `N/agent/core/backend.rs:160`, `N/agent/core/metadata.rs:4-8`: Für Seiten über den
  Hintergrunddienst (`features().service`) bleibt `hidden = false` im Listenweg; dort wirkt
  `include_hidden = false` weiter nicht, wenn das Backend hinter dem Dienst keinen eigenen Durchlauf hat
  (`N/daemon/os/shared/backend_hash.rs:253-262` antwortet `WalkHashed3` dann „nicht unterstützt“).
  Behebung V1 (C3) damit nur für direkte SSH-Agenten. Offen (nicht gelesen, O12): ob Desktop-Läufe Seiten
  über den Dienst öffnen.
- **G6** `N/agent/core/metadata.rs:8`: Punktdateien auf SSH-Agent-Verbindungen sind jetzt im Explorer
  „versteckt“ und gespeicherte Sync-Jobs mit `include_hidden = false` lassen sie auf Agent-Seiten neu aus.
  Gewollt (V1), aber eine sichtbare Verhaltensänderung bestehender Ansichten und Jobs ohne Hinweis in
  `docs/oberflaechen/` oder README.
- **G7** `N/share/os/shared/host_stream.rs:38-43`, `N/share/core/analysis_admission.rs:1`, `:50-58`: Der
  Gegenseiten-Durchlauf hält die Host-Zulassung „ein aktiver Auftrag je Gerät“ für den ganzen Durchlauf. Liegen
  beide Seiten eines Syncs auf demselben Share-Host, läuft Seite B erst nach Seite A; ebenso wartet ein Sync
  hinter einer laufenden Speicheranalyse desselben Geräts. Nicht durch diesen Batch verursacht (der
  Ordnerweg serialisierte je Auflistung), aber eine verbleibende Wartezeit ohne Bedarf.
- **G8** `N/mobile/os/shared/fs_edit.rs:134`, `A/ui/files/FileDialogs.kt:309-314`: Android bietet
  „Neuere ersetzen ältere“ für Namen oberster Ebene nur bei `choosable = all_local`; die Bridge nimmt
  `conflict: "update"` für jede Kombination (`transfer.rs:89`, `:256`). C7 („ausdrückliche Wahl … jeder
  Oberfläche“) für entfernte Kombinationen auf Android nicht erreichbar.
- **G9** `docs/oberflaechen/uebertragungen.md:26-28`: „Pakete an Share- und Agent-Gegenstellen haben im
  Drahtformat keinen Platz für eine Zeit; Dateien der Regel gehen dorthin einzeln“ widerspricht ad100297
  (`batch_put_mtime_v1`, `batch-mtime-v1`, Tag 81) und `docs/TRANSFER_ENGINE.md` Abschnitt „Grenzen“.
- **G10** `docs/oberflaechen/INDEX.md` „Pflege“: `se ls --recursive` (neue Fähigkeit, Gegenstück zur
  rekursiven Ansicht) hat keine Zeile in einem Teildokument; nur README.md:700.
- **G11** `N/transfer/os/shared/engine/test_backend.rs`: von 499 auf 521 Zeilen gewachsen (AGENTS.md
  „native Rust architecture“: unter 500 Zeilen).
- **G12** `N/transfer/os/shared/engine/update.rs:175-179`, `:118-123`: Wiederaufnahme (`RecoveryMode::Resume`)
  einer Datei mit Beleg entscheidet die Regel nicht; liefert `file_recovery::resume` kein Ergebnis, läuft
  die Datei als `Rename`: statt Ersetzen/Überspringen entsteht `Name (2)`.
- **G13** `N/copy/os/linux_os.rs:239-277`, `N/transfer/os/shared/engine/download.rs:105-117`,
  `publish_time.rs`: Jede lokale Kopie unter Linux/Android, jeder Download und jeder Upload trägt jetzt die
  Quellzeit, nicht nur Dateien der Regel. Dokumentiert in `TRANSFER_ENGINE.md`, aber im Plan (C7, D4) nicht
  entschieden; Änderung bestehenden Verhaltens aller Kopien (AGENTS.md „preserve existing behavior“).
- **G14** `N/cli/ls_recursive.rs:74-80`: Unter Windows liefert der lokale Scanner `PathBuf`-Pfade mit `\`
  (`Target::path` ist mit `/` normalisiert, `N/cli/target.rs:398-400`); relative Pfade tieferer Ebenen
  enthalten dann `\`, entfernte Ausgaben `/`. Annahme: der Scanner kanonisiert die Wurzel nicht in die Form
  `\\?\C:\…` (nicht gelesen); träfe das zu, gäbe `relative` den vollen Pfad aus.
- **G15** `N/agent_proto/os/shared/ordered_walk.rs:175-200`: Nach dem Fehler eines Fadens (`pool.fail`) liest
  ein anderer Faden seine lokalen Ordner bis zum Ende weiter und sendet sie, weil die Schleife `done` nicht
  prüft; nur Mehrarbeit, das Ergebnis bleibt ein Fehler.
- **G16** `N/transfer/os/shared/engine/update.rs:248-254` in `batch.rs:82`: `settle` ruft für jeden
  Paketkandidaten `folders.ensure(parent)`; neue Elternordner entstehen so einzeln je Kandidat statt
  gesammelt über `ensure_many` in `batch::run` (`batch.rs:136`).
- **G17** `N/agent_proto/os/shared/walk_hashed.rs:266-268`: Der Agent-Durchlauf lässt Pseudo-Ordner
  (`is_pseudo_dir`, z. B. `/proc`) aus; die rekursive Ansicht über `WalkHashed3` zeigt sie daher nicht, der
  Ordnerweg (`ListDir`) zeigt sie. Abweichung von „gleiche Zeilen“ (D3), inhaltlich erwünscht.

## Geprüft ohne Befund (Auswahl)

- Reihenfolge „Ordner vor Kindern“ im parallelen Durchlauf: Ein Ordner gelangt erst nach `Output::flush`
  des abgebenden Fadens in die gemeinsame Warteschlange (`ordered_walk.rs:193-199`, `:176-187`); der
  Share-Host sendet Container-Einträge vor `local::walk` (`host_hash_walk.rs:284`). Ende und Abbruch:
  `busy`-Zählung, `Enlisted` bei Panik, Leerlauf mit 100-ms-Takt.
- Drahtverträge: `walk-meta-v1`/`WalkHashed3`/`HashPart` (Tags 79/80, Flags, `MIN_ENTRY_BYTES` 22,
  Kreditkosten Tag 80), `batch-mtime-v1` (Tag 81 nur an Server mit Etikett), `hash_walk_meta_v1`,
  `batch_put_mtime_v1`, `transfer-update-v1` mit Code 3; ältere Clients erhalten unveränderte Frames
  (`WalkHashed2` einzeln, `meta` fehlt in JSON), neue Clients fragen ältere Gegenstellen nur nach Fähigkeit.
- Rechte des Share-Hosts im Durchlauf: Wurzel über `access.resolve`, `is_private` an der Wurzel,
  ausgeschlossene Speicherwurzeln je Kind wie `host_list::listing`, `ensure_local_share_handle_allowed` je
  Ordner, Widerruf über `authority.register_cancel` (`host_stream.rs:36`).
- Gleichheit Snapshot/Filter/Auslassungen: `Collector::replay` prüft jeden Ordner mit `scan_listing`;
  Links/Sonderdateien ohne `meta` werden als Einträge mit `is_symlink`/`special` nachgebildet; Mount-Grenze
  entfernt wie im Ordnerweg ungeprüft (C1); `sync_child_path` nur noch für behaltene Einträge (C4).
- `se sync compare`: dieselben Schritte wie `N/app/core/preview_core.rs:24-53`; Text- und JSON-Ausgabe,
  Tresor-Jobs verweisen auf vorhandenes `se vault compare`.
- Dateigrößenregel: alle übrigen geänderten oder neuen Rust-Dateien unter 500 Zeilen und 50 KiB.

## Richtigstellung bei der Behebung

Stand der Behebung: Arbeitsbaum nach `aebf5436` (noch nicht committet). Die Angaben unten sind aus dem Code gelesen; ausgeführt wurde nichts (Projektregel). Der ursprüngliche Text oben bleibt unverändert; die Zeilenangaben „Ausgangsstand“ beziehen sich auf `ad100297`, wie im Review.

### W7 (Richtigstellung)

- Der Medienwächter wurde im Ausgangsstand nicht nur je Ordner, sondern auch je Eintrag aufgerufen. Je Ordner: `N/scanner/os/walk.rs:89-96` (`GuardedIterator::new`). Je Eintrag: `N/source_media/reader.rs:150` (`GuardedIterator::next` ruft `acquire` auf, also `probe`). Diese Eintragsprüfung fehlte im Befund.
- Auf Windows kostet jede `probe` einen 64-KiB-Puffer (`N/source_media/os/windows.rs:30`) sowie `GetVolumePathNameW` und `GetDriveTypeW`. Auf Linux kostet jede `probe` ein `lstat` des Ordnerpfads und eine Sperre des Zwischenspeichers (`N/source_media/probe_cache.rs`).
- Die Fadenobergrenze `.min(4)` (`N/scanner/os/shared.rs:433-436`) führte Commit `44539036` ein. Im Repository steht dafür keine Begründung; die Treffer in `docs/` sind Plandokumente, die den Wert nur zitieren.
- Behebung: Die Geräteermittlung liegt in `N/source_media/walk_device.rs` (neu). Die Wurzel wird einmal geprüft. Ein Ordner übernimmt den Gerätewert seines Elternordners, solange die Gerätekennung gleich bleibt; Linux liest sie je Ordner mit einem `lstat`, Windows braucht keinen Systemaufruf. Eine Probe erfolgt nur bei Gerätewechsel. Ungeschützte Geräte: keine Sperre, keine Probe. Geschützte Geräte: dieselben Prüf- und Sperrschritte wie vorher. Die Prüfung des Wurzelgeräts je Ordner (`N/scanner/os/shared.rs:159-175`, `media_current`) bleibt bestehen; sie gehört zur Garantie.
- Erweiterung nach Auftrag: Dieselbe Stelle wird auch im Ordnerindex (`N/folder_index/os/walk.rs`, Ausgangsstand `:68-79` und `:98`) und in der Auswahl-Erfassung für rekursive Kopien (`N/scanner/os/collect.rs`) genutzt. Die Auswahl-Erfassung bleibt seriell: Die Reihenfolge ihrer Einträge geht in die Zwischenablage ein (`N/app/os/shared/clipboard_lifecycle.rs:283`, Schleife über `collected.entries`), die erste Meldung (`issues.first()`) und die Meldungsgrenze von 100 hängen von der Reihenfolge ab, und die Zugriffsanfrage erfolgt einmal beim ersten fehlgeschlagenen Ordner.

### G14 (Richtigstellung)

- Die Prämisse, relative Pfade tieferer Ebenen enthielten `\`, trifft nicht zu. `path_text` (`N/scanner/os/windows.rs:24-27`) ersetzt jeden `\` durch `/`, bevor der Scanner einen Pfad in einen Eintrag schreibt (`N/scanner/os/walk.rs:211` im Ausgangsstand). Die Ausgabe enthielt deshalb keine Backslashes.
- Die offene Annahme ist beantwortet: Der Scanner kanonisiert die Wurzel. `run_scan` ruft `normalize_scan_root` auf (`N/scanner/os/shared.rs:258` im Ausgangsstand). Unter Windows entsteht dabei die Form `\\?\C:\…` (`N/local_access/os/windows/paths.rs:22`). `display_path` (`paths.rs:135-144`) entfernt den Präfix wieder für den gemeldeten Wurzeltext.
- Der volle Pfad erscheint deshalb nicht durch die Kanonisierung. Der tatsächliche Fehler lag in der CLI: `N/cli/ls_recursive.rs:32` (Ausgangsstand) leitete die Wurzel aus dem Argument ab (`trim_end_matches`), also aus einem anderen Text als dem des Scanners. Die Zuordnung `strip_prefix` (`:74-80` im Ausgangsstand) schlug dann in drei Fällen fehl, und `relative` gab den vollen Pfad aus: bei einem relativen Ziel (zum Beispiel `.`, das `normalize_scan_root` absolut macht), bei einem Kleinbuchstaben im Laufwerk (`paths.rs:53` setzt nur den Laufwerksbuchstaben groß) und bei einem Argument in der Form `\\?\…` oder `//?/…`. Im Normalfall (zum Beispiel `C:/data`) lieferte die CLI die relativen Pfade richtig.
- Behebung: Die CLI übernimmt den Wurzeltext aus dem ersten Eintrag der Tiefe 0 (`N/cli/ls_recursive.rs:43-45`, neue Fassung) und trennt nur an den Grenzen eines Pfads (`relative`, `:84`). Unter Windows wird `\` in der relativen Angabe durch `/` ersetzt.
