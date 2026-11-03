# Übergabe D13/D12 bis zum Release (Stand 2026-11-03)

Ziel: D13 (Crate-Aufteilung) und D12 abschließen. Die Remote-Suite `crates-task` wird grün, danach folgen
das Release über `.github/workflows/release.yml` und die Aktualisierung der lokalen Installationen. Plan:
`plan.md` in diesem Ordner, Abschnitte „D13 — Stand“ und Tabelle D12–D16.

Stand: `main` = `origin/main` = `ccf29ebe`. D13 und D12 sind umgesetzt. D14 ist geprüft, D15 entschieden
(Brücken-Feed bleibt), D16 entfällt. Neuestes Release ist v0.5.193, gebaut vor D13. Die Workspace-Version
ist 0.5.193.

## Arbeitsregeln

- Es gelten `AGENTS.md`, Abschnitte „no local builds or test execution“, „mandatory release“ und
  „native Rust architecture“.
- Lokal erlaubt sind ausschließlich statische Prüfungen: rustfmt über stdin, `cargo metadata --offline`,
  actionlint, Python-AST, `bash -n` und graphify `--code-only`.
- Kompilieren und Testen geschieht nur remote.
- Commits: Betreff endet auf `[skip ci]`, die letzte Zeile ist `[task candidate]`.
- Nie pushen, während ein Release-Lauf läuft.
- Push, gh-Aufrufe, rustfmt und graphify laufen über die Wege im Agenten-Gedächtnis:
  `repo-access-and-push`, `graphify-not-accessible`, `update-local-install-after-release`.
- Gerätedaten (Gerätenamen, Kennungen, PINs, lokale Pfade, Freigaben) gehören nie in Dateien oder
  Commits des Repos.
- Nutzervorgaben zu dieser Arbeit, wörtlich:
  - 2026-11-02: „D13 verlange ich vollständig OHNE zwischen release und tests zwischen den umbauten.“
  - 2026-11-02: „und was machst wenn er nach 4 stunden immer noch nicht fertig ist? ihn dann abbrechen
    unnd die gesamte zeit den abfluss runter spülen lassen? weil rechenpower kostenlos ist oder was?“
  - 2026-11-03: „4h lauf ist nicht akzeptabel. das entscheide ich jetzt so, 4h lauf für n fucking test
    ist nicht akzeptabel.“
  - 2026-11-03: „ach fuck it i guess test dann doch ABER FIND HERAUS WO DIE 4 h HER KOMMEN“

## Aufgabe 1: Ursache der rund 4 h im Windows-Job von `crates-task` finden und beheben

**Messwerte** (GitHub-API-Schrittzeiten und gespeicherte Logs):

| Lauf | Job / Schritt | Dauer | Ergebnis |
|---|---|---|---|
| `41437549919` | Windows `workspace (windows-2025)`, Schritt „Every crate, every test“ | 3:50:08 | Compile-Fehler (`E0599 write_all`, behoben in `93aa73fd`); kein Test lief |
| `41488739503` | derselbe Schritt | 3:21:24 | abgebrochen, ohne Ausgabe |
| `41437549919` | Linux, derselbe Schritt | 59:27 | — |
| `41488739503` | Linux, derselbe Schritt | 55:26 | — |
| `41488739503` | windows-gnu (alle Testprogramme für Windows, auf Linux gebaut) | 15:03 | — |
| `41393646281` (Release-Probelauf) | Windows-MSVC-Release-Builds | 23:18 (`se` + Updater), 16:30 (App), 4:04 (Extras) | — |

- **Zusammensetzung des Linux-Schritts** in Lauf `41488739503`:
  - Crate-Tests: 15 min 12 s Kompilieren, 1 954,5 s Testausführung über 17 Testprogramme;
  - Programme: 4 min 44 s Kompilieren, 163,7 s Testausführung.
- **Was der Windows-Log von Lauf `41437549919` zeigt:** Der Abhängigkeits-Cache griff. Neu kompiliert wurden nur:
  - die Workspace-Crates;
  - `noq`, `noq-proto` und `iroh` (vendored);
  - `egui_glow`, `toml_datetime`, `eframe`, `rmcp`, `iroh-relay`, `toml_edit` und `clap_complete`.
- **Testprofil:** `native/Cargo.toml` setzt in `[profile.dev]` `opt-level = 1`. Das Testprofil erbt das und
  baut „optimized + debuginfo“.
- **Folgerung, ungeprüft:**
  - Der Windows-Testbau braucht ein Vielfaches des Linux-Testbaus (15 min) und des Windows-Release-Builds (unter 24 min).
  - Die Ursache liegt also im Windows-Test- oder Debug-Pfad.
  - Kandidaten:
    - `link.exe` mit voller Debuginfo/PDB für große Testprogramme;
    - der doppelte Bau der Crates (einmal mit, einmal ohne Feature `test-support`, `native/test-crates-task.py`, `part_workspace`);
    - Doc-Tests;
    - Dateisystem und Virenscanner des Runners.
- **Vorgehen:**
  - Einen Windows-Diagnoselauf ansetzen, der nur kompiliert (`cargo test --no-run --timings`, dieselben Aufrufe wie `part_workspace`). Dabei gilt:
    - Jobgrenze höchstens 60 min;
    - Ausgabe live;
    - `target/cargo-timings/*.html` als Artefakt hochladen.
  - Die Zeiten je Einheit (Codegen gegen Link) auswerten.
  - Danach eine gemessene Gegenprobe der stärksten Ursache, zum Beispiel:
    - `CARGO_PROFILE_DEV_DEBUG=line-tables-only` oder `0`;
    - `rust-lld` als Linker.
  - Die Ursache beheben.
- **Mängel der Suite, mit beheben:**
  - `native/test-crates-task.py`, Funktion `run()`, fängt die Ausgabe mit `subprocess.run` ab und gibt sie erst am Ende aus. Ein Abbruch oder eine Zeitüberschreitung verliert dadurch alles. Die Ausgabe soll laufend in das Step-Log und in die Logdatei gehen.
  - Hängende Tests sollen einzeln nach einer Frist beendet und benannt werden, zum Beispiel mit cargo-nextest (`slow-timeout`, `terminate-after`). Mit Prozess-Isolation je Test entfällt auch `--test-threads=1`.
  - Die Zeitgrenzen der Jobs in `.github/workflows/crates-task.yml` nach gemessenen Dauern setzen. Heute stehen sie bei 240, 180, 120 und 60 min.
- **Abnahme:** Der Windows-Job von `crates-task` hat eine gemessene Dauer und Ursache. Kein Lauf verbraucht Stunden ohne verwertbares Ergebnis.

## Aufgabe 2: S09-Tests in `crates-task` überspringen

- Betroffen sind die fünf Tests `review_task_s09_transport_*` in `native/crates/engine/src/share/os/shared/lan_link_transport_task_tests.rs` und `lan_link_transport_task_lifecycle_tests.rs`.
- Sie brauchen den Netzwerk-Namensraum ihrer eigenen Suite. Auf dem CI-Runner mit mehreren privaten Interfaces lehnt das Produkt ab: „LAN-Link hat kein eindeutiges privates Interface“.
- `review_task_s09_transport` mit Begründung in die Liste `skips` von `part_workspace` in `native/test-crates-task.py` aufnehmen.

## Aufgabe 3: Acht Fehlschläge aus Lauf `41488739503` (Linux) beheben

Für jeden Test gilt:

- Testkörper und geprüfte Produktionsfunktion lesen.
- Entscheiden: Ist der Test veraltet? Dann an das heutige Verhalten anpassen oder löschen. Zeigt er einen echten Fehler? Dann den Code beheben.
- Die Kriterien stehen in `docs/TODO.md`, Zeile TESTS-BEWERTUNG.

| Test | Datei (Testanfang) | Fehlschlag |
|---|---|---|
| `drive_task_devices_offline_peer_does_not_block_host_start` | `native/crates/engine/src/mount/core/device_tree_tests.rs:120` | Zeile 136: `connects` ist 2 statt 1 nach dem Neustart des Hosts mit Gerät offline |
| `mount_optimization_task_speculative_admission_preserves_demand` | `native/crates/engine/src/mount/core/optimization_metadata_tests.rs:196` | Zeile 218: 2 statt 0 |
| `drive_task_vault_requires_unlock_after_crash` | `native/crates/engine/src/mount/os/shared/host_storage_tests.rs:196` | Zeile 225: `Some(Unknown)` statt `Some(RequiresUnlock { pending: 1 })` |
| `mount_recovery_cache_task_fallback_and_rw_keep_whole_file_semantics` | `native/crates/engine/src/mount/core/range_read_task_tests.rs:133` | Zeile 144: 1 statt 0 |
| `mount_recovery_cache_task_incomplete_audit_cannot_clean_from_drop` | `native/crates/engine/src/mount/core/recovery_cache_task_tests.rs:85` | Zeile 100: `spool::audit_recovery(...)` ist nicht `Err` |
| `batches_p13_linux_retirement_cleanup_preserves_foreign_stage_entries` | `native/crates/engine/src/mcp_launcher/os/linux_os/retirement.rs:134` | Zeile 151: `uninstall_handoff_stage_cleanup: …/stage: Directory not empty (os error 39)` statt `uninstall_handoff_stage_image_replaced` |
| `concurrent_fresh_share_status_commands_converge_on_one_worker` | `native/tests/se_cli/share.rs:209` | Zeile 248: 3 statt 1 abgelöste Singleton-Warter |
| `legacy_identity_requires_explicit_repair_and_reloads_the_replacement` | `native/tests/se_cli/share.rs:338` | Zeile 375: stderr enthält „se share identity --repair“ nicht |

**Befunde zum ersten Test:**

- `TestLink::connect` in `device_tree_test_support.rs:214-220` zählt jeden Verbindungsversuch, auch einen erfolglosen, bevor es den Online-Zustand prüft.
- Der Zählerstand wird vor `env.restart()` genommen. Damit zählt auch, was der alte Host beim Beenden tut.
- `ChildEngine::retry` (`device_tree_children.rs:161-174`) ruft `prepare_host_remote` auf.
- Die Hintergrund-Uploads (`upload_queue.rs`, `write_back.rs`) kamen am 2026-10-28 mit `70946d7c`, nach dem Test vom 2026-10-19 (`ce9db8c0`).
- Die Ursache des zusätzlichen Versuchs ist noch nicht belegt.

**Hinweis, ungeprüft:** Die mount-Tests liefen bisher nach Aktenlage nur in Windows-Suiten. Mit `crates-task` laufen sie erstmals unter Linux.

## Aufgabe 4: Zwei Fehlschläge per Sicherheitsumsetzung durch einen separaten Subagenten

Beide Aufgaben erledigt ein separater Subagent mit der Arbeitsweise `sicherheitsumsetzung`. Ziel ist dasselbe wie in Aufgabe 3: Test veraltet (anpassen oder löschen) oder echter Fehler (Code beheben).

| Test | Datei (Testanfang) | Fehlschlag |
|---|---|---|
| `vault_task_backend_identity_distinct_from_inner_and_other_vaults` | `native/crates/engine/src/vault/os/shared/backend_tests.rs:453` | Identitätsvergleich des Tresor-Backends schlägt fehl (Lauf `41488739503`, Log „Tests of every crate“) |
| `transfer_engine_task_sync_lists_destination_once_instead_of_stat_per_file` | `native/crates/transfer/src/sync/os/shared/sync_parallel_tests.rs:291` | Zähler `mkdir_all` ist 5 statt 0 |

## Aufgabe 5: `crates-task` einmal neu laufen lassen

- Erst nach den Aufgaben 1 bis 4 und einem Push.
- Ein Lauf: `crates-task.yml` per `workflow_dispatch` auf die volle SHA von `main`.
- Auswerten. Nur bei neuen Befunden beheben und denselben Lauf wiederholen.

## Aufgabe 6: Release

- Nach grüner Suite: `gh workflow run release.yml --ref main -f source_sha=<volle main-SHA> -f dry_run=false`. Ablauf und Prüfungen stehen in `docs/RELEASING.md` und in `AGENTS.md`.
- Erwartet wird v0.5.194 mit den 26 Assets.
- Den Lauf höchstens alle 30 min prüfen.
- Fertig ist das Release erst, wenn die GitHub Release sichtbar ist und `releases/latest/download/version.txt` die neue Version nennt.

## Aufgabe 7: Lokale Installationen aktualisieren

Nach dem Release, Weg und Prüfungen laut Agenten-Gedächtnis `update-local-install-after-release` und `server-desktop-access`. Nichts davon ins Repo schreiben.

## Aufgabe 8: Danach, laut `docs/TODO.md`

- **SUITES-ALT:** veraltete Testnamen und Signale älterer Suiten.
- **TESTS-BEWERTUNG:**
  - die übrigen Bereiche bewerten;
  - rund 280 Mehrfachprüfungen zusammenführen;
  - ein Register anlegen, welcher Test welche Anforderung schützt;
  - Tests mit Umgebungsabhängigkeit ohne `#[ignore]` behandeln.
- Die Bewertungsdaten vom 2026-11-02 liegen außerhalb des Repos. Ihre Ergebnisse stehen in der Zeile TESTS-BEWERTUNG.
