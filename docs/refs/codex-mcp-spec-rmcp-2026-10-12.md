# Codex, MCP-Spezifikation 2026-07-28 und rmcp 3.5.1 (abgerufen 2026-10-12)

Zweck: Grundlage für `se mcp` (`docs/plaene/2026-10-12-ki-agenten-plugin/plan.md`).
Codex-Quellen: `learn.chatgpt.com/docs/{plugins,extend/mcp,config-file/config-reference,build-skills,permissions,agent-approvals-security}`
(die früheren `developers.openai.com/codex/*`-Seiten leiten per 308 dorthin) und der Codex-Quelltext
`github.com/openai/codex` Tag `rust-v0.162.1` (2026-10-09). MCP: `modelcontextprotocol.io/specification/2026-07-28/*`.
rmcp: crates.io-Paket `rmcp-3.5.1.crate` (lokal entpackt und gelesen).

## 1. Codex
- **Protokoll:** Codex startet jeden Server mit `initialize` und `protocolVersion` `2025-06-18`;
  `2026-07-28` nur in einem Sondermodus mit Server-Env `CODEX_MCP_PROTOCOL_VERSION`. Codex nutzt
  selbst `rmcp =3.3.0`.
- **Konfiguration** `~/.codex/config.toml`, Tabelle `[mcp_servers.<name>]`: `command` (Pflicht),
  `args`, `env`, `env_vars`, `cwd`, `startup_timeout_sec`, `tool_timeout_sec`, `enabled`, `required`,
  `enabled_tools`, `disabled_tools`, `default_tools_approval_mode`, `tools.<t>.approval_mode`
  (`auto|prompt|writes|approve`), `tools.<t>.output_token_limit`.
- **Timeouts:** Doku: Start 10 s, Tool 60 s; Quelle (`codex-mcp/src/rmcp_client.rs`): 30 s / 300 s.
  Für Planung gilt der kleinere dokumentierte Wert (60 s).
- **CLI:** `codex mcp add <NAME> [--env K=V]... -- <COMMAND>...`, `codex mcp list|get <name> [--json]`,
  `codex mcp remove <name>` (`codex-rs/cli/src/mcp_cmd.rs`).
- **Freigaben (Modus `auto`, Standard):** fragt immer bei `destructiveHint == true`, nie bei
  `readOnlyHint == true`, sonst nur dann nicht, wenn `destructiveHint` und `openWorldHint` beide
  ausdrücklich `false` sind (`core/src/mcp_tool_call.rs`).
- **Ergebnisse:** Ist `structuredContent` gesetzt, sieht das Modell nur dieses JSON
  (`protocol/src/models.rs`). Standard-Kürzung je Tool-Ausgabe 10 000 Token
  (`models-manager/models.json`); eine stdio-Zeile > 8 MiB schließt den Transport.
- **Fortschritt/Abbruch:** Fortschritt landet nur im Codex-Log; beim eigenen Timeout verwirft
  Codex die Anfrage, ein `notifications/cancelled` ist nach Quelltext nicht zu erwarten (Annahme,
  nicht laufzeitgeprüft).
- **Sandbox:** stdio-Server laufen außerhalb der Befehls-Sandbox als Kind des Orchestrators.
  Shellbefehle laufen dagegen in `workspace-write` ohne Netz und mit Schreibrecht nur im Workspace.
- **Umgebung:** Unix gibt nur `HOME, LOGNAME, PATH, SHELL, USER, __CF_USER_TEXT_ENCODING, LANG,
  LC_ALL, TERM, TMPDIR, TZ` plus `env`/`env_vars` weiter (`rmcp-client/src/utils.rs`); Windows nur
  `WINDOWS_CORE_ENV_VARS` (`protocol/src/shell_environment.rs`: PATH, PATHEXT, SHELL, COMSPEC,
  SYSTEMROOT, WINDIR, SYSTEMDRIVE, USERNAME, USERDOMAIN, USERPROFILE, HOMEDRIVE, HOMEPATH,
  PROGRAMFILES, PROGRAMFILES(X86), PROGRAMW6432, PROGRAMDATA, LOCALAPPDATA, APPDATA, TEMP, TMP,
  TMPDIR, POWERSHELL, PWSH). Folge für SE: unter Linux fehlen `XDG_DATA_HOME`, `XDG_RUNTIME_DIR`,
  `DBUS_SESSION_BUS_ADDRESS`; unter Windows ist `APPDATA` vorhanden.
- **Skills:** Nutzer `$HOME/.agents/skills/<name>/SKILL.md` (Repo `.agents/skills`, veraltet
  `$CODEX_HOME/skills`); Frontmatter `name` (≤ 64) und `description` (≤ 1024) Pflicht.
- **Plugins:** Agent-Plugins-Format (`plugin.json` + `mcp.json`, auch von Cursor und VS Code
  gelesen); stdio-`command` ist ein einzelnes Token (Programmname oder `./`-Pfad im Plugin), lokale
  Plugins werden nach `~/.codex/plugins/cache/<mp>/<plugin>/local/` kopiert; `codex plugin
  marketplace add <src>`, `codex plugin add <p>@<mp>`, `codex plugin remove`.
- **Windows-Start:** Programmauflösung über `which` mit PATH/PATHEXT; Server in einem Job Object.

## 2. MCP-Spezifikation
- Aktuell `2026-07-28` (veröffentlicht 2026-07-28), davor `2025-11-25`. 2026-07-28 ist zustandslos:
  kein `initialize`, jede Anfrage trägt `_meta` mit `io.modelcontextprotocol/protocolVersion` und
  Client-Fähigkeiten, Server MÜSSEN `server/discover` anbieten; Clients mit beiden Generationen
  proben auf stdio mit `server/discover` und fallen auf `initialize` zurück. Ergebnisse tragen
  `resultType`; `ping` und `logging/setLevel` entfallen; Logging ist veraltet (stderr empfohlen).
- Fortschritt: `_meta.progressToken` → `notifications/progress {progressToken, progress, total?,
  message?}`, `progress` MUSS steigen. Abbruch: `notifications/cancelled {requestId}`; der Server
  SOLL aufhören und sendet für diese Anfrage nichts mehr.
- Tasks: in 2026-07-28 offizielle Erweiterung `io.modelcontextprotocol/tasks`; laut
  Client-Matrix unterstützt sie kein Client → eigenes Jobmodell in Tools.
- Tool-Annotationen (Standardwerte): `readOnlyHint` false, `destructiveHint` true,
  `idempotentHint` false, `openWorldHint` true. `structuredContent` SOLL zusätzlich als Text-JSON
  geliefert werden. Elicitation: Formmodus darf keine Passwörter abfragen (dafür URL-Modus).
- JSON-RPC-Batching entfiel mit 2025-06-18.

## 3. rmcp 3.5.1
- 2026-10-05, Apache-2.0, MSRV 1.88, Edition 2024. Bekannte Versionen `2024-11-05`, `2025-03-26`,
  `2025-06-18`, `2025-11-25`, `2026-07-28`.
- **Beide Lebenszyklen auf einer stdio-Verbindung** (gelesen `src/service/server.rs` Z. 572–690):
  `ping` vor dem Start wird beantwortet; `server/discover` wird als Probe beantwortet, danach bleibt
  der Server offen für `initialize` (alt) oder eine erste moderne Anfrage mit vollständigen
  Inline-Metadaten (neu); fehlende Metadaten → Fehlerantwort, nicht unterstützte Version →
  `unsupported_protocol_version`.
- Features: Standard `base64, macros, server`; `server` = `transport-async-rw, schemars, pastey,
  uuid`; `transport-io` = `transport-async-rw` + `tokio/io-std`. Pflichtabhängigkeiten u. a.
  `tokio` (sync, macros, rt, time), `tokio-util`, `futures`, `indexmap`, `thiserror 2`; mit
  `server` `schemars 1.x`.
- Alternativen geprüft: `rust-mcp-sdk` 2.0.0 (nur 2026-07-28 → Codex-Start scheitert),
  `pmcp` 2.22.7 (MSRV 1.91), `turbomcp` 3.6.0 (MIT) — Drittanbieter, ohne Vorteil gegenüber dem
  offiziellen SDK.
