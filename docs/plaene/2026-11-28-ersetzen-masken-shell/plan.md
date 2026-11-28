# Ersetzen in Dateien, MCP für komplettes Coding, Maskenlisten, Shell-Werkzeug

Auftrag: Nutzernachricht vom 2026-11-28 („Neuer Weiterführungsauftrag“, fünf Punkte), wörtlich im
Journal der Arbeitsweise. Stand: Erstplan aus der Nachricht, noch ohne Bestandslektüre.

## Erstplan aus der Nachricht

### Beauftragte Ergebnisse (Quelle: Nachricht 2026-11-28)

1. **Ersetzen in Dateien:** „Gefundene Treffer/Stellen aus der Volltextsuche sollen auch direkt ersetzt
   werden können (Search & Replace in Dateien, inkl. MCP-Unterstützung).“
2. **MCP für komplettes Coding:** „Prüfe alle vorhandenen MCP-Tools und baue sie – soweit noch Lücken
   bestehen – so vollständig aus, dass sie allein für komplettes Coding ausreichen (Ordner-/Dateierkundung,
   Namens- und Volltextsuche, Ersetzen, Lesen/Editieren/Schreiben usw.).“
3. **Maskenlisten:** exakte Wörter/Volltext, Regex-Ausdrücke und komplexere Muster („\s, \n, \d/Zahlen“)
   werden durch Platzhalter ersetzt; durchgängig auf Datei-Inhalte (Lesen, Bearbeiten, Suchen, Ersetzen),
   Dateinamen, Ordnerlisten/Pfade (Dateierkundung, Namenssuche) und Shell-/Command-Ausgaben; in beide
   Richtungen (Platzhalter in Eingaben des Agenten werden zurückersetzt: Editieren/Schreiben, Pfade,
   Suchen/Ersetzen, Shell-Befehle); über den MCP anlegbar/erweiterbar, „sofern sie vom Nutzer nicht gesperrt
   wurden“; Nutzer kann sperren; Klartext-Einträge/Zuordnungen für den Agenten „NICHT lesbar“.
4. **Shell-Werkzeug im MCP/Plugin:** Shell-Befehle durch Smart Explorer statt der eigenen Shell des Agenten;
   Maskenlisten in beide Richtungen; „Befehle laufen unter den Privilegien des Smart Explorers“; erhöhte
   Ausführung; „Hat Smart Explorer selbst gerade keine erhöhten Rechte, muss Smart Explorer die Elevation
   beim Nutzer anfragen (nativer Elevation-Flow / Pop-up unter Windows bzw. entsprechender System-Dialog
   unter Linux)“.
5. **Arbeitsziel:** Das gesetzte Arbeitsziel (lokal und remote: „SSH/SFTP, Share usw.“) wirkt mit allen neuen
   und ausgebauten Werkzeugen, auch dem Shell-Werkzeug „direkt auf der verbundenen lokalen bzw. Remote-Shell
   im gesetzten Session-Root / Session-Pfad“; Maskenlisten greifen lokal und remote identisch.

6. **Plugin-Hook** (Ergänzung 2026-11-28): „Wenn eine Maskenliste auf einen Projektpfad oder Ordnerpfad
   angewendet ist, muss das Plugin einen Hook bereitstellen, der alle anderen Tool- und Shell-Zugriffe außerhalb
   des Smart-Explorer-MCP auf diese geschützten Pfade hart blockiert – egal welcher Art (z. B. cd, Shell/Bash,
   Lesen, Schreiben, Editieren, Grep/Suche, Ordnerzugriffe, alle sonstigen Zugriffe).“ Ziel: „Der Agent darf eine
   aktive Maskenliste niemals über normale native Tools oder Shell-Befehle umgehen können“.

### Grundfunktion (eigene Folgerung)

- Maskenlisten sind eine Übersetzungsschicht an der Grenze zwischen se und Agent: jede Ausgabe an den Agenten
  läuft durch „echt → Platzhalter“, jede Eingabe des Agenten durch „Platzhalter → echt“. Damit sie lokal und
  remote identisch greift, sitzt sie im se-Prozess, der den MCP bedient, nicht auf der Gegenstelle.
- Ein Platzhalter muss eindeutig auf genau einen echten Wert zurückführen, auch bei Regex-Mustern, die viele
  verschiedene echte Werte treffen (eine Zuordnungstabelle je Liste, nicht nur das Muster).
- Ersetzen mit Masken: Der Agent sucht und ersetzt mit Platzhaltern; se ersetzt in den echten Daten.

### Was erhalten bleibt (eigene Folgerung, vor Bestandslektüre)

- Die vorhandenen MCP-Werkzeuge (u. a. `exec`, `root`, `grep`, `read`, `write`, `edit`) behalten ihre
  Parameter und Antworten; neue Fähigkeiten kommen als neue Parameter oder neue Werkzeuge hinzu.
- Die Volltextsuche (Dialog „In Dateien suchen“, `se grep`, Android-Seite) bleibt; Ersetzen kommt dort hinein.

### Grenzen und Autorisierung

- Keine lokalen Builds/Tests (AGENTS.md); eine Task-Suite remote; ein Release am Ende.
- Keine Installationen auf ASUS und Shadow (Nutzer 2026-11-27). Keine Reviews, kein Workflow-Werkzeug,
  höchstens sieben Subagenten.

### Fehlende Angaben

- Durch Bestandslektüre klärbar: vorhandene MCP-Werkzeuge und Lücken; Exec-Weg (lokal, Ziele, `run_as`,
  vorhandene Elevation); Inhalts-Suchkern für Ersetzen; Ablage geschützter Daten (versiegelte Geheimnisse);
  Oberflächeninventar (`docs/oberflaechen/INDEX.md`).
- Nur vom Nutzer klärbar: keine bekannt.

Planungstiefe: 3 (neue Fähigkeiten mit neuen Vertrags-, Speicher- und Bedienentscheidungen; kein Umbau
bestehender Modulgrenzen).

## B — Zweitplan aus dem Bestand (gelesen 2026-11-28, Stand `e8277f52`)

### Belegter Bestand

| Bereich | Befund | Quelle |
|---|---|---|
| MCP-Katalog | 27 Werkzeuge: status, connections, root, list, stat, read, write, edit, mkdir, remove, copy, move, search, grep, exec, job_status, job_input, job_cancel, se, sessions, screen, screen_window, screen_input, tcp_forward, apps, rdp, login | `native/crates/cli/src/cli/mcp/catalog.rs:78-219` |
| Aufrufweg | `server.rs::dispatch` ruft je Werkzeug eine Funktion; Ergebnisse sind `CallToolResult` mit `structuredContent` (Codex zeigt nur dieses) und Text-JSON; Argumente kommen als `JsonObject` über `Args` | `mcp/server.rs:96-241`, `mcp/response.rs:11-37`, `mcp/args.rs` |
| Aufträge | lange Arbeit (copy, move, search, grep, exec, se) läuft als Job; Ausgabe in verschlüsselten Spools (AES-256-GCM, Schlüssel nur im Speicher), Seiten über `job_status` | `mcp/jobs.rs`, `mcp/spool.rs:1-10`, `mcp/job_view.rs:140` |
| exec | nur entfernt: gespeicherte SFTP-Gegenstelle (SSH) oder Share-Gerät mit Exec-Recht; ohne `target` auf dem SSH- oder Share-Arbeitsziel; **kein lokaler Befehl** | `mcp/exec_tool.rs:52-131`, `mcp/root_tool.rs:81-104` |
| Sitzungen | dauerhafte Shells über SSH- oder Share-Exec-Strom; `local_transport.rs` startet eine lokale Shell, ist aber nur Testtransport | `cli/src/cli/shell_session/local_transport.rs:1-4` |
| Arbeitsziel | `root` set/show/clear; relative Pfade aller Dateiwerkzeuge; Verbindung bleibt offen (`tab_cache.rs`); ohne Root nur absolute lokale Pfade | `mcp/root_tool.rs`, `mcp/targets.rs:1-11`, `mcp/root_path.rs` |
| Share-Arbeitsziel unterhalb der Freigabe | `exec` braucht `cwd`, weil das Gerät den Plattenpfad nicht nennt (TODO SUCHE-EXEC-SHARE-CWD) | `mcp/exec_tool.rs:137-155` |
| Schreiben | `write`/`edit` stagen in einer privaten Datei und veröffentlichen über die Baum-Pipeline der CLI mit `if_match` (etag) | `mcp/file_write.rs:157-225` |
| Backend-Schreibweg | `open_write_copy_stage` + `promote_staged` (Stufe neben dem Ziel, atomar ersetzen) im Trait aller Backends | `storage/src/vfs/core/core.rs:92-213` |
| Suche | Kern `ContentQuery` (Normal/Erweitert/Regex, Notepad++-Filter, Ordner-Regex), läuft lokal, im SSH-Agenten, auf dem Share-Host oder liest über das Backend; Oberflächen CLI `se grep`, MCP `grep`, Desktop-Dialog, Android-Seite | `docs/oberflaechen/dateisuche.md` |
| Rechteerhöhung | Windows: erhöhter Helfer über `ShellExecuteExW` „runas“ mit Named Pipe, gesperrtem Programmabbild und Broker (`local_access`); Linux: pkexec-Wege in Login/RDP/Uplink mit Auswertung von 126/127 | `core/src/local_access/os/windows/elevation.rs`, `pipe.rs`, `image_lock.rs`; `engine/src/login/os/linux_os/package_install.rs:353-428`, `engine/src/net/os/linux_os/uplink_polkit.rs` |
| Geheimnisse | Schlüsselbund/Dateispeicher `creds::set_secret`/`get_secret` (≤ 64 KiB je Eintrag, Linux-Datei versiegelt mit ChaCha20-Poly1305) | `core/src/creds/os/shared.rs:43-92`, `core/src/creds/core/sealed_record.rs` |
| Plugin | Claude Code: lokaler Marketplace mit `plugin.json` und `.mcp.json`, **keine Hooks**; Codex: `config.toml`-Eintrag und Skill | `cli/src/cli/plugin/files.rs:55-100`, `plugin/codex.rs` |

### Lücken gegenüber dem Auftrag (eigene Folgerung aus dem Bestand)

1. Kein Ersetzen in Dateien auf irgendeiner Oberfläche (`dateisuche.md`, Abschnitt „Lücken und Ausnahmen“).
2. MCP für komplettes Coding: es fehlen ein lokaler Befehl (heute nur entfernt), Pfad-Globs mit `**`
   und Sortierung nach Änderungszeit bei `search` (heute nur Namen), Beachten von `.gitignore` bei
   `grep`/`search`, Ersetzen über Dateien, Mehrdatei-Änderungen in einem Aufruf (Codex-Format
   `apply_patch`), `exec` unter einem Share-Arbeitsziel ohne `cwd`.
3. Keine Maskierung, keine Maskenlisten, kein Speicher dafür.
4. Kein Shell-Werkzeug für lokale Befehle, keine erhöhte Ausführung über den MCP.
5. Keine Sperre nativer Werkzeuge des Agenten-Hosts.

## Kreative Schleifen

### S1 — Vollsperre (r7, Nachrichten 2026-11-28 „Ergänzung … Plugin-Hook“ und „Weitere Ergänzung … Vollsperre“)

- Frage: Welcher Mechanismus verhindert zuverlässig, dass ein Agent auf Pfade mit aktiver Maskenliste
  anders als über den maskierten MCP zugreift?
- Kriterien:
  - K1: native Datei-Werkzeuge des Hosts (Lesen, Schreiben, Editieren, Suchen, Ordner) auf geschützten
    Pfaden werden abgelehnt (Nutzerwortlaut „egal welcher Art“).
  - K2: Shell-Befehle des Hosts erreichen geschützte Pfade nicht (cd, Lesen, Kopieren, Interpreter).
  - K3: Der Agent kann die Sperre nicht selbst abschalten (Konfiguration, Hook-Dateien, se-App-Daten,
    Maskenspeicher).
  - K4: Daten verlassen den Schutz nicht über den MCP selbst (copy/move aus dem Schutzbereich, Shell-Ausgabe).
  - K5: Die eigenen Programme des Nutzers bleiben unberührt (eigene Folgerung, Annahme: der Nutzer meint
    Zugriffe des Agenten, Grundlage „Der Agent darf … niemals … umgehen“).
- Möglichkeiten (Hypothesen, Befunde folgen aus `docs/refs/agenten-sperre-elevation-2026-11-28.md`):
  - M1: PreToolUse-Hooks der Hosts (Pfadprüfung, Ablehnen) – erfüllt K1; K2 nur durch Wortanalyse.
  - M2: Hooks plus Betriebssystem-Isolation jedes Shell-Befehls (Linux Mount-Namespace, Windows
    eingeschränktes Token mit Deny-ACE) über `updatedInput`.
  - M3: Host-eigene Sandbox und Permission-Regeln (Claude Code `sandbox`, `Read(...)`-deny), von se gesetzt.
  - M4: Verwaltete Einstellungen (managed settings) mit Adminrecht, damit K3 hält.
  - M5: Daten nur für ein Dienstkonto zugänglich (Besitz/ACL), se-Dienst als einziger Zugang – erfüllt
    K1–K3 auch gegen Prozesse desselben Benutzers, verletzt K5.
- Befund (`docs/refs/agenten-sperre-elevation-2026-11-28.md`):
  - Claude Code: `PreToolUse`-Hooks aus Plugin und Managed Settings laufen in Subagenten, für fremde
    MCP-Werkzeuge und Code Mode; `deny` gilt in jedem Modus; `onFailure: "block"` macht Startfehler und
    Timeout blockierend (§1.3–1.8). Nutzer-Settings `disableAllHooks` schalten Plugin-Hooks ab, Managed
    Hooks nicht; Mods können Plugin-Hooks übergehen, Managed `PreToolUse` nicht (§1.6). Bash-Befehlstext
    ist nur „best-effort“ prüfbar (§1.4); OS-Sandbox für Bash fehlt unter nativem Windows und startet
    unter Ubuntu 24.04 nur mit Root-AppArmor-Profil (§1.7, §3.1 beobachtet).
  - Codex: Hooks fail-open, Nutzerhooks erst nach Vertrauen in `/hooks`, Managed Hooks
    (`requirements.toml`) gelten als vertraut; `write_stdin` erreicht keinen Hook (§2.1–2.2).
  - Eine Sperre nur des Prozessbaums schützt nicht vor Kopien, die ein anderer erlaubter Weg (die
    se-Shell selbst) außerhalb des Schutzpfads ablegt (eigene Folgerung aus §3.4).
- Bewertung:
  - M1 allein erfüllt K2 nicht (Wortanalyse „best-effort“). M2 scheitert an der Verfügbarkeit
    (Ubuntu ohne Profil, Windows-Kompatibilität unbelegt) und an Kopien über die se-Shell (K4). M3 deckt
    Windows nicht und Codex nur mit Sandbox. M5 verletzt K5.
  - Gewählt (eigene Entscheidung, Grundlage Nutzer 2026-11-28 „die technische Umsetzung ist frei
    wählbar. Entscheidend ist allein, dass die Vollsperre zuverlässig funktioniert“):
    **Agenten-Vollsperre, solange eine aktive Maskenliste einen Schutzpfad hat.** Dann lehnen die Hooks
    jedes native Datei-, Shell-, Notebook-, LSP- und fremde MCP-Werkzeug ab, Webabrufe auf
    Loopback/private Adressen und `file:` ebenfalls; erlaubt bleiben die se-Werkzeuge und Werkzeuge ohne
    Datei- oder Prozesszugriff (Planung, Fragen, Subagenten, Websuche, öffentliche Webabrufe).
    Begründung: Ohne verlässliche OS-Isolation kann jede Shell den Schutzpfad erreichen und Kopien
    ablegen; nur wenn Agenten ausschließlich über se arbeiten und se **jede** Ausgabe mit **allen**
    aktiven Listen maskiert, bleibt kein Weg offen. Die se-Werkzeuge reichen dafür nach Teil 2 allein
    für komplettes Coding (Nutzer: „so vollständig ausbauen, dass sie allein für komplettes Coding
    ausreichen“).
  - Härtung K3: Hooks in Managed Settings (Claude Code `managed-settings`, Codex `requirements.toml`)
    mit einmaliger Rechteerhöhung; zusätzlich Plugin-/Nutzerhooks, damit die Sperre auch ohne
    Erhöhung greift; Claude-Code-Managed-Regeln `Read(//schutzpfad/**)`/`Edit(//…/**)` deny für
    @-Erwähnungen und IDE-Kontext (§1.9). se selbst verweigert Lesen und Schreiben seiner App-Daten
    und der Host-Konfiguration über seine Werkzeuge; die se-Shell weist Befehle ab, die diese Pfade oder
    schwächende `se`-Befehle nennen, und prüft nach jedem Befehl den Maskenspeicher (Wiederherstellung
    aus dem Speicher, Sitzung gesperrt). Hook entscheidet fail-closed: Maskenspeicher vorhanden, aber
    nicht lesbar → Sperre.
  - Verbleibend (wird im Bericht genannt): Codex-Shells, die vor dem Aktivieren offen waren
    (`write_stdin`), ein Codex-Hook, der abstürzt (fail-open), ein absichtlicher Abfluss über die
    se-Shell ins Internet mit anschließendem öffentlichen Webabruf, andere Agentenprogramme, die der
    Agent über die se-Shell startet (deren Ausgabe kommt maskiert zurück, ihr Modell sieht Klartext).
- Status: Ansatz gewählt; Ausarbeitung in Teil D.

### S2 — Maskenlisten (r3, r8)

- Frage: Platzhalterform, Zuordnung Platzhalter ↔ Wert bei Regex und Fuzzy, Treffer über Seitengrenzen,
  Toleranz ohne Über-Maskieren, Speicherung ohne Lesbarkeit für den Agenten.
- Kriterien: K1 jeder Treffer jeder Liste wird vor der Ausgabe ersetzt (auch über Seitengrenzen);
  K2 jeder ausgegebene Platzhalter wird in jeder Eingabe eindeutig auf genau seinen Wert zurückgeführt;
  K3 „Schwein Braten“, „Weinsbraten“, „Weinbraten“ werden je nach Toleranz für „Schweinsbraten“
  erkannt, unkenntlicher Müll nicht (Nutzerbeispiel); K4 kein Klartext über MCP/CLI lesbar; K5 Sperre durch
  den Nutzer verhindert Anlegen/Ändern über den MCP.
- Befund: `docs/refs/maskierung-fuzzy-ersetzen-2026-11-28.md` §2–3 (eine Schwelle trennt „Weinbraten“ nicht von
  „Schweinshaxe“, §2.4; Zwei-Budget-Regel trennt alle 20 nachgerechneten Fälle; Platzhalter brauchen ein
  schließendes Zeichen, §3.4; vorhandene Platzhalterform im Original muss maskiert werden, §3.6; Seitengrenzen
  und Zeilennummern, §3.6; Presidio/LiteLLM-Tabellenprinzip „gleicher Wert → gleicher Platzhalter“, §3.1/3.3).
- Entscheidungen (eigene Wahl mit Grund):
  - Platzhalter `⟦Bezeichnung-N⟧` (U+27E6/U+27E7), Abwandlung eines unscharf gefundenen Werts
    `⟦Bezeichnung-N.V⟧`; Bezeichnung wählt der Nutzer je Eintrag (Standard `Maske`), nur Buchstaben, Ziffern,
    `_`; N und V fortlaufend und dauerhaft in der Zuordnungstabelle der Liste. Grund: schließendes Zeichen
    (§3.4), in Text und Pfaden praktisch nicht vorkommend, ohne Shell-/Regex-Bedeutung, die die Rückersetzung
    stört (Rückersetzung ist wörtlich, §3.6).
  - Ein Treffer über k Zeilenenden trägt dieselben k Zeilenenden im Platzhalter (`⟦Adresse-1⏎⏎⟧`); dadurch
    bleiben Zeilennummern von `read`, `grep` und Kontextzeilen gleich (§3.6 „Zeilennummern“). Der Rückweg
    erkennt den Platzhalter mit oder ohne innere Zeilenenden.
  - Ein `⟦` im Originaltext wird als `⟦⟦` ausgegeben und als `⟦` zurückgeführt; ein unbekannter Platzhalter in
    einer Eingabe des Agenten wird abgelehnt (§3.6 „Platzhalter steht schon im Originaltext“).
  - Rückführung: `⟦X-N⟧` → Listenwert (Nutzer: „durch die echten Werte aus der Maskenliste“), `⟦X-N.V⟧` → die
    gefundene Form, damit unveränderte Stellen beim Zurückschreiben unverändert bleiben (§3.6 letzte Zeilen).
  - Überlappende Treffer werden zur Vereinigung zusammengefasst (kein Teilstück eines Werts bleibt sichtbar).
  - Unscharf: Faltung (Groß/Klein, Akzente, ä↔ae, ö↔oe, ü↔ue, ß↔ss), Trennzeichen (Leerraum, `-`, U+00AD,
    U+2010–2013, Zeilenende) nur innerhalb des Treffers frei, Treffer an Wortgrenzen des Originals (§2.2);
    Budget 1 Kürzung an den Wortenden, Budget 2 innere gewichtete Fehler, OCR-Paare (rn↔m, 0↔O, 1↔l↔I, ß↔B,
    e↔c, h↔b) zu 0,4, Vertauschung 1 (§2.4). Stufen je Eintrag: `aus` (wörtlich), `gering` (Kürzung 0,
    innen AUTO), `mittel` (Kürzung ⌊0,3·L⌋, innen AUTO), `hoch` (Kürzung ⌊0,35·L⌋, innen AUTO+1); dazu frei
    einstellbare Zahlen. Exakte Vorkommen (gefaltet) werden auch innerhalb von Komposita maskiert.
    Kandidaten über Aho-Corasick auf Stücke (Navarro, §2.1), Prüfung mit gewichteter DP; eigene Umsetzung mit
    `aho-corasick` statt `fuzzy-aho-corasick` (jung, ein Autor, nicht erprobt, §2.6).
  - Literal-Einträge standardmäßig ohne Groß/Klein-Unterscheidung, Regex nach Rusts `regex`-Syntax
    (mehrzeilig möglich).
  - Geltung: Jede aktive Liste maskiert **jede** Ausgabe an den Agenten (alle Werkzeuge, alle Ziele), weil
    Daten über die Shell aus dem Schutzpfad kopiert werden können (S1). Ein Schutzpfad einer aktiven Liste
    löst die Agenten-Vollsperre aus (S1).
  - Speicher: `<App-Daten>/masks/lists.sealed` (ChaCha20-Poly1305, Schlüssel im Geheimnisspeicher von se,
    `creds::set_secret`), daneben `state.json` ohne Geheimnisse (Generation, Vollsperre ja/nein) für den Hook;
    Sperre fail-closed. Keine Oberfläche außer der Desktop-GUI zeigt Klartext (dort nur auf Klick);
    Entsperren, Deaktivieren, Löschen von Listen, Einträgen und Schutzpfaden verlangen die Bestätigung des
    Nutzers über den Systemdialog der Rechteerhöhung (UAC/polkit).
- Status: erreicht (Entwurf); Nachweis durch die Task-Suite (Teil D, M3).

## C — Entscheidungen der übrigen Teile

| Frage | Gewählt | Verworfen und Grund | Beleg |
|---|---|---|---|
| Ersetzungssyntax | Notepad++-treu: Normal wörtlich, Erweitert mit Escape-Umwandlung auch im Ersatztext, Regex mit Boost-`format_all` (`$&`/`$0`/`${n}`/`\n`/`$+{name}`/`` $` ``/`$'`/`$+`/`$$`, `\U \L \E \u \l`, Bedingungen `?n…:…`), eigene Auswertung | `fancy_regex::Expander`, `grep_matcher::interpolate`: kennen Groß/Klein und Bedingungen nicht | Ref. maskierung §1.1 |
| Wo ersetzt wird | Suche wie bisher dort, wo die Dateien liegen; Schreiben je Datei über das Backend des Ziels (Stufe + `promote_staged`, Zustand vor dem Veröffentlichen geprüft) | Ersetzen auf SSH-Agent/Share-Host: neues Protokoll ohne Vorteil bei den Rechten (Share-Host prüft Schreibrechte ohnehin beim Schreiben über `PeerBackend`) | `storage/src/vfs/core/core.rs:92-213` |
| Auswahl | Alle Treffer oder ausgewählte Treffer/Dateien aus dem Ergebnis (wie VS Code „Replace“/„Dismiss“), nur in Dateien, die seit der Suche unverändert sind | Notepad++ kennt keine Auswahl (Ref. §1.6); der Nutzer verlangt „Gefundene Treffer/Stellen … direkt ersetzt“ | Nachricht 2026-11-28 |
| Bestätigung | Desktop/Android: Dialog mit Ordner, Filter, Zahl der Stellen und Dateien, vorgewählt Abbrechen (Notepad++ + VS Code); CLI fragt im Terminal, ohne Terminal `--yes`; MCP mit `dry_run` als Vorschau | ohne Bestätigung | Ref. §1.2, §1.7 |
| Sicherung | Vor dem Überschreiben verschlüsselte Sicherung des Originals (Rückgängig je Lauf über alle Oberflächen); scheitert die Sicherung, bleibt die Datei unverändert und wird gemeldet | Notepad++ sichert nicht (Ref. §1.4) – AGENTS.md verlangt Umkehrbarkeit bei Überschreiben | AGENTS.md „native Rust architecture“ |
| Kodierung | BOM, UTF-8, ANSI bleiben; nicht-ANSI-Ersatz in ANSI-Datei → Datei übersprungen mit Grund; Zeilenenden unverändert außer im Ersatztext | – | Ref. §1.3 |
| Schreibschutz, Binär, zu groß | übersprungen mit Grund (Notepad++ still, se nennt den Grund) | stilles Überspringen | Ref. §1.3 |
| Shell-Werkzeug | neues MCP-Werkzeug `shell`: ohne Arbeitsziel lokal, sonst auf dem Gerät/Server des Arbeitsziels im Ordner des Ziels; `exec` bleibt für ausdrückliche entfernte Ziele | `exec` umbauen: ändert bestehendes Verhalten | Nachricht 2026-11-28 Punkt 4/5 |
| Erhöhung lokal | Windows: erhöhter se-Helfer über `ShellExecuteExW` „runas“, Rückkanal Named Pipe mit expliziter ACL (nur Nutzer-SID, NETWORK verweigert), `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`, Gegenstelle über `GetNamedPipeClientProcessId`/Server-PID und SHA-256 des Programmabbilds; Linux: pkexec mit Sitzungsagent (126 abgelehnt, 127 kein Agent/Fehler), ohne Agent `sudo -n`, sonst Fehler mit genauer Ursache; je Befehl ein Dialog, eine erhöhte `session` fragt einmal | gsudo-Zwischenspeicher (stille Erhöhung durch andere Prozesse, Ref. sperre §4.1) | Ref. sperre §4 |
| Erhöhung entfernt | Share-Gerät: das se des Geräts fragt dort per UAC/polkit (neues Feld im Exec-Start); SSH: `sudo -n`, braucht sudo ein Passwort, fragt se den Nutzer lokal in einem eigenen Passwortdialog (Windows CredUI, Linux Fenster der App) und gibt es über `sudo -S` weiter, ohne es zu speichern | Passwort im Chat (Projektregel verbietet) | catalog.rs:40-42 |
| `exec`/`shell` unter Share-Arbeitsziel | Exec-Start trägt den Share-Pfad; das Gerät übersetzt ihn innerhalb der Freigabe in seinen Plattenpfad (schließt TODO SUCHE-EXEC-SHARE-CWD) | `cwd` vom Agenten verlangen (heute) | `mcp/exec_tool.rs:137-155` |
| Coding-Lücken | `search`: Pfad-Globs mit `/` und `**`, Sortierung `mtime`, `gitignore`; `grep`: `gitignore` (`.gitignore`, `.ignore`, `.git/info/exclude` wie ripgrep über die Crate `ignore`); neues `apply_patch` (Codex-Patchformat, mehrere Dateien, Hinzufügen/Löschen/Verschieben) | eigene Ignore-Logik | Ref. sperre §2.2 (Codex nutzt `apply_patch`) |
| Ignorierdateien entfernt | Feld in `ContentQuery`; SSH-Agent ab Protokoll 14, Share-Host mit Fähigkeit `content_search_ignore_v1`; ältere Gegenstellen: se liest über das Backend und filtert selbst | ältere Gegenstellen ablehnen | `storage/src/agent_proto/core/content_query.rs` |

## D — Arbeitsplan

### Meilensteine mit prüfbarem Ergebnis

| M | Ergebnis | Prüfung in der Task-Suite |
|---|---|---|
| M1 Maskenkern | `engine::masks`: Listen, Einträge (wörtlich, Regex, unscharf mit Stufe/Zahlen), Schutzpfade, Sperre, verschlüsselter Speicher, Zuordnungstabelle, `mask`/`unmask` für Text und Pfade, Strom-Maskierer, Zustand für den Hook | Tabelle Ref. maskierung §2.4 (20 Fälle), Rundlauf `unmask(mask(x)) == x`, `⟦`-Maskierung, unbekannter Platzhalter abgelehnt, Zeilenzahl erhalten, Strom über Puffergrenzen, Speicher ohne Klartext auf der Platte |
| M2 Ersetzen-Kern | Boost-Ersetzung, Erweitert, Kodierung, Auswahl, Sicherung/Rückgängig, Lauf über Backends mit Zustandsprüfung, `se replace` | Ersetzungssyntax-Fälle aus Ref. §1.1, ANSI/BOM, geänderte Datei übersprungen, Rückgängig, CLI-Rückgabewerte |
| M3 Ersetzen Oberflächen | Desktop-Dialog/Ergebnisbereich, Android-Seite, Brücke | GUI-Harness, Kotlin-Batch, Brückentest |
| M4 MCP-Maskenschicht | jede Ausgabe maskiert, jede Eingabe rückgeführt, Werkzeug `masks` (nur anlegen/erweitern, nie Klartext), se-App-Daten und Host-Konfiguration über se-Werkzeuge gesperrt, Bild-Ausgaben bei aktiven Listen verweigert | MCP-Tests je Werkzeug mit Liste |
| M5 MCP-Coding | `replace`, `apply_patch`, `search` Globs/mtime/gitignore, `grep` gitignore, Protokoll 14 | MCP-Tests, Agent-Protokolltests, Share-Wire-Tests |
| M6 Shell und Erhöhung | `shell` lokal/Arbeitsziel, lokale Sitzungen, Erhöhung Windows/Linux, Share-Exec erhöht und Share-Pfad, SSH sudo, Masken in Befehl und Ausgabe, Integritätsprüfung | Tests der Pipe-Prüfung, pkexec-Ausgänge, `sudo -n` auf dem CI-Runner, Share-Wire |
| M7 Vollsperre und Verwaltung | `se hook` (Claude Code, Codex), Plugin-Hooks, Managed-Einrichtung mit Erhöhung, Regeln für Schutzpfade, `se masks`, Desktop-Seite „Maskenlisten“, Bestätigung schwächender Änderungen | Hook-Tests mit Eingabe-JSON beider Hosts, Plugin-Dateien, CLI, GUI-Harness |
| M8 Doku, Register, Suite | Oberflächen-Dokumente, Register, Suite | `native/test-register.py`, Suite grün |
| M9 Release | v0.5.209 | `release-check.sh` |

### Erhaltene Verträge

- Bestehende MCP-Werkzeuge behalten Namen, Parameter und Antwortfelder; neue Felder kommen hinzu. Ohne aktive
  Maskenliste ist jede Ausgabe byte-gleich zu heute.
- `ContentQuery` bekommt ein Feld mit Standard `false`; ältere SSH-Agenten (Protokoll 13) und Share-Hosts ohne
  Fähigkeit bekommen es nie.
- Exec-Protokoll: neue Felder nur, wenn die Gegenstelle sie ankündigt.

### Verträge zwischen den Paketen (vom Hauptagenten vorgegeben)

Schon angelegt (Commit des Plans): CLI-Befehle `Replace(replace::ReplaceArgs)`, `Masks(masks_cli::MasksArgs)`,
`Hook(hook::HookArgs)` (versteckt), `Elevated(elevated::ElevatedArgs)` (versteckt) in
`native/crates/cli/src/cli/mod.rs` mit `run(args) -> Result<i32, String>` je Modul; `pub mod masks;` und
`pub mod elevate;` (nicht Android) in `native/crates/engine/src/lib.rs`; Feld `ContentQuery::respect_ignore_files`
(Standard `false`); Abhängigkeiten `aho-corasick`, `ignore` (Workspace), `regex`/`aho-corasick` in engine,
`ignore` in storage.

1. **`engine::masks`** (Paket P1):
   - `MaskStore::open_default() -> Result<MaskStore, MaskError>` (App-Daten des Nutzers), `MaskStore::open_at(dir,
     key)` für Tests.
   - Verwaltung mit `Actor` (`Agent`, `Cli`, `Gui`, `VerifiedUser`): `lists() -> Vec<ListSummary>` (ohne Werte),
     `create_list`, `add_entries -> Vec<EntrySummary>` (Eintrags-ID, Art, Bezeichnung, Toleranz; nie der Wert),
     `add_scope`, `set_locked`, `set_enabled`, `remove_entry`, `remove_scope`, `delete_list`, `update_entry`;
     schwächende Änderungen nur mit `Actor::VerifiedUser`, Änderungen an gesperrten Listen nicht mit
     `Actor::Agent`; `reveal(list) -> Vec<EntryWithValue>` nur `Actor::Gui`.
   - `active() -> Result<Arc<MaskSet>, MaskError>`; `MaskSet::is_empty`, `mask(&str) -> MaskedText` (Text mit
     Platzhaltern und Abbildung `map_range(Range) -> Range` vom Original auf den maskierten Text),
     `mask_path(&str) -> String`, `mask_bytes(&[u8]) -> Vec<u8>`, `unmask(&str) -> Result<String, UnmaskError>`,
     `stream() -> StreamMasker` (`push(&[u8]) -> Vec<u8>`, `flush() -> Vec<u8>`), `lockdown() -> bool`,
     `local_scopes() -> Vec<PathBuf>`; neue Zuordnungen werden sofort im Speicher gesichert.
   - `lockdown_state() -> LockdownState` (`Off`, `On`, `Unknown` = fail-closed) ohne Schlüssel, für den Hook.
   - `IntegrityGuard` (`MaskStore::guard()`, `check_and_restore() -> Result<(), Tampered>`).
2. **Ersetzen** (Paket P2), in storage:
   - `ReplaceRequest { query: ContentQuery, replacement: String, selection: ReplaceSelection, dry_run: bool }`,
     `ReplaceSelection::{All, Files(Vec<String>), Hits(Vec<HitRef>)}`, `HitRef { rel, line, ranges, size,
     mtime_ms }`.
   - `vfs::content_replace(backend, root, &request, location, emit, cancel) -> VfsResult<ReplaceSummary>` mit
     Ereignissen `ReplaceEvent::{File, Skipped, Progress, Preview}`; `ReplaceSummary { files_searched,
     files_changed, replacements, skipped, undo_id }`.
   - `ReplaceJournal`: `history()`, `undo(id, resolve: &dyn Fn(&str) -> VfsResult<BackendHandle>)`, `discard(id)`;
     `location` ist die Zielschreibweise der Oberfläche (se-Zielgrammatik).
3. **`engine::elevate`** (Paket P5):
   - `is_elevated() -> bool`, `spawn_elevated(ElevatedSpec) -> Result<ElevatedChild, ElevateError>` (stdin,
     stdout, stderr, Ende, Abbruch), `confirm_user_presence(reason: &str) -> Result<(), ElevateError>`,
     `ElevateError::{Declined, NoAgent, NotAllowed(String), Failed(String)}`.
4. **MCP** (Pakete P4, P5, P6):
   - Ergebnisse werden genau einmal maskiert: entweder maskiert das Werkzeug selbst und gibt
     `response::masked(value)` zurück, oder `server.rs::dispatch` maskiert jede Zeichenkette des Ergebnisses.
   - Job-Ströme sind `Raw` (Ausgabe von Befehlen; `jobs.rs` maskiert beim Anhängen mit `StreamMasker`) oder
     `PreMasked` (Treffer, die das Werkzeug selbst maskiert).
   - Jede Zeichenkette der Argumente wird vor dem Werkzeug zurückgeführt (`MaskSet::unmask`).
   - Werkzeugmodule mit `DESCRIPTION`, `properties()` und `start()`/Aufruf: `shell_tool.rs` (P5),
     `replace_tool.rs`, `patch_tool.rs` (P4b), `masks_tool.rs` (P4a); Eintrag in `catalog.rs`/`server.rs` durch P4a.

### Pakete und Dateien

| Paket | Inhalt | Schreibt (und nur diese) |
|---|---|---|
| P1 | Maskenkern (M1) | `native/crates/engine/src/masks/**` |
| P2 | Ersetzen-Kern und `se replace` (M2) | neue Dateien `storage/src/agent_proto/os/shared/content_replace*.rs`, `storage/src/vfs/os/shared/content_replace*.rs`, `storage/src/vfs/os/shared/replace_journal*.rs`, Moduleinträge in `storage/src/agent_proto/mod.rs`, `storage/src/vfs/mod.rs`; `cli/src/cli/replace.rs`, `cli/src/cli/grep.rs` (Schalter `--gitignore`, gemeinsame Optionen) |
| P3 | Ersetzen auf Desktop und Android (M3) | `gui/src/app/core/content_search_*.rs`, `text_tools.rs`, `frame_keyboard.rs` (nur Ersetzen-Kürzel), neue `gui/src/app/core/content_replace_*.rs`; `mobile/src/mobile/os/shared/content_search*.rs`, `dispatch.rs` (nur neue Methoden); Android `ui/files/ContentSearch*.kt`, `ContentResults.kt`, `api/*` für die neuen Methoden; `docs/superpowers/plans/2026-09-25-android-apk/api.md` §4.14 |
| P4a | MCP-Maskenschicht, `masks`-Werkzeug, Registrierung (M4) | `cli/src/cli/mcp/{catalog,catalog_tests,server,response,args,jobs,job_view,spool,file_tools,file_write,line_read,targets,transfer_tools,cli_tool,status_tools,session,root_tool,screen_tool}.rs`, neue `mcp/masks_tool.rs`, `mcp/mask_layer*.rs`, `cli/src/cli/plugin/assets/SKILL.md` |
| P4b | MCP-Coding (M5) | `cli/src/cli/mcp/{grep_tool,grep_tool_tests,grep_records,search_tool}.rs`, neue `mcp/replace_tool.rs`, `mcp/patch_tool*.rs`; `cli/src/cli/search.rs`, `SearchArgs` in `cli/src/cli/mod.rs`; `storage/src/agent_proto/core/{content_frames,content_frames_tests,features,codec}.rs`, `storage/src/agent_proto/os/shared/content_walk.rs`, `content_local.rs`; `engine/src/share/core/content_search_wire*.rs`, `wire_capabilities.rs`, `fs_guard_content.rs`, `share/os/shared/host_content_search*.rs`, `daemon/os/shared/backend_version.rs`; `se-agent/Cargo.toml` |
| P5 | Shell, Erhöhung, Share-Exec (M6) | `engine/src/elevate/**`, `cli/src/cli/elevated.rs`, neue `mcp/shell_tool*.rs`, `mcp/exec_tool.rs`, `mcp/exec_session_job.rs`, `mcp/exec_share_job.rs`, `mcp/exec_ssh_job.rs`, `mcp/shell_registry.rs`, `cli/src/cli/shell_session/**`, `cli/src/cli/exec*.rs`, Share-Exec-Dateien in `engine/src/share/` (Exec-Start, Host-Exec), `[target.'cfg(windows)'.dependencies]` in `engine/Cargo.toml` |
| P6 | Vollsperre und Verwaltung (M7) | `cli/src/cli/hook*.rs`, `cli/src/cli/masks_cli*.rs`, `cli/src/cli/plugin/**` außer `assets/SKILL.md`, neue `gui/src/app/core/settings_masks*.rs`, `settings_navigation.rs`, `settings_ui.rs` (nur Seiteneintrag), `batches_settings_task_tests.rs`, neue `docs/oberflaechen/masken.md` |
| Hauptagent | Verträge, Doku über mehrere Pakete, Register, Suite, Integration, Release | `docs/**` außer `masken.md`, `docs/tests/register.tsv`, `native/test-crates-task.py`, Workflows |

### Fortschritt

- 2026-11-28: Plan und Verträge committet (`90a400e5`). Sieben Umsetzungsagenten gestartet (P1, P2, P3, P4a, P4b,
  P5, P6), jeweils Claude Code, Modell `claude-opus-5-5`, Effort `high` nach `modellwahl` (Profil coding,
  Einstufung 8, Umfang large); gemeinsame Vorgaben in der Briefdatei des Hauptagenten.
- Zweiter Weg mit Umgebungsbeleg (Arbeitsweise „Kein zweiter Weg ohne Tatsache der Umgebung“): Linux-Erhöhung
  pkexec, ohne registrierten polkit-Agenten `sudo -n`; Beleg: pkexec endet ohne Agent mit Exit 127 „No
  authentication agent found“ (Ref. sperre §4.2, beobachtet 2026-11-28).
- 2026-11-28: Alle sieben Pakete fertig und zusammengeführt (Commits `2a01a99a` bis `ccccf34c`): Moduleinträge
  in `mcp.rs`, Schema von `search` aus `search_tool.rs`, Share-Suche nur mit `content_search_ignore_v1`,
  Härtung nach `masks` `add_path`, Ordnerauswahl `MaskScope`, Menüeintrag „In Dateien ersetzen…“, Sperre der
  ganzen MCP-Sitzung nach einer Manipulation des Maskenspeichers, `elevated` im Schema von `exec`,
  Suite-Listen, 147 Register-Zeilen, Dokumente. Lock-Dateien auf CI aufgelöst (lock-refresh 46140564566,
  46140567172). Agent-Bundles für Protokoll 14: crates-task part agent-bundles 46140739416.
