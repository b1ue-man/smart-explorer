# Testregister

`register.tsv` nennt jeden Test des Repositorys:
- die Rust-Tests unter `native/crates`, `native/src`, `native/tests`, `share-server` und `se-agent`: jede Funktion mit `#[test]` oder `#[tokio::test]` und jeden Test, den ein `macro_rules!`-Makro erzeugt (je Aufruf ein Test pro `<Name> =>`, so in `native/tests/speed_task/` und `native/tests/transfer_task/speed.rs`);
- die Kotlin-Tests (`@Test`) unter `android/app/src/test` und `android/app/src/androidTest`.

Die Zeile eines früheren Tests, der als Fall in einem tabellengetriebenen Test aufging, entfällt: sein Name bleibt als gewöhnliche Funktion ohne Testattribut stehen und ist kein Test.

Zu jedem Test nennt es die Anforderung, die er schützt, und die Suite, die ihn ausführt.

Wozu: Schlägt ein Test fehl, zeigt seine Zeile, was er schützt. Gilt diese Anforderung heute nicht mehr, ist der Test veraltet und wird angepasst oder entfernt. Gilt sie, zeigt der Fehlschlag ein echtes Problem im Code.

Nutzerauftrag vom 2026-11-02:
- „bevor man blind löscht, mach ne bestandsaufnahme und evaluiere. ich denke es ist nicht zu viel verlangt keine unnützen tests laufen zu lassen.“
- „und die in zukunft besser zu verwalten“

Der Plan steht in `docs/plaene/2026-11-07-tests-bewertung/plan.md`.

## Spalten

Die Datei ist UTF-8, die Spalten sind durch Tab getrennt, die erste Zeile ist die Kopfzeile.

| Spalte | Inhalt |
|---|---|
| Datei | Pfad der Testdatei, relativ zum Repository |
| Test | Name der Testfunktion |
| Urteil | `behalten`: Der Test schützt eine heute geltende Anforderung. `unklar`: Ob er bleibt, hängt an einer offenen Nutzerentscheidung in `docs/TODO.md`; Spalte Schützt nennt sie. |
| Kategorie | `K1` Verhalten, `K2` Sicherheit, `K3` Datensicherheit, `K4` Kompatibilität, `K5` nicht triviale Parser-, Format- und Planungslogik. Mehrere werden mit `/` verbunden. `U` steht bei `unklar`. |
| Schützt | die geprüfte Produktfunktion und das Verhalten, das der Test verlangt, oder bei `unklar` die offene Frage |
| Suite | der Workflow unter `.github/workflows`, der den Test ausführt. Mehrere werden mit `,` getrennt. Führt keine Suite ihn aus, steht dort `keine (<Grund>)`. |

Welche Suite ausführt:
- Gewöhnliche Rust-Tests und die JVM-Tests der App (`android/app/src/test`) führt `crates-task` aus.
- Ein Rust-Test mit `#[ignore = "<Bedarf>; Suite <Name>"]` braucht Dienste, Geräte, Rechte oder Umgebungsvariablen. Ihn startet die Suite, die sein Grund nennt.
- Die Instrumentierungstests (`android/app/src/androidTest`) brauchen ein Gerät. Ihre Klassen starten `android-task` (Job `device`), `review-task`, `rechte-desktop-tresore-task`, `luecken-task` und `sync-reliability-task`; welche Klasse wohin gehört, steht in `android/test-android-task.sh` (Klassenwacht von `emulator-run`).

## Pflege

- Wer einen Test hinzufügt, entfernt, umbenennt oder verschiebt, ändert seine Zeile im selben Commit.
- `native/test-register.py` vergleicht das Register mit dem Code. `crates-task` führt die Prüfung unter Linux aus. Sie scheitert:
  - bei einem Test ohne Zeile;
  - bei einer Zeile ohne Test;
  - bei einer Zeile mit falscher Spaltenzahl, doppeltem Schlüssel oder leerer Spalte Schützt;
  - bei einem unbekannten Urteil, einer unbekannten Kategorie oder einer unbekannten Suite;
  - bei einem Makro, das Tests erzeugt, deren Namen die Prüfung nicht lesen kann.
- Wer das Register neu erzeugt, liest die Rust-Tests über `rust_tests()` aus `native/test-register.py` (geschriebene und von Makros erzeugte Tests, je mit ihren Attributen), damit Erzeugung und Prüfung dieselben Tests sehen.
- Die Prüfung liest nur Quelltext. Lokal läuft sie mit `python3 native/test-register.py`.
- Ein Test, der dasselbe Verhalten wie ein bestehender prüft, wird dessen Tabellenzeile oder Fall. Er entsteht nicht als eigener Test.
- Eine Testdatei hat höchstens 500 Zeilen (AGENTS.md).

## Bewertung vom 2026-11-02 und 2026-11-07

Bewertet wurden 5 281 Rust-Tests, dazu 113 Messfälle, die `macro_rules!`-Makros erzeugen, und 413 Android-Tests und 46 Suiten: 38 Workflows und 8 Einzelskripte. Nicht im Bestand der Bewertung waren die 113 Rust-Tests, die Makros erzeugen (sie stehen nicht als `fn <Name>` im Quelltext): die `#[ignore]`-Messläufe in `native/tests/speed_task/` (81), `native/tests/transfer_task/speed.rs` (20) und `native/crates/transfer/src/transfer/os/shared/engine/speed_task_local_tests.rs` (12).

| Ergebnis | Zahl |
|---|---|
| Veraltet | 6 von 4 415 Tests der ersten Bewertung, also nicht mehr als die Hälfte. Die Nutzerregel, dann alles zu löschen, greift nicht. |
| Gelöscht, Rust | 66 am 2026-11-02, 19 am 2026-11-07 |
| Gelöscht, Android | 16 |
| Zusammengeführt | als Fälle tabellengetriebener Tests: 322 Rust-Tests und 18 Android-Tests. Weitere 68 Tests blieben eigene Tests, weil Aufbau, Plattformbedingung oder Ignorier-Status ihres Ziels nicht passten. |
| Suiten entfernt | 16 Workflows. Ihre noch wirksamen Prüfungen führen jetzt `crates-task`, `ai-plugin-task`, `android-task` und `sync-reliability-task` aus. |

Die genauen Zahlen der Umsetzung stehen im Plan. Die Bewertungsdaten selbst liegen nicht im Repository. Ihr Ergebnis für jeden heutigen Test ist dieses Register.
