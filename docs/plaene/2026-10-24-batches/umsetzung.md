# Umsetzung des freigegebenen Gesamtplans

Stand: 2026-10-26. Ausgangskandidat: `bea312ea79731d913a7dd3e79de2f5404401208a`.
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
bearbeitet. Die gemeinsame Bildreferenz-/Codec-/Ratenpipeline, Anmeldung,
Fernfenster, Programmstart und RDP besitzen abgegrenzte Umsetzungsaufträge.

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
| P1e authentifizierter TCP-Tunnel samt Bedienung/Consent | `tcp_transport` | `gpt-6.1-sol`, `xhigh` | 9 |
| P1b/c privilegierter Loginbroker und feste Helfer | `login_broker` | `gpt-6.1-sol`, `xhigh` | 9 |
| B1 gezielte Nachprüfung der Reviewkorrekturen | `review_b1_fixes` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| P5 kataloggebundener Programmstart | `app_launch` | `gpt-6.1-sol`, `xhigh` | 9 |
| P4 Hostvertrag für Fensterfamilien und Operationen | `window_family` | `gpt-6.1-sol`, `xhigh` | 9 |
| P1f/P4e vorhandene RDP-Clients und RemoteApp | `rdp_adapter` | `gpt-6.1-sol`, `xhigh` | 9 |
| B2 statischer kritischer Batchreview | `review_batch2` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B2 Dateirecovery mit dauerhaften Publikationsbelegen | `file_recovery` | `gpt-6.1-sol`, `xhigh` | 9 |
| B2 lokale Publikation und Quarantänenidentität | `copy_publication` | `gpt-6.1-sol`, `xhigh` | 9 |
| P6/P27 tatsächlicher Darstellungsbeleg beider Viewer | `frame_presented` | `gpt-6.1-sol`, `xhigh` | 9 |
| P14 Dateiidentität über Agent-/Daemon-/Share-RPC | `file_identity_rpc` | `gpt-6.1-sol`, `xhigh` | 9 |
| B4 statischer kritischer Batchreview | `review_batch4` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| P1f RDP-Abschluss beim GUI-Schließen/Update | `rdp_adapter` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B2 gezielter Nachreview der Recovery-/Readerkorrekturen | `review_b2_fixes` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| P5 Appkatalog-/Startoberflächen | `app_launch` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B2 Nachreviewkorrekturen am Datei-Recoveryjournal | `file_recovery` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B2 dauerhafte Inkarnations-/Publikationsbelege und Livebereinigung | `copy_recovery_proof` | `gpt-6.1-sol`, `xhigh` | 9 |
| P4 authentifizierte Host-/Worker-Fensterfamilien | `window_family` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| P1 Logininstallation, besessene IPC-Sitzungen und CLI/MCP | `login_broker` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B5 statischer kritischer Transportreview | `review_batch5` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| P30b tatsächliche Capture-/Codec-/Rendergrößen | `screen_pipeline` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| P5 lokale Erstfreigabe aus dem Hostkatalog | `app_launch` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| P1 Desktop-/Android-Loginrenderer und Hostaktivität | `login_broker` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B5 frische Pfadvalidierung und begrenzter Kandidatenersatz | `route_revalidation` | `gpt-6.1-sol`, `xhigh` | 9 |
| P5 OS-Prozessbeleg und P4-Fensterkorrelation | `app_launch` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| P4 Desktop-/Android-Familienviewer | `window_family` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| P4d Wayland-Serial und tatsächliche Android-Systemauswahl | `portal_source_binding` | `gpt-6.1-sol`, `xhigh` | 9 |
| B2 erneuter Review der Publikations-/Inkarnationsbelege | `review_b2_fixes` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B5 Nachreview aller sechs Transportkorrekturen | `review_batch5` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B2 optische Pakete, Stagebereinigung, Commitwiderruf und Hashabbruch | `copy_recovery_proof` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B5 unabhängiger Besitzerabbruch am lokalen Exec-IPC | `route_revalidation` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B5 finaler Nachreview des unabhängigen IPC-Abbruchs | `review_batch5` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| P4/P5 Nodepin und Layoutrevision über Screen-IPC | `route_revalidation` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B2 finaler Publikations-/Cleanup-Nachreview | `review_b2_fixes` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B2 Crossdevice-Fallback vor unsicherer Mutation | `copy_recovery_proof` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B6 statischer Review von Bildübertragung, Icons, Widget und Medienvorschau | `review_batch6` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B2 finaler Nachreview des Crossdevice-Fallbacks | `review_b2_fixes` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B8 statischer Review von Programmstart und Fensterfamilien | `review_batch8` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B6 tatsächliche Identität gepufferter Decoderbilder | `decoder_identity` | `gpt-6.1-sol`, `xhigh` | 9 |
| B6 belegte Android-Codec-Latenz und Fallback | `screen_pipeline` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B7 statischer Review von Maschinenlogin, TCP und RDP | `review_batch7` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B8 Familienkoordinaten und Freigabe gehaltener Eingaben | `window_family` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B6 Hostanschluss von Lossless, Sharpness und Fallbackdiagnose (72) | `screen_quality_policy` | `gpt-6.1-sol`, `xhigh` | 9 |
| B7 IPv6-Zielwahl und atomare TCP-Registrierung (73) | `route_revalidation` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B7 Installerrollback und bestätigte Dienstbereitschaft (74) | `login_broker` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B7 X11-Capture-/Inputgeometrie und Login-Kontext (75) | `window_family` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B8 abschließende Erstgrant-Zusammenführung (76) | `review_batch8` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B6 vollständige GOP-Identität im Android-Pullvertrag (77) | `decoder_identity` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B7 Nachreview der fünf Integrationskorrekturen (78) | `review_batch7` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B6 abschließender Decoder-/GOP-/Qualitätsnachreview (79) | `review_b6_final` | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B6 ausgehandelte Paket-/Queue-/Pullspeichergrenzen (80) | `decoder_identity` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |
| B8 Pixmapgrenze für X11-Familienmitglieder (81) | `review_batch8` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B6 Nachreview des vollständigen Paket-/IPC-/Decoderbudgets (83) | `review_b6_final` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |

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

Für den nach vollständiger Integration vorgeschriebenen frischen Gesamtreview
hat Auswahl 82 am 2026-10-26 `claude-opus-5-5` mit `xhigh` bestimmt
(Repositoryprofil, Einstufung 10, großer zusammenhängender Umfang). Der lokale
Claude-Client 2.1.293 ist verfügbar; ein tatsächliches Antwortmodell ist erst
nach seinem Start belegt. Dieser reine Review schreibt keinen Produktcode.
Sein vorbereiteter Starter beschränkt lokale Zugriffe auf eine isolierte
Quellkopie und bricht bei abweichendem Antwortmodell ab. Die Umsetzung bleibt
vollständig bei Codex. Der Gesamtreview ist noch nicht gestartet.

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
Der B1-Nachreview schließt seine sechs Befundgruppen einschließlich der
nachträglich gefundenen Journalauditkorrektur statisch. Der B3-Nachreview
schließt die sieben ursprünglichen und zwei zusätzlich gefundenen Statusfehler
statisch. Der B2-Nachreview schließt N1–N11 einschließlich des Crossdevice-
Stagefallbacks vor unklarer Mutation. B4 ist statisch geschlossen. B2 und B4
sind mit `d0138b4a` und `c5d3e899` auf `main` gepusht.
Der finale B5-Nachreview schließt R01–R06 einschließlich des unabhängigen
lokalen Exec-IPC-Abbruchs statisch. B6 korrigiert noch Decoder-/Latenzgrenzen.
B7 korrigiert die fünf Befunde zu X11-Geometrie, Installerrollback,
Dienstbereitschaft, IPv6-Loopback und Skriptdistribution. B8 schließt seine fünf
Befunde einschließlich der Erstgrant-Zusammenführung statisch.
Kein Bericht ersetzt
die gemeinsame Remote-Abnahme.

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
  20 auf zunächst 24 Assets; P1 ergänzt anschließend die beiden administrativen
  Skripte auf insgesamt 26. Vorhandene Releasebytes wurden nicht verändert.
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
- B3: [Astras Nachreview](review-b3.md) schließt die sieben ursprünglichen
  Befundgruppen und die nachträglich korrigierten Statuspfade statisch.
  Runtime-/Updategrundlage ist unter `16052ba5` auf `main` gepusht;
  Daemonretainer-/MCP-Laufzeitintegration bleibt Teil des aktiven Gesamtbatches.
- B1: [Astras Nachreview](review-b1.md) schließt R01–R06 statisch.
  R05-F1 zeigte einen schreibenden Offlineaudit versiegelter Journale.
  Die Korrektur ersetzt ihn durch einen rein lesenden Audit mit explizitem
  Unknown; auch diese Korrektur ist statisch nachgeprüft und geschlossen.
  Der Quellcheckpoint `7442afc0` ist auf `main` gepusht.
- B2: [Astras Review](review-b2.md) belegt doppelte Dateikopien und blockierte
  Datei-Move-Wiederaufnahme. Die Reparatur ergänzt dauerhafte Publikations- und
  Quarantänenbelege; der bisherige Größenvergleich ist dafür kein Nachweis.
  Der Wallpaperreader besitzt nun Hintergrundpermits je begrenztem Read/Seek,
  gebundene Medienprüfungen und Abbruch bei Austausch oder Ende der Ansicht.
  Der erneute Astra-Nachreview bestätigt die korrigierten Journalneuanlagen,
  Vorpublikationszustände bei vorhandenem Ziel, Drive-Exportquellen und
  Ziel-Dauerhaftigkeit. Der finale Nachreview schließt N1–N11 statisch:
  optische Mitglieder gemischter Pakete, Live-Stagebereinigung, dauerhafter
  Commitwiderruf, Btrfs-Kennung, Hashabbruch und Crossdevice-Fallback sind
  korrigiert. Der Crossdevice-Beleg entsteht vor dem Rename; ein unklarer
  Fehler nach Dispatch erhält weiterhin Quarantäne und Wiederaufnahmebeleg.
  Noch kein Laufzeitnachweis.
- P32: Medienzustand und adaptive Anordnung sind in `20caeb5e` gepusht.
  Die Nutzerpräzisierung legt Android-Systemeinstellungen fest; `3c24152a`
  entfernt die angenommene Appauswahl und ist ebenfalls auf `main` gepusht.
  `126f9325` verhindert einen unzulässigen MediaPlayer-Pauseaufruf im Zustand
  Prepared; der B6-Nachreview bestätigt die Korrektur statisch. Dieser Commit
  ist ebenfalls auf `main` gepusht.
- P6/P3: Capturepause durchläuft inzwischen einen nichtblockierenden
  Quellenübergang. Sichtbarkeitsbestätigung folgt erst nach dessen Abschluss.
  Iconabrufe und Quellenlisten sind an Wire, IPC und beide Viewer angebunden.
  Ein Transport-ACK belegt keine Darstellung. Die neue v2-Referenzgrenze
  erhält deshalb einen gesonderten, an die dargestellte GOP-/Frame-ID
  gebundenen Beleg in Desktop und Android. Die Integration ist geschrieben.
- P28: Auch ein Wechsel zwischen zwei konkreten Pfaden derselben Routenklasse
  erhöht die Unterbrechungsgeneration, damit Bildreferenzen erneuert werden.
- Architekturausnahme: `share/mod.rs` überschreitet als bestehende
  Registrierungsdatei mit 569 Zeilen die Grenze von 500. Die Ergänzungen für
  Routen, TCP, Login und Apps enthalten dort ausschließlich Modulregistrierungen
  und schmale Reexports; die neue Implementierung liegt in separaten Dateien
  unter der vorgegebenen Größenbegrenzung. Die Ausnahme betrifft allein diese
  vorhandene Fassade, nicht neue Verhaltensmodule.

Die Integrationsbefunde sind Quelltextbefunde. Praktische Funktionsnachweise,
die gemeinsame Remote-Suite, Versionserhöhung und Veröffentlichung stehen
noch aus; dafür wurde kein lokaler Build oder Test gestartet.

- B4-Nachreviewkorrekturen: falsche Elternmodulpfade der Rechte-, Vault-,
  Screen-Codec- und Protokolldialoge verwenden jetzt die App-Wurzel. Der
  Rechteeditor verändert ausschließlich seine vier sichtbaren Basisfähigkeiten;
  Appstart, Anmeldezugang und TCP bleiben bei Presets, Rückfrageschalter und
  Raumsammeländerung erhalten. Schmale Radios erhalten den Fähigkeitsnamen als
  zugängliche Beschriftung. Android zeigt die workerbestimmte Erreichbarkeit je
  exaktem Beziehungstarget und aktualisiert bei Status-/Friständerung. Der
  Dialoggeometriestore führt Lesen/Mergen/Ersetzen unter einem prozessübergreifenden
  privaten Filelock aus. Der abgeschlossene Astra-Nachreview schließt alle fünf
  Befundgruppen statisch; Laufzeit und Kompilierung bleiben ungeprüft.
- P5-IPC: Appicons sind mit App-ID, Inhalts-Hash und Katalogrevision gebunden;
  die 2-MiB-IPC-Grenze bleibt maßgeblich. `apps_icon` ist in der Daemonfassade
  exportiert. Native Geräteseiten, Desktopfläche, Android-Geräteseite und Launcher
  erreichen den Appkatalog. Lokale Erstfreigaben benutzen konkrete AppIDs aus dem
  lokalen Katalog; CLI/MCP sind an denselben tokengeprüften Weg angebunden.
  Die P4-Korrelation verbindet tatsächliche OS-Prozessinkarnationen mit dem
  atomar gespeicherten Familienselektor; die Autorintegration ist beendet und
  der separate B8-Review läuft.
- P1f-Abschluss: Der MCP-Host drainiert besessene RDP-Starts/Sitzungen vor dem
  Runtimeende. Die GUI besitzt dieselbe automatisch fortgesetzte Bereinigung für
  Schließen und Update; der Quellfix ist geschrieben. Loginviewer,
  Installationsoberflächen und P4-Workerintegration sind angeschlossen;
  ihre statischen Batchreviews laufen vor der Remote-Suite.
- P1: Maschinen-Login-IPC, CLI und MCP sind registriert. Beim Ende erhalten Daemon
  und MCP ihre Runtime, bis ihre Loginbesitzer bestätigt geschlossen sind.
  Desktop-/Androidrenderer und der lokale Hostaktivitätsnachweis sind
  angeschlossen. Der separate B7-Review umfasst auch diese Grenzen.
- B5: Der Nachreview bestätigt die korrigierte Generationenfrische, Pfadkapazität,
  Vorzulassung, Revalidierung und aktiven Screenfristen. Das unabhängige innere
  Exec-Cancelsignal und der unabhängige Besitzerabbruch über einen eigenen
  lokalen Kontrollkanal sind geschrieben. Der finale Nachreview schließt
  R01–R06 statisch. Keine Laufzeitbestätigung.
- P30b: Geöffnete Quellen melden ihre tatsächlichen FrameLimits. Workerantworten
  transportieren diese Grenzen; Paketbudgets werden daraus abgeleitet. Pipeline
  und Downsampler beachten Source-, Codec-Pixel- und ausgehandelte Viewergrenzen.
  Der Familiencompositor prüft zusätzlich die Summe seiner gehaltenen Memberpixel
  vor dem Öffnen neuer Capturequellen. Altschnittstellen behalten LEGACY.
- P4d: Linux bindet PipeWire an die vom Portal gewährte Serial und Sitzung;
  Disconnect invalidiert diese Quelle endgültig. Android weist unbelegte
  paketgebundene Captureanfragen ab und benennt tatsächliche Projektionen als
  Systemauswahl. Beide Quellenmenüs erhalten diese Unterscheidung. Statischer
  Quellenabgleich ist vorhanden; praktische Plattformnachweise stehen aus.

- P4/P5: Der Host kennzeichnet jedes Familien-GOP mit der Layoutrevision der
  tatsächlich aufgenommenen CaptureBoundary. Desktop und Android verknüpfen
  Darstellung und Eingabefreigabe damit. Die Screen-IPC bewahrt zusätzlich den
  authentifizierten Nodepin des Appstartbelegs; fehlende Altversionunterstützung
  führt zur ausdrücklichen Abweisung. Der Familienbesitzer umfasst auch die
  Beziehungsart, damit Direct- und Raumrechte getrennt bleiben.
- P1/P4/P5: Desktop-Geräteseiten registrieren Login- und Familienansicht samt
  Appfensteröffner. Android führt Maschinenanmeldung, Familienauswahl und
  bestätigten Appstart über konkrete Dialogrouten mit vollständigen Zielbelegen.
  In der Familienansicht bleiben Quellenwechsel und automatischer Neustart aus;
  lokale Menüs geben keine Berührungen an den Host weiter.

- B6-Reviewkorrekturen: `gop_stream.rs` verlangt passende Vertragspräsenz und
  identische ausgehandelte Grenzen; v1 kann keinen v2-Speichervertrag umgehen.
  X11 verrechnet Opaque-Hinweise nur innerhalb der erneut abgefragten
  XShape-Boundingregion. Grundlage, geprüft 2026-10-25:
  [EWMH 5.18](https://specifications.freedesktop.org/wm/latest/ar01s05.html),
  [x11rb 0.13.2 Shape](https://docs.rs/x11rb/0.13.2/x11rb/protocol/shape/trait.ConnectionExt.html)
  und die lokal gepinnten Bindings. Unbekannte Shapes zählen nicht als
  verdeckende Fläche. DXGI dreht 90/180/270-Grad-Aufnahmen aufrecht und
  erneuert bei Rotation die Bildreferenz. Der Legacy-Android-Surfaceweg
  behält seine bisherige Grenze; der v2-Weg verwendet das belegte Heapbudget.
  MF und OpenH264 erhalten jetzt die tatsächliche Ausgabeidentität;
  verzögerte Ausgaben übernehmen keine neuere Präsentationsepoche. Die
  Qualitätsflags erreichen den Host, und ein tatsächlicher Encoderfallback
  bleibt als nichtterminale Meldung sichtbar. Android bindet den ersten
  Hybridcapture an die bestätigte konkrete Encoderinstanz. Ein ungeeigneter
  Codec führt in den dauerhaft gehaltenen verlustfreien Modus; spätere
  Qualitätsänderungen bewahren die Nutzerpräferenzen. Der Pullvertrag trägt
  vollständige 64-Bit-GOP- und Frame-IDs. Diese Übergänge sind im Nachreview;
  Typ- und Laufzeitnachweise stehen weiterhin aus.
- B6-Ressourcengrenze: Der Nachreview belegt, dass `PullQueue::push` trotz
  eines kleineren v2-Angebots mindestens 64 MiB zulässt und der Kotlin-Pull
  nur die globale Bildgrenze verwendet. Korrekturziel ist ein durchgängiges,
  aus `working_bytes` abgeleitetes Paket-, Queue- und Pullbudget einschließlich
  der tatsächlich gleichzeitig gehaltenen Bild-/Paketkopien. Betroffen sind
  `frame_budget.rs`, `frame_size_caps.rs`, GOP-/Recordreader, Android-Senke
  und Kotlin-Decoder. LEGACY behält seinen bisherigen Vertrag. Erwarteter
  Nachweis in der gemeinsamen Suite: Kleine v2-Verträge weisen zu große
  Records vor deren Allokation zurück; zulässige verlustfreie Frames,
  vollständige GOP-/Frame-Identität und Legacyübertragung bleiben möglich.
  Konkrete Pufferrechnung und Korrektur sind noch offen.
- B8-Reviewkorrekturen: Viewer verwenden die tatsächlich vorhandenen
  HostFrame-Varianten. Bestätigte Appstarts vergleichen den inneren Window-ID-
  Beleg mit dem korrekt typisierten beziehungsweise präfixierten Selektor.
  Windows-Capture-/Eingabekoordinaten, Freigabe gehaltener Eingaben bei
  beobachtetem Fokuswechsel und Androids unklarer Startausgang nach Dispatch
  sind korrigiert und statisch nachgeprüft. Ein vollständig zwischen zwei
  Abfragen liegender Fokuszyklus ist nicht als beobachtet belegt.

- B7-Distributionskorrektur: Der bestehende Releasevertrag enthält jetzt
  zusätzlich `install-login-service.ps1` und `install-login-service.sh`
  (26 Assets). NSIS bettet das PowerShell-Skript ein; die Kandidatenprüfung
  vergleicht dessen Bytes mit dem veröffentlichten Quellasset. Die normale
  Appinstallation aktiviert keinen privilegierten Login-Dienst. README und
  RELEASING beschreiben den expliziten Administrationsweg.

- B5-Quellstand: `ed83a910` ist auf `main` gepusht. Der Commit enthält die
  Routenfreigabe, den unabhängigen Exec-Besitzerabbruch und die vollständig
  gepinnten Vendorpakete Iroh/Noq/Noq-proto 1.3.0. Gemeinsame Modulregistrierungen
  und plattformübergreifende B6/B7/B8-Anschlüsse folgen im Integrationscommit;
  dieser Entwicklungscheckpoint ist kein veröffentlichter Kandidat.

- B8-Appstart: `8d970a63` ist auf `main` gepusht. Der abschließende Erstgrant-
  Review bestätigt die Erhaltung bestehender Rechte, Policy-/Dateirevisions-CAS
  und Identitätspins. Ein Appgrant verleiht keine Login-Maschinenrechte.
- B7-Nachreview: Installerrollback, bestätigte lokale Dienstbereitschaft,
  getrennte IPv4-/IPv6-Autorisierung und administrative Skriptdistribution sind
  statisch geschlossen. `.gitattributes` bindet beide Quellassets an LF.
  Für TCP bleiben FIN als möglicher Halbschluss und gepufferte Begrüßungen
  erhalten; ein Socketstatus belegt keine künftige Anwendungsbereitschaft.
- B7-Geometrieanschluss: Main ergänzt den zur Präsentation gehörenden
  Quellenzähler im Pipelineactor sowie terminale Helper-/Brokerbehandlung.
  Die OS-Adapter binden Aufnahme und Input an dieselbe bestätigte Geometrie.
  Ein beobachteter Wechsel beendet den Login-Kontext einschließlich alter
  Darstellungsbelege. Normale v1-X11-Betrachtung bleibt bei Geometriewechsel
  möglich; erneute Steuerung verlangt dann ausdrücklich eine neue Verbindung.
  Umsetzung und statischer Nachreview dieser Grenze sind abgeschlossen;
  tatsächliches OS-Verhalten bleibt bis zur Remoteprüfung unbestätigt.
- B7-Quellstand: `1e6f77b1` ist auf `main` gepusht. Enthalten sind Loginbroker,
  Gateway, OS-Dienste, Installer, TCP-/RDP-Verträge und ihre spezifischen
  Oberflächen. Gemeinsame Screen-/Worker-/Rootregistrierungen folgen mit der
  verbleibenden B6/B8-Integration. B7-01 bis B7-06 und B7-R1 sind statisch
  geschlossen. Dieser Entwicklungscheckpoint ist keine Veröffentlichung.
- B8-Pixmapgrenze: Der gemeinsame Familienöffner verwendete bisher denselben
  X11-Quellweg wie eine normale rohe Fensterauswahl. Dessen dokumentierter
  Rootcrop-Fallback kann überdeckende Fremdfenster zeigen. Der P4-Familienweg
  erhält deshalb eine eigene typisierte Öffnungsgrenze: ausschließlich
  Mitgliedspixmap/WGC, bei fehlendem oder verlorenem Beleg ein konkreter Fehler.
  Der normale Raw-Windowpfad bleibt erhalten. Betroffene Module sind
  `host_backend.rs`, `family_backend.rs` sowie Linux-/Windows-Hostadapter und
  `x11_capture.rs`. Erwarteter gemeinsamer Nachweis: Kein Familienbild aus
  Rootpixeln bei fehlendem Compositor oder Pixmapfehler; normale Fensterauswahl
  und Windows-Familien bleiben erreichbar. Korrektur und statischer Nachreview
  sind abgeschlossen; ein wirklicher Compositorverlust wurde nicht ausgeführt.
- B8-Familienquellstand: `0c9d791d` ist auf `main` gepusht. Enthalten sind der
  Familienvertrag, OS-Mitgliedsidentitäten, Komposition und die spezifischen
  Desktop-/Android-/CLI-/MCP-/IPC-Oberflächen. Die gemeinsamen Backend- und
  Modulanschlüsse folgen mit B6. Der Commit ist keine Veröffentlichung.
- B6-Qualitätseinstellungen: Full und Preview halten Bildgröße und Bildrate
  getrennt. Ein Android-Codecfehler setzt den verlustfreien Modus anhand der
  aktuellen Moduswahl; der Fallback sendet keine alten Größen-/Bildratenwerte
  zurück. R3, R4 und R8 sind statisch geschlossen; R7 bleibt bis zum Abschluss
  der Speicherbudgetkorrektur offen.
- B6-Workerbudget: Der Pipe-Leser erhält je serialisierter PRODUCE-Anfrage ein
  Budgetticket aus deren tatsächlich vereinbartem Framevertrag. Das Ticket
  erreicht den Leser vor dem Antwort-Read; ein wartender Leser verwendet damit
  kein altes Budget. Die Quellfähigkeiten im Reference-Header ersetzen diesen
  Sitzungsvertrag nicht. `host_worker_reader.rs` kapselt den begrenzten
  Ticket-/Antwortkanal, `host_worker_client.rs` bindet ihn an die Anfrage.
  Erwarteter Nachweis in der gemeinsamen Suite: Ein Budgetwechsel gilt bereits
  vor Allokation der nächsten Antwort; normale Verwaltungsantworten bleiben
  lesbar. Der abschließende B6-Nachreview umfasst diesen Main-Anschluss.
- B6-IPC-Budget: Die Daemonweiterleitung besitzt zusätzlich acht wartende
  Recordslots und temporäre Encode-/Read-/Decodekopien. Diese Grenze ist in die
  gemeinsame Speicherrechnung aufgenommen. `ScreenStream` hält den tatsächlich
  Accepted-Vertrag; `read_relay_budgeted` prüft die daraus folgende
  Bildrecordgrenze vor der Payloadallokation. Ein fehlendes Accepted, doppelte
  Annahme oder geänderte Vertragsform/Limits liefert einen Protokollfehler.
  Legacy bleibt bei ausdrücklich ohne v2 angenommenen Sitzungen erhalten;
  Kontrollnachrichten behalten ihre getrennte bisherige Protokollgrenze.
- B6-Produceranschluss: Der Worker prüft kodierte Frames vor der
  Reply-Serialisierung mit demselben `validate_encoded_budget` wie die
  In-Process-Pipeline. Ein Budgetfehler liefert reguläres `EncoderFailed`;
  derselbe lebende Worker kann daraufhin auf Tiles wechseln. Ein bereits
  abgebrochener Pipe-Reader wird nicht als vermeintlich nutzbarer Fallback
  ausgegeben. Ungültige IPC-Records bleiben terminale Protokollfehler.
- B6-Android-Fallback nach Queue-Reset: Die native Warteschlange kann nach einer
  Sekunde einen neuen GOP anfordern, während der Hybriddecoder bis zu zwei
  Sekunden auf seine Ausgabe wartet. Die verspätete Unavailable-Meldung darf
  deshalb einen noch bekannten vollständigen GOP der aktiven Sitzung betreffen.
  `ScreenSession` fordert einmalig den sitzungsweiten Losslessfallback an,
  entzieht der aktuellen GOP den Darstellungsbeleg und verwirft weitere
  H264-Darstellungen. Geschlossene/beendete Sitzungen und unbekannte GOPs lösen
  keinen Moduswechsel aus. Astra hat den konkreten Patch statisch nachgelesen;
  die gemeinsame Remote-Suite muss den Ereigniswechsel noch ausführen.
- B6-Pullfixtures: Die bestehenden Native- und Android-Decoderfixtures verwenden
  nun den 16-Byte-Kopf mit anschließend vollständiger GOP- und Frame-ID.
  Unmittelbare PullQueue-Layoutfälle behalten ihren rohen Payloadvertrag.
  Die Sessioncache-Erwartungen benutzen vollständige `Long`-IDs. Diese
  Vertragsanpassung ist noch kein ausgeführter Funktionsnachweis.
