# Claude Code: Plugins, Marketplaces und MCP-Server (abgerufen 2026-10-12)

Zweck: Grundlage für das SE-Plugin (`docs/plaene/2026-10-12-ki-agenten-plugin/plan.md`).
Quellen, alle abgerufen 2026-10-12:
`https://code.claude.com/docs/en/plugins/manifest-reference`, `…/plugins/components`,
`…/plugins/marketplace-reference`, `…/plugins/cli-reference`, `…/plugins/install`,
`…/plugins/loading`, `…/mcp`, `…/permissions`, `…/headless`, `…/env-vars` (nur die ersten
100 000 Zeichen gelesen), `…/skills`.

## 1. Plugin-Aufbau
- `.claude-plugin/plugin.json` ist optional; Pflichtfeld nur `name` (kebab-case, ohne Leerzeichen,
  `@`, `:` oder Pfadtrenner). Weitere Felder: `$schema`, `displayName`, `version`, `description`,
  `author` (Objekt, `name` Pflicht), `homepage`, `repository`, `license`, `keywords`, `metadata`,
  `icon`, `documentationUrl`, `supportUrl`, `privacyPolicyUrl`, `termsOfServiceUrl`,
  `defaultEnabled` (Standard `true`), `dependencies`, `settings`, `userConfig`, `types`,
  `channels`, `skills`, `commands`, `agents`, `hooks`, `mcpServers`, `lspServers`,
  `outputStyles`, `workflows`, `experimental`. Unbekannte Felder werden mit Warnung entfernt.
- `version` ist ein String ohne Semver-Prüfung; gesetzt pinnt er Nutzer bis zur nächsten Änderung.
- Pfade im Manifest beginnen mit `./` und bleiben im Plugin-Ordner.
- MCP-Server: `.mcp.json` im Plugin-Ordner (`{"mcpServers": {...}}`) und/oder `mcpServers` im
  Manifest; beide werden geladen, ein später gleichnamiger Server ersetzt den früheren. Server
  starten, sobald das Plugin aktiviert ist.
- Skills: `skills/<name>/SKILL.md`; Frontmatter optional (`name`, `description`, `when_to_use`,
  `allowed-tools`, `disable-model-invocation`, `user-invocable`, weitere). Aufruf als
  `/<plugin>:<skill>`.
- Variablen: `${CLAUDE_PLUGIN_ROOT}` (installierter Versionsordner, wechselt beim Update),
  `${CLAUDE_PLUGIN_DATA}` (`~/.claude/plugins/data/<id>/`, bleibt), `${CLAUDE_PROJECT_DIR}`,
  `${user_config.KEY}`. In stdio-MCP-Einträgen werden `${CLAUDE_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_DATA}`
  in `command`, `args`, `env` ersetzt. Allgemeines `${VAR}`/`${VAR:-default}` ist für `.mcp.json`
  dokumentiert, für Plugin-`.mcp.json` nicht ausdrücklich.

## 2. Marketplaces und CLI
- `.claude-plugin/marketplace.json` im Marketplace-Wurzelordner; Pflicht: `name`
  (Buchstaben, Ziffern, `.`, `_`, `-`; Beginn mit Buchstabe/Ziffer; kein `..`), `owner`
  (`name` Pflicht), `plugins` (je Eintrag `name`, `source` Pflicht). `source` als relativer Pfad
  `./plugins/x` (ohne `..`), `{"source":"github","repo":"owner/repo","ref":…}`,
  `{"source":"url","url":"https://…git"}`, `git-subdir`, `npm`, `archive`, `command`.
- Lokaler Marketplace: `claude plugin marketplace add <ordner>` (Formen `./`, `../`, `/`, `~/`;
  unter Windows auch `.\`, `..\`, `C:\`). **Relative Plugins eines lokalen Marketplace werden
  nicht kopiert, sondern bei jedem Sitzungsstart aus dem Quellordner geladen** (unabhängig von
  `version`). Git-/GitHub-Quellen werden nach `~/.claude/plugins/cache/<mp>/<plugin>/<version>/`
  kopiert.
- Befehle (Exit 0 Erfolg, 1 Fehler):
  `claude plugin marketplace add <quelle> [--scope user|project|local] [--json]`,
  `claude plugin marketplace list [--json]`, `… marketplace update [name]`,
  `… marketplace remove <name>` (entfernt auch dessen Plugins und deren Daten),
  `claude plugin install <plugin>[@<mp>] [-s user|project|local] [-y] [--json]` (ohne TTY und ohne
  `-y` abgelehnt, Exit 1; bereits installiert → Meldung, Exit 0),
  `claude plugin update …`, `claude plugin uninstall <plugin>[@<mp>] [-s …] [-y]`,
  `claude plugin enable|disable`, `claude plugin list [--json]`, `claude plugin validate <pfad>`.
  Standard-Scope von install/uninstall/marketplace add: `user`.
- Versionserkennung: Manifest-`version`, sonst Marketplace-`version`, sonst aus der Quelle
  (GitHub/Git: Commit-SHA, 12 Zeichen). Ohne gesetzte `version` erzeugt jeder Commit eine neue
  Version.

## 3. `claude mcp`
`claude mcp add [--scope local|project|user] [--transport stdio] [--env K=V] <name> -- <befehl> [args…]`,
`claude mcp add-json <name> '<json>'`, `claude mcp list`, `claude mcp get <name>`,
`claude mcp remove <name>`. Scope `user` liegt in `~/.claude.json`. Rangfolge gleichnamiger Server:
local, project, user, Plugin, claude.ai-Connector.

## 4. MCP-Verhalten
| Thema | Dokumentiert |
|---|---|
| Wanduhr-Timeout je Aufruf | `MCP_TOOL_TIMEOUT`, ohne Wert ca. 28 h; je Server `timeout` (ms) |
| Leerlauf-Timeout | `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`, stdio Standard 1 800 000 ms; **Fortschrittsmeldungen setzen ihn zurück** |
| Ausgabegröße | `MAX_MCP_OUTPUT_TOKENS` Standard 25 000 Token, Warnung ab 10 000 |
| Große Ergebnisse | Text > 50 000 Zeichen wird in eine Datei unter `~/.claude/projects/…/tool-results` gelegt; je Tool anhebbar per `_meta["anthropic/maxResultSizeChars"]` bis 500 000 |
| Beschreibungen | Tool-Beschreibung und Server-`instructions` je 2 048 Zeichen (Kürzung) |
| Tool Search | Standard an: nur Toolnamen und Server-Anweisungen werden beim Start geladen; `_meta["anthropic/alwaysLoad"]: true` lädt vorab |
| Elicitation | Form- und URL-Modus unterstützt |
| `_meta["anthropic/requiresUserInteraction"]: true` | erzwingt Bestätigung bei jedem Aufruf; Allow-Regeln überspringen sie nicht; in `dontAsk` abgelehnt |
| `readOnlyHint`/`destructiveHint`, `outputSchema`/`structuredContent`, Esc-Abbruch (`notifications/cancelled`) | nicht dokumentiert |
| `notifications/tools/list_changed` | unterstützt |
| Protokoll | v2-Laufzeit fordert `2026-07-28` an (TS-SDK 2.0); `MCP_PROTOCOL_NEGOTIATION=legacy` erzwingt Altverhalten |
| stdio | kein automatischer Neustart; `CLAUDE_PROJECT_DIR` in der Serverumgebung |

## 5. Berechtigungen
- Regeln `mcp__<server>`, `mcp__<server>__<tool>`, `mcp__<server>__*`. Plugin-Server heißen
  `mcp__plugin_<plugin>_<server>__<tool>` (Zeichen außerhalb `A-Za-z0-9_-` → `_`).
- Plugins können **keine** Allow-Regeln mitliefern (Plugin-`settings` wirkt nur für `agent`,
  `subagentStatusLine`).
- Headless `-p`: ohne Berechtigungsgeber wird alles abgelehnt, was fragen würde; `--allowedTools`
  mit Regelsyntax.

## 6. Windows
PATH-/PATHEXT-Auflösung eines MCP-`command` ist nicht dokumentiert. Folgerung für SE: erzeugte
Konfigurationen tragen den absoluten Pfad der installierten `se`-Datei.

## 7. Nachtrag 2026-10-14: Vorab laden und Längen

Quelle: `https://code.claude.com/docs/en/mcp`, abgerufen 2026-10-14, Abschnitte „Scale with MCP
tool search“, „Tool search for MCP server authors“, „Mark a tool to load upfront or stay
deferred“, „Exempt a server from deferral“ und „Defer a server's tools“.

- Tool Search ist Standard. Beim Sitzungsstart laden nur Toolnamen und Server-Anweisungen.
  Server-Anweisungen sollen sagen, wann Claude die Tools suchen soll.
- `_meta["anthropic/alwaysLoad"]: true` im `tools/list`-Eintrag eines Tools lädt es vorab. Der
  Start wartet deswegen nicht auf den Server.
  - Mit `false` bleibt ein Tool verzögert, auch wenn der Nutzer `"alwaysLoad": true` für den
    Server setzt. Das gilt für `--mcp-config`, Agent-SDK- und Plugin-Server und erst ab v2.1.285.
  - Ein Servereintrag mit `"alwaysLoad": false` verzögert alle Tools, auch die vorab markierten.
    Das gilt ab v2.1.287.
- Server-weites `"alwaysLoad": true` im Servereintrag lädt alle Tools vorab. Der Start wartet
  dann auf den Server, höchstens 5 s Verbindungstimeout.
- Kürzung: Server-Anweisungen und Beschreibungen vorab geladener Tools nach 4 096 Zeichen,
  Beschreibungen per Tool-Suche geladener Tools nach 16 384 Zeichen. Vor v2.1.296 lag die kürzere
  Grenze bei 2 048 Zeichen, vor v2.1.295 galt sie für alle Beschreibungen.
  `CLAUDE_CODE_MAX_MCP_DESCRIPTION_LENGTH` ersetzt beide Grenzen (ab v2.1.280).
- Die Tabelle in §4 („je 2 048 Zeichen“) beschreibt den Stand vor v2.1.296. SE hält die
  Anweisungen weiter unter 2 048 Zeichen, damit auch ältere Claude-Code-Versionen sie vollständig
  zeigen.
- rmcp 3.5.1: `Tool::with_meta(MetaObject)` serialisiert das Feld als `_meta`
  (`src/model/tool.rs`, `pub meta: Option<MetaObject>`).
