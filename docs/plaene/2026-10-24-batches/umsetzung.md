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

Die Aufträge besitzen ausdrücklich zugeordnete Schreibflächen. Die erste
Gruppe bearbeitete B1, B3 und den unabhängigen Geometrieteil aus B4. Die zweite
Gruppe integriert B2 und B4 sowie die Recovery- und Rootgrenzen aus B1.
Abhängige Bildstrom-, Transport- und Anmeldepakete sind noch nicht besetzt.

| Ergebnis | Aufrufname | Selektor-Ergebnis | Einstufung |
|---|---|---|---|
| P10 Laufwerksereignisse und Snapshot | `drive_events` | `gpt-6.1-sol`, `medium` | 7 |
| P26 Accountbindung und Refresh | `cloud_auth` | `gpt-6.1-sol`, `xhigh` | 8 |
| P13 geprüfte MCP-Laufzeitgenerationen | `mcp_runtime` | `gpt-6.1-sol`, `xhigh` | 8 |
| P16/P17 Picker- und Dialoggeometrie | `dialog_geometry` | `gpt-6.1-sol`, `medium` | 6 |
| P23 Providerkapazität und Mountcache | `capacity` | `gpt-6.1-sol`, `medium` | 7 |
| P18d eingeschränkte Rechte | `access_v2` | `gpt-6.1-sol`, `xhigh` | 8 |
| P21 Rootpolicy und gehaltene Local-Adapter | `vault_open` | `gpt-6.1-sol`, `xhigh` | 8 |
| P18a–c/P19a–b/P20 Einstellungsintegration | `settings_integration` | `gpt-6.1-sol`, `xhigh` | 8 |
| P14 Ordnerkonflikte und dauerhafte Aliase | `folder_conflicts` | `gpt-6.1-sol`, `xhigh` | 8 |
| P14 Remoteprovider-Reservierung | `provider_reservations` | `gpt-6.1-sol`, `xhigh` | 8 |
| P24 gezielter Teilscan | `partial_rescan` | `gpt-6.1-sol`, `medium` | 7 |
| P22 inaktiver Recoverybestand | `offline_recovery` | `gpt-6.1-sol`, `xhigh` | 8 |
| P15 Ordneröffnung | `desktop_open` | `gpt-6.1-sol`, `medium` | 6 |

Belegt sind die expliziten Modell-/Effortparameter der genannten Hostaufrufe.
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

## Zusätzlicher Reviewauftrag

Am 2026-10-25 ergänzte der Nutzer wörtlich: „aufgrund des umfangs der aufgaben,
ohne dass das weiteres vorgehen blockt, will ich dass du danach n astra xhigh
die sachen kritisch reviewn lässt. per Batch“. Die Präzisierung lautet:
„also rein codeseitig und gegen quellen reviewen lässt“.

Jedes der acht fachlichen Pakete erhält nach Integration einen eigenen
kritischen Review mit `gpt-6-astra`, `xhigh`. Gegenstand sind der konkrete
Quellstand, seine Vertragsübergänge und die passenden Primärquellen mit
Versionsbezug. Diese Reviews führen keine Builds, Tests oder Laufzeitversuche
aus. Ihre Durchführung hält die autorisierte Arbeit an Folgebatches nicht an.
Bestätigte Befunde werden vor der gemeinsamen Remote-Suite behoben; der
vereinbarte abschließende Gesamtreview bleibt erhalten. Die Reviewer belegen
Fundstelle, Auswirkung, Quelle und noch ungeprüfte Annahmen. Bisher wurde noch
kein Batchreview gestartet, weil kein vollständiges fachliches Paket
abschließend integriert ist.

## Integrationsbefunde

- P26: Der gemeinsame Credentialrecord und die Tokenrotation sind integriert.
  Bei der Autorprüfung wurde die Sperre persistierter terminaler
  `ProviderError`-Zustände auf bereits gecachte Bearertokens ergänzt.
  `server_error` bleibt ein temporärer Providerfehler. Die Desktopformulare
  werden im gemeinsamen Einstellungsauftrag weitergeführt. Die Aussage zum
  tatsächlichen Google-Projektstatus bleibt `unconfirmed`.
- P10/P24: Offene Volumebindungen und partielle Scanworker erhalten dieselbe
  Identitäts-/Generationsgrenze. Kapazitätsabfragen laufen getrennt vom
  Topologieabgleich; ein blockierender Netzwerkpfad hält die Laufwerksliste
  nicht an.
- P13: Installer und Updatepfad erhalten denselben geprüften Helfereinstieg.
  Das Windows-Installationsskript beendet nicht mehr pauschal `se.exe`.
  Der Auslieferungsvertrag wächst um Windows-/Linux-Starter samt Hashes von
  20 auf 24 Assets; vorhandene Releasebytes wurden dadurch nicht verändert.
- P21: Der Klartextdialog darf keine App-Ende-Löschung versprechen.
  `shutdown.rs` erhält offene Bearbeitungen unter `open-temp` zur Recovery.
  Rootvertrauen und Klartextkopie bleiben getrennte Entscheidungen.
- P14: Eine Namensprobe plus Anlage ist keine exklusive Reservierung.
  Der neue Vertrag benötigt exklusive Anlage, stabile Objektidentität und
  atomaren NoReplace-Directory-Rename. Provider ohne belegte Primitive
  behalten ihre bisherigen Mergewege und melden die neue strikte Operation
  als technisch nicht unterstützt; eine objektbasierte Alternative muss
  ihre tatsächliche Locatoridentität erhalten.

Die Integrationsbefunde sind Quelltextbefunde. Praktische Funktionsnachweise,
die gemeinsame Remote-Suite, Versionserhöhung und Veröffentlichung stehen
noch aus; dafür wurde kein lokaler Build oder Test gestartet.
