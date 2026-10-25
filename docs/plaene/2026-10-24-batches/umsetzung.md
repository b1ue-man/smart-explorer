# Umsetzung des freigegebenen Gesamtplans

Stand: 2026-10-25. Ausgangskandidat: `bea312ea79731d913a7dd3e79de2f5404401208a`.
Die offene Produktarbeit bleibt ausschließlich in [TODO](../../TODO.md) geführt.
Dieses Dokument hält Ausführungsentscheidungen und Integrationsbelege fest.

## Autorisierung und Umfang

Der Nutzer hat nach Abschluss der Planung am 2026-10-25 beauftragt:
„gut. ich hätte gerne, dass du die umsetzung managest. nutze gerne die
modellselektion von der arbeitsweise, aber die umsetzung läuft komplett über
codex, über credits. das wird nicht berücksichtigt von dem skript also falls
es dir nie codex agenten gibt hol es dir lokal und pass es an.“

Die frühere Planungsgrenze ist damit aufgehoben. Alle acht Pakete aus
[batches.md](batches.md), [vertraege.md](vertraege.md) und
[agentenvertrag.md](agentenvertrag.md) bilden einen gemeinsamen Umfang.
Die Planentscheidungen und Abnahmesignale bleiben gültig. Historische Sätze
über einen künftigen Umsetzungsauftrag beschreiben den vorherigen Stand.
Ausgeschlossen bleiben P8, P11 Stufen 2/3, P12, P25 sowie E7/E8/E13.

Nach vollständiger Integration und der vorgegebenen Gesamtreview folgt genau
eine gemeinsame Remote-Suite mit ihren notwendigen Fixwiederholungen und ein
terminaler vollständiger Remote-Release. Es gibt keine lokalen Builds oder
Tests, keine Releases je Paket und keine Versuche mit produktiven Nutzerkonten.

## Ausführung und Modellwahl

Die ersten vier Aufträge besitzen unabhängige Schreibflächen. Sie bearbeiten
B1, B3 und den unabhängigen Geometrieteil aus B4. Spätere fachliche
Abhängigkeiten werden dadurch nicht vorweggenommen.

| Ergebnis | Aufrufname | Selektor-Ergebnis | Einstufung |
|---|---|---|---|
| P10 Laufwerksereignisse und Snapshot | `drive_events` | `gpt-6.1-sol`, `medium` | 7 |
| P26 Accountbindung und Refresh | `cloud_auth` | `gpt-6.1-sol`, `xhigh` | 8 |
| P13 geprüfte MCP-Laufzeitgenerationen | `mcp_runtime` | `gpt-6.1-sol`, `xhigh` | 8 |
| P16/P17 Picker- und Dialoggeometrie | `dialog_geometry` | `gpt-6.1-sol`, `medium` | 6 |

Belegt sind die expliziten Modell-/Effortparameter der vier Hostaufrufe.
Antwortmodell-Metadaten sind über das verwendete Collaboration-Werkzeug nicht
verfügbar. Gewählt wurde ausschließlich unter Codex-Kandidaten auf dem
tatsächlich verfügbaren Collaboration-Host, ohne die Aboquote eines getrennten
lokalen CLI-Kontos als Sperre. Der Selektor liefert damit passende Kandidaten;
eine lokale Skriptänderung war nicht erforderlich. Credits-Saldo und konkrete
Hostabrechnung sind nicht einsehbar. Die offizielle
[Preisdokumentation](https://learn.chatgpt.com/docs/pricing) bestätigt am
2026-10-25 die Fortsetzung über verfügbare Credits nach Verbrauch enthaltener
Kontingente. API-Preise werden nicht als Creditsverbrauch ausgegeben.

Leseflächen, Schreibflächen, Verbote, Ergebnismerkmale und Berichtsanforderungen
sind in den konkreten Agentenaufträgen festgehalten. Gemeinsame Registrierungen,
Abhängigkeiten, Suite, CI, Commits, Pushes und Releaseintegration liegen beim
Hauptagenten. Beim Start meldete die GitHub-API keine laufende/wartende Pipeline.

## Bestandsentscheidungen während der Umsetzung

- P10: `App.drive_info` versorgt Sidebar, Picker, Analyse, Startseite,
  Aufräumen und Export; `App.drives` versorgt Schnellsuche, Index und Watcher.
  Der Gerätebaum projiziert Direct-/Room-Peers. Lokale Volumeereignisse dürfen
  diese Peeridentität nicht umdeuten.
- P26: `gdrive/core/auth.rs::refresh_for_account` prüft bereits den bestätigten
  Accountkey. Die koordinierte Persistenz erhält diese Grenze.
- P13: Formatwahrendes Codex-TOML verwendet den bereits transitiv gebundenen
  `toml_edit`-Stand `0.25.11+spec-1.1.0`, MSRV 1.85, MIT/Apache-2.0. Dessen
  Registryquellen belegen `DocumentMut`, Parse und Display. Keine 0.22-Nebenlinie.
- P16: Die alte Berechnung besitzt eine Mindestlistenfläche von 80 pt in einer
  horizontalen Zeile. **Implementierungsannahme:** Der betroffene initiale
  Fehlerfall erhält 160 pt. Ein höheres tatsächlich gemessenes Altmaß wird in
  der gemeinsamen Suite gegen V7 bewertet. 560 pt Außenhöhe beweist keine
  nutzbare Listenhöhe.
- P17: Dedizierter privater Geometriestore mit festen Dialogtyp-IDs und
  numerischer Geometrie. Kein Persistieren sämtlicher egui-Widgetzustände nur
  für Fenstergrößen; Texte und geheime Pfade gehören nicht in diesen Store.

Noch liegt kein integrierter oder praktisch bestätigter Produktmeilenstein
vor. Suite, Versionserhöhung und Veröffentlichung wurden nicht gestartet.
