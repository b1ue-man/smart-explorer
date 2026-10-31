# Rekursive Ansicht und `se ls --recursive`

Stand 2026-10-31. Die rekursive Ansicht zeigt alle Einträge unterhalb eines Ordners in einer Liste: am Desktop mit Strg+R, auf Android mit dem Schalter „Rekursiv“, in der CLI mit `se ls -R`. Jede Zeile belegt die Fähigkeit mit der Codestelle (`Datei:Zeile`). Lücken stehen unten.

| Oberfläche | Einstieg | Belege | Stand |
|---|---|---|---|
| Desktop-GUI (Windows, Linux) | Strg+R schaltet die Ansicht um und startet den Scan neu; ohne Filter der ganze Baum, mit Filter die gefilterte Ansicht | Taste `native/src/app/core/frame_keyboard.rs:193-194` (`KbdAct::ToggleRecursive`), Umschalten und Neustart `:309-311`; Scanstart lokal `native/src/app/core/scanning.rs:104` (`scanner`), entfernt `:97` (`rscan`) | vorhanden |
| Android-App | Schalter „Rekursiv“ im Filterpanel; der Zustand setzt zugleich das Panel offen | Schalter `android/app/src/main/java/app/smartexplorer/android/ui/files/FilterBar.kt:69-70`; `setRecursive` in `ui/files/BrowserTab.kt:121-128`; Fensteransicht des Scans `ui/files/ScanWindow.kt:30`; Bridge `native/src/mobile/os/shared/scan.rs:90-95` (`scan.start`, `scan.view`) | vorhanden |
| CLI `se`, Text | `se ls -R ZIEL`; je Zeile: Art, Linkmarke, Größe, Zeit in Millisekunden, Pfad relativ zum Ordner | Befehl `native/src/cli/mod.rs:168-169` (Hilfetext), Argumente `:214-229` (`LsArgs`, Schalter `-R`), Verzweigung `:374-378`; Ausgabe `native/src/cli/ls_recursive.rs:11` (`list`), `:120` (`print_line`) | vorhanden |
| CLI `se`, JSON | `se ls -R ZIEL --json`; `--json` verlangt `-R` | Argument `native/src/cli/mod.rs:223-229`; Zeile je Eintrag `native/src/cli/ls_recursive.rs:131` (`row`, Felder `path`, `is_dir`, `is_symlink`, `size`, `mtime_ms`, `hidden`) | vorhanden |
| MCP `se mcp` | Das Werkzeug `se` führt jeden CLI-Befehl aus, also auch `ls -R` und `ls -R --json` | Werkzeug `native/src/cli/mcp/catalog.rs:183`; Beschreibung „runs any other Smart Explorer terminal command“ `:42` | vorhanden über das allgemeine Werkzeug; kein eigenes Baumwerkzeug |

## Gegenstück am Netz

Ein entferntes Ziel nutzt denselben Scan über `rscan` (Start `native/src/app/core/scanning.rs:97`). Wo die Gegenseite den Durchlauf des ganzen Baums in einer Anfrage anbietet, nimmt die Ansicht ihn: `native/src/rscan/os/shared/rscan.rs:97,140` (`whole_tree`), `native/src/rscan/os/shared/hash_walk_view.rs:31` (`walk_in_one_request`). Ohne dieses Angebot listet sie Ordner für Ordner. Der lokale Scan folgt keinen Verknüpfungen (`follow_symlinks: false` in `native/src/cli/ls_recursive.rs:17` und `native/src/app/core/scanning.rs:107`).

## Verhalten, das alle Oberflächen gleich halten

- Nicht lesbare Pfade gehen an die Fehlerausgabe; die CLI endet dann mit Status 1 und behält die vorher gelesenen Einträge (`native/src/cli/ls_recursive.rs:11`, Funktion `list`).
- Ein Scan am Budgetende meldet das unvollständige Ergebnis (CLI-Hinweis `native/src/cli/ls_recursive.rs:68-69`).
- Die relativen Pfade der CLI nutzen die Wurzel, die der Scanner selbst meldet (Eintrag der Tiefe 0, `native/src/cli/ls_recursive.rs:43-45`). Sie tragen `/` zwischen den Bestandteilen auf allen Systemen (`native/src/cli/ls_recursive.rs:84`, `relative`).

## Lücken und Ausnahmen

- Fehlende Oberfläche: im Quellstand keine der vier genannten Oberflächen (Desktop, Android, CLI Text und JSON, MCP). Nicht geprüft sind Oberflächen außerhalb dieser Tabelle (Share-Fenster, se Desktop, se Launcher); dazu trifft dieses Dokument keine Aussage.
- Ausnahme: `se ls -R` gibt den Wurzelordner nicht aus; die Ausgabe enthält nur Einträge unterhalb der Wurzel (Modulbeschreibung `native/src/cli/ls_recursive.rs:4`).
- Nicht geprüft: welche Spalten die Desktop-Ansicht und die Android-Ansicht je Zeile zeigen; dieses Dokument belegt nur die CLI-Spalten.
- Offen (Befund G5 des Sync-Abgleich-Reviews, `docs/plaene/2026-10-31-sync-abgleich-schnell/review.md`): Über den Hintergrunddienst bleibt das Versteckt-Merkmal von SSH-Agent-Einträgen ohne Punkt-Regel; die Wirkung von `include_hidden = false` dort ist nicht vollständig geprüft.
