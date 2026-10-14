# 2026-10-14 Shell-Sitzungen mit echtem Terminal wie SSH

Stand: 2026-10-14. Repository-Stand bei Planbeginn: Zweig `main`, HEAD `a24b89a1cc260e67f567caf0c944508451a892f0`. Am Ende der Planung: HEAD `63733be530af9cfb26946a09c762e7d2f7abce9d` (drei Commits des Batches „Ordner löschen“ nach Planbeginn).

## Nutzerauftrag

Nachricht 1 (2026-10-14):

> further notice, shell is not a true shell, it is more like a forwarded textlog of the shell. if i connect via ssh it is truly like using the shell, with formating, shell programms working like claude, codex, nvim etc. this is what i aimed for. this may be a safety relevant

Nachricht 2 (2026-10-14):

> with tab support, stuff like this, i expect a full shell support

Ziel: `se shell` verhält sich in einem Terminal wie `ssh HOST`: Die Gegenstelle bekommt ein Pseudo-Terminal (PTY), die interaktive Login-Shell der Gegenstelle läuft darin, das lokale Terminal ist im Rohmodus und reicht Tastendrücke und Bildschirmausgabe unverändert durch. Damit funktionieren Formatierung (Farben, Cursorsteuerung), interaktive und Vollbildprogramme (claude, codex, nvim und jedes andere Programm, das ein Terminal verlangt), Tab-Vervollständigung der Gegenstellen-Shell, Passwortabfragen, Strg+C an den laufenden Vordergrundbefehl und Fenstergrößenänderungen. Das gilt für beide Zielarten von `se shell`: gespeicherte SFTP-Remotes (SSH-Kanal) und Share-Gegenstellen mit Exec-Freigabe (Linux-, Windows- und Android-Hosts).

Aus den beiden Nachrichten abgeleitete Anforderungen:

| Nr. | Anforderung | Quelle |
|---|---|---|
| R1 | Die Shell ist eine echte Shell wie über SSH, kein weitergeleitetes Textprotokoll | Nachricht 1: „shell is not a true shell, it is more like a forwarded textlog of the shell. if i connect via ssh it is truly like using the shell“ |
| R2 | Formatierung der Ausgabe wie bei SSH | Nachricht 1: „with formating“ |
| R3 | Interaktive und Vollbildprogramme funktionieren, genannt claude, codex, nvim | Nachricht 1: „shell programms working like claude, codex, nvim etc.“ |
| R4 | Tab-Unterstützung | Nachricht 2: „with tab support“ |
| R5 | Volle Shell-Unterstützung | Nachricht 2: „i expect a full shell support“ |
| R6 | Die Umsetzung wird als möglicherweise sicherheitsrelevant behandelt | Nachricht 1: „this may be a safety relevant“ |

## A. Annahmen

Der Nutzer hat vier Rückfragen nicht beantwortet: die Bedeutung von „tab support“, den Umfang von „full shell support“, welche Programme laufen müssen, und welche Sicherheitssorge er hat. Es liegen nur die zwei Nachrichten vor. Der Plan löst die Fragen über den Maßstab „wie SSH“, den der Nutzer in Nachricht 1 selbst setzt. Jede Annahme gilt nur so weit, wie sie hier steht.

- A1 (zu „tab support“): Annahme: Gemeint ist die Tabulatortaste in der Sitzung, die Befehle und Pfade auf der Gegenstelle vervollständigt, so wie bei `ssh HOST`. Umsetzung: Die Vervollständigung leistet die Shell der Gegenstelle selbst (bash/zsh-Readline, PowerShell PSReadLine), sobald sie in einem PTY läuft. Reiter in der Desktop-App und mehrere Sitzungen in einem Terminal gehören nicht dazu.
- A2 (zum Umfang von „full shell support“): Annahme: Umfang ist das, was `ssh HOST` in einem Terminal bietet. Enthalten: Passwortabfragen und Strg+C an den Vordergrundbefehl, beides folgt aus dem PTY. Nicht enthalten: Wiederanbinden nach Verbindungsverlust, weil `ssh` das nicht kann; es bleibt als offener Punkt in `docs/TODO.md`.
- A3 (zu den Programmen): Annahme: Jedes Programm, das ein Terminal verlangt. Die Suite prüft stellvertretend nvim, die TUI-Programme claude und codex, eine Passwortabfrage über `/dev/tty` und den Rohmodus von stdin.
- A4 (zur Sicherheit): Annahme: Keine konkrete Sorge des Nutzers ist bekannt. Der Plan prüft die Sicherheit nach den Projektregeln und prüft ausdrücklich, wer eine Sitzung nutzen oder mitnutzen darf. Der Abschnitt „Sicherheit“ enthält beides.
- A5: Terminalmodus nur, wenn stdin und stdout von `se shell` Terminals sind. Ohne Terminal bleibt der bisherige Befehlsmodus unverändert.
- A6: Der Nutzer will den echten Terminalmodus als Standard. `ssh` fordert ebenfalls standardmäßig ein PTY an, wenn stdin ein Terminal ist (OpenSSH clientloop.c, siehe „Recherche“).

Ergebnis: Keine Angabe fehlt, die nur der Nutzer liefern kann. Der Plan geht ohne Nutzerentscheidung weiter.

## B. Bestand

Alle Angaben wurden an den genannten Dateien gelesen. „Neu geprüft“ kennzeichnet Ergänzungen, die während der Planung hinzukamen.

- B1 Hilfe von `se shell`: `native/src/cli/shell.rs` Zeilen 27-28 („There is no PTY …“, „Ctrl+C ends the whole session …“). Der Test `shell_session_task_cli_parses_shell_sessions_and_attach` in `native/src/cli/mod.rs` Zeilen 420-447 verlangt die Wörter `--attachable`, `--attach`, `--list`, `no PTY`, `Ctrl+C` in der Hilfe.
- B2 Zeilenweise Eingabe: `native/src/cli/shell_repl.rs` `read_lines` (Zeilen 288-307); Strg+C über `ctrlc::set_handler` beendet die Sitzung (Zeilen 76-79, 262-267).
- B3 Befehlssitzungen über Marken: `native/src/cli/shell_session/dialect.rs` (POSIX `eval` mit Marken, PowerShell Base64-Scriptblöcke), `engine_state.rs` (Warteschlange, Marken), `engine.rs` (Teilnehmer, Fan-out). Diese Maschinerie setzt einen Pipe-Strom ohne Terminal voraus.
- B4 Transporte: `transports.rs` `TransportEvent` (Stdout, Stderr, Ended), `TransportInput` (stdin, eof, cancel) — kein Größen- oder Terminalfeld.
- B5 SSH: `native/src/sftp/core/command.rs` `start_command` (Exec-Kanal, `want_reply`, Pump mit `Control::{Stdin, Eof, Cancel}`), `native/src/sftp/core/exec.rs` `request_exec`. Kein `request_pty` im Quellbaum (`rg -n request_pty native/src`: keine Treffer, neu geprüft). russh 0.61.2 bietet `request_pty`, `request_shell`, `window_change` (neu geprüft in der Crate-Quelle).
- B6 Share-Exec: Protokoll `native/src/share/core/exec_protocol.rs` (ALPN `smart-explorer/share-exec/2`, Rahmen START/STDIN/STDIN_EOF/CANCEL/PING/RESULT_ACK, `ServerProtocolState`), Typen `exec_types.rs` (`EXEC_PROTOCOL_VERSION = 2`, `EXEC_CAPABILITY = "exec_stream_v2"`, `ExecStart` ohne `deny_unknown_fields`), Host `exec_admission.rs` → `exec_server.rs` → `exec_job.rs` → `exec_platform.rs` → Supervisor-Protokoll `exec_supervisor_protocol.rs` (START/STDIN/STDIN_EOF/CANCEL) → Supervisor je Plattform: Linux und Android `share/os/linux_os/exec_supervisor.rs` (Pipes, Zeilen 206-210), Android-Zwischenprozess `share/os/android/exec_contain.rs` (`clone` im `pre_exec`, `setpgid` im Root), Windows `share/os/windows/exec_supervisor.rs` (Pipes, Kindprozess im Job Object). Client `exec_client.rs` (Handshake, `ExecHelloOk`, START), `exec_client_active.rs` (`ExecClientInput::{Stdin, StdinEof, Cancel}`). Neu geprüft: Der Server prüft `start.digest()` gegen den gesendeten Digest (`exec_protocol.rs` Zeilen 124-128); ein alter Host, der ein unbekanntes Feld verwirft, lehnt den Start daher ab.
- B7 Daemon-IPC: `native/src/daemon/os/shared/exec_ipc.rs` (Eingabe-Tags 1-3, Ereignis-Tags 16-21). Neu geprüft: `ensure_worker_ready` ersetzt einen Hintergrunddienst anderer Version (`ipc_client.rs` Zeilen 181-203, 325-365); CLI und Dienst haben deshalb stets dieselbe Version.
- B8 Anhängedienst: `server.rs` (nur 127.0.0.1, Token im ersten Rahmen, 5 s, 16 Anmeldungen, `constant_time_eq`, Version muss gleich `PROTOCOL_VERSION = 1` sein), `registry.rs` (private Einträge `<App-Daten>/shell-sessions/<id>.json` mit Sperrdatei, `Entry` ohne `deny_unknown_fields`), `client.rs`, `wire.rs` (Rahmengrenzen 64 KiB Daten, 4 KiB Hello, 8 MiB Submit).
- B9 MCP: `cli/mcp/shell_registry.rs` (`get_or_open`, `attach`, `list`, `activity`, `close_all`), `sessions_tool.rs`, `exec_session_job.rs`; Katalogtext „no PTY“ in `cli/mcp/catalog.rs` Zeile 138; Skill `cli/plugin/assets/SKILL.md` Zeilen 51-57 und 70-72.
- B10 Lokale Terminal-Adapter: `cli/os/linux_os.rs` (`read_hidden_line` mit `tcgetattr`/`tcsetattr`), `cli/os/windows.rs` (`read_hidden_line` mit `GetConsoleMode`/`SetConsoleMode`). `cli` wird für Android nicht gebaut (`native/src/lib.rs` Zeilen 17-18, neu geprüft).
- B11 Abhängigkeiten: `libc 0.2.186` (nicht-Windows, auch Android) mit `posix_openpt`, `grantpt`, `unlockpt`, `ptsname_r`, `openpty`, `TIOCSCTTY`, `TIOCSWINSZ`; `windows-sys 0.59` mit `Win32_System_Console`, `Win32_System_Threading`, `Win32_System_LibraryLoader`, `Win32_System_Pipes` bereits aktiviert. `CreatePseudoConsole` ist in windows-sys ein Ladezeit-Import aus kernel32.dll. Keine neue Crate nötig.
- B12 Release-Ziele: `se` Linux ist `x86_64-unknown-linux-musl`, die Desktop-App `x86_64-unknown-linux-gnu` mit glibc-2.17-Basis (`docs/RELEASING.md` Zeilen 594-598, neu geprüft). Unter glibc vor 2.34 liegt `openpty` in libutil; `posix_openpt`/`grantpt`/`unlockpt`/`ptsname_r` liegen in libc selbst.
- B13 Vorhandene Entscheidung gegen PTY: `docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md` Zeilen 158-160 und `docs/TODO.md` Zeile `EXEC-PTY` (Status `later`). Dieser Auftrag ersetzt diese Entscheidung (siehe E1).
- B14 Suite-Vorlage: `.github/workflows/analysis-plugin-task.yml` → `native/test-analysis-plugin-task.py`, Hilfen aus `native/test-ai-plugin-task.py` (Ergebnisse, OpenSSH-Container `lscr.io/linuxserver/openssh-server`, Share-Paar über `share-server`, MCP-Client). Share-Paare laufen dort nur unter Linux.

## Prüfung je Plattform

| Rolle und Plattform | Passt der Ansatz? | Begründung und Grenze |
|---|---|---|
| Share-Host Linux | ja | POSIX-PTY im Supervisor innerhalb der systemd-Unit; Unit ohne `PrivateDevices` (`exec_systemd.rs` Zeilen 50-66), `/dev/ptmx` erreichbar; cgroup beendet auch Prozesse, die den Slave offen halten |
| Share-Host Android | ja, mit Gerätelücke | `create_pty(untrusted_app_all)` erlaubt Apps PTYs; `setsid`/`TIOCSCTTY` im Root nach `clone`, nie im Subreaper; Phantomprozess-Grenze und Einfrieren im Hintergrund gelten wie für jeden Exec (`docs/refs/android-child-processes.md`); kein Gerätetest in der Suite (O1) |
| Share-Host Windows | ja ab Windows 10 1809 / Server 2019 | ConPTY im Supervisor im Job Object; ältere Systeme melden keine Fähigkeit (Laufzeit-Laden), Client fällt zurück |
| SSH-Ziel Linux/Unix (OpenSSH) | ja | pty-req + shell wie `ssh`; `PermitTTY no`/`no-pty` → Rückfall |
| SSH-Ziel Windows (Win32-OpenSSH) | ja laut Protokoll, ungeprüft | sshd nutzt ConPTY und seine Standard-Shell; kein Testziel im Runner (`AIPLUG-WINSSH`) |
| Client Linux | ja | termios-Rohmodus, SIGWINCH; Terminal-Emulator beliebig |
| Client Windows | ja in Konsolen mit VT (Windows Terminal, conhost ab Windows 10) | `ReadConsoleInputW` mit VT-Eingabe wie Win32-OpenSSH; mintty/Git Bash geben eine Pipe als stdin → Zeilenmodus (O3) |
| Client Android | entfällt | `cli` wird für Android nicht gebaut (`lib.rs` Zeilen 17-18) |
| Agenten (MCP) | bewusst nicht | E7 |

## C. Entscheidungen

Jede Entscheidung nennt die verworfenen Alternativen.

- E1 Echtes PTY auf der Gegenstelle für `se shell` im Terminal (ersetzt die Entscheidung in `docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md` Zeilen 158-160 und die TODO-Zeile `EXEC-PTY` für den PTY-Teil). Verworfen: Lokale Zeileneditor-Emulation mit Vervollständigung über eigene Remote-Abfragen — erfüllt R3 nicht (Vollbildprogramme brauchen ein PTY) und dupliziert die Shell der Gegenstelle.
- E2 Zwei Sitzungsarten: „Befehlssitzung“ (bestehend, Marken, strukturierte Ergebnisse für Agenten) und „Terminalsitzung“ (neu, PTY, Byte-Strom). Verworfen: Marken über Shell-Integration in einem PTY für beide Zwecke — fragil (Prompt-Erkennung, Tastaturpuffer des Menschen) und unsicher (Eingabe in ein laufendes Vollbildprogramm).
- E3 Auswahl: Terminalmodus, wenn stdin und stdout Terminals sind und `-T/--no-tty` fehlt; sonst Befehlsmodus unverändert. `-T/--no-tty` wie bei `ssh -T`. Verworfen: Terminalmodus nur auf Flag (`-t`) — widerspricht A6 und dem SSH-Standard.
- E4 Rückfall: Lehnt die Gegenstelle das PTY ab (SSH pty-req `Failure`, Share-Host ohne Fähigkeit `exec_terminal_v1`), meldet `se shell` das in einer Zeile auf stderr und öffnet die bisherige Befehlssitzung (wie OpenSSH ohne erzwungene PTY). Verworfen: stiller Rückfall (verbirgt das Problem); harter Abbruch (nimmt Nutzern alter Gegenstellen die bisherige Funktion).
- E5 Share-Protokoll: gleiche ALPN `smart-explorer/share-exec/2`, gleiche `EXEC_PROTOCOL_VERSION = 2`; neue optionale Felder mit `#[serde(default)]` und `skip_serializing_if` (Prüfung auf „nicht gesetzt“ bzw. „leer“); Fähigkeit als Liste `features` in `ExecHelloOk` (Wert `exec_terminal_v1`); neuer Client-Rahmen RESIZE (Tag 22); neuer Befehl `ExecCommand::Login` (Tag `"mode": "login"`) nur zusammen mit `terminal`. Der Client sendet einen Terminal-Start nur, wenn der Host die Fähigkeit meldet; der Host lehnt einen Terminal-Start ohne eigene PTY-Fähigkeit mit `terminal_unsupported` ab. Verworfen: neue ALPN bzw. Protokollversion 3 — bräuchte doppelten Serverpfad; Feld in `ExecServerHello` — die Fähigkeit gehört zur autorisierten Antwort neben `provider`.
- E6 Login-Shell auf Share-Hosts entscheidet der Host wie sshd: Linux absolutes `SHELL` der Dienstumgebung sonst `/bin/sh`, Android `/system/bin/sh`, jeweils mit `argv[0] = "-<Name>"` (Login-Konvention wie sshd); Windows `powershell.exe -NoLogo` (Profil geladen, interaktiv; passt zum bisherigen PowerShell-Dialekt). `cwd`/`env` gehen über die vorhandenen Felder `ExecStart.cwd`/`env`. Linux/Android setzen `TERM` aus der Anforderung; Windows setzt kein `TERM` (Windows-Konvention). Verworfen: Startzeile vom Client über `$SHELL -lc` — lädt das Profil doppelt.
- E7 Agenten (MCP) treten Terminalsitzungen nicht bei. `sessions` listet sie mit `"kind": "terminal"`, `attach` lehnt mit Hinweis auf `se shell -T --attachable` ab. Agenten-Sitzungen bleiben Befehlssitzungen; `se shell --attach` an eine Agenten-Sitzung bleibt der bisherige Zeilenmodus. Begründung: Eingaben eines Agenten in ein Terminal träfen jedes gerade laufende Programm (nvim, sudo, ein REPL) und den halb getippten Text des Menschen (Sicherheit, A4). Die bisherige Fähigkeit „Agent tritt einer Shell des Menschen bei“ bleibt mit `-T --attachable` erhalten. Verworfen: Shell-Integration mit OSC-133-Marken (siehe E2).
- E8 Mehrere Terminals in einer Terminalsitzung (`--attachable` + `se shell --attach`): Ausgabe verlustfrei an alle, Eingabe jedes Teilnehmers an das PTY, Größe des zuletzt aktiven Teilnehmers (Eingabe, Größenänderung oder Beitritt), Hinweiszeile „[se] <Label> attached“ bzw. „[se] <Label> detached“ an alle. Ein angehängter Teilnehmer, dessen Warteschlange 30 s voll bleibt, wird getrennt (bestehende Grenze `ATTACHED_STALL`). Verworfen: kleinste Größe aller Teilnehmer — der Tippende sähe ein zu kleines Fenster; verlustbehaftete Ausgabe — zerreißt Escape-Sequenzen.
- E9 Escape-Folgen wie `ssh`: nach Zeilenanfang `~.` verlassen (Besitzer beendet die Sitzung, Angehängter löst sich), `~~` sendet `~`, `~?` zeigt Hilfe; `--escape-char CHAR|none` wie `ssh -e`. Verworfen: Strg+D zum Lösen — gehört im Terminal der Shell der Gegenstelle.
- E10 Anhängeprotokoll Version 2 nur für Terminalsitzungen: Der Host nimmt Version 1 und 2 für Befehlssitzungen an, Terminalsitzungen nur Version 2. Ein Client sendet an Befehlssitzungen Version 1 (so bleiben Besitzer 0.5.178 erreichbar). Registereintrag und `SessionInfo` bekommen `kind` (Standard „commands“ beim Lesen alter Einträge); `dialect` wird für Terminalsitzungen optional und fehlt im JSON, wenn unbekannt (alte Leser überspringen solche Einträge). `se shell --list` hängt die Art als sechste Spalte an; die fünf bisherigen Spalten bleiben.
- E11 PTY-Größen 1..=32767 Spalten und Zeilen (ConPTY `COORD` ist `SHORT`); `TERM` 1..=64 Zeichen aus `A-Za-z0-9._+-`; ohne lokales `TERM` (Windows) `xterm-256color`. SSH-Terminalmodi fest: VINTR=3, VQUIT=28, VERASE=127, VKILL=21, VEOF=4, VSUSP=26, IUTF8=1.
- E12 Linux/Android-PTY über `posix_openpt`/`grantpt`/`unlockpt`/`ptsname_r` statt `openpty` (B12: glibc-2.17-Basis der Desktop-App). Windows-ConPTY über `GetProcAddress` aus kernel32.dll statt Ladezeit-Import, damit `se.exe` und die App auf Systemen ohne ConPTY weiter starten und dort nur keine Fähigkeit melden.
- E13 Lokales Terminal Unix: `cfmakeraw` auf einer Kopie, SIGWINCH/SIGTERM/SIGHUP/SIGQUIT/SIGINT über `sigaction` mit Selbst-Pipe in einem `os`-Adapter; Windows: Eingabe `ReadConsoleInputW` mit `ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT` ohne `ENABLE_PROCESSED_INPUT`/`ENABLE_LINE_INPUT`/`ENABLE_ECHO_INPUT`, Ausgabe mit `ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN` über einen zustandsbehafteten UTF-8-Dekoder (ungültige Bytes → U+FFFD), Konsolen-Steuerhandler stellt die Modi bei Strg+Untbr, Schließen, Abmelden und Herunterfahren wieder her. Verworfen: crossterm (parst Eingaben zu Tastenereignissen statt Bytes durchzureichen; neue Abhängigkeit); Größe per Zeitschleife abfragen (Ereignisse existieren auf beiden Plattformen).
- E14 Exitcodes im Terminalmodus: Exitstatus der Gegenstellen-Shell (`exit N` → N, Signal n → 128+n); `~.` des Besitzers → 130; `~.` eines Angehängten → 0; Fehler oder Verbindungsverlust → 125; Update-Stopp → Meldung und 125 (wie bisher); lokales SIGTERM/SIGHUP/SIGQUIT/SIGINT → 128+n nach Wiederherstellen des Terminals (im Rohmodus erzeugt die Tastatur kein SIGINT; Strg+C geht als Byte 3 an die Gegenstelle); Windows Strg+Untbr/Schließen/Abmelden/Herunterfahren → 130.
- E15 Dateigrößen: `engine.rs` (466 Zeilen), `engine_state.rs` (487), `exec_client.rs` (480) und `transports.rs` (374) werden nicht wesentlich erweitert; neue Logik kommt in neue Dateien.
- E16 Keine neue Crate, keine Änderung an `native/Cargo.toml` (B11).
- E17 Suite: eigene `terminal-task.yml` → `native/test-terminal-task.py`, Testpräfix `terminal_task_`, Batchbasis `63733be530af9cfb26946a09c762e7d2f7abce9d` (HEAD am Ende der Planung; der Batch „Ordner löschen“ hat nach Planbeginn `a24b89a1` noch `ab41cd96`, `2ce10314` und `63733be5` committet). Die fmt-Stufe prüft genau die Rust-Dateien dieses Batches (Dateilisten der Meilensteine M2 bis M8), nicht den ganzen Git-Diff ab der Basis, damit spätere Commits des Batches „Ordner löschen“ (sein Release) die Prüfung nicht verfälschen.

## Sicherheit

| Eigenschaft | Vertrauensgrenze | Prüfung vor jeder Wirkung | Fehlerverhalten (fail closed) | Abbruch, Widerruf, Aufräumen | Begrenzungen | Missbrauchsfälle und Abwehr | Abnahmesignal in der Suite |
|---|---|---|---|---|---|---|---|
| S1 Ausführungsrechte auf der Gegenstelle | lokales `se` ↔ Gegenstelle (SSH-Server bzw. Share-Host) | SSH: gespeicherte Anmeldung; Share: `authorize_client_hello` (Grant aktiv, Identität, HMAC), `ExecRegistry::prepare`, danach Prüfung der Terminal-Fähigkeit des Hosts | Ohne Fähigkeit kein Start (`terminal_unsupported`); unbekannter Rahmen → Protokollfehler → Job abgebrochen; `Login` ohne Terminal und Terminal ohne `Login` abgelehnt | Grant-Entzug bricht laufende Terminalsitzung ab (Registry-Widerruf → Containment beendet Prozessbaum) | 2 gleichzeitige Execs je Principal (bestehend); SSH `MaxSessions` | Terminal als Rechteausweitung: kein neues Recht, gleiche Freigabe wie `exec`; manipulierter Start (Digest) abgelehnt | Share: Entzug während Terminalsitzung → `se shell` endet mit 125, Terminal wiederhergestellt; Unit: Client gegen Host ohne Fähigkeit → `terminal_unsupported` ohne gesendeten Start; E2E SSH mit `PermitTTY no` → Rückfallhinweis und Zeilenmodus |
| S2 Eingaben in das PTY | lokale Teilnehmer ↔ Terminalsitzung | Teilnehmer nur mit Besitzer- oder Anhängetoken (32 Byte, Vergleich in konstanter Zeit, 5 s, nur 127.0.0.1); Anhängetoken existiert nur mit `--attachable` | Falsches Token, Version 1 an Terminalsitzung, falscher Rahmen → Verbindung getrennt | `~.` löst bzw. beendet; Besitzerende beendet die Sitzung für alle | 8 angehängte Teilnehmer; Rahmen ≤ 64 KiB | Agent tippt in Terminal des Menschen: MCP lehnt `attach` an Terminalsitzungen ab; Terminal-Anhängen verlangt Terminal an stdin und stdout (das MCP-Tool `se` startet Kinder mit Pipes); ohne `--attachable` existiert kein Anhängetoken | MCP `attach` → Fehler mit „terminal“; `se shell --attach` ohne Terminal → Fehler; Version-1-Hello → abgelehnt (Unit) |
| S3 Vertraulichkeit getippter Eingaben (Passwörter) | Teilnehmer untereinander | Eingabe eines Teilnehmers geht nur an das PTY, nie an andere Teilnehmer | — | — | — | Mitleser sieht Passwort: nur das Echo der Gegenstelle wird verteilt (bei Passwortabfragen aus) | Unit: Eingabe von Teilnehmer A erscheint nicht als Ausgabe bei B, solange die Gegenstelle nicht echot |
| S4 Gegenstellen-Ausgabe im lokalen Terminal | Gegenstelle ↔ lokales Terminal-Programm | wie `ssh`: Ausgabe der Gegenstelle unverändert; eigene Texte (Hinweise, Labels, Fehler) ohne Steuerzeichen | — | — | — | Steuersequenzen der Gegenstelle (Titel, OSC 52): gleiches Vertrauensmodell wie `ssh`, dokumentiert; Labels und Hinweise gefiltert (bestehende `label`-Funktion) | Unit: Label mit ESC wird gefiltert; Hinweiszeile enthält keine Steuerzeichen außer CR LF |
| S5 Lokaler Terminalzustand | `se shell` ↔ Terminal des Nutzers | Rohmodus erst nach erfolgreichem Verbindungsaufbau | Kann der Modus nicht gesetzt werden → Abbruch mit Meldung, Terminal unverändert | Wiederherstellung bei normalem Ende, Fehler, Panik (Drop-Wächter), SIGTERM/SIGHUP/SIGQUIT/SIGINT (Unix), Strg+Untbr/Schließen/Abmelden/Herunterfahren (Windows), Update-Stopp | — | Abgebrochenes `se` hinterlässt Rohmodus: Signal- und Konsolenhandler stellen wieder her | E2E: nach `exit`, `~.`, Update-Stopp, Grant-Entzug und SIGTERM sind ICANON und ECHO am Test-PTY wieder gesetzt |
| S6 Ressourcen | alle Teilnehmer und Gegenstelle | Größen 1..=32767, TERM-Zeichensatz und Länge geprüft (Client und Host) | ungültige Werte → Ablehnung des Starts bzw. Rahmens | — | Daten-Rahmen ≤ 64 KiB (Share, IPC, Anhängen); RESIZE-JSON ≤ 256 Byte (Share) bzw. ≤ 64 KiB Steuerrahmen (Anhängen); Warteschlange je Teilnehmer 256 Rahmen; Stau > 30 s trennt einen Angehängten; je Sitzung ein PTY; RESIZE nur bei Änderung weitergereicht | Flut von RESIZE: nur Änderungen wirken; riesige Größe: abgelehnt; langsamer Leser: getrennt, Besitzer bremst den Strom (Flusskontrolle) | Unit: Grenzen von Größe und TERM; Rahmen über 64 KiB abgelehnt |
| S7 Aufräumen | Gegenstelle | — | — | Besitzerende/`~.`/Fehler → Transport-Abbruch: Share beendet den Prozessbaum (cgroup, Job Object mit ConPTY-Host, Subreaper), SSH schließt den Kanal (sshd schließt das PTY, SIGHUP an die Sitzung; mit `nohup` gestartete Prozesse überleben wie bei `ssh`); PTY- und ConPTY-Handles geschlossen; Registereintrag entfernt | — | Verwaiste Prozesse auf Share-Hosts: Containment unverändert | E2E Share: nach `~.` findet `pgrep` den im Terminal gestarteten `sleep` nicht mehr; Eintrag nicht mehr gelistet |
| S8 Protokollierung | lokale Logdateien, Host-Historie | — | — | — | — | Terminalinhalt in Logs: verboten; Host-Historie zeigt `<shell>` | Unit: `display_program` von `Login` ist `<shell>` |
| S9 Versionsmischung | alte und neue `se`-Prozesse und Hosts | Fähigkeitsliste, Anhänge-Version, `kind` im Eintrag | Unbekanntes wird abgelehnt, nicht geraten | — | — | Alter Host bekäme unbekanntes Feld: Client sendet es nie ohne Fähigkeit; Host verwirft wegen Digest ohnehin | Unit: siehe M2 und M6 |

## D. Arbeitsplan und erwartete Ergebnisse

Pfade relativ zu `native/src`, außer wo anders angegeben. „Neu“ = neue Datei.

| Nr. | Meilenstein | Dateien | Prüfbares Ergebnis (Abnahme in der Task-Suite) |
|---|---|---|---|
| M1 | Plan im Repository | `docs/plaene/2026-10-14-echtes-terminal/plan.md` (neu, Repo-Wurzel) | Datei enthält Ziel, Anforderungen, Entscheidungen, Meilensteine, Sicherheit, Suite; keine Gerätedaten |
| M2 | Share-Exec-Terminalprotokoll | `share/core/exec_terminal.rs` (neu), `share/core/exec_types.rs`, `exec_protocol.rs`, `exec_admission.rs`, `exec_server.rs`, `exec_job.rs`, `exec_platform.rs`, `exec_supervisor_protocol.rs`, `exec_client.rs`, `exec_client_active.rs`, `share/mod.rs`, `share/api_exports.rs`, `daemon/os/shared/exec_ipc.rs`, Testdateien, `terminal: None` an allen `ExecStart`-Literalen | Unit: Start ohne Terminal serialisiert byte-gleich wie bisher (gleicher Digest); Terminal-Start mit Größe/TERM außerhalb der Grenzen abgelehnt; `Login` ohne Terminal abgelehnt; RESIZE nur bei Terminal-Start angenommen, sonst Protokollfehler; Client sendet keinen Terminal-Start an Host ohne `exec_terminal_v1` (Fehler `terminal_unsupported`, `start_may_have_been_sent = false`); Host ohne PTY lehnt Terminal-Start mit `terminal_unsupported` ab; IPC RESIZE wird zu `ExecClientInput::Resize` |
| M3 | PTY auf Linux- und Android-Hosts | `share/os/linux_os/exec_supervisor.rs`, `share/os/linux_os/exec_pty.rs` (neu), `share/os/linux_os/exec.rs`, `share/os/android/exec.rs`, `share/os/android/exec_contain.rs`, optional `share/os/linux_os/exec_self_test.rs` | Unit (Linux): Terminal-Start eines `Login` mit 30×100 liefert `tty` = `/dev/pts/N`, `stty size` = `30 100`, nach RESIZE 40×120 `stty size` = `40 120`, `TERM` = angeforderter Wert, Strg+C (Byte 3) beendet `sleep 100` und die Shell lebt weiter, `exit 7` → Exitcode 7; E2E über Share-Paar (Linux) |
| M4 | ConPTY auf Windows-Hosts | `share/os/windows/exec_supervisor.rs`, `share/os/windows/exec_conpty.rs` (neu), `share/os/windows/exec.rs`, optional `share/os/windows/exec_self_test.rs` | Unit (Windows): `terminal_supported()` auf dem Runner wahr; Terminal-Start `Login` startet PowerShell im ConPTY, Ausgabe enthält VT-Sequenzen, `$Host.UI.RawUI.WindowSize.Width` = angeforderte Spalten, nach RESIZE neue Breite, `exit 7` → 7, Abbruch leert das Job Object |
| M5 | SSH-Terminalkanal | `sftp/core/terminal.rs` (neu), `sftp/core/command.rs`, `sftp/mod.rs` | E2E (OpenSSH-Container): PTY angefordert (`tty` → `/dev/pts/N`), window-change wirkt, abgelehntes PTY (`PermitTTY no`) meldet den Rückfall |
| M6 | Terminalsitzung im Sitzungskern | `cli/shell_session.rs`, `cli/shell_session/{types,wire,server,registry,host,transports,client,local_transport,dialect}.rs`, neu: `cli/shell_session/{terminal_engine,terminal_server,terminal_client,terminal_transports,terminal_tests}.rs` und ein Test-PTY-Transport | Unit (Unix, lokale PTY-Shell): Bytes beider Richtungen unverändert; Größe des zuletzt aktiven Teilnehmers; zweiter Teilnehmer sieht Ausgabe und tippt; Hinweise ohne Steuerzeichen; Ende überträgt Exitcode und entfernt Eintrag; Version-1-Hello an Terminalsitzung abgelehnt, an Befehlssitzung angenommen; alter Eintrag ohne `kind` gelesen als Befehlssitzung |
| M7 | Terminal-Frontend `se shell` | `cli/shell.rs`, neu `cli/shell_terminal.rs`, `cli/shell_escape.rs`, `cli/os/terminal_unix.rs`, `cli/os/terminal_windows.rs`, `cli/os/mod.rs`, `cli/mod.rs` | E2E unter Linux-PTY: Tab-Vervollständigung, Farben, nvim (`ESC[?1049h`/`l`), claude und codex starten ihre Oberfläche ohne TTY-Fehler, Passwortabfrage über `/dev/tty` ohne Echo, Strg+C an `sleep`, Größenänderung, `exit 7` → 7, `~.` → 130, Terminalmodi nach jedem Ende wiederhergestellt; `-T` und Pipe-Modus unverändert; unter Windows-ConPTY (pywinpty) gegen Windows-Share-Paar: PowerShell-Prompt, Tab-Vervollständigung, Strg+C, Größe, Exitcode |
| M8 | MCP und Skill | `cli/mcp/shell_registry.rs`, `cli/mcp/sessions_tool.rs`, `cli/mcp/catalog.rs`, `cli/plugin/assets/SKILL.md` | MCP: `sessions` zeigt `kind`; `attach` an Terminalsitzung abgelehnt mit Hinweis `-T`; Beitritt zu `se shell -T --attachable` wie bisher; Befehlssitzungen unverändert |
| M9 | Dokumentation | `README.md`, `docs/AI_AGENT_PLUGIN.md`, `docs/TODO.md`, `docs/ARCHITEKTUR.md`, `docs/REMOTE_EXECUTION_PLAN.md` | Verhalten, Grenzen, Sicherheit, Suite beschrieben; „kein PTY“ nur noch für Agenten-Befehlssitzungen; offene Punkte in `docs/TODO.md` |
| M10 | Task-Suite | `.github/workflows/terminal-task.yml` (neu), `native/test-terminal-task.py` (neu) | Ein Remote-Lauf (Linux und Windows) prüft M2 bis M8 und das zu erhaltende Verhalten |
| M11 | Release | Release-Pfad (`native\publish-release-local.ps1` auf dem Remote-Runner) | eine Version mit allen Assets auf GitHub |

Ablauf nach der Umsetzung: Commits je zusammenhängender Änderung mit `[task candidate]` als letzter Zeile; Graph-Neuaufbau nach den nativen Änderungen (AGENTS.md „graphify“); Push erst, wenn der Release des Batches „Ordner löschen“ abgeschlossen ist; Suite einmal remote auslösen; Fehler beheben; ein Release.

## Zu erhaltendes Verhalten

- V1 `se shell` ohne Terminal an stdin oder stdout: unverändert (eine Zeile je Befehl, Fortsetzung mit `\` bzw. `` ` ``, Exitcodes letzter Befehl / `exit N` / 125 / 130).
- V2 `se shell -T` im Terminal: der bisherige Zeilenmodus mit Prompt `ziel:cwd$` auf stderr.
- V3 `se shell --attach` an eine Befehlssitzung (Agenten-Sitzung oder `-T --attachable`): bisheriger Zeilenmodus, auch im Terminal; Strg+D löst sich.
- V4 MCP `exec` mit `session`, `sessions` list/attach/activity/close für Befehlssitzungen unverändert; Agent tritt `se shell -T --attachable` bei wie bisher.
- V5 Versionsmischung: Clients 0.5.178 hängen sich an Befehlssitzungen neuer Besitzer; neue Clients an Befehlssitzungen von Besitzern 0.5.178.
- V6 `se exec` und MCP `exec` ohne `session`: unverändert (Exitcodes 124/125/130).
- V7 Share-Exec ohne Terminal: gleiche ALPN, Protokollversion, Rahmen und Digest; alte Clients ↔ neue Hosts und neue Clients ↔ alte Hosts funktionieren.
- V8 Exec-Freigabe, Entzug beendet die Sitzung, Containment je Plattform (systemd-cgroup, Job Object, Android-Subreaper), Exec-Grenzen je Gerät (2 gleichzeitig je Principal) unverändert.
- V9 Daemon-IPC für Nicht-Terminal-Exec unverändert.
- V10 Update-Stopp beendet `se shell` mit Meldung und 125 (beide Modi); Terminal wiederhergestellt.
- V11 GUI und Android-App: keine Verhaltensänderung; Android-Host bekommt nur PTY für Terminal-Starts.
- V12 Vervollständigung des Ziels von `se shell` (`exec_target_candidates`) und `se shell --list`/`--list --json` (fünf Spalten bzw. Felder bleiben, `kind` kommt hinzu).
- V13 Protokolle: kein Befehlstext, keine Ausgabe, kein Token in Logdateien; Exec-Historie zeigt `<shell>`.

## Task-Suite

- Einstieg: `.github/workflows/terminal-task.yml` (`workflow_dispatch` mit `candidate_sha`, Matrix `ubuntu-latest` und `windows-2025`, `timeout-minutes: 300`, Schritt-Zeitlimit 290 min, Artefakte immer hochladen) → `python native/test-terminal-task.py --log-root "$RUNNER_TEMP/terminal-task"`.
- Stufen: fmt (Rust-Dateien der Meilensteine M2 bis M8), build (`se` und Lib-Tests, Warnungen in Batchdateien sind Fehler), unit (`terminal_task_`, `shell_session_task_`, `mcp_task_`, betroffene Share-Exec- und IPC-Module), mcp, Linux-E2E (OpenSSH-Container mit bash, nvim, python3; Share-Paar mit nvim, claude, codex), Windows-E2E (Share-Paar unter Windows, Terminal per pywinpty), Erhalt (Pipe-Modus, `-T`, Befehlssitzungen, `se exec`).
- Auslösen: `gh workflow run terminal-task.yml --ref main -f candidate_sha=<sha>` oder gleichwertig über die GitHub-API.

## Recherche

Alle Quellen abgerufen bzw. gelesen am 2026-10-14.

- RFC 4254 (SSH Connection Protocol), Abschnitte 6.2 pty-req (TERM, Spalten, Zeilen, Pixelmaße, Modi), 6.5 shell/exec (Antwort anfordern und prüfen empfohlen), 6.7 window-change (keine Antwort), 8 Encoding of Terminal Modes (Opcode-Byte 1..159 mit uint32, Ende TTY_OP_END=0): https://www.rfc-editor.org/rfc/rfc4254
- OpenSSH portable, `clientloop.c` (Zweig master): https://raw.githubusercontent.com/openssh/openssh-portable/master/clientloop.c — pty-req mit Fenstergröße aus `TIOCGWINSZ`, SIGWINCH löst window-change aus; abgelehnte PTY-Anforderung ergibt eine Fehlermeldung und die Sitzung läuft ohne PTY weiter, nur bei erzwungener PTY (`-t`, RequestTTY force/yes) Abbruch.
- russh 0.61.2, Crate-Quelltext im Cargo-Registry (`src/channels/mod.rs`, `src/pty.rs`; keine URL, kein Webabruf): `Channel::request_pty(want_reply, term, cols, rows, px, py, &[(Pty, u32)])`, `request_shell(want_reply)`, `window_change(cols, rows, px, py)`; Antworten kommen als `ChannelMsg::Success`/`Failure` in Anforderungsreihenfolge; Opcodes in `Pty`: VINTR=1, VERASE=3, VEOF=5, IUTF8=42 (die übrigen Opcodes sind hier nicht aufgeführt).
- libc 0.2.186, Crate-Quelltext im Cargo-Registry (keine URL, kein Webabruf): `posix_openpt`, `grantpt`, `unlockpt`, `ptsname_r`, `openpty`, `login_tty`, `TIOCSCTTY`, `TIOCSWINSZ`, `TIOCGWINSZ`, `cfmakeraw`, `winsize` für Linux und Android. Vorgehen im Kind: `setsid` und `ioctl(TIOCSCTTY)`; Größe per `TIOCSWINSZ` auf dem Master. Ohne Webquelle übernommen (Kernelverhalten): Der Kernel sendet danach SIGWINCH an die Vordergrund-Prozessgruppe.
- Android: AOSP system/sepolicy, `private/untrusted_app_all.te` (Zweig main): https://android.googlesource.com/platform/system/sepolicy/+/refs/heads/main/private/untrusted_app_all.te — enthält `create_pty(untrusted_app_all)` („Allow the allocation and use of ptys“). Herkunft des Makros: AOSP-Commit von 2013 (Spiegel https://gitlab.cs.fau.de/Matombo/AndroidSystemSEPolicy/-/commit/2dc4acf33b78284f514fe9a6c5102cc783c4309f): `ptmx_device` rw für alle Domänen über `domain.te`.
- Microsoft Learn, „Creating a Pseudoconsole session“ (Stand der Seite 2025-08-05): https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session — jede Kanalrichtung auf eigenem Thread; nach `CreateProcess` die an `CreatePseudoConsole` übergebenen Pipe-Enden schließen; `EXTENDED_STARTUPINFO_PRESENT` mit `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`; `ClosePseudoConsole` beendet alle angehängten Clients.
- Microsoft Learn, „ClosePseudoConsole“ (Stand 2025-07-09): https://learn.microsoft.com/en-us/windows/console/closepseudoconsole — sendet CTRL_CLOSE_EVENT; vor Windows 11 24H2 (Build 26100) wartet der Aufruf, bis die Ausgabe-Pipe geleert ist, daher nie auf dem Lesethread aufrufen; Mindestversion Windows 10 1809 / Server 2019.
- portable-pty 0.9.0, `src/win/psuedocon.rs` auf docs.rs: https://docs.rs/crate/portable-pty/latest/source/src/win/psuedocon.rs — `STARTF_USESTDHANDLES` mit `INVALID_HANDLE_VALUE` für hStdInput/hStdOutput/hStdError, damit das Kind keine umgeleiteten Std-Handles des startenden Prozesses erbt; `bInheritHandles` falsch; Flags `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT`; ConPTY-Funktionen zur Laufzeit geladen. Ein erster Abruf derselben Datei unter `https://raw.githubusercontent.com/wezterm/wezterm/main/pty/src/win/psuedocon.rs` lieferte HTTP 404; die docs.rs-Fassung wurde stattdessen gelesen.
- Microsoft Learn, „SetConsoleMode“ (Stand 2021-12-30): https://learn.microsoft.com/en-us/windows/console/setconsolemode — `ENABLE_VIRTUAL_TERMINAL_INPUT` liefert VT-Sequenzen über ReadFile/ReadConsole; Fenstergrößenereignisse (`ENABLE_WINDOW_INPUT`) nur über `ReadConsoleInput`; ohne `ENABLE_PROCESSED_INPUT` landet Strg+C als Zeichen im Eingabepuffer.
- PowerShell/openssh-portable (Win32-OpenSSH), Zweig latestw_all, `contrib/win32/win32compat/tncon.c`: https://github.com/PowerShell/openssh-portable/blob/latestw_all/contrib/win32/win32compat/tncon.c — liest `ReadConsoleInputW` bei VT-Eingabe, wandelt `uChar.UnicodeChar` nach UTF-8, behandelt `WINDOW_BUFFER_SIZE_EVENT` als Fensteränderung (kein Abfragen per Zeitschleife).
- Rust std, Windows-Konsole: https://doc.rust-lang.org/std/io/struct.Stdout.html und https://rust.googlesource.com/rust/+/HEAD/library/std/src/sys/stdio/windows.rs — `Stdout` auf einer Windows-Konsole lehnt ungültiges UTF-8 mit `InvalidData` ab; unvollständige Sequenzen über Schreibaufrufe werden gepuffert.
- tmux(1): https://man7.org/linux/man-pages/man1/tmux.1.html — Option `window-size`: `latest` nimmt die Größe des zuletzt aktiven Clients.
- VS Code, Terminal Shell Integration: https://code.visualstudio.com/docs/terminal/shell-integration — Einschleusung per Argumenten/Umgebung, Marken OSC 633 bzw. OSC 133 A/B/C/D für Prompt und Befehlsende; für diesen Auftrag verworfen (E7).

## Offene Punkte

- O1 Nicht E2E prüfbar in einem Runner: Linux-Client ↔ Windows-Host und Windows-Client ↔ Linux-Host über zwei Rechner, Android-Host mit PTY auf einem Gerät, Windows-OpenSSH als SSH-Ziel (bestehend `AIPLUG-WINSSH`). Der Meilenstein M9 trägt sie in `docs/TODO.md` ein.
- O2 Wiederanbinden nach Verbindungsverlust, `se exec -t` für einzelne Befehle mit PTY, Beobachten einer Terminalsitzung durch Agenten: nicht Teil dieses Auftrags (A2, E7). Der Meilenstein M9 trägt sie in `docs/TODO.md` ein.
- O3 Terminals ohne Windows-Konsole (mintty/Git Bash mit Pipe-stdin) bleiben im Befehlsmodus (A5); dokumentiert im Meilenstein M9.

## Fortschritt

- [x] M1 Plan im Repository
- [x] M2 Share-Exec-Terminalprotokoll (umgesetzt; Übersetzungs- und Laufbeleg folgen mit der Suite)
- [x] M3 PTY auf Linux- und Android-Hosts (umgesetzt; Übersetzungs- und Laufbeleg folgen mit der Suite)
- [x] M4 ConPTY auf Windows-Hosts (umgesetzt; Übersetzungs- und Laufbeleg folgen mit der Suite)
- [x] M5 SSH-Terminalkanal (umgesetzt; Übersetzungs- und Laufbeleg folgen mit der Suite)
- [x] M6 Terminalsitzung im Sitzungskern (umgesetzt; Übersetzungs- und Laufbeleg folgen mit der Suite)
- [x] M7 Terminal-Frontend `se shell` (umgesetzt; Übersetzungs- und Laufbeleg folgen mit der Suite)
- [x] M8 MCP und Skill (umgesetzt; Übersetzungs- und Laufbeleg folgen mit der Suite)
- [x] M9 Dokumentation
- [ ] M10 Task-Suite (geschrieben, Remote-Lauf offen)
- [ ] M11 Release
