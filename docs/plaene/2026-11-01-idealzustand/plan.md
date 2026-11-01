# Idealzustand von Repo, Builds und Updates (2026-11-01)

Grundlage: Statusbericht vom 2026-11-01 (Befunde mit Läufen 41278089635, 41270618915,
41286102866; Maßnahmen M1–M8). Diese Datei führt Plan, Entscheidungen und Fortschritt.

## A — Erstplan aus dem Auftrag

Nutzer 2026-11-01:
- „gut. leite schritte ein Um möglichst den Idealzustand zu erreichen.“
- „Das Löschen von alten Builds aus dem Git-Repo ist erlaubt. diese sind auch nicht lokal in
  diesem repo zu sichern, gerne aber wo anders auf dem server bis ich mich entschieden habe ob und
  wo diese behalten werden.“
- „in dem zug wenn du das testest kannst du auch eine sache angehen die ich gefunden habe. das
  vergleichen von sync locations ist zu langsam. also es werden jetzt alle dateien zygig gefunden,
  aber der vergleich der einzelnen dateien dauert einfach zu lange. cpu und ram und datenträger
  haben noch n haufen kapazitäten, der vergleich läuft einfach extremst ineffizient. das muss auch
  so schnell wie möglich laufen. was iterativ ist so schnell wie möglich, bottelnecks finden und
  lösen, parallelisieren, multithreaden was auch immer, alle cores nutzen die verfügbar sind,
  sicherstellen dass einzelne langsamere teile schnellere nicht aufhalten. ggf ne [n]=>[2n]=>[n/2]
  struktur von mehreren einzelnen prozessen wenn schnellere und langsamere prozesse aufeinander
  volgen und voneinander abhängig sind. wobei die faktoren nicht fix sein sollten und es basierend
  auf welcher wartet entschieden werden sollte.“
- „halt. deine hauptaufgabe jetzt ist das projekt zum idealzustand zu führen. das ist was du jetzt
  planst und umsetzt. den fix lässt du dabei mit einfließen nicht anders herum.“
- Vorher: „ich hab kein problem damit wenn upldates über die release taggs statt raw access laufen,
  wenn das immer noch automatisch laufen kann via windows, linux und android“.

Grenzen: keine lokalen Builds oder Tests (AGENTS.md); ein Release am Ende des Stapels; alte Builds
nicht im Repo sichern, sondern außerhalb auf dem Server.

## B — Bestand (belegt)

- Git: Verlauf 7,25 GiB (GitHub size 7 597 511 KB), 97 % Binärdateien unter `release-native/`;
  aktueller Stand 3399 MiB, davon 116 Installer 2594 MiB. Wachstum 258–315 MiB Pack je Release.
- Release: 130,4 min Wand; Android 38,4 min vor Windows 91,9 min; Linux in WSL1 nach Windows;
  Windows `CARGO_BUILD_JOBS=1` + ThinLTO; Linux nproc + LTO aus
  (`native/publish-linux-feed-wsl.sh:19-23`) entgegen `docs/RELEASING.md` (Abschnitt Cargo-Leaf).
- Veröffentlichung: `native/release-publication.ps1` `Get-PublicationReleaseCommitPaths` committet
  Installer, portable Programme, Share-Server, DLL und Feed; der Lauf „Publish release candidate“
  veröffentlicht die committeten Bytes.
- Feed: `native/update_source.txt` = Raw-Feed in `main`; Desktop (`native/src/updater/os/shared/feed.rs`)
  und Android (`native/src/mobile/os/shared/domains/update.rs`) lesen ihn. `install-linux.sh` lädt
  bereits über `releases/latest/download`, schreibt aber `https://github.com/<repo>` als Quelle, die
  `normalize_http_feed` auf den Raw-Feed abbildet. Alle Feed-Dateien sind schon Release-Assets.
- Sync-Vergleich: `bisync/os/shared/orchestration_plan.rs` `prepare_spelled` prüft jede Datei der
  Liste `verify` (gleiche Größe, ohne Basis oder beidseitig geändert; `plan_decide.rs`
  `both_changed`/`worth_hashing`) nacheinander: Seite A, dann B, je stat → ganze Datei lesen → MD5
  → stat. Ein Thread, keine Sammelabfrage (`content_md5_many`) für Gegenseiten.
- Crate: 863 121 Zeilen in einem Crate; im Produktionscode 42 von 62 Top-Modulen in einem Zyklus.

## C — Entscheidungen

| Nr. | Entscheidung | Grund |
|---|---|---|
| E1 | Verlauf einmal umschreiben: `release-native/` aus allen Refs entfernen; danach den Raw-Feed v0.5.190 als ein Commit wieder in `main` | Nutzerfreigabe „Löschen von alten Builds“; installierte Updater lesen den Raw-Feed in `main` bis zur Brücke |
| E2 | Vollständige Sicherung vorher: Mirror aller Refs von GitHub außerhalb des Repos auf dem Server (Ort nur dem Nutzer berichtet), geprüft mit Ref-Vergleich und Objektvollständigkeit | Nutzervorgabe „wo anders auf dem server“; macht E1 umkehrbar |
| E3 | Commit-Zuordnung alt → neu als Datei im Repo (`docs/plaene/2026-11-01-idealzustand/commit-map.txt`) | Doku nennt viele alte Commit-IDs |
| E4 | Standard-Feed `https://github.com/b1ue-man/smart-explorer/releases/latest/download`; Raw-`main`-URLs und `https://github.com/<repo>` werden darauf abgebildet | belegt: `version.txt` über latest/download erreichbar; Nutzerfreigabe |
| E5 | Brücken-Release über die bestehende Pipeline (committet den Feed ein letztes Mal nach `main`) | nur so finden installierte Versionen ≤ 0.5.190 den neuen Updater |
| E6 | `graphify-out/graph.json` nicht mehr versionieren; lokal neu erzeugen | erzeugtes Artefakt, 79,6 MiB, wächst je Release |
| E7 | Sync-Vergleich: adaptive Prüfpipeline mit nebenläufigen Seiten, Sammelabfragen und Stufen, deren Arbeiterzahl der wartende Teil bestimmt | Nutzervorgabe i4–i6 |
| E8 | Veröffentlichung nur über Release-Assets und parallele Build-Jobs (M3) sowie Crate-Aufteilung (M6) im nächsten Kontingentfenster | Wochenkontingent zu 83 % verbraucht (Rücksetzung 2026-11-02 21:00 UTC); beide verlangen mehrere CI-Runden |

## D — Arbeitsplan und Fortschritt

| Punkt | Inhalt | Ergebnis (prüfbar) | Stand |
|---|---|---|---|
| D1 | Sicherung + Verlaufsbereinigung (E1–E3) | Mirror verifiziert; GitHub-Größe deutlich kleiner; `main` enthält nur den Feed v0.5.190 unter `release-native/` | erledigt 2026-11-01: Sicherung 267 Refs identisch, 0 fehlende Objekte; `git filter-repo --path release-native/ --invert-paths` über alle Refs; Pack 7,24 GiB → 342 MiB (davon Feed v0.5.190 und graphify-Verlauf); Feed v0.5.190 als Commit 2b73bbe1 mit denselben Blob-IDs; Force-Push 85 Branches + 182 Tags, Remote-Refs = Umschreibung; `commit-map.txt` hier |
| D2 | `graphify-out/` aus Git (E6); AGENTS.md (Graph lokal, einmalige Umschreibung) | `git ls-files graphify-out` leer | in Arbeit; `.gitignore` für Installer und portable Programme erst mit D8, weil die bestehende Pipeline sie für den Brücken-Release committet |
| D3 | Feed über Releases (E4) in Desktop, Android, `install-linux.sh`, README, Rollback-Pfaden | Tests: Normalisierung und latest-URL | umgesetzt (34336698, Doku README/RELEASING); `install-linux.sh` schreibt `https://github.com/<repo>`, das jetzt auf den Release-Feed normalisiert |
| D4 | Sync-Vergleich parallel (E7) für Vergleich, Lauf, Einfügen mit Zusammenführen und gleichartige Prüfwege | Suite-Messung: Vergleich gleicher Inhalte mit anderen Zeiten nutzt alle Kerne | umgesetzt (f04c4ac8): `verify_pool.rs` (Seiten parallel, Sammelabfrage je Ordner, `Pace`), Vorauslesen großer Dateien in `snapshot_hash.rs`; Einfügen mit Zusammenführen entscheidet über Ordnerschlüssel und Zeitfenster in der bereits parallelen Transfer-Engine (`transfer/os/shared/engine/update.rs`) und liest keine Inhalte |
| D5 | Schnelle Build-Gewinne: cargo-about vorgebaut, Lizenzliste einmal, einheitliche LTO-Angabe Linux/Windows | kürzere Release-Schritte | verschoben nach D8: jede Änderung an den Release-Skripten trifft den Brücken-Release, der ungeprüft nur einmal laufen kann |
| D6 | Eine Task-Suite für D3–D5, Lauf auf CI | grün | `.github/workflows/ideal-task.yml` + `native/test-ideal-task.py`; Lauf 41315575220 auf 77c274e1 |
| D7 | Brücken-Release, lokale Installation | Release v0.5.191 mit 26 Assets | offen |
| D8 | (nächstes Fenster) Veröffentlichung nur über Assets, parallele Jobs, Linux nativ (M3); danach Feed aus `main` | — | geplant |
| D9 | (nächstes Fenster) Crate-Aufteilung Phase 1 (M6), Messjob `--timings`/Speicher/`cargo bloat` (M4, M7) | — | geplant |
| D10 | Build-Einstellungen für CI-Runner (M4): Speicherspitze bei 2 und 4 Jobs messen, danach `CARGO_BUILD_JOBS` je Runner; eine LTO-Angabe für Windows und Linux, Laufzeitwirkung mit der Speed-Suite geprüft; cargo-about als fertiges Programm; Lizenzliste einmal je Lauf; Share-Server als Workspace-Mitglied | Windows-Schritt kürzer, Linux und Windows gleich optimiert | geplant (mit D8) |
| D11 | Android-ABIs arm64 und x86_64 als parallele Jobs statt nacheinander (heute 12,7 + 11,7 min) | Android-Job ≈ ein ABI lang | geplant (mit D8) |
| D12 | Suiten (M5): gemeinsame Einrichtung (Checkout, Toolchain, ein Abhängigkeits-Cache je OS ohne eigenes Crate) für alle Task-Workflows; Vergleichsversionen aus Release-Assets statt aus der Quelle bauen (in `ideal-task.yml` schon so) | weniger Cache-Verdrängung, ~12 min je Vergleichsbau gespart | geplant |
| D13 | Crate-Aufteilung weitere Phasen (M6): Zyklus von 42 Modulen lösen, Protokoll-Backends, Share, Transfer, GUI und CLI als eigene Crates | eigenes Crate je Bereich, Teil-Neubau bei Änderungen | geplant nach D9 |
| D14 | Binärgröße (M7): `cargo bloat`, zweiter Renderer (glow + wgpu) | kleinere Programme | geplant nach Messung in D9 |
| D15 | Repo-Ordnung: Task-Skripte aus dem Wurzelordner `native/` in einen eigenen Ordner, verwaiste Branches (`release/v*`, alte Arbeits-Branches) prüfen, Feed und Installer-Ausnahmen aus `main`/`.gitignore` nach der Brücke | übersichtlicher Bestand | geplant (Feed nach D8) |
| D16 | Signatur der Desktop-Updates (Windows/Linux prüfen heute nur SHA-256 aus derselben Quelle; Android hat die APK-Signatur) | Update nur mit gültiger Signatur | geplant, eigener Sicherheitspunkt |

## Kreative Schleifen

- S1 Prüfpipeline Sync-Vergleich (E7): Ziel i4–i6; Kriterien: alle Kerne/Datenträger ausgelastet,
  langsame Stufe hält schnelle nicht auf, Garantien (stat-Bindung, Abbruch, Auslassungen) gleich.
  Stand: Ansatz gewählt (Arbeiterpools je Seite und Stufe, Anpassung nach Wartezeiten); offen.
- S2 Release nur über Assets (E8/D8): Stand: Ansatz (Entwurf → Upload → Prüfen → Veröffentlichen)
  beschrieben; Umsetzung im nächsten Fenster.
