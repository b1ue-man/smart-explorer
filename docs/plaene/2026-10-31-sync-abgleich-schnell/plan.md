# Sync-Abgleich so schnell wie die Speicheranalyse (2026-10-31)

Stand: A–D umgesetzt; Nachtrag E (N4, N5) in Umsetzung.

## A — Erstplan aus dem Nutzerauftrag

Nutzernachrichten vom 2026-10-31 (wörtlich):

- N1: „funktioniert gyt, 50% cap kannst entfernen. was ich mich gewundert habe ist, waeum dauert es eigentlixh so viel länger über recursive oder über n vergleich von 2 syncs diese ab zu gleichen, als wenn die soeicheranalyse analysiert was wie virl speicher braucht. das wäre das nächste ziel, das beschlrunigen wo möglich wie möglich. bei syncs über 2 direct shares oder n ssh mit agent wie bei speicheranalysrn, filroperationen wenn möglich lokal halten, gerne asynchron, diese lokal die zum vergleich sammelnden daten wie bearbeitungszeit und gröse oder die prüfschlüss3l, sammeln lassen so schnell und effizient wie möglich und so weiter. kurze wege, senden nur was muss. aso und beim kopieren einfügen, u ter zusammenführen stellte ich mir vor die fehlenden werden eingefügt, neuere vom wann zetzt bearbeitet ersetzen ältere.“
- N2: „gleiche werden überdprungen“
- N3: „oh ich hab mich nicht nur auf remotes bezogen. auch lokal ist revursive, sync zeisvhen z.B. zwei laufwerken davon betroffen. alles was ich nenne sind beispiele keine svopes. scopes ist alle dateioperationen der art.“

Ergebnisanforderungen:

| ID | Ergebnis | Quelle |
|----|----------|--------|
| E1 | Ursache benannt, warum der Abgleich über die rekursive Ansicht und über den Vergleich eines Syncs („Vergleichen“ beider Seiten) so viel länger dauert als die Speicheranalyse | N1 |
| E2 | Der Abgleich ist beschleunigt, wo und wie es geht | N1 |
| E3 | Bei Syncs über zwei Direct Shares oder SSH mit Agent sammelt die Seite, auf der die Dateien liegen, die Vergleichsdaten (Änderungszeit, Größe, Prüfschlüssel) selbst, so schnell und effizient wie möglich, gern asynchron; über die Verbindung geht nur, was nötig ist | N1 |
| E4 | Einfügen mit „Zusammenführen“ fügt fehlende Dateien ein und ersetzt ältere durch neuere (nach Änderungszeit) | N1 |
| E5 | Einfügen mit „Zusammenführen“ überspringt gleiche Dateien | N2 |
| E6 | Die 50-%-Grenze des Wochenlimits entfällt | N1 |
| E7 | Ein Release am Ende (Projektregel AGENTS.md) | AGENTS.md |

Umfang (N3): jede Dateioperation dieser Art, lokal ebenso wie entfernt. Die genannten Fälle (rekursive Ansicht, „Vergleichen“ eines Syncs, Sync über zwei Direct Shares, SSH mit Agent, Sync zwischen zwei Laufwerken) sind Beispiele. Eigene Folgerung, nicht Wortlaut des Nutzers: „dieser Art“ sind alle Wege, die einen Verzeichnisbaum erfassen oder zwei Bäume vergleichen – rekursive Ansicht, Vorschau und Lauf eines Syncs, Vorschau des Einfügens mit „Zusammenführen“, Ordnergrößen, Duplikatsuche, Tresor-Vergleich –, auf jeder Seitenkombination (lokal–lokal, lokal–entfernt, entfernt–entfernt) und jedem Backend (lokal, Direct Share, SSH mit Agent, SFTP, FTP, SMB, WebDAV, Google Drive, Tresor). E3 gilt damit für jede Seite: Die Vergleichsdaten entstehen dort, wo die Dateien liegen, mit dem schnellsten Verfahren, das diese Seite bietet; lokal heißt das der schnelle Scanner der Speicheranalyse.

Grundfunktion (eigene Folgerung): Der Abgleich zweier Bäume braucht je Seite eine vollständige, gefilterte Liste mit Pfad, Art, Größe, Änderungszeit und bei Bedarf Prüfschlüssel; der Vergleich selbst ist Rechenarbeit im Speicher. Jede Wartezeit außerhalb des Erfassens und Übertragens dieser Liste ist Arbeit ohne Bedarf.

Erhaltene Ansichten und Bedienwege: Sync-Fenster mit „Vergleichen“, rekursive Ansicht (Strg+R), Ordnerkonflikt-Dialog „Zielordner entscheiden“ (`app/core/folder_conflict_ui.rs`), Kopierdialog mit Dateikonfliktwahl (`app/core/copy_dialog.rs`), CLI `--conflict`/`--folder-conflict`, Android-Dialoge (`FileDialogs.kt`), MCP über `se`. Die neue Zusammenführungs-Regel kommt in den Ordnerkonflikt-Dialog und in die Dateikonfliktwahl jeder Oberfläche (docs/oberflaechen/uebertragungen.md).

Grenzen und Autorisierung: Projektregeln aus AGENTS.md (keine lokalen Builds/Tests, eine Task-Suite remote, ein Release). Entscheidungen sind an mich delegiert (Nutzer 2026-10-01, 2026-10-29). Bestehendes Verhalten bleibt erhalten (AGENTS.md „preserve existing behavior“): Schutz ausgelassener Links/Junctions, Mount-Grenze, Filter, versteckte Dateien, Sicherungen und Konflikte, Wiederaufnahme.

Promptwissen: Die Speicheranalyse ist auf denselben Verbindungen deutlich schneller als der Abgleich (Beobachtung des Nutzers; Zahlen fehlen).

Fehlende Angaben:
- Durch Bestandslektüre klärbar: welche Wege die rekursive Ansicht, „Vergleichen“ und ein Sync-Lauf für Share- und Agent-Seiten nehmen; was die Speicheranalyse anders macht; was „Zusammenführen“ beim Einfügen heute tut.
- Nur vom Nutzer klärbar: keine. Annahme A1: „recursive“ meint die rekursive Ansicht des Explorers (Strg+R, `rscan`), „vergleich von 2 syncs“ den Knopf „Vergleichen“ eines Sync-Jobs (beide Seiten vergleichen, ohne etwas zu ändern); trifft beides nicht zu, ändert sich nur die Gewichtung, nicht der Weg, weil beide denselben Datenweg nutzen.

Vorläufige Tiefe: 2 (mehrere gekoppelte Komponenten und Drahtverträge).

## B — Bestand

Vollständiger Bestand mit Datei:Zeile: [bestand-datenwege.md](bestand-datenwege.md) (Code-Stand 29310ace, nicht gemessen). Zusammenfassung der Ursachen, die die Speicheranalyse nicht hat:

Entfernt (Direct Share, SSH mit Agent, SFTP):
- B1 Die Speicheranalyse stellt eine Anfrage für den ganzen Baum; die Gegenseite erfasst lokal (Share `FsStorageAnalysis`, Agent `WalkTree`; bestand 7.1).
- B2 Der Sync-Snapshot hätte mit `walk_snapshot_via_agent` denselben Weg (Share `hash_walk_v1`, Agent `WalkHashed2`), schaltet ihn aber bei `cross_mounts = false` ab – der Voreinstellung jedes Sync-Jobs (`syncjobs/core/types.rs:227`, `snapshot_agent.rs:26`). Begründung im Code: das Protokoll trägt die Mount-Grenze nicht. Belegt: Der Ordner-für-Ordner-Weg prüft die Mount-Grenze auf entfernten Seiten ebenfalls nicht (`snapshot_policy.rs:98` nur `is_local()`; bestand 3.6, V2). Das Abschalten kostet also Zeit, ohne ein anderes Ergebnis zu liefern.
- B3 Im Ordner-für-Ordner-Weg: je Ordner `sync_stat` + Listing unter Flow-Permit (Grenze Flow+1, Start 8), je Share-Eintrag eine eigene Rundreise `SyncChildPath`, auch für gefilterte Einträge (`snapshot_dir.rs:120` vor dem Filter `:141`; bestand 3.2, R1).
- B4 Die Durchläufe auf der Gegenseite sind seriell (Agent `ext_ops.rs:150-259`, Share-Host `host_hash_walk.rs:150-276`); der Share-Host öffnet jede Datei auch ohne angeforderten Hash und fragt zweimal Metadaten ab (`host_hash_walk.rs:240-249`).
- B5 Rekursive Ansicht entfernt: Blöcke von `parallelism` Ordnern mit Barriere je Block und Ebene (`rscan/os/shared/parallel.rs:24-62`); kein Durchlauf auf der Gegenseite.
- B6 Prüfschlüssel: im Standardmodus `MtimeSize` keine; im Modus `Checksum` ohne Host-Durchlauf je Datei der vollständige Download plus vier Metadatenanfragen (bestand 3.4).

Lokal (Sync zwischen zwei Laufwerken, lokal gegen entfernt):
- B7 Je lokalem Ordner im Sync unter Linux: `canonicalize(parent)` (ein Systemaufruf je Pfadkomponente), Neueinlesen und Parsen von `/proc/self/mountinfo` (`vfs/os/linux_os/local_platform.rs:100-113`), dazu `stat` von `nested-mounts.json` unter einem globalen Mutex, den alle Lese-Threads beider Seiten teilen (`snapshot_mounts.rs:51-55`). Windows: `mount_boundary` ist leer (`vfs/os/windows/local_platform.rs:206`), der Mutex mit `stat` bleibt.
- B8 Je lokalem Eintrag: `own_path` liest fünf Umgebungsangaben und sperrt einen Mutex (`snapshot_policy.rs:6-64`), dazu `apptrash::hidden_app_folders_in(parent)` (Kosten ungelesen).
- B9 Die lokale Auflistung selbst ist dieselbe wie in der Analyse (`vfs/os/shared/local.rs:120-152` → `local_access::read_directory`).
- B10 Rekursive Ansicht lokal: jeder Eintrag als `FileEntry` an die GUI, höchstens 4 Threads, je Ordner ein Medienwächter (bestand 6.2, L5, L6).

Gemeinsam:
- B11 Der Zwei-Richtungs-Standardjob liest bei jedem Lauf und jeder Vorschau beide Seiten vollständig; inkrementell laufen nur einseitige Spiegel (bestand 8).
- B12 Einfügen-Vorschau „Zusammenführen“: seriell über den Quellbaum, je Quelldatei ein `stat` am Ziel (`transfer/os/shared/engine/selection.rs:114-224`).
- B13 Dateikonflikt beim Zusammenführen: feste Wahl `Conflict::{Rename, Overwrite, Skip}` des Auftrags (`types/core/types.rs:123`, Anzeige `app/core/folder_conflict_ui.rs:86-88`); kein Vergleich von Zeit oder Größe.

Zu erhaltendes Verhalten (bestand 11.3): Auslassungen von Links/Sonderdateien, Mount-Grenze lokal, versteckte Dateien (Agent liefert `hidden = false`, `agent/core/metadata.rs:13`), Abbruch der entfernten rekursiven Ansicht bei Auslassungen (V4, zu prüfen, ob gewollt).

## C — Entscheidungen

Grundlage: [bestand-datenwege.md](bestand-datenwege.md), [referenzen-abgleich.md](referenzen-abgleich.md) (Abschnitte 4–6). Geordnet nach belegtem Beitrag (referenzen 5).

C1 Seitenlokale Erfassung (referenzen 5 Platz 1, rsync `flist.c`, Syncthing Index, Mutagen-Agent): Jede entfernte Sync-Seite mit Gegenseiten-Durchlauf (Share `hash_walk_v1`, Agent `ext-v1`) nutzt ihn, auch bei `cross_mounts = false`. Begründung: Der Ordner-für-Ordner-Weg prüft die Mount-Grenze auf entfernten Seiten ebenfalls nicht (B2), das Ergebnis bleibt also gleich. Verworfen: Mount-Grenze neu auf entfernten Seiten durchsetzen – ändert Ergebnisse bestehender Jobs (Teilbäume würden ausgelassen), ohne dass der Nutzer es verlangt hat.
C2 Gegenseiten-Durchlauf parallel ohne Ebenenbarriere (Platz 4, ignore-Crate `walk.rs:1653-1670`, dua `lib.rs:761`): Share-Host und se-agent durchlaufen mit Arbeitsklau; Fadenzahl aus der verfügbaren Parallelität des Hosts zur Laufzeit (wie der lokale Sync-Durchlauf, `vfs/core/core.rs:261`), kein fester Wert. Ohne angeforderten Prüfschlüssel öffnet der Share-Host keine Datei; Größe und Zeit kommen aus der handle-gebundenen Verzeichnisauflistung (wie die Analyse).
C3 Metadaten im Gegenseiten-Durchlauf (neue Fähigkeit `hash_walk_meta_v1` Share, `walk-meta-v1` Agent): Einträge tragen auf Anfrage `hidden`, `system`, `btime_ms`, `is_symlink`, `special` und melden Links/Sonderdateien als Einträge statt als Auslassungen. Damit tragen ihn auch `include_hidden = false` (bisher Abwahl) und die rekursive Ansicht. Ältere Gegenstellen: bisheriger Weg. Agent-Listing setzt `hidden` nach Punktnamen wie SFTP (`sftp/core/metadata.rs:28`) – Behebung von V1 (bestand 11.3), weil `include_hidden = false` auf Agent-Seiten sonst wirkungslos ist.
C4 Ordner-für-Ordner-Weg (ältere Gegenstellen, SFTP ohne Agent, FTP, SMB, WebDAV, Drive): `sync_child_path` erst nach Filter und Ausschluss (R1); `sync_stat` je Ordner bleibt (Schutz gegen Austausch des Ordners durch einen Link, Garantie).
C5 Lokaler Sync-Durchlauf (B7, B8): Mount-Grenze ohne Systemaufruf je Ordner – Mounttabelle einmal je Durchlauf, Wurzel einmal kanonisch, neue Mounts während des Durchlaufs über den Gerätewechsel am Eintrag (rsync `-x`, `find -xdev`); `nested-mounts.json` einmal je Durchlauf geladen, kein globaler Mutex je Ordner; eigene App-Ordner (`own_path`) und Volumenwurzel einmal je Durchlauf. Ergebnis und Auslassungen unverändert. Nachtrag 2026-10-31 (Review G3): umgesetzt ist statt des Gerätewechsels am Eintrag ein `poll` auf das offene `/proc/self/mountinfo` je Ordner (kein Lesen der Tabelle, kein Zugriff auf das Kind; `POLLPRI|POLLERR` nach jedem Ein- und Aushängen im Namensraum, proc(5)). Grund: Ein Bind-Mount (und ein Mount desselben Dateisystems an anderer Stelle) liegt auf demselben Gerät wie sein Elternordner, `st_dev` wechselt dort nicht; der Gerätewechsel fände ihn nicht, `mount_boundary` (Mounttabelle) findet ihn. Ergebnis damit gleich dem bisherigen Ordnerweg, auch für Bind-Mounts und autofs-Auslöser. Kein Fenster mit alter Tabelle: Änderungen werden nur unter der Lesesperre gemeldet, das Neuladen leert die Meldung unter der Schreibsperre vor dem Lesen der Tabelle (`vfs/os/linux_os/mount_boundaries.rs`, `reload`).
C6 Rekursive Ansicht (B5, B10): entfernt Arbeitsklau statt Block-/Ebenenbarriere; Share/Agent mit C3 über eine Anfrage; GUI holt je Bild alle anstehenden Scan-Nachrichten (Zeitbudget statt 64 Nachrichten) und baut die Ansicht während des Laufs nur so oft neu auf, dass der Neuaufbau höchstens einen kleinen Teil der Bildzeit belegt (Abstand aus der gemessenen Dauer des letzten Neuaufbaus).
C7 Zusammenführen beim Einfügen (E4, E5; referenzen 4.3 M1–M6): neue Dateiregel „Neuere ersetzen ältere“ (`Conflict::Update`):
  - fehlt am Ziel → einfügen;
  - gleich = gleiche Größe und Zeitabstand ≤ Fenster → überspringen (M1, M6);
  - Quelle um mehr als das Fenster neuer → ersetzen, mit dem bestehenden Schutz des Überschreibens (M5);
  - Ziel um mehr als das Fenster neuer → Ziel behalten, als „Ziel neuer“ gemeldet;
  - gleiche Zeit im Fenster, andere Größe → beide behalten (M4, FreeFileSync-Kategorie „Konflikt“), gemeldet;
  - Fenster = gröbere Zeitgenauigkeit beider Seiten (rclone `fs.go:100-115`), unbekannt → 2 s.
  - Zeit einer Seite unbekannt (0) → Konflikt, beide behalten, auch bei gleicher Größe (Größe beweist keine Gleichheit; Review W1, M1/M6). Ebenso in der Vorschau.
  - Ersetzen nur, wenn das Ziel noch ist, was die Entscheidung sah: unmittelbar vor der Veröffentlichung Größe und Zeit des Ziels erneut mit dem Stand der Entscheidung vergleichen, bei Abweichung beide behalten (Review B1, M5); gilt entfernt (`promote_staged_to_id`), lokal und beim Download.
  - Ein ausgelassener Zielname (tolerante Auflistung) gilt als vorhanden und unbekannt → beide behalten; die übrigen Dateien des Ordners laufen normal (Review W3).
  - Quellzeit auf allen Kopien (Entscheidung des Koordinators 2026-10-31, Review W5/G13): bleibt, weil ein Zusammenführen zurück sonst jede Kopie ersetzt (Windows `CopyFile2` tat es schon); aber ohne zusätzliche Rundreise je Datei gegenüber v0.5.189, wo das Protokoll die Zeit im Schreib- oder Veröffentlichungsschritt tragen kann (WebDAV/Drive beim Hochladen, SFTP als `FSETSTAT` auf dem offenen Handle, SMB im Abschluss-Compound, Pakete mit der Zeit je Datei). Wo jedes Protokoll eine eigene Anfrage verlangt (FTP `MFMT`, Share-/Agent-Stufen größerer Dateien, SMB-Stufen über einem WRITE, SFTP ohne Pool-Kanal), steht das als Grenze in `docs/TRANSFER_ENGINE.md`.
  Gilt für Dateien in zusammengeführten Ordnern beim Einfügen (Desktop, Android) als Vorgabe; als ausdrückliche Wahl im Kopierdialog, in der CLI (`--conflict update`), im MCP und in der Daemon-/Job-Persistenz. Die Vorschau zählt neu/neuer/gleich/Ziel neuer/Konflikt über eine Zielauflistung je Ordner statt eines `stat` je Datei (B12). Verworfen: Sommerzeit-Verschiebung (FreeFileSync) – braucht eine Einstellung, die der Nutzer nicht verlangt hat; bleibt als Grenze im Bericht.
C8 Zurückgestellt mit Grund:
  - Zwei-Richtungs-Folgeläufe ohne Vollerfassung (Platz 5, Dateisystembeobachtung): verlangt einen neuen Zwei-Richtungs-Index samt Vertrauensmodell für Beobachtungsereignisse (X10: Ereignisse gehen verloren); berührt die Löschsicherheit des Syncs; eigener Batch.
  - Persistenter Hash-Cache mit Datei-Kennung (Platz 3): wirkt nur im Modus `Checksum`; mit C1 rechnet die Gegenseite die Hashes lokal.
  - MFT/USN (Platz 11, X9): Administratorrecht, nur NTFS.
  - io_uring (Platz 12, X7): keine Verzeichnisoperation.

## D — Arbeitsplan

| Paket | Ergebnis (prüfbar) | Dateien |
|---|---|---|
| D1 Gegenseiten-Durchlauf | Share-Host und se-agent liefern denselben Eintrags- und Auslassungssatz wie bisher, parallel; ohne Algorithmus keine Dateiöffnung; mit `meta` die Felder aus C3; alte Gegenstellen unverändert; Agent-`hidden` nach Punktnamen | `share/os/shared/host_hash_walk.rs`, `share/core/{hash_walk_wire,fs_request,fs_response,wire_capabilities,peer_hash_walk}.rs`, `vfs/core/extension_types.rs`, `agent_proto/os/shared/ext_ops.rs`, `agent_proto/core/{server,features,frame_*}.rs`, `agent/core/{extensions,search,metadata}.rs`, `daemon/os/shared/backend_hash.rs` |
| D2 Sync-Snapshot | entfernte Seiten mit Gegenseiten-Durchlauf bei `cross_mounts = false` und (mit `meta`) bei `include_hidden = false`; `sync_child_path` nur für behaltene Einträge; lokal keine Mounttabelle/kein `canonicalize`/kein globaler Mutex je Ordner; gleiche Snapshots und Auslassungen | `bisync/os/shared/{snapshot,snapshot_agent,snapshot_dir,snapshot_policy,snapshot_mounts,apply_path_context}.rs`, `vfs/os/linux_os/{local_platform,mountinfo}.rs`, `vfs/os/shared/local_extensions.rs` |
| D3 Rekursive Ansicht | entfernt ohne Barriere, Share/Agent über eine Anfrage; GUI ohne Rückstau; gleiche Zeilen | `rscan/os/shared/*.rs`, `app/os/shared/platform_helpers.rs`, `app/core/{drains_connect,frame_update,view_selection}.rs` |
| D4 Zusammenführen | Regel C7 auf jedem Kopierweg (lokal, Download, Upload, entfernt–entfernt, Pakete) und jeder Oberfläche; Vorschau mit Zielauflistung | `types/core/types.rs`, `transfer/os/shared/engine/*`, `copy/os/shared/*`, `daemon/os/shared/transfer_job_wire.rs`, `cli/*`, `cli/mcp/*`, `mobile/os/shared/transfer.rs`, `android/.../ui/files/*`, `android/.../api/FilesApi.kt`, `app/core/{copy_dialog,folder_conflict_ui,transfer_route}.rs`, `docs/oberflaechen/uebertragungen.md` |
| D5 Task-Suite | eine Remote-Suite deckt D1–D4 und die erhaltenen Verträge ab, mit Messung der Wege (lokal–lokal, Share-Schleife, Agent über sshd) vorher/nachher | `.github/workflows/`, `native/test-*.sh` |
| D6 Gesamtreview, Suite-Lauf, Release | ein frischer Opus-Review vor dem Suite-Lauf; Suite grün; ein Release | – |

Messplan (Analyse-Regel): Wege lokal–lokal (ein Laufwerk, zwei Verzeichnisse), Share über Schleife (Host im CI-Runner), Agent über sshd im CI-Runner, SFTP ohne Agent; je Weg „Vergleichen“ und rekursive Ansicht auf einem erzeugten Baum (Annahme: 100 000 Dateien in 10 000 Ordnern, weil der Nutzer keine Last genannt hat), kalt und warm getrennt, vorher (v0.5.189) und nachher. Lokale Messung auf diesem Rechner ist durch AGENTS.md ausgeschlossen (keine lokalen Tests); die Messung läuft in der Remote-Suite.

## Schleifenstand (kreative Schleife)

- Ziel: E2/E3 (N1, N3) – Wege, auf denen jede Seite ihre Vergleichsdaten selbst und ohne Arbeit ohne Bedarf erfasst; E4/E5 (N1, N2) – Regel für fehlend/neuer/gleich.
- Kriterien: K1 gleiches Ergebnis wie bisher (Baum, Auslassungen, Filter, Mount-Grenze) – offen bis Suite; K2 eine Anfrage je entfernter Seite statt je Ordner – entworfen (C1–C3); K3 nur nötige Felder über die Verbindung – entworfen (C2, C3); K4 alte Gegenstellen nutzbar – entworfen (Fähigkeiten); K5 Zusammenführen nach N1/N2 – entworfen (C7); K6 lokale Arbeit je Ordner/Eintrag ohne Bedarf entfernt – entworfen (C5, C6).
- Möglichkeiten (Runde 1, Befund: bestand + referenzen): seitenlokale Erfassung (gewählt), Metadatenvergleich vor Inhalt (vorhanden, `MtimeSize`), persistenter Cache (zurückgestellt), Arbeitsklau (gewählt), Beobachtung für Folgeläufe (zurückgestellt), MFT/io_uring (ausgeschlossen).
- Aktiver Stand: C1–C7 gewählt; Umsetzung D1–D4.
- Nächster Schritt: Ausarbeiten – D1–D4 umsetzen, D5 Suite, Wirkung durch Messung belegen; Rückkehrpunkt: Bewertung an K1–K6 nach dem Suite-Lauf.
- Status: aktiv.


## E — Nachtrag nach dem ersten Abschluss (2026-10-31)

Nutzernachrichten (wörtlich):
- N4: „du hast noch offene punkte also bist nicht fertig. meine operation die mit einem hash gemacht werden kann der lokal erstellt werden kann darf einen filetransfer verlangen.“ (eigene Lesart: „meine“ = „keine“)
- N5: „und natürlich wie immer, so schnell wie möglich heist so wenig wie möglichm wenn n hash von nem ordner gleich ist, fertig. ordner ist gleich. das nehme ich zumindest leihenhaft an validiere“

Ergebnisse:
| ID | Ergebnis | Quelle |
|----|----------|--------|
| E8 | jeder bisher als offen/zurückgestellt geführte Punkt ist umgesetzt oder trägt einen zulässigen Grund (Quellstelle, Messung, Regel, Garantie, Nutzervorgabe) | N4 |
| E9 | keine Operation, deren Prüfschlüssel die Seite mit der Datei selbst bilden kann, überträgt die Datei (Prüfsummen-Vergleich im Sync, Überprüfen nach Kopien, Zusammenführen, Duplikate) | N4 |
| E10 | Ordner-Prüfschlüssel: gleicher Schlüssel beider Seiten → Ordner gleich, kein Abstieg, keine Auflistung über die Verbindung; nur ungleiche Ordner werden weiter verglichen | N5 |
| E11 | Validierung der Annahme „gleicher Ordner-Hash = gleicher Ordner“ mit Primärquellen | N5 |

Validierung (E11, Stand vor der Umsetzung): gilt in Richtung „gleich ⇒ gleich“, wenn der Schlüssel aus einer kollisionsresistenten Hashfunktion über eine kanonische Darstellung aller verglichenen Angaben entsteht (Name in der Faltung des Paars, Art, Größe, Zeit auf die gröbere Genauigkeit beider Seiten gerundet, im Prüfsummen-Modus der Inhalts-Hash, Filter, Mount-Grenze, Auslassungen) – das Verfahren von Git-Bäumen und dem Merkle-Abgleich in Dynamo/Cassandra (Primärquellen ergänzt das Umsetzungspaket). Die Umkehrung gilt nicht (gerundete Zeiten an Rundungsgrenzen, Sommerzeit, Faltung): ein ungleicher Schlüssel führt nur zum Abstieg und zum bisherigen Vergleich. Im Zwei-Richtungs-Sync zählt zusätzlich der Schlüssel der Basis: A = B = Basis → Teilbaum unverändert, aus der Basis übernommen; A = B ≠ Basis → Einträge einer Seite für die Basis.

Offene Punkte aus Plan, Berichten und Review (E8), mit Entscheidung:
| Punkt | Entscheidung |
|---|---|
| Zwei-Richtungs-Folgeläufe ohne Vollerfassung (C8) | umsetzen über E10 mit Basis-Ordnerschlüsseln: unveränderte Teilbäume ohne Auflistung über die Verbindung |
| persistenter Hash-Cache (C8) | umsetzen auf der Seite der Datei (Share-Host, se-agent, lokal), Schlüssel Dateikennung + Größe + Zeit, Regel gegen „racy“ Zeitstempel (Git) |
| Sommerzeit FAT/exFAT (C7) | umsetzen: Seiten mit Ortszeit-Dateisystem vergleichen eine Abweichung von genau einer Stunde (± Fenster) bei gleicher Größe als gleich |
| G5 Rest (Explorer-Listen über den Dienst ohne `hidden`) | umsetzen: Frames mit Merkmal hinter Fähigkeit |
| W8 Rest (leere und nicht geöffnete Ordner erst am Ende) | umsetzen: Ordner-Ende-Marke im `meta`-Strom |
| Agent-Listing ohne Erstellzeit | umsetzen hinter Fähigkeit |
| Kopierauswahl seriell (collect.rs) | umsetzen: geordnete Pipeline (parallel listen, in bisheriger Reihenfolge verarbeiten) |
| Android `scan.view` hält die Sperre beim Baumaufbau | umsetzen |
| Drive-Einschritt-Uploads ohne Zeit | umsetzen (`modifiedTime` beim Anlegen) |
| MFMT bei FTP (eigene Anfrage) | bleibt: Protokoll (RFC 3659 bietet keine Zeit im STOR) |
| Fenster zwischen Prüfung und Umbenennen ohne Objekt-ID | bleibt: Protokoll (SFTP/SMB/FTP/WebDAV-Rename ohne Vorbedingung) |
| Host-Zulassung „ein Auftrag je Gerät“ (G7) | bleibt: Garantie (Schutz des Hosts) |
| share/mod.rs > 500 Zeilen | bleibt: Regel (AGENTS.md verlangt das Herauslösen nur vor neuer bedeutsamer Logik; dort stehen nur Moduldeklarationen) |
| Prüfschlüssel bei SMB-Zielen | bleibt Übertragung: Protokoll (SMB 2/3 kennt keine serverseitige Prüfsumme) |
| Prüfschlüssel verschlüsselter Tresor-Inhalte | bleibt Übertragung: Garantie (der Host hat keinen Schlüssel für den Klartext) |

Pakete: F4 (opus) Ordnerschlüssel und Basis-Schlüssel im Sync, Hash-Cache auf Share-Host/Agent/lokal, Hash im Sync über die Gegenseite statt Download, Ordner-Ende-Marke, G5-Rest, Agent-Erstellzeit; F5 (sonnet) Prüfschlüssel ohne Übertragung bei SFTP ohne Agent, FTP, WebDAV, Drive und beim Überprüfen nach Kopien, Sommerzeit-Regel, Drive-Zeit beim Anlegen; F6 (haiku) geordnete Kopierauswahl, Android-Sperre; danach F7 Zusammenführen mit Ordnerschlüsseln und Erweiterung der Suite.

### E11 — Validierung „gleicher Ordner-Hash = gleicher Ordner“ (Primärquellen, abgerufen 2026-10-31)

| Quelle | Stelle | Aussage |
|---|---|---|
| Git-Buch, „Git Internals – Git Objects“, https://git-scm.com/book/en/v2/Git-Internals-Git-Objects | Abschnitt „Tree Objects“ | „A single tree object contains one or more entries, each of which is the SHA-1 hash of a blob or subtree with its associated mode, type, and filename.“ Ein Unterordner ist ein Eintrag mit dem Hash eines weiteren Trees (`040000 tree … lib`). |
| DeCandia u. a., „Dynamo: Amazon’s Highly Available Key-value Store“, SOSP 2007, https://www.allthingsdistributed.com/files/amazon-dynamo-sosp2007.pdf | Abschnitt 4.7 „Handling permanent failures: Replica synchronization“ | „if the hash values of the root of two trees are equal, then the values of the leaf nodes in the tree are equal and the nodes require no synchronization. If not, … the nodes may exchange the hash values of children and the process continues until it reaches the leaves“; Merkle-Bäume „minimize the amount of data that needs to be transferred for synchronization“. |
| Apache Cassandra, Dokumentation „Repair“, https://cassandra.apache.org/doc/latest/cassandra/managing/operating/repair.html | Einleitung der Seite „Repair“ | Die Reparatur „compares the data with merkle trees, which are a hierarchy of hashes“ und überträgt nur „the differences for any out of sync sections“. |
| Git, „racy-git“, https://git-scm.com/docs/racy-git | ganze Seite (Abschnitte zum Problem und zur Abhilfe) | Ein Eintrag, dessen `st_mtime` gleich oder neuer als der Zeitpunkt der Aufzeichnung ist, ist „racily clean“ und wird inhaltlich nachgeprüft (Grundlage der Regel des Hash-Caches, E9). |
| Microsoft, „exFAT file system specification“, https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification | Abschnitt 7.4.10 „UtcOffset Fields“, 7.4.6 | Zeiten sind Ortszeit mit einem Feld für den Abstand zu UTC, das „daylight saving and regional summer time changes“ einschließt (Grundlage, exFAT nicht als Ortszeit-Seite der Sommerzeit-Regel zu führen). |

Ergebnis der Validierung:
- Richtung, die gilt: gleicher Schlüssel ⇒ gleicher Ordner für den Vergleich. Voraussetzungen (alle erfüllt, Codestellen): kollisionsresistente Hashfunktion (SHA-256, `native/src/agent_proto/core/sha256.rs`, FIPS 180-4); kanonische Darstellung ohne Mehrdeutigkeit (jede Angabe mit Länge oder fester Breite, Einträge nach ihrer Kodierung sortiert, Bereichstrenner `se-folder-run-v1`/`se-folder-key-v1`, `native/src/agent_proto/core/folder_key.rs`); der Schlüssel deckt genau die Angaben ab, die der Vergleich aus einem Ordner nimmt (Name exakt, Art, Größe und Zeit in ms und Prüfschlüssel bei Dateien, versteckt, Auslassungen mit Grund, eigenes Scheitern, Schlüssel aller durchlaufenen Unterordner); gesendete Einträge werden auf genau diese Angaben verkürzt, bevor der Vergleich sie sieht (`Part::item`, `native/src/bisync/os/shared/folder_key_cache.rs`), also ist der Vergleich eine Funktion des Schlüsselinhalts. Filter, Mount-Grenze, Schutzregeln, eigene Ordner und die Faltung des Paars wirken weiter auf der Client-Seite über genau diese Angaben; sie müssen deshalb nicht in den Schlüssel.
- Abweichung vom Auftragstext: Namen exakt statt in der Faltung des Paars und Zeiten in ms statt gerundet. Grund (Garantie „Ergebnis gleich dem vollständigen Vergleich“): Ein gleicher Schlüssel ersetzt die Einträge; bei gefalteten Namen oder gerundeten Zeiten bekäme die Seite Schreibweise und Zeit der anderen Seite in ihren Schnappschuss und in die Basis. Die strengere Gleichheit kostet nur Abstiege, nie ein anderes Ergebnis.
- Richtung, die nicht gilt: ungleicher Schlüssel ⇒ nur „ansehen“. Ein ungleicher Ordner kommt ganz über die Verbindung und läuft durch den bisherigen Vergleich (Zeitfenster, Sommerzeit, Faltung).
- Basis-Fälle: Gespeichert werden je Seite die Ordnerschlüssel ihres letzten vollständigen Durchlaufs mit ihrem Inhalt, inhaltsadressiert (`folder-keys/<Seite>.sefk` im Sync-Datenordner). Seite = gespeichert (A = Basis, B = Basis, also auch A = B = Basis) → keine Auflistung, keine Einträge über die Verbindung; die Einträge kommen aus der Datei. Seite gleich dem, was die andere Seite gespeichert hat (A = B ≠ Basis, z. B. nach einer Kopie im letzten Lauf) → ebenfalls keine Übertragung, weil die Schlüssel beider Seiten als bekannt angeboten werden. Sonst wie bisher. Die Basis des Syncs (`baseline_*.sebl`) bleibt unverändert; Aktionen, Konflikte, Auslassungen, Basis und Löschschutz entstehen aus demselben Schnappschuss wie ohne Schlüssel.

### F4 — Umsetzung (Quellstand 2026-10-31, nicht gebaut, nicht ausgeführt)

| Ergebnis | Umsetzung |
|---|---|
| E10 Schlüssel am Ort der Dateien | `agent_proto/core/folder_key.rs` (Kodierung, `KeyTree`: Ordner fertig, wenn sein Lauf und alle durchlaufenen Unterordner fertig sind; bekannter Schlüssel → eine Marke statt des Teilbaums, gepufferte Läufe höchstens 32 MiB, darüber gehen sie unverkürzt); se-agent `WalkKeyed` (Tag 85, Antworten `FolderRun` 89, `HashPart` 80, `FolderKey` 86; Etikett `walk-keys-v1`; `agent_proto/os/shared/walk_keyed.rs`, `walk_hashed.rs`); Share-Host `hash_walk_keys_v1` (`FsHashWalk.keys/known`, Meldungen `Run`, `Key`; `share/os/shared/host_hash_walk_keys.rs`); Hintergrunddienst reicht `WalkKeyed` an den Durchlauf seines Backends weiter (`daemon/os/shared/backend_hash_keyed.rs`); Client `vfs::hash_walk_keyed` (`agent/core/hash_walk.rs`, `share/core/peer_hash_walk.rs`, `vfs/core/cache_extensions.rs`) |
| E10 Vergleich | `bisync/os/shared/snapshot_keyed.rs` (zuerst für jede entfernte Seite; bekannte Schlüssel = gespeicherte Schlüssel dieser und der anderen Seite, die mit den meisten Ordnern zuerst; Share nimmt 10 240, der Agent 262 144 je Anfrage), `folder_key_cache.rs`, `folder_key_file.rs`; ältere Gegenstellen und Dienste ohne eigenen Durchlauf: bisheriger Weg über `meta` |
| E9 | Hash-Cache `agent_proto/os/shared/hash_cache.rs` (Gerät, Inode, Größe, Änderungs- und Statusänderungszeit in ns; Racy-Regel 2 s; 500 000 Einträge; Agent `$XDG_CACHE_HOME/se-agent` bzw. `~/.cache/se-agent`, App im App-Datenordner); genutzt im Agent-Durchlauf (MD5), in `FILE_DIGEST` (SHA-256), im Share-Host-Durchlauf (lokale Exporte) und lokal im Sync (`snapshot_hash::hash_file_cached`, `orchestration_plan::hash_checked`); `vfs::content_md5` als Weg für eine MD5 am Ort der Datei vor jedem Download in `snapshot_dir::content_hash`; Agent ohne `FILE_DIGEST` gibt `content_digest`/`content_md5` an seine SSH-Verbindung weiter |
| W8 | Laufmarken im Schlüssel-Durchlauf: ein leerer Ordner ist bei der nächsten Marke fertig; ein nicht geöffneter Ordner sofort mit dem Ende des Elternlaufs bzw. mit seiner Fehlermeldung (`snapshot_host_stream.rs`) |
| G5-Rest, Agent-Erstellzeit | `ListDirMeta` (87), `StatMeta` (88), beim Agenten auch `ListMeta`, Etikett `meta-list-v1` (`agent_proto/os/shared/meta_list.rs`, Dienst `daemon/os/shared/backend_hash.rs`, Client `agent/core/meta_list.rs`) |
| Sommerzeit | `bisync/core/compare.rs` `TimeRules::across`, `same_time_as` über `transfer::time_window::dst_shift_equal`; gilt, wenn eine der beiden Seiten FAT ist, unabhängig von der Genauigkeit der anderen |
| Tests | `abgleich_task_` in `agent_proto/core/folder_key_tests.rs`, `walk_key_frames_tests.rs`, `agent_proto/os/shared/hash_cache_tests.rs`, `agent/core/abgleich_task_keys_tests.rs`, `share/core/abgleich_task_folder_keys_tests.rs`, `bisync/os/shared/snapshot_keyed_tests.rs`, `bisync/core/abgleich_task_compare_tests.rs`, `daemon/os/shared/backend_abgleich_task_tests.rs` |

Nachtrag F4b (2026-10-31, nach „du hast noch offene punkte also bist nicht fertig“):
| Punkt | Umsetzung |
|---|---|
| Erstvergleich ohne gespeicherte Schlüssel | Zwei Phasen, beide Seiten gleichzeitig (`bisync/os/shared/snapshot_pair_keys.rs`, `snapshot_pair_side.rs`): Phase 1 nur die Schlüssel (`KeyedWalk::KeysOnly`; Agent `WalkKeyed` mit Optionsbyte, Share `FsHashWalk.keys_only`); der Client vergleicht die Schlüsselbäume beider Seiten mit den gespeicherten und den lokalen Schlüsseln und nennt je Seite die Ordner, deren Inhalt noch niemand hält, jeden Schlüssel einmal, verteilt auf die Seite mit weniger Ordnern; Phase 2 nur deren Läufe (`KeyedWalk::Runs`; Agent `ListRuns` Tag 90, Share `FsHashWalk.folders`), wieder gleichzeitig; jede Seite spielt ihren Baum aus allen Aufzeichnungen in die Prüfung ein. Ein zwischen den Phasen geänderter Ordner passt nicht mehr zu seinem Schlüssel; die Seite nimmt dann den Durchlauf mit bekannten Schlüsseln. Gilt, wenn eine teilnehmende entfernte Seite noch keine Schlüssel gespeichert hat; sonst reicht der Durchlauf mit bekannten Schlüsseln (eine Marke für die Wurzel). |
| Lokale Seiten | bilden Schlüssel und Läufe mit dem Durchlauf des Agenten im eigenen Prozess (`agent_proto::local_keyed`), gleichzeitig mit ihrem eigenen Durchlauf; gleiche Ordner der entfernten Gegenseite werden nicht übertragen. |
| IPC-Weg des Dienstes | `daemon/os/shared/ipc_backend_extensions.rs` reicht `hash_walk_keyed` (alle drei Arten) und `content_md5` an die Worker-Verbindung weiter; der Dienst reicht `WalkKeyed` und `ListRuns` an den Durchlauf seines Backends weiter (`backend_hash_keyed.rs`). |
| Hash-Cache unter Windows | Kennung aus `GetFileInformationByHandleEx`: `FileIdInfo` (Volumen-Seriennummer, 128-Bit-Datei-ID) und `FileBasicInfo` (`LastWriteTime`, `ChangeTime`) (`agent_proto/os/windows/local_platform.rs` `file_stamp`, `path_stamp`); Share-Host und lokale Seite nutzen ihn auch unter Windows. |
| Tests | `abgleich_task_first_comparison_sends_keys_then_only_unequal_folders`, `abgleich_task_local_side_keys_spare_the_remote_side_equal_folders` (`snapshot_pair_keys_tests.rs`), `abgleich_task_service_relays_the_key_phases` (`daemon/os/shared/backend_abgleich_task_tests.rs`), `abgleich_task_hash_cache_windows_stamp_follows_every_write` (cfg(windows)); die übrigen Hash-Cache-Tests laufen jetzt auf beiden Plattformen. |

Verbleibende Punkte mit Grund:
| Punkt | Grund |
|---|---|
| Sommerzeit-Regel für exFAT | Quellstelle: exFAT-Spezifikation 7.4.10 (UTC-Abstand je Zeitstempel einschließlich Sommerzeit); `vfs/core/review_task_contract_tests.rs` hält das fest. |
| Prüfsummen-Signatur bleibt MD5 | Garantie: `Sig.hash` steht in jeder gespeicherten Basis; ein Wechsel auf SHA-256 läse jede Basis als geändert. Die MD5 kommt am Ort der Datei (`vfs::content_md5`, umgesetzt für SFTP ohne Agent, FTP, WebDAV; Host-Durchlauf bei Share und Agent). |
