# Smart Explorer für KI-Agenten (`se mcp`, `se plugin`)

Stand: 2026-10-14. Plan und Entscheidungen:
[`docs/plaene/2026-10-12-ki-agenten-plugin/plan.md`](plaene/2026-10-12-ki-agenten-plugin/plan.md),
Vorrang vor den Datei-Werkzeugen des Hosts:
[`docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md`](plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md).
Hostverträge: [`refs/claude-code-plugins-mcp-2026-10-12.md`](refs/claude-code-plugins-mcp-2026-10-12.md),
[`refs/codex-mcp-spec-rmcp-2026-10-12.md`](refs/codex-mcp-spec-rmcp-2026-10-12.md).

Die Terminalversion `se` bringt einen MCP-Server mit (Model Context Protocol). KI-Agenten wie
Claude Code oder Codex starten ihn selbst und bekommen damit dieselben gespeicherten
Verbindungen, Zugangsdaten, Share-Peers und denselben Hintergrunddienst wie die App: Dateien auf
allen Remotes, Übertragungen, Namenssuche, Befehle auf entfernten Rechnern und jeden weiteren
`se`-Befehl. Android ist ausgenommen (siehe „Grenzen“).

## Installieren

| Befehl | Wirkung |
|---|---|
| `se plugin install` | in jeden gefundenen Host (Claude Code und/oder Codex) |
| `se plugin install claude` | nur Claude Code |
| `se plugin install codex` | nur Codex |
| `se plugin status [--json]` | gefundene Host-CLIs, installierter Stand, ob die erzeugten Dateien zu diesem `se` passen |
| `se plugin uninstall [claude\|codex]` | Plugin/Servereintrag entfernen und nur die von `se` erzeugten Dateien löschen |
| `se plugin config <claude\|codex\|generic\|vscode\|opencode>` | Konfiguration für weitere MCP-Clients mit absolutem Pfad ausgeben |

Danach eine neue Agent-Sitzung starten.

- **Claude Code:** `se` schreibt einen lokalen Plugin-Marketplace nach
  `<App-Daten>/smart_explorer/ai-plugins/claude/` (Plugin `smart-explorer` mit `.mcp.json` →
  absoluter Pfad dieses `se` + `mcp`, Skill `smart-explorer`) und ruft
  `claude plugin marketplace add <Ordner> --scope user` sowie
  `claude plugin install smart-explorer@smart-explorer --scope user -y`. Claude Code lädt Plugins
  lokaler Marketplaces bei jedem Sitzungsstart direkt aus dem Ordner. Die Tools heißen
  `mcp__plugin_smart-explorer_se__<tool>`; dauerhaft erlauben lassen sie sich mit der
  Berechtigungsregel `mcp__plugin_smart-explorer_se__*` in den eigenen Claude-Code-Einstellungen
  (Plugins dürfen keine Regeln mitbringen).
- **Codex:** `codex mcp add smart-explorer -- <se> mcp` (absoluter Pfad) und der Skill unter
  `~/.agents/skills/smart-explorer/SKILL.md`. Unter Linux gibt die Installation die zur
  Installationszeit gesetzten `XDG_DATA_HOME`, `XDG_RUNTIME_DIR` und `DBUS_SESSION_BUS_ADDRESS`
  per `--env` mit, weil Codex sie aus der Umgebung von MCP-Servern herausfiltert (Ort der
  App-Daten, Start des Hintergrunddienstes). Codex fragt nach Freigabe für jedes Tool, das nicht
  als nur lesend markiert ist.
- **Andere Clients** (Cursor, VS Code, Gemini CLI/Antigravity, OpenCode, Claude Desktop):
  `se plugin config generic|vscode|opencode` ausgeben und in deren MCP-Konfiguration eintragen.

Ein Update ersetzt `se` an derselben Stelle; `se mcp` frischt beim nächsten Start die erzeugten
Plugin- und Skill-Dateien auf (Versions-/Pfadmarker `.se-generated.json`).

## Vorrang vor den Datei-Werkzeugen des Hosts

Ab 0.5.178 empfiehlt sich das Plugin dem Agenten selbst als erste Wahl für jede Dateiaufgabe,
lokal wie entfernt:

- **Server-Anweisungen** (`instructions` der MCP-Initialisierung): Der erste Absatz verlangt, die
  Smart-Explorer-Tools vor den eingebauten Datei-Werkzeugen (Read, Write, Edit, Glob, LS) und vor
  Shell-Dateibefehlen (`ls`, `cat`, `find`, `cp`, `mv`, `rm`, `scp`, `rsync`) zu nutzen. Auf diese
  weicht der Agent nur aus, wenn ein Smart-Explorer-Tool scheitert oder die Funktion fehlt. Der
  Absatz ist mit 503 Zeichen in den ersten 512 Zeichen vollständig, die Codex auswertet.
- **Vorab geladen in Claude Code:** `connections`, `list`, `stat`, `read`, `write`, `edit`,
  `mkdir`, `remove`, `copy`, `move`, `search` und `job_status` tragen
  `_meta["anthropic/alwaysLoad"]: true`. Claude Code zeigt sie damit ohne Tool-Suche, ohne den
  Sitzungsstart aufzuhalten. `exec`, `job_input`, `job_cancel`, `status` und `se` bleiben
  verzögert; die Anweisungen nennen die Suche nach `smart-explorer`. Wer alle Tools verzögert
  haben will, setzt im eigenen Servereintrag `"alwaysLoad": false` (ab Claude Code v2.1.287).
- **Tool-Beschreibungen** nennen jeweils das ersetzte Werkzeug, zum Beispiel „Use it instead of
  Read or `cat`“.
- **Skill und Manifest:** Die Skill-Beschreibung (Auslöser in Claude Code und Codex) beginnt mit
  dem Vorrang. Der Skill führt eine Tabelle Aufgabe → Smart-Explorer-Tool → ersetztes Werkzeug.
  Die Plugin-Beschreibung in `plugin.json`/`marketplace.json` nennt den Vorrang ebenfalls.
- Inhaltssuche in Dateien (Grep) und lokale Builds bleiben beim Host, weil `search` nur Namen
  sucht.

Bestehende Installationen übernehmen das ohne erneutes `se plugin install`. Nach dem Update
schreibt `se mcp` beim nächsten Start Skill und Plugin-Dateien neu, und die Anweisungen und
Tool-Metadaten kommen direkt vom Server.

## Tools

| Tool | Zweck | Markierung |
|---|---|---|
| `status` | Version, Installation, Zugangsdatenspeicher, Hintergrunddienst, Share-Zustand, Plugin-Dateien | nur lesend |
| `connections` | gespeicherte Remotes und Share-Peers mit `target`-Präfix und `exec`-Art (`ssh`/`share`) | nur lesend |
| `list`, `stat`, `read` | Verzeichnis (seitenweise, `type` `file`/`dir`/`symlink`/`special`), Metadaten mit `etag`, Inhalt ab Byte-Offset (Text oder base64; über einen Link der Inhalt seines Ziels mit `size`/`etag` = null und `through_link: true`) | nur lesend |
| `write`, `edit` | Datei anlegen/ersetzen, exakte Textersetzungen; `if_match` = `etag` schützt vor verlorenen Änderungen; nie durch einen Link als Datei hindurch | zerstörend |
| `mkdir`, `remove` | Ordner anlegen; löschen wie `se rm --force` (Wurzeln nur mit `allow_root`) | ändernd / zerstörend |
| `copy`, `move` | wie `se cp`/`se mv` zwischen beliebigen Zielen, als Job; Ergebnis mit `omitted` (übersprungene Links/Sonderdateien) und `kept_directories` (beim Verschieben behaltene Quellordner) | zerstörend (Überschreiben) |
| `search` | Namenssuche (Text oder Glob), Treffer seitenweise, als Job; Links und Sonderdateien als eigene Treffer (`symlink`, `special`), nie betreten | nur lesend |
| `exec` | Befehl auf gespeichertem SFTP-Remote (SSH) oder Share-Peer, als Job | zerstörend |
| `job_status`, `job_input`, `job_cancel` | Jobs verfolgen (Ausgabe ab Offset), stdin senden, abbrechen | lesend / ändernd |
| `se` | jeder weitere `se`-Befehl als Kindprozess (Share-Verwaltung, Verbindungen anlegen, Laufwerke, Update, Diagnose) | zerstörend |

**Ziele** folgen der `se`-Grammatik: `@label:/pfad`, gespeicherte `sftp://`/`ftp://`/`ftps://`/
`webdav://`-Endpunkte, `share://direct/<id>/pfad`, `share://room/<raum>/<gerät>/pfad`,
`gdrive:///pfad`, lokale und UNC-Pfade. Lokale Pfade müssen absolut sein: das Arbeitsverzeichnis
des Servers bestimmt der Agent-Host. Innerhalb einer Sitzung wird je gespeicherter SFTP-, FTP-,
FTPS- und WebDAV-Verbindung eine Anmeldung wiederverwendet.

## Lange Vorgänge, Ausgaben, Antwortgrößen

- Ein Aufruf wartet höchstens `wait_seconds` (Standard 30 s = halbe dokumentierte
  Codex-Standardfrist von 60 s) und liefert sonst `status: running` mit `job_id`. `job_status`
  liefert Zustand, Fortschritt, Ergebnis und je Ausgabestrom die nächste Seite ab
  `stdout_offset`/`stderr_offset`/`hits_offset`; ohne `job_id` listet es alle Jobs der Sitzung.
- Während des Wartens gehen bei vorhandenem `progressToken` alle 10 s Fortschrittsmeldungen
  hinaus (Claude Code setzt damit seine 30-min-Leerlauffrist zurück).
- Ausgaben werden vollständig in Dateien der Sitzung geschrieben
  (`<App-Daten>/smart_explorer/mcp/sessions/<pid>-<zeit>/`); es gibt wie bei `se exec` keine
  Standardgrenze für Laufzeit oder Ausgabe. Beim Sitzungsende werden laufende Jobs abgebrochen
  und der Ordner gelöscht; Ordner abgestürzter Sitzungen (Sperrdatei frei) entfernt der nächste
  Start.
- Jede Antwort enthält standardmäßig höchstens 20 000 Byte Nutzinhalt (`max_bytes` hebt das an):
  Codex kürzt jede Tool-Ausgabe bei 10 000 Token, Claude Code warnt ab 10 000 Token. Das
  vollständige Ergebnis steht in `structuredContent` (Codex zeigt dem Modell nur diesen Teil) und
  als JSON-Text.
- Ein vom Client abgebrochener Aufruf bricht den von ihm gestarteten Job mit ab (der Client kennt
  dessen `job_id` noch nicht); ein abgebrochenes `job_status` lässt den Job laufen.

## Remote-Ausführung

- **SFTP-Remotes:** eigener SSH-Exec-Kanal auf der gespeicherten Anmeldung, Ausführung durch die
  Login-Shell des Remote-Benutzers wie `ssh host 'befehl'`. `argv`, `cwd` und `env` werden
  POSIX-gequotet (`cd -- '…' || exit $?; export N='…'; …`); Windows-OpenSSH-Ziele verwenden die
  Form `command` in ihrer eigenen Shell-Syntax. Abbruch und Zeitlimit senden die Kanalanfrage
  `signal` TERM, nach 10 s KILL, dann wird der Kanal geschlossen; OpenSSH ≥ 7.9 stellt sie mit
  `killpg` an die Prozessgruppe des Befehls zu (nicht bei Forced Command/Subsystem). Dieselbe
  Funktion bietet die CLI: `se exec @label[:/dir] -- …`.
- **Share-Peers:** derselbe Weg wie `se exec PEER` über den Hintergrunddienst; das Zielgerät muss
  diesem Gerät Exec erlaubt haben (`se share grants exec enable` dort). Prozessbaum-Eingrenzung,
  Entzug und Verlauf: [`REMOTE_EXECUTION_PLAN.md`](REMOTE_EXECUTION_PLAN.md).
- **FTP, WebDAV, SMB, Drive:** keine Befehlsausführung möglich; `exec` meldet das.

## Sicherheit

- Der Server läuft mit den Rechten des angemeldeten Benutzers und sieht alle gespeicherten
  Verbindungen. Ob ein Tool ohne Rückfrage läuft, entscheidet der Agent-Host anhand seiner
  Einstellungen und der Tool-Markierungen; die Codex-Sandbox gilt für MCP-Server nicht.
- Kein Tool nimmt Passwörter entgegen. Gespeicherte Zugangsdaten werden automatisch verwendet;
  neue Passwort-Anmeldungen legt der Benutzer im Terminal an
  (`se connections add … --password-stdin`), Schlüssel-Anmeldungen kann der Agent über das
  `se`-Tool anlegen.
- Der Skill weist den Agenten an, Share-Vertrauen (Anfragen annehmen, Exec-Grants, Auffindbarkeit,
  neue Peers/Räume) nur auf ausdrücklichen Wunsch des Benutzers zu ändern.
- Der JSON-RPC-Strom läuft über ein Duplikat der ursprünglichen Standardausgabe; die
  Standardausgabe des Prozesses zeigt auf stderr, damit keine Ausgabe den Protokollstrom stört.

## Updates

Der Desktop-Updater ersetzt `se` erst, wenn jeder `se`-Prozess beendet ist, und wartet darauf
höchstens 30 s. Schreibt er dazu `daemon.stop` = `stop`, beendet sich `se mcp` geordnet (Jobs
abbrechen, höchstens 10 s warten). Danach im Agenten den MCP-Server neu verbinden oder eine neue
Sitzung starten. Ein beim Start bereits vorhandener alter Stoppwert wird ignoriert; das
Terminal-Update `se update` (Versionsübergabe) beendet den Server nicht.

## Grenzen

- **Android:** kein `se mcp`. Agenten in Termux könnten zwar lokale Programme starten, die Daten
  der Android-App sind aber app-privat; ein Anschluss bräuchte eine exportierte App-Schnittstelle
  (offen in `docs/TODO.md`, AIPLUG-ANDROID).
- Abbrechen eines `se`-Tool-Jobs fragt unter Linux per SIGINT (wie Strg+C) und beendet nach 10 s;
  unter Windows läuft das Kind ohne Konsolenfenster und wird direkt beendet.
- `copy`/`move` übernehmen die Grenzen von `se cp`: höchstens 1 000 000 Einträge, 128 MiB
  Pfadtext, Tiefe 512, und lesen jede Quelldatei vor dem Veröffentlichen in einen lokalen Spool.
- Links: Links innerhalb eines kopierten Ordners werden nie verfolgt oder neu angelegt (kein
  Backend kann Links anlegen); ein als `source`/`destination` genannter Link wird verfolgt, Links
  auf Ordner nur bei lokalen Pfaden. Die serverseitige Suche von SSH-Agenten und Share-Peers
  überspringt Links; nur die Ordnerdurchsuchung meldet sie.
- SSH-Server ohne `signal`-Unterstützung sehen beim Abbruch nur das Schließen des Kanals.

## Fehlersuche

`status` aufrufen (Hintergrunddienst, Share-Verbindung, Zugangsdatenspeicher), `se plugin status`
im Terminal (Host-CLIs, installierter Stand, Aktualität), `se mcp --help`. Fehlermeldungen des
Servers stehen in den MCP-Protokollen des Hosts (stderr).
