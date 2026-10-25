# Übergabe an einen anderen Agenten (Stand 2026-10-25)

Anlass: Der Nutzer gibt die Umsetzung wegen Nutzungsgrenzen an Codex ab (2026-10-25: „ich plane aufgrund der usage limits die umsetzung an codex abzugeben“). Dieses Dokument enthält, was bisher nur im lokalen Gedächtnis von Claude stand und für die Arbeit an diesem Repository gebraucht wird.

Gerätebezogene Betriebsschritte des Nutzers stehen nach der Projektregel nicht hier. Dazu gehören Updates der lokalen Installation, Gerätekopplungen und Rechte. Sie liegen in einer Notiz außerhalb des Repositorys; der Nutzer gibt sie weiter.

## Stand

- Letztes Release: 0.5.184 (`native/Cargo.toml`, `release-native/update-feed/version.txt`).
- Nichts aus P1 bis P31 ist begonnen. Der Nutzer sagte am 2026-10-24 zweimal „noch nicht starten“ und dann „nur dass es klar ist, noch immer nicht starten mit der musetzung“. Den Start gibt der Nutzer.

## Lesereihenfolge

1. `AGENTS.md`: Projektregeln. Keine lokalen Builds und Tests; eine Remote-Task-Suite; ein Release je Batch.
2. `docs/ARCHITEKTUR.md`: Architektur. Im Abschnitt „Zweck“ steht das Kernprinzip direkt zwischen den Geräten und die langfristige Richtung.
3. `docs/plaene/2026-10-24-batches/batches.md`: vorläufiger Zuschnitt in fünf Batches. Er ist ausdrücklich ein Vorschlag; Zuschnitt und Agenten werden erst nach der Planung festgelegt.
4. Die Machbarkeitsnotizen mit Befunden, Quellen, Lücken und den offenen Entscheidungen:
   - `docs/plaene/2026-10-22-fernzugriff-desktop/machbarkeit.md`: P1 bis P9 und den Nachtrag P27 bis P31, Entscheidungen E1 bis E6 und E14.
   - `docs/plaene/2026-10-22-laufwerke-medien-agenten/machbarkeit.md`: P10 bis P26, Entscheidungen E9 bis E13.
5. `docs/RICHTUNG.md`: langfristige Richtung. Sie ist kein Batch-Inhalt, gibt aber vier Einwebungen in die Batches vor.
6. `docs/TODO.md`: die Brettzeilen der Punkte, jeweils mit „Vorläufig Batch N“.
7. `docs/RELEASING.md` und `docs/GOTCHAS.md`: Release-Weg und bekannte Fallen.

## Vorgehen

Das Vorgehen regelt die Arbeitsweise (zentral, programmierung, modellwahl), die jeder Agent selbst lädt. Am 2026-10-24 wurden dort zwei Regeln des Nutzers angenommen:
- zentral, „Zeitpunkt der Agentenwahl“: Agenten für Detailplanung und Entwürfe werden erst nach dem ersten Planungsschritt gewählt, Umsetzungsagenten und Zuschnitt erst nach der Planung.
- zentral, „Gesamtreview vor den Tests“, mit der Angleichung in programmierung (Delegation): Hat ein anderes Modell als Opus umgesetzt, prüft vor dem ersten Test ein eigener, frischer Opus-Agent die gesamte Umsetzung hochkritisch.

Die Entscheidungen E1 bis E6, E9 bis E12 und E14 hat der Nutzer dem Agenten übertragen (2026-10-22: „die liegen bei dir, was für die bestmögliche umsetzung sorgt, bei dir oder in der planung und recherche“). E1, E2 und E5 brauchen vor der Festlegung eine eigene Sicherheitsprüfung.

## Regeln des Nutzers

Diese Regeln stehen noch nicht in `AGENTS.md` oder einer Arbeitsweise. Sie sind wörtlich zitiert, wo der Wortlaut vorliegt.

- **Rot wird nicht veröffentlicht** (2026-10-21: „hab ich gesagt du sollst unfertigen scheiss releasen?“):
  - Ein Release kommt erst, wenn die Suite grün ist. Ausgenommen sind nur belegt nicht ausgeführte Fälle.
  - Eine Anweisung, keine weiteren Tests zur Verbesserung oder Beschleunigung zu machen, beendet nicht die Fixschleife für Funktionsfehler.
- **Schlussregel der Fixschleife** (2026-10-21, damals für einen bestimmten Batch gegeben):
  - Zeigt der nächste Suite-Lauf neue Fehler, wird nicht weiter getestet. Ein Opus-Agent mit Effort xhigh prüft alles seit dem letzten Release, seine Korrekturen kommen hinein, dann folgt das Release ohne weiteren Suite-Lauf.
  - Bleiben nur schon bekannte Fehler, folgt das Release.
  - Annahme: Die Regel gilt nur, wenn der Nutzer sie für einen Lauf wieder nennt.
- **Selbst entscheiden** (2026-09-28 und 2026-10-01):
  - Unkritische Gestaltungs- und Verhaltensentscheidungen trifft der Agent selbst und begründet sie im Plan. Es gibt keinen Haltepunkt zur Planfreigabe; der Lauf geht bis zum Release durch.
  - Rückfragen nur bei kritischer Wirkung: Datenverlust oder Wegfall einer vorhandenen Fähigkeit.
- **Keine willkürlichen Obergrenzen:** Eine Grenze braucht einen harten Grund, etwa eine Protokoll-, Plattform- oder Ressourcengrenze. Ziel ist, die echten Grenzen auszuschöpfen, etwa volle Bandbreite und wenige Umläufe.
- **Vollständigkeit liegt beim Agenten:** Nach einer Änderung müssen alle berührten Teile weiter funktionieren:
  - Syncs, entfernte Pfade, gespeicherte Orte, Ordnerauswahl;
  - alle Oberflächen: Terminal mit JSON, Desktop-App, Android-App, MCP, IPC und Brücke, Doku.
- **Übertragene Lösungen hinterfragen:** Wird ein Ansatz auf eine andere Plattform oder Größenordnung übertragen, gehört eine Prüfung je Plattform in jeden Plan. Beispiele: ein Windows-Scan auf Android, LAN-Verhalten über das Relay.
- **CI selbst begleiten:** Nach dem Start einer Remote-Suite oder eines Releases den Lauf selbst beobachten, Fehler beheben, neu starten und veröffentlichen, ohne auf Nachrichten des Nutzers zu warten. Release-Läufe nach `AGENTS.md` höchstens alle 30 Minuten prüfen.
- **Kernprinzip Verbindungen:** siehe `docs/ARCHITEKTUR.md`, Abschnitt „Zweck“.
- **Modelle:** Die Nutzervorgabe vom 2026-10-19 nennt Claude-Modelle: „sonnet für implementierungen die keine interpretation mehr zulassen, opus nut für sicherheitsrelevantes einer und design n anderer, gebatched die beiden also. der rest wenn möglich via sonnet.“ Für andere Anbieter wählt `modellwahl`. Annahme: Die Gesamtreview verlangt der Nutzer ausdrücklich mit Opus; ob er das für Codex-Umsetzungen auf ein Codex-Modell überträgt, ist nicht gesagt.

## Offen beim Nutzer

- **Umfang einer Abwahl der Arbeitsweise:** Der Antrag dazu wurde zentral abgelehnt. Er braucht die Festlegung des Nutzers, zum Beispiel „nur der Auftrag“ oder „nur der Chat“.
- **Konflikt `AGENTS.md` gegen zentral:**
  - `AGENTS.md` („subagent scope and RAM budget“) verlangt im Auftrag an einen Subagenten Abnahmezeichen und Berichtsformat.
  - zentral („nur Produktionsauftrag, nie Vorgehen“) verbietet jede Berichtsform im Auftrag.
  - Der Konflikt steht im Batchplan und wird in der Planung von Batch 1 geklärt.

## CI- und Suite-Fallen aus früheren Läufen

Jede dieser Fallen hat mindestens einen Remote-Lauf gekostet.

- **Bestehende Formatierungs- und Lint-Altlasten:**
  - `cargo fmt --all -- --check` meldet sehr viele Funde in vorhandenen Dateien, und der Linux-Host-Build warnt über Windows-Code.
  - Gates deshalb auf die Dateien des Batches beschränken: rustfmt im stdin-Modus je geänderter Datei, Clippy-Meldungen nach Batch-Dateien filtern. Vorbild: `native/test-lan-cleanup-task.sh`.
  - Wer eine vorhandene, nicht rustfmt-saubere Datei anfasst, formatiert sie ganz.
- **Windows-only-Code** (`#[cfg(windows)]`) übersetzt nur der Windows-Job. Solche Diffs vor dem Lauf statisch prüfen. Cargo gibt Windows-Pfade mit Backslash aus, also Filter auf geänderte Zeilen normalisieren.
- **Suite-Stufen nicht beim ersten Fehler abbrechen:** keine Kombination aus `set -e` und Pipe. Fehler festhalten und weiterlaufen, sonst zeigt ein Lauf nur ein Problem.
- **Opt-in-Umgebung:** Tests, die eine Suite-Umgebung brauchen, brechen in einfachen Modulläufen ab. Ein Beispiel ist `SMART_EXPLORER_COPY_PASTE_TASK` mit `CopyPastePeerFixture`. `#[ignore]`-Meilensteine brauchen einen eigenen Schritt mit `--ignored`.
- **Rust-Sichtbarkeit:** `pub use` auf ein `pub(crate)`-Element übersetzt nicht. `pub(super)` in verschachtelten CLI-Modulen braucht `pub(in crate::cli)`, wenn es weiter exportiert wird.
- **Absolute Pfade in Tests:** `/usr/bin/x` gilt unter Windows nicht als absolut. Stattdessen `std::env::temp_dir().join(..)` verwenden.
- **Exakte JSON-Erwartungen** (`assert_eq!(…, &json!({…}))`) in vorhandenen `android_task_*`-Tests brechen bei jedem neuen Feld. Suchen, bevor `share.status` oder andere Brückenobjekte erweitert werden.
- **SSH-Testserver:** Die Strafregel von OpenSSH 9.8+ (`PerSourcePenalties`) trennt den Runner nach vielen abgebrochenen Handshakes. Die Suite setzt `PerSourcePenalties no`.
- **Git Bash unter Windows** darf keinen POSIX-`CARGO_TARGET_DIR` bekommen.
- **Android-Gerätetests:** Das Ende von `am instrument` beendet die App. Danach die App einmal öffnen und Home drücken.
- **Release-Lauf:** Während das vollständige Release läuft, nicht auf `main` pushen, weil das Wrapper-Skript den Release-Commit pusht. Die Anzeige „build“ direkt nach dem Start ist kein Grund für einen zweiten Start.
- **Festplatte des Hosts:** Läuft sie voll, gehen Ausgaben verloren. Ausgaben nach `/dev/shm` umleiten und nur eigene Temporärdaten löschen.
- **Vorbild einer Suite:** `.github/workflows/rechte-desktop-tresore-task.yml` mit `native/test-rechte-desktop-tresore-task.py` und `native/rdt_task/`. Die Datei `expected_signals.txt` dient dort als Inventur der Meilensteine.
