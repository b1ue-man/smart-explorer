# TESTS-BEWERTUNG: Bestand, Bewertung und Pflege aller Tests (2026-11-07)

Auftrag: Übergabe `docs/plaene/2026-11-01-idealzustand/uebergabe.md`, Aufgabe 8, Zeile TESTS-BEWERTUNG in `docs/TODO.md`.

Nutzervorgaben (2026-11-02), wörtlich:

- „bevor man blind löscht, mach ne bestandsaufnahme und evaluiere. ich denke es ist nicht zu viel verlangt keine unnützen tests laufen zu lassen.“
- „wenn mehr als 50% der tests veraltet sind werden alle gelöscht.“
- „und die in zukunft besser zu verwalten“

## Bestand am 2026-11-07 (Stand `4b456dec`)

- **Rust:** 5 281 Tests.
  - 5 106 sind seit 2026-11-02 bewertet: 4 694 „behalten“, 373 „zusammenführen“, 24 „löschen“, 15 „unklar“.
  - 175 sind nicht bewertet:
    - 61 in `native/src/bin`;
    - 87 in `native/tests`;
    - 27 sind seit der Bewertung in den Crates hinzugekommen: 26 in engine, 1 in mobile.
  - 28 Urteile werden nachgeprüft: die 15 unklaren und 13 nicht behaltene, deren Datei sich seit `6a6d3930` geändert hat.
- **Android:** 413 Tests in 86 Dateien, davon 258 JVM- und Robolectric-Tests in 54 Dateien und 155 Instrumentierungstests in 32 Dateien.
- **Task-Suiten:** 38 Workflows `.github/workflows/*-task.yml` und ihre 42 Skripte `native/test-*` sowie `native/rdt_task/`.
- **Veraltete Tests:** 6 von 4 415 bewerteten (0,14 %). Die 50-%-Regel greift also nicht, und es wird nicht alles gelöscht.

Die Bewertungsdaten liegen außerhalb des Repos, in der Arbeitsablage der Sitzung. Ins Repo kommen nur die Ergebnisse: Register und Code.

## Meilensteine

| # | Ergebnis | Betroffen | Abnahme |
|---|---|---|---|
| T1 | Jeder heutige Test hat ein Urteil nach den Kriterien der Zeile TESTS-BEWERTUNG: behalten (K1 bis K5), löschen (D1 bis D5), zusammenführen (mit Ziel) oder unklar (mit offener Frage). Die 28 Nachprüfungen sind am heutigen Code entschieden. | Rust, Android, Task-Suiten | Für alle 5 281 Rust- und 413 Android-Tests sowie die 38 Suiten liegt je ein Urteil mit Beleg vor. |
| T2 | Die belegt nutzlosen Tests sind entfernt. Mehrfachprüfungen desselben Verhaltens sind zu tabellengetriebenen Tests zusammengeführt, und jede Variante bleibt als Tabellenzeile erhalten. Kein geschütztes Verhalten geht verloren. | Testdateien der Crates, `native/tests`, `android/app/src/{test,androidTest}` | Die eine Suite ist grün. Jede entfernte Variante steht als Zeile in ihrem Ziel. |
| T3 | Tests, die Dienste oder Umgebung ihrer eigenen Suite brauchen, tragen `#[ignore = "<Grund>"]`. Ihre Suite startet sie ausdrücklich, und ein normales `cargo test` besteht ohne Sonderliste. | Die 10 Einträge in `skips` von `native/test-crates-task.py` und weitere Funde aus T1, dazu ihre Suiten | `crates-task` braucht keine `skips`-Liste mehr. Jede Suite, die einen solchen Test braucht, schließt ihn ein. |
| T4 | Register: Zu jedem Test sind die Anforderung, die er schützt, sein Urteil und seine Suite verzeichnet. `crates-task` scheitert, wenn ein Test ohne Registerzeile hinzukommt oder eine Registerzeile ohne Test bleibt. So erkennt man bei einem Fehlschlag, ob der Test veraltet ist oder ein echtes Problem zeigt. | Neues Verzeichnis `docs/tests/`; `native/test-crates-task.py` | Die Registerprüfung ist Teil des Suite-Laufs und besteht. |
| T5 | Task-Suiten: veraltete Suiten sind entfernt, und Suiten mit veralteten Testnamen sind berichtigt. | `.github/workflows/*-task.yml`, `native/test-*`, Doku | Nach T1 hat jede bleibende Suite nur Namen, die es gibt. |

Reihenfolge: T1 → T2, T3, T5 parallel, getrennt nach Dateien → T4 → Gesamtreview → ein Lauf von `crates-task` → Abschluss. Ein Release folgt, falls T1 einen echten Fehler im Produkt findet und T2 ihn behebt. Reine Teständerungen ändern keine ausgelieferte Datei.

## Stand

- 2026-11-07: Bestand erhoben. Die Ergebnisse von Bereich 3 (übrige Engine-Module, 758 Tests, Urteile vom 2026-11-02) sind zusammengesetzt. T1 für den Rest beginnt.
- 2026-11-07, T1 abgeschlossen: Jeder Test hat ein Urteil.
  - Rust: 5 281 Tests. 4 848 behalten, 385 zusammenführen, 25 löschen, 23 unklar.
  - Android: 413 Tests. 366 behalten, 19 zusammenführen, 16 löschen, 12 unklar.
  - Ausnahme von D2: Die 9 Android-Tests, die Manifest- oder Ressourcen-XML lesen, bleiben als K2. Android liest diese Dateien zur Laufzeit selbst.
  - Ein echtes Produktproblem (R) fand die Bewertung nicht.
  - Die unklaren Tests hängen an den offenen Punkten E13-MEASURE-REST, HYBRID-REGIONS-OFF und DEVICES-WIDGET-API30.
  - Suiten: 38 Workflows und 8 Einzelskripte. Davon bleiben 21 Suiten und 5 Skripte; 8 Suiten und 1 Skript werden gelöscht, 8 Suiten und 1 Skript gehen in einer anderen Suite auf, 1 Skript ist unklar.
  - Neu gefunden: 97 Tests des Pakets share-server und 2 se-agent-Tests laufen in keiner Pipeline.
  - Neu gefunden: Mehrere Suiten verweisen auf Commits oder Builds aus der Zeit vor der Umschreibung vom 2026-11-01.
- T2 und T5 laufen als sechs Pakete ohne gemeinsame Dateien:
  - I1 bis I4 für die Rust-Bereiche;
  - I5 für Android;
  - I6 für die Suiten.
- T3 folgt als zweiter Teil von I6. Danach kommt T4 (Register), dann die Gesamtreview.
