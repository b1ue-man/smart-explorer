# Referenzen-Abgleich: Vergleich und Zusammenführen von Verzeichnisbäumen

- Stand: 2026-10-31
- Auftrag: Nutzervorgaben vom 2026-10-31 und Ergänzung des Koordinators um den lokalen Fall (zwei lokale Bäume, zum Beispiel zwei Laufwerke) und die schnellen lokalen Baum-Erfasser.
- Zugehörig: [plan.md](plan.md), Abschnitt B (Bestand). Die Ergebnisse dienen den Abschnitten C (Entscheidungen) und D (Arbeitsplan) des Plans; diese bleiben offen.
- Art: Recherche und Bestandsbefund. Keine Änderung am Code, keine Builds, keine Tests, keine Commits, keine Pushes. Einzige geänderte Datei: dieses Dokument.

Beleglage: Jede Aussage trägt einen Beleg in eckigen Klammern. `[I-<Kürzel>]` verweist auf eine Datei des Repositorys (Zeilen beziehen sich auf den Arbeitsbaum bei Abruf), `[Q-<Kürzel>]` auf eine externe Quelle mit Commit oder Abrufdatum (Abschnitt 9). „Abgeleitet“ kennzeichnet eine Folgerung aus belegten Aussagen. Laufzeiten sind nicht gemessen; alle Befunde zu Laufzeit beschreiben Roundtrips, Threads, Warte- und Leerlaufpunkte und feste Werte im Code.

## 0 Auftrag und Umfang

### 0.1 Ziele
- Z1: Technik je etablierter Referenz mit Primärquelle, Randbedingungen und berührten Garantien (Abschnitt 3).
- Z2: Je Referenz ein Schritt-für-Schritt-Vergleich mit dem heutigen Weg von Smart Explorer. Jede Referenz hat eine Tabelle mit neun Eigenschaften: Aufrufkette, Systemaufrufe und Roundtrips, Threads und Prozesse, Puffer, Synchronisierung und Sperren, Reihenfolge, Warte- und Leerlaufzeiten, feste Werte, Garantien (Abschnitt 3).
- Z3: Zusammenführen beim Einfügen mit Primärquelle für die Fälle fehlend, neuer, älter und gleich, einschließlich ungenauer Zeitstempel und gleicher Zeit bei verschiedener Größe (Abschnitt 4).
- Z4: Rangfolge der Techniken nach belegtem Beitrag, Liste mit belegtem Ausschlussgrund, Liste nicht geklärter Eigenschaften mit Grund (Abschnitte 5 bis 7).
- Z5: Ergänzung des Koordinators: lokaler Fall und schnelle lokale Baum-Erfasser mit Technik, Primärquelle und Vergleich zum eigenen Weg (Abschnitt 3.9 bis 3.12).

### 0.2 Umfang
Vollständige Liste der Dateioperationen, die dieses Dokument behandelt:
- O1: Erfassen eines Baums (Auflisten mit Metadaten), lokal sowie remote über Direct Share, SSH-Agent, SFTP, SMB, FTP/FTPS, WebDAV und Google Drive.
- O2: Speicheranalyse (Baumgrößen), lokal sowie remote über Direct Share (Analyse auf dem Host) und über Gegenstellen ohne eigene Analyse.
- O3: Rekursive Ansicht, lokal (Modul `scanner`) und remote (Modul `rscan`).
- O4: Vergleich zweier Bäume (Knopf „Vergleichen“ eines Sync-Jobs und die Erfassung eines Sync-Laufs, Modul `bisync`), lokal und remote.
- O5: Einfügen mit Zusammenführen: Erfassung der Auswahl (Datei `scanner/os/collect.rs`) und die Zusammenführungsregel.
- O6: Folge-Läufe: Wiederverwendung von Basis und Prüfschlüsseln sowie Änderungserkennung.

Ausdrücklich nicht Teil dieses Dokuments: Löschen ganzer Bäume (`RemoveTree`), Such- und Filterfunktionen, der Übertragungsweg (Pakete, Kompression, Fenster) außer soweit er die Erfassung beeinflusst, Papierkorb- und Sicherungssemantik. Diese Punkte stehen in Abschnitt 7 als offen.

Vollständigkeit gegenüber allen Dateioperationen des Repositorys ist nicht geprüft. Gelesen wurde der Umfang nach Abschnitt 0.4.

### 0.3 Annahmen
- A1: „Rekursive Ansicht“ ist die Strg+R-Ansicht des Explorers: lokal das Modul `scanner`, remote das Modul `rscan` [I-PLAN, Abschnitt A, A1].
- A2: „Vergleich von zwei Syncs“ ist der Knopf „Vergleichen“ eines Sync-Jobs, der beide Seiten erfasst und nichts ändert [I-PLAN, Abschnitt A, A1]. Welchen Prüfschlüssel-Modus dieser Knopf setzt, ist nicht geklärt (Abschnitt 7, U1).
- A3: Keine Laufzeit wurde gemessen. Alle Befunde beschreiben den Code.
- A4: Quelltexte sind mit Commit in Abschnitt 9 benannt. Webseiten wurden am 2026-10-31 abgerufen. Versionsangaben stehen nur, wenn die Quelle sie nennt.
- A5: Begriffe. „gleich (Metadaten)“ heißt: gleiche Größe und Zeit im Fenster. „gleich (Inhalt)“ heißt: gleicher Prüfschlüssel. Welche der beiden Smart Explorer je Pfad nutzt, ist belegt.
- A6: Im gelesenen Code von Smart Explorer gibt es für den Vergleich keinen Zeitfenster-Wert. Die Wiederverwendung eines Prüfschlüssels verlangt gleiche Größe und gleiche Zeit in Millisekunden [I-DIR:227–231]. Ob eine andere Stelle (Entscheidung „geändert“ im Sync-Lauf) ein Fenster nutzt, ist nicht geprüft (Abschnitt 8, L1).

### 0.4 Leseumfang
Smart Explorer, nur die freigegebenen Dateien:
- [I-PLAN] Abschnitt B.
- [I-TE] `docs/TRANSFER_ENGINE.md`, Abschnitte Aufbau (Zeilen 29–58), Parallelität (Zeilen 60–103, Tabelle 71–77) und Je Protokoll (Zeilen 167–200).
- [I-SNAP] `native/src/bisync/os/shared/snapshot.rs`, [I-AGENT] `snapshot_agent.rs`, [I-WALK] `snapshot_walk.rs`, [I-DIR] `snapshot_dir.rs`.
- [I-RSCAN] `native/src/rscan/os/shared/parallel.rs`.
- [I-ANB] `native/src/analytics/os/shared/analytics_backend.rs`, [I-AN] `native/src/analytics/os/shared/analytics.rs`.
- [I-FSR] `native/src/share/core/fs_request.rs`.
- Modul `scanner`: `mod.rs`, `core/entry.rs`, `core/retention.rs` (Zeilen 1–140), `os/budget.rs`, `os/collect.rs`, `os/linux_os.rs`, `os/retry.rs`, `os/shared.rs`, `os/walk.rs`, `os/windows.rs`. Kürzel: [I-SC-MOD], [I-SC-ENTRY], [I-SC-RET], [I-SC-BUDGET], [I-SC-COLLECT], [I-SC-LINUX], [I-SC-RETRY], [I-SC-SHARED], [I-SC-WALK], [I-SC-WIN].
- Modul `local_access`: `mod.rs` [I-LA-MOD], `os/linux_os.rs` Zeilen 1–75 [I-LA-LINUX], `os/linux/directory_handle.rs` Zeilen 90–245 [I-LA-DH], `os/windows/directory.rs` Zeilen 1–300 [I-LA-WDIR], `os/windows/directory_records.rs` nur über Suchtreffer [I-LA-DREC], `os/windows/confined_names.rs` Zeilen 1–70 [I-LA-WNAMES], `os/windows/read.rs` Zeilen 1–60 [I-LA-WREAD].

Nicht gelesen (als Dateien innerhalb der gelesenen Verzeichnisse): `scanner/os/denied_roots.rs`, `scanner/os/search_recursive_access_task.rs`, `scanner/os/walk_tests.rs`, `scanner/os/shared_tests.rs`, `scanner/os/failed_paths_tests.rs`, `local_access/os/linux/confined.rs`, `local_access/os/linux/remove.rs`, `local_access/os/linux/quarantine.rs`, `local_access/os/linux/create.rs`, `local_access/os/linux/private_ancestors.rs`, `local_access/os/linux/directory_handle.rs` außerhalb der Zeilen 90–245, `local_access/core/*.rs`, sowie alle Dateien unter `local_access/os/windows/` außer den oben genannten (`directory.rs`, `directory_records.rs` über Suchtreffer, `confined_names.rs`, `read.rs`). Aussagen über diese Dateien stehen nicht in diesem Dokument.

Externe Quellen: Abschnitt 9.

## 1 Befunde auf einen Blick

- B1 (Ursache, belegt im Code, nicht gemessen): Der Vergleich zweier Bäume braucht je Ordner mehr Anfragen und Prüfungen als die Speicheranalyse, und er liest bei Prüfschlüssel „Voll“ den Inhalt jeder neuen oder geänderten Datei.
  - Remote: je Ordner eine Stat-Abfrage und eine Auflistung [I-WALK:74–92]; je Eintrag `sync_child_path` und Schutzprüfung [I-DIR:120, 143]; je zu hashender Datei Identitätsprüfung, Erlaubnis, Stream in 65 536-Byte-Blöcken und erneute Prüfung [I-DIR:256–278, 282–316].
  - Speicheranalyse: je Ordner eine Auflistung, Summen über Größen [I-ANB:176–248]. Über Direct Share analysiert der Host selbst, eine Anfrage [I-FSR:206].
  - Remote rekursive Ansicht: je Block von `parallelism` Ordnern wartet der Durchlauf auf die langsamste Auflistung [I-RSCAN:24–44].
  - Lokal: je Ordner Stat und Schutzprüfung [I-WALK:74–92], je Eintrag dieselben Prüfungen, je neuer oder geänderter Datei ein Lesen [I-DIR:276]. Die lokale Speicheranalyse liest je Ordner einmal, hasht nicht und prüft keine Pfade je Eintrag [I-AN:260–333].
  - Lokale rekursive Ansicht: je Eintrag ein vollständiges Objekt mit vier `Arc<str>`-Feldern und Vollpfad [I-SC-WALK:221–235], gesendet in Blöcken von 1024 Einträgen oder alle 60 ms [I-SC-WALK:15–16, 361–364].
- B2 (Agent-Durchlauf bei Standardwerten nicht erreichbar, belegt im Code): `walk_snapshot_via_agent` liefert `None` für lokale Backends, bei `cross_mounts == false`, bei ausgeschlossenen versteckten Dateien und bei doppelten Namen [I-AGENT:25–31]. Dass die Voreinstellung eines Sync-Jobs `cross_mounts = false` ist, stammt aus [I-PLAN, B2] und ist nicht gegengeprüft (Abschnitt 8, L2).
- B3 (vier Wege, die Smart Explorer nicht nutzt; Belege in Abschnitt 5):
  - a) Seitenlokale Erfassung mit einer Anfrage oder einem Strom je Seite [Q-RS1 flist.c:2647].
  - b) Metadatenvergleich vor jedem Inhaltslesen, Prüfschlüssel nur bei Zweifel [Q-RS1 generator.c:667–705; Q-ST1 walk.go:476–490; Q-RC1 operations.go:251–318].
  - c) Persistenter Zwischenspeicher je Datei mit Dateikennung und exakter Zeit, geschützt gegen „racy“ Zeitstempel [Q-MU1 scan.go:148–200; Q-UN1 fpcache.ml:243–258; Q-GIT1 racy-git.adoc:57–102].
  - d) Auflistung ohne Ebenenbarriere mit Arbeitsklau [Q-IG1 walk.rs:1537–1539, 1667–1670; Q-DU1 lib.rs:1075–1090].
- B4 (Zusammenführen): Keine der untersuchten Quellen macht „neuer ersetzt älter“ bei gleicher Zeit und verschiedener Größe zur Standardregel. rsync überträgt bei verschiedener Größe unabhängig von der Zeit [Q-RS1 generator.c:671–672]. rclone prüft bei gleicher Größe und abweichender Zeit den Prüfschlüssel [Q-RC1 operations.go:308–318]. Syncthing benennt bei gleicher Zeit anhand der Gerätekennung um [Q-ST2 syncing.html:64–66]. FreeFileSync führt „gleiches Datum, andere Größe“ als Konflikt-Kategorie [Q-FF1 comparison-settings]. Explorer fragt nach [Q-MS1]. Finder: nicht belegt (Abschnitt 3.8).
- B5 (Überspringen gleicher Dateien): Die Quellen definieren „gleich“ übereinstimmend als gleiche Größe und gleiche Zeit im Fenster, mit Prüfschlüssel als Ausweg (Abschnitt 4). Überspringen allein über die Zeit ist belegt unsicher: rsync `--update` mit positivem Zeitfenster überspringt eine Datei mit gleicher Zeit auch dann, wenn die Größe verschieden ist (abgeleitet aus [Q-RS1 generator.c:2176–2177] und der Reihenfolge vor dem Größenvergleich, Abschnitt 6, X3).
- B6 (Zeitstempel): Smart Explorer vergleicht Zeiten in Millisekunden ohne Fenster (A6). Die Quellen belegen unterschiedliche Genauigkeiten: FAT mit zwei Sekunden und lokaler Zeit [Q-LNX2 misc.c:332, 202–206; Q-MS2 Zeile 826]; NTFS mit 100-Nanosekunden-Intervallen in UTC [Q-MS2 Zeile 805, 826]; NFS mit Serverzeiten in Sekunden und Nanosekunden ohne festgelegte Genauigkeit [Q-RFC1 nfstime3]; im Linux-Speicher können Zeitstempel feiner sein als die gespeicherten [Q-GIT1 Zeilen 41–50].

## 2 Heutiger Weg in Smart Explorer

Alle Angaben beziehen sich auf die freigegebenen Dateien (Abschnitt 0.4). Eine Eigenschaft ohne Beleg steht als „nicht geprüft“ und ist in Abschnitt 7 begründet.

### 2.1 Remote-Vergleich (Sync-Snapshot)

Aufrufkette in Schritten:
1. [I-SNAP:196] Wurzelprüfung (`apply_boundary::guard`).
2. [I-SNAP:197–199] Versuch `walk_snapshot_via_agent`. Liefert `None` bei lokalem Backend, `cross_mounts == false`, `include_hidden == false` oder doppelten Namen [I-AGENT:25–31]. Bei Erfüllung: ein Aufruf `hash_walk` über einen Kanal mit 1024 Plätzen [I-AGENT:125–143].
3. [I-SNAP:213] Erlaubnisfluss je entfernter Seite (`flow_for`); lokal kein Fluss. Details des Flusses: L9 (Abschnitt 8).
4. [I-SNAP:236] `walk_tree(context, flow, be.parallelism().max(1))`.
5. [I-WALK:110–117] Zustand: Mutex für Zustand, Condvar, Mutex für den Ergebnisbaum. [I-WALK:150–176] Koordinator. [I-WALK:218–262] Arbeiter.
6. [I-WALK:178–186] Wachstum: ein neuer Faden nur, wenn kein Faden auf eine Erlaubnis wartet (`waiting == 0`), die Obergrenze nicht erreicht ist und die Warteschlange nicht leer ist. Obergrenze: remote `flow.snapshot().limit + 1`, lokal `be.parallelism()`.
7. [I-WALK:274–325] Je Ordner: remote Erlaubnis `flow.acquire_meta` [I-WALK:288–305]; dann `list_plain_directory` [I-WALK:74–92]: Stat (`sync_stat`), Prüfung auf Link, Sonderdatei und Nicht-Ordner, Schutzprüfung `snapshot_policy::protected` außer an der Wurzel, Auflistung `list_dir_tolerant`.
8. [I-WALK:340] `scan_listing` [I-DIR:62–205]: Namensprüfung, Duplikatprüfung je Name [I-DIR:91–109], `sync_child_path` [I-DIR:120], Schutzprüfung [I-DIR:143], Filter und Größen- und Zeitgrenzen [I-DIR:157–171], Unterordner in die Warteschlange [I-DIR:172–178], Dateien mit `signature_hash` [I-DIR:180].
9. [I-DIR:214–245] `signature_hash`. `None` ergibt 0. `NativeOnly` nimmt den nativen MD5-Wert (Drive, Nextcloud) oder 0. `Full` nimmt den nativen Wert, sonst die Wiederverwendung aus der Basis bei gleicher Größe, gleicher Zeit in Millisekunden und vorhandenem Wert [I-DIR:227–231], sonst Inhalt lesen. `FullFresh` nimmt den nativen Wert oder liest immer [I-DIR:238–243].
10. [I-DIR:250–280] `content_hash`: Identität erfassen (`capture`, [I-DIR:256]), Erlaubnis `flow.acquire_for(job)` [I-DIR:268–272], Stream mit MD5 in 65 536-Byte-Blöcken [I-DIR:282–316], erneute Prüfung [I-DIR:278]. Lokal: `hash_file` [I-DIR:276].
11. [I-WALK:361–389] Zusammenführung je Ordner: Einträge nach Name, dann neueste Zeit zuerst; je Name bleibt der erste Eintrag.
12. [I-WALK:48–61] Abbruch oder Fehler: der ganze Durchlauf schlägt fehl; kein Teilbaum wird als Basis gespeichert [I-DIR:59–61].

| Eigenschaft | Befund |
|---|---|
| Aufrufkette | Schritte 1 bis 12 oben. |
| Systemaufrufe und Roundtrips | Je Ordner eine Stat-Abfrage (`sync_stat`) und eine Auflistung (`list_dir_tolerant`) [I-WALK:74–92]. Je Eintrag `sync_child_path` [I-DIR:120] und `snapshot_policy::protected` [I-DIR:143]; Anzahl Anfragen je Aufruf nicht geprüft (Abschnitt 8, L3). Bei `Full` je Datei Identität, Stream und erneute Identität [I-DIR:256–278]. |
| Threads, Prozesse, Parallelität | Remote: bis `flow.snapshot().limit + 1` Listing-Fäden, je Auflistung eine Erlaubnis [I-WALK:178–186, 288–305]. Lokal: `be.parallelism().max(1)` [I-SNAP:236], Wert nicht geprüft (L3). Prüfschlüssel: je Datei eine Erlaubnis [I-DIR:268–272]. |
| Puffer | Prüfschlüssel-Lesepuffer 65 536 Byte [I-DIR:21]. Auflistung eines Ordners als Vektor im Speicher [I-DIR:62, I-WALK:340]. Kanal des Agent-Durchlaufs 1024 Plätze [I-AGENT:125]. |
| Synchronisierung und Sperren | Mutex für Zustand (`WalkState`), Condvar für Wartezeiten, Mutex für den Ergebnisbaum [I-WALK:94–117]. Ergebnisübergabe je Ordner unter dem Mutex [I-WALK:370–380]. |
| Reihenfolge | Ordner in Warteschlangen-Reihenfolge. Einträge je Auflistung nach Name, dann neueste Zeit zuerst [I-WALK:361–389]. |
| Warte- und Leerlaufzeiten | Koordinator weckt alle 50 ms (`COORDINATOR_SLICE`) [I-WALK:23, 174]. Arbeiter warten höchstens 250 ms auf neue Ordner, wenn andere Arbeit läuft (`WORKER_LINGER`) [I-WALK:27, 240–244]. Neuer Faden nur, wenn kein Faden auf eine Erlaubnis wartet [I-WALK:178–186]. |
| Feste Werte | `MAX_WALK_DEPTH` 512 [I-WALK:20]; 50 ms; 250 ms; 65 536 Byte; 1024 Plätze; Speichergrenzen `SyncLimits::for_memory` [I-SNAP:227], Werte nicht geprüft (L3). Flussgrenzen aus [I-TE:71–77]: Startgrenze 2, Obergrenze 256, Fenster 100 ms bis 5 s, Überlast halbiert die Grenze und hält sie drei Fenster. |
| Garantien | Ganzer Durchlauf oder Fehler; kein Teilbaum als Basis [I-WALK:48–61; I-DIR:59–61]. Auslassungen (Links, Sonderdateien, nicht lesbar, verschwunden) werden gemeldet [I-WALK:326–339]. Identität vor und nach dem Prüfschlüssel [I-DIR:262, 278]. |

### 2.2 Remote-Speicheranalyse

Gegenstellen ohne eigene Analyse [I-ANB:20–59]:
- Fadenzahl `backend.parallelism().clamp(1, 16)` [I-ANB:34]; Obergrenze `MAX_LISTERS = 16` [I-ANB:17].
- Rayon-Pool mit Stapel 64 MiB [I-ANB:35–43; I-AN:50]; `pool.install(visit)` [I-ANB:54–57].
- Je Ordner [I-ANB:176–248] eine Auflistung `list_dir_tolerant` [I-ANB:181]. Unterordner werden parallel verarbeitet, wenn es mehr als einen gibt [I-ANB:131–143]; die Dokumentation nennt „no barrier per level“ [I-ANB:1–7].
- Aufbewahrung: die größten Dateien je Ordner, der Rest als Summe [I-ANB:252–284].

Share (Host analysiert selbst) [I-FSR:53–71, 206]:
- `StorageAnalysis` mit `node_budget` (Knotenzahl) und `compress` (Deflate-Strom).
- Eine Anfrage für den Gesamtbaum; das Ergebnis kommt als ein Strom zurück [I-FSR:58–71].

| Eigenschaft | Befund |
|---|---|
| Aufrufkette | Gegenstelle: Pool → `list` je Ordner → `list_dir_tolerant` → Unterordner parallel [I-ANB:107–143, 176–248]. Share: eine Anfrage `StorageAnalysis` [I-FSR:206]. |
| Systemaufrufe und Roundtrips | Gegenstelle: eine Auflistung je Ordner; keine Stat-Abfrage je Ordner; im Code dieser Datei keine Schutz- oder Identitätsprüfung je Eintrag [I-ANB:176–248]. Share: eine Anfrage für den ganzen Baum [I-FSR:206]. |
| Threads, Prozesse, Parallelität | Gegenstelle: bis 16 Fäden [I-ANB:34]. Share: Analyse auf dem Host, Fadenzahl nicht geprüft. |
| Puffer | Knoten mit Name, Größe, Ordnerkennzeichen und Kindern [I-AN:38–46]. Aufbewahrung je Ordner `MAX_RETAINED_FILES_PER_DIRECTORY` [I-ANB:8, 254]. |
| Synchronisierung und Sperren | Rayon-Arbeitsklau und atomare Zähler [I-ANB:14]. Keine Mutex-Sperre je Eintrag im Code dieser Datei. |
| Reihenfolge | Ordner in Warteschlangen-Reihenfolge; das Ergebnis eines Ordners entsteht, wenn seine Unterordner fertig sind [I-ANB:131–168]. |
| Warte- und Leerlaufzeiten | Keine Ebenenbarriere [I-ANB:1–7]. Wartezeiten im Pool nicht geprüft. |
| Feste Werte | 16 Fäden [I-ANB:17]; 64 MiB Stapel [I-AN:50]; `MAX_RETAINED_FILES_PER_DIRECTORY` [I-ANB:8]. |
| Garantien | Fehler je Eintrag werden gezählt, der Ordner bleibt erhalten [I-ANB:4–7]. |

### 2.3 Remote-rekursive Ansicht (`rscan`)

- [I-RSCAN:9] `parallelism().clamp(2, 16)`; Pool [I-RSCAN:10–19]; Schleife über Ebenen [I-RSCAN:21–63].
- Je Block von `parallelism` Ordnern `par_iter` mit `list_dir` [I-RSCAN:24–44]; danach Verarbeitung auf dem Treiberfaden, nacheinander [I-RSCAN:45–57]. Serieller Rückfall bei Fehlern des Pools [I-RSCAN:67–86].

| Eigenschaft | Befund |
|---|---|
| Aufrufkette | Je Ebene: Blöcke → Pool → `list_dir` je Ordner → Treiber verarbeitet die Ergebnisse [I-RSCAN:24–57]. |
| Systemaufrufe und Roundtrips | Eine Auflistung je Ordner; keine Stat-Abfrage [I-RSCAN:36–41]. |
| Threads, Prozesse, Parallelität | 2 bis 16 Fäden, fest je Lauf [I-RSCAN:9]. |
| Puffer | Ergebnisse eines Blocks im Speicher [I-RSCAN:29–44]. |
| Synchronisierung und Sperren | Keine Sperre; Verarbeitung auf dem Treiberfaden [I-RSCAN:45–57]. |
| Reihenfolge | Ebene für Ebene; jede Ebene vollständig vor der nächsten [I-RSCAN:21–63]. |
| Warte- und Leerlaufzeiten | Barriere je Block: der Block wartet auf die langsamste Auflistung, weil `collect()` nach `par_iter` steht [I-RSCAN:29–44]. |
| Feste Werte | `clamp(2, 16)` [I-RSCAN:9]. |
| Garantien | Fehler je Ordner werden behandelt [I-RSCAN:55]. Ob ein Teilergebnis als vollständig gilt, ist im Modul nicht geprüft. |

### 2.4 Lokaler Vergleich (Sync-Lauf über zwei lokale Bäume)

Derselbe Pfad wie remote, ohne Erlaubnisse:
- Kein Fluss: [I-SNAP:213] `flow` ist `None` für lokale Backends.
- Fäden: `be.parallelism().max(1)` [I-SNAP:236], Wert nicht geprüft.
- Je Ordner: `sync_stat`, Schutzprüfung außer an der Wurzel, Auflistung [I-WALK:74–92].
- Je Eintrag wie in Abschnitt 2.1 [I-DIR].
- Prüfschlüssel `Full`: `hash_file` je neuer oder geänderter Datei [I-DIR:276]; Wiederverwendung aus der Basis nur bei gleicher Größe und gleicher Zeit in Millisekunden [I-DIR:227–231].

| Eigenschaft | Befund |
|---|---|
| Aufrufkette | Wie 2.1, Schritte 4, 5, 7 bis 10 ohne Erlaubnisse. |
| Systemaufrufe und Roundtrips | Je Ordner eine Stat-Abfrage und eine Auflistung; je Eintrag die Prüfungen aus 2.1; bei `Full` je geänderter Datei ein Lesen. Für lokale Dateisysteme sind die Aufrufe der Funktionen `sync_stat`, `list_dir_tolerant`, `sync_child_path` nicht geprüft (L3). |
| Threads, Prozesse, Parallelität | `be.parallelism().max(1)` [I-SNAP:236]. Wert nicht geprüft (L3). |
| Puffer | Wie 2.1; Lesepuffer 65 536 Byte [I-DIR:21]. |
| Synchronisierung und Sperren | Wie 2.1; ohne Erlaubnisse. |
| Reihenfolge | Wie 2.1. |
| Warte- und Leerlaufzeiten | Koordinator 50 ms, Arbeiter 250 ms [I-WALK:23, 27]. |
| Feste Werte | Wie 2.1. |
| Garantien | Wie 2.1: ganzer Durchlauf oder Fehler. |

### 2.5 Lokale Speicheranalyse (`analytics.rs`)

- Fadenzahl `local_scan_threads()` [I-AN:225–231]: Umgebungsvariable `SMART_EXPLORER_ANALYTICS_THREADS`, sonst Plattform-Standard. Die Dokumentation nennt „Android: one per core up to 4; desktop: 2“ [I-AN:223–224]. Begrenzung auf 1 bis 4 [I-AN:230].
- Pool nur, wenn `threads > 1` und `parallel_scan_allowed()` [I-AN:169–177]. Stapel 64 MiB [I-AN:50, 172].
- Je Ordner [I-AN:260–333]: optionaler Wächter [I-AN:291–296]; `read_directory` oder `handle.read_directory()` [I-AN:297–317]; Absturzfang je Ordner [I-AN:320–330].
- Keine Prüfschlüssel, keine Basis, keine Vergleichsbedingung im Code dieser Datei.

| Eigenschaft | Befund |
|---|---|
| Aufrufkette | Pool → `scan_dir_with` je Ordner → Plattform-Auflistung (Abschnitt 2.8) → Unterordner [I-AN:260–333]. |
| Systemaufrufe und Roundtrips | Je Ordner die Plattform-Auflistung (2.8); keine Stat-Abfrage des Ordners im Code dieser Datei; keine Schutzprüfung je Eintrag im Code dieser Datei. |
| Threads, Prozesse, Parallelität | 1 bis 4 Fäden; Standard 2 auf dem Desktop [I-AN:223–231]. Rayon-Pool [I-AN:169–177]. |
| Puffer | Knoten mit Name und Größe [I-AN:38–46]. Stapel 64 MiB je Faden [I-AN:50]. |
| Synchronisierung und Sperren | Rayon-Pool; atomare Zähler im Fortschritt (`Progress`, Datei nicht gelesen). |
| Reihenfolge | Ordner je Pool; Ergebnis eines Ordners nach seinen Unterordnern [I-AN:260–333]. |
| Warte- und Leerlaufzeiten | Nicht geprüft (Datei `analytics_walk.rs` nicht gelesen, L5). |
| Feste Werte | 1 bis 4 Fäden; 64 MiB Stapel; Knotenbudget aus `analytics_budget.rs` (nicht gelesen, L5). |
| Garantien | Fehler je Ordner werden im Ergebnis gesammelt; Abbruch über das Abbruchsignal [I-AN:220]. Kein Hashing. |

### 2.6 Lokale rekursive Ansicht (`scanner`)

- Treiberfaden „scan-driver“ [I-SC-SHARED:87–91]; Fortschrittsfaden alle 150 ms [I-SC-SHARED:401–408].
- Pool aus `min(available_parallelism, 4)` Fäden mit 8 MiB Stapel [I-SC-SHARED:433–440].
- Fork-Join über die Ordner: `into_par_iter` je Ebene und rekursiver Aufruf [I-SC-WALK:40–65]. Keine Ebenenbarriere.
- Je Ordner ein `GuardedIterator` um die Plattform-Auflistung [I-SC-WALK:89–96]. Je Eintrag ein `FileEntry` mit `Arc<str>` für Pfad, Elternpfad, Name und Endung, dazu ein Vollpfad-Text [I-SC-WALK:150, 211, 221–235].
- Ausgabe in Blöcken: `BatchSink` sendet bei 1024 Einträgen oder nach 60 ms über einen Kanal [I-SC-WALK:15–16, 324–383]. Flush der Unterordner bei 128 wartenden Ordnern [I-SC-WALK:290–302].
- Budget je Eintrag: zwei atomare Zähler [I-SC-BUDGET:32–39]. Grenzen 1 000 000 Einträge, 128 MiB Namenstext, Tiefe 512 [I-SC-BUDGET:3–5].
- Wahlweise Aufbewahrungsfilter [I-SC-WALK:261–287; I-SC-RET:1–140].
- Fehler werden gesammelt, Teilergebnisse bleiben [I-SC-SHARED:452–461; I-SC-WALK:150–160].

| Eigenschaft | Befund |
|---|---|
| Aufrufkette | Treiber → Pool → `walk_parallel` → `list_directory` → Plattform-Auflistung → `BatchSink` → Kanal [I-SC-SHARED:87–91, 442; I-SC-WALK:28–96, 324–383]. |
| Systemaufrufe und Roundtrips | Je Ordner: Auflistung (2.8) und, nur mit Verfolgen von Verknüpfungen, `enter_directory` mit Kanonisierung [I-SC-SHARED:177–209]. Je Eintrag Metadaten aus der Auflistung [I-LA-LINUX:36–46]. Optischer Medienprüfer nur bei erkanntem Medium [I-SC-SHARED:159–175]. |
| Threads, Prozesse, Parallelität | Bis 4 Fäden [I-SC-SHARED:433–436]. |
| Puffer | Stapel 8 MiB je Faden [I-SC-SHARED:439]; Blöcke 1024 Einträge [I-SC-WALK:15]; Fortschritt 150 ms [I-SC-SHARED:407]. |
| Synchronisierung und Sperren | Atomare Zähler [I-SC-SHARED:112–125]; Kanal zur Oberfläche; Mutex für fehlgeschlagene Pfade [I-SC-SHARED:121]. |
| Reihenfolge | Ebenen überlappen; Ausgabe in Blöcken nach Ordnerabschluss [I-SC-WALK:290–302]. |
| Warte- und Leerlaufzeiten | Fortschrittsfaden wacht alle 150 ms [I-SC-SHARED:407]. Keine Barriere [I-SC-WALK:40–65]. |
| Feste Werte | 4 Fäden; 8 MiB; 1024 Einträge; 60 ms; 128 Unterordner; 1 000 000 Einträge; 128 MiB; Tiefe 512. |
| Garantien | Best effort: Fehler je Pfad gesammelt [I-SC-SHARED:215–219], Teilergebnis bleibt. Keine Sicherung gegen Vollständigkeit wie im Vergleich. |

### 2.7 Lokale Auswahl-Erfassung für Kopieren (`collect.rs`)

- Eine Schleife mit Stapel [I-SC-COLLECT:114–115]. Die Datei enthält keine Parallelität.
- Je Ordner die Plattform-Auflistung über `GuardedIterator` [I-SC-COLLECT:138–146]; je Eintrag ein `FileEntry` [I-SC-COLLECT:282–296].
- Vollständigkeit: nur ohne Abbruch, ohne Problem und ohne unterdrückte Probleme [I-SC-COLLECT:31–35]. Grenzen 1 000 000 Einträge, 128 MiB Namen, Tiefe 512, 100 Problemeinträge [I-SC-COLLECT:8–11].
- Optische Medien: Prüfung je Ordner nur bei erkanntem Medium [I-SC-COLLECT:106–112, 120–129].

| Eigenschaft | Befund |
|---|---|
| Aufrufkette | Schleife → `read_directory` je Ordner → `FileEntry` → Stapel [I-SC-COLLECT:114–146, 282–303]. |
| Systemaufrufe und Roundtrips | Je Ordner die Plattform-Auflistung (2.8); je Eintrag keine weitere Abfrage außer bei Verfolgen von Verknüpfungen [I-SC-COLLECT:186–229]. |
| Threads, Prozesse, Parallelität | Ein Faden. |
| Puffer | Liste aller Einträge im Speicher, bis 1 000 000 Einträge [I-SC-COLLECT:8, 268]. |
| Synchronisierung und Sperren | Keine. |
| Reihenfolge | Tiefensuche mit Stapel: der zuletzt gefundene Ordner zuerst [I-SC-COLLECT:115]. |
| Warte- und Leerlaufzeiten | Keine Wartezeit im Code dieser Datei; die Wartezeit der Plattform-Auflistung (`GuardedIterator`) nicht geprüft (L6). |
| Feste Werte | 1 000 000 Einträge; 128 MiB Namen; Tiefe 512; 100 Problemeinträge [I-SC-COLLECT:8–11]. |
| Garantien | Alles oder nichts: eine unvollständige Auswahl darf nicht verschoben werden [I-SC-COLLECT:31–39]. |

### 2.8 Plattformschicht: Auflistung und Metadaten

Die Schicht wird von 2.4 bis 2.7 genutzt.

Linux [I-LA-LINUX:23–67]:
- `std::fs::read_dir(path)` sammelt alle Einträge in einen Vektor [I-LA-LINUX:30].
- Sortierung nach Inode-Nummer [I-LA-LINUX:27–31] (Kommentar: „coreutils `cp`, gnulib savedir SAVEDIR_SORT_FASTREAD“).
- Je Eintrag `file_type()` (aus `d_type`) [I-LA-LINUX:36] und `metadata()` [I-LA-LINUX:46]. Die Standardbibliothek fragt die Metadaten relativ zum Verzeichnisdeskriptor ab [Q-RU1 unix-fs.rs:1049–1074].
- Ausgabe `LocalEntry` mit Größe (nur Dateien), Zeiten in ms [I-LA-LINUX:59–62], versteckt bei führendem Punkt [I-LA-LINUX:53].

Linux, eingeschränkter Pfad über einen Verzeichnis-Handle [I-LA-DH:98–110, 178–212]:
- `openat(".")` und `fdopendir` je Auflistung [I-LA-DH:99–107].
- Je Eintrag `readdir` [I-LA-DH:188], dann `metadata_at`: `openat` mit `O_PATH|O_NOFOLLOW`, Metadatenabfrage auf dem Deskriptor, Schließen [I-LA-DH:128–131 (Definition), 207 (Aufruf)]. Je Eintrag also drei Systemaufrufe.

Windows, Auflistung [I-LA-WDIR:28–29, 72–89, 102–127, 136–147, 189–200, 215–238]:
- Attributabfrage des Ordners (`FileAttributeTagInfo`) vor der Auflistung [I-LA-WDIR:136–147].
- Auflistung mit `GetFileInformationByHandleEx` in den Klassen `FileIdExtdDirectoryRestartInfo`, danach `FileIdExtdDirectoryInfo` [I-LA-WDIR:189–200]. Die Klasse liefert den Reparse-Tag mit [I-LA-DREC:117–121; Q-MS3].
- Puffer 8192 Wörter, also 64 KiB; bei `ERROR_MORE_DATA` Vergrößerung ×4 bis 8192 × 16 Wörter, also 1 MiB [I-LA-WDIR:28–29, 211–215 (Schleife `refill`)].
- Rückfall auf `std::fs::read_dir` bei nicht unterstützten Klassen (Fehlercodes 1, 50, 87, 124) [I-LA-WDIR:91–93, 215–238].

Windows, Öffnen [I-LA-WREAD:9–52]:
- Zugriff `FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES`; Freigabe Lesen, Schreiben, Löschen.
- `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT`.
- Bei Zugriffsverweigerung zuerst `BackupRead::enable()` und erneuter Versuch; danach Broker `broker::open_granted` nur, wenn `parallel_scan_allowed()` gilt [I-LA-WREAD:9–52].

Windows, nur Namen [I-LA-WNAMES:18–70]:
- `NtQueryDirectoryFile` mit Klasse 12 (`FileNamesInformation`, [Q-MS4 Zeile 434]), Puffer 128 KiB, Warten auf den Abschluss [I-LA-WNAMES:43–56].

Keine Nutzung von MFT oder USN-Journal in den gelesenen Dateien (Suche über `scanner`, `local_access`, `snapshot_dir.rs`, `analytics.rs`: keine Treffer für `USN`, `FSCTL_`, `MFT`).

### 2.9 Ursache der Abweichung zwischen Vergleich und Speicheranalyse (B1, zusammengefasst)

| Merkmal | Vergleich (Snapshot, 2.1 und 2.4) | Speicheranalyse (2.2 und 2.5) |
|---|---|---|
| Anfragen je Ordner | Remote: zwei (Stat und Auflistung). Lokal: Stat, Schutzprüfung, Auflistung. | Remote: eine Auflistung. Lokal: eine Auflistung. |
| Prüfungen je Eintrag | `sync_child_path`, Schutzprüfung, Filter, Duplikat, Sig-Bildung [I-DIR:62–205]. | Keine im Code dieser Datei. |
| Inhaltslesen | Bei `Full` je neuer oder geänderter Datei [I-DIR:223–236]; keine Wiederverwendung ohne Basis. | Keines. |
| Erlaubnisse | Remote je Auflistung und je Lesen [I-WALK:288–305; I-DIR:268–272]. | Im Code dieser Dateien keine; interne Bindung der Backends nicht geprüft (L3). |
| Ergebnisgröße | Je Eintrag ein Eintrag mit Prüfschlüssel [I-DIR:193–201]. | Aggregiert: größte Dateien und Summe [I-ANB:252–284]. Über Share ein komprimierter Strom [I-FSR:58–71]. |
| Fäden | Remote: Fluss-Grenze plus 1 [I-WALK:178–186]; lokal: `be.parallelism()` (nicht geprüft). | Remote: 16 Fäden [I-ANB:34]; lokal: 1 bis 4, Standard 2 [I-AN:223–231]. |
| Ebenenbarriere | Keine im Vergleich; Barriere in der remote rekursiven Ansicht [I-RSCAN:29–44]. | Keine [I-ANB:1–7]. |

Dies ist eine Code-Ursache. Die Größe des Anteils an der Laufzeit ist nicht gemessen (A3).

## 3 Referenzen: Technik, Garantien, Vergleich mit Smart Explorer

Jeder Abschnitt enthält: Technik in Schritten, Randbedingungen, Garantien und die Tabelle mit neun Eigenschaften zum Vergleich mit dem heutigen Weg (Abschnitt 2).

### 3.1 rsync [Q-RS1]

Technik in Schritten:
1. Der Sender listet die Quelle und sendet je Eintrag Name, Größe, Zeit und Modus. Inkrementelle Rekursion ist Standard (`allow_inc_recurse = 1`, [Q-RS1 options.c:119]); abschaltbar mit `--no-inc-recursive` [Q-RS1 options.c:634–637; rsync.1.md:546]. Die Liste kommt je Ordner in Segmenten (`send_extra_file_list`, [Q-RS1 flist.c:2647]; Aufruf flist.c:3111).
2. Der Generator des Empfängers prüft jeden Listeneintrag gegen die eigene Zieldatei: `stat` der Zieldatei und `quick_check_ok` [Q-RS1 generator.c:667–705]. Bei regulären Dateien: verschiedene Größe ergibt „ungleich“ [Q-RS1 generator.c:671–672]; `--size-only` ergibt „gleich“ [Q-RS1 generator.c:682–683]; `--ignore-times` ergibt „ungleich“ [Q-RS1 generator.c:685–686]; sonst der Zeitvergleich `mtime_differs` [Q-RS1 generator.c:439–441, Aufruf im Bereich 687–705].
3. Zeitvergleich `same_time` [Q-RS1 util1.c:1746–1753]: Fenster 0 vergleicht nur ganze Sekunden; ein negatives Fenster vergleicht Sekunden und Nanosekunden; ein positives Fenster lässt Abweichungen bis zur Fensterweite zu, Nanosekunden spielen dort keine Rolle. Standardwert 0 [Q-RS1 options.c:149; rsync.1.md:908–911]. Der Hinweis auf FAT nennt das Fenster 1 [Q-RS1 rsync.1.md:911–917].
4. `--update` überspringt eine vorhandene Zieldatei, wenn der Empfänger neuer ist [Q-RS1 rsync.1.md:553]. Umsetzung: `update_only > 0`, Ziel vorhanden, `file->modtime − st_mtime < modify_window` [Q-RS1 generator.c:2176–2177].
5. Nur bei ungleicher Datei: Blockprüfsummen der Zieldatei, dann Übertragung der Unterschiede. Phasen: `max_phase = 2` ab Protokoll 29 [Q-RS1 receiver.c:825; sender.c:507].
6. Prozessmodell: Der Empfänger ist ein Kindprozess (`do_fork`, `am_receiver = 1`, [Q-RS1 main.c:1078–1092]); der Generator läuft im Elternprozess und ruft `generate_files` auf [Q-RS1 main.c:1167]. Lokale Kopien laufen über Pipes zu einem Kindprozess [Q-RS1 pipe.c:62, 124].
7. Puffer: `IO_BUFFER_SIZE` 32 KiB [Q-RS1 rsync.h:160]; Ausgabepuffer doppelt, Eingabepuffer einfach [Q-RS1 io.c:1574, 1594].

Randbedingungen:
- Zeiten sind Unix-Sekunden. FAT-Zeiten brauchen das Fenster 1 [Q-RS1 rsync.1.md:911–917].
- Der Vergleich liest Größe und Zeit des Ziels. Prüfsummen nur bei ungleicher Datei (Schritt 2 und 5).
- `--checksum` ersetzt Zeit und Größe durch den Prüfschlüssel: „skip based on checksum, not mod-time & size“ [Q-RS1 rsync.1.md:541].

Garantien:
- Temporäre Dateinamen je Ziel über `get_tmpname` [Q-RS1 receiver.c:322–410]. Die Umbenennung am Ende ist nicht geprüft (L10).
- Phasenwiederholung: `phase`, `redoing`, `batch_redo_list` [Q-RS1 receiver.c:78–79]. Die Prüfung der Gesamtdatei nach der Übertragung ist nicht geprüft (L10).
- Zeit und Attribute getrennt: `mtime_differs` [Q-RS1 generator.c:439–441] und `any_time_differs` [Q-RS1 generator.c:448–455].

Vergleich mit Smart Explorer (Schritt für Schritt):

| Eigenschaft | rsync | Smart Explorer heute |
|---|---|---|
| Aufrufkette | Liste → `quick_check_ok` je Eintrag [Q-RS1 generator.c:667–705]. | `snapshot.rs:197` → `snapshot_walk.rs` → `scan_listing` → `signature_hash` [I-SNAP, I-WALK, I-DIR]. |
| Systemaufrufe und Roundtrips | Eine gestreamte Liste je Lauf. Je Eintrag ein `stat` der Zieldatei. Prüfsummen nur bei Abweichung. | Je Ordner Stat und Auflistung; je Eintrag `sync_child_path` und Schutzprüfung [I-WALK:74–92; I-DIR:120, 143]; bei `Full` je Datei Identität, Stream und erneute Identität [I-DIR:256–278]. |
| Threads, Prozesse, Parallelität | Je Seite Prozesse: Generator und Empfänger (Kindprozess) [Q-RS1 main.c:1078–1092]; Sender auf der Gegenseite. | Remote bis `limit + 1` Fäden [I-WALK:178–186]; lokal `be.parallelism()` [I-SNAP:236]. |
| Puffer | 32 KiB Ein- und Ausgabe [Q-RS1 rsync.h:160; io.c:1574, 1594]. | Prüfschlüssel-Lesepuffer 64 KiB [I-DIR:21]; Auflistung je Ordner als Vektor. |
| Synchronisierung und Sperren | Prozesse mit Pipes oder Sockets. Sperren im gelesenen Umfang nicht genannt. | Mutex, Condvar, Mutex für den Ergebnisbaum [I-WALK:110–117]. |
| Reihenfolge | Liste in Dateireihenfolge; Vergleich je Eintrag; Ergebnis über den Strom [Q-RS1 flist.c:2647]. | Warteschlange der Ordner; Ergebnis je Ordner nach Abschluss der Auflistung [I-WALK:361–389]. |
| Warte- und Leerlaufzeiten | Keine Ebenenbarriere: der Generator wartet nur auf den nächsten Eintrag der Liste (abgeleitet). | Koordinator 50 ms; Arbeiter 250 ms; neuer Faden erst ohne Erlaubniswartende [I-WALK:23, 27, 178–186]. |
| Feste Werte | Puffer 32 KiB; Fenster 0; Phasen 2 ab Protokoll 29. | Tiefe 512; 50 ms; 250 ms; 64 KiB. |
| Garantien | Temporärnamen; Phasenwiederholung; Zeit und Attribute getrennt. | Ganzer Durchlauf oder Fehler; keine Teilbasis [I-WALK:48–61]. |

Befund (abgeleitet): rsync legt den Vergleich auf die Zielseite und überträgt die Liste der Quelle als Strom. Die Quelle braucht keine Abfrage des Ziels. Smart Explorer stellt umgekehrt je Ordner Anfragen an die Gegenseite. Übernahmefähig ist die Strom-Liste je Seite (Abschnitt 5, Platz 1).

### 3.2 rclone [Q-RC1]

Technik in Schritten:
1. Auflistung: Standard „List“, je Verzeichnis eine Anfrage. `--fast-list` nutzt die Fähigkeit ListR nur bei Backends, die sie anbieten; für `sync` und `copy` bevorzugt rclone „List“, weil es die Einträge im Speicher halten und parallel verarbeiten muss [Q-RC1 docs.md:3001–3031]. Die Auswahl erfolgt in [Q-RC1 fs/march/march.go:128–160].
2. Lokal unter Unix: `Readdirnames(1024)` in Stapeln und je Name `os.Lstat` [Q-RC1 backend/local/local.go:728, 735]. Unter Windows: `Readdir(1024)`, also Dateiinformationen direkt [Q-RC1 backend/local/local.go:39, 717–720].
3. Vergleich `equal` [Q-RC1 fs/operations/operations.go:248–320]:
   - a) Größe verschieden: ungleich [operations.go:251–255].
   - b) `--size-only`: gleich [operations.go:256–260].
   - c) `--checksum`: Prüfsumme gleich, dann gleich; ohne gemeinsame Prüfsumme Warnung und Rückfall auf die Größe [operations.go:265–286].
   - d) Sonst Zeitfenster: `GetModifyWindow` ist das größte Fenster aus Konfiguration und Präzision beider Seiten [Q-RC1 fs/fs.go:100–115]. Liegt die Zeitdifferenz innerhalb des Fensters: gleich [operations.go:288–306].
   - e) Zeitdifferenz außerhalb des Fensters: Prüfsumme beider Seiten (`CheckHashes`). Verschieden: ungleich. Keine Prüfsumme und kein `RefreshTimes`: ungleich. Gleich: gleich, und die Zeit des Ziels wird gesetzt, sofern `updateModTime` gilt [operations.go:308–318; Standard `updateModTime = !NoUpdateModTime`, operations.go:200–220].
4. `--update`: überspringt ein Ziel, das um mindestens das Fenster neuer ist als die Quelle [Q-RC1 docs.md:3066–3069; operations.go:1808–1822, Bedingung `dt >= modifyWindow`].
5. Präzision je Seite: `readPrecision` setzt Zeitstempel auf eine temporäre Datei in Schritten von 1 ns bis unter 1 s; Standard 1 s [Q-RC1 backend/local/local.go:1033–1062].
6. Prüfsummen vom Backend: SFTP führt `md5sum` und `sha1sum` auf dem Server aus [Q-RC1 backend/sftp/sftp.go:285–290, 575]. Der Server rechnet, nur das Ergebnis kommt zurück.

Randbedingungen:
- Das Standardfenster ist 1 ns, sofern die Gegenstelle nichts anderes meldet [Q-RC1 docs.md:2336–2345].
- Bei Zeitabweichung ohne Prüfsumme ist das Ergebnis „ungleich“ und die Datei wird übertragen [operations.go:315–318].
- Parallelität: 8 Prüfvergleiche (`--checkers`, [Q-RC1 docs.md:1234–1246]) und 4 Übertragungen (`--transfers`, [Q-RC1 docs.md:3056–3060]).

Garantien:
- Kommentar vor `Equal`: „including if there were errors reading info“: Lesefehler machen die Dateien ungleich [Q-RC1 operations.go:138–144]. Ein Fehler führt also zur Übertragung, nicht zum Überspringen.
- Gleiche Prüfsumme bei abweichender Zeit: die Zeit des Ziels wird gesetzt, wenn `--checksum` nicht gilt [Q-RC1 operations.go:138–143].

Vergleich mit Smart Explorer (Schritt für Schritt):

| Eigenschaft | rclone | Smart Explorer heute |
|---|---|---|
| Aufrufkette | Auflistung je Verzeichnis → `Equal` je Paar → Kopie oder Überspringen [Q-RC1 operations.go:248–320]. | Wie 3.1: `snapshot_walk.rs` → `scan_listing` → `signature_hash` [I-WALK, I-DIR]. |
| Systemaufrufe und Roundtrips | Unix: `Readdirnames(1024)` plus `Lstat` je Name [Q-RC1 local.go:728, 735]. Windows: `Readdir(1024)`. Prüfsumme je Seite nur bei Zeitabweichung. | Je Ordner Stat und Auflistung; je Eintrag `sync_child_path`; Prüfschlüssel bei `Full`. |
| Threads, Prozesse, Parallelität | 8 Vergleiche, 4 Übertragungen [Q-RC1 docs.md:1234–1246, 3056–3060]. | Remote bis `limit + 1`; lokal `be.parallelism()` [I-WALK:178–186]. |
| Puffer | Namen in Stapeln zu 1024 [Q-RC1 local.go:728]. | Listen als Vektor je Ordner [I-DIR:62]. |
| Synchronisierung und Sperren | Nicht geprüft (Kanäle und Sperren im gelesenen Umfang nicht untersucht). | Mutex und Condvar [I-WALK:110–117]. |
| Reihenfolge | Auflistung je Verzeichnis, dann Paarvergleich. | Warteschlange der Ordner [I-WALK:218–262]. |
| Warte- und Leerlaufzeiten | Nicht geprüft. | 50 ms; 250 ms [I-WALK:23, 27]. |
| Feste Werte | 8; 4; 1024; Fenster 1 ns; Ausweichfenster 1 s bei fehlender Präzision [operations.go:1815–1820]. | 512; 50 ms; 250 ms; 64 KiB. |
| Garantien | Lesefehler gleich „ungleich“; Zeit nur bei gleicher Prüfsumme gesetzt. | Ganzer Durchlauf oder Fehler; keine Teilbasis [I-WALK:48–61]. |

Befund: rclone gibt die Gleichheit ohne Prüfsumme nur bei gleicher Größe und Zeit im Fenster. Sonst liest es (bei Zeitabweichung und vorhandenem Hash) oder entscheidet „ungleich“ (ohne Hash). Übernahmefähig ist die Reihenfolge Größe, Zeit im Fenster, Prüfsumme nur bei Zeitabweichung, und die Regel „Fehler ergibt ungleich“ (Abschnitt 4).

### 3.3 Syncthing [Q-ST1, Q-ST2, Q-ST3]

Technik in Schritten:
1. Lokaler Scan je Ordner: die Namen des Verzeichnisses mit `Readdirnames(-1)` in einem Aufruf [Q-ST1 lib/fs/basicfs.go:224–238]; je Name `Lstat` [Q-ST1 lib/fs/basicfs.go:180–185]. Der Einstieg `Walk` liegt in `lib/fs/walkfs.go:142–152`: Lstat der Wurzel, dann die rekursive Funktion `walk` (Definition nicht gelesen, L11).
2. Unverändert, wenn Dateityp, Größe, Zeit im Fenster und, sofern nicht ignoriert, die Rechte gleich sind. Dann werden die Blöcke übernommen und nicht neu berechnet [Q-ST1 lib/scanner/walk.go:476–490 mit `IgnoreBlocks: true`; lib/protocol/bep_fileinfo.go:467–540; Gleichheit der Zeit in `ModTimeEqual`, bep_fileinfo.go:545–554].
3. Geändert: `HashFile` liest die Datei blockweise [Q-ST1 lib/scanner/blockqueue.go:19–40]. Blockgröße: die kleinste Zweierpotenz ab 128 KiB, bei der die Datei weniger als 2000 Blöcke hat; höchstens 16 MiB [Q-ST1 lib/protocol/protocol.go:48–58; bep_fileinfo.go:403–411; Q-ST3 Abschnitt „Selection of Block Size“].
4. Die Blockgröße bleibt erhalten, wenn die neue nur um den Faktor 2 abweicht [Q-ST1 lib/scanner/walk.go:449–461].
5. Hash-Fäden: `numHashers` nimmt die Konfiguration, sonst `GOMAXPROCS / 2` (auf Windows, iOS und Android `GOMAXPROCS / 4`), mindestens 1 [Q-ST1 lib/model/model.go:2556–2576].
6. Zeit: Sekunden und Nanosekunden [Q-ST1 lib/scanner/walk.go:774–775]. Fenster `ModTimeWindow` je Ordner [Q-ST1 lib/config/folderconfiguration.go:139–150]: auf Android 2 s, außer auf ext2, ext3 und ext4; sonst der Konfigurationswert (Feld ohne Standardwert, [Q-ST1 folderconfiguration.go:78], also 0).
7. Überwachung: Dateisystem-Watcher mit Verzögerung 10 s [Q-ST1 folderconfiguration.go:59]; Vollscan 3600 s [Q-ST1 folderconfiguration.go:57], Abstand zufällig zwischen 3/4 und 5/4 des Intervalls [Q-ST2 syncing.html:28–34].
8. Vollscans bleiben auch mit Watcher eingeschaltet, „as it is possible that some changes aren't picked up by it“ [Q-ST2 syncing.html:28–34].
9. Geräteabgleich: `Index` ist der vollständige Inhalt eines Ordners, `IndexUpdate` eine Änderung dazu [Q-ST3 bep-v1.html:248–256, 263]; Blöcke werden mit `Request` angefordert [Q-ST3 Zeile 360].
10. Konflikt: Bei gleichzeitiger Änderung mit verschiedenem Inhalt wird die Datei mit der älteren Zeit umbenannt. Bei gleicher Zeit wird die Datei des Geräts mit dem größeren Wert der ersten 63 Bit der Geräte-ID umbenannt [Q-ST2 syncing.html:60–66].

Randbedingungen:
- Änderung von Zeit, Größe oder Rechten ergibt ein Neuhashen [Q-ST2 syncing.html:34–38].
- Ein Fehler beim Hashen wird über den Kanal gemeldet und in die Bilanz eingetragen (Details nicht geprüft, L11).

Garantien:
- Temporäre Dateien mit dem Präfix `.syncthing.` [Q-ST1 lib/fs/tempname.go:20].
- Konfliktkopien statt Verlust: eine Datei wird umbenannt, nicht gelöscht [Q-ST2 syncing.html:60–66].
- Unverändert nur bei gleichen Rechten, wenn die Rechte nicht ignoriert werden [Q-ST1 bep_fileinfo.go:476–478].

Vergleich mit Smart Explorer (Schritt für Schritt):

| Eigenschaft | Syncthing | Smart Explorer heute |
|---|---|---|
| Aufrufkette | Namen, Lstat, Unverändert-Prüfung, Hashen nur bei Änderung [Q-ST1 walk.go:476–490]. | Auflistung, Prüfung je Eintrag, Prüfschlüssel bei `Full` [I-WALK, I-DIR]. |
| Systemaufrufe und Roundtrips | Lokal: je Name ein Lstat, kein Netz. Zwischen Geräten: Index-Austausch [Q-ST3 Zeilen 248–263]. | Remote: je Ordner Stat und Auflistung; je Eintrag `sync_child_path` [I-WALK:74–92; I-DIR:120]. |
| Threads, Prozesse, Parallelität | `numHashers`: GOMAXPROCS/2, auf Windows, iOS und Android GOMAXPROCS/4, mindestens 1 [Q-ST1 model.go:2556–2576]. | Fluss-Grenze oder `be.parallelism()` [I-WALK:178–186]. |
| Puffer | Blöcke von 128 KiB bis 16 MiB [Q-ST1 protocol.go:48–51]; Puffer-Pool je Blockgröße [Q-ST1 lib/protocol/bufferpool.go:80–95]. | Hash-Lesepuffer 64 KiB [I-DIR:21]. |
| Synchronisierung und Sperren | Nicht geprüft (Hash-Warteschlangen in `walk.go:136–160`, nicht gelesen). | Mutex und Condvar [I-WALK:110–117]. |
| Reihenfolge | Scan je Ordner; Hashen nach Erkennung der Änderung [Q-ST1 walk.go:248]. | Warteschlange [I-WALK:218–262]. |
| Warte- und Leerlaufzeiten | Watcher-Verzögerung 10 s; Vollscan 3600 s mit Abstand 3/4 bis 5/4 [Q-ST1 folderconfiguration.go:57, 59; Q-ST2]. | 50 ms; 250 ms [I-WALK:23, 27]. |
| Feste Werte | 128 KiB bis 16 MiB; 2000 Blöcke; 10 s; 3600 s; Android 2 s [Q-ST1, Q-ST3]. | 512; 50 ms; 250 ms; 64 KiB. |
| Garantien | Unverändert bei gleicher Größe, Zeit im Fenster, gleichen Rechten; Konfliktkopie statt Verlust; Temporärname mit `.syncthing.`. | Ganzer Durchlauf oder Fehler; Identitätsprüfung [I-DIR:262, 278]. |

Befund: Syncthing hasht nur geänderte Dateien und behält die Blockliste im Index. Smart Explorer hasht bei `Full` jede neue Datei, außer die Basis hat einen Wert mit gleicher Größe und gleicher Zeit in Millisekunden [I-DIR:227–231]. Übernahmefähig: Unverändert-Prüfung mit Blockliste und Vollscan als Sicherung (Abschnitt 5, Platz 2, 5).

### 3.4 Unison [Q-UN1]

Technik in Schritten:
1. Archiv: je Wurzel der Stand des letzten Abgleichs. Die Änderungsfindung vergleicht den Ist-Zustand mit dem Archiv (`Update.updateArchive` [Q-UN1 src/files.ml:503, 721, 756, 809]).
2. Schnelle Erkennung (`fastcheck`): Standard `Default`, und `useFastChecking` ist für `Default` wahr [Q-UN1 src/update.ml:1375–1377]. Die Bedingung `Sys.unix` steht auskommentiert im Quelltext [Q-UN1 src/update.ml:1377].
3. Unverändert nach der Schnellprüfung, wenn Änderungszeit und Länge gleich sind, die Datei keine Excel-Datei ist und bei Inode-Stempel die Inode gleich ist [Q-UN1 src/fpcache.ml:243–258].
4. Sonst Fingerabdruck des Inhalts (`Os.safeFingerprint`, [Q-UN1 src/os.ml:273–285]). Wenn sich die Datei beim Lesen ändert, wird wiederholt; nach Ablauf der Versuche ist der Fehler „the file keeps on changing“ [Q-UN1 src/os.ml:274–285].
5. Persistenter Cache: Tabelle im Speicher [Q-UN1 src/fpcache.ml:30]; Plattenformat „Unison fingerprint cache format 3“ [Q-UN1 src/fpcache.ml:161]; Grenzen 5000 Einträge und 100 MiB [Q-UN1 src/fpcache.ml:215–216].
6. Verzeichnisse: Schnellprüfung nur unter Unix [Q-UN1 src/update.ml:2415].
7. Erster Lauf: alle Dateien werden vollständig geprüft [Q-UN1 src/update.ml:1349–1373, Text „This does not apply to the very first run“].
8. Konflikte: Präferenz `newer` oder `older` für die Wurzel [Q-UN1 src/recon.ml:74–94].
9. Gesicherte Temporärdatei umbenennen [Q-UN1 src/copy.ml:229].

Randbedingungen und Garantien:
- Handbuch: Die Schnellprüfung kann eine Änderung übersehen, wenn Zeit und Länge unverändert bleiben. Unison überschreibt eine solche Änderung nie, weil vor jeder Übertragung eine sichere Prüfung erfolgt [Q-UN1 doc/unison-manual.tex:2380–2384].
- Dateien mit der Erweiterung `.xls` oder `.mpp` sind immer ausgenommen [Q-UN1 manual 2387–2389; fpcache.ml:238–241].
- Experimentell: `fastercheckUNSAFE` erfasst neue Dateien ohne Fingerabdruck; das kann Konflikte übersehen [Q-UN1 src/fpcache.ml:272–300].
- Widerspruch im Quellmaterial (ungeklärt): Das Handbuch nennt „fast auf Unix, langsam auf Windows“ als Standard [Q-UN1 manual 2366–2378]. Der Quelltext schaltet `Default` ohne Betriebssystembedingung ein [Q-UN1 src/update.ml:1375–1377]. Welches Verhalten die Auslieferung zeigt, ist nicht geprüft (L12).
- Stempelfeld im Widerspruch: Das Handbuch nennt die Erstellungszeit als „Pseudo-Inode“ [Q-UN1 manual 2366–2370]; die Präferenz im Quelltext nennt Änderungszeit und Länge [Q-UN1 src/update.ml:1355–1357].

Vergleich mit Smart Explorer (Schritt für Schritt):

| Eigenschaft | Unison | Smart Explorer heute |
|---|---|---|
| Aufrufkette | Archiv → Ist-Zustand → Schnellprüfung → ggf. Fingerabdruck [Q-UN1 fpcache.ml:305–330]. | Auflistung → Prüfung je Eintrag → Prüfschlüssel bei `Full` [I-WALK, I-DIR]. |
| Systemaufrufe und Roundtrips | Stat je Eintrag [Q-UN1 src/fileinfo.ml:83, 89]; Inhalt nur bei Zweifel. Remote: im gelesenen Umfang nicht geprüft. | Je Ordner Stat; je Eintrag Prüfung; Inhalt bei `Full` [I-WALK:74–92; I-DIR:223–236]. |
| Threads, Prozesse, Parallelität | Nicht geprüft. | Fäden nach Fluss-Grenze oder `be.parallelism()` [I-WALK:178–186]. |
| Puffer | Nicht geprüft. | 64 KiB [I-DIR:21]. |
| Synchronisierung und Sperren | Globale Tabelle `tbl` im Modul; im gelesenen Abschnitt keine Sperre erkennbar [Q-UN1 fpcache.ml:30, 305–330]. | Mutex und Condvar [I-WALK:110–117]. |
| Reihenfolge | Ist-Zustand lesen, dann Schnellprüfung, dann Fingerabdruck. | Warteschlange der Ordner [I-WALK:218–262]. |
| Warte- und Leerlaufzeiten | Nicht geprüft. | 50 ms; 250 ms [I-WALK:23, 27]. |
| Feste Werte | 5000 Einträge; 100 MiB [Q-UN1 fpcache.ml:215–216]. | 512; 64 KiB; 50 ms; 250 ms. |
| Garantien | Unverändert nur bei gleicher Zeit, Länge und (mit Inode) Inode; sichere Prüfung vor jeder Übertragung [Q-UN1 manual 2380–2384]. | Ganzer Durchlauf oder Fehler; Identitätsprüfung [I-WALK:48–61; I-DIR:262, 278]. |

Befund: Unison zeigt, wie das Lesen des Inhalts durch Metadaten ersetzt wird. Die Schwäche (übersehene Änderung bei gleicher Zeit und Länge) belegt das Handbuch. Die Inode-Bedingung schließt einen Teil davon, nur auf Systemen mit Inode-Stempeln [Q-UN1 src/fpcache.ml:251–258]. Übernahmefähig: Metadaten-Cache mit Dateikennung (Abschnitt 5, Platz 3) und die Regel, dass eine Gleichheit aus Metadaten vor jeder Übertragung nochmals geprüft wird (Abschnitt 4).

### 3.5 Mutagen [Q-MU1]

Technik in Schritten:
1. Zwei Endpunkte: der lokale (`pkg/synchronization/endpoint/local`) und der entfernte über einen Agenten (`pkg/synchronization/endpoint/remote`, Protokoll in `protocol.proto`) [Q-MU1].
2. Scan je Wurzel (`pkg/synchronization/core/scan.go`, Funktion `Scan` ab Zeile 651): Namen eines Ordners mit `Readdirnames(0)` [Q-MU1 pkg/filesystem/directory_posix.go:317]; je Name `ReadContentMetadata` [Q-MU1 directory_posix.go:344]. Auf Windows liefert `ReadContents` die Einträge schneller über `FindFirstFile` und `FindNextFile` [Q-MU1 directory_posix.go:370–376].
3. Digest-Wiederverwendung (`file`, scan.go:148–200): Der Cache-Eintrag gilt nur, wenn Dateityp, exakte Änderungszeit (`ModificationTime.Equal`), Größe und Datei-ID gleich sind [Q-MU1 scan.go:148–200, Bedingung `cacheContentMatch`]. Sonst wird der Inhalt gelesen und der Digest neu berechnet.
4. Überwachung [Q-MU1 pkg/synchronization/watch_mode.proto]: `WatchModePortable` (native rekursive Überwachung, wo verfügbar, für Pfade unter dem Home-Verzeichnis; sonst Polling), `WatchModeForcePoll` (nur Polling), `WatchModeNoWatch` (keine Ereignisse).
5. Linux im Open-Source-Build: nicht-rekursive inotify-Überwachung mit Kapazität 50 und Standard-Höchstzahl 50 Überwachungen je Überwacher [Q-MU1 pkg/filesystem/watching/watch_non_recursive_linux.go:19–24]. Rekursive Linux-Überwachung steht nur im Build mit den Tags `mutagensspl` und `mutagenfanotify` [Q-MU1 pkg/filesystem/watching/watch_recursive_unsupported.go, Zeile 1].
6. Wiederverbindung alle 15 s [Q-MU1 pkg/synchronization/controller.go:28, Aufrufe 779, 841]; Wartezeit nach flüchtigem Scanfehler 5 s [Q-MU1 controller.go:31, 1093].
7. Übertragung: Paket `pkg/synchronization/rsync` mit Engine, Transmit und Receiver; die Paketbeschreibung nennt den rsync-Algorithmus und die Technischen Berichte als Quelle [Q-MU1 pkg/synchronization/rsync/doc.go].

Randbedingungen:
- Der Digest-Cache verlangt exakte Zeitgleichheit, keine Toleranz [Q-MU1 scan.go:148–200].
- Der Polling-Pfad hat im geprüften Code keine Dauerkonstante (L13, Abschnitt 7 U15).

Garantien:
- Digest nur bei gleicher Datei-ID: eine ersetzte Datei mit gleicher Größe und Zeit wird neu gelesen [Q-MU1 scan.go:148–200].
- Rechte-Änderungen zählen für die Wiederverwendung des Digests nicht, weil sie den Inhalt nicht ändern. Für die Wiederverwendung des Cache-Eintrags wird der volle Modus geprüft [Q-MU1 scan.go:148–200, Kommentar].

Vergleich mit Smart Explorer (Schritt für Schritt):

| Eigenschaft | Mutagen | Smart Explorer heute |
|---|---|---|
| Aufrufkette | Agent-Scan → Cache-Prüfung → Digest nur bei Änderung [Q-MU1 scan.go:148–200]. | Auflistung → Prüfung je Eintrag → Prüfschlüssel bei `Full` [I-WALK, I-DIR]. |
| Systemaufrufe und Roundtrips | Namen je Ordner; je Name Metadaten; Digest bei Änderung [Q-MU1 directory_posix.go:317, 344]. | Je Ordner Stat und Auflistung; je Eintrag `sync_child_path` [I-WALK:74–92; I-DIR:120]. |
| Threads, Prozesse, Parallelität | Nicht geprüft (Scan-Schleife nicht gelesen). | Fäden nach Fluss-Grenze [I-WALK:178–186]. |
| Puffer | Nicht geprüft. | 64 KiB [I-DIR:21]. |
| Synchronisierung und Sperren | Nicht geprüft. | Mutex und Condvar [I-WALK:110–117]. |
| Reihenfolge | Scan, dann Abgleich der Schnappschüsse (Abgleich im Detail nicht gelesen). | Warteschlange [I-WALK:218–262]. |
| Warte- und Leerlaufzeiten | 15 s (Wiederverbindung); 5 s (Rescan-Wartezeit) [Q-MU1 controller.go:28, 31]. | 50 ms; 250 ms [I-WALK:23, 27]. |
| Feste Werte | Überwacher 50; Kanal 50; 15 s; 5 s [Q-MU1]. | 512; 64 KiB; 50 ms; 250 ms. |
| Garantien | Digest nur bei gleicher Größe, exakter Zeit, gleichem Typ und gleicher Datei-ID [Q-MU1 scan.go:148–200]. | Ganzer Durchlauf oder Fehler; Basis nur nach Erfolg [I-WALK:48–61]. |

Befund: Mutagen sichert den Digest-Cache mit der Datei-ID. Smart Explorer vergleicht für die Wiederverwendung nur Größe und Zeit [I-DIR:227–231]. Das Protokoll enthält die Datei-ID bereits (`FileIdentity`, [I-FSR:164–166]). Übernahmefähig: Cache mit Datei-ID (Abschnitt 5, Platz 3).

### 3.6 FreeFileSync [Q-FF1]

Hinweis zur Quelle: Der Quelltext konnte nicht abgerufen werden (L14). Die Aussagen stammen aus dem Handbuch (Abruf 2026-10-31).

Technik in Schritten:
1. Vergleich über relative Pfade: „FreeFileSync compares two folders by analyzing the relative paths of the files they contain“ [Q-FF1 comparison-settings].
2. Variante „Zeit und Größe“: gleich, wenn Änderungszeit und Größe gleich sind. Kategorien: nur links, nur rechts, links neuer, rechts neuer, gleich, Konflikt („same date, different size“) [Q-FF1 comparison-settings].
3. Variante „Inhalt“: gleich bei identischem Inhalt; „This is the slowest of all comparison variants“ [Q-FF1 comparison-settings].
4. Variante „Größe“: gleich bei gleicher Größe; nur, wenn Zeiten nicht verlässlich sind [Q-FF1 comparison-settings].
5. Zeitverschiebung: „To handle daylight saving time differences, enter a single one-hour shift.“ Die Verschiebung gilt nur für Gleichheit: „File times must be equal or differ by exactly the specified time shift to be considered the same. Therefore, the time shift setting should not be confused with a time interval or tolerance.“ [Q-FF1 daylight-saving-time].
6. Synchronisationsvarianten [Q-FF1 synchronization-settings, „Synchronization Variants“]: Zweiseitig mit Datenbank `sync.ffs_db` zur Erkennung der Änderungen seit dem letzten Lauf; Spiegeln (Ziel wird exakte Kopie der Quelle); Aktualisieren („Update“): Dateien, die auf der Quelle gelöscht wurden, bleiben am Ziel; am Ziel gelöschte Dateien werden nicht erneut kopiert.
7. Verschobene Dateien erkennen: nur mit Datenbank und Datei-IDs des Dateisystems [Q-FF1 synchronization-settings, „Detect Moved Files“].
8. Parallele Dateioperationen je Gerät; dafür öffnet FreeFileSync mehrere Verbindungen zum Gerät. Das Handbuch nennt im Beispiel 6 parallele Operationen; einen Standardwert nennt es nicht [Q-FF1 performance, Zeilen 926–953].

Randbedingungen und Garantien:
- Eine Zeittoleranz für die Variante „Zeit und Größe“ nennt das Handbuch nicht; die Verschiebung ist ausdrücklich keine Toleranz [Q-FF1 daylight-saving-time].
- Die Datenbank ist Voraussetzung für die Erkennung von Änderungen und verschobenen Dateien [Q-FF1 synchronization-settings].

Vergleich mit Smart Explorer (Schritt für Schritt):

| Eigenschaft | FreeFileSync | Smart Explorer heute |
|---|---|---|
| Aufrufkette | Nicht geprüft (Quelltext nicht erhalten, L14). | Wie 2.1. |
| Systemaufrufe und Roundtrips | Nicht geprüft. | Je Ordner Stat und Auflistung [I-WALK:74–92]. |
| Threads, Prozesse, Parallelität | Mehrere Verbindungen je Gerät; Standardwert nicht genannt [Q-FF1 performance]. | Fluss-Grenze [I-WALK:178–186]. |
| Puffer | Nicht geprüft. | 64 KiB [I-DIR:21]. |
| Synchronisierung und Sperren | Nicht geprüft. | Mutex und Condvar [I-WALK:110–117]. |
| Reihenfolge | Vergleich, dann Aktion je Kategorie. | Warteschlange [I-WALK:218–262]. |
| Warte- und Leerlaufzeiten | Nicht geprüft. | 50 ms; 250 ms [I-WALK:23, 27]. |
| Feste Werte | Keine Zahl außer dem Beispiel mit 6 Operationen [Q-FF1 performance]. | 512; 64 KiB; 50 ms; 250 ms. |
| Garantien | Datenbank für Zweiseitig; Konfliktkategorie statt Gewinner; Verschiebung ist keine Toleranz [Q-FF1]. | Ganzer Durchlauf oder Fehler [I-WALK:48–61]. |

Befund: FreeFileSync zeigt drei Lösungen, die Smart Explorer nicht hat: eine Konfliktkategorie für gleiche Zeit bei verschiedener Größe, eine Datenbank für die Änderungserkennung, und die Erkennung verschobener Dateien über Datei-IDs. Übernahmefähig sind die Konfliktkategorie (Abschnitt 4) und die Trennung von Zeitverschiebung und Toleranz (Abschnitt 4.2).

### 3.7 robocopy [Q-RB1]

Hinweis: Die Dokumentation beschreibt nur die Optionen. Die interne Aufzählung und die Vergleichsreihenfolge sind nicht dokumentiert (L15, U11).

Technik (Optionen in der Dokumentation):
1. Vergleich nach Name, Größe und Zeit. Klassen nach Optionsbeschreibung: „gleich“ mit `/IS` („identical in name, size, times, and all attributes“), „geändert“ mit `/IT` („same name, size, and times, but different attributes“), „geänderte Zeit“ mit `/IM` („differing change times“), nur bei Dateisystemen mit Änderungszeit wie NTFS [Q-RB1 Zeilen 1039–1048, 1261].
2. `/XO`: Quelldateien, die älter sind als das Ziel, werden nicht kopiert [Q-RB1 Zeilen 1027–1028]. `/XN`: Quelldateien, die neuer sind als das Ziel, werden nicht kopiert [Q-RB1 Zeilen 1023–1024]. `/XC`: vorhandene Dateien mit gleicher Zeit und anderer Größe werden nicht kopiert [Q-RB1 Zeilen 1019–1020].
3. `/XL`: nur in der Quelle vorhandene Dateien („lonely“) werden ausgeschlossen [Q-RB1 Zeile 1036]. `/XX`: zusätzliche Dateien am Ziel werden ausgeschlossen und nicht gelöscht [Q-RB1 Zeile 1032].
4. `/FFT`: „Assumes FAT file times (two-second precision).“ [Q-RB1 Zeile 1080]. `/DST`: „Compensates for one-hour DST time differences.“ [Q-RB1 Zeilen 1083–1084].
5. `/MT[:n]`: Kopien mit n Fäden; Standard 8; Bereich 1 bis 128 [Q-RB1 Zeilen 921–922].
6. Beispiel: `robocopy C:\Source C:\Destination /XC /XN /XO` überspringt Dateien, die am Ziel existieren, „regardless of whether they're newer, older, or modified“ [Q-RB1 Zeilen 1341–1342].

Randbedingungen und Garantien:
- Die Dokumentation nennt keine Prüfsumme, keine Vergleichsreihenfolge und keine Garantie bei Abbruch.
- Die Semantik von `/IM` ist auf Dateisysteme mit Änderungszeit begrenzt [Q-RB1 Zeile 1261].

Vergleich mit Smart Explorer (Schritt für Schritt):

| Eigenschaft | robocopy | Smart Explorer heute |
|---|---|---|
| Aufrufkette | Nicht dokumentiert (U11). | Wie 2.1. |
| Systemaufrufe und Roundtrips | Nicht dokumentiert. | Je Ordner Stat und Auflistung [I-WALK:74–92]. |
| Threads, Prozesse, Parallelität | `/MT` Standard 8; Bereich 1 bis 128 [Q-RB1 Zeilen 921–922]. | Fluss-Grenze [I-WALK:178–186]. |
| Puffer | Nicht dokumentiert. | 64 KiB [I-DIR:21]. |
| Synchronisierung und Sperren | Nicht dokumentiert. | Mutex und Condvar [I-WALK:110–117]. |
| Reihenfolge | Nicht dokumentiert. | Warteschlange [I-WALK:218–262]. |
| Warte- und Leerlaufzeiten | Nicht dokumentiert. | 50 ms; 250 ms [I-WALK:23, 27]. |
| Feste Werte | 8 Fäden; 2 s (`/FFT`); 1 h (`/DST`). | 512; 64 KiB; 50 ms; 250 ms. |
| Garantien | Nur die Optionssemantik. | Ganzer Durchlauf oder Fehler [I-WALK:48–61]. |

Befund: robocopy ist die Vorlage für die Begriffe der Ausschlussoptionen (`/XO`, `/XN`, `/XC`). Es zeigt außerdem, dass „gleiche Zeit, andere Größe“ als eigene Klasse behandelt wird.

### 3.8 Windows Explorer und macOS Finder: Zusammenführen von Ordnern

Windows Explorer [Q-MS1, Microsoft DevBlogs, Beitrag vom 2019-06-04, Textauszug Zeilen 731–756]:
- Bei gleichem Namen erscheint der Dialog „Replace or Skip Files?“ mit den Optionen „Replace the file in the destination“, „Skip this file“ und „Compare info for both files“ (Zeilen 731–740).
- „Compare info for both files“ zeigt Datum und Größe je Datei. Die Option, beide Dateien zu behalten, liegt unter diesem Vergleich (Zeile 741).
- Ein Kontrollkästchen „Skip N files with the same date and size“ steht im Vergleich (Zeile 756).
- Der Beitrag nennt keine automatische Regel „neuer ersetzt älter“.
- Es handelt sich um einen Blogbeitrag eines Microsoft-Autors, nicht um Produktdokumentation. Eine Support-Seite zu diesem Dialog wurde nicht gefunden (Suchauszug, nicht Volltext). Ob die aktuelle Windows-Version dieselben Optionen zeigt, ist nicht geprüft (U13).

macOS Finder [Q-AP1, Apple-Hilfeseite „Organize files in folders on Mac“, Abruf 2026-10-31]:
- Der abgerufene HTML-Quelltext enthält im Text keine Regel zum Zusammenführen. Das Wort „merge“ kommt in einer Zeile des Quelltexts vor; der Kontext ist nicht geprüft.
- Suchauszüge der Apple-Domain nennen: Zusammenführen per Option-Ziehen; „Merge“ erscheint nur, wenn eine Seite etwas hat, das die andere nicht hat; bei gleichnamigen Dateien mit unterschiedlicher Version nur „Stop“ und „Replace“. Diese Angaben stammen nicht aus dem Volltext und sind nicht geprüft (U14).
- Die Behandlung von „gleich“, „neuer“ und „älter“ im Finder ist nicht belegt (U14).

Vergleich mit Smart Explorer (Schritt für Schritt): Entfällt. Beide Systeme entscheiden über das Zusammenführen interaktiv; eine Vergleichsreihenfolge oder ein Aufrufweg sind nicht veröffentlicht.

### 3.9 Schneller lokaler Baum-Erfasser: WizTree und Everything über MFT und USN-Journal

Technik laut Herstellerdokumentation:
- Everything: indexiert Namen, Größen und Zeiten [Q-VF1 Zeilen 50–51]. Für NTFS-Volumes gilt: „Everything will automatically keep your NTFS indexes up to date with the NTFS USN Journal.“ [Q-VF1 Zeilen 63–64]. „Everything requires administrative privileges for low level read access to NTFS volumes for NTFS indexing.“ [Q-VF1 Zeilen 122–123]. Netzlaufwerke, NAS und Netzfreigaben werden über den Reiter „Folders“ als Ordnerindex aufgenommen [Q-VF1 Zeilen 91–96]; FAT-, FAT32- und exFAT-Volumes ebenso [Q-VF1 Zeilen 84–90].
- Everything: „A fresh install of Windows 11 (about 250,000 files) will take about 5 seconds to index.“ [Q-VF1 Zeilen 50–51]. Diese Zahl ist eine Herstelleraussage und nicht geprüft.
- WizTree: „high speed NTFS scanning is only possible when running with admin rights“ [Q-VF2 Zeile 93]. Verzeichnisindizes und Sicherheitsinformationen zählen nicht als Dateien, daher ist die Summe kleiner als „Space Used“ [Q-VF2 Zeilen 97–98]. Die Befehlszeile bietet einen MFT-Dump [Q-VF2 Zeile 54].
- Microsoft: FSCTL_ENUM_USN_DATA „enumerates the update sequence number (USN) data ... to obtain master file table (MFT) records“ [Q-MS8 Zeile 420]. Änderungsprotokolle können nur Administratoren anlegen, löschen und neu anlegen [Q-MS9 Zeile 432].

Ablauf (abgeleitet aus den Herstellerangaben): Volumen lesen (Administratorrecht), Dateieinträge lesen, Index speichern, Index über das USN-Journal fortschreiben. Welche Schnittstelle WizTree für die MFT verwendet, nennt die Quelle nicht (U12).

Garantien (Herstellerangabe, nicht unabhängig geprüft): Änderungen werden über das USN-Journal nachgeführt, auch wenn Everything nicht läuft [Q-VF1 Zeilen 63–64].

Vergleich mit Smart Explorer (Schritt für Schritt):

| Eigenschaft | MFT/USN (WizTree, Everything) | Smart Explorer heute |
|---|---|---|
| Aufrufkette | Volumen → Dateieinträge → Index; Fortschreiben über USN [Q-VF1 Zeilen 63–64]. | Ordner → Auflistung → Eintrag [I-LA-LINUX:23–67; I-LA-WDIR:102–127]. |
| Systemaufrufe und Roundtrips | Ein Lesen je Volumen (Herstellerangabe; Schnittstelle nicht genannt). | Je Ordner eine Auflistung; unter Windows Attributabfrage des Ordners plus Batches [I-LA-WDIR:136–147]. |
| Threads, Prozesse, Parallelität | Nicht genannt. | Bis 4 Fäden im Scanner [I-SC-SHARED:433–436]. |
| Puffer | Nicht genannt. | 64 KiB bis 1 MiB je Ordner [I-LA-WDIR:28–29]. |
| Synchronisierung und Sperren | Nicht genannt. | Atomare Zähler und Kanal [I-SC-WALK:324–383]. |
| Reihenfolge | Volumenweit. | Ordnerweit, Tiefensuche je Ebene [I-SC-WALK:40–75]. |
| Warte- und Leerlaufzeiten | Nicht genannt. | 150 ms Fortschritt [I-SC-SHARED:407]. |
| Feste Werte | Herstellerangabe 250 000 Dateien in etwa 5 s. | 1 000 000 Einträge; 128 MiB Namen [I-SC-BUDGET:3–5]. |
| Garantien | Admin-Recht nötig; nur NTFS für den Index; NTFS-Metadaten nicht als Dateien [Q-VF2 Zeilen 93, 97–98]. | Auslassungen gemeldet; kein Admin-Recht im Normalfall. |

Befund: Die Methode ändert die Kostenstruktur von „eine Auflistung je Ordner“ zu „ein Lesen je Volumen“. Sie verlangt Administratorrechte und gilt nur für NTFS (Abschnitt 5, Platz 11; Abschnitt 6, X9). Die gelesenen Module von Smart Explorer nutzen weder MFT noch USN (Abschnitt 2.8).

### 3.10 Parallele lokale Erfasser: fd (ignore), dua, dust, ncdu

| Eigenschaft | fd (ignore-Crate) | dua | dust | ncdu |
|---|---|---|---|---|
| Quelle (Commit) | fd `8b14e8dc` [Q-FD1]; ignore-Crate im Ripgrep-Repository `91f5d22e` [Q-IG1] | `bc03fedd` [Q-DU1] | `b6c6d8dc` [Q-DS1] | `1b3d0a67`, Version 2.9.2 [Q-NC1] |
| Auflistung | Standardbibliothek `read_dir` im Crate [Q-IG1 walk.rs:1616–1617]; fd baut den Walker mit `build_parallel` [Q-FD1 walk.rs:345–363, 400]. | `read_dir` für Metadaten-Aufträge [Q-DU1 lib.rs:1090]. | Eintrag mit `file_type` [Q-DS1 dir_walker.rs:415–430]. | Iterator über `openDirZ` mit `no_follow` [Q-NC1 scan.zig:242, 303]. |
| Metadaten je Eintrag | Nur auf Abruf: `metadata()` nur bei Bedarf [Q-IG1 walk.rs:61–62, 172–187]. | Metadaten-Aufträge, außer bei `skip_metadata` [Q-DU1 lib.rs:1078–1090]; Typ-Durchläufe ohne Metadaten [Q-DU1 lib.rs:1075]. | Ein Stat je Verzeichnis [Q-DS1 dir_walker.rs:322]; je Datei ein Stat für die Größe („as before“, Kommentar in process_entry) [Q-DS1 dir_walker.rs:415–440]. | `fstatat` je Eintrag [Q-NC1 scan.zig:50–52]. |
| Fäden und Standard | Festgelegt durch `threads(n)` [Q-FD1 walk.rs:400]; ohne Angabe `min(Kerne, 12)` im Crate [Q-IG1 walk.rs:1537–1539]. | 0 bedeutet Kernzahl außerhalb macOS [Q-DU1 options.rs:66–69; main.rs:740–742]. | Rayon-Pool; Größe nicht geprüft. | 1 Faden [Q-NC1 main.zig:83]; 0 bedeutet Kernzahl [Q-NC1 main.zig:533]. |
| Warteschlange und Synchronisierung | Kanal mit Kapazität 2 × Fäden [Q-FD1 walk.rs:621]; Sender-Fäden [Q-FD1 walk.rs:441–450]. | Arbeitsklau mit Parkierung und Ereignissen [Q-DU1 lib.rs:761, 809–813]. | Rayon-Scope; Sammeln der Kinder je Ordner [Q-DS1 dir_walker.rs:132, 387]. | Warteschlange LIFO mit Mutex und Bedingungsvariable [Q-NC1 scan.zig:100–141]. |
| Ebenenbarriere | Keine [Q-IG1 walk.rs:1653–1670]. | Keine [Q-DU1 lib.rs:1045–1050]. | Keine (verschachtelt) [Q-DS1 dir_walker.rs:132]. | Keine (LIFO) [Q-NC1 scan.zig:100–141]. |
| Stapelgröße je Faden | Nicht geprüft. | Nicht geprüft. | Nicht geprüft. | 128 KiB [Q-NC1 scan.zig:318–321]. |
| Garantien | Nicht geprüft. | Nicht geprüft. | Fehler je Eintrag gesammelt [Q-DS1 dir_walker.rs:390–392]. | Nicht geprüft. |

Befund: Keines der vier Programme liest Metadaten, die der Zweck nicht braucht. fd und das ignore-Crate lesen Metadaten nur auf Abruf, dua trennt Typ-Durchläufe ab, dust liest je Datei nur die Größe. Smart Explorer liest Größe und Zeit je Eintrag, weil der Vergleich beides braucht. Für den Vergleich gibt es deshalb keinen Gewinn durch verzögertes Lesen; es bleibt der Gewinn bei Namenslisten und Typ-Durchläufen (Abschnitt 5, Platz 9 bis 10).

Weitere Feststellung: ncdu 2.9.2 ist standardmäßig einfädig [Q-NC1 main.zig:83]. Hohe Parallelität ist bei den Programmen nicht die Voreinstellung, sondern eine Konfiguration.

### 3.11 Plattform-Primitive (Windows und Linux)

Windows:
- `FindFirstFileExW` mit `FIND_FIRST_EX_LARGE_FETCH`: „Uses a larger buffer for directory queries, which can increase performance of the find operation.“ [Q-MS5 Zeilen 472–474]. Smart Explorer nutzt diesen Aufruf in den gelesenen Modulen nicht; es nutzt `GetFileInformationByHandleEx` [I-LA-WDIR:102–127].
- `WIN32_FIND_DATA` enthält `ftLastWriteTime` und `nFileSizeHigh` [Q-MS6 Zeilen 459–469]. Größe und Zeit kommen mit dem Eintrag (abgeleitet: kein eigener Aufruf nötig).
- `FILE_ID_EXTD_DIR_INFO` enthält `ReparsePointTag` [Q-MS3]. Smart Explorer nutzt die Klasse `FileIdExtdDirectoryInfo` [I-LA-WDIR:189–200; I-LA-DREC:117–121].
- `FileNamesInformation` hat den Wert 12 [Q-MS4 Zeilen 434, 520]. Smart Explorer nutzt ihn für eingeschränkte Namen [I-LA-WNAMES:43–56].
- `ReadDirectoryChangesW`: Ist der Puffer zu klein, liefert der Aufruf null Byte; dann „you should compute the changes by enumerating the directory or subtree“ [Q-MS7 Zeilen 523–527].

Linux:
- `getdents64`: Dateityp in `d_type` nur bei manchen Dateisystemen; „Currently, only some filesystems (among them: Btrfs, ext2, ext3, and ext4) have full support“. Anwendungen müssen DT_UNKNOWN behandeln [Q-LX1 Zeilen 64–76]. Smart Explorer liest den Typ aus `d_type` [I-LA-LINUX:36]; die Standardbibliothek behandelt DT_UNKNOWN gesondert [Q-RU1 unix-fs.rs:2565–2566].
- `statx` mit `AT_STATX_DONT_SYNC`: „Don't synchronize anything, but rather just take whatever the system has cached if possible. This may mean that the information returned is approximate, but, on a network filesystem, it may not involve a round trip to the server“ [Q-LX2 Zeilen 129–140]. Smart Explorer nutzt diese Option nicht (Standardbibliothek).
- `inotify`: „the event queue can overflow. In this case, events are lost.“ [Q-LX3 Zeilen 326–336]; Ereignis `IN_Q_OVERFLOW` [Q-LX3 Zeile 200]; Grenzen `max_queued_events` und `max_user_watches` [Q-LX3 Zeilen 253–262].
- io_uring: Die UAPI-Liste enthält `IORING_OP_STATX` [Q-LX4 Zeile 278]. Eine Verzeichnisauflistungs-Operation enthält die Liste nicht [Q-LX4 Zeilen 257–321].

### 3.12 Zeitstempel: Genauigkeit, Zeitzone, Gleichheit

| System | Speicherung und Genauigkeit | Zeitzone | Beleg |
|---|---|---|---|
| FAT (Linux-Schreibpfad) | Änderungszeit auf 2 s gekürzt | lokale Zeit; Versatz aus Einhängeoption oder Systemzone | [Q-LNX2 misc.c:332, 202–206] |
| FAT (Microsoft, Windows) | lokal gespeichert | lokal | [Q-MS2 Zeile 826] |
| FAT32 und exFAT (Handbuch) | lokal gespeichert | lokal; Stundenversatz bei Sommerzeit | [Q-FF1 daylight-saving-time] |
| NTFS | 100-Nanosekunden-Intervalle seit 1601 | UTC | [Q-MS2 Zeilen 805, 826] |
| NFS (NFSv3) | Sekunden und Nanosekunden (`nfstime3`); „server times“ | Server wandelt lokal und UTC um | [Q-RFC1 nfstime3, Zeilen 1160–1172] |
| SMB | nicht gelesen (L16) | nicht gelesen | nicht geprüft |
| exFAT (Genauigkeit) | nicht gelesen (L16) | lokal laut Handbuch | nicht geprüft |
| Linux allgemein | In-Core-Zeitstempel können feiner sein als die gespeicherten; Git prüft Einträge mit racy Zeit inhaltlich | UTC | [Q-GIT1 Zeilen 36–50, 57–102] |

Vergleichsregeln der Quellen:
- rsync: Fenster 0 ist ganze Sekunden; negativ schließt Nanosekunden ein; 1 für FAT [Q-RS1 rsync.1.md:908–917; util1.c:1746–1753].
- rclone: Standardfenster 1 ns [Q-RC1 docs.md:2336–2345]; Präzision je Seite per Messung [Q-RC1 local.go:1033–1062]; Fenster ist die größte Präzision beider Seiten [Q-RC1 fs.go:100–115].
- Syncthing: Fenster je Ordner; Desktop-Standard 0 (Feld ohne Standardwert, [Q-ST1 folderconfiguration.go:78]); Android 2 s außer ext2, ext3, ext4 [Q-ST1 folderconfiguration.go:139–150]; Gleichheit: exakt oder Differenz kleiner als das Fenster [Q-ST1 bep_fileinfo.go:545–554].
- FreeFileSync: genau eine Stunde oder die konfigurierte Verschiebung; keine Toleranz [Q-FF1 daylight-saving-time].
- robocopy: `/FFT` für zwei Sekunden, `/DST` für eine Stunde [Q-RB1 Zeilen 1080, 1083–1084].
- Explorer: Vergleich nach Datum und Größe; eine Toleranz wird nicht genannt [Q-MS1 Zeile 756].
- Smart Explorer: Vergleich in Millisekunden ohne Fenster [I-DIR:227–231].

Befund: Eine einheitliche Genauigkeit gibt es nicht. Ein Vergleich über Systemgrenzen braucht daher ein Fenster je Seite (größte Präzision) und eine Zeitverschiebung für Sommerzeit getrennt von der Toleranz (Abschnitt 4.2).

## 4 Zusammenführen beim Einfügen

Fall: Quelle S und Ziel Z haben denselben relativen Pfad. Die Tabelle zeigt, was die Quelle in jedem Werkzeug bewirkt. „nicht belegt“ bedeutet: die gelesene Primärquelle nennt die Regel nicht.

### 4.1 Regeltabelle

| Fall | rsync | rclone | robocopy | FreeFileSync | Windows Explorer | macOS Finder | Syncthing (zwischen Geräten) |
|---|---|---|---|---|---|---|---|
| fehlend (nur S) | übertragen; `--update` prüft nur vorhandene Ziele [Q-RS1 generator.c:2176 `statret == 0`] | kopiert; `--update` überspringt nur vorhandene Ziele [Q-RC1 docs.md:3066–3069] | kopiert; `/XL` nicht gesetzt [Q-RB1 Zeile 1036] | Kopie in Spiegeln und Aktualisieren („add files“) [Q-FF1 synchronization-settings]; Zweiseitig über Datenbank (Auszug ohne Aktion) | kein Dialog ohne gleichen Namen [Q-MS1 Zeile 731] | nicht belegt (U14) | Index-Austausch [Q-ST3 Zeilen 248–256] |
| gleich (Name, Größe, Zeit im Fenster) | übersprungen (Quick-Check: gleich) [Q-RS1 generator.c:667–705] | gleich, übersprungen [Q-RC1 operations.go:288–306] | nicht kopiert ohne `/IS` (abgeleitet aus [Q-RB1 Zeilen 1043–1044]) | Kategorie „gleich“ [Q-FF1 comparison-settings] | Kontrollkästchen „Skip N files with the same date and size“ [Q-MS1 Zeile 756] | nicht belegt (U14) | unverändert, Blöcke übernommen [Q-ST1 walk.go:476–490] |
| neuer (Quelle neuer) | übertragen; Zeit abweichend, Quick-Check schlägt fehl [Q-RS1 generator.c:667–705] | kopiert, wenn ungleich; `--update` schließt nur neuere Ziele aus [Q-RC1 operations.go:1808–1822] | kopiert, weil `/XN` nicht gesetzt ist [Q-RB1 Zeilen 1023–1024] | Kategorie „links neuer“ bzw. „rechts neuer“; Aktion je Variante nicht im Auszug (L14) | Dialog mit Ersetzen als Wahl [Q-MS1 Zeilen 731–740] | nicht belegt (U14) | bei gleichzeitiger Änderung: die Datei mit der älteren Zeit wird umbenannt [Q-ST2 Zeilen 60–66] |
| älter (Quelle älter) | ohne `--update`: übertragen. Mit `--update`: übersprungen, wenn `src − dst < window` [Q-RS1 generator.c:2176–2177] | ohne `--update`: kopiert. Mit `--update`: übersprungen, wenn das Ziel um mindestens das Fenster neuer ist [Q-RC1 operations.go:1808–1822] | ohne `/XO`: kopiert (ersetzt das neuere Ziel). Mit `/XO`: nicht kopiert [Q-RB1 Zeilen 1027–1028] | Kategorie „rechts neuer“; Aktion je Variante nicht im Auszug (L14) | Dialog; keine Regel [Q-MS1] | nicht belegt (U14) | Konflikt nur bei gleichzeitiger Änderung [Q-ST2 Zeilen 60–66]; sonst nicht im Auszug |
| gleiche Zeit, verschiedene Größe | ohne Fenster: übertragen [Q-RS1 generator.c:671–672]. Mit Fenster > 0 und `--update`: übersprungen (abgeleitet, X3) | ungleich, kopiert [Q-RC1 operations.go:251–255]; `--update` ändert das nicht, weil das Ziel nicht neuer ist (abgeleitet) | `/XC` schließt aus; ohne `/XC` kopiert (abgeleitet) [Q-RB1 Zeilen 1019–1020] | Kategorie „Konflikt (same date, different size)“ [Q-FF1 comparison-settings] | Dialog; „Compare info“ zeigt Datum und Größe [Q-MS1 Zeilen 731–741] | nicht belegt (U14) | Gerätekennung entscheidet, nicht die Zeit [Q-ST2 Zeilen 64–66] |
| gleiche Größe, Zeit verschieden (außerhalb des Fensters) | übertragen; Quick-Check schlägt bei der Zeit fehl [Q-RS1 generator.c:667–705] | Prüfsumme; gleich: gleich, und die Zeit des Ziels wird gesetzt; ohne Prüfsumme: ungleich [Q-RC1 operations.go:308–318] | kopiert (kein Ausschluss, abgeleitet) | Kategorie „links neuer“ bzw. „rechts neuer“ (L14) | Dialog [Q-MS1] | nicht belegt (U14) | Zeit geändert: neu hashen [Q-ST2 Zeilen 34–38] |

### 4.2 Ungenaue Zeitstempel und ihre Wirkung auf die Regel

- FAT-Zeiten werden auf zwei Sekunden gekürzt (Abschneiden, nicht Runden) [Q-LNX2 misc.c:332]. Gegen eine NTFS-Quelle mit Millisekunden ergibt die Rundung eine Abweichung, die ohne Fenster „ungleich“ wird. Fenster 1 für rsync [Q-RS1 rsync.1.md:911–917]; `/FFT` bei robocopy [Q-RB1 Zeile 1080]; rclone nimmt die größte Präzision beider Seiten [Q-RC1 fs.go:100–115]; Syncthing: 2 s auf Android für Dateisysteme außer ext2, ext3, ext4 [Q-ST1 folderconfiguration.go:139–150]. Die Wirkung ist abgeleitet; sie ist nicht gemessen.
- Sommerzeit und Zeitzone: FAT speichert lokale Zeit, NTFS UTC [Q-MS2 Zeile 826; Q-FF1 daylight-saving-time]. Ergebnis: Abweichungen von einer Stunde nach Zeitumstellung [Q-FF1 daylight-saving-time]. Gegenmaßnahmen: `/DST` [Q-RB1 Zeilen 1083–1084] und die Zeitverschiebung von FreeFileSync [Q-FF1 daylight-saving-time]. Die Verschiebung ist keine Toleranz (Zitat in 3.6).
- NFS: Serverzeiten; die Genauigkeit ist im RFC nicht festgelegt (U21). SMB: nicht gelesen (U20).
- Gleiche Zeit und verschiedene Größe: Die Quellen sind sich in der Regel einig, dass die Größe entscheidet (rsync, rclone); sie unterscheiden sich in der Behandlung: Übertragen (rsync), Konflikt (FreeFileSync), Ausschluss nur auf Anforderung (robocopy `/XC`), Dialog (Explorer), Gerätekennung (Syncthing).

### 4.3 Vorschlag für die Entscheidung (Plan, Abschnitt C; nicht entschieden)

- M1 Gleichheit ohne Inhalt: Größe gleich und Zeit im Fenster. Fenster = größte Präzision der beiden Seiten. Belege: [Q-RC1 operations.go:288–306; fs.go:100–115]; [Q-ST1 bep_fileinfo.go:545–554].
- M2 Größe verschieden: ungleich, unabhängig von der Zeit. Belege: [Q-RS1 generator.c:671–672]; [Q-RC1 operations.go:251–255].
- M3 Größe gleich, Zeit außerhalb des Fensters: nur mit Prüfschlüssel gleich; ohne Prüfschlüssel ungleich. Belege: [Q-RC1 operations.go:308–318]. Option: bei Gleichheit die Zeit des Ziels anpassen [Q-RC1 operations.go:138–143].
- M4 Gleiche Zeit, verschiedene Größe: Konflikt, beide Fassungen behalten; kein Zeitgewinner. Belege: [Q-FF1 comparison-settings, Kategorie Konflikt]; [Q-RB1 Zeilen 1019–1020, eigene Klasse]; [Q-MS1 Zeilen 731–741]. Der Gewinner-Mechanismus von Syncthing hängt an Gerätekennungen (Abschnitt 6, X11).
- M5 „Neuer ersetzt älter“: nur, wenn das Ziel um mindestens das Fenster älter ist. Beleg: [Q-RC1 operations.go:1808–1822] (Bedingung `dt >= modifyWindow` für das neuere Ziel).
- M6 Kein Überspringen allein über das Zeitfenster. Überspringen nur über M1. Beleg: Abschnitt 6, X3.

### 4.4 Grenzen des Vorschlags
- Explorer und Finder: keine vollständige Primärquelle für die Regel (U13, U14).
- FreeFileSync: die Aktion je Kategorie und Variante steht nicht im gelesenen Auszug (L14).
- Die Behandlung von Konflikten (M4) ist eine Produktentscheidung; die Quellen belegen die Kategorie, nicht den besten Weg für Smart Explorer.

## 5 Techniken nach belegtem Beitrag (Rangfolge)

Maßstab „belegter Beitrag“: Eine Quelle belegt einen Mechanismus, der mindestens eine der Kostenklassen aus Abschnitt 2 entfernt.
- K1 Anfragen oder Aufrufe je Ordner (Stat, Auflistung).
- K2 zusätzliche Prüfungen je Eintrag (Pfad, Schutz, Identität).
- K3 Inhaltslesen für Prüfschlüssel.
- K4 Ebenenbarrieren und Leerlauf.
- K5 wiederholte Vollläufe ohne Änderung.

Rangfolge: nach der Zahl der Kostenklassen, die entfernt werden, und nach der Stärke des Belegs (Quelltext vor Handbuch vor Herstellerblog). Die Rangfolge ist eine Bewertung, keine Messung.

| Platz | Technik | Belege | Entfernt | Bedingung und Risiko | Ort in Smart Explorer |
|---|---|---|---|---|---|
| 1 | Seitenlokale Erfassung: die Seite, die die Dateien hält, liefert die Liste als Strom oder in einer Anfrage; der Vergleich läuft auf der Zielseite | [Q-RS1 flist.c:2647; options.c:119; generator.c:667–705]; Share-Anfragen für Analyse, Hash und Beobachtung [I-FSR:164–166, 206, 346, 356] | K1, K2, K4 | Alte Gegenstellen müssen weiter funktionieren (Fähigkeitsabfrage); Mount-Grenze, Filter, Auslassungen erhalten (das alte Protokoll kennt keine Mount-Grenze, [I-AGENT:20–24]); Abbruch mitten im Strom darf keinen Teilbaum als Basis liefern | `bisync/os/shared/snapshot_walk.rs`, `snapshot_agent.rs`, `share/core/fs_request.rs` |
| 2 | Metadatenvergleich vor Inhaltslesen: Größe, Zeit im Fenster, Rechte; Prüfschlüssel nur bei Zweifel | [Q-RS1 generator.c:667–705]; [Q-ST1 walk.go:476–490]; [Q-RC1 operations.go:251–318]; [Q-ST2 Zeilen 34–38] | K3, K5 | Gleichheit aus Metadaten ist nur so sicher wie Zeitstempel und Fenster (Abschnitt 4); Prüfschlüssel bei Zweifel und bei `Checksum` | `bisync/os/shared/snapshot_dir.rs:214–245` |
| 3 | Persistenter Metadaten-Cache je Datei mit Datei-Kennung und exakter Zeit; Schutz gegen racy Zeitstempel | [Q-MU1 scan.go:148–200]; [Q-UN1 fpcache.ml:243–258]; [Q-GIT1 Zeilen 57–102] | K3, K5 | Datei-Kennung muss je Backend verfügbar sein (`file_identity_v1`, [I-FSR:164–166]); racy-Regel: ein Eintrag gilt nicht, wenn seine Zeit nicht älter ist als der Erfassungszeitpunkt [Q-GIT1 Zeilen 84–100, abgeleitet] | `snapshot_dir.rs:227–231` (heute ohne Datei-Kennung) |
| 4 | Auflistung ohne Ebenenbarriere mit Arbeitsklau | [Q-IG1 walk.rs:1537–1539, 1667–1670]; [Q-DU1 lib.rs:1075–1090]; im SE-Bestand schon im Analysebackend [I-ANB:1–7, 131–143] | K4 | Gleiche Ergebnisreihenfolge muss erhalten bleiben (Sortierung nach Name, Dubletten) | `rscan/os/shared/parallel.rs:24–44` (Barriere) |
| 5 | Dateisystem-Beobachtung für Folge-Läufe, mit Vollscan als Sicherung | [Q-ST2 Zeilen 28–34] (Vollscan trotz Watcher); [Q-LX3 Zeilen 326–336] (Ereignisse verloren); [Q-MS7 Zeilen 523–527] (null Byte: auflisten); [Q-MU1 watch_non_recursive_linux.go:19–24] (50 Überwachungen) | K5, K4 | Vollscan nach jedem Überlauf und periodisch; Obergrenze der Überwachungen; Ereignisse gelten nicht als Wahrheit (X10) | Kein Watcher im gelesenen Umfang; Protokoll `WatchExport` vorhanden [I-FSR:146–151, 356] |
| 6 | Lokale rekursive Ansicht ohne Vollpfad und Arc-Felder je Eintrag, wenn nur Name, Größe und Summe gebraucht werden | Eigener Befund der Analyse: Namen, Größe und Kinder je Knoten [I-AN:8–14, 38–46] | K2 (Speicher) | Nur für Ansichten ohne Pfadbedarf; die Ansicht braucht den Pfad | `scanner/os/walk.rs:221–235` |
| 7 | Windows: Auflistung mit großem Puffer und Attributklasse, mit Reparse-Tag im Eintrag | [Q-MS3]; [Q-MS5 Zeilen 472–474] (`FIND_FIRST_EX_LARGE_FETCH` als Alternative) | K1, K2 | Bereits vorhanden; Änderung nur bei Messung nötig (keine Messung vorhanden) | `local_access/os/windows/directory.rs:28–29, 189–200` (beibehalten) |
| 8 | Linux: Auflistung in Inode-Reihenfolge und ein Stat je Eintrag; Speicher durch Strom statt Vollsammlung senken | [I-LA-LINUX:27–31] (coreutils, gnulib); [Q-RU1 unix-fs.rs:1049–1074] | K1 (Plattenzugriff), Speicher | Vollsammlung je Ordner braucht Speicher für alle Einträge [I-LA-LINUX:30] | `local_access/os/linux_os.rs:30–31` (beibehalten; Strom als Erweiterung) |
| 9 | Namenslisten (`FileNamesInformation`) für reine Vorhandenseinsprüfungen | [Q-MS4 Zeilen 434, 520]; [I-LA-WNAMES:43–56] (schon im eingeschränkten Pfad) | K2 (nur Vorhandensein) | Reicht nicht für Größe und Zeit | `local_access/os/windows/confined_names.rs` |
| 10 | Metadaten verzögert lesen: zuerst Typ, Metadaten nur, wenn sie gebraucht werden | [Q-IG1 walk.rs:61–62, 172–187]; [Q-DS1 dir_walker.rs:415–440]; [Q-DU1 lib.rs:1075] | K2 (nur ohne Vergleich) | Für den Vergleich braucht jeder Eintrag Größe und Zeit; Gewinn nur in Namens- und Typphasen (abgeleitet) | `local_access/os/linux_os.rs:36–46` |
| 11 | Lesen der MFT und des USN-Journals für lokale NTFS-Volumes | [Q-VF1 Zeilen 50–51, 63–64, 122–123]; [Q-VF2 Zeilen 93, 97–98]; [Q-MS8 Zeile 420] | K1, K4 bei Volumenerfassung; Herstellerangabe „5 s für 250 000 Dateien“ nicht geprüft | Administratorrecht; nur NTFS; NTFS-Metadaten sind keine Dateien (X9) | Neues Modul, nicht vorhanden |
| 12 | io_uring: Stat-Aufträge im Stapel | [Q-LX4 Zeilen 257–321, 278] | K2 (nur Stat) | Keine Verzeichnisoperation in der UAPI-Liste; Gewinn nicht gemessen; hoher Aufwand; Einsatz im Bestand nicht geprüft | `local_access/os/linux/` (neu) |

Befund zur Rangfolge: Platz 1 bis 3 beseitigen die größten Kostenklassen (K1 bis K3) für O1, O4 und O6. Platz 4 und 5 beseitigen K4 und K5. Platz 6 bis 10 betreffen die lokale Erfassung und die Speicherbelastung. Platz 11 und 12 sind bedingt oder unsicher.

## 6 Techniken mit belegtem Ausschlussgrund

| ID | Technik | Ausschlussgrund | Beleg |
|---|---|---|---|
| X1 | Vollinhaltsvergleich als Standard | Die Quelle nennt die Variante die langsamste; sie ist nur für Konsistenzprüfungen gedacht | [Q-FF1 comparison-settings, Variante 2] |
| X2 | Unison-Schnellprüfung als alleinige Änderungserkennung | Eine Änderung mit gleicher Zeit und Länge kann übersehen werden; der Experimentalmodus `fastercheckUNSAFE` kann Konflikte übersehen | [Q-UN1 manual 2380–2384; src/fpcache.ml:272–300] |
| X3 | Überspringen gleicher Dateien allein über das Zeitfenster | Mit positivem Fenster überspringt rsync `--update` eine Datei mit gleicher Zeit, ohne die Größe zu prüfen (abgeleitet: die Prüfung in generator.c:2176–2177 kehrt vor dem Größenvergleich in generator.c:671–672 zurück). Gleichheit ohne Größenprüfung ist damit falsch | [Q-RS1 generator.c:671–672, 2176–2177] |
| X4 | Blockprüfsummen allein für den reinen Vergleich | Jede Blockprüfung liest den ganzen Inhalt. rsync rechnet Blockprüfsummen erst bei abweichender Datei [Q-RS1 generator.c:667–705, sender.c:507]; Syncthing bildet Blöcke für die Übertragung [Q-ST1 walk.go:449–471] | [Q-RS1]; [Q-ST1] |
| X5 | `AT_STATX_DONT_SYNC` für den Vergleich | Die Werte können ungenau sein; auf Netzdateisystemen fällt kein Serverkontakt an, also keine Garantie für den aktuellen Stand | [Q-LX2 Zeilen 129–140] |
| X6 | rclone `ListR` (`--fast-list`) als Verfahren für Sync über die Protokolle von Smart Explorer | Die Quelle bevorzugt für `sync` und `copy` die Auflistung „List“, weil ListR nicht parallel läuft und nur bei bestimmten Backends vorhanden ist. SFTP v3 (Entwurf -02) kennt keinen rekursiven Befehl, nur OPENDIR und READDIR je Ordner (Textsuche ohne Treffer) | [Q-RC1 docs.md:3001–3031]; [Q-SFTP1 Zeilen 292–302, 821–824] |
| X7 | io_uring für die Verzeichnisauflistung | Die UAPI-Liste enthält keine Verzeichnisoperation; die Auflistung bleibt bei `getdents` und `readdir` | [Q-LX4 Zeilen 257–321] |
| X8 | Explorer-Konfliktdialog als Regelbasis für das automatische Zusammenführen | Der Dialog bietet Ersetzen, Überspringen und Vergleichen; eine Regel „neuer ersetzt älter“ nennt der Beitrag nicht. Der Dialog verlangt eine Entscheidung je Datei | [Q-MS1 Zeilen 731–741] |
| X9 | MFT- oder USN-Lesung als allgemeines Verfahren | Administratorrecht; nur NTFS für den Index; auf Netzlaufwerken und FAT-Volumes nur Ordnerindex (Reiter „Folders“) | [Q-VF1 Zeilen 84–96, 122–123]; [Q-VF2 Zeile 93] |
| X10 | Dateisystem-Beobachtung als alleinige Wahrheit | Ereignisse können verloren gehen (inotify, Überlauf); Puffer-Überlauf bei ReadDirectoryChangesW erzwingt Auflisten; Syncthing rät zu Vollscans auch mit Watcher | [Q-LX3 Zeilen 326–336]; [Q-MS7 Zeilen 523–527]; [Q-ST2 Zeilen 28–34] |
| X11 | Gerätekennung als Gewinnerregel bei gleicher Zeit für lokale Vergleiche | Die Regel gilt nur zwischen Geräten mit Gerätekennung. Im gelesenen Code von Smart Explorer gibt es keine Gerätekennung je Datei; die Regel ist daher ohne eigenen Konfliktzustand nicht anwendbar (abgeleitet) | [Q-ST2 Zeilen 64–66]; [I-DIR:52–57] |
| X12 | Interaktive Dialoge als Teil eines automatischen Zusammenführens | Explorer und Finder fragen je Datei; die Dialoge sind für eine automatische Ausführung ungeeignet | [Q-MS1 Zeilen 731–741]; [Q-AP1, Suchauszug] |
| X13 | Verschieben über Datei-Kennung ohne Datenbank | FreeFileSync erkennt verschobene Dateien nur mit Datenbank und Datei-Kennungen; ohne die Datenbank entfällt die Erkennung | [Q-FF1 synchronization-settings, „Detect Moved Files“] |

## 7 Nicht geklärte Eigenschaften

Jede Zeile nennt die fehlende Tatsache, den Grund und die Stelle, an der sie steht. „Leseumfang“ bedeutet: die Datei liegt außerhalb der Freigaben nach Abschnitt 0.4 (Abschnitt 8).

| ID | Fehlende Tatsache | Grund | Stelle |
|---|---|---|---|
| U1 | Welchen Prüfschlüssel-Modus der Knopf „Vergleichen“ setzt (`None`, `NativeOnly`, `Full` oder `FullFresh`) | Leseumfang | [I-DIR:221–243]; Setzung in L1 |
| U2 | Ob die Voreinstellung eines Sync-Jobs `cross_mounts = false` ist | Leseumfang; nur Plan-Angabe | [I-PLAN B2]; L2 |
| U3 | Fadenzahl `be.parallelism()` je Backend (lokal und remote) | Leseumfang | [I-SNAP:236]; L3 |
| U4 | Anzahl der Anfragen je Aufruf von `sync_stat`, `list_dir_tolerant`, `sync_child_path` und `snapshot_policy::protected` | Leseumfang | [I-WALK:74–92]; [I-DIR:120, 143]; L3, L4 |
| U5 | Abbildung von `analytics::os::read_directory` und Verteilung der Unterordner in `analytics_walk.rs` | Leseumfang | [I-AN:297–317]; L5 |
| U6 | Standardwerte des Knotenbudgets der Analyse | Leseumfang | [I-AN:29]; L5 |
| U7 | Sperr- und Wartezeiten in `source_media` (`acquire`, `GuardedIterator`, `probe`) | Leseumfang; wirkt auf jede Auflistung in `scanner` und `collect` | [I-SC-WALK:89–96]; L6 |
| U8 | Fadenzahl und Speicherbedarf des Hosts für `StorageAnalysis` | Leseumfang | [I-FSR:53–71]; L7 |
| U9 | Wirkung von `parallel_scan_allowed` und des Brokers bei Zugriffsverweigerung | Leseumfang | [I-LA-WREAD:9–52]; L8 |
| U10 | FreeFileSync-Algorithmus und Quelltext | Quelltext nicht gefunden. Die Suche ergab alte Pakete; die offizielle Quelle war nicht zu ermitteln | [Q-FF1]; L14 |
| U11 | robocopy-Aufzählung und Vergleichsreihenfolge | Herstellerdokumentation nennt nur Optionen | [Q-RB1] |
| U12 | Schnittstelle, mit der WizTree die MFT liest | Herstellerdokumentation nennt Administratorrecht und MFT-Dump, aber keine Schnittstelle | [Q-VF2 Zeilen 54, 93] |
| U13 | Windows-Regel für neuer und älter; Dialogauswahl der aktuellen Windows-Version | Keine Microsoft-Produktdokumentation gefunden; der einzige Primärbeleg ist ein Blog von 2019 | [Q-MS1] |
| U14 | Finder-Regel für fehlend, gleich, neuer und älter | Der Volltext der Apple-Seite enthält keine Regel; die Suchauszüge sind nicht geprüft | [Q-AP1] |
| U15 | Dauer des Polling-Intervalls im Polling-Pfad von Mutagen | Im geprüften Code keine Konstante gefunden; bekannt sind nur 15 s und 5 s | [Q-MU1 controller.go:28, 31]; L13 |
| U16 | Warteschlangen der Hash-Fäden (`walk.go:136–160`) und die Lesegröße in `Blocks` | Nicht gelesen | [Q-ST1 lib/scanner/blockqueue.go:40]; L11 |
| U17 | Welches Schnellprüfungs-Verhalten Unison auf Windows ausliefert (Handbuch: langsam; Quelltext: an) | Widerspruch im Quellmaterial; das Laufzeitverhalten der Auslieferung ist nicht geprüft | [Q-UN1 manual 2366–2378; update.ml:1375–1377]; L12 |
| U18 | Prüfung der Gesamtdatei nach der Übertragung bei rsync und die Abschlussumbenennung | Nicht gelesen | [Q-RS1 receiver.c:78–79, 322–410]; L10 |
| U19 | Zeitgenauigkeit von exFAT | Quelle nicht abgerufen; das Handbuch nennt nur die lokale Zeit | [Q-FF1 daylight-saving-time]; L16 |
| U20 | Zeitgenauigkeit von SMB-Servern | Quelle nicht abgerufen | L16 |
| U21 | Zeitgenauigkeit von NFS-Servern | RFC 1813 nennt die Felder, aber keine Genauigkeit des Servers | [Q-RFC1 nfstime3]; L16 |
| U22 | Vergleichsalgorithmus und Fadenzahl bei Mutagen (Scan-Schleife und Abgleich) | Nicht gelesen | [Q-MU1 core/scan.go]; L13 |
| U23 | Ort und Schreibweise der Basis in Smart Explorer nach erfolgreichem Lauf | Nur der Kommentar ist gelesen | [I-WALK:48–50]; L1 |
| U24 | Löschen ganzer Bäume und Sicherungssemantik | Außerhalb des Umfangs (Abschnitt 0.2) | — |
| U25 | Sperren und Wartezeiten der rclone-Prüfvergleiche (`checkers`) und Übertragungen (`transfers`) | Nicht gelesen | [Q-RC1 docs.md:1234–1246, 3056–3060] |

## 8 Leseumfang: Dateien, die zum Schließen der Lücken nötig sind

Die folgenden Dateien liegen außerhalb des freigegebenen Leseumfangs (Abschnitt 0.4). Der Koordinator entscheidet über den Zugriff. Jede Zeile nennt, welche Lücke sie schließt.

| ID | Datei oder Verzeichnis | Schließt | Grund |
|---|---|---|---|
| L1 | Dateien, die `HashMode` für den Knopf „Vergleichen“ setzen (Sync-Job und bisync-Auftrag) | U1; A6; U23 | Festlegung des Prüfschlüssel-Modus |
| L2 | `native/src/syncjobs/core/types.rs` (Zeile 227 laut Plan) | U2 | Voreinstellung `cross_mounts` |
| L3 | `native/src/vfs/**` (`sync_stat`, `list_dir_tolerant`, `sync_child_path`, `hash_walk`, `parallelism`) und die Bindung der Backends an Erlaubnisse | U3, U4 | Anfragen je Aufruf und Fadenzahl |
| L4 | `native/src/bisync/os/shared/snapshot_policy.rs` und `apply_boundary.rs` | U4 | Kosten der Schutzprüfung je Eintrag |
| L5 | `native/src/analytics/os/shared/analytics_walk.rs`, `analytics_budget.rs`, Plattformzuordnung unter `native/src/analytics/os/` | U5, U6 | Verteilung und Budget der Analyse |
| L6 | `native/src/source_media/**` (`acquire`, `GuardedIterator`, `probe`) | U7 | Sperren und Warten je Auflistung |
| L7 | Host-Implementierung von `StorageAnalysis`, `HashWalk`, `WatchExport` unter `native/src/share/**` | U8 | Fadenzahl und Speicher des Hosts |
| L8 | `native/src/local_access/os/windows/privilege.rs`, `broker.rs` | U9 | Privileg und Broker |
| L9 | `native/src/transfer/**` (Fluss: `flow.rs`, `flow_control.rs`, `flow_window.rs`, `flow_registry.rs`) | Erlaubnisse `acquire_meta`, `acquire_for` und Grenzen (Abschnitt 2.1) | Wirkung der Erlaubnisse |
| L10 | rsync-Quelltext: Abschlussumbenennung (Funktion, in `syscall.c` oder `receiver.c`) und Prüfung der Gesamtdatei (`receiver.c` nach `receive_data`) | U18 | Garantie Integrität und Absturzsicherheit |
| L11 | Syncthing: `lib/fs/walkfs.go` (rekursive Funktion `walk`), `lib/scanner/walk.go:136–160`, `lib/scanner/blockqueue.go` (`Blocks`) | U16; Abschnitt 3.3 | Auflistung je Ordner und Lesegröße |
| L12 | Auslieferungsstand von Unison: Standardwert der Präferenz `fastcheck` (`Prefs`-Definition) und Build-Konfiguration | U17 | Laufzeitverhalten |
| L13 | Mutagen: `pkg/synchronization/core/scan.go` (`Scan`, Zeilen 651 ff.), Polling in `controller.go` | U15, U22 | Polling-Intervall und Scan-Schleife |
| L14 | FreeFileSync-Quelltext (offizielle Quelle) und die Aktion je Kategorie im Handbuch | U10; 4.1 | Aktion je Kategorie und Variante |
| L15 | robocopy: keine öffentliche Quelle; nur das Handbuch | U11 | Nicht zugänglich |
| L16 | Herstellerdokumentation für exFAT und SMB (Zeitgenauigkeit), NFS-Serverangaben | U19, U20, U21 | Genauigkeit der Zeitstempel |

## 9 Quellen

Smart Explorer (Dateien im Repository, Stand des Arbeitsbaums bei Abruf; Commit `29310ace`, Zeilen beziehen sich auf den Arbeitsbaum):
- [I-PLAN] `docs/plaene/2026-10-31-sync-abgleich-schnell/plan.md`, Abschnitte A und B.
- [I-TE] `docs/TRANSFER_ENGINE.md`.
- [I-SNAP] `native/src/bisync/os/shared/snapshot.rs`; [I-AGENT] `snapshot_agent.rs`; [I-WALK] `snapshot_walk.rs`; [I-DIR] `snapshot_dir.rs`.
- [I-RSCAN] `native/src/rscan/os/shared/parallel.rs`.
- [I-ANB] `native/src/analytics/os/shared/analytics_backend.rs`; [I-AN] `native/src/analytics/os/shared/analytics.rs`.
- [I-FSR] `native/src/share/core/fs_request.rs`.
- [I-SC-MOD] `native/src/scanner/mod.rs`; [I-SC-ENTRY] `scanner/core/entry.rs`; [I-SC-RET] `scanner/core/retention.rs`; [I-SC-BUDGET] `scanner/os/budget.rs`; [I-SC-COLLECT] `scanner/os/collect.rs`; [I-SC-LINUX] `scanner/os/linux_os.rs`; [I-SC-RETRY] `scanner/os/retry.rs`; [I-SC-SHARED] `scanner/os/shared.rs`; [I-SC-WALK] `scanner/os/walk.rs`; [I-SC-WIN] `scanner/os/windows.rs`.
- [I-LA-MOD] `native/src/local_access/mod.rs`; [I-LA-LINUX] `local_access/os/linux_os.rs`; [I-LA-DH] `local_access/os/linux/directory_handle.rs`; [I-LA-WDIR] `local_access/os/windows/directory.rs`; [I-LA-DREC] `local_access/os/windows/directory_records.rs`; [I-LA-WNAMES] `local_access/os/windows/confined_names.rs`; [I-LA-WREAD] `local_access/os/windows/read.rs`.

Externe Quellen (Quellen mit Commit; Webseiten und Rohdateien mit Abrufdatum 2026-10-31). Lokale Kopien liegen nicht im Repository, sondern im Arbeitsverzeichnis der Recherche (Ordner `refs-abgleich`).

| ID | Quelle | Version, Commit oder Abruf | Gelesen |
|---|---|---|---|
| Q-RS1 | rsync, `https://github.com/RsyncProject/rsync` | Commit `8718cd28c542985129aa6f5375d970f3b9c86bb1` (2026-10-30); `version.h` nennt `3.5.1` | `generator.c`, `util1.c`, `options.c`, `flist.c`, `main.c`, `receiver.c`, `sender.c`, `io.c`, `rsync.h`, `pipe.c`, `rsync.1.md` |
| Q-RC1 | rclone, `https://github.com/rclone/rclone` | Commit `989fd82b1311148c96d3857f8f997266bcb9dc83` (2026-10-30); `VERSION` `v1.76.0` | `fs/operations/operations.go`, `fs/fs.go`, `fs/config.go`, `fs/march/march.go`, `fs/walk/walk.go`, `backend/local/local.go`, `backend/sftp/sftp.go`, `docs/content/docs.md` |
| Q-ST1 | Syncthing, `https://github.com/syncthing/syncthing` | Commit `ed38528d7fba3dd6a42c9c9fc6780af432dc15d2` (2026-10-27) | `lib/scanner/walk.go`, `blockqueue.go`, `lib/protocol/protocol.go`, `bep_fileinfo.go`, `bufferpool.go`, `lib/config/folderconfiguration.go`, `lib/model/model.go`, `lib/model/folder.go`, `lib/fs/basicfs.go`, `walkfs.go`, `tempname.go` |
| Q-ST2 | Syncthing-Handbuch „Syncing“, `https://docs.syncthing.net/users/syncing.html` | Abruf 2026-10-31 | Abschnitte zu Vollscan, Wiederscan und Konflikten |
| Q-ST3 | Syncthing-Protokoll BEP v1, `https://docs.syncthing.net/specs/bep-v1.html` | Abruf 2026-10-31 | `Index`, `IndexUpdate`, `Request`, `Selection of Block Size`, `modified_s` |
| Q-UN1 | Unison, `https://github.com/bcpierce00/unison` | Commit `8772d26a2eb77cfd0cb6da81807e81f24f51044c` (2026-09-20) | `doc/unison-manual.tex`, `src/update.ml`, `src/fpcache.ml`, `src/os.ml`, `src/fileinfo.ml`, `src/recon.ml`, `src/copy.ml`, `src/files.ml` |
| Q-MU1 | Mutagen, `https://github.com/mutagen-io/mutagen` | Commit `6ccfeaaf6dfd261e59ef9aac56e3c157b62e605b` (2026-04-22) | `pkg/synchronization/core/scan.go`, `controller.go`, `watch_mode.proto`, `rsync/doc.go`, `pkg/filesystem/directory_posix.go`, `watching/watch_non_recursive_linux.go`, `watching/watch_recursive_unsupported.go` |
| Q-FF1 | FreeFileSync-Handbuch, `https://freefilesync.org/manual.php?topic=<Thema>` (`comparison-settings`, `synchronization-settings`, `daylight-saving-time`, `performance`) | Abruf 2026-10-31; Versionsangabe auf der Seite nicht gelesen | Abschnitte wie in 3.6 |
| Q-RB1 | robocopy-Befehlsreferenz, `https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/robocopy` | Abruf 2026-10-31 | Optionsbeschreibungen, Beispiele |
| Q-MS1 | Microsoft DevBlogs, Raymond Chen, „In the file copy conflict dialog …“, `https://devblogs.microsoft.com/oldnewthing/20190604-00/?p=102539` | Beitrag vom 2019-06-04; Abruf 2026-10-31 | Textauszug Zeilen 731–756 |
| Q-MS2 | Microsoft Learn, `FILETIME`, `https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-filetime` | Abruf 2026-10-31 | Zeilen 805, 826 |
| Q-MS3 | Microsoft Learn, `FILE_ID_EXTD_DIR_INFO`, `https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_extd_dir_info` | Abruf 2026-10-31 | `ReparsePointTag`, Klasse `FileIdExtdDirectoryInfo` |
| Q-MS4 | Microsoft Learn, `FILE_INFORMATION_CLASS`, `https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/ne-wdm-_file_information_class` | Abruf 2026-10-31 | Zeilen 434, 520 |
| Q-MS5 | Microsoft Learn, `FindFirstFileExW`, `https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-findfirstfileexw` | Abruf 2026-10-31 | Zeilen 472–474 |
| Q-MS6 | Microsoft Learn, `WIN32_FIND_DATAW`, `https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-win32_find_dataw` | Abruf 2026-10-31 | Zeilen 459–469 |
| Q-MS7 | Microsoft Learn, `ReadDirectoryChangesW`, `https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesw` | Abruf 2026-10-31 | Zeilen 523–527 |
| Q-MS8 | Microsoft Learn, `FSCTL_ENUM_USN_DATA`, `https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_data` | Abruf 2026-10-31 | Zeilen 420, 438 |
| Q-MS9 | Microsoft Learn, `Change Journals`, `https://learn.microsoft.com/en-us/windows/win32/fileio/change-journals` | Abruf 2026-10-31 | Zeile 432 |
| Q-VF1 | Everything, FAQ, `https://www.voidtools.com/faq/` | Abruf 2026-10-31 | Zeilen 50–51, 55, 63–64, 75, 84–92, 122–123 |
| Q-VF2 | WizTree, FAQ, `https://diskanalyzer.com/faq` | Abruf 2026-10-31 | Zeilen 54, 93, 97–98 |
| Q-IG1 | ignore-Crate (Teil von ripgrep), `https://github.com/BurntSushi/ripgrep`, Pfad `crates/ignore/src/walk.rs` | Commit `91f5d22ef48bde459d90a8c7dbeab416e97d0f2b` (2026-10-27) | Zeilen 61–62, 172–187, 345–363, 701, 773, 1537–1539, 1616–1617, 1653, 1667–1670 |
| Q-FD1 | fd, `https://github.com/sharkdp/fd`, Pfad `src/walk.rs` | Commit `8b14e8dc5f1bd8c2656d735e80f1e6335b335bbf` (2026-10-30) | Zeilen 345–363, 400, 441–450, 621 |
| Q-DU1 | dua-cli, `https://github.com/Byron/dua-cli` | Commit `bc03feddbdd946c16dd234a28b377d43c0688597` (2026-10-31) | `crates/dua-lib/src/lib.rs` 761, 809–813, 1045–1090; `src/options.rs` 66–69; `src/main.rs` 740–742 |
| Q-DS1 | dust, `https://github.com/bootandy/dust` | Commit `b6c6d8dc6e6c6cf19a69265493bb4737f2dcab75` (2026-10-30) | `src/dir_walker.rs` 132, 322, 387, 390–392, 415–440; `src/node.rs` 38–45 |
| Q-NC1 | ncdu, `https://code.blicky.net/yorhel/ncdu.git` | Commit `1b3d0a670e730b338de6b716e2a60cf86351e44b` (2025-10-24), Version 2.9.2 | `src/scan.zig` 50–52, 100–141, 242, 303, 318–321; `src/main.zig` 83, 533 |
| Q-RU1 | Rust-Standardbibliothek, `https://github.com/rust-lang/rust`, Datei `library/std/src/sys/fs/unix.rs` | Rohdatei vom Zweig master, Abruf 2026-10-31; Commit nicht ermittelt | Zeilen 142–190, 1049–1100, 2565–2566 |
| Q-LX1 | `getdents(2)`, `https://man7.org/linux/man-pages/man2/getdents.2.html` | Abruf 2026-10-31 | Zeilen 64–76 |
| Q-LX2 | `statx(2)`, `https://man7.org/linux/man-pages/man2/statx.2.html` | Abruf 2026-10-31 | Zeilen 124–140 |
| Q-LX3 | `inotify(7)`, `https://man7.org/linux/man-pages/man7/inotify.7.html` | Abruf 2026-10-31 | Zeilen 200, 253–262, 326–336 |
| Q-LX4 | Linux-UAPI `io_uring.h`, `https://github.com/torvalds/linux`, Pfad `include/uapi/linux/io_uring.h` | Rohdatei vom Zweig master, Abruf 2026-10-31; Commit nicht ermittelt | Zeilen 257–321, 278 |
| Q-LNX2 | Linux-Kernel, `fs/fat/misc.c` | Rohdatei vom Zweig master, Abruf 2026-10-31; Commit nicht ermittelt | Zeilen 202–206, 332 |
| Q-GIT1 | Git-Dokumentation „racy-git“, `https://github.com/git/git`, Pfad `Documentation/technical/racy-git.adoc` | Rohdatei vom Zweig master, Abruf 2026-10-31; Commit nicht ermittelt | Zeilen 36–50, 57–102 |
| Q-RFC1 | RFC 1813 (NFSv3), `https://www.rfc-editor.org/rfc/rfc1813` | Abruf 2026-10-31 | Zeilen 1160–1172 (`nfstime3`) |
| Q-SFTP1 | SFTP-Protokoll, Entwurf `draft-ietf-secsh-filexfer-02`, `https://filezilla-project.org/specs/draft-ietf-secsh-filexfer-02.txt` | Entwurf -02 (nicht die aktuelle Fassung); Abruf 2026-10-31 | Zeilen 292–302, 821–824 |
| Q-AP1 | Apple-Hilfe „Organize files in folders on Mac“, `https://support.apple.com/guide/mac-help/organize-files-with-folders-mh26885/mac` | Abruf 2026-10-31; nur HTML-Quelltext, ohne Volltext zum Zusammenführen | Keine Regel im Text; Suchauszüge nicht als Primärquelle gewertet |

Hinweise zu den Quellen:
- Webseiten sind mit Abrufdatum belegt. Die Zeilenangaben beziehen sich auf den Textauszug, der aus dem HTML erzeugt wurde.
- Quelltexte sind mit Commit belegt. Für Rohdateien vom Zweig master (Rust, Linux, Git) ist kein Commit ermittelt; der Abrufzeitpunkt ist der 2026-10-31.
- Suchauszüge von Suchmaschinen (Windows-Dialog, Finder, FreeFileSync-Quelltext) sind keine Primärquellen und sind als solche gekennzeichnet.
- Die Version von FreeFileSync und die Ausgabe der Apple-Hilfe sind auf den gelesenen Seiten nicht angegeben.
