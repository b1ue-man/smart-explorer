# CI-3-ROOT – konkrete Integrationsanschlüsse

Stand: 2026-10-03. Grundlage ist allein der beendete dritte RV1-Lauf
[37157166735](https://github.com/b1ue-man/smart-explorer/actions/runs/37157166735)
auf `71a8ca45697272453c213c9b0b5412d0cbed0f71`. Kein neuer Review.

## Quellenabschluss

- Der kandidaten-/SHA-256-gebundene dritte reine Formatpatch wurde unverändert als `c8c0d2ef` übernommen. Kein lokaler Formatter.
- Die vorhandenen Drive-Payload-/Queryhilfen liegen jetzt kohäsiv in `gdrive/core/gui_task_fixture.rs`; die GUI-Assertions bleiben unverändert. Die bisher genau 500 formatierten Zeilen große Datei wurde verkleinert.
- Die bestehende Android-WeakPin-Fixture ordnet kurze vorherige Fälle dem ausdrücklichen Opt-in zu und verwendet für akzeptierte Fälle den bereits dokumentierten sechs-Zeichen-/Unicodevertrag. Produktionsregel und schwache-PIN-Opt-in bleiben unverändert.
- Der Linux-OS-Adapter meldet die tatsächliche Sekundenauflösung von exfat-fuse für echte FUSE-Blockvolumes. Kernel-exFAT bleibt bei 10 ms; Namen, Größenlimits, UUID und konservativer FUSE-Flush bleiben erhalten. Der bestehende reale Volumefall prüft den passenden Treibervertrag und die gespeicherte Zeit.
- Der bestehende Linux-Suiteeintritt entdeckt den Watchlimitfall aus dem libtest-Listing, führt alle übrigen Fälle mit normalem Limit aus und diesen Fall im selben Runtimeeintritt mit echtem abgesenktem UID-Limit. Originalwert wird gelesen, festgehalten und bei EXIT/TERM/INT restauriert. Der gesamte ausgewählte Satz muss erfolgreiche Ergebnisse im gemeinsamen Log enthalten; kein Fall entfällt.

Refs vor Edit: `gdrive-ureq-throughput.md`, vorhandene Spec FC2,
`ShareDialogs.kt::isWeakPin`, `discovery_pin.rs`, `rv1-remote-suite.md` und
frische exfat-fuse/inotify/sysctl-Primärquellen. Aufgelöste Lücken und erwartete
Ergebnisse stehen in `ci-closure-fixes.md`. Bash-Syntax, Python-AST und statisches
Tree-sitter-Parsing der eigenen Rustquellen wurden geprüft; kein Produktlauf,
Compiler, Formatter, Test oder Release wurde lokal gestartet.

## Dateien

Geändert: `native/src/gdrive/core/gui_task_tests.rs`, `native/src/gdrive/mod.rs`,
`android/app/src/test/java/app/smartexplorer/android/ui/share/WeakPinTest.kt`,
`native/src/vfs/os/linux_os/local_platform.rs`,
`native/src/vfs/os/shared/review_task_local_tests.rs`,
`native/review-task-native.py`, `native/review-task-runtime.sh`,
`docs/refs/rv1-remote-suite.md`, `docs/refs/INDEX.md`,
`docs/ARCHITEKTUR.md`, `docs/RELEASING.md`, `docs/TODO.md` und
`fortsetzung.md`; diese dokumentieren den aktuellen Quellen-/Runtimevertrag
ohne erfolgreiche Abnahme- oder Releasebehauptung.

Erstellt: `native/src/gdrive/core/gui_task_fixture.rs`, dieser Bericht sowie
`ci-closure-fixes.md` und exakte `scopes/ci-3-*.json` vor Workergrants.
Die anderen Worker besitzen die im Plan benannten Sourcegrenzen; deren
Inventar/Entscheidungen stehen in ihren eigenen Berichten.

## Offenes Abnahmesignal

Alle vorhandenen RV1-Selektoren und direkt betroffenen Integrationen werden nur
über denselben Root-eigenen vollständigen Remote-Suiteeintritt bestätigt.
Statische Quellenkorrekturen ersetzen keine erfolgreiche Laufzeitabnahme.
Erst danach folgt der eine terminale Remote-Release. Keine offenen Ursprungs-
Deferrals oder allgemeinen Plattformgrenzen werden als geschlossen ausgegeben.

## Integrierter statischer Abschluss

Alle CI-3-Quellenanschlüsse sind kohäsiv committed, einschließlich E-ENGINE `16270cf3` und S-REVOKE `4ce64948`. Tree-sitter-Parsing der integrierten geänderten Rustquellen und `git diff --check` sind erfolgreich. Der Rootgraph wurde nach verifiziertem Entfernen ausschließlich seiner drei generierten Hauptdateien vollständig aus `native/src` extrahiert und neu geclustert: Manifest mit 1.755 Quelldateien, einschließlich aller drei neuen Teilmodule. Kein partieller oder zweiter Graph unter `native/src`. Generierte Graph-/Label-/Manifest-/Reportdateien gehören zur finalen Integrationsänderung. Die tatsächliche Auswertung des nächsten einzigen Remote-Suitekandidaten bleibt offen; kein Compiler- oder Laufzeitbeleg aus diesen statischen Schritten abgeleitet.
