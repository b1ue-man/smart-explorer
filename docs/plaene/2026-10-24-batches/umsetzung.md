# Umsetzung des freigegebenen Gesamtplans

Stand: 2026-10-27, abgeschlossen mit
[v0.5.185](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.185).
Ausgangskandidat: `bea312ea79731d913a7dd3e79de2f5404401208a`.
Die offene Produktarbeit bleibt ausschließlich in [TODO](../../TODO.md) geführt.
Dieses Dokument hält Ausführungsentscheidungen und Integrationsbelege fest.
Die folgenden Zwischenstände sind historisch; der Abschnitt „Release 0.5.185
abgeschlossen“ enthält den Abschlussbeleg und die fehlende erneute
Funktionsabnahme nach dem unmittelbaren Releaseauftrag.

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
| B6 Android-Skalierung und vollständige Bildbesitzerbilanz (84) | `decoder_identity` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |


| B1 Recovery-/Root-/Drive-/OAuth-Fixtures | `storage_fixtures` | `gpt-6-astra`, `medium` | 8 |
| B3 Runtime-/Profil-Fixtures | `launcher_fixtures` | `gpt-6-astra`, `medium` | 8 |
| B7/B8 Login-/RDP-/Familien-/Start-Fixtures | `session_fixtures` | `gpt-6-astra`, `high` | 9 |
| B5/B7 Route-/TCP-/Exec-Fixtures | `transport_fixtures` | `gpt-6-astra`, `high` | 9 |
| B6/P32 Android-Präsentations-/Medien-Fixtures | `android_fixtures` | `gpt-6-astra`, `medium` | 8 |
| B4 und Partial-Scan-/Ordner-Fixtures | `ui_fixtures` | `gpt-6-astra`, `medium` | 8 |
| B1 tatsächliche Recovery-Ownership-/Rootregistry-Anschlüsse | `storage_fixtures` (Folgeauftrag) | `gpt-6-astra`, `medium` | 8 |
| B6 Rekonstruktion, Parserbudget und Referenz-Fixtures | `screen_fixtures` | `gpt-6-astra`, `high` | 9 |
| B6 Icon-/Sendebudget-/Sichtbarkeits-/Exit-Fixtures | `ui_fixtures` (Folgeauftrag) | `gpt-6-astra`, `medium` | 8 |
| B6-R9 kritischer 4K-Vertragsabgleich | `review_b6_final` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `xhigh` | 9 |
| B6-R9 durchgängige größere v2-Ressourcenklasse | `screen_4k_admission` | `gpt-6-astra`, `high` | 9 |
| B6-R9/R10 Rendererbesitz und Sitzungswechsel, Detailplan | `renderer_retirement_plan` | Nutzervorgabe `gpt-6-astra`, `max` | 9 |
| B6-R9 cgroup-Wurzel, Abbruchbesitz und Snapshotbindung | `screen_4k_admission` (Folgeauftrag) | `gpt-6-astra`, `high` | 9 |
| B6-R9 native Capture-Zulassung, Detailplan | `capture_admission_plan` | Nutzervorgabe `gpt-6-astra`, `max` | 9 |
| B6-R9/R10 Rendererbesitz und Sitzungswechsel, Umsetzung | `renderer_retirement` | `gpt-6.1-sol`, `xhigh` | 9 |
| B6-R9 portabler Capture-Besitzerkern und Host-Trait | `capture_core` | `gpt-6.1-sol`, `xhigh` | 9 |
| B6-R9 produktiver Login-Empfang und Desktop-/Android-Bildbesitz, Ergänzungsplan | `capture_admission_plan` (Folgeauftrag) | Nutzervorgabe `gpt-6-astra`, `max` | 9 |
| B6-R9 Windows-Capture vor Allokation und letzter Bildbesitz | `capture_windows` | `gpt-6.1-sol`, `high` | 8 |
| B6-R9 Linux-Capture, X11-Replygrenze und PipeWire-Verhandlung | `capture_linux` | `gpt-6.1-sol`, `xhigh` | 9 |
| B6-R9 Host-/Workerzulassung, Bildtransfer und Familienbesitz | `capture_core` (Folgeauftrag) | `gpt-6.1-sol`, `xhigh` | 9 |

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

Die ausdrücklich präzisierte Nutzervorgabe vom 2026-10-26 verlangt sämtliche
weitere Umsetzung und Reviews über Codex. Der normale frische Gesamtreview vor
der Remote-Suite wird deshalb mit GPT-6-Astra/xhigh durchgeführt. Die frühere
Auswahl eines externen Opus-Ausführungswegs ist ersetzt und keine Voraussetzung
mehr. Der bei der ursprünglichen Planung tatsächlich ausgeführte Opus-Review
bleibt als historischer Nachweis in `kritik.md` erhalten.

Zusätzlich folgen zwei unabhängige Regressionsreviews mit GPT-6-Astra/max und
GPT-6.1-Sol/max. Ihr eigener vollständiger Auftrag, die belegte Releasebasis und
der verbindliche Fixvertrag stehen in `regressionsreview.md`. Kein Reviewstart
oder Reviewabschluss wird aus dieser Planung abgeleitet.

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
lokalen Exec-IPC-Abbruchs statisch. B6-R1 bis B6-R8 sind nach den
Decoder-, Referenz-, Qualitäts- und Speicherbesitzerkorrekturen statisch
geschlossen. B7-01 bis B7-06 und B7-R1 sind statisch geschlossen; B8
einschließlich Erstgrant-Zusammenführung und Pixmapgrenze ebenfalls.
Kein Bericht ersetzt die gemeinsame Remote-Abnahme.

## Integrationsbefunde

Die Einträge dokumentieren die zeitliche Folge der Integration. Frühere
offene Befunde beziehen sich auf den jeweiligen Zwischenstand; der aktuelle
Reviewstatus steht im Abschnitt „Zusätzlicher Reviewauftrag“.

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
- B6-Android-Vorallokation und Surfacewechsel: Die SPS-Prüfung erhält auf allen
  vier Decodereingängen das angenommene Bildbudget und prüft codierte Maße
  einschließlich des bereits geltenden maximalen 15-Pixel-Paddings vor
  `configure` beziehungsweise `queueInputBuffer`. Der lokale Encoder behält
  seine getrennte Quellgrenze. `ScreenSession` besitzt den Pullbuffer nun über
  Surfacepausen hinweg; Full und Preview reichen dieselbe Instanz weiter.
  Damit hängt die Wiederverwendung nicht mehr von einem noch laufenden
  Vorgängerthread ab. Diese beiden Main-Anschlüsse liegen im B6-Nachreview.
- Gemeinsamer Integrationscheckpoint: `1aea1216` verbindet die Batchmodule mit
  Native-Root, Backends, Daemon, CLI/MCP, Android-Bridge und Oberflächen. Er
  enthält außerdem die aktualisierten Oberflächendokumente und ist auf `main`
  gepusht. Der abschließende B6-Quellcheckpoint folgt; keine Veröffentlichung.
- B6 verbleibende R7-Grenze: Die CPU-Skalierung in `ScreenScaling.resize`
  begrenzte Ziel-IntArray und Zielbitmap nur global, obwohl eine gültige kleine
  Quelle innerhalb eines kleineren v2-Budgets auf einen großen Viewport
  vergrößert werden kann. Auswahl 84 beauftragt die durchgängige Budgetbindung
  einschließlich Quell-/Zielarrays, Skalierungszwischenpuffern, wartenden
  Ausgaben und Full-/Preview-Übergabe. Die bereits korrigierten SPS-,
  Session-Pullbuffer- und R4-Grenzen bleiben erhalten. Area-Verkleinerung,
  Catmull-Rom-Vergrößerung und Native-Zoom bleiben gefordert; ein wirklich
  nicht finanzierbares Ziel liefert den konkreten Ressourcenfehler vor der
  Allokation. Der Nachreview bleibt ausschließlich für diese Grenze offen.

- B6-Scalerkandidat: Die Umsetzung besitzt jetzt einen sitzungsweiten
  `ScreenScaleWorker` mit einer laufenden Skalierung, einem zusammengefassten
  Folgeauftrag und expliziten Ausgabeleases bis zur Compose-Ablösung.
  `ScreenResample` erhält Area-Verkleinerung und Catmull-Rom-Vergrößerung;
  Native-Zoom verwendet einen identischen Pixelcrop ohne Floatzeilen.
  v2 reserviert `20I+32(R+1099)+(E−I)≤W` bei `I≤W/24`; Android
  bilanziert 13 feste Bildbesitzer und die tatsächlichen Skalierungsausgaben
  samt Zwischenpuffern gegen den Rest. Eine Erstzulassung berücksichtigt
  außerdem die nächste Ersetzung derselben Zielgröße. Die letzten
  Autorabschlussmeldungen scheiterten an Modellkapazität. Main übernahm
  die vollständige statische Abschlusslektüre; Rust-/Kotlin-AST und
  Whitespaceprüfung sind ohne Befund. Die Produktionsfläche liegt jetzt
  beim abschließenden Astra-Nachreview. Kein Compiler-/Laufzeitbeleg.

- B6-Abschlussnachreview: Astra hat R7 nach vollständiger Lektüre der
  Scaler-/Lease-/Composegrenzen statisch geschlossen. Die 8-MiB-Bilanz mit
  `I=349524`, `N=39`, `R=40624`, `E=412510` und `Pull=349556` ergibt
  `8388602` Byte; die tatsächliche Skalierungsreserve beträgt `2446674` Byte.
  Diese Rechnung begrenzt lebende Produktpuffer, nicht Prozess-RSS,
  MediaCodec-/Treiberpuffer oder die spätere Speicherfreigabe durch GC.
  Unfinanzierbare Ziele liefern vor großen Allokationen den Ressourcenfehler.
  Rootgraph nach den abschließenden Nativeänderungen vollständig als AST
  extrahiert und geclustert; die vier Dokumente unter `native/src` bleiben
  durch `--code-only` außerhalb dieses AST-Graphen.

- B6-Auflösungsgrenze vor der R9-Korrektur: `MAX_WORKING_BYTES=512 MiB`
  und `I≤W/24` begrenzen v2 auf 5.592.405 Pixel. 3840×2160 wird dadurch
  verkleinert; die getrennten Legacy-/RAW-Verträge werden nicht auf diese
  Reserve umgestellt. Astra bestätigt diesen Widerspruch zu V7/P30 als offenen
  Produktfehler B6-R9. Eine Beschreibung der Grenze erfüllt den Auftrag nicht;
  die folgende Korrektur ist vor Gesamtreview und Remote-Suite erforderlich.


### B6-R9: finaler Korrekturplan vom 2026-10-26

Ziel ist native 3840×2160-Rekonstruktion bei 2 Mbit/s nach vollständiger
verlustfreier Korrektur, wenn Capture, Codec, Textur und Sitzungsressourcen
sie tragen. Die bestehende Bilanz `I≤W/24` und
`20I+32(R+1099)+(E−I)≤W` bleibt verbindlich. Der Befund und die zweite
Lückenrecherche stehen in `review-b6.md`, Abschnitt B6-R9.

1. Eine getrennte erweiterte v2-Klasse erlaubt höchstens 864 MiB
   (`24×4×4096×2304`); das ist eine Implementierungsgrenze, kein Gerätefakt.
   Die 512-MiB-RAW-/Legacygrenze bleibt erhalten. Typisierte Plattformfakten
   begrenzen die Freigabe, ein gemeinsam gezählter Besitzer bindet zugesagte
   Sitzungsbudgets bis zum Ende der Sitzung. Fehlende Ressourcenfakten
   begründen keine Erweiterung. Erwartung: genügend Ressourcen erlauben
   4K, kleinere Angebote und parallele Zusagen überschreiten das Budget nicht.
2. Windows berücksichtigt freien physischen Speicher, möglichen Prozess-Commit
   und verfügbaren virtuellen Adressraum. Linux berücksichtigt MemAvailable,
   die zugehörige cgroup-Hierarchie und wirksame Prozessgrenzen. Die Auswahl
   eines konservativen Anteils ist eine Ressourcenpolitik, kein Beweis gegen
   spätere globale Speicherkonkurrenz; die Sitzung behält ihre Zusage, statt
   durch ihre eigenen bereits angelegten Bilder nachträglich kleiner zu werden.
   Der vorhandene Transferhelper liefert lediglich AvailPhys beziehungsweise
   MemAvailable und reicht allein für eine Erweiterung nicht aus.
3. ServerHello signalisiert die größere Klasse vor ClientHello. Fehlt das
   Signal, bleibt das Clientangebot im alten 512-MiB-Rahmen. Der angenommene
   Vertrag ist stets die Schnittmenge. Erwartung: alte Hosts erhalten kein
   unverständliches Angebot, alte Viewer werden nicht über ihr Angebot hinaus
   zugelassen. Das vorhandene HMAC bindet Relationsdaten; eine zusätzliche
   kryptografische Bindung des Fähigkeitsmarkers wird nicht behauptet.
4. Capture-Einzelbildgrenzen bleiben von der sitzungsweiten 24-Besitzerbilanz
   getrennt. Worker, PRODUCE und Hostpipeline behalten Seiten-, Pixel- und
   individuelle Capturegrenzen; Android behält sein tatsächliches Heapangebot.
   Erwartung: ein zulässiges Desktop-4K-Capture wird nicht erneut durch ein
   fälschlich als Sitzungsbudget gelesenes RAW-Budget verkleinert.
5. GOP-Kopfstruktur wird vor der Vertragserweiterung ohne erfundene
   512-MiB-Sitzungsfreigabe gelesen; erst der vollständige, gegen Accepted
   gebundene Vertrag erlaubt Records und Allokationen. Renderer sowie
   Preview→Full halten dieselbe einmal zugelassene Klasse. Erwartung:
   größere gültige GOPs bleiben lesbar, gemischte Legacy/v2- oder überhöhte
   Verträge bleiben abgewiesen, SET_MODE erweitert kein Budget.
6. Die eine Remote-Task-Suite erhält native 4K-Zulassung, Lossless-Pixelvergleich,
   alte-Hello-Gegenstelle, kleine Ressourcen, gemeinsame Sitzungsbelegung und
   Preview→Full als zusammenhängende Fälle. Keine lokale Ausführung und keine
   zusätzliche Prüfpipeline. Der Astra-Nachreview bewertet den konkreten
   Anschluss; der frische Astra/xhigh-Gesamtreview folgt auf dem integrierten Stand.

Quellen erneut abgerufen am 2026-10-26: Microsoft MEMORYSTATUSEX
(<https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/ns-sysinfoapi-memorystatusex>),
Linux cgroup v2 (<https://docs.kernel.org/admin-guide/cgroup-v2.html>),
Serde unbekannte JSON-Felder (<https://serde.rs/container-attrs.html#deny_unknown_fields>).
Die Plattformabfragen liefern Momentaufnahmen, keine physische Reservierung.
Auswahl für die zusammenhängende R9-Produktkorrektur: coding, Grad 9, großer
Umfang; lokaler Codex-Client 0.164.0 nachgewiesen, GPT-6-Astra/high gewählt.
Codex-Credits sind der autorisierte Ausführungsweg; numerischer Kontingentverbrauch
ist nicht gemessen. Der Startparameter belegt die konfigurierte Modellwahl,
kein davon unabhängiger Nachweis jeder tatsächlich verarbeiteten Modellantwort.


### Integration der gemeinsamen Prüfquellen

Die Quellenfixtures der Speicher-, Launcher-, Sitzungs-, Transport-, Desktop-
und Androidpakete sind integriert, aber nicht kompiliert oder ausgeführt.
Die eine Suite verwendet das kandidatengebundene Manifest; ihr Gerätepfad
benötigt ausschließlich die zugehörigen APKs. Der Auswerter verlangt tatsächliche
Fallnamen und erfolgreiche Abschlüsse, statt einen fehlenden Medienpfad oder eine
fehlende Fixture als Erfolg zu behandeln. Erfasst sind auch die direkt betroffenen
Sync-/Backup-/Omissionverträge K1–K6. Die genaue Auswahl und die Grenzen stehen im
Manifest und in `docs/refs/rv1-remote-suite.md`.

Bei der Integration entdeckte Main einen B3-Produktfehler: Scheiterte nach einer
Codex-Erstregistrierung der Ownershipmarker, blieb die gerade neu erzeugte
Konfigurationsdatei zurück. `codex::restore_owned` stellt jetzt Abwesenheit oder
den vorherigen eigenen Inhalt wieder her; abweichende inzwischen gelesene
Benutzerbytes bleiben erhalten. Auch Skillfehler sammeln beide Rollbackfehler,
statt sie zu verwerfen. `d3c6a994` ist auf `main` gepusht; Rust-AST und
Whitespace wurden statisch geprüft, keine Kompilierung oder Laufzeitprüfung.

Der damals versuchte externe Opus-Gesamtreview lieferte keine Codeprüfung.
Dieser Ausführungsweg ist durch die spätere ausdrückliche Codex-Vorgabe ersetzt.
Der normale frische Astra/xhigh-Gesamtreview und die zwei zusätzlichen
Regressionsreviews beginnen nach vollständiger Integration. Ein fehlender
Zugang im Root-Benutzerkontext war kein Beleg für serverweit fehlenden Zugang.
Kein Test- oder Releaseerfolg wird behauptet.


### B6-R10: Preview nach erneutem Sichtbarwerden

Main hat am 2026-10-26 während der R9-Integration einen konkreten
Kompatibilitätsfehler im bereits berührten Previewpfad festgestellt:
`screen_tile_face` setzt beim Verbergen `wait_frame` und entfernt die Textur.
`screen_tile_grid` verlangt danach `session.input_ready()`, bevor es ein neues
Bild übernimmt. `ResumeGate::complete` kann jedoch erst durch die tatsächliche
Präsentation freigeben; die Kachel registrierte keinen solchen Callback und
verwarf mit `CpuTexture::update` die vorhandene `PresentedFrame`-Identität.

Der aktive Korrekturauftrag erhält deshalb den Ablauf „aktuelles zulässiges
Bild übernehmen → exakt dieses Bild darstellen → bestehenden Präsentationsbeleg
senden“. Der Eintritt in die Darstellung verwendet `can_present`, nicht die
bereits abgeschlossene Eingabefreigabe. `update_frame` erhält die Identität bis
zur fertigen Skalierung und zur Textur. Alte GOPs, alte Sichtbarkeitsepochen und
unsichtbare Bilder bleiben gesperrt. Das Abnahmesignal ist Hidden→Visible mit
passendem Ack und frischem Bild, ohne vorzeitige Inputfreigabe. Die neue
Previewfixture gehört zur selben gemeinsamen Suite; der B6-Nachreview prüft
R9 und diesen Anschluss zusammen. Das ist eine Behebung im betroffenen
Preview-/Full-Kompatibilitätsweg, kein eigener Releasebatch.

### B6-R9/R10: Integrationsstand und Renderer-Nachbefund

Am 2026-10-26 ist die größere Klasse mit OS-Zulassung, Hello-/Acceptedbindung,
getrennter Capturegrenze, GOP-/IPC-Validierung und Preview-Darstellungsbeleg als
Quelle integriert. Die geänderten Rust-/Kotlinquellen lassen sich statisch als
Syntaxbaum lesen; das ist kein Compiler- oder Laufzeitnachweis. Die gemeinsame
Suite verlangt die neuen 4K-, Hello-, Speicherbesitzer- und Previewfixtures
ausdrücklich; Linux verlangt zusätzlich die cgroup-Ancestorfixture.

Der erneut konfigurierte Astra/xhigh-Nachreview hat konkrete offene
Renderergrenzen gefunden: egui 0.29.1 hält `ColorImage` in `TexturesDelta.set`
über den `TextureHandle`-Drop hinaus; `YuvResources` hält seinen letzten
Sitzungslease bis zu einem späteren Upload und kann damit dessen Zulassung
verhindern. Ein Verschieben einer Zuweisung hinter `TextureHandle::set` behebt
den ausstehenden Upload nicht. Korrekturziel ist die Bindung bis zum tatsächlichen
Rendererverbrauch bei gleichzeitigem Abfluss nach Sitzungsende und CPU-Wechsel.
Die detaillierte Quellenprüfung und die konkrete Anschlussfläche werden im
laufenden B6-Review festgehalten. Vor dessen Abschluss wird R9 nicht geschlossen.

Die erneute Modellselektion für diesen Nachreview (coding, Grad 10) lieferte
`no_feasible_model`: Astra/xhigh liegt mit 57,99 knapp unter der konfigurierten
58er-Schwelle. Die ausdrückliche Nutzervorgabe Astra/xhigh gilt weiter; eine
stillschweigende Herabstufung der Aufgabe oder des Efforts findet nicht statt.

### R9-Nachkorrektur: Plattformwurzel, Abbruch und Snapshot

Bestandsbefund des Nachreviews: `mountinfo.root == "/"` beweist in einer
cgroup-Namespace nicht die vollständige Hierarchie. `DecodeState` gibt seinen
`Shared`-Besitzer vor dem noch gehaltenen GOP frei. Die Capture-Tasktupel geben
bei tatsächlichem Taskabbruch die Stage vor dem Ergebnis frei. Der normale
Actor-Drop wartet dagegen auf seinen Job; er ist nicht allein der Auslöser.
Schließlich sendet `snapshot_epoch` auch nach v2-Accepted weiterhin einen
vertragslosen Legacykopf und begrenzt das Standbild nicht durch Accepted.

Der abgegrenzte Korrekturplan ist vor der Änderung festgelegt:

1. Linux-v2-Ressourcen dürfen die größere Klasse nur bei belegter vollständiger
   Hierarchiesicht anbieten. Die Kernelmerkmale `cgroup.events` und `cgroup.type`
   existieren ausschließlich auf echten Nichtwurzel-cgroups, auch wenn eine
   Namespace sie als `/` zeigt. Nur ihr belegtes Fehlen an einer stabilen,
   lesbaren cgroup2-Wurzel ist ein positiver Wurzelbeleg. Sichtbare Dateien,
   Lesefehler, wechselnde Mounts und nicht aufgelöste v1-Vorfahren erlauben
   keine Erweiterung; bekannte engere Nachfahrenlimits bleiben wirksam.
   Erwartung: normaler belegter v2-Root bleibt 4K-fähig, ein verborgener
   limitierender Vorfahr wird nicht durch globales MemAvailable übergangen.
2. Decoder-/Captureergebnisse halten ihre Speicherzusage bis nach dem letzten
   eigenen Bild-/Recordfeld. `End` und Verbindungsverlust verwerfen den GOP;
   Struct-/Tuple-Reihenfolgen sichern auch den Abbruchfall. Erwartung:
   eine neue Zulassung darf den belegten Raum erst nach dem letzten Bilddrop
   wieder verwenden. Bestehender kontrollierter Actor-Abfluss bleibt erhalten.
3. Snapshot verwendet denselben Accepted-Geometrie-/Speichervertrag; der
   Tiles-Standbildstrom darf dessen Hybridfähigkeit einschränken, aber kein
   v2/Legacy-Mischpaar erzeugen. Capture, Downscale, Encoder und GOP-Kopf
   verwenden dieselben Grenzen. Erwartung: ein Standbild innerhalb einer
   kleinen v2-Sitzung ist durch den bestehenden Reader akzeptierbar und
   überschreitet dessen Budget nicht; Legacy-Snapshots bleiben unterstützt.
   Das gilt auch vor dem ersten PRODUCE im Capture-Worker: dessen CAPTURE-
   Nachricht muss den optionalen Vertrag bis zur Workerzulassung tragen.
   Fehlende optionale Angaben behalten die bisherige Legacybedeutung;
   ein neuer Vertrag darf eine bestehende Zulassung nicht stillschweigend
   erweitern. Die dafür notwendige direkte Fläche umfasst `worker_wire.rs`,
   `host_worker_client.rs` und `host_worker_main.rs`.
   Ein Familienstandbild behält die tatsächlich aufgenommene Layoutrevision.
   Nach dem höher nummerierten Snapshot-GOP beginnt Liveübertragung mit einem
   neuen vollständigen GOP: andernfalls verwirft der bestehende Viewer die
   fortgesetzten Bilder des alten, kleineren GOP. Alte Präsentationsbelege
   dürfen den neuen Livebezug nicht freigeben.

Quellenabgleich vom 2026-10-26: Rust Reference, Destructors
(<https://doc.rust-lang.org/reference/destructors.html>); Kernel cgroup v2
(<https://docs.kernel.org/admin-guide/cgroup-v2.html>), Kernelimplementierung
`cgroup_base_files` und `cgroup_addrm_files`
(<https://raw.githubusercontent.com/torvalds/linux/master/kernel/cgroup/cgroup.c>).
Der verbleibende Rendererplan wird unabhängig auf der gebundenen eframe-/epaint-
Version 0.29.1 ausgearbeitet. Die Quellenfixtures bleiben Teil derselben einen
Remote-Suite. Es gibt keinen Zwischenbuild und keinen Zwischenrelease.

### R9-Erstplan: native Aufnahme vor der Verkleinerung

Am 2026-10-26 bestätigt die Anschlussprüfung eine weitere Zulassungsgrenze:
`Worker::admit` hält nur die Bytes des Accepted-Vertrags. `Stage::memory`
klont die Sitzungszusage. `ScreenSource::capture_snapshot` erzeugt dagegen
zuerst das native Bild und verkleinert es danach. Bei einem kleinen Vertrag
können dieses Bild und die gespeicherte Quellkopie zusammen bereits dessen
gesamtes Budget überschreiten. Die konstante RAW-Grenze von 512 MiB ist ein
Allokationslimit und kein zusätzlich gehaltener Budgetbesitzer.

Ziel der Nachplanung ist eine gemeinsam gezählte Zusage vor der ersten
nativen Bildallokation, einschließlich Quellöffnung, Hintergrundproduktion,
Größenwechsel und letzter gespeicherter Quellkopie. Die unveränderliche
Accepted-Zusage und die 24 Bildbesitzer des v2-Vertrags bleiben erhalten.
Offen ist die genaue, aus den Quellen abzuleitende Pufferzahl für X11,
PipeWire, DXGI, WGC, Android-Projektion, synthetische Quellen und die
Komposition einer Fensterfamilie. Der Detailplan muss auch die Grenze
zwischen Daemon und getrenntem Capture-Worker ausdrücklich benennen.

Erwartetes Ergebnis: Eine kleine Anzeige darf eine größere native Quelle
weiter verkleinern, wenn deren Aufnahme zusätzlich zugelassen ist; bei
fehlendem Budget erfolgt die Ablehnung vor der betreffenden Allokation.
Ein Wechsel oder Abbruch gibt die Zusage erst nach ihren letzten Bilddaten
frei. 4K, Legacy, Preview/Full, Snapshot und Fensterfamilien bleiben als
Kompatibilitätsfälle im selben Auftrag. Diese Planung ergänzt die bereits
beauftragte R9-Korrektur und ist kein neuer Funktionsauftrag.

Die Modellauswahl für die abgegrenzte Detailplanung (planning, Grad 9)
lieferte `no_feasible_model`; die ausdrückliche Vorgabe Astra/max für Planung
wird im Agentenaufruf verwendet. Ein unabhängiger Beleg des tatsächlich
antwortenden Modells liegt für den Collaboration-Host nicht vor.

### R9/R10: abgeschlossener Rendererplan

Der [Renderer-Detailplan](renderer-speicher.md) ist am 2026-10-26 einschließlich
der zweiten API-Lückenprüfung abgeschlossen. Eigener Ressourcenbesitz,
begrenzte Uploadtickets, der Abfluss nach dem letzten Renderpass und eindeutige
Sitzungsinstanzen schließen die konkret geplanten CPU-/WGPU-/Glow-Übergänge.
Die Umsetzung wird gemäß Modellselektion mit Sol 6.1/xhigh konfiguriert.
Der tatsächliche Collaboration-Host ist im Selektor als verfügbarer und
verifizierter Codex-Client eingetragen; fremde lokale CLI-Abos sperren ihn
nicht. Die Antwortmodellmetadaten selbst sind weiterhin nicht einsehbar.

Die Aufnahmezulassung steht im ergänzten [Detailplan](aufnahme-speicher.md). Beide Änderungen
gehören zum selben B6-Batch und erhalten danach denselben Astra/xhigh-Nachreview.
Es gibt weiterhin keinen Build-, Test- oder Releasestart.

### R9: abgeschlossener Aufnahmeplan

Der [Aufnahme-Detailplan](aufnahme-speicher.md) ist einschließlich zweiter
Lückenrecherche und Main-Integrationsprüfung abgeschlossen. Er bindet native
Quellpuffer vor Allokation, verteilt eine gemeinsame Parentzusage auf
Capturehelfer und bewahrt Bildbesitz bis zum tatsächlichen Ergebnis-, Callback-
oder Prozessende. Die Legacy-/v2-Verträge bleiben erhalten. Erstes
Umsetzungsergebnis ist der plattformfreie Besitzerkern; die produktiven
Plattform- und Prozessadapter folgen auf dessen festen API-Vertrag.

Die Modellselektion für den abgegrenzten Capture-Kern wurde erst nach Abschluss
dieses Plans ausgeführt: coding, Grad 9, große zusammenhängende Änderung,
Codex-Collaboration als belegter Ausführungshost. Ergebnis und Startkonfiguration
sind `gpt-6.1-sol`/`xhigh`. Tatsächliche Antwort-Modellmetadaten sind im
Collaboration-Werkzeug nicht zugänglich; der Startparameter ist daher der
verfügbare Modellbeleg. Kontingentrest und tatsächlicher Creditsverbrauch sind
nicht gemessen. Die Lese-/Schreibfläche ist auf Pool, Capturetypen, Bildbesitz,
Host-Trait und deren Quellenfixtures begrenzt.

### R9-Ergänzung: produktiver Login-Empfang und Bildbesitz

Main hat am 2026-10-26 im betroffenen Login-Rendereranschluss eine konkrete
noch offene Grenze belegt: `LoginImage::show` bindet produktiv keine
`MemoryHold`; seine neue kontrollierte Fixture injiziert eine.
`login/os/shared/viewer.rs::ViewerFrame` enthält nur Token und Pixel-Vec,
`daemon/os/shared/login_ipc_client.rs` liest die Pixel vorher ohne gemeinsame
Zusage. Android legt im selben Loginpfad Direktbuffer, IntArray und Bitmaps an.
Die Grenze von `4096×2304×4` Byte ist ein Größenlimit, kein Besitzerbudget.

Erstplan: den bestehenden C10-Loginanschluss um die tatsächlich lokalen
Empfangs-/Renderbesitzer erweitern, deren gleichzeitige Ablagen aus dem Code
herleiten und vorherige Zulassung sowie letzten Besitz über IPC, Desktop und
Android binden. Bestehende Maschinen-/Benutzerrechte, Challenge, Bildtoken,
Präsentationsepoch und Release-Eingaben bleiben erhalten. Die genaue
Besitzermenge und JNI-Endgrenze werden vor Produktänderungen im Ergänzungsplan
geklärt. Es ist keine neue Nutzerentscheidung erforderlich.

Der begrenzte Login-Ergänzungsplan läuft im bestehenden Astra/max-Planungsagenten.
Die vorgeschaltete Planungsauswahl meldet für Grad 9 keinen Katalogkandidaten
(Astra/xhigh 51,14 unter Mindestwert 56); die ausdrückliche Nutzervorgabe
Astra/max für Planung bestimmt deshalb unverändert die Startkonfiguration.

### R9: Windowsadapter und fertiges Rendererpaket

Das abgegrenzte Rendererpaket ist zur Integration zurückgegeben. Es umfasst
eigene CPU-/YUV-Ressourcen, den Abfluss nach dem letzten Draw, Sitzungsidentität,
Skaliererabbruch und die tatsächlichen Übergänge in Vollansicht, Kacheln und
Loginansicht. Syntax-/Textbefunde liegen vor; Typprüfung und Laufzeit sind
unbestätigt. Die produktive Login-Empfangszusage bleibt im Ergänzungsplan offen.

Für die Windowsadapter wurde nach dem abgeschlossenen Aufnahmeplan eine
zusammenhängende Fläche aus WGC, DXGI, D3D-Readback, Host-Prepare und Loginquelle
festgelegt. Die sechs Auswahlkriterien ergeben 19 Punkte und Grad 8:
Schlusskette 3, offene Detailwahl 2, Abhängigkeiten 4, Autonomie 3,
Prüfbarkeit 3, Fehlerfolge 4. Der Codex-Selektor wählt `gpt-6.1-sol`/`high`.
Der Startparameter ist der zugängliche Modellbeleg; tatsächliche
Antwortmodellmetadaten und gemessener Creditsverbrauch sind nicht verfügbar.
Der Auftrag verändert keine Worker-, Codec-, Linux-, Android- oder
Rendererimplementierung. Neue Quellenfixtures gehören zur gemeinsamen
Remote-Suite und werden lokal nicht ausgeführt.

Der Linuxanschluss erhält eine eigene zusammenhängende OS-Fläche: X11-SHM,
Reply-/Cursorzulassung, PipeWire-Verhandlung, beide Portalwege und die gebundene
Loginquelle. Seine offene Vorallokations-/Endbesitzerherleitung ergibt Grad 9
aus 21 Punkten (4/2/4/3/4/4); der Codex-Selektor wählt
`gpt-6.1-sol`/`xhigh`. Kern- und Workerdateien bleiben außerhalb seiner
Schreibfläche. Die API `prepare_session_source` bleibt durch einen verifizierten
delegierten `SessionAdmission`-Kontext auch im Kindprozess verwendbar.
Familienpläne unterscheiden nun gleichzeitig lebende von seriell transienten
Mitgliedern; ihre tatsächlichen Slices unterliegen weiterhin einem gemeinsamen
Summenzähler. Diese Kernanschlüsse sind Quellcodebefunde ohne Laufzeitabnahme.


Der Besitzerkern und das Rendererpaket sind als Quellcheckpoint `8baeebbc`
auf `main` gepusht. Die vorherige native Plattformaktivierung ist damit nicht
als vollständig korrigiert erklärt. Der nächste zusammenhängende Anschluss
gehört dem bisherigen Kernagenten: Hostpipeline, Prepare/Activate, private
Workercredits, Antwortbesitz, synthetische Quelle und Familienkomposition.
Die erneute Codex-Auswahl (21 Punkte, Grad 9, coding) bestätigt seine
bestehende Startkonfiguration `gpt-6.1-sol`/`xhigh`. Native MF-/OpenH264-Adapter
bleiben eine folgende abgegrenzte Integration am dabei festgelegten Codecvertrag.


### Korrektur des unbeabsichtigten Push-Laufs

Der Push von `8baeebbc` löste entgegen der beabsichtigten Task-Markierung den
allgemeinen Lauf [40275123530](https://github.com/b1ue-man/smart-explorer/actions/runs/40275123530)
aus. Der Workflow verwendete `endsWith` für die gesamte Commitnachricht;
der Marker stand im Betreff, danach folgte ein Committext. Der Windowsjob
begann seinen Schritt „Native Windows library tests“. Main forderte nach
Feststellung den Abbruch an; die API bestätigte den Auftrag mit HTTP 202.
GitHub bestätigt den Abschluss als `completed/cancelled` am
2026-10-26 um 05:44:04 UTC. Das anschließend vollständig abgerufene Protokoll
von Windowsjob `121416138400` enthält Abhängigkeitskompilierung und am
05:43:59 UTC den Abbruchfehler, aber keine gestartete Testfunktion oder
Testzusammenfassung. Daraus folgt kein erfolgreicher Funktionstest.
Der Lauf ist keine vereinbarte Task-Abnahme und liefert
keinen bestätigten Meilenstein. Frühere Aussagen „kein Build/Test gestartet“
bezeichnen ausschließlich den Stand vor diesem unbeabsichtigten Push-Auslöser.

Der Push-Guard prüft den ausdrücklich gesetzten `[task candidate]`-Marker
künftig mit `contains`; README und Releaseanleitung beschreiben dieselbe
Semantik. Die geplante Reihenfolge bleibt Gesamtreview, genau eine gezielte
Remote-Suite und ein terminaler Remote-Release. Während des unbeabsichtigten Laufs wurde kein zusätzlicher
Umsetzungsagent gestartet.


### R9: Android-/Codecanschluss und finaler Loginbildplan

Der vollständige [Loginbildplan](login-bildspeicher.md) ergänzt C10 um L1–L8: tatsächliche lokale Claims, PNG-Zeilendecode, Write-Abschluss, Viewerplätze sowie Desktop-/Android-Endbesitz. Die temporäre Planübergabe wurde vollständig übernommen und entfernt. Die Umsetzung der lokalen Claims folgt nach Übergabe des Capturekerns, damit dessen Dateien einen Autor behalten.

C9 (`capture_android`) und die native Codecgrenze C2 (`capture_codecs`) besitzen getrennte Schreibflächen. Jeweils 21 Kriterienpunkte (4/2/4/3/4/4), Grad 9 und Profil coding liefern über den Codex-Selektor `gpt-6.1-sol`/`xhigh`. Diese Parameter wurden beim Start gesetzt; Antwortmodellmetadaten und gemessener Creditsverbrauch sind nicht zugänglich. Android besitzt Projection, Accessibility, Kotlin-/JNI-Puffer und Rust-Bridge; der Codecauftrag besitzt MF und OpenH264. Gemeinsame Modulregistrierungen verbleiben bei Main.


### R9: konkrete Endbesitzkorrektur und C7-Nachtrag

Die Integration fand im Windows-WGC-Fehlerweg einen vorzeitigen Drop: verbrauchendes Close gab den Native-Slice auch bei fehlgeschlagenem Session-/Pool-Close frei. Main hält jetzt Partial-Create und Closefehler in einem eigenen CaptureResources-Besitzer. Erfolg oder bereits geschlossenes Objekt beendet ihn; ansonsten behält ein Cleanupbesitzer Interfaces und Slice. Selbst ein gescheiterter Cleanupthreadstart erhält die Zusage bis zum vom Parent bestätigten Prozessende. Das ist ein Quellfix ohne Windows-Laufzeitbeleg. Microsofts [Framepool-Close](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.close) und [Session-Close](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.close) wurden am 2026-10-26 geprüft; ein Fehler wird nicht als erfolgreicher Ressourcenabschluss ausgegeben.

Linux-X11 und die vorbereitenden PipeWireanschlüsse sind zurückgegeben. C7 bleibt offen: INACTIVE kann bereits die Linkvorbereitung verhindern; Formatcallback und späteres update_params beweisen keinen asynchronen Allokationshalt. Der bestehende Astra/max-Planer erarbeitet einen quellengebundenen Nachtrag. Die erneute planning-Auswahl ergibt 21 Punkte/Grad 9 und keinen Katalogkandidaten; die explizite Nutzervorgabe bleibt maßgeblich. Kein dauerhaft ausstehender Waylandpfad gilt als fertig.

C10 besitzt einen getrennten Autor `login_capture` für Broker, Helper, privaten Kanal und OS-Prozessabschluss. L1 besitzt `image_admission` für exakte lokale Bildclaims. Beide coding-Auswahlen: 21 Punkte/Grad 9, `gpt-6.1-sol`/`xhigh`; Startparameter gesetzt, tatsächliche Antwortmodellmetadaten nicht zugänglich. Der Coreautor hat die vier Dateien frame_memory.rs, capture_grant.rs, capture_bytes.rs und capture_image.rs ausdrücklich an L1 abgegeben. Host-/Worker-/Planänderungen bleiben bei ihm.

Der Guardfix `66efeee7` ist auf main gepusht. GitHub-Lauf [40276492134](https://github.com/b1ue-man/smart-explorer/actions/runs/40276492134) ist abgeschlossen mit `skipped`; er hat keine Abnahmesuite ausgeführt.


### R9: bestätigter C7-Plan und produktive Loginbildintegration

Der [C7-Nachplan](pipewire-import.md) ist vollständig übernommen; die temporäre Arbeitskopie wurde nach Inhaltsvergleich entfernt. Die frühere Formatcallback-/INACTIVE-Annahme ist im Aufnahmeplan ersetzt. Der aktive Stream hält neue Core-Importnachrichten vor lokalem Mapping an, fordert den exakten Ganzobjektbetrag derselben Parentautorität an und setzt erst nach gebuchtem Mapping fort. Producer-/Compositorinternas bleiben externe Ressourcen. Coreautor und Linuxautor integrieren C7.1–C7.5 mit getrennten Schreibflächen. Die erneute Linuxauswahl ergibt 21 Punkte/Grad 9 und `gpt-6.1-sol`/`xhigh`; der vorhandene Agent wurde mit diesen zuvor gesetzten Parametern fortgesetzt.

L1 liegt als Quellkandidat vor: echte Rollenclaims am gemeinsamen Prozesspool, lokale immutable Bildbesitzer, getrennte Rollenplätze und Rootretirement auf tatsächlich noch lebende Slices. Main ergänzte die private Rootidentität für den Dispatcher, Membererkennung und die ausdrückliche Sperre von Bildallokationen aus Importtickets. Windows und X11 geben nach Ressourcen-/Cachedrop die unbenutzte Rootzusage auf; Familienmember stellen ihren gemeinsamen Root nicht einzeln still. Android beendet den alten Grant nach erfolgreicher Generationsumschaltung und beim Close. Ein abgelehnter Androidersatz erhält seine bisherige Generation.

Der erste C9-Autor meldete eine Provider-Kapazitätsgrenze. Die Fortsetzung wurde über die verfügbare Codex-Alternative `gpt-6-astra`/`high` abgeschlossen. Der Quellnachtrag korrigiert Accessibility-Ausgabelimits, vollständige synthetische Tilespläne und einmalige Skalierung aus nativer Stillgeometrie. Main hat JNI-/Mobile-/OS-Registrierungen ergänzt. Die bisherigen Instrumentationsaufrufer brauchen noch ihren vollständigen Abgleich mit der neuen Budget-API; ein Buildbeleg wird nicht behauptet.

L2–L4 und L5–L6 verwenden jetzt produktive CapturedImage-Claims an PNG-Empfang, IPC, Viewer, CPUkonversion, Skalierung, Upload und GPUbesitz. Der Registryowner lebt in der Blockingclosure und im Ergebnis; der Daemonwrite quittiert nach Bilddrop. Zwei tatsächlich gehaltene Rawbilder sperren den nächsten Capture, ohne Release/Close zu sperren. Ein gemeinsamer lokaler IPC-Viewer-Gegenpart wird mit LoginImage::show verbunden. Die Autorprüfung läuft noch; die Quellenfixtures wurden nicht ausgeführt.

L7–L8 ist als eigener zusammenhängender Android-JNI-/Rendererauftrag gestartet. Seine Modellwahl ergibt 21 Punkte/Grad 9 und `gpt-6.1-sol`/`xhigh`. NativeBridge gehört diesem Autor; Rustmodulregistrierungen bleiben bei Main. Startparameter sind belegt, Antwortmodellmetadaten und gemessener Creditsverbrauch sind nicht zugänglich. Die Ausrichtungsvorgabe P32 bleibt ausschließlich an Android-Systemeinstellungen gebunden.

Kein lokaler Build, Compiler, Formatter oder Testlauf wurde gestartet. Der unbeabsichtigte abgebrochene Remote-Pushlauf bleibt separat dokumentiert und zählt nicht als Abnahme. Frischer Astra/xhigh-Gesamtreview, zusätzliche Astra/max- und Sol-6.1/max-Regressionsreviews, eine ausgewertete Remote-Task-Suite und der terminale Remote-Release stehen aus.


### R9-Zwischenstand: durchgängiger Loginbildbesitz und native Encoder

Die folgenden Quellmeilensteine sind am 2026-10-26 einzeln committed; sie sind
keine Releases und noch keine Kompilierungs- oder Laufzeitabnahme:

| Commit | Integrierter Quellumfang |
|---|---|
| `93bec3f9` | OpenH264-/Media-Foundation-Eingaben und Access-Units erhalten vor ihrer eigenen Ablage Capture-Slices; COM-Aliase halten ihre tatsächlichen Bufferanteile. |
| `f06ff706` | WGC-/DXGI-Flächen werden vor Erzeugung zugelassen; fehlgeschlagener nativer Abschluss bleibt im Retirementbesitz. |
| `e79c2430` | Lokale Bildrollen und Capture-Slices teilen die Prozesskapazität; Claim-, Slice- und Grantverantwortung sind in zusammenhängende Module getrennt. |
| `adaef690` | Loginhelper, Reader und Parentantwort halten Capturebesitz bis zu ihren jeweils tatsächlichen Enden. |
| `33e211d0` | PNG-Decode, Write-Abschluss, IPC-Empfang und zwei Viewer-Rawplätze reichen immutable, zugelassene Bilder weiter. |
| `d5adc961` | LoginImage-Konversion, Skalierungsscratch, Upload und Renderfläche halten eigene Claims durch den vorhandenen Rendererabschluss. |
| `ca48b3fc` | Android-Capturearena, native Direktbuffer und Java-/Kotlincallbacks teilen die zugelassene Capturegeneration. Bestehende Instrumentaufrufer benötigen noch den geplanten Fixtureanschluss. |
| `df6de3d1` | Android-Loginbilder durchlaufen Bitmapticket, direkten NDK-Fill, GlobalRef, Drawborrow und geordneten HWUI-Abschluss. |

Das tatsächlich gemeldete Android-Heapangebot wird vor Hostprereqs, Viewer-Open
und Login-Open im gemeinsamen Pool beobachtet. Eine kleine ausgehandelte
Bildanforderung wird dadurch nicht versehentlich zur gesamten Prozessgrenze.
Eine kleinere neue Plattformbeobachtung darf bereits lebende Besitzer nicht
vorzeitig freigeben. Der Legacy-Fallback bleibt nur vor einer tatsächlich
vorliegenden Plattformangabe bestehen.

Der Android-Präsentationsbeleg ist im kanonischen `login-bildspeicher.md`
präzisiert: Producerabschluss und TextureView-Update allein genügen nicht;
der folgende Fenstercommit und der zu genau diesem Frame gehörende geänderte
SurfaceTexture-Zeitstempel ergänzen die vorhandenen Session-, Sichtbarkeits-
und Fokusprüfungen. Diese Quellentscheidung behauptet keinen physischen
GPU-Fence. Die bisherige HWUI-Instrumentfixture verwendet noch simulierte
NativeCalls; ein echter ViewerFrame→JNI→NDK→GlobalRef-Fall ist in ergänzender
Planung und wird nicht als vorhanden oder bestanden ausgewiesen.

Der zusätzliche statische Loginreview wurde als Codex-Aufruf mit
`gpt-6-astra`/`xhigh` gestartet. Die Modellwahl ergab Qualitätsgrad 9;
der ausdrückliche Nutzerwunsch bestimmt hier den nativen Effort. Belegt sind
die Startparameter; Antwortmodellmetadaten und tatsächlicher Creditsverbrauch
werden vom vorhandenen Kollaborationswerkzeug nicht bereitgestellt.
Core-/Workerintegration, Linux-Imports, Android-Fixtureanschluss, gemeinsame
Remote-Suite und terminaler Remote-Release bleiben noch offen.


### Historischer Zwischenstand der Reviews und Android-Fixtureplanung, 2026-10-26

Der Login-Batchreview liegt in `review-login-bildspeicher.md`. F1 (Rollback nach
nativer Androidbildübernahme) und F2 (frische Präsentationsbelege nach
Fensterfokusverlust) sind im Arbeitsbaum korrigiert und vom Reviewer statisch
nachgelesen. F3 (fehlende GL-Uploadfehlerweitergabe) ist noch offen. Die Korrektur
benötigt einen tatsächlichen Fehlerpfad bis zur Residentübernahme und
Präsentationsbestätigung; ein GL-Fence allein belegt keinen gültigen Bildinhalt.

Der von Astra/max ausgearbeitete konkrete Android-Capture-/JNI-Fixtureplan ist
vollständig nach `android-bildfixtures.md` übernommen. Seine Umsetzung und der
Anschluss an die eine Remote-Suite stehen aus. Der produktive
`loginBitmapDiscard`-Rollback liegt inzwischen vor. Die gezielte Suite umfasst
die direkt betroffenen Capturemethoden und Loginbildmethoden; eine vollständige
unbeteiligte Gesten-/Texttestsammlung wird dadurch nicht zusätzlich ausgelöst.

Der Fokusfix ist als `851c694d` committed: Android und Desktop verweigern frische
Präsentationsbelege nach Fensterfokusverlust; Desktop trennt dafür Fenster- und
Bildwidgetfokus, damit der SAS-Button weiterhin bedienbar bleibt. Die
Quellenfixture enthält die tatsächliche kontrollierte Retirement-/Uploadfolge
beim Refokus. Rust-AST und Whitespace sind statisch geprüft; keine Fixture wurde
ausgeführt. Der Commit ist ein Entwicklungscheckpoint, kein Release.


### Captureintegration und Batchbefunde vor den abschließenden Fixes, 2026-10-26

Die folgenden kohärenten Quellmeilensteine sind auf `main` committed und bis
`f120f910` erfolgreich auf `origin/main` gepusht:

| Commit | Konkreter Anschluss |
|---|---|
| `e3e778ea` | Login-Bitmap-/Rendererclaims werden bei fehlschlagender Kotlinübernahme zurückgegeben; Rohdeskriptorrollback behält genaue Ownerbindung. |
| `4e9f4202` | Erststart, Worker, Familienproduktion und Quellenwechsel teilen die reale Parentautorität; Rawbilder und Encoderecords behalten ihre Zusage bis zum tatsächlichen Besitzerende. |
| `b03182fe` | X11-/PipeWire-Import- und Portalinputbesitz sind an konkrete Quelle, Verbindung und Ressourcenende angeschlossen; frühe Portalvorbereitung besitzt ihr eigenes Registrierungsticket. |
| `2fb02432` | Android-Capture-/Loginfixtures verwenden reale Grants, JNI, NDK und HWUI. Die Syntheticquelle behält ohne Displaymetadaten ihren seit v0.5.184 belegten Standard 1280×720. |
| `f120f910` | Der gepinnte Glow-Painter reicht intern gelesene GL-Fehler durch Upload und Draw weiter; fehlgeschlagene Versuche erzeugen keinen neuen Präsentationsbeleg. |
| `105bd6a3` | Die bestehende Android-Releaseprüfung verweigert tatsächliche APK-Dateien mit Bildfixture-Klasse oder zugehörigen JNI-Exporten; noch kein Release ausgeführt. |

Diese Commits sind Entwicklungskandidaten. Es wurde keine lokale Kompilierung,
kein lokaler Testlauf und keine Releaseausführung vorgenommen. Der normale
Android-Batchreview und der zusammenhängende Core-/Worker-/Linux-Batchreview
sind mit `gpt-6-astra`/`xhigh` als Startparametern beauftragt. Die tatsächlichen
Antwortmodellmetadaten und der Creditsverbrauch sind nicht zugänglich.

Die abgeschlossenen Berichte `review-android-bildfixtures.md` und
`review-capture-integration.md` liefern belegte Fehler. Android betrifft
Claim-/Aliasübernahme, Encoderkonstruktion und Workerabschluss; die Kettenprüfung
des Fixautors hat zusätzlich konkrete Besitzübergaben in ProjectionFrames,
ControlService, CodecLatency und queueRgba erfasst. Capture betrifft den fehlenden
SHM-Typimport, Cursorzusatzbedarf gegenüber Parent und Familie, den
Ausgabe-/Retirement-Lockzyklus und den Workerabbruch bei Formatwechsel.

Der Nachreview von `f120f910` in `review-login-bildspeicher.md` bestätigt die
GL-Fehlerweitergabe, belegt aber eine verbleibende Login-Eingabelücke: Ein bereits
bestätigtes Bild kann nach einem späteren Upload-/Drawfehler weiter Input/SAS
autorisieren. Dieser Restbefund ist nicht als behoben markiert.

Drei getrennte Fixaufträge sind mit `gpt-6.1-sol` und Effort `max` als
Startparametern beauftragt: `android_capture_ownership_fix`,
`renderer_input_revocation_fix` und `capture_integration_fix`. Bereits diese
normalen Batchfixes folgen vollständig dem präzisierten Zuständigkeitsvertrag:
Jeder zuständige Fixautor erstellt selbst Funktionsauflistung, vollständige
direkte/indirekte Kettenkarte, Fixplan, Umsetzung und eigene Anti-Regressionsreview.
Die ursprünglichen Reviewer liefern Fehlerbefunde und übernehmen keinen Teil
dieses Fixzyklus. Die statischen Befunde behaupten keine praktisch ausgeführten
Fehlerfälle. Alle vorbereiteten Verhaltensfälle gehören zum selben Remoteeinstieg.

Die frische normale Gesamtreview und die zwei unabhängigen Regressionsreviews
gegen `v0.5.184` bleiben nach Integration dieser Korrekturen ausstehend. Erst
danach folgt die eine Remote-Task-Suite mit ihrem Fixloop und der eine terminale
vollständige Remote-Release.


### Integrierte Batchfixes und gemeinsamer Reviewkandidat, 2026-10-26

Die drei zuständigen Fixautoren haben ihren vollständigen Zyklus jeweils
selbst abgeschlossen: Funktionsauflistung, direkte und indirekte Kettenkarte,
Fixplan, Umsetzung und eigene statische Anti-Regressionsreview. Die
ursprünglichen Reviewer haben keinen Teil dieses Fixzyklus übernommen.

| Commit | Konkretes Ergebnis und eigener Abschlussbericht |
|---|---|
| `8ac10dc8` | Android erhält Claim-/Aliasbesitz bei fehlgeschlagener Übernahme, Konstruktor- und Threadstartfehlern sowie Projection-, Screenshot-, Access-Unit- und Hybridinputübergaben. Bericht: [fix-android-capturebesitz.md](fix-android-capturebesitz.md). |
| `43f46c8f` | Ein tatsächlicher Glow-Upload-/Drawfehler entzieht bereits bestätigte Login-Input-/SAS-Belege vor einem möglichen blockierenden Abschluss. Veraltete Callbacks lassen eine neue gültige Darstellung bestehen. Bericht: [fix-renderer-eingabefreigabe.md](fix-renderer-eingabefreigabe.md). |
| `f945ef59` | Bildgebundene Cursorcredits ersetzen keinen Parentgrant; Workertransport hält Ressourcen außerhalb der Ausgabesperre; alte Antworten und konkrete Pausengründe bleiben bei Quellenwechsel erhalten; PipeWire kann denselben Formatbedarf nach temporärem Creditmangel erneut zulassen. Bericht: [fix-capture-integration.md](fix-capture-integration.md). |

Die Aussagen in den vorstehenden chronologischen Zwischenständen über noch
laufende Coreintegration, Linuximporte, Android-Fixtureanschlüsse und diese
Batchfixes sind damit historisch. Die gemeinsame Suiteauswahl enthält ihre
konkreten vorbereiteten Verhaltensfälle. Die lokale AST-/Textprüfung liefert
keinen Typ-, Compiler-, Geräte- oder Laufzeitnachweis.

Die normale Astra/xhigh-Gesamtreview sowie die zusätzlichen Astra/max- und
Sol-6.1/max-Regressionsreviews sind am festen Kandidaten `ee75e91b` abgeschlossen.
Die Vergleichsbasis ist der tatsächlich veröffentlichte Release `v0.5.184`.
Befunde, Zuständigkeit und Stand der vollständigen Fixzyklen stehen in
[regressionsreview.md](regressionsreview.md). G5 ist nach Gegenbeleg zurückgenommen;
alle belegten Fehler haben einen zuständigen Sol-6.1/max-Fixautor.
Die gemeinsame Remote-Task-Suite und der terminale vollständige Remote-Release
stehen weiterhin aus. Kein Quellfix gilt dadurch als praktisch bestätigt.

### Integrierte Gesamtreviewfixes, 2026-10-26

Alle zehn Fixgruppen der normalen Gesamtreview und beider Release-Regressionsreviews
sind mit ihrem vollständigen eigenen Sol-6.1/max-Autorzyklus abgeschlossen.
Die Commit-/Berichtstabelle steht in [regressionsreview.md](regressionsreview.md).
Der gemeinsame Suiteeinstieg enthält die exakten zusätzlichen Verhaltensfälle,
die P27-Netzreferenz, den versionsgebundenen Bildqualitätsvergleich und den
isolierten X11-Anschluss. Die Vorbereitung führt Abhängigkeitsauflösung und
Formatprüfung ausschließlich remote aus; notwendige Kandidatenkorrekturen
bleiben im selben Suite-Fixloop. Rootgraphrefresh, Remoteauswertung und der
terminale vollständige Remote-Release sind die nächsten Schritte.

### Erste Remote-Vorbereitung, 2026-10-26

Lauf `40332971455` ist nach Abhängigkeits-/Formatprüfung beendet. Compiler- und
Gerätejobs blieben übersprungen; ein funktionaler Laufzeitnachweis liegt dadurch
noch nicht vor. Die gebundenen Remote-Patches sind `10988167`. Die bei der
Formatierung belegten Modulgrößen werden durch die bisherigen Autoren mit
fachlicher Aufteilung korrigiert. Derselbe Autor bleibt für Plan, Änderung und
Nachreview seiner Kette zuständig; Main integriert und wiederholt nur RV1.
Der terminale Release bleibt ausstehend.

### Strukturkorrektur integriert, 2026-10-26

Alle acht bisherigen Fixautoren haben ihre Modulaufteilung, Kartenfortschreibung
und eigene anschließende Anti-Regressionsreview beendet. Die getrennten Commits
stehen im [RV1-Modulplan](rv1-modulplan.md). Main hat die übergebenen Schreibflächen,
Syntax, normalen Quellgrößen und erhaltenen Fallnamen statisch abgeglichen.
Fünf bestätigte falsche Transfer-Typbezüge sind im selben Transferautorzyklus
korrigiert. R19 bindet die bereits bestehenden reinen Mount-Runtimevertragsfälle
an dieselbe Suite. Rootgraphrefresh und der neue Kandidat führen ausschließlich
zur Wiederholung dieses einen Remoteeinstiegs; Typ- und Laufzeitnachweis bleiben
bis zu dessen tatsächlicher Auswertung offen. Die Version wurde nicht geändert.

### Gemeinsamer Compilerbefund im Remote-Fixloop, 2026-10-26

Nach Übernahme des gebundenen Formatpatches ist die Vorbereitung von Lauf
`40344852826` für `7fb96ef0` erfolgreich. Die anschließenden Linux-, Windows-
und Androidjobs sind vollständig beendet und scheitern am selben E0451 im
vendorten Noq-proto-Konstruktor. Die komplette Suite ist damit nicht bestanden;
die isolierten Glow-Ergebnisse sind nur Teilnachweise. Ein begrenzter Fixautor
ist mit `gpt-6.1-sol`/`max` als Startparametern für Initialisierungsgrenze,
Verknüpfungskarte, Erhaltungsplan, Umsetzung und eigene Nachreview beauftragt.
Antwortseitige Modellmetadaten stellt das Agentwerkzeug nicht bereit.
Die Arbeitsweise-Selektion nennt für diesen kleinen, belegten Compilerfix
Grad 6 und Sol 6.1/low; der vom Nutzer verlangte Max-Effort hat Vorrang.
Der nächste Schritt bleibt die Korrektur und Wiederholung desselben
Remote-Suiteeinstiegs. Ein terminaler Release wurde noch nicht gestartet.

Der zuständige Autor hat die Korrektur und seine eigene Anti-Regressionsreview
abgeschlossen: [Konstruktorbericht](fix-path-policy-construction.md). Der neue
modulinterne Konstruktor erhält den abgeleiteten Default und die bereits
berechnete Verbindungskennung; öffentliche Verträge, private Zähler und
RNG-Verbrauch bleiben unverändert. Main hat beide Quellhunks und ihre
Einbindung geprüft; statische Rustsyntax und Whitespace sind ohne Befund.
Die Änderung betrifft ausschließlich Vendorcode und Dokumentation, daher
bleibt der auf `native/src` begrenzte Rootgraph unverändert gültig.
Compiler- und Funktionsbestätigung stehen weiterhin im selben Remote-Fixloop aus.

### Native Compilerkorrekturen, 2026-10-26

Lauf `40354605251` für `d95dc404` bestätigt die erfolgreiche Vorbereitung und
die Beseitigung des Noq-proto-Konstruktorfehlers. Windows, Linux und Android
erreichen die Produktquellen und liefern dort weitere Compilerdiagnosen.
Alle Jobs sind beendet; die Gerätestufe wurde übersprungen. Main hat die
vollständigen nativen JSONL-Ausgaben und das Androidlog an den Kandidaten
gebunden ausgewertet. Die [Korrekturgruppen](rv1-compilerplan.md) bleiben
im selben RV1-Fixloop. Jeder vollständige Fixzyklus bleibt bei einem Autor;
die gemeinsame Login-/Familyviewerregistrierung gehört vollständig zum
Captureautor. Kein Release wurde gestartet.

Die sechs Korrekturgruppen sind in `e841a579`, `5070f5ad`, `34d8c0d3`,
`1d9ba2db`, `b47507ec` und `2143299f` separat integriert. Jeder Autor hat
seine eigene anschließende Anti-Regressionsreview dokumentiert. Main hat die
Integrationsgrenzen und die zulässigen statischen Prüfungen ausgewertet.
Die direkt betroffenen vorhandenen Vertragsfälle sind im selben Manifest unter
`R20-Compiler-Integrationen` ergänzt. Compiler- und Funktionsbestätigung bleiben
bis zum nächsten erfolgreichen Lauf dieses Suiteeinstiegs offen.

Der integrierte Kandidat `531230bc` ist auf Main und Fixbranch gepusht.
Sein Lauf `40366764400` meldet ausschließlich einen Formatpatch für drei
Rustquellen, keine Lockänderung und keine Größenverletzung. Main hat den
gebundenen Patch unverändert übernommen und die erhaltenen syntaktischen
Blatttoken, Syntax und Größen statisch bestätigt. Compiler- und Gerätestufen
dieses Versuchs sind übersprungen. Es folgt derselbe Remote-Suiteeinstieg.

### Verbleibende Modul-/Typgrenzen und JNI, 2026-10-26

Der formatierte Kandidat `b233e533` ist gepusht. Sein Lauf `40368746815`
hat die Vorbereitung bestanden; die native Desktopkompilierung und die
separate Android-JNI-Brücke melden weitere Fehler. Alle Jobs sind beendet,
die Gerätestufe ist übersprungen. Die vollständige Diagnoseauswertung und
Primärquellenklärung stehen in [rv1-compilerplan.md](rv1-compilerplan.md).
Fünf bisherige Fixautoren besitzen jeweils die vollständige Fehlerkarte,
den vorherigen Plan, die Änderung und die eigene anschließende Nachreview.
Ihre Flächen sind disjunkt; das erforderliche Windowsfeature gehört allein
zum Geräteautor, die gesamte Android-Hidden-Textkette zum Zustandsautor.
Kein Release wurde gestartet.

Die fünf Korrekturgruppen sind inzwischen mit eigenen Nachreviews abgeschlossen
und in `7a1f0931`, `f9306ea9`, `31560a85`, `cd0be5a9` und `f86bcb63`
integriert. Der Geräteautor erhält in der korrigierten Kontenfixture den Status-
und Versionsnachweis über den bestehenden Diagnosebereich. Main hat Diffs,
Berichte und statische Syntax-/Größenbefunde ausgewertet und die vorhandenen
verbundenen Fälle im selben Manifest ergänzt. Der nächste Kandidat folgt nach
Rootgraphrefresh und Push demselben Remote-Suiteeinstieg; noch kein Funktionspass.

Lauf `40379623936` hat am Kandidaten `5609ce9f` nur eine Formatabweichung
in der JNI-Capturefunktion gemeldet. Der kandidatengebundene Patch ist ohne
inhaltliche Änderung übernommen; alle syntaktischen Blatttoken bleiben exakt
gleich. Die Lockdatei ist unverändert, die Größenprüfung ohne Befund.
Compiler- und Funktionsstufen dieses Versuchs wurden übersprungen.

### Erfolgreiche native Kompilierung, verbleibende Kotlin-/Suitegrenzen

Lauf `40381452096` für `0425aa46` ist vollständig beendet. Windows und Linux
kompilieren Produktbibliotheks-Prüfbinärdatei und `se`; Android kompiliert die
native Bibliothek samt JNI. Die Funktionsabnahme ist noch nicht vollständig
gelaufen: Android scheitert danach an K01–K11, die Desktop-Hauptauswahl an
belegten veralteten beziehungsweise plattformfremden Pflichtnamen. Linux
erreicht außerdem die reale P27-Netzwerkmessung, deren Abschluss mit
`wire ACK stream closed` scheitert. Das ist noch kein Produktleistungsurteil.

Die [begrenzte Fortsetzung](rv1-compilerplan.md) umfasst drei vollständige
Autorenzyklen: Iconkonsum, Android-Capture/Puffer und Messlebensdauer. Main
besitzt die anschließende gemeinsame Manifestkorrektur. Sämtliche bisherigen
Funktionsverträge und V7-Orakel bleiben erhalten. Es folgt ausschließlich
derselbe Remote-Suiteeinstieg; kein Release wurde gestartet.

Die drei vollständigen Autorenzyklen sind jetzt integriert: Iconkonsum in
`00c97f88`, Capturestart und Budgetbindung in `69f86c46`, gemeinsamer
Messabschluss in `578357a8`. Main hat die geänderten Quellen und die eigenen
anschließenden Anti-Regressionsreviews gelesen. Kotlin-/Rust-AST, XML und
Whitespace sind statisch ohne Befund; das ist keine Typ- oder Laufzeitabnahme.

Das gemeinsame Manifest bindet den tatsächlichen Snapshotnachfolger, ordnet
mobile Text-/Bitmapfälle Linux zu und erhält den zusätzlichen JVM-/JNI-Nachweis.
`R20-Androidtext-Vertrag` trennt die Textgrenze von den gemeinsamen
Desktopintegrationen. `R21-P27-Messabschluss` nimmt die gezielten
Lebensdauerfälle innerhalb desselben Linuxeinstiegs auf; Netzprofile,
Messzeitfenster und V7-Orakel bleiben unverändert. Der statische Auswahlvergleich
gegen beide vorhandenen Binärlisten trennt die neuen, noch nicht kompilierten
Abschlussfälle ausdrücklich vom vorhandenen Bestand. Kein bestehender
Ressourcenfall entfällt. Nach Rootgraphrefresh und gemeinsamem Push folgt
ausschließlich derselbe Remote-Fixloop.

Kandidat `4a42b936` ist auf Main und Fixbranch gepusht. Sein Lauf
`40400605922` meldet ausschließlich eine Formatabweichung am P27-
Abschlussaufruf. Der gebundene Patch ist unverändert übernommen; sämtliche
syntaktischen Blatttoken bleiben gleich. Die Lockdatei bleibt unverändert,
Größenprüfung ohne Befund. Compiler- und Funktionsstufen dieses Versuchs
sind übersprungen; der nächste Kandidat bleibt im selben Remote-Fixloop.

Lauf `40406246032` für `1ffbf4b0` bestätigt Vorbereitung und beide
Desktopkompilierungen. Die Funktionsstufen liefern F01–F56; P27 scheitert
nach vorhandenen ersten Referenz-/Screenrohdateien beim nächsten
Netzprofilaufbau (F57). Der [Funktionsplan](rv1-funktionsplan.md) enthält
vollständigen Befundumfang, zwei Recherchepässe, Erhaltungsverträge und
M01–M13. Main hat disjunkte Schreibflächen vorbereitet. Android war beim
Abruf 22:10:07 UTC noch im Checkout. Kein Fixagent dieses Durchgangs wird
während der Pipeline aktiviert; weder Quelle noch Leistungsgrenzwert wurden
aufgrund der fehlgeschlagenen Fälle verändert.


Android-Nachtrag zum selben Lauf: Abruf 22:40:17 UTC und gebundenes Artefakt
liefern A01–A03; M14 ergänzt nach zwei Quellenpässen den Funktionsplan.
Alle Build-/Teststufen sind beendet. Ohne Handoffprovenienz kann kein Gerätejob
mehr starten. Der Plan präzisiert deshalb die Startgrenze auf das tatsächliche
Ende dieser Arbeitsstufen; bloße Cache-Nachbereitung ist kein zusätzlicher
Haltepunkt. Modellwahl und getrennte Fixaufträge folgen jetzt, ohne neue Suite.


M14/A01–A03 ist quellseitig samt vollständigem eigenem Fixzyklus abgeschlossen:
Compose-Memberimport korrigiert, isolierte Loginbitmap-Application und exakte
geometrische x/y-Vergleiche mit Delta null. Main bestätigt den begrenzten Diff,
statische Syntax und unveränderte bestehende Auswahl. Keine Produktionsdatei
oder Rotationspolitik ist verändert, kein Laufzeitergebnis vorweggenommen.
Dieser Absatz beschreibt den Integrationsstand von M14; der folgende Nachtrag
führt den aktuellen Fortschritt. Alle Fixautoren sind ausdrücklich mit
Sol 6.1/max über Codex konfiguriert; Antwortmodellmetadaten fehlen im Host.

M05 (`515e8286`), M07 (`da059df3`) und M02 (`4d931637`) sind nach vollständigem
eigenem Autorenzyklus statisch integriert und auf Main/Fixbranch gepusht.
Die Registrykorrektur vermeidet ein künstliches Nullfeld, die P27-Fixture
ersetzt nur ihre eigene Profilwurzel, und relative Ordneraliase behalten ihre
gültige Bedeutung. Vorhandene Quittungs-, Konflikt- und Schreibschutzgrenzen
bleiben erhalten. Die Laufzeitbestätigung erfolgt gemeinsam nach sämtlichen
M01–M14-Korrekturen. Anschließend wurden M03 (`252f3fe8`), M06 (`a6a54ae3`),
M08 (`c7c49f12`), M12 (`9dde70f1`) und M11 (`625df513`) mit vollständigen
Autorenberichten und statischer Integrationsprüfung gepusht. M04 (`49974aa4`),
M01 (`3c54f558`), M09 (`5209fe97`) und M13 (`98a9ed1f`) sind ebenfalls nach
vollständiger eigener Autorenreview integriert. M10 schließt die ausdrücklich
belegten verbundenen Root-/Parent-/ID-Fixturefehler ab; kein Paket wartet auf
seinen Start. Die gemeinsame Manifestintegration und der vollständige
Rootgraph folgen nach dem letzten Fix.
Der finale Abruf des bisherigen Laufs um 23:10:44 UTC
bestätigt dessen Ende und den übersprungenen Gerätejob.


### Releaseabschluss angefordert, 2026-10-27

Alle M01–M14 sind nach vollständiger eigener Autorenreview integriert; letzter
Quellcommit `5be2ba02`. Die gemeinsame Manifestintegration ergänzt die gesammelten
Fälle ohne bestehende Auswahl zu entfernen. Auf den unmittelbaren Releaseauftrag
des Nutzers folgt kein weiterer Tasklauf und keine zusätzliche Reviewrunde.
Die zuvor geplante Suitewiederholung entfällt für diesen Abschluss. Der letzte
ausgeführte Lauf `40406246032` bleibt fehlgeschlagen; die anschließenden Fixes
sind statisch geprüft, ihre erneute funktionale Bestätigung fehlt ausdrücklich.
Nach Rootgraph und Kandidatenpush übernimmt einmal der vorhandene vollständige
Remote-Releaseweg. Signatur-, Hash-, Versions- und Publikationsintegrität bleiben
verbindlich. Releaseerfolg wird erst mit sichtbarer vollständiger Veröffentlichung
behauptet.


### Release 0.5.185 abgeschlossen, 2026-10-27

Der vollständige [Remote-Release 40450627176](https://github.com/b1ue-man/smart-explorer/actions/runs/40450627176)
auf Quellkandidat `662aa0e5aa76df64ed36412e3675915645fff1ec` ist erfolgreich.
Er enthält den gesamten Batch B1–B8/P31/P32, M01–M14 und R01/R02.
Android, Windows und Linux einschließlich der statischen musl-Programme
wurden durch denselben bestehenden Releaseweg gebaut. Es wurde keine
zusätzliche funktionale Task-Suite gestartet.

Der Wrapper erzeugte Releasecommit `278a827fdae21c25446f717972cd9adc3732c6e0`
auf `main` und den unveränderten Tag `v0.5.185`. Der zugehörige einzige
[Publikationslauf 40463241931](https://github.com/b1ue-man/smart-explorer/actions/runs/40463241931)
veröffentlichte die vorhandenen Artefakte ohne erneuten Build oder Tasklauf.
Sein Job `publish-release` ist erfolgreich. Der vollständige Wrapper meldete
am 2026-10-27 um 06:54:04 UTC die erfolgreiche vollständige Veröffentlichung.
Die [GitHub Release](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.185)
ist seit 06:53:44 UTC sichtbar, kein Entwurf und keine Vorabversion.

Abschlussabgleich durch Main am 2026-10-27: Tag und Release zeigen auf
`278a827f`; `native/Cargo.toml`, `release-native/update-feed/version.txt`
und der Installer stimmen auf 0.5.185 überein. Die exakten 26 Namen aus
`Get-PublicationReleaseAssetMap` sind veröffentlicht. Größe und SHA-256 jedes
GitHub-Assets stimmen mit den getaggten lokalen Bytes überein. Sämtliche
neun `.sha256`-Dateien des Feeds stimmen mit ihren Nutzdateien überein.
APK-Signatur und versionsgebundene Übergabe wurden im Remotejob geprüft;
die Publikationspipeline prüfte dieselben Bytes vor Veröffentlichung erneut.

Die früheren Abschnitte sind historische Zwischenstände. Der letzte
funktionale Lauf `40406246032` blieb fehlgeschlagen; nach den integrierten
Korrekturen wurde auf den unmittelbaren Releaseauftrag hin keine erneute
Funktionssuite ausgeführt. Der Releaseerfolg ersetzt weder diesen fehlenden
Laufzeitnachweis noch eine Geräte- oder Leistungsabnahme. Der beauftragte
terminal veröffentlichte Batch ist abgeschlossen; es läuft keine weitere
Releasepipeline und es wird keine zusätzliche Version erzeugt.
