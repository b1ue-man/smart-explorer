# R9-Nachplan: Aufnahmezulassung vor nativen Bildallokationen

Stand: 2026-10-26. Dieser Auftrag liefert ausschließlich einen Detailplan. Es gibt keine Produktänderung und keinen Laufzeitnachweis.

## Ziel und Erstplan

Auftragsquelle ist der delegierte Ergebnisauftrag `capture_admission_plan` mit dem verbindlichen Erstplan in `docs/plaene/2026-10-24-batches/umsetzung.md`, Abschnitt „R9-Erstplan: native Aufnahme vor der Verkleinerung“. Ziel ist eine tatsächlich gehaltene, gemeinsam gezählte Speicherzusage vor Öffnung oder Produktion einer nativen Aufnahme. Die Zusage muss Größenwechsel, Quellcache, Ergebnislebensdauer, Abbruch und getrennte Workerprozesse abdecken.

Die Accepted-Zusage und der bestehende v2-Vertrag mit 24 Bildbesitzern bleiben erhalten. Kleine Verträge dürfen größere zugelassene native Quellen weiterhin verkleinern. Native 4K, Legacy, Snapshot, Preview/Full, Fensterfamilien, Berechtigungen, Isolation und Abbruch bleiben erhalten. RAW512 ist eine Allokationsgrenze; sie ist keine Zusage.

Erstplan aus dem Auftrag: Die erlaubten Aufnahmepfade bis zu ihrem letzten Bildbesitzer verfolgen; die Pufferzahl und die belegbare Größe aus dem Bestand und den Primärquellen bestimmen; eine gemeinsame Zulassung einschließlich Prozessgrenze entwerfen; verbleibende API-Fragen in einer zweiten Recherche prüfen; den konkreten Änderungs- und Abnahmeplan festhalten. Die Schreibfläche ist ausschließlich diese Datei. Lokale Builds, Tests und Produktänderungen sind ausgeschlossen.

Im Erstplan waren die tatsächlichen Pufferzahlen von WGC, DXGI, X11, PipeWire, Android, synthetischer Quelle und Familienkomposition, die IPC-Zusage vor OPEN und die letzte Eigentümergrenze nach Abbruch noch durch Lektüre zu klären. Die untenstehenden Befunde und Entscheidungen beantworten diese Planfragen. Es fehlt keine Entscheidung des Nutzers.

## Belegter Bestand und Anspruchsgrenze

Die folgende Bestandsaufnahme bezieht sich auf die gelesenen Arbeitsdateien vom 2026-10-26, einschließlich der bereits vorhandenen R7-/R9-Korrekturen. Sie ist kein Laufzeitnachweis.

1. `core/host_server.rs` öffnet die Quelle über `family_listing::open`, bevor es `frame_memory::reserve` aufruft. `os/shared/host_worker_main.rs::Worker::admit` bindet erst CAPTURE/PRODUCE an das Accepted-Budget. OPEN und OPEN_FAMILY können vorher bereits native Ressourcen erzeugen. Die `OnceLock`-Pools in Daemon und Worker haben getrennte Adressräume und zählen ihre Zusagen nicht gemeinsam.
2. `core/host_backend.rs::ScreenSource::capture_snapshot` ruft zuerst `capture` auf und verkleinert danach. `core/host_capture.rs` und der Worker können nochmals skalieren. Ein enger Übertragungsvertrag deckt daher nicht automatisch das größere Quellbild. `FrameLimits::RESOURCE`, RAW512, `capture_limits` und das Familien-Pixelsummenlimit prüfen Zahlen; sie halten keine Speicherzusage.
3. `core/frame_memory.rs` hält Zusagen bis zum letzten `MemoryLease`-Besitzer. Mit OS-Fakten beträgt die gemeinsame Kapazität ein Viertel des Minimums aus beobachtetem freiem physischem Speicher, Commit-Spielraum und virtuellem Adressraum. Ohne Fakten wird die Kohortenkapazität heute beim ersten Besitzer auf `min(512 MiB, requested)` festgelegt. Insbesondere Android hat `facts=None`; sein separat gemeldeter freier Javaheap ist keine Erlaubnis für eine zusätzliche große native Klasse.
4. `StageOutput` hält die bisherige Zusage nach Abbruch einer blockierenden Arbeit bis nach dem Ergebnis. Diese Richtung ist richtig. Ein nacktes `BgraImage`, ein Cacheklon, ein ausstehender Portalcallback oder ein JNI-Queueelement besitzt jedoch noch keine eigene Capturezusage. `BgraImage::clone` kopiert Pixel.
5. WGC startet seine zweiflächige Framepool-Produktion beim Öffnen. PipeWire kann nach dem Portalzustand `Streaming` produzieren, bevor eine CAPTURE-Anfrage läuft. Android öffnet Reader/Encoder und kann während einer Konfigurationsänderung die alte und neue Generation gleichzeitig halten. Das spätere Prüfen eines Frames schließt diese Öffnungs- und Hintergrundpfade nicht.

**Geltungsbereich der Zusage:** gerechnet werden lebende, vom Produkt angelegte Bytebereiche und die Nutzpixel der ausdrücklich angeforderten, bekannten Captureflächen. Die Zusage ist eine interne Ressourcenentscheidung, keine physische Reservierung gegenüber fremden Prozessen, späterem Speicherdruck oder dem Betriebssystem. Unbekannte Grafiktreiber-, Compositor-, Codec-, Kernel-, Allocator- und GC-Kosten bleiben in der bereits bestehenden Außenreserve von drei Vierteln; sie werden weder als null behauptet noch durch eine erfundene Pitchkonstante als abgedeckt ausgegeben. Bekannte produktseitige Codec-Eingabe-/Ausgabekopien gehören dagegen zur Besitzerrechnung.

Für eine D3D-Textur ist vor `CreateTexture2D` ihre angeforderte Nutzpixelgröße bekannt, nicht ihr späterer physischer Treiberbedarf. `RowPitch` darf Padding enthalten; Microsoft veröffentlicht dort keine universelle obere Pitchkonstante. Der Plan reserviert die angeforderte Fläche und prüft die tatsächlich gelieferte Zeilenspanne vor jedem Zugriff. Er behauptet damit keine vollständige GPU-Bytebilanz. [D3D11_MAPPED_SUBRESOURCE](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ns-d3d11-d3d11_mapped_subresource).

## Lösungsentscheidung

Untersuchte Möglichkeiten und Entscheidung:

| Möglichkeit | Befund und Entscheidung |
|---|---|
| RAW512 vor dem Capture prüfen | Verhindert übergroße Einzelanforderungen, zählt aber weder parallele Sitzungen noch Lebensdauer. Als alleinige Lösung verworfen. |
| Ein weiterer globaler Pool pro Worker | Die Prozesse besitzen unabhängige Zähler; ihre Summe kann die Daemonzusage überschreiten. Verworfen. |
| Erst nach `capture` reservieren | WGC, PipeWire, JNI und Quellcache haben dann bereits allokiert. Verworfen. |
| Immer das volle W plus ein volles separates Capturepaket buchen | Zählt bereits finanzierte Besitzer doppelt und würde den gesamten Legacy-Pfad bei `facts=None` unnötig sperren. Verworfen. |
| Capture in den Daemon verschieben | Würde die vorhandene Prozessisolation verändern. Für R9 nicht erforderlich und verworfen. |
| Eine maßlose Worst-Case-RAW-Reserve für jede Quelle | Verdrängt kleine Verträge ohne Bezug zur gewählten Quelle und löst die Besitzerübergänge nicht. Verworfen. |
| Eine gemeinsame Autorität mit unterteilten Sitzungszusagen und delegierten Captureguthaben | Gewählt: vorhandenes W behalten, eindeutige Besitzer einmal zählen, nur ungedeckten Bedarf zusätzlich zusagen, bevor der betroffene Adapter allokiert. |

Der Daemon besitzt für seine Sitzungen und die von ihm gestarteten Captureworker **eine** Buchungsautorität. Ein Worker erhält ein an Verbindung, Sitzung, Quelle und Generation gebundenes Guthaben; er liest keine eigene verfügbare Kapazität aus und bucht W nicht nochmals in einem lokalen Pool. Im Androidprozess wird dieselbe Autorität direkt über JNI erreicht. Für die eigenständige privilegierte Loginbroker-/Helpergruppe besitzt entsprechend der Broker eine gemeinsame Autorität für ihre Capturehelper. Innerhalb einer dieser Gruppen ist eine zusätzliche unabhängige Workerautorität ausgeschlossen. Ein systemweiter Pool über den unabhängig laufenden Screen-Daemon und Loginservice ist ausdrücklich nicht Bestandteil dieses Auftrags; ebenso wenig eine Gesamt-RSS-Garantie. Diese Abgrenzung wurde vom Hauptauftrag am 2026-10-26 bestätigt.

### Typisierte Grenzen

Die nachstehenden Namen sind vorgeschlagene neue interne APIs, kein vorhandener Code:

- `SessionAdmission`: hält das unveränderte Accepted-W, den benannten Besitzerplan und gegebenenfalls den zusätzlich zugelassenen Betrag. Lebensdauer umfasst Pipeline, Worker, ausstehende lokale Ergebnisse und verzögerte Aufräumarbeit.
- `CaptureAuthority`: `reserve_exact(requirement, generation)` und `reserve_transition(old, new)` liefern eine vollständig gedeckte Zusage oder einen Ressourcenfehler. Anders als `MemoryPool::reserve` darf der Allokationspfad keinen kleineren Teilbetrag als Erfolg erhalten.
- `CaptureRequirement`: enthält die Quellenidentität, native Geometrie/Format, einzelne Besitzerklassen, Anzahl, Bytegröße beziehungsweise angeforderte Flächengröße, dauerhafte/transiente Phase und Generation. Checked-Arithmetik geht jeder Multiplikation und jeder Allokation voraus. Eine Zahl ohne Besitzerklasse ist kein freies Guthaben.
- `CaptureLease`/`CaptureSlice`: die Zusage und ihre disjunkten Teilbereiche. Unterteilen erzeugt keine zweite Kapazität. Ein Teilbereich darf nach tatsächlicher Freigabe erneut verwendet werden; ein Alias erzeugt keine zweite Pixelbuchung.
- `CapturedImage`: unveränderliche gemeinsame Pixelablage mit `Arc` und der dazugehörigen `CaptureSlice`. Cache, Familienbild und zurückgegebenes Bild können dieselbe Ablage teilen. Neue Pixelkopien benötigen eine neue Teilzusage; ein `Clone` dieses Typs kopiert nur den Besitzer. Die allgemeine Viewer-Bild-/Renderer-API wird nicht nebenbei ersetzt.
- `PreparedSource`: identifiziert Quelle und zulässige Berechtigung, ermittelt Metadaten und Anforderungen, produziert aber keine Pixel. `activate(grant)` ist die einzige Grenze zu WGC-Pool, DXGI-Duplikation, X11-Bildanforderung, PipeWire-Bufferverhandlung, Android-Reader/Still-Anforderung und synthetischem Pixelbild.

Wenn eine Plattform die endgültige Geometrie erst asynchron meldet, bleibt die Quelle `Prepared` beziehungsweise `AwaitingGrant`, bis die zuständige Anforderung finanziert ist. Sichtbare Quellwahl, Portalzustimmung und Identitätsprüfung sind weiterhin möglich. Ein CAPTURE-Aufruf ersetzt diese Öffnungszulassung nicht.

Im Rustbesitzer stehen Ressourcen und Pixelablage vor dem letzten Leasefeld oder werden ausdrücklich vor der Lease freigegeben. Eine `Drop`-Methode allein ist noch kein Beweis: Felder werden nach ihrer Deklarationsreihenfolge zerstört. [Rust Reference: Destructors](https://doc.rust-lang.org/reference/destructors.html).

### Verbindlicher Kernvertrag für getrennte Adapterarbeiten

Der erste getrennt implementierbare Teil liefert ausschließlich den plattformfreien Besitzerkern und seinen Host-Trait-Anschluss. Er legt die folgenden `pub(crate)`-Typen und Methoden fest, damit Desktop-, Android-, Login- und Workeradapter anschließend keine konkurrierenden Kerntypen einführen. Die Signaturen sind Entwurfsvertrag; Fehlertexte und interne Container sind Implementierungsdetails. Alle Bytezahlen werden als checked `u64` geführt und erst an einer konkreten Allokationsgrenze nach `usize` konvertiert.

```rust
// core/capture_admission.rs: keine OS-Erkennung, keine Pipes, keine JNI-/COM-Typen.
pub(crate) struct SessionAdmission { /* unverändertes W, Zusatz X, Besitzerbuch */ }
pub(crate) struct SourceKey { /* Sitzung + vorbereitete Quelle; nicht nur PID */ }
pub(crate) struct CaptureGeneration(u64);
pub(crate) struct CapturePlan { /* geprüfte lokale Phasen-/Besitzermenge */ }
pub(crate) struct CaptureGrant { /* Quelle + Generation + zugelassene Teilbereiche */ }
pub(crate) struct CaptureSlice { /* eindeutiger Bereich; kein Clone, keine Serde */ }
pub(crate) enum CaptureOwner {
    NativeSurface, Readback, SourcePixels, CompositionPixels,
    ScaledPixels, FilterScratch, Metadata, Transport, CodecInput, CodecOutput,
}

pub(crate) trait CaptureAuthority: Send + Sync {
    fn approve(&self, plan: CapturePlan) -> std::io::Result<CaptureGrant>;
}
impl SessionAdmission {
    fn session(&self) -> &str;
    fn working_bytes(&self) -> u64; // W, niemals W + X
    fn bind_contract(&self, contract: Option<FrameContract>) -> std::io::Result<()>;
    fn authority(&self) -> std::sync::Arc<dyn CaptureAuthority>;
}
impl CaptureGrant {
    fn source(&self) -> &SourceKey;
    fn generation(&self) -> CaptureGeneration;
    fn claim(&self, owner: CaptureOwner, bytes: u64) -> std::io::Result<CaptureSlice>;
}
impl CaptureSlice {
    fn bytes(&self) -> u64;
    fn split(self, first_bytes: u64) -> std::io::Result<(CaptureSlice, CaptureSlice)>;
}
```

`frame_memory::MemoryPool` erzeugt `SessionAdmission` aus den bereits von außen gelieferten Fakten, einem **expliziten** Plattform-/Kohortenhöchstwert und dem angeforderten W. Die vorhandene v2-Teilvergabe darf weiterhin ein kleineres verhandelbares W ergeben; Legacy verlangt weiterhin seinen bestehenden Rahmen. `approve` ist dagegen exakt: Erfolg finanziert den vollständigen Plan, sonst gibt es keinen Grant. Die Autorität berücksichtigt alle noch lebenden Pläne/Slices derselben Gruppe. Sie ermittelt X aus der Host-Phasensumme, nicht aus einer weiteren unabhängigen Kapazität. Eine Erhöhung des zusätzlichen X ist vor der neuen Allokation abgeschlossen; die Freigabe eines alten Zusatzanteils ist erst nach dessen letztem Besitzer möglich. `bind_contract(None)` bindet ausdrücklich Legacy und ist von „noch ungebunden“ unterschieden. Zweite abweichende Bindung scheitert.

Ein `CapturePlan` wird über checked Konstruktoren aus `SourceKey`, Generation und typisierten Besitzeranforderungen aufgebaut. Die API akzeptiert keine freie zweite Budgetzahl. Jede Anforderung benennt Objektidentität, `CaptureOwner`, Bytekapazität, Stückzahl und mögliche gleichzeitige Phase. Alias-Identitäten entstehen ausschließlich durch tatsächliches Teilen eines vorhandenen Besitzers; der Worker kann sie nicht durch frei gewählte IDs vortäuschen. Native Adapter melden Metadaten, der portable Planer berechnet die Beträge. Familienpläne enthalten die geprüfte Summe ihrer Mitgliedspläne und des Canvas; die Wireform überträgt begrenzte aggregierte Angaben und gebundene Mitgliedsidentitäten über den bestehenden Familienkanal.

`CaptureGrant::claim` ist ein schneller lokaler Teilbereichserwerb ohne OS-Abfrage und ohne IPC. Er prüft atomar **sowohl die konkrete Ownerkapazität als auch die Summe sämtlicher gleichzeitig lebender Slices** gegen den zugelassenen Gesamtbereich. Phasennamen sind keine voneinander unabhängigen Geldtöpfe. Wenn zwei im Plan als disjunkt angenommene Phasen doch gleichzeitig Besitzer halten, zählen beide realen Bestände; die zweite beanspruchte Slice muss noch in die Gesamtzusage passen oder scheitert vor Allokation. Die Phasenrechnung wird außerdem als überprüfbare Gleichzeitigkeitstabelle im Plan gespeichert. Sie ersetzt diesen Summenzähler nicht. `split` erhält exakt die bisherige Summenbuchung, Aliasclone vermehrt nur Referenzen, finaler Drop vermindert sie einmal.

Der lokale Erwerb kann deshalb vor einer Callbackkopie eingesetzt werden. Das Beschaffen oder Erweitern eines Grants über `approve` liegt ausschließlich im vorbereitenden/control Pfad; der entfernte Workeradapter implementiert `CaptureAuthority` über den beschriebenen Dispatcher. Ein serialisierter Grantdeskriptor wird nur vom verifizierten Parentkanal angenommen. Ein Worker-Creditpool ist damit eine begrenzte Unterteilung eines bereits im Parent gezählten Grants, kein eigener `MemoryPool` mit neu beobachteter Kapazität. Parentantwort-/Workeranteile erhalten disjunkte Limits aus demselben Grant; ihre lokalen Summen können zusammen die im Parent gehaltene Gesamtsumme nicht überschreiten. Eine Übertragung dieser Teilbereiche ist ein quittierter Eigentumswechsel, kein gleichzeitiges Gutschreiben beim Sender und Empfänger.

Der delegierte Kernanschluss konsumiert einmalig einen nicht clonbaren, nicht
serialisierbaren Credit mit Quellen-/Generationsbindung, Gesamt-/Ownergrenzen
und einem Lebensdauerbesitzer für die Retirementmeldung. Der OS-Dispatcher
erzeugt ihn ausschließlich nach Prüfung des konkreten Parentkanals und der
Grant-ID. Der portable Kern prüft die Übereinstimmung von Plan und Credit;
er behauptet keine eigene Authentisierung des Kanals. Erst der letzte lokale
Grant-/Slicebesitzer beendet diesen Credit. Der Parent behält bis zur bestätigten
Retirement- oder Prozessendegrenze die tatsächliche gemeinsame Zusage.

Slices sind disjunkte Buchungsbesitzer, keine physischen zusammenhängenden
Arenaabschnitte. Freies Guthaben darf deshalb nicht durch künstliche
Adressfragmentierung unbrauchbar werden. Ein Split erhält die Summe. Die Zahl
jemals gewählter Quellen einer Sitzung erhält keine neue feste Obergrenze;
Buchungsmetadaten folgen den tatsächlich lebenden Besitzern und checked,
falliblen Containeroperationen. Vorhandene begrenzte Wireframes bleiben erhalten.

```rust
// core/capture_image.rs
pub(crate) struct CaptureImageBuilder { /* mutable Pixel + einzigartige Slice */ }
pub(crate) struct CapturedImage { /* Arc<PixelStorage>; Storage hält die Slice */ }
impl CaptureImageBuilder {
    fn new(size: FrameSize, limits: FrameLimits, grant: &CaptureGrant,
           owner: CaptureOwner) -> std::io::Result<Self>;
    fn pixels_mut(&mut self) -> &mut [u8];
    fn row_mut(&mut self, y: u32) -> Option<&mut [u8]>;
    fn set_regions(&mut self, regions: AdmittedRegions) -> std::io::Result<()>;
    fn freeze(self) -> CapturedImage;
}
impl CapturedImage {
    fn image(&self) -> &BgraImage;
    fn size(&self) -> FrameSize;
    fn view(&self) -> BgraView<'_>;
}
// Clone von CapturedImage teilt nur Storage; keine neue Pixelallokation.
```

`new` prüft Limits, erhält den Slice und allokiert erst danach. Für X11-/JNI-übernommene Bytes gibt es ergänzend einen `AdmittedBytes`-Besitzer mit demselben Ablauf: reservieren, Bytebereich anlegen, als Bild validieren/übernehmen. Ein öffentliches `from_vec(Vec, lease_nachtraeglich)` ist ausgeschlossen. Der Builder gibt ausschließlich feste Pixel-/Zeilenslices heraus, niemals `&mut BgraImage` oder `&mut Vec`: ein ganzer Bildersatz per Zuweisung könnte sonst den Grant umgehen. `AdmittedRegions` hält die vor ihrer Allokation gebuchten Dirty-/Move-Vektoren samt Metadata-Slice und wird beim Freeze in Storage übernommen. Downscale/Rotation erzeugen einen neuen Builder aus dem zuständigen Session-/Capturegrant und geben immutable Ergebnisse zurück. Der vorhandene `ScreenEncoder` kann weiterhin `&BgraImage` aus `CapturedImage::image` ausleihen. Encodierte Ergebnisse und Parent-Rohreplys erhalten entsprechende `AdmittedBytes`-/Slice-Besitzer; ein Bildwrapper allein wäre keine vollständige IPC-Lebensdauerkorrektur.

```rust
// core/host_backend.rs: Metadaten enthalten weiterhin die bestehende Quellenbindung.
pub(crate) trait PreparedSource: Send {
    fn source_key(&self) -> &SourceKey;
    fn description(&self) -> PreparedSourceInfo;
    fn capture_plan(&self, output: &HostOutputPlan)
        -> std::io::Result<Option<CapturePlan>>;
    fn activate(self: Box<Self>, grant: CaptureGrant)
        -> Result<Box<dyn ScreenSource>, SourceRefusal>;
}
// None bei capture_plan = Metadaten noch ausstehend; keine Aktivierungserlaubnis.
// Ergänzung des ScreenHostBackend mit den bestehenden Identitätsargumenten:
fn prepare_session_source(&self, owner: &str, session: &str, source: &SourceId,
    wanted: Option<FrameSize>, control: bool, admission: Arc<SessionAdmission>)
    -> Result<Box<dyn PreparedSource>, SourceRefusal>;
// Zielzustand: SourceFrame::Frame(CapturedImage).
```

`PreparedSourceInfo` enthält native bekannte Größe, getypte Quelllimits, Regionsfähigkeit und den bestehenden Bereitschaftszustand; `HostOutputPlan` enthält ausdrücklich Legacy/v2, den gebundenen Vertrag und die lokale Encode-/Snapshot-Spitze. Eine unbekannte Pixelgröße bleibt unbekannt, sie wird nicht durch Portal-Compositorkoordinaten ersetzt. `activate` prüft Grantidentität und Generation erneut. Für `open_family_member` wird derselbe Admissionkontext aus der Familie weitergereicht; für Login ist `open_login_source(grant)` der aktivierende Anschluss. `ScreenSource::size`, Referenzgeneration, Familienrevision, Inputbindung, `set_capture_active`, HostStage und die Codec-Schnittstelle bleiben semantisch bestehen.

Zuständigkeit dieses ersten Kernteils: neue `capture_admission.rs`/`capture_image.rs`, die dafür nötigen Exporte in `core/mod.rs`, `frame_memory.rs`, der ergänzte Host-Trait und ausschließlich die typbezogenen Anschlüsse in `host_stage.rs`/`host_capture.rs`. Die separate Workerarbeit besitzt danach die privaten Wire-/Dispatcherdateien, die Desktoparbeit die OS-Adapter, die Androidarbeit Kotlin/JNI/Bridge, die Loginarbeit Broker/Helper. Die Aktivierung der neuen Pflichtgrenze im Hostserver erfolgt erst zusammen mit den integrierten Adapterimplementierungen. Es wird keine Zwischenfassung mit einem grantlosen Default als fertiger Zustand abgenommen. Änderungen an Kernsignaturen laufen anschließend über diesen einen Kernverantwortlichen; die Adapter melden nötige Vertragsanpassungen, statt konkurrierend neue Varianten einzubauen.

### Unveränderter v2-Vertrag und gemeinsame Rechnung

`W`, `I≤W/24`, die 24-Besitzer-Klasse und `PacketBudget` bleiben unverändert. Die bestehende Formel lautet mit Recordgröße R, Headerreserve H=1099 und encodierter Framegrenze E:

`P_v2 = 20I + 32(R + H) + (E − I) ≤ W`.

Entscheidend ist die bereits vorhandene lokale Rollentrennung: `host_server` hält W für die lokale Host-Sitzung; der Viewer besitzt seine eigene Sitzungszusage. Ein Hostworker erzeugt keine Viewerdecoder- oder Viewer-Skalierbilder. Die Hostzusage wird deshalb anhand ihrer tatsächlich lokalen Source-/Encode-/IPC-Besitzermenge unterteilt. Eine gleichzeitig lokal laufende Viewersitzung bleibt eine weitere vollständige W-Buchung und wird nicht zur Finanzierung des Hosts herangezogen. Der unveränderte Vertragswert W und seine v2-Validierung werden nicht auf den kleineren augenblicklichen Hostverbrauch reduziert.

Die zusätzlich notwendige Capturezusage wird als **Differenz einer Besitzermenge** berechnet: bereits finanzierte konkrete Bilder zählen einmal. Aus einem gleich großen Bytewert folgt keine Identität. Insbesondere die zwei laufenden/nächsten Viewer-Skalierquellen aus der R7-Dokumentation sind keine freien Hostcaptureplätze. Die vollständige 24-Besitzer-Rechnung bleibt als Vertrag und Abnahme bestehen.

Formal: Für jede lokal mögliche Hostphase q enthält `U_q` die Vereinigung der Capture-, Encode-, Snapshot-, Worker- und Parentbesitzer, dedupliziert nur bei identischer Pixelablage. `L_host = max_q bytes(U_q)`. Die Autorität hält `W + X` mit `X = max(0, L_host − W)`. Disjunkte Subleases setzen U tatsächlich durch. Das ist eine Unterteilung der unverändert vollständigen W-Zusage, kein Umdefinieren von W/24. Bei einem neuen Format/Codec wird die neue Phasenspitze vor dessen Allokation zugelassen. Ein währenddessen lebender alter Besitzer wird zusätzlich in U aufgenommen. Eine neue Grafikfläche gehört nicht automatisch zu einem bestehenden Tile- oder Queueplatz.

### Legacy und fehlende OS-Fakten

Legacy bleibt ein ausdrücklicher Vertragsmodus mit den bisherigen Seiten-/Pixelgrenzen und acht Bildäquivalenten; numerisch gleiche Limits machen v2 nicht zu Legacy. `capture_limits` in `d3d_device.rs` enthält bereits die Absicht `zwei native + sechs gelieferte Bilder`. Diese Absicht muss durch die konkreten gleichzeitigen Besitzer ersetzt werden. Sie belegt weder den WGC-Pool noch eine vollständige Codec-/IPC-Bilanz.

Belegte Encodebesitzer: `TileEncoder` hält ein Previous-BGRA-Bild, maximal einen 64×64-Tile-Scratch und ein u64-Hash je Tile. MF hält produktseitig während der Konvertierung I420 und NV12 gleichzeitig (bei geraden Maßen jeweils 3I/8), anschließend ein NV12-MF-Sample; sein expliziter Outputbuffer richtet sich nach `GetOutputStreamInfo().cbSize` beziehungsweise dem vorhandenen Fallback und muss vor `MFCreateMemoryBuffer` gedeckt sein. Coalescing und AU-Zusammenführen können zusätzliche Kopien erzeugen. OpenH264 hält produktseitig I420 und die erzeugte AU-Kopie. Unveränderte interne Codec-Referenzbilder sind nicht als Produktkopien in diese Rechnung einzusetzen.

Die Legacy-Unterteilung erhält folglich kein frei erfundenes pauschales Zusatzbudget. Für die Quelle werden die unten belegten native-Puffer gezählt; zu ihrer tatsächlich überlappenden Phase kommen der konkrete Encode-/Snapshot-/IPC-Pfad und seine vorhandenen Bildbesitzer hinzu. Die bisherige W-Zusage finanziert diese Summe zuerst. Nur ein darüber hinausgehender **konkreter** Betrag X benötigt eine zusätzliche gemeinsame Zusage. Eine bereits gehaltene unveränderliche Quelle darf gleichzeitig Encoderinput und Quellcache sein, ohne zweimal Pixel zu besitzen.

Die Kapazität ohne OS-Fakten wird explizit von der ersten Teilanforderung getrennt: Bei Beginn einer Kohorte wird ein `CapacityCeiling` aus der vorhandenen Plattformpolitik festgelegt, nicht aus dem zufällig ersten kleinen Capture-Slice. Desktop ohne weitergehenden Beleg bleibt bei höchstens 512 MiB; Android bleibt bei höchstens dem bereits gemeldeten freien Javaheap und 512 MiB. Der neue Capturepfad darf diese Grenzen weder vergrößern noch in einem zweiten Pool umgehen. Bereits lebende Zusagen werden durch eine neue Heapmessung nicht kleiner gerechnet. Eine neue Kohorte darf erst nach Ende sämtlicher Besitzer eine neue Kapazität wählen.

Ein einzelner Legacyfall darf bei tatsächlich unzureichender Kapazität scheitern. Die Konstruktion `W=512 MiB vollständig buchen; danach stets C>0 zusätzlich fordern; deshalb bei facts=None immer scheitern` ist ausdrücklich ausgeschlossen. Die folgende phasenweise Rechnung zeigt den erhaltenen Legacyanschluss.

### Konkrete Hostphasen für Legacy und v2

Der gelesene `host_pipeline.rs::produce` wartet auf genau eine blockierende Stagearbeit und danach auf `host_frame_send.rs::send`; `gop_stream.rs::write_frame` übernimmt Records und schreibt sie der Reihe nach. Es gibt auf diesem Pfad keine nebenläufige zweite Vollbildproduktion. Ein bereits abgebrochener, noch laufender Stagebesitzer ist dagegen weiterhin eine eigene lebende Sitzung/Generation in der gemeinsamen Autorität.

`C` bezeichnet im Folgenden die Capture-Slots aus der Adaptertabelle; `D` ist das zusätzlich skalierte Encoder-/Snapshotbild (höchstens I; null bei Alias auf das bereits in C enthaltene Quellbild). `V` ist das weiter lebende Previous-Bild des Live-Tileencoders (höchstens I), `S` das zusätzliche Previous-Bild eines Snapshot-Tileencoders (höchstens I). `T = ceil(breite/64) × ceil(höhe/64)`. Für den normalen Legacy-Tilepfad ist die tatsächlich auszugebende Nutzdatenmenge `E_tiles ≤ I + 13T`: Pixel, elf Headerbytes pro Tile und konservativ zwei Recordheaderbytes pro Tile. Die temporäre LZ4-Maximalausgabe, Tile-/Hasharrays, Regionsdaten, Container und IPC-Köpfe werden als separat berechnetes `M` addiert. Vektoren bekommen explizit zugelassene Kapazitäten; unkontrolliertes geometrisches Vec-Wachstum darf nicht aus einer reinen `len`-Rechnung verschwinden. Allokatorinterne Rundung bleibt von den bewusst angeforderten Kapazitäten getrennt.

| Phase | Gleichzeitige Besitzer zusätzlich zu C | Begründung |
|---|---|---|
| Live-Tileencode | `D + V + E_tiles + M` | Aktuelles gesondertes Ziel, Previous, Recordliste; Hash und Tile-Scratch. |
| Encoded-Workerübertragung | `V + 4E + M` als konservative Spitze | Worker hält Records und äußeres serialisiertes Payload, während Parent Rohpayload und decodierte Records halten kann. Beim Aufbau der Referenzhülle leben Records, inneres und äußeres Payload (drei Kopien); die vierseitige Sende-/Decodephase ist die größere konservative Grenze. Quelle und live Previous bleiben bestehen. |
| RAW-Snapshotübertragung | `V + D + 3I + M ≤ 5I + M` | Worker-Rawbild ist D oder ein Alias in C; dazu äußeres Sendepayload, Parent-Rohpayload und decodiertes Parentbild. Die innere Serialisierung ist vor dem Senden zerstört. |
| Snapshot-Tileencode im Parent | `V + I + S + E_tiles + M ≤ 3I + E_tiles + M` | Liveencoder bleibt im Worker bestehen; Parent hält Snapshotinput, neuen Snapshotencoder und Records. Eine vorangehende weitere Verkleinerung überlappt höchstens altes/neues Parentbild und endet vor Encode. |
| Geordneter GOP-Sender | `V + E + M` | Records werden übernommen, nicht nochmals zu einem Vollframe zusammenkopiert. Der nächste Produce beginnt erst nach dem abgeschlossenen Sendeschritt. |

Die Tabelle zählt Quellcache und Encoderinput nur dann getrennt, wenn tatsächlich verschiedene Pixelablagen existieren. Für eine ausreichend konservative Vorabzulassung kann `D≤I` eingesetzt werden; der Suballocator erzeugt daraus keinen zweiten Eigentümer eines Alias. Für v2 gelten zusätzlich unverändert R/E/Recordanzahl aus `PacketBudget`; die vorhandenen repack-/Hybrid-Zwischenkopien werden als lokale Phase mit ihren bereits abgeleiteten Grenzen aufgenommen. Es werden keine entfernten Viewerbilder zum Hostmaximum addiert.

Bei H.264 wird der lokale Encodeanteil aus dem ausgewählten Adapter berechnet: OpenH264 benötigt zusätzlich `D + Y + E`, wobei `Y = w×h + 2×ceil(w/2)×ceil(h/2)` die I420-Bytezahl ist. MF benötigt während BGRA→I420→NV12 `D + 2Y`; während der Ausgabekopie konservativ `D + Y + L_out + 2E`, mit `L_out=max(cbSize, vorhandener Fallback)`, nach dessen existierender 64-MiB-Grenze. `E≤16 MiB` bleibt für Legacy eine Einzel-AU-Grenze; v2 benutzt seine engere R-Grenze. Codecinterne Bilder sind unverändert Außenreserve. **Erforderliche Korrektur:** Vor `OpenH264::to_vec`, MF-Coalescing, Outputsample und Recordkopie muss der entsprechende Slice bereits bestehen. Bei OpenH264 werden die NAL-Längen checked summiert, bevor die Produktkopie angelegt wird. Encoderwechsel zerstören den alten Encoder wie bisher zuerst, es sei denn, beide Besitzer sind ausdrücklich zugelassen. Die vorhandenen Codec-/Tiles-Rückfälle bleiben erhalten.

Die MF-Formeln beschreiben die neue Encodeoperation; noch vom MFT gehaltene **produktseitig erzeugte** Samples kommen als `H_retained` hinzu. `ProcessInput` darf das Sample per Referenz behalten. Deshalb endet dessen Slice nicht am Ende des Rustaufrufs. Vorgesehen ist ein kleiner `IMFMediaBuffer`-Besitzerwrapper für die selbst erzeugten Input-/Outputbuffer (neues `os/windows/mf_buffer_lease.rs`), der Daten/inneren Buffer und Lease bis zum letzten COM-Besitzer hält und keine unbegleitete innere Bufferreferenz herausgibt. Die verfügbare Anzahl weiterer Samples folgt aus den noch freien zugelassenen Y-/L_out-Slices; sie wird nicht mit einer unbelegten Codec-Queuezahl gleichgesetzt. Mangelndes Guthaben führt vor neuer Sampleallokation zum vorhandenen Encoder-/Tiles-Rückfall. Unveränderte interne MFT-Kopien bleiben davon getrennt. [IMFTransform::ProcessInput](https://learn.microsoft.com/en-us/windows/win32/api/mftransform/nf-mftransform-imftransform-processinput), [IMFMediaBuffer](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nn-mfobjects-imfmediabuffer).

Konkrete Quelltextrechnung für normale native WGC-4K bei `N=I` und die konservative Capturegrenze 5N: 3840×2160 ergibt I=33.177.600 Byte, T=2040 und `E_tiles≤33.204.120`. Die größte obige reine Bild-/Payloadhülle mit konservativ D=I beträgt `C + max(2I+E, I+4E, 5I, 3I+E) = 331.882.080 Byte`, rund 316,51 MiB. Bis W512 bleiben 204.988.832 Byte für konkret berechnetes M und zusätzliche nachgewiesene Quellenkosten. Bei 4096×2304 ergibt dieselbe Rechnung 377.607.168 Byte, rund 360,11 MiB. Diese Beispiele belegen, dass der neue Legacypfad nicht methodisch leer ist; sie sind keine Gerätekapazitätsmessung. Für große native Quellen mit Bandreadback wird die entsprechende N-/B-/I-Formel eingesetzt, nicht fälschlich 5N Vollreadback verlangt.

Bei einem kleinen v2-Vertrag und einer großen Quelle kann `L_host>W` sein. Dann muss X vor Aktivierung aus dem noch freien gemeinsamen Kapazitätsrahmen kommen; andernfalls meldet die konkrete Quelle einen Ressourcenfehler. Das Accepted-Bild wird nicht eigenmächtig verkleinert, W wird nicht erhöht und eine neue Sourcewahl nicht stillschweigend vorgenommen. Bei passendem Plattformrahmen bleiben kleine Verträge mit größerer zugelassener Quelle zulässig.

## Pufferrechnung der Aufnahmeadapter

Notation: `N = 4 × native_breite × native_höhe`, `I = 4 × gelieferte_breite × gelieferte_höhe`, `F = 4 × familien_canvas_breite × familien_canvas_höhe`. Gemeint sind Nutzpixelbytes. `B` bezeichnet eine tatsächlich angeforderte, vorab bekannte Stride-/Bandgröße. Header, Tile-/Regionsmetadaten und Filterarbeitsfelder werden zusätzlich mit ihrer checked berechneten Länge gebucht. Die Zahlen gelten für eine lebende Geometriegeneration; Übergänge werden gesondert gerechnet.

| Adapter | Belegter heutiger Besitz | Geplanter gleichzeitig gedeckter Capturebesitz |
|---|---|---|
| Windows WGC | `POOL_BUFFERS=2`, ein Readback-Staging, alter Cache, neues Bild und dessen tiefer Cacheklon. Normale Spitze: 2 angeforderte Poolflächen + 1 Staging + 3 CPU-Bilder = 6N Nutzpixel. Ausgecheckte Frames sind Referenzen auf die Poolflächen. | Immutable-Cache: 2N Pool + N Staging + höchstens 2N alte/neue CPU-Ablage = 5N. Wird die alte CPU-Ablage vor dem nächsten Capture freigegeben, sinkt die aktuelle Belegung, nicht die zugesagte Spitze. Framepool vor `CreateFreeThreaded` und `Recreate`, Readback vor `CreateTexture2D`, CPU vor `vec`/Kopie finanzieren. |
| Windows DXGI | Eine erworbene Duplikationsfläche, ein Staging, alter Cache und unrotiertes/rotiertes Bild beziehungsweise Cachekopie. Der Produktpfad hält höchstens ein `AcquireNextFrame` bis `ReleaseFrame`. | Identität: N angeforderte/acquirierte Fläche + N Staging + 2N CPU = 4N. Rotation: dieselben 2N Flächen + alter Cache, unrotiertes Bild, rotiertes Bild = 5N. `DuplicateOutput` erst nach Metadaten-/Grantphase. Unbekannte DXGI-interne Warteschlangen sind nicht als „eine Fläche insgesamt“ ausgegeben. |
| D3D-Bandreadback für größere Quellen | Bestehender Pfad liest höchstens 64 native Zeilen je Band und skaliert in ein geliefertes Bild. Er erzeugt Stagingband, tight Bandkopie, Zielbild, Spalten-/Summenfelder. Bei wechselnder Bandhöhe kann das alte Staging während der Neuanlage leben. | Pool-/Duplikationsflächen bleiben N-basiert. Ein wiederverwendbares Stagingband `B=4×native_breite×min(64,native_höhe)`, eine tight Bandkopie B, geliefertes Ziel I sowie explizite Arbeitsfelder `8×zielbreite + 32×zielbreite` Byte auf dem vorhandenen 64-Bit-Pfad; tatsächliche `size_of`-Rechnung im Code. Tailband verwendet dieselbe Kapazität. Kein nachträglicher Pitchfaktor. |
| X11 mit MIT-SHM | Persistent ein SHM-Segment N, neues tight Bild N, alter Cache N und zeitweise dessen Ersatzklon N. `shm_detach` wird bisher ohne nachgewiesenen Abschluss ausgelöst. | Segment N + höchstens alte/neue immutable CPU-Ablage 2N = 3N; bei XComposite zusätzlich die gehaltene Pixmapfläche P, Cursorbild und Protokollkopf. Segment vor memfd/ftruncate/mmap/AttachFd finanzieren. Budget bis bestätigtem Detach oder geschlossenem Captureanschluss halten. |
| X11 ohne SHM | `get_image` liefert eine kopierte Replydaten-Vec; der x11rb-Parser kopiert aus dem vollständigen Reply. Bestehendes Ziel-Vec und Cache kommen hinzu. | `reply.data` als Pixelablage übernehmen, statt zusätzlich ein Vollbildziel zu kopieren: alter Cache N + Parserrohpuffer N+32 + Replydaten N, also 3N+32 während des Parsevorgangs. XComposite addiert P. Danach höchstens alter/neuer Cache 2N. Eine weitere bereits gehaltene Ergebnisablage benötigt ihre eigene belegte Slice; kein ungezählter dritter Cache. |
| PipeWire | Streambufferzahl und -Stride werden heute nicht ausgehandelt; `on_process` hält beim Latest-Auswählen höchstens zwei **entnommene Referenzen**, nicht den ganzen Pool. `LatestFrame`, `take`-Kopie und paralleler Producer können 3N CPU-Pixel halten. | Vor lokaler SHM-Abbildung pausiert Core.add_mem den Nachrichtendispatch. Exakter Grant der wirklichen FD-Ganzabbildung, getrennt vom Bildplan mit drei tight CPUplätzen. Fehlender Bildslot gibt den nativen Buffer ohne Kopie zurück. Native Producerpools sind externe Ressourcen; kein k/B-Schätzwert. |
| Rust SyntheticSource | Das beim Öffnen erzeugte mutable Quellbild plus tiefer Rückgabeklon. | Zwei N-Slots (Rendererbild und gehaltenes Ergebnis) oder ein vollständig in einen freien zugelassenen Slot gerendertes neues Bild; Entscheidung für zwei Slots erhält den bestehenden Mutationspfad. `open` darf das erste Bild erst mit Grant anlegen. |
| Familienkomposition | Alle geöffneten Mitgliedsquellen, je ein gespeichertes Mitgliedsbild und ein neu erzeugtes Canvas; Mitglieder können eigene Nativepools besitzen. Das vorhandene Summenlimit ist keine Zusage. | Summe der persistenten Mitgliedsanforderungen, maximaler transienter Anteil des jeweils seriell aufgenommenen Mitglieds, Canvas F und noch lebende alte Canvasergebnisse. Mitgliedsbild aliasiert nach Möglichkeit dessen immutable Cache. Die Familie reserviert vor dem Öffnen neuer Mitglieder und vor Canvasallokation; keine doppelte Buchung desselben `Arc`-Bilds. |

Der bestehende X11-Fallback bleibt erhalten. Die zusätzliche Parserkopie ist direkt im gebundenen x11rb 0.13.2 belegt: `GetImageReply::try_parse` erzeugt `data.to_vec()` aus dem Rohreply. [x11rb 0.13.2, xproto.rs](https://raw.githubusercontent.com/psychon/x11rb/v0.13.2/x11rb-protocol/src/protocol/xproto.rs). Der Plan verändert weder den Full-Desktop-Fallback einer regulären X11-Quelle noch die streng gebundene Familienquelle in eine unzulässige Rootaufnahme.

XComposite benennt vorhandenes Offscreenstorage und kann dessen Lebensdauer über Unmap/Resize hinaus verlängern. Diese konkrete vom Produkt gehaltene Fläche wird vor `NameWindowPixmap` als P gebucht, einschließlich aus der Geometrie ermittelter Fensterborder; sie endet erst nach abgearbeitetem `FreePixmap` oder geschlossenem Anschluss. Die Anwendung erzeugt kein zusätzliches serverseitiges Redirect. Um eine Größenänderung zwischen Metadaten und Benennung zu behandeln, erfolgt nach der Finanzierungsentscheidung eine kurze atomare Metadaten-/Benennungssequenz; innerhalb einer gegebenenfalls notwendigen X-Server-Sperre wird niemals auf Budget-IPC gewartet. Bei abweichender Geometrie wird zunächst entsperrt und neu zugelassen. [Composite Protocol 0.4, NameWindowPixmap](https://xorg.freedesktop.org/archive/X11R7.7/doc/compositeproto/compositeproto.txt).

Auch der Cursor darf nicht erst nach `xfixes_get_cursor_image().reply()` finanziert werden: seine Pixelzahl ist vorher unbekannt. Der konkrete Anschluss ist ein begrenzter `AdmissionStream` für den Captureanschluss von `RustConnection<S>`: Er sammelt den festen 32-Byte-Replykopf, prüft Sequenz, Länge und erwartete Antwortart, fordert vor Weitergabe des vollständigen Kopfes die Bytezusage an und gibt erst danach den Body an x11rb frei. Der x11rb-Protoparser vergrößert sein Paket genau nach diesem 32-Byte-Kopf. Für einen Cursor K werden Rohreply K+32 und geparste ARGB-Ablage K gedeckt, bis der Blend beendet ist; kein fester erfundener Cursorgrößenwert. Nicht blockierendes `read`, abbrechbares `poll`, Authentisierung, FD-Reihenfolge und die existierenden SHM-FDs bleiben erhalten. Das Verfahren bindet auch `GetImage` an die vorher zugelassene Antwortgröße. [x11rb Stream](https://raw.githubusercontent.com/psychon/x11rb/v0.13.2/x11rb/src/rust_connection/stream.rs), [generische RustConnection](https://raw.githubusercontent.com/psychon/x11rb/v0.13.2/x11rb/src/rust_connection/mod.rs), [Paketvergrößerung](https://raw.githubusercontent.com/psychon/x11rb/v0.13.2/x11rb-protocol/src/packet_reader.rs).

WGCs angeforderte zwei Poolframes erklären zwei Nutzflächen; sie belegen keine vollständige Treiberspeicherzahl. Vor dem Recreate wird das alte Captureframe freigegeben. Werden alte und neue Poolgeneration überlappt, müssen beide Grants gleichzeitig leben; bevorzugt wird der bestehende Pause-/Reopen-Mechanismus mit beendetem Altpool und separat weiter finanzierten alten CPU-Ergebnissen. [Microsoft: Screen capture](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture), [Direct3D11CaptureFramePool](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool).

Bei DXGI bleiben die neue Rotation und ihre Referenzgeneration erhalten. Native Dirty-/Move-Metadaten sind optional: ihre tatsächlich angeforderten Vektorkapazitäten werden vor der Allokation finanziert; ist nur dieses Optimierungsbudget nicht verfügbar, bleibt der vorhandene vollständige Pixelvergleich zulässig. Ein fehlendes Pixelbudget wird dadurch nicht umgangen.

### PipeWire: Verhandlung und Lebensdauer

Der [C7-Nachplan](pipewire-import.md) ersetzt die frühere Annahme eines asynchronen Formatcallbacks. Nachgeprüft an PipeWire 0.3.65 und gezielt 1.6.9: INACTIVE kann die Linkvorbereitung verhindern, `param_changed` quittiert synchron, und `MAP_BUFFERS` mappt vor `add_buffer`. Ein späterer `update_params`-Aufruf ist deshalb keine Allokationssperre.

Der gewählte öffentliche Anschluss aktiviert den gebundenen Stream ohne MAP_BUFFERS/ALLOC_BUFFERS. `Core.add_mem` pausiert die weitere Nachrichtenzustellung. Nach exakter Unbound-Importzusage derselben Parentautorität wird der tatsächliche FD vollständig und genau einmal über seinen Core-Mempool gemappt; Resume folgt erst danach. Fstat-Länge und tatsächliche Seitengröße bestimmen die vorher gebuchte Mappingkapazität. MemPtr-/Metadaten-/MemFd-Teilansichten nutzen diesen Besitzer. Producer-/Compositorallokationen bleiben fremde, nicht vollständig beobachtbare Ressourcen.

Die Bildzulassung bleibt getrennt: drei tight CPUplätze plus bestehender Hostplan, ohne doppelte Nativeflächenbuchung. Fehlende Producer-Portparameter verhindern keine aktive Anfangsverhandlung. Die tatsächliche Formatgröße, Datenbereiche und Chunkstrides werden geprüft; keine Bufferzahl und kein Pitch werden erfunden. Beide Portalöffnungen verwenden denselben Anschluss und behalten Node-/Serial-/Sitzungsbindung.

Pause schließt die CPUkopie, beendet aber keine gehaltene Abbildung. RemoveBuffer, RemoveMem, Mempool-Removal, Unmap-Rückkehr und Stream-/Core-Destroy sind getrennte Lebensdauerereignisse. Der Slice endet erst nach letzter Abbildung und abgeschlossenem Callback-/Cleanupstack; ein bereits ausgegebenes CPUbild hält seine eigene Zusage weiter. Der vollständige Nachplan enthält die API, private PREPARE-Dispatcherkette, Quellen und Abnahmesignale.

### Android: zusätzlich erforderliche Besitzergrenzen

Android benutzt denselben begrenzten Sitzungsbesitz in Rust und Kotlin. Der vom Javaheap abgeleitete Höchstwert bleibt erhalten. Native Direktpuffer oder ImageReader werden nicht als Begründung für eine größere Gerätezusage verwendet.

| Pfad | Geplanter konkreter Produktbesitz und Haltepunkt |
|---|---|
| Projection-ImageReader | Die bestehenden drei gleichzeitig erwerbbaren Images bleiben erhalten: ein gehaltenes Latest plus zwei zeitweilige Acquires für `acquireLatestImage`. Dafür werden vor `newInstance` drei angeforderte RGBA-Nutzflächen N gerechnet. Das ist **keine** Behauptung, dass die gesamte BufferQueue drei Flächen besitzt. Zusätzlich existieren möglicherweise noch vom Producer verwendete, gefüllte oder freie native Queuebuffer; ihre Gesamtzahl und ihr Pitch sind durch diese öffentliche API nicht festgelegt. Sie bleiben ausdrücklich außerhalb der exakten Produktbytebilanz. |
| RGBA aus Projection | Höchstens drei unterschiedliche ByteArray-Ablagen N: vom Encoder verarbeiteter Frame, neuester wartender Frame und gerade kopierter Producerframe. `latest`, `pending` und `lastPixels` sind nur dann Mehrfachreferenzen, wenn sie nachweislich dieselbe Arrayidentität tragen. Eine vierte Ablage darf nicht aus einem weiteren Callback entstehen; Slotprüfung vor `ByteArray`. |
| JNI-Picture | Bei Paketgröße `J=N + Kopf + Videoanteil` höchstens drei Paketablagen: Producer-Direktbereich J, ein wartendes natives Picture J und ein vom Consumer gehaltenes Picture J; zusätzlich das decodierte BGRA N. Vor einer Ersatzkopie wird der alte wartende Slot unter dem Queuelock entfernt oder der neue Frame ohne Kopie abgewiesen. `bridge_channels::push` prüft Grant/Generation/Slot **vor** `.to_vec`. Es darf nicht erst bei voller Queue eine vierte Paketablage erzeugen. |
| Still über Projection | Still und Encoder teilen das identische immutable RGBA-Bild, wenn Geometrie/Generation passen. Snapshot-Paket/Native-Consumer erhalten eigene Teilzusagen. Die Bytes des ausgewählten native Quellbilds werden vor einer späteren Downscaleallokation nicht freigegeben. |
| Accessibility-Still | Eine ausstehende System-Screenshotanforderung mit erwarteter Displaygeometrie, ihr Hardwarebild N als angeforderte Nutzfläche, eine softwareseitige ARGB-Kopie N, ein wiederverwendbarer Direktbereich N+8, ein natives Picture N+8 und ein decodiertes BGRA N: konservativ fünf N plus 16 Byte, vor zusätzlichem Zielbild I. Eine unnötige zweite ARGB-Konvertierung wird vermieden. Jeder gleichzeitig weiterlebende alte Stillbuffer addiert seine eigene Slice. |
| Synthetische Androidquelle | Die angeforderte Canvas-/Codec-Surfacegeometrie ist vor Start gedeckt. Der Zeichenworker hält die Zusage bis zu seinem tatsächlichen Ende; ein abgelaufenener Join ohne Ende gibt sie nicht frei. Keine zusätzliche erfundene RGBA-Kopie, wenn nur in die Surface gezeichnet wird. |

`ImageReader.maxImages` bezeichnet laut Android nur gleichzeitig erworbene Images. Die API dokumentiert zusätzlich gefüllte, freie und vom Producer verwendete Buffer. Sie erlaubt deshalb keine vollständige Poolbytezahl aus `3×N`. Die konkrete offene Beobachtungsgrenze wird in Diagnose und Dokumentation beibehalten; Produktarrays und JNI-Kopien werden trotzdem vollständig vorher finanziert. [ImageReader](https://developer.android.com/reference/android/media/ImageReader).

`ScreenControlService.still` muss den Grant **vor** `takeScreenshot` besitzen. Eine tatsächliche Rückgabegeometrie wird vor der softwareseitigen Kopie gegen den Grant geprüft; Rotation darf dieselbe Pixelmenge nutzen, eine größere Geometrie benötigt eine erneute Zulassung. Das bereits vom System gelieferte Hardwarebild wird bei Ablehnung sofort geschlossen; dieser systemseitige Ergebniswechsel ist keine nachträgliche Zusage für eine Produktkopie. Der Hardware-Bitmapwrapper wird ausdrücklich recycelt, die softwareseitige Bitmap nach dem letzten Leser freigegeben. `wrapHardwareBuffer` hält selbst eine Referenz, daher reicht `HardwareBuffer.close` allein nicht. `allocationByteCount` wird bei einem wiederverwendeten Bitmapbesitzer für die tatsächlich gehaltene Kapazität berücksichtigt. [Bitmap](https://developer.android.com/reference/android/graphics/Bitmap).

Wiederholtes `allocateDirect` mit anschließendem Verlust der lokalen Referenz gilt nicht als deterministische Freigabe. Vorgesehen ist ein begrenzter, nativ besessener Capturearena-Puffer mit JNI-`NewDirectByteBuffer`-Sicht oder eine nachweislich gleichwertige wiederverwendete, über die gesamte Sitzung gehaltene Ablage. Für die native Arena gilt: kein Resize im übergebenen Bereich; Zugriff nur während eines gültigen Handles; Kotlin-Callbacks/Encoder vollständig beenden, dann Sicht entfernen, dann native Ablage und Slice freigeben. Kein Use-after-free durch einen weitergegebenen ByteBuffer. JNI verlangt ausdrücklich, dass die referenzierte Region während des Java-Zugriffs gültig bleibt. [JNI Direct Buffers](https://docs.oracle.com/en/java/javase/21/docs/specs/jni/functions.html#newdirectbytebuffer).

`ScreenHostControl.applyConfig` wird zu einem finanzierten Generationswechsel: Bedarf vor Reader-/Encodererzeugung, Alt- und Neubesitz gleichzeitig gedeckt oder alte Produktion vollständig stoppen, dann Ersatz erzeugen. `close()`-/Joinfehler werden nicht ignoriert. Ein verspäteter Callback behält seine alte Lease, darf aber kein Bild unter der neuen Epochkennung veröffentlichen. Projectionzustimmung, Accessibilityberechtigung, ausgewählte Quelle, Epochbindung und vorhandene Codec-/Tiles-Rückfälle bleiben erhalten.

## Prozessprotokoll, Öffnung und Abbruch

### Normaler Screenworker

Die private Workerprotokollerweiterung benutzt die bereits geerbten Pipes. Sie benötigt keinen neuen Netzwerkport, kein neues Benutzerkonto und keine Änderung des öffentlichen Screenprotokolls. Alte Viewer und ausdrücklich unversionierte Sitzungen behalten ihren Vertrag. Ein nicht passender Worker derselben Installation wird vor Capture mit einem klaren Protokollfehler beendet; es gibt keinen grantlosen Kompatibilitätsmodus.

1. **Vorbereitung ohne Pixel:** Nach der bestehenden Berechtigungsprüfung startet der Parent den Worker. Ein neues `PREPARE_SOURCE` beziehungsweise `PREPARE_FAMILY` bindet Quellidentität und gegebenenfalls Portalzustimmung, ermittelt verfügbare Metadaten und hält noch keine Produktpixel. `PreparedSourceId` ist an genau diese Pipe und Quelle gebunden. WGC-StartCapture, DuplicateOutput, X11-Bildreply, Android-Reader/Still und Synthetic-Bild sind in diesem Zustand verboten. PipeWire darf die native Verhandlung starten; jeder lokale SHM-Import benötigt vor Mapping seine getrennte typisierte Unbound-Zusage nach C7.
2. **Gemeinsame Zusage vor OPEN:** `host_server` erhält über das vorbereitete Backend die Quelllimits und berechnet seinen unveränderten angenommenen Vertrag. Die Autorität hält W und den berechneten Zusatz X **vor** dem aktivierenden OPEN/OPEN_FAMILY. OPEN enthält vorbereitete Identität, Grant-ID, Generation, Besitzerplan und gebundenen Vertragsmodus. Bereits gehaltene W-Bytes werden in disjunkte Teilzusagen verteilt; im Worker fällt die bisherige unabhängige `frame_memory::reserve`-Buchung weg. Ein kleiner gelieferter Teillease gilt nicht als ausreichender Capturegrant.
3. **Asynchrone Metadaten:** Bei Portal/Android muss die endgültige Formatmeldung gegebenenfalls erst eintreffen. PREPARE bleibt dann asynchron ausstehend; Kontrollnachrichten dürfen laufen, die bildproduzierende Aktivierung wartet auf die konkrete Zulassung. Nach Accepted darf ein späterer geometrischer Wechsel einen neuen Capturebedarf melden, jedoch W und den Framevertrag nicht verändern.
4. **Produktion:** CAPTURE und PRODUCE behalten die einmal gebundene Version, Limits und Referenzgeneration. Beide erhalten vor dem ersten großen Parentread ein passendes Antwortticket. Der bisher nur für PRODUCE vergebene Ticketpfad wird auch für CAPTURE benutzt. Ein RAW-Snapshot darf dadurch nur bereits auf den angenommenen Bildvertrag begrenzte Pixel über die Pipe schicken; die größere native Quelle bleibt unter dem Capturegrant im Worker.
5. **Hintergrundbedarf:** Feste begrenzte Kontrollframes `CAPTURE_NEED`, `CAPTURE_GRANTED`, `CAPTURE_REFUSED` und `CAPTURE_RETIRED` tragen Request-ID, Grant-ID, Source-ID und Generation. Sie dürfen auch ohne laufendes PRODUCE eintreffen. Der Parentreader multiplexiert Kontrollnachrichten und genau die ausstehende budgetierte Bildantwort. Ein Bild ohne Antwortticket oder über dessen Länge beendet den Kanal vor Payloadallokation. Eine grantlose Hintergrundkopie wird bereits am lokalen Slot-Erwerb verhindert.
6. **Keine Sperrkreise:** Der Budgetdispatcher wartet weder auf den `WorkerLink`-Mutex eines laufenden `ask` noch auf die Antwort desselben Captureauftrags. Workerinput wird in Benutzeroperationen und Grantantworten aufgeteilt. Nur kurze fertige Frames halten den jeweiligen Write-Mutex. PipeWire-/Android-Callbacks stellen ihren Bedarf in eine begrenzte Kontrollwarteschlange und warten nicht innerhalb des nativen Callbacks auf Parent-IPC. Wiederholte identische Formatbedarfe werden pro Generation zusammengeführt.
7. **Freigabe:** Der Worker meldet `CAPTURE_RETIRED` erst nach letztem CPU-Slot, geschlossenem OS-Ressourcenbesitz und beendetem zugehörigem Callback. Der Parent darf einen Grant nicht wegen EOF, Close-Anforderung oder Requesttimeout allein freigeben. Ist der Worker nachweislich beendet, können dessen verbliebene lokale Besitzer als beendet gelten; Parentreplys, Snapshotdaten und ausstehende Readerantworten halten weiter ihre eigenen Slices.

Für die phasenweise IPC-Unterteilung wird `worker_frame_wire::encode_reference` auf eine einmalig zugelassene äußere Ablage umgestellt: Referenzpräfix und innere Felder werden unmittelbar in diese Ablage geschrieben. Ein zusätzliches vollständiges inneres Payload entfällt. Worker und Parent erhalten disjunkte Anteile für RAW beziehungsweise Encoded; nach dem Schreiben gibt der Worker seine temporären Slices ausdrücklich zurück. Benötigt der Parent sie für den anschließenden Snapshotencode, erfolgt die Gutschrift erst nach dieser Rückgabequittung. Ein Parentreadticket allein macht Workerbytes nicht frei. Die bestehende vierfache konservative Encoded-Rechnung und `V+D+3I` für RAW bleiben damit auch über die verteilte lokale Durchsetzung tragfähig.

Neue Kontrollframes verwenden die bestehende kleine Kontrollgrößengrenze und feste, checked Zahlenfelder; sie enthalten keine unbeschränkte Bild- oder Ownerliste. Der Parent leitet zulässige Besitzerklassen/Multiplikationen selbst ab. Wiederverwendete Request-IDs, fremde Quellen, alte Generationen, doppelte Freigaben und nicht gedeckte Beträge werden abgewiesen. Eine Workerangabe ist kein OS-Ressourcenfakt. Die vorhandene Programm-/Parentbindung und der langfristige Spawnerthread bleiben erhalten.

### Besitz bei Timeout und Prozessende

`WorkerReader` erhält einen joinbaren Lebensdauerbesitzer. Seine Antwort ist ein `BudgetedReply { payload, slice }`, nicht eine unbegleitete `Vec`. Nach dem Decode geht der Teilbesitz in das zurückgegebene Bild beziehungsweise die Recordliste über; das Rohpayload endet vor der Freigabe seines eigenen Slice. Eine ausstehende Antwort im Kanal hält den Speicher auch nach Drop des normalen Aufrufers.

`WorkerLink` besitzt eine `WorkerLifetime` mit SessionAdmission. Close, Kill und Wait liefern einen unterscheidbaren Zustand `Exited` oder `Retiring`. Bei fehlgeschlagenem Wait verbleiben Kindhandle, Grant und Reader bei einem begrenzten Aufräumbesitzer; sie werden nicht als freier Speicher verbucht. Der Bildschirmaufruf bleibt abbrechbar und muss nicht im UI auf das gesamte Aufräumen warten. Neue Zusagen sehen den weiter gebuchten Altbesitz. Der Aufräumbesitzer beendet sich nach bestätigtem Kindende und geleerten/übertragenen Parentantworten. Fehler werden diagnostiziert; kein Zeitablauf gilt als Freigabebeweis.

Dieselbe Regel gilt ohne Kindprozess: Ein abgebrochener `spawn_blocking`-Auftrag behält `StageOutput`, Capturebild und SessionAdmission bis zum tatsächlichen Ende. Das bestehende Ergebnis-vor-Stage-Drop bleibt erhalten. `SourceFrame`, Snapshot, Encoderinput, Familiencache und JNI-Picture dürfen nach dem Umbau keine unbegleitete Kopie aus einer Captureablage herausgeben.

### Größenwechsel, Pause und Wiederaufnahme

Eine neue Größe ist ein neuer Captureplan, auch wenn der Accepted-Vertrag gleich bleibt. Ein Wechsel verwendet entweder gleichzeitig gedeckten Alt-/Neubesitz oder stoppt zuerst den alten nativen Producer und gibt dessen Flächen nachweislich frei. Bereits veröffentlichte alte CPU-Ergebnisse bleiben unabhängig davon gedeckt. Erst danach darf die neue Allokation die frei gewordenen Slices nutzen.

Bei gescheiterter Zusage wird die bestehende Referenz ungültig, die Quelle pausiert beziehungsweise meldet den bestehenden Capturefehler mit Ressourcenursache. Es wird kein Bild aus abgeschnittenen alten Flächen veröffentlicht. Die Pixelgeometrie, Familienrevision und Inputreferenz wechseln gemeinsam. Preview/Full und Snapshot ändern lediglich ihre bisherige Ausgabe-/Encodewahl innerhalb des Vertrags. Pause bedeutet nur dann Freigabe, wenn der betreffende Adapter seine Ressourcen tatsächlich geschlossen hat; ein inaktiver DXGI- oder PipeWire-Besitzer bleibt gezählt. Input-Releases dürfen bei Budgetmangel und während des Aufräumens weiterhin erfolgen und benötigen keinen neuen Pixelgrant.

## Loginbroker und Loginhelper

Die konkrete Öffnungsgrenze ist `login/os/shared/helper.rs::run`: `HelperInit` empfangen, `prepare_helper(binding,parent_id,probe)` prüfen, anschließend derzeit sofort `screen::os::open_login_source`. Der Parent ist `login/os/shared/child.rs::HelperLink::start`, aufgerufen aus `broker.rs::serve`; dies ist kein gewöhnlicher Screenhostworker.

Der vorhandene Kanal bekommt dieselbe Prepare-/Granttrennung. Der Broker besitzt eine gemeinsame Captureautorität für seine tatsächlich lebenden Loginhelpersitzungen. `HelperInit` enthält zusätzlich die Ressourcen-Protokollversion und delegierte Sitzungs-/Grantkennung; `cookie`, `parent_id`, `SessionBinding`, verifizierte Installation, signierte Operationen und die Prüfung vor/nach jeder Wirkung bleiben verbindlich. Ein Budgetgrant erteilt keinerlei Loginrecht. Der Probe-Modus bleibt pixelarm und darf keinen Capturepool erzeugen.

Der Helper ermittelt nach `prepare_helper` die gebundene Ausgabegeometrie, meldet seinen Bedarf und öffnet die Loginquelle erst nach dem Grant. Seine eigenen Ressourcenfakten ersetzen nicht die gemeinsame Parentbuchung. Die Größenbindung bleibt unveränderlich; der bisher erforderliche neue Loginaufbau nach Geometrie-/Desktopwechsel wird erhalten.

Windows `login_source::Context` hält DXGI-Capturegrant, Pending-Bild und Quellenidentität gemeinsam. `context_matches` und `inject_login_input` können `probe()` und damit Capture auslösen; sie sind daher auch ohne ausdrückliche Captureoperation gedeckt. Linux `LoginSource` übergibt den Grant an `X11Source` und hält den bestehenden Geometry-Lease. `pending` wird auf immutable Capturebesitz umgestellt, nicht grantlos geklont.

Der Login-Pixelpfad schreibt bereits unmittelbar aus der Quellablage; er benötigt keine neue Vollbildserialisierung im Helper. `child.rs` allokiert dagegen beim Lesen ein Pixel-Vec und kann eine Antwort im Eintragskanal halten; `broker.rs` hält dieses Vec bis zum abgeschlossenen Weiterreichen. Diese konkrete Parentkopie erhält ihre Slice vor `wire::read_frame(PIXELS)` und trägt sie bis zum letzten Weiterreichen. `pixel_bytes` allein ist kein Grant. Unter der seriellen Anfragegrenze wird vor der nächsten Pixelantwort ein neuer freier Slot nachgewiesen; ein still weiterlaufender Answerthread erzeugt keine ungezählte zweite Antwort.

Die jetzige `child::Lifetime::stop()`-Signatur bestätigt keinen Prozessabschluss. `login/os/linux_os/spawn.rs::Life` ignoriert abschließend Kill-/Waitfehler; `login/os/windows/spawn.rs::Life` ignoriert das Ergebnis von `TerminateProcess` und dem abschließenden `WaitForSingleObject`. Beide werden um den nachprüfbaren Exit-/Retiring-Zustand ergänzt. Unter Linux bestätigt ein erfolgreicher Child-Wait das Ende; unter Windows bestätigt ausschließlich der passende signalisierte Prozesshandlezustand das Ende. Prozess-/Jobhandle und Grant verbleiben bis dahin beim Aufräumbesitzer. Der vorhandene abgesicherte Spawnpfad mit Hashprüfung, Greeteridentität beziehungsweise Windows-Token-/Sitzungsbindung wird erhalten.

## Umsetzungsmeilensteine und Abnahmesignale

Die Dateiangaben nennen bestehende gelesene Integrationspunkte sowie ausdrücklich vorgeschlagene neue Module. Der Hauptagent integriert die Meilensteine als eine zusammenhängende R9-Korrektur. Neue oder wesentlich bearbeitete Rustdateien werden nach Projektregel unter 500 Zeilen/50 KiB gehalten; insbesondere die bereits großen Host-/Wire-/Androidadapter bekommen kohärente Submodule. Kein Meilenstein autorisiert eine lokale Prüfungsausführung oder eine eigenständige Veröffentlichung.

| Nr. | Konkretes Ergebnis und Dateien | Abhängigkeit | Erwartetes, später prüfbares Ergebnis |
|---|---|---|---|
| C1 | `core/frame_memory.rs` um explizite Kohortenkapazität, exakte Zusatzreservierung und disjunkte Subleases ergänzen; reine Planungstypen in neuem `core/capture_admission.rs`; `os/shared/session_memory.rs` und die drei OS-`session_memory.rs` reichen getypte Fakten/Höchstwerte. | Keine | Zwei parallele Hostgrants, ein separater Viewergrant und ein Workeranteil werden gemeinsam einmal gezählt; Teilzulassung kann keine große Allokation freigeben; `facts=None` friert nicht versehentlich einen kleinen ersten Slice als Gerätegrenze ein; Android bleibt unter seinem bisherigen Heapwert. |
| C2 | `SessionAdmission` und immutable `CapturedImage` in neuem `core/capture_image.rs`; `host_backend.rs`, `host_stage.rs`, `host_capture.rs`, `host_pipeline.rs`, `pixel_convert.rs`, `synthetic.rs`/`synthetic_backend.rs` anpassen. Hostphasenplan für Tiles/MF/OpenH264; Allokations-/Encodegrenzen in `tiles_codec.rs`, `host_hybrid.rs`, `codec_registry.rs`, `mf_media.rs`, `mf_encoder.rs`, `mf_encoder_sync.rs`, `openh264_codec.rs`; neuer `os/windows/mf_buffer_lease.rs` für den COM-Endbesitz selbst angelegter Samples. | C1 | Cachealias, Rückgabebild und Encoderborrow zählen dieselbe Ablage einmal. Eine wirkliche Kopie bekommt vorher einen Slice. Kein Snapshot-/Encoderresultat verliert seine Zusage durch Stageabbruch; ein MFT-AddRef hält den Sampleanteil weiter. Legacy-4K bleibt innerhalb seines belegten W512-Beispiels. |
| C3 | Prepare/Activate in `family_listing.rs`, `host_server.rs`, `host_backend.rs` sowie `os/{windows,linux_os}/host_backend.rs`, `os/android/bridge_backend.rs` und `os/shared/host_worker_backend.rs`. Vorgeschlagene Hilfstypen in `core/capture_admission.rs`. | C1–C2 | Der instrumentierte Allokator sieht vor einem abgelehnten OPEN null Bild-/Captureflächenallokationen. Berechtigungsverlust zwischen Prepare und Activate verhindert Start; Input-Releases bleiben möglich. Keine Änderung von W/24 oder PacketBudget. |
| C4 | Getyptes Ressourcenprotokoll in `core/worker_wire.rs`, neuem `core/worker_capture_wire.rs`, `worker_frame_wire.rs`; Dispatcher/Lebensdauer in `host_worker_client.rs`, `host_worker_reader.rs`, `host_worker_main.rs` und neuem `os/shared/capture_broker.rs`. | C1–C3 | Kindprozesse buchen keine unabhängige Kapazität. Vor OPEN ist ein Parentgrant gehalten. Resizebedarf ohne laufendes PRODUCE funktioniert ohne Mutex-Sperrkreis. RAW-CAPTURE erhält sein eigenes Accepted-Ticket. Falsche Generation/ID und übergroße Antworten allokieren keine Payload. |
| C5 | WGC/DXGI/Readback in `capture_source.rs`, `capture_wgc.rs`, `capture_dxgi.rs`, `d3d_device.rs`; Grants vor Pool, Duplikation, Staging und CPUcopy; bandweises Staging wiederverwenden. | C2–C4 | Gemessene Ereignisfolge im späteren Adapterfake entspricht 5N-WGC-/4N- beziehungsweise 5N-DXGI-Plan; Rotation bleibt korrekt. Resize mit gehaltenem Altframe überschreitet keine Zusage. Fehlende Zusatzkapazität stoppt vor Recreate/CreateTexture2D. |
| C6 | X11-SHM/Reply/Cache in `x11_capture.rs`, Geometrie in `x11_capture_target.rs`, Cursor in `x11_cursor.rs`; neuer `os/linux_os/x11_admission_stream.rs` als `RustConnection<AdmissionStream>`-Anschluss. | C2–C4 | SHM, vollständiger GetImage-Fallback und XComposite bleiben möglich. Replykopf kann keine große Parserallokation vor der Zusage auslösen; Cursorgröße wird tatsächlich gelesen und finanziert. Pixmap/SHM-Lease endet nach bestätigter serverseitiger Freigabe. |
| C7 | PipeWire-ABI/POD und Callbackbesitz in `pipewire_abi.rs`, `spa_pod.rs`, `pipewire_stream.rs`, `pipewire_pause.rs`; beide Öffnungen `portal_session.rs`, `portal_input.rs`; Identitätsbindung `portal_source_binding.rs`/`pipewire_identity.rs` erhalten. | C2–C4 | Core.add_mem pausiert vor dem nächsten lokalen Mapping. Exakte FD-Ganzabbildungen und drei CPU-Slots sind getrennt gedeckt; ein unkonfigurierter Producer startet aktiv. Pause gibt Imports nicht frei; Retirement wartet auf tatsächliches Mapping-/Coreende. Kein Wechsel auf eine ungebundene Node oder Rootquelle. |
| C8 | Familien-Besitzerplan in `remote_window/os/shared/family_source.rs`, `screen/os/shared/family_backend.rs` und `core/family_listing.rs`; OpenMember erhält Prepare-/Grantkontext. | C3, C5–C7 | Neue Mitglieder und Canvas werden vor Allokation gemeinsam zugelassen. Überlappende Fenster addieren ihre tatsächlichen Ressourcen unabhängig vom Canvasunionmaß. Entfernte Mitglieder mit gehaltenen Ergebnissen bleiben gezählt. Bestehende Layoutrevision/Identität und fremdfensterfreie Komposition bleiben erhalten. |
| C9 | Android-Capturearena und Grantpfad in `bridge_backend.rs`, `bridge_source.rs`, `bridge_channels.rs`, `mobile/os/shared/domains/screen_host.rs`, `android-bridge/src/screen_jni.rs`, `NativeBridge.kt`; Java-/Kotlinbesitz in `ScreenHostControl.kt`, `ScreenProjectionFrames.kt`, `ScreenStillSource.kt`, `ScreenControlService.kt`, `ScreenEncoder.kt`, `SyntheticSurfaceSource.kt`, `ScreenFrameLimits.kt`, `ScreenHostService.kt`. Neue auf Capture begrenzte Typen `ScreenCaptureBudget.kt` und `ScreenCaptureBuffer.kt`. | C1–C3, C2-Hostphasen | Reader, Arrays, native Arena und Picturekopien erhalten Grant vor Allokation. Voller Queue-Slot erzeugt keine zusätzliche Kopie. Altes und neues Configbudget überlappen nur bei Deckung; Callback nach Close hält alte Lease und publiziert nicht neu. Stillhardwarewrapper/Softwarebitmap werden am letzten Zugriff geschlossen. Kleiner Heap wird nicht angehoben. |
| C10 | Loginintegration in `login/core/wire.rs`, `login/os/shared/{broker,child,helper,mod}.rs`, `login/os/linux_os/spawn.rs`, `login/os/windows/spawn.rs`, `screen/os/{windows,linux_os}/login_source.rs` und `screen/os/mod.rs`. | C1–C2, C5–C6 | Helper kann vor Grant weder öffnen noch durch Geometry-Probe capturen. Cookie/Binding-/Signaturfehler bleiben abgewiesen. Parentpixel und Pendingcache halten ihre Slices. Stop ohne bestätigten Exit gibt kein Budget frei. |
| C11 | Ereignisbasierte Abnahmefälle in `core/frame_memory_tests.rs`, `os/shared/host_worker_main_tests.rs`, `core/batches_snapshot_task_tests.rs` und fokussierten neuen Capture-Plan-/Adapterfake-Modulen; bestehende Batchsuite durch den Hauptagenten erweitern. | C1–C10 integriert | Jede große Produktallokation hat einen vorherigen Grant; jeder Grant endet nach seinem letzten Besitzer. Dieselben Fälle prüfen Legacy, kleine v2-Verträge, 4K, Snapshot, Preview/Full, Familien, Login und Abbruch. Es bleibt genau eine abschließende Remote-Task-Suite. |

Die neuen Android-JNI-Funktionen werden in C9 zusätzlich über die gelesene Kette `mobile/os/shared/domains/screen_host.rs → mobile/os/shared/domains/mod.rs → mobile/mod.rs → android-bridge/src/screen_jni.rs → NativeBridge.kt` ergänzt. Die Captureimplementierung übernimmt keine Viewer-CPU-, WGPU- oder Glow-Endbesitzeränderungen; die parallele Rendererarbeit besitzt ihre getrennten App-/Libwrapperflächen.

## Eine abschließende Abnahmespezifikation

Die spätere gemeinsame Remote-Suite beantwortet diese konkreten Fragen; sie wird durch diesen Plan weder gestartet noch in mehrere Plattform-/Meilensteinläufe aufgeteilt:

- **Vorherige Zusage:** Ein instrumentierter Buffer-/Flächenerzeuger protokolliert `grant → allocate → publish → final_drop → release`. Jede Allokation ohne gültigen Slice lässt den Fall fehlschlagen. Ablehnung direkt vor OPEN, Recreate, SHM-Anlage, PipeWire-Mapping, Android-Reader und Screenshotkopie erzeugt keine große Produktallokation.
- **Eine Parentautorität:** Zwei echte synthetische Captureworker teilen denselben Parentpool; parallele genehmigte Summen überschreiten dessen Kapazität nicht. Ein absichtlich großer Kindbedarf scheitert trotz lokal freiem Kindadressraum. Ein zusätzlich lokal laufender Viewer behält seine eigenständige W-Zusage. Für die Loginbroker-/Helpergruppe gilt derselbe Fall an deren vorhandenen Pipes.
- **Lebensdauer:** Der Test hält ein Ergebnis nach Source-Drop, nach Abbruch eines blockierenden Stageauftrags und nach Close weiter fest. Budget bleibt belegt. Separat bleibt ein Worker-/JNI-/Portalcallback im Aufräumen stehen; eine neue Sitzung darf dessen noch lebende Bytes nicht erneut verbrauchen. Erst bestätigtes Ende plus letzter Ergebnisdrop macht sie frei. Kill-/Waitfehler werden als Retiring geprüft.
- **Geometrie und Generation:** Alter Cache und alter RAW-/Snapshotbesitzer bleiben während Resize erhalten; neue Nativefläche muss vorher gedeckt sein. Ein abgelehnter Übergang publiziert keine geclippte Referenz. Die Generation bindet Grants, Familienrevision und Eingaben; eine gleich große neue Quelle darf kein altes Guthaben übernehmen.
- **Plattformzählung:** WGC-Poolreferenzen werden nicht zusätzlich zu denselben Poolflächen gezählt; DXGI-Rotation besitzt ihren gesonderten CPU-Slot. X11-SHM und GetImage-Fallback werden beide geprüft, ebenso Pixmap-/Cursorreplys mit tatsächlicher Länge. PipeWire zählt die tatsächlichen FD-Ganzabbildungen mit realer Seitengröße und getrennter RemoveMem-/Unmap-/Core-Destroy-Lebensdauer. Android unterscheidet die drei erworbenen Images ausdrücklich von unbekannten native Queueinternas und zählt seine Arrays/JNI-Pictures separat.
- **Vertragserhalt:** Unverändertes W/24 und unverändertes PacketBudget; Legacy ohne Capabilities bleibt ein gültiger Pfad. Die obige 4K-Legacyrechnung muss ohne einen zusätzlichen pauschalen RAW-Topf zulassen. Ein kleiner v2-Vertrag mit größerer Quelle funktioniert bei ausreichender gemeinsamer Kapazität und scheitert bei unzureichender Kapazität vor Capture. Keine zweite Android-Heapreserve. Die vorhandenen 4K-/2-Mbit/s-/Lossless-, Preview/Full-, Snapshot- und Familienintegrationsfälle des aktiven Batches werden weiterverwendet.
- **Berechtigung und Isolation:** Ablehnung nach entzogener Zustimmung/Quellidentität; keine Familien-Rootaufnahme; vorhandene Portalnodebindung; Loginbinding, Cookie, Signatur und Release-Eingaben; Workercrash beendet die betroffene Sitzung und gibt erst nach Prozessende die dortigen Ressourcen frei.

Ein Adapterfake belegt den Produktpfad und die Reihenfolge, keine tatsächliche Treiberpufferzahl. Verfügbare Plattformausführung kann diesen Beleg ergänzen. Fehlende Livegeräte oder Providerzugänge werden als nicht ausgeführt dokumentiert und nicht als bestanden ausgegeben; sie sind nach Projektregel kein eigener Releasehaltepunkt. Native Implementierung, geforderter kritischer Batchreview, die eine Remote-Suite und ein terminaler Remote-Release liegen beim Hauptauftrag. Dieser reine Plan löst weder Build noch Release aus.

## Zweite Lückenrecherche und verbleibende Belege

Die zweite Recherche hat fünf anfängliche Annahmen konkret ersetzt: zwei unabhängige Worker-OnceLocks werden durch eine Parentautorität ersetzt; W+C pauschal wird durch die lokale phasenweise Host-Unterteilung ersetzt; `maxImages=3` wird nicht als gesamte Androidqueue ausgegeben; D3D-RowPitch erhält keine erfundene obere Konstante; die X11-Cursorantwort wird schon vor x11rbs Header-ausgelöster Payloadvergrößerung zugelassen. Der gelesene Loginhelperpfad ergänzt ausdrücklich Capture außerhalb PRODUCE.

Noch fehlende **Implementierungsbelege**, die C1–C11 herstellen müssen:

1. Der konkrete Slotallocator und die typed Übergaben existieren noch nicht. Daher ist weder die vollständige Durchsetzung aller Besitzerpläne noch R9 selbst als geschlossen erklärt. Die genannten maximalen Slotzahlen sind umzusetzende, aus den vorhandenen Producer-/Consumerpfaden abgeleitete Grenzen.
2. C7 muss den im [Nachplan](pipewire-import.md) belegten Core.add_mem-/Pause-Anschluss, exakte Unbound-Importgrants und das vollständige Mappingretirement implementieren. Die frühere Behauptung einer Formatcallback-Allokationssperre ist widerlegt. Producerinternas werden nicht als gedeckte Produktbuffer ausgegeben.
3. Der `AdmissionStream` muss im späteren Code nachweisen, dass kein vollständiger großer Replykopf vor Budgetfreigabe an x11rb gelangt, dass er nicht blockierend liest und dass X11-Authentisierung/FD-Reihenfolge unverändert bleiben. Der generische Streamanschluss und der Allokationszeitpunkt sind in x11rb 0.13.2 belegt; der neue Adapter ist noch nicht geschrieben.
4. Die nachgelesenen Login-`Life`-Adapter müssen ihren Endzustand künftig zuverlässig zurückgeben; ein Rückgabewert der alten `stop()`-Signatur existiert nicht. Die JNI-Reexportkette ist inzwischen vollständig benannt. Offen sind hier die tatsächliche Umsetzung der Exit-/Retiring-Übergabe und der JNI-Arena-Endbesitz, keine Dateisuche und keine neue Dienstverbindung.

Bewusst **außerhalb** des Ergebnisanspruchs bleiben unbekannte ImageReader-/MediaCodec-/Compositor-Queueflächen, native Treiberpadding-/Allokationsgrößen, zukünftiger Speicherdruck, fremde Prozesse und eine gemeinsame systemweite Autorität über unabhängige Dienste. Diese Grenze darf nicht zur Ausnahme für selbst angelegte RAW-/ByteArray-/Vec-/DirectBuffer-/Kompositionskopien werden.

Alle externen Quellen in diesem Plan wurden am 2026-10-26 gelesen. Bibliotheksbelege sind auf x11rb 0.13.2 und den nachgewiesenen PipeWire-API-Stand 0.3.65 bezogen; aktuelle API-Dokumentation wurde ergänzend gelesen. Es wurde kein Geräteverhalten, keine Speicherbelegung und kein Testresultat behauptet.

## Abschlussstand der Planentscheidung

Ziel: eine umsetzbare native Capturezulassung aus dem belegten Bestand. Kriterien: tatsächlicher Grant vor Allokation, gemeinsame Parent-/Kindbuchung, vollständiger Ergebnis-/Abbruchbesitz, konkrete Plattformpuffer und unveränderter Vertrag. Der ausgewählte Ansatz erfüllt diese Kriterien als Detailplan; die benannten Implementierungsbelege bleiben Teil der späteren Umsetzung. Die verworfenen Ansätze und ihre Gründe sind oben festgehalten. Nächster autorisierter Schritt dieses abgegrenzten Auftrags ist die Übergabe an den Hauptagenten; keine Produktänderung. Schleifenstatus: Planentscheidung erreicht.


## Arbeitsbericht und gelesene Dateien

Erstellt und während der Untersuchung aktualisiert wurde ausschließlich `docs/plaene/2026-10-24-batches/aufnahme-speicher.md`. Es wurden keine Repositorydateien geändert und keine Builds, Compiler, Tests, Server, Installationen, Graphneuberechnungen, Commits, Pushes oder Releasekommandos ausgeführt. Der Plan wurde anhand der Quellen, seiner Zahlenbeispiele und seiner Besitzerübergänge statisch gegengelesen; eine erfolgreiche Umsetzung oder Laufzeitabnahme ist damit nicht behauptet.

Die folgende Liste nennt sämtliche gelesenen Repositorydateien innerhalb der freigegebenen Fläche. Bei großen Dateien wurden die relevanten Symbole, Treffer und Abschnitte gelesen; die Liste behauptet keine Vollprüfung jeder Datei. `AGENTS.md` lag zusätzlich als vollständiger Nutzer-Anweisungstext vor. Gelesen wurden außerdem das ausdrücklich freigegebene Lesemanifest `/tmp/smartexplorer-capture-admission-plan-scope.json`, diese Plandatei, die gelieferte Arbeitsweise und deren ausgelagerte kreative Referenz. Die verwendeten externen Primärquellen stehen jeweils beim betreffenden Befund.

- `android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenControlService.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenFrameLimits.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostService.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionFrames.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenStillSource.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/SyntheticSurfaceSource.kt`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/review-b6.md`
- `docs/plaene/2026-10-24-batches/umsetzung.md`
- `native/Cargo.lock`
- `native/Cargo.toml`
- `native/android-bridge/src/screen_jni.rs`
- `native/src/login/core/wire.rs`
- `native/src/login/os/linux_os/spawn.rs`
- `native/src/login/os/shared/broker.rs`
- `native/src/login/os/shared/child.rs`
- `native/src/login/os/shared/helper.rs`
- `native/src/login/os/shared/mod.rs`
- `native/src/login/os/windows/spawn.rs`
- `native/src/mobile/mod.rs`
- `native/src/mobile/os/shared/domains/mod.rs`
- `native/src/mobile/os/shared/domains/screen_host.rs`
- `native/src/remote_window/os/shared/family_source.rs`
- `native/src/screen/core/batches_snapshot_task_tests.rs`
- `native/src/screen/core/codec_registry.rs`
- `native/src/screen/core/family_listing.rs`
- `native/src/screen/core/frame_budget.rs`
- `native/src/screen/core/frame_memory.rs`
- `native/src/screen/core/frame_memory_tests.rs`
- `native/src/screen/core/frame_size_caps.rs`
- `native/src/screen/core/gop_stream.rs`
- `native/src/screen/core/host_backend.rs`
- `native/src/screen/core/host_capture.rs`
- `native/src/screen/core/host_frame_send.rs`
- `native/src/screen/core/host_hybrid.rs`
- `native/src/screen/core/host_pipeline.rs`
- `native/src/screen/core/host_server.rs`
- `native/src/screen/core/host_stage.rs`
- `native/src/screen/core/mod.rs`
- `native/src/screen/core/pixel_convert.rs`
- `native/src/screen/core/synthetic.rs`
- `native/src/screen/core/tiles_codec.rs`
- `native/src/screen/core/wire_binary.rs`
- `native/src/screen/core/worker_frame_wire.rs`
- `native/src/screen/core/worker_wire.rs`
- `native/src/screen/os/android/bridge_backend.rs`
- `native/src/screen/os/android/bridge_channels.rs`
- `native/src/screen/os/android/bridge_source.rs`
- `native/src/screen/os/android/session_memory.rs`
- `native/src/screen/os/linux_os/host_backend.rs`
- `native/src/screen/os/linux_os/login_source.rs`
- `native/src/screen/os/linux_os/pipewire_abi.rs`
- `native/src/screen/os/linux_os/pipewire_identity.rs`
- `native/src/screen/os/linux_os/pipewire_pause.rs`
- `native/src/screen/os/linux_os/pipewire_stream.rs`
- `native/src/screen/os/linux_os/portal_input.rs`
- `native/src/screen/os/linux_os/portal_session.rs`
- `native/src/screen/os/linux_os/portal_source_binding.rs`
- `native/src/screen/os/linux_os/session_memory.rs`
- `native/src/screen/os/linux_os/spa_pod.rs`
- `native/src/screen/os/linux_os/x11_capture.rs`
- `native/src/screen/os/linux_os/x11_capture_target.rs`
- `native/src/screen/os/linux_os/x11_cursor.rs`
- `native/src/screen/os/mod.rs`
- `native/src/screen/os/native_ffi/openh264_codec.rs`
- `native/src/screen/os/shared/codec_inventory.rs`
- `native/src/screen/os/shared/family_backend.rs`
- `native/src/screen/os/shared/host_worker_backend.rs`
- `native/src/screen/os/shared/host_worker_client.rs`
- `native/src/screen/os/shared/host_worker_main.rs`
- `native/src/screen/os/shared/host_worker_main_tests.rs`
- `native/src/screen/os/shared/host_worker_reader.rs`
- `native/src/screen/os/shared/session_memory.rs`
- `native/src/screen/os/shared/synthetic_backend.rs`
- `native/src/screen/os/windows/capture_dxgi.rs`
- `native/src/screen/os/windows/capture_source.rs`
- `native/src/screen/os/windows/capture_wgc.rs`
- `native/src/screen/os/windows/d3d_device.rs`
- `native/src/screen/os/windows/host_backend.rs`
- `native/src/screen/os/windows/login_source.rs`
- `native/src/screen/os/windows/mf_encoder.rs`
- `native/src/screen/os/windows/mf_encoder_sync.rs`
- `native/src/screen/os/windows/mf_media.rs`
- `native/src/screen/os/windows/session_memory.rs`

Wesentliche Entscheidungen sind eine Autorität je tatsächlicher Parent-/Capture-Kindgruppe, unverändertes W mit belegter Host-Unterteilung, getypte unveränderliche Bildbesitzer, vorherige exakte Adaptergrants sowie nachgewiesenes Ende vor Freigabe. Die noch fehlenden Belege betreffen die geplante Implementierung, insbesondere PipeWire-Verhandlung, X11-Replygate, verteilte Subleaseübergabe und native/JNI-/COM-Endbesitzer. Es fehlt keine Nutzerentscheidung und keine erforderliche weitere Dateisuche für den beschriebenen Plananschluss.


## Integrationsentscheidung des Hauptagenten

Der Hauptagent übernimmt diesen Detailplan am 2026-10-26 nach Prüfung der
Host-/Workergrenze, des Login-Prozessendes, der JNI-Reexportkette und des
Besitzerkerns. Die API gibt ausschließlich feste Pixelslices aus und prüft
gleichzeitig belegte Owner- und Gesamtbytes. Das verhindert ungebuchte
Bildersatzallokationen und mehrfach verwendete Phasenbudgets.

Die Umsetzung beginnt mit dem abgegrenzten portablen Besitzerkern und dem
additiven Host-Trait-Anschluss. Die produktive Pflichtgrenze wird anschließend
mit den Worker-, Windows-, Linux-, Android-, Familien- und Loginadaptern
integriert. Der Kernteil allein schließt R9 nicht. Die bestehende Viewer-
MemoryLease-Schnittstelle bleibt für die parallel laufende Rendererarbeit
kompatibel. Alle Teilkorrekturen gehören zu derselben abschließenden Suite und
demselben terminalen Release.


### Verbindliche Anschlüsse nach der Kernintegration

Der portable Kern liegt am 2026-10-26 als Quellcode vor; Plattformaktivierung,
Workerprotokoll und Login-Empfang bleiben offen. Der additive Host-Trait behält
seinen `Arc<SessionAdmission>`-Parameter. Für das Kind konstruiert
`SessionAdmission::from_verified_parent(session, working, contract, authority,
lifetime)` einen verifizierten delegierten Kontext ohne lokalen Pool und ohne
neue W-Zusage. Seine Autorität reicht an den Parentdispatcher weiter; jeder
zurückgegebene Grant hält zusätzlich den Sitzungs-Endbesitzer. Der Konstruktor
selbst authentisiert keinen Kanal.

`CapturePlan::add_serial_member(persistent, transient)` bildet die Summe der
persistenten Mitgliedsbesitzer und die alternativen transienten Phasen ab.
`concurrency_cases()` erhält die tatsächlichen Gleichzeitigkeitstabellen;
`add_member` zählt unabhängige Pläne gleichzeitig. Die realen Slices bleiben
zusätzlich durch den gemeinsamen Gesamt- und Ownerzähler begrenzt.
`prepare_family_member(owner, session, native, admission)` ist die entsprechende
Metadata-only-Backendgrenze. Ihr Default verweigert die Vorbereitung.

`CapturedImage::grant()` klont ausschließlich den zugehörigen Root-/Memberscope.
Damit können generische Downscale- und Codecanschlüsse denselben Besitzerplan
verwenden. `AdmittedBytes::from_allocator(grant, owner, bytes, allocate)` erlaubt
einen vorher begrenzten Bibliotheksallocator innerhalb eines bereits erworbenen
Slices. Beim X11-Anschluss ist das der belegte Parser. Die Kapazitätskontrolle
nach der Closure ersetzt keinen vorherigen Library-Allokationsbeleg. Ein
kleineres Resultat darf später nicht über seine tatsächliche Vec-Kapazität
wachsen. Eine zusätzliche Vollbildkopie ist für diesen Besitzerübergang unnötig.


### Ergänzung des produktiven Loginbildpfads

Der [Loginbildplan](login-bildspeicher.md) konkretisiert C10 für den Empfang und die Darstellung in L1–L8. Er verwendet denselben lokalen Pool für tatsächliche Produktkopien und begrenzt Rollen, ohne eine zweite W512-Sitzungszusage einzuführen. Die dortigen Empfangs-/Rendererbesitzer ergänzen die hier geplante Broker-/Helpergruppe; sie ersetzen deren Capturezulassung nicht.
