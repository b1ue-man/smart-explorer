# Ersetzen in Dateien, unscharfe Maskierung, Platzhalter mit Rückersetzung – Referenzbefunde

Abgerufen und nachgerechnet am 2026-11-28. Grundlage für drei Nutzervorgaben vom 2026-11-28: „Ersetzen in
Dateien“ zur Volltextsuche (mit MCP), Maskenlisten mit Regex und Platzhaltern in beide Richtungen, unscharfe
Erkennung mit einstellbarer Toleranz je Eintrag. Diese Datei enthält nur Befunde und keinen Entscheid. Sie
legt weder Datenmodell noch Bedienung fest.

Kennzeichen jeder Aussage:

- **[B]** belegt aus Primärquelle (Doku oder Quelltext mit Version, Fachliteratur mit Fundstelle),
- **[R]** nachgerechnet oder beobachtet: Aufruf mit Eingabe und Ergebnis (`python3 -I` 3.12.3, nur Standardbibliothek),
- **[S]** Sekundärquelle oder Suchauszug, Primärquelle nicht gelesen,
- **[A]** Schluss aus gelesenem Quelltext, nicht durch einen Lauf bestätigt,
- **[U]** unbelegt: Lücke, die genannte Tatsache fehlt,
- **[W]** Widerspruch zwischen Quellen oder zwischen Quelle und Annahme.

Quellen Q01 bis Q41 und der DSGVO-Wortlaut stehen in Abschnitt 6. Fremdquelltext lag während der Untersuchung nur im Scratchpad der
Sitzung. Er gehört nicht ins Repository.

## 0. Ausgangslage im Repository (gelesen 2026-11-28)

- [B] Die Suche bildet Notepad++ „Find in Files“ nach: `TextMode` Normal/Erweitert/Regex, Groß/Klein, ganzes
  Wort (bei Regex ohne Wirkung) und „. findet Zeilenumbruch“
  (`native/crates/storage/src/agent_proto/core/content_query.rs`, `TextMode` und `ContentQuery`).
  „Ersetzen in Dateien“ ist ausdrücklich ausgenommen: `docs/oberflaechen/dateisuche.md`, Abschnitt
  „Lücken und Ausnahmen“, und `docs/plaene/2026-11-27-dateien-suche-ansicht/plan.md`, Abschnitt G.
- [B] Abhängigkeiten in `native/Cargo.toml` `[workspace.dependencies]`: `grep-searcher = "0.1.18"`,
  `grep-regex = "0.1.15"`, `grep-matcher = "0.1.10"`, `fancy-regex = "0.16"`, `regex = "1"`. Es gibt dort keine
  Crate für unscharfes Suchen.
- [B] crates.io am 2026-11-28: fancy-regex ist bei **0.20.2**, die 0.16-Reihe endet bei 0.16.2 (Q30). Laut
  `Cargo.toml` hält syntect das Projekt auf 0.16. Diesen Zusammenhang habe ich nicht weiter geprüft.

## 1. Notepad++ „Replace in Files“ / „Replace All“ und Gegenproben

Notepad++ ist beim Tag **v8.9.9.1** (neuester Release, veröffentlicht 2026-11-12, Q01). Den Quelltext habe ich
an diesem Tag gelesen (Q02 bis Q06). Das Handbuch ist `npp-usermanual` im Stand Commit `f70c52eb`
(2026-11-28, Q07). Es wird als npp-user-manual.org/docs/searching/ veröffentlicht.

### 1.1 Modi und Syntax

| Punkt | Befund | Beleg |
|---|---|---|
| Modi | [B] Normal (alles wörtlich), Extended (`\n, \r, \t, \0, \x...`), Regular Expression mit „. matches newline“. Der Modus gilt für **beide** Felder, „Find what“ und „Replace with“. | Q07 `searching.md` „Search Mode“ („This determines how the text in the **Find what** and **Replace with** text fields will be treated“) |
| Extended-Escapes | [B] `\n` LF, `\r` CR, `\t` TAB, `\0` NUL, `\\`, `\bNNNNNNNN` (Binärbyte), `\oNNN` (oktal), `\dNNN` (dezimal), `\xHH`, `\uHHHH`. Laut Handbuch weichen die mit † markierten (`\0 \b \o \d \u`) von Regex ab. | Q07 „Extended Search Mode“. Umsetzung: Q02 `Searching::convertExtendedToString` (Fälle `r n 0 t \\ b d o x u`). |
| Extended beim Ersetzen | [B] `processRange` wandelt bei `FindExtended` den Suchtext **und** den Ersetzungstext mit `convertExtendedToString` um. Danach ersetzt es wörtlich (`replaceTarget`). | Q02 `FindReplaceDlg::processRange`, Zweig `if (pOptions->_searchType == FindExtended)` |
| Stiller Wechsel nach Extended | [B] Ein mehrzeilig ausgewählter Suchtext stellt „Find All …“ und „Find in Files“ im Normal-Modus stillschweigend auf Extended um. | Q07 „Automatic Switch to Extended Search Mode“ |
| Regex-Ersetzung | [B] Notepad++ ruft `SCI_REPLACETARGETRE` auf. In `boostregex/BoostRegExSearch.cxx` heißt das `_match.format(…, boost::format_all)`, also Boost-Extended-Formatsyntax. | Q05 `SubstituteByPosition` |
| Platzhalter in der Ersetzung | [B] `$&`, `$MATCH`, `${^MATCH}`, `$0`, `${0}` (ganzer Treffer); `` $` ``/`$PREMATCH`; `$'`/`$POSTMATCH`; `$^N`/`$LAST_SUBMATCH_RESULT`; `$+`/`$LAST_PAREN_MATCH`; `$$` oder `\$`; `$ℕ`, `${ℕ}`, `\ℕ` (über 9 nur `${ℕ}`); `$+{name}`. `\g…` und `\k…` gelten nur in der Suche. Andere `\x` geben das Zeichen wörtlich aus. | Q07 „Substitutions“ / „Substitution Escape Sequences“ |
| Groß/Klein in der Ersetzung | [B] `\l` und `\u` wirken auf ein Zeichen, `\L` und `\U` bis `\E`. | Q07; Boost Q08 |
| Bedingungen | [B] `?ℕja:nein`, `?{ℕ}ja:nein`, `?{name}ja:nein`, Klammern gruppieren. Wörtliche `( ) ? :` sind zu escapen (`\(` usw.). Wörtlich ist alles außer `$ \ ( ) ? :`. | Q07 „Substitution Conditionals“, „Substitution Grouping“; Q08 |
| Weitere Boost-Escapes | [B] `\a \e \f \n \r \t \v \xDD \x{DDDD} \cX` (Boost-Doku). Laut Notepad++-Handbuch gelten in der Ersetzung auch die Steuerzeichen- und Zeichencode-Schreibweisen der Suche. | Q08; Q07 „Substitution Escape Sequences“ |
| [W] `$0` | Die Boost-Seite „Boost-Extended Format String Syntax“ (1.90) nennt `$0` nicht ausdrücklich, aber `$n`. Notepad++ nennt `$0` und `${0}`. Ich werte das als denselben Fall mit n = 0. Einen Lauf dazu gibt es nicht. | Q07, Q08 |
| Längenbegrenzung | [B] „Find what“ hat 1 bis 16383 **Bytes**, „Replace with“ 0 bis 16383 Bytes. Eine leere Ersetzung löscht. | Q07 |
| Ganzes Wort | [B] Im Regex-Modus ist „Match whole word only“ abgeschaltet und ausgegraut. | Q07 |
| Leere Treffer | [B] Bei Replace All gilt `SCFIND_REGEXP_EMPTYMATCH_NOTAFTERMATCH`: leere Treffer sind erlaubt, aber nicht direkt nach einem Treffer. | Q02 `processRange` |
| Replace All ≠ `/g` | [B] Laut Handbuch entspricht Replace All im Regex-Modus mehrfachem Ausführen und nicht dem Flag `/g` anderer Sprachen. | Q07 „Replace All“ |

Rust-Seite zum Vergleich:

- [B] fancy-regex 0.16.2 `Expander::default()` kennt `$n`, `${name}` und `$$`. `Expander::python()` kennt `\num` und
  `\g<name>`. Groß/Klein-Umschaltung (`\U \L \E \u \l`), Bedingungen (`?n…:…`), `$+{name}`, `` $` ``, `$'` und
  `$+` gibt es dort **nicht** (Q31 `src/expand.rs`).
- [B] `grep_matcher::interpolate` (ripgrep 16.0.0, Q12) kennt nur `$N`, `$name`, `${…}` und `$$`.
- [A] Daraus folgt: Boost-Formatsyntax in Notepad++-Treue müsste se selbst auswerten. Keine der beiden Crates
  bietet sie vollständig.

### 1.2 Ablauf, Bestätigung, Zählung, Meldung

- [B] Bestätigung: Nach dem Klick auf „Replace in Files“ zeigt `replaceInFilesConfirmCheck` einen Dialog mit
  dem Titel „Are you sure?“ und dem Text „Are you sure you want to replace all occurrences in:“, darunter
  `<Ordner>`, „For file type:“ und `<Filter>` (leer wird zu `*.*`). Die Knöpfe sind OK/Abbrechen,
  vorgewählt ist **Abbrechen** (`MB_DEFBUTTON2`). Eine Abschaltung gibt es nicht: Der Dialog kommt immer.
  Fundstellen: Q02 `FindReplaceDlg::replaceInFilesConfirmCheck`, Aufruf im Zweig `IDD_FINDINFILES_REPLACEINFILES`,
  Texte in Q06 `english.xml` (`replace-in-files-confirm-*`).
- [B] Ein leeres „Directory“ startet nichts. Der Fokus springt in das Feld (Q07 „Find in Files tab“, Q02).
- [B] Bestätigt wird nur der Ordner und der Filter. Es gibt **keine** Vorschau und keine Zahl der Treffer
  vor dem Ersetzen (Q02 `replaceInFilesConfirmCheck`).
- [B] Ablauf `Notepad_plus::replaceInFilelist` (Q03, Zeilen ab 2076): Zuerst wird die Dateiliste gebildet,
  mit Fortschrittsdialog „Discovering file candidates...“. Bei mehr als einer Datei folgt der
  Fortschrittsdialog „Replace In Files progress...“ mit Abbrechen und laufender Ersetzungszahl. Je Datei:
  Puffer holen, `processAll(ProcessReplaceAll, …)`, bei `nbReplaced > 0` `saveBuffer` (Zeile 2139),
  den eigens geladenen Puffer schließen (Zeile 2143). Abbrechen bricht nach der aktuellen Datei ab. Bereits
  geschriebene Dateien bleiben geschrieben.
- [B] Ein ungültiger Regex bricht die ganze Schleife ab (`FIND_INVALID_REGULAR_EXPRESSION`, `break`). Die
  Statuszeile zeigt dann den Regex-Fehler (Q03).
- [B] Meldung in der Statuszeile des Dialogs: „Replace in Files: 1 occurrence was replaced“ oder
  „Replace in Files: N occurrences were replaced“ (Q03; Q06 `find-status-replaceinfiles-*`). Eine Liste der
  Dateien und eine Zahl je Datei gibt es nicht.
- [A] Die Gesamtzahl zählt auch Ersetzungen in Dateien, deren Speichern fehlschlug: Der Rückgabewert von
  `saveBuffer` (`SavingStatus`) wird in `replaceInFilelist` nicht ausgewertet (Q03 Zeile 2139). Ein
  Schreibfehler erscheint demnach weder in der Zählung noch in der Meldung.
- [B] Zum Vergleich innerhalb eines Dokuments: Replace All im offenen Dokument klammert die Ersetzung mit
  `SCI_BEGINUNDOACTION`/`SCI_ENDUNDOACTION`, also ein Rückgängig-Schritt (Q02, Zweig `IDREPLACEALL`).
  Für „Replace All in All Opened Documents“ ist eine Bestätigung zuschaltbar, seit v8.9.9 als Kästchen
  neben dem Knopf (Q07).

### 1.3 Kodierung, Zeilenenden, Schreibschutz, offene Dokumente

- [B] Offene Dokumente: Ohne die Einstellung „Find in Files: Ignore unsaved changes in opened files“ (seit
  v8.9.3, Q07 `preferences.md`) nimmt Replace in Files den **Puffer** einer bereits geöffneten Datei
  (`getBufferFromName`), nicht die Datei auf der Platte (Q03 Zeile 2108). Danach speichert es diesen
  Puffer (`saveBuffer`).
- [A] Damit werden auch zuvor ungesicherte Änderungen des Nutzers in dieser Datei mitgespeichert. Ein
  Hinweis darauf fehlt im Handbuch.
- [B] Dateien, die nicht offen sind, lädt `MainFileManager.loadFile` in einen unsichtbaren Puffer, mit der
  üblichen Kodierungserkennung (BOM, sonst UTF-8 wenn gültig, sonst ANSI; Plan 2026-11-27 B1, aus derselben
  Quelle).
- [B] Beim Schreiben behält `FileManager::saveBuffer` den Unicode-Modus des Puffers. UTF-8 ohne BOM wird als
  `uni8Bit` geschrieben („to prevent converter from adding BOM“). Hat der Puffer eine ANSI-Codepage, wandelt
  `WcharMbcsConvertor::encode` von UTF-8 in diese Codepage zurück (Q04).
- [B] Ein Suchtext mit Zeichen außerhalb von ANSI findet in ANSI-Dokumenten nichts. Ein solcher
  Ersetzungstext wird in ANSI-Dokumenten abgelehnt: „Replace: can't replace with non-ANSI text in ANSI
  document“, Ergebnis 0 (Q02 `processAll`, `isSearchUnicodeCharOnAnsi`).
- [A] Zeilenenden: Scintilla hält den Text mit den Original-Zeilenenden. Ersetzt wird byteweise im Puffer.
  Eine Umwandlung der Zeilenenden habe ich in `replaceInFilelist`/`saveBuffer` nicht gefunden. Ein `\n` im
  Ersetzungstext (Extended/Regex) fügt deshalb LF ein, auch in CRLF-Dateien.
- [B] Schreibgeschützt: `processAll` und `processRange` brechen bei `ProcessReplaceAll` sofort mit 0 ab, wenn der
  Puffer `isReadOnly()` meldet. Das ist der Fall bei Dateiattribut `FILE_ATTRIBUTE_READONLY` (`_isFileReadOnly`)
  oder bei Nutzer-Schreibschutz. Die Meldung „Replace All: Cannot replace text. The current document is read
  only.“ setzt `processAll` (Q02; Q04 `Buffer::checkFileState`, `Buffer.h` `isReadOnly`).
- [A] In Replace in Files überschreibt die Schlussmeldung diese Zeile. Eine schreibgeschützte Datei wird also
  still übersprungen.
- [B] Ein Zugriffsfehler beim Öffnen zum Schreiben (`ERROR_ACCESS_DENIED`, etwa ACL) führt in `saveBuffer` in den
  UAC-Erhöhungsweg. Andere Öffnungsfehler ergeben `SaveOpenFailed`. Versteckte oder System-Attribute nimmt
  `saveBuffer` zum Schreiben weg und setzt sie danach wieder (Q04).
- [A] `saveBuffer` öffnet die Zieldatei selbst zum Schreiben (`UnicodeConvertor.openFile(fullpath)`). Ein
  Schreiben in eine Temporärdatei mit anschließendem Umbenennen habe ich dort nicht gefunden. `Utf8_16.cpp`
  habe ich nicht gelesen ([U]: ob `openFile` intern doch atomar schreibt).

### 1.4 Sicherungskopien, Rückgängig

- [B] „Backup on save“ (None / Simple `.bak` / Verbose mit Zeitstempel, eigenes Verzeichnis möglich, Q07
  `preferences.md`) steckt in `Notepad_plus::fileSave` (Q09 `NppIO.cpp` Zeile 1796, `nppgui._backup`).
- [A] Replace in Files ruft dagegen direkt `MainFileManager.saveBuffer` auf (Q03 Zeile 2139). Diese
  Einstellung greift dort also nicht: Notepad++ legt keine Sicherung an.
- [A] Rückgängig: Eigens geladene Puffer werden nach dem Speichern geschlossen (Q03 Zeile 2143). Für diese
  Dateien gibt es kein Rückgängig.
- [U] Ob die Ersetzungen in einer **offenen** Datei in deren Undo-Verlauf landen, habe ich nicht geprüft.
  `replaceInFilelist` setzt dort kein `BEGINUNDOACTION`.

### 1.5 Treffer über Zeilengrenzen

- [B] Gesucht wird im ganzen Dokument, von Anfang bis Ende (`processAll(…, isEntire=true)`, Q03). Regex mit
  `\R`, `\n` oder `(?s)` und Extended mit `\r\n` treffen über Zeilen und werden ebenso ersetzt.
- [B] Das Ergebnisfenster zeigt bei mehrzeiligen Treffern nur die erste Zeile. Der Sprung dorthin markiert den
  ganzen Treffer (Q07 „Search results window“). Zeilen über 1024 Zeichen werden gekürzt (Q07; Q02
  `SC_SEARCHRESULT_LINEBUFFERMAXLENGTH`).

### 1.6 Nur ausgewählte Treffer ersetzen

- [B] Handbuch und Quelltext v8.9.9.1 bieten dafür nichts. Das Kontextmenü des Ergebnisfensters kennt Kopieren,
  Pfade kopieren/öffnen, Alles auswählen, Auf-/Zuklappen, Leeren, „Find in these search results…“,
  Zeilenumbruch und „Purge for every search“ (Q07 „RightClick commands …“). Mit Entf entfernt man Treffer,
  Dateien oder ganze Suchen aus der **Anzeige** (Q07). Replace in Files arbeitet aber auf der neu gebildeten
  Dateiliste (`createFilelistForFiles`) und nicht auf dem Ergebnisfenster (Q03). Entfernte Treffer werden
  also trotzdem ersetzt.
- [S] Eine Websuche nach einem GitHub-Wunsch dazu ergab am 2026-11-28 keinen Treffer. Das ist kein Beweis,
  dass es keinen gibt.

### 1.7 Gegenprobe VS Code 1.148.0

Quelle ist der Quelltext beim Tag `1.148.0` (Q10), dazu die Doku „Basic Editing“ (Q11, Stand laut Seite 11/25/2026).

- [B] Einzelne Treffer: In der Suchansicht gibt es die Aktion „Replace“ je Treffer, „Replace All“ je Datei und je
  Ordner (Tastenkürzel Strg+Umschalt+1). „Dismiss“ (Entf, macOS Cmd+Backspace) **entfernt** einen Treffer oder
  eine Datei aus dem Ergebnis. Was entfernt ist, wird nicht ersetzt, weil Replace All die verbleibenden
  Ergebniselemente nimmt (`searchResult.replaceAll`). Fundstellen: Q10 `searchActionsRemoveReplace.ts`
  (`RemoveAction.label "Dismiss"`, `match.replace.label`, `file.replaceAll.label`), `searchView.ts`.
- [B] Vorschau: Beim Eintippen des Ersatztexts zeigt VS Code eine Diff-Ansicht der anstehenden Änderungen.
  `openReplacePreview` öffnet „`<Datei>` ↔ `<Datei>` (Replace Preview)“ (Q11; Q10 `replaceService.ts`).
- [B] Bestätigung vor Replace All: Dialog „Replace All“ mit „Replace N occurrences across M files with 'X'?“,
  Knopf „Replace“. Danach meldet VS Code „Replaced N occurrences across M files with 'X'.“ (Q10 `searchView.ts`,
  `buildReplaceAllConfirmationMessage`, `buildAfterReplaceAllMessage`).
- [B] Anwenden: Die Änderungen gehen über `IBulkEditService.apply`. Danach wird jede betroffene Datei, die als
  Textdateimodell vorliegt, gespeichert (`textFileService.files.get(resource)?.save`) (Q10 `replaceService.ts`).
- [U] Ob Replace All danach als **ein** Rückgängig-Schritt über alle Dateien zurückgeht, ist weder in der Doku
  (Q11) noch in den gelesenen Dateien belegt.
- [B] Ersetzungssyntax (JavaScript-Regex): `\n`, `\t`, `\\`, `\u`, `\U`, `\l`, `\L` (wirken nur auf den
  Ersetzungstext), `$$`, `$&` und `$0`, `$n` für n < 100. Alles andere bleibt unverändert (Q10
  `editor/contrib/find/browser/replacePattern.ts` `parseReplaceString`). Es gibt kein `\E` und keine
  Bedingungen. „Preserve Case“ ist eine eigene Option (`buildReplaceStringWithCasePreserved`).
- [B] Mehrzeilig: Text mit Zeilenumbrüchen lässt sich in Such- und Ersetzungsfeld einfügen. Strg+Enter fügt einen
  Zeilenumbruch ein (Q11).

### 1.8 Gegenprobe ripgrep 16.0.0 `--replace`

- [B] „Replaces every match with the text given when printing results. Neither this flag nor any other ripgrep
  flag will modify your files.“ `$5`, `$name`, `${1}a` und `$$` funktionieren. Ein ungültiger Bezug wird leer.
  Ersetzt wird der Treffer, nicht die Zeile (Q12 `crates/core/flags/defs.rs`, `struct Replace`).
- [A] ripgrep liefert also eine Ausgabevorschau ohne Schreiben. Für se ist das ein Muster für eine
  Vorschau- oder Trockenlaufstufe vor dem Schreiben.

## 2. Unscharfe Teilstring-Suche für Maskeneinträge

### 2.1 Verfahren und Laufzeit

| Verfahren | Befund | Beleg |
|---|---|---|
| Levenshtein-Distanz | [B] kleinste Zahl von Einfügen, Löschen und Ersetzen | Levenshtein 1966 (Q13, [S]: Fundstelle aus der Literatur, Volltext nicht gelesen) |
| Damerau / OSA | [B] Damerau 1964 nimmt die Vertauschung benachbarter Zeichen als vierte Operation hinzu. Lucene FuzzyQuery rechnet mit „Damerau-Levenshtein (optimal string alignment)“, `defaultTranspositions = true`. | Q14 (CACM 7(3):171–176, DOI geprüft); Q20 |
| Teilstring-DP (Sellers) | [B] Das Muster muss ganz passen, der Textanfang ist frei: O(m·n) | Q15 (J. Algorithms 1(4):359–373, 1980, DOI geprüft) |
| Ukkonen „cut-off“ | [B] erwartet O(k·n), O(m) Speicher. Spalten werden nur bis zur letzten aktiven Zelle (Wert ≤ k) berechnet. Bewiesen von Chang/Lampe 1992 und Baeza-Yates/Navarro. | Q16 (Inf. & Control 64:100–118, 1985); Q17 Navarro-Übersicht, Abschnitt „cut-off heuristic“ |
| Myers bitparallel | [B] O(⌈m/w⌉·n) mit Wortbreite w. Bei Mustern ≤ 64 (u64) oder ≤ 128 (u128) Zeichen ist das linear in n, bei langen Mustern (Blöcke) O(n·k). Start und Ausrichtung kosten O(m+k). | Q18 (JACM 46(3):395–415, 1999, DOI geprüft); Q24 `bio` Modul `pattern_matching::myers` |
| Wu-Manber / agrep | [B] bitparallele Suche mit k Fehlern und Shift-Or-Erweiterung | Q19 (CACM 35(10):83–91, 1992, DOI geprüft) |
| TRE (ungefährer Regex) | [B] Fehlergrenzen und Kosten **je Teilausdruck**: `{~3}`, `{+2~5}`, `{ 2i + 1d + 2s < 5 }`. Einfügen (`+`), Löschen (`-`), Ersetzen (`#`) und Gesamt (`~`) sind getrennt begrenzbar. Gewichtet werden nur diese drei Operationen, nicht einzelne Zeichenpaare. | Q21 `doc/tre-syntax.html` (TRE 0.9.0, Commit `d2c19b74`) |
| Filter für viele Muster | [B] Wird das Muster in k+1 Stücke geteilt, bleibt in jedem Vorkommen mit k Fehlern mindestens ein Stück unverändert. Man sucht die Stücke exakt, etwa alle zugleich mit Aho-Corasick in O(n), und prüft dann die Umgebung (Länge m+2k). Bei Vertauschungen braucht es 2k+1 Stücke. | Q17, Abschnitt 8.2 „Moderate Patterns“, Absatz „Wu and Manber 1992“ (Preprint-Fassung) |
| Gewichtete Mehrzeichen-Ersetzungen | [B] Brill/Moore nehmen beliebige Ersetzungen α→β (α, β ∈ Σ*) mit eigener Wahrscheinlichkeit, z. B. P(ant∣ent), statt nur Einzelzeichen. Kolak/Byrne/Resnik modellieren OCR mit Segmentierungsfehlern (Wörter zusammengezogen oder getrennt) und Ersetzungen von Zeichenfolgen. | Q22 (ACL 2000, S. 286–293), Abschnitt 2; Q23 (HLT-NAACL 2003, S. 55–62), Abschnitte 2.3 und 2.4 |

- [A] Gewichte unter 1 hebeln den k+1-Stücke-Filter aus, wenn k in Kostenpunkten gemessen wird. Bei Kosten 4 und
  OCR-Ersetzungen zu je 0,4 sind bis zu 10 Operationen möglich. Der Filter muss dann mit der Höchstzahl der
  **Operationen** rechnen: ⌊Höchstkosten / kleinste Einzelkosten⌋.
- [A] Ebenso rechnet Myers' Verfahren nur mit Einheitskosten. Gewichtete Kosten oder Mehrzeichen-Ersetzungen
  (rn→m) gehen dort nur als Vorfilter mit Einheitskosten. Die Prüfung läuft dann mit einer gewichteten DP auf
  dem Kandidatenfenster.

### 2.2 Normalisierung

- [R] Faltung `casefold` + NFKD ohne Kombinationszeichen, dazu Entfernen von Leerraum, `-`, weichem Trennzeichen
  U+00AD, U+2010 bis U+2013 und U+00A0: `fold("Schwein Braten") = "schweinbraten"`, `fold("Schweins-\nbraten") =
  "schweinsbraten"`, `fold("Straße") = fold("STRASSE") = "strasse"`, `fold("Müller") = "muller"`, aber
  `fold("Mueller") = "mueller"` (Distanz 1 zu „muller“). Skript:
  `scratchpad/scripts/fuzzy_calc.py` der Sitzung.
- [A] Die NFKD-Faltung macht ä zu a, nicht zu ae. Die deutsche Ersatzschreibung ae/oe/ue braucht deshalb einen
  eigenen Eintrag als Mehrzeichen-Gleichsetzung (ä↔ae) oder kostet einen Fehler.
- [R] Byte- gegen Zeichenebene: Levenshtein(„Müller“, „Muller“) ist 1 auf Zeichen, aber **2** auf UTF-8-Bytes.
  Ebenso Levenshtein(„Schweinsbraten“, „Schweinsbräten“): 1 auf Zeichen, 2 auf Bytes. Byte-Crates
  (triple_accel, bio Myers mit `u8`) verfälschen also Schwellen bei Umlauten, wenn man nicht vorher auf ein
  eigenes Alphabet abbildet.
- [R] Fallstrick der Leerraum-Entfernung: Im Text „Heute gibt es Weinbraten mit Knoedel.“ ist die billigste
  Fundstelle nach der Faltung **„sweinbraten“** (Kosten 3,0). Das „s“ stammt aus „es“. Wird Leerraum vor der
  Suche global entfernt, leiht sich der Treffer Zeichen aus dem Nachbarwort. Die Maske würde dann in „es“
  hineinreichen.
- [A] Daraus folgt: Fundstellen müssen in den Originaltext zurückgerechnet werden (Positionstabelle der
  Faltung) und auf Wortgrenzen gezogen werden, oder Leerraum wird nur **innerhalb** eines Kandidatenfensters
  verworfen.
- [U] Silbentrennung mit echtem Trennstrich am Zeilenende („Schweins-⏎braten“) lässt sich nicht von einem echten
  Bindestrich-Kompositum („Schweins-Braten“) unterscheiden. Für die Maskierung schadet das nicht, beides soll
  maskiert werden. Für das Zurückschreiben der Originalform siehe 3.6.

### 2.3 OCR-Verwechslungen

- [B] Brill/Moore 2000 (Q22) und Kolak/Byrne/Resnik 2003 (Q23) begründen gewichtete Ersetzungen von Zeichenfolgen
  im Fehlermodell. Kolak/Resnik 2005 (Q25, HLT/EMNLP 2005, S. 867–874) trainieren ein Mehrzeichen-Fehlermodell
  für OCR-Nachkorrektur.
- [S] Vorlesungsfolien „OCR Post-Processing“ (M. Richter, Saarbrücken, Q26): „low cost for in ↔ m, high cost
  for w ↔ R“. Das ist das einzige gelesene Dokument mit einem konkreten Paar.
- [U] Eine Primärquelle mit einer Tabelle der Paare rn↔m, 0↔O, 1↔l↔I, ß↔B und ihren Häufigkeiten habe ich nicht
  gefunden. Die in 2.4 verwendeten Paare und Kosten (0,3 bis 0,4) sind **Annahmen zur Nachrechnung** und nicht
  kalibriert.
- [B] Fertige Bausteine für Gleichsetzungen: `bio` `MyersBuilder::ambig(symbol, äquivalente)` nur für einzelne
  Symbole und nur auf der Musterseite, Kosten 0 (Q24 `myers/builder.rs`). fuzzy-aho-corasick
  „Multi-Character Mappings … (bidirectional, scored)“, z. B. `ß`↔`ss` (Q27 README).

### 2.4 Nachgerechnete Beispiele für „Schweinsbraten“

[R] Ich habe mit `python3 -I scratchpad/scripts/fuzzy_calc.py` und `budget_calc.py` gerechnet, Eingaben wie in
der Tabelle. Das Muster wird gefaltet zu `schweinsbraten`, Länge L = 14. „Lev roh“ ist die Levenshtein-Distanz
ohne Faltung. „OSA gef.“ ist OSA nach der Faltung, „rel.“ = OSA gef. / 14. „Zwei-Budget“ ist die Regel unten
(Annahme zur Prüfung, kein Entscheid).

| Kandidat | soll | Lev roh | OSA gef. | rel. | Kürzung vorn+hinten | innere Kosten (gewichtet) | Zwei-Budget |
|---|---|---|---|---|---|---|---|
| Schweinsbraten | ja | 0 | 0 | 0,00 | 0+0 | 0,0 | ja |
| Schwein Braten | ja | 2 | 1 | 0,07 | 0+0 | 1,0 | ja |
| Schweins-⏎braten | ja | 2 | 0 | 0,00 | 0+0 | 0,0 | ja |
| Weinsbraten | ja | 4 | 3 | 0,21 | 3+0 | 0,0 | ja |
| Weinbraten | ja | 5 | 4 | 0,29 | 3+0 | 1,0 | ja |
| Schweinsbratcn (OCR e→c) | ja | 1 | 1 | 0,07 | 0+0 | 0,4 | ja |
| Scbweinsbraten (OCR h→b) | ja | 1 | 1 | 0,07 | 0+0 | 0,4 | ja |
| Schwcinsbratcn (2× e→c) | ja | 2 | 2 | 0,14 | 0+0 | 0,8 | ja |
| Schweinsbarten (Dreher) | ja | 2 | 1 | 0,07 | 0+0 | 1,0 | ja |
| Schweinebraten | offen | 1 | 1 | 0,07 | 0+0 | 1,0 | ja |
| Weinbrand | nein | 8 | 7 | 0,50 | 3+1 | 3,0 | nein |
| Schweinshaxe | nein | 4 | 4 | **0,29** | 0+1 | 2,4 | nein (knapp) |
| Schweinsohren | nein | 4 | 4 | **0,29** | 0+0 | 3,4 | nein |
| Rinderbraten | nein | 7 | 7 | 0,50 | 4+0 | 4,0 | nein |
| Braten | nein | 9 | 8 | 0,57 | 8+0 | 0,0 | nein (Kürzung 8 > 4) |
| Schwein | nein | 7 | 7 | 0,50 | 0+7 | 0,0 | nein (Kürzung 7 > 4) |
| Weinberg | nein | 10 | 9 | 0,64 | 3+3 | 3,0 | nein |
| `Sx#q!nzvr4tqm` (Müll) | nein | 10 | 10 | 0,71 | 0+0 | 10,0 | nein |
| `ll1\|;:.,'iI!` (Müll) | nein | 14 | 14 | 1,00 | 3+0 | 11,3 | nein |
| `Scwnbrtxq` (Müll) | nein | 7 | 7 | 0,50 | 0+0 | 7,0 | nein |

Befunde aus der Tabelle:

- [R] **Eine einzige relative Schwelle trennt nicht.** „Weinbraten“ (soll maskiert werden) hat OSA 4, rel. 0,29.
  Die gewöhnlichen Wörter „Schweinshaxe“ und „Schweinsohren“ haben ebenfalls 4 und 0,29. Jede Schwelle auf der
  Gesamtdistanz, die „Weinbraten“ trifft, trifft diese beiden auch.
- [R] Elasticsearch `AUTO:3,6` ergibt für L = 14 genau **2** Änderungen (Q28). Damit fallen „Weinsbraten“ (3) und
  „Weinbraten“ (4) heraus. Die Nutzervorgabe verlangt mehr als AUTO.
- [R] Die geprüfte **Zwei-Budget-Regel** trennt alle 20 Fälle:
  - (a) Kürzung an den Wortenden bis ⌊0,3·L⌋ = 4 Zeichen, vorn und hinten zusammen;
  - (b) auf dem verbleibenden Kern innere gewichtete Kosten höchstens AUTO(Kernlänge), also 2 für Kerne ab 6
    Zeichen;
  - (c) OCR-Paare kosten 0,4 statt 1, Vertauschung 1.

  „Schweinshaxe“ liegt mit 2,4 nur 0,4 über dem Budget. Der Abstand ist klein und hängt an den angenommenen
  Gewichten.
- [R] Ohne Kürzungs-Budget (Kürzung kostet dasselbe wie ein Fehler) bekommen „Braten“ und „Schwein“ in der
  Variante „gewichtete Teilstringkosten mit Kürzung 0,5“ die Werte 4,0 und 3,5, „Weinbraten“ 2,5. Das liegt nah
  beieinander. Die Begrenzung der Kürzung auf einen Anteil von L ist deshalb nötig, damit Teilwörter nicht
  maskiert werden.
- [A] OCR-Müll fällt bei jeder geprüften Regel weit heraus: innere Kosten ≥ 7 bei Budget 2.
- [U] Ob „Schweinebraten“ oder Komposita wie „Schweinsbratensoße“ (enthält den Eintrag exakt) maskiert werden
  sollen, legt die Nutzervorgabe nicht fest. Das ist eine Entscheidung im Plan.

### 2.5 Wie etablierte Werkzeuge die Toleranz einstellbar machen

| Werkzeug | Einstellung | Beleg |
|---|---|---|
| Elasticsearch | [B] `fuzziness`: `0`, `1`, `2` oder `AUTO`. Ohne Angabe gilt `AUTO:3,6`: Länge 0–2 exakt, 3–5 eine Änderung, über 5 zwei Änderungen. „`AUTO` should generally be the preferred value“. Bei `match` gelten `prefix_length` (Standard 0), `max_expansions` (50) und `fuzzy_transpositions` (Standard `true`). „Fuzzy matching builds a `fuzzy` query for each term“, das heißt je Term. | Q28, Q29 (Seiten ohne Versionsangabe, Stand 2026-11-28) |
| Lucene 10.3.0 | [B] `FuzzyQuery`: höchstens 2 Änderungen (`defaultMaxEdits = 2`, `LevenshteinAutomata.MAXIMUM_SUPPORTED_DISTANCE = 2`), `defaultPrefixLength = 0`, `defaultMaxExpansions = 50`, `defaultTranspositions = true`, `floatToEdits(minimumSimilarity, termLen)` | Q20 |
| Solr 10.1 | [B] `roam~` oder `roam~1`, Distanz 0 bis 2, Standard 2 | Q32 |
| TRE agrep 0.9.0 | [B] `-#` (0–9) oder `-E NUM` für die Höchstfehlerzahl. `-D`, `-I`, `-S` setzen die Kosten für Löschen, Einfügen und Ersetzen. `-B` liefert nur die besten Treffer, `-w` ganze Wörter (Wortgrenzen zählen nicht als Fehler). | Q21 `doc/agrep.1.in` |
| [W] „agrep -k“ | In TRE agrep heißt `-k` **`--literal`** (Muster ohne Sonderzeichen), **nicht** Fehlerzahl. Die Fehlerzahl ist `-#`/`-E`. Die Nutzerfrage nennt „agrep -k“ als Toleranz. Das stimmt mit dieser Quelle nicht überein. Die Optionen des ursprünglichen Wu-Manber-agrep habe ich nicht gelesen ([U]). | Q21 |
| Volltextsuche in OCR-Archiven | [S] Trove (NLA): Ein Tippsblatt nennt `~` für unscharfe Suche. Die Hilfeseite war am 2026-11-28 nicht abrufbar („Access Denied“, Anubis). [U]: Ein Archiv mit dokumentierter Toleranz je Suchwort über 2 Änderungen habe ich nicht gefunden. Solr und Lucene (Obergrenze 2) liegen vielen solchen Archiven zugrunde. Das ist eine Annahme, nicht je Archiv geprüft. | Q33 (Suchauszug) |

- [A] Keines dieser Werkzeuge bietet eine Kürzung am Wortrand als eigenes Budget. Lucenes `prefix_length` wirkt
  umgekehrt: Es verlangt einen exakten Anfang. Das Wegfallen von „Sch“ ist dort nur über die Gesamtdistanz
  abgedeckt, also höchstens 2.

### 2.6 Rust-Crates (crates.io, abgefragt 2026-11-28)

| Crate | Version | Lizenz | letzte Veröffentlichung | passt für | Grenzen |
|---|---|---|---|---|---|
| strsim | 0.11.1 | MIT | 2024-04-02 | Distanzen ganzer Zeichenketten auf `char`: `levenshtein`, `osa_distance`, `damerau_levenshtein`, `normalized_*`, `jaro_winkler`, `sorensen_dice` | [B] keine Teilstringsuche, keine Gewichte (Q34 `src/lib.rs`) |
| rapidfuzz (rapidfuzz-rs) | 0.5.0 | MIT | 2023-12-01 | Levenshtein mit `WeightTable` (Einfügen, Löschen, Ersetzen je ein Wert), OSA, Damerau, Indel, LCS, Jaro, `fuzz::ratio` | [B] kein `partial_ratio` (Teilstring), keine Gewichte je Zeichenpaar (Q35 `src/distance/levenshtein.rs`, `src/fuzz.rs`) |
| triple_accel | 0.4.0 | MIT | 2021-06-18 | Teilstringsuche `levenshtein_search_simd_with_opts(needle, haystack, k, SearchType, EditCosts, anchored)`, SIMD AVX2/SSE4.1, sonst skalar; `EditCosts` (Fehlpaarung, Lücke, Lückenstart, Vertauschung), Kosten u8 | [B] arbeitet auf **Bytes**, einheitliche Fehlpaarungskosten, SIMD nur x86 (Q36 `src/lib.rs`, `src/levenshtein.rs`). [A] Seit 2021 keine neue Version. |
| bio | 4.2.2 | MIT | 2026-10-12 | `pattern_matching::myers` (Myers 1999; `Myers::<u64>`/`<u128>`, `long::Myers` für lange Muster, `find_all` mit Start/Ende/Distanz, Ausrichtung), `MyersBuilder::ambig`, `pattern_matching::ukkonen` (O(n·k) auf Zufallstext, Vertauschung, eigene Kostenfunktion) | [B] Symbole `u8`. [B] 32 direkte Abhängigkeiten (u. a. ndarray, petgraph, statrs, csv, serde), also ein großer Baum für eine Teilfunktion (Q24 `Cargo.toml`) |
| levenshtein_automata | 0.2.1 | MIT | 2021-05-18 | Levenshtein-DFA (`LevenshteinAutomatonBuilder::new(max_distance, transposition_cost_one)`, `build_dfa`, `build_prefix_dfa`) gegen Wortlisten oder FST (tantivy) | [B] Wort-gegen-Wort, keine Teilstringsuche im Fließtext (Q37) |
| fuzzy-aho-corasick | 0.5.2 | MIT | 2026-10-03 (angelegt 2025-05-21, 32 Versionen) | [B] mehrere Muster zugleich, unscharf (Einfügen, Löschen, Ersetzen, Vertauschen), Grenzen global **oder je Muster**, Graphem-Cluster, Mehrzeichen-Gleichsetzungen mit Kosten, Auswahl ohne Überlappung, „Fuzzy Replacer“, Streaming über `Read` mit u64-Positionen, bitparalleler Vorfilter | [A] junges Projekt mit einem Autor, 10 560 Downloads. Abhängigkeiten `memchr`, `unicode-segmentation`, 3 Treffer für `unsafe` in `src`. Nicht gebaut und nicht ausprobiert (Projektregel) (Q27 README, `Cargo.toml`, `src/builder.rs`) |
| sassy | 0.3.1 | MIT | 2026-11-21 | SIMD-Suche (AVX2 und NEON) kurzer Muster, `agrep`-Modus für ASCII | [B] für DNA entworfen, Alphabete ASCII/DNA/IUPAC (Q38 README) |
| tre-regex | 0.6.0 | BSD-2-Clause | 2026-08-07 | Bindungen an TRE (ungefährer Regex) | [A] C-Bibliothek über FFI, nicht geprüft |
| aho-corasick | 1.1.5 | Unlicense OR MIT | 2026-08-03 | exakte Mehrmustersuche, etwa für Stücke beim k+1-Filter | [B] Metadaten Q30 |
| nucleo-matcher / fuzzy-matcher / sublime_fuzzy | 0.3.1 / 0.3.7 / 0.7.0 | MPL-2.0 / MIT / nicht standardisiert | 2024 / 2020 / 2020 | Teilfolgen-Bewertung für Dateiauswahl (fzf-Art) | [A] Teilfolgen-Treffer, keine Edit-Distanz. Für Maskierung ungeeignet, weil sie verstreute Buchstaben treffen. |

- [A] Laufzeit bei E Einträgen und Textlänge n: Ein Myers-Lauf je Eintrag kostet O(E·n) bei Mustern ≤ 64
  Zeichen. Der k+1-Filter mit **einem** Aho-Corasick-Automaten über alle Stücke aller Einträge kostet O(n) plus
  Prüfungen (Q17). Er wächst also nicht mit E, solange die Stücke selten treffen. Kurze Einträge mit hoher
  Toleranz ergeben kurze Stücke und damit viele Prüfungen.
- [U] Messwerte für se-typische Textmengen (Anzahl Einträge, Dateigrößen) gibt es nicht. Hier wurde nichts
  gebaut oder gemessen.

## 3. Platzhalter und Rückersetzung

### 3.1 Microsoft Presidio 2.2.366 (Tag, Commit `211afc33`, Q39)

- [B] Operatoren: `replace` (Standard `<ENTITY_TYPE>`), `redact`, `hash` (sha256/sha512; seit 2.2.361 zufälliges
  Salz je Entität, eigenes Salz für gleiche Hashes, mindestens 128 Bit), `mask`, `encrypt`, `custom` (Lambda),
  `surrogate_ahds`, `keep`. Zum Zurückführen gibt es nur `decrypt` (`docs/anonymizer/index.md`, „Built-in
  operators“).
- [B] `encrypt`/`decrypt`: AES-CBC mit zufälligem IV und PKCS7, Ausgabe Base64url aus IV und Chiffrat, Schlüssel
  128/192/256 Bit (`operators/aes_cipher.py`, `encrypt.py`). [A] Der Platzhalter ist das Chiffrat selbst, also
  lang und für ein Sprachmodell unlesbar. Bei jedem Aufruf ist er anders. Eine Authentisierung (MAC) gibt es
  nicht.
- [B] `DeanonymizeEngine.deanonymize(text, entities: List[OperatorResult], operators)` braucht die **Positionen**
  der Platzhalter im Text (`deanonymize_engine.py`). Ersetzt wird von hinten nach vorn (`sorted(…,
  reverse=True)` in `core/engine_base.py`).
- [A] Daraus folgt: Für Text, den ein Agent neu schreibt (andere Positionen), taugt Presidios Rückweg nicht ohne
  eigene Suche nach den Platzhaltern.
- [B] Pseudonymisierung mit Zuordnungstabelle steht nur als Beispiel im Notebook
  `docs/samples/python/pseudonymization.ipynb`. `InstanceCounterAnonymizer` mit `REPLACING_FORMAT =
  "<{entity_type}_{index}>"` nutzt eine Tabelle `entity_mapping[typ][wert] = platzhalter`, die der Aufrufer
  hält. Gleicher Wert ergibt gleichen Platzhalter. Ausgabe des Beispiels:
  `<PERSON_1> gave his book to <PERSON_2> … <PERSON_0> …`. Hinweise dort: „not thread-safe“, „the user/client
  is holding the entity_mapping“.
- [B] Überlappungen („Handling overlaps between entities“): Bei voller Überdeckung gewinnt der höhere Score,
  bei gleichem Score zufällig. Ist eine Entität in einer anderen enthalten, gewinnt die längere, auch bei
  niedrigerem Score. Bei teilweiser Überschneidung wird jede einzeln ersetzt und aneinandergehängt, z. B.
  `I'm <PERSON><LOCATION>.`

### 3.2 LLM Guard v0.3.16 (Tag, Commit `32b14a4a`, 2025-05-19, Q40)

- [B] Platzhalter `[REDACTED_{entity_type}_{index}]` (`input_scanners/anonymize.py`
  `_get_entity_placeholder`). Mit `use_faker` gibt es erfundene, echt wirkende Werte statt Platzhaltern. Ein
  schon im Vault vorhandener Wert bekommt denselben Platzhalter.
- [B] `Vault` ist eine Liste von Tupeln `(platzhalter, wert)` im Speicher, ohne Verschlüsselung und ohne
  Persistenz (`llm_guard/vault.py`).
- [B] Rückweg `Deanonymize` mit `MatchingStrategy` EXACT (`str.replace` je Vault-Eintrag), CASE_INSENSITIVE
  (`re.sub(platzhalter, wert, text, flags=IGNORECASE)`), FUZZY (`fuzzysearch.find_near_matches(…,
  max_l_dist=3)`) und COMBINED_EXACT_FUZZY (`output_scanners/deanonymize.py`).
- [R] Fehler in CASE_INSENSITIVE, beobachtet: `re.sub('[REDACTED_PERSON_1]', 'Max Muster', 'Hallo
  [REDACTED_PERSON_1], Termin am 2.', flags=re.IGNORECASE)` ergibt `'HMax MusterllMax Muster [Max MusterMax
  Muster…], Max MusterMax MusterMax MustermiMax Muster Max Musterm 2.'`. Der Platzhalter wird unescaped als
  Zeichenklasse gelesen.
- [R] Ein echter Wert mit Backslash als Ersetzung, etwa `C:\Users\x`, lässt `re.sub` mit `bad escape \U at
  position 2` abbrechen.
- [A] FUZZY mit Abstand 3 kann im Agententext Stellen, die einem Platzhalter ähneln, durch echte Werte ersetzen.
  Der Rückweg wird also selbst unscharf.

### 3.3 LiteLLM (Presidio-Guardrail, `main` bei Commit `3261387b`, Q41)

- [B] Mit `output_parse_pii` gilt: nummerierte Platzhalter `<{entity_type}_{n}>`, Zuordnung
  `request_data["metadata"]["pii_tokens"][platzhalter] = original` je Anfrage. Ist derselbe Platzhalter in der
  Tabelle schon an einen **anderen** Wert vergeben, nimmt LiteLLM eine Ersatznummer (`spare_numbers`). Positionen
  stammen aus der Analyse des Originaltexts, ersetzt wird von hinten
  (`_finalize_presidio_anonymize_numbered_tokens`).
- [B] Rückweg `_unmask_pii_text`: `str.replace` je Platzhalter. Zusätzlich gibt es einen Rückfall für Platzhalter,
  die am **Ende** abgeschnitten wurden (`max_tokens`): Überlappung von mindestens `min(20, len(token)//2)` Zeichen.
- [B] Beim Streaming puffert LiteLLM Abschnitte. Bei gemischten Formaten gibt es sie ungemasked weiter
  (`_flush_unmaskable_buffer`, Warnung im Log).
- [A] Ein über Abschnittsgrenzen geteilter Platzhalter ist das Gegenstück zum Treffer über Seitengrenzen (3.6).

### 3.4 Gestaltung der Platzhalter

- [B] Belegte Formen: `<PERSON>` (Presidio-Standard, ohne Zähler, also nicht eindeutig umkehrbar),
  `<PERSON_1>` (Presidio-Beispiel, LiteLLM), `[REDACTED_PERSON_1]` (LLM Guard), Chiffrat (Presidio `encrypt`),
  erfundene Werte (LLM Guard `use_faker`, Presidio `surrogate_ahds`).
- [R] Das schließende Zeichen verhindert Präfix-Kollisionen bei `str.replace`: `'[REDACTED_PERSON_1] und
  [REDACTED_PERSON_10]'` mit nacheinander `…_1]`→A, `…_10]`→B ergibt `A und B`. Ohne schließendes Zeichen ergibt
  `'<PERSON_1> und <PERSON_10>'.replace('<PERSON_1', 'A')` dagegen `A> und A0>`.
- [A] Erfundene Werte sind für ein Sprachmodell am natürlichsten. Sie lassen sich aber nicht sicher zurückführen,
  wenn der erfundene Wert auch echt im Text vorkommt. Kollisionsfrei werden Platzhalter erst durch eine Form, die
  im Quelltext praktisch nicht vorkommt.
- [U] Zur Frage, welche Platzhalterform Sprachmodelle am zuverlässigsten unverändert wiedergeben, habe ich keine
  Messung und keine Studie gefunden.

### 3.5 Speicherung und Schutz der Zuordnung

- [B] Presidio: Die Zuordnung hält der Aufrufer, die Bibliothek speichert nichts (Q39 Notebook). LLM Guard:
  Liste im Prozessspeicher (Q40). LiteLLM: Wörterbuch in den Metadaten der jeweiligen Anfrage (Q41). Keine der
  drei verschlüsselt die Zuordnung oder schreibt sie dauerhaft.
- [S] DSGVO Art. 4 Nr. 5 (Wortlaut über dsgvo-gesetz.de, EUR-Lex lieferte am 2026-11-28 eine leere Antwort):
  „Pseudonymisierung“ ist die Verarbeitung so, „dass die personenbezogenen Daten ohne Hinzuziehung zusätzlicher
  Informationen nicht mehr einer spezifischen betroffenen Person zugeordnet werden können, sofern diese
  zusätzlichen Informationen gesondert aufbewahrt werden und technischen und organisatorischen Maßnahmen
  unterliegen …“
- [A] Für se heißt das: Die Zuordnungstabelle ist diese „zusätzliche Information“. Sie muss dort liegen und
  geschützt sein, wo der Agent nicht lesen kann. Ein Agent, der über MCP `read`, `grep` oder `list` die
  gespeicherte Maskenliste erreicht, hebelt die Vorgabe „für den Agenten NICHT lesbar“ aus.
- [U] Wie se seine eigenen Geheimnisse ablegt, habe ich nicht geprüft (Lesegrenze dieses Auftrags). Das muss die
  Planung klären.

### 3.6 Fallstricke

| Fallstrick | Befund |
|---|---|
| Platzhalter steht schon im Originaltext | [A] Keines der drei Werkzeuge prüft das. Steht etwa `[REDACTED_PERSON_1]` wörtlich in einer Datei und der Agent schreibt diesen Text zurück, macht der exakte Rückweg (LLM Guard EXACT, LiteLLM `str.replace`) daraus den echten Wert. Ein Geheimnis gelangt so in eine Datei, die es nie enthielt. Gegenmittel: Platzhalterform mit Sitzungskennung, und vorhandene Vorkommen der Platzhalterform beim Maskieren selbst als „wörtlich“ kennzeichnen. |
| Nummernkollision zwischen Teilen | [B] LiteLLM vergibt Ersatznummern, wenn ein Platzhalter schon an einen anderen Wert gebunden ist (3.3). |
| Überlappende Muster | [B] Presidio: Bei voller Überdeckung gewinnt der Score, sonst die längere Entität, Teilüberschneidungen werden einzeln ersetzt (3.1). [B] fuzzy-aho-corasick bietet eine Auswahl ohne Überlappung mit einstellbarer Heuristik (Q27). [A] Bei unscharfen Treffern enden mehrere Fundstellen an benachbarten Positionen (triple_accel-Beispiel: `Match{start:2,end:4,k:1}` und `Match{start:2,end:5,k:1}`, Q36). Ohne Auswahlregel ersetzt man doppelt oder versetzt. |
| Unscharfer Treffer leiht sich Nachbarzeichen | [R] siehe 2.2 („sweinbraten“ aus „es Weinbraten“) |
| Seitengrenzen beim seitenweisen Lesen | [B] MCP `read` liest nach Zeilen mit `line`/`lines` und `next_line` (`docs/oberflaechen/dateisuche.md`, MCP). [A] Wird jede Seite für sich maskiert, wird ein Eintrag, der über die Seitengrenze läuft (mehrzeiliger Regex, „Schweins-⏎braten“), auf keiner Seite erkannt und gelangt ungemasked zum Agenten. Gegenmittel: die ganze Datei maskieren und dann aufteilen, oder ein Überlappungsfenster von mindestens der größten möglichen Trefferlänge. Dieselbe Lage beschreibt LiteLLM beim Streaming (3.3). |
| Längenänderung, Zeilennummern, Spalten | [B] Presidio liefert für den maskierten Text neue Positionen (`OperatorResult` start/end, 3.1). [A] Ersetzt ein Platzhalter einen mehrzeiligen Treffer, verschieben sich alle späteren Zeilennummern. Das trifft die Zeilen aus `grep`, `read` mit `line` und die `ranges` in `se grep --json`. Ein Agent, der danach nach Zeilennummern oder Positionen editiert, trifft im Original die falsche Stelle. Gegenmittel: Zeilenumbrüche im Platzhalter erhalten (gleiche Zahl `\n`) oder alle Positionen über eine Abbildungstabelle zurückrechnen. |
| Rückersetzung verliert die Originalform | [A] Wurde „Schwein Braten“ oder „Weinbraten“ unscharf maskiert, entscheidet die Rückersetzung, welcher Text eingesetzt wird: der Listeneintrag oder die tatsächliche Fundform. Die Nutzervorgabe sagt „durch die echten Werte aus der Maskenliste“. Die Fundform je Fundstelle müsste zusätzlich gespeichert werden, wenn Dateien beim Zurückschreiben unverändert bleiben sollen, außer an den Stellen, die der Agent geändert hat. [U] Das ist nicht entschieden. |
| Regex in Rückersetzung und Ersetzungstext | [R] Ein echter Wert als `re.sub`-Ersetzung bricht ab oder wird verändert (3.2). [A] Rückersetzung muss den Wert wörtlich einsetzen, ohne Ersetzungssyntax. |
| Unscharfer Rückweg | [A] LLM Guard FUZZY (Abstand 3) kann Nicht-Platzhalter zurückersetzen (3.2). Eine eindeutige, exakt geprüfte Platzhalterform macht Unschärfe auf dem Rückweg unnötig. |

## 4. Antwortstand je Frage

| Frage | Stand |
|---|---|
| 1 Modi, Ersetzungssyntax, Extended-Escapes | gesichert [B] (1.1). Widerspruch zu `$0` [W] ohne Folgen. |
| 1 Bestätigung, Zählung, Meldung | gesichert [B] (1.2). Zählung trotz Schreibfehler [A]. |
| 1 Kodierung, Zeilenenden, Schreibschutz, offene Dokumente | gesichert [B] für Kodierung, Schreibschutz und offene Puffer. Zeilenenden [A]. Atomares Schreiben [U]. |
| 1 Sicherung, Rückgängig | keine Sicherung bei Replace in Files [A] aus Quelltext. Rückgängig in offenen Dateien [U]. |
| 1 Treffer über Zeilengrenzen | gesichert [B] (1.5) |
| 1 nur ausgewählte Treffer | gibt es in Notepad++ nicht [B]. VS Code: einzeln ersetzen und „Dismiss“ [B]. |
| 1 VS Code, ripgrep | gesichert [B]. VS Code Undo über Dateien [U]. |
| 2 Verfahren und Normalisierung | gesichert [B]/[R] (2.1, 2.2) |
| 2 OCR-Paare | Methode belegt [B], konkrete Paartabelle [U] (2.3) |
| 2 Kosten und Schwellen mit Beispielen | nachgerechnet [R] (2.4). Gewichte [U] nicht kalibriert. |
| 2 Toleranz in Werkzeugen | gesichert [B] für Elasticsearch, Lucene, Solr und TRE agrep. „agrep -k“ [W]. OCR-Archive [S]/[U]. |
| 2 Rust-Crates | gesichert [B] aus crates.io und Quelltext. Laufzeitverhalten in se [U]. |
| 3 Muster Presidio, LLM Guard, Gateways | gesichert [B] für Presidio, LLM Guard und LiteLLM. Weitere Gateways nicht untersucht [U]. |
| 3 Platzhaltergestaltung | Formen belegt [B], Kollision nachgerechnet [R], LLM-Verständlichkeit [U] |
| 3 Speicherung und Schutz | Werkzeuge [B], DSGVO [S], se-eigene Ablage [U] |
| 3 Fallstricke | je Zeile in 3.6 gekennzeichnet |

## 5. Offene Punkte

- [U] Kalibrierte OCR-Verwechslungskosten (Primärquelle mit Häufigkeiten fehlt).
- [U] Undo-Verhalten bei Notepad++ (offene Dateien) und bei VS Code (Replace All über mehrere Dateien).
- [U] Atomares Schreiben in Notepad++ `Utf8_16_Write::openFile` (Datei nicht gelesen).
- [U] Optionen des ursprünglichen Wu-Manber-agrep (nur TRE agrep gelesen).
- [U] Messung der Crates an se-typischen Daten (verboten: kein Build auf diesem Rechner, nur entfernte CI).
- [U] Wie se die Maskenliste vor dem Agenten verbirgt (se-eigene Geheimnisablage nicht gelesen).
- Eintrag in `docs/refs/INDEX.md` steht aus. Dieser Auftrag erlaubte nur das Schreiben dieser Datei.

## 6. Quellen

| Nr. | Quelle | Version / Stand | abgerufen |
|---|---|---|---|
| Q01 | GitHub API `repos/notepad-plus-plus/notepad-plus-plus/releases/latest` | Tag v8.9.9.1, veröffentlicht 2026-11-12 | 2026-11-28 |
| Q02 | `PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp`, raw.githubusercontent.com/notepad-plus-plus/notepad-plus-plus/v8.9.9.1/… | v8.9.9.1 | 2026-11-28 |
| Q03 | `PowerEditor/src/Notepad_plus.cpp` (`replaceInFiles`, `replaceInFilelist`) | v8.9.9.1 | 2026-11-28 |
| Q04 | `PowerEditor/src/ScintillaComponent/Buffer.cpp` (`saveBuffer`, `checkFileState`, `loadFile`), `Buffer.h` | v8.9.9.1 | 2026-11-28 |
| Q05 | `boostregex/BoostRegExSearch.cxx` (`SubstituteByPosition`, `boost::format_all`) | v8.9.9.1 | 2026-11-28 |
| Q06 | `PowerEditor/installer/nativeLang/english.xml` | v8.9.9.1 | 2026-11-28 |
| Q07 | github.com/notepad-plus-plus/npp-usermanual `content/docs/searching.md`, `preferences.md` (= npp-user-manual.org/docs/searching/, /docs/preferences/) | Commit f70c52eb, 2026-11-28 | 2026-11-28 |
| Q08 | Boost.Regex „Boost-Extended Format String Syntax“, boost.org/doc/libs/1_90_0/libs/regex/doc/html/boost_regex/format/boost_format_syntax.html | Boost 1.90.0 | 2026-11-28 |
| Q09 | `PowerEditor/src/NppIO.cpp` (`fileSave`, Backup on save) | v8.9.9.1 | 2026-11-28 |
| Q10 | github.com/microsoft/vscode `src/vs/workbench/contrib/search/browser/searchActionsRemoveReplace.ts`, `replaceService.ts`, `searchView.ts`; `src/vs/editor/contrib/find/browser/replacePattern.ts` | Tag 1.148.0 | 2026-11-28 |
| Q11 | code.visualstudio.com/docs/editing/codebasics | Seite zeigt 11/25/2026 | 2026-11-28 |
| Q12 | github.com/BurntSushi/ripgrep `crates/core/flags/defs.rs` (`Replace`), `crates/matcher/src/interpolate.rs` | Tag 16.0.0 | 2026-11-28 |
| Q13 | V. I. Levenshtein, „Binary codes capable of correcting deletions, insertions, and reversals“, Soviet Physics Doklady 10(8):707–710, 1966 | Fundstelle aus Literatur, nicht gelesen | – |
| Q14 | F. J. Damerau, „A technique for computer detection and correction of spelling errors“, CACM 7(3):171–176, 1964, doi:10.1145/363958.363994 | Crossref geprüft | 2026-11-28 |
| Q15 | P. H. Sellers, „The theory and computation of evolutionary distances: Pattern recognition“, J. Algorithms 1(4):359–373, 1980, doi:10.1016/0196-6774(80)90016-4 | Crossref geprüft | 2026-11-28 |
| Q16 | E. Ukkonen, „Algorithms for approximate string matching“, Information and Control 64(1–3):100–118, 1985, doi:10.1016/S0019-9958(85)80046-2; ders., „Finding approximate patterns in strings“, J. Algorithms 6(1):132–137, 1985, doi:10.1016/0196-6774(85)90023-9 | Crossref geprüft | 2026-11-28 |
| Q17 | G. Navarro, „A guided tour to approximate string matching“, ACM Computing Surveys 33(1):31–88, 2001, doi:10.1145/375360.375365; Preprint users.dcc.uchile.cl/~gnavarro/ps/acmcs01.1.pdf (Abschnitt cut-off, Abschnitt 8.2) | Preprint gelesen, Crossref geprüft | 2026-11-28 |
| Q18 | G. Myers, „A fast bit-vector algorithm for approximate string matching based on dynamic programming“, JACM 46(3):395–415, 1999, doi:10.1145/316542.316550 | Crossref geprüft | 2026-11-28 |
| Q19 | S. Wu, U. Manber, „Fast text searching: allowing errors“, CACM 35(10):83–91, 1992, doi:10.1145/135239.135244 | Crossref geprüft | 2026-11-28 |
| Q20 | Lucene 10.3.0 Javadoc `FuzzyQuery`, lucene.apache.org/core/10_3_0/core/org/apache/lucene/search/FuzzyQuery.html und constant-values.html | 10.3.0 | 2026-11-28 |
| Q21 | github.com/laurikari/tre `doc/tre-syntax.html`, `doc/agrep.1.in`, `configure.ac` (TRE 0.9.0) | Commit d2c19b74, 2026-11-17 | 2026-11-28 |
| Q22 | E. Brill, R. C. Moore, „An improved error model for noisy channel spelling correction“, ACL 2000, S. 286–293, doi:10.3115/1075218.1075255, aclanthology.org/P00-1037.pdf | gelesen (Abschnitt 2) | 2026-11-28 |
| Q23 | O. Kolak, W. Byrne, P. Resnik, „A Generative Probabilistic OCR Model for NLP Applications“, HLT-NAACL 2003, S. 55–62, aclanthology.org/N03-1018.pdf | gelesen (Abschnitte 2.3 und 2.4) | 2026-11-28 |
| Q24 | crate bio 4.2.2 (crates.io-Archiv): `src/pattern_matching/myers/mod.rs`, `builder.rs`, `ukkonen.rs`, `Cargo.toml` | 4.2.2 | 2026-11-28 |
| Q25 | O. Kolak, P. Resnik, „OCR Post-Processing for Low Density Languages“, HLT/EMNLP 2005, S. 867–874, doi:10.3115/1220575.1220684, aclanthology.org/H05-1109.pdf | Titel und Metadaten geprüft, Tabellen nicht ausgewertet | 2026-11-28 |
| Q26 | M. Richter, „OCR Post-Processing“ (Folien), coli.uni-saarland.de/courses/tm-hist10/slides/OCR_Post-Processing2.pdf | ohne Datum | 2026-11-28 |
| Q27 | crate fuzzy-aho-corasick 0.5.2: `README.md`, `Cargo.toml`, `src/builder.rs` | 0.5.2 | 2026-11-28 |
| Q28 | elastic.co/docs/reference/elasticsearch/rest-apis/common-options („Fuzziness“) | ohne Versionsangabe | 2026-11-28 |
| Q29 | elastic.co/docs/reference/query-languages/query-dsl/query-dsl-match-query | ohne Versionsangabe | 2026-11-28 |
| Q30 | crates.io API `api/v1/crates/<name>` (Versionen, Lizenz, Datum, Downloads) für alle Crates in 2.6 und fancy-regex | Stand am Abruftag | 2026-11-28 |
| Q31 | crate fancy-regex 0.16.2 `src/expand.rs` | 0.16.2 | 2026-11-28 |
| Q32 | solr.apache.org/guide/solr/latest/query-guide/standard-query-parser.html („Fuzzy Searches“) | Solr 10.1 | 2026-11-28 |
| Q33 | Websuche „Trove newspapers search help fuzzy search tilde“ (Suchauszüge), Hilfeseite trove.nla.gov.au/help/searching/constructing-complex-search-query: Access Denied | – | 2026-11-28 |
| Q34 | crate strsim 0.11.1 `src/lib.rs` | 0.11.1 | 2026-11-28 |
| Q35 | crate rapidfuzz 0.5.0 `src/distance/levenshtein.rs`, `src/fuzz.rs` | 0.5.0 | 2026-11-28 |
| Q36 | crate triple_accel 0.4.0 `src/lib.rs`, `src/levenshtein.rs` | 0.4.0 | 2026-11-28 |
| Q37 | crate levenshtein_automata 0.2.1 `src/lib.rs` | 0.2.1 | 2026-11-28 |
| Q38 | crate sassy 0.3.1 `README.md` | 0.3.1 | 2026-11-28 |
| Q39 | github.com/microsoft/presidio `docs/anonymizer/index.md`, `docs/samples/python/pseudonymization.ipynb`, `presidio-anonymizer/presidio_anonymizer/{deanonymize_engine.py, anonymizer_engine.py, core/engine_base.py, operators/aes_cipher.py, operators/encrypt.py}` | Tag 2.2.366, Commit 211afc33 | 2026-11-28 |
| Q40 | github.com/protectai/llm-guard `llm_guard/vault.py`, `llm_guard/input_scanners/anonymize.py`, `llm_guard/output_scanners/deanonymize.py` | Tag v0.3.16, Commit 32b14a4a | 2026-11-28 |
| Q41 | github.com/BerriAI/litellm `litellm/proxy/guardrails/guardrail_hooks/presidio.py` | `main` bei Commit 3261387b | 2026-11-28 |
| – | DSGVO Art. 4 Nr. 5, Wortlaut über dsgvo-gesetz.de/art-4-dsgvo/ (EUR-Lex CELEX 32016R0679 lieferte eine leere Antwort) | Sekundärquelle | 2026-11-28 |
