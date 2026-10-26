# R9-Ergänzungsplan: produktiver Login-Empfang und Bildbesitz

Stand: 2026-10-26. Abgeschlossene Ergänzungsplanung; keine Produktimplementierung und kein Laufzeitnachweis. Von Main in die kanonische Batchplanung übernommen; die temporäre Übergabedatei wurde nach vollständiger Übernahme entfernt.

## Auftrag und Ergebnisgrenze

Der Folgeauftrag ergänzt `docs/plaene/2026-10-24-batches/umsetzung.md`, Abschnitt „R9-Ergänzung: produktiver Login-Empfang und Bildbesitz“. Verbindliche Anschlussverträge sind `aufnahme-speicher.md` und `renderer-speicher.md` im selben Verzeichnis. Capture- und Rendererquellen werden parallel umgesetzt; die hier beschriebenen neuen APIs sind ein Ergänzungsvertrag, kein Beleg ihres bereits vorhandenen Betriebs.

Ziel ist eine gemeinsam gezählte, vor der jeweiligen Bildallokation gehaltene lokale Zusage vom produktiven Login-Empfang bis zum tatsächlichen letzten CPU-, JNI- und Rendererbesitzer. Erhalten bleiben Login ohne Screen-v2-Verhandlung, bisher zulässige Bildgeometrien einschließlich 3840×2160 und 4096×2304, verlustfreies RGB8-PNG, Maschinen-/Schlüssel-/Boot-/Benutzer-/Sitzungsbindung, Challenge und Sequenzprüfung, FrameToken, presentation_epoch, Fokus-/Sichtbarkeitsprüfung, Eingabefreigabe und sofortige Release-/Close-Anforderung. Screen-v2 mit W/24 und PacketBudget bleibt unverändert.

Die gemeinsame Autorität gilt je tatsächlich zusammengehöriger lokaler Prozessgruppe. Empfangender Desktopdaemon und Desktop-GUI haben jeweils ihren Prozesspool; ihre IPC-Kopien sind verschiedene physische Besitzer. Zwischen unabhängigem Daemon und GUI wird keine neue Systemautorität eingeführt. Broker und privilegierter Helper behalten die Parentgruppe des Aufnahmeplans. Auf Android befinden sich Daemonempfang, Viewer und JNI im selben Prozess: Sie benutzen denselben Pool und den bisherigen Javaheap-Höchstwert, keinen separaten JNI- oder Grafiktopf.

Wie in `frame_memory.rs` sind dies interne Zusagen, keine physische Reservation gegen fremde Prozesse oder künftigen Druck. Die bestehenden drei Viertel der beobachteten Desktopkapazität für Basisprozess, Codecs, Treiber und Konkurrenz bleiben erhalten. Gezählt werden nachweisbare Produktpuffer und bekannte logische Bild-/Uploadanforderungen. Nicht vollständig beobachtbare Android-, GPU-, Treiber- und Codec-Innenflächen werden ausdrücklich abgegrenzt.

## Erstplan und bestätigte Quellenbefunde

Erstplan: den Empfang vor seiner ersten Pixelallokation, den lokalen IPC-Übergang sowie Desktop- und Android-Endbesitz verfolgen; gleichzeitig lebende Ablagen bestimmen; Pool-/Plananschluss prüfen; APIs, Besitzerzahlen, Meilensteine und Abnahmesignale festlegen. Die zweite Recherche prüfte PNG-Zeilendecode, den echten spawn_blocking-Abbruch, Android-Bitmapallokation und RenderThread-Freigabe. Die zunächst vermutete Datei `LoginCanvas.kt` existiert nicht; die tatsächliche Klasse steht in der nachfreigegebenen `LoginInput.kt`.

| Grenze | Gelesene Quelle/Symbol | Befund |
|---|---|---|
| Netzwerk | `login/core/gateway_wire.rs::{receive_reply,receive_frame,decode_png}` | Der Wireinhalt ist unabhängiges RGB8-PNG. Nach dem Header entstehen PNG-Vec P, RGB-Vec 3N/4 und BGRA-Vec N. Größenprüfung ersetzt keine Zusage. |
| Decodeabbruch | derselbe `receive_reply`; `login/os/shared/gateway_client.rs::signed_operation_async` | Die gestartete blocking-Closure kann den aufrufenden Future überleben. Pixel und Zusage müssen in Closure und Ergebnis leben. |
| Daemonantwort | `login_ipc_session.rs`, `login_ipc_registry.rs`, `ipc_protocol_login.rs::write_reply` | `busy=false` folgt bereits dem Einreihen, nicht dem Writeabschluss. Nächster Decode und vorheriger Write können überlappen. Der Write leiht Pixel und erzeugt keinen vollständigen zweiten Bild-Vec. |
| Lokaler Empfang | `ipc_protocol_login.rs::read_pixels`, `login_ipc_client.rs::LoginViewer` | Vollständiger Empfangs-Vec plus vorübergehender Chunk bis 1 MiB, ohne Bildzusage. |
| Viewer | `login/os/shared/viewer.rs::{ViewerFrame,run,pull,take_frame,close}` | Read und Latest-Slot können überlappen. ViewerFrame trägt nur Token und Pixel. Ein der UI übergebener Frame lebt unabhängig vom Close weiter. |
| Desktop | `login_viewer_image.rs::show`; `screen_texture_cpu.rs::update` | Produktiver BGRA-Empfang wird ohne Vollbildkopie übernommen, aber ohne MemoryHold. Die RGBA-Konversion erzeugt eine zusätzliche Ablage. Nur die Fixture injiziert eine Lease. |
| CPU-Worker | `screen_texture_worker.rs`; `screen_scaling.rs` | Laufender Auftrag, vorgemerkter Auftrag und fertiges Ergebnis können sich überlappen. Quell-Arc-Aliase sind keine Kopien. Ausgabe, Gewichtstabellen und Filterzeilen sind zusätzliche Ablagen. |
| Renderer | `screen_render_upload.rs`, `screen_render_owner.rs`, `screen_render_wgpu.rs`, `screen_render_glow.rs` | Die übernommenen Retirementgrenzen können echte Login-Slices halten; produktiver Anker und vorherige Zulassung seiner Kopien fehlen. |
| Android | `LoginSession.kt::pictures`, `LoginInput.kt::LoginCanvas`, `login_jni.rs`, `login_view.rs` | Native BGRA-Pixel werden in vollständigen DirectBuffer, vollständiges IntArray und neues ARGB_8888-Bitmap kopiert. GC und StateFlow-Austausch bestätigen kein Canvasende. |
| Pool | `frame_memory.rs`, `ScreenFrameLimits.kt::workingBytes`, Android-`session_memory.rs` | Explizite CapacityCeiling trennt die Kohortenkapazität von der ersten kleinen Anfrage. Android bleibt facts=None. Der Javaheap-Höchstwert muss vor Loginopen Native erreichen. |

N ist checked `4×Breite×Höhe`. Das bestehende Loginlimit ist N≤37.748.736 Byte (36 MiB), beide Seiten≤4096, Pixelanzahl≤9.437.184. P ist die tatsächliche deklarierte PNG-Länge, 0<P≤36 MiB. P darf nicht aus N geschätzt werden: Das bestehende Protokoll begrenzt P unabhängig von der konkreten Geometrie. Die neun ViewerGuard-Präsentationsbelege sind Tokenmetadaten, keine neun Bildpuffer.

## Entscheidung und minimaler API-Vertrag

Login erhält keinen pauschalen zweiten W512-Schritt. Jede tatsächliche Ablage erhält vor ihrer Allokation einen exakten Slice aus dem gemeinsamen `frame_memory`-Pool. Eine Rollenbuchung begrenzt zusätzlich gleichzeitig lebende Ablagen. Die Worst-Case-Rechnung ist Begründung und Abnahmekriterium; sie wird nicht bei jeder Sitzung blind vorab belegt. Aliase teilen ihren Slice, Kopien benötigen eigene Slices.

Teilweise erfolgreiche `reserve`-/`reserve_with_ceiling`-Aufrufe genügen nicht: Ein kleinerer Betrag darf keinen größeren Bildbuilder öffnen. `reserve(None, N)` ist ebenfalls ungeeignet, weil der frühere Pfad die facts=None-Kohorte auf den ersten kleinen Betrag einfrieren kann. Der neue Einstieg verwendet die explizite Plattformkapazität und exakte Claims. Eine später kleinere Beobachtung beschränkt neue Claims, ohne Altbesitzer aus der Bilanz zu entfernen.

Der Ergänzungskern liegt in einem kleinen neuen `screen/core/image_admission.rs`; `frame_memory.rs` bleibt der einzige Rootzähler. Der vorhandene CaptureSlice erhält einen lokalen exakten Besitzerzweig. SessionAdmission und der Host-W-Vertrag aus dem Aufnahmeplan bleiben unverändert. Modulregistrierung und OS-Policyweitergabe erfolgen im bereits geplanten C1-Anschluss; `core/` ermittelt keine OS-Fakten.

```rust
// Sämtliche Typen/Methoden pub(crate); Signaturen sind Ergänzungsvertrag.
struct LocalImagePolicy { facts: Option<MemoryFacts>, ceiling: CapacityCeiling }
struct ImageAdmission { /* Rootpool, Identität, begrenztes Rollenbuch */ }
// Plattformbeobachtung im Rootpool; kein neuer Topf, keine Verkleinerung lebender Slices.
fn observe_capacity_ceiling(ceiling: CapacityCeiling) -> Result<()>;
enum ImageRole {
    EncodedInput, DecodedInput, ViewerInput, ColorSource,
    ScaledOutput, FilterScratch, UploadStaging, RenderSurface,
    AndroidBitmap, ImageMetadata,
}
struct ImageOwnerLimits { /* checked Kapazität/Stückzahl je Rolle */ }
fn admit_images(policy: LocalImagePolicy, limits: ImageOwnerLimits)
    -> Result<Arc<ImageAdmission>>;
impl ImageAdmission {
    fn try_claim(&self, role: ImageRole, bytes: u64) -> Result<CaptureSlice>;
}
enum ImageMemory {
    Contract(Arc<MemoryLease>),
    Slice(CaptureSlice),
}
type ImageHold = Arc<ImageMemory>;
struct LoginFrame { reply: LoginReply, image: Option<CapturedImage> }
struct ViewerFrame { token: FrameToken, image: CapturedImage }
```

`admit_images` erzeugt einen gebundenen Controller, keinen Vollbildpuffer. `try_claim` ist linearisiert: Rollenplatz, Rollenbytes, gemeinsame aktive Bytes und Rootbetrag werden unter festgelegter Lockreihenfolge zusammen geprüft und erworben; Fehler verändert keinen Wert. Drop nach tatsächlichem Ressourcenende vermindert sie einmal. Aliasclone vermehrt nur Referenzen. Größen-/Generationswechsel lassen alte Slices bis zum letzten Besitzer in der gemeinsamen Summe. Bytearithmetik ist checked; Umwandlung nach usize erfolgt erst an der konkreten Allokationsgrenze.

Die Rollenlimits sind keine unabhängigen Geldtöpfe. Zwei gleichzeitig aktive Phasen zählen ihre Summe, selbst wenn eine Sollrechnung sie disjunkt vorsieht. Noch gehaltene alte Ablagen werden nicht aus künftig freien Phasen finanziert. Login-, Screen-, Snapshot- und Capturebesitzer im selben Prozess konkurrieren im einen Rootpool.

CapturedImage, CaptureImageBuilder, AdmittedBytes und feste Pixel-/Zeilenslices aus dem Aufnahmeplan werden wiederverwendet. Slice vor Allokation; kein nachträgliches `from_vec(vec, lease)`, kein frei ersetzbares `&mut BgraImage`, kein herausgegebenes `&mut Vec`. Kontrollantworten tragen None. Neue Bildticket-/Regionsmetadaten werden vor Containeranlage gebucht. Die bestehenden begrenzten Kontrollnachrichten und unveränderten Codec-Innenzustände bleiben getrennt von dieser Bildbilanz.

Ein Lebensdaueranker erlaubt keine zusätzliche Kopie. Die Renderer-API erhält neben ImageHold einen getypten Allokationszugang: vorhandener Screen-Vertragsanteil oder ImageAdmission für Login. Screen nutzt sein bereits gezähltes W; Login beansprucht für RGBA, Ausgabe, Scratch, Upload und Textur konkrete Slices. Ein Raw-N-Slice finanziert nicht gleichzeitig eine zusätzliche RGBA-N-Kopie.

## Native Empfangskette und Spitzen

### PNG im empfangenden Daemon

Nach begrenztem Kontrollheader und bisheriger Schema-/Geometrieprüfung werden P, N und `3×Breite` vor dem Pixelread zugelassen. `receive_frame` erhält eine gebuchte feste Ablage; es allokiert PNG nicht vorher. IHDR-, Farbtiefen-, Farbtyp-, Noninterlace-, Chunk- und IEND-Prüfungen bleiben erhalten, ebenso die Beschränkung auf IHDR/IDAT/IEND ohne Animations-/Profil-/Textchunks.

`decode_png` verwendet png 0.18.1 Reader::read_row mit einer gebuchten RGB-Zeile und schreibt jede Zeile unmittelbar in die feste BGRA-Zeile. Alpha bleibt 255. finish() und Gesamtdimensionsprüfung bleiben erhalten. Die Produktspitze wird **P + N + 3×Breite**, statt P + 3N/4 + N. Die unveränderten Library-Innenpuffer verbleiben in der Codecaußenreserve; deren Limits-Konstante ist kein gezählter Produktpuffer. [png 0.18.1 Reader](https://docs.rs/png/0.18.1/png/struct.Reader.html).

spawn_blocking übernimmt PNG, Builder-/Zeilenzusage, Cancelzustand und einen Clone des bestehenden Daemon-Laufzeitbesitzers. Das Ergebnis trägt CapturedImage. Ein gedroppter JoinHandle, Timeout oder geschlossener Socket gibt diese Slices nicht frei. Cancellation wird zwischen Zeilen geprüft; bis zum tatsächlichen Closureende bleiben alle Besitzer gezählt. PNG und Zeile werden vor ihren Slices zerstört. Die Registry darf nicht leer erscheinen, solange die Closure beziehungsweise ein unzugestelltes Ergebnis lebt. [Tokio 1.52.3 blocking.rs](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/task/blocking.rs).

### Daemonwrite und lokaler Client

Die Antwort erhält einen Write-Abschlussbesitzer. Der nächste bildproduzierende Vorgang beginnt erst nach abgeschlossenem beziehungsweise endgültig verworfenem vorherigem Write und Freigabe seines Bildbesitzers. Der bisherige busy-Zeitpunkt genügt nicht. Kontroll-, Release- und Close-Signale dürfen durch die Bildschranke nicht hinter einem wartenden Produzenten blockieren. Begonnene signierte Operationen werden bei Fehlern nicht automatisch wiederholt.

Dieser Empfangs-/Weiterreichpfad hält damit höchstens ein BGRA-Ergebnis N. Sein Write folgt auf PNG/Zeile; die Spitze bleibt P + N + 3×Breite. Das wird durch die Abschlussregel erzwungen, nicht aus Kanalgröße eins abgeleitet. `gateway_wire::prepare_reply` auf der Senderseite übernimmt den dort bereits durch den Aufnahmeplan finanzierten Parentbild-/Encodebesitz in seine Closure; ein nackter Vec darf ihn nicht wieder verlieren. Dafür wird keine neue Empfänger-W-Reserve beim Sender eingeführt.

`ipc_protocol_login::read_pixels` prüft bestehende Chunkheader und liest unmittelbar in zugelassene feste Zielslices. Der vorübergehende 1-MiB-Chunk-Vec entfällt. Wireformat, Chunkgrenze und Fehlerfälle bleiben unverändert. LoginViewer::open/command erhalten Admission vor der ersten Pixelantwort. Derselbe Client ist auch für CLI-/MCP-Verbraucher gebucht; der vorhandene Einstieg kann einen Defaultprofil-Controller erzeugen, ohne dass Caller eine Dummy-Lease injizieren.

Der Viewer besitzt **zwei Rawplätze**: Read sowie Latest-/Consumerbesitz. take_frame und JNI-Abholung übertragen Besitz, sie buchen ihn nicht frei. Vor weiterem Capture muss ein tatsächlicher Platz frei sein. Noch ungezeigtes Latest darf verworfen werden; ein extern übernommenes Bild bleibt gezählt. Ein voller Platz verschiebt Capture oder fasst einen nicht übergebenen Kandidaten zusammen. Release und Close laufen weiter. Ungedeckte neue Bytes scheitern vor Allokation und aktivieren keine Eingabe für ein ungezeigtes Bild.

Zwischen Desktopdaemon und GUI können N Senderpixel und 2N Empfängerpixel in getrennten Pools leben; ein Sockettoken macht sie nicht zu einer Ablage. Auf Android begrenzt dieselbe Rootsumme beide Teilpfade.

## Desktop: vorherige Kopiezulassung und letzter Besitzer

D ist checked `4×Zielbreite×Zielhöhe` des tatsächlich zugelassenen CPU-/Rendererziels. Bestehende Limitprüfung bleibt erhalten. Die folgende Obergrenze ist zugleich eine durchgesetzte Platzschranke:

| Rolle | Gleichzeitig höchstens | Begründung |
|---|---:|---|
| BGRA-Empfang | 2N | IPC-Read plus Latest-/UI-Borrow. |
| RGBA-Quellablagen | 3N | Aktuelle Quelle, vom laufenden Worker oder alten nativen Präsentationspaket gehaltene Vorgängerquelle, neue Konversion. Pending-Work derselben Quelle ist Alias. Sämtliche Überschneidungen müssen in diesen Plätzen bleiben. |
| Skalierte Ausgaben | 3D | Laufende Ausgabe, fertiges/übergebenes Ergebnis, bestehender Präsentationskandidat/-besitzer. UI-Übernahme und Retirement zählen mit. |
| Angeforderte GPU-Texturen | 2D | Bestehender/auslaufender und neuer Besitzer am gehaltenen logischen Loginplatz. Der Rendererplan kann weiter serialisieren. |
| Uploadnutzlast | D | Höchstens ein eigener ausstehender Upload; Zulassung vor write_texture beziehungsweise GL-Upload. |
| Filterscratch | S | Ein Worker; exakte Gewichte und begrenzte Zeilen, siehe unten. |

Die konservative Produktobergrenze einer zugelassenen Desktoppipeline beträgt **5N + 6D + S**, zuzüglich gebuchter neuer Bildmetadaten. Mehrere tatsächlich unabhängige Viewer-/Decoderpipelines addieren ihre Bestände im Rootpool; diese Formel ist keine globale Zusage für beliebig viele Sitzungen. Beim Sessionwechsel am selben Pane bleiben die Quell-/Ausgabe-/Renderrollen des logischen Präsentationsbesitzers bestehen. Ein noch laufender alter Transport-/Decodercontroller bleibt zusätzlich mit seinen tatsächlichen Slices gezählt, bis sein Ende bestätigt ist. Bei N=D=36 MiB: **396 MiB + S**. Bei N=D für 3840×2160: **364.953.600 Byte + S**. Native Darstellung beansprucht weder skalierte Ausgabe noch Filterscratch; tatsächliche Buchung liegt entsprechend darunter. Diese Beispiele verhindern eine methodisch leere Legacyzulassung. Sie garantieren keine Zulassung bei bereits belegtem Pool oder kleiner tatsächlicher Plattformkapazität.

Fertiges Workergebnis, UI-Übernahme, alter Kandidat und laufende Neuberechnung dürfen nicht ungeprüft vier Ausgaben erzeugen. Ein alter unübernommener Kandidat endet vor Ersatz, oder Neuberechnung wird verschoben. Die GUI wartet nicht auf Workerjoin oder Ressourcenfreigabe. Ein altes gültiges Bild bleibt sichtbar, soweit seine Guardbindung gültig ist.

S folgt aus `screen_scaling.rs`: horizontale Tapanzahl zunächst ohne Heapallokation zählen; bei Reduktion höchstens `Quellbreite + 2×Zielbreite`, bei Vergrößerung höchstens `4×Zielbreite`. Gebucht werden die tatsächliche checked Anzahl, Zielbreite Tabellenköpfe, maximal nötige vertikale Gewichtsliste und höchstens sechs Zeilen zu `Zielbreite×size_of::<[f32;3]>()`: vier Cachezeilen, Ergebniszeile, vorübergehende horizontale Zeile. HashMap wird durch vier explizite Cacheeinträge ersetzt, damit keine unbelegte Containerwachstumsschätzung nötig ist. size_of und tatsächliche Kapazitäten bestimmen S. Area-/Catmull-Rom-Filter, Randbehandlung und Cancelprüfung bleiben identisch.

LoginImage::show übernimmt einen gebuchten ViewerFrame. Vor ColorImage-Konversion wird ein ColorSource-Slice erworben. CpuTexture, Work, Finished, PreparedImage und Paintclosure tragen das passende ImageHold. Ein Egui-Arc<ColorImage> darf nicht ohne einen bis zum letzten Alias gehaltenen Anker entkommen. Der ColorSource-/ScaledOutput-Controller gehört deshalb zum stabilen logischen CpuTexture-/Paneplatz, nicht zur austauschbaren aktuellen Session; Retirement alter Generationen beansprucht weiterhin dessen Rollen. Raw und RGBA haben getrennte Slices.

Vor GPU-Erzeugung werden RenderSurface und UploadStaging erworben. Der GPUbesitzer übernimmt den Texturslice; das Uploadticket hält Quelle und Uploadslice bis zur Fertigmeldung. Uploadende beendet keinen später noch gezeichneten Texturbesitz. WGPU verwendet den Submission-/Retirementweg, Glow den eigenen Painter-/Fence-/Shutdownweg des übernommenen Rendererplans. Der getypte Sliceanker durchläuft denselben Lebensdauerweg wie die Vertragslease. Ein Weak auf Sessionstatus oder eine Lease einer fremden Quelle ersetzt ihn nicht.

Close/Reset/Neuverbinden invalidiert Platz und Guard, entfernt Pending-/Latestbesitz und fordert Retirement an. Laufender Filter behält Input, Output und Scratch bis zum tatsächlichen Ende; Renderer halten ihre Anker bis Freigabe. Der logische Paneplatz bleibt über Neuverbindungen stabil. Eine beendet angezeigte Statussitzung behält keinen funktionslosen Bildanker.

## Android: gebuchte Bitmap ohne Vollbildzwischenkopien

### Vorherige Zulassung und JNI-Übergabe

LoginApi.open ergänzt den bestehenden Core.request-Aufruf login.viewOpen um den existierenden Wert ScreenFrameLimits.workingBytes(). Der tatsächliche Empfänger ist mobile/os/shared/domains/login_view.rs::handle; er übernimmt den Höchstwert vor ViewerSession::start. Native verwendet facts=None und CapacityCeiling::bounded(heap_offer). Der ergänzende Poolanschluss beobachtet diesen Prozesshöchstwert atomar, bevor der Viewer seinen lokalen IPC-Open auslöst. Daemon-Admission und Viewer-Admission lesen denselben beobachteten Höchstwert; es genügt nicht, nur dem Viewer eine kleinere private Policy zu geben. Neue Claims aller betroffenen Poolpfade respektieren den beobachteten Betrag abzüglich der gemeinsam gehaltenen Bytes. Eine kleinere Beobachtung beendet keine Altbesitzer und vergrößert keine aktive Kohorte; erst eine neue leere Kohorte kann mit einer neuen Plattformbeobachtung beginnen. Ein kleiner Heapwert wird weder durch Desktopfakten noch durch einen zweiten Pool vergrößert. Dieser portable observe_capacity_ceiling-Anschluss ist Bestandteil von L1; der Androidaufruf liegt in L7. NativeBridge wird für die separate Bitmap-/Release-/Close-JNI-Grenze angepasst.

Der produktive Vollbild-DirectBuffer-/IntArray-Umweg wird durch besessene Bitmapabholung ersetzt. Die alten internen Vollbildexports loginPull/login_pull werden an allen gelesenen API-/JNI-/Mobile-Exportgrenzen gemeinsam ersetzt; kein unbudgetierter paralleler Vollbildrückweg bleibt. FrameToken und bisherige Headerprüfungen gehen in die kleine typisierte Rückgabe ein. Das interne JNI-Format verändert keinen Netzwerk-Wirevertrag.

1. Native nimmt einen bereits gebuchten ViewerFrame in ein gebundenes BitmapFillTicket auf. Es prüft Session-ID, Generation, FrameToken, Geometrie und einen von höchstens zwei Bitmapplätzen. Vor Bitmap.createBitmap hält das Ticket AndroidBitmap-Slice N und ImageMetadata-Zusage.
2. JNI erzeugt ein veränderbares Softwarebitmap ARGB_8888 und hält während Create, Registrierung und Fill einen starken Besitzer. Java-Exception oder Nativefehler beendet den Kandidaten geordnet; der Slice endet nach dem Pixelobjekt.
3. AndroidBitmap_getInfo prüft Dimensionen, RGBA_8888-Format und tatsächlichen Stride. AndroidBitmap_lockPixels liefert das feste Ziel. Native schreibt BGRA zeilenweise als RGBA unter Beachtung dieses Strides. Kein vollständiges IntArray und kein Vollbild-DirectBuffer entsteht. Jeder erfolgreiche Lock endet auf Erfolgs-/Fehler-/Abbruchweg mit Unlock; kein Await und kein Rohpointerescape während des Locks. [Android NDK Bitmap-API](https://developer.android.com/ndk/reference/group/bitmap).
4. Nach Fill ist das Bild für Consumer unveränderlich. Native hält GlobalRef und Slice in einem LoginBitmapOwner. Kotlin erhält einen opaken LoginPicture-Besitzer mit Token/Generation; das frei zugängliche Bitmapfeld entfällt. Nur der Rendereradapter leiht den Inhalt für einen gebundenen Draw aus.
5. Nicht veröffentlichter oder durch Close überholter Kandidat geht direkt ins Retirement. Veröffentlichte Kandidaten bleiben bis zum tatsächlichen Rendererende gehalten. Erst dann erfolgt Bitmap.recycle(), danach Ende von GlobalRef und Slice. GC ist kein Zulassungs-Freigabesignal.

Der JNI-Vertrag trennt Sitzung und Präsentationsplatz: Ein nativer ImageOwnerHandle mit Arc<ImageAdmission> und zwei Bitmapplätzen wird einmal für die Lebensdauer des Loginpanes erzeugt. `take_bitmap(session_id, image_owner_handle, wait)` erzeugt das gebundene Fillticket; `retire_bitmap(image_owner_handle, picture_id, generation)` wird ausschließlich nach tatsächlichem Renderer-/Softwaredrawende aufgerufen. `close_image_owner` sperrt neue Tickets, entfernt aber keine lebenden Bitmaps. Ungültige/fremde Handles und Generationen scheitern ohne Zugriff. Der JNI-Besitzer hält GlobalRefs, bis der letzte Renderborrow beendet und recycle abgeschlossen ist. Diese Namen sind der minimale interne Zielvertrag; sie werden an NativeBridge, LoginApi, JNI und Mobile-Reexports gemeinsam umgesetzt.

LoginViewScreen hält diesen Präsentationsbesitzer außerhalb von `key(session)` und außerhalb des attempt-abhängigen LoginSession-Objekts. Damit zählen auslaufende Bitmaps früherer Verbindungen weiterhin in denselben zwei Plätzen. Compose-Disposal übergibt ihn einem begrenzten Retirementbesitzer, statt die Buchung durch ein neues default-Objekt zu ersetzen. Ein Session- oder Surfacewechsel verleiht keinen zusätzlichen Bitmapplatz.

Der Android-only Anschluss kann in neuem `native/android-bridge/src/login_bitmap_jni.rs` die öffentliche jnigraphics-ABI verwenden. `native/android-bridge/src/lib.rs` registriert ihn. Der vorhandene Exportpfad lautet `mobile/os/shared/domains/mod.rs` → `mobile/mod.rs`. Eine allgemeine neue Java-/C++-Bridge ist nicht erforderlich.

### Zwei Bitmapplätze und tatsächliche Rechnung

Es gibt höchstens zwei Bitmap-Pixelablagen je Login-Präsentationsplatz: aktuelles Bitmap sowie Kandidat beziehungsweise auslaufender Vorgänger. Filling, Pending, Recorded und Retiring teilen diese beiden Plätze. Ein dritter Frame ersetzt nur einen ungezeichneten Pending-Kandidaten nach dessen wirklicher Freigabe oder wartet. StateFlow darf keine zusätzliche Bildschlange bilden. Native-Read und Native-Latest/Borrow teilen die zwei Rawplätze des Viewerabschnitts.

Der Android-Viewer hält höchstens **2N Raw + 2N Bitmap = 4N** angeforderte Produktpixel. Mit empfangendem Daemon im selben Prozess lautet die CPU-Pixel-/PNG-Spitze **P + 5N + 3×Breite**, zuzüglich Ticketmetadaten. Bei P=N=36 MiB: **216 MiB + 3×Breite**, keine zwei unabhängigen zulässigen Beträge. Diese Spitze gilt für eine laufende Empfangspipeline und ihren Präsentationsplatz. Ein nach Abbruch noch laufender Vorgängerdecoder und weitere unabhängige Logins werden zusätzlich nach ihren tatsächlich gehaltenen Slices im selben Rootpool gezählt; das Schließen eines Sessioneintrags setzt ihre Buchung nicht zurück. Zusätzlich wird D_live vor Aktivierung eigener Renderziele aus demselben Pool gebucht: D_live ist die Summe der checked RGBA-Zielnutzlasten sämtlicher lebender eigener Renderziele. Ein logischer Platz erlaubt höchstens ein aktuelles und ein auslaufendes Ziel; ein weiterer Attach wartet. Bei Größenwechsel zählen alte und neue Bitmaps/Renderziele gleichzeitig oder Altbesitz endet vor Neuanlage. D_live ist keine Behauptung über die interne Bufferanzahl eines Surface.

Diese Bilanz zählt Produktpixel. Android liefert getAllocationByteCount und NDK-Stride erst am vorhandenen Objekt. Für ein frisch erzeugtes Softwarebitmap verwendet der geprüfte AOSP-Pfad minimale RowBytes und RowBytes×Höhe. Das belegt die N-Nutzlast des gewählten ARGB_8888-Pfads, keine universelle Obergrenze für OEM-Heapheader, Rowpadding und Grafikverwaltung. Tatsächliche Stride/AllocationByteCount werden diagnostiziert; nachträgliche Beobachtung wird nicht als vorheriger Grantbeweis ausgegeben. Unbekannte native Zusatzbytes verbleiben ausdrücklich in der Android-/Treiberaußenreserve; keine Pitchkonstante wird erfunden. Der N-Slice bleibt bis zum echten Bitmapende gehalten. [Bitmap-API](https://developer.android.com/reference/android/graphics/Bitmap), [AOSP Bitmap-Erzeugung](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/libs/hwui/jni/Bitmap.cpp), [AOSP Heapbitmap-Allokation](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/libs/hwui/hwui/Bitmap.cpp).

### Eindeutiger Canvas-/RenderThread-Endbesitzer

Die bestehende View zeichnet Bitmapreferenzen in Framework-Displaylisten. StateFlow-Austausch, Coroutine-Cancel, View.post, Eintritt in onDetachedFromWindow und FrameCommitCallback alleine belegen deren Ende nicht. Die Bitmapdokumentation verlangt vor Wiederverwendung eine tatsächlich abgeflossene Zeichnung ohne alte Bitmapreferenz. Ausbleibender nächster Draw nach Close darf weder vorzeitige Freigabe noch unbegrenzte Retirementlisten erzeugen.

Der ausgewählte Anschluss ist ein eigener kleiner LoginBitmapRenderer mit öffentlichem HardwareRenderer und eigenem RenderNode. Projekt-minSdk ist 30; diese APIs bestehen seit API 29. LoginCanvas behält Eingabe, IME, Fokus, Rectberechnung und Präsentationsbindung. Sein hardwarebeschleunigter Inhalt wird über eine TextureView-Surface eingebunden; Clipping und Transformation bleiben innerhalb der Viewhierarchie. Der echte Software-Canvas-Pfad bleibt als separater Adapter erhalten, da TextureView im Softwarefenster nichts zeichnet. [TextureView](https://developer.android.com/reference/android/view/TextureView), [HardwareRenderer](https://developer.android.com/reference/android/graphics/HardwareRenderer).

Ein serialisierter Renderbesitzer zeichnet ausschließlich den gebundenen Frame. Vor Ersatz entfernt clearContent() die alten Displaylisten; danach wird derselbe RenderNode mit dem neuen Bitmap neu aufgezeichnet, mit setContentRoot erneut gebunden und gezeichnet. Nicht mehr benötigte eigene vorgemerkte Displaylisten werden mit RenderNode.discardDisplayList verworfen; die RenderNode-Referenz wird erst nach dem geordneten RenderThreadabbau entfernt. Voriger Surfaceinhalt kann bis zum neuen Commit sichtbar bleiben. Erst nach bestätigter neuer Zeichnung, passender Session-/Rendergeneration, unverändertem presentation_epoch und weiterhin vollständiger Sichtbarkeit wird presented für den zugehörigen FrameToken gemeldet. Reines Record/Upload genügt nicht. Verlorene Surface, SYNC_FRAME_DROPPED, SYNC_CONTEXT_IS_STOPPED oder veralteter Callback aktiviert keine Eingabe. Renderabschluss und TextureView-Inhaltsaktualisierung werden dem identischen Frame zugeordnet; Serialisierung verhindert mehrdeutige Callbacks mehrerer gleichzeitig eingereichter Loginbilder.

Beim Ende sperrt UI/Session zuerst neue Draws und Eingaben, erhöht die Generation und sendet Release/Close über den vorhandenen schnellen Weg. Der Renderbesitzer führt danach geordnet stop(), clearContent() und destroy() aus. Die öffentliche API zerstört Inhaltsdisplaylisten und Rendererressourcen. Der geprüfte AOSP-Weg nutzt RenderProxy::destroyHardwareResources/destroy mit queue().runSync; RenderNode::destroyHardwareResources löscht aktive und vorgemerkte Displaylisten. Nach bestätigtem Ende und ohne aktiven Softwaredraw dürfen Bitmaps recycelt und Slices freigegeben werden. Ein verlorener Callback oder Timeout ist kein Ersatzbeleg. [HardwareRenderer-Implementierung](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/graphics/java/android/graphics/HardwareRenderer.java), [RenderProxy](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/libs/hwui/renderthread/RenderProxy.cpp), [RenderNode](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/libs/hwui/RenderNode.cpp).

Close wartet im UI nicht auf PNG oder Grafikcleanup. Der Renderbesitzer hält Bitmap, Slice und Surface bis Abschluss. Bei TextureView-Detach übernimmt er mit onSurfaceTextureDestroyed(...)=false die SurfaceTexture-Freigabeverantwortung. Nach beendetem Renderer gibt er Surface und SurfaceTexture frei. Ein neuer Attach benötigt einen weiteren vorher zugelassenen Zielbesitzer oder wartet auf Retirement. Unbestätigtes Retirement bleibt belegt; es entsteht keine unbegrenzte Neuattachliste. Diese eigene Releaseverantwortung ist Teil der öffentlichen API. [SurfaceTextureListener](https://developer.android.com/reference/android/view/TextureView.SurfaceTextureListener).

Der Softwareadapter zeichnet nur auf einem tatsächlichen nichthardwarebeschleunigten Canvas. Drawborrow hält den Bitmapbesitzer bis zur Rückkehr des synchronen Rasteraufrufs; Viewzustand hält ihn bis Austausch/Detach. Er erzeugt keine eigene zusätzliche Vollbildsoftwarelayer. Eine erzwungene LAYER_TYPE_SOFTWARE-Abkürzung ersetzt den Hardware-Endbesitz nicht. Adapterwechsel beendet Altbesitz zuerst oder finanziert beide tatsächlichen Zustände.

Interne HardwareRenderer-/TextureView-BufferQueue, Treibercaches, Uploadkopien, Compositorflächen und physische GPU-Retirementdauer sind nicht als vollständige Bytezahl verfügbar. Eine Surface bedeutet nicht genau einen physischen Buffer. Eine typische Triple-Buffering-Zahl wäre kein Gesamtbeleg. Gezählt und gehalten werden explizite Renderzielnutzlast und sämtliche eigenen Bitmaps. Displaylistenabbau beweist das Ende produktseitiger Bitmapnutzung; er wird nicht als allgemeiner physischer GPU-Fence für unbekannte Treiberkopien bezeichnet. [FrameRenderRequest](https://developer.android.com/reference/android/graphics/HardwareRenderer.FrameRenderRequest).

## Sicherheits- und Abbruchverträge

- Speicher folgt geprüften Headern, authentisiert aber keine Maschine und keinen Benutzer. Maschinenpin, Benutzerrechte, Challengefolge, Boot-/Binding-/Policy-Kontext und Route-Pause-Prüfung bleiben unverändert.
- FrameToken bleibt an Kontext, Frame-ID und Geometrie gebunden. Er wird weder zum Pixelhash umgedeutet noch durch Memoryhandle ersetzt. Bildtickets haben zusätzlich lokale Session-/Generationsbindung, verleihen aber keine Loginrechte.
- Nicht dargestellte Frames melden kein presented. Größenwechsel, Fokusverlust, unsichtbare Fläche, neue presentation_epoch und Close entwerten Eingabeautorisierung unverändert. Die neun Guardbelege bleiben erhalten.
- Bildrückstau hält keine gedrückte Eingabe fest. releaseFast, Socketshutdown, Cancelzustand und ReleaseInputs laufen unabhängig von Pixelplätzen. Begonnene signierte Eingaben werden bei Fehlern nicht wiederholt.
- Falsche PNG-/IPC-Längen, falsche JNI-Tickets, veraltete Rendercallbacks und verspätete Decoderresultate werden nicht veröffentlicht. Lebende Besitzer halten ihre Zusage bis zur tatsächlichen Zerstörung.

## Meilensteine und konkrete Änderungsdateien

Alle Erwartungen werden in der einen abschließenden entfernten Tasksuite des aktiven Batches gesammelt. Dieser Plan führt keine Suite, keinen Build und keinen Formatter aus. Neue/wesentlich bearbeitete Rustmodule bleiben unter 500 Zeilen/50 KiB; Besitzermanagement wird in die genannten kleinen Module ausgelagert.

| Schritt | Änderungsdateien/Anschluss | Abhängigkeit und erwartetes Ergebnis |
|---|---|---|
| L1 Exakte lokale Claims | `native/src/screen/core/frame_memory.rs`; neu `native/src/screen/core/image_admission.rs`; CaptureSlice-Anschluss im durch C1 bereitgestellten `native/src/screen/core/capture_admission.rs`; C1-Modul-/OS-Policyexport | Nach C1. Atomare Rollen-/Rootbuchung ohne Teilzulassung/zweiten Pool; explizites facts=None-Limit, W/24 unverändert. |
| L2 PNG-Empfang | `native/src/login/core/gateway_wire.rs`; neu `native/src/login/core/image_receive.rs`; `native/src/login/os/shared/gateway_client.rs` | Nach L1. PNG vor Read gebucht, Zeilendecode, immutable Ergebnis, Closurebesitz über Abbruch; Sender behält seinen vorhandenen Capture-/Encodeanker. |
| L3 Daemon-/IPC-Übergabe | `native/src/daemon/os/shared/ipc_protocol_login.rs`, `login_ipc_session.rs`, `login_ipc_registry.rs`, `login_ipc.rs`, `login_ipc_client.rs` im selben Verzeichnis | Nach L2. Keine Chunkkopie, ein Daemonbild bis Writeende, Registry umfasst Decoder, tatsächlicher Client liefert gebuchte Bilder. |
| L4 Viewer und Kontrolle | `native/src/login/os/shared/viewer.rs`; neu `native/src/login/os/shared/viewer_images.rs`; Guardsemantik in `native/src/login/core/viewer_guard.rs` unverändert | Nach L3. Zwei Rawplätze, Besitzübernahme, Release/Close unabhängig von Bildrückstau. |
| L5 Desktop-CPU | `native/src/app/core/login_viewer_image.rs`, `screen_texture_cpu.rs`, `screen_texture_worker.rs`, `screen_scaling.rs` im selben Verzeichnis; neu dort `screen_image_memory.rs` | Nach L1/L4 und Renderergrundanschluss. Produktiver Slice, Konversion/Ausgabe/Scratch vor Allokation, durchgesetzte Quell-/Ausgabeplätze. |
| L6 Desktop-GPU | `native/src/app/core/screen_render_upload.rs`, `screen_render_cpu.rs`, `screen_render_owner.rs`, `screen_render_wgpu.rs`, `screen_render_glow.rs` im selben Verzeichnis | Nach L5/Rendererplan. Vertragslease und Slice durchlaufen echten Retirementweg; Textur/Upload mit eigenen Claims. Keine zweite Rendererimplementierung. |
| L7 Android-JNI | `native/android-bridge/src/login_jni.rs`; neu dort `login_bitmap_jni.rs`; dort `lib.rs`; `native/src/mobile/os/shared/domains/login_view.rs`, dort `mod.rs`; `native/src/mobile/mod.rs`; `android/app/src/main/java/app/smartexplorer/android/api/LoginApi.kt`; `android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt` | Nach L1/L4. Heapangebot vor Open, gemeinsamer Pool, zwei Bitmaptickets, direkter NDK-Fill ohne Vollbild-DirectBuffer/IntArray. |
| L8 Android-Renderer | `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginSession.kt`, `LoginInput.kt`, `LoginViewScreen.kt` im selben Verzeichnis; neu dort `LoginBitmapOwner.kt`, `LoginBitmapRenderer.kt` | Nach L7. Opaquer Owner, öffentlicher HardwareRenderer-Endbesitz, Softwareadapter, bestehende Sichtbarkeits-/Fokus-/IME-/Tokenbindung. Close sofort anforderbar, Freigabe erst nach Ende. |

Main integriert L1 mit dem parallel arbeitenden Kernagenten; L5/L6 folgen nach Übergabe des Rendereragenten. Dadurch entstehen keine konkurrierenden Kerntypen. Die plattformübergreifende Policyweitergabe verwendet den C1-OS-Anschluss; keine OS-Erkennung wird in das neue portable Modul verlagert.

## Abnahmesignale für die eine entfernte Tasksuite

1. Produktiver Einstieg: Ein begrenzter lokaler Gateway-/IPC-Gegenpart speist tatsächlichen LoginViewer, ViewerSession und LoginImage::show. Keine Fixture injiziert MemoryHold. Rootbuchung besteht vor Pixelallokation und endet beim letzten Bild-/Rendererbesitzer.
2. Vorallokationsfehler: Erzwungener fehlender Betrag oder voller Rollenplatz an PNG, BGRA, RGBA, Scratch, Ausgabe, GPU-Upload und Bitmap verursacht dort keine Allokation. Teilgrant genügt nie. Altbild behält seine Zusage; Release/Close bleiben ausführbar.
3. Vertragserhalt: Kleine Bilder, 3840×2160, 4096×2304 und zulässige schmale/hohe Bilder funktionieren bei ausreichender Kapazität. P>N wird nach tatsächlichem P zugelassen. Bildlimit und kleiner Androidheap bleiben unverändert. Parallele Screen-/Loginbesitzer addieren sich im lokalen Pool.
4. Decode/Abbruch: Falsche IHDR-Geometrie, Farbtyp, Interlace, Zusatzchunks, Trunkierung und fehlerhaftes Ende werden weiterhin abgewiesen. Angehaltener blocking-Decoder behält Besitz nach Future-/Socket-/Sessionabbruch. Registrydrain wartet auf tatsächliches Closure-/Ergebnisende.
5. Transport: Langsamer Writer und früh sendender Client erzeugen kein zweites Daemonbild. Chunks werden in bestehende Ziele gelesen. Gehaltenes Viewerbild verhindert dritten Rawclaim; Guardtokenliste erzeugt keine Pixelkopien.
6. Desktopende: Verzögerter CPU-Worker, unübernommenes Ergebnis, verworfener Paintoutput, später WGPU-Abschluss, Glow-Retirement und Close ohne neues Bild halten passende Slices. Wirkliches Ende gibt Rollen frei. Grenzen gelten auch bei Resize/Neuverbinden.
7. Androidbesitz: JNI-Fehler bei Create, Lock, Fill, Unlock und Publish hinterlassen keinen Owner und geben keinen verwendeten Owner vorzeitig frei. Tatsächlicher Stride wird korrekt verwendet. StateFlow-Wechsel alleine vermindert keine Bitmapbuchung. Dritter Kandidat, Resize und verspäteter JNI-Return überschreiten zwei Plätze nicht.
8. Androidende/Eingabe: Hardware-/Softwarepfad bewahren Filter, Rect, Clipping, IME und Pointerkoordinaten. Close/Detach ohne Folge-Draw, verlorene Surface, fehlender Commitcallback und Neuattach halten Retirement bis clearContent/destroy beziehungsweise letztem Softwaredraw. presented folgt nur tatsächlichem Frame mit aktueller Generation/epoch. Späte Callbacks aktivieren nach Fokusverlust/Close keine Eingabe/SAS.
9. Bindung: Falsche Maschine/Zielbenutzer, alte Challenge, falscher FrameToken, Desktopkontextwechsel und alte presentation_epoch werden weiter abgewiesen. Speichertickets ändern keine Rechte; Rückstau wiederholt keine signierte Operation.

Die Suite verwendet vorhandene Entwicklungsbinaries und die vorgesehene entfernte Runnerstruktur. Ihre konkrete gemeinsame Datei/Runnerregistrierung gehört Main und liegt außerhalb dieser Lese-/Schreibfläche. Diese Liste autorisiert keine lokale Ausführung und keine zusätzlichen Testkampagnen.

## Verbleibende Integrationsbelege und verworfene Ansätze

Es fehlt keine Nutzerentscheidung. Vor einer Implementierungsbehauptung fehlen die Produkt-/Typbelege für L1–L8: lokaler Slicezweig am C1-Kern, gemeinsame Android-Policy vor Daemonempfang, Registry-Closurebesitzer, Write-Abschlussquittung, echte Renderer-Slices und Android-JNI-/RenderThread-Endbesitzer. Dies sind konkrete Umsetzungsschritte. Der aktuelle C1-Modul-/Policyexport wird bei Integration mit dem Kernagenten abgeglichen; dieser Plan benennt die benötigte API und Grenze.

Nicht zugesagt sind vollständiges Prozess-RSS, systemweite Daemon-/GUI-Buchung, vorab bekannte OEM-Bitmapgesamtallokation oder vollständige Android-BufferQueue-/Treiberbilanz. Dafür fehlen öffentliche Vorabgrößen-/Endbesitzsignale; bestehende Außenreserven bleiben die Grenze. Stride/AllocationByteCount werden beobachtet, nicht als erfundene Vorallokationsbelege verwendet.

Verworfen wurden Dummy-W512 im UI, pauschaler zweiter Vollvertrag, reine Größenprüfung, nachträgliche Leasebindung, vollständiger RGB-Zwischenframe trotz Zeilen-API, GC-/StateFlow-Freigabe, FrameCommitCallback als allgemeiner GPU-Fence, View.post/Detach-Eintritt als Displaylistenbeweis, private ViewRoot-APIs und unbegrenztes Retirement. Der gewählte öffentliche HardwareRendererbesitz schließt die letzte produktseitige Canvasreferenz unabhängig von einem zufälligen späteren Draw.

## Quellenstand und Bericht der Planarbeit

Externe Primärquellen wurden am 2026-10-26 geprüft. Rustabhängigkeiten wurden gegen native/Cargo.lock gelesen: png 0.18.1 und Tokio 1.52.3. Androidquellen sind öffentliche API-Dokumentation und AOSP android-16.0.0_r1; der AOSP-Beleg wird nicht als universelle OEM-Speichergrößengarantie ausgegeben. Der Rendereranschluss stützt sich auf den übernommenen Plan und die freigegebenen aktuellen Quellen.

Gelesen wurden relevante Abschnitte der folgenden exakt freigegebenen Dateien sowie das Lesemanifest /tmp/smartexplorer-login-memory-plan-scope.json. Die AGENTS-Anweisungen wurden vollständig vom Nutzer bereitgestellt und angewandt; die lokale AGENTS-Datei wurde in diesem Folgeauftrag nicht erneut geöffnet.

```text
android/app/build.gradle.kts
android/app/src/main/java/app/smartexplorer/android/api/LoginApi.kt
android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenFrameLimits.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginInput.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginSession.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginViewScreen.kt
android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenScaling.kt
android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenSession.kt
docs/ARCHITEKTUR.md
docs/plaene/2026-10-24-batches/aufnahme-speicher.md
docs/plaene/2026-10-24-batches/renderer-speicher.md
docs/plaene/2026-10-24-batches/umsetzung.md
native/Cargo.lock
native/Cargo.toml
native/android-bridge/Cargo.toml
native/android-bridge/src/lib.rs
native/android-bridge/src/login_jni.rs
native/src/app/core/login_viewer_image.rs
native/src/app/core/screen_render_cpu.rs
native/src/app/core/screen_render_glow.rs
native/src/app/core/screen_render_owner.rs
native/src/app/core/screen_render_upload.rs
native/src/app/core/screen_render_wgpu.rs
native/src/app/core/screen_scaling.rs
native/src/app/core/screen_texture_cpu.rs
native/src/app/core/screen_texture_worker.rs
native/src/daemon/os/shared/ipc_protocol_login.rs
native/src/daemon/os/shared/login_ipc.rs
native/src/daemon/os/shared/login_ipc_client.rs
native/src/daemon/os/shared/login_ipc_registry.rs
native/src/daemon/os/shared/login_ipc_session.rs
native/src/login/core/gateway_types.rs
native/src/login/core/gateway_wire.rs
native/src/login/core/model.rs
native/src/login/core/viewer_guard.rs
native/src/login/core/wire.rs
native/src/login/os/shared/gateway_client.rs
native/src/login/os/shared/viewer.rs
native/src/mobile/mod.rs
native/src/mobile/os/shared/domains/login_view.rs
native/src/mobile/os/shared/domains/mod.rs
native/src/screen/core/frame_budget.rs
native/src/screen/core/frame_memory.rs
native/src/screen/os/android/session_memory.rs
```

Der Planungsagent erstellte ausschließlich die temporäre Übergabe dieses Plans. Main übernahm ihren vollständigen Inhalt in diese kanonische Datei; es verbleibt keine zweite Arbeitskopie. Keine Produktquellen, Repositorydokumente oder graphify-Artefakte wurden geändert. Es wurden keine Builds, Compiler, Tests, Formatter, Server, Paketinstallationen, Graphneuberechnungen, Commits, Pushes, Releases oder Unteragenten ausgeführt. Die Ergebnisse sind Quellenentscheidungen und konkrete Abnahmesignale; erfolgreiche Produktumsetzung und Laufzeitabnahme sind nicht behauptet. Der Planauftrag endet mit der Übergabe an Main.
