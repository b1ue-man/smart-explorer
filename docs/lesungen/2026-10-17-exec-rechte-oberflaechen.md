# Exec-Rechte: Bestand der Oberflächen

Stand 2026-10-17, Lesestand da2646d2

Auftrag: Bestandslektüre für Strang S1 „Exec-Rechte nutzergebunden, Kontowechsel je OS“ (Ergebnisse E1, E2, E3) aus `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, Abschnitt A. Die Datei beschreibt den Code. Sie enthält keine Bewertung und keine Empfehlung.

## Ziel und Fragen

Ziel: den heutigen Bedienweg, die Programmliste, die Grant-Felder, die Texte und die Belege der Inventur `docs/oberflaechen/share-exec.md` am Code belegen.

1. Vollständiger Bedienweg, um einem Gerät Terminal- bzw. Befehlszugriff zu geben, je Oberfläche (CLI, Desktop, Android): jeder Schritt, jede Option, jeder Standardwert, jede Bestätigung; Schritte gezählt.
2. Programmliste: Bearbeitung, Einträge, Validierung, `--full`; Standardwahl des Desktop-Dialogs beim Aktivieren; Standardwahl auf Android.
3. Jede IPC-Nachricht, Bridge-Methode, CLI-Option, jedes JSON-Feld und MCP-Werkzeug mit Grant-Feldern.
4. Vervollständigung, Hilfe- und Fehltexte, die das Rechtemodell nennen (wörtlich).
5. Stimmen die Belege in `docs/oberflaechen/share-exec.md` mit dem Code überein? Jede Abweichung.

### Scope

- Oberflächen: CLI (Text, JSON, Vervollständigung), Desktop-GUI (egui), Android-GUI (Kotlin), Android-Bridge (JSON-Methoden `share.*`), Daemon-IPC (Loopback), MCP (`se mcp`), Kotlin-Datenmodelle. Das Netzprotokoll (`native/src/share/core/exec_protocol.rs`) ist nicht gelesen.
- Grant-Felder: `enabled`; Konto (`run_as`, Bridge `runAs`); Programmliste (`programs`); Revision (`policy_revision`, Bridge `policyRevision`, CLI-Text `exec_revision`, Ausgabe `revision`); `changed_at`; Basisfreigabe (Bridge `baseAuthorized`, CLI-JSON `authorization`, IPC `state`; die Gleichsetzung der CLI- und IPC-Felder mit der Basisfreigabe ist Annahme); Zielidentität (`device_id`, `fingerprint`, `public_key`, `node_id`, Zielschlüssel). Das Schreibrecht `write` ist ein Grant-Feld ohne Exec-Bezug und wird getrennt markiert.
- Texte: Hilfetexte (clap-Attribute, UI-Erklärtexte), Vervollständigungstexte, Fehl-, Hinweis- und Bestätigungstexte, die Exec-Freigaben, Konten, Programmlisten, Shell- oder Terminalrechte, Exec-Entzug oder Exec-Bestätigung nennen. Schaltflächen-Labels zählen nur, wenn sie eine Rechteaussage tragen.
- Kennzeichnung: „Annahme:“ markiert jede nicht belegte Aussage. „Unbekannt:“ benennt die fehlende Tatsache.

### Dateiverzeichnis

| Kurzname | Pfad |
|---|---|
| plan.md | `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md` |
| share-exec.md | `docs/oberflaechen/share-exec.md` |
| grants_exec.rs | `native/src/cli/share/grants_exec.rs` |
| grants.rs | `native/src/cli/share/grants.rs` |
| exec_status.rs | `native/src/cli/share/exec_status.rs` |
| exec.rs | `native/src/cli/exec.rs` |
| shell.rs | `native/src/cli/shell.rs` |
| shell_terminal.rs | `native/src/cli/shell_terminal.rs` |
| completions.rs | `native/src/cli/completions.rs` |
| catalog.rs | `native/src/cli/mcp/catalog.rs` |
| exec_tool.rs | `native/src/cli/mcp/exec_tool.rs` |
| exec_session_job.rs | `native/src/cli/mcp/exec_session_job.rs` |
| exec_share_job.rs | `native/src/cli/mcp/exec_share_job.rs` |
| exec_ssh_job.rs | `native/src/cli/mcp/exec_ssh_job.rs` |
| share_exec_ui.rs | `native/src/app/core/share_exec_ui.rs` |
| share_exec_programs_ui.rs | `native/src/app/core/share_exec_programs_ui.rs` |
| share_exec_log_ui.rs | `native/src/app/core/share_exec_log_ui.rs` |
| share_exec_jobs_ui.rs | `native/src/app/core/share_exec_jobs_ui.rs` |
| share_device_page_ui.rs | `native/src/app/core/share_device_page_ui.rs` |
| ipc_protocol.rs | `native/src/daemon/os/shared/ipc_protocol.rs` |
| mobile/share_exec.rs | `native/src/mobile/os/shared/domains/share_exec.rs` |
| ExecHostSection.kt | `android/app/src/main/java/app/smartexplorer/android/ui/share/ExecHostSection.kt` |
| ExecProgramsDialog.kt | `android/app/src/main/java/app/smartexplorer/android/ui/share/ExecProgramsDialog.kt` |
| ExecAuditDialog.kt | `android/app/src/main/java/app/smartexplorer/android/ui/share/ExecAuditDialog.kt` |
| ExecDialog.kt | `android/app/src/main/java/app/smartexplorer/android/ui/share/ExecDialog.kt` |
| ShareExecApi.kt | `android/app/src/main/java/app/smartexplorer/android/api/ShareExecApi.kt` |
| api.md | `docs/superpowers/plans/2026-09-25-android-apk/api.md` |

Jede Zeilenangabe nennt ihre Datei in der Form `Kurzname:Zeile` oder `Kurzname:Zeile-Zeile`; der Kurzname steht in der ersten Spalte dieser Tabelle.

## Befunde je Frage mit Beleg

### Frage 1: Bedienweg

**Vorbedingung (nicht gelesen, nicht gezählt):** Die CLI wählt nur akzeptierte Direktfreigaben (`grants_exec.rs:231`) und Raummitglieder aus Räumen mit `auto_join` (`grants_exec.rs:256`), die nicht gesperrt sind (`grants_exec.rs:257`). Ohne Auswahl lautet die Meldung „exec_grants\t0\taccept a direct request or join a room first“ (`grants_exec.rs:298`). Der Weg zur Freigabe (Anfrage, Raumbeitritt) liegt außerhalb des Lesestands (Unbekannt).

#### 1a CLI: `se share grants exec enable`

Pfad „volle Shell, ein Gerät, Standardnutzer“. Gezählt wird jede Eingabe, die der Nutzer selbst tätigt.

1. (optional) `se share grants exec` listet die Auswahl (`grants_exec.rs:109-111`; Ausgabe `grants_exec.rs:277-317`).
2. `se share grants exec enable [SELEKTOR] --yes`. Ohne `SELEKTOR` genügt die Befehlszeile, wenn genau eine deaktivierte Freigabe passt (`grants_exec.rs:44-48`, `grants_exec.rs:319-349`). Mehrere Treffer ergeben „multiple Exec grants match; choose one: …“ (`grants_exec.rs:340-347`).
3. Ohne `--yes`: Warnungen auf stderr (`grants_exec.rs:359-376`), dann die Eingabeaufforderung `Type YES to enable: ` (`grants_exec.rs:377`). Angenommen wird nur die exakte Eingabe `YES` nach trim (`grants_exec.rs:383`). Jede andere Eingabe ergibt „Exec grant was not enabled“ (`grants_exec.rs:385`).

Ergebnis: Nach der Bestätigung ruft die CLI `crate::daemon::mutate_exec_grant` auf (`grants_exec.rs:135-136`). Bei gesetzten Einstellungen folgt eine Rückprüfung (`grants_exec.rs:137-139`, `grants_exec.rs:186-207`). Die Textausgabe ist eine Zeile `exec_grant` mit `requested`, `persisted`, `applied`, `revision`, `retry` (`grants_exec.rs:146-154`). Erfolg nur bei `persisted` und `applied` (`grants_exec.rs:159-166`).

Optionen von `enable`, vollständig (`grants_exec.rs:42-78`):
- `SELEKTOR` (positional, optional).
- `--run-as USER` (`grants_exec.rs:60-65`). Standard: Konto der bestehenden Freigabe (`grants_exec.rs:90-91`); bei neuer Freigabe der Standardnutzer. `standard` oder leer ergibt den Standardnutzer (`grants_exec.rs:92`), jeder andere Wert ein Konto (`grants_exec.rs:93`).
- `--allow-program PATH`, wiederholbar (`grants_exec.rs:66-71`). Die Liste ersetzt die bisherige vollständig (`grants_exec.rs:97-100`).
- `--full` (`grants_exec.rs:72-77`). Leert die Liste (`grants_exec.rs:95-96`); schließt `--allow-program` aus (`grants_exec.rs:74`, clap `conflicts_with`).
- `--yes` (`grants_exec.rs:49-53`). Standard: aus. Mit `--yes` ruft die CLI `confirm_enable` nicht auf (`grants_exec.rs:132-134`); die Warnungen erscheinen dann nicht. Die Warnungen stehen nur in `confirm_enable` (`grants_exec.rs:351-376`).
- Global für `se share grants`: `--json` (`grants.rs:19-21`).

Ohne Optionen erhält die Freigabe die volle Shell des Standardnutzers (`grants_exec.rs:21-25`). `settings()` liefert dann `None` (`grants_exec.rs:87-89`).

Auswahlregel: Ohne Änderungsoptionen berücksichtigt `enable` nur deaktivierte Freigaben (`grants_exec.rs:117-119`, `grants_exec.rs:327`). Mit `--run-as`, `--allow-program` oder `--full` berücksichtigt es auch aktivierte Freigaben (`grants_exec.rs:117-119`). Ohne Treffer: „no matching Exec grant; run `se share grants exec` to list choices“ (`grants_exec.rs:339`).

Validierung: vor der Bestätigung (`grants_exec.rs:129-131`). Die Regeln von `validate_for_this_host()` liegen außerhalb des Lesestands (Unbekannt).

Schrittzählung CLI:
- Volle Shell, eine Freigabe, Bestätigung mit `--yes`: 1 Befehl.
- Volle Shell, Bestätigung per Eingabe: 2 Eingaben (Befehl, `YES`).
- Mehrere deaktivierte Freigaben: zusätzlich `SELEKTOR` im Befehl.
- Mit Programmliste: 1 Befehl mit einer `--allow-program`-Angabe je Programm, plus `--yes` oder die Eingabe `YES`.
- Optionaler Vorab-Schritt: `se share grants exec`.

**CLI, deaktivieren:** `se share grants exec disable [SELEKTOR] [--yes]` (`grants_exec.rs:17-18`, `grants_exec.rs:43-54`). Auswahl nur aktivierter Freigaben (`grants_exec.rs:327`). Keine Bestätigung: die Abfrage erfolgt nur bei `enabled` (`grants_exec.rs:132`). `--yes` ist für `disable` zulässig und wirkungslos.

Wirkung auf laufende Befehle (nur Text belegt): „Changing an already enabled grant's account or programs ends that device's running commands.“ (`grants_exec.rs:29-30`). Die Umsetzung liegt außerhalb des Lesestands (Unbekannt).

**CLI, Gegenseite (Befehl auf dem Gerät, kein Grant):** `se exec TARGET -- PROGRAMM …` oder `se exec TARGET --shell BEFEHL` (`exec.rs:31-75`). Standards: `--timeout` 0 = unbegrenzt (`exec.rs:44-49`); `--max-output` 0 = unbegrenzt (`exec.rs:50-55`); Arbeitsordner: Home des Remote-Nutzers (`exec.rs:39-43`). Keine Bestätigung auf dem Client (`exec.rs:77-127`). `se shell TARGET` (`shell.rs:96-147`): `-e` Standard `~` (`shell.rs:138-146`), `--name` Standard `shell-<pid>` (`shell.rs:210-211`), `-T` erzwingt die Befehlszeile statt eines Terminals (`shell.rs:132-137`).

#### 1b Desktop (egui)

Einstieg (1 Schritt; das Öffnen des Fensters liegt außerhalb des Lesestands):
- A: Tab Befehle, Zeile „Einstellungen“ (`share_exec_ui.rs:64-66`). Der Aufrufer, der die Karte öffnet, liegt außerhalb des Lesestands (Unbekannt).
- B: Tab Geräte, „Details“ (`share_device_page_ui.rs:132-134`; für Freigaben ohne Kontakt `share_device_page_ui.rs:147-148`), danach der Abschnitt „BEFEHLE (EXEC)“ (`share_device_page_ui.rs:359-363`).

Aktivieren, Pfad „volle Shell, ein Gerät, Standardnutzer“ (3 Aktionen in der Karte):
1. „Remote-Ausfuehrung aktivieren …“ (`share_exec_ui.rs:281-289`). Aktiv nur bei `base_authorized` (`share_exec_ui.rs:280`).
2. Checkbox „Ich bestaetige diese uneingeschraenkte Codeausfuehrung bewusst“ bei leerer Liste; bei gesetzter Liste „Ich bestaetige diese Codeausfuehrung der gelisteten Programme bewusst“ (`share_exec_ui.rs:330-340`). Standard: aus (`share_exec_ui.rs:331-332`).
3. „Exec jetzt aktivieren“ (`share_exec_ui.rs:341-350`). Aktiv nur bei bestätigter Checkbox, `provider.available`, `base_authorized` (`share_exec_ui.rs:357-363`) und gültigen Einstellungen (`share_exec_ui.rs:323-325`).

Optional vor Schritt 2: Programmliste im Editor (`share_exec_programs_ui.rs:88-113`); Konto-Feld „Konto (leer = Standardnutzer):“ (`share_exec_ui.rs:226-239`, `share_exec_ui.rs:294-298`). Das Konto-Feld erscheint nur, wenn `exec_run_as_supported()` gilt (`share_exec_ui.rs:294`). Standard: leer.

Mindestpfad: 1 Einstieg + 3 Aktionen = 4 Klicks. Das Öffnen des Fensters ist nicht gezählt.

Anwendung: `apply_exec_grant` ruft `crate::daemon::mutate_exec_grant` auf (`share_exec_ui.rs:401`). Erfolg: Hinweis „Exec aktiviert; Policy-Revision … dauerhaft gespeichert und im Worker angewendet“ (`share_exec_ui.rs:404-410`). Sonst Fehlermeldung (`share_exec_ui.rs:413-425`).

Warnungen beim Aktivieren (`share_exec_ui.rs:474-490`): volle Shell „FULL {Konto} CODE EXECUTION“ (`share_exec_ui.rs:483-484`) oder „{Konto} EXECUTION OF {Programme}“ (`share_exec_ui.rs:486`). Erhöhter Zustand „REMOTE ROOT SHELL“ bzw. „REMOTE ADMINISTRATOR SHELL“ nur ohne benanntes Konto (`share_exec_ui.rs:458-469`, `share_exec_ui.rs:488`). Erklärtext (`share_exec_ui.rs:307-311`).

Deaktivieren: „Exec sofort deaktivieren“ (`share_exec_ui.rs:197-201`): ein Klick, keine Bestätigung.

Konto ändern (aktive Freigabe): Konto-Feld und „Konto uebernehmen“ (`share_exec_ui.rs:254-269`): ein Klick, keine Bestätigung. Hover-Text: „Laufende Befehle dieses Geraets enden wie bei jedem Rechtewechsel.“ (`share_exec_ui.rs:259`).

Liste ändern (aktive Freigabe): „Programmliste bearbeiten“ aufklappen (`share_exec_programs_ui.rs:138`), Einträge bearbeiten, Checkbox „Ich bestaetige diese Aenderung bewusst“ (`share_exec_programs_ui.rs:176`), dann „Programmliste uebernehmen“ (`share_exec_programs_ui.rs:185`). Unveränderte Liste: keine Aktion (`share_exec_programs_ui.rs:142-145`). Die Änderung behält das Konto (`share_exec_programs_ui.rs:164-167`).

Protokoll: „EXEC-PROTOKOLL DIESES HOSTS“, standardmäßig eingeklappt (`share_exec_log_ui.rs:21-25`); Auswahl 50, 200 und das Maximum (`share_exec_log_ui.rs:11`, `share_exec_log_ui.rs:52-62`); „Neu laden“ (`share_exec_log_ui.rs:64`).

Befehlsbetrieb: Jobs mit „Job abbrechen“ ohne Bestätigung (`share_exec_jobs_ui.rs:170-179`).

Befehlsdialog: In den Desktop-Dateien der Leseliste gibt es keinen Dialog zum Ausführen eines Befehls. Annahme: Desktop-Dateien außerhalb der Leseliste enthalten keinen solchen Dialog (nicht geprüft; Liste in „Grenzen der Lektüre“).

#### 1c Android (Kotlin)

Einstieg (nicht gelesen, nicht gezählt): Tab Teilen, Abschnitt „Befehle auf diesem Telefon“ (`ExecHostSection.kt:69`).

Erlauben, Pfad „volle Shell, ein Gerät“ (3 Aktionen):
1. „Erlauben…“ (`ExecHostSection.kt:145`). Aktiv nur bei `provider.available && target.baseAuthorized`.
2. Dialog „Befehle von „…“ erlauben?“ (`ExecHostSection.kt:168-209`) mit Checkbox „Ich habe verstanden“ (`ExecHostSection.kt:197-203`). Standard: aus (`ExecHostSection.kt:169`).
3. „Aktivieren“ (`ExecHostSection.kt:206`). Aktiv nur bei bestätigter Checkbox. Der Aufruf `actions.allow(target)` erfolgt in `ExecHostSection.kt:107-110`; sein Aufrufer liegt außerhalb des Lesestands (Unbekannt).

Der Dialog hat kein Programmfeld und kein Konto (`ExecHostSection.kt:166-209`).

Konto: nicht wählbar. Anzeige „Standardnutzer (…, Konto der App)“ (`ExecProgramsDialog.kt:51-57`). Die Bridge setzt bei jeder Programmliste `run_as` auf Standard (`mobile/share_exec.rs:132-135`). `api.md:605`: „Nicht auf Android: … Exec als anderes Konto.“

Deaktivieren: „Entziehen“ (`ExecHostSection.kt:143`), ohne Rückfrage (KDoc `ExecHostSection.kt:57`: „[Entziehen] without a question“).

Liste ändern (aktive Freigabe): Menü „⋮“ (`RowMenu`, Datei nicht gelesen) → „Programme…“ (`ExecHostSection.kt:147`) → `ExecProgramsDialog` (`ExecProgramsDialog.kt:69-143`). Eingabefeld mit „Hinzufügen“ oder Enter (`ExecProgramsDialog.kt:104-124`); „Volle Shell (Liste leeren)“ (`ExecProgramsDialog.kt:125`); Checkbox „Ich habe verstanden“ nur bei erlaubtem Gerät und geänderter Liste (`ExecProgramsDialog.kt:128`, `ExecProgramsDialog.kt:138`); „Übernehmen“, aktiv bei Änderung und (Gerät nicht erlaubt oder bestätigt) (`ExecProgramsDialog.kt:89-95`, `ExecProgramsDialog.kt:90`).

Befehlsprotokoll: „Befehlsprotokoll anzeigen“ (`ExecHostSection.kt:89`) öffnet `ExecAuditDialog`, das 200 Einträge lädt (`ExecAuditDialog.kt:31`, `ExecAuditDialog.kt:56-59`).

Laufenden eingehenden Befehl stoppen: „Stopp“ (`ExecHostSection.kt:159`), ohne Rückfrage.

Befehl auf dem Gerät der Gegenseite (kein Grant): `ExecDialog` (`ExecDialog.kt:67-170`). Felder: Befehl; Zeitlimit mit Auswahl 30 s, 60 s, 5 min, 15 min, Standard 60 s (`ExecDialog.kt:56`, `ExecDialog.kt:76`, `ExecDialog.kt:144-154`); Schalter „Über die Shell des Geräts“, Standard an (`ExecDialog.kt:75`, `ExecDialog.kt:156`). Keine Bestätigung im Dialog.

#### 1d Schritte im Vergleich

Zählung ohne Navigation außerhalb des Lesestands.

| Vorgang | CLI | Desktop | Android |
|---|---|---|---|
| Volle Shell aktivieren, Minimum | 1 Befehl mit `--yes` (mit `SELEKTOR`, falls mehrere deaktivierte Freigaben bestehen); ohne `--yes` 2 Eingaben | 4 Klicks (Einstieg + 3 Aktionen) | 3 Aktionen |
| Deaktivieren | 1 Befehl, ohne Bestätigung | 1 Klick, ohne Bestätigung | 1 Tipp, ohne Rückfrage |
| Konto ändern (aktiv) | `enable --run-as KONTO`, mit Bestätigung (`YES` oder `--yes`) | Konto-Feld + 1 Klick, ohne Bestätigung | nicht möglich |
| Liste ändern (aktiv) | `enable --allow-program …` mit vollständiger Liste, mit Bestätigung | Aufklappen, Einträge, Checkbox, Übernehmen (3 Aktionen + Einträge) | ⋮, Programme…, Einträge, Checkbox, Übernehmen (4 Aktionen + Einträge) |
| Protokoll | `se share exec log [--lines N]`, Standard 50 (`exec_status.rs:38-41`) | Aufklappen (1 Aktion), Auswahl optional, Standard 50 | 1 Tipp, lädt 200 |

### Frage 2: Programmliste

Ort der Bearbeitung je Oberfläche:
- CLI: nur über `enable` mit `--allow-program` und `--full` (`grants_exec.rs:66-77`). Die Unterbefehle sind nur `enable` und `disable` (`grants_exec.rs:10-19`).
- Desktop: Editor `list_editor` bei der Aktivierung (`share_exec_programs_ui.rs:61-127`, aufgerufen in `share_exec_ui.rs:293`) und „Programmliste bearbeiten“ bei aktiven Freigaben (`share_exec_programs_ui.rs:132-197`, aufgerufen in `share_exec_ui.rs:205`).
- Android: „Programme…“ im Menü der Gerätezeile (`ExecHostSection.kt:147`), `ExecProgramsDialog` (`ExecProgramsDialog.kt:69-143`), Bridge `share.setExec {programs}` (`mobile/share_exec.rs:124-140`; `api.md:579-580`).

Einträge und Validierung:
- CLI: Jeder Wert von `--allow-program` ist ein Eintrag (`grants_exec.rs:71`, `grants_exec.rs:100`). Der Code trimmt nicht (`grants_exec.rs:100`). Die Prüfung `validate_for_this_host()` läuft vor der Bestätigung (`grants_exec.rs:129-131`); eine Rückprüfung nach der Speicherung folgt (`grants_exec.rs:137-139`, `grants_exec.rs:186-207`). Die Regeln von `validate_for_this_host()` liegen außerhalb des Lesestands (Unbekannt).
- CLI-Hilfe: „A program is a bare name or an absolute path.“ (`grants_exec.rs:32-33`).
- Desktop: `add_program` trimmt (`share_exec_programs_ui.rs:39`). Leer: „Bitte einen Programmnamen oder absoluten Pfad eingeben.“ (`share_exec_programs_ui.rs:41`). Duplikat: „{program} steht schon in der Liste.“ (`share_exec_programs_ui.rs:43-45`). Einzelprüfung mit `ExecGrantSettings::validate()` (`share_exec_programs_ui.rs:46-50`), dokumentiert als „bare name or absolute path, no NUL, at most 4096 bytes“ (`share_exec_programs_ui.rs:35-37`). Vor dem Speichern: `validate_for_this_host()` (`share_exec_ui.rs:323-325`, `share_exec_programs_ui.rs:168-171`).
- Android: `programInputError` (`ExecProgramsDialog.kt:40-49`) prüft: leer; Duplikat; NUL oder mehr als 4096 Bytes („Ungültiger Programmname.“, `ExecProgramsDialog.kt:27`, `ExecProgramsDialog.kt:45`); „/“ im Namen ohne führenden „/“ („Nur ein Programmname oder ein absoluter Pfad (mit / am Anfang).“, `ExecProgramsDialog.kt:46`). Vor dem Speichern: `program_settings` → `validate_for_this_host()` → Fehler `invalid` mit „Programmliste ungültig: …“ (`mobile/share_exec.rs:136-138`).

`--full` bzw. volle Shell:
- CLI: `--full` (`grants_exec.rs:72-77`, `grants_exec.rs:95-96`).
- Desktop: „Volle Shell (Liste leeren)“ (`share_exec_programs_ui.rs:110-113`).
- Android: „Volle Shell (Liste leeren)“ (`ExecProgramsDialog.kt:125`). Die Bridge übernimmt `programs: []` als volle Shell (`mobile/share_exec.rs:124-140`; `api.md:579-580`).

Standardwahl beim Aktivieren:
- CLI: keine Liste, also volle Shell des Standardnutzers (`grants_exec.rs:21-25`).
- Desktop: Der Entwurf startet mit der gespeicherten Liste (`share_exec_programs_ui.rs:63-66`). Bei einer neuen Freigabe ist sie leer, also volle Shell (`share_exec_ui.rs:22`, `share_exec_ui.rs:293`). Konto: leer = Standardnutzer (`share_exec_ui.rs:226-231`). Bestätigungs-Checkbox aus (`share_exec_ui.rs:331-332`).
- Android: Der Erlauben-Dialog hat kein Listenfeld (`ExecHostSection.kt:166-209`). Annahme: Das Erlauben sendet keine Liste, denn `ShareExecApi.setExec(targetKey, enabled)` setzt `programs = null` (`ShareExecApi.kt:106`). Unbekannt: ob der Aufrufer von `actions.allow` eine Liste mitsendet. Der Listen-Editor startet mit der gespeicherten Liste (`ExecProgramsDialog.kt:71`). Standardwert des Zielmodells: leere Liste (`ShareExecApi.kt:43`).

Unterschiede der Oberflächen:
- Konto bei Listenänderung: CLI und Desktop behalten das Konto (`grants_exec.rs:90-91`, `grants_exec.rs:98`; `share_exec_programs_ui.rs:164-167`). Android setzt `run_as` bei jeder Programmliste auf Standard (`mobile/share_exec.rs:132-135`).
- Ergänzen: Die CLI ersetzt die Liste bei jeder Angabe vollständig (`grants_exec.rs:97-100`). Desktop und Android bearbeiten einen Entwurf (`share_exec_programs_ui.rs:61-66`; `ExecProgramsDialog.kt:71`, `ExecProgramsDialog.kt:81`).
- Trimmen: die CLI nicht (`grants_exec.rs:100`); Desktop (`share_exec_programs_ui.rs:39`) und Android (`mobile/share_exec.rs:130`) ja.

### Frage 3: Grant-Felder je Schnittstelle

Die Listen sind vollständig für die jeweils genannte Menge: alle Varianten von `IpcRequest` (20 Varianten, `ipc_protocol.rs:251-347`) und `IpcResponse` (17 Varianten, `ipc_protocol.rs:378-436`) sowie alle JSON-Ausgaben, Bridge-Methoden, CLI-Optionen, MCP-Werkzeuge und Vervollständigungen der gelesenen Dateien.

#### 3.1 Daemon-IPC (`ipc_protocol.rs`)

Mit Grant-Feldern:
- Anfrage `MutateExecGrant { token, target, enabled, settings }` (`ipc_protocol.rs:262-270`).
  - `target`: `ExecGrantTarget`. Variante Direct mit `device_id`, `public_key`, `fingerprint`, `node_id` (`grants_exec.rs:243-248`). Variante RoomMember mit `room_id`, `device_id`, `public_key`, `fingerprint`, `node_id` (`grants_exec.rs:259-265`).
  - `enabled`: bool (`ipc_protocol.rs:265`).
  - `settings`: optional, `ExecGrantSettings` mit `run_as` und `programs` (`ipc_protocol.rs:266-269`; `grants_exec.rs:102`). Doku: „absent keeps the grant's own (clients that predate run-as send none)“ (`ipc_protocol.rs:266-267`).
- Antwort `ExecGrantMutation { result }` (`ipc_protocol.rs:407-409`). Genutzte Felder: `persisted`, `applied`, `error`, `retry_state`, `revision` (`grants_exec.rs:146-158`; `share_exec_ui.rs:401-420`; `mobile/share_exec.rs:104-111`). Die Struktur liegt außerhalb des Lesestands (Unbekannt: JSON-Schlüssel).
- Antwort `ShareEvents { snapshot }` (`ipc_protocol.rs:398-400`). Das Snapshot `ShareWorkerSnapshot` trägt `profiles` (`ipc_protocol.rs:203-205`). Darin stehen die Freigaben mit ihren `exec`-Feldern (`grants.rs:121-127`; `grants_exec.rs:252-253`, `grants_exec.rs:269-270`). Außerdem trägt es `exec_grant_retry` (`ipc_protocol.rs:208-210`). Unbekannt: welche Anfrage diese Antwort liefert; die Zuordnung ist in den gelesenen Dateien nicht festgelegt.
- Antwort `ExecJobs { snapshot }` (`ipc_protocol.rs:413-415`). Die Jobansichten tragen `policy_revision` (`share_exec_jobs_ui.rs:149`; Testfixture `share_exec_jobs_ui.rs:352`).

Ohne Grant-Felder im Typ, Anfragen (vollständig, die übrigen 18 Varianten): Ping (`ipc_protocol.rs:252-254`), RefreshShare (`ipc_protocol.rs:255-257`), ShareCommand (`ipc_protocol.rs:258-261`; Inhalt `ShareCmd` nicht gelesen), DrainShareEvents (`ipc_protocol.rs:271-273`), ShareSnapshot (`ipc_protocol.rs:275-277`), AnalyzeShare (`ipc_protocol.rs:278-284`), OpenShare (`ipc_protocol.rs:285-288`), ProbeShareMount (`ipc_protocol.rs:289-293`), ExecShare (`ipc_protocol.rs:294-298`; `ExecRequest` nicht gelesen), ExecStream (`ipc_protocol.rs:299-303`; `ExecStart` mit `exec_id`, `command`, `cwd`, `env`, `timeout_ms`, `max_output_bytes`, `terminal`, `exec.rs:116-124`), ExecJobs (`ipc_protocol.rs:304-306`), CancelExec (`ipc_protocol.rs:307-310`; `ExecCancelTarget` mit `direction`, `exec_id`, `peer_device_id`, `exec_status.rs:122-127`), StartMount (`ipc_protocol.rs:311-314`), StopMount (`ipc_protocol.rs:315-318`), ListMounts (`ipc_protocol.rs:319-321`), RetryMount (`ipc_protocol.rs:322-325`), MountHostAttach (`ipc_protocol.rs:326-329`), MountHostBackend (`ipc_protocol.rs:330-333`), MountHostStatus (`ipc_protocol.rs:334-346`).

Ohne Grant-Felder im Typ, Antworten (vollständig, die übrigen 13 Varianten): Pong (`ipc_protocol.rs:379-386`), Ok (`ipc_protocol.rs:387`), RefreshOk (`ipc_protocol.rs:388-390`), OpenOk (`ipc_protocol.rs:391-394`), MountPathCapabilities (`ipc_protocol.rs:395-397`), ShareCommand mit `ShareCommandReply` (`ipc_protocol.rs:401-403`; `ShareCommandReply` in `ipc_protocol.rs:233-246` ohne Grant-Feld), ExecResult (`ipc_protocol.rs:404-406`; Typ außerhalb des Lesestands), ExecReady (`ipc_protocol.rs:410-412`), ExecCancelled (`ipc_protocol.rs:416-418`), Mount (`ipc_protocol.rs:419-421`), Mounts (`ipc_protocol.rs:422-424`), MountHostReady (`ipc_protocol.rs:425-431`), MountHostStop (`ipc_protocol.rs:432`), Err (`ipc_protocol.rs:433-435`).

#### 3.2 Android-Bridge (`mobile/share_exec.rs`, `ShareExecApi.kt`, `api.md`)

Exec-Methoden der Bridge (vollständig für die Methoden mit Exec-Bezug in `mobile/share_exec.rs`):
- `share.status`, Felder `execProvider` und `execTargets`. `execProvider` hat `available`, `provider`, `detail`, `userLabel`, `runAsSupported` (`mobile/share_exec.rs:30-39`; `api.md:453`). `execTargets[]` hat `targetKey`, `relation`, `roomId`, `roomName`, `deviceId`, `name`, `fingerprint`, `enabled`, `baseAuthorized`, `policyRevision`, `runAs`, `programs` (`mobile/share_exec.rs:49-69`; `api.md:454-456`). Die Aufrufstelle von `share.status` liegt außerhalb des Lesestands (`mobile/share_status.rs`, nicht gelesen).
- `share.setExec {targetKey, enabled, programs?}` → `{revision}` (`mobile/share_exec.rs:76-79`, `mobile/share_exec.rs:157-165`; `ShareExecApi.kt:113-120`, `ShareExecApi.kt:140-141`).
- `share.execJobs` → `{active, history}`. Jobfelder: `direction`, `execId`, `peerDeviceId`, `peerName`, `program`, `state`, `startedAt`, `finishedAt`, `exitCode`, `message` (`mobile/share_exec.rs:183-211`). Kein Revisionsfeld (`ShareExecApi.kt:46-63`).
- `share.cancelExecJob {direction, execId, peerDeviceId}` → `{}` (`mobile/share_exec.rs:213-223`, `mobile/share_exec.rs:225-239`). Ohne Grant-Feld.
- `share.execLog {lines}` → `{entries, max}`. Eintragsfelder: `at`, `event`, `title`, `device`, `deviceId`, `detail` (`mobile/share_exec.rs:142-153`; `ShareExecApi.kt:72-92`). Ohne Grant-Feld.

Weitere `share.*`-Methoden mit Bezug zu Grant-Feldern, nur laut `api.md` belegt (Code nicht gelesen):
- `share.setContactWrite {…, write}` (`api.md:543-545`): Feld `write`, kein Exec-Feld.
- `share.allowGrantAgain` (`api.md:546-548`): „Kein Exec-Opt-in.“
- `share.withdrawGrant` (`api.md:549-553`): „Exec-Entzug“ innerhalb derselben Transaktion.
- `share.setRoomPolicy` (`api.md:556-559`): „Exec wird nicht aktiviert.“
- `share.setRoomMember` (`api.md:560-563`): „Wiederzulassen aktiviert keine Befehle.“
- `share.resetContactTrust {contactId}` (`api.md:597`): „entzieht die Autorisierung, Exec aus“.
- `share.renewCode {}` (`api.md:604`): „Exec aus“.

Hinweise zur Bridge:
- `provider_json` enthält kein Feld `elevated` (`mobile/share_exec.rs:30-39`). In der CLI und am Desktop steuert `elevated` die Warnung (`grants_exec.rs:367-376`, `share_exec_ui.rs:458-469`).
- Der KDoc-Satz „Always `false` on Android: commands cannot run as another account.“ (`ShareExecApi.kt:21`) ist nicht am Code geprüft; die Funktion `exec_run_as_supported()` liegt außerhalb des Lesestands (Unbekannt).

#### 3.3 CLI-Optionen mit Grant-Feldern

Vollständig für die gelesenen CLI-Dateien:
- `se share grants exec enable`: `SELEKTOR` (`grants_exec.rs:44-48`); `--run-as USER` (Feld `run_as`, `grants_exec.rs:60-65`); `--allow-program PATH` (Feld `programs`, `grants_exec.rs:66-71`); `--full` (Feld `programs`, `grants_exec.rs:72-77`); `--yes` (Bestätigung, kein Grant-Feld, `grants_exec.rs:49-53`).
- `se share grants exec disable`: `SELEKTOR` (`grants_exec.rs:44-48`); `--yes` (ohne Wirkung, `grants_exec.rs:49-53`, `grants_exec.rs:132`).
- `se share grants exec` ohne Unterbefehl: keine Grant-Option; globales `--json` (`grants.rs:19-21`).
- `se share grants` (Liste): keine Exec-Option; globales `--json` (`grants.rs:19-21`).
- Ohne Grant-Felder: `se share exec list|show|cancel|history` (`exec_status.rs:14-21`, nur Selektoren), `se share exec log --lines N` (`exec_status.rs:38-41`), `se exec` (`exec.rs:31-75`), `se shell` (`shell.rs:96-147`).

#### 3.4 JSON-Felder mit Grant-Feldern

- `se share grants --json` (`grants.rs:103-131`):
  - Exec-Felder: `exec.enabled` (`grants.rs:122`), `exec.policy_revision` (`grants.rs:123`), `exec.changed_at` (`grants.rs:124`), `exec.run_as` (`grants.rs:125`), `exec.programs` (`grants.rs:126`).
  - Weitere Grant-Felder: `write` (`grants.rs:112`), `authorization.state` und `authorization.active` (`grants.rs:113-120`), `grant_state` (`grants.rs:110`).
  - Zielfelder: `device_id`, `fingerprint`, `public_key`, `node_id` (`grants.rs:104-109`).
- `se share grants exec --json` (`grants_exec.rs:280-290`): `target` (`grants_exec.rs:284`), `device_name`, `device_id`, `fingerprint` (`grants_exec.rs:284-285`), `enabled` (`grants_exec.rs:286`), `run_as` mit Standard `standard` (`grants_exec.rs:287`), `standard_user` (`grants_exec.rs:288`), `programs` (`grants_exec.rs:289`).
- `se share grants exec enable|disable --json` (`grants_exec.rs:140-144`): Serialisierung der Persistenz-Antwort. Schlüssel: Unbekannt (Struktur außerhalb des Lesestands).
- `se share exec list --json` (`exec_status.rs:203-220`): Felder `direction` und `job` (Serialisierung von `ExecJobView`). Annahme: `job` enthält `policy_revision`, weil die Textausgabe es ausgibt (`exec_status.rs:230`) und die Testfixture es setzt (`exec_status.rs:288`). Unbekannt: die serde-Attribute von `ExecJobView`.
- `se share exec log --json` (`exec_status.rs:100-111`): Logzeilen als JSON-Werte. Die Doku nennt je Eintrag: Gerät, Fingerprint, Art, Programm bzw. `<shell>`, Digest, Arbeitsordner, Konto, Ergebnis (`exec_status.rs:24-32`). Die Schlüssel stehen in der Log-Schreibstelle außerhalb des Lesestands (Unbekannt).
- Android-Bridge (`mobile/share_exec.rs`): `execProvider` mit 5 Schlüsseln (`mobile/share_exec.rs:30-39`); `execTargets[]` mit 12 Schlüsseln, darunter `policyRevision` (`mobile/share_exec.rs:63`), `runAs` (`mobile/share_exec.rs:65`) und `programs` (`mobile/share_exec.rs:67`); `{revision}` (`mobile/share_exec.rs:164`); `{entries, max}` (`mobile/share_exec.rs:152`); Jobs ohne Revision (`mobile/share_exec.rs:196-211`).

#### 3.5 MCP-Werkzeuge (`catalog.rs`, 18 Werkzeuge)

Mit Grant-Bezug:
- `se` (`catalog.rs:179-186`): Parameter `args` (`catalog.rs:182`) führt jeden `se`-Befehl aus. Damit erreichbar sind `--run-as`, `--allow-program`, `--full`, `--yes` und `SELEKTOR` aus 3.3. Parameter `stdin` (`catalog.rs:183`). Unbekannt: ob `stdin` die Abfrage „Type YES to enable:“ (`grants_exec.rs:377-384`) beantwortet; `native/src/cli/mcp/cli_tool.rs` ist nicht gelesen.
- `connections` (`catalog.rs:70-72`): Beschreibung nennt `exec: "ssh"` und `exec: "share"`; für Share-Peers „works once that device granted this one Exec“. Ausgabefelder: Unbekannt (`native/src/cli/mcp/status_tools.rs` nicht gelesen).
- `status` (`catalog.rs:67-69`): Beschreibung ohne Grant-Feld. Ausgabe: Unbekannt (`status_tools.rs` nicht gelesen).
- `exec` (`catalog.rs:140-155`): Eingabefelder ohne Grant-Feld (`target`, `command`, `argv`, `cwd`, `env`, `stdin`, `keep_stdin_open`, `timeout_seconds`, `session`, `attachable`, `wait_seconds`, `max_bytes`). Das interne Modell `ExecRequest` (`exec_tool.rs:11-19`) hat kein Grant-Feld.

Ohne Grant-Feld (die übrigen 13 Werkzeuge): `list` (`catalog.rs:73-79`), `stat` (`catalog.rs:80-82`), `read` (`catalog.rs:83-91`), `write` (`catalog.rs:92-100`), `edit` (`catalog.rs:101-116`), `mkdir` (`catalog.rs:117-119`), `remove` (`catalog.rs:120-126`), `copy` und `move` (`catalog.rs:127-128`, Schema `catalog.rs:206-221`), `search` (`catalog.rs:129-139`), `job_status` (`catalog.rs:156-165`), `job_input` (`catalog.rs:166-172`), `job_cancel` (`catalog.rs:173-178`), `sessions` (`catalog.rs:187-192`).

Jobergebnisse der MCP-Exec-Dateien, ohne Grant-Feld: `exec_share_job.rs:115-121` (`outcome`, `exit_code`, `signal`, `cli_exit_code`, `output_truncated`); Fehlerereignis `exec_share_job.rs:72-75` (`code`, `kind`); `exec_session_job.rs:208-215` (`session`, `exit_code`, `cwd`, `session_ended`, `message`, `cli_exit_code`); `exec_ssh_job.rs:112-116` (`exit_code`, `signal`, `cli_exit_code`).

#### 3.6 Vervollständigung (`completions.rs`)

- `exec_grant_candidates` (`completions.rs:68-118`): Wert `exec://direct/{device_id}/{fingerprint}` (`completions.rs:86`) bzw. `exec://room/{room_id}/{device_id}/{fingerprint}` (`completions.rs:104-106`). Hilfetext `{device_name} · direct · {Zustand}` (`completions.rs:87-91`) bzw. `{room.name} / {member.device_name} · room · {Zustand}` (`completions.rs:108-112`). Zustand aus `exec_state_label` (`completions.rs:122-131`): `enabled` oder `disabled`, dazu ` · as {Konto}` und ` · programs {Liste}`.
- `active_grant_candidates` (`completions.rs:46-66`): Hilfetext `{device_name} · fingerprint {fingerprint} · active`; ohne Exec-Feld.
- `exec_target_candidates` (`completions.rs:156-198`): Werte `@{connection}`, `share://direct/{id}`, `share://room/{room}/{device}`; ohne Grant-Feld.
- `exec_id_candidates` (`completions.rs:200-242`): ohne Grant-Feld.
- Zwei URI-Schemata für Exec-Ziele: `exec://` für Grant-Befehle (`grants_exec.rs:396-410`) und `share://` für Befehlsziele (`exec.rs:287`, `exec.rs:310`; `completions.rs:175`, `completions.rs:187`). Der Bridge-Schlüssel `targetKey` hat ein drittes Format: `direct/<deviceId>/<fingerprint>` bzw. `room/<roomId>/<deviceId>/<fingerprint>` (`share_exec_ui.rs:96-103`; `api.md:578`).

### Frage 4: Vervollständigung, Hilfe- und Fehltexte (wörtlich)

Scope: Hilfe- und Erklärtexte, Vervollständigungstexte, Fehl-, Hinweis- und Bestätigungstexte aus den gelesenen Dateien, die Exec-Freigaben, Konten, Programmlisten, Shell- oder Terminalrechte, Exec-Entzug oder Exec-Bestätigung nennen. Nicht aufgeführt, weil ohne Rechteaussage: `grants_exec.rs:45-46` (Selektor-Hilfe), `exec_status.rs:23` (`about` des Logs), `exec.rs:65`, `exec.rs:97-101`, `exec.rs:115`, `exec.rs:250`, `exec.rs:265`, `exec.rs:275-278`, `exec.rs:339`, `exec.rs:347`, `exec_status.rs:83`, `exec_status.rs:113`, `exec_status.rs:188`, `shell.rs:216`, `shell.rs:245`, `completions.rs:6-12`, `exec_tool.rs:57`, `mobile/share_exec.rs:219`, `mobile/share_exec.rs:229-232`, die Texte in `share_exec_jobs_ui.rs` (Jobs und Abbruch) und `share_device_page_ui.rs:207` (Autorisierungstext ohne Exec-Bezug). Labels gelten nur, wo sie eine Rechteaussage tragen.

#### 4.1 Hilfetexte CLI

- `grants.rs:34` (Befehl `allow`): „Deliberately reactivate a blocked or suspended grant; Exec stays disabled“
- `grants.rs:44` (Befehl `exec`): „Inspect, enable, or disable unrestricted per-device Exec grants“
- `grants_exec.rs:13` (`enable`): „Enable user-code execution for exactly one device“
- `grants_exec.rs:17` (`disable`): „Disable Exec and terminate that device's contained active commands“
- `grants_exec.rs:21-40` (`ENABLE_HELP`, `long_about` von `enable`), vollständig:

```
Enable user-code execution for exactly one device.

Without options the device gets the full shell of the Standardnutzer: the
account this Smart Explorer worker runs as, exactly as every grant made before
these options existed. On a Linux host whose worker runs as root,
--run-as USER runs the device's commands as that local account instead.
--allow-program PATH (repeatable) limits the device to starting exactly these
programs in argv mode: no shell command and no terminal. Changing an already
enabled grant's account or programs ends that device's running commands.

A program is a bare name or an absolute path. It is found only by that
absolute path or in the host's fixed system search path, never through a PATH
the device sends, and it starts only if it is the same file (under the same
name) as a listed program. The device may pass only LANG, LC_* and TERM as
environment variables; every other variable (for example PATH, LD_PRELOAD or
GIT_PAGER) refuses the start. Arguments stay free, so shells, env,
interpreters (for example python, perl, node) and programs that run commands
(for example find, git, ssh, xargs, awk, sudo) are equal to a full shell: do
not list them.
```

- `grants_exec.rs:50-52` (Option `--yes`): „Confirm the FULL USER CODE EXECUTION warning without a prompt“
- `grants_exec.rs:63` (Option `--run-as`): „Run this device's commands as USER (Linux root worker); `standard` is the worker's own account“
- `grants_exec.rs:69` (Option `--allow-program`): „Allow only this program (repeatable): argv mode, no shell, no terminal; free arguments, so no shells, interpreters or command runners“
- `grants_exec.rs:75` (Option `--full`): „Remove the program list again: full shell“
- `exec.rs:7-8` (`EXEC_HELP`, erster Absatz): „Run a command as the Smart Explorer user's unrestricted shell authority on one explicitly authorized Share peer. File access does not imply Exec access.“
- `exec_status.rs:24-32` (`long_about` von `log`), vollständig: „Show the last entries of this host's persistent Exec audit log: refused authentications and penalties, refused starts, starts and ends, each with device, fingerprint, mode, program or <shell>, digest of the whole start, working directory, account and result. Shell command text is never logged; text sent by the other device is cut to 256 characters. The log survives restarts. It has two parts in the app data folder, each keeping between 4 and 8 MiB (the previous file ends in .1): exec-audit.log holds refused starts, starts and ends of authorized devices; exec-audit-rejected.log holds refused authentications, penalized EndpointIds and devices without Exec permission, so strangers cannot push out what authorized devices did.“
- `shell.rs:33-35` (`SHELL_HELP`): „Open a remote shell on a saved SFTP remote (@label[:/dir], its sftp:// endpoint) or on a Share peer that granted this device Exec (name, id, share:// endpoint; omit it when exactly one peer is ready).“
- `shell.rs:68-70`: „--attachable lets the user's other se processes join this session. Only processes of the same OS user can join, and only while this shell runs. Without it nobody can join.“
- `shell.rs:73-76`: „A terminal session can only be joined from a terminal and not with -T. AI agents (the plugin's `sessions` tool) join only command sessions: to let an agent join your shell, start it with `se shell -T --attachable`.“
- `shell.rs:114` (Option `--attachable`): „Let the user's other se processes (se shell --attach, AI agents) join this session“

Hilfetexte MCP (`catalog.rs`, die Stellen mit Rechteaussage):
- `catalog.rs:22-23` (`INSTRUCTIONS`): „It also runs commands on saved SSH remotes and authorized Share peers (`exec`);“
- `catalog.rs:36-38` (`INSTRUCTIONS`): „Change Share trust (access requests, Exec grants and their program lists, device and room settings, discoverability) only when the user explicitly asks.“
- `catalog.rs:70-72` (Werkzeug `connections`): „`exec: \"share\"` for Share peers (works once that device granted this one Exec).“ (Quelltext mit Escape-Zeichen; der Text ist `exec: "share"`.)
- `catalog.rs:141` (Werkzeug `exec`): „or on a Share peer that granted this device Exec (peer name, id or share:// endpoint).“ und „full terminal sessions of `se shell` cannot be joined.“
- `catalog.rs:152` (Parameter `attachable`): „Only when the call opens a session: let the user's `se shell --attach` and other agents join it (default true).“
- `catalog.rs:188` (Werkzeug `sessions`): „`se shell` terminal sessions are listed with kind terminal and cannot be joined“ und „Only sessions of the same OS user are visible.“

#### 4.2 Vervollständigung

- `completions.rs:122-131` (Format `exec_state_label`): `enabled` oder `disabled`, dann ` · as {Konto}` und ` · programs {Liste}`. Testerwartung `completions.rs:410`: „enabled · as deploy · programs uptime,/usr/bin/df“.
- `completions.rs:87-91` (Hilfetext direkte Freigabe): „{device_name} · direct · {Zustand}“
- `completions.rs:108-112` (Hilfetext Raummitglied): „{room.name} / {member.device_name} · room · {Zustand}“

#### 4.3 Fehl-, Hinweis- und Bestätigungstexte CLI

- `grants_exec.rs:298`: „exec_grants\t0\taccept a direct request or join a room first“
- `grants_exec.rs:339`: „no matching Exec grant; run `se share grants exec` to list choices“
- `grants_exec.rs:341`: „multiple Exec grants match; choose one: {Liste der Selektoren}“
- `grants_exec.rs:200-204` (Rückprüfung nach dem Speichern): „the background worker did not store the requested account/program settings (a worker older than this `se` ignores them); finish the update so the worker restarts, then try again“
- `grants_exec.rs:163`: „Exec grant is fail-closed pending retry: persisted={}, applied={}, retry={:?}“
- `grants_exec.rs:360` (Warnung, volle Shell): „WARNING: enabling FULL {account} CODE EXECUTION for {label}.“
- `grants_exec.rs:363` (Warnung, Programmliste): „WARNING: enabling {account} execution of {Programme, durch Komma getrennt} for {label}.“
- `grants_exec.rs:369` (erhöhte Warnung, nur wenn `provider.elevated` und kein benanntes Konto, `grants_exec.rs:367`): „This is a REMOTE {ADMINISTRATOR|ROOT} SHELL with unrestricted authority.“ Das Wort ist `ADMINISTRATOR` bei `cfg!(windows)`, sonst `ROOT` (`grants_exec.rs:370-374`).
- `grants_exec.rs:377` (Eingabeaufforderung): „Type YES to enable: “
- `grants_exec.rs:385`: „Exec grant was not enabled“

#### 4.4 Desktop (`share_exec_ui.rs`, `share_exec_programs_ui.rs`, `share_exec_log_ui.rs`, `share_device_page_ui.rs`)

Karte und Übersicht:
- `share_exec_ui.rs:49`: „Keine exakten Direkt-Freigaben oder Raummitglieder vorhanden.“
- `share_exec_ui.rs:133` (Überschrift): „REMOTE-AUSFUEHRUNG PRO GERAET“
- `share_exec_ui.rs:138`: „Dateifreigaben erlauben niemals automatisch Codeausfuehrung. Exec gilt nur fuer die angezeigte, exakt gepinnte Geraeteidentitaet.“
- `share_exec_ui.rs:144`: „Lokale Shell: {user_label}“; `share_exec_ui.rs:145`: „Kapselung: {provider}“
- `share_exec_ui.rs:149`: „nicht verfuegbar — {detail}“
- `share_exec_ui.rs:173`: „AKTIVIERT — volle Remote-Codeausfuehrung erlaubt“
- `share_exec_ui.rs:174`: „AKTIVIERT — nur die gelisteten Programme, ohne Shell und Terminal“
- `share_exec_ui.rs:175`: „DEAKTIVIERT — keine Remote-Codeausfuehrung“
- `share_exec_ui.rs:179`: „Exec-Policy-Revision: {policy_revision}“
- `share_exec_ui.rs:186-187`: „Nur diese Programme (keine Shell, kein Terminal): {Liste}“
- `share_exec_ui.rs:194`: „Basisfreigabe ist inaktiv; Exec kann nicht aktiviert werden.“
- `share_exec_ui.rs:220`: „Standardnutzer ({user_label})“; `share_exec_ui.rs:221`: „Konto {name}“
- `share_exec_ui.rs:233`: „Konto (leer = Standardnutzer):“
- `share_exec_ui.rs:259` (Hover-Text „Konto uebernehmen“): „Laufende Befehle dieses Geraets enden wie bei jedem Rechtewechsel.“
- `share_exec_ui.rs:308`: „Dieses Geraet kann danach ohne Befehlsfilter beliebige Programme und Shell-Befehle mit den Rechten des gewaehlten Kontos starten (Standardnutzer: das Konto des Smart-Explorer-Prozesses). Eine Programmliste oben beschraenkt es auf einzelne Programme.“
- `share_exec_ui.rs:310`: „Dieses Geraet kann danach nur die gelisteten Programme mit freien Argumenten und den Rechten des gewaehlten Kontos starten, ohne Shell und Terminal; nur LANG, LC_* und TERM gelten als Umgebung. Enthaelt die Liste eine Shell, einen Interpreter oder ein Programm, das Befehle ausfuehrt, ist das eine volle Shell.“
- `share_exec_ui.rs:334`: „Ich bestaetige diese uneingeschraenkte Codeausfuehrung bewusst“
- `share_exec_ui.rs:336`: „Ich bestaetige diese Codeausfuehrung der gelisteten Programme bewusst“
- `share_exec_ui.rs:406`: „Exec {aktiviert|deaktiviert}; Policy-Revision {revision} dauerhaft gespeichert und im Worker angewendet“
- `share_exec_ui.rs:416-418` (unvollständige Änderung): „Exec-Aenderung nicht vollstaendig (gespeichert={}, angewendet={}): {detail}“; `detail` ist sonst „Worker-Anwendung steht noch aus“ (`share_exec_ui.rs:416`).
- `share_exec_ui.rs:424` (Fehler): „Exec-Aenderung fehlgeschlagen: {error}“
- `share_exec_ui.rs:460`: „FULL {user_label} CODE EXECUTION“; `share_exec_ui.rs:463`: „REMOTE ROOT SHELL“; `share_exec_ui.rs:465`: „REMOTE ADMINISTRATOR SHELL“ (Wortwahl nach `user_label` gleich „root“, `share_exec_ui.rs:462`).
- `share_exec_ui.rs:483-486`: „FULL {label} CODE EXECUTION“ (`share_exec_ui.rs:484`); „{label} EXECUTION OF {Programme}“ (`share_exec_ui.rs:486`).

Programmliste (`share_exec_programs_ui.rs`):
- `share_exec_programs_ui.rs:12` (Regeln, Überschrift „Regeln fuer Programmlisten“ in `share_exec_programs_ui.rs:118`), vollständig: „Ein Eintrag ist ein Programmname (gesucht nur im festen Systemsuchpfad dieses Hosts, nie im PATH des anderen Geraets) oder ein absoluter Pfad. Gestartet wird nur dieselbe Datei unter demselben Namen; ein Eintrag, den dieser Host nicht findet, erlaubt nichts. Das Geraet darf nur LANG, LC_* und TERM als Umgebung senden; jede andere Variable (zum Beispiel PATH, LD_PRELOAD oder GIT_PAGER) lehnt den Start ab. Die Argumente bleiben frei: Shells, env, Interpreter (zum Beispiel python, perl, node) und Programme, die Befehle ausfuehren (zum Beispiel find, git, ssh, xargs, awk, sudo), sind eine volle Shell und gehoeren nicht in die Liste. Keine Sandbox.“
- `share_exec_programs_ui.rs:41`: „Bitte einen Programmnamen oder absoluten Pfad eingeben.“
- `share_exec_programs_ui.rs:44`: „{program} steht schon in der Liste.“
- `share_exec_programs_ui.rs:73`: „Leer: volle Shell (beliebige Programme und Shell-Befehle, Terminal).“
- `share_exec_programs_ui.rs:156`: „Die leere Liste gibt diesem Geraet wieder die volle Shell.“
- `share_exec_programs_ui.rs:159`: „Dieses Geraet darf danach nur noch die gelisteten Programme starten, ohne Shell und Terminal.“
- `share_exec_programs_ui.rs:161`: „Dieses Geraet darf danach genau die gelisteten Programme starten.“
- `share_exec_programs_ui.rs:163`: „Laufende Befehle dieses Geraets enden wie bei jedem Rechtewechsel.“
- `share_exec_programs_ui.rs:176`: „Ich bestaetige diese Aenderung bewusst“

Protokoll und Geräteseite:
- `share_exec_log_ui.rs:31` (Beschreibung des Protokolls): „Abgelehnte Anmeldungen, abgelehnte Starts, Starts und Enden, mit Geraet, Fingerprint, Art, Programm, Konto und Ergebnis. Kein Text von Shell-Befehlen; Text des anderen Geraets ist auf 256 Zeichen gekuerzt. Dateien exec-audit.log und exec-audit-rejected.log im App-Datenordner (je 4 bis 8 MiB). Terminal: se share exec log.“
- `share_device_page_ui.rs:269` (Hover-Text „Vertrauen zuruecksetzen“): „Vergisst die gepinnte Identitaet dieses Geraets und entzieht seine Autorisierung (Exec aus); der Kontakt bleibt und muss neu bestaetigt werden.“
- `share_device_page_ui.rs:362` (Fallback ohne Exec-Karte): „Fuer dieses Geraet ist keine Ausfuehrung einstellbar.“

#### 4.5 Android (Kotlin)

Gerätezeilen und Erlauben-Dialog (`ExecHostSection.kt`):
- `ExecHostSection.kt:70` (Hinweiszeile): „Andere Geräte dürfen hier nur nach deiner ausdrücklichen Erlaubnis Befehle ausführen – je Gerät bzw. Raummitglied.“
- `ExecHostSection.kt:73` (Fehlerzeile): „Nicht verfügbar: {detail, sonst provider}“
- `ExecHostSection.kt:125`: „ · Befehle erlaubt“ oder „ · Keine Befehle“; `ExecHostSection.kt:126`: „ · Freigabe inaktiv“; `ExecHostSection.kt:127`: „Ausführen als: {…}“; `ExecHostSection.kt:128`: „Programme: {…}“
- `ExecHostSection.kt:173`: „Befehle von „{name}“ erlauben?“
- `ExecHostSection.kt:177-178` (ohne Programmliste): „„{name}“ kann danach ohne weitere Rückfrage beliebige Shell-Befehle auf diesem Telefon starten.“
- `ExecHostSection.kt:180-182` (mit Programmliste): „„{name}“ kann danach ohne weitere Rückfrage diese Programme mit beliebigen Argumenten starten, ohne Shell und Terminal: {Programme}. Enthält die Liste eine Shell oder einen Interpreter, ist das eine volle Shell.“
- `ExecHostSection.kt:186-188`: „Die Befehle laufen mit den Rechten von Smart Explorer: Sie können die Daten der App lesen – gespeicherte Verbindungspasswörter, die Share-Identität und Google-Drive-Tokens – und alle Dateien, auf die die App zugreifen darf.“
- `ExecHostSection.kt:191`: „Android kann Befehle im Hintergrund beenden oder anhalten; mit der App enden auch laufende Befehle.“
- `ExecHostSection.kt:193-194`: „Die Erlaubnis gilt nur für genau diese Geräte-Identität ({Beziehung}, Fingerabdruck {fingerprint}) und lässt sich jederzeit entziehen.“

Programmliste (`ExecProgramsDialog.kt`):
- `ExecProgramsDialog.kt:32-37` (Regeln), vollständig: „Ein Eintrag ist ein Programmname (gesucht nur im festen Systemsuchpfad dieses Telefons, nie im PATH des anderen Geräts) oder ein absoluter Pfad. Gestartet wird nur dieselbe Datei unter demselben Namen; ein Eintrag, den dieses Telefon nicht findet, erlaubt nichts. Das Gerät darf nur LANG, LC_* und TERM als Umgebung senden. Die Argumente bleiben frei: Shells, env, Interpreter (zum Beispiel sh, python) und Programme, die Befehle ausführen (zum Beispiel find, xargs, awk, su), sind eine volle Shell und gehören nicht in die Liste. Keine Sandbox.“
- `ExecProgramsDialog.kt:43`: „Bitte einen Programmnamen oder absoluten Pfad eingeben.“
- `ExecProgramsDialog.kt:44`: „{program} steht schon in der Liste.“
- `ExecProgramsDialog.kt:45`: „Ungültiger Programmname.“
- `ExecProgramsDialog.kt:46`: „Nur ein Programmname oder ein absoluter Pfad (mit / am Anfang).“
- `ExecProgramsDialog.kt:54`: „Standardnutzer ({userLabel}, Konto der App)“; `ExecProgramsDialog.kt:56`: „Konto {runAs} (auf Android nicht möglich)“
- `ExecProgramsDialog.kt:86`: „Programme für „{name}““
- `ExecProgramsDialog.kt:97`: „Leer: volle Shell (beliebige Programme und Shell-Befehle, Terminal).“ bzw. „Nur diese Programme, ohne Shell und Terminal:“
- `ExecProgramsDialog.kt:130-133` (nach Änderung, erlaubtes Gerät): „„{name}“ darf danach wieder beliebige Shell-Befehle auf diesem Telefon starten.“ bzw. „„{name}“ darf danach nur noch {Programme} starten.“
- `ExecProgramsDialog.kt:137`: „Laufende Befehle dieses Geräts enden.“
- `ExecProgramsDialog.kt:138` (Checkbox): „Ich habe verstanden“
- `ExecProgramsDialog.kt:140` (nicht erlaubtes Gerät): „Die Liste gilt, sobald du die Befehle dieses Geräts erlaubst.“

Protokoll (`ExecAuditDialog.kt`):
- `ExecAuditDialog.kt:70-71`: „Abgelehnte Anmeldungen und Starts, Starts und Enden von Befehlen anderer Geräte auf diesem Telefon. Shell-Befehle werden nie im Wortlaut gespeichert.“
- `ExecAuditDialog.kt:74`: „Protokoll nicht lesbar: {Fehler}“

Keine Rechtetexte: `ExecDialog.kt` (Befehl an ein anderes Gerät). Der Schalter „Über die Shell des Geräts“ (`ExecDialog.kt:156`) ist eine Befehlsoption ohne Rechteaussage. `ShareExecApi.kt` enthält nur KDoc.

#### 4.6 Bridge-Fehlertexte (`mobile/share_exec.rs`)

- `mobile/share_exec.rs:84` (`not_found`): „Gerät nicht gefunden oder seine Identität hat sich geändert – bitte neu laden.“
- `mobile/share_exec.rs:91` (`unsupported`): „Befehle auf diesem Telefon sind nicht verfügbar: {detail}“
- `mobile/share_exec.rs:100` (`conflict`): „Die Freigabe für dieses Gerät ist nicht aktiv; Befehle lassen sich nicht erlauben.“
- `mobile/share_exec.rs:105` (`internal`): „Befehlsfreigabe nicht geändert: {error}“
- `mobile/share_exec.rs:138` (`invalid`): „Programmliste ungültig: {error}“
- `mobile/share_exec.rs:168-171` (unvollständige Änderung): „Befehlsfreigabe nicht vollständig geändert (gespeichert: {ja|nein}, angewendet: {ja|nein}): {error oder „die Anwendung im Hintergrund-Dienst steht noch aus“}“

#### 4.7 Abweichungen zwischen den Texten

- „unrestricted“: `grants.rs:44` („unrestricted per-device Exec grants“) und `exec.rs:7` („unrestricted shell authority“) beschreiben jede Freigabe so. Die Programmliste schränkt sie ein (`grants_exec.rs:28-30`). `exec.rs:7` nennt außerdem „as the Smart Explorer user's“, obwohl `--run-as` ein Konto setzt (`grants_exec.rs:60-65`).
- `grants_exec.rs:50-52` (`--yes`): „Confirm the FULL USER CODE EXECUTION warning without a prompt“. `--yes` bestätigt jede Aktivierung (`grants_exec.rs:132-134`), auch die Warnung für eine Programmliste (`grants_exec.rs:362-364`).
- Regeln der Programmliste: Der Desktop-Text ist der CLI-Text (`grants_exec.rs:21-40`) plus „Keine Sandbox.“ (`share_exec_programs_ui.rs:12`). Der CLI-Text enthält „Keine Sandbox.“ nicht. Der Android-Text (`ExecProgramsDialog.kt:32-37`) lässt den Satz „jede andere Variable (zum Beispiel PATH, LD_PRELOAD oder GIT_PAGER) lehnt den Start ab“ weg. Er nennt die Beispiele „sh, python“ und „find, xargs, awk, su“; die CLI nennt „python, perl, node“ und „find, git, ssh, xargs, awk, sudo“.
- Erhöhte Warnung: Die CLI nennt „with unrestricted authority“ (`grants_exec.rs:369`); der Desktop-Text nennt diesen Zusatz nicht (`share_exec_ui.rs:463`, `share_exec_ui.rs:465`). Die Wortwahl folgt in der CLI `cfg!(windows)` (`grants_exec.rs:370-374`), am Desktop dem Vergleich von `user_label` mit „root“ (`share_exec_ui.rs:462`).
- Datenhinweis: Der Android-Text nennt die erreichbaren Daten (`ExecHostSection.kt:186-188`). Die CLI-Warnung (`grants_exec.rs:359-376`) und die Desktop-Erklärung (`share_exec_ui.rs:307-311`) nennen keine solchen Daten.
- Entzug: Die CLI-Hilfe von `disable` nennt das Beenden laufender Befehle (`grants_exec.rs:17`). Der Desktop-Schalter „Exec sofort deaktivieren“ hat keinen Hinweis (`share_exec_ui.rs:197-201`). „Entziehen“ auf Android hat keinen Text (`ExecHostSection.kt:143`).
- Konto ändern: Der Desktop-Hover nennt das Enden laufender Befehle (`share_exec_ui.rs:259`); die CLI-Hilfe nennt es ebenfalls (`grants_exec.rs:29-30`).

### Frage 5: Belege der Inventur am Code

Methode: Jeder Beleg aus `share-exec.md` mit einem Pfad der Leseliste ist an der angegebenen Zeile geprüft (Zeilen, Funktions- oder Variantenname). Belege auf Dateien außerhalb der Leseliste sind nicht geprüft; sie sind in „Grenzen der Lektüre“ vollständig aufgeführt. Die Bestand-Tabelle (Lesestand `7e7203c9`) wird gegen da2646d2 geprüft; Zeilenverschiebungen dort sind historisch, keine Fehler. Die Abweichungen sind in „Abweichungen der Inventur“ mit Nummer geführt.

#### 5a Oberflächen-Abschnitt (`share-exec.md` Zeilen 9-17)

| Zeile | Beleg | Ergebnis | Code-Beleg |
|---|---|---|---|
| 14 | Bridge `N/mobile/os/shared/domains/mod.rs:160-208` | nicht geprüft (Datei außerhalb) | keiner |
| 14 | Kotlin `A/api/Share*.kt` | teilweise geprüft: `ShareExecApi.kt` bestätigt; übrige Dateien nicht gelesen | `ShareExecApi.kt:13-142` |
| 15 | IPC `ipc_protocol.rs:262-310`: `MutateExecGrant`, `ExecShare`, `ExecStream`, `ExecJobs`, `CancelExec` | bestätigt | `ipc_protocol.rs:262-270`, `:294-298`, `:299-303`, `:304-306`, `:307-310` |
| 15 | IPC `ipc_protocol.rs:262-310`: `ShareCommand` | abweichend: Variante bei 258-261 (A1) | `ipc_protocol.rs:258-261` |
| 15 | IPC-Antworten `ipc_protocol.rs:404-416` | im Wesentlichen bestätigt; `ExecCancelled` endet bei 418 (A2) | `ipc_protocol.rs:404-418` |
| 16 | MCP-Werkzeuge `status`, `connections`, `exec`, `sessions`, `se` | unvollständig: Liste nennt 5 von 18 Werkzeugen (A4) | `catalog.rs:287-308` |
| 17 | Netz `exec_protocol.rs:82-94`, `exec_policy.rs:171-199`, `exec_program_path.rs:73-78` | nicht geprüft (Dateien außerhalb) | keiner |
| 100 | `catalog.rs:176-183` („Alle CLI-Einstellungen“) | abweichend: `se`-Werkzeug bei 179-186 (A3) | `catalog.rs:179-186` |
| 100 | „Beispiele im Katalog nennen Exec-Einstellungen nicht“ | präzisieren: das Beispiel bei 181 nennt zwei Exec-Lesebefehle, keine Einstellbefehle (A5) | `catalog.rs:181` |
| 100 | „Exec-Ziele“ `connections` nennt `exec: "share"` (`status_tools.rs:66-81`) | Beschreibung bestätigt (`catalog.rs:70-72`); Ausgabe nicht geprüft (Datei außerhalb) | `catalog.rs:70-72` |

#### 5b Bestand-Abschnitt Exec (`share-exec.md` Zeilen 25-33, Lesestand `7e7203c9`)

| Zeile | Beleg | Ergebnis | Code-Beleg (da2646d2) |
|---|---|---|---|
| 27 | `grants_exec.rs:277-317` | bestätigt | `grants_exec.rs:277-317` |
| 27 | `grants.rs:121-125`, `:149-162` | historisch: JSON-Block heute 121-127, Text heute 150-169 | `grants.rs:121-127`, `grants.rs:151-168` |
| 27 | `share_exec_ui.rs:74-133` | historisch: Funktionen heute 42-74 und 155-211 | `share_exec_ui.rs:42-74`, `share_exec_ui.rs:155-211` |
| 27 | `ExecHostSection.kt:104-132` | historisch: heute Bestätigungsdialog 101-113, `JobsTitle` 116-119, `ExecTargetRow` ab 122 | `ExecHostSection.kt:101-131` |
| 27 | `share_exec.rs:43-59` | historisch: heute `targets_json` 41-47, `target_json` 49-69 | `mobile/share_exec.rs:41-69` |
| 27 | `completions.rs:77-126` | historisch: heute 77-131 | `completions.rs:77-131` |
| 28 | `grants_exec.rs:16-18` | bestätigt | `grants_exec.rs:16`, `grants_exec.rs:18` |
| 28 | `share_exec_ui.rs:121` | historisch: Zeile heute leer | `share_exec_ui.rs:121` |
| 28 | `share_exec_ui.rs:193-263` | historisch: Aktivierung heute 271-355 | `share_exec_ui.rs:271-355` |
| 28 | `ExecHostSection.kt:124-128` | historisch: „Entziehen“ und „Erlauben…“ heute 143 und 145 | `ExecHostSection.kt:143`, `ExecHostSection.kt:145` |
| 28 | `share_exec.rs:63-105` (`share.setExec`) | historisch: heute 76-119 | `mobile/share_exec.rs:76-119` |
| 29 | `grants_exec.rs:61-65` (`--run-as`) | bestätigt | `grants_exec.rs:60-65` |
| 29 | `share_exec_ui.rs:125-127`, `:147-191` (Kontofeld) | historisch: heute `share_exec_ui.rs:202`, `:294-298` | `share_exec_ui.rs:202`, `share_exec_ui.rs:294-298` |
| 30 | `grants_exec.rs:66-77` und Hilfe `:21-40` | bestätigt | `grants_exec.rs:66-77`, `grants_exec.rs:21-40` |
| 30 | `share_exec_ui.rs:107-112` | historisch: `exec_summary` heute 106-111 | `share_exec_ui.rs:106-111` |
| 31 | `exec_status.rs:15-21` | bestätigt | `exec_status.rs:14-21` |
| 31 | `share_exec_jobs_ui.rs:24-96` | bestätigt (Funktion endet bei 97) | `share_exec_jobs_ui.rs:24-97` |
| 31 | `ExecHostSection.kt:76-85` (eingehende Jobs mit „Stopp“) | historisch: Zielliste heute 80-88, „Stopp“ heute 159 | `ExecHostSection.kt:80-88`, `ExecHostSection.kt:159` |
| 32 | `exec_status.rs:22-34`, `:100-120` | bestätigt | `exec_status.rs:22-34`, `exec_status.rs:100-120` |
| 33 | `ExecDialog.kt` („Befehl ausführen“) | bestätigt (Datei und Dialog vorhanden) | `ExecDialog.kt:67` |
| 100-103 | MCP-Belege `status_tools.rs:38-49`, `:66-81`; `exec_policy.rs:171-199` | nicht geprüft (Dateien außerhalb) | keiner |

#### 5c Abschnitt „Stand nach dem Stapel“ (`share-exec.md` Zeilen 105-158, ausgeliefert in v0.5.182)

| Zeile | Beleg | Ergebnis | Code-Beleg (da2646d2) |
|---|---|---|---|
| 113 | `grants.rs:121-127`, `:151-168` (JSON `exec.run_as`, `exec.programs`; Text `exec_run_as=`, `exec_programs=`) | bestätigt | `grants.rs:121-127`, `grants.rs:167-168` |
| 113 | `completions.rs:122` (`enabled · as KONTO · programs …`) | bestätigt | `completions.rs:122-131`; Testerwartung `completions.rs:410` |
| 113 | `share_exec_ui.rs:42-76` („Zeile je Gerät mit Zustand und Konto“) | abweichend: Funktion endet bei 74 (A7); Konto steht nur bei aktiven Geräten (A6) | `share_exec_ui.rs:42-74`, `share_exec_ui.rs:61-63` |
| 113 | `ExecHostSection.kt:127` („Ausführen als“, „Programme“) | bestätigt (127 und 128) | `ExecHostSection.kt:127-128` |
| 113 | `target_json` / `provider_json` (`mobile/share_exec.rs`) | bestätigt | `mobile/share_exec.rs:30-39`, `mobile/share_exec.rs:49-69` |
| 114 | `share_exec_programs_ui.rs:61`, `:132` (Editor, „Regeln“, Validierung, Bestätigung) | Belegstelle unvollständig: der Aufruf von `mutate_exec_grant` steht in `share_exec_ui.rs:401` und wird über `ExecAction` erreicht (A8) | `share_exec_programs_ui.rs:61`, `:132`; `share_exec_ui.rs:27-31`, `:89-90`, `:401` |
| 114 | „Regeln wie die CLI-Hilfe“ | präzisieren: der Desktop-Text enthält zusätzlich „Keine Sandbox.“ (A10) | `share_exec_programs_ui.rs:12` |
| 114 | „Validierung `ExecGrantSettings::validate`/`validate_for_this_host`“ | bestätigt | `share_exec_programs_ui.rs:46-50`, `:168-171`; `share_exec_ui.rs:323-325` |
| 114 | „Bestätigung je Änderung“ | bestätigt für die Programmliste; für eine Kontoänderung fehlt sie in der GUI (Befund, Frage 1) | `share_exec_programs_ui.rs:172-180`; `share_exec_ui.rs:254-269` |
| 114 | `share.setExec {programs}` → `ExecProgramsDialog.kt:69` | bestätigt | `mobile/share_exec.rs:124-140`; `ExecProgramsDialog.kt:69` |
| 115 | „Kontofeld (nur Linux-root-Worker)“ | nicht belegt: die Bedingung `exec_run_as_supported()` liegt außerhalb des Lesestands; die CLI-Hilfe nennt den Linux-root-Worker | `share_exec_ui.rs:202`, `:294`; `grants_exec.rs:26-27`, `grants_exec.rs:63` |
| 116 | `se share exec log` (`exec_status.rs:22-34`, `:100-120`) | bestätigt; die CLI liest mit `exec_audit_tail` | `exec_status.rs:100-102` |
| 116 | „Leser `N/share/os/shared/exec_audit_view.rs`“ | nicht geprüft (Datei außerhalb); die Leser der Oberflächen unterscheiden sich (A9) | `share_exec_log_ui.rs:178`; `mobile/share_exec.rs:150` |
| 116 | Desktop `share_exec_log_ui.rs:21` | bestätigt | `share_exec_log_ui.rs:21` |
| 116 | Android `ExecAuditDialog.kt:52` | bestätigt | `ExecAuditDialog.kt:52` |
| 127 | `share_device_page_ui.rs:160`, `:216` (Geräteseite: Verbindung, Rückfreigabe, Vertrauen) | bestätigt | `share_device_page_ui.rs:160`, `:216` |
| 117, 123-136 | übrige Belege (Raumseite, Gerätekarte, Listen, Papierkorb, Gemeinsamer Kern) | nicht geprüft (Dateien außerhalb der Leseliste) | keiner |
| 142-157 | Tabelle „Desktop-Bedienweg vorher → nachher“ | ohne Dateibelege; die Labels „C1“ bis „C3“ werden in `share-exec.md` Zeilen 1-158 nicht erklärt (Unbekannt) | keiner |

Zusammenfassung Frage 5: Alle Belege mit einem Pfad der Leseliste sind geprüft. Bestätigt sind die in 5a bis 5c mit „bestätigt“ markierten Belege. Abweichend sind A1 bis A10 (Abschnitt „Abweichungen der Inventur“). Nicht geprüft sind alle Belege auf Dateien außerhalb der Leseliste.

## Tabelle „Oberfläche × Grant-Feld“

Legende: „ja“ = das Feld steht im Typ oder in der Ausgabe der Oberfläche (Beleg in der letzten Spalte). „nein“ = das Feld steht dort nicht. „nicht gelesen“ = die Datei liegt außerhalb der Leseliste. Basisfreigabe = `baseAuthorized` (Bridge und Kotlin), „Basisfreigabe ist inaktiv“ bzw. „Freigabe inaktiv“ (Desktop und Android), `authorization` (CLI), `state` (IPC; Zuordnung zur Basisfreigabe ist Annahme). Zielidentität = Schlüssel oder Felder, die das Gerät bzw. Raummitglied eindeutig bezeichnen. „Name“ allein zählt nicht als Zielidentität.

| Oberfläche | enabled | Konto (`run_as`) | Programme | Revision | Basisfreigabe | changed_at | Zielidentität | Beleg |
|---|---|---|---|---|---|---|---|---|
| CLI `se share grants --json` | ja (`exec.enabled`) | ja (`exec.run_as`) | ja (`exec.programs`) | ja (`exec.policy_revision`) | ja (`authorization.active`) | ja (`exec.changed_at`) | ja (`device_id`, `fingerprint`, `public_key`, `node_id`) | `grants.rs:103-131` |
| CLI `se share grants` Text | ja (`exec=`) | ja (`exec_run_as=`) | ja (`exec_programs=`) | ja (`exec_revision=`) | ja (`authorization=`) | nein | ja (`device_id=`, `fingerprint=`) | `grants.rs:150-169` |
| CLI `se share grants exec` Text | ja (`state=`) | ja (`run_as=`) | ja (`programs=`) | nein | nein | nein | ja (`device_id=`, `fingerprint=`, `target=`) | `grants_exec.rs:298-313`, `grants_exec.rs:209-224` |
| CLI `se share grants exec --json` | ja (`enabled`) | ja (`run_as`, Standard `standard`) | ja (`programs`) | nein | nein | nein | ja (`target`, `device_id`, `fingerprint`) | `grants_exec.rs:280-290` |
| CLI `enable`/`disable`, Ausgabe | ja (`requested=`) | nein | nein | ja (`revision=`) | nein | nein | nein (nur Name im Feld `exec_grant`) | `grants_exec.rs:146-158` |
| CLI Eingaben `enable` | ja (Unterbefehl) | ja (`--run-as`) | ja (`--allow-program`, `--full`) | nein | nein | nein | ja (`SELEKTOR`) | `grants_exec.rs:42-78` |
| CLI `se share exec list` | nein | nein | nein (`program=` ist das gestartete Programm) | ja (`policy_revision=`) | nein | nein | nein | `exec_status.rs:225-232` |
| CLI `se share exec log` | nein | ja (Feld „Konto“ laut Doku) | nein (Feld „Programm“ ist das gestartete Programm) | nein | nein | nein | ja (Gerät und Fingerprint laut Doku) | `exec_status.rs:24-32`, `exec_status.rs:100-111` |
| Vervollständigung `exec://` | ja (`enabled`/`disabled`) | ja (` · as {Konto}`) | ja (` · programs {Liste}`) | nein | nein | nein | ja (`exec://direct/…`, `exec://room/…`) | `completions.rs:84-131` |
| Desktop, Befehle-Zeile | ja (`Befehle: …`) | ja (nur bei aktiven Geräten, `als …`) | nein (nur Anzahl) | nein | nein | nein | nein (nur Name) | `share_exec_ui.rs:51-68`, `share_exec_ui.rs:106-129` |
| Desktop, Gerätekarte | ja (Statuszeile) | ja (`Ausfuehren als`) | ja (Liste) | ja (`Exec-Policy-Revision`) | ja („Basisfreigabe ist inaktiv“) | nein | ja (Geräte-ID, Fingerprint) | `share_exec_ui.rs:155-211` |
| Desktop, Programmliste | nein | ja (unverändert übernommen) | ja (Entwurf) | intern (Bestätigungsschlüssel) | nein | nein | nein | `share_exec_programs_ui.rs:132-197`, `share_exec_ui.rs:376-383` |
| Desktop, Jobs | nein | nein | nein | ja (`Policy-Revision`) | nein | nein | nein | `share_exec_jobs_ui.rs:146-149` |
| Desktop, Geräteseite | nein | nein | nein | nein | ja („Freigabe inaktiv“); Schreibrecht `write` ist kein Exec-Feld | nein | nein | `share_device_page_ui.rs:335-339`, `share_device_page_ui.rs:357`, `share_device_page_ui.rs:57-69` |
| Android, Gerätezeile | ja (Text „Befehle erlaubt“) | ja (`Ausführen als`) | ja (`Programme`) | nein | ja (Text „Freigabe inaktiv“) | nein | nein (nur Name) | `ExecHostSection.kt:122-151` |
| Android, Erlauben-Dialog | nein (nur Bestätigung) | nein | nein | nein | nein | nein | ja (Name, Beziehung, Fingerabdruck) | `ExecHostSection.kt:168-209` |
| Android, Programme-Dialog | ja (steuert Checkbox und Übernehmen) | nein | ja | nein | nein | nein | nein | `ExecProgramsDialog.kt:69-143` |
| Android, Befehlsprotokoll | nein | nein | nein | nein | nein | nein | ja (`deviceId` im Modell) | `ExecAuditDialog.kt:52-88`, `ShareExecApi.kt:72-85` |
| Bridge `share.status` | ja (`enabled`) | ja (`runAs`) | ja (`programs`) | ja (`policyRevision`) | ja (`baseAuthorized`) | nein | ja (`targetKey`, `deviceId`, `fingerprint`) | `mobile/share_exec.rs:49-69`, `api.md:453-456` |
| Bridge `share.setExec`, Anfrage | ja (`enabled`) | nein (Bridge setzt Standard) | ja (`programs`, optional) | nein | nein | nein | ja (`targetKey`) | `mobile/share_exec.rs:76-79`, `mobile/share_exec.rs:124-140`, `mobile/share_exec.rs:132-135` |
| Bridge `share.setExec`, Antwort | nein | nein | nein | ja (`revision`) | nein | nein | nein | `mobile/share_exec.rs:157-165` |
| Bridge `share.execJobs` | nein | nein | nein | nein | nein | nein | nein (`peerDeviceId` ist Peer, kein Grant-Ziel) | `mobile/share_exec.rs:183-211` |
| Kotlin `ExecTarget` | ja (`enabled`) | ja (`runAs`) | ja (`programs`) | ja (`policyRevision`) | ja (`baseAuthorized`) | nein | ja (`targetKey`, `deviceId`, `fingerprint`) | `ShareExecApi.kt:25-44` |
| Kotlin `ExecJob` | nein | nein | nein | nein | nein | nein | nein | `ShareExecApi.kt:46-63` |
| IPC `MutateExecGrant`, Anfrage | ja (`enabled`) | ja (`settings.run_as`) | ja (`settings.programs`) | nein | nein | nein | ja (`target`) | `ipc_protocol.rs:262-270`, `grants_exec.rs:243-265` |
| IPC `ExecGrantMutation`, Antwort | ja (`persisted`, `applied`) | nein | nein | ja (`revision`) | nein | nein | nein | `ipc_protocol.rs:407-409`, `grants_exec.rs:146-158` |
| IPC `ShareEvents`, Snapshot | ja (in `profiles`) | ja (in `profiles`) | ja (in `profiles`) | ja (`policy_revision`) | ja (`state`, Annahme) | ja (`changed_at`) | ja (`device_id`, `public_key`, `fingerprint`, `node_id`) | `ipc_protocol.rs:199-227`, `ipc_protocol.rs:398-400`, `grants.rs:121-127` |
| IPC `ExecJobs`, Antwort | nein | nein | nein | ja (`policy_revision` je Job) | nein | nein | nein | `ipc_protocol.rs:413-415`, `share_exec_jobs_ui.rs:149` |
| MCP `se` (`args`) | ja (über Unterbefehl) | ja (über `--run-as`) | ja (über `--allow-program`, `--full`) | nein | nein | nein | ja (über `SELEKTOR`) | `catalog.rs:179-186` |
| MCP `connections`, `status` | Ausgabe unbekannt | Ausgabe unbekannt | Ausgabe unbekannt | Ausgabe unbekannt | Ausgabe unbekannt | Ausgabe unbekannt | Ausgabe unbekannt | `catalog.rs:67-72`; `status_tools.rs` nicht gelesen |
| MCP `exec` | nein | nein | nein | nein | nein | nein | nein | `catalog.rs:140-155`, `exec_tool.rs:11-19` |
| Netz (`exec_protocol.rs`) | nicht gelesen | nicht gelesen | nicht gelesen | nicht gelesen | nicht gelesen | nicht gelesen | nicht gelesen | keiner |

## Abweichungen der Inventur

Nummerierung A1 bis A10. Jede Abweichung nennt die Stelle in `share-exec.md`, den Code-Beleg und die Art. Die Zeilenbelege der Bestand-Tabelle (Lesestand `7e7203c9`) sind historisch und zählen hier nicht; ihre Einordnung steht in Frage 5, Tabelle 5b.

- **A1** (`share-exec.md` Zeile 15, IPC): Der Bereich „`ipc_protocol.rs:262-310`“ nennt `ShareCommand`. Die Variante steht bei `ipc_protocol.rs:258-261`, außerhalb des Bereichs. Art: falsche Zeilenangabe.
- **A2** (Zeile 15, IPC-Antworten): Der Bereich „`:404-416`“ endet vor der Variante `ExecCancelled`, die bei `ipc_protocol.rs:416-418` steht. Art: geringfügig, Bereich unvollständig.
- **A3** (Zeile 100, MCP): „`catalog.rs:176-183`“ für das Werkzeug `se`. Das Werkzeug `se` steht bei `catalog.rs:179-186`; die Zeilen 176 bis 178 gehören zu `job_cancel` (`catalog.rs:173-178`). Art: falsche Zeilenangabe.
- **A4** (Zeile 16, MCP): Die Zeile „Werkzeuge `status`, `connections`, `exec`, `sessions`, `se`“ nennt 5 von 18 Werkzeugen des Katalogs (Test `catalog.rs:287-308`). Nicht genannt: `list`, `stat`, `read`, `write`, `edit`, `mkdir`, `remove`, `copy`, `move`, `search`, `job_status`, `job_input`, `job_cancel`. Art: unvollständige Aufzählung; die Zeile sagt nicht „nur“.
- **A5** (Zeile 100, MCP): „Beispiele im Katalog nennen Exec-Einstellungen nicht.“ Das Beispiel in `catalog.rs:181` nennt `["share","grants","exec","--json"]` und `["share","exec","log","--json"]`. Beide sind Lesebefehle. Einstellbefehle (`enable`, `disable`) kommen in keinem Beispiel vor. Art: die Aussage gilt nur für Einstellbefehle; präzisieren.
- **A6** (Zeile 113, Stapel): „Zeile je Gerät mit Zustand und Konto“ (Desktop-Befehle-Seite). Das Konto erscheint nur bei aktiven Geräten (`share_exec_ui.rs:61-63`). Bei deaktivierten Geräten zeigt die Zeile „Befehle: aus“ (`share_exec_ui.rs:122-128`). Art: unvollständige Beschreibung.
- **A7** (Zeile 113, Stapel): Der Bereich „`share_exec_ui.rs:42-76`“ enthält die Funktion `ui_exec_overview` nur bis Zeile 74 (`share_exec_ui.rs:42-74`); Zeilen 75 und 76 sind Leerzeile und Dokumentation von `ui_exec_target`. Art: geringfügig.
- **A8** (Zeile 114, Stapel): Die Belegstellen `share_exec_programs_ui.rs:61` und `:132` enthalten keinen Aufruf von `crate::daemon::mutate_exec_grant`. Der Aufruf steht in `share_exec_ui.rs:401` (`apply_exec_grant`) und wird über den Typ `ExecAction` erreicht (`share_exec_ui.rs:27-31`, `share_exec_ui.rs:89-90`). Art: die Aussage stimmt in der Kette, der Beleg ist unvollständig.
- **A9** (Zeile 116, Stapel): Der Modulkommentar von `share_exec_log_ui.rs:1-4` sagt, die Ansicht zeige die Einträge „as `se share exec log` shows them“ und lese mit `crate::share::exec_audit_entries`. Die CLI liest aber mit `crate::share::exec_audit_tail` (`exec_status.rs:102`). Desktop (`share_exec_log_ui.rs:178`) und Android (`mobile/share_exec.rs:150`) lesen mit `exec_audit_entries`. Ob die Ausgabe gleich ist, ist nicht belegt (Unbekannt: Inhalt beider Funktionen liegt außerhalb). Standardanzahl: CLI 50 (`exec_status.rs:39-40`), Desktop erste Auswahl 50 (`share_exec_log_ui.rs:37`), Android 200 (`mobile/share_exec.rs:149`; `ExecAuditDialog.kt:31`). Art: nicht belegte Gleichheit.
- **A10** (Zeile 114, Stapel): „Regeln wie die CLI-Hilfe“ für die Programmliste am Desktop. Der Desktop-Text (`share_exec_programs_ui.rs:12`) enthält den Schlusssatz „Keine Sandbox.“, den die CLI-Hilfe (`grants_exec.rs:21-40`) nicht enthält. Art: Präzisierung. Die Android-Regeln weichen zusätzlich ab (Frage 4.7).

Gruppierung nach Zeilen von `share-exec.md`: A1 und A2 Zeile 15 (IPC). A4 Zeile 16 (MCP-Werkzeuge). A3 und A5 Zeile 100 (MCP-Einstellungen und Beispiele). A6 bis A10 Abschnitt „Stand nach dem Stapel“ (Zeilen 113, 114 und 116).

## Lücken und offene Punkte

Jede Lücke nennt die fehlende Tatsache. „Befund“ bedeutet: belegt, aber nicht bewertet.

1. **Terminal auf dem Telefon.** Unbekannt: ob Android eingehende Terminalsitzungen (`se shell`) annimmt und mit welchem Recht. Die Android-Freigabe zeigt nur Befehle (`ExecHostSection.kt:168-209`).
2. **Kontowechsel je OS.** Belegt: Linux mit Root-Worker (`grants_exec.rs:26-27`, `grants_exec.rs:63`); Android nur Standardnutzer (`mobile/share_exec.rs:132-135`, `api.md:605`). Unbelegt: Windows. Die CLI zeigt dort nur die Wortwahl ADMINISTRATOR (`grants_exec.rs:370-374`). Außerhalb des Lesestands liegen `validate_for_this_host()` (`grants_exec.rs:130`) und `exec_run_as_supported()` (`share_exec_ui.rs:202`, `share_exec_ui.rs:294`, `mobile/share_exec.rs:37`).
3. **Laufende Befehle bei Rechteänderung.** Belegt sind nur Texte (`grants_exec.rs:29-30`, `share_exec_ui.rs:259`, `share_exec_programs_ui.rs:163`, `ExecProgramsDialog.kt:137`, `ShareExecApi.kt:111`). Unbekannt: die Umsetzung im Daemon.
4. **MCP und Bestätigung.** Belegt: `se` mit `args` (`catalog.rs:179-186`) kann `--yes` übergeben. Unbekannt: ob `stdin` die Eingabe „YES“ liefert (`cli_tool.rs` nicht gelesen). Die einzige Schranke im Lesestand ist der Katalogtext „only when the user explicitly asks“ (`catalog.rs:36-38`).
5. **Konto-Änderung am Desktop ohne Bestätigungsschritt.** Befund: `share_exec_ui.rs:254-269`. Die CLI fragt bei jeder `enable`-Ausführung nach (`grants_exec.rs:132-134`).
6. **Entzug am Desktop.** Befund: „Exec sofort deaktivieren“ (`share_exec_ui.rs:197-201`) ohne Rückfrage und ohne Hinweis auf laufende Befehle.
7. **Entzug auf Android.** Befund: „Entziehen“ (`ExecHostSection.kt:143`) ohne Rückfrage und ohne Text.
8. **Erlauben auf Android.** Unbekannt: ob der Aufrufer von `actions.allow` (`ExecHostSection.kt:107-110`) eine Programmliste mitsendet.
9. **Räume.** Befund: CLI und Vervollständigung wählen nur Räume mit `auto_join` (`grants_exec.rs:256`, `completions.rs:97`). Unbekannt: ob `exec_target_views` (Desktop und Android) dasselbe tut; die Funktion liegt außerhalb.
10. **Job-JSON der CLI.** Annahme: `se share exec list --json` enthält `job.policy_revision` (`exec_status.rs:203-220`). Unbekannt: die serde-Attribute von `ExecJobView`.
11. **Revision je Oberfläche.** Befund: Die CLI-Freigabeliste (`grants_exec.rs:298-313`) und die Android-Gerätezeile (`ExecHostSection.kt:122-151`) zeigen keine Revision. Am Desktop steht sie in der Karte (`share_exec_ui.rs:179`) und bei den Jobs (`share_exec_jobs_ui.rs:149`). Android-Jobs haben kein Revisionsfeld (`mobile/share_exec.rs:196-211`).
12. **Elevated-Flag.** Befund: `elevated` fehlt in der Bridge (`mobile/share_exec.rs:30-39`), obwohl CLI und Desktop es für die Warnung nutzen (`grants_exec.rs:367`, `share_exec_ui.rs:458-469`).
13. **Drei Schlüsselschemata.** Befund: `exec://…` (`grants_exec.rs:396-410`), `share://…` (`exec.rs:287`, `exec.rs:310`) und `direct/…`, `room/…` für `targetKey` (`share_exec_ui.rs:96-103`).
14. **Netzprotokoll.** Unbekannt: ob eine Programmliste oder ein Konto an die Gegenseite übertragen wird. Die Inventur (Zeile 103) nennt `ExecHelloOk` ohne Einschränkung; `share/core/exec_protocol.rs` ist nicht gelesen.
15. **IPC-Antwort `ShareEvents`.** Unbekannt: welche Anfrage sie beantwortet (`ipc_protocol.rs:398-400`).
16. **IPC-Strukturen.** Unbekannt: Felder und JSON-Schlüssel von `ExecGrantPersistResult` (`ipc_protocol.rs:407-409`), `ExecRequest` (`ipc_protocol.rs:294-298`), `ShareCmd` (`ipc_protocol.rs:258-261`), `ExecResult` (`ipc_protocol.rs:404-406`).
17. **Ausgaben der MCP-Werkzeuge `status` und `connections`.** Unbekannt, weil `status_tools.rs` nicht gelesen ist (`catalog.rs:67-72`).
18. **Testtext „grant removed“.** Testfixture in `exec_share_job.rs:150-154`. Unbekannt: der Produktionstext.
19. **Option `shell` der Android-Befehlsausführung.** Nur Doku belegt (`api.md:568-571`: `shell:false` trennt an Leerraum). Unbekannt: Umsetzung in `ShareViewModel.startExec` (`ExecDialog.kt:110`).
20. **Maximum des Protokolls.** Nur Doku belegt (`api.md:593`: `max` = 1000). Unbekannt: `MAX_EXEC_AUDIT_VIEW` (`mobile/share_exec.rs:146`).
21. **Desktop-Befehlsübersicht.** Unbekannt: der Aufrufer von `ui_exec_overview` und das Ziel der Schaltfläche „Einstellungen“ (`share_exec_ui.rs:64-66`).
22. **Desktop-Befehlsdialog.** Annahme: keiner außerhalb der Leseliste. Nicht geprüft.
23. **Bedienweg-Tabelle der Inventur.** Befund: `share-exec.md` Zeilen 142-157 verwenden die Labels „C1“ bis „C3“, die im Dokument nicht erklärt sind (Unbekannt).
24. **Basisfreigabe.** Annahme: CLI `authorization`, IPC `state` und Bridge `baseAuthorized` bezeichnen dieselbe Bedingung. Nicht belegt.
25. **Konto-Ansicht am Desktop.** Befund: `run_as` erscheint nur bei aktiven Geräten (`share_exec_ui.rs:61-63`); siehe A6.

## Grenzen der Lektüre

Gelesen, vollständig: `plan.md`, `share-exec.md`, `grants_exec.rs`, `grants.rs`, `exec_status.rs`, `exec.rs`, `shell.rs`, `shell_terminal.rs`, `completions.rs`, `catalog.rs`, `exec_tool.rs`, `exec_session_job.rs`, `exec_share_job.rs`, `exec_ssh_job.rs`, `share_exec_ui.rs`, `share_exec_programs_ui.rs`, `share_exec_log_ui.rs`, `share_exec_jobs_ui.rs`, `share_device_page_ui.rs`, `ipc_protocol.rs`, `mobile/share_exec.rs`, `ExecHostSection.kt`, `ExecProgramsDialog.kt`, `ExecAuditDialog.kt`, `ExecDialog.kt`, `ShareExecApi.kt`.

Gelesen, teilweise: `api.md` nur Zeilen 436-460 und 540-629. Abschnitt 4.8 (Zeile 347) und die übrigen Abschnitte sind nicht gelesen.

Nicht gelesen, obwohl in `share-exec.md` genannt (Dateien außerhalb der Leseliste, 57 Einträge):
- Android (`android/app/src/main/java/app/smartexplorer/android/`): `service/ExecHostNotifier.kt`, `ui/settings/BackgroundSettings.kt`, `ui/settings/ShareServerSettings.kt`, `ui/share/ShareConnectDialog.kt`, `ui/share/ShareConnectionDialog.kt`, `ui/share/ShareDeviceCard.kt`, `ui/share/ShareLists.kt`, `ui/share/SharePeers.kt`, `ui/share/ShareScreen.kt`, `api/Share*.kt` (nur `ShareExecApi.kt` gelesen).
- Rust-Desktop (`native/src/app/core/`): `menus_settings.rs`, `quickshare_ui.rs`, `share.rs`, `share_connections_ui.rs`, `share_diagnostics_ui.rs`, `share_direct_ui.rs`, `share_discovery_publish_ui.rs`, `share_discovery_ui.rs`, `share_exports_ui.rs`, `share_host_trash_ui.rs`, `share_identity_rotation.rs`, `share_lan_ui.rs`, `share_lan_uplink_ui.rs`, `share_lifecycle_ui.rs`, `share_policy_ui.rs`, `share_removed_devices_ui.rs`, `share_room_page_ui.rs`, `share_rooms_ui.rs`, `share_this_device_ui.rs`, `share_window_ui.rs`.
- Rust-CLI (`native/src/cli/`): `connections.rs`, `share.rs`, `mcp/cli_tool.rs`, `mcp/status_tools.rs`, `share/devices.rs`, `share/discoverable.rs`, `share/exports.rs`, `share/exports_order.rs`, `share/exports_policy.rs`, `share/host_trash.rs`, `share/identity_command.rs`, `share/lan.rs`, `share/requests.rs`, `share/requests_policy.rs`, `share/rooms.rs`, `share/status.rs`, sowie die übrigen Dateien von `share/` außer den gelesenen (`grants.rs`, `grants_exec.rs`, `exec_status.rs`).
- Rust-Mobile (`native/src/mobile/os/shared/domains/`): `mod.rs`, `share_policy.rs`, `share_settings.rs`, `share_status.rs`.
- Rust-Share (`native/src/share/`): `core/exec_policy.rs`, `core/exec_program_path.rs`, `core/exec_protocol.rs`, `core/profile_settings.rs`, `core/signal_commands_local.rs`, `os/shared/exec_audit_view.rs`.
- Dokumente, nur als Inventur-Beleg genannt: `docs/QUICKSHARE.md`, `docs/TODO.md`, `docs/oberflaechen/INDEX.md`, `docs/plaene/2026-10-16-luecken-schliessen/plan.md`.

Ausgeführte Befehle (alle nur lesend): `git log -1 --oneline`; `git status --short -- native android docs`; `wc -l` auf den Dateien der Leseliste; `grep` mit Stichworten über die Dateien der Leseliste; `grep -l` über `native/src/cli/mcp/*.rs` (Lesezugriff auf alle Dateien dieses Ordners, Ausgabe nur Dateinamen: `catalog.rs`, `exec_share_job.rs`, `exec_tool.rs`, `status_tools.rs`); `grep -o` über `share-exec.md` (Pfadliste); `ls` und `wc` auf `docs/lesungen/` (Ausgabeordner).

Nicht getan: keine Builds, Tests, Server, Paketinstallationen, Commits oder Pushes (AGENTS.md). Die `#[cfg(test)]`-Blöcke sind gelesen, nicht ausgeführt. Keine Subagenten. Keine `arbeitsweise`-Werkzeuge (Abwahl durch den Nutzer). Keine `graphify`-Abfragen: der Auftrag begrenzt die Lektüre auf die Leseliste; die Regel in AGENTS.md, vor Codefragen `graphify query` zu verwenden, ist dadurch nicht eingehalten.

Annahme: Die Quelldateien wurden während der Lektüre nicht geändert. Nicht geprüft. Die Ausgabedatei wurde während der Arbeit mehrfach außen verändert (Anzeige des Werkzeugs); der Inhalt wurde vor jedem Anhängen über Zeilenzahl und Überschriften kontrolliert.

Keine Geräte- oder Nutzerdaten wurden gelesen oder ausgegeben.
