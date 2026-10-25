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
Der Transportkern wird als zusammenhängender Vendorpatch und als Appadapter
bearbeitet. Die gemeinsame Bildreferenz-/Codec-/Ratenpipeline ist beauftragt;
Anmeldung und Fernfenster sind noch nicht besetzt.

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
| P29 Terminalkopplung mit tatsächlichem Austauschbesitzer | `discovery_pairing` | `gpt-6.1-sol`, `xhigh` | 8 |
| P28 Noq-/Iroh-Nutzdatensperre | `route_vendor` | `gpt-6.1-sol`, `xhigh` | 9 |
| P23 Host-/Daemon-/CLI-/Desktop-Kapazitätsstatus | `capacity_status` | `gpt-6.1-sol`, `medium` | 7 |
| P28 gemeinsamer Routeautomat und Appzulassung | `route_adapter` | `gpt-6.1-sol`, `xhigh` | 9 |
| P21 Desktop-/Android-Verwaltung der Öffnungsfreigaben | `vault_settings` | `gpt-6.1-sol`, `medium` | 7 |
| P11.1 gemeinsame physische optische Lesergrenze | `optical_reader` | `gpt-6.1-sol`, `high` | 8 |
| B3 statischer kritischer Batchreview | `review_batch3` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| P7/P27a/P30b–c/P31 Bildpipeline | `screen_pipeline` | `gpt-6.1-sol`, `xhigh` | 9 |
| P18d Zulassung und aktiver Rechteentzug am Host | `access_runtime` | `gpt-6.1-sol`, `high` | 8 |
| B1 statischer kritischer Batchreview | `review_batch1` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| P6/P9/P30 Viewerlebenszyklus und Skalierung | `viewer_lifecycle` | `gpt-6.1-sol`, `xhigh` | 9 |
| P32 Android-Medienorientierung | `media_preview` | `gpt-6.1-sol`, `low` | 6 |
| B1-R01/R05 historische Recoverybindung | `recovery_identity` | `gpt-6.1-sol`, `xhigh` | 9 |
| B1-R03 laufende Android-Providerfreigabe | `vault_provider_guard` | `gpt-6.1-sol`, `high` | 8 |
| P3 Appicons mit gebundener Identität | `app_icons` | `gpt-6.1-sol`, `xhigh` | 9 |
| B1-R04/R06 Providerfehlerklassifikation | `provider_errors` | `gpt-6.1-sol`, `medium` | 7 |

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

Eine erste Auswahl für den breiten P28-Repositoryauftrag lieferte
`no_feasible_model`. Der Vendorauftrag wurde auf die drei zusammenhängenden
Transportcrates begrenzt; für diesen Codierungsauftrag wählte das Profil
`coding` bei unveränderter Einstufung 9 Sol 6.1/xhigh. Der Appadapter besitzt
einen separaten belegten Auswahlaufruf desselben Codingprofils.
Auch die B1-/B3-Reviewselektoren meldeten bei Einstufung 9 im Repositoryprofil
`no_feasible_model`; die ausdrückliche Nutzervorgabe Astra/xhigh hat Vorrang
und wurde unverändert im Hostaufruf gesetzt.

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
Fundstelle, Auswirkung, Quelle und noch ungeprüfte Annahmen. Der erste Review
für B3/P13 ist dokumentiert; seine Korrekturen erhalten einen Nachreview.
Der B1-Erstreview fand sechs konkrete Defekte; ihre Korrekturen werden integriert. Der gezielte B3-Nachreview hat die sieben ursprünglichen und zwei zusätzlich gefundenen Statusfehler statisch als korrigiert bewertet. Beide Berichte ersetzen keine Remote-Abnahme. Die Reviews von B2 und
B4–B8 beginnen nach Integration ihres jeweiligen vollständigen Paketumfangs.

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
  ihre tatsächliche Locatoridentität erhalten. Wo exklusive Anlage, aber
  keine dauerhafte Objekt-ID belegt ist, erlaubt ausschließlich die eigene
  bestätigte Anlageantwort den laufenden Auftrag. Recovery bleibt dort
  unbestätigt; bestehende Verzeichnisse werden nicht adoptiert oder neu gezählt.
- P22: Aktiver Mounteintrag und inaktiver Recoverybesitzer wechseln atomar im
  vorhandenen Registrydokument. Rohspools werden vor dem Öffnen einer Engine
  gesichert; Originalcache und veröffentlichte Sicherung bleiben erhalten.
- P24: Teilscan-Metriken stammen aus tatsächlichen Besuchen vor Filterretention.
  Ohne vollständige Teilmessung wird kein alter erfolgreicher Teilbaum durch
  unbestätigte Daten oder errechnete Filterlistenlängen ersetzt.
- P23: Authentifizierte Hostberichte tragen Remote- und Cachekapazität getrennt
  durch Daemon, CLI und Desktop. Der tatsächliche Sessiontoken wird beim
  Schreiben des Snapshots erneut geprüft; ein Gerätebaum erhält keine Summe
  inkompatibler Providerquoten.
- P29/P13: Ein verlorenes Start-ACK beendet nicht den Besitz eines bereits
  gestarteten Austauschs. Der Coreauftrag hält den Operationslease bis zum
  tatsächlichen Ende; EOF/Abbruch markiert ausschließlich diesen Austausch.
- Agentprotokoll: Neue Kapazitäts-/Verzeichnisframes heben die bestehende
  exakte Handshakeversion von 11 auf 12. Die Deploycache-Identität enthält
  weiterhin Protokollversion und Binaryhash.
- P13: Die Launcherfähigkeit wird am geprüften Programm belegt, nicht allein
  an der Produktversionsnummer. Erstinstallation und Legacy-Rückkehr besitzen
  denselben Updatebesitzer wie verwaltete Updates. Der Linux-Releaseabschluss
  installiert bei vorhandener Desktop-App ein vollständiges Bundle ohne
  GUI-Start; CLI-only mischt keine abweichenden App-/CLI-Generationen.
- P28: Iroh, Noq und Noq-proto sind exakt an 1.3.0 und lokale Vendorpfade
  gebunden. Der Patch enthält eine Sendesperre vor Frameserialisierung und
  invalidiert bereits geplante I/O bei Policy-/Netzgenerationswechsel.
- P2: Das Android-Launcherwidget liefert bei Überschreiten der Ziehschwelle
  einmal sein vorhandenes Viewabbild. Erst dann wird das Original verborgen;
  ein fehlgeschlagener Snapshot lässt es sichtbar. Ende, Abbruch, Größenwechsel
  und Viewdetach lösen den Ziehzustand. Quelle ist der AndroidX-Vertrag für
  [Viewabbilder](https://android.googlesource.com/platform/frameworks/support/+/2fd71467c295a57331c2c23e66e3574230d209d3/core/core-ktx/src/main/java/androidx/core/view/View.kt),
  geprüft am 2026-10-25. Dieser Software-Canvasweg ist kein Beleg für identische
  Hardwareeffekte. Kotlin-Syntax ist statisch geparst; Gestenlaufzeit ist offen.
- P21/P14: Der CLI-Quellspool verschlüsselt Inhalte vor dem Schreiben mit dem
  vorhandenen Cryptomator-Chunkformat und einem nur im RAM gehaltenen Schlüssel
  je Datei. Fertige Ciphertextdateien halten keine offenen Dateihandles. Dadurch
  sind auch lokale Tresormounts abgedeckt, ohne aus ihrem Localbackend einen
  unverschlüsselten Spool abzuleiten. Namen im Spool sind laufende Nummern.
- B3: [Der erste Astra-Bericht](review-b3.md) enthält sieben belegte
  Befundgruppen. Korrekturen für Prozessidentität, Windows-Persistenz und
  Postcommit-Refresh wurden punktuell nachgelesen. Katalogzulassung,
  Mountretainer, stabile Starterpfade und Claude-Feldbesitz erhalten einen
  gezielten Nachreview nach Integration ihrer Korrekturen.
- B1: Der separate Astra-Review ist gestartet; der Befund ist noch offen.

Die Integrationsbefunde sind Quelltextbefunde. Praktische Funktionsnachweise,
die gemeinsame Remote-Suite, Versionserhöhung und Veröffentlichung stehen
noch aus; dafür wurde kein lokaler Build oder Test gestartet.
