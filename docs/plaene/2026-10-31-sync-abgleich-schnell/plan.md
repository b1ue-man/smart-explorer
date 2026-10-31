# Sync-Abgleich so schnell wie die Speicheranalyse (2026-10-31)

Stand: A–D fertig; Umsetzung D1–D4 als nächster Schritt.

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
