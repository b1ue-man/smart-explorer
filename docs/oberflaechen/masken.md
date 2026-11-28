# Maskenlisten und Agenten-Sperre

Stand 2026-11-28 (Plan `docs/plaene/2026-11-28-ersetzen-masken-shell/plan.md`, Abschnitte „S1 — Vollsperre“ und „S2 — Maskenlisten“; Befunde `docs/refs/agenten-sperre-elevation-2026-11-28.md` §1–2 und §5). Eine Maskenliste ersetzt Werte (wörtlich, Regex, unscharf mit Toleranz) in jeder Ausgabe von Smart Explorer an einen KI-Agenten durch Platzhalter `⟦Bezeichnung-N⟧` und führt Platzhalter in den Eingaben des Agenten auf die echten Werte zurück. Schützt eine aktive Liste einen Ordner (Schutzpfad), gilt die **Agenten-Sperre**: Claude Code und Codex dürfen dann nur noch die Smart-Explorer-Werkzeuge und Werkzeuge ohne Datei- oder Prozesszugriff nutzen. Pfade unten sind relativ zu `native/crates/`.

## Kern

| Fähigkeit | Verhalten | Beleg |
|---|---|---|
| Listen, Einträge, Schutzpfade, Sperre | Speicher, Rechte je Handelndem (`Agent`, `Cli`, `Gui`, `VerifiedUser`), Klartext nur für `Gui` | `engine/src/masks/mod.rs`, Rechte `engine/src/masks/core/rights.rs`, Verwaltung `engine/src/masks/os/shared/store_admin.rs` (Paket P1) |
| Zustand für den Hook | `lockdown_state()`: `Off`, `On`, `Unknown` (Speicher vorhanden, aber nicht lesbar: wie `On`) ohne Schlüssel | `engine/src/masks/os/shared/store.rs:59` |
| Erlaubte fremde MCP-Server | Namensliste ohne Geheimnis in `<App-Daten>/ai-plugins/lockdown-servers.json`; fehlt sie oder ist sie beschädigt, ist kein fremder Server erlaubt | `cli/src/cli/hook_servers.rs:19` (`allowed`), `:51` (`valid`), `:85` (`change`) |

## Agenten-Sperre (Hook `se hook`)

| Fähigkeit | Verhalten | Beleg |
|---|---|---|
| Aufruf | `se hook claude` / `se hook codex` (versteckt) liest die `PreToolUse`-Eingabe (Claude Code zusätzlich `PreSkillShellCommand`) von stdin | `cli/src/cli/hook.rs:40` (`HookArgs`), `:54` (`run`) |
| Sperre aus | keine Ausgabe, Exit 0: der Host entscheidet wie bisher | `cli/src/cli/hook.rs:84` (`decide_bytes`) |
| Sperre an oder Zustand unbekannt | Ablehnung `hookSpecificOutput.permissionDecision = "deny"` mit Grund, der das passende se-Werkzeug nennt (Read → `read`, Edit/Write/NotebookEdit → `edit`/`write`/`apply_patch`, Glob/LS → `search`/`list`, Grep → `grep`, Bash/PowerShell/Monitor → `shell`, Ausgabe von Hintergrund-Shells → `job_status`); Exit 2 mit Grund auf stderr, wenn stdout nicht schreibbar ist; `PreSkillShellCommand` immer Exit 2 | `cli/src/cli/hook.rs:114` (`emit`), Gründe `cli/src/cli/hook_policy.rs:82` (`instead`) |
| Abgelehnt | jedes native Datei-, Shell-, Notebook- und LSP-Werkzeug, `BashOutput`/`TaskOutput`, jedes unbekannte Werkzeug (fail-closed), jedes fremde MCP-Werkzeug außer den erlaubten Servern, MCP-Ressourcen fremder Server, `WebFetch` auf Loopback, private, Link-Local-, CGNAT- (100.64/10), unbestimmte und Multicast-Adressen, Namen ohne Punkt, `.local`/`.localhost`/`.internal`/`.lan`/`.home.arpa`/`.intranet`, jedes andere Schema als http/https (auch `file:`); Namen werden aufgelöst und jede Adresse geprüft | `cli/src/cli/hook_policy.rs:116` (`decide_tool`), `:160` (`decide_mcp`); `cli/src/cli/hook_web.rs:9` (`check_url`), `:98` |
| Erlaubt | se-Werkzeuge: Claude Code `mcp__plugin_smart-explorer_se__*` mit `mcp_server.source = "plugin"` (gleichnamige Nutzer-Server zählen nicht), Codex `mcp__smart-explorer__*`; Claude Code ohne Datei- oder Prozesszugriff: Agent, Task, AskUserQuestion, TodoWrite, TaskCreate/Update/List/Get/Stop, KillShell, KillBash, ExitPlanMode, EnterPlanMode, WebSearch, Skill, ToolSearch, SlashCommand, EndConversation, CronCreate/Delete/List, SendMessage, REPL (jeder Unteraufruf von Code Mode kommt einzeln am Hook an); Codex: update_plan, spawn_agent, Agent, send_input, wait, close_agent, resume_agent, request_user_input | `cli/src/cli/hook_policy.rs:42` (`CLAUDE_SAFE`), `:70` (`CODEX_SAFE`) |

## Plugin `se plugin`

| Fähigkeit | Verhalten | Beleg |
|---|---|---|
| Claude Code | erzeugtes Plugin enthält `hooks/hooks.json` mit `PreToolUse` und `PreSkillShellCommand` (Matcher `*`), Handler mit `onFailure: "block"`, Timeout 30 s; Linux/macOS `[ -x '<se>' ] \|\| exit 0; exec '<se>' hook claude`, Windows `shell: "powershell"` mit `Test-Path`: ein deinstalliertes se legt den Host nicht lahm, ein fehlschlagender Hook blockiert | Datei `cli/src/cli/plugin/files.rs:102`, Handler `cli/src/cli/hook_install.rs:48`, `:74` |
| Codex | eine Gruppe `[[hooks.PreToolUse]]` (Matcher `*`, `command` über `$SHELL -lc`, `commandWindows` über `cmd /C`) in `config.toml` samt `[hooks.state."<Datei>:pre_tool_use:<i>:0"] trusted_hash` nach der Normalisierung von Codex 0.166.0, damit der Hook ohne Freigabe in `/hooks` läuft; nur die eigene Gruppe und der eigene Vertrauenseintrag werden geändert oder entfernt | `cli/src/cli/plugin/codex_hooks.rs:41` (`apply`), `:124` (`remove`), `:165` (`status`); Hash `cli/src/cli/hook_install.rs:96` |
| Programm des Hooks | das installierte CLI der Installation (`<Installationsordner>/se`, nicht der MCP-Starter, der nur `mcp` annimmt), sonst das registrierte se | `cli/src/cli/hook_install.rs:26` (`hook_program`) |
| Entfernen | `se plugin uninstall` entfernt Hook-Datei, Codex-Gruppe und die verwalteten Einträge; bei aktiver Sperre erst nach Bestätigung im Systemdialog | `cli/src/cli/plugin/claude.rs:68`, `cli/src/cli/plugin/codex.rs:270`, `:306` |
| Zustand | `se plugin status --json`: Codex-Feld `server.hook` mit `present`, `current`, `trusted` | `cli/src/cli/plugin/codex.rs:403` |

## Härtung über verwaltete Einstellungen

| Fähigkeit | Verhalten | Beleg |
|---|---|---|
| Claude Code | `managed-settings.d/90-smart-explorer.json` im Systemordner (`/etc/claude-code/`, `/Library/Application Support/ClaudeCode/`, `C:\Program Files\ClaudeCode\`): derselbe Hook plus `Read(//…)`/`Edit(//…)`-Ablehnungen für die lokalen Schutzpfade aktiver Listen und die se-App-Daten (wirkt auch auf @-Erwähnungen und IDE-Kontext); Nutzer-Settings (`disableAllHooks`) und Mods schalten ihn nicht ab | `cli/src/cli/hook_managed.rs:55`, `:94` (`deny_rules`), `:116` (`claude_document`) |
| Codex | Gruppe in `requirements.toml` (`/etc/codex/`, `%ProgramData%\OpenAI\Codex\`); verwaltete Hooks gelten als vertraut; alle übrigen Schlüssel, Gruppen und Kommentare bleiben | `cli/src/cli/hook_managed.rs:67`, `:125` (`codex_document`), `:153` |
| Erhöhung | `se elevated managed-hooks apply <hex>` / `remove <claude\|codex\|all>` mit Administratorrechten über `engine::elevate` (UAC, polkit, `sudo -n`) | `cli/src/cli/hook_managed.rs:345` (`run_elevated`), Start `cli/src/cli/hook_elevate.rs` |
| Wann | beim ersten Aktivieren einer Liste mit Schutzpfad und bei jeder Änderung der Schutzpfade (`se masks scope add/remove`, `enable`, `disable`, `delete`, desktop ebenso); nur wenn der Inhalt abweicht, sonst kein Dialog; ohne lokalen Schutzpfad und ohne frühere Einträge nichts | `cli/src/cli/hook_harden.rs:84` (`refresh_if_needed`) |
| Zustand | `current`, `outdated`, `absent`, `unreadable`, `conflict` je Host | `cli/src/cli/hook_managed.rs:209` (`state_for`), `cli/src/cli/hook_harden.rs:65` |

## CLI `se masks`

| Fähigkeit | Einstieg | Beleg |
|---|---|---|
| Listen anzeigen | `se masks list`, `se masks show <Liste>` (Kennung oder Name): Kennung, Name, aktiv, gesperrt, Schutzpfade, Einträge mit Kennung, Art, Bezeichnung, Toleranz, Groß/Klein, ganzes Wort; **nie ein Wert**; `--json` | `cli/src/cli/masks_cli.rs:49` (`MasksCommand`), Ausgabe `cli/src/cli/masks_cli_print.rs` |
| Anlegen und Erweitern | `se masks create <Name>`; `se masks add <Liste> [--kind literal\|regex\|fuzzy] [--label] [--tolerance off\|low\|medium\|high\|<Kürzung>/<Fehler innen>] [--case-sensitive true\|false] [--whole-word true\|false] [--whole-input]`, Werte von stdin, je Zeile einer | `cli/src/cli/masks_cli.rs:241` (`execute`) |
| Ändern | `se masks update <Liste> <Eintrag> [Optionen] [--value-stdin]` | ebd. |
| Schutzpfade | `se masks scope add\|remove <Liste> <Ziel>` (lokaler Pfad oder se-Ziel, auch entfernt) | ebd. |
| Sperren, Aktivieren | `se masks lock\|unlock\|enable\|disable\|delete <Liste>`, `se masks remove-entry <Liste> <Eintrag>` | ebd. |
| Fremde MCP-Server | `se masks servers list\|allow <Name>\|remove <Name>` | `cli/src/cli/masks_cli.rs:357` |
| Zustand, Härtung | `se masks status` (Sperre, Listenzahl, erlaubte Server, Härtung je Host), `se masks harden` | `cli/src/cli/masks_cli.rs:380` |
| Bestätigung | schwächende Änderungen (entsperren, deaktivieren, entfernen, löschen, Wert ändern, Toleranz senken, Server erlauben) und **jede** Änderung einer gesperrten Liste fragen erst über den Systemdialog (`engine::elevate::confirm_user_presence`), dann als `VerifiedUser`; so kann ein Agent über eine Shell nichts schwächen | `cli/src/cli/masks_cli_store.rs:96` (`change`) |

## Desktop-GUI

| Fähigkeit | Bedienweg | Beleg |
|---|---|---|
| Seite | Einstellungen → „Maskenlisten“ (eigener Bereich nach „Integration“) | `gui/src/app/core/settings_navigation.rs:14`, `gui/src/app/core/settings_ui.rs:139` |
| Zustand | Agenten-Sperre an/aus/unbekannt, Härtung je Host mit Pfad, „Härtung jetzt verankern“ | `gui/src/app/core/settings_masks.rs:177` (`status_section`) |
| Listen | anlegen, aktivieren/deaktivieren, sperren/entsperren, löschen | `gui/src/app/core/settings_masks.rs:279` (`list_body`) |
| Einträge | Wert verdeckt („••••••“), „Anzeigen“ liest ihn als `Actor::Gui`, „Verbergen“; Bearbeiten, Entfernen | `gui/src/app/core/settings_masks.rs:364` (`entry_row`) |
| Eintragsformular | Art (wörtlich, Regex, unscharf), Wert verdeckt mit „sichtbar“, Bezeichnung, Toleranz als Stufe (aus, gering, mittel, hoch) oder Zahlen (Kürzung, Fehler innen), Groß/Klein, nur ganzes Wort | `gui/src/app/core/settings_masks_form.rs:161` (`form_ui`) |
| Schutzpfade | „Ordner wählen …“ (Systemdialog, lokal) oder Ziel als Text (`@nas:/pfad`, `sftp://…`, `share://…`); Freigeben | `gui/src/app/core/settings_masks.rs:279`, Übernahme `:91` (`add_scope`) |
| Erlaubte MCP-Server | Liste, Erlauben (Systemdialog über `se masks servers allow`), Entfernen | `gui/src/app/core/settings_masks.rs:177` |
| Bestätigung | Änderung als `Actor::Gui`; lehnt der Speicher sie als schwächend ab, Systemdialog, dann als `VerifiedUser` | `gui/src/app/core/settings_masks_work.rs:91` (`as_user`) |
| Härtung nach Änderung | nach Schutzpfad-, Aktiv- und Löschänderungen `se masks harden` | `gui/src/app/core/settings_masks_work.rs:43` (`touches_scopes`), `:181` (`run`) |

## MCP `se mcp`

| Fähigkeit | Einstieg | Beleg |
|---|---|---|
| Werkzeug `masks` | `list` (ohne Werte), `create`, `add`, `add_path`; gesperrte Listen lehnen jede Änderung ab; Entsperren, Deaktivieren, Entfernen nur für den Nutzer | `cli/src/cli/mcp/masks_tool.rs` (Paket P4a), Katalog `cli/src/cli/mcp/catalog.rs` |
| Maskierung | jede Ausgabe mit allen aktiven Listen maskiert, jede Eingabe zurückgeführt | `cli/src/cli/mcp/mask_layer.rs` (Paket P4a) |

## Lücken und Ausnahmen

- **Android:** keine Maskenlisten-Oberfläche. Grund: Android führt keinen MCP-Server aus; Maskenlisten wirken nur dort, wo ein Agent `se mcp` nutzt.
- **In-App-Ordnerauswahl für Schutzpfade:** die Seite nimmt entfernte Ziele als Text und lokale Ordner über den Systemdialog; die vorhandene In-App-Ordnerauswahl (`gui/src/app/core/picker_types.rs`) braucht dafür einen eigenen Zweck mit Übernahme in `MasksPage::add_scope`.
- **Codex:** Hooks sind fail-open (ein abstürzender Hook lässt den Aufruf laufen); `write_stdin` in eine vor dem Aktivieren geöffnete Shell erreicht keinen Hook (Ref. §2.1–2.2). Die Vertrauens-Hash-Bildung folgt Codex 0.166.0; weicht die Pfadschreibweise der Konfigurationsdatei ab, zeigt `/hooks` den Hook zur Freigabe, und die verwaltete Gruppe in `requirements.toml` gilt ohnehin.
- **Claude Code:** Mods können Plugin-Hooks übergehen, verwaltete `PreToolUse`-Hooks nicht (Ref. §1.6); ein vorhandenes MDM- oder Server-Profil ersetzt die Dateiquelle der verwalteten Einstellungen (CC-managed „Precedence within the managed tier“).
- **Kosten:** jeder Werkzeugaufruf startet `se hook` einmal; unter Windows startet Claude Code dafür PowerShell.
