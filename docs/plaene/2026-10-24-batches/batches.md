# Nächste Läufe: Batches und Modelle (2026-10-24)

**Status: vorläufiger Vorschlag.** Nichts ist begonnen. Der Zuschnitt unten ist ein erster Vorschlag. Die Batches werden erst nach der Planung festgelegt, die nach der Planungsvorgehensweise der Arbeitsweise läuft. Erst danach werden auch die Umsetzungsagenten zugeteilt (Abschnitt „Zeitpunkt der Agentenwahl“).

## Auftrag

Nutzer, 2026-10-24:
- „fass die aufgaben in vernünftige batches zusammen und strukturiere sie sauber. danach wähle die entsprechenden models aus. das ziel im prinzip bleibt so wenig agents wie möglich. so viele wie nötig.“
- Korrektur am selben Tag: „für planung gibt es eine bestimmte vorgehensweise in der arbeitsweise. der ist eigentlich zu volgen und erst danach kann man die umsetzungsagenten und batches wirklich zuteilen.“ Außerdem: „agentenwahl zur umsetzung erst nach der planung, agentenwahl zur detailplanung oder entwürfe erstellen erst nach dek ersten planungssvhritt. wenn wir umsetzung via sonnet machen, was an sich ok ist, will ich auch einen separaten opus am ende zur review der gesamtumsetzung vor tests, zur hoch kritisvhen review.“
- Danach: „noch nicht starten“.

Grundlage sind zwei Machbarkeitsnotizen:
- `docs/plaene/2026-10-22-fernzugriff-desktop/machbarkeit.md`: Punkte P1 bis P9, Entscheidungen E1 bis E6;
- `docs/plaene/2026-10-22-laufwerke-medien-agenten/machbarkeit.md`: Punkte P10 bis P26, Entscheidungen E9 bis E13.

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

## Vorschlag: fünf Batches

| Batch | Name | Punkte | Board-Zeilen | Entscheidungen im Batch |
|---|---|---|---|---|
| 1 | Laufwerke, Dateien und Anbindungen | P10, P11 Stufe 1, P13, P14, P21, P22, P23, P24, P26 | `LAUFWERKE-LIVE`, `OPTISCH-LESEN`, `AGENT-PLUGIN-UPDATE`, `KOPIE-ORDNER-ZUSATZ`, `TRESOR-OEFFNEN-WURZEL`, `LAUFWERK-RESTEINTRAG`, `LAUFWERK-PLATZ`, `LESERECHTE-NUR-NEU`, `GDRIVE-ABLAUF` | E9, E10, Weg A oder B bei P21 je Speicherart |
| 2 | Bildschirm-Grundlagen | P2, P3, P6, P7, P9 | `LAUNCHER-WIDGET-MOVE`, `DESKTOP-APP-ICONS`, `SCREEN-NUR-SICHTBAR`, `SCREEN-HANDY-BILD`, `SCREEN-VOLLBILD-TASTE` | keine aus E1 bis E6 |
| 3 | Fenster und Einstellungen | P15, P16, P17, P18, P19, P20 | `DESKTOP-ORDNER-EXPLORER`, `AUSWAHL-HOEHE`, `FENSTER-GROESSE`, `EINSTELLUNGEN-GERAETE`, `EINSTELLUNGEN-TECHNIK`, `EINSTELLUNGEN-AUSRICHTUNG` | E12 |
| 4 | Anmeldung und Remote Desktop | P1 (Linux, Windows, RDP) | `SCREEN-ANMELDUNG`, `SCREEN-RDP` | E1, E2, E5 |
| 5 | Ein Desktop, alle Programme | P4, P5 | `DESKTOP-ALLE-PROGRAMME`, `DESKTOP-PROGRAMME-STARTEN` | E3, E4 |

### Vorgeschlagene Reihenfolge

Die vorgeschlagene Reihenfolge ist 1, 2, 3, 4, 5, nacheinander. Sie ist der Vorschlag für E6 und E11; festgelegt wird sie nach der Planung. Die Begründung:

- **Batch 1 zuerst.** Er enthält die Fehler, die heute den Alltag stören:
  - eine Fehlermeldung bei jedem Start (P22);
  - Tresor-Dateien lassen sich nicht öffnen (P21);
  - die Google-Drive-Anmeldung läuft ab (P26);
  - neue Laufwerke erscheinen erst nach einem Neustart (P10).
- **Batch 2 vor Batch 3.**
  - Batch 2 ist kleiner und liefert die kleinen Bildschirmpunkte P9, P2 und P7 früh aus.
  - P3 und P6 sind Grundlagen für P4.
- **Batch 3 vor Batch 4 und 5,** weil beide neue Rechte bringen, die auf die Geräteseite aus P18 gehören.
- **Batch 5 zuletzt,** weil er auf P3 und P6 aus Batch 2 aufbaut.

## Zeitpunkt der Agentenwahl

Nutzervorgabe vom 2026-10-24:

| Wann | Was wird gewählt |
|---|---|
| vor dem ersten Planungsschritt | noch keine Agenten für Detailplanung, Entwürfe oder Umsetzung |
| nach dem ersten Planungsschritt | die Agenten für Detailplanung und Entwürfe |
| nach der Planung | der endgültige Zuschnitt der Batches und die Umsetzungsagenten |
| nach der Umsetzung, vor den Tests | ein eigener Opus-Agent für die hochkritische Review der gesamten Umsetzung, wenn sonnet umgesetzt hat |

## Modelle je Rolle

Die Regeln stammen vom Nutzer:
- vom 2026-10-19: „sonnet für implementierungen die keine interpretation mehr zulassen, opus nur für sicherheitsrelevantes einer und design n anderer, gebatched die beiden also. der rest wenn möglich via sonnet.“
- vom 2026-10-17: „planning at xhigh, design too.“
- vom 2026-10-24: der eigene Opus-Agent für die Gesamtreview (Zitat oben).
- Kein Workflow-Werkzeug.

Die Tabelle legt fest, welches Modell eine Rolle bekommt, falls ein Batch sie braucht. Ob und wie oft eine Rolle besetzt wird, entscheidet der Zeitpunkt aus dem Abschnitt davor.

| Rolle | Modell | Effort | Aufgabe |
|---|---|---|---|
| Hauptagent | Opus 5.5 (diese Sitzung) | Sitzungsstandard | Planung, Entscheidungen ohne Sicherheitsbezug, Integration, Commits, Pushes, Remote-CI, kleine Korrekturen in der Fixschleife, Release |
| Bestand | haiku | medium | Code und Doku lesen. Ergebnis: eine Inventur mit `Datei:Zeile` über alle Oberflächen (Terminal mit JSON, Desktop-GUI, Android-GUI, MCP, IPC und Brücke, Doku) |
| Recherche | sonnet | medium | Primärquellen zu den Lücken, in zwei Runden |
| Entwurf | opus | xhigh | Gestaltung von Oberfläche oder Architektur, als Vorgabe ohne Auslegungsspielraum; höchstens einer je Batch |
| Sicherheit | opus | xhigh | Sicherheitsprüfung und Entscheidungen mit Sicherheitsbezug, dann Umsetzung des sicherheitskritischen Kerns; höchstens einer je Batch |
| Umsetzung | sonnet | high | Umsetzung nach dem Endplan. Jeder Umsetzer hat eine eigene Dateifläche, die sich mit keiner anderen überschneidet |
| Gesamtreview | opus | xhigh | hochkritische Review der gesamten Umsetzung des Batches vor der Suite, als eigener, frischer Agent; genau einer je Batch mit sonnet-Umsetzung |
| Suite | sonnet | high | die eine Task-Suite des Batches mit ihrem Workflow |

Ohne eigenen Agenten:
- **Zweite Recherche:** läuft im Rechercheagenten derselben Batch weiter, über SendMessage.
- **Fixschleife:** Kleine Korrekturen macht der Hauptagent selbst. Größere Korrekturen gehen an den zuständigen Umsetzer, über SendMessage. Das gilt für Befunde der Gesamtreview und der Suite.
- **Sicherheitsprüfung der Umsetzung:** Sie ist Teil der Gesamtreview. Eine zweite Schlussprüfung durch den Sicherheitsagenten entfällt.

Ein frischer Agent wird nur dann gestartet, wenn der fortzusetzende Agent beendet oder abgebrochen ist. Ausnahmen sind die Gesamtreview und die Suite, die immer ein frischer Agent übernimmt:
- Die Gesamtreview soll unabhängig von den Umsetzern urteilen.
- Bei der Suite gilt eine Annahme: Ein frischer Agent ist billiger, weil ein fortgesetzter Agent seinen ganzen Verlauf bei jeder Anfrage neu liest.

## Ablauf je Batch

1. **Erster Planungsschritt** nach der Planungsvorgehensweise der Arbeitsweise.
2. **Agentenwahl für Detailplanung und Entwürfe,** auf Grundlage des ersten Planungsschritts.
3. **Detailplanung, Entwürfe und Recherche** bis zum Endplan. Er enthält:
   - die Meilensteine mit prüfbarem Ergebnis;
   - die Zuordnung jedes Meilensteins zu einem Suite-Test.
4. **Festlegung von Zuschnitt und Umsetzungsagenten,** mit dem Dateibesitz je Umsetzer.
5. **Umsetzung:** Die Umsetzer arbeiten parallel; ein Sicherheitsagent setzt seinen Kern um, soweit der Batch einen hat.
6. **Gesamtreview** durch den eigenen Opus-Agenten. Danach werden ihre Befunde behoben.
7. **Abschluss:**
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
