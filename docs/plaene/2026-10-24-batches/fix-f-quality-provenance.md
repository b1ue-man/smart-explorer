# M11 / F38 – Qualitätsprovenienz bei Checkout-Zeilenenden

Stand: 2026-10-26. Autor: derselbe beauftragte Fixautor für Fehlerliste, Verknüpfungskarte, Vorplan, Umsetzung und anschließenden eigenen Review. Arbeitskopie: `/tmp/smartexplorer-review-fixes-ee75e91b`, Branch `codex/review-fixes-ee75e91b`. Der beobachtete Fehler gehört zum Kandidaten `1ffbf4b0bc90b2479947680abcd0a5941cce00f8` und zum beendeten Remote-Lauf `40406246032`; das vor dieser Änderung gelesene HEAD ist `92cf56a4`.

## A. Erstplan aus dem Ergebnisauftrag

Quelle ist der delegierte Auftrag M11/F38 vom 2026-10-26, zusammen mit dem wörtlichen Nutzervertrag `/tmp/smartexplorer-final-review-user-contract.txt` und dem ersetzenden Scope `/tmp/smartexplorer-f-quality_provenance-scope.json`. Gefordert sind auf Windows und Linux dieselben eingefrorenen Releasequellen und Codecversionen trotz Checkout-Zeilenenden; Commit-, Hash-, Inhalts- und Qualitätsbindung bleiben vollständig.

Vor der Projektlektüre festgehalten: Erst die genaue fehlerhafte Fixturefunktion und ihre Aufrufer/Zulieferungen bestimmen, dann die Darstellungskorrektur mit Erhaltungsvertrag planen, ändern und selbst nachprüfen. Zu klären waren der tatsächliche F38-Panic, eingebettete Frozen-Dateien, unveränderliche Gitblobs, aktuelle Codecversionsprüfung, Plattformpins und bestehende gemeinsame Abnahme. Es fehlt keine nur vom Nutzer lieferbare Angabe.

Schreibbar sind ausschließlich `native/src/screen/os/native_ffi/quality_acceptance_provenance.rs`, `.gitattributes` und dieser Bericht. Keine Unteragenten, lokalen Builds, Compiler, Formatter, Tests, Server, Installationen, lang laufenden Prozesse, Gitmutationen oder Graphupdates. Main besitzt Manifestregistrierung, Integration und denselben Remote-Suite-/Releaseweg. Die eigene Abgabe ist ein statischer Autorenabschluss.

## B. Eigene Fehlerfunktionsliste und Bestand

| Funktion / Grenze | Eigener Befund | Einordnung und sichtbare Folge |
|---|---|---|
| `quality_acceptance_provenance::frozen` | Der Startmarker enthält fest `// BEGIN_FROZEN_{name}\n`. Ein eingebetteter CRLF-Text enthält diese Folge nicht; `split_once` liefert `None`. | F38 ist ein belegter Fehler der Qualitätsfixture: Windows bricht an deren Zeile 32 ab, bevor die physische Qualitätsauswertung beginnt. Kein belegter Fehler von Capture, Encoder oder Decoder. |
| `quality_acceptance_provenance::verify`, aktuelle Lockfileprüfung | Gesucht wird jeweils `name = "…"<LF>version = "0.9.9"`; `include_str!(Cargo.lock)` kann bei Checkout CRLF enthalten. | Statisch belegter Folgefehler derselben Darstellungsgrenze: Nach einer bloßen Markerreparatur würde die aktuelle Versionsassertion ebenfalls fehlschlagen. Dieser Folgefehler wurde im Remote-Lauf nicht separat erreicht. |
| `git`, `original`, `range` und die Rohquellenhashes | `git show <Releasecommit>:<Pfad>` liefert die unveränderlichen Releaseblobs. Beide SHA-256 stimmen mit den vorhandenen Konstanten überein. Alle drei eingefrorenen Blöcke sind im gelesenen LF-Bestand exakt gleich den zugehörigen Releasebereichen. | Diese Funktionen und Bindungen sind unverändert zu erhalten. Es gibt keinen Beleg für geänderte Releaseinhalte oder falsche Qualitätsmetriken. |

Der statische Byte-/Textvergleich las die freigegebenen Releaseblobs aus Commit `a814f263096fe69d3746e840b5f3fad3fd7e88a0` ausschließlich über read-only `git show`. Er führte weder Rustcode noch eine Projektfixture aus.

- `native/src/screen/core/pixel_convert.rs`: SHA-256 `322c0c1221f98f3d5993b3f9b84f24cfd6552dcf1b91a5bfc84167b16fb61165`.
- `native/src/screen/os/native_ffi/openh264_codec.rs`: SHA-256 `7f1f1ee52d5da0130a6ad6e21aa4c7655c99f3067fe7b4e504c2c9e514c40ac1`.
- Die Rohblobs enthalten kein CR. Die identischen Frozen-Bereiche umfassen PIXELS 4.624, ENCODER 4.868 und DECODER 3.659 Bytes.
- Bei ausschließlich synthetisch auf CRLF dargestellten aktuellen Texten fehlt jeder bisherige Frozen-Startmarker; beide bisherigen aktuellen Lockfile-Versionsmuster fehlen ebenfalls. Das ist ein statischer Textbefund, kein Windows-Laufzeitnachweis.

## Direkte und indirekte Verknüpfungskarte

Die Tabelle umfasst den tatsächlich betroffenen Weg bis zu seinen unveränderten Produktions-/Artefaktgrenzen. Die vorhandenen Graphknoten und Kanten wurden ohne Graphmutation gelesen; entscheidend sind die anschließend gelesenen konkreten Quellen.

| Station | Direkter Anschluss und indirekte Wirkung | Erhaltungsvertrag |
|---|---|---|
| `.github/workflows/review-task.yml` | Der Workflow bindet den Kandidaten über `CANDIDATE_SHA` und checkt den vollständigen Quellstand aus. | Kein zusätzlicher Pipelineweg und keine Änderung von Kandidat, Checkout oder Releaseverfahren. |
| `native/review-task-manifest.json` → `native/review-task-native.py::native_tests` | Bestehende Auswahl enthält `batches_task_b6_quality_`; ein serieller Bibliotheksdurchgang ruft die Fälle auf. Der Runner trennt tatsächliche Ergebnisse von `SE_TASK_UNVERIFIED_JSON`. | Bestehende Auswahl, Erfolge und ehrliche Nichtausführung erhalten. Main registriert den ergänzten Fall. |
| `review-task-native.py::openh264_fixture` → `openh264_pins.rs` | Die gemeinsame Vorbereitung liest den ausgewählten Plattformpin, kontrolliert URLgrenze, komprimierte Länge und SHA-256 und übergibt `SE_SCREEN_TASK_OPENH264`. | Keine Änderung an Download-/Pinprüfungen oder an der Bibliotheksauswahl. |
| `screen/os/mod.rs` → `openh264_codec.rs` | Der native Codec wird für Windows und Linux eingebunden. Unter `cfg(test)` bindet er `quality_acceptance_tests`, `quality_baseline` und `quality_provenance` ein. | Die Provenancekorrektur bleibt testintern; keine neue Produktionsfunktion oder Plattformfreigabe. |
| `batches_quality_acceptance_task_tests.rs::batches_task_b6_quality_openh264_baseline_transport_and_still` | Nach Plattform-/Bibliotheksvoraussetzung und `TEST_LOCK` läuft zuerst `quality_provenance::verify`, danach der echte OpenH264-Vergleich. | Der Provenancecheck darf nicht übersprungen werden. Fehlende Bibliothek bleibt ausdrücklich unbestätigt. |
| `verify` → `git(["rev-parse", "HEAD"])` und Umgebung | Der aktuelle Commit muss `CANDIDATE_SHA` entsprechen; die gemeinsame Logwurzel muss vorliegen. | Beide Assertions bleiben wortgleich und an derselben Stelle. |
| `verify` → `original` → `git show` → `metrics::sha256` | Der gepinnte Releasecommit und die beiden vollständigen Rohdateien bestimmen den Ursprung. | Keine Normalisierung der Gitblobs, keine neuen Hashwerte, keine Ersatzbasis. |
| `range` + eingebettete Core-Baseline → `frozen("PIXELS")` | Der historische Bereich ab `YuvMatrix` bis vor NV12 wird vollständig mit dem eingefrorenen CPU-Konverter verglichen. | Nur die CRLF-Darstellung des eingebetteten Textes darf nach LF zurückgeführt werden; anschließend bleibt der vollständige exakte Bereichsvergleich. |
| `range` + eingebettete native Baseline → `frozen("ENCODER"/"DECODER")` | Beide historischen Codecabschnitte werden vollständig mit den eingefrorenen Encoder-/Decoderabschnitten verglichen. | Dieselbe enge Darstellungskorrektur; kein Entfernen von Leerraum, Kommentaren oder Programmtext. |
| ursprüngliche und aktuelle `native/Cargo.lock`, `native/Cargo.toml` | Beide Lockstände müssen weiterhin `openh264` und `openh264-sys2` in Version `0.9.9` enthalten. Die aktuellen Abhängigkeiten wählen denselben dynamischen Ladeweg. | Nur der eingebettete aktuelle Locktext erhält CRLF→LF. Der ursprüngliche Lockblob und beide Versionswerte bleiben unverändert. |
| `openh264_pins::PIN` + ursprüngliche Pindatei | Die aktuelle Plattformbibliotheks-SHA muss bereits in der Release-Pindatei enthalten sein. Windows- und Linuxbibliotheken besitzen absichtlich verschiedene Binärhashes für dieselbe gepinnte Codecversion. | Keine Gleichsetzung verschiedener Plattformbinärdateien und keine Änderung eines Pins. |
| ursprüngliches `rate_control.rs` | Die historischen Grenzen 150.000 bis 240.000.000 bit/s bleiben belegt. | Beide ursprünglichen Konstantenprüfungen bleiben erhalten. |
| `quality_baseline::open/decode` + eingefrorene Pixelkonverter | Die historische CPU-Baseline verwendet ihren eigenen BT.709-Konverter und eigene geladene Encoder-/Decoderbesitzer. `decode` verlangt genau die synchrone Ausgabe ohne verbleibende Decoderframes. | Keine Änderung an Codec-, Reader-, Bild- oder Bibliotheksbesitz; kein Kandidatenkonverter als Baseline. |
| `codec_profile` + `hybrid_still` | Gleiche Capturebytes und zugeordnete Ausgabeidentität speisen Baseline und Kandidat; sechs Größen-/Bitratenprofile, echte abhängige Frames, GOP-Abbruch, Replay und Keyframerecovery werden ausgewertet. Hybridstill fordert exakte rekonstruierte Pixel. | Alle bestehenden Assertions, Profile, Wiederherstellungs- und Speicherfreigabeprüfungen bleiben unverändert. |
| Core-`batches_quality_acceptance_task_tests.rs` + `quality_acceptance_metrics.rs` | Analytische Orakel, Frame-/Zeit-/Keyframebindung, Geometrie, RGB-Fehler, Alpha und Luma-SSIM begrenzen die Qualitätsaussage. | Keine Änderung an Metrik, Epsilon, Mindestgeometrie oder No-Regressionsentscheidung. |
| `metrics::retain/evidence` → `native/review_task_quality.py::quality_evidence/retained_pixels` | Artefakte binden Kandidat, Releasebasis, ausgewertete Profile und tatsächlich erhaltene BGRA-Dateien an Größe und SHA-256. | JSON-Schlüssel und insbesondere `source_blocks` bleiben unverändert; weder Erfolg noch fehlende physische Ausführung werden umetikettiert. |
| `.gitattributes` → beide Baselines und aktuelles Lockfile | Diese drei Dateien sind die vollständigen eingebetteten Textzulieferungen der betroffenen Vergleiche. | Drei genaue `text eol=lf`-Pfade; keine globale Rust-/Lockfile-Regel und keine Änderung von Binär-/Lizenzattributen. |

## C. Entscheidungsplan und Quellen

Die offizielle Git-Dokumentation beschreibt `text` als LF-Normalisierung im Index und `eol=lf` als festgelegte Checkout-Darstellung. Ohne diese Festlegung können Konfiguration und Plattform die Checkout-Zeilenenden bestimmen. Quelle: [Git gitattributes, text/eol](https://git-scm.com/docs/gitattributes#_text), abgerufen am 2026-10-26. Die konkrete EOL-Ursache wird zusätzlich durch den fehlenden Windows-Startmarker und den statischen Vergleich der erlaubten Quellen belegt.

Gewählt ist die Verbindung aus enger CRLF→LF-Rückführung der drei eingebetteten Vergleichstexte und drei exakten LF-Checkoutattributen. Die Rückführung behandelt bereits vorhandene CRLF-Arbeitsdateien; die Attribute halten neue reguläre Checkouts einheitlich. Beide Schritte betreffen ausschließlich die Darstellung des eingebetteten Vergleichs und nicht die unveränderliche Gitquelle.

Verworfene Wege:

- Nur den Startmarker optional auf CRLF prüfen: Dann bliebe der Blockinhalt CRLF und der exakte Releasevergleich falsch; die Lockprüfung wäre ebenfalls nicht behoben.
- Nur Attribute ergänzen: Ein bereits materialisierter oder nachträglich von einem Editor auf CRLF geschriebener Vergleichstext wäre ohne erneuten Checkout weiterhin fehlerhaft.
- Gitquellen vor der Hashprüfung normalisieren, Hashwerte ersetzen oder den festgelegten Releasecommit wechseln: Das würde den unveränderten Ursprung abschwächen und ist ausgeschlossen.
- Allgemeines `trim`, zeilenweises Filtern oder whitespaceunabhängiger Vergleich: Dadurch könnten tatsächlich veränderte Inhalte durch die Abnahme gelangen; diese Wege sind ausgeschlossen.
- Globale `*.rs`- oder Lockfile-Attribute: Der konkrete Nachweis braucht ausschließlich die drei benannten Include-Dateien.

## D. Konkreter Vorplan vor der Produktänderung

| Schritt | Dateien / Abhängigkeit | Konkrete Handlung und prüfbares Ergebnis | Stand vor Änderung |
|---|---|---|---|
| Q1 Eingebettete Darstellung | `quality_acceptance_provenance.rs`; unveränderte Releaseblobs, Frozen-Zulieferungen und Lockfile | Einen privaten Helfer einführen, der ausschließlich CR unmittelbar vor LF entfernt. Diesen nur auf beide eingebetteten Baselines und das aktuelle eingebettete Lockfile anwenden. Alle bisherigen Vergleichs-, Hash-, Kandidaten-, Versions-, Pin- und Bitratenassertions bleiben erhalten. | Geplant |
| Q2 Checkoutvertrag | `.gitattributes` | Exakt die beiden Baselinepfade und `/native/Cargo.lock` mit `text eol=lf` ergänzen. Bestehende Attributzeilen unverändert lassen. | Geplant |
| Q3 Aussagekräftige Regression | `quality_acceptance_provenance.rs`; Main besitzt Manifest | Einen Fall `batches_task_b6_quality_provenance_checkout_line_endings_keep_exact_content` ergänzen. Reale Frozen-/Locktexte in LF und CRLF müssen gleich auswertbar sein; eingefügte Programminhaltsbytes, Leerraum und ein einzelnes CR dürfen nicht verschwinden. Kein bestehender Fall oder Vergleich wird entfernt. | Geplant |
| Q4 Eigener Nachreview | beide geänderten Quelldateien und dieser Bericht | Volltext-/Strukturvergleich, enger statischer Rust-Syntaxparser falls vorhanden, vollständiger Abgleich gegen die Karte und Git-Verträge. Keine lokale Funktionsausführung. Vollständiges Inventar und offene Remote-Frage festhalten. | Geplant |

Vor dem Nachreview festgelegte Prüffragen: Bleiben die Rohhashes exakt? Erreicht die Normalisierung nur die drei Include-Texte? Werden außerhalb von CRLF keine Bytes entfernt? Bleiben beide Codecversionen, Plattformpins, Bitratengrenzen, Qualitätsprofile und der JSON-Vertrag erhalten? Stimmen Sichtbarkeit, Einbindung, neue Fallauswahl und Dateigröße mit den Projektvorgaben überein?

## Umsetzung und anschließender eigener Anti-Regressionsreview

Q1 bis Q4 sind als Quelltextänderung und eigener statischer Autorenreview abgeschlossen. Es wurde keine lokale Fixture ausgeführt.

In `quality_acceptance_provenance.rs:30` entfernt `checkout_lf` ausschließlich das CR einer CRLF-Folge. Die drei Anwendungspunkte sind die eingebettete Pixelbaseline (Zeile 56), die eingebettete Encoder-/Decoderbaseline (Zeile 65) und das eingebettete aktuelle Lockfile (Zeile 83). Die Releaseblobs durchlaufen diesen Helfer nicht. Die existierende Frozen-Extraktion bleibt exakt; der vollständige extrahierte kanonische Text muss weiterhin dem rohen historischen Bereich entsprechen.

`.gitattributes:24` bis Zeile 26 legen ausschließlich diese drei Include-Dateien auf `text eol=lf` fest. Der vollständige vorherige Attributtext bleibt unverändert. Cargo-Dateien, eingefrorene Inhalte, Codecquellen, Pins, Qualitätsmetriken, Artefaktverbraucher und Manifest wurden von diesem Autor nicht geändert.

Der zusätzliche Fall `screen::os::openh264_codec::quality_provenance::batches_task_b6_quality_provenance_checkout_line_endings_keep_exact_content` steht in derselben testinternen Provenancedatei. Er verwendet die real eingebetteten Frozen-/Locktexte in LF-/CRLF-Darstellung. Eingefügter Kommentarinhalt, ein Leerzeichen und ein einzelnes CR bleiben im Vergleich sichtbar; Version `0.9.8` wird nicht als die verlangte `0.9.9` behandelt. Die gezielte negative Inhaltsprüfung schützt gegen eine stillschweigend zu großzügige Normalisierung. Sie ist kein Ersatz für den unveränderten physischen Qualitätsfall.

| Nachreviewfrage | Tatsächlich erhobener statischer Beleg / Grenze |
|---|---|
| Bleibt der Ursprung unverändert gebunden? | Die vollständigen vorherigen Funktionsquellen von `git`, `original`, `range` und `frozen` sind bytegleich. Beide Release-SHA-Konstanten, der Releasecommit und die Pfade bleiben unverändert. |
| Bleiben sämtliche bisherigen Prüfungen erhalten? | Nach Rückführung ausschließlich der drei geplanten Include-Eingabeänderungen ist die vollständige Rust-Tokenfolge von `verify` identisch mit der vorherigen Funktion. Das umfasst Kandidat, Logwurzel, Rohhashes, alle drei Frozen-Vergleiche, beide ursprünglichen/aktuellen Codecversionen, Bibliotheks-SHA, Bitraten und das vollständige JSON. |
| Kann eine Darstellungskorrektur echte Unterschiede verdecken? | Der Helfer verwendet genau `replace("\r\n", "\n")`, keine Trimmung und keine Entfernung einzelner CR. Die unveränderte Gleichheitsassertion vergleicht danach den vollständigen Block. Der neue Fall enthält positive Darstellungs- und negative Inhalts-/Versionsaussagen; seine Ausführung ist noch unbestätigt. |
| Bleiben Fehler ehrlich? | Es gibt keinen neuen Skip, kein `catch_unwind`, keinen Defaultbeleg und keinen Fallback auf Kandidatenquellen. Nicht auffindbare Marker und ungleiche Inhalte bleiben harte Fehler. |
| Bleiben Codec-/Bildbesitz und Qualitätsanforderungen erhalten? | Geändert wurden nur die testinterne Provenance und drei Textattribute. Baseline-/Kandidatenencoder, Decoderidentität, Capturebytes, Grantfreigabe, synchrone Baselineausgabe, Qualitätsmetriken und die sechs physischen Profile bleiben unverändert. Zusätzliche Bildkopien entstehen nicht. |
| Bleiben JSON und Artefaktverbraucher kompatibel? | Die ganze JSON-Tokenfolge ist unverändert, einschließlich `source_blocks: "byte-identical to immutable git source"`. Der weiterhin exakte Vergleich bezieht sich auf den nach Git-Konvention rückgeführten eingebetteten Quelltext. Der gelesene Verbraucher bindet weiterhin Kandidat, Releasebasis, Profile und gespeicherte Pixelhashes. |
| Bleiben die Änderungen in der erlaubten Fläche? | Eigene Schreiboperationen betreffen genau die unten genannten drei Dateien. Die gelesenen parallelen Gitstatusnamen wurden nicht inhaltlich untersucht oder geändert. |
| Ist die Rustquelle syntaktisch konsistent und klein? | Der bestehende Tree-sitter-Rustparser meldet für die gesamte geänderte Quelldatei keinen Syntaxfehler. Sie umfasst 140 Zeilen / 5.614 Bytes in normaler Formatierung. Das ist kein Typ-, Link- oder Compilerbeleg. |
| Bleiben Checkoutattribute eng? | Der Volltextvergleich bestätigt ausschließlich das geplante Anhängen der drei genauen Pfade mit Kommentar. Die Datei umfasst 26 Zeilen / 1.298 Bytes; bestehende Binär-, Lizenz-, Gradle- und Skillattribute bleiben vollständig erhalten. |

Der Dokukontextgate wurde mit freigegebenem read-only `git status --short`, `git log -1 --oneline`, gezieltem M11/F38-`rg` und der aktuellen Quell-/Bloblektüre vor dem neuen Bericht durchlaufen. Erwartete disjunkte Workeränderungen sind erhalten. Die Prüfung lief über den im Auftrag benannten bestehenden Interpreter `/root/.local/share/uv/tools/graphifyy/bin/python`. Der unqualifizierte Befehl `python` und der Marker `graphify-out/.graphify_python` waren nicht verfügbar; dafür wurde weder eine Installation noch ein Graphschreibvorgang benötigt.

## Vollständiges Dateiinventar dieses Autorauftrags

Gezielt gelesene Projektdateien; eine aufgeführte Datei bedeutet keine Ausführung ihres Inhalts:

- `.gitattributes`
- `.github/workflows/review-task.yml`
- `AGENTS.md`
- `docs/ARCHITEKTUR.md` – Architektur-/Screen-/Suitebezüge
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json` – F38
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md` – F38/M11 und Erhaltungsgrenze
- `docs/plaene/2026-10-24-batches/fix-f-quality-provenance.md`
- `graphify-out/graph.json` – Wortschatz, Provenance-/Qualitätsknoten und Verknüpfungen, ausschließlich lesend
- `native/Cargo.lock`
- `native/Cargo.toml` – OpenH264-Abhängigkeiten
- `native/review-task-manifest.json` – Qualitätsauswahl, Plattformfälle, Artefakte und R11
- `native/review-task-native.py` – Pinvorbereitung, Kandidatenbindung und nativer Qualitätsaufruf
- `native/review_task_quality.py`
- `native/src/screen/core/mod.rs` – testinterne Einbindung
- `native/src/screen/core/batches_quality_acceptance_task_tests.rs`
- `native/src/screen/core/pixel_convert.rs` – Konvertergrenze und unveränderlicher Releaseblob
- `native/src/screen/core/pixel_convert_tests.rs`
- `native/src/screen/core/quality_acceptance_baseline.rs`
- `native/src/screen/core/quality_acceptance_metrics.rs`
- `native/src/screen/core/rate_control.rs` – aktuelle Konstanten und unveränderlicher Releaseblob
- `native/src/screen/os/mod.rs` – Windows-/Linuxeinbindung
- `native/src/screen/os/native_ffi/openh264_codec.rs` – Aufrufer-/Besitzgrenze, Testregistrierung und unveränderlicher Releaseblob
- `native/src/screen/os/native_ffi/openh264_pins.rs` – aktuelle Plattformpins und unveränderlicher Releaseblob
- `native/src/screen/os/native_ffi/quality_acceptance_baseline.rs`
- `native/src/screen/os/native_ffi/quality_acceptance_provenance.rs`

Gelesene Auftrags-/Werkzeugquellen:

- `/tmp/smartexplorer-f-quality_provenance-scope.json` einschließlich der durch Main ausdrücklich ergänzten Lesepfade
- `/tmp/smartexplorer-final-review-user-contract.txt`
- `/tmp/smartexplorer-rv1-40406246032/native-failures.json` – Kandidat und vollständige F38-Diagnose
- `/tmp/smartexplorer-rv1-functional-selections.json` – im Auftrag benannter Eintrag `quality_provenance`
- `/root/.codex/AGENTS.md`
- `/root/.codex/skills/graphify/SKILL.md`
- `/root/.codex/skills/graphify/references/query.md`
- Aktuelle MCP-Arbeitsweise mit den gelieferten Programmierungs-, Recherche-, Planungs-, Änderungs- und Reviewanweisungen
- [Git gitattributes](https://git-scm.com/docs/gitattributes), Abschnitte `text`/`eol`, abgerufen am 2026-10-26

Leseversuch ohne vorhandene Datei: `graphify-out/.graphify_python`; der tatsächlich verwendete Interpreter ist oben vollständig angegeben.

Geändert bzw. erstellt:

| Datei | Eigene Änderung |
|---|---|
| `native/src/screen/os/native_ffi/quality_acceptance_provenance.rs` | Private enge Darstellungskorrektur, deren drei Include-Anschlüsse und zusätzlicher Provenancegrenzfall. |
| `.gitattributes` | Drei genaue LF-Pfade für die eingebetteten Texte. |
| `docs/plaene/2026-10-24-batches/fix-f-quality-provenance.md` | Neu: eigener vollständiger Fehler-/Karten-/Vorplan-/Fix-/Nachreviewbericht. |

## Fallnamen, Übergabe und verbleibende Laufzeitgrenze

Der bestehende belegte F38-Fall heißt unverändert `screen::os::openh264_codec::quality_acceptance_tests::batches_task_b6_quality_openh264_baseline_transport_and_still`. Die bestehenden unmittelbaren Qualitätsfälle `screen::core::batches_quality_acceptance_task_tests::batches_task_b6_quality_metrics_match_independent_analytic_oracles` und `screen::core::batches_quality_acceptance_task_tests::batches_task_b6_quality_binding_rejects_foreign_capture_and_missing_geometry` bleiben unverändert.

Main ist der zusätzliche genaue Name `screen::os::openh264_codec::quality_provenance::batches_task_b6_quality_provenance_checkout_line_endings_keep_exact_content` zur M11/F38-Zuordnung im selben Remote-Einstieg gemeldet. Das bestehende Präfix `batches_task_b6_quality_` wählt ihn bereits aus. Er benötigt keine physische Bibliothek und darf deshalb nicht als umgebungsabhängig ausgenommen werden. Dieser Autor änderte weder Manifest noch Runner.

Offen bleibt ausschließlich die praktische Bestätigung durch Main: Kompilierung des aktuellen Kandidaten und Auswertung der LF-/CRLF-Inhaltsgrenze auf Windows und Linux sowie des bestehenden physischen F38-Qualitätsfalls mit der jeweiligen gepinnten Bibliothek. Ein fehlender physischer Codec bleibt wie bisher unbestätigt und wird nicht als Qualitätsnachweis gemeldet. Eigene lokale Syntax-/Textbefunde sind kein Laufzeitpass.

Es besteht keine offene Scopeabhängigkeit und kein im eigenen Nachreview verbleibender Quelltextbefund. Der abgegrenzte Autorenauftrag ist abgegeben; Integration, gemeinsame Remote-Ausführung, Commit, Push, Graphaktualisierung und terminaler Release bleiben Main.


