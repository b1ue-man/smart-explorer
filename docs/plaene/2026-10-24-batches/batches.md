# Nächste Läufe: Batches und Modelle (2026-10-24)

## Auftrag

Nutzer, 2026-10-24: „fass die aufgaben in vernünftige batches zusammen und strukturiere sie sauber. danach wähle die entsprechenden models aus. das ziel im prinzip bleibt so wenig agents wie möglich. so viele wie nötig.“

Grundlage sind zwei Machbarkeitsnotizen:
- `docs/plaene/2026-10-22-fernzugriff-desktop/machbarkeit.md`: Punkte P1 bis P9, Entscheidungen E1 bis E6;
- `docs/plaene/2026-10-22-laufwerke-medien-agenten/machbarkeit.md`: Punkte P10 bis P26, Entscheidungen E9 bis E13.

Stand: Nichts ist begonnen. Dieser Plan legt nur Zuschnitt, Reihenfolge und Besetzung fest. Die Meilensteinpläne entstehen je Batch in dessen eigener Planung nach `AGENTS.md`, Abschnitt „mandatory workflow“.

## Umfang

**In den Batches** (23 Punkte): P1, P2, P3, P4, P5, P6, P7, P9, P10, P11 Stufe 1, P13, P14, P15, P16, P17, P18, P19, P20, P21, P22, P23, P24, P26.

**Nicht in den Batches:**
- P8: ein Hinweis, kein Arbeitspunkt.
- P11 Stufen 2 und 3 sowie P12: ruhen mit `OPTISCH-DATENTRAEGER` (`later`).
- P25: zurückgestellt und optional (`WINDOWS-OHNE-FREMDTREIBER`, `later`).
- Die übrigen offenen Zeilen in `docs/TODO.md`: nicht Teil dieses Auftrags.

## Regeln für den Zuschnitt

- **Ein Batch ist ein Lauf.** Er hat eine eigene Planung, eine Task-Suite und am Ende ein Release (`AGENTS.md`).
- **Zuschnitt nach Codefläche.** Ein Batch berührt möglichst wenige Teilsysteme. So hängt die eine Suite zusammen, und die Fixschleife bleibt kurz.
  - Befund zum Vorgänger: Die Suite `rechte-desktop-tresore-task.yml` des Stapels vom 2026-10-17 mit fünf Hauptsträngen lief am 2026-10-21 neunmal, mit 4 roten und 5 abgebrochenen Läufen (GitHub-Actions-Liste, abgefragt am 2026-10-24).
- **Abhängigkeiten bestimmen die Reihenfolge:**
  - P4 braucht P3 und P6, und P5 braucht P4.
  - Neue Rechte sollen gleich auf der Geräteseite aus P18 entstehen, statt erst in die heutigen Funktionsreiter zu kommen. Das betrifft das Recht für den Anmeldebildschirm (E5) und das Recht für den Programmstart (E4).
- **Kleine, sichtbare Fehler zuerst.**

## Die fünf Batches

| Batch | Name | Punkte | Board-Zeilen | Entscheidungen im Batch | Umfang |
|---|---|---|---|---|---|
| 1 | Laufwerke, Dateien und Anbindungen | P10, P11 Stufe 1, P13, P14, P21, P22, P23, P24, P26 | `LAUFWERKE-LIVE`, `OPTISCH-LESEN`, `AGENT-PLUGIN-UPDATE`, `KOPIE-ORDNER-ZUSATZ`, `TRESOR-OEFFNEN-WURZEL`, `LAUFWERK-RESTEINTRAG`, `LAUFWERK-PLATZ`, `LESERECHTE-NUR-NEU`, `GDRIVE-ABLAUF` | E9, E10, Weg A oder B bei P21 je Speicherart | mittel bis groß |
| 2 | Bildschirm-Grundlagen | P2, P3, P6, P7, P9 | `LAUNCHER-WIDGET-MOVE`, `DESKTOP-APP-ICONS`, `SCREEN-NUR-SICHTBAR`, `SCREEN-HANDY-BILD`, `SCREEN-VOLLBILD-TASTE` | keine aus E1 bis E6 | mittel |
| 3 | Fenster und Einstellungen | P15, P16, P17, P18, P19, P20 | `DESKTOP-ORDNER-EXPLORER`, `AUSWAHL-HOEHE`, `FENSTER-GROESSE`, `EINSTELLUNGEN-GERAETE`, `EINSTELLUNGEN-TECHNIK`, `EINSTELLUNGEN-AUSRICHTUNG` | E12 | groß |
| 4 | Anmeldung und Remote Desktop | P1 (Linux, Windows, RDP) | `SCREEN-ANMELDUNG`, `SCREEN-RDP` | E1, E2, E5 | groß, sicherheitskritisch |
| 5 | Ein Desktop, alle Programme | P4, P5 | `DESKTOP-ALLE-PROGRAMME`, `DESKTOP-PROGRAMME-STARTEN` | E3, E4 | sehr groß |

### Reihenfolge (entscheidet E6 und E11)

Die Batches laufen in der Reihenfolge 1, 2, 3, 4, 5, nacheinander. Damit sind E6 und E11 entschieden. Die Begründung:

- **Batch 1 zuerst.** Er enthält die Fehler, die heute den Alltag stören:
  - eine Fehlermeldung bei jedem Start (P22);
  - Tresor-Dateien lassen sich nicht öffnen (P21);
  - die Google-Drive-Anmeldung läuft ab (P26);
  - neue Laufwerke erscheinen erst nach einem Neustart (P10).
- **Batch 2 vor Batch 3.**
  - Batch 2 ist kleiner und liefert die kleinen Bildschirmpunkte P9, P2 und P7 früh aus. Das entspricht dem Vorschlag in E6.
  - P3 und P6 sind Grundlagen für P4.
- **Batch 3 vor Batch 4 und 5,** weil beide neue Rechte bringen, die auf die Geräteseite aus P18 gehören.
- **Batch 5 zuletzt,** weil er auf P3 und P6 aus Batch 2 aufbaut.

Abweichung von den Vorschlägen:
- E11 sah für P10 bis P26 vier Stufen in einem Lauf vor. Jetzt verteilen sich diese Punkte auf Batch 1 und Batch 3.
- E6 stellte P1 vor P4 und P5. Das bleibt so (Batch 4 vor Batch 5).

## Modelle je Rolle

Die Regeln stammen vom Nutzer:
- vom 2026-10-19: „sonnet für implementierungen die keine interpretation mehr zulassen, opus nur für sicherheitsrelevantes einer und design n anderer, gebatched die beiden also. der rest wenn möglich via sonnet.“
- vom 2026-10-17: „planning at xhigh, design too.“
- Kein Workflow-Werkzeug.

| Rolle | Modell | Effort | Aufgabe | Begründung |
|---|---|---|---|---|
| Hauptagent | Opus 5.5 (diese Sitzung) | Sitzungsstandard | Plan Stufe 1 und Endplan, Entscheidungen ohne Sicherheitsbezug, Integration, Commits, Pushes, Remote-CI, kleine Korrekturen in der Fixschleife, Release | die Planung bleibt beim Hauptagenten, dafür entsteht kein eigener Planungsagent |
| Bestand | haiku | medium | Code und Doku lesen. Ergebnis: eine Inventur mit `Datei:Zeile` über alle Oberflächen des Batches (Terminal mit JSON, Desktop-GUI, Android-GUI, MCP, IPC und Brücke, Doku) | reine Lektüre ohne Gestaltung |
| Recherche | sonnet | medium | Primärquellen zu den Lücken des Batches, in zwei Runden | „der rest wenn möglich via sonnet“ |
| Entwurf | opus | xhigh | Gestaltung von Oberfläche oder Architektur, als Vorgabe ohne Auslegungsspielraum | ein gebündelter Opus-Agent für Design |
| Sicherheit | opus | xhigh | Sicherheitsprüfung und Entscheidungen mit Sicherheitsbezug, dann Umsetzung des sicherheitskritischen Kerns und Schlussprüfung des Batches | ein gebündelter Opus-Agent für Sicherheitsrelevantes |
| Umsetzung | sonnet | high | Umsetzung nach dem Endplan. Jeder Umsetzer hat eine eigene Dateifläche, die sich mit keiner anderen überschneidet | „sonnet für implementierungen die keine interpretation mehr zulassen“ |
| Suite | sonnet | high | die eine Task-Suite des Batches mit ihrem Workflow | ebenfalls Umsetzung |

Ohne eigenen Agenten:
- **Zweite Recherche:** läuft im Rechercheagenten derselben Batch weiter, über SendMessage.
- **Schlussprüfung Sicherheit:** läuft im Sicherheitsagenten derselben Batch weiter.
- **Fixschleife:** Kleine Korrekturen macht der Hauptagent selbst. Größere Korrekturen gehen an den zuständigen Umsetzer, über SendMessage.

Ein frischer Agent wird nur dann gestartet, wenn der fortzusetzende Agent beendet oder abgebrochen ist.

Die Suite schreibt ein frischer Agent statt eines fortgesetzten Umsetzers. Annahme: Das ist billiger, weil ein fortgesetzter Agent seinen ganzen Verlauf bei jeder Anfrage neu liest. Ein frischer Agent liest nur den Endplan und den Diff.

## Besetzung je Batch

Pfadkürzel: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

Die genauen Dateilisten der Aufträge legt der Endplan des Batches fest (`AGENTS.md`, Abschnitt „subagent scope“). Die Flächen unten nennen die Befunde der Machbarkeitsnotizen. Annahme: Dort liegt der Schwerpunkt; die Inventur des Batches vervollständigt ihn.

### Batch 1: Laufwerke, Dateien und Anbindungen

| Agent | Modell, Effort | Auftrag und Fläche |
|---|---|---|
| Bestand | haiku, medium | Inventur der Oberflächen von P10, P11, P13, P14, P21, P22, P23, P24, P26 |
| Recherche | sonnet, medium | Lücken 1, 3, 5, 14 und 16 der Notiz Laufwerke. Das sind: Geräteereignisse unter Windows (`WM_DEVICECHANGE`, `CM_Register_Notification`, Netzlaufwerke); Cache und Wiederholungen optischer Laufwerke; MCP-Neuladen bei Codex und Claude Code sowie das Umbenennen einer laufenden `se.exe`; die Platzabfrage je Speicherart; der Tokenweg in App, Hintergrunddienst und Terminal |
| Sicherheit | opus, xhigh | Weg A bei P21 je Speicherart prüfen und den Wurzelschutz umsetzen (`N/daemon/os/shared/mount_paths.rs`, `N/daemon/os/shared/rooted_backend.rs`, `N/app/os/shared/vault_open.rs`). Stufe B von P13 umsetzen: laufende Server nicht beenden und das Ersetzen an den SHA-256 binden. Den Tokenweg bei P26 umsetzen (`N/cloud/core/token_persistence.rs`, `N/cloud/core/cloud.rs`). Schlussprüfung |
| Umsetzer „Laufwerke“ | sonnet, high | P22 (`N/daemon/os/shared/mount_manager.rs`, `N/app/core/mount_ui.rs`, neuer Befehl `se drive`). P23 (`N/mount/os/windows/callbacks_metadata.rs`, `N/mount/os/linux_os/fuse_ops_meta.rs`, Platzabfrage der Backends). P21 außerhalb des Sicherheitskerns, also Oberfläche und Meldungen |
| Umsetzer „Dateien“ | sonnet, high | P11 Stufe 1 (`N/transfer/core/flow_control.rs`, `N/transfer/os/shared/engine/local.rs`). P14 (`N/app/core/transfer_route.rs`, `N/transfer/os/shared/engine/folders.rs`, Einfügedialoge am Desktop und unter Android, Option für `se cp`, MCP). P24 (`N/app/core/read_access.rs`, `N/app/core/analytics_access.rs`) |
| Umsetzer „Laufwerksliste und Anbindungen“ | sonnet, high | P10 (`N/app/core/init.rs`, `N/app/os/windows/platform.rs`, `N/app/os/linux_os.rs` mit UDisks2). P13 Stufe A (Updater, `N/cli/plugin/files.rs`, `N/cli/mcp/server.rs`). P26: Diagnose und Neuanmeldung mit einem Klick in Desktop, Android und Terminal |
| Suite | sonnet, high | Suite mit Linux-, Windows- und Android-Job. Der Windows-Job ist nötig für Dokany, den optischen Leseweg und `se.exe` |

**Summe: 7 Agenten,** davon 1 Opus.

Entscheidungen:
- E9 (Stufen von P13) und E10 (Voreinstellung von P14 in Terminal und MCP) fallen im Endplan.
- Weg A oder B bei P21 entscheidet der Sicherheitsagent.

Messung: P11 Stufe 1 lässt sich nur an einem Gerät mit optischem Laufwerk messen. Die Messung ist ein Beitrag des Nutzers mit einem Befehl, den der Plan nennt. Sie ist keine Bedingung für das Release. Ohne Messung meldet der Lauf die Wirkung als ungeprüft.

### Batch 2: Bildschirm-Grundlagen

| Agent | Modell, Effort | Auftrag und Fläche |
|---|---|---|
| Bestand | haiku, medium | Inventur der Oberflächen von P2, P3, P6, P7, P9 |
| Recherche | sonnet, medium | Symbolquellen (Store-Apps über AppUserModelID, `_NET_WM_ICON`, Icon-Theme, PackageManager); Verdeckung unter Windows und X11; MediaCodec (Bitratenmodus, Schlüsselbild auf Anforderung, Intra-Refresh); Vollbild unter Android mit Hardwaretastatur |
| Umsetzer „Nativ“ | sonnet, high | Host unter Windows und Linux (`N/screen/core/window_list.rs`, `N/screen/os/windows/`), Protokoll, Symbolabruf mit Zwischenspeicher. Desktop-Betrachter (`N/app/core/screen_viewer_app.rs`, `N/app/core/screen_input_keys.rs`, `N/app/core/screen_view_widget.rs`, `N/app/core/screen_tile_grid.rs`). Kacheln der se Desktop-Fläche (`N/app/core/desktop_surface_tile_ui.rs`). Ratensteuerung (`N/screen/core/rate_control.rs`). `se screen windows --json`, MCP über `se`, `docs/oberflaechen/bildschirm.md` |
| Umsetzer „Android“ | sonnet, high | P2 (`A/launcher/ui/CellGrid.kt`, `A/launcher/ui/DragLayer.kt`, `A/launcher/widgets/WidgetView.kt`). P7 (`A/screen/ScreenEncoder.kt`). P9 und P6 im Android-Betrachter (`A/ui/screen/ScreenViewScreen.kt`). Symbole des Android-Hosts (`A/api/ScreenApi.kt`, `A/screen/RunningApps.kt`) |
| Suite | sonnet, high | Suite mit Linux-, Windows- und Android-Job |

**Summe: 5 Agenten,** kein Opus.

- **Kein Sicherheitsagent.** Das einzige sicherheitsnahe Stück ist das Dekodieren fremder Symbolbilder. Dafür legt der Endplan feste Grenzen fest: nur PNG, Höchstgröße in Bytes und Pixeln, Zwischenspeicher nach Inhalts-Hash.
- **Kein Entwurfsagent.** Die Gestaltung ist klein: Ausstiegskombination, Hinweis beim Eintritt ins Vollbild, Symbol auf der Kachel. Sie steht im Endplan des Hauptagenten.

Messung bei P7: Die Messung mit `se screen bench` und dem Telefon als Host ist ein Beitrag des Nutzers. Ohne Messung setzt der Lauf nur die Maßnahmen um, die unabhängig von der Ursache richtig sind:
- Bitratenmodus und Priorität ausdrücklich setzen;
- ein Schlüsselbild anfordern, wenn ein Verlust erkannt wird;
- ein kürzerer I-Bild-Abstand.

Die Wirkung meldet er dann als ungeprüft.

### Batch 3: Fenster und Einstellungen

| Agent | Modell, Effort | Auftrag und Fläche |
|---|---|---|
| Bestand | haiku, medium | Inventur aller Einstellungs- und Freigabeseiten am Desktop und unter Android, der 14 gesperrten Dialoge und der 18 Listen mit fester Höhe. Dazu die Code-Lücken 7, 9, 10 und 11 der Notiz Laufwerke: der Startauftrag „Ort öffnen“, das Speichern der Fenstergrößen, der Weg zu „Geräte & Freigaben“ und Raumrechte gegen Geräterechte, sowie die Android-Einstellungen |
| Entwurf | opus, xhigh | E12, die Gliederung. Ein Formularbaustein und ein Tabellenbaustein. Die Zuordnung jeder heutigen Einstellung zu ihrer neuen Seite am Desktop und unter Android. Die Liste aus P19, was bleibt und was weggeht, einschließlich der Angaben mit unklarem Nutzen. „Ordner öffnen mit“ aus P15. Mindestgrößen und das Merken der Größe aus P16 und P17 |
| Umsetzer „Fenster und se Desktop“ | sonnet, high | P15 (`N/desktop/os/shared/open_target.rs`, `se desktop config`). P16 (`N/app/core/picker_impl.rs`). P17 in allen Dialogen und Listen außerhalb der Einstellungs- und Freigabeseiten |
| Umsetzer „Einstellungen Desktop“ | sonnet, high | P18, P19 und P20 am Desktop, dazu P17 innerhalb dieser Seiten (`N/app/core/settings_*.rs`, `N/app/core/share_*_ui.rs`, neue Datei für den Formularbaustein) |
| Umsetzer „Einstellungen Android“ | sonnet, high | P18, P19 und P20 unter Android (`A/ui/settings/`, `A/ui/share/`) |
| Suite | sonnet, high | Suite mit Linux-, Windows- und Android-Job |

**Summe: 6 Agenten,** davon 1 Opus.

- **Kein Rechercheagent.** Die offenen Fragen dieses Batches betreffen den Code und gehen an den Bestandsagenten.
- **Ein Sicherheitsagent nur, wenn ein neuer Schreibweg für Rechte entsteht.** P18 verschiebt Einstellungen nur; Rechte, ihre Werte und ihre Speicherorte bleiben. Bringt der Endplan einen Befehl, der alle Rechte eines Geräts setzt, prüft ein Sicherheitsagent (opus, xhigh) diesen Weg. Dann sind es 7 Agenten.

### Batch 4: Anmeldung und Remote Desktop

| Agent | Modell, Effort | Auftrag und Fläche |
|---|---|---|
| Bestand | haiku, medium | Inventur von Eingabeweg, Worker, Rechten, Installer und Share-Tunnel für P1 |
| Recherche | sonnet, medium | Muster für einen Windows-Systemdienst (RustDesk, WTS-Sitzungen, Token in die Konsolensitzung, `SendSAS` mit seiner Richtlinie). DXGI auf dem Winlogon-Desktop. Windows Hello Enhanced Sign-in Security. X-Berechtigung am Anmeldebildschirm bei LightDM, GDM und SDDM. RDP-Hosts (xrdp, GNOME Remote Desktop, KRdp) und ob sich TermService an Loopback binden lässt. Stand und Lizenz von IronRDP. RDP-Clients unter Android |
| Sicherheit | opus, xhigh | Bedrohungsmodell, dann E1, E2 und E5 entscheiden. Kern umsetzen: Systemdienst, Start des Helfers mit SYSTEM-Token, abgesicherte Verbindung zwischen Dienst und Helfer, Prüfung des neuen Rechts, Freigabeliste für die Port-Weiterleitung im Share-Tunnel. Schlussprüfung |
| Umsetzer „Windows“ | sonnet, high | Dienst im Installer einrichten und entfernen. Erfassung (DXGI) und Eingabe (`SendInput`) auf dem Eingabe-Desktop. Erkennen des Desktopwechsels. Windows-Teile der Doku |
| Umsetzer „Linux und Clients“ | sonnet, high | P1 unter Linux: Eingabe am X11-Anmeldebildschirm und Betrieb des root-Workers am Display-Manager. Start des RDP-Clients aus Terminal, Desktop-GUI und Android-GUI. Das neue Recht auf der Geräteseite aus Batch 3 und in `se share grants access`. MCP über `se`. `docs/oberflaechen/bildschirm.md` |
| Suite | sonnet, high | Suite mit Linux-, Windows- und Android-Job |

**Summe: 6 Agenten,** davon 1 Opus.

### Batch 5: Ein Desktop, alle Programme

| Agent | Modell, Effort | Auftrag und Fläche |
|---|---|---|
| Bestand | haiku, medium | Inventur von Fensterlisten, Fensterquellen, Betrachter, se Desktop-Fläche, Launcher, `se screen` und `se exec` |
| Recherche | sonnet, medium | Fenstererfassung mit WGC, Composite, ScreenCast-Portal mit `restore_token`, Mutter `RecordWindow`, KWin `stream_window` und `ext-image-capture-source-v1`. Unter Android VirtualDisplay und Shizuku (E3) sowie die Ausnahmen für Starts aus dem Hintergrund. `IApplicationActivationManager`. Start als `app.slice`-Einheit. Vorbilder: RemoteApp, xpra seamless, VirtualBox Seamless, Parallels Coherence, VMware Unity |
| Entwurf | opus, xhigh | Protokoll für Fensterbeschreibungen, einen Strom je Fenster, einen lokalen Prozess mit nativen Fenstern, Fokus und Eingaben je Fenster, Z-Ordnung und Bandbreite über das Relay. Bedienung auf der se Desktop-Fläche und im Launcher |
| Sicherheit | opus, xhigh | E4 (Recht für den Programmstart) und E3 (Shell-Rechte unter Android) entscheiden. Kern umsetzen: Prüfung des Programmstarts, Treffsicherheit der Eingaben. Schlussprüfung |
| Umsetzer „Host Windows“ | sonnet, high | Fensterliste mit Geometrie, Z-Ordnung und Ereignissen. WGC je Fenster. Aktivieren und Eingabe. Programmliste mit Store-Apps. Start in der Nutzersitzung |
| Umsetzer „Host Linux“ | sonnet, high | X11 (EWMH, Composite, XTest) und Wayland (Portal je Fenster). Programmliste aus `.desktop`. Start als `app.slice`-Einheit |
| Umsetzer „Lokale Fenster“ | sonnet, high | Ein Prozess mit je einem nativen Fenster pro Fernfenster, mit Taskleiste, Symbol aus P3 und Titel „Programm – Gerät“. se Desktop-Fläche und Launcher. Öffnen, Auflisten und Starten im Terminal mit `--json`. MCP über `se`. Doku |
| Umsetzer „Android“ | sonnet, high | Android-Host: eine App zugleich ab Android 14, Programmliste über `<queries>`, Start mit Ausnahme. Android-Betrachter: eine Aufgabe je Fernprogramm |
| Suite | sonnet, high | Suite mit Linux-, Windows- und Android-Job |

**Summe: 9 Agenten,** davon 2 Opus. Gleichzeitig laufen höchstens 5, nämlich die 4 Umsetzer und der Sicherheitsagent. Erlaubt sind 7 (`AGENTS.md`).

## Gesamtzahl

| Batch | haiku | sonnet | opus | Summe |
|---|---|---|---|---|
| 1 | 1 | 5 | 1 | 7 |
| 2 | 1 | 4 | 0 | 5 |
| 3 | 1 | 4 | 1 | 6 (7 mit Sicherheitsagent) |
| 4 | 1 | 4 | 1 | 6 |
| 5 | 1 | 6 | 2 | 9 |
| zusammen | 5 | 23 | 5 | 33 (34) |

Warum nicht weniger:
- **Getrennte Bestands- und Rechercheagenten.**
  - Bestand braucht nur Lektüre und läuft deshalb auf haiku.
  - Recherche braucht Webquellen und Urteil. Ein gemeinsamer Agent müsste für beides sonnet sein.
- **Mehrere Umsetzer je Batch, weil die Dateiflächen getrennt liegen:**
  - Batch 1: Einhängedienst, Kopier-Engine mit Explorer, sowie App-Start mit Updater und Cloud.
  - Batch 2: Rust und Kotlin.
  - Batch 3: Dialoge, Desktop-Einstellungen und Android-Einstellungen.
  - Batch 4: Windows-Dienst sowie Linux mit den Clients.
  - Batch 5: Windows-Host, Linux-Host, lokale Fenster und Android.

  Annahme: Ein einzelner Umsetzer über zwei Flächen hätte einen doppelt so langen Verlauf, den er bei jeder Anfrage neu liest.
- **Ein Opus-Agent je Rolle und Batch,** und nur in Batches, die diese Rolle brauchen. Batch 2 hat keinen.

Der Endplan eines Batches darf einen Umsetzer teilen, wenn seine Fläche zwei unabhängige Teile hat. Er darf zwei Umsetzer zusammenlegen, wenn sich ihre Flächen als klein erweisen. Jede Änderung der Besetzung steht im Endplan mit Grund.

## Ablauf je Batch

1. **Bestand und erste Recherche** laufen parallel.
2. **Plan Stufe 1** schreibt der Hauptagent.
3. **Entwurf und Sicherheitsprüfung** laufen parallel, soweit der Batch sie hat. Sie arbeiten auf Bestand, Recherche und Plan Stufe 1.
4. **Zweite Recherche** zu den Lücken aus Schritt 3. Sie läuft im selben Rechercheagenten weiter.
5. **Endplan** schreibt der Hauptagent. Er enthält:
   - die Meilensteine mit prüfbarem Ergebnis;
   - den Dateibesitz je Umsetzer;
   - die Zuordnung jedes Meilensteins zu einem Suite-Test.
6. **Umsetzung:** Die Umsetzer arbeiten parallel. Der Sicherheitsagent setzt seinen Kern um, als Fortsetzung.
7. **Schlussprüfung Sicherheit** über alle Änderungen mit Sicherheitsbezug, ebenfalls als Fortsetzung des Sicherheitsagenten. Nur in Batches mit Sicherheitsagent.
8. **Abschluss:**
   - Der Suite-Agent schreibt die Suite.
   - Der Hauptagent committet und pusht.
   - Die Suite läuft einmal remote, danach folgt die Fixschleife.
   - Danach kommt ein Release über den Remote-Weg.

Zwischen dem Start der Remote-Suite und dem Ende des Releases startet der Hauptagent keine Agenten (`AGENTS.md`).

## Regeln für jeden Agentenauftrag

- Der Auftrag beginnt mit dem Satz, der die Arbeitsweise abwählt.
- Er nennt:
  - die Dateien zum Lesen;
  - die Dateien zum Ändern;
  - was verboten ist;
  - das Ergebnis, das Abnahmezeichen und das Berichtsformat (`AGENTS.md`).
- Agenten bauen, testen, committen und pushen nicht.
- Aufträge an haiku und sonnet verbieten das Werkzeug `advisor`.
- Jeder Aufruf setzt Modell und Effort ausdrücklich. Das tatsächliche Modell wird am Transkript belegt.
- Recherche arbeitet so:
  - WebFetch mit gezieltem Prompt, keine vollständigen Seiten über die Shell;
  - kurze Shell-Ausgaben;
  - Ergebnisse schrittweise in die Ergebnisdatei schreiben.
- Gerätedaten des Nutzers kommen in kein Ergebnis im Repository. Dazu gehören Gerätenamen, Geräte-IDs, PINs und lokale Pfade.

## Releases

- Jeder Batch endet mit genau einem Release.
- Annahme: Ohne ein dazwischenliegendes Release sind das die Versionen 0.5.185 bis 0.5.189. Heute gilt 0.5.184 (`native/Cargo.toml`, `release-native/update-feed/version.txt`).
