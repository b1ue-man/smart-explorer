# G6: versionsgebundene Screen-Qualitätsabnahme

Stand: 2026-10-26. Fixautor: delegierter Codex-Agent `fix_quality_acceptance`.

## Auftrag und Erstplan aus dem Prompt

Quelle: delegierter G6-Auftrag mit `/tmp/smartexplorer-final-review-user-contract.txt`
und `/tmp/smartexplorer-fix-quality-acceptance-scope.json`. Derselbe Autor besitzt
die Funktionsauflistung, vollständige direkte und indirekte Verknüpfungskarte,
Planung, Umsetzung und anschließende Anti-Regressionsreview dieses Fixes.

Ergebnis: V7 vergleicht den belegten versionsgebundenen Vorherpfad mit dem
Kandidaten gegen dieselben Captureframes; Codecfehler und Transportschäden werden
getrennt ausgewiesen. Referenzverlust, erste bestätigte Keyframeerholung und P31s
verlustfreier Stillstand erhalten überprüfbare Qualitätsbelege. Fehlende physische
Codecvarianten bleiben ausdrücklich ungeprüft. G6 bezeichnet eine belegte
Abnahmelücke, keinen bereits gemessenen Produktqualitätsfehler.

Promptwissen: Die vorgegebene Releasebasis ist
`a814f263096fe69d3746e840b5f3fad3fd7e88a0` (`v0.5.184`), der ursprüngliche
Reviewkandidat `ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b`. Gleichzeitige Autoren
ändern ausschließlich ihre zugewiesenen Flächen. Suite-, CI-, Commit-, Push-,
Graphupdate- und Releaseverantwortung bleibt bei Main. Es gibt keinen lokalen
Compiler-, Formatter-, Test-, Build- oder Serverlauf dieses Fixautors.

Zunächst durch Bestandslektüre zu klären: bestehende Metrikdefinitionen, tatsächliche
Vorher-/Kandidatenpfade, Codecverfügbarkeit, Referenz-/Keyframebelege, Transport-
und P31-Rekonstruktionsgrenzen sowie Registrierung in Mains gemeinsamer Suite.
Es fehlt zu Beginn keine zwingend vom Nutzer zu liefernde Angabe.

Vorläufige Planungstiefe: zusammenhängende fachliche Untersuchung der gesamten
Qualitätsbelegkette, weil dieselbe Captureidentität, Decoder-Ausgabeidentität und
Referenzgeneration über Codec, Transport und Darstellung erhalten bleiben muss.
Nächster konkreter Schritt: G6 und V7 am aktuellen Quellstand abgleichen, die
betroffenen Funktionen und ihre Aufrufer selbst verfolgen, danach den belegten
Zweitplan und die Detailmeilensteine vor der Quelländerung festhalten.

## B — Eigene Funktions- und Lückenauflistung

Die folgenden vier Funktionen der beauftragten Abnahme sind unvollständig; daraus
wird kein gemessener Produktqualitätsfehler abgeleitet:

1. Der versionsgebundene Codecvergleich fehlt. `batches_4k_task_tests.rs` und
   `batches_reconstruction_task_tests.rs` prüfen Pixelidentität, aber weder SSIM
   noch einen eingefrorenen Vorhercodec. `openh264_codec_tests.rs` enthält einen
   lokalen PSNR-Bench ohne Vorhervergleich.
2. Die Zuordnung von Captureframe und Qualitätswert fehlt. Ein Barcode oder ein
   aktueller Decoderaufruf ersetzt keine bestätigte Ausgabeidentität; der
   Kandidat besitzt dafür `DecodedFrame` und `FrameIdentities`.
3. Codecfehler, absichtlich verworfene Transportreferenzen und das gültig gehaltene
   Bild besitzen keinen getrennten Qualitätsbericht. `ReferenceDecoder` und
   `HybridDecoder` schützen bereits ihre Referenzen; dieser Fix ersetzt sie nicht.
4. P31s verlustfreier Stillstand besitzt bereits direkte Pixelgleichheitsfälle,
   aber keinen gemeinsamen Qualitätsbeleg mit gemessener Codecbewegung und
   anschließender produktiver Stillstandskorrektur.

Wiederverwendung: dieselbe gepinnte OpenH264-Bibliothek, der vorhandene
`TEST_LOCK`, der temporäre verifizierte OpenH264-Store, die produktiven
`CodecRegistry`-/`CaptureGrant`-/`CapturedHybrid`-Grenzen, `FrameRecord`, die
bestehenden Rekonstruktionsfälle und Mains unveränderter gemeinsamer Suiteeinstieg.

### Vollständige direkte und indirekte Verknüpfungskarte des G6-Änderungswegs

| Grenze | Direkte und indirekte Verknüpfungen und zu erhaltendes Ergebnis |
|---|---|
| Auftrag → Auswahl → Evidenz | V7/P7/P30/P31 in `vertraege.md`, B6 in `batches.md`, G6 in `review-gesamt.md` → `review-task-manifest.json` → `review-task-native.py::native_tests`/`milestone_evidence` → `test-review-task.sh` → `.github/workflows/review-task.yml`. Nur Main ändert diese Auswahl und führt sie später remote aus. Ein `SE_TASK_UNVERIFIED_JSON` wird aus der Passmenge entfernt. |
| Referenzcapture → Eingang | Deterministisches BGRA-Capture mit Frame-ID, Capturezeit, Geometrie und SHA-256 → `CaptureImageBuilder` → `CaptureGrant`, `SourceKey`, `CaptureGeneration` → `HostOutputPlan` mit OpenH264 und `SessionAdmission`. Beide Vergleichsarme erhalten exakt dieselben Quellpixel; Kandidatenbesitz und Freigabe bleiben produktiv geprüft. |
| Vorherarm | Commit `a814f263` → originale Quellblöcke aus `pixel_convert.rs` und `openh264_codec.rs` → ausschließlich testseitige, bytegleich überprüfte Konvertierung und Encoder-/Decoderimplementierung → derselbe verifizierte Bibliothekspin. Der Arm behauptet keine Ausführung einer unveränderten alten App und keine damals noch fehlende Ausgabe-ID. |
| Kandidatenarm | `CodecRegistry::open_encoder_for_stage_admitted` → `OpenH264EncoderFactory::open_admitted` → `openh264_capture::{capture_limit,admitted_i420,copy_access_unit}` → `OpenH264Encoder::encode_captured`; bei v2 zusätzlich `host_hybrid::wrap_captured` → `CapturedHybrid::{captured_impl,needs_refresh}` → `CapturedRecords`. Geometrie, Vertragsbindung, Eigentümer und Abbruch bleiben unverändert. |
| Decoder und Referenz | `CodecRegistry::open_decoder` → `OpenH264Decoder::decode` mit SPS-/Geometrieprüfung und echter `FrameIdentities`-Ausgabe → `ReferenceDecoder`; bei v2 davor `HybridDecoder` → `frame_protocol::{decode,apply_dirty}`, `frame_regions::copy_rect`. Nur ein bestätigter vollständiger Commit darf die Referenz ersetzen; Video, Moves und Dirty teilen denselben Frame. |
| Transportfehler und Wiederanlauf | `GopWriter::reset`/`read_picture_stream` → `PictureEvent::Broken`, `RecordSequence` und GOP-Wechsel → verworfener Decoderzustand → abhängige Frames bleiben unbestätigt → erste vollständige Keyframegeneration bestätigt ein neues Bild. Die Fixture injiziert einen dokumentierten Abbruch an der Decodergrenze; sie behauptet keinen gemessenen QUIC-Paketverlust oder Netzwerkdurchsatz. |
| Darstellung Desktop | `app/core/screen_presented.rs::{begin_frame,can_present,presented,invalidate_presentation}` bindet bestätigte Bilder an Sitzung, GOP, Frame und Rendergeneration; Empfang allein öffnet keine Eingabe. Der Qualitätsfix ändert diesen Pfad nicht und beansprucht keinen Rendererpass. |
| Android Host/Viewer | `ScreenEncoder.kt::queueRgba` hält Original-RGBA, Input-PTS und MediaCodec-Ausgabe zusammen → `screen_capture_jni.rs`/`screen_jni.rs` → `native/src/mobile/os/shared/domains/screen_host.rs::{screen_push_admitted,screen_capture_claim,screen_capture_release}` → `bridge_channels::{push_admitted,channel}`/Capturearena → `BridgeSource`. In Gegenrichtung: `screen_view_run.rs::{run,on_event,pull,ViewSession::send}` → `ViewerSink::{record,broken}`/`PullQueue::pull_into` → JNI `screenPull` → `ScreenDecoder.kt::accept` und `ScreenHybridDecoder.kt::{accept,decodeVideo}` → `ScreenSession.kt` mit `ScreenPresentedFrame` und `presented` → `ScreenApi.viewPresented` und die bestehenden Screen-Verträge. Physische MediaCodecvarianten bleiben separat ungeprüft; ein nativer Softwarecodecwert wird ihnen nicht zugeschrieben. |
| Qualitätsauswertung | Festgehaltenes Capture → Original gegen eingefrorene Ausgabe, Original gegen Kandidat, saubere Kandidatenausgabe gegen beeinträchtigte Ausgabe und gehaltenes gültiges Bild gegen aktuelles Original → RGB-SSE/PSNR, Luma-SSIM und vollständige Pixel-/Alphaabweichung → kandidatengebundene JSON-Evidenz. Fehlende Ausgabe hat keinen erfundenen Qualitätswert. |

Diese Karte verfolgt den G6-Änderungsweg bis zu seinen betroffenen Produkt- und
Auswertungsgrenzen. Berechtigungen, Eingabe, Netzrouting, Persistenz, Android-UI
und Codecpräferenzen werden nicht geändert. Sie behauptet keine Prüfung sämtlicher
unbeteiligter Programmdomänen.

## C — Erste Recherche und Entscheidungsplan

Am 2026-10-26 geprüft: [Wang et al., SSIM 2004, Gleichungen 12–17](https://ece.uwaterloo.ca/~z70wang/publications/ssim.pdf)
und [Autorenreferenz](https://ece.uwaterloo.ca/~z70wang/research/ssim/).
Entscheidung: Single-Scale-Luma-SSIM, gültige 11×11-Gaußfenster, Sigma 1,5,
K1=0,01, K2=0,03 und L=255; Mittelwert über alle gültigen Zentren. Die
RGB-PSNR verwendet die gesamte RGB-SSE und 3×Pixelzahl. Alphaabweichung und
vollständige Pixelabweichung werden zusätzlich gezählt. Exakte RGB-Gleichheit
bedeutet PSNR=unendlich, ausdrücklich getrennt von fehlender Messung. Ein
Wahrnehmungsversprechen wird daraus nicht abgeleitet.

Alternativen: Der bestehende statische PSNR-Bench alleine erfüllt weder SSIM noch
Versionierung; eine neue FFmpeg-/SSIM-Laufzeitabhängigkeit würde einen zweiten
Codec und neue Bereitstellung benötigen; ein vollständiger Altappbuild würde die
verbotene zweite Build-/Suitekette erzeugen. Gewählt ist daher ein testseitiger
Originalquellauszug mit Laufzeitvergleich seiner Quellblöcke gegen den bekannten
Git-Commit sowie ein kleiner unabhängig nachgerechneter Metrikkern.

Der physische Vergleich hält Geometrie, Framerate, Zielrate, Encoderkonfiguration,
Capturefolge und Bibliothek konstant. Damit prüft er die Codecgrenze vor/nach der
Änderung; er misst weder die veränderte Ratenregelung noch eine vermeintliche
Gleichwertigkeit unterschiedlicher Frameraten. P30s 4K-Korrektur bleibt ein
getrennter vollständiger Pixelbeleg, kein erfundener Echtzeitcodecvergleich.

## D — Konkreter Detailplan und zweite Lückenrecherche

| Meilenstein | Dateien und Abhängigkeiten | Konkretes späteres Abnahmesignal |
|---|---|---|
| Q1 Metrik und Capturebindung | Neue testseitige `quality_acceptance_metrics.rs`, `batches_quality_acceptance_task_tests.rs`, Registrierung in `core/mod.rs` | Analytische konstante Motive und bekannte Einzelpixelabweichung ergeben erwartete SSIM/PSNR; Geometrie-/Captureverwechslungen werden abgewiesen. |
| Q2 Eingefrorener Vorherpfad | Neue testseitige `core/quality_acceptance_baseline.rs`, `os/native_ffi/quality_acceptance_baseline.rs` und `quality_acceptance_provenance.rs`; beide zusätzlichen OS-Dateien sind von Main ausdrücklich freigegeben | Originalblöcke stimmen bytegleich mit den per SHA-256 verifizierten Git-Dateien aus `a814f263` überein. Abweichung oder fehlendes Git-Objekt ist Fehler, kein Ersatzbaselinepass. |
| Q3 Physische Codec-/Transport-/Stillstandsevidenz | Neue `os/native_ffi/batches_quality_acceptance_task_tests.rs`, testseitige Registrierung in `openh264_codec.rs` | Derselbe Capturedigest beider Arme; tatsächlicher gepinnter OpenH264; getrennte Codecmetriken, Abbruch-/Reorderfälle, verworfene abhängige Ausgabe, erste IDR-Erholung und pixelgleiche P31-Korrektur über den zugelassenen Kandidatenpfad. Fehlender Codec bleibt ungeprüft. |
| Q4 Bestehende deterministische Teilbelege | `batches_4k_task_tests.rs`, `batches_reconstruction_task_tests.rs` | Vorhandene Pixel-/Referenzverträge bleiben erhalten und liefern ehrliche SSIM-/PSNR-/Pixelbelege mit ausdrücklich begrenzter Aussage. |
| Q5 Eigene Anti-Regressionsreview und Übergabe | Dieser Bericht; Main besitzt Suite/CI/Graph/Commit/Release | Sämtliche ermittelten G6-Verbindungen sind auf unveränderte Produktverträge geprüft; genaue Fallnamen und Evidenzregistrierung stehen für Mains eine gemeinsame Remote-Suite bereit. |

Zweite Recherche vor Quelländerung: Die Releasebasis enthält OpenH264 und
openh264-sys2 0.9.9; der aktuelle Lock hält dieselben Versionen. Die
[0.9.9-Decoder-API](https://docs.rs/openh264/latest/openh264/decoder/struct.Decoder.html)
liefert `Result<Option<DecodedYUV>>`, kann bei fehlender Ausgabe puffern und
verlangt den Erhalt der ausgeliehenen Ebenen bis zur Kopie. Die
[0.9.9-EncoderConfig](https://docs.rs/openh264/latest/openh264/encoder/struct.EncoderConfig.html)
belegt die vorhandenen Konfigurationsaufrufe. Die abgerufenen Seiten nennen
explizit 0.9.9; die versionsnumerischen Docs.rs-URLs waren im Webwerkzeug nicht
abrufbar. Die [Paket-VCS-Referenz](https://docs.rs/crate/openh264/latest/source/.cargo_vcs_info.json)
nennt `68f56e8443f1d619a9d96af79dd7f836442b6d76`. Es wird keine neue API eingeführt.

Die zweite Recherche bestätigt außerdem: Der produktive Kandidat benutzt
`encode_captured`, nicht den rohen Benchpfad. Der Qualitätsfall muss deshalb
seinen `HostOutputPlan` und seine `CaptureGrant` verwenden. Der Vorherdecoder
besitzt keine echte Ausgabeidentität; nur eine lückenlose synchrone Folge mit
genau einer Ausgabe ab dem ersten Input und leerem Restpuffer ist als gepaarter
Vorhervergleich zulässig. Sonst schlägt die Zuordnung fehl. Sämtliche
Laufzeitbelege bleiben bis Mains Remoteausführung offen.

## Umsetzungsergebnis der G6-Abnahmefixtures vor der Modulgrößenfortsetzung

Q1–Q4 sind in der freigegebenen Fix-Arbeitskopie quellseitig umgesetzt. Der
Vorherarm enthält originale Konvertierungs-, Encoder- und Decoderblöcke aus der
vorgegebenen Releasebasis. Seine Herkunftsprüfung bindet diese Blöcke bytegleich
an die verifizierten vollständigen Git-Dateien. Die Datei mit der Herkunfts-IO
ist vom Codecadapter getrennt; keine Produktionsfactory wurde ersetzt.

Der Kandidatenarm verwendet `open_encoder_for_stage_admitted` und
`encode_captured`, einschließlich produktiver Ressourcenbesitzer. Verglichen
werden 320×192, 192×320 und 640×360 jeweils bei 150.000 und 2.000.000 bit/s,
konstanter logischer Eingangskadenz und identischer Konfiguration. Original,
Vorherausgabe und Kandidatenausgabe werden als BGRA-Dateien mit Geometrie und
SHA-256 für die spätere gemeinsame Auswertung aufbewahrt. Das ist kein Beleg
einer tatsächlich erreichten Framerate.

Für die Codecgrenze gilt ohne zusätzliches Qualitätsbudget: Kandidaten-SSE darf
nicht größer, Luma-SSIM nicht kleiner und Alphaabweichung nicht größer als im
Vorherarm sein. Die SSIM-Toleranz von 10⁻¹² deckt ausschließlich numerische
Summationsabweichung ab; für RGB-SSE gibt es keine Toleranz. Transportschäden
werden getrennt gegen die saubere Kandidatenausgabe und gegen die für die
Analyse aufbewahrten zuletzt bestätigten Pixel ausgewiesen. Fehlende Ausgabe
bleibt `null` und wird weder als PSNR=unendlich noch als perfekte SSIM behandelt.
Ein aufbewahrtes Pixelbild ist kein beobachteter Rendererzustand.

Die physische Fehlerfolge berücksichtigt tatsächliche, auch automatisch vom
Encoder erzeugte Schlüsselbilder. Sie verwirft abhängige Frames während der
ungültigen Referenz und verlangt die Wiederherstellung beim ersten bestätigten
IDR nach jedem Fehler. Der produktive Hybridfall verbindet Bewegung mit einer
darauffolgenden Stillstandskorrektur ohne Videooperation; seine vollständigen
Pixel müssen dann gleich dem Original sein. Der vorhandene 4K-Fall zählt jetzt
zusätzlich Begin, Commit und Recordheader für seinen ausdrücklich modellierten
Bytes-/Raten-Untergrenzwert; gemessene Pipelinezeit bleibt `null`.

## Eigene anschließende Anti-Regressionsreview der G6-Abnahmefixtures

Gegenstand waren die vollständig oben ermittelten G6-Aufrufer, Datenübergänge,
Identitäten, Ressourcenbesitzer, Referenzzustände, Qualitätswerte und die konkrete
Registrierung in Mains gemeinsamer Suite. Maßstab waren V7, die originale
Releasequelle, die gepinnten API-Verträge und das Verbot, ungeprüfte physische
Codecs als bestanden zu werten.

| Prüffrage | Eigener statischer Befund und Auswirkung |
|---|---|
| Bleibt Produktverhalten erhalten? | Sämtliche neuen Module werden ausschließlich mit `#[cfg(test)]` registriert. Die Änderungen in `core/mod.rs` und `openh264_codec.rs` ergänzen nur diese Registrierung. Die übrigen Produktfunktionen für Codecwahl, Berechtigung, Sitzung, UI, Transport, Eingabe, Persistenz und Abbruch sind durch diesen Fix nicht verändert. |
| Ist der Vorherpfad tatsächlich alt? | Originalblöcke für Pixelkonvertierung, Encoder und Decoder wurden selbst gegen `a814f263` verglichen; alle sind bytegleich. Laufzeitseitig werden vollständige Originaldatei-SHA, Originalblöcke, Crateversionen, Bibliothekspin und damalige Bitratengrenzen geprüft. Der Adapter behauptet keine alte Appausführung und keine damals noch fehlende Ausgabeidentität. |
| Können Ausgaben dem falschen Capture zugeschrieben werden? | Beide Arme erhalten dieselben Quellpixel; die kopierten zugelassenen Kandidatenpixel und der unveränderte Originaldigest werden verglichen. Kandidatenausgabe muss ihre echte Frame-/Capturezeit-/Keyframeidentität melden. Der Vorherarm wird nur bei genau einer synchronen Ausgabe ab dem ersten Input und leerem Decoderrestpuffer zugeordnet. |
| Verdecken Metriken einen Qualitätsverlust? | Geometrieabweichungen sind Fehler. RGB-SSE/PSNR und Luma-SSIM werden je Frame separat für beide Arme geführt; Alpha und vollständige Pixelabweichung bleiben sichtbar. Die analytischen Fälle enthalten exakte Gleichheit, Schwarz/Weiß, Einzelpixelabweichung und eine unabhängig aus Gaußsymmetrie berechnete kontrastinvertierte Rampe. Ein perfektes Referenzbild akzeptiert die Einzelpixelverschlechterung nicht. |
| Werden Referenzfehler als Codecfehler gemischt? | Saubere Codecqualität, Fehlerzustand, zurückgehaltene Ausgabe, zuletzt bestätigte Pixel und wiederhergestellte Ausgabe besitzen getrennte Felder. Reorder vergiftet die Referenz; GOP-Abbruch erzeugt einen frischen wartenden Decoder. Die erste gültige Keyframeausgabe muss zur sauberen Ausgabe desselben AU pixelgleich sein. Das ist ein kontrollierter Decodergrenzfall und keine reale Netzverlustmessung. |
| Bleibt atomare P31-Rekonstruktion erhalten? | Bestehende Begin/Video/Moves/Dirty/Commit- und verspätete Ausgabeidentitätsfälle bleiben erhalten. Der neue deterministische Qualitätsfall verlangt, dass eine unvollständige Schlüsselgeneration und ein abhängiger Frame unbestätigt bleiben; erst ein vollständiger neuer Commit darf die exakten Pixel veröffentlichen. Der zusätzliche physische Hybridfall verwendet die produktive zugelassene Quelle und überprüft Stillstand und Wiederanlauf. |
| Bleibt Ressourcenbesitz richtig? | Die Produktseite der Fixture hält `CaptureImageBuilder`, Grant, kodierte Records und Hybridreferenz bis zur tatsächlichen Nutzung. Nach Encoderabschluss muss der Grant keine lebenden Bytes mehr besitzen. Rohdateien für die Analyse sind ausdrücklich Fixtureartefakte und werden nicht als Produktpufferbilanz ausgegeben. Der testseitige SSIM-Ringspeicher wächst nur mit der Bildbreite. |
| Sind alle bestehenden Oberflächen berücksichtigt? | Die gelesenen Desktop-, Android-, mobilen Fassaden- und JNI-Grenzen bleiben unverändert. Die Qualitätsauswertung wird keiner Oberfläche als neue Bedienfunktion zugeschrieben. Physische MediaCodec-/MF- und Rendererwerte werden durch einen Softwarecodecwert nicht ersetzt. Der parallel entwickelte Screenbudgetfix bleibt Eigentum seines Autors und Mains Integration. |
| Ist Mains Registrierung konkret passend? | Nach Mains Ergänzung wurden `review-task-manifest.json`, `review-task-native.py` und `review_task_quality.py` selbst abgeglichen. Alle fünf Labels, sechs Profiltripel, Kandidaten-/Releasebindung und SHA-/Größenfelder passen. Der Helper liest vorhandene Artefakte nach `native_tests`; er startet keine zweite Fixture. Der umgebungsabhängige OpenH264-Fall wird aus der Passmenge entfernt, wenn sein physischer Codec fehlt. |

Im eigenen Schlussreview korrigiert: Ein nach der Helpertrennung falscher
Metriknamensraum wurde berichtigt. Die Fehlerfolge wurde von fest angenommenen
Keyframepositionen auf tatsächliche Codecflags umgestellt. Herkunftsprüfung,
Captureaufbau und Metrikartefakte wurden nach Verantwortung getrennt, damit die
Dateien unter der Projektgröße bleiben. Diese Korrekturen wurden selbst erneut
statisch gelesen; kein Reviewer und kein zusätzlicher Autor hat einen Teil
dieses G6-Autorzyklus übernommen.

Statische Belege: `git diff --check` für die geänderten vorhandenen Dateien;
lesender Tree-sitter-Rust-Parse aller eigenen neuen/geänderten Rustdateien ohne
ERROR- oder missing-Knoten; eigener exakter Text-/SHA-Abgleich der drei
eingefrorenen Quellblöcke. Tree-sitter wurde aus dem bereits installierten
Graphify-Python verwendet, ohne Installation oder Graphänderung. Dies belegt
weder Rusttypen noch Linkbarkeit noch ausgeführte Codecqualität. Die
Graphnavigation meldete Skill 0.9.11 bei Paket 0.9.17; sie lieferte nur
Navigation, keine erfundenen Aufrufkanten und keinen Laufzeitpass.

Der finale Originalblockabgleich nach der Dateitrennung bestätigte dieselben
bytegleichen Blöcke mit SHA-256:

- Pixelkonvertierung: `639417c6d401ad922adab2f3332e8d0573c857d2a891410c64109bfd5cd7cda6`.
- Encoder: `55864ef7c160352eb14d1bf7e855083b61d1c6ec10b88f77b3827ed8e8351679`.
- Decoder: `14fa85e0e95d9ff53da245f6bc0e22b27d0a2225cfbb28c041b263ec6f5a629f`.

Der finale Diffvergleich der beiden Produktmodule zeigt ausschließlich die
`#[cfg(test)]`-Registrierungen. Alle eigenen Rustdateien liegen vor der von Main
verwalteten Remoteformatierung unter 500 Zeilen und 50 KiB. Die thematische
Dokumentationssuche fand in diesem Bericht keinen verbliebenen offenen
Platzhalter und keine widersprüchliche Behauptung eines aktuellen Releases.

### Exakte vorbereitete Fälle und Main-Registrierung

| Fallname | Erwarteter Beleg |
|---|---|
| `batches_task_b6_quality_metrics_match_independent_analytic_oracles` | `quality-acceptance/analytic-metrics.json` |
| `batches_task_b6_quality_binding_rejects_foreign_capture_and_missing_geometry` | Identitäts-/Geometrieabweichung und Alphaunterschied bleiben als eigener erfolgreicher Fall erforderlich; kein physischer Codecpass |
| `batches_task_b6_lossless_correction_matches_every_source_pixel` | `quality-acceptance/deterministic-lossless-still.json` |
| `batches_task_b6_quality_reference_loss_recovers_only_with_complete_keyframe` | `quality-acceptance/deterministic-reference-recovery.json` |
| `batches_task_b6_4k_lossless_correction_retains_every_source_pixel` | `quality-acceptance/4k-lossless-still.json` |
| `batches_task_b6_quality_openh264_baseline_transport_and_still` | `quality-acceptance/openh264-baseline-transport-still.json` und die darin benannten BGRA-Dateien; umgebungsabhängig, ohne Bibliothek kein Pass |

Alle JSON-Berichte besitzen `schema=1`, `candidate=CANDIDATE_SHA`, das eindeutige
Fixturelabel, die Metrikdefinition und `observations`. Der physische Bericht
bindet `observations.provenance.release_commit` an
`a814f263096fe69d3746e840b5f3fad3fd7e88a0`; seine Reihenfolge ist 320×192/150.000,
320×192/2.000.000, 192×320/150.000, 192×320/2.000.000, 640×360/150.000,
640×360/2.000.000. `observations.result` ist nur nach vollständiger Auswertung
`passed`; ein Qualitätsrückgang ergibt `failed` und einen fehlgeschlagenen Fall.
Main hat die Bindung als `R11-G6-Bildqualitaet` registriert und liest die
vorhandenen Artefakte mit `native/review_task_quality.py`; dieser Fixautor hat
weder Manifest noch Orchestrierung bearbeitet.

## Dateien, Entscheidungen und Grenzen der G6-Abnahmefixtures

Erstellt:

- `docs/plaene/2026-10-24-batches/fix-quality-acceptance.md`
- `native/src/screen/core/batches_quality_acceptance_task_tests.rs`
- `native/src/screen/core/quality_acceptance_baseline.rs`
- `native/src/screen/core/quality_acceptance_metrics.rs`
- `native/src/screen/os/native_ffi/batches_quality_acceptance_task_tests.rs`
- `native/src/screen/os/native_ffi/quality_acceptance_baseline.rs`
- `native/src/screen/os/native_ffi/quality_acceptance_provenance.rs`

Geändert:

- `native/src/screen/core/batches_4k_task_tests.rs`
- `native/src/screen/core/batches_reconstruction_task_tests.rs`
- `native/src/screen/core/mod.rs`
- `native/src/screen/os/native_ffi/openh264_codec.rs`

Eigene Quelllektüre, jeweils die zu G6 gehörigen Abschnitte:

- Regeln und Auftrag: `/root/.codex/AGENTS.md`, beide lokalen
  `arbeitsweise/SKILL.md`, `graphify/SKILL.md`, Projekt-`AGENTS.md`,
  `/tmp/smartexplorer-final-review-user-contract.txt` und der eigene Scope.
- Dokumentation und Graph: `docs/ARCHITEKTUR.md`, `docs/oberflaechen/INDEX.md`,
  `docs/oberflaechen/bildschirm.md`, `batches.md`, `vertraege.md`,
  `review-gesamt.md` aus der von Main genannten ursprünglichen Arbeitskopie,
  thematische Treffer in `regressionsreview.md` und `docs/refs/rv1-remote-suite.md`,
  Graphify-Abfragen des freigegebenen Rootgraphen.
- Gemeinsame Registrierung: `.github/workflows/review-task.yml`,
  `native/review-task-manifest.json`, `native/review-task-native.py`,
  `native/test-review-task.sh`, `native/review_task_quality.py`,
  `native/Cargo.toml` und `native/Cargo.lock`.
- Screenkern: alle eigenen oben genannten Rustdateien sowie `pixel_convert.rs`,
  `decoded_frame.rs`, `metrics.rs`, `synthetic.rs`, `frame_budget.rs`,
  `frame_size_caps.rs`, `frame_reference.rs`, `viewer_reconstruct.rs`,
  `host_hybrid.rs`, `captured_hybrid.rs`, `host_stage.rs`, `host_mode.rs`,
  `rate_control.rs`, `codec_registry.rs`, `capture_records.rs`,
  `capture_grant.rs`, `capture_admission.rs`, `capture_source_contract.rs`,
  `capture_test_support.rs`, `host_codec_adapters.rs`,
  `host_codec_phase_plan.rs`, `batches_capture_pipeline_task_tests.rs`,
  `viewer_client.rs` und `gop_stream.rs`.
- Native Adapter: `screen/os/mod.rs`, `native_ffi/openh264_codec_tests.rs`,
  `native_ffi/openh264_encode.rs`, `native_ffi/openh264_capture.rs`,
  `native_ffi/openh264_pins.rs`, `native_ffi/batches_capture_codec_task_tests.rs`,
  `android/bridge_channels.rs`, `android/bridge_source.rs` und
  `android/viewer_sink.rs`.
- Oberflächengrenzen: `app/core/screen_presented.rs`, die freigegebenen mobilen
  `domains/screen_host.rs` und `domains/screen_view_run.rs`,
  `native/android-bridge/src/screen_capture_jni.rs` und `screen_jni.rs`,
  Android `screen/ScreenEncoder.kt`, `ui/screen/ScreenDecoder.kt`,
  `ui/screen/ScreenHybridDecoder.kt`, `ui/screen/ScreenSession.kt` und
  `api/ScreenApi.kt`.
- Originale Git-Dateien aus `a814f263`: `native/Cargo.lock`,
  `screen/core/pixel_convert.rs`, `screen/core/rate_control.rs`,
  `screen/core/host_mode.rs`, `screen/os/native_ffi/openh264_codec.rs` und
  `screen/os/native_ffi/openh264_pins.rs`; Screenpfade jeweils unter `native/src/`.

Entscheidung: Keine Produktionsparameteränderung und keine neue Abhängigkeit;
kein zweiter Build, keine zweite Suite und kein eigener Releasezyklus. Die
Quelländerung mit eigener Funktionsauflistung, vollständiger G6-Verknüpfungskarte,
konkretem Plan, eigener Umsetzung und eigener anschließender Review ist
abgeschlossen. Innerhalb der exklusiven Schreibfläche bleibt keine bekannte
unbehobene G6-Quelllücke.

Praktische Grenzen bleiben ausdrücklich offen: Main führt später die eine
gemeinsame Remote-Suite einschließlich Typ-/Compilerprüfung aus und besitzt
Graphrefresh, Commit, Push und den terminalen Remote-Release. Kein lokaler
Compiler, Formatter, Test, Build, Server, Commit, Push, Graphupdate oder Release
wurde ausgeführt. Die native OpenH264-Qualität ist bis dahin ungemessen.
Windows-MF synchron, Windows-MF asynchron und Android-MediaCodec-Qualität bleiben
jeweils ungeprüft; die konkreten physischen Androidcodecvarianten sind ohne
Geräteinventur unbekannt. Die kontrollierten Abbrüche belegen keine reale
Paketverlustzeit, und ein 4K-Bytes-/Ratenmodell belegt keine gemessene Pipelinezeit.
Fehlende physische Codecs sind kein Pass und kein eigenständig erfundener
Releaseblocker.

## Fortsetzung: fachliche Modulgrößenkorrektur nach Remoteformatierung

### A — Erhaltener Auftrag und Erstplan

Quelle: Mains Folgeauftrag vom 2026-10-26 und aktualisierte eigene Scope-JSON.
Remotevorbereitung `40332971455` hat vor jeder Kompilierung am Kandidaten
`34f3361de13104e386eba26bf16d6a0160475481` diese formatierten Größen gemeldet:
`codec_registry.rs` 507 Zeilen, `frame_size_caps.rs` 632 Zeilen und
`wire_control.rs` 523 Zeilen. Der eingesehene kandidatengebundene
`format.json` bestätigt die Angaben. Die Formatpatches wurden laut Auftrag
mit `10988167` übernommen; Main hat danach weitere getrennte Strukturarbeit
integriert. Dieser Autor bearbeitet ausschließlich seine freigegebene Fläche.

Beauftragtes Ergebnis: Die fachlichen Verantwortungen dieser drei Module und
ihrer notwendigen Auszüge liegen bei üblicher Rustformatierung unter 500 Zeilen
und 50 KiB. Codecwahl, Fallbackreihenfolge, Bild-/Paket-/AVC-Grenzen,
Control-Wirebytes, Rechte, bestehende Namen und G6-Qualitätsreferenz bleiben
erhalten. Keine Zeilenverdichtung, Gateausnahme oder Änderung eingefrorener
Releaseblöcke ist autorisiert. Git, Graph, Manifest und dieselbe spätere
Remote-Suite bleiben bei Main; lokaler Compiler, Formatter, Tests, Server und
Unteragenten bleiben verboten.

Promptwissen und Beleg werden getrennt gehalten: Die drei konkreten Größen
sind aus dem Remoteformatartefakt nachgelesen. Welche Funktionen gemeinsam
auszuziehen sind und welche Modulpfade die unveränderte Aufruferfläche bewahren,
wird als nächster Schritt aus dem aktuellen Quelltext ermittelt. Es fehlt
keine zwingende Nutzerangabe. Planungstiefe 2 bleibt passend, weil bestehende
Codec-, Geometrie-, Paket- und Controlverträge zusammenhängen, ohne dass eine
neue Produktfunktion entsteht.

### B — Bestandsaufnahme und Funktionsverknüpfungen

Die konkret beanstandete Funktion ist die fachliche Quellorganisation der drei
bereits formatierten Dateien. Ein falsches Codec-, Bild- oder Protokollergebnis
wird aus dem Größenbefund nicht abgeleitet. Alle folgenden Funktionen sind im
aktuellen Quellstand gelesen und werden im selben Autorzyklus erhalten:

| Verantwortung und eigene Funktionsauflistung | Direkte und indirekte Verknüpfungen bis zur betroffenen Grenze |
|---|---|
| `CodecRegistry` hält Fabriken, erzeugt Angebote und Reports; seine sieben Öffnungsmethoden verbinden Stage-/Admissionprüfung, Auswahl, Fallback und Decoderreferenz. | OS-`codec_inventory` → `CodecRegistry::new`/Angebote → `negotiate` → Hostpipeline/Worker und G6-Fixture → `open_encoder_for_stage` bzw. `open_encoder_for_stage_admitted` → geprüfte Fabrik, Tiles als Rückfall, `host_hybrid::{wrap,wrap_captured}`; `open_decoder` → `open_decoder_raw` → `HybridDecoder` und `ReferenceDecoder`. Hardwarepräferenz, verweigerte Fabriknamen, Reihenfolge, Größen-/Vertragsprüfung, Grantklone und Fehlertexte bleiben identisch. Der Bestand und `CodecReport` bleiben im bisherigen Modul. |
| `FrameSize`/`Rect` halten Geometrie; `PacketBudget` samt `RECORD_BUFFER_OVERHEAD` und `validate_encoded_budget` begrenzt Paket- und Pufferbesitz. | `frame_budget`/`session_admission` → Budgetberechnung → `capture_transport`, `host_codec_phase_plan`, `host_capture`, `host_hybrid`/`captured_hybrid`, `wire_binary`, `gop_stream`, `worker_wire`/`worker_frame_wire` sowie OpenH264-/MF-Captureausgabe und Android-Pullpfad. V1-Ganzbildrecords, lokale V1-Empfangsgrenze und V2-20-Bild-/32-Recordbilanz behalten ihre Formeln; die Produzentenprüfung bleibt vor der Veröffentlichung erreichbar. |
| `AvcLimits`, `avc_limits`, `macroblocks`, `fit_macroblocks`, `check_avc_dimensions` und der private `AvcBits` prüfen Codecgröße und SPS. | `negotiate`/Viewer-/Hostgrößenwahl → Makroblockgrenze; OpenH264-Decoder und MF-Encoder/-Decoder/-Captureausgabe → SPS-Prüfung vor nativer Bildallokation → unveränderte Profil-, Level-, 8-Bit-4:2:0-, Interlace-, Crop-, Padding-, Referenz- und Exp-Golomb-Grenzen. `FrameSize` bleibt der gemeinsame Typ; sein ursprünglicher Importpfad bleibt erhalten. |
| `ViewerFrame`/`HostFrame`, ihre Domainmodelle und `FrameAck` halten die Nachrichtensemantik; vier Encoder-/Decoderfunktionen ordnen Tags und Bytes zu. | Desktop-/Android-/mobile Sitzung → `viewer_client` bzw. `host_session`/`icon_host` → unveränderte Send-/Empfangswrapper → vier Codec-Funktionen → `wire`/Eingabecodec/Familien-/Icon-/Sichtbarkeitsmodelle. Die Rückrichtung führt durch dieselben Decoder und bisherigen Kontroll-/Präsentationszustände. Taglimit, Richtungstrennung, 24-Byte-ACK, 16-Byte-Ping, 32-Byte-Pong, Big Endian, JSON-Felder, Familienvalidierung und neutraler Iconfehler bleiben erhalten. |
| Bestehende Abnahme und unveränderte G6-Referenz | `batches_frame_budget_task_tests`, `wire_tests`, `negotiate_tests`, `wire_binary_tests`, die zugelassenen OpenH264-/MF-Fixtures und die eigenen G6-Fälle benutzen die bisherigen Namen. `review-task-manifest.json` wählt bereits `batches_` und die benannten Wire-/Negotiationfälle. Main bestätigt zusätzlich die unveränderten eingefrorenen Releaseblöcke nach `10988167`; diese liegen außerhalb der aktuellen Schreibfläche. |

Der Rootgraph wurde nur zur Navigation abgefragt. Die Symbol-/Dateibeziehungen
oben sind gegen aktuellen Quelltext verfolgt. Der bereits dokumentierte
Graphify-Skill-/Paketunterschied liefert keinen neuen Laufzeitbeleg. Die
gleichzeitige Strukturarbeit an Hostsession, Androidadaptern und Desktopdecode
bleibt außerhalb dieser Schreibfläche; sie ruft die unveränderten bisherigen
Namen weiterhin auf.

### C — Strukturentscheidung und zweite Lückenrecherche

Gewählt sind private Kindmodule mit expliziten Imports und gezielten
`pub(crate)`-Reexports am bisherigen API-Ort. Dadurch bleibt `CodecRegistry`
einschließlich seiner privaten Fabrikfelder am selben Ort; seine Methoden
werden als unveränderte inhärente Implementierung ausgelagert. Für Paketbudget
und AVC bleiben die bisherigen `frame_size_caps`-Namen als Reexports bestehen.
Beim Controlprotokoll bleiben sämtliche Datentypen, Serdeattribute,
`FrameAck`-Methoden, Integerhelfer und die async Send-/Empfangswrapper am
bisherigen Ort. Nur die vier vollständigen Tagged-Codec-Funktionen werden
ausgelagert. Damit wird auch keine bislang private Hilfsmethode enger sichtbar.

Verworfen: bloßes Verdichten verletzt den ausdrücklichen Auftrag; `include!`
würde die tatsächliche Modulverantwortung verbergen; ein Verschieben sämtlicher
öffentlichen Nachrichtentypen würde unnötig mehr Typ- und Importgrenzen ändern.
Eine neue Abhängigkeit, ein neues Protokoll oder ein zusätzlicher Testlauf ist
für diesen reinen Strukturschnitt nicht erforderlich.

Erste Recherche, geprüft 2026-10-26: Die Rust Reference dokumentiert
[Zugriff aus Kindmodulen](https://doc.rust-lang.org/reference/visibility-and-privacy.html),
[relative `#[path]`-Moduldateien](https://doc.rust-lang.org/reference/items/modules.html)
und [gezielte Reexports](https://doc.rust-lang.org/reference/items/use-declarations.html).
Diese Mechanismen erlauben den fachlichen Schnitt ohne öffentliche Kindmodule
oder zusätzliche Feldsichtbarkeit. Es handelt sich um Grundmechanismen der im
Projekt vorhandenen Rustmodule; es wird keine neue Bibliotheks-API eingeführt.

Zweite Lückenrecherche vor der Produktionsänderung: `super::` verweist nach
Auszug in ein Kindmodul auf einen zusätzlichen Vorfahren. Alle betroffenen
Verweise auf Screenkernmodule müssen deshalb ausschließlich um diese eine
Ebene korrigiert werden. Die privaten Fabrikfelder und `offers_stream` bleiben
für das Kind erreichbar. `open_decoder_raw` wird zusammen mit seinem Aufrufer
verschoben. `FrameAck::{encode,decode}`, `SetMode::validate` und die Integerhelfer
bleiben bewusst im Elternmodul und sind aus dem Codec-Kind zugänglich. Alle
Dateipfade werden als direkt aus der jeweiligen Elternquelldatei relative
`#[path]`-Attribute registriert; `core/mod.rs` braucht keine neue Freigabe.

Der geplante unveränderte Körper aller verschobenen Funktionen, Konstanten und
Typdefinitionen lässt sich anschließend gegen `10988167` als Text vergleichen,
wobei nur die notwendige zusätzliche `super::`-Ebene rücknormalisiert wird.
Das ist ein statischer Erhaltungsbeleg und ersetzt keine Typ-/Compilerprüfung.

### D — Umsetzung und eigene anschließende Anti-Regressionsreview

| Meilenstein | Konkrete Dateien und Abhängigkeit | Erwarteter Erhaltungsbeleg vor Mains Remoteausführung |
|---|---|---|
| S1 Codecöffnung trennen | `codec_registry.rs` → `codec_registry_open.rs`; private Kindimplementierung | Alle sieben Öffnungsmethoden besitzen dieselben Signaturen, Prüfungen, Fabrikreihenfolgen, Fallbacks, Wrapper und Fehlertexte; Konstruktion, Angebote und Report bleiben erreichbar. Beide Dateien bleiben normal formatiert unter der Größenregel. |
| S2 Paketbudget und AVC trennen | `frame_size_caps.rs` → `frame_size_packet.rs`, `frame_size_avc.rs`; gezielte Reexports | Die bisherigen öffentlichen Namen und Geometrie bleiben erreichbar; Formeln, Tabellen, SPS-Parser, Rejectionpfade und Konstanten sind abgesehen von Modulpfaden unverändert. |
| S3 Controlcodec trennen | `wire_control.rs` → `wire_control_codec.rs`; vier Funktionen reexportieren | Domainmodelle, private ACK-/Integerhelfer und async Wrapper bleiben identisch; jeder Matcharm, Tag, feste Bytefolge und JSON-/Validierungspfad bleibt identisch. |
| S4 Eigene Schlussreview und Übergabe | Gesamte oben aufgeführte Funktionskette und dieser Bericht | Statischer Funktionskörper-/Deklarationsvergleich, Import-/Sichtbarkeitslektüre, Syntaxparse, Diff-/Größenprüfung und unveränderte vorhandene Fallnamen; keine lokale Test- oder Formatierausführung. Main nutzt dieselbe Remote-Suite, ohne neuen Build-/Releasezyklus. |

Die Grundlagen und der konkrete Strukturplan standen vor der Produktionsänderung.
S1–S3 sind umgesetzt. Die sieben beteiligten Dateien behalten lesbare bisherige
Funktionsblöcke und besitzen nach den notwendigen expliziten Zeilenumbrüchen
keine Zeile mit mehr als 100 Zeichen:

| Datei unter `native/src/screen/core/` | Zeilen | Bytes | Verantwortung |
|---|---:|---:|---|
| `codec_registry.rs` | 224 | 7.587 | Fabriktraits, Bestand, Angebote, Diagnosebericht |
| `codec_registry_open.rs` | 298 | 11.591 | Sieben Öffnungsmethoden, Auswahl, Fallback, Admission und Referenzwrapper |
| `frame_size_caps.rs` | 127 | 4.428 | Geometrie und unveränderte öffentliche Namen über gezielte Reexports |
| `frame_size_packet.rs` | 244 | 10.533 | V1-/V2-Paketbudget, Produktpufferbilanz und Produzentenprüfung |
| `frame_size_avc.rs` | 284 | 9.622 | Makroblock-/Levelgrenzen und bounded SPS-Parser |
| `wire_control.rs` | 330 | 9.885 | Domainmodelle, private ACK-/Integerhelfer und async IO |
| `wire_control_codec.rs` | 218 | 9.010 | Vier Tagged-Encoder/-Decoderfunktionen |

Es wurde kein Formatter aufgerufen. Die ursprünglichen formatierten Blöcke
wurden übernommen; ausschließlich längere durch Modulpfade entstandene Zeilen
wurden explizit lesbar umgebrochen. Die tatsächliche Remoteformatbestätigung
bleibt Bestandteil derselben von Main verwalteten Vorbereitung.

#### Eigene anschließende Anti-Regressionsreview der Strukturkorrektur

Nach Abschluss der Auslagerungen wurden alle früheren Funktionen, Typen,
Traits und Konstanten der drei Originaldateien gegen `10988167` verglichen.
Der lesende Tree-sitter-Abgleich erfasst dabei die vollständigen Deklarationen
und Funktionskörper mit ihren Literalinhalten. Nach Rücknormalisierung genau
der durch den Kindmodulpfad zusätzlich erforderlichen `super::`-Ebene sind alle
Quelltokens identisch; lediglich Layoutwhitespace wird dabei ignoriert. Kein
bisheriges Item fehlt und kein bisheriges Item ist doppelt vorhanden. Der
zusätzliche Syntaxparse der tatsächlichen sieben Dateien meldet keine ERROR-
oder missing-Knoten. Das ist kein Compiler- oder Typprüfungsbeleg.

| Betroffene Kette | Eigener statischer Anti-Regressionsbefund |
|---|---|
| Bestand → Auswahl → Fabrik → Hybrid-/Referenzwrapper | `CodecRegistry`, Fabrikfelder, Fabriktraits und Report bleiben im Elternmodul. Das private Kind darf nach Rustprivacy auf diese Felder und `offers_stream` zugreifen. Alle Methodensichtbarkeiten und Signaturen bleiben erhalten. Prüfung vor Allokation, Hardwarepriorität, verweigerte Fabriken, Fehlerreihenfolge, Tilespfad, Grantweitergabe und Reihenfolge raw → hybrid → reference sind tokenidentisch. `open_decoder_raw` bleibt mit seinem einzigen Öffnungsaufrufer im selben privaten Kind. |
| Geometrie → V1/V2-Admission → Record-/Worker-/Pullgrenze | `FrameSize` und `Rect` bleiben am ursprünglichen Ort. `PacketBudget`, sein Overhead und `validate_encoded_budget` werden dort mit identischer `pub(crate)`-Reichweite reexportiert. Bild-/Recordanzahl, checked/saturating Arithmetic, V1-Ganzbildrecord, lokale Legacygrenze, V2-Bilanz sowie Produzenten-Rejection besitzen dieselben Tokens und Fehlertexte. Kein Budget wird neu berechnet, angehoben oder fallengelassen. |
| Codecangebot → Makroblockfit → SPS vor Allokation | Die bisherige Außenfläche `AvcLimits`, `avc_limits`, `macroblocks`, `fit_macroblocks`, `check_avc_dimensions` bleibt unverändert reexportiert. Tabellen, progressive 8-Bit-4:2:0-Grenze, Profil-/Level-/Crop-/Paddingprüfung und bounded Exp-Golomb-Lesen sind unverändert. Der private `AvcBits` bleibt ausschließlich innerhalb seines mitverschobenen Parsers erforderlich. |
| Controlmodell → Tags/Bytes → Async IO → Host/Viewer | Sämtliche Nachrichtentypen, Serdeattribute, ACK-/Ping-/Pongfelder, privaten `FrameAck`-Methoden, Integerhelfer und async Wrapper bleiben im bisherigen Elternmodul. Die vier reexportierten Codec-Funktionen dürfen auf die privaten Elternhelfer zugreifen. Ihre Matcharme, Tags, Literale, Big-Endian-Reihenfolge, Tag-/Richtungsgrenzen, Familien-/Iconvalidierung und Fehlerpfade sind tokenidentisch. Berechtigungs- und Präsentationszustände erhalten dieselben Nachrichten. |
| Bisherige Fallnamen und G6-Referenz | Es wurde kein Test oder Testmodul verschoben oder umbenannt. Die bestehenden Importpfade bleiben durch inhärente Methoden beziehungsweise gezielte Reexports bestehen. Die eigenen initialen Qualitätsquellen, eingefrorenen Releaseblöcke und beide Cargo-Dateien besitzen durch diesen Folgeauftrag keinen Worktree-Diff. Mains zusätzlicher Byte-/Paketversionsbeleg wird dadurch nicht verändert. |
| Integrationsgrenze | Alle neuen Module bleiben privat und plattformneutral; es gibt keine neue OS-Abhängigkeit, keine öffentliche Fabrikfeldfreigabe, kein `cfg`-Gate, keine neue Ausnahme und keine neue Laufzeitaktion. Die funktionale Bestätigung erfolgt ausschließlich durch Mains dieselbe Remote-Suite. |

Zusätzlich wurde `git diff --check` für die geänderten vorhandenen Dateien und
den Bericht ohne Befund ausgeführt. Die Prüfung des nachfolgenden neuen
Dateiinhalts erfolgte ebenfalls als statische Syntax-/Quelllektüre. Keine
Build-, Test-, Formatter-, Server-, Commit-, Push- oder Graphaktion wurde von
diesem Autor ausgeführt. Der ursprüngliche Vorbereitungsfehlschlag fand laut
Remoteauftrag vor jeder Kompilierung statt; weder dieser Befund noch der
statische Abgleich wird als erfolgreicher Remote-Test ausgegeben.

#### Dateien und bestehende Suiteanbindung dieser Fortsetzung

Neu erstellt:

- `native/src/screen/core/codec_registry_open.rs`
- `native/src/screen/core/frame_size_packet.rs`
- `native/src/screen/core/frame_size_avc.rs`
- `native/src/screen/core/wire_control_codec.rs`

Geändert:

- `native/src/screen/core/codec_registry.rs`
- `native/src/screen/core/frame_size_caps.rs`
- `native/src/screen/core/wire_control.rs`
- `docs/plaene/2026-10-24-batches/fix-quality-acceptance.md`

Für diese Fortsetzung gezielt gelesen: die aktualisierte eigene Scope-JSON,
der eigene bisherige Bericht, `/tmp/smartexplorer-rv1-40332971455/format.json`
und die Struktur von `/tmp/smartexplorer-rv1-size-inventory.json`; aktuelle
`codec_registry.rs`, `frame_size_caps.rs`, `wire_control.rs`, `screen/mod.rs`
und ihre originalen Fassungen aus `10988167`; die neuen vier Auszüge;
`core/mod.rs`, `negotiate.rs`, `frame_budget.rs`, `wire_binary.rs`,
`gop_stream.rs`, `worker_wire.rs`, `worker_capture_wire.rs`,
`host_stage.rs`, `host_pipeline_init.rs`, `host_capture.rs` und die bereits
bekannten OpenH264-/Qualitätsquellen; die konkreten Aufrufstellen in
`session_admission.rs`, `capture_transport.rs`, `worker_frame_wire.rs`,
`host_codec_phase_plan.rs`, `host_hybrid.rs`, `captured_hybrid.rs`,
`viewer_client.rs`, `host_session.rs`, `icon_host.rs`,
`batches_rate_reference_transport.rs`, den OS-Codec-/Capture-/Viewerschnittstellen
und den freigegebenen mobilen/JNI-Grenzen; `wire_tests.rs`, `negotiate_tests.rs`,
`wire_binary_tests.rs`, `batches_frame_budget_task_tests.rs`,
`batches_session_budget_compatibility_task_tests.rs`,
`window_list_tests.rs` und `native/review-task-manifest.json`.
Die Recherche blieb bei den zugewiesenen Modulen und ihren konkreten
Aufruf-/API-Grenzen; ein Suchtreffer allein wurde nicht als Laufzeitbeleg benutzt.

Es werden keine neuen Fälle oder neuen Artefaktlabels benötigt. Die bisherige
gemeinsame Suite enthält bereits die unmittelbar betroffenen Fälle:

- `screen_task_negotiate_matrix`
- `screen_task_wire_roundtrip_all_tags`
- `screen_task_wire_picture_stream_roundtrip`
- `screen_task_frame_limits_reject_oversize`
- `batches_task_b6_producer_rejects_record_count_and_payload_over_budget`
- `batches_task_b6_record_reader_limits_whole_transaction`
- `batches_task_b6_worker_rejects_negotiated_packet_before_payload_read`
- die im vorigen Abschnitt vollständig aufgelisteten sechs G6-Qualitätsfälle.

Main muss für diesen Strukturschnitt keinen zusätzlichen Testlauf, Build,
Testnamen oder Releasezyklus registrieren. Main übernimmt die freigegebenen
Dateien in den Kandidaten, aktualisiert den Rootgraphen und führt die bestehende
Remotevorbereitung/-Suite fort. Innerhalb dieser Schreibfläche bleibt keine
bekannte offene Strukturabhängigkeit. Die finale Format-, Typ- und
Laufzeitbestätigung bleibt ausschließlich remote offen; dieser Autor stoppt
nach seinem vollständigen Quellergebnis und eigenen Review.

Nach Mains Stagingkontrolle wurde am 2026-10-26 ausschließlich je eine
zusätzliche EOF-Leerzeile in `frame_size_packet.rs` und `wire_control_codec.rs`
entfernt. Der direkte Bytevergleich bestätigt unveränderten Inhalt vor diesem
jeweils letzten LF-Byte; Funktions- und Referenzcode bleibt unverändert. Die
Größentabelle ist auf 244/218 Zeilen und 10.533/9.010 Bytes angepasst. Das erneute
Staging und dessen Indexprüfung bleiben bei Main.
