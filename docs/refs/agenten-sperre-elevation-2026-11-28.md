# Agenten-Vollsperre maskierter Pfade und erhöhte Ausführung (abgerufen 2026-11-28)

Zweck: belegte Grundlage für die Nutzervorgaben vom 2026-11-28 an `se mcp` und das se-Plugin für
Claude Code und Codex:

- Vollsperre: Ist eine Maskenliste auf einen Projekt- oder Ordnerpfad angewendet, darf kein
  Werkzeug- oder Shell-Zugriff außerhalb des se-MCP diesen Pfad erreichen (cd, Shell, Lesen,
  Schreiben, Editieren, Suche, Ordnerzugriffe, alle übrigen Zugriffe). Die technische Umsetzung ist
  frei; entscheidend ist die Zuverlässigkeit.
- Shell-Werkzeug im MCP: Befehle laufen mit den Rechten von se; erhöhte Ausführung fragt die
  Elevation über den nativen Systemdialog an (Windows UAC, Linux Systemdialog).

Kennzeichnung in dieser Datei: **Belegt** = Primärquelle genannt. **Beobachtet** = Aufruf auf einem
Ubuntu-24.04-Rechner (Kernel 6.8.0-149, x86_64) am 2026-11-28 mit Eingabe und Ergebnis.
**Unbelegt** = Lücke, die genaue fehlende Tatsache steht dabei. **Widerspruch** = Quellen
widersprechen sich. **Folgerung** = eigene Ableitung aus belegten Aussagen, keine Quelle.

## 0. Quellen

| Kürzel | Quelle | Stand |
|---|---|---|
| CC-hooks | `https://code.claude.com/docs/en/hooks.md` | abgerufen 2026-11-28 |
| CC-hooks-guide | `https://code.claude.com/docs/en/hooks-guide.md` | abgerufen 2026-11-28 |
| CC-perm | `https://code.claude.com/docs/en/permissions.md` | abgerufen 2026-11-28 |
| CC-modes | `https://code.claude.com/docs/en/permission-modes.md` | abgerufen 2026-11-28 |
| CC-sandbox | `https://code.claude.com/docs/en/sandboxing.md` | abgerufen 2026-11-28 |
| CC-settings | `https://code.claude.com/docs/en/settings-reference.md` | abgerufen 2026-11-28 |
| CC-managed | `https://code.claude.com/docs/en/managed-settings.md` | abgerufen 2026-11-28 |
| CC-tools | `https://code.claude.com/docs/en/tools-reference.md` | abgerufen 2026-11-28 |
| CC-codemode | `https://code.claude.com/docs/en/code-mode.md` | abgerufen 2026-11-28 |
| CC-plugin | `https://code.claude.com/docs/en/plugins-reference.md` (Manifest-Referenz) und `https://code.claude.com/docs/en/plugins/components.md` | abgerufen 2026-11-28 |
| CC-mods | `https://code.claude.com/docs/en/plugins/mods/events.md` | abgerufen 2026-11-28 |
| CC-bin | installierte Claude Code 2.1.332: `claude --help` und Zeichenketten der ausgelieferten ELF-Datei (Werkzeug-Schemata) | gelesen 2026-11-28 |
| CX-src | `github.com/openai/codex`, Tag `rust-v0.166.0`, Commit `6ff1e1ca3b30fc8b914720681fcf5fe62e95f856` (2026-10-29); installiert ist `codex-cli 0.166.0` | gelesen 2026-11-28 |
| CX-hooks | `https://learn.chatgpt.com/docs/hooks.md` (Weiterleitung von `developers.openai.com/codex/hooks`) | abgerufen 2026-11-28 |
| CX-perm | `https://learn.chatgpt.com/docs/permissions.md` | abgerufen 2026-11-28 |
| CX-win | `https://learn.chatgpt.com/docs/windows/windows-sandbox.md` | abgerufen 2026-11-28 |
| LL | `https://docs.kernel.org/userspace-api/landlock.html` | abgerufen 2026-11-28 |
| UBU | `https://ubuntu.com/blog/ubuntu-23-10-restricted-unprivileged-user-namespaces` | abgerufen 2026-11-28 |
| BWRAP | `github.com/containers/bubblewrap`, Tag `v0.9.0`, `README.md` | abgerufen 2026-11-28 |
| MS-cpau | `https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw` | abgerufen 2026-11-28 |
| MS-rt | `https://learn.microsoft.com/en-us/windows/win32/secauthz/restricted-tokens` | abgerufen 2026-11-28 |
| MS-crt | `github.com/MicrosoftDocs/sdk-api`, Zweig `docs`, `sdk-api-src/content/securitybaseapi/nf-securitybaseapi-createrestrictedtoken.md` | abgerufen 2026-11-28 |
| MS-mic | `https://learn.microsoft.com/en-us/windows/win32/secauthz/mandatory-integrity-control` | abgerufen 2026-11-28 |
| MS-ac | `https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation` | abgerufen 2026-11-28 |
| MS-pipe | `github.com/MicrosoftDocs/sdk-api`, Zweig `docs`, `sdk-api-src/content/winbase/nf-winbase-createnamedpipea.md` | abgerufen 2026-11-28 |
| MS-sei | `github.com/MicrosoftDocs/sdk-api`, Zweig `docs`, `sdk-api-src/content/shellapi/ns-shellapi-shellexecuteinfow.md` | abgerufen 2026-11-28 |
| GSUDO | `github.com/gerardog/gsudo`, Standardzweig Commit `a4eda9b263d72a3e174f73b6b6b3ecfc5bc6656b` (2026-05-19; neuester Tag `v2.7.0-rc.2`) | gelesen 2026-11-28 |
| PKEXEC | `man pkexec` (polkit 124) und `/usr/share/polkit-1/actions/org.freedesktop.policykit.policy` auf Ubuntu 24.04 | gelesen 2026-11-28 |
| SUDO | `man sudo` auf Ubuntu 24.04 | gelesen 2026-11-28 |
| RUN0 | `https://www.freedesktop.org/software/systemd/man/latest/run0.html` | abgerufen 2026-11-28 |
| SE | `native/crates/cli/src/cli/plugin/claude.rs`, `files.rs`, `codex.rs` (Stand Commit `e8277f52`) | gelesen 2026-11-28 |

Ausgangslage se (SE, belegt): `files::write_claude` erzeugt `.claude-plugin/marketplace.json`,
`smart-explorer/.claude-plugin/plugin.json`, `smart-explorer/.mcp.json` und
`smart-explorer/skills/smart-explorer/SKILL.md`; es gibt keine `hooks/hooks.json` und kein
`hooks`-Manifestfeld. `codex.rs` trägt `[mcp_servers.smart-explorer]` in `config.toml` ein und
schreibt ein Skill; es schreibt keine Hooks und keine Permission-Profile. Die se-Werkzeuge heißen in
Claude Code `mcp__plugin_smart-explorer_smart-explorer__<tool>` (`claude.rs`, Feld `tools`).

## 1. Claude Code (Doku Stand 2026-11-28, installiert 2.1.332)

### 1.1 Ereignisse vor einem Werkzeugaufruf (belegt, CC-hooks „Hook lifecycle“)

- `PreToolUse`: vor jedem Werkzeugaufruf, kann blockieren. Ausnahme: `EndConversation` löst weder
  `PreToolUse` noch `PostToolUse` aus.
- `PermissionRequest`: wenn ein Aufruf eine Berechtigungsentscheidung braucht.
- `PreSkillShellCommand`: vor einem in ein Skill eingebetteten Shell-Befehl (`` !`cmd` ``), kann nur
  blockieren. Für diese Befehle läuft **kein** `PreToolUse` (CC-hooks, Warnung unter „PreToolUse“).
- Kein Werkzeugaufruf und damit kein `PreToolUse`: mit `@` erwähnte Dateien werden ohne
  Werkzeugaufruf eingefügt; eine `Read`-Deny-Regel blockiert sie trotzdem, ebenso eingefügte
  Bildpfade, @-erwähnte Ordnerlisten, nach Änderung neu gelesene Dateien und IDE-Kontext (CC-hooks
  „PreToolUse“; CC-perm „What `Read` and `Edit` rules cover“, „Attachments through a symlink“).
  `CLAUDE_CODE_DISABLE_FILE_MENTIONS=1` (ab 2.1.299, nur aus Shell, Nutzer- oder Managed-Settings)
  schaltet die Einfügung ab.
- Vom Nutzer am `!`-Prompt getippte Befehle sind keine Werkzeugaufrufe und laufen in den meisten
  Sitzungen ohne Sandbox (CC-sandbox „What runs outside the sandbox“).
- Monitor: Ein `PreToolUse`-Hook mit Matcher `Bash` läuft auch für Monitor-Befehle und kann sie
  ablehnen; `allow` und `updatedInput` wirken dort nur mit einem Matcher, der `Monitor` einschließt
  (ab 2.1.314, CC-hooks „Match Monitor commands“).
- Code Mode: Werkzeugaufrufe aus dem JavaScript des `REPL`-Werkzeugs gehen unter ihrem normalen Namen
  (`Bash`, MCP-Name) durch Deny-/Ask-Regeln und `PreToolUse`-Hooks; ein Bash-Befehl kann dort als
  `cd 'app' && git push` ankommen; der Code selbst steht in `tool_input.code` eines `REPL`-Aufrufs
  (CC-codemode „Permission checks“). Abweichung: Ein nicht fertig werdender `PreToolUse`-Hook führt
  in Code Mode zur Ablehnung, ein anders fehlschlagender Hook lässt den Aufruf laufen (CC-codemode
  „Code Mode limitations“).

### 1.2 Eingabe-JSON

Gemeinsame Felder (belegt, CC-hooks „Common input fields“): `session_id`, `prompt_id`,
`transcript_path`, `cwd` (aktuelles Arbeitsverzeichnis beim Aufruf), `scratchpad_dir` (ab 2.1.257),
`permission_mode` (`default`, `plan`, `acceptEdits`, `auto`, `dontAsk`, `bypassPermissions`),
`effort`, `hook_event_name`; in Subagenten zusätzlich `agent_id`, `agent_type`. `PreToolUse` dazu
`tool_name`, `tool_input`, `tool_use_id`; bei MCP-Werkzeugen `mcp_server` mit `name` und `source`
(`plugin`, `sdk`, `user`, `project` u. a.; ab 2.1.274). Ein Bash-`cd` wird über `CwdChanged`
gemeldet (kein Matcher, keine Entscheidung).

`tool_input` je Werkzeug:

| Werkzeug | Felder | Quelle |
|---|---|---|
| `Bash` | `command`, `description`, `timeout` (ms), `run_in_background`; zusätzlich der Parameter `dangerouslyDisableSandbox` (Sandbox-Ausweg, siehe 1.7) | CC-hooks „Bash“; CC-sandbox „The unsandboxed retry escape hatch“; CC-bin |
| `PowerShell` | wie Bash: `command`, `description`, `timeout`, `run_in_background` | CC-hooks „PowerShell“ |
| `Read` | `file_path` (immer absolut), `offset`, `limit` | CC-hooks „Read“ |
| `Write` | `file_path` (immer absolut), `content` | CC-hooks „Write“ |
| `Edit` | `file_path` (immer absolut), `old_string`, `new_string`, `replace_all` | CC-hooks „Edit“ |
| `NotebookEdit` | `notebook_path` („must be absolute“), `cell_id`, `new_source`, `cell_type` (`code`/`markdown`), `edit_mode` (`replace`/`insert`/`delete`) | CC-bin (Schema-Zeichenkette); in CC-hooks nicht beschrieben |
| `Glob` | `pattern`, `path` (optional, Standard = cwd) | CC-hooks „Glob“ |
| `Grep` | `pattern`, `path` (optional), `glob`, `output_mode`, `-i`, `multiline` | CC-hooks „Grep“ |
| `LSP` | `operation`, `filePath` („absolute or relative“), `line`, `character`, `query` | CC-bin; Regelformat `Read(...)` gilt für LSP (CC-tools) |
| `Agent` | `prompt`, `description`, `subagent_type`, `model` | CC-hooks „Agent“ |
| `WebFetch` | `url`, `prompt`, `offset` (ab 2.1.290) | CC-hooks „WebFetch“ |
| `REPL` (Code Mode) | `code` | CC-codemode |
| MCP-Werkzeug | dessen Argumente | CC-hooks „PreToolUse input“ |

Pfadnormalisierung (belegt, CC-hooks „PreToolUse input“): Für `Write`, `Edit`, `Read` expandiert
Claude Code `~` und relative Pfade vor dem Hook; unter Windows kommen Backslashes an. Für `Glob`,
`Grep`, `LSP` und `NotebookEdit` sagt die Doku das nicht (Unbelegt: ob `path`/`filePath` vor dem
Hook absolut gemacht wird; `LSP.filePath` darf laut Schema relativ sein).

`MultiEdit` und `LS`: in der Werkzeugtabelle von CC-tools nicht aufgeführt; CC-perm nennt
„legacy `MultiEdit`“ nur bei Regeln. Unbelegt: ob 2.1.332 eines der beiden als Werkzeug registriert
(in der ELF-Datei stehen beide Namen nur in Regel-/Prompttexten; eine Sitzung wurde nicht gestartet).
`Glob` und `Grep` fehlen unter macOS, Linux und WSL standardmäßig; Claude sucht dann mit `find` und
`grep` über das Bash-Werkzeug (eingebettete `bfs`/`ugrep`), und diese Suchen erreichen Hooks und
Regeln als `Bash`-Aufrufe (CC-tools „Glob tool behavior“). `Read` liest keine Ordner; Ordnerlisten
laufen über `ls` im Bash-Werkzeug (CC-tools „Read tool behavior“).

### 1.3 Ausgabe: Ablehnen und Eingabe ändern (belegt, CC-hooks „PreToolUse decision control“)

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny",
 "permissionDecisionReason":"<Text, den Claude liest>"}}
```

- `permissionDecision`: `allow`, `deny`, `ask`, `defer` (nur `-p`). Vorrang bei mehreren Hooks:
  `deny` > `defer` > `ask` > `allow`. Exit-Code 2 wirkt wie `deny`, stderr ist der Grund.
- `updatedInput` ersetzt das ganze Eingabeobjekt (unveränderte Felder mitgeben); Regeln werden gegen
  die geänderte Eingabe geprüft. Mit `allow` ohne Rückfrage, mit `ask` als Rückfrage. Ohne
  `permissionDecision` kann Auto-Mode den geänderten Aufruf ablehnen.
- `deny` gilt in jedem Modus, auch `bypassPermissions` und `--dangerously-skip-permissions`;
  `PreToolUse` läuft vor jeder Modusprüfung (CC-hooks-guide, Abschnitt zu `"deny"`). Deny- und
  Ask-Regeln werden unabhängig vom Hook-Ergebnis geprüft (CC-perm „Precedence among rules, hooks,
  and mods“).
- Veraltet: oberste Ebene `decision`/`reason` (`approve`→`allow`, `block`→`deny`).

### 1.4 Matcher (belegt, CC-hooks „Matcher patterns“, „Match MCP tools“)

- `"*"`, `""`, fehlend: alles. Nur Buchstaben, Ziffern, `_`, `-`, Leerzeichen, `,`, `|`: exakte
  Liste. Jedes andere Zeichen: JavaScript-RegExp, nicht verankert (`Edit.*` trifft auch
  `NotebookEdit`; für ganze Namen `^…$`).
- MCP: `mcp__<server>__<tool>`; ganzer Server nur mit `.*` (`mcp__memory` trifft nichts).
  Plugin-Server: `mcp__plugin_<plugin>_<server>__<tool>`.
- Feld `if` je Handler: genau eine Permission-Regel (`Bash(git *)`, `Edit(*.ts)`); für Bash
  „best-effort“; die Doku empfiehlt für harte Sperren das Permission-System statt eines Hooks.
- Hooks mit `if` laufen nur bei Werkzeugereignissen und `PreSkillShellCommand`.

### 1.5 Geltung in Subagenten, für fremde MCP-Server, aus Plugins

- Subagenten (belegt, CC-hooks „Hook locations“): Hooks aus Settings, Managed Settings und Plugins
  laufen auch in Subagenten, die Eingabe trägt `agent_id`/`agent_type`. Subagenten und deren
  Bash-Befehle nutzen dieselbe Sandbox-Konfiguration (CC-sandbox „Scope“).
- MCP-Werkzeuge anderer Server (belegt, CC-hooks „Match MCP tools“): erscheinen als normale
  Werkzeuge in `PreToolUse`; Vertrauensentscheidungen sollen auf `mcp_server.source` beruhen, nicht
  auf dem Namen.
- Plugins (belegt, CC-plugin): `hooks/hooks.json` im Plugin-Ordner (oberster Schlüssel `"hooks"`)
  und/oder Manifestfeld `hooks` (Pfad, Inline-Objekt oder Array); beide werden geladen. Plugin-Hooks
  werden beim Laden des Plugins registriert, nicht erst bei Skill-Nutzung. Hook-Prozesse erhalten
  `CLAUDE_PLUGIN_ROOT`, `CLAUDE_PLUGIN_DATA`, `CLAUDE_PLUGIN_OPTION_<KEY>`. Bei zwei gleichnamigen
  aktiven Plugins registriert nur eines seine `hooks/hooks.json`.

### 1.6 Was Hooks abschaltet oder umgeht

Belegt:

- `disableAllHooks: true` in einer beliebigen Settings-Datei schaltet Nutzer-, Projekt-, lokale und
  Plugin-Hooks ab; nur Managed-Hooks, SDK-Hooks und Hooks von per Managed `enabledPlugins`
  erzwungenen Plugins bleiben (CC-settings `disableAllHooks`). Es gilt der Wert nach Settings-Vorrang:
  ein `false` in `.claude/settings.json` des Projekts überstimmt ein `true` im Nutzer-Settings; für
  einen Lauf: `--settings '{"disableAllHooks": true}'` (CC-hooks „Disable or remove hooks“).
- `allowManagedHooksOnly: true` (nur Managed): blockiert Nutzer-, Projekt-, lokale und Plugin-Hooks,
  außer Plugins, die Managed Settings per `enabledPlugins` erzwingen (CC-settings). Ebenso
  `strictPluginOnlyCustomization` (Managed) für Settings-Hooks (CC-hooks „Limits on skill hooks“).
- `--bare` überspringt Hooks aus Settings und installierten Plugins; `--safe-mode` deaktiviert
  installierte Plugins, Hooks, MCP-Server; Managed-Hooks bleiben (CC-bin `claude --help`).
- `--restricted` ignoriert Nutzer-, Projekt- und lokale Settings-Dateien (CC-bin `claude --help`).
  Unbelegt: ob dabei auch Plugin-Hooks entfallen.
- `bypassPermissions` / `--dangerously-skip-permissions` schalten `PreToolUse`-`deny` **nicht** ab
  (1.3). Sie erlauben aber Schreibzugriffe auf geschützte Pfade ohne Rückfrage, darunter
  `.claude`-Settings (CC-modes „Protected paths“, Zeile `bypassPermissions | Allowed`) und den
  Sandbox-Ausweg ohne Rückfrage (1.7). Folgerung: Der Agent kann in diesem Modus über `Edit`/`Write`
  oder Bash die Settings ändern, die die Sperre tragen, sofern ein Hook das nicht ablehnt.
- Mods: Ein `tool.call`-Hook eines installierten Mods, der ohne `next` antwortet, verhindert, dass
  `PreToolUse`-Hooks aus Settings und Plugin-`hooks/hooks.json` laufen; nur `PreToolUse`-Hooks aus
  Managed Settings laufen vor allen Mods (CC-mods „Where settings hooks run in the order“). Ohne
  Managed Settings, `managed-mcp.json` oder Team-/Enterprise-Anmeldung kann ein Mod auch einen
  Aufruf genehmigen, den ein `PreToolUse`-Hook blockiert hat (CC-perm „Precedence among rules, hooks,
  and mods“).
- Startfehler oder Timeout: siehe 1.8.

### 1.7 Sandbox

Belegt (CC-sandbox):

- Gilt nur für Bash-, PowerShell- und Monitor-Befehle und deren Kindprozesse. Nicht für `Read`,
  `Edit`, `Write`, `WebFetch`, `WebSearch` (diese folgen Permission-Regeln), nicht für Hooks,
  lokale MCP-Server, Plugin-Monitore, LSP-Server, Statuszeile, Mods.
- Plattformen: macOS (Seatbelt), Linux und WSL2 (bubblewrap + socat; Seccomp-Filter optional für
  Unix-Socket-Sperre). **Native Windows: keine Sandbox**, Befehle laufen ungesandboxt.
- Ubuntu: Die AppArmor-Beschränkung unprivilegierter User-Namespaces verhindert den Start; Abhilfe
  per AppArmor-Profil für `/usr/bin/bwrap` (sudo) oder `sysctl
  kernel.apparmor_restrict_unprivileged_userns=0` (sudo, bis Neustart). Ohne Abhilfe laufen Befehle
  ungesandboxt, außer `sandbox.failIfUnavailable: true` (dann startet Claude Code nicht).
- Einstellungen: `sandbox.enabled`, `sandbox.filesystem.denyRead`, `allowRead`, `denyWrite`,
  `allowWrite`, `disabled`, `sandbox.failIfUnavailable`, `sandbox.allowUnsandboxedCommands`,
  `sandbox.excludedCommands`, `sandbox.autoAllowBashIfSandboxed`,
  `permissions.blockReadsOutsideWorkingDirectories`. Pfade: `/abs`, `~/…`, relativ zum
  Settings-Ort (anders als `//abs` bei Permission-Regeln).
- `allowUnsandboxedCommands` (Standard `true`): erlaubt Claude die Wiederholung mit
  `dangerouslyDisableSandbox`; in `bypassPermissions` ohne Rückfrage, in Manual/acceptEdits mit
  Rückfrage, in `dontAsk` abgelehnt. `false` ignoriert den Parameter („Strict sandbox mode“).
  Ein `false` im Nutzer-Settings, in `--settings` oder Managed hält gegen ein Projekt-`true`
  (ab 2.1.285).
- `excludedCommands`: Muster wie `Bash(...)`-Regeln; alle Teilbefehle müssen passen; Aufrufe mit
  `cd`, Umleitung, `$(...)`, `sudo`, `eval`, `xargs` bleiben gesandboxt.
- Settings-Orte: jede Datei (`any file`); Arrays werden über alle Scopes zusammengeführt. Ist die
  Sandbox „admin-required“ (Managed oder `--settings` mit `allowUnsandboxedCommands: false`), ignoriert
  Claude Code lockernde Einträge in `.claude/settings.json`/`.claude/settings.local.json`.
- Managed-Orte: `/etc/claude-code/managed-settings.json` (+ `managed-settings.d/`) unter Linux/WSL,
  `/Library/Application Support/ClaudeCode/` unter macOS, `C:\Program Files\ClaudeCode\` und
  `HKLM\SOFTWARE\Policies\ClaudeCode` unter Windows; `HKCU\SOFTWARE\Policies\ClaudeCode` ist
  nutzerschreibbar und gilt nur, wenn keine Admin-Quelle vorhanden ist (CC-managed). Folgerung: se
  kann Managed Settings unter Linux/macOS nur mit Root/Admin schreiben.

`Read(...)`-Deny und Bash in der Sandbox (belegt, CC-settings `sandbox.filesystem.denyRead`):
„Claude Code … adds the paths from your `Read(...)` deny permission rules“ – eine `Read`-Deny-Regel
wird bei aktiver Sandbox zum OS-erzwungenen Lesesperrpfad für Bash. Ohne Sandbox wirkt sie auf
Bash nur, wo Claude Code den Befehl analysiert (Umleitungsziele, `cat`, `head`, `tail`, `sed`,
`tee`), nicht auf `grep -r pattern .` aus dem Elternordner und nicht auf Skripte, die Dateien selbst
öffnen (CC-perm, Warnung unter „What `Read` and `Edit` rules cover“).

Plugin und Sandbox: Die Manifest-Feldtabelle (CC-plugin „Fields“) enthält kein Feld für Settings,
Sandbox oder Permission-Regeln; das Feld `network` „doesn't allow or block any host“. Folgerung:
Ein Plugin kann die Sandbox nicht setzen; se müsste `~/.claude/settings.json`, Projekt-Settings
oder (mit Admin) Managed Settings schreiben oder Claude mit `--settings` starten.
**Widerspruch:** `docs/refs/claude-code-plugins-mcp-2026-10-12.md` §1/§5 nennt ein Manifestfeld
`settings` (wirksam nur für `agent`, `subagentStatusLine`); die Feldtabelle vom 2026-11-28 nennt es
nicht. Unbelegt: ob 2.1.332 ein Plugin-`settings` noch liest.

### 1.8 Exit-Code und Timeout (belegt, CC-hooks „Exit code output“, „Timeouts“, „Block the action when a hook fails“)

- Exit 0: JSON-Felder gelten. Exit 2: blockierender Fehler, stderr ist der Grund. Jeder andere Code,
  Startfehler, ungültiges JSON: **nicht blockierend**, der Aufruf läuft weiter.
- Timeout Standard 600 s (`command`, `http`, `mcp_tool`); ein `PreToolUse`-Hook im Timeout blockiert
  **nicht**, die Ausgabe wird verworfen.
- `"onFailure": "block"` (ab 2.1.295; bei `mcp_tool` ab 2.1.328): Startfehler, Exit ≠ 0/2,
  HTTP-Fehler, MCP-Fehler, Timeout und ungültige Ausgabe blockieren den Aufruf. Unwirksam bei
  `async`/`asyncRewake`.
- Ein Hook, der nach einer Entscheidung einen Hintergrundprozess ohne umgeleitete Ausgabe startet,
  führt zur Blockade („stdio went quiet“).

### 1.9 Eingebaute Pfadsperre über Permission-Regeln

Belegt (CC-perm „Read and Edit“, „How path patterns match“, „Symlinks“, „Settings precedence“):

- `Read(//abs/pfad/**)` deny sperrt `Read`, `Grep`, `Glob`, `LSP` (best effort für Grep/Glob),
  @-Erwähnungen, eingefügte Bilder, Ordnerlisten und IDE-Kontext; zusätzlich `Edit` und `Write`
  (Write ab 2.1.228). `NotebookEdit` wird davon nicht erfasst → zusätzlich `Edit(//abs/pfad/**)`
  deny. Regeln `Write(...)`, `Glob(...)`, `Grep(...)` mit Pfad werden angenommen, aber nie gematcht.
- `//pfad` = absolut ab Dateisystemwurzel; `/pfad` = relativ zum Settings-Ort; Windows wird zu
  POSIX normalisiert (`C:\x` → `//c/x`).
- Symlinks: Deny trifft, wenn angefragter Pfad **oder** Ziel passt; gilt auch für Junctions.
- Bash: siehe 1.7 (nur mit Sandbox OS-erzwungen).
- MCP-Werkzeuge: `Read`/`Edit`-Regeln gelten für eingebaute Datei-Werkzeuge; ein fremder
  Dateisystem-MCP-Server wird davon nicht erfasst (Folgerung aus „apply to Claude's built-in file
  tools“); Sperre dort nur per `mcp__…`-Regel oder Hook.
- Orte: jede Settings-Datei (Nutzer, Projekt, lokal, Managed, `--settings`, CLI-Flags); Deny aus
  irgendeiner Ebene kann keine andere Ebene erlauben. `allowManagedPermissionRulesOnly` (Managed)
  macht Managed zur einzigen Regelquelle.
- `permissions.blockWritesOutsideWorkingDirectories` (ab 2.1.314) beschränkt nur Datei-Werkzeuge,
  nicht Shell, MCP oder Hook-Programme (CC-modes).

## 2. Codex CLI 0.166.0

### 2.1 Hooks vor Werkzeugaufrufen

Belegt (CX-hooks; CX-src `codex-rs/hooks/schema/generated/*.json`,
`hooks/src/events/pre_tool_use.rs`, `hooks/src/engine/output_parser.rs`):

- Ereignisse: `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`,
  `UserPromptSubmit`, `SubagentStart`, `SubagentStop`, `Stop`, `PreModelRequest`,
  `PostModelRequest`, `Interrupt`, `SessionStart`, `SessionEnd`. Feature `hooks` ist Standard an
  (`features/src/lib.rs`: `CodexHooks`, `default_enabled: true`); abschalten mit
  `[features] hooks = false`.
- Eingabe `PreToolUse` (Schema `pre-tool-use.command.input.schema.json`, Pflicht):
  `session_id`, `turn_id`, `transcript_path`, `cwd`, `hook_event_name`, `model`, `permission_mode`
  (`default|acceptEdits|plan|dontAsk|bypassPermissions`), `tool_name`, `tool_input`,
  `tool_use_id`; optional `agent_id`, `agent_type`.
- Ausgabe: `hookSpecificOutput.permissionDecision: "deny"` **mit nichtleerem**
  `permissionDecisionReason` (ohne Grund gilt die Ausgabe als ungültig und der Aufruf läuft), oder
  alt `{"decision":"block","reason":"…"}`, oder Exit 2 mit nichtleerem stderr. Umschreiben nur mit
  `permissionDecision: "allow"` plus `updatedInput` (bei `Bash`/`apply_patch` mit String `command`).
  `allow` ohne `updatedInput`, `ask`, `decision: "approve"`, `continue: false`, `stopReason`,
  `suppressOutput`: als Fehler gewertet, **der Aufruf läuft weiter**.
- Exit-Code ≠ 0/2, fehlender Status, Fehler: Status `Failed`, keine Blockade (fail-open). Timeout
  Standard 600 s; CX-hooks: „A `PreToolUse` error, timeout, or malformed response can let the tool
  run.“ Unbelegt: ein `onFailure: block`-Gegenstück; im Schema gibt es keines.
- Matcher: `""`/`"*"`/fehlend = alles; nur `[A-Za-z0-9_|]` = exakte Liste; sonst Rust-Regex,
  nicht verankert (`hooks/src/engine/matcher.rs`).
- Konfiguration: `hooks.json` oder `[hooks]` in `config.toml` in `/etc/codex/`,
  `%ProgramData%\OpenAI\Codex\`, `$CODEX_HOME/` (Standard `~/.codex/`) und `.codex/` vertrauter
  Projekte; TOML-Form `[[hooks.PreToolUse]] matcher = "…"` mit `[[hooks.PreToolUse.hooks]] type =
  "command" command = "…" timeout = 30`; Handler `command` (mit `commandWindows`) und `mcp_tool`
  werden ausgeführt, `prompt`/`agent` übersprungen.
- **Vertrauen:** Nicht verwaltete Hooks (Nutzer, Projekt, Plugin) laufen nur, wenn der Nutzer die
  exakte Definition in `/hooks` vertraut hat; Codex speichert einen Hash
  (`[hooks.state."<schlüssel>"] trusted_hash`, `config/src/hook_config.rs`;
  `hook_trust_status` in `hooks/src/engine/discovery.rs`); geänderte oder neue Hooks werden
  übersprungen. Bei Plugins verlangt eine geänderte Skriptdatei eine neue Prüfung.
  `--dangerously-bypass-hook-trust` hebt das für einen Aufruf auf. Managed Hooks
  (`requirements.toml` `[hooks]`, System-, MDM-, Cloud-Quellen) gelten als vertraut;
  `allow_managed_hooks_only = true` ignoriert Nutzer-, Projekt-, Sitzungs- und Plugin-Hooks.
  Folgerung: Ein von se geschriebener Hook läuft erst nach einer Nutzerbestätigung in `/hooks`,
  außer se schreibt den Vertrauens-Hash selbst; das umginge die von Codex vorgesehene
  Nutzerprüfung (Unbelegt: ob Codex einen fremd geschriebenen `trusted_hash` akzeptiert, ist nur
  aus dem Vergleich `trusted_hash == current_hash` abgeleitet, nicht beobachtet).
- Plugins: `hooks/hooks.json` im Plugin-Ordner oder Manifestfeld `hooks` in
  `.codex-plugin/plugin.json` (`core-plugins/src/loader.rs`, `manifest.rs`); Umgebungsvariablen
  `PLUGIN_ROOT`, `PLUGIN_DATA`, `CLAUDE_PLUGIN_ROOT`, `CLAUDE_PLUGIN_DATA`.

### 2.2 Werkzeugnamen und Hook-Abdeckung (belegt, CX-src `core/src/tools/`, CX-hooks „Tool coverage“)

| Werkzeug (Modellname) | Hook-`tool_name` | `tool_input` | PreToolUse |
|---|---|---|---|
| `exec_command` (Unified Exec) | `Bash` | `{"command": <cmd>}`; `cwd` aus `workdir` | ja |
| `write_stdin` | — | — | **nein**: „transport for an existing exec session“ (`handlers/unified_exec/write_stdin.rs`) |
| `apply_patch` | `apply_patch` (Matcher-Aliase `Write`, `Edit`) | `{"command": <patch>}` | ja |
| `view_image` | `view_image` | Argumente (`path`) | ja (Standardpfad `registry.rs` `pre_tool_use_payload`) |
| MCP-Werkzeuge | `mcp__<server>__<tool>` | Argumente | ja |
| `spawn_agent` | `spawn_agent` (Alias `Agent`) | Argumente | ja |
| übrige lokale Funktionen (z. B. `update_plan`) | Funktionsname | Argumente | ja |
| gehostete Werkzeuge (`WebSearch`) | — | — | nein |
| Code Mode (verschachtelte Aufrufe) | Name des Zielwerkzeugs | Argumente | ja (CX-hooks „Tool calls from code mode“) |

In 0.166.0 gibt es keine Handler `read_file`, `list_dir`, `grep_files`, `shell`; registriert sind
u. a. `ExecCommandHandler`, `WriteStdinHandler`, `ApplyPatchHandler`, `ViewImageHandler`,
`CodeModeExecuteHandler`, MCP-Handler (`core/src/tools/spec_plan.rs`). Lesen geschieht über die
Shell. CX-hooks: „Treat tool hooks as a useful guardrail, not a complete enforcement boundary.“
Folgerung: `exec_command` mit einer interaktiven Shell und danach Befehle über `write_stdin`
erreichen keinen `PreToolUse`-Hook; ein Hook allein ist in Codex keine Vollsperre.

### 2.3 Sandbox: Lesesperre bestimmter Pfade

Belegt (CX-perm, CX-win, CX-src `linux-sandbox/README.md`):

- Permission-Profile (Beta): `default_permissions = "<name>"`, `[permissions.<name>]`,
  `extends = ":workspace"`, `[permissions.<name>.filesystem]` mit `"<pfad>" = "read"|"write"|"deny"`;
  `deny` sperrt Lesen und Schreiben und gewinnt gegen gleich spezifisches `read`/`write`.
  Nicht kombinierbar mit `sandbox_mode`/`[sandbox_workspace_write]`; steht `sandbox_mode` in
  irgendeiner geladenen Datei oder wird `--sandbox` übergeben, gelten die alten Einstellungen.
- Linux/WSL2: bubblewrap + Seccomp, Standardansicht `--ro-bind / /`; Deny-Pfade werden maskiert;
  Glob-Denies werden beim Start jedes gesandboxten Prozesses aufgelöst (später erzeugte Treffer bleiben
  für diesen Prozess lesbar). Landlock-Altmodus wird für dateisystembeschränkte Profile abgelehnt.
  Fehlt `bwrap`, nutzt Codex ein mitgeliefertes `codex-resources/bwrap`; kann bubblewrap keine
  User-Namespaces anlegen, gibt Codex eine Startwarnung aus. Unbelegt: ob Codex dann Befehle
  ablehnt oder ungesandboxt ausführt.
- macOS: Seatbelt; nicht durchsetzbare Richtlinie → Befehl wird verweigert.
- Windows: `mxc` (Windows 11 24H2 Build 26100.9278 / 25H2 Build 26200.9278, ohne Admin),
  `elevated` (eigene Sandbox-Benutzer, Admin-Einrichtung nötig), `unelevated` (eingeschränktes Token
  des Nutzers; **unterstützt keine Lesesperrpfade**, CX-win).
- Abdeckung: Profile regeln lokale Befehle; MCP-Server, Konnektoren, Browser, Computer Use und
  „approved escalations“ haben eigene Kontrollen (CX-perm „What profiles control“). `view_image` und
  `apply_patch` nutzen den Dateisystem-Sandbox-Kontext (`handlers/view_image.rs`,
  `runtimes/apply_patch.rs`). MCP-Server laufen außerhalb der Befehls-Sandbox
  (`docs/refs/codex-mcp-spec-rmcp-2026-10-12.md` §1).
- Ausweg: Der Nutzer kann im Permissions-Menü **Full access** wählen (CX-sandbox-Seite
  `https://learn.chatgpt.com/docs/sandboxing.md`, abgerufen 2026-11-28); Freigaben für Läufe mit
  mehr Rechten steuert die Approval-Policy. Unbelegt: welche `requirements.toml`-Schlüssel
  `danger-full-access` und Eskalationen für Nutzer ohne Admin sperren (nicht gelesen).

### 2.4 Plugins und Konfiguration

- Codex-Plugin-Manifest kennt `name`, `version`, `description`, `keywords`, `skills`,
  `mcpServers`, `apps`, `hooks`, `interface`, `extensions` (`core-plugins/src/manifest.rs`
  `RawPluginManifest`) – **keine** Sandbox- oder Permission-Felder. Hooks ja (2.1), mit
  Vertrauenspflicht.
- `~/.codex/config.toml` (Nutzer) und `.codex/config.toml` vertrauter Projekte nehmen `[hooks]`,
  `default_permissions`, `[permissions.<name>]` auf; `/etc/codex/` und `requirements.toml` für
  Systemverwaltung.

## 3. Betriebssystem-Mittel ohne Root/Admin des Aufrufers

### 3.1 Linux, Ubuntu 24.04

Beobachtet (2026-11-28):

| Eingabe | Ergebnis |
|---|---|
| `sysctl kernel.apparmor_restrict_unprivileged_userns` | `= 1` |
| `cat /sys/kernel/security/lsm` | `lockdown,capability,landlock,yama,apparmor` |
| `unshare -Urm sh -c 'mount -t tmpfs none <ordner>'` | `unshare: write failed /proc/self/uid_map: Operation not permitted`, Exit 1 |
| `bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs <ordner> cat <ordner>/x` (bubblewrap 0.9.0, nicht setuid) | `bwrap: setting up uid map: Permission denied`, Exit 1 |
| `landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION)` (Syscall 444) | `4` (Landlock-ABI 4) |
| `cat /proc/sys/kernel/yama/ptrace_scope` | `1` |
| `ls /etc/apparmor.d/` (Auswahl) | Profile `unprivileged_userns`, `lxc-unshare`, `lxc-usernsexec`; **kein** Profil für `bwrap` |

- User-Namespaces (UBU, belegt): Unprivilegierte Prozesse dürfen User-Namespaces nur anlegen, wenn
  sie per AppArmor-Profil mit `userns,` eingeschlossen sind oder `CAP_SYS_ADMIN` haben; Freigabe
  einzelner Programme über ein Profil `flags=(default_allow)` mit `userns,`. Folgerung aus Beobachtung
  und UBU: bubblewrap und `unshare` mit Mount-Namespace sind auf diesem Ubuntu 24.04 ohne ein von
  Root installiertes AppArmor-Profil nicht nutzbar. Dasselbe gilt für die Sandbox von Claude Code
  (CC-sandbox) und von Codex (CX-src README: Startwarnung).
- Landlock (LL, belegt): für unprivilegierte Prozesse; Regeln sind Erlaubnislisten: behandelte
  Rechte sind verboten, solange keine Regel sie erlaubt; eine einzelne Pfadsperre geht nur, indem
  alle übrigen benötigten Pfade erlaubt werden (Folgerung aus dem Erlaubnislistenmodell; ein
  Unterpfad eines erlaubten Baums kann nicht gesondert verboten werden, weil Regeln nur gewähren).
  Vererbung an alle Kinder, nicht aufhebbar. Grenzen: vor der Einschränkung geöffnete
  Dateideskriptoren bleiben nutzbar; `/proc/<pid>/fd/*` nicht nutzersichtbarer Dateisysteme nicht
  einschränkbar; Verbindungen zu Pfad-Unix-Sockets erst ab ABI 9
  (`LANDLOCK_ACCESS_FS_RESOLVE_UNIX`) einschränkbar, abstrakte Sockets ab ABI 6. Folgerung: Mit ABI 4
  (Kernel 6.8) kann ein Landlock-Prozess den D-Bus-Socket der Sitzung erreichen und über
  `systemd-run --user` einen Prozess außerhalb der Landlock-Domäne starten.
- seccomp mit `PR_SET_NO_NEW_PRIVS` (Codex nutzt es nach bwrap, CX-src README): `no_new_privs`
  verhindert setuid; sudo meldet dann „The “no new privileges” flag is set, which prevents sudo from
  running as root“ (SUDO, DIAGNOSTICS). Folgerung: Ein in Landlock/seccomp eingeschlossener
  Agentenprozess kann `sudo`/`pkexec` nicht nutzen; die erhöhte Ausführung muss außerhalb erfolgen.

### 3.2 Windows

Belegt:

- `CreateRestrictedToken` braucht laut Doku kein Privileg; Restricting-SIDs: das System prüft zweimal
  (aktivierte SIDs und Restricting-SIDs), Zugriff nur, wenn beide Prüfungen erlauben (MS-rt).
  `WRITE_RESTRICTED`: Restricting-SIDs zählen nur bei Schreibzugriff (MS-crt). `DISABLE_MAX_PRIVILEGE`
  entfernt alle Privilegien außer `SeChangeNotifyPrivilege`; `LUA_TOKEN` = LUA-Token (MS-crt).
- `CreateProcessAsUser` mit eingeschränkter Version des **eigenen** Primärtokens braucht kein
  `SE_ASSIGNPRIMARYTOKEN_NAME`; „This enables ordinary applications to create restricted processes.“
  (MS-rt, MS-cpau). `SE_INCREASE_QUOTA_NAME` wird laut MS-cpau „typically“ benötigt und bei Bedarf
  für den Aufruf aktiviert. Unbelegt: ob ein Standardnutzer ohne Admin `SE_INCREASE_QUOTA_NAME`
  im Token hat (keine Windows-Beobachtung).
- Kompatibilität: MS-rt empfiehlt, eingeschränkte Prozesse auf einem eigenen Desktop laufen zu
  lassen, sonst können sie per `SendMessage`/`PostMessage` uneingeschränkte Fenster steuern.
  `CreateProcessAsUser` lädt kein Nutzerprofil und übernimmt die Umgebung des Aufrufers (MS-cpau).
- Deny-ACE auf Ordner für eine eigene SID: Codex `unelevated` nutzt genau
  `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED)` mit Restricting-SIDs
  in der Reihenfolge Capability-SIDs, weitere, Logon-SID, Everyone, setzt die Default-DACL auf die
  Logon-SID und startet mit `CreateProcessAsUserW` (CX-src `windows-sandbox-rs/src/token.rs`,
  `process.rs`). Deny-Read-ACEs setzt Codex über `apply_deny_read_acls(paths, psid)` auf den
  lexikalischen und den kanonischen Pfad, legt fehlende Pfade als Ordner an, verweigert die
  Laufwerkswurzel und widerruft bei Fehlern (CX-src `deny_read_acl.rs`). CX-win: `unelevated`
  „doesn't support denied read paths“ – passend zu `WRITE_RESTRICTED`, das Restricting-SIDs nur beim
  Schreiben prüft. Folgerung: Für eine Lesesperre müsste das Token **ohne** `WRITE_RESTRICTED`
  erzeugt werden; dann müssen alle Objekte, die der Prozess lesen muss (Systemdateien,
  Programmordner, Registry, Pipes), einer Restricting-SID Zugriff gewähren. Codex nimmt dafür
  Everyone und Logon-SID in die Restricting-Liste. Unbelegt: welche üblichen Werkzeuge (git, Node,
  Python, PowerShell) mit einem nicht schreibbeschränkten Restricted Token ohne weitere ACL-Änderung
  funktionieren.
- `elevated`: eigene niedrig privilegierte Sandbox-Benutzer, Dateisystemgrenzen, Firewallregeln,
  Admin-Einrichtung (einmal, auch durch IT: `codex sandbox setup --elevated --user …`) (CX-win).
- AppContainer (MS-ac): Isolation von Dateien, Registry, Netzwerk, Prozessen, Fenstern;
  Lese-/Schreibzugriff wird je Datei/Schlüssel gewährt, „Read-only access is less restricted“.
  Unbelegt in MS-ac: ob Anlegen und Start ohne Admin möglich ist und wie sich LPAC unterscheidet
  (die Seite nennt LPAC nicht).
- Integritätsstufen (MS-mic): Labels `NO_WRITE_UP`, `NO_READ_UP`, `NO_EXECUTE_UP` im
  Mandatory-Label-ACE der SACL; Standard ist `NO_WRITE_UP`; Standardnutzer laufen mit Medium.
  Folgerung: Ein Ordner mit Medium-Label + `NO_READ_UP` und Agentenprozesse mit Low-Integrity
  wären lesegesperrt; Low-Prozesse können aber ohnehin nicht in Medium-Objekte schreiben, also in
  fast keinen Nutzerordner. Unbelegt: ob ein Medium-Prozess `NO_READ_UP` auf eigene Dateien setzen
  darf und ob `NO_READ_UP` für Dateiobjekte durchgesetzt wird.
- MXC (CX-win): neue Windows-11-Prozessisolation ohne Admin, Verfügbarkeit ab den genannten Builds.
  Unbelegt: öffentliche Microsoft-API-Dokumentation zu MXC (nicht gefunden/gelesen).

### 3.3 Ausbruchswege

- D-Bus/systemd (BWRAP, belegt): „if you bind a D-Bus socket into the sandbox, it can be used to
  execute commands via systemd“; Filter: `xdg-dbus-proxy`. `TIOCSTI` ohne Seccomp-Filter: ohne
  `--new-session` Befehlsausführung außerhalb (CVE-2017-5226). `systemd-run --user` spricht den
  Dienstmanager des Nutzers an (`man systemd-run`), der außerhalb jeder Sandbox läuft (Folgerung).
- Landlock: Pfad-Unix-Sockets vor ABI 9 (3.1).
- Laufende Fremdprozesse: Yama `ptrace_scope = 1` (beobachtet) erlaubt ptrace nur auf Nachfahren.
  Unbelegt: Primärquelle zur Yama-Semantik (kernel.org-Abruf scheiterte mit HTTP 429).
  Prozesse desselben Nutzers außerhalb der Sandbox (Editor, Terminal, IDE) können Dateien
  weiterhin lesen und Befehle ausführen, wenn der Agent sie erreichen kann (z. B. über
  IDE-Erweiterungen, Sockets, Fenster) (Folgerung).
- Windows-Aufgabenplanung, COM/RPC-Dienste, Fensternachrichten: Fensternachrichten belegt (MS-rt);
  Unbelegt: ob ein Prozess mit Restricted Token oder in AppContainer über `schtasks`/Task-Scheduler-COM
  eine Aufgabe als voller Nutzer registrieren kann.
- Agenten-eigene Auswege: Claude Code `dangerouslyDisableSandbox` (1.7), `excludedCommands`,
  Änderung der Settings in `bypassPermissions` (1.6), Mods (1.6); Codex `write_stdin` (2.2),
  „Full access“ (2.3), fail-open Hooks (2.1).

### 3.4 Schutz auch gegen Programme mit denselben Benutzerrechten

- Unter Linux/macOS/Windows schützt eine Lesesperre, die nur auf den Prozessbaum des Agenten wirkt
  (Namespace, Landlock, Seatbelt, Restricted Token), nicht gegen andere Programme desselben Nutzers
  außerhalb dieses Baums (Folgerung aus 3.1–3.3). Belegter Gegenentwurf: Codex `elevated` lässt die
  Agentenbefehle als **eigene Sandbox-Benutzer** laufen und setzt Dateisystemgrenzen für diese
  Konten; Einrichtung braucht einmal Admin (CX-win). Das schränkt nur die Agentenbefehle ein, nicht
  die übrigen Programme des Nutzers.
- Besitz/ACL an ein Dienstkonto (Dateien gehören einem se-Dienstkonto, der Nutzer hat keinen
  Zugriff) oder Verschlüsselung mit einem Schlüssel, den nur ein se-Dienst hält: Unbelegt (keine
  Primärquelle gelesen). Folgerung zu den Kosten: Editor, IDE, git, Build-Werkzeuge und Dateimanager
  des Nutzers könnten die maskierten Pfade dann ebenfalls nur über se erreichen; ein Nutzer mit
  Admin-/sudo-Recht kann Besitz und ACL jederzeit zurückholen. Microsoft: „Same-desktop Elevation
  in UAC isn't a security boundary“ (zitiert in GSUDO `docs/docs/security.md`, Microsoft-Seite
  `learn.microsoft.com/…/disable-user-account-control` nicht selbst abgerufen).

## 4. Erhöhte Ausführung mit Ausgabe an einen nicht erhöhten Prozess

### 4.1 Windows: UAC über ShellExecuteEx „runas“ und Rückkanal

- `runas`: „Launches an application as Administrator. User Account Control (UAC) will prompt …“
  (MS-sei). `SHELLEXECUTEINFOW` hat die Felder `cbSize`, `fMask`, `hwnd`, `lpVerb`, `lpFile`,
  `lpParameters`, `lpDirectory`, `nShow`, `hInstApp`, `lpIDList`, `lpClass`, `hkeyClass`, `dwHotKey`,
  `hIcon`/`hMonitor`, `hProcess` – **kein** Feld für Standard-Handles (MS-sei); mit
  `SEE_MASK_NOCLOSEPROCESS` liefert es `hProcess`. Folgerung: stdin/stdout/stderr des erhöhten
  Prozesses müssen über einen eigenen Kanal (Named Pipe) zurück.
- gsudo (GSUDO, belegt):
  - Start: `ProcessStartInfo { UseShellExecute = true, Verb = "runas" }`; Win32-Fehler 1223 =
    Nutzer hat abgebrochen (`Helpers/ProcessFactory.cs`).
  - Pipe-Name: SHA-256 über `allowedSid`, `targetSid`, `allowedPid`, Flags und Hash des eigenen
    Programmpfads; Präfix `ProtectedPrefix\Administrators\gsudo` für erhöhte Server, sonst `gsudo`
    (`Rpc/NamedPipeNameFactory.cs`).
  - Pipe-ACL: Allow ReadWrite für die aufrufende SID, Allow ReadWrite + CreateNewInstance für die
    erhöhte eigene SID, **Deny FullControl für NETWORK (S-1-5-2)**; zwei Pipes (Daten, `_control`),
    Nachrichtenmodus, max. 20 Instanzen (`Rpc/NamedPipeServer.cs` `Listen`).
  - Gegenstellenprüfung: `GetNamedPipeClientProcessId`; erhöhter Server prüft Pfad, Länge und
    Änderungszeit der Client-EXE (ohne Hash, „too slow“), lehnt bei angehängtem Debugger ab und
    akzeptiert nur die erlaubte PID oder deren Nachfahren; der Quelltext nennt die Elternprüfung
    selbst umgehbar („It's trivial on Windows to fake a your parent PID“). Läuft der Server nur mit
    Medium-Integrität, wird jede Verbindung angenommen („not much to protect“)
    (`Rpc/NamedPipeServer.cs` `IsAuthorized`).
  - Zwischenspeicher: `CacheMode` Standard `Explicit`, `CacheDuration` Standard 300 s Leerlauf
    (`AppSettings/Settings.cs`); Sitzung endet, wenn der erlaubte Prozess endet
    (`CancelIfAllowedProcessEnds`). Risiko laut gsudo: ein aktiver Cache erlaubt stille Elevation
    durch andere Medium-Prozesse; gleiche Konsole erlaubt Tastatureingaben in den erhöhten Prozess
    (`docs/docs/security.md`).
- Pipe-Anforderungen (MS-pipe, belegt): Standard-ACL einer Pipe gewährt LocalSystem, Administratoren
  und Ersteller Vollzugriff und **Everyone sowie Anonymous Lesezugriff** → explizite ACL nötig.
  `FILE_FLAG_FIRST_PIPE_INSTANCE`: zweite Instanz scheitert mit `ERROR_ACCESS_DENIED` (gegen
  vorab angelegte fremde Pipes). `PIPE_REJECT_REMOTE_CLIENTS`: Remote-Clients werden abgewiesen.
  Unbelegt: Microsoft-Dokumentation zum Namensraum `ProtectedPrefix\Administrators\` (nur aus GSUDO).

### 4.2 Linux

- pkexec (PKEXEC, belegt): führt `PROGRAM` als root aus; Exit = Exit von PROGRAM; 127 bei fehlender
  Berechtigung/Fehler, 126 bei abgelehntem Dialog. Nutzt den für Prozess/Sitzung registrierten
  Authentifizierungsagenten; ohne Agent ein eigener Textagent, abschaltbar mit
  `--disable-internal-agent`. Minimale Umgebung, `PKEXEC_UID` gesetzt, kein `DISPLAY`/`XAUTHORITY`;
  Arbeitsordner = Home des Zielnutzers, außer `--keep-cwd`. Aktion
  `org.freedesktop.policykit.exec` auf Ubuntu 24.04: `allow_any`, `allow_inactive`,
  `allow_active` jeweils `auth_admin` (kein Merken der Anmeldung).
  Beobachtet: `pkexec --disable-internal-agent /bin/true </dev/null` in einer Sitzung ohne
  grafischen Agenten → `Error executing command as another user: No authentication agent found.`,
  Exit 127.
  Unbelegt: Weitergabe von stdin/stdout/stderr steht nicht in der Manpage (Folgerung: pkexec ist
  setuid und startet PROGRAM per exec, daher erbt PROGRAM die Deskriptoren; nicht beobachtet).
- `sudo -A` (SUDO, belegt): liest das Passwort über ein (möglicherweise grafisches) Hilfsprogramm
  aus `SUDO_ASKPASS` oder `Path askpass` in `sudo.conf`; das Hilfsprogramm gibt das Passwort auf
  stdout aus. Beobachtet: auf dem Rechner ist kein `ssh-askpass`, `zenity` oder `kdialog` installiert.
  Folgerung: Das Passwort läuft durch einen Nutzerprozess (kein vom System isolierter Dialog);
  sudoers kann den Nutzer ausschließen.
- run0 (RUN0, belegt): Authentifizierung über polkit, „isolating the authentication prompt from the
  terminal (if possible)“; kein setuid; Befehl läuft als frischer Dienst des Dienstmanagers; eigenes
  Pseudo-TTY standardmäßig; `--pipe` (ab v257) reicht stdio durch, sind alle drei Ströme kein TTY,
  werden sie direkt durchgereicht; Umgebung vom Dienstmanager, `$TERM` kopiert, `SUDO_USER`/`SUDO_UID`/
  `SUDO_GID` gesetzt; früheste Versionsangabe auf der Seite „Added in version 256“.
  Beobachtet: Ubuntu 24.04 hat `systemd 255 (255.4-1ubuntu8.18)` und **kein** `run0`.
- Zuverlässiger Systemdialog in einer Desktop-Sitzung: pkexec mit dem polkit-Agenten der Sitzung
  (GNOME, KDE, XFCE bringen je einen mit – Unbelegt, keine Quelle gelesen). Voraussetzung ist ein
  registrierter Agent für die Sitzung des aufrufenden Prozesses (PKEXEC). Unbelegt: ob ein von
  Claude Code/Codex gestarteter MCP-Prozess (ohne `DBUS_SESSION_BUS_ADDRESS`, siehe
  `docs/refs/codex-mcp-spec-rmcp-2026-10-12.md` §1 „Umgebung“) von polkit derselben grafischen
  Sitzung zugeordnet wird; polkit ordnet über die Login-Sitzung (logind) zu, nicht über D-Bus-Umgebung
  (Annahme, nicht belegt).

## 5. Folgerungen für die se-Vorgaben (keine Quelle; Ableitung aus 1–4)

1. Claude Code: Ein Plugin-`PreToolUse`-Hook mit Matcher `.*`, `onFailure: "block"` und Ausnahme
   `^mcp__plugin_smart-explorer_smart-explorer__` sperrt `Read`, `Write`, `Edit`, `NotebookEdit`,
   `Glob`, `Grep`, `LSP`, dieselben Werkzeuge in Subagenten (1.5), `REPL`-Unteraufrufe und fremde
   MCP-Werkzeuge per Pfadvergleich, solange kein Schalter aus 1.6 ihn abschaltet und kein Mod ihn
   übergeht. Für
   `Bash`/`PowerShell`/`Monitor` reicht der Befehlstext nicht (1.4 „best-effort“, 1.9); dort trägt
   nur die Sandbox (`Read(//pfad/**)` deny + `sandbox.enabled` + `allowUnsandboxedCommands: false` +
   `failIfUnavailable: true`), die auf Ubuntu 24.04 ein Root-AppArmor-Profil und unter nativem Windows
   gar nicht verfügbar ist (1.7, 3.1).
2. Codex: Hooks sind fail-open, verlangen Nutzervertrauen und decken `write_stdin` nicht ab (2.1,
   2.2); die Vollsperre trägt dort nur ein Permission-Profil mit `deny` (2.3), unter Windows nur mit
   `mxc` oder `elevated`.
3. Eine Sperre, die keine der Agenten-Einstellungen voraussetzt, verlangt, dass se den gesamten
   Agentenprozess (nicht nur dessen Shell) in eine Umgebung ohne die maskierten Pfade startet; der
   von Claude/Codex gestartete `se mcp` würde diese Einschränkung erben und müsste die Pfade über
   einen außerhalb laufenden se-Prozess erreichen. Linux braucht dafür User-Namespaces (Root-Profil
   auf Ubuntu 24.04) oder Landlock mit Erlaubnisliste und dem D-Bus-Ausweg vor ABI 9; Windows ein
   Token ohne `WRITE_RESTRICTED` mit Deny-ACE, `elevated`-ähnliche eigene Konten (Admin einmal) oder
   MXC.
4. Erhöhte Ausführung: Windows `runas` + eigene Named Pipe mit expliziter ACL (nur Nutzer-SID,
   NETWORK deny), `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`, Prüfung der
   Gegenstelle per `GetNamedPipeClientProcessId` und Hash-Bindung (Projektregel: SHA-256 statt
   gsudos Länge/Zeit); Linux pkexec mit Sitzungsagent, Exit 126/127 unterscheiden, run0 erst ab
   systemd 256.

## 6. Offene Punkte (Unbelegt, genaue fehlende Tatsache)

- Claude Code 2.1.332: ob `MultiEdit`/`LS` als Werkzeuge registriert sind; ob `Glob.path`,
  `Grep.path`, `LSP.filePath`, `NotebookEdit.notebook_path` vor dem Hook absolut gemacht werden; ob
  `--restricted` Plugin-Hooks lädt; ob ein Plugin-Manifestfeld `settings` noch gelesen wird.
- Codex 0.166.0: Verhalten bei nicht startbarem bubblewrap (Ablehnung oder ungesandboxt);
  `requirements.toml`-Schlüssel gegen `danger-full-access`/Eskalation; ob ein fremd geschriebener
  `trusted_hash` angenommen wird (nur aus Quelltext abgeleitet).
- Windows: `SE_INCREASE_QUOTA_NAME` im Standardnutzer-Token; Werkzeugkompatibilität mit nicht
  schreibbeschränktem Restricted Token; AppContainer/LPAC ohne Admin; `NO_READ_UP` auf
  Dateiobjekten; Task-Scheduler/COM-Ausbruch aus Restricted Token; Microsoft-Doku zu
  `ProtectedPrefix\Administrators\` und zu MXC.
- Linux: Yama-Primärquelle (Abruf HTTP 429); stdio-Weitergabe durch pkexec (Manpage schweigt);
  polkit-Agenten-Zuordnung für einen MCP-Kindprozess ohne D-Bus-Umgebung.
- Dienstkonto-Besitz/Verschlüsselung als Schutz gegen Programme desselben Nutzers: keine
  Primärquelle gelesen.
