# Smart Explorer für KI-Agenten (`se mcp`, `se plugin`)

Quellstand: 2026-10-26; die neue Generationsverwaltung wartet noch auf die
gemeinsame Remote-Abnahme und den terminalen Release. Aktueller Vertrag:
[`P13/P29`](plaene/2026-10-24-batches/agentenvertrag.md).
Historische Planung und Entscheidungen:
[`docs/plaene/2026-10-12-ki-agenten-plugin/plan.md`](plaene/2026-10-12-ki-agenten-plugin/plan.md),
Vorrang vor den Datei-Werkzeugen des Hosts sowie dauerhafte Shell-Sitzungen und Anhängen
(Abschnitt R4):
[`docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md`](plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md).
Echte Terminalsitzungen in `se shell` (PTY, Anhängen von Terminals, Agenten nur an Befehlssitzungen):
[`docs/plaene/2026-10-14-echtes-terminal/plan.md`](plaene/2026-10-14-echtes-terminal/plan.md).
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

Dieselben Schritte bietet die Desktop-App unter **Einstellungen → Integration → KI-Agenten
(Claude Code, Codex)**: Status je Host, „In alle gefundenen installieren“, „Nur Claude Code“,
„Nur Codex“, „Entfernen“ und „Konfiguration zeigen“ für die übrigen MCP-Clients. Die App führt dafür
das neben ihr installierte `se` mit `plugin …` aus, sodass GUI und Terminal dieselben Dateien und
Host-Einträge erzeugen (`native/src/app/core/settings_ai_plugin.rs`). Auf Android gibt es kein `se`
und damit keinen MCP-Server.

- **Claude Code:** `se` schreibt einen lokalen Plugin-Marketplace nach
  `<App-Daten>/smart_explorer/ai-plugins/claude/` (Plugin `smart-explorer` mit `.mcp.json` →
  absoluter Pfad des geprüften Installationsstarters + `mcp`, bei einer bisherigen
  Nur-Terminal-Installation ohne Managed-Manifest der vorhandene lokale `se`-Pfad,
  Skill `smart-explorer`) und ruft
  `claude plugin marketplace add <Ordner> --scope user` sowie
  `claude plugin install smart-explorer@smart-explorer --scope user -y`. Claude Code lädt Plugins
  lokaler Marketplaces bei jedem Sitzungsstart direkt aus dem Ordner. Die Tools heißen
  `mcp__plugin_smart-explorer_se__<tool>`; dauerhaft erlauben lassen sie sich mit der
  Berechtigungsregel `mcp__plugin_smart-explorer_se__*` in den eigenen Claude-Code-Einstellungen
  (Plugins dürfen keine Regeln mitbringen).
- **Codex:** formatwahrende Änderung von `mcp_servers.smart-explorer` in
  `$CODEX_HOME/config.toml`, ohne `CODEX_HOME` in `~/.codex/config.toml`, sowie der
  Skill unter `~/.agents/skills/smart-explorer/SKILL.md`. Der Eintrag startet den
  geprüften Installationsstarter mit `mcp`; ohne Managed-Manifest bleibt der
  vorhandene absolute lokale `se mcp`-Einstieg verwendbar. Eigentumsfingerprints schützen die
  verwalteten Felder `command` und `args`; Kommentare, unbekannte Schlüssel,
  Toolfilter und vorhandene `env`-Werte bleiben erhalten. Ein neu angelegter
  Eintrag übernimmt auf Linux die gesetzten Werte `XDG_DATA_HOME`, `XDG_RUNTIME_DIR` und
  `DBUS_SESSION_BUS_ADDRESS`. Ein beobachteter Bearbeitungskonflikt erscheint
  als `integration_conflict`; er löst kein Entfernen und erneutes Anlegen aus.
- **Andere Clients** (Cursor, VS Code, Gemini CLI/Antigravity, OpenCode, Claude Desktop):
  `se plugin config generic|vscode|opencode` ausgeben und in deren MCP-Konfiguration eintragen.

Ein normales Runtimeupdate ändert den registrierten Starterpfad nicht.
Neue Serverprozesse wählen die bestätigte Generation der Installation.
Die Basisskill-Datei bleibt versionsunabhängig; der laufende MCP-Server liefert
seine maßgeblichen Anweisungen und seinen Werkzeugkatalog. Die Migration eines
alten Eintrags setzt einen belegten verwalteten Zustand voraus. Manuell
bearbeitete Einträge werden bei einem Konflikt erhalten.

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
  Sitzungsstart aufzuhalten. `exec`, `job_input`, `job_cancel`, `sessions`, `status` und `se`
  bleiben verzögert; die Anweisungen nennen die Suche nach `smart-explorer`. Wer alle Tools verzögert
  haben will, setzt im eigenen Servereintrag `"alwaysLoad": false` (ab Claude Code v2.1.287).
- **Tool-Beschreibungen** nennen jeweils das ersetzte Werkzeug, zum Beispiel „Use it instead of
  Read or `cat`“.
- **Skill und Manifest:** Die Skill-Beschreibung (Auslöser in Claude Code und Codex) beginnt mit
  dem Vorrang. Der Skill führt eine Tabelle Aufgabe → Smart-Explorer-Tool → ersetztes Werkzeug.
  Die Plugin-Beschreibung in `plugin.json`/`marketplace.json` nennt den Vorrang ebenfalls.
- Inhaltssuche in Dateien (Grep) und lokale Builds bleiben beim Host, weil `search` nur Namen
  sucht.

Nach der Migration zum Installationsstarter wählen neue Serverprozesse die neue
Runtime ohne erneute Registrierung. Eine laufende kompatible Verbindung behält
ihren bisherigen Katalog. Ein Dateirefresh allein lädt keine neuen Tools in
einen bereits initialisierten Hostprozess.

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
| `exec` | Befehl auf gespeichertem SFTP-Remote (SSH) oder Share-Peer, als Job; mit `session` in einer dauerhaften Shell | zerstörend |
| `sessions` | dauerhafte Shell-Sitzungen auflisten (mit `kind` `commands` oder `terminal`), einer mit `se shell -T --attachable` geöffneten Befehlssitzung beitreten (`attach`; Terminalsitzungen werden abgelehnt), Aktivität der anderen lesen (`activity`), schließen bzw. lösen (`close`) | ändernd |
| `job_status`, `job_input`, `job_cancel` | Jobs verfolgen (Ausgabe ab Offset), stdin senden, abbrechen | lesend / ändernd |
| `screen` | Geräte, Bildschirmfähigkeit, Quellen und begrenzte Standbilder | nur lesend |
| `screen_input` | Eingabe über den autorisierten Bildschirmvertrag | ändernd |
| `screen_window` | Nativen Fensterbetrachter mit gebundener Quelle öffnen | ändernd |
| `tcp_forward` | Autorisierten TCP-Tunnel starten, anzeigen und schließen | ändernd |
| `apps` | Installierten Hostkatalog lesen und eine ausdrücklich freigegebene Kataloganwendung starten | lesend / ändernd |
| `rdp` | Authentifizierten Dienstbestand und vorhandene Clients verwenden; Client und Lease gemeinsam besitzen | lesend / ändernd |
| `login` | Explizites gepinntes Maschinenziel und autorisierte Login-Sitzung; keine Loginpixel in JSON | lesend / ändernd |
| `se` | jeder weitere `se`-Befehl als Kindprozess (Share-Verwaltung, Verbindungen anlegen, Laufwerke, Update, Diagnose) | zerstörend |

Die neuen Fernzugriffsanschlüsse sind im laufenden Kandidaten registriert
(`native/src/cli/mcp/catalog.rs`, `server.rs`); die Tabelle ist kein
Releasebeleg. Rechte, Besitz und Plattformausnahmen stehen in
[`oberflaechen/fernzugriff.md`](oberflaechen/fernzugriff.md).

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
  Standardgrenze für Laufzeit oder Ausgabe. Die Dateien enthalten nur Chiffrat: jeder angehängte
  Block ist ein AES-256-GCM-Datensatz unter einem Schlüssel, der nur im Speicher dieses Jobs liegt
  (`native/src/cli/mcp/spool.rs`). So bleibt auch die Ausgabe von `se cat vault://…` aus einem
  entsperrten Tresor nie im Klartext auf der Platte (Ort K49 der Klartextgrenze). Beim Sitzungsende werden laufende Jobs abgebrochen
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
  Funktion bietet die CLI: `se exec @label[:/dir] -- …`. Ein noch unbekannter Hostschlüssel wird
  seit 2026-10-16 nie still gespeichert: der Aufruf scheitert mit Schlüsseltyp und
  SHA-256-Fingerabdruck. Bestätigen soll ihn der Mensch nach Vergleich mit dem Server, im
  Terminal per Rückfrage von `se` oder mit
  `se connections trust-host-key HOST:PORT --fingerprint SHA256:…`; ein Agent, der den
  Fingerabdruck aus derselben Fehlermeldung übernimmt, prüft nichts.
- **Share-Peers:** derselbe Weg wie `se exec PEER` über den Hintergrunddienst; das Zielgerät muss
  diesem Gerät Exec erlaubt haben (`se share grants exec enable` dort). Ein Grant kann ein Konto
  (`--run-as`, Linux-Host mit root-Worker) und eine Programmliste (`--allow-program`, dann weder
  Shell noch Sitzung) festlegen. Bei einer Programmliste muss `argv[0]` ein gelistetes Programm
  sein (gefunden über einen absoluten Pfad oder den festen Suchpfad des Hosts, nie über ein
  mitgesendetes `PATH`), und `env` darf nur `LANG`, `LC_*` und `TERM` enthalten; jede andere
  Variable lehnt den Start mit ihrem Namen ab. Die Liste ist keine Sandbox: ein gelisteter
  Interpreter oder Befehlsläufer entspricht einer vollen Shell. Das Zielgerät protokolliert jeden
  Start dauerhaft (`se share exec log`). Prozessbaum-Eingrenzung, Entzug und Verlauf:
  [`REMOTE_EXECUTION_PLAN.md`](REMOTE_EXECUTION_PLAN.md).
- **FTP, WebDAV, SMB, Drive:** keine Befehlsausführung möglich; `exec` meldet das.

## Dauerhafte Shell-Sitzungen und Anhängen

Ab 0.5.178 läuft Remote-Ausführung auf Wunsch wie eine SSH-Sitzung: Jeder Befehl kehrt mit
Ergebnis zurück, Arbeitsverzeichnis, Umgebung, Variablen und Funktionen bleiben erhalten.
Es gibt zwei Arten von Sitzungen (Feld `kind`): **Befehlssitzungen** (`commands`: die Sitzungen
der Agenten, `se shell -T`, `se shell` ohne Terminal und der Rückfall bei einer Gegenstelle ohne
PTY) und **Terminalsitzungen** (`terminal`:
`se shell` in einem Terminal, mit PTY auf der Gegenstelle). Agenten arbeiten nur in
Befehlssitzungen.

- **Agent:** `exec` mit `session: "<name>"` (Buchstaben, Ziffern, `-`, `_`, `.`). Der erste Aufruf
  öffnet die Sitzung auf `target` (ein Pfad in `@label:/ordner` ist das Startverzeichnis), spätere
  Aufrufe brauchen kein `target`; ein angegebenes `target` muss zum Ziel der Sitzung passen.
  `cwd`/`env` ändern den Sitzungszustand vor dem Befehl, `stdin` wird als Eingabe übergeben
  (POSIX als Heredoc mit ergänztem Zeilenende, PowerShell als `$input`); `keep_stdin_open` gibt es
  hier nicht. Das Ergebnis enthält `exit_code`, `cwd`, `session` und `session_ended`. Die Sitzung
  ist immer eine Befehlssitzung ohne PTY.
- **Terminal (Terminalsitzung):**
  `se shell [ZIEL] [--cwd DIR] [--env N=W]… [--name NAME] [--attachable] [-e ZEICHEN|none]`.
  Sind Standardeingabe und Standardausgabe Terminals und fehlt `-T`, verhält sich `se shell` wie
  `ssh HOST`: Die Gegenstelle gibt der Login-Shell ein PTY, das lokale Terminal läuft im
  Rohmodus, und Tasten und Ausgabe gehen unverändert durch (einzige Ausnahme: die Anforderung des
  win32-input-mode, siehe „Sicherheit“). Damit funktionieren Formatierung,
  Vollbildprogramme (nvim, claude, codex), die Tab-Vervollständigung der Shell der Gegenstelle,
  Passwortabfragen, Strg+C an das Vordergrundprogramm und die Fenstergröße. Die Sitzung endet, wenn
  die Shell der Gegenstelle endet. Exitcode: der Exitstatus dieser Shell (`exit N` ergibt N, Signal
  n ergibt 128+n); 130 nach `~.` des Besitzers, 0 nach `~.` eines angehängten Terminals; 125 bei
  Fehler, Verbindungsverlust, Entzug der Exec-Freigabe und Update-Stopp; 128+n nach lokalem
  SIGTERM, SIGHUP, SIGQUIT oder SIGINT (Windows: 130 nach Strg+Untbr, Schließen des Fensters,
  Abmelden und Herunterfahren). Escape-Folgen wie `ssh(1)` nur am Zeilenanfang: `~.` verlässt die
  Sitzung, `~~` sendet ein `~`, `~?` listet sie; `-e ZEICHEN` setzt ein anderes ASCII-Zeichen (nicht
  `.`, `?`, Wagenrücklauf, Zeilenvorschub), `-e none` schaltet sie ab. Andere `ssh`-Folgen (`~^Z`, `~#`,
  `~&`, `~B`, `~C`, `~R`, `~V`, `~v`, `~I`) gibt es nicht.
- **Zeilenmodus (Befehlssitzung):** Mit `-T`/`--no-tty`, ohne Terminal an Standardeingabe oder
  Standardausgabe (Pipe) und bei einer Gegenstelle ohne PTY läuft `se shell` als Befehlssitzung. Im
  Terminal zeigt der Prompt `ziel:cwd$` auf stderr, eine Zeile mit abschließendem `\` (PowerShell:
  `` ` ``) wird fortgesetzt, Eingaben während eines eigenen Befehls gehen an diesen Befehl
  (POSIX). Mit Pipe ist jede Zeile ein Befehl ohne eigene Eingabe. Exitcode: der des letzten
  Befehls oder von `exit N`; 130 nach Strg+C, 125 bei Fehler der Sitzung. Rückfall: Lehnt ein
  SSH-Server das PTY ab (`PermitTTY no`), meldet ein Share-Host die Fähigkeit `exec_terminal_v1`
  nicht, oder kann das lokale Terminal nicht in den Rohmodus (mintty/Git Bash unter Windows hat
  eine Pipe als Standardeingabe), schreibt `se` den Grund in einer Zeile auf stderr und öffnet
  diesen Modus.
- **Ablauf der Terminalsitzung:** `se` prüft Name, lokale Fenstergröße und `TERM` (das lokale
  `TERM`, wenn es 1 bis 64 Zeichen aus `A-Za-z0-9._+-` hat, sonst `xterm-256color`), baut die
  Verbindung auf und setzt erst danach das lokale Terminal in den Rohmodus. SSH-Ziel: ein Kanal mit
  `pty-req` (feste Terminalmodi VINTR=3, VQUIT=28, VERASE=127, VKILL=21, VEOF=4, VSUSP=26,
  IUTF8=1) und `shell`; Größenänderungen gehen als `window-change`. Share-Ziel: derselbe
  Exec-Weg wie bei `exec` (Grant, ALPN `smart-explorer/share-exec/2`, Registry, Containment) mit
  dem Befehl `Login` und einer Terminalanforderung (`TERM`, Größe); Größenänderungen gehen als Rahmen
  RESIZE. Der Host legt das PTY an (Linux und Android: POSIX-PTY mit `setsid`/`TIOCSCTTY`; Windows:
  ConPTY) und startet die Login-Shell darin: Linux das absolute `SHELL` der Job-Umgebung
  (Dienstumgebung plus `--env`), sonst `/bin/sh`, mit `argv[0]` `-<Name>` (Login-Konvention wie
  `sshd`); Android `/system/bin/sh`, ebenfalls mit `argv[0]` `-sh`; Windows
  `powershell.exe -NoLogo` ohne `TERM`. Startverzeichnis und Umgebung (`--cwd`, `--env`, Ordner im
  Ziel) setzt `se` beim Start. Die Ausgabe ist der unveränderte Byte-Strom des PTY; Marken, Nonce
  und der Befehlsstrom der Befehlssitzung gibt es hier nicht.
- **Ablauf der Befehlssitzung:** Eine Probe in der Login-Shell (`echo %OS%;$PSVersionTable`) erkennt
  cmd.exe bzw. PowerShell (dann `powershell.exe -NoLogo -NoProfile -NonInteractive -Command -`) oder
  eine POSIX-Shell (dann `bash -l`, sonst `sh -l`). Jeder Befehl endet mit Marken, die Exitcode und
  Arbeitsverzeichnis tragen und eine zufällige Nonce je Sitzung enthalten.
- **Anhängen, Shell an Sitzung:** `se shell --list [--json]` zeigt die anhängbaren Sitzungen des
  Benutzers (Name, ID, Dialekt oder `-`, Ziel, Besitzer, Art `terminal` oder `commands`; in
  `--json` hat jeder Eintrag `kind` immer und `dialect` nur, wenn er bekannt ist),
  `se shell --attach NAME|ID` tritt bei. Befehlssitzungen von Agenten sind anhängbar, solange der
  Agent nicht `attachable: false` setzt; Befehle und Ausgaben der anderen erscheinen mit
  `[se mcp (pid …)] $ …`, `--attach` arbeitet dort im Zeilenmodus, Strg+D löst sich, die Sitzung
  läuft weiter. Eine Terminalsitzung (`se shell ZIEL --attachable` im Terminal) tritt ein zweites
  Terminal bei, wenn Standardeingabe und Standardausgabe Terminals sind und `-T` fehlt (sonst
  Meldung und Exitcode 125): Alle Teilnehmer sehen dieselbe Ausgabe und tippen in dasselbe PTY, die
  Größe des PTY folgt dem zuletzt aktiven Teilnehmer (Eingabe, Größenänderung oder Beitritt), und
  eine Hinweiszeile „attached/detached“ geht an alle. `~.` löst einen Angehängten, `~.` des
  Besitzers beendet die Sitzung für alle.
- **Anhängen, Sitzung an Shell:** Eine mit `se shell ZIEL -T --attachable` geöffnete Shell (oder
  `se shell … --attachable` ohne Terminal) ist eine Befehlssitzung; der Agent übernimmt sie mit
  `sessions` `{"action": "attach", "session": "<name>"}` und arbeitet dann mit `exec` und
  `session: "<name>"` darin; `sessions` `activity` liefert, was die anderen dort seitdem
  ausgeführt und ausgegeben haben (höchstens 64 KiB je Sitzung). `close` beendet eine eigene
  Sitzung oder löst eine übernommene. Eine Terminalsitzung kann der Agent nicht übernehmen:
  `sessions` listet sie mit `kind` `terminal`, `attach` meldet einen Fehler mit dem Hinweis auf
  `se shell ZIEL -T --attachable`.
- **Gemeinsamer Zustand (Befehlssitzung):** Befehle aller Teilnehmer laufen nacheinander in
  derselben Shell. Ein Abbruch (`job_cancel`, `timeout_seconds`, Strg+C) beendet die ganze Sitzung
  für alle, weil der Exec-Strom keinen Signalweg zu einem einzelnen Befehl hat; ebenso `exit`. Das
  Ende des MCP-Servers beendet seine Sitzungen; angehängte Terminals erfahren das. In einer
  Terminalsitzung geht Strg+C als Byte 3 an das Vordergrundprogramm der Gegenstelle und beendet die
  Sitzung nicht.

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
- Sitzungen erweitern keine Rechte: Sie laufen über dieselbe SSH-Anmeldung bzw. denselben
  freigegebenen Share-Exec wie `exec`; entzieht die Gegenstelle die Exec-Freigabe, endet die
  Sitzung für alle.
- Anhängen: Der besitzende Prozess (`se mcp` oder `se shell`) bedient seine Sitzungen über einen
  Dienst nur auf `127.0.0.1`. Jeder Client weist im ersten Rahmen binnen 5 s ein Token aus 32
  Zufallsbytes nach (Vergleich in konstanter Zeit); das Anhängetoken steht nur im
  besitzergeschützten Eintrag `<App-Daten>/smart_explorer/shell-sessions/<id>.json` (Unix 0700/0600,
  Windows private DACL), das Besitzertoken nie auf der Platte. Listen zeigen weder Port noch Token.
  Eine `se shell` ist nur mit `--attachable` gelistet; ohne Zustimmung des Menschen kann kein Agent
  sie übernehmen. Der Skill weist den Agenten an, nur auf Wunsch des Nutzers beizutreten.
- Angehängte Clients dürfen Befehle einreihen, Eingabe nur an ihren eigenen laufenden Befehl
  senden, abbrechen und sich lösen; schließen kann nur der Besitzer. Bezeichner und gezeigte
  Befehlstexte anderer Teilnehmer werden ohne Steuerzeichen angezeigt.
- Terminalsitzungen (`se shell` im Terminal) bringen kein neues Recht: Sie nutzen dieselbe
  SSH-Anmeldung bzw. denselben Exec-Grant wie `exec`; entzieht die Gegenstelle die Freigabe, endet
  die Sitzung für alle (Exitcode 125, Terminal wiederhergestellt). Ein Share-Host startet ein
  Terminal nur mit der Fähigkeit `exec_terminal_v1`; der Client sendet keinen Terminal-Start an einen
  Host ohne sie, und ein Host ohne PTY lehnt ihn mit `terminal_unsupported` ab. Gleiche ALPN,
  Protokollversion und Rahmen ohne Terminal: ältere Clients und Hosts arbeiten wie bisher.
- Agenten tippen nie in Terminalsitzungen: `sessions` `attach` lehnt sie ab, denn Eingaben eines
  Agenten träfen jedes gerade laufende Programm (nvim, `sudo`, einen Interpreter) und den halb
  getippten Text des Menschen. Das Anhängen eines Terminals ist nur mit `--attachable` möglich (ohne
  das Flag gibt es kein Anhängetoken) und verlangt ein Terminal an Standardeingabe und
  Standardausgabe; Kindprozesse des MCP-Tools `se` haben Pipes und können keiner Terminalsitzung
  beitreten. Das Anhängeprotokoll Version 1 (ältere `se`) wird von Terminalsitzungen nach der
  Tokenprüfung abgelehnt; ältere Besitzer bleiben für Befehlssitzungen erreichbar.
- Die Eingabe eines Teilnehmers erreicht nur das PTY, nie die anderen Teilnehmer; diese sehen
  allein, was die Gegenstelle zurückschreibt (bei Passwortabfragen kein Echo). Die Ausgabe der
  Gegenstelle gelangt wie bei `ssh` unverändert an das lokale Terminal, mit einer Ausnahme: `se`
  entfernt den Parameter 9001 (win32-input-mode) aus jeder Folge `ESC[?…h` bzw. `ESC[?…l` und
  verwirft die Folge, wenn kein anderer Parameter bleibt. Diese Anforderung schreibt das ConPTY
  eines Windows-Hosts beim Start; befolgte sie eine Windows-Konsole, meldete sie jede Taste als
  Steuerfolge, und `~.`, `~?` und `~~` wirkten nicht. Von einer über zwei Ausgabestücke geteilten
  Folge hält `se` höchstens 64 Byte zurück; eine längere Folge geht unverändert durch. Die übrigen
  Steuersequenzen der Gegenstelle (Fenstertitel, Zwischenablage über OSC 52, Terminalabfragen)
  gelten mit demselben Vertrauensmodell wie bei `ssh`. Eigene Texte von `se` (Hinweise, Namen
  anderer Teilnehmer, Fehler) enthalten keine Steuerzeichen. Terminalinhalt steht in keiner
  Protokolldatei; die Exec-Historie des Hosts zeigt `<shell>`.
- Terminalzustand: Der Rohmodus wird erst nach erfolgreichem Verbindungsaufbau gesetzt und bei
  normalem Ende, Fehler, Panik, SIGTERM, SIGHUP, SIGQUIT, SIGINT (Unix), Strg+Untbr, Schließen des
  Fensters, Abmelden, Herunterfahren (Windows) und Update-Stopp wiederhergestellt. Wie `ssh` setzt
  `se` Modi, die ein Programm der Gegenstelle gesetzt hat (Alternativbildschirm, Mausmeldung,
  geklammertes Einfügen), nicht zurück.
- Grenzen gegen Missbrauch: Fenstergrößen 1 bis 32767 Spalten und Zeilen, `TERM` 1 bis 64 Zeichen
  aus `A-Za-z0-9._+-` (Client und Host prüfen); RESIZE-Rahmen höchstens 256 Byte, Datenrahmen
  höchstens 64 KiB; höchstens 8 angehängte Terminals; ein Angehängter, dessen Warteschlange 30 s
  voll bleibt, wird getrennt.

## Updates

Nach dem ersten Selbstupdate einer bisherigen Nur-Terminal-Installation kann
das Managed-Manifest noch fehlen: Der alte Updater ersetzte ausschließlich `se`.
`se plugin install claude`, `se plugin install codex` und
`se plugin config generic` verwenden dann den vorhandenen absoluten CLI-Pfad.
Diese lokale Auflösung benötigt keinen zusätzlichen Download. Der am
Prozessstart erfasste Imagehash verhindert dabei, dass eine inzwischen ersetzte
alte CLI ihre Vorlagen für neue Programmbytes zurückschreibt. Ein späteres
reguläres Managed-Update stellt die Launcher-/Runtimegeneration bereit und kann
die eigenen Profile über denselben Eigentumsnachweis auf den Starter migrieren.
Der alte Updateabschluss und dessen Rollback erhalten keine neue Voraussetzung.
Legacy-Plugininstallation und Profilauffrischung öffnen die vorhandene CLI
lesend und benötigen keine Schreibrechte im Installationsverzeichnis.
Während der Vorlagenerzeugung verhindert ihre Imagelease unter Linux einen
parallelen aktuellen Terminal-Swap; Windows hält das Image ohne Schreib-/
Löschfreigabe offen. Eine vorhandene historische Update-Lockdatei wird nur
gelesen. Die reine Konfigurationsausgabe erzeugt keine Sperrdatei.

Unveränderte mit v0.5.184 erzeugte Claude-/Codex-Profile lassen sich mit
`se plugin uninstall claude` beziehungsweise `se plugin uninstall codex`
unmittelbar entfernen. Eine vorherige Migration ist dafür nicht erforderlich.
Die Migration erkennt die drei früher auf Linux vom Produkt gesetzten
XDG-/DBus-Schlüssel als eigene Werte; zusätzliche Benutzerschlüssel und
Toolfilter bleiben geschützt. Bei neuen Eigentumsnachweisen werden auch die
aufgezeichneten Envwerte vor der Entfernung verglichen. Auf Windows gelten
manuell hinzugefügte Unix-Envwerte weiterhin als Benutzereinstellungen.

Windows-Installer, Linux-Installer und Desktop-Updater verwenden denselben
hashgebundenen Helfereinstieg. Unter dem Installationsstamm liegen
`mcp/launchers/<SHA-256>/`, `mcp/runtimes/<SHA-256>/` und der bestätigte Zeiger
`mcp/current.v1.json`. Der Starter prüft Manifest, Pfad, Dateischutz und Hash
vor dem Start; eine laufende Generationsdatei wird nicht überschrieben.

Eine aktive Operation oder dauerhafte Shell verschiebt den Updatebeginn.
Eine kompatible ruhende Sitzung kann ihre Generation behalten, wenn deren
konkreter Runtimehash, IPC-Vertrag und Speichervertrag ausdrücklich zulässig
sind. Während des Austauschs bleiben daemonabhängige Aufrufe gesperrt und
handeln danach die bestätigte Daemongeneration neu aus. Eine unterbrochene
Mutation wird nicht automatisch wiederholt. Unversionierte oder inkompatible
Prozesse benutzen den bisherigen geordneten Stopweg. Ein verwaister
vorbereiteter Austausch bleibt als `update_recovery_required` gesperrt, bis
der reguläre Updateweg ihn bestätigt zurücknimmt oder abschließt.

`se plugin status --json`, das MCP-Tool `status` und die Desktopseite zeigen
die installierte Generation, `next_runtime_sha256`, die aktiven Server des
aktuellen Benutzerprofils und Profilkonflikte. `active_runtime_sha256` ist
außerhalb eines tatsächlichen Runtimeprozesses `null`.
`reconnect_required` bezeichnet eine nicht mehr zugelassene Generation;
`new_process_for_new_capabilities` zeigt neue Fähigkeiten bei weiterhin
kompatibler alter Sitzung. `busy` allein verlangt kein Neuverbinden.

Neue Fähigkeiten benötigen einen tatsächlichen MCP-Prozessstart über den Host.
Smart Explorer erzwingt keinen Austausch innerhalb einer laufenden
stdio-Verbindung und startet bei einer administrativen Installation keinen
Benutzer-MCP-Server mit Administratorrechten.

Die reguläre Windows-Deinstallation entfernt die nachgewiesen eigenen
Runtimekopien, Launcher, Manifeste und Installationsmetadaten, bevor sie die
Hauptprogramme löscht. Die Prozesse dieser Installation müssen zuvor regulär
geschlossen sein. Eine belegte aktive Nutzung, ein ungeklärter Prozessbesitz,
eine offene Updatewiederherstellung oder ein Löschfehler beendet den Versuch mit
einer Meldung und erhält den Deinstallationseintrag für die Wiederholung.
Fremde Dateien und Benutzerprofile bleiben erhalten. Ein begonnenes Entfernen
sperrt neue Starts aus der betroffenen MCP-Installation und wird nach einem
Teilfehler gegen dieselben ursprünglichen Dateihashes fortgesetzt.

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
- Agenten-Sitzungen (Befehlssitzungen, auch `se shell -T`): kein PTY (Vollbildprogramme und
  Passwortabfragen funktionieren nicht) und kein Abbruch einzelner Befehle (`EXEC-PTY` in
  `docs/TODO.md`). Höchstens 8 angehängte Clients und 64 wartende Befehle je Sitzung, je Befehl
  höchstens 1 MiB Befehl und Eingabe. Der PowerShell-Dialekt ist nur lokal unter Windows geprüft
  (`EXECSESS-WIN`).
- Alle Sitzungen: keine Wiederanbindung nach Verlust der Verbindung bzw. des Exec-Stroms (auch
  `ssh` kann das nicht; `EXEC-PTY`). Jede Sitzung, auch eine Terminalsitzung, belegt einen
  SSH-Kanal (OpenSSH erlaubt standardmäßig 10 je Verbindung) bzw. einen der zwei Exec-Plätze, die
  eine Share-Gegenstelle je Gerät gleichzeitig zulässt.
- Terminalsitzungen: Escape-Folgen nur `~.`, `~~`, `~?`. Fenstergrößen 1 bis 32767. Ein Windows-Host
  braucht Windows 10 1809 bzw. Server 2019 (ConPTY); ältere Versionen melden `exec_terminal_v1`
  nicht, `se shell` fällt dann in den Zeilenmodus zurück. Ein lokales Terminal unter Windows ohne
  Konsole (mintty/Git Bash mit Pipe als Standardeingabe) läuft im Zeilenmodus. Nach `~.` meldet
  `se shell` das Ende sofort. Eine SSH-Gegenstelle endet über Abbruchsignale (TERM, nach 10 s KILL)
  und das Schließen des Kanals bzw. der Verbindung (der Server sendet SIGHUP an die Sitzung); das
  kann bis etwa 20 s dauern, und mit `nohup` gestartete Prozesse überleben wie bei `ssh`. Auf einem
  Share-Host beendet das Containment (systemd-cgroup, Job Object, Android-Subreaper) den
  gesamten Prozessbaum, auch mit `nohup` gestartete Prozesse.
  Nicht geprüft: Android-Host auf einem Gerät, Windows-OpenSSH als SSH-Ziel, Linux-Client gegen
  Windows-Host und Windows-Client gegen Linux-Host (`ETERM-ANDROID`, `AIPLUG-WINSSH`,
  `ETERM-XHOST` in `docs/TODO.md`). `se exec -t` (ein einzelner Befehl mit PTY) und lesendes
  Beobachten durch Agenten gibt es nicht (`ETERM-EXEC-T`, `ETERM-AGENT-VIEW`).

## Fehlersuche

`status` aufrufen (Hintergrunddienst, Share-Verbindung, Zugangsdatenspeicher), `se plugin status`
im Terminal (Host-CLIs, installierter Stand, Aktualität), `se mcp --help`. Fehlermeldungen des
Servers stehen in den MCP-Protokollen des Hosts (stderr).
