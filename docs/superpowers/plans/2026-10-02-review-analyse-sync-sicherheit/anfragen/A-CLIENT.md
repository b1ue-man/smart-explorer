# Anfragen A-CLIENT

Stand: 2026-10-03. Historische Teil-1-Anfragen mit dem Umsetzungsergebnis abgeglichen;
keine globale Planänderung. Externe Ausführung und optionale Leistungs-Erweiterung bleiben getrennt.

## 1. A35 Ortsidentität – erledigt in A-CLIENT

Die Freigabe für `app_models.rs`/`picker_impl.rs` ist erfolgt. `StorageScanSource::remote_at`,
Picker-Konto-/Präfixauflösung über `connect::saved_location`, direkte Analyse-/Reclaim-Aufrufer und
Explorer-Navigation sind verbunden. Bekannte Präfix-/Kontounterschiede verhindern die Wiederverwendung
eines fremden Explorer-Orts; Literalnamen und alte Konstruktor-Aufrufer bleiben erhalten.

## 2. Veraltete Listing-Assembly – erledigt

H-ANALYSIS hat den fremden Import/Test entfernt und bestätigt; A-CLIENT hat anschließend die
überholten `ChildMeta`-/`build_from_listings`-Helfer entfernt. Die neue ordnerweise Assembly und ihr
Aufbewahrungsbudget stehen in `analytics_backend.rs` samt zugehörigen Quelltests.

## 3. Begrenzte Bereichs-Lesungen – weiterhin optional, außerhalb des Vertrags

Vorschlag für K1/K2: `Backend::open_read_range(path,id,offset,len)` und additiver Share-
`ReadAt.len: Option<u64>`; das Agent-Protokoll hat `Read.len` bereits. Der bestehende Rückfall ist
über `open_read_at` und begrenzte Reads verbunden und liest nur Kandidaten gleicher Größe. Ohne
zusätzlichen Host-Längenparameter kann das Stromfenster mehr Daten vorladen als der Vergleich
verbraucht. Keine eigenständige VFS-/Share-API außerhalb des Scopes ergänzt.

## 4. Agent-Nutzlasten und übergebener Remote-Fehler – Ausführung beim Hauptagenten

Agent-Protokoll 11 ist im Quellvertrag verbunden (`special`, strukturierte Auslassungen,
`ReadyPartial`, sichere Hash-/Stage-Handles). Die eingebetteten `native/agent-bin/se-agent-*` müssen
über das bestehende `native/build-agent-bundles.sh` auf dem Remote-Runner mitgebaut werden. Keine
Nutzlast lokal gebaut und keine separate Zwischenveröffentlichung. Der letzte übergebene Remote-
Fehler `missing WireMeta.special` in `daemon/os/shared/backend_server.rs` ist im Scope korrigiert.

## 5. Gemeinsame Analysebudgets und Host-Retention – H-ANALYSIS

H-ANALYSIS liefert `Progress::{node_budget,set_node_budget}`, `AnalysisReceiver::with_node_budget`,
`AnalyticsBudget::for_progress` und `PlatformFigures/Approximations::estimated_heap_bytes`.
A-CLIENT bindet IPC, Agent-Legacy-Empfänger, Listing-Retention und Mobile-Ergebnisspeicher an diese
Signaturen. Host-/Peer-Scanner und Decoder im H-ANALYSIS-Scope bleiben dort verantwortlich;
A-CLIENT zählt keine Stub-Weiterreichung als Host-Funktion. Das Gesamtabnahmesignal ist derselbe
kleine Empfängeretat über Fassade → IPC → Peer → Host und im alten Baum-/Listing-Rückfall.

Letzte Integrationsmeldung erledigt: H-ANALYSIS setzt `Progress.directories_unreported` beim
Legacy→Scanning-Wechsel zurück. Die konkrete Rücksetzung ist im aktuellen read-only-Quellstand
statisch bestätigt; ein Laufnachweis bleibt Teil der gemeinsamen Remote-Abnahme.

## 6. Suite und Android-API-Zusammenführung

Die einzige kombinierte Remote-Task-Suite und der vollständige Remote-Release liegen beim
Hauptagenten. Die in `abnahme/A-CLIENT.md` genannten Quelltests/Umgebungssignale wurden lokal nicht
ausgeführt. Keine CI-/Release-Erfolgsbehauptung. AND-SHARE-UI übernimmt das additive Delta aus
`api-delta/A-CLIENT.md` in die zentrale API-Dokumentation.
