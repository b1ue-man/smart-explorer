# S1 — Exec-Rechte nutzergebunden, Kontowechsel je OS

Stand: 2026-10-17 (Endfassung nach der Nachrecherche), Lesestand `da2646d2`. Teil des Plans `plan.md` (Ergebnisse E1, E2, E3). Nichts in dieser Datei ist gebaut oder ausgeführt (AGENTS.md: keine lokalen Builds/Tests). Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`, „Lesung Host“ = `docs/lesungen/2026-10-17-exec-rechtemodell-host.md`, „Lesung Oberflächen“ = `docs/lesungen/2026-10-17-exec-rechte-oberflaechen.md`, „Ref Win“ = `docs/refs/exec-konto-windows-2026-10-17.md`, „Ref Linux“ = `docs/refs/exec-konto-linux-2026-10-17.md`, „Ref Android“ = `docs/refs/exec-konto-android-2026-10-17.md`, „Ref Modelle“ = `docs/refs/fernzugriff-rechtemodelle-2026-10-17.md`, „NR“ = `docs/plaene/2026-10-17-rechte-desktop-tresore/nachrecherche-rechte.md` (Antworten N1–N24, Revalidierung, Folgen 1–13), „S4“ = `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-bildschirm.md` (Endfassung, 542 Zeilen, Abschnitt E1).

## Schleifenstand

**Ziel:** Ein Zugriffsrechtesystem für Befehle und Terminals (und, über den Vertrag in Abschnitt E, für Bildschirm ansehen/steuern aus S4), das ein Mensch mit wenigen Entscheidungen einrichtet, das an ein Betriebssystemkonto gebunden ist (das OS setzt Datei- und Prozessrechte durch) und auf Wunsch eingehende Sitzungen unter einem anderen Konto als dem des Smart-Explorer-Workers startet. Quelle: E1–E3 in `plan.md` A.

**Kriterien** („erreicht“ heißt hier: für den Planungsteil ist die Entscheidung belegt und das Abnahmesignal der Remote-Suite definiert; die Umsetzung bestätigt erst die Suite):

| Kriterium | Herkunft | Nachweis | Status |
|---|---|---|---|
| K1 Gewöhnliche Befehle (`ls`) laufen nach dem Erlauben ohne Einzelfreigabe | E1, Nutzer „even ls would be blocked“ | Bedienweg + Suite-Test „erlauben → `ls` läuft“ | erreicht: Befund B2 (volle Shell blockiert `ls` nicht), Entscheidung C1/C6; Signale `perm_task_allow_own_ls_runs`, `perm_task_remote_account_ls_runs_linux_root`, `perm_task_linux_helper_setup_and_remote_account`, `perm_task_windows_helper_setup_remote_account` |
| K2 Rechte an Nutzer/Konto gebunden, OS setzt sie durch | E1/E2 „user bound“, „handled on the actual operating system“ | Entwurf + Kontowechsel-Test je OS | erreicht: Linux über systemd `User=` mit eigenem PAM-Dienst (C3; NR N7, N8, N20), Windows über S4U-Anmeldung lokaler Konten (C2; NR N1 indirekt belegt über Microsoft-Doku zu OpenSSH und OpenSSH-Quelltext); Einhegung in beiden Wegen neu entschieden (C2, C3); Signale je OS in PRM-M5 bis PRM-M7 |
| K3 Anderes OS-Konto als das von se wählbar (Windows, Linux; Android soweit möglich) | E2 | Recherche + Umsetzung je OS | erreicht: Linux root-Worker (PRM-M5), Linux Nutzer-Worker (PRM-M6), Windows (PRM-M7) mit Kontenliste aus einer Anmeldeprobe je Konto (macht die nicht belegbaren Fälle N1b, N19 zur angezeigten Tatsache statt zur Annahme); Android nur App-Konto mit belegtem Grund (C4) |
| K4 Android-Möglichkeiten recherchiert und ehrlich angezeigt | E3 | Ref Android + Android-UI | erreicht: Ref Android, NR N11/N21/N24; Text C4; Signal `perm_task_bridge_status_account_choices_android` |
| K5 Einrichtung mit wenigen Entscheidungen; Bedienaufwand vorher/nachher je Oberfläche | E1 „above accepatbl effort“ | Vorher/Nachher-Tabelle (C0–C4) | erreicht: Tabelle D.3, Zählung „vorher“ am Code bestätigt (NR N13); Signale `perm_task_gui_access_dialog_one_confirm`, `perm_task_android_access_dialog_two_actions` |
| K6 Keine Rechteerweiterung ohne bewusste Handlung des Host-Nutzers; bestehende Grants behalten ihre Wirkung | AGENTS.md preserve existing behavior | Migrationsentwurf + Test alter Grants | erreicht: Portierung D.2.1, Signale `perm_task_port_*`, `perm_task_old_reader_sees_new_grant_off` |
| K7 Alle Oberflächen (CLI Text/JSON/Vervollständigung, Desktop, Android, Bridge, IPC, MCP, Netz, Doku) | Nutzerregel, `docs/oberflaechen/INDEX.md` | Inventur `docs/oberflaechen/share-exec.md` | erreicht: C7, PRM-M8 bis PRM-M11; drei Ausnahmen mit Grund (MCP: Rückfragen entscheiden; MCP: Kontodienst einrichten/freigeben/aktualisieren/entfernen; Android: nur App-Konto) |

**Aktiver Stand:** Entwurf gewählt und nach der Nachrecherche korrigiert (C1–C8):
- Zugriffsrecht je Gerät und je Raummitglied mit „Ausführen als“ (`own`, `remote` = `se-remote`, `account` = benanntes Konto), Fähigkeiten `exec_commands`, `exec_terminal` (S4: `screen_view`, `screen_control`) je `off`/`ask`/`allow`, optionaler Programmliste und optionalem Ablauf.
- „Nur lesen“ entfällt in diesem Stapel (C8; NR N4, N9 widerlegen die Grundlage).
- Linux: systemd-Einheit mit `User=` und eigenem PAM-Dienst `smart-explorer-exec` ohne `pam_systemd` (Einhegung bleibt in der Einheit); `ExitType=main` wird entfernt (Bestandsdefekt auf systemd 249); Kontodienst für Nutzer-Worker als root-Socketdienst.
- Windows: Kontodienst (LocalSystem) mit S4U-Anmeldung `Network` + Primärtoken, Aufruferprüfung über `ImpersonateNamedPipeClient`, Job im Dienst (Sitzung 0), Verbindung des Relais über zwei vom Worker angelegte benannte Pipes; Kontenliste aus Anmeldeproben mit gemessener Token-Stufe.
- Android: nur App-Konto; Rückfrage über eigenen Kanal hoher Wichtigkeit; Bildschirm-Rückfragen mit Aktivitäts-Intent (S4).

**Nächster Schritt:** Umsetzung PRM-M1 bis PRM-M11; Entscheidungsbedarf beim Hauptagenten siehe Abschnitt F, Liste „Entscheidung des Hauptagenten“.

**Schleifenstatus:** erreicht (Planungsteil): K1–K7 wie oben belegt; die verbleibenden Annahmen A1–A12 (Abschnitt F) tragen je ein Abnahmesignal der Remote-Suite oder eine angezeigte Rückfallwirkung.

## B — Bestand dieses Strangs

Befunddateien (vollständig gelesen): Lesung Host, Lesung Oberflächen, Ref Win, Ref Linux, Ref Android, Ref Modelle, NR, `docs/oberflaechen/share-exec.md`, `docs/oberflaechen/INDEX.md`, `docs/ARCHITEKTUR.md` (Zeile „Exec-Host“, Zeile 66), S4 Abschnitte C7, SCR-M2, SCR-M8, E1. Eigene gezielte Code-Belege (Lesestand `da2646d2`) in der rechten Spalte.

| Nr. | Befund, der eine Entscheidung trägt | Beleg |
|---|---|---|
| B1 | Grant `ExecGrant` je Direktfreigabe (`DirectGrant.exec`) und je Raummitglied (`RoomMember.exec`); Felder `enabled`, `policy_revision`, `changed_at`, `source_request_id`, `source_decision_revision`, `run_as`, `programs`; Standard aus | Lesung Host Frage 1; `N/share/core/direct_relation.rs:74` |
| B2 | Volle Shell (leere Programmliste) lehnt nichts ab; `ls` läuft; Programmliste sperrt Shell-Befehle und Terminal, Argumente frei | Lesung Host Frage 2; `N/share/core/exec_policy.rs:156-183` |
| B3 | Gespeicherte Form: eingeschränkter Grant wird für Leser bis 0.5.180 mit `enabled:false` und `restricted_enabled:true` geschrieben; `run_as` ist ein intern markiertes Enum; `ExecGrantWire` erhält keine unbekannten Felder | `N/share/core/exec_policy_wire.rs:1-74`; `N/share/core/exec_policy.rs:24-35` |
| B4 | Folgerung aus B3: Eine neue `run_as`-Variante ließe Leser 0.5.181/0.5.182 beim Lesen scheitern; neue Bedeutungen brauchen ein neues Feld | `exec_policy.rs:24-26`, `exec_policy_wire.rs:29-30` |
| B5 | Profilschema `SHARE_PROFILE_VERSION` = 8 | `N/share/core/profiles.rs:17` |
| B6 | Kontowechsel heute nur Linux mit root-Worker (setuid im `pre_exec` des Supervisors, ohne PAM) | Lesung Host Frage 3; `N/share/os/linux_os/exec.rs:304-324` |
| B7 | Worker läuft je Nutzer: Linux XDG-Autostart, Windows `HKCU\…\Run` (`SmartExplorerSync`); Android als Thread der App | `N/autostart/os/linux_os.rs:4-20`, `N/autostart/os/windows.rs:1-2`, `N/daemon/mod.rs:1-14` |
| B8 | Installation je Nutzer und nutzerbeschreibbar: Linux `~/.local/opt/smart-explorer`, Windows `%LOCALAPPDATA%\Programs\Smart Explorer` (`RequestExecutionLevel user`) | `install-linux.sh:8-9`; `native/installer.nsi:14`, `:59-60` |
| B9 | Linux-Jobs laufen in einer transienten systemd-Einheit, Supervisor `/proc/<pid>/exe --share-exec-supervisor <socket>`, Socket `0600` des Worker-Kontos; Einhegung über `KillMode=control-group` und Prüfung `cgroup_populated` | `N/share/os/linux_os/exec_systemd.rs:41-72`; `N/share/os/linux_os/exec.rs:73-93` |
| B10 | Windows: Worker startet `se --share-exec-supervisor` per `CreateProcessW` mit `HANDLE_LIST` und `JOB_LIST`; der Supervisor liest Befehle von stdin, schreibt Ereignisse auf stdout, öffnet selbst die ConPTY und startet `powershell.exe -NoLogo` (Relais-Muster) | Ref Win §1.3; `N/share/os/windows/exec.rs:328-354`; `exec_supervisor.rs:22`; `exec_conpty.rs:108-126` |
| B11 | Prüfkette: Strafsperre → ALPN → Identität → HMAC über ein feldweises Transkript → Exec-Recht; nur `Authentication` zählt als Fehlversuch | Lesung Host Frage 5; `N/share/core/exec_auth.rs:231-247` |
| B12 | Folgerung aus B11: Ein zusätzliches Feld in `ExecClientHello` außerhalb des Transkripts bricht ältere Hosts nicht | `exec_auth.rs:231-247` |
| B13 | `ExecHelloOk.features` ist optional (`serde(default)`); Terminal-Merkmal `exec_terminal_v1` | `N/share/core/exec_protocol.rs:82-88`; `N/share/core/exec_terminal.rs:13`, `:66` |
| B14 | Einschränkungsrahmen: `RightsRestriction`, `RestrictionReason` (Varianten `Removed`, `Blocked`, `Reconfirm`, `IdentityChanged`, `WriteRevoked`, `ExecRevoked`, `ExportsNarrowed`, `RelationInactive`, `Unattributed`), `invalidate_restrictions_at` | `N/share/core/relation_rights.rs:70-94`; `N/share/core/node_restrictions.rs:73` |
| B15 | IPC `MutateExecGrant { token, target, enabled, settings }`; `settings` fehlt = Grant behält Konto und Liste; `ipc_protocol.rs` hat 475 Zeilen | `N/daemon/os/shared/ipc_protocol.rs:262-270`; `wc -l` |
| B16 | Android-Bridge setzt bei jeder Programmliste `run_as` auf Standard | `N/mobile/os/shared/domains/share_exec.rs:120-140` |
| B17 | Android: Kindprozesse tragen die App-UID; `setuid`-Familie per Seccomp gesperrt; Shizuku-Dienste und `su`-Shells laufen außerhalb des Prozessbaums der App; Shizuku 13.6.0 unter Android 17 gemeldet defekt (Issue #2180 offen, NR Revalidierung) | Ref Android F1.1, F2.1, F3.1, F3.6, F4.1 |
| B18 | Windows: `CreateProcessWithLogonW` nimmt laut Doku nur `STARTUPINFO` und ist aus LocalSystem nicht aufrufbar; `CreateProcessAsUserW` nimmt `STARTUPINFOEX`; S4U braucht `SeTcbPrivilege` (LocalSystem); S4U-Token ohne Netzanmeldedaten; S4U vom Typ `Network` liefert ein Impersonationstoken; lokale Administratorkonten erhalten bei Netzwerkanmeldung standardmäßig ein gefiltertes Token (`LocalAccountTokenFilterPolicy` = 0), Domänenkonten ein volles | Ref Win §2.2.2, §2.3; NR N1, N3 (KB 951016), Revalidierung 2.2 |
| B19 | Linux: `systemd-run --uid` mit polkit-Regel kann `User=` nicht einschränken; machined startet Shells als Einheit mit `User=`, `PAMName=login`, `WorkingDirectory=-~`; mit `PAMName=` und `pam_systemd` im Stapel wandern Unterprozesse in `session-<ID>.scope` und verlassen die Einheit | Ref Linux §2.2.5, §2.1.5; NR Revalidierung 2.2 Z-L1 (systemd.exec(5) v249/v259), N20 |
| B20 | Desktop-Benachrichtigung vorhanden (Linux freedesktop ohne Aktionen, Windows Ballon ohne Knöpfe); Android `ExecHostNotifier` wird durch `share`-Ereignisse geweckt; Android-Kanal `exec` hat `IMPORTANCE_LOW`; ohne `POST_NOTIFICATIONS` zeigt `Notifications.notify` still nichts | `N/notify_desktop/os/linux_os.rs:1-30`, `N/notify_desktop/os/windows.rs:1-12`; `A/service/ExecHostNotifier.kt:32-40`; NR N24 (`A/system/Notifications.kt:52`, `:71-81`) |
| B21 | Vorbild für einmalige Erhöhung im Repo: LAN-Uplink-Helfer (geplante Aufgabe, eine UAC-Abfrage, Prüfung jeder Anfrage) | `N/net/os/windows/uplink_helper.rs:1-8` |
| B22 | Dateigrößen nahe 500 Zeilen: `N/app/core/share_exec_ui.rs` 494, `N/cli/share/grants_exec.rs` 485, `N/daemon/os/shared/ipc_protocol.rs` 475, `N/share/os/linux_os/exec_supervisor.rs` 460, `N/share/os/linux_os/exec.rs` 459, `N/daemon/os/shared/exec_ipc.rs` 455, `N/daemon/os/shared/exec_grant_journal.rs` 454, `N/share/core/exec_registry.rs` 451, `N/share/os/windows/exec.rs` 446 | `wc -l` am Lesestand |
| B23 | Inventur `docs/oberflaechen/share-exec.md` hat zehn Abweichungen A1–A10 und undefinierte Kostenlabels C1–C3 | Lesung Oberflächen, „Abweichungen der Inventur“, Lücke 23 |
| B24 | Vergleichsprodukte: Zielkonto vom Verbindenden genannt und gegen Freigabe geprüft (Tailscale, Teleport, OpenSSH), mit OS-Zugangsdaten nachgewiesen (TeamViewer, RustDesk/Windows, Cockpit) oder vom Betreiber festgelegt (Webmin, RustDesk/Linux); Rückfrage am Zielgerät wählbar bei RustDesk, AnyDesk, TeamViewer, MeshCentral | Ref Modelle §3 |
| B25 | Die se-IPC läuft unter Linux und Windows über TCP auf Loopback mit einem Token in `daemon.token` im Datenordner des Worker-Kontos (Linux Modus `0600`) | NR N4, N9 (`N/daemon/os/linux_os/platform.rs:78`, `:301`; `ipc_storage.rs:13-15`; `N/daemon/os/windows/platform.rs:80`; `N/daemon/os/windows/ipc_storage.rs:8`) |
| B26 | Bestandsdefekt: Die transiente Einheit setzt `ExitType=main` ohne Bedingung; `ExitType` gibt es erst ab systemd v250; systemd v249 (Ubuntu 22.04) lehnt unbekannte Eigenschaften mit „Cannot set property %s, or unknown property.“ ab | `N/share/os/linux_os/exec_systemd.rs:54`; NR N7 (`systemd@v249:src/core/dbus-unit.c:2472`) |
| B27 | Alter Exec-Client (0.5.180–0.5.182) bleibt nach `START` ohne `STARTED` verbunden, solange der Host `Ping` mit `Pong` beantwortet (sonst Abbruch nach 20 s); ein alter `se shell` bricht nach 60 s ohne `Started` ab | NR N12 (`exec_client_active.rs:40-74`, `terminal_transports.rs:24`) |
| B28 | Android-App: `targetSdk` 36, `minSdk` 30; Benachrichtigungsaktionen über den nicht exportierten Receiver `ExecHostStopReceiver` | NR N21 (`android/app/build.gradle.kts:89-90`, `AndroidManifest.xml:116-118`) |

## C — Entscheidungen

Bewertungszeichen: „+“ erfüllt das Kriterium, „○“ teilweise oder mit Auflage, „−“ verfehlt es. Zusätzliche verbindliche Kriterien: Z1 bestehendes Verhalten bleibt (Protokoll, Strafkonto, Abbruch, Einhegung, Prüfkette, Grenzen), Z2 `core/` plattformneutral, Z3 Lizenz verträglich mit MIT, Z4 keine UI-Platzhalter.

### C1 — Rechtemodell

**Einheit, die Rechte erhält:**

| Lösung | K2 | K5 | K6 | Z1 | Bewertung |
|---|---|---|---|---|---|
| E-a Bestehende Einheit: genau eine gepinnte Geräteidentität (Direktfreigabe) und genau ein Raummitglied, dazu die Sammelaktion „für alle zugelassenen Mitglieder eines Raums übernehmen“, die je Mitglied ein eigenes Recht schreibt (Momentaufnahme) | + | ○ (Sammelaktion für Gruppen) | + (künftige Mitglieder erhalten nichts) | + | gewählt |
| E-b Person über mehrere Geräte | ○ | + | − (kein Vertrauensanker; Suche nach `owner_id`, `same_owner`, `own_devices`, `account_id` in `N/share/core/` ohne Treffer außer `lan_status.rs`) | − | verworfen |
| E-c Regeln Quelle → Zielkonto (Tailscale-Richtliniendatei) | + | − (Ref Modelle §2.1 B8: 8 Schritte für eine Richtlinienänderung) | ○ | ○ | verworfen |
| E-d Raum als Gruppe mit Rolle, wirkt auf künftige Mitglieder | + | + | − (neu zugelassenes Mitglied erhielte Befehle ohne Handlung des Host-Nutzers) | ○ | verworfen |

**Inhalt eines Zugriffsrechts (gewählt):**
- **Ausführen als** (gilt für `exec_commands` und `exec_terminal`): `own` „Mein Konto“ (Konto des Workers, bisher „Standardnutzer“), `remote` „Fernzugriffskonto `se-remote`“ (lokales Standardkonto, von se mit Administratorbestätigung angelegt), `account` „Konto …“ (vorhandenes lokales Konto mit Namen).
- **Fähigkeiten** mit Entscheidung `off` | `ask` | `allow`: `exec_commands` „Befehle“ (Startarten `argv`, `shell`), `exec_terminal` „Terminal“ (Startart `login` mit PTY). S4 ergänzt `screen_view`, `screen_control` (Abschnitt E).
- **Programmliste** (optional): engt `exec_commands` auf `argv`-Starts genau dieser Programme ein, Regeln unverändert (B2); mit Programmliste ist `exec_terminal` immer `off` (Validierung, wie heute).
- **Gültig bis** (optional, Unix-Sekunden, Uhr des Hosts): danach gelten alle Fähigkeiten des Rechts als `off`.
- Unverändert: `policy_revision` (Compare-and-Swap), `changed_at`, `source_request_id`, `source_decision_revision`.

**Voreinstellungen** (keine gespeicherten Vorlagenobjekte): „Volle Rechte als mein Konto“ (`own`, Befehle und Terminal `allow`), „Eingeschränktes Fernzugriffskonto“ (`remote`, beide `allow`), „Nur bestimmte Programme“ (`own`, Befehle `allow` mit Liste, Terminal `off`); „Mit Rückfrage“ setzt jede erlaubte Fähigkeit auf `ask`. Grund gegen gespeicherte Vorlagen: kein belegter Bedarf über die Raum-Sammelaktion hinaus. Die frühere Voreinstellung „Nur lesen“ entfällt (C8).

**Portierung bestehender Grants (ohne Bedeutungsänderung):**

| Gespeicherter Zustand | Ergebnis |
|---|---|
| `enabled:true`, `run_as` standard, keine `programs` | `own`, `exec_commands=allow`, `exec_terminal=allow` |
| aktiv, `programs` nicht leer (über `enabled` oder `restricted_enabled`) | Kontoprofil wie `run_as`, `exec_commands=allow` mit Liste, `exec_terminal=off` |
| aktiv, `run_as` account NAME, keine `programs` | `account NAME`, beide `allow` |
| `enabled:false` (ohne gültiges `restricted_enabled`) | beide `off`; `run_as` und `programs` bleiben gespeichert |
| Feld `access` vorhanden | `access` gilt; `programs` und Revisionsfelder aus den bisherigen Feldern |
| `access.version` größer als 1 | alle Fähigkeiten `off` (fail closed); das Objekt bleibt beim Speichern erhalten, bis der Host das Recht ändert |
| Bildschirmfähigkeiten in jedem alten Profil | `off` |

**Schreiben, Altleser-Schutz:** Ein Recht, das die Bedeutung des alten Formats exakt trifft (Kontoprofil `own` oder `account`, kein Ablauf, keine Entscheidung `ask`, Bildschirmfähigkeiten `off`, `(exec_commands, exec_terminal)` gleich `(allow, allow)` ohne Liste, `(allow, off)` mit Liste oder `(off, off)`), wird byte-gleich wie heute geschrieben. Jedes Recht außerhalb dieser Menge wird mit `enabled:false`, ohne `restricted_enabled`, mit `run_as` = `standard` (für `own`) bzw. `{"kind":"account","name":…}` (für `account`, und für `remote` mit Name `se-remote`) und dem Objekt `access` geschrieben. Leser 0.5.180 bis 0.5.182 sehen es ausgeschaltet (fail closed, wie B3).

**Kompatibilität mit Peers bis 0.5.182:** Rechte wirken nur beim Host. Alter Client an neuem Host: ignoriert `ExecHelloOk.access` (B13), erhält nie Rahmen `APPROVAL_PENDING`, bleibt während einer Rückfrage verbunden, weil der Host `Ping` beantwortet (C5, B27); ein alter `se shell` wartet höchstens 60 s. Neuer Client an altem Host: Anzeige „Konto: {user_label} (älterer Host, Rechte nicht gemeldet)“.

**Verworfen:** Rückfrage als Grant-weites Feld (S4 braucht `off|ask|allow` je Fähigkeit); neue `run_as`-Varianten (B4).

Quellen: Lesung Host Fragen 1–2; Ref Modelle §3; S4 E1.

### C2 — Kontowechsel Windows

Ausgangslage: Worker je Nutzer ohne Erhöhung (B7, B8); der Supervisor ist ein eigener Prozess, der über stdin/stdout mit dem Worker spricht und die ConPTY selbst öffnet (B10).

**Anmeldung und Prozessstart:**

| Lösung | K2/K3 | K5 | Sicherheit | Bewertung |
|---|---|---|---|---|
| W-a `CreateProcessWithLogonW` aus dem Worker mit gespeichertem Passwort | ○ (Microsoft-Konten, PIN unbelegt) | − (Passwort je Konto, bricht bei Passwortänderung) | ○ (Passwort im Nutzerprofil) | verworfen (nur `STARTUPINFO`, B18) |
| W-b Kontodienst unter LocalSystem, Start bei Bedarf, einmal per UAC eingerichtet; S4U-Anmeldung vom Typ `Network`, Umwandlung in ein Primärtoken mit `DuplicateTokenEx(…, SecurityImpersonation, TokenPrimary, …)`, Profil über `LoadUserProfileW` (mit aktivierten `SeBackupPrivilege`/`SeRestorePrivilege`), Umgebung über `CreateEnvironmentBlock`, Start des Supervisors (Relais) mit `CreateProcessAsUserW` | + (Muster Win32-OpenSSH, Tailscale; NR N1, N2) | + (einmal Administratorbestätigung, danach Auswahl aus einer Liste) | ○ (Aufruferprüfung und Kontenliste unten) | gewählt |
| W-c `ConptyCreatePseudoConsoleAsUser` | ○ | ○ | ○ | verworfen (gleiche Privilegien wie W-b, undokumentiert, zusätzliche DLL) |
| W-d Aufgabenplanung je Sitzung | ○ | ○ | ○ (Anfrage über Datei) | verworfen (Latenz je Befehl, keine Pseudokonsolen-Aussage) |
| W-e PsExec, `runas /savecred`, WinRM/`winrs` | ○ | − | − | verworfen |
| W-f Eigenes Standardkonto `se-remote` (`NetUserAdd`, `USER_PRIV_USER`, Gruppe Users, zufälliges Passwort, das niemand kennt) | + | + | + | gewählt als Teil von W-b; auf Windows Home unbelegt (A6) |

**Einhegung (NR N6 widerlegt „Worker ordnet das Relais seinem Job zu“: alle Prozesse eines Jobs müssen in der Sitzung des Jobs laufen; das S4U-Relais läuft in Sitzung 0):**

| Lösung | Z1 Einhegung | Sicherheit | Aufwand/Risiko | Bewertung |
|---|---|---|---|---|
| WE-1 Token-Sitzung vor dem Start auf die Sitzung des Workers setzen (`SetTokenInformation(TokenSessionId)` mit `SeTcbPrivilege`), Worker-Job wie heute | + | ○ (Prozesse des Zielkontos in der Bildschirmsitzung des Nutzers) | − (Handle-Vererbung über Sitzungsgrenzen verboten: „You cannot inherit handles across sessions“, Ref Win §2.1.3; Zugriff des Zielkontos auf Fensterstation/Desktop der Nutzersitzung unbelegt, Ref Win L05) | verworfen |
| WE-2 Job im Kontodienst (Sitzung 0, Kill-on-close, Relais über `PROC_THREAD_ATTRIBUTE_JOB_LIST` zugeordnet); Abbruch und Leerprüfung über das Kontodienst-Protokoll (`stop`, `status` mit `JobObjectBasicAccountingInformation.ActiveProcesses`); Sitzungen sind an die Pipe-Verbindung des Workers gebunden: schließt sie, beendet der Dienst alle Jobs dieser Verbindung | + (Job und Relais in derselben Sitzung 0; NR N6 Auswege) | + | + (gleiches Muster wie der Linux-Kontodienst, C3) | gewählt |
| WE-3 Relais legt selbst einen Job für seine Kinder an | ○ (Relais selbst ungehegt; Absturz des Relais lässt Kinder laufen, wenn der Job nicht beim Dienst liegt) | ○ | ○ | verworfen |

**Verbindung Relais ↔ Worker (gewählt):** Der Worker legt je Sitzung zwei benannte Pipes `\\.\pipe\SmartExplorerExec-<128 Bit Zufall>-in` (Worker schreibt) und `…-out` (Worker liest) an, je eine Richtung wie die heutigen zwei anonymen Pipes (B10), mit `FILE_FLAG_FIRST_PIPE_INSTANCE`, höchstens einer Instanz und einer DACL, die nur `SYSTEM` Zugriff gibt. Der Dienst öffnet beide als Client und übergibt sie dem Relais über `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` und `STARTF_USESTDHANDLES` als stdin/stdout (Dienst und Relais in Sitzung 0, Vererbung zulässig). Der Supervisor bleibt dadurch unverändert. Grund gegen `DuplicateHandle` in den Worker: der Dienst bräuchte einen Prozess-Handle des Workers, den er nur über die fälschbare Client-PID fände (NR N5).

**Aufruferprüfung (NR N5 widerlegt `GetNamedPipeClientProcessId` als Sicherheitsprüfung):** Pipe `\\.\pipe\SmartExplorerExecAccounts` mit `PIPE_REJECT_REMOTE_CLIENTS` und DACL `SYSTEM` + eingetragene Nutzer-SIDs; nach dem ersten gelesenen Rahmen `ImpersonateNamedPipeClient` → `OpenThreadToken` → Nutzer-SID → `RevertToSelf`; die SID muss in `%ProgramData%\Smart Explorer\exec-accounts.json` stehen, das Zielkonto für diese SID freigegeben sein.

**Kontenliste aus Anmeldeprobe (NR N1b, N3, N17, N19 nicht belegbar bzw. widerlegt):** Der Dienst bietet `probe{account}`: S4U-Anmeldung ohne Prozessstart, Ergebnis `ok`, Fehlertext, `elevated` (Token-Erhöhungsart) und `admin_group` (`BUILTIN\Administrators` aktiviert oder nur „deny“). Die Kontoauswahl zeigt nur Konten mit erfolgreicher Probe; gescheiterte Konten erscheinen mit „nicht verfügbar: Windows lehnt die Anmeldung ohne Passwort ab ({Fehler})“. Die Warnung „REMOTE ADMINISTRATOR SHELL“ erscheint genau bei `elevated = true` (Domänenadministratoren, oder lokale Administratoren bei `LocalAccountTokenFilterPolicy` = 1); bei lokalem Administrator mit gefiltertem Token lautet der Hinweis „Konto {Name} ist Administrator; über den Kontodienst läuft es ohne Administratorrechte (Windows-Standard für Netzwerkanmeldungen lokaler Konten).“ Damit hängt keine Anzeige an einer unbelegten Annahme.

**Weiteres gewählt:**
- Dienstname `SmartExplorerExecAccounts`, `SERVICE_DEMAND_START`, LocalSystem; Dienst-DACL: `SERVICE_START`, `SERVICE_QUERY_STATUS` für eingetragene Nutzer-SIDs (NR N5 bestätigt); Leerlaufende nach 10 Minuten ohne Sitzung und ohne Anfrage (Grund: Befehlsfolgen von KI-Agenten ohne Neustart je Befehl; ohne Nutzung kein Speicher).
- Programmdatei: Kopie `%ProgramFiles%\Smart Explorer\exec-accounts\se.exe` (nur Administratoren/SYSTEM schreiben), weil die Installation nutzerbeschreibbar ist (B8); Aktualisierung nur über „Kontodienst aktualisieren“ (UAC); Protokollversion im `hello`.
- `UnloadUserProfile` nach Ende des Relais; scheitert es wegen offener Registrierungshandles (NR Revalidierung 2.2), versucht der Dienst es beim nächsten Leerlauf erneut und protokolliert den Fehler.
- Hinweis in der Kontoauswahl: „In dieser Sitzung sind Netzlaufwerke und mit EFS verschlüsselte Dateien nicht erreichbar (Anmeldung ohne Passwort). Programme mit Fenster erscheinen nicht auf dem Bildschirm.“ (S4U ohne Anmeldedaten, Sitzung 0).
- Konto `se-remote` wird nicht auf dem Anmeldebildschirm ausgeblendet (NR N18: `SpecialAccounts\UserList` undokumentiert und als Verschleierungstechnik gemeldet).

Quellen: Ref Win §2.1–§2.7; NR N1–N6, N17–N19; B10, B18.

### C3 — Kontowechsel Linux

**Weg zum Konto:**

| Lösung | K2/K3 | Sicherheit | Z1 | Bewertung |
|---|---|---|---|---|
| L-a Transiente Einheit mit `User=`, PAM-Sitzung, `WorkingDirectory=-~`, Supervisor-Verbindung als Deskriptor (`StandardInputFileDescriptor`, `StandardOutputFileDescriptor`, Typ `h`, NR N7), root-eigene Programmdatei | + | + | siehe Einhegung | gewählt (ersetzt den setuid-Weg `N/share/os/linux_os/exec_supervisor_account.rs`) |
| L-a' PAM über libpam im Supervisor | + | ○ (`pam_limits` verlangt einthreadig) | + | verworfen |
| L-b Kontodienst für den Nutzer-Worker: root-Socketdienst `smart-explorer-exec-helper.socket` + `smart-explorer-exec-helper@.service` (`Accept=yes`), Prüfung `SO_PEERCRED` (UID, vom Kernel beim Verbinden gesetzt) und Kontenliste, startet die Einheit aus L-a im Systemmanager, Deskriptor per `SCM_RIGHTS`, `stop`/`status`/`probe`; Sitzungen an die Verbindung gebunden | + | + | + | gewählt |
| L-c sudo-Drop-in, L-d `pkexec`-Regel, L-e `systemd-run --uid` mit polkit, L-f `machinectl shell`, L-g SSH auf localhost, L-h `su` mit Passwort | ○ | − bis ○ | − bis ○ | verworfen (Gründe unverändert: sudo-rs/CVE/kein Deskriptor; keine Argumentprüfung; `User=` nicht einschränkbar; nur PTY ohne Exit-Code; sshd und Schlüssel; Passwortablage) |

**Einhegung unter PAM (NR Revalidierung Z-L1 widerlegt „Einhegung über die Einheit“ bei `PAMName=login`, weil `pam_systemd` Unterprozesse in `session-<ID>.scope` verschiebt):**

| Lösung | Z1 Einhegung | PAM-Wirkung | Bewertung |
|---|---|---|---|
| LC-a Eigener PAM-Dienst `smart-explorer-exec` ohne `pam_systemd`: Debian/Ubuntu `@include common-account`, `@include common-session-noninteractive`, dazu `session required pam_loginuid.so`, `session required pam_limits.so`, `session optional pam_keyinit.so force revoke`; Fedora/RHEL `account include system-auth`, Sitzung explizit `pam_loginuid`, `pam_keyinit force revoke`, `pam_limits`, `pam_unix`; Distributionen, deren `/etc/os-release` in `ID` und `ID_LIKE` weder `debian`, `ubuntu`, `fedora` noch `rhel` nennt, explizit `account required pam_unix.so`, Sitzung `pam_loginuid`, `pam_keyinit`, `pam_limits`, `pam_unix` (nur lokale Konten) | + (keine logind-Sitzung, Prozesse bleiben in der Einheit; `KillMode=control-group` und `cgroup_populated` gelten wie heute, B9) | + (Kontoprüfung mit Ablauf und Verzeichnisdiensten aus `common-account`/`system-auth`, Grenzen, `loginuid`); kein `update-motd.d` je Start (NR N8 Punkt 2) | gewählt |
| LC-b `PAMName=login` und Einhegung über den Sitzungs-Scope (Scope ermitteln, beim Abbruch beenden, leer prüfen) | ○ (zwei cgroups, Rennen bis zur Scope-Kenntnis, `KillUserProcesses=` bestimmt das Verhalten beim Abmelden) | ○ (`update-motd.d` je Start auf Debian/Ubuntu) | verworfen |
| LC-c Kein PAM (setuid wie heute) | + | − (keine Kontoprüfung, keine Grenzen) | verworfen (E2 verlangt Durchsetzung durch das OS) |
| LC-d `pam_systemd` mit Klasse `none` über `XDG_SESSION_CLASS` | + | + | verworfen (Klasse `none` erst ab systemd v258; Ubuntu 22.04/24.04 und Debian 13 haben 249/255/257) |

Annahme A1: `common-session-noninteractive` enthält auf Debian/Ubuntu kein `pam_systemd` (Ref Linux §2.1.2: `pam_systemd` trägt „Session-Interactive-Only: yes“). Wirkung bei Fehlschlag: Unterprozesse verließen die Einheit. Abnahmesignal `perm_task_linux_account_unit_contains_children` (auf dem Ubuntu-Runner: Kind `sleep` im Terminal liegt in der cgroup der Einheit; nach Abbruch ist die Einheit leer und kein Prozess von `se-remote` übrig).

**Weitere Folgen der PAM-Wahl:** Ein Konto mit erzwungenem Passwortwechsel (`sp_lstchg == 0`) scheitert (`PAM_NEW_AUTHTOK_REQD`, NR N20); Text: „Konto {Name} verlangt einen Passwortwechsel; vorher ist kein Fernzugriff möglich.“ Abgelaufene Konten scheitern (`PAM_ACCT_EXPIRED`). `/etc/nologin` wird nicht geprüft (kein `pam_nologin` im eigenen Dienst). Ohne `pam_systemd` gibt es für Fernsitzungen keinen Benutzerdienstmanager und kein `XDG_RUNTIME_DIR` (Hinweis in GOTCHAS: `systemctl --user` im Fernterminal funktioniert nicht).

**Bestandsdefekt `ExitType=main` (B26, NR N7):** wird in PRM-M5 behoben: die Eigenschaft entfällt ganz. Grund: `ExitType=` hat ab v250 den Standardwert `main`, also dieselbe Wirkung wie das heutige Setzen; v249 kennt die Eigenschaft nicht und lehnt die Einheit ab (Annahme A2: Standardwert `main` ab v250 laut systemd.service(5); Signal `perm_task_systemd_unit_props_v249_compatible` prüft, dass jede gesendete Eigenschaft in v249 bekannt ist, und `perm_task_exec_linux_ubuntu2204_starts` startet einen Job auf einem Ubuntu-22.04-Runner, Annahme A10).

**Weiteres gewählt:**
- Programmdatei der Konto-Einheit und des Kontodienstes: die se-Datei, wenn sie und jedes Verzeichnis ihres Pfads root gehören und nicht für Gruppe/Andere beschreibbar sind; sonst die root-eigene Kopie `/usr/local/sbin/smart-explorer-exec` (SHA-256 geprüft; Typ `bin_t` unter Fedora-SELinux, NR N15; `/usr/local/lib/` wäre `lib_t`).
- PAM-Dienstdatei `/etc/pam.d/smart-explorer-exec`: der root-Worker legt sie bei der ersten Wahl eines Kontoprofils `remote` oder `account` an (Dialogtext nennt das); im Nutzer-Worker-Weg die Einrichtung des Kontodienstes.
- Kontodienst-Konfiguration `/etc/smart-explorer/exec-accounts.toml` (root, `0644`); Socket `/run/smart-explorer/exec-helper.sock` (`SocketMode=0666`, Zulassung nur über `SO_PEERCRED`); Dienst-Härtung `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `NoNewPrivileges=yes`, `RestrictAddressFamilies=AF_UNIX`, `CapabilityBoundingSet=` leer (Annahme A3, NR N16; Signal `perm_task_linux_helper_hardened_unit_starts`).
- Fernzugriffskonto: `useradd --system --create-home --home-dir /var/lib/smart-explorer/se-remote --shell /bin/bash se-remote` (ohne `/bin/bash` mit `/bin/sh`); Passwort gesperrt, keine Alterungsdaten (besteht `pam_acct_mgmt`, NR N20).
- Einrichtung aus der GUI über `pkexec`; fehlt `pkexec` (Debian 13 nicht gesichert, NR N14) oder scheitert es ohne Agenten, zeigt die GUI den Befehl `sudo se share exec accounts setup …` zum Kopieren.
- Ohne systemd oder ohne cgroup v2: Kontowechsel und Kontodienst nicht verfügbar, Grund wird angezeigt.

Quellen: Ref Linux §2.1–§2.4, §3.1–§3.2; NR N7, N8, N14–N16, N20, Revalidierung 2.2.

### C4 — Android

| Lösung | K3/K4 | Z1 (Einhegung) | Bewertung |
|---|---|---|---|
| A-1 App-Konto (immer) | + für K4 | + | gewählt |
| A-2 Shell-UID über Shizuku | ○ | − (Dienstprozess außerhalb des Prozessbaums; `ContainmentNotConfirmed`) | verworfen (zusätzlich Android-17-Störung, kein Release seit 2025-05-25) |
| A-3 root über `su` | ○ | − (Shells unter `magiskd`) | verworfen |
| A-4 zweites Android-Profil | + (eigene UID, eigenes Gerät) | + | gewählt als Hinweis; kein eigener Code |
| A-5 Termux `RUN_COMMAND` | ○ | − | verworfen |

**Anzeige auf Android (wörtlicher Text, Abschnitt „Ausführen als“ im Zugriffsdialog):** „App-Konto von Smart Explorer. Befehle anderer Geräte können die Daten der App lesen (gespeicherte Verbindungspasswörter, Share-Identität, Google-Drive-Tokens) und alle Dateien, auf die die App zugreifen darf. Ein Konto außer dem App-Konto ist auf Android nicht möglich: Eine App kann keinen Prozess unter einer fremden Nutzerkennung starten. Root (über su) und das Shell-Konto (über Shizuku) laufen außerhalb der App, sodass ein abgebrochener Befehl nicht sicher beendet werden kann. Ein zweites Android-Profil (Arbeitsprofil oder privater Bereich) ist ein eigenes Gerät: dort Smart Explorer installieren und koppeln.“

Quellen: Ref Android F1–F6; NR N11, N21, N24.

### C5 — Rückfrage, Zeitbegrenzung, Widerruf, Benachrichtigung, Protokoll

| Lösung | K6 | K5 | Bewertung |
|---|---|---|---|
| R-a Entscheidung je Fähigkeit `ask`: jeder Exec-Start bzw. jede S4-Sitzung fragt am Host „Diesmal erlauben / Immer erlauben / Ablehnen“ | + | + | gewählt |
| R-b Rückfrage nur beim ersten Start je Zeitraum (Tailscale `check`) | ○ (prüft den Verbindenden) | + | verworfen |
| R-c Frist für die Rückfrage | ○ | ○ | verworfen: keine Frist (wie S4 C7); Ende mit Entscheidung, Abbruch des Clients oder Verbindungsende |

**Gewählt im Einzelnen:**
- Rückfragedienst `access_approval` im Kern. Jede offene Rückfrage (Exec und Bildschirm, S4 E1 Punkt 6) belegt eine Reservierung der Exec-Registry; die bestehenden Grenzen (2 je Gerät, 8 gesamt; `N/share/core/exec_registry.rs:96-97`) gelten damit für alle Rückfragen zusammen. Nach der Entscheidung gibt der Dienst eine Bildschirm-Reservierung frei (die Bildschirmsitzung zählt in der S4-Registry).
- Während eine Rückfrage offen ist, beantwortet der Host `Ping` mit `Pong` (B27); ein alter Client bleibt dadurch verbunden. Fehlt dem Client das Merkmal `exec_approval_v1` und ist der Start ein Terminal, zeigt die Rückfrage am Host zusätzlich „Älteres Smart Explorer auf {Gerät}: das Terminal wartet dort höchstens 60 Sekunden.“
- „Diesmal erlauben“ startet genau diesen Start; „Immer erlauben“ setzt die Fähigkeit dieses Rechts von `ask` auf `allow` (Erweiterung) und startet; „Ablehnen“ → `approval_denied`.
- Anzeige: Gerät, Beziehung (direkt oder Raumname), Fingerabdruck (gekürzt), Fähigkeit, Konto, für Befehle Programm und Argumente bzw. Shell-Befehl (höchstens 256 Zeichen, nur angezeigt, nie protokolliert; `N/share/os/shared/exec_audit.rs:7-8`).
- Desktop (Windows, Linux): Der Daemon startet `smart_explorer --access-consent <ID>` in der Sitzung des Workers und zeigt eine Desktop-Benachrichtigung; das Fenster kommt nicht zuverlässig nach vorn (NR N22, N23: Windows blinkt in der Taskleiste, unter Wayland entscheidet der Kompositor). Ohne grafische Sitzung (root-Worker, Server) nur `se share exec approvals` und Desktop-Tab **Befehle**.
- Android: neuer Benachrichtigungskanal `access_approval` mit `IMPORTANCE_HIGH` (Kanal `exec` hat `IMPORTANCE_LOW`, B20). Ist die App sichtbar, zeigt sie die Rückfrage als Dialog in der App; sonst als Benachrichtigung. Exec-Rückfragen: drei Aktionen als Broadcast an einen nicht exportierten Receiver (Muster `ExecHostStopReceiver`, B28). Bildschirm-Rückfragen (S4 E1 Punkt 7): „Diesmal erlauben“ und „Immer erlauben“ sind `PendingIntent.getActivity` auf `A/screen/ScreenConsentActivity.kt`, erzeugt mit `ActivityOptions.setPendingIntentCreatorBackgroundActivityStartMode(MODE_BACKGROUND_ACTIVITY_START_ALLOWED)`; „Ablehnen“ bleibt Broadcast. Meldet `Notifications.notify` `false` (kein `POST_NOTIFICATIONS`) und ist die App nicht sichtbar, ruft die App `share.reportApprovalUndeliverable {approvalId}`; der Host lehnt dann mit `approval_unavailable` ab („{Gerät} kann die Rückfrage nicht anzeigen: Benachrichtigungen für Smart Explorer sind aus.“). Steht ein Recht auf `ask` und sind Benachrichtigungen aus, zeigt der Abschnitt „Befehle auf diesem Telefon“ eine Warnzeile mit „Benachrichtigungen erlauben“.
- Rahmen `APPROVAL_PENDING` trägt `surface`: `window` (Desktop-Fenster gestartet), `notification` (Android oder Desktop ohne Fenster), `cli_only` (keine grafische Sitzung); der Client zeigt „Warte auf Bestätigung am Gerät {Gerät} …“ bzw. bei `cli_only` „… (am Gerät nur mit `se share exec approvals` zu bestätigen)“.
- Gültig bis: bei jeder Prüfung ausgewertet; Zeitgeber erzeugt beim Ablauf `RestrictionReason::GrantExpired`, laufende Sitzungen enden (`Revoked`, Protokollereignis `grant_expired`); keine Obergrenze.
- Widerruf: jede Verengung (Rang `off` < `ask` < `allow` sinkt, Kontoprofil oder Programmliste ändert sich, Ablauf früher oder neu, Entzug) beendet die Sitzungen der betroffenen Fähigkeit dieses Rechts; eine reine Erweiterung lässt sie laufen und bindet sie an die neue Revision. Bisher endete jede Änderung von Konto oder Liste die Sitzungen des Geräts (Texte Lesung Oberflächen 4.1, 4.4; `stale_job` mit `Revoked`, `N/share/core/exec_registry.rs:409-418`); neue Regel nötig, damit „Immer erlauben“ die eben erlaubte Sitzung nicht beendet. Texte `share_exec_ui.rs:259`, `share_exec_programs_ui.rs:163`, `ExecProgramsDialog.kt:137` werden „Laufende Befehle dieses Geräts enden, wenn die Änderung Rechte einschränkt.“
- Protokoll: Dateien und Rotation unverändert (`exec-audit.log`, `exec-audit-rejected.log`); Einträge erhalten `kind` (`exec`, S4 `screen`), `account_profile`, `capability`, `approval` (`not_required`, `once`, `always`, `denied`, `cancelled`, `unavailable`); neue Ereignisse `approval_requested`, `approval_granted`, `approval_denied`, `approval_cancelled`, `approval_unavailable`, `grant_expired`, `helper_refused`. Strafkonto unverändert (Ablehnung, Ablauf, `right_not_granted` zählen nicht als Fehlanmeldung).
- Kontodienst-Protokoll: Linux über stderr ins Journal; Windows `%ProgramData%\Smart Explorer\exec-accounts.log` (nur SYSTEM und Administratoren).

Quellen: Ref Modelle §2.5–§2.8; S4 C7, E1; NR N12, N21–N24.

### C6 — Bedienwege

**Kostenklassen** (Eintrag in `docs/oberflaechen/INDEX.md` in PRM-M11; B23):

| Klasse | Bedeutung |
|---|---|
| C0 | keine Handlung (Anzeige oder Standard ohne Zutun) |
| C1 | genau eine Handlung (ein Befehl, ein Klick, ein Tipp) an der Stelle, an der der Nutzer schon ist |
| C2 | zwei oder drei Handlungen in einem Dialog oder auf einer Seite; Texteingabe nur als Auswahl aus einer Liste |
| C3 | Navigation zu einer anderen Seite plus Handlungen, zusammen vier bis sechs; oder Freitext je Eintrag (Programm- oder Kontoname) |
| C4 | Handlung oder Wissen außerhalb von se (Konto im Betriebssystem anlegen, Systemdatei bearbeiten, Administratorbestätigung) oder mehr als sechs Handlungen oder eine Eingabe je erlaubtem Programm |

| Lösung | K5 | K6 | Bewertung |
|---|---|---|---|
| B-a Ein Zugriffsdialog je Gerät mit drei Voreinstellungen, Rückfrage-Schalter, Zeitraum, aufklappbarer Programmliste; Bestätigung durch einen Knopf, dessen Beschriftung die Folge nennt („Als se-remote erlauben“) | + (eine Handlung weniger als heute) | + | gewählt |
| B-b Weiter Kästchen „Ich bestätige …“ plus Knopf | ○ | + | verworfen |
| B-c Einrichtung über Kommandozeile/Systemdateien | − | + | verworfen |

Die CLI behält `Type YES` bzw. `--yes`.

**Text-Korrekturen und neue Fehltexte** (CLI englisch, GUI und Android deutsch):

| Stelle | Bisher | Neu |
|---|---|---|
| `N/cli/share/grants.rs:44` | „Inspect, enable, or disable unrestricted per-device Exec grants“ | „Inspect and change what each device may do here: commands, terminal, the account they run as“ |
| `N/cli/exec.rs:7-8` | „Run a command as the Smart Explorer user's unrestricted shell authority on one explicitly authorized Share peer. …“ | „Run a command on a Share peer that allowed this device commands; it runs as the account and with the rights that peer chose. File access does not imply Exec access.“ |
| `N/cli/share/grants_exec.rs:50-52` | „Confirm the FULL USER CODE EXECUTION warning without a prompt“ | „Confirm the access warning without a prompt“ |
| `N/app/core/share_exec_ui.rs:334` | „Ich bestaetige diese uneingeschraenkte Codeausfuehrung bewusst“ | entfällt; Knopf „Als {Konto} erlauben“ |
| `N/app/core/share_exec_ui.rs:173` | „AKTIVIERT — volle Remote-Codeausfuehrung erlaubt“ | „Befehle: erlaubt · Terminal: erlaubt · als {Konto}“ |
| `N/app/core/share_exec_ui.rs:308` | „… ohne Befehlsfilter beliebige Programme und Shell-Befehle mit den Rechten des gewaehlten Kontos …“ | „{Gerät} kann danach Programme, Shell-Befehle und ein Terminal mit den Rechten von {Konto} starten. Was {Konto} im Betriebssystem nicht darf, darf auch das Gerät nicht.“ |
| Client-Text bei `permission_denied` (Produktionsstellen `N/share/core/exec_auth.rs:253`, `N/share/core/exec_admission.rs:81`) | „exec authentication failed“ | „{Gerät} erlaubt diesem Gerät keine Befehle (nicht erlaubt, abgelaufen oder entzogen). Nächster Schritt: den Besitzer von {Gerät} um Freigabe bitten.“ (Grund bleibt unbestimmt, damit kein Fremder erfährt, ob ein Recht existiert) |
| neu `right_not_granted` | – | „{Gerät} erlaubt diesem Gerät kein Terminal (nur Befehle).“ bzw. „… keine Befehle (nur ein Terminal).“ + „Nächster Schritt: den Besitzer bitten, „Terminal“ zu erlauben.“ |
| neu `approval_denied` | – | „{Gerät}: Zugriff am Gerät abgelehnt.“ |
| neu `approval_unavailable` | – | „{Gerät} kann die Rückfrage nicht anzeigen: Benachrichtigungen für Smart Explorer sind aus.“ |
| neu `account_not_allowed` | – | „Konto {Name} ist auf {Gerät} nicht für Fernzugriff freigegeben. Nächster Schritt am Gerät: Geräte & Freigaben → Befehle → Konten → {Name} freigeben (Administratorbestätigung) oder `se share exec accounts allow {Name}`.“ |
| neu `account_helper_unavailable` | – | „Auf {Gerät} ist der Kontodienst nicht eingerichtet oder veraltet ({Detail}); Befehle als {Name} brauchen ihn. Nächster Schritt am Gerät: Geräte & Freigaben → Befehle → Konten → Kontodienst einrichten bzw. aktualisieren.“ |
| `account_unavailable` (bestehend), neue Gründe | – | „Konto {Name} verlangt einen Passwortwechsel; vorher ist kein Fernzugriff möglich.“; „Konto {Name} ist abgelaufen oder gesperrt.“; „Windows lehnt die Anmeldung ohne Passwort für {Name} ab ({Fehler}).“ |
| Client-Statuszeile bei Rückfrage | – | „Warte auf Bestätigung am Gerät {Gerät} … (Strg+C bricht ab)“; bei `surface = cli_only` Zusatz „(am Gerät nur mit `se share exec approvals` zu bestätigen)“ |
| Client-Kopfzeile (`se shell`, `se exec --verbose`, MCP-Jobstatus) | – | „{Gerät}: als {Konto}{, mit Rückfrage}“ |

### C7 — Netzvertrag und Oberflächen

| Lösung | K7 | Kompatibilität | Bewertung |
|---|---|---|---|
| N-a Rechteansicht `access` in `ExecHelloOk`, `ExecClientHello.client_features` außerhalb des Transkripts (B12), Rahmen `APPROVAL_PENDING` nur für Clients mit `exec_approval_v1`; Host-Merkmale `exec_access_v1`, `exec_approval_v1` | + | + | gewählt |
| N-b Protokollversion 3 mit neuem ALPN | + | − | verworfen |
| N-c Rechte vorab über den Share-Status | ○ | ○ | verworfen (Auskunft an Fremde) |
| I-a IPC: eine neue Variante `Access { token, request: AccessIpcRequest }` mit Antwort `Access(AccessIpcReply)`; Untertypen in `ipc_protocol_access.rs`; `MutateExecGrant` bleibt und wird abgebildet | + | + (alter Daemon lehnt unbekannte Variante ab) | gewählt; Grund für eine Sammelvariante: `ipc_protocol.rs` hat 475 Zeilen (B22), S4 fügt eigene Varianten hinzu (S4 SCR-M2) |
| I-b Neue Felder in `MutateExecGrant` | + | − (Daemon 0.5.182 ignorierte sie still) | verworfen |

**MCP:** alle neuen CLI-Befehle über `se`. Ausnahmen mit Grund: `se share exec approve|deny` und `se share exec accounts setup|allow|disallow|update|remove` lehnt das MCP-Werkzeug `se` ab (Sperrliste in `N/cli/mcp/cli_tool.rs`), Text „Diese Entscheidung trifft nur ein Mensch am Gerät.“ Grund: Die Rückfrage holt im Moment des Zugriffs eine menschliche Entscheidung ein; die Einrichtung legt Menschen eine Administratorabfrage vor. Lesende Befehle bleiben erreichbar; Rechteänderungen wie heute mit der Katalogregel „only when the user explicitly asks“ (`N/cli/mcp/catalog.rs:36-38`).

### C8 — „Nur lesen“ (neu entschieden nach NR N4, N9)

Befund: Beide gewählten Mechanismen tragen nicht. Im eigenen Konto kann eine eingeschränkte Shell das IPC-Token des Workers lesen und den Worker über TCP-Loopback steuern (B25); Linux-Landlock lässt zusätzlich Abkürzen von Dateien bis ABI 2 und Verbindungen zu Pfad-UNIX-Sockets bis ABI 8 zu (Sitzungsbus, Benutzerdienstmanager; Ubuntu 24.04/26.04, Debian 13 betroffen); Windows-Integritätsstufe Low erlaubt Sockets und RPC, der Start ohne `SeIncreaseQuotaPrivilege` und das Verhalten von `powershell.exe`/`cmd.exe` auf Low sind nicht belegbar.

| Lösung | Sicherheit | Plattform | Bewertung |
|---|---|---|---|
| RO-1 Landlock/Low wie geplant, Bezeichnung „Nur lesen (Dateien)“ | − (Umgehung über se-IPC im eigenen Konto) | Linux, Windows | verworfen |
| RO-2 Linux: Landlock ab ABI 6 + seccomp-Verbot von `socket()` für alle Adressfamilien; Windows: Low + Label „No-Read-Up“ auf `daemon.token` | ○ (offen: `chmod`/`chown`/`utime`/`setxattr` nie einschränkbar; `io_uring`-Wege, Ptrace-Grenze von Landlock und RPC-Wege unter Low nicht recherchiert) | Linux nur Debian 13, Ubuntu 24.04 HWE, Ubuntu 26.04, Fedora 44; Windows unbelegt | verworfen für diesen Stapel: eine Sicherheitszusage mit vier unrecherchierten Umgehungsklassen widerspräche Z4 und K6 |
| RO-3 „Nur lesen“ nur zusammen mit `remote`/`account` (Token des Workers dort unlesbar) | ○ (Grenze ist das Konto; „Nur lesen“ wäre nur Zusatzschutz innerhalb des Kontos, mit den Umgehungen aus RO-1/RO-2) | wie RO-2 | verworfen: geringer Nutzen, weil `se-remote` ohnehin nur seine eigenen Dateien schreiben darf |
| RO-4 Einschränkung über das OS-Konto (`se-remote` oder ein vom Host-Nutzer angelegtes Konto mit nur Leserechten auf gewünschte Ordner) | + (OS setzt durch, E2) | Linux, Windows | gewählt als tragfähiger Weg zum Ziel „eingeschränkt“ |

Wahl: „Nur lesen“ als Sandbox-Schalter entfällt in diesem Stapel auf allen Plattformen (keine Zeile, kein Speicherfeld). Eingeschränkter Zugriff läuft über das Kontoprofil `remote` bzw. `account` (RO-4). Ein `docs/TODO.md`-Eintrag nennt die belegten Bedingungen für einen späteren Stapel: Linux ABI ≥ 6 plus seccomp-Socketverbot (RO-2), Windows „No-Read-Up“ auf dem Token und Recherche der RPC-Wege, Android 17 mit GKI ab `android16-6.12` (NR N11). Abnahmesignal für die tragende Annahme von RO-4: `perm_task_remote_account_cannot_read_ipc_token_linux` und `perm_task_remote_account_cannot_read_ipc_token_windows` (Shell als `se-remote` liest `daemon.token` des Worker-Kontos nicht; Annahme A4).

## D — Meilensteine (Umsetzungsreihenfolge)

### D.1 Meilensteine PRM-M1 bis PRM-M11

Zeilenzahlen neuer Dateien sind Obergrenzen der Planung (jede neue Rust-Datei unter 500 Zeilen und 50 KiB). Dateien nahe 500 Zeilen (B22) erhalten keinen nennenswerten neuen Code. Alle Tests gehören zur einen Remote-Task-Suite des Stapels (Präfix `perm_task_`); nichts davon wird lokal ausgeführt.

#### PRM-M1 — Zugriffsrecht im Kern, Speicherform, Portierung, Relationsbeweis

- **Ergebnis:** Jedes Gerät und jedes Raummitglied hat ein Zugriffsrecht mit „Ausführen als“, „Befehle“ und „Terminal“ (je Aus/Fragen/Erlauben), Programmliste und „Gültig bis“. Bestehende Freigaben wirken unverändert; ein auf 0.5.182 zurückgestuftes Gerät sieht neue Rechte als ausgeschaltet.
- **Neue Dateien (`core/`):** `N/share/core/access_grant.rs` (≤ 400: `Capability`, `CapabilityDecision`, `CapabilityState`, `AccountProfile`, `AccessGrant`, `AccessGrantChange`, `Expiry`, Validierung, Rangvergleich `narrowing_against`), `access_grant_wire.rs` (≤ 350: Speicherform, Portierung, Altleser-Schutz, Erhaltung unbekannter Fähigkeitsschlüssel und unbekannter `access.version`), `access_authorize.rs` (≤ 300: `capability_for`, Startart → Fähigkeit, Ablauf), `access_catalog.rs` (≤ 200: `capability_catalog(&HostFacts)`, Einträge Befehle/Terminal, Aufruf `register_screen_entries` aus S4), `relation_proof.rs` (≤ 300: `prove_relation`, `ProvenPrincipal`, aus `exec_auth.rs:91-160` und `:207-229`), `access_grant_runtime.rs` (≤ 300: Verengung/Erweiterung, Ablaufzeitgeber), Tests `access_grant_tests.rs`, `access_grant_wire_tests.rs`, `relation_proof_tests.rs`; `N/daemon/os/shared/access_grant_journal.rs` (≤ 200).
- **Geänderte Dateien:** `N/share/core/exec_policy.rs`, `exec_policy_wire.rs`, `exec_auth.rs`, `exec_admission.rs` (Fähigkeit je Startart, `right_not_granted`), `exec_grant_runtime.rs`, `exec_grant_persistence.rs`, `relation_rights.rs` (`CapabilityNarrowed(Capability)`, `GrantExpired`), `exec_registry_authority.rs` (Erweiterung bindet laufende Jobs an die neue Revision), `exec_targets.rs`, `N/daemon/os/shared/exec_grant_journal.rs` (nur Typwechsel).
- **Verträge:** D.2.1, Abschnitt E.
- **Oberflächen:** keine sichtbare Änderung (Anzeigen ab PRM-M8).
- **Abhängigkeiten:** keine; S4 SCR-M2 hängt hiervon ab.
- **Plattformprüfung:** reiner Kern.
- **Abnahmesignal:** `perm_task_port_full_shell_0_5_180`, `perm_task_port_program_list_0_5_182`, `perm_task_port_run_as_root_worker`, `perm_task_port_disabled_keeps_settings`, `perm_task_write_legacy_form_bytes_unchanged`, `perm_task_old_reader_sees_new_grant_off`, `perm_task_unknown_access_version_fail_closed`, `perm_task_relation_proof_matches_exec_auth`, `perm_task_narrowing_matrix`, `perm_task_expiry_ends_sessions`, `perm_task_allow_own_ls_runs`.
- **Release:** keine Auswirkung.

#### PRM-M2 — Netzvertrag: Rechteansicht, Client-Merkmale, Rückfragerahmen, Fehltexte

- **Ergebnis:** Vor dem Start erfährt das verbindende Gerät Konto, Fähigkeiten und Rückfrage; während einer Rückfrage zeigen `se shell`/`se exec`/MCP/Android-`ExecDialog` „Warte auf Bestätigung …“; Ablehnungen nennen Grund und nächsten Schritt.
- **Neue Dateien:** `N/share/core/exec_access_view.rs` (≤ 200), `N/share/core/exec_client_messages.rs` (≤ 250).
- **Geänderte Dateien:** `exec_protocol.rs` (Rahmen 39, `client_features`, `access`), `exec_terminal.rs` (Merkmale), `exec_server.rs` (`Pong` während offener Rückfrage), `exec_client.rs`, `exec_client_active.rs` (Ereignis `ApprovalPending`), `N/cli/exec.rs`, `N/cli/shell.rs`, `N/cli/shell_terminal.rs`, `N/cli/mcp/exec_share_job.rs` (Zustand `waiting_for_approval`), `A/ui/share/ExecDialog.kt`.
- **Verträge:** D.2.2.
- **Oberflächen:** CLI Text/JSON (Ereignis `{"event":"approval_pending","device":…,"surface":…}`), MCP-Jobstatus, Android `ExecDialog`; Desktop ohne Befehlsdialog (bestehende Ausnahme, `share-exec.md` Zeile 33).
- **Abhängigkeiten:** PRM-M1.
- **Plattformprüfung:** plattformneutral; Statuszeile unter Windows über `N/cli/shell_output_filter.rs`.
- **Abnahmesignal:** `perm_task_hello_ok_access_view`, `perm_task_old_client_no_pending_frame`, `perm_task_old_client_survives_long_approval` (Client ohne `client_features`, Entscheidung nach 45 s → `STARTED`), `perm_task_new_client_old_host`, `perm_task_error_texts_have_next_step`, `perm_task_terminal_off_rejects_login`.
- **Release:** keine Auswirkung.

#### PRM-M3 — Rückfragedienst, Rückfragefenster, IPC

- **Ergebnis:** Steht eine Fähigkeit auf „Fragen“, erscheinen am Host Fenster bzw. Benachrichtigung mit „Diesmal erlauben / Immer erlauben / Ablehnen“; der Start wartet bis zur Entscheidung oder zum Abbruch.
- **Neue Dateien:** `N/share/core/access_approval.rs` (≤ 450: Dienst, Kennung 16 Zufallsbytes hex, Reservierung in der Exec-Registry für Exec und Bildschirm, Entscheidung, Abbruch, `surface`, Trait `ConsentNotifier`, Umsetzung des S4-Traits `ConsentBroker`), `access_approval_tests.rs`, `N/daemon/os/shared/ipc_protocol_access.rs` (≤ 250: `AccessIpcRequest::{MutateGrant, Approvals, Decide, ReportUndeliverable, Accounts}`, `AccessIpcReply`), `N/daemon/os/shared/access_ipc.rs` (≤ 400: Handler), `N/share/os/shared/consent_launch.rs` (≤ 150), `N/app/core/access_consent_ui.rs` (≤ 300).
- **Geänderte Dateien:** `N/daemon/os/shared/ipc_protocol.rs` (je eine Anfrage- und Antwortvariante `Access`), `N/daemon/os/shared/exec_ipc.rs` (Weiterleitung), `N/share/core/exec_admission.rs` (Warten vor `prepare` innerhalb der Auftragsschleife), `N/notify_desktop/mod.rs`, Einstieg des GUI-Programms (Modus `--access-consent`).
- **Verträge:** D.2.3, Abschnitt E Punkt 6.
- **Oberflächen:** Desktop-Fenster; CLI (PRM-M8); Android (PRM-M10).
- **Abhängigkeiten:** PRM-M1, PRM-M2.
- **Plattformprüfung:** Linux X11/Wayland und Windows: Fenster plus Benachrichtigung, Vordergrund nicht garantiert (NR N22, N23); ohne grafische Sitzung `surface = cli_only`.
- **Abnahmesignal:** `perm_task_ask_once_starts_and_logs`, `perm_task_ask_always_sets_allow_keeps_session`, `perm_task_ask_deny_returns_approval_denied`, `perm_task_ask_client_cancel_withdraws_prompt`, `perm_task_ask_counts_against_peer_limit`, `perm_task_screen_approval_counts_against_exec_reservations` (Bildschirm-Rückfrage mit Testprinzipal belegt eine der zwei Reservierungen des Geräts), `perm_task_narrowing_withdraws_pending`, `perm_task_pending_surface_cli_only_without_display`.
- **Release:** GUI-Programm erhält den Modus `--access-consent`.

#### PRM-M4 — Protokoll mit Art-Feld

- **Ergebnis:** Das Protokoll zeigt je Eintrag Art, Konto, Fähigkeit und Rückfrageergebnis; Rückfragen, Abläufe und Ablehnungen des Kontodienstes sind eigene Ereignisse.
- **Neue Dateien:** `N/share/os/shared/access_audit.rs` (≤ 250: `record_access(AccessEntry)`, `AccessKind::Exec{…}`, `AccessKind::Screen{mode, event}`).
- **Geänderte Dateien:** `exec_audit.rs`, `exec_audit_view.rs`, `N/cli/share/exec_status.rs`, `N/app/core/share_exec_log_ui.rs`, `N/mobile/os/shared/domains/share_exec.rs` (`share.execLog`: `kind`, `account`, `approval`), `A/ui/share/ExecAuditDialog.kt`, `A/api/ShareExecApi.kt`.
- **Verträge:** D.2.4; Abschnitt E Punkt 5.
- **Abhängigkeiten:** PRM-M1, PRM-M3.
- **Abnahmesignal:** `perm_task_audit_fields_exec`, `perm_task_audit_no_shell_text`, `perm_task_audit_rotation_unchanged`.
- **Release:** keine Auswirkung.

#### PRM-M5 — Linux: Behebung `ExitType`, Konto-Einheit mit eigenem PAM-Dienst (root-Worker), Fernzugriffskonto

- **Ergebnis:** (a) Eingehende Linux-Befehle und Terminals starten auch auf systemd 249 (Ubuntu 22.04) — Behebung des Bestandsdefekts B26. (b) Ein Linux-Host mit root-Worker startet Befehle und Terminals als gewähltes Konto in einer PAM-Sitzung (Kontoablauf, Grenzen, `loginuid`), ohne dass Unterprozesse die Einhegung verlassen. (c) „Fernzugriffskonto anlegen“ legt `se-remote` an.
- **Neue Dateien:** `N/share/os/linux_os/exec_account_unit.rs` (≤ 350: Eigenschaften `User=`, `PAMName=smart-explorer-exec`, `WorkingDirectory=-~` bzw. gewünschter Ordner, Deskriptoren Typ `h`, Programmdatei nach C3), `exec_pam_service.rs` (≤ 250: Erkennung der Distribution über `/etc/os-release` `ID`/`ID_LIKE`, Inhalt der PAM-Datei je Familie, Anlage und Entfernen), `exec_binary_copy.rs` (≤ 200: Pfadprüfung, Kopie `/usr/local/sbin/smart-explorer-exec`, SHA-256), `exec_remote_account.rs` (≤ 250), `exec_supervisor_stdio.rs` (≤ 200: Modus `--share-exec-supervisor-stdio`).
- **Geänderte Dateien:** `exec_systemd.rs` (Eigenschaft `ExitType` entfernt; Kontoeigenschaften aus `exec_account_unit.rs`), `exec.rs` (Weg-Auswahl: `own` wie heute; `remote`/`account` → Konto-Einheit), `exec_supervisor.rs` (kein Kontowechsel im `pre_exec`), `exec_supervisor_account.rs` (entfällt), `exec_account.rs` (Warnung bei Shell `nologin`/`false`).
- **Verträge:** D.2.6, D.2.7.
- **Oberflächen:** Kontoauswahl „Mein Konto (root)“, „Fernzugriffskonto se-remote“, „Konto …“ (UID ≥ `UID_MIN` aus `/etc/login.defs` plus `se-remote`; Freitext erlaubt); Anzeigen in PRM-M8/PRM-M9.
- **Abhängigkeiten:** PRM-M1, PRM-M2. Die Behebung (a) hängt von nichts ab und kommt als erste Änderung.
- **Plattformprüfung:** Debian 12/13, Ubuntu 22.04/24.04/26.04, Fedora 43/44: Eigenschaften `User`, `PAMName`, `WorkingDirectory`, `Standard*FileDescriptor` ab v249 vorhanden (NR N7); PAM-Familien Debian/Ubuntu und Fedora/RHEL wie C3, Distributionen außerhalb beider Familien (nach `/etc/os-release`) nur lokale Konten; Android nicht übertragen (kein systemd).
- **Abnahmesignal:** `perm_task_systemd_unit_props_v249_compatible` (jede gesendete Eigenschaft steht in der festen v249-Liste `Description`, `Type`, `ExecStart`, `KillMode`, `KillSignal`, `SendSIGKILL`, `Restart`, `OOMPolicy`, `TimeoutStartUSec`, `TimeoutStopUSec`, `CollectMode`, `RuntimeMaxUSec`, `User`, `PAMName`, `WorkingDirectory`, `StandardInputFileDescriptor`, `StandardOutputFileDescriptor`), `perm_task_exec_linux_ubuntu2204_starts` (Annahme A10), `perm_task_remote_account_ls_runs_linux_root` (`id -un` = `se-remote`, `ls /` läuft), `perm_task_linux_root_pam_session` (`/proc/self/loginuid` = UID von `se-remote`; Grenze aus `/etc/security/limits.d/perm-task.conf` in `ulimit -n`), `perm_task_linux_root_expired_account_refused`, `perm_task_linux_password_change_required_refused`, `perm_task_linux_terminal_as_account`, `perm_task_linux_account_unit_contains_children` (Annahme A1), `perm_task_linux_cancel_contains_account_unit`, `perm_task_remote_account_cannot_read_ipc_token_linux` (Annahme A4), `perm_task_own_account_path_unchanged`.
- **Release:** keine neue Datei; `/etc/pam.d/smart-explorer-exec` und `/usr/local/sbin/smart-explorer-exec` entstehen nur auf Hosts mit Kontoprofil `remote`/`account`.

#### PRM-M6 — Linux: Kontodienst für den Nutzer-Worker

- **Ergebnis:** Auf einem Linux-Host mit Nutzer-Worker richtet der Nutzer einmal den Kontodienst ein (eine Administratorbestätigung); danach laufen Befehle und Terminals als `se-remote` oder als freigegebene Konten wie in PRM-M5.
- **Neue Dateien:** `N/share/core/exec_helper_protocol.rs` (≤ 300, plattformneutral, auch für Windows), `N/share/os/linux_os/exec_helper_server.rs` (≤ 450: Modus `--exec-account-helper`, `SO_PEERCRED`, Konfiguration, Konto-Einheit im Systemmanager, Deskriptor per `SCM_RIGHTS`, Einheiten dieser Verbindung bei Verbindungsende stoppen), `exec_helper_client.rs` (≤ 300), `exec_helper_config.rs` (≤ 200), `exec_helper_setup.rs` (≤ 400: Einheiten-Dateien, Kopie, PAM-Datei über `exec_pam_service.rs`, Konfiguration, `se-remote`, Aktivieren, Aktualisieren, Entfernen; Start über `pkexec` oder Anzeige des `sudo`-Befehls).
- **Geänderte Dateien:** `N/share/os/linux_os/exec.rs`, `N/share/core/exec_platform.rs`, `N/share/api_exports.rs` (`exec_account_choices()`; `exec_run_as_supported()` bleibt als Ableitung).
- **Verträge:** D.2.7.
- **Abhängigkeiten:** PRM-M5.
- **Plattformprüfung:** `pkexec` auf Ubuntu 24.04/26.04 Desktop und Fedora belegt, Debian 13 nicht gesichert (NR N14) → `sudo`-Befehl; SELinux Fedora `bin_t` für `/usr/local/sbin` (NR N15; Ausführung durch `init_t` Annahme A5).
- **Abnahmesignal:** `perm_task_linux_helper_setup_and_remote_account`, `perm_task_linux_helper_refuses_unlisted_account`, `perm_task_linux_helper_refuses_foreign_uid`, `perm_task_linux_helper_connection_close_stops_units`, `perm_task_linux_helper_hardened_unit_starts` (Annahme A3), `perm_task_linux_helper_outdated_reported`, `perm_task_linux_helper_remove_cleans_units`.
- **Release:** keine neue Release-Datei; `docs/RELEASING.md` erhält den Satz, dass der Kontodienst nicht Teil des Feeds ist.

#### PRM-M7 — Windows: Kontodienst mit S4U, Job im Dienst, Relais über benannte Pipes

- **Ergebnis:** Auf einem Windows-PC richtet der Nutzer einmal den Kontodienst ein (eine UAC-Bestätigung, auf Wunsch mit `se-remote`); danach laufen Befehle und Terminals eines Geräts als `se-remote` oder als freigegebenes lokales Konto ohne Passwortablage; die Kontoauswahl zeigt nur Konten, deren Anmeldeprobe gelang, und die tatsächliche Rechtestufe.
- **Neue Dateien:** `N/share/os/windows/exec_account_service.rs` (≤ 450: Modus `--exec-account-service`, Dienststeuerung, Pipe-Server mit `PIPE_REJECT_REMOTE_CLIENTS`, `ImpersonateNamedPipeClient`-Prüfung, Leerlaufende, Sitzungen je Verbindung), `exec_account_s4u.rs` (≤ 400: `LsaRegisterLogonProcess`, `LsaLookupAuthenticationPackage`, `LsaLogonUser` Typ `Network`, `DuplicateTokenEx(TokenPrimary)`, Probe mit Erhöhungsart), `exec_account_spawn.rs` (≤ 400: Job mit Kill-on-close, Profil mit `SeBackupPrivilege`/`SeRestorePrivilege`, Umgebung, Öffnen der zwei Worker-Pipes, `CreateProcessAsUserW` mit `HANDLE_LIST` und `JOB_LIST`, `stop`, `status`, Profil entladen), `exec_account_pipes.rs` (≤ 250: Worker-Seite: zwei Pipes mit `FILE_FLAG_FIRST_PIPE_INSTANCE` und DACL nur `SYSTEM`), `exec_account_client.rs` (≤ 300: Worker-Seite des Protokolls), `exec_account_setup.rs` (≤ 450: UAC-Start `--exec-accounts-setup`, Kopie, `CreateServiceW`, Dienst-DACL, `NetUserAdd`/`NetLocalGroupAddMembers`, Konfiguration mit ACL, Aktualisieren, Entfernen mit `NetUserDel`/`DeleteProfileW` nur auf ausdrücklichen Wunsch).
- **Geänderte Dateien:** `N/share/os/windows/exec.rs` (nur Weg-Auswahl und Einhegung über den Dienst bei Kontoprofil ≠ `own`), `exec_supervisor.rs` (Versionsrahmen am Anfang der Verbindung), `native/installer.nsi` (Deinstallation entfernt Dienst und Kopie mit UAC; bei Ablehnung Ausgabe von Dienstname und Ordner).
- **Verträge:** D.2.7, D.2.8.
- **Abhängigkeiten:** PRM-M1, PRM-M2; `exec_helper_protocol.rs` aus PRM-M6.
- **Plattformprüfung:** Windows 10 ab 1809 und Windows 11 (ConPTY); lokale Konten über S4U indirekt belegt (NR N1); Konten mit verknüpftem Microsoft-Konto, Entra-Konten und Windows 11 Home nicht belegbar → Probe und Fehlertext (Annahmen A6, A7); Linux/Android nicht übertragen.
- **Abnahmesignal:** `perm_task_windows_helper_setup_remote_account` (`whoami` = `<PC>\se-remote`), `perm_task_windows_terminal_as_remote_account` (Arbeitsordner = Profilordner; frisch angelegtes Konto, Annahme A8), `perm_task_windows_helper_refuses_unlisted_account`, `perm_task_windows_pipe_caller_impersonation` (Client eines nicht eingetragenen Kontos wird abgelehnt, obwohl er die PID eines eingetragenen Prozesses vorgibt), `perm_task_windows_cancel_kills_job_tree` (Abbruch → `ActiveProcesses` 0, `powershell.exe` des Kontos beendet), `perm_task_windows_worker_disconnect_kills_jobs`, `perm_task_windows_probe_reports_token_level` (lokales Administratorkonto: `elevated = false`, Warnung entfällt; Annahme A9), `perm_task_remote_account_cannot_read_ipc_token_windows` (Annahme A4), `perm_task_windows_own_account_path_unchanged`.
- **Release:** keine neue Release-Datei; Installer nur Deinstallationsschritt.

#### PRM-M8 — CLI, Vervollständigung, MCP

- **Ergebnis:** `se share grants access` stellt jedes Recht in einem Befehl ein; `se share exec approvals|approve|deny` entscheidet Rückfragen; `se share exec accounts` zeigt und richtet Konten ein; `--room RAUM --all-members` übernimmt ein Recht für alle zugelassenen Mitglieder.
- **Neue Dateien:** `N/cli/share/grants_access.rs` (≤ 450), `grants_access_output.rs` (≤ 250), `exec_approvals.rs` (≤ 300), `exec_accounts.rs` (≤ 400), `N/cli/completions_access.rs` (≤ 200).
- **Geänderte Dateien:** `N/cli/share/grants_exec.rs` (`enable`/`disable` delegieren; Hilfetexte), `grants.rs` (JSON `exec.access`, Text `exec_as=`, `exec_commands=`, `exec_terminal=`, `exec_until=`), `N/cli/exec.rs`, `N/cli/completions.rs`, `N/cli/mcp/cli_tool.rs` (Sperrliste), `N/cli/mcp/catalog.rs` (Beispiele `["share","exec","approvals","--json"]`, `["share","exec","accounts","--json"]`).
- **Verträge:** D.2.4.
- **Abhängigkeiten:** PRM-M1 bis PRM-M7.
- **Plattformprüfung:** Linux-Einrichtung ohne root: `pkexec` bei interaktivem Terminal, sonst Ausgabe des `sudo`-Befehls; Windows UAC; Android ohne CLI.
- **Abnahmesignal:** `perm_task_cli_grants_access_json_shape`, `perm_task_cli_enable_alias_unchanged`, `perm_task_cli_room_all_members_snapshot`, `perm_task_cli_approvals_roundtrip`, `perm_task_cli_accounts_json`, `perm_task_completions_access`, `perm_task_mcp_blocks_approve_and_setup`, `perm_task_mcp_reads_approvals`.
- **Release:** keine Auswirkung.

#### PRM-M9 — Desktop

- **Ergebnis:** Geräteseite und Tab **Befehle** zeigen den Abschnitt **Zugriff** (Zustand je Fähigkeit, „Ausführen als“); **Ändern…** öffnet den Zugriffsdialog; der Tab **Befehle** zeigt oben ausstehende Freigaben, unten **Konten dieses Geräts** (Kontodienst einrichten, Konten freigeben mit Probe-Ergebnis, Fernzugriffskonto anlegen, entfernen); die Raumseite bietet **Zugriff für alle Mitglieder…**.
- **Neue Dateien:** `N/app/core/share_access_dialog_ui.rs` (≤ 450), `share_access_accounts_ui.rs` (≤ 350), `share_access_approvals_ui.rs` (≤ 250), `share_access_room_ui.rs` (≤ 250).
- **Geänderte Dateien:** `share_exec_ui.rs` (Aktivierungsteil und Kontofeld wandern in den Dialog; Texte nach C6), `share_exec_programs_ui.rs`, `share_device_page_ui.rs` („BEFEHLE (EXEC)“ → „ZUGRIFF“), `share_room_page_ui.rs`, `share_pages_ui.rs` (Tab 7 bleibt).
- **Abhängigkeiten:** PRM-M1 bis PRM-M8.
- **Plattformprüfung:** Einrichtung Windows UAC, Linux `pkexec` oder kopierbarer `sudo`-Befehl.
- **Abnahmesignal:** `perm_task_gui_access_dialog_presets`, `perm_task_gui_access_dialog_one_confirm`, `perm_task_gui_approvals_block`, `perm_task_gui_accounts_page_states` (nicht eingerichtet, eingerichtet, veraltet, Konto mit gescheiterter Probe), `perm_task_gui_room_all_members`, `perm_task_gui_setup_without_pkexec_shows_sudo_command`.
- **Release:** keine Auswirkung.

#### PRM-M10 — Android-Bridge und Android-App

- **Ergebnis:** „Erlauben…“ öffnet den Zugriffsdialog mit „Ausführen als: App-Konto“ samt Begründung (C4), Befehle/Terminal je Aus/Fragen/Erlauben, Zeitraum und Programmen; Rückfragen erscheinen als Dialog in der App oder als Benachrichtigung hoher Wichtigkeit; Bildschirm-Rückfragen öffnen beim Erlauben direkt die S4-Zustimmungsaktivität; fehlende Benachrichtigungsfreigabe wird angezeigt und führt beim anderen Gerät zu `approval_unavailable`.
- **Neue Dateien:** `N/mobile/os/shared/domains/share_access.rs` (≤ 400: `share.setAccess`, `share.accessApprovals`, `share.decideAccessApproval`, `share.reportApprovalUndeliverable`, Felder in `share.status`), `A/api/ShareAccessApi.kt`, `A/ui/share/AccessDialog.kt`, `A/service/AccessApprovalNotifier.kt` (Kanal `access_approval`, `IMPORTANCE_HIGH`; Aktionen je Fähigkeit nach C5), `A/service/AccessApprovalReceiver.kt` (nicht exportiert).
- **Geänderte Dateien:** `N/mobile/os/shared/domains/share_exec.rs` (`program_settings` behält das Kontoprofil, B16), `A/ui/share/ExecHostSection.kt` (Warnzeile Benachrichtigungen), `A/ui/share/ExecProgramsDialog.kt`, `A/service/ExecHostNotifier.kt` (weckt den Rückfrage-Notifier), `A/system/Notifications.kt` (Kanal), `android/app/src/main/AndroidManifest.xml` (Receiver), `docs/superpowers/plans/2026-09-25-android-apk/api.md`.
- **Abhängigkeiten:** PRM-M1 bis PRM-M4; S4 SCR-M8 liefert `ScreenConsentActivity`.
- **Plattformprüfung:** API 30–36; `setPendingIntentCreatorBackgroundActivityStartMode` gibt es ab API 34, darunter genügt der Benachrichtigungstipp als Ausnahme (NR N21; Annahme A11); `POST_NOTIFICATIONS` ab API 33.
- **Abnahmesignal:** `perm_task_bridge_set_access_absent_keeps`, `perm_task_bridge_status_account_choices_android`, `perm_task_bridge_program_list_keeps_profile`, `perm_task_bridge_approvals_roundtrip`, `perm_task_android_approval_channel_high_importance`, `perm_task_android_undeliverable_reports_unavailable`, `perm_task_android_screen_approval_actions_are_activity_intents` (gemeinsam mit S4 `screen_task_android_consent_action_is_activity_intent`), `perm_task_android_access_dialog_two_actions`.
- **Release:** APK mit neuem Receiver und Kanal; keine neue Berechtigung.

#### PRM-M11 — Inventur, Doku, TODO

- **Ergebnis:** Inventur und Doku beschreiben das Modell mit Belegen; A1–A10 der Inventur behoben; Kostenklassen definiert.
- **Geänderte Dateien:** `docs/oberflaechen/share-exec.md` (Abschnitt „Stand nach Stapel Rechte“: Zugriffsrecht anzeigen/ändern, Ausführen als, Rückfrage entscheiden, Gültig bis, Konten dieses Geräts, Raum-Sammelaktion, Protokoll; Ausnahmen MCP und Android-Konten; A1–A10), `docs/oberflaechen/INDEX.md` (Kostenklassen), `docs/ARCHITEKTUR.md` (Zeile „Exec-Host“), `README.md` (Exec-Rechte, Kontodienst, Plattformgrenzen, Verhaltensänderung root-Worker: PAM-Kontoprüfung), `docs/GOTCHAS.md` (S4U ohne Netzanmeldedaten, Sitzung 0, eigener PAM-Dienst ohne `pam_systemd` und ohne `XDG_RUNTIME_DIR`, `ExitType` auf v249, Programmkopie), `docs/RELEASING.md`, `docs/TODO.md` („Nur lesen“ mit den Bedingungen aus C8; Android-Kontexte root/Shizuku mit Einhegungsfrage), `api.md`.
- **Abhängigkeiten:** PRM-M1 bis PRM-M10.
- **Abnahmesignal:** `perm_task_inventory_paths_exist`.
- **Release:** keine Auswirkung.

### D.2 Verträge

#### D.2.1 Speicherform und Portierung

```json
"exec": {
  "enabled": false,
  "policy_revision": 12,
  "changed_at": 1760700000,
  "source_request_id": null,
  "source_decision_revision": null,
  "run_as": {"kind": "account", "name": "se-remote"},
  "access": {
    "version": 1,
    "account": {"kind": "remote"},
    "capabilities": {"exec_commands": "allow", "exec_terminal": "ask"},
    "valid_until": 1760728800
  }
}
```

- `access.account`: `{"kind":"own"}` | `{"kind":"remote"}` | `{"kind":"account","name":"<Name>"}` (Name höchstens 256 Byte).
- `access.capabilities`: Schlüssel `exec_commands`, `exec_terminal`, `screen_view`, `screen_control`, Werte `off|ask|allow`; fehlend = `off`; unbekannte Schlüssel bleiben erhalten und wirken nicht.
- `access.valid_until`: Unix-Sekunden oder fehlend. `programs` bleibt das bisherige Feld.
- Portierungs- und Schreibregeln: C1.

#### D.2.2 Netz (ALPN `smart-explorer/share-exec/2`, Protokollversion 2 unverändert)

- `ExecClientHello.client_features: Vec<String>` (`serde(default)`, leer weggelassen); neue Clients senden `["exec_approval_v1","exec_access_v1"]`; nicht im Transkript.
- `ExecHelloOk.access: Option<ExecAccessView>`: `{"account_label":"se-remote","account_profile":"remote","commands":"allow","terminal":"ask","programs_restricted":false,"valid_until":1760728800}`; Host-Merkmale zusätzlich `exec_access_v1`, `exec_approval_v1`.
- Rahmen Host → Client `APPROVAL_PENDING` = 39, JSON `{"exec_id":"<32 hex>","capability":"exec_terminal","surface":"window"|"notification"|"cli_only"}`; nur an Clients mit `exec_approval_v1`; danach `STARTED` (32) oder `EXEC_ERROR` (36).
- Während der Rückfrage beantwortet der Host `PING` (20) mit `PONG` (37).
- Neue Codes in `ExecWireError.code`: `right_not_granted`, `approval_denied`, `approval_cancelled`, `approval_unavailable`, `account_not_allowed`, `account_helper_unavailable`; bestehende Codes unverändert. Ablauf und Entzug vor dem Handshake bleiben `permission_denied`.

#### D.2.3 Daemon-IPC

- `IpcRequest::Access { token, request: AccessIpcRequest }`, `IpcResponse::Access(AccessIpcReply)`.
- `AccessIpcRequest`: `MutateGrant { target: ExecGrantTarget, change: AccessGrantChange, expected_revision: Option<u64> }`, `Approvals`, `Decide { approval_id: String, decision: "once"|"always"|"deny" }`, `ReportUndeliverable { approval_id: String }`, `Accounts`.
- `AccessGrantChange` (jedes Feld optional, fehlt = behalten): `account`, `capabilities` (nur genannte Schlüssel), `programs`, `expiry` (`"none"` | `{"at": i64}`).
- `AccessIpcReply`: `GrantMutation { result: ExecGrantPersistResult, grant: AccessGrantView }`, `ApprovalList { pending: Vec<ApprovalView> }`, `Decided`, `AccountList { helper: HelperStatus, choices: Vec<AccountChoice> }`.
- `ShareEvents`: UI-Ereignis `AccessApprovalRequested { approval_id }`.
- `MutateExecGrant { target, enabled, settings }` bleibt: `enabled:true` ohne `settings` = Befehle und Terminal `allow` (mit gespeicherter Liste: Befehle `allow`, Terminal `off`); `settings.run_as`/`programs` wie bisher; `access` bleibt sonst unberührt.

#### D.2.4 CLI, JSON, Bridge, MCP

```
se share grants access [SELEKTOR | --room RAUM --all-members]
    [--as own|remote|account:NAME]
    [--commands off|ask|allow] [--terminal off|ask|allow] [--ask]
    [--for DAUER | --until ZEITPUNKT | --no-expiry]
    [--allow-program PFAD]... [--full] [--off] [--yes] [--json]
se share exec approvals [--json]
se share exec approve ID [--always]
se share exec deny ID
se share exec accounts [--json]
se share exec accounts setup [--remote-account] [--allow KONTO]... [--yes]
se share exec accounts allow KONTO | disallow KONTO | update | remove [--delete-remote-account]
```

- `SELEKTOR` wie bei `se share grants exec`. Ohne Änderungsoption zeigt `access` das Recht. `--ask` setzt jede erlaubte Fähigkeit auf `ask`; `--off` setzt beide Exec-Fähigkeiten auf `off`; `DAUER` mit `m`, `h`, `d`; `ZEITPUNKT` RFC 3339, ohne Zone Ortszeit des Hosts. S4 registriert `--screen-view off|ask|allow`, `--screen-control off|ask|allow` aus dem Katalog.
- `se share grants exec enable|disable` bleiben mit allen bisherigen Optionen.
- JSON `se share grants --json`: bisherige Felder bleiben (`exec.enabled` = eine Exec-Fähigkeit nicht `off` und nicht abgelaufen); neu `exec.access` = `{"account":{"kind":"remote"},"account_label":"se-remote","capabilities":{"exec_commands":"allow","exec_terminal":"ask"},"valid_until":1760728800,"expired":false}`. `se share grants exec --json`: plus `access`.
- `se share exec approvals --json`: `[{"id","device_name","device_id","fingerprint","relation":"direct"|"room","room","capability","account_label","command":{"mode":"argv","program","args"}|{"mode":"shell","text"}|{"mode":"login"}|null,"requested_at","surface"}]` (`command` `null` bei Bildschirmfähigkeiten).
- `se share exec accounts --json`: `{"helper":{"state":"not_needed"|"not_installed"|"installed"|"outdated"|"unavailable","version","detail"},"choices":[{"profile":"own","label","available":true},{"profile":"remote","account":"se-remote","available","reason","next_step","elevated":false,"admin_group":false},{"profile":"account","account","available","reason","elevated","admin_group"}]}`.
- Vervollständigung: `enabled · as se-remote · commands allow · terminal ask · until 2026-10-17 18:00`; Werte für `--as` aus `accounts`, für `approve`/`deny` aus `approvals`.
- Bridge: `share.status.execProvider` plus `accountChoices` (Form wie `choices`, camelCase), `otherAccountsNote` (Text C4), `elevated`; `share.status.execTargets[]` plus `access` (`{account, accountLabel, capabilities:{execCommands, execTerminal, screenView?, screenControl?}, validUntil, expired}`); `share.setAccess {targetKey, account?, capabilities?, programs?, validUntil?}` → `{revision, access}` (fehlt = behalten, `validUntil: null` = Ablauf entfernen); `share.accessApprovals {}` → `{pending:[…]}`; `share.decideAccessApproval {approvalId, decision}` → `{}`; `share.reportApprovalUndeliverable {approvalId}` → `{}`; `share.setExec` bleibt; `share.execLog` plus `kind`, `account`, `approval`.
- MCP: über `se`; Sperrliste und lesende Beispiele nach C7.

#### D.2.5 Rust-Schnittstelle für S4

Siehe Abschnitt E.

#### D.2.6 Linux-Konto-Einheit

Eigenschaften: `Description`, `Type=notify` (Suite-Lauf 2: mit `PAMName=` hält der PAM-Sitzungsprozess von systemd 249 und 255 den `execve()`-Melder offen, `Type=exec` endete nach `TimeoutStartUSec`; der Supervisor meldet `READY=1`, sobald er seine Verbindung hält), `ExecStart=<root-eigene se-Datei> --share-exec-supervisor-stdio`, `KillMode=control-group`, `KillSignal=SIGKILL`, `SendSIGKILL=true`, `Restart=no`, `OOMPolicy=stop`, `TimeoutStartUSec=15 s`, `TimeoutStopUSec=2 s`, `CollectMode`, optional `RuntimeMaxUSec`, `User=<Konto>`, `PAMName=smart-explorer-exec`, `WorkingDirectory=-~` (bzw. gewünschter Ordner ohne `-`), `StandardInputFileDescriptor=<h>`, `StandardOutputFileDescriptor=<h>`. `ExitType` wird nicht gesendet (auch nicht in der bestehenden Einheit, B26). Einheitsname `smart-explorer-exec-<exec_id>.service`.

#### D.2.7 Kontodienst-Protokoll (Linux und Windows, `N/share/core/exec_helper_protocol.rs`)

Rahmen: 4 Byte Länge (Big Endian) + JSON, höchstens 64 KiB. `{"hello":{"protocol":1,"se_version":"0.5.x"}}` → `{"hello_ok":{"protocol":1,"se_version":"…"}}` oder `{"error":{"code":"outdated","detail":"…"}}`. `{"probe":{"account":"se-remote"}}` → `{"probe":{"ok":true,"elevated":false,"admin_group":false,"detail":null}}`. Linux: `{"start":{"account":"se-remote","cwd":null}}` mit Deskriptor per `SCM_RIGHTS`; Windows: `{"start":{"account":"se-remote","cwd":null,"pipe_in":"\\\\.\\pipe\\SmartExplorerExec-<id>-in","pipe_out":"…-out"}}` → `{"started":{"session":"<unit>|<id>"}}`. `{"stop":{"session":…}}` → `{"stopped":{}}`; `{"status":{"session":…}}` → `{"status":{"active":true,"empty":false}}`. Alle Sitzungen einer Verbindung enden, wenn die Verbindung schließt. Fehlercodes: `not_allowed`, `unknown_account`, `logon_failed`, `password_change_required`, `outdated`, `start_failed`, `bad_request`.

#### D.2.8 Konfiguration des Kontodienstes

Windows `%ProgramData%\Smart Explorer\exec-accounts.json`: `{"version":1,"users":[{"sid":"S-1-5-21-…","accounts":["se-remote","deploy"]}]}`; ACL: SYSTEM und Administrators Vollzugriff, Users Lesen. Linux `/etc/smart-explorer/exec-accounts.toml`: `[[user]]` `uid = 1000` `accounts = ["se-remote"]`. PAM-Datei Linux `/etc/pam.d/smart-explorer-exec` mit Inhalt je Familie nach C3.

### D.3 Bedienaufwand vorher → nachher

Zählung wie Lesung Oberflächen 1d (Öffnen des Fensters nicht gezählt, Einstieg gezählt); „vorher“ am Code bestätigt (NR N13).

| Aufgabe | CLI vorher → nachher | Desktop vorher → nachher | Android vorher → nachher |
|---|---|---|---|
| T1 Befehle und Terminal als mein Konto | 1 Befehl (C1) → 1 Befehl `se share grants access GERÄT --yes` (C1) | 4: Einstieg, „Remote-Ausfuehrung aktivieren …“, Kästchen, „Exec jetzt aktivieren“ (C3) → 3: Einstieg, „Erlauben…“, „Als {Konto} erlauben“ (C2) | 3: „Erlauben…“, „Ich habe verstanden“, „Aktivieren“ (C2) → 2: „Erlauben…“, „Erlauben“ (C2) |
| T2 Eingeschränkt durch das OS (eigenes Konto) | Linux-root-Worker: Konto im OS anlegen + `enable --run-as NAME --yes` (C4); Nutzer-Worker, Windows: nicht möglich → einmal je Host `se share exec accounts setup --remote-account` + Administratorbestätigung (C4, einmal), danach `--as remote` (C1) | Linux-root-Worker: Konto im OS anlegen, Feld tippen (C4); sonst nicht möglich → einmal „Konten dieses Geräts“ → „Kontodienst einrichten“ → „Einrichten“ → Administratorbestätigung (C4, einmal), danach 4: Einstieg, „Erlauben…“, „Eingeschränktes Fernzugriffskonto“, „Als se-remote erlauben“ (C3) | nicht möglich → nicht möglich, Begründung im Dialog (C0 Anzeige) |
| T3 Nur bestimmte Programme | je Programm `--allow-program` (C4) → unverändert (C4) | Liste je Eintrag (C4) → unverändert, im Dialog aufklappbar (C4) | Liste je Eintrag (C4) → unverändert (C4) |
| T4 Rückfrage je Sitzung einstellen | nicht möglich → `--ask` (C1) | nicht möglich → „Mit Rückfrage“ im Dialog (+1) | nicht möglich → Schalter (+1) |
| T5 Rückfrage beantworten | – → `se share exec approve ID` (C1) | – → ein Klick im Rückfragefenster (C1) | – → eine Aktion in Benachrichtigung oder App-Dialog (C1) |
| T6 Zeitlich begrenzen | nicht möglich → `--for 8h` (C1) | nicht möglich → Zeitraum im Dialog (+1) | nicht möglich → Zeitraum-Chip (+1) |
| T7 Alle zugelassenen Mitglieder eines Raums | n × Befehl (C4 ab n > 6) → 1 Befehl `--room RAUM --all-members` (C1) | n × 4 (C4) → 3: Raumseite, „Zugriff für alle Mitglieder…“, „Für n Mitglieder erlauben“ (C2) | n × 3 (C4) → 3: Raumzeile ⋮, „Zugriff für alle Mitglieder…“, „Erlauben“ (C2) |
| T8 Entziehen | 1 (C1) → 1 (C1), Ausgabe nennt beendete Sitzungen | 1 (C1) → 1 (C1), Hinweistext | 1 (C1) → 1 (C1), Hinweistext |
| T9 Als wen arbeite ich auf dem anderen Gerät | nicht sichtbar → Kopfzeile (C0) | kein Befehlsdialog (Ausnahme bleibt) | nicht sichtbar → Zeile im `ExecDialog` (C0) |
| T10 Warum abgelehnt | „exec authentication failed“ → Text mit Grund und nächstem Schritt (C0) | wie CLI über Jobansicht | wie CLI im `ExecDialog` |

### D.4 Plattformprüfung je übertragenem Mechanismus

| Mechanismus (Herkunft) | Windows | Linux | Android |
|---|---|---|---|
| Relais im Zielkonto, das die Pseudokonsole selbst öffnet (Tailscale, OpenSSH) | übertragen; Supervisor unverändert über zwei benannte Pipes als stdin/stdout | nicht nötig (Supervisor läuft in der Konto-Einheit als Konto) | nicht übertragen (C4) |
| S4U ohne Passwort, Typ `Network` + Primärtoken (Tailscale, OpenSSH) | übertragen (NR N1); Probe statt Annahme für Microsoft-/Entra-Konten | nicht nötig | nicht übertragbar |
| Job beim privilegierten Dienst statt beim Worker | übertragen (Sitzungsregel der Jobs, NR N6) | Entsprechung: Einheit im Systemmanager, Steuerung über den Kontodienst | nicht übertragbar |
| Aufruferprüfung | `ImpersonateNamedPipeClient` (NR N5) | `SO_PEERCRED` (UID vom Kernel) | entfällt |
| PAM-Sitzung über systemd `PAMName=` (machined) | entfällt | übertragen mit eigenem Dienst ohne `pam_systemd` (NR Z-L1) | nicht übertragbar |
| Rückfrage am Host (RustDesk, AnyDesk, TeamViewer, MeshCentral) | Fenster + Ballon, Vordergrund nicht garantiert (NR N23) | Fenster + freedesktop-Benachrichtigung (NR N22); ohne grafische Sitzung `cli_only` | Kanal hoher Wichtigkeit; Bildschirm mit Aktivitäts-Intent (S4) |
| Programmkopie an geschütztem Ort | `%ProgramFiles%` | `/usr/local/sbin` (`bin_t`, NR N15) | nicht nötig |
| „Nur lesen“-Sandbox | nicht übertragen (C8) | nicht übertragen (C8) | nicht übertragen (C8) |

### D.5 Auswirkung auf Release und Installer

- Release-Artefakte: keine neue Datei; Programme erhalten die Modi `--exec-account-helper` (Linux), `--exec-account-service` (Windows), `--exec-accounts-setup`/`--exec-accounts-remove` (beide, erhöht), `--share-exec-supervisor-stdio` (Linux), `--access-consent` (GUI). Feed, Hashes, APK-Liste unverändert.
- Windows-Installer: nur Deinstallationsschritt; der Updater ersetzt die Kopie in `%ProgramFiles%` nicht; bei inkompatibler Protokollversion meldet die App „Kontodienst aktualisieren“ (eine UAC-Abfrage).
- Linux: `install-linux.sh` unverändert; Einrichtung/Entfernen über `se share exec accounts setup|remove`; root-Worker legt `/etc/pam.d/smart-explorer-exec` und `/usr/local/sbin/smart-explorer-exec` bei der ersten Kontowahl an. Verhaltensänderung root-Worker: Kontoablauf und erzwungener Passwortwechsel lehnen jetzt ab (Release-Text).
- Rechte nach Einrichtung: Windows-Dienst LocalSystem (Start bei Bedarf, Leerlaufende 10 min); Linux-Socketdienst root mit Härtung.
- Deinstallation: Windows-Uninstaller entfernt Dienst und Kopie mit UAC; `se-remote` nur auf ausdrückliche Wahl. Linux `se share exec accounts remove [--delete-remote-account]`.
- CI: Die Suite braucht einen Linux-Runner mit root (Muster Paar `exec-root`, `docs/ARCHITEKTUR.md:146`), einen Ubuntu-22.04-Runner für `perm_task_exec_linux_ubuntu2204_starts` (Annahme A10) und einen Windows-Runner mit Administratorrechten (Annahme A12).

## E — Schnittstelle zu S4 (Rechte für Bildschirm ansehen/steuern)

Vertrag in der Form, die S4 E1 (Endfassung) übernommen hat:

1. **Fähigkeiten** (`N/share/core/access_grant.rs`): `pub enum Capability { ExecCommands, ExecTerminal, ScreenView, ScreenControl }` (fest im Enum, kein Registrierverfahren für Fähigkeiten), `pub enum CapabilityDecision { Off, Ask, Allow }`, `pub struct CapabilityState { pub decision, pub policy_revision: u64, pub changed_at: i64 }`; je Direktfreigabe und je Raummitglied; Standard `Off`; alte Profile → `Off`; `policy_revision`/`changed_at` des ganzen Rechts; „Gültig bis“ gilt auch für Bildschirmfähigkeiten; Kontoprofil gilt nicht für sie.
2. **Beweis und Prüfung:** `prove_relation(&RelationServerHello, &RelationClientHello, remote_node: &str, &ShareAuthState) -> Result<ProvenPrincipal, RelationRejection>` mit Transkript-Präfix als Feld (S4: `smart-explorer/share-screen/v1|`); `capability_for(&ShareAuthState, &ProvenPrincipal, Capability, now: i64) -> CapabilityState`. Exec-Verhalten unverändert (`perm_task_relation_proof_matches_exec_auth`).
3. **Katalog und Oberflächen:** `capability_catalog(&HostFacts) -> Vec<CapabilityEntry>`; `HostFacts { graphical_session: bool, account_helper: HelperStatus }` (genau diese zwei Felder; das frühere Feld `read_only` entfällt mit C8); `access_catalog.rs` ruft `crate::screen::core::catalog_entries::register_screen_entries(&mut entries, &facts)` (S4-Datei `N/screen/core/catalog_entries.rs`, Bedingung `facts.graphical_session`); bis SCR-M2 enthält der Aufruf keine Einträge, und keine Oberfläche zeigt Bildschirmzeilen. CLI `se share grants access … --screen-view … --screen-control …`, JSON `screen_view`/`screen_control`, Bridge `screenView`/`screenControl` über `share.setAccess`, Desktop-Zugriffsdialog, `AccessDialog.kt`.
4. **Entzug:** `RestrictionReason::CapabilityNarrowed(Capability)` und `RestrictionReason::GrantExpired` über `invalidate_restrictions_at`; S4 hängt `screen_registry().restrict_authorization(epoch, restrictions)` ein.
5. **Protokoll:** `record_access(AccessEntry { kind: AccessKind::Screen { mode, event }, principal, outcome, approval, … })` in `N/share/os/shared/access_audit.rs`, Dateien `exec-audit.log`/`exec-audit-rejected.log`; Anzeigen mit Art „Bildschirm“.
6. **Rückfragedienst:** `access_approval` setzt `ConsentBroker` um (`request(principal, capability, summary) -> ApprovalTicket`, `once|always|deny`, Abbruch durch den Betrachter); Fenster `smart_explorer --access-consent <ID>` mit „{Gerät} möchte den Bildschirm ansehen“ bzw. „… steuern“; IPC `Access::Approvals`/`Access::Decide`; MCP entscheidet nicht. Eine offene Bildschirm-Rückfrage belegt eine Reservierung der Exec-Registry (2 je Gerät, 8 gesamt) bis zur Entscheidung (`perm_task_screen_approval_counts_against_exec_reservations`).
7. **Android (Anforderung von S4, übernommen):** Für `ScreenView`/`ScreenControl` sind „Diesmal erlauben“ und „Immer erlauben“ der S1-Benachrichtigung `PendingIntent.getActivity` auf `A/screen/ScreenConsentActivity.kt`, erzeugt mit `ActivityOptions.setPendingIntentCreatorBackgroundActivityStartMode(MODE_BACKGROUND_ACTIVITY_START_ALLOWED)` (ab API 34); die Aktivität meldet die Entscheidung über `share.decideAccessApproval`; „Ablehnen“ bleibt Broadcast. Signale `perm_task_android_screen_approval_actions_are_activity_intents` (S1) und `screen_task_android_consent_action_is_activity_intent` (S4).
8. **Bedingung von S4:** Der Kontodienst ersetzt den Daemon in der Nutzersitzung nicht als Bildschirm-Host; er startet nur Exec-Supervisoren (Windows in Sitzung 0, Linux in Konto-Einheiten).

## F — Risiken, Annahmen und Entscheidung des Hauptagenten

**Annahmen mit Wirkung und Abnahmesignal:**

| Nr. | Annahme | Wirkung, falls falsch | Abnahmesignal bzw. Rückfall |
|---|---|---|---|
| A1 | Debian/Ubuntu `common-session-noninteractive` enthält kein `pam_systemd` | Unterprozesse verlassen die Konto-Einheit | `perm_task_linux_account_unit_contains_children`; Rückfall: Sitzungsteil explizit wie für Distributionen außerhalb beider Familien (C3 LC-a) |
| A2 | `ExitType=` hat ab v250 den Standardwert `main` | Verhalten der Einheit beim Ende des Hauptprozesses ändert sich | `perm_task_linux_cancel_contains_account_unit`, bestehende Linux-Paarläufe |
| A3 | systemd erlaubt `StartTransientUnit` einem root-Dienst mit leerem `CapabilityBoundingSet` (NR N16) | Kontodienst startet keine Einheit | `perm_task_linux_helper_hardened_unit_starts`; Rückfall: `CapabilityBoundingSet=CAP_SYS_ADMIN` |
| A4 | Ein anderes Konto kann `daemon.token` des Workers nicht lesen (Linux `0600`; Windows `write_private_atomic`, Inhalt nicht gelesen) | Fernkonto könnte den Worker über die IPC steuern | `perm_task_remote_account_cannot_read_ipc_token_linux`, `…_windows` |
| A5 | systemd (`init_t`) führt `/usr/local/sbin/smart-explorer-exec` (`bin_t`) unter Fedora-SELinux aus (NR N15 Lücke) | Kontodienst/Konto-Einheit startet auf Fedora nicht | `perm_task_linux_helper_setup_and_remote_account` auf Fedora (falls Runner vorhanden, sonst als ungeprüft berichtet) |
| A6 | `NetUserAdd` funktioniert auf Windows 11 Home aus einem erhöhten Prozess (NR N17) | `se-remote` auf Home nicht anlegbar | Fehlertext mit nächstem Schritt „vorhandenes Konto freigeben“; ungeprüft (kein Home-Runner) |
| A7 | S4U für Konten mit verknüpftem Microsoft-Konto (NR N1b) | Konto nicht wählbar | keine Annahme in der Anzeige: Probe zeigt es als nicht verfügbar; ungeprüft (kein Microsoft-Konto im CI) |
| A8 | `LoadUserProfileW` legt das Profil eines nie angemeldeten Kontos an (NR N2) | Relais ohne Profil, Arbeitsordner fehlt | `perm_task_windows_terminal_as_remote_account`; Rückfall: vorher `CreateProfile` |
| A9 | Die Filterung lokaler Administratorkonten gilt auch für S4U vom Typ `Network` aus LocalSystem (NR N3 Lücke) | Warnung falsch | keine Wirkung auf die Anzeige (Probe misst); Signal `perm_task_windows_probe_reports_token_level` |
| A10 | CI hat einen Ubuntu-22.04-Runner | Behebung `ExitType` nur statisch geprüft | `perm_task_systemd_unit_props_v249_compatible` prüft statisch; Lauf als ungeprüft berichten |
| A11 | Unter API 30–33 startet ein `PendingIntent.getActivity` aus einer Benachrichtigungsaktion die Aktivität ohne weitere Option (NR N21) | Bildschirm-Freigabe auf Android 11–13 öffnet keine Aktivität | `perm_task_android_screen_approval_actions_are_activity_intents` (Instrumentierung je API, soweit Emulatoren vorhanden) |
| A12 | CI-Windows-Runner mit Administratorrechten erlaubt Dienst und `NetUserAdd` | Windows-Signale nicht lauffähig | als ungeprüft berichten |

**Risiken:**

| Nr. | Risiko | Wirkung | Gegenmaßnahme |
|---|---|---|---|
| R1 | Kompromittierter Worker nutzt jedes für ihn freigegebene Konto | Rechte dieser Konten | Liste nur per Administrator; `se-remote` Standardkonto; Dienst-Protokoll unabhängig vom Worker |
| R2 | Kopie des Kontodienstes veraltet nach Updates | Kontowechsel abgelehnt bis zur Aktualisierung | Protokollversion, Text, `perm_task_linux_helper_outdated_reported` |
| R3 | PAM-Kontoprüfung ändert den root-Weg (Ablauf, Passwortwechsel lehnen ab) | bisher erlaubte Starts scheitern | gewollte Verengung (E2); Texte C6; Release-Text |
| R4 | Rückstufung auf 0.5.182 verliert `access` beim Speichern | neue Rechte aus | fail closed (wie B3) |
| R5 | Rückfragefenster erscheint nicht vorn (NR N22, N23) | Rückfrage übersehen | Benachrichtigung; Client zeigt Wartezustand und `surface` |
| R6 | Sitzung 0 auf Windows: Programme mit Fenster unsichtbar | Verwirrung | Hinweistext in der Kontoauswahl |
| R7 | Dateien nahe 500 Zeilen (B22) | Regelverstoß | Code in neue Dateien; IPC als Sammelvariante |
| R8 | Wegfall von „Nur lesen“ gegenüber der Erwartung „Profile … nur lesen“ im Auftrag | Funktion fehlt in diesem Stapel | C8 RO-4 (Konto), TODO mit belegten Bedingungen; Entscheidung des Hauptagenten Punkt 1 |
| R9 | Debian 13 ohne `pkexec` | Einrichtung aus der GUI nur über kopierten `sudo`-Befehl (C4) | `perm_task_gui_setup_without_pkexec_shows_sudo_command` |
| R10 | Deinstallation ohne UAC-Zustimmung lässt Dienst und Kopie zurück | Restbestand mit SYSTEM-Rechten | Ausgabe von Dienstname und Ordner; `se share exec accounts remove` nachträglich |

**Entscheidung des Hauptagenten (Punkte, die dieser Strang nicht allein festlegen sollte):**
1. Bestätigung, dass „Nur lesen“ in diesem Stapel entfällt (C8), obwohl der Auftrag es als Beispielprofil nannte.
2. Verfügbarkeit der CI-Runner für die Suite: Linux root, Ubuntu 22.04, Windows mit Administratorrechten, Fedora (A5, A10, A12); ohne sie werden die betroffenen Signale als ungeprüft berichtet.
3. Zusammenführung mit S4 in `N/daemon/os/shared/ipc_protocol.rs` (475 Zeilen): S1 fügt nur die Sammelvariante `Access` hinzu; S4 plant eigene Varianten (S4 SCR-M2) — Reihenfolge der Änderungen festlegen.
4. Release-Text zur Verhaltensänderung des Linux-root-Workers (R3) und zur Behebung von `ExitType` auf Ubuntu 22.04 (B26).

## G — Restlücken nach der Nachrecherche

Keine weitere Nachrecherche für diesen Strang. Verbleibende Lücken sind über Annahmen A1–A12, Probe-Anzeigen (Windows-Konten) oder `docs/TODO.md` („Nur lesen“) behandelt. Nicht recherchiert und für die gewählten Wege nicht entscheidungsrelevant: Wirkung von `PIPE_REJECT_REMOTE_CLIENTS` (zusätzlich zur DACL gesetzt), KWin-Fokusverhalten (Benachrichtigung deckt es ab), Laufzeit von `update-motd.d` (entfällt mit dem eigenen PAM-Dienst).

## Prüfprotokoll Nachrecherche

**Widerlegte Aussagen:**

| NR-Stelle | Bisherige Aussage | Behandlung in dieser Fassung |
|---|---|---|
| Revalidierung 2.1 (1), N6 | Windows-Worker ordnet das Relais aus Sitzung 0 seinem Kill-on-close-Job zu | C2 Einhegung: WE-2 (Job im Dienst, `stop`/`status`, Sitzungen an die Verbindung gebunden); Relais-Verbindung über zwei benannte Pipes; PRM-M7 Dateien und Signale `perm_task_windows_cancel_kills_job_tree`, `perm_task_windows_worker_disconnect_kills_jobs` |
| N5, Revalidierung 2.2 | Aufruferprüfung über `GetNamedPipeClientProcessId` | C2: `ImpersonateNamedPipeClient` + Thread-Token; kein `DuplicateHandle` in den Worker mehr; `perm_task_windows_pipe_caller_impersonation` |
| N3, Revalidierung 2.2 | S4U liefert Administratorkonten ein volles Token; Warnung „REMOTE ADMINISTRATOR SHELL“ | B18 korrigiert; C2: Probe misst `elevated`/`admin_group`, Warnung nur bei gemessener Erhöhung; Hinweistext für lokale Administratoren; `perm_task_windows_probe_reports_token_level` |
| Revalidierung 2.1 (3), N9 | Landlock genügt für „Nur lesen“ | C8: entfällt; TODO mit Bedingungen; Tests `perm_task_read_only_*` gestrichen |
| Revalidierung 2.1 (4), N4 | Integritätsstufe Low genügt für „Nur lesen“ unter Windows | C8: entfällt; `exec_low_integrity.rs` gestrichen |
| Revalidierung 2.2 Z-L1 | Einhegung über die Einheit mit `PAMName=login` | C3: LC-a eigener PAM-Dienst `smart-explorer-exec` ohne `pam_systemd`; D.2.6; Annahme A1 mit `perm_task_linux_account_unit_contains_children` |
| N7, Revalidierung 2.2 | `ExitType=main` „unverändert“ in der Einheit | B26; C3 und PRM-M5 (a): Eigenschaft entfernt; D.2.6; `perm_task_systemd_unit_props_v249_compatible`, `perm_task_exec_linux_ubuntu2204_starts` |
| C6-Beleg, Revalidierung 2.2 | „`exec_auth.rs:365-371`“ als Beleg der unbestimmten Ablehnung | C6: Produktionsstellen `exec_auth.rs:253`, `exec_admission.rs:81` |
| N15 | Ablageort `/usr/local/lib/smart-explorer/` (bzw. `libexec` als Alternative) | C3: `/usr/local/sbin/smart-explorer-exec` (`bin_t`); Annahme A5 |
| N8 Punkt 1 | R4 „`/etc/nologin` lehnen jetzt ab“ (alle Distributionen) | entfällt mit eigenem PAM-Dienst; C3 nennt: `/etc/nologin` wird nicht geprüft |
| N20 | `pam_unix` lässt jedes Systemkonto mit gesperrtem Passwort zu | C3 ergänzt: `sp_lstchg == 0` scheitert; Text und `perm_task_linux_password_change_required_refused` |

**Nicht belegbare Aussagen:**

| NR-Stelle | Aussage | Behandlung |
|---|---|---|
| N1 (b), Revalidierung 2.1 (5) | S4U für lokale Konten mit verknüpftem Microsoft-Konto | Probe je Konto statt Annahme in der Anzeige; Annahme A7 |
| N17 | `NetUserAdd` auf Windows 11 Home | Annahme A6, Fehlertext mit nächstem Schritt |
| N19 | Entra-Konten auf Entra-Geräten | Probe zeigt „nicht verfügbar“ mit Windows-Fehler |
| N4 | Start mit Integritätsstufe Low ohne `SeIncreaseQuotaPrivilege`; `powershell.exe`/`cmd.exe` auf Low | nicht mehr entscheidungsrelevant (C8) |
| N2 | `LoadUserProfileW` legt ein fehlendes Profil an | Annahme A8 mit Signal und Rückfall `CreateProfile` |
| N3 Lücke (1) | Filterung auch für S4U aus LocalSystem | Annahme A9 ohne Anzeigewirkung (Probe misst) |
| N14 | `pkexec` auf Debian 13 Desktop | `sudo`-Befehl als Rückfall (C3), Signal `perm_task_gui_setup_without_pkexec_shows_sudo_command` |
| N15 Lücke | Ausführung einer `bin_t`-Datei aus einer Einheit unter Fedora | Annahme A5 |
| N16 Annahme | D-Bus-Credentials ohne Fähigkeiten | Annahme A3 mit Rückfall |
| N18 | Ausblenden von `se-remote` auf dem Anmeldebildschirm | nicht ausgeblendet (C2); keine Annahme nötig |
| N21 | Verhalten von `PendingIntent.getActivity` unter API 30–33 ohne Startmodus-Option | Annahme A11 mit Signal |
| N22, N23 | Fenster in den Vordergrund | Benachrichtigung zusätzlich, `surface` im Rahmen (C5); Risiko R5 |

**Bestätigte Aussagen mit Ergänzung, übernommen:** N1 (Typ `Network` + `DuplicateTokenEx(TokenPrimary)`), N2 (`SeBackupPrivilege`/`SeRestorePrivilege`), N5 (`SERVICE_START` für Nutzer-SID), N7 (Eigenschaftstypen, Deskriptor `h`), N11 (Android-17-Bedingungen für Landlock → TODO), N12 (`Pong` während der Rückfrage, 60 s für alte `se shell`), N13 (Zählung vorher), N20 (`pam_systemd`-Klassen entfallen mit eigenem PAM-Dienst), N21 (Broadcast-Aktionen für Exec), N24 (Kanal hoher Wichtigkeit, `approval_unavailable`).
