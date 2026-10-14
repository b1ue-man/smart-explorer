# 2026-10-14 Analyse-Auftrags-ID und Plugin-Vorrang

Nutzerauftrag vom 2026-10-14 (eine Nachricht, drei Teile):

1. Für das neu gekoppelte Gerät Remote-Codeausführung erlauben (Betrieb, kein Code).
2. „Scan fehlgeschlagen: /Root: Ungültige Analyse-Auftrags-ID“ beim Start einer Analyse
   beheben; Codeänderung mit Build und Release ist autorisiert.
3. Das KI-Plugin soll sich bei der Installation selbst anpreisen, damit die KI es vor
   den normalen Dateiexplorer-Werkzeugen nutzt.

## A. Erstplan

- Teil 1 erledigt am 2026-10-14 um 06:40 UTC mit
  `se share grants exec enable 256f4564-9960-4939-899a-860285ae0c6f --yes` (Gerät
  `SILAS_ASUS`, gekoppelt um 06:34 UTC). Ausgabe: `requested=enabled persisted=true
  applied=true revision=1`.
- Teile 2 und 3 brauchen Codeänderungen, danach eine Task-Suite und einen Release.

## B. Bestand

- Host-Prüfung `share/os/shared/analysis_tasks.rs` `task()`: `request_id` muss 16 bis
  64 Zeichen lang sein und darf nur `0-9a-f` enthalten, sonst kommt `InvalidInput`
  „Ungültige Analyse-Auftrags-ID“.
- Client `share/core/peer_storage_analysis.rs` `scan()` und `share/core/peer_duplicates.rs`
  `find()`: `request_id = random_token(16)` erzeugt URL-sicheres Base64 (22 Zeichen
  `A-Za-z0-9_-`, `share/core/crypto.rs` `b64`). Das trifft die Host-Prüfung fast nie.
- Beide Seiten kamen mit Commit `5098ee06` (2026-10-03) und sind seit v0.5.170
  ausgeliefert. Seitdem scheitert jede Remote-Speicheranalyse und Duplikatsuche bei
  Gegenstellen mit `analysis_reattach_v1`. Beobachtet: Der Linux-Host (0.5.177) gibt
  `Root` → `/` frei, und der Laptop analysiert `/Root`.
- Plugin: Die Server-Anweisungen stehen in `cli/mcp/catalog.rs` `INSTRUCTIONS` und
  werden von `server.rs` `with_instructions` gesetzt. Der Skill liegt in
  `cli/plugin/assets/SKILL.md`, die Manifest-Beschreibung in `cli/plugin/files.rs`
  `DESCRIPTION`. `se mcp` schreibt veraltete erzeugte Plugin-Dateien beim Start neu
  (`files::refresh`).
- Claude Code (code.claude.com/docs/en/mcp, abgerufen 2026-10-14):
  - Tool Search ist Standard. Beim Sitzungsstart werden nur Toolnamen und
    Server-Anweisungen geladen.
  - `_meta["anthropic/alwaysLoad"]: true` im `tools/list`-Eintrag lädt ein Tool vorab,
    ohne den Start aufzuhalten.
  - Server-Anweisungen und vorab geladene Tool-Beschreibungen werden nach 4 096
    Zeichen gekürzt, vor v2.1.296 nach 2 048.
  - Anweisungen sollen sagen, wann Claude die Tools suchen soll.
- rmcp 3.5.1: `Tool::with_meta(MetaObject)`, das als `_meta` serialisiert wird.

## C. Entscheidungen

- Analyse-ID:
  - Clients senden künftig 32 Kleinbuchstaben-Hexzeichen (`random_hex_token::<16>`).
    Das passt zur dokumentierten Host-Prüfung, auch bei Hosts 0.5.170 bis 0.5.177.
  - Hosts akzeptieren zusätzlich `A-Za-z0-9_-`, Länge 16 bis 64. So funktionieren auch
    ausgelieferte Clients 0.5.170 bis 0.5.177 mit einem aktualisierten Host.
  - Die ID ist nur ein Tabellenschlüssel zusammen mit dem Principal. Sie wird nie
    Pfad, Dateiname oder Logbefehl, deshalb ist das größere Alphabet unkritisch.
  - Beide Regeln liegen in einem plattformneutralen Modul
    `share/core/analysis_request_id.rs`.
  - Verworfen: Nur den Client ändern. Dann bliebe jeder bereits ausgelieferte Client
    mit jedem Host kaputt.
  - Verworfen: Nur den Host ändern. Dann blieben neue Clients gegenüber ausgelieferten
    Hosts kaputt.
- Plugin-Vorrang:
  - Die ersten 512 Zeichen der Server-Anweisungen (Codex) nennen den Vorrang vor den
    eingebauten Datei-Werkzeugen und Shell-Befehlen.
  - Die Datei-Tools `connections`, `list`, `stat`, `read`, `write`, `edit`, `mkdir`,
    `remove`, `copy`, `move`, `search` und `job_status` tragen
    `anthropic/alwaysLoad`. Claude Code zeigt sie damit ohne Suchschritt.
  - Die Tool-Beschreibungen nennen das ersetzte Werkzeug.
  - Skill-Beschreibung und Manifest-Beschreibung nennen den Vorrang.
  - Die Anweisungen bleiben unter 2 048 Zeichen, weil ältere Claude-Code-Versionen
    danach kürzen.
  - Verworfen: `CLAUDE.md` oder `AGENTS.md` des Nutzers zu beschreiben. Das greift in
    fremde Dateien ein.
  - Verworfen: Server-weites `alwaysLoad` in `.mcp.json`. Es hält den Sitzungsstart
    bis zu 5 s auf und gilt nur für Claude Code.

## D. Arbeitsplan und erwartete Ergebnisse

| Nr. | Meilenstein | Dateien | Erwartetes Ergebnis |
|---|---|---|---|
| M1 | Gemeinsames ID-Format | `share/core/analysis_request_id.rs` (neu), `share/mod.rs`, `peer_storage_analysis.rs`, `peer_duplicates.rs`, `analysis_tasks.rs` | Remote-Analyse und Duplikatsuche über zwei echte Loopback-Knoten liefern ein Ergebnis; eine Base64url-ID wie von 0.5.170 bis 0.5.177 wird vom Host angenommen; leere, zu kurze und Pfad-IDs werden abgelehnt |
| M2 | Plugin-Vorrang | `cli/mcp/catalog.rs`, `cli/plugin/assets/SKILL.md`, `cli/plugin/files.rs` | `tools/list` von `se mcp` liefert für die zwölf Datei-Tools `_meta["anthropic/alwaysLoad"]=true` und für `exec`, `job_input`, `job_cancel`, `status` und `se` keinen solchen Eintrag; `instructions` nennt in den ersten 512 Zeichen den Vorrang und bleibt unter 2 048 Zeichen; der erzeugte Skill und das Manifest nennen den Vorrang |
| M3 | Doku | `docs/AI_AGENT_PLUGIN.md`, `README.md`, `docs/TODO.md` falls betroffen | Verhalten beschrieben |
| M4 | Suite | `native/test-analysis-plugin-task.py`, `.github/workflows/analysis-plugin-task.yml` | Ein Remote-Lauf (Linux und Windows) prüft M1 und M2 |
| M5 | Release | `build.yml` `complete_release_source_sha` | v0.5.178 auf GitHub, danach die lokale `se`-Installation aktualisiert und der Worker auf 0.5.178 |

Zu erhaltendes Verhalten: Hosts ohne `analysis_reattach_v1` bekommen weiterhin keine ID.
Die Wiederanbindung nach Verbindungsverlust (gleiche ID, gleiche Parameter) bleibt
unverändert. Die übrigen Tools bleiben verzögert ladbar.

## Fortschritt

- [x] Teil 1, exec für `SILAS_ASUS`
- [ ] M1
- [ ] M2
- [ ] M3
- [ ] M4
- [ ] M5
