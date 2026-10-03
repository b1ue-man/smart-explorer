# D-SYNCUI – Anfragen und verbleibende Abnahme

Stand: 2026-10-03. Desktop-Consumer quellseitig abgeschlossen, keine ungelöste
Implementierungsanfrage. Exaktes Lese-/Änderungs-/Neudatei-Inventar und statische
Evidenz: [Abnahme](../abnahme/D-SYNCUI.md).

Koordiniert geschlossene Fremdanschlüsse:

- Root: `bisync::preview_with(..., RunSettings)`, echter Jobowner
  (Handoff `977aa899`); keine Retag-/Planned-Rekonstruktion.
- E-APPLY: `pending_merge_for_key/pending_merge_relatives` für ursprüngliche
  Restart-Inputs; recorded Resolve/Merge und Versions-APIs konsumiert.
- E-APPLY: `SyncHandle::take_worker(&mut self) -> Option<JoinHandle<()>>` für Mirror,
  gemäß Parent-Handoff geliefert.
- Root: eigener `app/os/shared/sync_exit_gate.rs` und äußere
  `dialogs/frame_update/shutdown`-Änderungen konsumieren die drei Trackingmethoden.
  Explizite Abbruchwahl, 10s-Frist bleibt bei lebenden Workern im Fenster.
  Diese fremden Dateien wurden hier nicht erkundet oder geändert.

Ausstehend ist ausschließlich die zentrale Integration/Remote-Abnahme nach allen
Engine-/Consumer-Quellen; genaue Signale/Testnamen in der Abnahme. Kein zusätzlicher
lokaler Prüfauftrag, keine globalen Plan-/Engine-/Protokoll-/Android-/Share-/Releaseänderungen.

