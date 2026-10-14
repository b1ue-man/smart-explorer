# 2026-10-14 Analyse-Auftrags-ID und Plugin-Vorrang

Nutzerauftrag vom 2026-10-14, zwei Teile:

1. „Scan fehlgeschlagen: <Pfad>: Ungültige Analyse-Auftrags-ID“ beim Start einer
   Remote-Analyse beheben; Codeänderung mit Build und Release ist autorisiert.
2. Das KI-Plugin soll sich bei der Installation selbst anpreisen, damit die KI es vor
   den normalen Dateiexplorer-Werkzeugen nutzt.

## A. Erstplan

- Beide Teile brauchen Codeänderungen, danach eine Task-Suite und einen Release.

## B. Bestand

- Host-Prüfung `share/os/shared/analysis_tasks.rs` `task()`: `request_id` muss 16 bis
  64 Zeichen lang sein und darf nur `0-9a-f` enthalten, sonst kommt `InvalidInput`
  „Ungültige Analyse-Auftrags-ID“.
- Client `share/core/peer_storage_analysis.rs` `scan()` und `share/core/peer_duplicates.rs`
  `find()`: `request_id = random_token(16)` erzeugt URL-sicheres Base64 (22 Zeichen
  `A-Za-z0-9_-`, `share/core/crypto.rs` `b64`). Das trifft die Host-Prüfung fast nie.
- Beide Seiten kamen mit Commit `5098ee06` (2026-10-03) und sind seit v0.5.170
  ausgeliefert. Seitdem scheitert jede Remote-Speicheranalyse und Duplikatsuche bei
  Gegenstellen mit `analysis_reattach_v1`.
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
| M5 | Release | `build.yml` `complete_release_source_sha` | v0.5.178 mit allen Assets auf GitHub |

Zu erhaltendes Verhalten: Hosts ohne `analysis_reattach_v1` bekommen weiterhin keine ID.
Die Wiederanbindung nach Verbindungsverlust (gleiche ID, gleiche Parameter) bleibt
unverändert. Die übrigen Tools bleiben verzögert ladbar.

## Fortschritt

- [x] M1 `0f746923`, ergänzt um `46725dc0`. Die nie aufgerufene Funktion `cancel_principal`
  ist entfernt: `NodePolicy::invalidate` bricht zurückgehaltene Analyseaufträge eines
  eingeschränkten Principals bereits über den in `task()` registrierten Abbruchmarker ab.
- [x] M2 `87cf2e27`, dazu der Hilfetext von `se plugin` in `ed347872`
- [x] M3 `a520a3f3`, Graph `17943ec4`
- [x] M4 `ed347872`. Erster Lauf 38525109436 (`ed347872`): Linux und Windows je 21 von 23
  Prüfungen bestanden. Die Loopback-Analyse und -Duplikatsuche mit neuer und alter ID liefen
  unter Linux in 0,54 s. Gescheitert sind nur die zwei Prüfungen auf Warnungen in
  Batch-Dateien, beide wegen `cancel_principal` (tote Funktion seit `5098ee06`). Zweiter Lauf
  38526643502 auf `46725dc0`.
- [x] M5 Release v0.5.178 (`ad99f3a4`, Lauf 38561765530) zusammen mit S7

## Erweiterung 2026-10-14: persistente Shell-Sitzungen (R4)

Nutzerauftrag: Remote-Ausführung soll wie eine SSH-Sitzung nutzbar sein. Jeder Befehl kehrt mit
einem Ergebnis zurück, Umgebungsvariablen, Verzeichnis und Shell-Zustand bleiben aber über
aufeinander aufbauende Befehle erhalten. Der Release v0.5.178 wartet auf diesen Teil.

### B. Bestand

- Jede Ausführung ist ein eigener Prozess:
  - Share-Exec: `share/core/exec_types.rs` `ExecStart` mit `ExecCommand::Shell` bzw.
    `ExecCommand::Argv`. Linux nutzt `$SHELL -lc` bzw. `/bin/sh -lc` mit systemd-cgroup,
    Windows `cmd.exe /C` mit Job Object, Android `/system/bin/sh`.
  - SSH: pro Befehl ein Exec-Kanal (`sftp/core/command.rs` `start_command`).
  - Zustand gibt es nur über `cwd`/`env` je Aufruf.
- Beide Wege streamen stdin, stdout und stderr binär.
  - Share: `daemon::connect_exec` → `ExecIpcSession::next_event`, `ExecIpcInput::stdin/eof/cancel`.
  - SSH: `SshCommand::next_event`, `SshCommandControl::stdin/eof/cancel`.
  - Damit trägt der vorhandene Transport eine dauerhaft laufende Shell ohne Protokolländerung.
- Kein PTY. `cancel` beendet bei Share-Gegenstellen den ganzen enthaltenen Prozessbaum. Bei SSH
  gehen TERM und nach 10 s KILL an die Prozessgruppe des Befehls (OpenSSH ≥ 7.9); Server ohne
  `signal`-Unterstützung sehen nur das Schließen des Kanals (`sftp/core/command.rs`).

### C. Entscheidungen

- Sitzung = eine dauerhaft laufende Shell über einen vorhandenen Exec-Strom.
  - Jeder Befehl wird mit einer Endmarke gesendet: `\x1eSE<nonce>:<seq>:<exitcode>:<cwd>\x1e`
    auf stdout, `\x1eSE<nonce>:<seq>\x1e` auf stderr. `nonce` sind 16 zufällige Hexzeichen
    je Sitzung.
  - Der Befehl gilt als fertig, wenn beide Marken da sind.
  - Ausgabe vor den Marken gehört zum Befehl. Teilmarken am Chunk-Ende werden zurückgehalten.
- Dialekte:
  - **POSIX** (Linux, Android, SSH): Start mit `bash -l`, falls vorhanden, sonst `sh -l`.
    Jeder Befehl läuft als `eval '<Befehl>'` in der laufenden Shell. Damit bleiben `export`,
    `cd`, Variablen, Funktionen und `source` erhalten.
    - stdin: `</dev/null` (Standard), Heredoc mit Nonce-Begrenzer (Parameter `stdin`) oder
      durchgereicht (`se shell` interaktiv).
  - **PowerShell** (Windows-Gegenstellen, Windows-OpenSSH): Start mit
    `powershell.exe -NoLogo -NoProfile -NonInteractive -Command -`.
    - Jeder Befehl läuft als dot-gesourcter Scriptblock aus Base64-UTF-8, Ausgabe über
      `Out-Default` vor der Marke.
    - Exitcode: `$LASTEXITCODE`, sonst 1 bei neuem Eintrag in `$Error` oder einer Ausnahme,
      sonst 0.
- Plattformerkennung beim Öffnen: Ein kurzer Probe-Befehl `echo %OS% && command -v bash`
  zeigt `Windows_NT` (cmd.exe) und den bash-Pfad.
- Bedienwege:
  - MCP: `exec` bekommt `session: <name>`. Die Sitzung entsteht beim ersten Aufruf und lebt
    bis `sessions {close}` oder bis zum Ende der MCP-Verbindung.
    - `cwd`/`env` ändern den Sitzungszustand vor dem Befehl.
    - Ergebnis: `exit_code`, `cwd`, `session_ended`.
    - Neues Tool `sessions` (auflisten, schließen).
  - CLI: `se shell [ZIEL] [--cwd] [--env]`.
    - Interaktiv: Prompt `ziel:cwd$` auf stderr. Eingaben während eines laufenden Befehls
      gehen an diesen Befehl. Exitcode ungleich 0 wird angezeigt.
    - Mit Pipe: eine Zeile je Befehl, abschließendes `\` setzt fort.
    - Der Exitcode von `se shell` ist der des letzten Befehls oder des `exit N` der Shell.
- Grenzen dieser Fassung:
  - kein PTY;
  - `job_cancel`, ein Timeout oder Strg+C beenden die ganze Sitzung, weil der Exec-Strom
    keinen Signalweg zum einzelnen Befehl hat;
  - keine Wiederanbindung nach Verbindungsende.
- Verworfen: Host-seitige Sitzungen mit neuem Protokoll, PTY und Wiederanbindung. Das braucht
  Updates auf beiden Seiten und PTY-Code je Plattform (openpty, ConPTY); aufgeschoben als
  `EXEC-PTY` in `docs/TODO.md`.

### D. Arbeitsplan

| Nr. | Meilenstein | Dateien | Erwartetes Ergebnis |
|---|---|---|---|
| S1 | Sitzungskern | `cli/shell_session.rs`, `cli/shell_session/{marker,dialect,engine,transports}.rs` | Lokale bash- bzw. PowerShell-Sitzung: `export`/`cd`/Variable bleiben über Befehle erhalten; Exitcodes 0, 1 und `exit 3` (Sitzungsende); Ausgabe ohne abschließenden Zeilenumbruch; getrenntes stderr; mehrzeiliger Befehl; Heredoc-stdin (POSIX) |
| S2 | MCP | `cli/mcp/exec_tool.rs`, `cli/mcp/exec_session_job.rs`, `cli/mcp/shell_registry.rs`, `cli/mcp/catalog.rs`, `cli/mcp/server.rs`, `cli/mcp/session.rs` | `exec {session}` auf SSH-Remote und Share-Gegenstelle: zweiter Befehl sieht `export`/`cd` des ersten; `sessions` listet und schließt; Ende der MCP-Verbindung beendet die Shells |
| S3 | CLI | `cli/shell.rs`, `cli/mod.rs` | `se shell @ssh < Zeilen` liefert den Zustand über Zeilen und den Exitcode von `exit 4` |
| S4 | Doku | `README.md`, `docs/AI_AGENT_PLUGIN.md`, `SKILL.md`, `docs/TODO.md` | Verhalten und Grenzen beschrieben |
| S5 | Suite | `native/test-analysis-plugin-task.py` | S1 bis S3 auf Linux (SSH-Container, Share-Paar) und Windows (PowerShell lokal, Share-Paar) |

## Erweiterung 2026-10-14: gegenseitiges Anhängen von Shells und Sessions (R4, Teil 2)

Nutzerauftrag: Die Exec-Pläne umsetzen, ergänzt um „shells sollen sich auch an ne session hängen
können und umgekehrt“; die Umsetzung ist sicherheitsrelevant. Dieser Abschnitt ergänzt R4 und
ersetzt die Dateilisten der Tabelle D von R4 durch die Tabelle unten.

### B. Bestand

- R4 legt eine Sitzung in den `se mcp`-Prozess; andere Prozesse haben keinen Zugang. Der einzige
  Zugriff auf laufende Jobs ist `job_status`/`job_input`/`job_cancel` innerhalb desselben Servers.
- Lokale Authentifizierung im Bestand: Daemon-IPC über Loopback-TCP mit Token aus einer
  besitzergeschützten Datei, Vergleich in konstanter Zeit (`daemon/os/shared/ipc.rs`).
- Share-Gegenstellen erlauben je Gerät zwei gleichzeitige Exec-Jobs
  (`share/core/exec_registry.rs` `ExecRegistryLimits`); OpenSSH erlaubt standardmäßig 10 Kanäle je
  Verbindung (`MaxSessions`).

### C. Entscheidungen

- Begriffe: Sitzung = dauerhaft laufende Shell über einen Exec-Strom, besessen von genau einem
  lokalen `se`-Prozess (`se mcp` oder `se shell`). Shell = das Terminalkommando `se shell`.
  Anhängen = ein weiterer lokaler Client tritt einer laufenden Sitzung bei.
- Shell an Sitzung: `se shell --attach NAME|ID`. Sitzung an Shell: MCP-Tool `sessions` mit
  `attach`, danach `exec` mit `session`. `se shell --list` zeigt die anhängbaren Sitzungen.
- Aufbau: Der Besitzerprozess hostet Sitzung und einen Anhänge-Dienst nur auf `127.0.0.1`. Jeder
  Client, auch das eigene Frontend, meldet sich im ersten Rahmen binnen 5 s mit einem Token an
  (32 Zufallsbytes, Vergleich in konstanter Zeit). Besitzertoken nur im Prozess, Anhängetoken nur
  in `<App-Daten>/smart_explorer/shell-sessions/<id>.json` (privat), daneben die vom Besitzer
  gehaltene Sperre `<id>.lock`; verwaiste Einträge entfernt die nächste Auflistung.
- Zustimmung: MCP-Sitzungen sind anhängbar, solange der Agent nicht `attachable: false` setzt
  (Aufsicht des Menschen über den Agenten). `se shell` ist nur mit `--attachable` anhängbar, weil die
  Shell des Menschen erhöhte Rechte oder Geheimnisse halten kann.
- Rollen: Besitzer schließt (beendet die Sitzung); Angehängte reihen Befehle ein, senden Eingabe
  nur an ihren eigenen Befehl, brechen ab und lösen sich.
- Ein- und Ausgabe: ein Befehl zur Zeit in Einreichungsreihenfolge; der Einreicher erhält seine
  Ausgabe verlustfrei, die anderen sehen Befehlstext (ohne Steuerzeichen), Ausgabe und Exitcode als
  Aktivität (bei langsamen Lesern verlustbehaftet mit Hinweis).
- Abbruch, Zeitlimit, Strg+C und `exit` beenden die Sitzung für alle; Lösen nicht. Ende des
  Besitzerprozesses oder ein Desktop-Update beenden seine Sitzungen.
- Dialekterkennung über die Probe `echo %OS%;$PSVersionTable` in der Login-Shell (cmd.exe oder
  PowerShell → PowerShell-Dialekt, sonst POSIX). PowerShell erhält je Befehl zwei Anweisungen
  (Ausführung, dann Marken), alle Literale als Base64-UTF-8. Exitcode PowerShell abweichend von C
  oben: 1 bei abbrechendem Fehler, sonst `$LASTEXITCODE` ungleich 0, sonst 1 bei einem
  Fehlerdatensatz, der nicht von einem nativen Programm stammt, sonst `$LASTEXITCODE` oder 0
  (der Vergleich mit `$Error` meldete auch selbst abgefangene Fehler).
- Grenzen: 8 angehängte Clients, 64 wartende Befehle, 1 MiB Befehl und Eingabe, Rahmen 64 KiB
  (Einreichung 8 MiB, Anmeldung 4 KiB), 16 gleichzeitige Anmeldungen, 30 s Stau eines angehängten
  Clients, 1 MiB Ausgabe zwischen Befehlen, 60 s Start, 30 s Probe, 64 KiB Aktivitätsprotokoll im
  MCP-Server. Jede Grenze hat einen Speicher-, Thread- oder Protokollgrund.
- Verworfen: Vermittlung über den Hintergrunddienst (er müsste SSH-Anmeldungen halten, neue
  IPC-Fläche im langlebigen Dienst); Unix-Socket bzw. Named Pipe mit Peer-Credentials (zwei
  Plattformimplementierungen ohne Mehrwert gegenüber dem besitzergeschützten Token); alles
  standardmäßig anhängbar (Agent könnte eine erhöhte Shell des Menschen übernehmen).

### Sicherheit

| Eigenschaft | Abwehr | Prüfung |
|---|---|---|
| Ausführungsrechte | Sitzung = gewöhnlicher Exec über dieselbe SSH-Anmeldung bzw. denselben Share-Grant; Entzug beendet die Sitzung | Suite: Grant-Entzug beendet die Sitzung |
| Lokale Anmeldung | Token im ersten Rahmen, 5-s-Frist, konstanter Vergleich, generische Ablehnung, nur `127.0.0.1` | Unit: gefälschtes Token, Müll und Schweigen am Port |
| Zustimmung | `se shell` nur mit `--attachable` gelistet | Unit und Suite: ohne `--attachable` kein Anhängen |
| Tokenvertraulichkeit | private Einträge, Listen ohne Port und Token | Unit und Suite: Listen ohne `token`/`port` |
| Eingabe-Isolation | Eingabe nur an den eigenen interaktiven Befehl | Unit |
| Einschleusung | POSIX `eval` mit Einzelquotierung und Nonce-Heredoc, PowerShell Base64 | Unit |
| Steuerzeichen | Labels und gezeigte Befehle ohne Steuerzeichen | Unit |
| Aufräumen | Abbruch beendet die Prozessgruppe bzw. den Container; Ende des Servers entfernt Sitzungen und Einträge | Suite: `pgrep` nach Abbruch, Einträge nach MCP-Ende |

Protokolle: kein Befehlstext, keine Ausgabe und kein Token in Logdateien; die Exec-Historie der
Gegenstelle bleibt redigiert (eine Sitzung erscheint als ein Exec mit Programm `<shell>`).

### D. Arbeitsplan

| Nr. | Meilenstein | Dateien | Erwartetes Ergebnis |
|---|---|---|---|
| S1 | Sitzungskern | `cli/shell_session/{types,marker,dialect,transports,engine_state,engine_state_tests,engine,local_transport}.rs` | Lokale bash- bzw. PowerShell-Sitzung: `export`/`cd`/Variable/Funktion bleiben; Exitcodes 0, 1 und `exit 3` (Sitzungsende); Ausgabe ohne Zeilenende; getrenntes stderr; mehrzeiliger Befehl; Eingabe als Heredoc bzw. `$input` |
| S6 | Anhängen | `cli/shell_session.rs`, `cli/shell_session/{wire,registry,server,client,host,tests}.rs` | gemeinsamer Zustand Besitzer↔Angehängter, Aktivität, gefälschtes Token abgewiesen, Lösen hält die Sitzung, Abbruch beendet für alle, ohne Zustimmung kein Eintrag |
| S2 | MCP | `cli/mcp.rs`, `cli/mcp/{exec_tool,exec_session_job,shell_registry,sessions_tool,session,server,catalog}.rs` | `exec {session}` auf SSH-Remote und Share-Gegenstelle: zweiter Befehl sieht `export`/`cd` des ersten; `sessions` listet, tritt bei, zeigt Aktivität, schließt; Zeitlimit 124; Ende der MCP-Verbindung beendet die Shells |
| S3 | CLI | `cli/shell.rs`, `cli/shell_repl.rs`, `cli/mod.rs`, `cli/exec.rs` | `se shell @ssh < Zeilen` liefert den Zustand über Zeilen und den Exitcode von `exit 4`; `--attach` an eine Agentensitzung; Agent an `--attachable` |
| S4 | Doku | `README.md`, `docs/AI_AGENT_PLUGIN.md`, `native/src/cli/plugin/assets/SKILL.md`, `docs/TODO.md`, `docs/ARCHITEKTUR.md` | Verhalten, Anhängen, Sicherheit und Grenzen beschrieben |
| S5 | Suite | `native/test-analysis-plugin-task.py`, `native/test-ai-plugin-task.py`, `.github/workflows/analysis-plugin-task.yml` | Linux (OpenSSH-Container, Share-Paar) und Windows (lokale PowerShell) in einem Remote-Lauf grün |
| S7 | Release | Release-Pfad | v0.5.178 mit allen Assets, zusammen mit M5 |

Zu erhaltendes Verhalten: `exec` ohne `session` und `se exec` unverändert (Exitcodes 124/125/130);
MCP-Vorrangabsatz, `alwaysLoad` nur auf den zwölf Datei-Tools; Share-Gegenstellen ohne Protokoll-
oder Grenzänderung; Schließen des MCP-Servers innerhalb der 30-s-Wartezeit des Updaters.

Recherche (abgerufen 2026-10-14): Microsoft Learn about_PowerShell_exe (`-Command -` führt die
Eingabe Anweisung für Anweisung wie am Prompt aus); PowerShell Issue #21808 (Leerzeile beendet
mehrzeilige Anweisungen bei stdin); Rust `File::try_lock` (stabil seit 1.89); tmux(1) (Socket im
benutzereigenen Verzeichnis); sshd_config(5) (`MaxSessions` 10); Jupyter-Verbindungsdatei
(Loopback-Ports, Schlüssel, nur für den Benutzer lesbar).

### Fortschritt

- [x] S1 und S6 `76192d2c`, PowerShell-Arbeitsverzeichnis `3dbc3127`
- [x] S2 `ca760a00`
- [x] S3 `54919662`
- [x] S4 `1842e425`
- [x] S5 `ebf969b0`, `50dbee44`; Lauf 38551633780 fehlgeschlagen (behoben in `3dbc3127`), Lauf 38559316038 erfolgreich
- [x] S7 Release v0.5.178 (`ad99f3a4`, Lauf 38561765530)
