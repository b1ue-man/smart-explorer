# API-Delta A-CLIENT (Kotlin ↔ Rust, api.md §3/§4.9)

Stand: 2026-10-02, Teil 1 (FA1, Duplikat-Rückfall). Alles additiv; ältere Antworten ohne die neuen Felder
dekodieren weiter (Kotlin-Defaults).

## `analyze.start` / `reclaim.start`
- Antwort neu: `{taskId, remote: Boolean}`. `remote = true`, wenn der Ort kein lokaler Pfad ist (Share,
  SFTP, FTP, WebDAV, SMB, Drive). Die App hält dann für die Dauer des Tasks einen Partial-Wakelock
  (`TaskForegroundService`, Tag `SmartExplorer:remote-task`, `acquire(10 min)`, alle 60 s erneuert,
  sofort freigegeben, wenn kein solcher Task mehr läuft).
- `analyze.start` mit entferntem Ort läuft wie am Desktop über `analytics::scan_remote`: der Host analysiert
  selbst (Share: Host-Worker über den eingebetteten Dienst), SSH-Agent läuft serverseitig, nur Backends ohne
  beides werden von hier aus gelistet. Vor dem Start wird die gepoolte Verbindung geprüft und bei Verlust
  neu geöffnet (`resolve_live`); das gilt auch für `reclaim.start`.
- Fortschritt (Task-Felder) bei entferntem Ort:
  - `message`: eine Tatsache je Zeile – Phase (`Verbindung zur Gegenstelle wird vorbereitet …`,
    `Wartet auf einen freien Analyse-Worker der Gegenstelle`, `Gegenstelle durchsucht · N Ordner` + aktueller
    Ordner, `Gegenstelle stellt das Ergebnis zusammen`, `Ergebnis wird übertragen` + `X von Y · P %`,
    `Empfangenes Ergebnis wird geprüft`, `Älterer Analysepfad der Gegenstelle: …` + Ordner), dazu
    `Letzte Meldung der Gegenstelle vor N s`, wenn die Gegenstelle ≥ 5 s schweigt. Lokal unverändert
    `N Ordner · Ordner`.
  - `doneItems` = erfasste Dateien; `doneBytes` = erfasste Bytes und `totalBytes = 0` – außer während der
    Ergebnisübertragung: dann `doneBytes`/`totalBytes` = übertragene/gesamte Ergebnisbytes (Übertragungs-
    balken). Am Ende wieder die erfassten Bytes.
- `result` von `analyze.start` additiv: `notes` (Anzahl der Hinweise).

## `analyze.issues {taskId}`
- Antwort: `{count, text, notes: [String], protectedCount, protectedText}`.
- Geändert: `text` enthält nur noch die Leseprobleme (früher hingen die Hinweise an); die Hinweise stehen
  in `notes` (Hinweise des Walks oder des analysierenden Geräts: zusammengefasste Detailansicht, älterer
  Analysepfad, geschützte Bereiche des Hosts, solange K2 sie noch als Hinweis sendet). Die App zeigt sie an
  der Wurzel auch ohne Leseprobleme.

## `analyze.release {taskId}` / `reclaim.release {taskId}` (neu)
- → `{released: Boolean}`. Gibt das aufbewahrte Ergebnis (Analyse oder Duplikatsuche) sofort frei – auch
  das künftige eines noch laufenden Tasks; `false`, wenn nichts aufbewahrt war. Danach liefern
  `analyze.node/issues`, `reclaim.groups/summary` `not_found`.
- Die App ruft es beim Start einer neuen Suche/Analyse für die bisherige, bei „Anderer Ort“ und beim Ende
  der Activity (`onCleared`).
- Aufbewahrung im Kern: statt fester 4 Plätze bleiben fertige Ergebnisse, bis sie freigegeben werden oder
  der Speicher knapp wird (Schätzung je Ergebnis; Budget = `transfer::memory_budget()`, ¼ des freien
  Speichers, 64 MiB … 2 GiB); dann gehen die ältesten fertigen zuerst, das neueste bleibt immer.

## `reclaim.start` (entfernter Ort) / `reclaim.summary`
- Entfernte Orte: jede Datei ≥ `minSize` ist Kandidat (keine 200er-Kappung mehr), alle Gruppen werden
  geliefert. Ohne Prüfsumme des Backends (Share-Host ohne Host-Suche, FTP, SMB, WebDAV, SFTP ohne Agent)
  werden nur gleich große Kandidaten gelesen: erst Anfang/Ende (SHA-256 wie lokal), nur bei Gleichstand der
  ganze Inhalt. Share-Orte nutzen nie mehr den Hash-Walk des Hintergrunddienstes (der jede Datei
  herunterlud).
- `reclaim.summary.compared` (schon im Kern vorhanden) wird jetzt in Kotlin dekodiert und angezeigt
  („… ab 1 MB, N gleich große verglichen“). `limit` enthält je Zeile einen vom Kern formulierten Grund
  (Walk-Grenze, „Kandidatenspeicher (64 MiB) ausgeschöpft: die N kleinsten Dateien nicht verglichen“); die
  App zeigt ihn unverändert unter „Ergebnis unvollständig“.

## Kotlin-Datenklassen (`api/AnalyzeApi.kt`)
- `AnalyzeIssues.notes: List<String> = emptyList()`, `ReclaimSummary.compared: Long = 0`.
- `AnalyzeApi.release(taskId): Boolean`.
- `AnalyzeApi.start`/`reclaimStart` geben weiter die Task-ID zurück und melden Fern-Tasks selbst bei
  `TaskKeeper.keepCpuAwake(taskId)` an.
