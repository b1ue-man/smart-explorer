# Nächste Läufe: Batches und Modelle (2026-10-24)

**Status: vorläufiger Vorschlag.** Nichts ist begonnen. Der Zuschnitt unten ist ein erster Vorschlag. Die Batches werden erst nach der Planung festgelegt, die nach der Planungsvorgehensweise der Arbeitsweise läuft. Erst danach werden auch die Umsetzungsagenten zugeteilt (Abschnitt „Vorgehen und Agenten“).

## Auftrag

Nutzer, 2026-10-24:
- „fass die aufgaben in vernünftige batches zusammen und strukturiere sie sauber. danach wähle die entsprechenden models aus. das ziel im prinzip bleibt so wenig agents wie möglich. so viele wie nötig.“
- Korrektur am selben Tag: „für planung gibt es eine bestimmte vorgehensweise in der arbeitsweise. der ist eigentlich zu volgen und erst danach kann man die umsetzungsagenten und batches wirklich zuteilen.“ Außerdem: „agentenwahl zur umsetzung erst nach der planung, agentenwahl zur detailplanung oder entwürfe erstellen erst nach dek ersten planungssvhritt. wenn wir umsetzung via sonnet machen, was an sich ok ist, will ich auch einen separaten opus am ende zur review der gesamtumsetzung vor tests, zur hoch kritisvhen review.“
- Danach: „noch nicht starten“ und „wat, das war definitiv nicht in diesem chat. nutz die arbeitsweise“.

Grundlage sind zwei Machbarkeitsnotizen:
- `docs/plaene/2026-10-22-fernzugriff-desktop/machbarkeit.md`: Punkte P1 bis P9, Entscheidungen E1 bis E6;
- `docs/plaene/2026-10-22-laufwerke-medien-agenten/machbarkeit.md`: Punkte P10 bis P26, Entscheidungen E9 bis E13.

## Umfang

**In den Batches** (27 Punkte): P1, P2, P3, P4, P5, P6, P7, P9, P10, P11 Stufe 1, P13, P14, P15, P16, P17, P18, P19, P20, P21, P22, P23, P24, P26, P27, P28, P29, P30. P27 bis P30 stehen im Nachtrag vom 2026-10-24 der Notiz zu Fernzugriff und Desktop.

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
| 2 | Bildschirm-Grundlagen | P2, P3, P6, P7, P9, P27, P28, P30 | `LAUNCHER-WIDGET-MOVE`, `DESKTOP-APP-ICONS`, `SCREEN-NUR-SICHTBAR`, `SCREEN-HANDY-BILD`, `SCREEN-VOLLBILD-TASTE`, `SCREEN-BITRATE`, `DIREKT-ZUERST`, `SCREEN-SCHAERFE` | keine aus E1 bis E6 |
| 3 | Fenster und Einstellungen | P15, P16, P17, P18, P19, P20, P29 | `DESKTOP-ORDNER-EXPLORER`, `AUSWAHL-HOEHE`, `FENSTER-GROESSE`, `EINSTELLUNGEN-GERAETE`, `EINSTELLUNGEN-TECHNIK`, `EINSTELLUNGEN-AUSRICHTUNG`, `CLI-PIN-KOPPLUNG` | E12 |
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

## Vorgehen und Agenten

Dieses Dokument legt keine Agenten, Modelle, Rollen oder Abläufe fest. Das regelt die Arbeitsweise, die der Lauf lädt:
- **zentral**, Abschnitt „Agenten, Modelle und Orchestrierung“: wann Agenten gewählt werden, wie Modell und Effort über `modellwahl` bestimmt werden, die Gesamtreview vor den Tests und die Form der Agentenaufträge;
- **programmierung**: die Planstufen A bis D und der Schritt „Delegation und Zusammenführung“.

Die Nutzervorgaben vom 2026-10-24 sind dort seit diesem Tag als Pending-Änderungen eingetragen. Ihr Wortlaut:
- „agentenwahl zur umsetzung erst nach der planung, agentenwahl zur detailplanung oder entwürfe erstellen erst nach dek ersten planungssvhritt.“
- „wenn wir umsetzung via sonnet machen, was an sich ok ist, will ich auch einen separaten opus am ende zur review der gesamtumsetzung vor tests, zur hoch kritisvhen review.“

Frühere Nutzervorgabe zu Modellen, vom 2026-10-19: „sonnet für implementierungen die keine interpretation mehr zulassen, opus nut für sicherheitsrelevantes einer und design n anderer, gebatched die beiden also. der rest wenn möglich via sonnet.“

**Offener Konflikt:** Er wird in der Planung des ersten Batches geklärt.
- `AGENTS.md`, Abschnitt „subagent scope and RAM budget“, verlangt im Auftrag an einen Subagenten auch das Abnahmezeichen und das Berichtsformat.
- zentral, „Aufträge an Agenten: nur Produktionsauftrag, nie Vorgehen“, verbietet im Auftrag jede Dokumentations- und Berichtsform.
- Vereinbar sind beide bei den Dateiflächen: Sie gehören nach zentral als „potentiell betroffene Dateien“ in den Auftrag.

## Releases

- Jeder Batch endet mit genau einem Release.
- Annahme: Ohne ein dazwischenliegendes Release sind das die Versionen 0.5.185 bis 0.5.189. Heute gilt 0.5.184 (`native/Cargo.toml`, `release-native/update-feed/version.txt`).
