# Suche in Dateien (Volltextsuche)

Stand 2026-11-27 (Plan `docs/plaene/2026-11-27-dateien-suche-ansicht/plan.md`, Teile B, C und E). Die Suche findet Text in Dateien unter einem Ordner wie Notepad++ „Find in Files“: drei Suchmodi (Normal, Erweitert, Regulärer Ausdruck), Groß-/Kleinschreibung, ganze Wörter, „. findet Zeilenumbruch“, das Filterfeld von Notepad++ und zusätzlich zwei reguläre Ausdrücke auf den Ordnerpfad (Nutzer: „ordnerpfad mit regex“). Sie läuft dort, wo die Dateien liegen: lokal auf den eigenen Kernen, auf SSH-Servern im `se-agent`, bei Share Direct und Gruppen auf dem Host, sonst durch Lesen über die Verbindung. Binär- und Office-Dateien sind ausgenommen (Nutzer: „bleibt erstmal aus“). Pfade unten sind relativ zu `native/crates/`.

## Kern

| Fähigkeit | Verhalten | Beleg |
|---|---|---|
| Abfrage | `ContentQuery`: Muster, `TextMode` Normal/Erweitert/Regex, Groß/Klein, ganzes Wort (bei Regex ohne Wirkung), „. findet Zeilenumbruch“, Notepad++-Filter, Ordnerpfad-Regex einschließen/ausschließen (relativer Pfad mit `/`, der Ordner selbst ist leer), Unterordner, versteckte Ordner, Kontextzeilen, Höchstzahl, nur Dateien, Zeilenlänge (Standard 1024 Bytes) | `storage/src/agent_proto/core/content_query.rs:12` (`TextMode`), `:48` (`ContentQuery`) |
| Prüfung | ein ungültiges Muster, ein ungültiger Ordner-Regex oder ein ungültiger Filter wird vor der Suche mit deutschem Text abgelehnt | `storage/src/agent_proto/os/shared/content_match.rs:38` (`ContentMatcher::new`), `storage/src/agent_proto/core/content_filter.rs:86` (`PathFilter::new`) |
| Ort der Suche | `vfs::content_search` fragt zuerst das Backend (lokal, SSH-Agent, Share-Host), sonst liest es die Dateien über das Backend | `storage/src/vfs/os/shared/content_read.rs:28` |

## CLI `se grep`

| Fähigkeit | Einstieg | Beleg |
|---|---|---|
| Suche | `se grep <Ziel> <Muster>`; Ziel jeder Art (lokaler Pfad, `@label:/pfad`, gespeicherter Endpunkt, `share://…`, `gdrive:///…`) | Argumente `cli/src/cli/grep.rs:20` (`GrepArgs`), Lauf `:146` (`run`), Eintrag `cli/src/cli/mod.rs:215` |
| Notepad++-Optionen | `--regex`, `--extended`, `--case`, `--word`, `--dotall`, `--filter`, `--no-subfolders`, `--hidden` | `cli/src/cli/grep.rs:20-70` |
| Ordnerpfad-Regex | `--folders <REGEX>`, `--exclude-folders <REGEX>` | `cli/src/cli/grep.rs:54-65` |
| Ausgabe | `rel:zeile:text` je Treffer, Kontext mit `-C N` (`rel-zeile-text`, Gruppen durch `--` getrennt), `--files-only`, `--count`, `--max-results N`, `--json` (eine Zeile je Treffer mit `rel`, `line`, `text`, `ranges`), `--color`; Summen auf der Standardfehlerausgabe; Rückgabewert 0 mit Treffern, 1 ohne, 2 bei Fehler, 130 nach Strg+C | `cli/src/cli/grep.rs:1-9`, `:71-95`; JSON-Zeilen `cli/src/cli/mcp/grep_records.rs:172` |

## MCP `se mcp`

| Fähigkeit | Einstieg | Beleg |
|---|---|---|
| Inhaltssuche | Werkzeug `grep` mit `target` (Standard: das Arbeitsziel), `pattern`, `mode`, `match_case`, `whole_word`, `dot_matches_newline`, `filters`, `folders`, `exclude_folders`, `subfolders`, `hidden`, `context_lines`, `output` (content, files, count), `format` json, Seiten über `hits_offset`, Summen am Ende des Auftrags | Beschreibung `cli/src/cli/mcp/grep_tool.rs:21`, Felder `:23`, Katalog `cli/src/cli/mcp/catalog.rs:156` |
| Arbeitsziel | Werkzeug `root`: `set` (jede Zielart), `show`, `clear`; danach nehmen die Dateiwerkzeuge (`list`, `stat`, `read`, `write`, `edit`, `mkdir`, `remove`, `copy`, `move`, `search`, `grep`) relative Pfade darunter, `..` verlässt es nicht; die Verbindung bleibt für die Sitzung offen | `cli/src/cli/mcp/root_tool.rs:17`, Katalog `cli/src/cli/mcp/catalog.rs:86`, Auflösung `cli/src/cli/mcp/root_path.rs` |
| Lesen nach Zeilen | `read` mit `line` (1-basiert) und `lines`: nummerierte Ausgabe wie `cat -n`, Fortsetzung über `next_line` | Felder `cli/src/cli/mcp/catalog.rs:101-102`, Ausführung `cli/src/cli/mcp/line_read.rs:17` |

## Desktop-GUI

| Fähigkeit | Bedienweg | Beleg |
|---|---|---|
| Dialog „In Dateien suchen“ öffnen | Strg+Umschalt+F (C1); Kontextmenü eines Ordners und des leeren Bereichs: entfernte Tabs im eigenen Menü, lokal unter Windows als eigener Eintrag im Shell-Menü; Menü „»“ (Weitere Befehle) → „In Dateien suchen…“ für jede Ansicht, auch Linux lokal ohne Kontextmenü | Tastenkürzel `gui/src/app/core/frame_keyboard.rs:142`, `:404`; entfernt `gui/src/app/core/remote_context_plan.rs:181`, `:193`, Ausführung `gui/src/app/core/remote_context_menu.rs:272-273`; Windows `gui/src/app/os/windows/shell_menus.rs:60`, `:101`, `:143`, `:154`; Menü `gui/src/app/core/shell_commands.rs:119`; Öffnen `gui/src/app/core/text_tools.rs:345` (`open_find_in_files`) |
| Felder | Suchen nach (mit Verlauf, in den App-Daten `find_in_files_history.json`, je 20 Einträge), Filter (Notepad++-Syntax, Tooltip mit den Beispielen `*.cpp *.cxx *.h *.hxx *.hpp`, `*.* !*.exe !*.obj !*.log`, `*.* !\tests !\bin*`, `*.* !+\log*`), Ordner (vorbelegt mit dem Ordner des aktiven Tabs, „<<“ übernimmt ihn samt Ziel, die Zielart steht als „auf: …“ darunter), Ordnerpfad (Regex), Ordnerpfad ausschließen (Regex) mit Tooltip zum relativen Pfad mit `/`, In allen Unterordnern (an), In versteckten Ordnern (aus), Nur ganze Wörter (bei Regex gesperrt), Groß-/Kleinschreibung, Kontextzeilen, Suchmodus Normal / Erweitert / Regulärer Ausdruck mit „. findet Zeilenumbruch“, „Alle suchen“ | Formular `gui/src/app/core/content_search_dialog.rs:27`, Tooltips `:12`, `:19`, Sperre `:77`, Abfrage `:86`, Verlauf `:119`, Fenster `:213`, Optionen `:331` |
| Prüfung | leerer Text, fehlender Ordner, ungültiges Muster, ungültiger Regex oder Filter: Meldung im Dialog, keine Suche | `gui/src/app/core/content_search_dialog.rs:103` (`validate`) |
| Lauf | Hintergrund-Thread mit `vfs::content_search` auf dem Backend des Ziels (lokal ein Backend über den Ordner); Fortschritt (Dateien, Bytes, Treffer) im Dialog und im Kopf des Ergebnisses; „Abbrechen“ im Dialog und im Ergebnisbereich | `gui/src/app/core/content_search_run.rs:30` (`start`), `:74` (`drain`); Start `gui/src/app/core/text_tools.rs:136` |
| Ergebnisbereich | unten unter der Dateiliste angedockt; Kopf `Suche „foo“ (N Treffer in M Dateien von K durchsuchten) [Normal: Groß/Klein/Wort]` mit Tausenderpunkten; je Datei eine aufklappbare Zeile mit Pfad und Trefferzahl, je Treffer `Zeile 12: <Text>` mit hervorgehobenem Treffer, Kontextzeilen gedämpft ohne Doppelpunkt, gekürzte Zeilen mit „…“; neue Suchen oben, frühere zugeklappt; ausgelassene Dateien (Binärdatei, nicht lesbar, Ordner nicht lesbar) als zuklappbarer Abschnitt „Ausgelassen (N)“ mit Grund; virtualisierte Zeilen | Modell `gui/src/app/core/content_search_results.rs:226` (`header`), `:288` (`begin`), `:378` (`rows`), Gründe `:132`; Fläche `gui/src/app/core/content_search_results_ui.rs:236` (`show`), `:370` (`paint_row`) |
| Öffnen eines Treffers | Doppelklick oder Enter öffnet die Textansicht an der Trefferzeile mit markiertem Treffer; F4 / Umschalt+F4 nächster / voriger Treffer über Dateien hinweg mit Umlauf, auch wenn der Fokus woanders liegt (außer in Textfeldern) | `gui/src/app/core/text_tools.rs:190` (`open_row`), `:226` (`step_hit`); Navigation `gui/src/app/core/content_search_nav.rs:45` (`next_hit`) |
| Kontextmenü | Alle aufklappen, Alle zuklappen, Kopieren, Ausgewählte Zeilen kopieren, Pfade kopieren, Pfade öffnen (Textansicht), Alles auswählen, Leeren, In diesen Ergebnissen suchen… (Normal/Regex, Groß/Klein; neues Ergebnis oben), Lange Zeilen umbrechen, Bei jeder Suche leeren; Tasten im Bereich: ↑/↓, Pos1/Ende, Enter, ←/→ (zu-/aufklappen), Strg+A, Strg+C, Esc | `gui/src/app/core/content_search_panel_menu.rs:213` (`menu`), `:12` (`apply_choice`), `:65` (Eingabe), `:124` (`keys`); Kopiertexte und Filter `gui/src/app/core/content_search_nav.rs:90`, `:124`, `:133`, `:143` |
| Tastaturzuständigkeit | ein Klick auf Dialog, Ergebnisbereich oder Ansicht gibt dieser Fläche die Tastatur; solange sperrt die Dateiliste ihre eigenen Kürzel | `gui/src/app/core/text_tools.rs:86` (`track_pointer`), `gui/src/app/core/frame_keyboard.rs:469` |
| Ansicht-Menü | „Ansicht“ → „Suchergebnisse“ blendet den Bereich ein und aus | `gui/src/app/core/shell_commands.rs:242` |

## Android-App

Pfade relativ zu `android/app/src/main/java/app/smartexplorer/android/`.

- Einstieg: Menü „⋮“ eines lokalen oder entfernten Ordners → „In Dateien suchen“ (`ui/files/FilesTopBar.kt:103`, `ui/files/FilesScreen.kt:437`); die Seite sucht in diesem Ordner.
- Felder wie Notepad++ „Find in Files“ (`ui/files/ContentSearchPage.kt:218-300`): Suchen nach, Filter (`*.kt !*.log !\build !+\tmp*`), Ordner, „Ordnerpfad (Regex)“ und „Ordnerpfad ausschließen (Regex)“ (relativer Pfad mit `/`), In allen Unterordnern, In versteckten Ordnern, Nur ganze Wörter (im Regex-Modus gesperrt), Groß-/Kleinschreibung beachten, Nur Dateinamen anzeigen, Suchmodus Normal / Erweitert / Regulärer Ausdruck mit „. findet Zeilenumbruch“, Kontextzeilen, Höchstzahl Treffer (leer = unbegrenzt). [Alle suchen] (`:296`), [Abbrechen] während der Suche; ein ungültiges Muster meldet der Kern sofort.
- Ergebnis (`ui/files/ContentResults.kt:105-127`): Kopfzeile `Suche „x“ (N Treffer in M Dateien von K durchsuchten) [Modus: Optionen]`, je Datei eine aufklappbare Zeile `pfad (n Treffer)`, darunter `Zeile 12: …` mit markiertem Treffer (`ContentResults.kt:151`), am Ende „Ausgelassen (n)“ mit Grund (Binärdatei, nicht lesbar, Ordner nicht lesbar). Menü: Alle aufklappen/zuklappen (`ContentSearchPage.kt:97`).
- Tippen auf eine Trefferzeile öffnet die Textansicht an dieser Zeile mit markiertem Treffer; das Symbol an der Dateizeile öffnet die Datei am ersten Treffer.
- „Zurück“ lässt eine laufende Suche weiterlaufen; die Ergebnisse bleiben bis zur nächsten Suche erhalten (`ui/files/ContentSearchState.kt:158`). Die Suche steht nicht in den Übertragungen (`ui/transfers/TransfersBar.kt:25`).
- Brücke: `search.content.start|poll|cancel` (api.md §4.14; `native/crates/mobile/src/mobile/os/shared/content_search.rs:165-171`). Gesucht wird dort, wo die Dateien liegen (lokal auf allen Kernen, SSH-Agent, Share-Host, sonst Lesen über das Backend).

## Verhalten, das alle Oberflächen gleich halten

- Die Suche läuft über `vfs::content_search` dort, wo die Dateien liegen; keine Oberfläche lädt Dateien dafür selbst herunter.
- Fehlertexte zu Muster, Filter und Ordner-Regex kommen unverändert aus dem Kern (`ContentMatcher::new`, `PathFilter::new`).
- Ganze Wörter wirken bei regulären Ausdrücken nicht (wie Notepad++), „. findet Zeilenumbruch“ nur bei ihnen.
- Binärdateien werden ausgelassen und mit Grund gemeldet; Verknüpfungen werden nie betreten.

## Lücken und Ausnahmen

- Ausnahme: Ersetzen in Dateien („Replace in Files“) ist nicht Teil des Auftrags (Plan, Abschnitt G).
- Ausnahme: Volltextsuche in Binär- und Office-Dateien (Plan, Abschnitt G).
- Ausnahme Desktop-GUI: Linux hat lokal kein Kontextmenü (ein Rechtsklick öffnet den Dateimanager); dort führt das Menü „»“ zum Dialog.
- Desktop-GUI: Strg+Umschalt+F öffnete vorher die globale Ordnersuche; sie liegt jetzt auf Strg+Umschalt+O und bleibt über „/“ in der Suchleiste erreichbar (`gui/src/app/core/frame_keyboard.rs:144`).
- Desktop-GUI: „In diesen Ergebnissen suchen“ prüft die Trefferzeilen mit Rusts `regex` zeilenweise (ohne die Boost-Erweiterungen des Suchkerns).
- Nicht geprüft: das Verhalten auf echten entfernten Zielen wurde nicht mit laufender Verbindung erprobt.
