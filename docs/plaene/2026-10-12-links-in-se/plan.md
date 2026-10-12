# Links, Sonderdateien und „unsupported“-Abbrüche in `se` (2026-10-12)

Stand: 2026-10-12. Quellen: Nutzernachrichten 2 und 3 vom 2026-10-12 (dieselbe Sitzung wie
[KI-Agenten-Plugin](../2026-10-12-ki-agenten-plugin/plan.md)).

## A — Auftrag

| Nr. | Ergebnis | Quelle |
|---|---|---|
| L1 | `se` scheitert an Link-Einträgen beim Durchsuchen von Ordnern („link-like … unsupported“); Unterstützung hinzufügen, beheben | Nachricht 2 |
| L2 | Weitere Stellen suchen, an denen `se` wegen „unsupported“ bricht: wo möglich unterstützen, sonst sauber behandeln | Nachricht 2 |
| L3 | Lokale Installation aktualisieren, selbst testen, Fehler suchen und beheben | Nachricht 3 |

Grenzen: keine lokalen Builds (AGENTS.md); Prüfung über eine Remote-Suite, danach ein Release; die
lokale Installation wird danach erneut aktualisiert und mit denselben Handgriffen nachgeprüft.

## B — Befunde (lokales `se` 0.5.176, aktualisiert von 0.5.173 am 2026-10-12)

Testbaum lokal und auf einem SFTP-Server (Container `atmoz/sftp`): Datei-Link, Ordner-Link,
toter Link, Schleifen-Link `sub/loop -> ..`, absoluter Link `-> /etc`, FIFO.

| Nr. | Befehl | Beobachtet | Ursache |
|---|---|---|---|
| F1 | `se search <dir> …` (jede Abfrage, lokal und SFTP; MCP `search`) | Abbruch beim ersten Link: „link-like search entry is unsupported“ | `cli/ops.rs::fallback_search` |
| F2 | `se cp -r <dir> …`, `se mv -r …` (MCP `copy`/`move`) | ganze Übertragung bricht beim ersten Link ab, nichts wird kopiert | `cli/tree_plan.rs` `reject_link` für jedes Kind und in `validate_source_tree` |
| F3 | `se cp <link> <ziel>` | verweigert, obwohl `se cat <link>` den Inhalt liest | `reject_link` für die Wurzel |
| F4 | `se mv <link> <neuer name>` im selben Ordner | verweigert | `transfer.rs::try_rename_fast` schließt Links aus |
| F5 | `se mv` innerhalb FTP, Google Drive, ZIP | „backend has no atomic no-replace rename“ statt Kopieren + Löschen | `try_rename_fast` reicht `ErrorKind::Unsupported` als Fehler durch |
| F6 | `se stat <link>`, `se stat <fifo>` | `type file`, bei Links die Größe des Linkpfads | `ops::stat` kennt nur dir/file |
| F7 | `se ls` | FIFO erscheint wie eine Datei | `ops::list` |
| F8 | `se search --dirs` | Hilfe sagt „directories instead of files“, Agent-/Peer-/GUI-Suche liefern Ordner **zusätzlich** (`agent_proto/core/types.rs`) | falscher Hilfetext (auch MCP-Katalog) |
| F9 | MCP `read <link>` | `size` = Linkpfad-Länge; bei größerer Zieldatei fiele `next_offset` auf `null` → stiller Abschnitt | `mcp/file_tools.rs::read` nutzt die Link-Metadaten |
| F10 | MCP `edit <link>` | liest erst, scheitert dann beim Speichern | spätes Erkennen |
| F11 | `se cat <toter link>`, `se cp a.txt <link auf ordner>` | knappe bzw. „link-like and unsafe“-Meldung ohne Ausweg | Meldungstexte |

Funktioniert bereits: `se rm <link>` und `se rm -r` folgen Links nie; `se cat <fifo>` verweigert mit
klarer Meldung; Schreiben durch einen Link als Ziel wird verweigert (bewusst,
`tree_preflight_tests::link_like_destination_ancestor_is_rejected_before_apply`).
Server-Suchen (SSH-Agent `agent_proto/os/shared/search.rs`, Hintergrunddienst `backend_walk.rs`,
Share-Peer `share/core/walk.rs`) überspringen Links still. Die GUI-Übertragung meldet Links je
Eintrag („Links, Reparse-Punkte und Spezialdateien werden nicht übertragen“) und überträgt den Rest.

## C — Entscheidungen

- **Links werden nie verfolgt, wenn sie in einem Baum liegen** (Projektregel „protected
  omissions“, AGENTS.md; Schleifen wie `sub/loop -> ..`; Ziele außerhalb der Wurzel wie `/etc`).
  Kein Backend kann Links anlegen oder ihr Ziel lesen (`vfs::Backend` hat dafür keine Methode);
  Links an anderen Orten neu anzulegen ist deshalb nicht möglich.
- **Suche (F1):** Links werden über ihren Namen gefunden und als Typ `symlink` gemeldet, aber nie
  betreten; FIFO/Socket/Gerät als `special`. Server-Suchen bleiben unverändert (ihr Protokoll kennt
  keinen Link-Typ). `--dirs` heißt „Ordner zusätzlich“ (F8, Hilfe und MCP korrigiert).
- **Kopieren (F2):** Links und Sonderdateien im Baum werden ausgelassen, der Rest übertragen; jede
  Auslassung wird gemeldet (CLI stderr, MCP `omitted`). Exitcode 0, weil alles Übertragbare
  übertragen wurde (wie die GUI).
- **Verschieben (F2/F4/F5):** Innerhalb derselben Verbindung wird umbenannt, wenn das Ziel nicht
  existiert und der Anbieter keine doppelten Namen kennt (dann wandern Links unverändert mit, nichts
  wird gelesen); Ordner wie Dateien. `Unsupported` beim Umbenennen fällt auf Kopieren + Löschen
  zurück (F5). Beim Kopieren + Löschen bleiben ausgelassene Links in der Quelle; deren Ordner werden
  nicht gelöscht und gemeldet. Ein Link als Wurzel wird nur umbenannt; anderswohin verschieben ist
  nicht möglich und wird klar abgelehnt.
- **Link als Wurzel einer Kopie (F3):** Ein Link auf eine Datei wird wie bei `se cat` gelesen und
  dessen Inhalt kopiert; ein Link auf einen Ordner wird mit Hinweis abgelehnt (Bäume folgen keinen
  Links).
- **Anzeige (F6/F7):** `se stat` meldet `symlink`/`special`, `se ls` markiert Sonderdateien mit `s`.
- **MCP (F9/F10):** `read` über Links ohne bekannte Größe (`size` null, Weiterlesen über
  Mehr-Lesen erkannt); `edit` lehnt Links sofort ab.
- **Meldungen (F11):** toter Link und Link als Ziel nennen Ursache und Ausweg.

## D — Arbeitsplan

| Nr. | Ergebnis | Dateien | Abnahme |
|---|---|---|---|
| N1 | Eintragsart (`file`/`dir`/`symlink`/`special`) gemeinsam für ls/stat/search/MCP | neu `cli/entry_kind.rs`; `ops.rs`, `mcp/file_tools.rs` | Unit `links_task_` |
| N2 | Suche in eigene Datei, Links/Sonderdateien als Treffer, `--dirs`-Text | neu `cli/search.rs`; `ops.rs`, `mod.rs`, `ops_search_tests.rs`, `mcp/search_tool.rs`, `mcp/catalog.rs` | Unit + E2E lokal/SFTP |
| N3 | Baumkopie mit Auslassungen, Meldung, Verschieben behält Ordner mit Auslassungen | `tree_plan.rs`, `tree_ops.rs`, `tree_remove.rs`, `ops.rs`, `mod.rs`, `mcp/transfer_tools.rs` | Unit + E2E |
| N4 | Umbenennen für Ordner und Links, `Unsupported` → Rückfall | `transfer.rs` | Unit (Fake-Backend) + E2E |
| N5 | Link-Wurzel kopieren (Dateiinhalt) / ablehnen (Ordner) | neu `cli/link_copy.rs`; `ops.rs`, `mcp/file_write.rs` (gemeinsames Veröffentlichen) | E2E |
| N6 | MCP `read`/`edit` über Links, Meldungen F11 | `mcp/file_tools.rs`, `mcp/file_write.rs`, `ops.rs`, `tree_guard.rs`, `transfer.rs` | E2E |
| N7 | Suite `links-task.yml` → `native/test-links-task.py` (Linux mit SFTP-Container, Windows mit Symlink und Junction) | neu | ein Lauf |
| N8 | Doku (README, AI_AGENT_PLUGIN, TODO), Graph, Release, lokale Nachprüfung | — | Release + Handprüfung |
