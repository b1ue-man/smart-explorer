# API-Delta A-CLIENT (Kotlin ↔ Rust, api.md §3/§4.9)

Stand: 2026-10-03, A-CLIENT angebunden; Remote-Abnahme steht aus. Kotlin-/IPC-Ergänzungen sind
additiv mit Legacy-Defaults. Das binäre Agent-Protokoll steigt auf 11; eingebettete Agent-Nutzlasten
müssen im vollständigen Remote-Release mitgebaut werden.

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
  Analysepfad, geschützte Bereiche des Hosts). Die App zeigt sie an
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


## `analyze.node {taskId, path}` – Host-Zahlen

- Additiv: `remote: Boolean = false`; an der Wurzel optional `volumeTotal: Long? = null` und
  `volumeFree: Long? = null`. Fehlende Host-Zahlen bleiben unbekannt. Entfernte Ergebnisse werden
  ausschließlich mit `ScanOutcome.volume/platform` angezeigt; die Platte oder Android-App-Statistik
  des empfangenden Geräts ergänzt sie nicht.
- Die vorhandenen `apps`-/`app`-Knoten tragen die Zahlen des analysierenden Hosts. Kotlin öffnet für
  einen entfernten App-Knoten keine lokalen Android-App-Einstellungen. App-, Rest- und geschützte
  Zeilen bleiben Ansichtszeilen und verändern weder Ergebnisbaum noch Dateizähler.
- Die Retentionsschätzung berücksichtigt beide aufbewahrten App-Listen (`PlatformFigures` und
  `Approximations`) samt String-/Vektor-Kapazitäten und die Diagnose-/Ortstexte.

## `reclaim.summary` / `reclaim.groups` – Papierkorbfähigkeit

- Summary additiv: `remote: Boolean = false`, `canRecycle: Boolean = false`,
  `recycleNote: String = ""`; bestehendes `protectedText` wird angezeigt. Der Kern prüft die aktuelle
  Host-Fähigkeit nach dem Suchlauf erneut. Eine verlorene Verbindung lässt den abgeschlossenen
  Bericht abrufbar und liefert `canRecycle:false` mit Hinweis.
- Gruppen additiv: `contentVerified: Boolean = false`. SHA-256-Gruppen sind für den
  inhaltsgebundenen Fern-Papierkorb geeignet; MD5-/Provider-Gruppen bleiben anzeigbar. Der Kern prüft
  zusätzlich den vollständigen 64-stelligen Hex-Hash und eindeutige Pfade.
- Ohne ausdrücklich bestätigtes `canRecycle` bietet die App keine Papierkorbaktion an, auch bei
  älteren Antworten ohne dieses Feld. Der aktuelle Kern liefert für lokale Ergebnisse weiterhin
  `canRecycle:true`; der vorhandene lokale Papierkorbpfad bleibt erhalten.

## `reclaim.recycle {taskId, locations:[String]}` / `reclaim.recycleResult {taskId}` (neu)

- `taskId` der Startanfrage bezeichnet das gespeicherte Duplikatergebnis; `locations` sind dessen
  vollständige Orte mit Backend-/Verbindungsidentität. Antwort: `{taskId, remote:true}` mit der neuen
  Aktion-ID. Kotlin `AnalyzeApi.reclaimRecycle` gibt diese ID zurück und meldet den CPU-Hold an.
- Der Kern reserviert das Ergebnis für eine Aktion, weist unbekannte/mehrdeutige Pfade oder
  unbestätigte Gruppen zurück und verlangt mindestens eine verbleibende Kopie je Gruppe. Jeder
  Host-Aufruf erhält erwartete Größe und SHA-256; geänderter Inhalt wird nicht verschoben.
- Der laufende Task meldet Fehler je Ort. Sein `result` enthält `{moved: Anzahl}`; die separate
  Ergebnisanfrage liefert `{moved:[vollständige Orte]}` (`RecycleResult.moved = emptyList()` als
  Decoder-Default). Diese Liste bleibt auch bei Teilfehlern oder Abbruch exakt abrufbar. Die App
  entfernt ausschließlich diese Orte aus der Anzeige und gibt das Aktionsergebnis danach frei.
- Erfolgreiche Verschiebungen aktualisieren zugleich das gespeicherte Suchergebnis. Wiederholte
  Aktionen können dadurch die verbliebene letzte Kopie nicht nachträglich auswählen. Aktive
  Papierkorb-Reservierungen werden vom Speicherdruck-Trimmer nicht entfernt.

## Native IPC, Agent und Desktop-Ortsidentität

- `IpcRequest::AnalyzeShare.node_budget: Option<u64>` ist additiv mit `serde(default)`; `None` nutzt
  das bestehende Empfängerbudget. Client und IPC-Worker frieren das angebotene Limit vor der Anfrage
  ein. `AnalysisReceiver::with_node_budget` prüft die angekündigte Form vor Allokation; IPC nutzt
  Deflate für den fertig übertragenen Baum.
- `Progress::{node_budget,set_node_budget}` erreicht den Listing-Walker über
  `AnalyticsBudget::for_progress`. Größen/Zähler bleiben vollständig; Detailknoten sind einschließlich
  Aggregatreserve begrenzt. Der ältere SSH-Baum-Walk registriert `TreeDecodeBudget` vor dem ersten
  Reply: Knoten, Text und Frame-Speicher sind vor Allokation begrenzt. Ein übergroßer Tree-Frame wird
  ohne Baum-Allokation geleert; nur diese Anfrage wechselt zu begrenzten toleranten Listen, die
  nächste multiplexierte Anfrage bleibt lesbar. Peer-/Host-Budgetierung gehört H-ANALYSIS.
- Agent-Protokoll 11: `WireMeta.special` im vorhandenen Flag-Byte (Link Bit 0, Special Bit 1).
  `WireChange.kind = 4` steht für `ReadyPartial`; `0` bleibt volle Bereitschaft. Unvollständige
  Watch-Abdeckung darf periodische Abfragen nicht abschalten. Der Agent-Handshake verlangt dieselbe
  Protokollversion; gemischte Agent-Binärversionen werden nicht stillschweigend dekodiert.
- `DirPart` und `DupGroup` werden in Portionen von höchstens 1 MiB übertragen. Aufeinanderfolgende
  Duplicate-Teile mit identischer Größe, Algorithmus, Hash und Evidenz bilden beim Empfänger wieder
  eine Gruppe; keine neue binäre Feldstruktur. Auslassungen und besondere Dateiklassen bleiben
  erhalten. Ein einzelner übergroßer Eintrag wird ausdrücklich abgewiesen.
- Desktop `StorageScanSource::Remote` behält additiv `endpoint_prefix`, `account`, `host_volume`,
  `host_platform`; `remote(...)` bleibt verfügbar, `remote_at(...)` übernimmt die Identität vom
  gemeinsamen `connect::saved_location`-/Picker-Vertrag. Fehlende Host-Figuren haben keinen lokalen
  Ersatz. Literalnamen mit Backslash werden im entfernten Wurzelpfad erhalten.

Die zentrale `api.md`-Zusammenführung bleibt beim vorgesehenen AND-SHARE-UI-Block. Dieser Bericht
beschreibt implementierte Aufruferverträge; er ist kein Nachweis ausgeführter Remote-Abnahme.
