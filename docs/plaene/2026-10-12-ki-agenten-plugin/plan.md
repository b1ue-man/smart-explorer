# KI-Agenten-Plugin für die Terminalversion `se` (2026-10-12)

Stand: 2026-10-12. Quelle: Nutzernachricht 1 vom 2026-10-12 (diese Sitzung). Planungstiefe 3
(neue Fähigkeit mit erstmals eingebundenem Protokoll MCP und Installation in fremde
Agent-Konfigurationen; bestehende Modulgrenzen bleiben).

## A — Erstplan aus der Nachricht

| Nr. | Ergebnis | Quelle |
|---|---|---|
| R1 | Agenten wie Claude Code oder Codex installieren ein SE-Plugin, das sie lokal an SE (Terminalversion `se`) anbindet. | Nachricht 1 |
| R2 | Über das Plugin ist die vollständige Funktionspalette nutzbar: Dateien, Zugriff auf Remotes, Remote-Ausführung. | Nachricht 1 |
| R3 | Sauber und gründlich planen, damit die Implementierung verlässlich und vollständig ist. | Nachricht 1 |
| R4 | Android ist vorerst ausgenommen (Nutzerannahme: Agenten können dort keine vom Nutzer erstellten lokalen Tools nutzen). | Nachricht 1 |

- Autorisierung: Umsetzung ist beauftragt („was ich implementiert haben will“); laut Projektregel
  folgen eine Remote-Task-Suite und ein Release. Kein Planfreigabe-Halt (Memory
  `decide-independently`, 2026-10-01).
- **Grundfunktion:** Die für Menschen gebaute Terminalbedienung (Textausgabe, Rückfragen über stdin,
  blockierende Befehle, Strg+C) muss in eine maschinenbedienbare, nicht-interaktive Schnittstelle
  übersetzt werden: strukturierte Ergebnisse, begrenzte Antwortgrößen mit Fortsetzung,
  lange Vorgänge (Kopien, Remote-Befehle) ohne Abbruch durch Host-Timeouts, ausdrückliche
  Kennzeichnung zerstörender Aktionen, keine Geheimnisse im Agentenkontext.
- **Erweitern statt ersetzen:** Alle `se`-Befehle, ihre Ausgaben und Sicherheitsgrenzen
  (`--force`, `--recursive`, `--no-preserve-root`, Alias-/Vorfahrprüfung) bleiben unverändert. Die
  Agent-Schnittstelle kommt als neuer Befehl in dieselbe Binärdatei `se` und nutzt dieselbe
  Zielauflösung, dieselben gespeicherten Verbindungen, Zugangsdaten und denselben Hintergrunddienst.

## B — Bestand (gelesen 2026-10-12, Commit `67cde29f`)

1. **`se`-Einstieg** `native/src/bin/se.rs`: interne Modi (`--sync-daemon`, `--sync-guardian`,
   Exec-Supervisor, Mount-Host, Uplink-Helfer, Windows-PATH-Registrierung), sonst
   `smart_explorer::cli::run()` (clap). Befehle in `native/src/cli/mod.rs`: `doctor`, `share …`,
   `drive …` (Windows-Laufwerksbuchstaben), `connections …`, `completions`, `update`, `ls`, `stat`,
   `cat`, `get`, `put`, `cp`, `mv`, `rm`, `mkdir`, `search`, `exec`.
2. **Zielgrammatik** `cli/target.rs::resolve`: `@label:/pfad` (gespeicherte Verbindung, Pfad unter
   ihrer Wurzel, `.`/`..` verboten), `sftp|ftp|ftps|webdav://user@host:port/pfad` (gespeicherter
   Endpunkt), `share://direct/<id>/…`, `share://room/<raum>/<gerät>/…`, `gdrive:///…`, lokale und
   UNC-Pfade. Alles läuft über `connect::resolve_endpoint` (gemeinsame Ortsgrenze laut AGENTS.md).
   `Target` trägt Namespace-/Rename-Schlüssel und `preserved_root`.
3. **Dateioperationen** `cli/ops.rs`: Ausgabe direkt mit `println!` (Tab-getrennt), keine
   Datenrückgabe. Kopieren/Verschieben über `tree_plan` → `tree_spool` → `tree_apply`
   (`cli/tree_ops.rs`), ohne Abbruch und ohne Fortschritt. Suche serverseitig, sonst begrenzter
   Fallback-Walk (1 000 000 Knoten, 128 MiB Text, Tiefe 512).
4. **Remote-Ausführung** gibt es nur auf Share-Peers: `cli/exec.rs` → `daemon::connect_exec`
   (`daemon/os/shared/exec_ipc.rs`: `ExecIpcSession::next_event` liefert `Authorized`, `Started`,
   `Stdout`, `Stderr`, `Failed`, `Terminal`; `ExecIpcInput::stdin/eof/cancel`). Vertrag
   `docs/REMOTE_EXECUTION_PLAN.md`: getrennte Exec-Berechtigung je Gerät, kein Standardlimit für
   Laufzeit oder Ausgabe, Prozessbaum-Eingrenzung (Job Objects / systemd-cgroup). Exitcodes der CLI:
   124 Timeout, 130 Abbruch, 125 Transport/Entzug.
5. **SSH-Befehlsausführung** existiert intern: `sftp/core/exec.rs` (`exec_capture` mit 30-s-Frist und
   64-KiB-Ausgabe für das Deploy des Beschleunigungs-Agents, `open_exec_streams` für dessen
   Rahmenprotokoll). Für Nutzerbefehle auf SFTP-Remotes gibt es keinen Weg.
6. **Hintergrunddienst/IPC** `daemon/os/shared/ipc_protocol.rs::IpcRequest`: Share-Befehle,
   Exec-Grants, Snapshot, `AnalyzeShare`, `OpenShare`, Exec-Stream, Exec-Jobs, Cancel, Mounts.
   Keine Sync-Befehle; Sync-Jobs startet nur GUI (`app/os/shared/sync_manual_run.rs`) bzw. Android
   (`mobile/os/shared/domains/sync_run.rs`); `syncjobs::Runner::Cli` existiert ohne Läufer.
7. **JSON-Ausgaben vorhanden:** `doctor --json`, `connections list --json`, `share exec … --json`,
   `share grants … --json`, `share exports --json`, `share discoverable … --json`.
8. **Begriffskollision:** „Agent“ bezeichnet im Bestand den SSH-Beschleunigungs-Agent
   (`se-agent/`, `agent_proto/`, `--agent`). Die neue Funktion braucht andere Namen.
9. **Installation von `se`:** Linux `install-linux.sh` legt `se` unter dem Installationsordner ab
   und verlinkt `$HOME/.local/bin/se` (Hinweis, falls nicht im PATH); Windows registriert den
   PATH über `se --install-cli-path`. Repo `b1ue-man/smart-explorer` ist öffentlich (`gh api`, 2026-10-12).
10. **Abhängigkeiten** `native/Cargo.toml`: `tokio` (rt, rt-multi-thread, net, io-util, sync, time),
    `serde`/`serde_json`, `clap` 4, `crossbeam-channel`, `ctrlc`; Edition 2021, Version 0.5.175.

11. **Desktop-Update und laufende `se`-Prozesse:** Der Updater-Helfer
    (`native/src/bin/smart_explorer_updater.rs` Z. 147–160, `…/process.rs::stop_target_processes_for_update`)
    schreibt `<appdata>/sync/daemon.stop` = `stop` und wartet höchstens 30 s, bis **jeder** Prozess
    mit dem Abbild der App **oder von `se`** beendet ist (Windows: Toolhelp-Snapshot nach Bildpfad,
    Linux: `/proc/<pid>/exe`); sonst bricht das Update mit „Smart Explorer läuft noch und blockiert
    das Update“ ab. `stop` schreibt in Produktion nur dieser Helfer; `se update` (nur Terminal) nutzt
    `handoff:<generation>` (`daemon/os/shared/handoff.rs`), `se share worker stop` geht über IPC.
    Folge: Ein dauerhaft laufender Agent-Server aus `se` muss auf `stop` reagieren und sich binnen
    der 30 s beenden, sonst blockiert er jedes Desktop-Update.
12. **Aufbau des CLI-Kopierpfads:** `tree_spool.rs::SourceSpool::collect` liest alle Quelldateien
    in einen privaten Spool (`copy_exact`, `io::copy`), `tree_apply.rs::publish_file` schreibt je
    Datei in eine eindeutige Stage und veröffentlicht ohne Ersetzen bzw. ersetzend; ein Fehler
    entfernt die eigene Stage. Abbruch-/Fortschrittspunkte passen zwischen Dateien und in
    `copy_exact`.
13. **SSH-Signale:** OpenSSH ≥ 7.9 unterstützt die Kanalanfrage `signal` (RFC 4254 §6.9) für
    Befehls-/Login-Sitzungen (nicht für Forced Command/Subsystem); `session.c::session_signal_req`
    liefert per `killpg(s->pid, sig)` an die Prozessgruppe, `do_exec_no_pty` ruft `setsid()`;
    Namen HUP, INT, KILL, QUIT, TERM, USR1, USR2 (Quelle: openssh-portable `session.c` master,
    abgerufen 2026-10-12; OpenSSH-7.9-Release, Commit Oktober 2018, bz#1424). russh 0.61.2
    (`Cargo.lock`): `Channel::exec`, `signal(Sig)`, `data`, `eof`, `close`, `wait() ->
    Option<ChannelMsg>` mit `Data`, `ExtendedData{ext}`, `ExitStatus{exit_status}`,
    `ExitSignal{signal_name,…}`, `Eof`, `Close` (docs.rs 0.61.2, abgerufen 2026-10-12).
14. **SFTP-Handle für Befehle:** `connect/os/shared/connector.rs::connect_sftp` hält
    `RemoteState.sftp: Some(Arc<SftpBackend>)`; `open_saved_at` gibt nur das `BackendHandle` zurück.

Lücken: (L1) keine maschinenlesbare Schnittstelle für Datei-/Remote-Operationen, (L2) kein Abbruch
und kein Fortschritt im CLI-Kopierpfad, (L3) keine Remote-Ausführung auf SFTP-Remotes,
(L4) keine Plugin-/Konfigurationsdateien für Agent-Hosts, (L5) keine Installationsbefehle,
(L6) ein langlebiger `se`-Prozess blockiert Desktop-Updates (B11).

## Kreative Schleife K1 — Zuschnitt der Agent-Schnittstelle

- **Ziel:** R1/R2 — wie wird die `se`-Funktionspalette für Agent-Hosts installierbar und vollständig
  bedienbar? Offene Fragen: Protokoll/Transport, Toolzuschnitt, Modell für lange Vorgänge,
  Ausgabegrenzen, Geheimnisse, Paketierung je Host, Remote-Ausführung auf SFTP-Remotes.
- **Kriterien:** (K1.1) Claude Code und Codex können installieren (Nachweis: dokumentierter
  Host-Vertrag + Installationsbefehl); (K1.2) jede `se`-Funktion ist erreichbar (Nachweis:
  Abdeckungstabelle Befehl → Tool); (K1.3) lange Vorgänge überleben Host-Timeouts und sind
  abbrechbar (Nachweis: Hostdoku + Jobmodell); (K1.4) Antworten bleiben unter den Ausgabegrenzen
  der Hosts (Nachweis: Hostdoku); (K1.5) bestehendes CLI-Verhalten unverändert (Nachweis: Suite);
  (K1.6) keine Passwörter im Agentenkontext (Nachweis: Entwurf); (K1.7) Windows und Linux.
- **Status:** aktiv — externe Recherche zu MCP-Spezifikation, rmcp, Claude Code und Codex läuft.

### Möglichkeiten (Runde 1, 2026-10-12)

| Ansatz | Weg | Zielbeitrag | Tragende Annahme / Befund |
|---|---|---|---|
| A — nur dedizierte MCP-Tools | jeder `se`-Befehl (~45) als eigenes strukturiertes Tool | beste Ergonomie, genaue Annotationen | hoher Umbau der Share-Verwaltungsbefehle (Textausgabe), jede neue CLI-Funktion braucht ein neues Tool → Abdeckung driftet |
| B — nur Skill, `se` über die Shell des Agenten | SKILL.md erklärt die CLI | volle Abdeckung ohne Code | Shell-Timeouts der Hosts, ungebremste Ausgaben (`cat`), kein Jobmodell; Codex-Sandbox (zu prüfen) sperrt Netz/IPC für Shellbefehle → K1.3/K1.4 nicht erfüllt |
| C — Hybrid | MCP-Server `se mcp` (stdio) mit dedizierten Tools für Dateien, Übertragungen, Remote-Ausführung, Verbindungen, Status + ein generisches Tool für jeden übrigen `se`-Befehl + Skill | volle Abdeckung jetzt und für künftige Befehle; strukturierte, begrenzte Ergebnisse dort, wo Menge/Laufzeit zählen | generisches Tool hat nur grobe Annotation (gilt als potenziell zerstörend) |
| D — HTTP-MCP im Hintergrunddienst | Daemon bietet Loopback-HTTP-Endpunkt | Jobs überleben Agent-Sitzungen | Token-/Origin-Schutz nötig, Daemon muss laufen, größere Angriffsfläche; Mehrwert nicht verlangt |
| E — Tools automatisch aus dem clap-Baum | jeder Unterbefehl als Tool, Ausführung als Kindprozess | automatische Abdeckung | ~50 Tools Kontextlast, nur Textausgabe, Abbruch per Signal plattformabhängig |

Vorläufige Bewertung: C erfüllt K1.2–K1.6 am vollständigsten (A verfehlt K1.2 auf Dauer, B
K1.3/K1.4, D/E bringen Nachteile ohne beauftragten Mehrwert). Offen bis zur Recherche: Codex-
Paketierung und -Timeouts, MCP-Protokollversion, rmcp gegen Eigenimplementierung.

## C — Entscheidungen

- **C1 Protokoll/Transport:** MCP über stdio, gestartet vom Agent-Host als `se mcp`. Beide
  Zielhosts unterstützen stdio-MCP (Claude Code: `refs/claude-code-plugins-mcp-2026-10-12.md` §1–3).
  Verworfen: HTTP im Daemon (Ansatz D: Token/Origin-Schutz, Daemonzwang, ohne beauftragten
  Mehrwert), reiner Skill (Ansatz B: K1.3/K1.4 verfehlt).
- **C2 Bibliothek:** `rmcp` 3.5.1 (offizielles SDK, Apache-2.0, MSRV 1.88), `default-features =
  false`, Features `server`, `transport-io`; nur für Nicht-Android-Ziele. Ausschlaggebend: Codex startet
  mit `initialize` `2025-06-18`, Claude Code v2 probt `server/discover` (`2026-07-28`, zustandslos);
  rmcp bedient beide Lebenszyklen auf derselben stdio-Verbindung (`service/server.rs` Z. 572–690),
  setzt `resultType`/Cache-Felder je ausgehandelter Version und führt Anfragen nebenläufig aus
  (`spawn_service_task`). Verworfen: Eigenimplementierung (zwei Protokollgenerationen, hohes
  Fehlerrisiko ohne lokalen Compiler), `rust-mcp-sdk` 2.x (nur 2026-07-28 → Codex scheitert),
  `pmcp`/`turbomcp` (Drittanbieter ohne Mehrwert). Tool-Schemas werden von Hand als JSON gebaut
  (keine Makros), damit Beschreibungen, Annotationen und Schemas genau kontrolliert sind.
- **C3 Zuschnitt (Ansatz C):** dedizierte Tools `status`, `connections`, `list`, `stat`, `read`,
  `write`, `edit`, `mkdir`, `remove`, `copy`, `move`, `search`, `exec`, `job_status`, `job_input`,
  `job_cancel` und ein generisches Tool `se` für jeden weiteren `se`-Befehl (Share-Verwaltung,
  Laufwerke, Update, Verbindungen anlegen/entfernen, Diagnose). Toolnamen ohne Präfix; der Host
  stellt den Servernamen voran (Claude Code: `mcp__plugin_smart-explorer_se__<tool>`).
- **C4 Lange Vorgänge:** einheitliches Jobmodell im Serverprozess. Ein Aufruf wartet bis
  `wait_seconds` (Standard unter dem kleinsten dokumentierten Host-Timeout) und liefert entweder das
  Ergebnis oder `job_id` + Fortschritt; `job_status` liest weiter (Ausgabeseiten ab Offset),
  `job_input` schreibt stdin, `job_cancel` bricht ab. Fortschrittsmeldungen
  (`notifications/progress`) bei vorhandenem `progressToken` mindestens alle 10 s (setzen Claude
  Codes 30-min-Leerlauf-Timeout zurück). Ausgaben werden ungekürzt in Spooldateien der Sitzung
  geschrieben (kein Standardlimit, wie `se exec`), Antworten liefern Seiten daraus.
- **C5 Antwortgrößen:** Standardbudget 20 000 Byte Nutzinhalt je Antwort. Herleitung: Codex kürzt
  jede Tool-Ausgabe bei 10 000 Token, Claude Code warnt ab 10 000 Token; 20 000 Byte bleiben auch
  bei dichtem JSON (≈ 2 Zeichen/Token) darunter. `max_bytes` darf es anheben (Beschreibung nennt die
  Hostkürzung). Jede Liste/jeder Inhalt meldet `next_offset`. Weil Codex bei `structuredContent`
  nur dieses JSON sieht, ist `structuredContent` immer das vollständige Ergebnis; der Textblock
  enthält dasselbe JSON.
- **C6 Geheimnisse:** Dedizierte Tools nehmen keine Passwörter an. Das generische Tool reicht
  `stdin` nur durch (z. B. `--pin-stdin`); der Skill weist an, Passwörter nicht im Chat zu
  erfragen, sondern Schlüssel/SSH-Agent zu nutzen oder den Nutzer den Befehl selbst ausführen zu
  lassen.
- **C7 Remote-Ausführung auf SFTP-Remotes:** neuer SSH-Exec-Pfad auf der bestehenden
  authentifizierten Sitzung (`sftp/core`), getrennt stdout/stderr, Exitstatus/Signal, stdin,
  Abbruch = `signal TERM` an die Prozessgruppe (B13), nach 10 s (Vorgabe wie `docker stop`)
  `signal KILL`, dann Kanal schließen. Gleicher Weg für `se exec @label -- …` in der CLI.
  `argv`-Form wird POSIX-gequotet (dokumentiert; Windows-OpenSSH-Ziele nutzen die Shell-Form).
- **C8 Paketierung:** Claude Code — `se plugin install claude` erzeugt einen lokalen Marketplace
  unter den App-Daten (Plugin mit `.mcp.json` = absoluter Pfad der installierten `se`-Datei + `mcp`,
  Skill) und ruft `claude plugin marketplace add` + `claude plugin install … -s user -y`. Lokale
  Marketplace-Plugins werden bei jedem Sitzungsstart aus dem Ordner geladen, daher aktualisiert
  `se mcp` beim Start veraltete erzeugte Dateien (Versionsvergleich). Codex — `se plugin install
  codex` ruft `codex mcp add smart-explorer [--env …] -- <absoluter se-Pfad> mcp` und legt den Skill
  unter `$HOME/.agents/skills/smart-explorer/SKILL.md` ab. Verworfen für Codex: Agent-Plugin-Format
  (stdio-`command` nur als einzelnes Token/Plugin-relativ und Kopie in den Cache → hinge am PATH).
  Unter Linux übergibt die Installation die bei der Installation gesetzten `XDG_DATA_HOME`,
  `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS` als `--env`, weil Codex sie herausfiltert (App-Daten-
  ort, Start des Hintergrunddienstes). Weitere Hosts (Cursor, VS Code, Gemini/Antigravity,
  OpenCode, beliebige MCP-Clients): `se plugin config <host>` gibt die passende Konfiguration mit
  absolutem Pfad aus.
- **C9 Namen:** `se mcp` (Server), `se plugin …` (Installation), Rust-Module `native/src/cli/mcp/`
  und `native/src/cli/plugin/` (Teil der Terminalversion; `pub(super)`-Funktionen der CLI bleiben
  ohne Sichtbarkeitsumbau nutzbar). Servername im Claude-Plugin `se`, in Codex `smart-explorer`.
  „agent“ bleibt dem SSH-Beschleunigungs-Agent vorbehalten (B8).
- **C10 Protokollkanal schützen:** `se mcp` dupliziert stdout für das Protokoll und leitet den
  Prozess-stdout auf stderr um (Unix `dup`/`dup2`, Windows `DuplicateHandle`/`SetStdHandle`), damit
  kein `print!` den JSON-RPC-Strom zerstört.
- **C11 Update-Verträglichkeit (L6):** `se mcp` beendet sich geordnet (Jobs abbrechen,
  Log-Benachrichtigung), sobald `daemon.stop` nach seinem Start auf `stop` gesetzt bzw. neu
  geschrieben wird; ein beim Start bereits vorhandener Altwert wird ignoriert. `handoff:<gen>`
  (Terminal-Update) betrifft den Server nicht. Prüfintervall 1 s innerhalb der 30-s-Frist des Helfers.
- **C12 Lokale Pfade:** Im MCP-Server nur absolute lokale Pfade (relative Pfade hängen vom
  Startordner des Hosts ab); Fehlermeldung nennt die Korrektur.
- **C13 Verbindungswiederverwendung:** Innerhalb einer Sitzung werden geöffnete Backends je
  Verbindung wiederverwendet (SFTP-Handshake je Aufruf entfällt), bei Transportfehlern verworfen.
- **C14 Android (R4):** kein `se mcp` auf Android. Prüfung „übertragene Lösung“: Agenten in Termux
  könnten zwar lokale Werkzeuge starten, aber SE-Android-Daten sind app-privat; ein Anschluss
  bräuchte eine exportierte App-IPC. Als offene Bewertung in `docs/TODO.md`.
- **C15 Tool-Annotationen:** nur lesende Tools `readOnlyHint: true` (Codex fragt dann nicht);
  `write`, `edit`, `copy`, `move`, `remove`, `exec`, `se` `destructiveHint: true`; `mkdir`,
  `job_input`, `job_cancel` `destructiveHint: false`; entfernte Ziele `openWorldHint: true`.
  Kein `anthropic/requiresUserInteraction`: Freigaben bleiben Sache der Host-Einstellungen.

## D — Arbeitsplan (Meilensteine, Abnahme in der einen Remote-Suite)

Zu erhaltende Verträge (Suite prüft sie mit): alle bestehenden `se`-Befehle und ihre Ausgaben
(Text und `--json`) unverändert; `se exec` auf Share-Peers unverändert (Exitcodes 124/125/130,
Hilfetexte der vorhandenen Tests); Zielgrammatik und Schutzregeln von `cp/mv/rm`
(`--force`, `--recursive`, `--no-preserve-root`, Alias-/Vorfahrprüfung, Baumbudgets); Desktop-Update
bleibt durch laufende `se mcp` unblockiert; Android-Bibliothek baut weiter (rmcp nur
Nicht-Android); Release-Artefaktsatz unverändert (Plugin-Dateien stecken in `se`).

| Nr. | Ergebnis | Dateien | Abnahme (Suite) |
|---|---|---|---|
| M1 | CLI-Datenfunktionen ohne Ausgabeänderung: Suche über Senke, `connections_value`, `doctor_value`, Share-`status_value` | `cli/ops.rs`, `cli/connections/output.rs`, `cli/doctor.rs`, `cli/share/status.rs` | bestehende CLI-Tests grün; `mcp_task_*`-Tests: CLI-JSON == Wertfunktion |
| M2 | Abbruch und Fortschritt im Baumkopierpfad (Beobachter, Abbruch je 64-KiB-Block und zwischen Dateien, eigene Stage wird entfernt) | neu `cli/tree_observer.rs`; `tree_spool.rs`, `tree_apply.rs`, `tree_ops.rs`, `ops.rs` | Abbruch hinterlässt keine Stage, veröffentlichte Dateien bleiben vollständig; Zähler stimmen |
| M3 | SSH-Befehle auf SFTP-Remotes: Start, stdout/stderr getrennt, Exitstatus/Signal, stdin, Abbruch TERM→10 s→KILL→Kanal schließen; `se exec @label …`; Vervollständigung nennt SFTP-Remotes | neu `sftp/core/command.rs`, `connect/os/shared/ssh_session.rs`, `cli/exec_ssh.rs`; `cli/exec.rs`, `cli/completions*.rs` | Quoting-/Befehlsaufbau-Unit-Tests; Linux-E2E gegen OpenSSH-Container: Ausgaben, Exitcode, stdin, `--env`, `--cwd`, Timeout 124, Abbruch beendet Prozessgruppe |
| M4 | MCP-Kern `se mcp`: rmcp-Server, Toolkatalog mit Schemas/Annotationen, Anweisungen, geschützter Protokollkanal, Update-Stopp, Sitzungsordner mit Sperre und Aufräumen | neu `cli/mcp/{mod,server,catalog,args,response,session,update_stop}.rs`, `cli/mcp/os/{mod,linux_os,windows}.rs`; `cli/mod.rs`, `Cargo.toml`, `Cargo.lock`, `daemon/os/shared/handoff.rs` | E2E: `initialize` 2025-06-18 und `server/discover` 2026-07-28 liefern denselben Katalog; Testausgabe per `println!` landet auf stderr; `daemon.stop`=`stop` nach Start → Ende < 30 s, Altwert beim Start ignoriert; verwaiste Sitzungsordner werden entfernt |
| M5 | Jobmodell: Register, Spool-Ausgaben, Warten mit Fortschritt/Abbruch, `job_status`/`job_input`/`job_cancel`, Sitzungsende bricht Jobs ab | neu `cli/mcp/{jobs,spool,wait}.rs` | Unit: Seiten an UTF-8-Grenzen, Wartezeit → `running` + `job_id`, Abbruch; E2E: Job ohne bekannte ID über `job_status` auffindbar |
| M6 | Dateitools `list/stat/read/write/edit/mkdir/remove/copy/move/search` über CLI-Pfade, Backend-Wiederverwendung, `etag`-Vorbedingung, absolute lokale Pfade | neu `cli/mcp/{targets,file_tools,file_write,transfer_tools,search_tool}.rs` | E2E lokal (Linux/Windows) und SFTP (Linux): Inhalte, Seiten, Binärhinweis, `etag`-Konflikt, Wurzelschutz, Kopieren in Ordner wie `se cp`, Verschieben |
| M7 | `exec` über Share-Peers und SFTP-Remotes als Job | neu `cli/mcp/{exec_tool,exec_share,exec_ssh}.rs` | Unit: Ereignis→Jobzustand; E2E Linux: SSH (M3) und Share (zwei gekoppelte Clients mit lokalem Relay wie `test-share-lifecycle-e2e.sh`, Exec-Grant) inkl. Abbruch |
| M8 | `status`, `connections`, generisches Tool `se` (Kindprozess als Job, `mcp` verweigert, geordnetes Beenden SIGINT/CTRL_BREAK → 10 s → Kill) | neu `cli/mcp/{status_tools,cli_tool}.rs`, `cli/mcp/os/*` | E2E: `se`-Tool liefert dieselbe JSON wie der direkte Aufruf; Verweigerung `mcp`; Abbruch eines lang laufenden Kindes |
| M9 | `se plugin install|uninstall|status|config` für Claude Code und Codex, Skill, Auffrischung beim Serverstart | neu `cli/plugin/{mod,files,claude,codex,config,host_cli}.rs`, `cli/plugin/assets/SKILL.md` | E2E mit echten npm-CLIs (`@anthropic-ai/claude-code`, `@openai/codex`, gepinnt) unter Linux und Windows: installiert/gelistet/validiert, Codex-Eintrag zeigt absoluten Pfad + `mcp`, Deinstallation entfernt nur eigene Dateien, Versionswechsel frischt Dateien auf |
| M10 | Doku: README (Abschnitt KI-Agenten), `docs/AI_AGENT_PLUGIN.md`, ARCHITEKTUR (Einstiege/Datenwege), TODO (Android/Termux-Bewertung, Windows als SSH-Ziel ohne E2E), GOTCHAS (rmcp-MSRV, Codex-Umgebungsfilter) | Doku | statische Prüfung im Suite-Lauf (Links/Begriffe) |
| M11 | Eine Remote-Suite `ai-plugin-task.yml` → `native/test-ai-plugin-task.py` (Linux, Windows, Android-Build) | neu | ein Lauf deckt M1–M10 ab |
| M12 | Graph-Refresh, Commits, Push, Suite, Fixschleife, ein Release | — | GitHub-Release mit vollständigem Asset-Satz |

Reihenfolge: M1 → M2 → M3 → M4 → M5 → M6 → M7 → M8 → M9 → M10 → M11 → M12. Testpräfix:
`mcp_task_`. Keine lokalen Builds; Prüfung ausschließlich über M11.
