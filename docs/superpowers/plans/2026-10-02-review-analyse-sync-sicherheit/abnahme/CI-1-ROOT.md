# CI-1-ROOT – Vault-I/O-Fixture und gemeinsame Integration

Stand: 2026-10-03. Ausschließlich konkrete Evidenz aus
[RV1-Lauf 37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629),
kein neuer Review.

Der Remote-Formatter meldete `vault_frame_task_tests.rs` mit 507 Zeilen.
Die vorhandenen `ProbeWriter`- und `InterruptedReader`-Implementierungen liegen
jetzt im privaten Kindmodul `vault_frame_fixture.rs`. Nur die zum Elternmodul
nötige Typ-/Feldsichtbarkeit ist ergänzt. Die bisherigen Szenarionamen,
Testfunktionskörper und Assertions sind per AST/Textvergleich unverändert.
Keine neue Fixture oder neue Suiteauswahl wurde eingeführt.

Gelesen: `ci-fixes.md`, `docs/refs/rv1-remote-suite.md`, beide genannten
Rustdateien sowie die vorhandenen Agentenberichte/Scopecorrekturen für die
Integration. Geändert: `vault_frame_task_tests.rs` und `fortsetzung.md`.
Erstellt: `vault_frame_fixture.rs` und dieser Bericht. Die Rustdateien bleiben
unter 500 Zeilen und 50 KiB; statisches Tree-sitter-Parsing ist fehlerfrei.
Kein Compiler, Formatter, Test oder nativer Einstieg wurde lokal ausgeführt.

Die tatsächliche Typ-/Verhaltensbestätigung bleibt beim selben Remote-RV1-
Einstieg. Nach allen CI-Anschlüssen aktualisiert Root den vollständigen
AST-Graph, committed/pusht den gemeinsamen Kandidaten und löst ausschließlich
die bestehende Suite erneut aus. Release bleibt bis zum Erfolg ausstehend.
