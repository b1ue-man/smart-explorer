# H-ANALYSIS API-Übergabe

Stand 2026-10-03; eigene Umsetzung statisch abgeschlossen, Remote-Abnahme ausstehend.

- Host-Funktionen: `storage_snapshot::serve(send, root, access, principal)`; Task-, Hash-, Listen- und Watch-Ströme prüfen denselben autorisierten `FsAccess`. Retention hält den vollständigen Principal und die Rechtebindung, entfernt nur Transport-Cancel.
- Empfängerbudget: `Progress::node_budget/set_node_budget`, `AnalysisReceiver::with_node_budget`, `AnalyticsBudget::for_progress`. `PlatformFigures` und `Approximations` liefern `estimated_heap_bytes`.
- Draht: `FsDuplicateGroup.more` und `FsWatchEvent::Ready.complete` haben Default false; `ChangeNotice::ReadyPartial` behält Hybrid-Abfragen. `FsHostFeatures::host` verwendet die zentrale tatsächliche OS-Papierkorbfähigkeit.
- Sichere lokale Scans öffnen relative Kinder am gehaltenen DirectoryHandle. Engine-/Stage-/Papierkorbnamen werden vor Öffnung ausgelassen. Linux/Android-Papierkorb benutzt geprüfte Quarantäne und veröffentlichte Wiederherstellungsdaten.
- Android-Produzent: `analytics::remember_platform_totals(volume, totals)` verbindet aktuelle eigene Plattformzahlen ausschließlich mit dem tatsächlichen Host-Primärvolumen.

Windows-Papierkorbanschluss und Android-Produzent sind getrennte Anschlussblöcke; Rechte-/Lease-Masken in server_capabilities.rs liegen jetzt bei H-DISPATCH. Details und Fälle: [Abnahme](../abnahme/H-ANALYSIS.md), [Anfragen](../anfragen/H-ANALYSIS.md).
