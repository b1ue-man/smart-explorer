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
- 2026-11-07, T2 bis T5 umgesetzt. Kompiliert ist noch nichts, geprüft nur statisch: rustfmt ohne neue Abweichung, Python, Shell, PowerShell, JSON und actionlint.
  - T2:
    - 322 Rust-Tests und 18 Android-Tests sind Fälle ihres Ziels.
      - Die Fälle heißen nach dem früheren Test.
      - In storage und in Teilen von engine nennt ein Fehlschlag den Fall.
    - 19 Rust-Tests und 16 Android-Tests sind gelöscht.
    - 67 Tests blieben eigene Tests, je mit Grund: anderer Aufbau, andere Plattformbedingung, anderer Ignorier-Status oder eine Datei an der 500-Zeilen-Grenze.
  - T3:
    - 31 Tests sind neu `#[ignore]` mit Bedarf und Suite, und 286 bestehende Gründe nennen jetzt ihre Suite.
    - `crates-task` braucht keine `skips`-Liste mehr.
  - T4:
    - `docs/tests/register.tsv` verzeichnete 5 430 Tests: 5 282 behalten, 148 unklar (seit dem Regressionstest des Schnellspiegels, `bfedc70a`, 5 431 und 5 283 behalten). Unter den unklaren sind 113 Messfälle aus `macro_rules!`-Makros, die Review B gefunden hat; die Registerprüfung liest diese Makros jetzt mit.
    - Eine Suite fehlt nur bei den 8 Tests der zwei Instrumentierungsklassen ohne Läufer (TODO SUITES-ANDROID-KLASSEN).
    - `native/test-register.py` prüft das Register in `crates-task`.
  - T5:
    - 16 Workflows und 18 Skripte sind entfernt.
    - `crates-task` hat zusätzlich:
      - den Job `servers` für share-server und se-agent;
      - je OS die ignorierten Tests der zusammengeführten Suiten;
      - die systemd-Prüfung;
      - das Übersetzen der Instrumentierungsquellen.
    - Die bleibenden Suiten verweisen nur auf Bestehendes. Die Stapel-Gates gegen Basiscommits früherer Stapel sind aus filter-transfer, cli-discoverable-update, transfer-engine, luecken, ai-plugin, delete-speed, links und dem Job `host` von android-task entfallen.
    - Geblieben sind die Zeilen-Gates gegen Tags in fixes-2026-10-28 (Clippy seit v0.5.187), greeter-keyboard (Clippy seit v0.5.188), speed-setup (Clippy und rustfmt seit v0.5.188) und sync-abgleich (Clippy und rustfmt seit v0.5.189), die Formatstufen gegen Basiscommits in review-task (`c935b763`) und sync-reliability (`a78f4d1c`) und die rustfmt-Stufe der festen Dateiliste in terminal.
- 2026-11-07, Reviews (Opus, Effort xhigh):
  - Review A (I1 bis I4) hat behoben:
    - 3 Kompilierfehler;
    - einen Fall, dessen Prüfungen beim Zusammenführen verloren gegangen waren;
    - 5 Löschungen, deren deckender Test sie nicht ganz prüfte; sie sind jetzt Fälle;
    - die zwei ungedeckten gdrive-Prüfungen;
    - fehlende Fallnamen in den I1-Tabellen;
    - vier Dateien ab 500 Zeilen;
    - neue ungenutzte Importe.
  - Review B (I5, I6, Register, Dokumente) hat behoben:
    - die Inventur von rechte-desktop-tresore, die Fallfunktionen als Tests zählte;
    - ein totes Pflichtpräfix in review-task;
    - die luecken-Altstände, die jetzt aus dem Release v0.5.180 kommen;
    - zwei ungenaue Zählfilter;
    - einen NameError in review-task;
    - die Makro-Tests im Register.
  - Offen geblieben und danach behoben: se-agent braucht `tempfile` als Dev-Abhängigkeit, der Lock wird auf dem Runner aufgelöst (`lock-refresh.yml`).
  - Wiederhergestellt: `screen_task_android_screen_keys_file_current`. Er liest keinen Quelltext. Er vergleicht die Schlüsselliste aus den Android-Testressourcen mit den Rust-Enums, also einen Vertrag zwischen beiden Seiten, den der Kotlin-Test `ScreenTextsTest` voraussetzt.
- 2026-11-07, Läufe von `crates-task` (alle registrierten Tests erstmals zusammen):
  - 42356828421 (`9b881aa9`), vier Fehler, behoben:
    - Bildschirmfreigabe: Ein Schreibfehler beim Schließen durch den Betrachter endete als `connection_lost` statt `viewer_closed` (Produkt, `ca94b6cb`).
    - Laufwerk: Das Leeren beim Aushängen wartete nicht auf laufende Uploads (Produkt, `967c2034`).
    - Zwei Oberflächen-Harness-Tests passten nicht mehr zur App: Seite „Darstellung“, Drive-Wurzelabfrage (Tests, `209e3c9d`).
  - 42361964538 (`d6b6f826`), zwei Fehler unter Windows, behoben:
    - Schnellspiegel: Eine veraltete NTFS-Verzeichnisabfrage verhinderte jede Löschung; die Beobachtung kommt jetzt aus `stat` (Produkt, `bfedc70a`, neuer Test `mirror_deletion_compares_with_the_file_not_a_stale_listing`).
    - Bisync-Abbruchtest: Seite B war schneller als der 50-ms-Wächter (Test, `6018e8e8`).
  - 42366382899 (`6018e8e8`), ein Fehler unter Linux, behoben:
    - FTP: Nach ABOR auf eine schon beendete Übertragung blieb die Antwort des Servers ungelesen, und die Verbindung meldete sich neu an; ein Bereich, der auf seine eigene Anmeldung wartete, übernahm sie, das ergab eine fünfte Anmeldung (Produkt, `b327f4ee`; Test der Sicherheitsrunde umbenannt, er deckt jetzt auch den Rückfall auf die Neuverbindung).
  - 42371819683 (`b327f4ee`): bestanden, alle sieben Jobs.
- 2026-11-07: Release [v0.5.195](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.195) (Release-Lauf 42376415603, Quelle `b4cbb7bb`, Release-Commit `9d00dd8e`), 26 Assets; `releases/latest/download/version.txt` meldet 0.5.195.
