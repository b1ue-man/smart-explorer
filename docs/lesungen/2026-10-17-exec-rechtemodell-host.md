# Exec-Rechtemodell des Hosts (bestandslesend)

Stand 2026-10-17, Lesestand da2646d2

## Ziel und Fragen

Ziel: Bestandsbefund für Strang S1 „Exec-Rechte nutzergebunden, Kontowechsel je OS“ (Ergebnisse E1, E2, E3) aus `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, Abschnitt A. Diese Datei bewertet nichts, empfiehlt nichts und ändert keinen Code.

Pfadangaben: `core/` steht für `native/src/share/core/`, `os/` für `native/src/share/os/`. Zeilenangaben beziehen sich auf den Lesestand da2646d2.

Fragen (Nummern wie im Auftrag):
1. Aufbau eines Exec-Grants: Felder, Standardwerte, Format, Revisionen, Portierung, Kompatibilität zu Peers.
2. `authorize_start` und Programmauflösung: Verhalten mit und ohne Programmliste, Shell, Terminal, Skripte, Pfade.
3. Kontowechsel je OS (Linux, Windows, Android).
4. Konto, unter dem der Worker normalerweise je OS läuft.
5. Prüfkette einer Anfrage, Rolle von Direkt-Kontakten, Räumen und Raummitgliedern, Rückfrage pro Sitzung.
6. Protokoll, Strafkonto, Abbruch, Laufzeitgrenzen mit Werten.
7. Ablehnungs- und Fehltexte, die der Client sieht.

Annahmen (ausdrücklich markiert):
- A1: „Standardnutzer“ bezeichnet das Konto, unter dem der Smart-Explorer-Worker läuft. Quelle: `docs/plaene/2026-10-16-luecken-schliessen/plan.md:81`. Im Code steht das Wort nur in Kommentaren (`core/exec_policy.rs:27`, `:29`, `:112`).
- A2: `CreateProcessW` (`os/windows/exec.rs:31`, Aufruf `:367`) erhält kein Token-Argument. Quelle: Win32-API-Kenntnis, nicht im Repository belegt.
- A3: „Standardeinstellung“ in Frage 2 heißt: Kontoart `Standard` und leere Programmliste (`core/exec_policy.rs:46-55`).
- A4: `ContainedExec` in `core/exec_platform.rs` ist der Live-Pfad des Starts. Beleg: `docs/lesungen/2026-09-26-exec-host-flow.md`, Zeile 35. Der Aufruf von `serve_job` (`core/exec_admission.rs:193`) liegt außerhalb der gelesenen Dateien.

## Befunde je Frage

### Frage 1: Aufbau eines Exec-Grants

Felder von `ExecGrant` (`core/exec_policy.rs:120-128`): `enabled: bool`, `policy_revision: u64`, `changed_at: i64`, `source_request_id: Option<DirectRequestId>`, `source_decision_revision: Option<u64>`, `run_as: ExecRunAs`, `programs: Vec<String>`.

Standardwerte:
- `ExecGrant::default()` (abgeleitet, `core/exec_policy.rs:118`): `enabled` false, `policy_revision` 0, `run_as` Standard, `programs` leer. Test: `core/exec_policy.rs:290-293` („default deny“).
- `ExecRunAs::default()` ist `Standard` (`core/exec_policy.rs:29-30`, `#[default]`).
- `ExecGrantSettings` (Eingabe beim Ändern, `core/exec_policy.rs:46-55`) ist abgeleitet `Default`: `run_as` Standard, `programs` leer. Doku: leere Liste heißt „the full shell as before“ (`:50-52`).
- Umschalten ohne Einstellungen lässt Konto und Liste unverändert (`core/exec_grant_persistence.rs:9-10`, `:19-27`).
- Welchen Wert CLI, GUI, MCP oder Android beim Erlauben übergeben: Lücke L1.

Revisionen:
- Jede Änderung von `enabled` erhöht `policy_revision` mit `checked_add`; Fehler „Exec policy revision exhausted“ (`core/exec_policy.rs:202-216`).
- Änderung von Konto oder Liste läuft über `set_runtime_policy` → `set_runtime_enabled` und erhöht die Revision ebenso (`core/exec_policy.rs:133-145`). Test: `core/exec_policy_tests.rs:327` („changing account or programs advances the revision and switching keeps them“).
- Deaktivieren erhöht mit `saturating_add` (`core/exec_policy.rs:235-246`).
- Persistenz prüft Compare-and-Swap: gewünschte Revision = erwartete + 1 (`core/exec_grant_persistence.rs:98-110`); Konfliktmeldung (Formatzeichenkette, `:55-58`): „Exec-Grant revision changed concurrently: expected {expected_revision}, found {}“.
- Laufzeit-Epoche `authorization_epoch` steigt je Mutation (`core/exec_grant_runtime.rs:35-38`, `:53`).

Gespeichertes Format (`core/exec_policy_wire.rs`):
- `ExecGrantWire` (`:17-37`) schreibt: `enabled`, `policy_revision`, `changed_at`, `source_request_id`, `source_decision_revision`, `run_as` (ohne `skip_serializing_if`, also immer, `:29-30`), `programs` (nur wenn nicht leer, `:31-32`), `restricted_enabled` (nur wenn true, `:33-36`).
- Aktiver eingeschränkter Grant (Konto ≠ Standard oder Liste nicht leer) wird mit `enabled: false` und `restricted_enabled: true` geschrieben (`:60-74`; `:62`, `:64`, `:71`).
- Lesen: `enabled = wire.enabled || (wire.restricted_enabled && restricted)` (`:47-49`). Ein `restricted_enabled` ohne echte Einschränkung schaltet nichts ein (Test `core/exec_policy_tests.rs:295`).
- JSON von `ExecRunAs` (aus Serde-Attributen abgeleitet, `core/exec_policy.rs:24-26`, `:31-33`): `{"kind":"standard"}` oder `{"kind":"account","name":"<Name>"}`.
- Abweichung Doku/Code: `core/exec_policy_wire.rs:9-10` sagt „Unrestricted grants are stored exactly as before“. Der Code schreibt `run_as` immer. Die Byte-Form eines unrestriktiven Grants enthält daher das Feld `run_as` zusätzlich. Der Test `core/exec_policy_tests.rs:270-282` prüft nur `enabled` und das Fehlen von `restricted_enabled`.

Portierung:
- Profile ohne `run_as` und `programs` lesen sich als Standard mit voller Shell (`core/exec_policy.rs:48-49`, `:53-54`, `:112-114`; `core/exec_policy_wire.rs:29-32`, alle Felder `#[serde(default)]`).
- Test `core/exec_profile_compat_tests.rs:36-60`: Profil mit exakt fünf 0.5.180-Feldern lädt; `enabled` bleibt; `run_as` Standard; `programs` leer; Revision 7 und 3 bleiben.
- Test `core/exec_profile_compat_tests.rs:83-118`: eingeschränkter Grant wird für 0.5.180 als aus gelesen (`:99`), mit `restricted_enabled` true (`:101`); nach Speichern durch 0.5.180 bleibt `enabled` false (`:115-116`); dieser Stand liest wieder aktiv (`:118`).
- Legacy-Migration: `reset_all_for_legacy_migration` setzt jeden Grant auf `ExecGrant::default()` (`core/exec_policy.rs:271-282`; Doku `:271-272`: der alte exportweite Bit „was never a safe grant“).
- Schemaversion `SHARE_PROFILE_VERSION` (Test `core/exec_profile_compat_tests.rs:38`): Wert nicht gelesen (Lücke L3).

Kompatibilität zu Peers bis 0.5.180 und 0.5.182:
- Über das Netz gehen kein `run_as` und keine `programs`. `ExecStart` hat genau: `exec_id`, `command`, `cwd`, `env`, `timeout_ms`, `max_output_bytes`, `terminal` (`core/exec_types.rs:55-71`). `ExecHelloOk` wird aus `authorization`, `provider`, `features` gebaut (`core/exec_admission.rs:125-131`). Das Konto wirkt also nur beim Host.
- `ExecProviderStatus` (`available`, `provider`, `detail`, `elevated`, `user_label`; `core/exec_types.rs:221-228`) geht mit HelloOk an den Peer (`core/exec_admission.rs:122-131`). `user_label` ist für den Peer sichtbar.
- Konstanten: `EXEC_PROTOCOL_VERSION` = 2 (`core/exec_types.rs:9`); `EXEC_CAPABILITY` = `exec_stream_v2` (`:10`); ALPN `smart-explorer/share-exec/2` (`core/exec_protocol.rs:11`); Transkript-Präfix `smart-explorer/share-exec/v2|` (`core/exec_auth.rs:231-233`).
- Terminal-Feature `exec_terminal_v1` (`core/exec_terminal.rs:13`): Host listet es nach Plattformfähigkeit, unabhängig vom Grant (`:66-72`; `core/exec_admission.rs:130`). Client sendet Terminal-Starts nur an Hosts mit diesem Feature (`core/exec_terminal.rs:86-99`).
- Feld `terminal` fehlt im JSON, wenn es nicht gesetzt ist; Bytes und Digest bleiben wie vor Terminals (`core/exec_types.rs:67-70`; Test `:338-352`).
- Welche Release welche Protokollversion sprach: Lücke L4.

### Frage 2: `authorize_start` und Programmauflösung

Ohne Programmliste:
- `authorize_start` gibt `Ok(None)` zurück, ohne Befehls-, Argument- oder Umgebungsprüfung (`core/exec_policy.rs:168-170`). Doku: „A grant without a program list is the full shell: it refuses nothing“ (`:156-158`).
- Ein aktiver Grant ohne Liste blockiert `ls` also nicht durch eine Programmregel. `ls` wird als Argv-Start ohne Pinning übergeben (`core/exec_platform.rs:66-68` pinnt nur bei `Some`; `program` ist das Ergebnis von `authorize_start`, bei leerer Liste `None`: `core/exec_policy.rs:168-170`, `core/exec_admission.rs:267-272`, `:286`, `:292`). Wie der Supervisor den Namen dann auflöst: Lücke L10.
- Ist der Grant nicht aktiv (Standardwert `enabled: false`), wird jeder Start auf dem Handshake abgelehnt: `NotAuthorized` (`core/exec_auth.rs:156-158`), Antwort `permission_denied` (`core/exec_admission.rs:79-82`). Also auch `ls`.

Mit Programmliste (Befehle):
- `ExecCommand::Shell` wird abgelehnt: „this host allows this device only its listed programs, no shell commands“ (`core/exec_policy.rs:173-178`). Shell-Builtins und Pipes sind damit ausgeschlossen, weil sie nur in dieser Befehlsform vorkommen (`core/exec_types.rs:49-51`).
- `ExecCommand::Login` (interaktives Terminal) wird abgelehnt: „this host allows this device only its listed programs, no terminal“ (`core/exec_policy.rs:179-183`). Ein Terminal ist bei Liste nicht möglich, auch wenn das Feature `exec_terminal_v1` gelistet ist.
- `ExecCommand::Argv { program, args }`: Argumente werden nicht geprüft. Das Muster in `core/exec_policy.rs:172` ignoriert `args` (`..`).
- Umgebung: nur `LANG`, `TERM` und `LC_*` (Großbuchstaben, Ziffern, `_`, höchstens 32 Zeichen nach `LC_`); Werte nur aus `[A-Za-z0-9._@+-]`, höchstens 64 Bytes (`core/exec_policy.rs:185-197`, `:251-261`, `:264-269`, Grenze `:21`).

Programmauflösung (`core/exec_program_path.rs`):
- Absoluter Pfad: genau dieser Pfad; Endungen werden nur ergänzt, wenn der Name keine Endung hat (`:31-33`, `:83-91`).
- Relativer Pfad mit Trennzeichen: Fehler „program {program:?} names a directory but is not an absolute path“ (`:34-37`).
- Bare Name: nur in den festen Suchverzeichnissen des Hosts, nie im `PATH` des Aufrufers oder des Workers (`:39-44`; Doku `:3-6`).
- Kandidat muss eine Datei sein (`:46`) und sich kanonisieren lassen (`:50`).
- Erlaubt, wenn die kanonische Datei des Aufrufs gleich der kanonischen Datei eines Listeneintrags ist und die Dateinamen ASCII-unabhängig gleich sind (`:63-81`, `:95-100`). Die Namensgleichheit soll Multi-Call-Binaries schützen (Doku `:7-10`).
- Der aufgelöste Pfad ersetzt den Programmnamen vor dem Start (`core/exec_platform.rs:66-68`, `:123-139`; Doku `:44-46`).
- Feste Suchpfade je OS: Linux `ACCOUNT_PATH` in 6 Verzeichnissen ohne Endungen (`os/linux_os/exec_program_search.rs:10-15`; `core/exec_supervisor_protocol.rs:48-49`); Windows `System32`, `SystemRoot` und `System32\Wbem` mit `.exe` und `.com` (`os/windows/exec.rs:229-241`, Doku `:225-228`); Android `/system/bin`, `/system/xbin`, `/vendor/bin` ohne Endungen (`os/android/exec.rs:319-327`).

Skripte, Interpreter, Pfade:
- Ein als absoluter Pfad gelistetes Skript (ausführbare Datei) besteht die Auflösung. Die gelesenen Dateien prüfen weder Inhalt noch Shebang.
- Unter Windows ergänzt ein bare Name nur `.exe` und `.com`; ein Befehlsskript wird nicht ergänzt (Test `core/exec_program_path.rs:126-127`, Kommentar „it would start a shell“). Ob der Start eines absolut angegebenen `.cmd` eine Shell startet: Lücke L9.
- Wird ein Interpreter (zum Beispiel `sh`) gelistet, prüft `authorize_start` seine Argumente nicht (`core/exec_policy.rs:172`). Folge (Annahme A3, aus dem Code abgeleitet, nicht getestet): der Interpreter kann Befehle ausführen, die nicht im Programmnamen stehen. Die Testnamen `core/exec_policy_tests.rs:50`, `:153`, `:175`, `:193` belegen Shell- und Terminal-Ablehnung und die Namensregel, aber keine Argumentprüfung. Der Inhalt dieser Tests wurde nicht gelesen.
- Umgebung bei voller Shell: `ExecStart::validate` prüft Umgebungsnamen nur auf NUL und `=` (`core/exec_types.rs:101-107`). `environment_for` übernimmt die Worker-Umgebung ohne `SMART_EXPLORER_*` und legt Aufruferwerte darüber (`core/exec_supervisor_protocol.rs:105-119`). Der Aufrufer kann daher `PATH` und `HOME` setzen (Lücke L11 für weitere Begrenzungen).

### Frage 3: Kontowechsel je OS

Zusammenfassung:

| OS | Kontowechsel | Beleg |
|---|---|---|
| Linux | ja: nur mit Worker als root und nur über systemd (ohne Zwischenprozess) | `os/linux_os/exec.rs:303-307`, `:314-324`; `os/linux_os/exec_supervisor.rs:327-330` |
| Windows | nein | `os/windows/exec.rs:244-258` |
| Android | nein | `os/android/exec.rs:329-342` |

Linux, Ablauf (Reihenfolge im Code):
1. Start: `grant_launch` (`core/exec_admission.rs:172-185`, Funktion `:260-298`): erst `authorize_start` (`:267-272`), dann `resolve_run_as` (`:273-281`).
2. `resolve_run_as` → `resolve_account` (`core/exec_platform.rs:163-170`). Os-Adapter `os/linux_os/exec.rs:314-324`: Worker nicht root → PermissionDenied „only a Smart Explorer worker running as root can run commands as another account“ (`:320`). Sonst `exec_account::resolve` (`:323`).
3. `exec_account::resolve` (`os/linux_os/exec_account.rs:18-63`): `getpwnam_r`, Puffer 16 KiB, Verdopplung bis 1 MiB (`:12-13`, `:36-39`). Unbekanntes Konto → NotFound „local account {name:?} does not exist on this host“ (`:43-48`). Heimat aus `pw_dir`, leer wird `/` (`:49`, `:59`). Shell aus `pw_shell`, wenn absolut, sonst `/bin/sh` (`:16`, `:50-52`). Gruppen über `getgrouplist` mit primärer GID (`:53`, `:75-99`; Obergrenze `MAX_GROUPS` 65 536 in `:15`).
4. Startpaket `environment_for_account` (`core/exec_supervisor_protocol.rs:84-103`): aus dem Worker übernommen nur `LANG`, `LC_ALL`, `LC_CTYPE`, `TZ` (`:52`, `:89-93`). Gesetzt werden `HOME`, `USER`, `LOGNAME`, `SHELL` und `PATH` = `/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin` (`:48-49`, `:94-98`). Danach überschreiben Aufruferwerte (`:99-101`). Dann `SupervisorStart { request, environment, account }` (`core/exec_platform.rs:74-78`).
5. Supervisor: Konto zusammen mit Zwischenprozess → Fehler „a run-as account needs the Linux systemd host“ (`os/linux_os/exec_supervisor.rs:327-330`). Sonst `account::switch_to` als `pre_exec` (`os/linux_os/exec_supervisor_account.rs:23-69`): `setgroups` (`:47-49`), `setgid` (`:50-52`), `setuid` (`:53-55`), Prüfung „Root zurückholbar“ mit EPERM (`:56-60`), `chdir` in das Arbeitsverzeichnis (`:61-64`). Die Ausführung im Heimatverzeichnis ohne gewünschtes cwd fällt auf `/` zurück (`os/linux_os/exec_supervisor.rs:331-333`; Doku `exec_supervisor_account.rs:7-8`).
6. Terminal: `fchown` auf Konto-UID und Gruppe `tty` (Fallback Konto-GID), `fchmod` 0620 (`os/linux_os/exec_supervisor_account.rs:71-82`; `:19-21`).
7. Ausführung: `StartTransientUnit` mit Modus `fail` (`os/linux_os/exec_systemd.rs:70`). Manager-Verbindung: Worker root → Systembus; sonst Nutzerbus `unix:path=/run/user/<euid>/bus` (`:85-96`).
- PAM: kein Aufruf in den vollständig gelesenen Linux-Dateien `exec_account.rs` und `exec_supervisor_account.rs`; Suche nach `pam`, `utmp`, `initgroups`, `login` in `os/linux_os/exec.rs`, `exec_systemd.rs`, `exec_supervisor.rs` ohne Treffer. `exec_pty.rs` nur per Suche (`setsid` in `:151`). Lücke L9.
- Gruppen: primäre und alle Zusatzgruppen des Kontos (`exec_account.rs:53`, `:75-99`; `exec_supervisor_account.rs:29-33`, `:47-49`).
- Heimatverzeichnis: `pw_dir`. Fehlt es, startet der Prozess in `/` nur, wenn kein cwd verlangt wurde (`exec_supervisor_account.rs:61-64`; `exec_supervisor.rs:331-333`).

Windows:
- `run_as_supported()` ist `false` (`os/windows/exec.rs:248-250`). Doku: „another account would need that account's password (`CreateProcessWithLogonW`) or a SYSTEM service with `SeTcbPrivilege` (S4U logon), neither of which the per-user worker has“ (`:244-247`).
- `resolve_account` → Unsupported „Windows hosts run commands only as the Smart Explorer user“ (`os/windows/exec.rs:252-258`).
- Start über `CreateProcessW` (Import `:31`, Aufruf `:367`). Keine Aufrufe von `CreateProcessAsUser`, `CreateProcessWithLogon`, `LogonUser` oder `DuplicateToken` in `os/windows/` (Suche). Prozess läuft unter dem Konto des Workers (Annahme A2).
- Job Object „kill-on-close“ (`os/windows/exec.rs:284-292`; Detailtext `:211-212`). Elevation: `process_is_elevated` (`:214`, `:410-434`).
- Terminal: ConPTY, `CreatePseudoConsole` über `GetProcAddress` (`os/windows/exec_conpty.rs:63`, `:78-80`); Shell per `CreateProcessW` (`:174`); Kommentar `:105` „Windows PowerShell, interactive, with the user's profile“.

Android:
- Jede App läuft unter eigener UID; die App kann keinen Prozess unter einem fremden Konto starten (`os/android/exec.rs:329-331`). `run_as_supported()` ist `false` (`:331-333`). `resolve_account` → Unsupported „Android hosts run commands only as the app“ (`:335-342`).
- Supervisor ist ein Thread der App, nicht ein versteckter Prozess (`os/android/exec.rs:344-345`).
- Jeder Befehl läuft unter `/system/bin/sh` in einem Subreaper-Zwischenprozess (`os/android/exec.rs:40-47`; `os/android/exec_contain.rs:160-170`, `prctl(PR_SET_CHILD_SUBREAPER)` `:289`). Abbruch tötet die Prozessgruppe (`exec_contain.rs:398-401`, `:476`).
- Terminal: PTY, SELinux erlaubt PTYs (Kommentar `os/android/exec.rs:311-313`).
- Detailtext des Providers (`os/android/exec.rs:42-46`, Code-Text, vollständiger Satz): „Grenzen von Android: Befehle enden mit der App, im Hintergrund kann Android Kindprozesse beenden (ab Android 12 höchstens 32 solcher Prozesse systemweit) oder einfrieren, und Befehle kommen nur an, solange der Share-Dienst läuft.“ Die Zahl 32 ist nicht im Repository geprüft.

### Frage 4: Konto des normalen Workers je OS

| OS | Belegter Befund | Beleg | Lücke |
|---|---|---|---|
| Linux | Der Code kennt root- und Nutzerworker. `provider_status` setzt `elevated` = euid 0 (`os/linux_os/exec.rs:276`). Root → Systembus, sonst Nutzerbus (`os/linux_os/exec_systemd.rs:85-96`). Doku: „a normal user worker starts a normal user process“ und „a root Linux worker starts a root process“ (`docs/REMOTE_EXECUTION_PLAN.md:66-68`). Plan 2026-10-16: Transient-Unit „ohne `User=`“ (`docs/plaene/2026-10-16-luecken-schliessen/plan.md:49`). | siehe Spalte | Die Installationsform (root oder Nutzer, Dienst oder Sitzung) steht in den gelesenen Dateien nicht. Eigenschaften der Unit (`exec_systemd.rs:60-70`) nicht gelesen. L8. |
| Windows | Kommentar „per-user worker“ (`os/windows/exec.rs:247`). Doku: „an elevated Windows worker starts an elevated process“ (`docs/REMOTE_EXECUTION_PLAN.md:67`). | siehe Spalte | Ob Dienst oder Nutzerprozess: nicht belegt (L8). |
| Android | Konto ist die App-UID; Supervisor ist ein Thread der App (`os/android/exec.rs:329-331`, `:344-345`). | siehe Spalte | keine |

### Frage 5: Prüfkette einer Anfrage

Reihenfolge (Code):
1. Transport: Iroh-TLS weist die EndpointId vor jedem Share-Frame nach (`core/auth_penalty.rs:10-12`, Doku). Accept-Schleife liest `remote_id` (`core/node_accept.rs:46`).
2. Knoten-Tor: Strafsperre (`core/node_accept.rs:51-63`); Handshake-Pool mit Kennung bekannt/unbekannt (`:66-73`); Frist 3 s für unbekannte, 20 s für bekannte Endpunkte (`:64`; Konstanten `:131-132`); Ablehnung schließt die Verbindung mit Code 2 und Grund `Share admission rejected` (`:133-135`).
3. ALPN `EXEC_ALPN` → `exec_server::handle_connection` (`core/node_accept.rs:144`).
4. Serverhello mit 32-Byte-Challenge (`core/exec_admission.rs:32-38`).
5. Clienthello lesen; Fehler oder Frist → Strafe `NoAuthentication` (`:45-61`).
6. `authorize_client_hello_in` (`core/exec_auth.rs:91-160`); Reihenfolge laut Kommentar `:89-90`: Protokoll und Identität, dann Beweis, dann Exec-Recht.
   - Protokoll-, Capability-, Node-ID-, Server- und Nonce-Prüfung (`:207-229`); Fehler → Authentication.
   - Relation `direct` (Lookup-ID, `:98-99`): Direktverbindung muss online sein (`:100-102`); es muss ein akzeptierter `DirectGrant` mit exakter Identität existieren (`:103-113`); HMAC (`:114-120`); Fingerprint (`:121-123`); Ergebnis `&grant.exec` (`:124`).
   - Relation `room` (`:126-153`): Raum mit `room_id` und `auto_join` (`:127-131`); Mitglied mit exakter Identität und `is_admitted()` (`:132-142`); Raumgeheimnis (`:143-145`); HMAC (`:146-148`); Fingerprint (`:149-151`); Ergebnis `&member.exec` (`:152`).
   - Andere Relation → Authentication (`:154`).
   - Schalter: `!grant.enabled` → NotAuthorized (`:156-158`). Dann `authorized()` mit Revision, Epoche und einer 18-Byte-Sitzungs-ID (`:159`, `:162-184`).
7. Strafe: nur `Authentication` zählt als fehlgeschlagene Authentifizierung (`core/exec_auth.rs:36-38`); `NotAuthorized` (Schalter aus, Peer offline) zählt nicht (`:33-35`, Test `:365-378`). Exec-Verbindungen werden bei Strafsperre immer abgewiesen (`core/auth_penalty.rs:44-46`).
8. Bindung: `bind_incoming_principal`, `apply_authorization` (`core/exec_admission.rs:97-109`).
9. HelloOk mit `authorization`, `provider`, `features` (`:122-134`).
10. Start: `host_denial` bei Terminal ohne Plattformunterstützung (`core/exec_terminal.rs:76-81`; Aufruf `core/exec_admission.rs:159-169`); dann `grant_launch` (`:172-185`).
11. Registrierung `prepare` (`core/exec_registry.rs:163-238`): Policy vorhanden und aktiv (`:178-183`); Revision und Epoche (`:184-189`); Doppelstart (`:190-203`); globale Grenze und Grenze je Peer (`:204-214`).

Rollen:
- Direkt-Kontakte (`direct_contacts`): nicht Teil der Exec-Prüfung. `core/exec_auth.rs` liest nur `direct_grants` (`:103-105`). `direct_contacts` erscheint in `core/node_accept.rs:111-120` nur zur Einstufung „bekannt“ (Frist, Strafe). Die Exec-Zielliste enthält nur `direct_grants` (`core/exec_targets.rs:42-65`).
- Räume: Der Raum selbst hat keinen Exec-Grant. Jedes Mitglied hat einen eigenen `exec` (`core/exec_targets.rs:66-93`; `core/exec_auth.rs:132-153`). Sperre: `base_authorized = auto_join && !blocked` (`core/exec_targets.rs:88`); Persistenzprüfung `member.blocked` (`core/exec_grant_persistence.rs:218-222`).
- `is_admitted()` ist in `core/room_relation_members.rs:69` definiert (per Suche gefunden, nicht gelesen). Lücke L6.
- Rückfrage pro Sitzung: keine in den gelesenen Exec-Dateien. Suche nach `confirm`, `Rückfrage`, `prompt`, `approv`, `ask`, `consent`, `interactive` ergab nur `confirm_empty` (Prozesshülle) und `ContainmentNotConfirmed` (`core/exec_registry.rs:74`, `:337-342`). Die Entscheidung für Erlauben liegt außerhalb der Lektüre (L7).

### Frage 6: Protokoll, Strafkonto, Abbruch, Laufzeitgrenzen

Protokoll (`core/exec_protocol.rs`, `core/exec_types.rs`):
- Rahmen: Kopf `HEADER_BYTES` = 5 (`exec_protocol.rs:14`).
- Handshake-Nachrichten: `SERVER_HELLO` 1, `CLIENT_HELLO` 2, `HELLO_OK` 3, `HELLO_ERROR` 4 (`:16-19`).
- Client → Host: `START` 16, `STDIN` 17, `STDIN_EOF` 18, `CANCEL` 19, `PING` 20, `RESULT_ACK` 21, `RESIZE` 22 (`:20-26`).
- Host → Client: `STARTED` 32, `STDOUT` 33, `STDERR` 34, `TERMINAL` 35, `EXEC_ERROR` 36, `PONG` 37, `RESULT_ACKNOWLEDGED` 38 (`:27-33`).
- Größen: Handshake-Rahmen 64 KiB (`exec_protocol.rs:12`); Resize 256 Byte (`:13`); Start 128 KiB JSON (`exec_types.rs:11`, `:108-111`); Daten 64 KiB (`exec_types.rs:12`).
- Supervisor-Protokoll (intern): Steuerung 512 KiB, Daten 64 KiB, Resize 256 Byte (`core/exec_supervisor_protocol.rs:8-10`).
- Audit: JSON-Objekt je Zeile, Ablage im App-Datenordner (`os/shared/exec_audit.rs:4`, `:78-80`); Klassen `Executions` und `Rejections` (`os/shared/exec_audit_files.rs:30`); Rotation ab 4 MiB (`:19`); Anzeige höchstens 1000 Einträge (`os/shared/exec_audit_view.rs:16`); Feldtext höchstens 256 Zeichen (`os/shared/exec_audit_text.rs:16`); Doku `exec_audit.rs:7-8`: kein Text eines Shell-Befehls im Log.

Strafkonto (`core/auth_penalty.rs`; Modell nach OpenSSH `PerSourcePenalties`, Doku `:3-9`):
- `AUTH_FAILURE` 5 s (`:17`); `NO_AUTHENTICATION` 1 s (`:18`).
- `MIN_PENALTY` 15 s: ab dieser Summe wird die Quelle abgewiesen (`:19`, `:99-100`).
- `MAX_PENALTY` 600 s (`:20`); `MAX_SOURCES` 65 536 (`:21`); Endpunktkennung höchstens 128 Byte (`:80`). Bei vollem Speicher ohne abgelaufene Einträge wird die Quelle nicht gezählt (`:87-91`).
- Abweisung: `refusal` liefert Restzeit solange `active` (`:63-76`); Schließen mit `Share admission rejected` (`core/node_accept.rs:133-135`); Audit-Ereignis `PenaltyRefused` (`:55-59`).
- Auslöser: `AuthFailure` bei Authentication-Fehler (`core/exec_admission.rs:66-72`); `NoAuthentication` bei fehlendem Clienthello (`:54-58`).
- Der Speicher ist ein In-Memory-`HashMap` (`core/auth_penalty.rs:56-59`); persistiert wird er in den gelesenen Dateien nicht.

Abbruch (`core/exec_registry.rs`):
- Gründe `ExecCancelReason`: `User`, `Timeout`, `Revoked`, `Disconnected`, `WorkerStopping` (`:16-23`).
- Endzustand je Grund (`:434-447`): `User` und `WorkerStopping` → `Cancelled`; `Timeout` → `TimedOut`; `Revoked` → `Revoked`; `Disconnected` → `Disconnected`; ohne Grund → `Exited` oder `Failed`.
- Auslöser: `cancel` (`:300-308`), `cancel_exact` nach Peer-Geräte-ID (`:310-326`), `cancel_all` (`:328-331`), `stale_job` mit `Revoked` (`:409-418`).
- Endzustände `ExecTerminalKind`: `Exited`, `Failed`, `TimedOut`, `Cancelled`, `Revoked`, `Disconnected` (`core/exec_types.rs:166-175`).
- Vor Endzustand muss die Prozesshülle leer bestätigt sein; sonst `ContainmentNotConfirmed` (`core/exec_registry.rs:337-342`).
- Heartbeat: Intervall 5 s, Peer-Timeout 20 s, Schreib-Timeout 10 s (`core/exec_heartbeat.rs:35-39`). Server-Frist bis ResultAck = Peer-Timeout + Schreib-Timeout + Intervall = 35 s (`exec_heartbeat.rs`, Funktion `server_result_ack_timeout`, Formel; Zeilen ab 31 nicht gelesen, L13).
- Das Ende einer Sitzung bei Peer-Ausfall: Heartbeat-Frist `last_ping + peer_timeout` (`core/exec_server.rs:52`, `:118`).

Laufzeitgrenzen (Werte):

| Grenze | Wert | Beleg |
|---|---|---|
| Handshake unbekannter Endpunkt | 3 s | `core/node_accept.rs:131` |
| Handshake bekannter Endpunkt | 20 s | `core/node_accept.rs:132` |
| Exec-Handshake und erster Start-Rahmen | 20 s | `core/exec_server.rs:24`; `core/exec_admission.rs:18`, `:149-151` |
| Client: Handshake, Ereignis, Fehlerereignis | 15 s, 30 s, 1 s | `core/exec_client.rs:22-24` |
| Client: Eingabe-, Ereigniswarteschlange | 16, 32 Einträge | `core/exec_client.rs:25-26` |
| Globale aktive Jobs | 8 | `core/exec_registry.rs:96` |
| Aktive Jobs je Peer | 2 | `core/exec_registry.rs:97` |
| Verlauf, Terminal-Cache | je 128 | `core/exec_registry.rs:98-99` |
| Laufzeit (`timeout_ms`) | optional; fehlend oder 0 = unbegrenzt; keine Obergrenze im Live-Pfad | `core/exec_types.rs:139-141`, `:74-113` |
| Ausgabegrenze (`max_output_bytes`) | optional; fehlend oder 0 = unbegrenzt; keine Obergrenze im Live-Pfad | `core/exec_types.rs:143-145`; `core/exec_job.rs:288` |
| Start-Rahmen | 128 KiB | `core/exec_types.rs:11`, `:108-111` |
| Argumente im Live-Pfad | keine Anzahl- oder Größengrenze | `core/exec_types.rs:74-113` |
| Umgebung: Liste erlaubt | `LANG`, `TERM`, `LC_*` | `core/exec_policy.rs:251-261` |
| Umgebung: Wert der Liste | höchstens 64 Byte | `core/exec_policy.rs:21`, `:264-266` |
| Kontoname | höchstens 256 Byte | `core/exec_policy.rs:18`, `:61` |
| Programmname | höchstens 4096 Byte | `core/exec_policy.rs:19`, `:72` |
| Terminal-Größe | 1 bis 32 767 Spalten und Zeilen | `core/exec_terminal.rs:18`, `:29-38` |
| Terminal-Typ `TERM` | 1 bis 64 Zeichen aus `[A-Za-z0-9._+-]` | `core/exec_terminal.rs:19`, `:50-62` |
| Containment-Stopp | 15 s | `core/exec_job.rs:21` |
| Wurzel-Ausgabe-Nachlauf | 250 ms | `core/exec_job.rs:20` |
| Ereignisabfrage, Eingabewiederholung | 25 ms, 5 ms | `core/exec_job.rs:17`, `:19` |
| Linux-Kontopuffer, Obergrenze, Gruppen | 16 KiB, 1 MiB, 65 536 | `os/linux_os/exec_account.rs:12-15` |
| Android: Beenden, Rest, Freigabe | 2 s, 3 s, 5 s | `os/android/exec_contain.rs:35`, `:37`; `os/android/exec.rs:47` |

Der Abschnitt `core/exec.rs` (alter Pfad mit `ExecRequest`) enthält zusätzlich Grenzen: Laufzeit höchstens 15 min (`:10`, `:146`), Ausgabe höchstens 2 MiB (`:11`, `:140`), 256 Argumente, 64 KiB gesamt, 16 KiB je Argument (`:12-14`, `:106-128`), Verzeichnis 4 KiB (`:15`, `:132`), 2 global und 1 je Peer (`:16-17`). Diese Werte gelten nur für diesen Pfad; `PreparedExec::run` schlägt immer fehl (`:95-98`). Ob dieser Pfad im Live-Betrieb erreichbar ist: Lücke L5.

### Frage 7: Ablehnungs- und Fehltexte, die der Client sieht

Vollständige Liste der Produktionstexte in `core/exec_policy.rs` und `core/exec_program_path.rs` (Tests ausgenommen). „Peer“ = Text kommt über `program_not_permitted` zum Peer; der Pfad ist belegt in `core/exec_admission.rs:172-185` und `:300-305`.

`core/exec_policy.rs`:
- `:67` `invalid local account name {name:?}` (Grant-Setzen, nicht Peer)
- `:75` `invalid allowed program {program:?}` (Grant-Setzen, nicht Peer)
- `:82-83` `allowed program {program:?} must be a bare name or an absolute path` (Grant-Setzen, nicht Peer)
- `:98-100` `this host runs commands only as the Smart Explorer user (Standardnutzer); another account needs a Linux worker running as root` (Funktion `validate_for_this_host`; im Bestand ohne Aufrufer in den gelesenen Ordnern, L2)
- `:174-176` `this host allows this device only its listed programs, no shell commands` (Peer)
- `:180-182` `this host allows this device only its listed programs, no terminal` (Peer)
- `:187-189` `this host allows this device only its listed programs; the environment variable {name:?} is not allowed (only LANG, LC_* and TERM)` (Peer)
- `:193-195` `this host allows this device only its listed programs; the value of {name} must be a plain locale or terminal name` (Peer)

`core/exec_program_path.rs`:
- `:35-36` `program {program:?} names a directory but is not an absolute path` (Peer)
- `:57-59` `program {program:?} is not an absolute path to a file and not in this host's fixed search path` (Peer)
- `:77-79` `this host allows this device only its listed programs; {program:?} is not one of them` (Peer)

Wire-Antworten an den Peer (`ExecWireError { code, message }`):
- `permission_denied` / `exec authentication failed` (`core/exec_admission.rs:79-82`). Gilt für jede Handshake-Ablehnung, auch bei ausgeschaltetem Grant (`core/exec_auth.rs:21-23`, `:156-158`). Der Peer kann daher nicht unterscheiden, ob ein Grant existiert (Test `core/exec_auth.rs:365-371`).
- `already_running` / `execution {exec_id} is already running` (`core/exec_admission.rs:213-215`).
- `admission_denied` / Text = Debug-Format von `ExecRegistryError` (`core/exec_admission.rs:235-237`; `core/exec_registry.rs:79-83`): `InvalidAuthorization`, `InvalidStart`, `NotAuthorized`, `StaleAuthorization`, `DuplicateMismatch`, `GlobalLimit`, `PrincipalLimit`, `UnknownExecution`, `NotPreparing`, `ContainmentNotConfirmed`, `TerminalReasonMismatch`, `LaunchFailed(<Text in Anführungszeichen>)`.
- `account_unavailable` / `run-as account {name:?} of this device: {error}` (`core/exec_admission.rs:293-296`). Fehlertexte: `local account {name:?} does not exist on this host` (`os/linux_os/exec_account.rs:46`); `only a Smart Explorer worker running as root can run commands as another account` (`os/linux_os/exec.rs:320`); `Windows hosts run commands only as the Smart Explorer user` (`os/windows/exec.rs:256-257`); `Android hosts run commands only as the app` (`os/android/exec.rs:340`). Zusätzlich `account_unavailable` mit dem Text des Join-Fehlers (`core/exec_admission.rs:278-281`).
- `program_not_permitted` (`core/exec_admission.rs:300-305`): trägt alle Texte aus `exec_policy.rs` und `exec_program_path.rs` oben, ebenso Fehler aus `authorize_start` im Join-Fehlerfall: `the program check of this host did not complete`.
- `terminal_unsupported` / `this host cannot provide a terminal for remote commands` (`core/exec_terminal.rs:76-81`).
- Clientseitig: `terminal_unsupported` / `the peer's Smart Explorer has no terminal support; update it` (`core/exec_terminal.rs:93-98`).
- HelloOk-Provider (kein Fehlercode; `provider` und `detail`): Linux `systemd transient cgroup`, Detail `cgroup v2 and the systemd manager are available` (`os/linux_os/exec.rs:281-282`) oder Fehlertext von `require_cgroup_v2` oder `manager_connection` (`:288-291`; ein belegter Wortlaut: `cgroup v2 is unavailable`, `os/linux_os/exec_systemd.rs:191`; die übrigen Fehlertexte: Lücke L19); Windows `Windows Job Object`, Detail `kill-on-close Job Objects are available` (`os/windows/exec.rs:211-212`); Android `android-subreaper`, Detail DETAIL (`os/android/exec.rs:42-46`).
- Host-Validierung, Weg zum Peer nicht belegt (L12): `exec id must contain exactly 32 hexadecimal characters` (`core/exec_types.rs:27-28`); `remote program is empty` (`:81`); `remote shell command is empty` (`:90`); `remote working directory is empty` (`:98`); `environment name is not representable by the OS` (`:105`); `remote execution start frame exceeds 128 KiB` (`:110`); `a terminal is only valid for the login shell` (`:120`); `an interactive login shell needs a terminal` (`:121`); `remote program contains a NUL byte`, `remote argument contains a NUL byte`, `remote shell command contains a NUL byte`, `remote working directory contains a NUL byte`, `environment name contains a NUL byte`, `environment value contains a NUL byte` (`:233-238`); `terminal columns and rows must be between 1 and 32767` (`core/exec_terminal.rs:35-36`); `terminal type must have 1 to 64 characters` (`:53`); `terminal type may only contain letters, digits and . _ + -` (`:58-59`).
- Nur im alten Pfad `core/exec.rs` (Lücke L5): `remote full-code execution is not enabled for this account` (`:72-74`); `remote execution is disabled: full process-tree containment is unavailable` (`:96-97`).
- Linux-Supervisor: `a run-as account needs the Linux systemd host` (`os/linux_os/exec_supervisor.rs:329`); Weg zum Peer nicht belegt (L12). Linux-Konto (`os/linux_os/exec_account.rs:94`): `the group list of the run-as account could not be read`.

## Wiederverwendbare Bausteine

- Modell und Validierung: `ExecRunAs`, `ExecGrantSettings`, `validate`, `validate_for_this_host` (`core/exec_policy.rs:24-105`).
- Altleser-Schutz im Speicherformat: Feld `restricted_enabled` mit `enabled` false (`core/exec_policy_wire.rs:33-36`, `:60-74`).
- Programmauflösung ohne `PATH` des Aufrufers, mit Kanonisierung und Namensgleichheit (`core/exec_program_path.rs:30-100`).
- Feste Suchpfade je OS (`core/exec_supervisor_protocol.rs:48-49`; `os/linux_os/exec_program_search.rs:10-15`; `os/windows/exec.rs:229-241`; `os/android/exec.rs:319-327`).
- Kontoauflösung Linux über `getpwnam_r` und `getgrouplist` (`os/linux_os/exec_account.rs:18-99`).
- Umgebung eines Kontos neu aufgebaut (`core/exec_supervisor_protocol.rs:84-103`).
- Konto-Wechsel vor `exec` mit Rückholsperre (`os/linux_os/exec_supervisor_account.rs:23-69`).
- Strafkonto mit begrenzter Quellenzahl und Ablauf (`core/auth_penalty.rs:17-102`).
- Zulassung getrennt: `authorize_client_hello_in` und `handle_connection` (`core/exec_auth.rs:91-160`; `core/exec_admission.rs:10-305`).
- Registry mit Grenzen, Doppelstart-Erkennung und Terminal-Cache (`core/exec_registry.rs:85-238`, `:333-386`).
- Zielansicht je Grant mit Schlüssel `direct/<deviceId>/<fingerprint>` oder `room/<roomId>/<deviceId>/<fingerprint>` (`core/exec_targets.rs:17-111`).
- Terminal-Feature mit Client- und Host-Gate (`core/exec_terminal.rs:11-99`).
- Heartbeat-Policy mit abgeleiteten Fristen (`core/exec_heartbeat.rs:35-39`).
- Audit mit Klassen und Rotation (`os/shared/exec_audit.rs:78-91`; `os/shared/exec_audit_files.rs:19`, `:30`).

## Betroffene Verträge

- Persistenz: Profil-JSON, Felder `direct_grants[].exec` und `rooms[].members[].exec` mit dem Wire-Typ aus `core/exec_policy_wire.rs`. Schemaversion `SHARE_PROFILE_VERSION` (Wert nicht gelesen, L3). Daemon-Journal `exec_grant_journal.rs` nur in `docs/lesungen/2026-09-26-exec-host-flow.md` (Zeilen 20 und 106-110) genannt, nicht gelesen.
- Netz: ALPN `smart-explorer/share-exec/2`; `EXEC_PROTOCOL_VERSION` 2; Capability `exec_stream_v2`; Transkript `v2`; `ExecStart` ohne `run_as` und `programs`; `ExecHelloOk` mit `ExecProviderStatus` inklusive `user_label` und `elevated`; Feature `exec_terminal_v1`.
- IPC: `ExecGrantMutation` ist `Serialize` und `Deserialize` und trägt `policy: ExecGrant` mit `run_as` und `programs` (`core/exec_grant_runtime.rs:17-23`). Die IPC-Nachricht `MutateExecGrant { target, enabled }` ist laut `docs/plaene/2026-10-16-luecken-schliessen/plan.md:52` die Schnittstelle; ihr Handler wurde nicht gelesen (L14).
- Bridge (Android): `core/exec_targets.rs:1-2` nennt die „Android facade“; deren Code wurde nicht gelesen (L14).
- CLI: `se share grants exec enable|disable` (`docs/plaene/2026-10-16-luecken-schliessen/plan.md:52`); `cli/share/grants_exec.rs` nicht gelesen (L14).
- GUI: Desktop-Share-Seite nach `core/exec_targets.rs:1-2`; `app/core/share_exec_ui.rs` (laut `docs/lesungen/2026-09-26-exec-host-flow.md:97`) nicht gelesen (L14).

## Lücken und offene Punkte

- L1: Standardwerte, die CLI, GUI, MCP und Android beim Erlauben übergeben. Die Aufrufer von `prepare_persisted` liegen außerhalb der gelesenen Ordner.
- L2: Aufrufer von `validate_for_this_host` (`core/exec_policy.rs:95`), von `reset_all_for_legacy_migration` (`core/exec_policy.rs:273`) und von `prepare_persisted` (`core/exec_grant_persistence.rs:11`) sind nicht gelesen.
- L3: Wert von `SHARE_PROFILE_VERSION` und die Migrationsstufen des Profils (Dateien außerhalb der gelesenen Ordner).
- L4: Zuordnung Release zu Protokollversion. Unbelegt: ob 0.5.180 und 0.5.182 `EXEC_PROTOCOL_VERSION` 2 sprechen und ob die Version je angehoben wurde.
- L5: Ob `core/exec.rs` (alter Pfad mit `ExecRequest`, `PreparedExec::run` mit Fehler `Unsupported`) im Live-Betrieb erreichbar ist. Die Lesung `docs/lesungen/2026-09-26-exec-host-flow.md:89` nennt `run()` als fail-closed; die Aufrufer sind nicht gelesen.
- L6: Definition `is_admitted()` in `core/room_relation_members.rs:69` (Suchfund, nicht gelesen).
- L7: Rückfrage beim Host pro Sitzung oder pro Grant: in den gelesenen Exec-Dateien nicht vorhanden; Oberflächen (GUI, CLI, Android) nicht gelesen.
- L8: Betriebsform des Workers je OS (Linux root oder Nutzer; Windows Dienst oder Nutzerprozess; Unit-Eigenschaften in `os/linux_os/exec_systemd.rs:60-70` nicht gelesen).
- L9: Linux: Verhalten bei Scripts und `.cmd`-Pfaden; PAM-Freiheit für `os/linux_os/exec_pty.rs` nur per Suche; Startwerk der Supervisor-Namenssuche in `os/linux_os/exec.rs` und `os/linux_os/exec_supervisor.rs` nicht vollständig gelesen.
- L10: Namensauflösung des Supervisors für nicht gepinnte Programme (`ls` ohne Liste): liegt in `os/linux_os/exec_supervisor.rs` und `os/windows/exec_supervisor.rs`, nicht vollständig gelesen.
- L11: Weitere Begrenzungen von Umgebungsnamen und Werten bei voller Shell (über `ExecStart::validate` hinaus): nicht gefunden in den gelesenen Dateien; nicht vollständig geprüft.
- L12: Weg der Host-Validierungsfehler (`core/exec_types.rs:27-121`, `:233-238`, `core/exec_terminal.rs:35-59`) und des Supervisor-Fehlers `os/linux_os/exec_supervisor.rs:329` zum Peer: nicht belegt.
- L13: Client-Frist `client_result_acknowledged_timeout` (Zeilen ab 31 in `core/exec_heartbeat.rs`) nicht gelesen.
- L14: Oberflächen und Brücken (IPC-Handler, CLI, GUI, Android-Brücke, MCP): nicht gelesen.
- L15: Windows-Job-Flags (`os/windows/exec.rs:284-292`) nicht gelesen; nur Detailtext.
- L16: Android-Behauptung „ab Android 12 höchstens 32 solcher Prozesse systemweit“ (`os/android/exec.rs:42-46`): Text im Code, nicht geprüft.
- L17: Veraltete Dokumentstellen: `docs/lesungen/2026-09-26-exec-host-flow.md` nennt `exec_auth.rs:54-117` und beschreibt die Prüfung `grant.exec.enabled && fingerprint_matches(...)` vor der HMAC-Prüfung (Zeilen 41-44). Der Code hat `authorize_client_hello_in` bei `:91-160` und prüft HMAC (`:114-120`) vor `enabled` (`:156-158`). `docs/plaene/2026-10-16-luecken-schliessen/plan.md:48-52` beschreibt den Stand vor Umsetzung (ohne `run_as` und `programs`).
- L18: Keine Testausführung und kein Build. Die Testnamen sind aus `core/exec_policy_tests.rs`, `core/exec_profile_compat_tests.rs` und Modultests belegt, nicht ihre Ergebnisse.
- L19: Vollständige Fehlertexte von `require_cgroup_v2` und `manager_connection` (`os/linux_os/exec_systemd.rs:85-96`, `:185-191`) nicht gelesen; nur der Wortlaut `cgroup v2 is unavailable` (`:191`) ist belegt.

## Grenzen der Lektüre

Methode: vollständige Dateien mit dem Read-Werkzeug; Ausschnitte und Suchen mit grep und sed, begrenzt auf die freigegebenen Ordner. Git nur lesend (`git log -1 --oneline`, `git rev-parse HEAD`). Kein graphify-Aufruf, weil die Vorgabe nur diese eine Ausgabedatei als Änderung erlaubt und nicht geprüft wurde, ob `graphify query` Dateien schreibt (Annahme). Keine Build-, Test- oder Paketbefehle. Keine Zwischendateien.

Gelesen vollständig (18 Dateien):
- `core/`: `exec.rs`, `exec_admission.rs`, `exec_auth.rs`, `exec_grant_persistence.rs`, `exec_grant_runtime.rs`, `exec_platform.rs`, `exec_policy.rs`, `exec_policy_wire.rs`, `exec_program_path.rs`, `exec_registry.rs`, `exec_targets.rs`, `exec_terminal.rs`, `exec_types.rs` (13 Exec-Dateien), dazu `node_accept.rs` und `auth_penalty.rs`.
- `os/linux_os/`: `exec_account.rs`, `exec_program_search.rs`, `exec_supervisor_account.rs`.

Teilweise gelesen (Zeilenbereiche oder Konstanten, 20 Dateien):
- `core/`: `exec_client.rs` (Konstanten), `exec_heartbeat.rs` (Zeilen 1-30, 35-39), `exec_job.rs` (17-21, 102-115, 288), `exec_policy_tests.rs` (Funktionsnamen, Zeilen 232-300), `exec_profile_compat_tests.rs` (Zeilen 1-60, 83-118), `exec_protocol.rs` (Zeilen 8-14, Konstanten 16-33), `exec_server.rs` (Suchtreffer, Konstante 24), `exec_supervisor_protocol.rs` (Zeilen 36-52, 80-120, Konstanten 8-22).
- `os/linux_os/`: `exec.rs` (Zeilen 41-45, 270-330), `exec_supervisor.rs` (Zeilen 21-49, 318-335, Suchtreffer), `exec_systemd.rs` (Zeilen 8-11, 70, 80-96).
- `os/windows/`: `exec.rs` (Zeilen 31, 205-262, Suchtreffer), `exec_conpty.rs` (Suchtreffer, Zeile 105), `exec_supervisor.rs` (Suchtreffer).
- `os/android/`: `exec.rs` (Zeilen 39-47, 293-345), `exec_contain.rs` (Zeilen 1-3, 33-42, 160-170, 289, 398-401, 476).
- `os/shared/`: `exec_audit.rs` (Zeilen 1-10, 20-26, 54-99, 170, 186, Suchtreffer), `exec_audit_files.rs` (Zeilen 19, 30), `exec_audit_text.rs` (Zeile 16), `exec_audit_view.rs` (Zeile 16).

Nur über Suche berührt, nicht gelesen (Exec-Dateien):
- `core/`: `exec_client_active.rs` (Suchtreffer Zeile 99), `exec_client_active_tests.rs`, `exec_client_heartbeat_integration_tests.rs`, `exec_client_tests.rs`, `exec_configuration_sync_task_tests.rs`, `exec_frame_reader.rs`, `exec_grant_persistence_tests.rs`, `exec_grant_runtime_tests.rs`, `exec_protocol_tests.rs`, `exec_registry_authority.rs`, `exec_registry_tests.rs`, `exec_registry_view.rs`, `exec_server_tests.rs`, `exec_session.rs`.
- `os/linux_os/`: `exec_pty.rs` (Suchtreffer), `exec_pty_tests.rs` (Suchtreffer), `exec_self_test.rs` (Konstanten).
- `os/windows/`: `exec_handles.rs` (Kopfkommentar), `exec_conpty_tests.rs`, `exec_self_test.rs`, `exec_self_test_process.rs` (nicht berührt).
- `os/android/`: `exec_proc.rs`, `exec_proc_tests.rs` (nicht berührt).
- `os/shared/`: `exec_audit_tests.rs` (nicht berührt).

Dokumente (nur Trefferzeilen oder Abschnitte, nicht vollständig):
- `docs/REMOTE_EXECUTION_PLAN.md`: Zeilen 4, 10-14, 21, 30, 34, 41-43, 58-68, 70-77.
- `docs/lesungen/2026-09-26-exec-host-flow.md`: Zeilen 6-21, 35-60, 89, 95-111.
- `docs/refs/luecken-2026-10-16.md`: Zeilen 9, 48, 64, 66, 69, 104, 118-120, 122, 127, 137.
- `docs/plaene/2026-10-16-luecken-schliessen/plan.md`: Zeilen 5, 12, 14, 18, 20, 28-29, 31, 35, 40-52, 61, 68, 72, 81, 86, 90-94.

Nicht geprüft: Aufrufer außerhalb der freigegebenen Ordner, Oberflächen, Daemon-Journal, Profil-Migration, Schemaversion, Laufzeitumgebung und Installationsform.
