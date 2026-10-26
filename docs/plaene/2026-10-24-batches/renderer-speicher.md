# Bildschirm-Renderer: konkreter Korrekturplan

Stand: 2026-10-26. Ergebnis dieses Auftrags ist ausschließlich dieser Plan. Die beschriebenen Produktänderungen sind nicht umgesetzt. Es wurden keine Builds, Compiler, Formatter, Tests, Server, Paketinstallationen, Graphneubauten, Commits oder Pushes ausgeführt.

## Entscheidung

Die Bildschirmtexturen benötigen einen eigenen, vom eframe-App-Lebenszyklus verwalteten Ressourcenbesitzer. `CpuTexture` behält seine BGRA-Konvertierung, seinen begrenzten Skalierer und seine UI-Aufrufer; große Bildschirmbilder werden jedoch nicht mehr als unbegleitete `ColorImage`-Deltas an den globalen egui-`TextureManager` abgegeben.

Der kleinste von den gelesenen APIs vollständig gedeckte Anschluss besteht aus:

1. WGPU: app-eigene `wgpu::Texture`, Registrierung ihres Views im vorhandenen eframe-Renderer über `register_native_texture`, ausdrückliche Entfernung über `free_texture`, anschließendes Destroy des tatsächlich selbst besessenen Texturobjekts und belegter Queue-Abfluss.
2. Glow: ein zusätzlicher, eng begrenzter `egui_glow::Painter` derselben gebundenen Bibliotheksversion für Bildschirmmeshes. Seine `set_texture`-, `paint_primitives`-, `free_texture`- und `destroy`-Methoden werden durch den App-Besitzer auf dem GL-Thread aufgerufen. Die Einbettung erfolgt über `egui_glow::CallbackFn`; die bestehende eframe-Oberfläche bleibt der äußere Painter. Es wird kein Shader kopiert, keine Registrydatei verändert und kein Framework geforkt.
3. YUV: die vorhandenen Shader und Ebenenformate bleiben erhalten. Die globale `YuvResources` hält nur noch Pipeline, Layout und Sampler. Bildabhängige Ebenen, Uniformdaten, Uploadzustand und Speicherzusage gehören zu einem widerrufbaren Darstellungsbesitzer.
4. Ein begrenztes Uploadticket je Darstellungsplatz. Ein weiteres Bild ersetzt höchstens den noch nicht hochgeladenen neuesten Kandidaten. Ein bereits an den Renderer übergebener Upload wird erst nach seinem belegten Abfluss ersetzt. Retirement ist ein ausdrücklicher Zustandsübergang; Zeitablauf und GUI-Tickzahl sind keine Freigabebelege.
5. Eine eindeutige Sitzungsinstanz in allen Präsentationsstempeln und eine getrennte Rendergeneration für Hidden, Quellenwechsel, CPU/YUV-Wechsel und Slotübernahme. Numerisch gleiche GOP-, Frame- und Epochwerte einer neuen Verbindung dürfen ein altes Skalierergebnis nicht zulassen.

Der zusätzliche Glow-Painter ist gegenüber der zunächst untersuchten Weak-/Hook-Lösung begründet: Der normale eframe-Painter ist im Glow-Callback nur als `&Painter` erreichbar. Seine mutierende `free_texture`-API kann die Anwendung dort nicht aufrufen. Nach einem vor dem Painter verworfenen `FullOutput` ist die einzige Freigabe bereits aus dem TextureManager herausgenommen; dessen Metadaten kennen den freigegebenen ID nicht mehr. Eine dauerhaft gehaltene Lease, eine direkte GL-Löschung unter einer verbleibenden Painterzuordnung oder ein unbegrenzt wachsender Vorrat kleiner Ersatztexturen lösen diese Grenze nicht. Ein selbst besessener Bibliotheks-Painter erlaubt dagegen dieselbe idempotente Freigabe bei jedem tatsächlichen Cleanupaufruf erneut.

## Quellbelege und Fehlergrenzen

Die genannten Frameworkpfade sind relativ zum jeweiligen versionsgebundenen Paket im lokalen Cargo-Quellcache. `native/Cargo.lock` bindet eframe, egui-wgpu, egui_glow und epaint an 0.29.1 sowie wgpu an 22.1.0.

| Grenze | Beleg | Konsequenz |
|---|---|---|
| CPU-Bildübergabe | `screen_texture_cpu.rs:104–116`; `epaint-0.29.1/src/texture_handle.rs:25–28/68–73` | `uploaded_memory` wird am UI-Objekt gehalten; das eigentliche Bild gelangt unabhängig davon in einen Delta. |
| Delta überlebt Handle | `epaint-0.29.1/src/textures.rs:49–81/98–103`; `image.rs:13–15` | Ein vollständiges Set ersetzt frühere noch im Manager stehende Sets desselben ID. `free` entfernt diese Sets nicht. Bereits abgetrennte Deltas besitzen eigene `Arc<ColorImage>`. |
| WGPU-Abbruch | `egui-wgpu-0.29.1/src/winit.rs:531–568/580–599/671–686` | Fehlende Surface kann den gesamten Upload überspringen; ein Surfacefehler nach `update_texture` kann Uploadwrites zurücklassen und Free sowie Submit überspringen. |
| WGPU-Vollupdates | `egui-wgpu-0.29.1/src/renderer.rs:631–681` | Jedes Full-Delta erzeugt eine neue Textur. Bloßer CPU-Arc-Abfluss begrenzt unsubmitted Uploadwrites bei wiederholten Surfacefehlern nicht. |
| Native WGPU-IDs | dieselbe Datei `:684–695/703–790` | `register_native_texture` legt einen User-ID mit `texture: None` und eigenem BindGroup an. `free_texture` entfernt diese Zuordnung; die Anwendung muss ihre native Textur selbst zerstören. |
| Glow-Zugriff | `egui_glow-0.29.1/src/painter.rs:108–124/650–658`; `eframe-0.29.1/src/epi.rs:661–694` | Callbackzugriff ist unveränderlich; `Frame::gl` liefert den Kontext, aber keinen veränderlichen äußeren Painter. |
| Glow-Fehlerpfad | `eframe-0.29.1/src/native/glow_integration.rs:611–659/1451–1508` | Ein Output kann vor dem Painter-Aufruf verworfen werden. Ein Callbackbeleg, der niemals aufgerufen wird, darf keine Freigabe behaupten. |
| Glow-Ressourcenbesitz | `egui_glow-0.29.1/src/painter.rs:75–105/362–379/402–466/510–553/650–653/720–741` | Der eigene Painter kann eigene IDs wiederholt entfernen. `destroy` ist ausdrücklich erforderlich. Der äußere Painter stellt seinen GL-Zustand nach einem Callback wieder her. |
| YUV-Altlast | `screen_wgpu_yuv.rs:79–100` und `YuvResources::upload`/`YuvCallback::prepare`/`paint` | Globale Ebenen und ihr voller Sitzungslease bleiben bis zum nächsten Upload bestehen. Der nächste Upload kann seinerseits an dieser Zusage scheitern. |
| Alte Kachelressourcen | `screen_tile_grid.rs:155–190/251–324`; `screen_tile_face.rs:151–157` | Übergabe, Ende, Offline und Retry entfernen nicht zuverlässig Textur und Skalierarbeit. |
| Fehlende Sitzungsidentität | `screen_presented.rs:5–25/50–57`; `screen_texture_cpu.rs:15–16/129–135` | Gleiche Geometrie und erneut beginnende Zahlen können ein fertiges Ergebnis einer alten Verbindung zulassen. |
| Beendeter Statusanker | `screen_session.rs:251/258–270/318–325`; `screen_decode_thread.rs:86–93` | `Shared.memory` lebt mit einer im Statusbildschirm behaltenen beendeten Sitzung weiter. Vor seinem Entfernen braucht der tatsächlich noch laufende Decoder-/Transportaufruf einen eigenen Leaseanker. |
| Tatsächliche Budgetgrenze | `frame_budget.rs:18–22/64–73/164–170`; `frame_size_caps.rs:145–175` | `I≤W/24` sowie die Produktpufferformel bleiben verbindlich. Codec-/Treiberstorage wird bereits ausdrücklich außerhalb dieser Produktpufferrechnung abgegrenzt; daraus folgt keine Erlaubnis für unbegrenzte Produkt-Uploadqueues. |

Externe Primärquellen, am 2026-10-26 geprüft:

- [wgpu 22.1.0 Queue](https://docs.rs/wgpu/22.1.0/wgpu/struct.Queue.html): `write_texture` kopiert sofort in Staging, überträgt aber erst beim nächsten Submit. Ein leeres `submit([])` startet diese Writes. `on_submitted_work_done` bezieht sich auf den vorherigen Submit; Callbackfortschritt braucht Submit oder Poll.
- [Gebundene wgpu-Quelle](https://raw.githubusercontent.com/gfx-rs/wgpu/v22.1.0/wgpu/src/lib.rs): `Device::poll`, `Maintain::WaitForSubmissionIndex`, `Texture::destroy` und Device-Lost-Callback sind verfügbar. `destroy` allein verspricht keine sofortige physische Speicherfreigabe.
- [Gebundener egui-wgpu-RenderState](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui-wgpu/src/lib.rs): Device, Queue und Renderer sind öffentlich als `Arc` zugänglich; der Renderer besitzt ein `RwLock`.
- [Gebundener eframe-Eventloop](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/eframe/src/native/run.rs): `exiting` ruft `save_and_destroy`; `run_and_return` besitzt den vollständigen `WinitAppWrapper` lokal. Auch die Rückkehr über `?` droppt diesen Besitzer. Der threadlokale Cache hält den EventLoop, nicht die konkrete Running-/Glutin-Instanz. Zusammen mit den schwachen Immediate-Viewport-Verweisen in `glow_integration.rs:315–335` belegt dies den Abbau der nativen Instanz vor Rückkehr des konkreten Rendereraufrufs.
- [Khronos Sync Objects](https://wikis.khronos.org/opengl/Sync_Object) und [OpenGL-Synchronisation](https://wikis.khronos.org/opengl/Synchronization): Ein signalisiertes Fence belegt abgeschlossene vorherige GL-Kommandos; Flush oder Buffer-Swap allein ist dieser Beleg nicht. `glFinish` ist die kompatible synchrone Rückfallebene, falls das vorhandene GL keinen Sync-Objektpfad bereitstellt.

Die fehlgeschlagenen docs.rs-Abrufe für einzelne Texture-/Device-Seiten wurden durch die gebundene upstream-Quelle ersetzt. Aus neueren wgpu-Versionen wurde kein API-Anschluss übernommen.

## Besitzmodell

### Sitzungs- und Renderidentität

`screen_presented.rs` erhält `SessionToken`, einen kleinen `Arc`-Identitätstoken ohne Speicherzusage und ohne Rückverweis auf `Shared`. Gleichheit prüft die Allocationidentität. Damit gibt es weder einen wiederbeginnenden Zähler noch eine still überlaufende globale Nummer. `PresentationState` erzeugt den Token bei einer neuen Sitzung; `PresentedFrame` trägt ihn neben Epoch, GOP und Frame. `PresentedFrame` ist danach `Clone`, nicht mehr `Copy`. `begin_frame`, `matches`, `can_present`, `presented_callback`, `Picture`, `Work` und `Finished` reichen dieselbe Identität weiter.

Zusätzlich besitzt ein Darstellungsplatz eine `RenderGeneration`. Eine neue Generation widerruft alte Uploads und Callbacks sofort. Eine Generation kann als weiterer Allocationtoken umgesetzt werden. Die Rendergeneration ist keine neue Protokollversion und wird nicht auf dem Draht übertragen.

Für LoginImage bindet der Renderer die konkrete `Arc<login::ViewerSession>`-Instanz und den vorhandenen `presentation_epoch`. Der heute konstante `PresentedFrame { epoch: 1, gop_id: 1, ... }` darf nicht als sitzungsübergreifender Beleg weiterverwendet werden. Die tatsächlichen Login-`FrameToken`- und Autorisierungsprüfungen bleiben maßgeblich; der neue lokale Token ersetzt sie nicht.

### Datenobjekte

Vorgesehene schmale neue Module unter `native/src/app/core/`, jeweils unter 500 Zeilen und 50 KiB:

- `screen_render_owner.rs`: `RendererOwner`, widerrufbare Renderplätze, Cleanupzustand und ein schwacher Context-Verweis auf den Besitzer.
- `screen_render_upload.rs`: `PreparedImage`, `UploadTicket`, Generation und begrenzte Zustandsübergänge. Keine Betriebssystemimporte und keine eigene Bildwarteschlange.
- `screen_render_wgpu.rs`: native RGBA-Texturregistrierung, Queue-Abfluss und tatsächliche Entfernung.
- `screen_render_glow.rs`: der eigene Bibliotheks-Painter, Bildschirmmesh, GL-Fences und `destroy`.
- `screen_render_app.rs`: eframe-App-Wrapper für Install, Update, Shutdown und die Rückkehr aus `run_native`.

Die Dateiaufteilung trennt Besitzentscheidungen, Backendzugriffe und App-Lebenszyklus. Die Implementierung darf diese fünf Dateien enger zusammenfassen, wenn die Verantwortungen und Größenregeln erhalten bleiben; sie darf keinen neuen plattformabhängigen Code in `screen/core` einführen.

`PreparedImage` hält zusammen: `Arc<ColorImage>`, den eigenen `MemoryHold`, den vollständigen Präsentationsstempel, Rendergeneration und Zielmaße. Die Anwendung gibt das Paket an ihren eigenen Renderer. Eine Paint-Callback-Closure hält bei Bedarf genau dieses Paket; sie erhält keinen großen `ColorImage` ohne Leasebegleitung. Wird `FullOutput` vor dem Painter verworfen, fällt lediglich dieses Paket weg. Ein vorher bereits angelegter GPU-Besitzer verbleibt beim `RendererOwner` und kann ausdrücklich bereinigt werden.

Pro Renderplatz sind die folgenden Zustände zulässig:

`Leer → Kandidat → Upload läuft → Resident → Retiring → Abgeflossen`

Ein Kandidat ist der jeweils neueste noch nicht hochgeladene fertig skalierte Frame. Er ersetzt den vorigen Kandidaten. Während eines Uploads darf kein zweites Uploadticket beginnen. Ein Resident kann während seines Ersatzes leben, aber es dürfen höchstens der alte und der neue Rendererbildbesitzer existieren. Vor einer dritten Ressourcenanlage muss das frühere Retirement abgeschlossen sein. Die Registry besitzt höchstens einen aktiven und einen bereits eingeplanten Retirementzustand je Renderplatz; sie bekommt keine neue Listenzeile pro Frame.

Der Renderplatz bleibt über Hidden und CPU/YUV-Wechsel derselben Sitzung identisch. Endgültig beendete Plätze werden nach Abfluss entfernt. Eine neue Sitzung benötigt ihre eigene einmalige MemoryPool-Zusage; alte Retirementbesitzer werden nicht dieser neuen Zusage zugeschlagen.

Die Platzidentität darf nicht aus jedem neuen `CpuTexture::default()` entstehen. Vollansicht und Loginpane behalten jeweils ihren logischen Platz über Reset und Neuverbindung; Kacheln verwenden die bereits vorhandene stabile Geräteidentität und ihren ausdrücklich übernommenen SessionToken. Die bestehenden Zuweisungen `texture = CpuTexture::default()` werden deshalb durch `reset`/`retire_generation` am gehaltenen Platz ersetzt. Eine Übergabe Preview/Full überträgt den Platz oder nimmt einen vorher reservierten zweiten Rollenplatz derselben Sitzung; wiederholte Übergaben dürfen keine zusätzliche Rollenliste anlegen. Login hat heute keinen Screen-MemoryLease und muss deshalb besonders den einen Paneplatz mit höchstens einem auslaufenden Vorgänger wiederverwenden. Es entsteht keine neue feste Obergrenze für die Anzahl der Geräte; die vorhandene gemeinsame Sitzungszulassung bleibt deren Ressourcengrenze.

Die drei produktiven CPU-Zeichner `ViewWidget`, `draw_tile` und `LoginImage` verwenden nach `prepare` dieselbe neue `CpuTexture::paint`-Grenze. Diese Grenze wählt den tatsächlich zu zeichnenden residenten beziehungsweise im Glow-Callback hochzuladenden Kandidaten mitsamt dessen Stempel aus. Der Receipt wird an genau diesen ausgewählten Stempel gebunden. Im Glow-Pfad darf ein UI-seitiger Test auf „bereits resident“ den Callback für den ersten Kandidaten nicht verhindern: Upload und Draw finden dort erst im tatsächlichen Callback statt. Die bisherige getrennte Folge `id → painter.image → generischer cpu_callback` bleibt nur innerhalb des WGPU-Adapters erhalten, der die native ID wirklich besitzt. Kein Aufrufer darf einen alten residenten Stempel mit dem Receipt der neuesten Quelle kombinieren.

## WGPU-Anschluss

1. Beim Install speichert der Owner den zum konkreten `CreationContext` gehörenden RenderState. Capabilities stammen aus diesem Kontext; kein vergangener WGPU-Versuch darf den späteren Glow-Kontext als WGPU kennzeichnen.
2. CPU-RGBA bleibt `Rgba8UnormSrgb` mit `TEXTURE_BINDING | COPY_DST`, einem Miplevel und Nearest-Sampler, entsprechend dem bisherigen egui-Pfad. Die CPU-Area-/Catmull-Rom-Rekonstruktion und der Native-Pixelbypass bleiben unverändert. Der View wird über `register_native_texture` im vorhandenen Renderer registriert; normale egui-Bildmeshes können diesen User-ID weiterhin zeichnen.
3. Die Daten gehen unmittelbar aus dem gehaltenen `PreparedImage` an `queue.write_texture`; es wird kein Full-Delta in `ctx.tex_manager()` angelegt. Derselbe native Texture-Slot kann bei unveränderter Geometrie wiederverwendet werden. Bei Geometriewechsel darf ein alter Slot erst nach der beschriebenen Retirementgrenze ersetzt werden.
4. Jeder gestartete Upload erhält einen Submissionbeleg. Ein leeres `queue.submit([])` nach den eigenen Writes macht diesen Beleg von eframes Surface-Erfolg unabhängig. Danach wird `on_submitted_work_done` registriert. Die Lease bleibt beim Ticketbesitzer, bis die Bestätigung tatsächlich eingetroffen ist. Die Closure darf nur Zustand setzen und wecken; ihr eigener Drop gilt nicht als Bestätigung.
5. Ein einzelner Renderer-Aufräumdienst treibt `Device::poll` und bereinigt bestätigte Tickets. Er kann für ausstehende eigene SubmissionIndices außerhalb des GUI-Threads warten. Er startet keinen Thread pro Frame und hält beim Poll weder den egui-Rendererlock noch den MemoryPool-Lock. Er entfernt keine Rendererzuordnung und zerstört keine noch registrierte Textur. `MemoryPool::reserve` wird niemals unter einem Rendererlock aufgerufen.
6. Beim Retirement wird zuerst die Rendergeneration widerrufen. Dadurch darf ein noch im `FullOutput` enthaltener Callback weder neu hochladen noch Präsentation bestätigen. Dann entfernt der Owner seinen User-ID über `renderer.write().free_texture(&id)`. Er zerstört und droppt anschließend seine tatsächlich selbst besessene native Textur; die Registrierung hatte diese Textur nicht übernommen. Bereits ausgegebene eigene Writes werden mit einem Submissionbeleg abgeführt. Die letzte zugehörige Zusage wird erst nach bestätigtem Abfluss und nach Drop sämtlicher CPU-Paketbesitzer freigegeben.
7. Entfernung und Destroy erfolgen ausschließlich auf dem GUI-Thread an der Wrappergrenze vor beziehungsweise nach dem inneren App-Update, außerhalb eines laufenden `CallbackTrait::prepare`/`paint`. Der vorangegangene eframe-Paintaufruf hat dann seinen Submit ausgeführt oder seine CommandBuffer verworfen. Ein Hintergrund-Cleanup allein unter dem Rendererlock wäre unzureichend: eframe kann zwischen Renderpass-Aufzeichnung und Submit diesen Lock freigeben; ein dann gesetztes Fence könnte vor dem noch nicht eingereichten Renderpass fertig werden. Ein Callback darf auch nicht erneut den Rendererlock nehmen, den eframe während seines Callback-Aufrufs bereits hält. CPU-Paintproof liest nur den eigenen Slotzustand und prüft den wirklich residenten Stempel.
8. Der Fall „Surface fehlt“ und der Fall „get_current_texture scheitert nach Upload“ benötigen keine neue Bildschirmaufnahme. Eigene Uploadwrites wurden bereits submitted; die selbst besessene Textur kann unabhängig von eframes Free-Loop entfernt werden. Ein dauerhaft defektes Device wird für neue Slots geschlossen. Es wird keine erfolgreiche Fertigstellung allein aus einem Timeout, einem Device-Lost-Text oder dem Drop einer Callback-Closure abgeleitet.

Der heutige `CpuPresented`-Callback darf nur dann quittieren, wenn der genaue User-ID noch dem aktiven Slot und dem residenten Frame gehört. Das verhindert einen Beleg hinter einem Mesh, dessen Textur bereits retired wurde. Die Quittierung bleibt ein Beleg des vorhandenen Rendercallback-Vertrags; der Plan behauptet keine neue Bestätigung durch Monitor oder Compositor.

## Glow-Anschluss

1. Der Owner erzeugt höchstens einen zusätzlichen `egui_glow::Painter` je eframe-Rendererkontext. Die eingebettete Bibliotheksimplementierung übernimmt Texturformat, Shader, Farbverarbeitung und Meshzeichnung. Der Laufwrapper reicht dieselben `shader_version`-/`dithering`-Optionen weiter, mit denen der äußere eframe-Painter angelegt wird. `Painter::new`-Fehler liefern den bestehenden sichtbaren Ressourcenfehler und keinen Präsentationsbeleg.
2. `CpuTexture::paint` legt einen `CallbackFn` an. Im tatsächlichen Aufruf prüft der Callback Slot und Generation, lädt den gehaltenen Kandidaten mit `set_texture` hoch und zeichnet ausschließlich das zugehörige Bildmesh mit `paint_primitives`. Meshrechteck, UVs, weiße Tintfarbe, tatsächliches `pixels_per_point` und die Schnittmenge aus Callback-Clip und Bildclip werden weitergereicht. Der äußere Painter stellt seinen Zustand nach dem Callback wieder her (`painter.rs:451–454`). Es wird kein `clear` auf den äußeren Framebuffer angewendet.
3. Die Bild-ID gehört ausschließlich dem eigenen Painter. Sie wird nie beim äußeren Painter registriert. `free_texture` entfernt daher dieselbe Zuordnung, deren native Textur es löscht; ein späterer zweiter Cleanup bleibt wirkungslos. Eine direkte Löschung unter einer fortbestehenden fremden Painterzuordnung ist ausgeschlossen.
4. Eine Retirementanforderung bleibt im Owner, bis dessen GL-Cleanup wirklich gelaufen ist. Das Cleanup ist Teil des App-Lebenszyklus vor und nach dem inneren Update und kann zusätzlich von einem kleinen unabhängigen Cleanup-Callback getrieben werden. Es hängt weder von einem neuen Bild noch vom sichtbaren Bildbereich der alten Sitzung ab. GL-Aufrufe erfolgen ausschließlich auf dem aktuellen eframe-GL-Thread; der WGPU-Aufräumdienst ruft keine GL-Funktion auf.
5. Nach Entfernung der eigenen Textur erfasst ein GL-Fence die vorangegangenen Nutzungen. Nur ein signalisiertes Fence gibt das GPU-Retirement frei. Die nichtblockierende Prüfung führt zu weiteren Repaints, solange tatsächlicher Cleanup offen ist. Falls Sync-Objekte vom vorliegenden GL-Kontext nicht unterstützt werden, verwendet nur dieser Retirementpfad `glFinish`; dieser kompatible Rückfall kann das UI während des tatsächlichen GPU-Abflusses blockieren und muss als solche Grenze dokumentiert werden. Die aktive CPU-Skalierung wartet weiterhin nicht auf den GUI-Thread.
6. Wird ein `FullOutput` vor dem Painter-Aufruf verworfen, wurden dessen Callbackkandidaten nicht hochgeladen. Ein bereits residentes Bild bleibt im eigenen Painter und im Owner. Der nächste echte Lifecycle-Cleanup kann denselben ID erneut freigeben. Es gibt weder ein verlorenes fremdes `TexturesDelta.free` noch eine bereits unwiederbringlich entfernte TextureManager-Metadatenzeile.
7. Beim regulären `on_exit(gl)` widerruft der Owner sämtliche Slots, beendet neue Uploads, leert seine CPU-Kandidaten, entfernt seine Texturen und ruft `Painter::destroy` auf. Der abschließende tatsächliche GL-Abfluss wird vor Freigabe der Retirementzusagen abgewartet. eframe ruft das App-`on_exit` vor dem Destroy des äußeren Painters auf (`glow_integration.rs:370–379`); dies ist der vorhandene Cleanupanschluss.

## App-Ende und Fehler

Ein gemeinsamer `ScreenRenderApp<A>`-Wrapper ist gegenüber einem neuen Feld in der sehr großen Main-App-State-Struktur vorzuziehen. Er installiert den Besitzer, ruft Cleanup vor und nach `A::update` auf und garantiert den Cleanup auch bei den bestehenden frühen `return`-Pfaden in `frame_update.rs`. Er leitet alle für `eframe::App` 0.29.1 verwendeten Einstellungen und Lifecycle-Methoden an die innere App weiter; insbesondere bleiben Main-App-`finish_exit`, bestehende Sync-Exitgates, Speicherpersistenz und Clear-Color-Verhalten erhalten.

Der vollständige native Forwardingumfang der gebundenen App-Trait lautet `update`, `save`, `on_exit`, `auto_save_interval`, `clear_color`, `persist_egui_memory` und `raw_input_hook`. Der Wrapper ergänzt ausschließlich Rendererarbeit um diese Delegation. Dadurch benötigen die 492 Zeilen von `app/core/init.rs` und die große Main-App-Struktur keine zusätzliche Rendererinitialisierungsverantwortung. Der Modulanschluss kann in der vorhandenen schmaleren `app/screen_modules.rs`-Registrierung erfolgen; deren Inhalt wurde in diesem Planauftrag nicht benötigt und nicht gelesen.

Konkrete Anschlüsse:

- `native/src/lib.rs:200–213`: Main-App einschließlich LoginImage in den Wrapper nehmen.
- `screen_launch.rs:50–68`: jeden WGPU-/Glow-`run_native`-Versuch einzeln an den Wrapper und einen äußerlich gehaltenen Laufabschlussbesitzer binden.
- `family_viewer.rs:21–29`: beide Renderer-Versuche derselben Grenze unterstellen.
- `screen_viewer_app.rs:78–85` und `family_viewer.rs:42–46`: YUV-Install und tatsächliche Rendererfähigkeit mit dem installierten Owner verbinden.
- `frame_update.rs:5–31`: innere `on_exit`-Arbeit und frühe Rückgaben unverändert durch den Wrapper abdecken. Ein weiterer, davon getrennter Timer ist nicht nötig.

Contextdaten halten nur `Weak<RendererOwner>`. Der Owner darf keine Schleife `Context → Owner → Context` erzeugen. Callbacks halten einen Slot beziehungsweise ein Datenpaket und höchstens einen schwachen Owner-Verweis. Der App-/Laufbesitzer hält den Owner stark.

Der äußere Laufbesitzer markiert die konkrete Rendererinstanz nach Rückkehr ihres `run_native`-Aufrufs geschlossen. Ein fehlendes `on_exit` auf einem Fehlerpfad wird dadurch erkannt. Er lässt den app-eigenen WGPU-Besitz geordnet abfließen und entfernt sämtliche app-eigenen nativen Rendererreferenzen, bevor ein Glow-Fallback denselben prozessweiten MemoryPool benutzen darf. Im Glow-Fehlerfall wird kein GL-Aufruf mit einem nicht mehr aktuellen Kontext erfunden; CPU-Pakete werden verworfen, und die große Zusage bleibt bis zum nachgewiesenen Ende der zugehörigen nativen Rendererinstanz gebunden. Eine zurückgekehrte App allein ist kein Ersatz für diesen Instanzabschluss.

**Abgeschlossene zweite Lückenprüfung:** Die zusätzlich abgerufene gebundene upstream-Datei `native/run.rs:180–186/269–278/292–331` zeigt den regulären Destroyaufruf und den lokalen Besitz der konkreten WinitApp über die gesamte Ausführung. Bei Rückkehr wird dieser lokale Besitzer vollständig gedroppt; der threadlokal wiederverwendete EventLoop enthält ihn nicht. Die lokale Glow-Integration hält Immediate-Viewport-Zugriffe ausdrücklich nur schwach (`:315–335`). Damit kann der äußere Laufbesitzer den konkreten `run_native`-Rückkehrpunkt als Instanzabschluss verwenden, sofern die neue Implementierung selbst keine native Running-/Glutin-Instanz festhält. Sie benötigt dafür keine solche Referenz. Weder `Weak<glow::Context>` noch ein pauschaler `App::drop` allein wird als Ersatzbeleg verwendet; `glow::Context` ist nicht der Glutin-Kontextbesitzer. Bei einem nicht zurückkehrenden Prozessabbruch gibt es keinen fortgesetzten Prozess, in dem eine Zusage erneut zugelassen werden könnte.

## Sitzung, Skalierer und Übergänge

`Shared.memory` ist nach Aufnahme ein Zugang zu der einmaligen Zusage, kein zulässiger ewiger Besitz des Statusbildschirms. `screen_decode_thread::run` beziehungsweise `DecodeState` erhält vor jeder möglichen Freigabe dieses Shared-Ankers einen eigenen Laufanker. Der Anker umfasst Decoder, die noch aktive `next()`-Verarbeitung und die dort gehaltenen Produktpuffer. `Shared::end`/`stop_presentation` können danach den Zugriff auf neue Bilder schließen, `picture` leeren und ihren Shared-Anker lösen. Bereits erzeugte `Picture`-, Work-, Finished- und Rendererpakete behalten jeweils ihren eigenen Arc. Ein laufender Decode wird niemals allein durch das Entfernen des UI-Statusobjekts unfinanziert.

Der CPU-Skalierer behält höchstens eine laufende Arbeit, eine zusammengefasste nächste Arbeit und eine fertige Ausgabe. Retirement invalidiert die Generation unter derselben Synchronisation, leert `pending` und `finished` und weckt den Worker. Die laufende Arbeit prüft den Widerruf auch während der vorbereitenden Tapberechnung und zwischen den bestehenden Zeilenarbeiten. Sie gibt ihre Zusage erst beim tatsächlichen Ende ihrer Puffer frei. Ein neuer Worker darf nicht als beliebig oft wiederholter Ersatz neben noch auslaufenden alten Workern derselben Sitzung entstehen: Hidden und Rebind verwenden den bestehenden Worker mit neuer Generation oder warten dessen konkretes Exit-Signal ab.

`Finished` wird nur bei passender Sitzungsinstanz, Rendergeneration, gültiger Quellgeometrie und aktuellem Ziel übernommen. Innerhalb derselben gültigen Generation darf ein bereits fertiges älteres Frame weiter die laufende Livevorschau versorgen, sofern es neuer als das bereits residente Bild ist und die vorhandene `can_present`-Grenze besteht. Eine pauschale Forderung `done.seq == neueste source.seq` würde bei kontinuierlichem Eingang den asynchronen Skalierer aushungern und ist deshalb nicht der vorgeschlagene Fix.

Vollständige Übergangsliste:

| Auslöser | Notwendiger Anschluss | Erwartetes Ergebnis |
|---|---|---|
| Sitzungsende/Verbindungsbruch | `Shared::end`, `ViewWidget::show`, `ViewState::end_session`, `ScreenTileGrid::step` | Bild-/Worker-/Renderbesitz wird retired; der Fehler-/Statusinhalt bleibt sichtbar; keine alte Quittierung. |
| Offline | `ScreenTileGrid::step` | Sitzung, Bild, Kandidat, Workerauftrag und Rendergeneration werden gemeinsam geschlossen. |
| Retry/Neuverbinden | `screen_tile_face::draw_tile`, `ViewState::reconnect/open` | Alte Instanz wird vor dem Neubinden widerrufen; eine wegen des eigenen noch abfließenden Besitzes blockierte Zulassung wird ereignisgebunden fortgesetzt. |
| Hidden/ausgeclippter Bildbereich | `ViewWidget::show`, `draw_tile`, `LoginImage::show` | Kein altes Bild und keine alte Inputfreigabe; Cleanup läuft unabhängig vom geclippten Bildcallback. Beim Sichtbarwerden gelten neuer Sichtbarkeitsbeleg und frischer gültiger Frame. |
| Quellen-/GOPwechsel | `ViewWidget::source_changed`, Präsentations-/Decoderstempel | Alte Worker-/Callbackgeneration wird ungültig; neue Quelle nutzt weiter dieselbe zulässige Sitzungszusage. |
| CPU → YUV | `ViewWidget::accept` | CPU-Kandidat und Worker werden retired; höchstens erlaubter alter/neuer Renderbesitz; kein verwaister CPU-Lease im Texturslot. |
| YUV → CPU/Fallback | `ViewWidget::accept`, YUV-Slot | YUV-Ebenen werden ausdrücklich entfernt; kein Warten auf einen zukünftigen YUV-Upload. |
| Preview → Full | `take_session`, `ScreenViewerApp::to_view` | Transport und zugesagter Vertrag bleiben erhalten; der alte Darstellungsplatz wird übergeben oder retired, niemals zusätzlich vergessen. |
| Full → Preview | `ScreenViewerApp::to_grid`, `place_pending` | Derselbe Transport bleibt verwendbar; alte Full-Ebenen und Skalierausgabe werden vor neuer unbegrenzter Ressourcenanlage retired. |
| Kachel verschwindet/Gitter wird ersetzt | `sync_devices`, Modewechsel, Drop der UI-Besitzer | Drop stellt nur einen idempotenten Retirementauftrag; er gibt eine noch benötigte Zusage nicht selbst frei. |
| Login Ende/Stop/Neue Sitzung/Hidden | `login_viewer.rs:17–20/51–55`, `login_viewer_image.rs` | Dieselbe Rendererbereinigung und eindeutige lokale Instanz; Loginrechte, FrameToken, Fokus, Freigabe und bestehende Wiederstartentscheidung bleiben erhalten. |
| Fensterende/Rendererfehler/Fallback | Wrapper und konkrete `run_native`-Grenze | Kein Leaseanker überlebt einen tatsächlich beendeten Renderer als funktionslose Altlast. |

Ein vorübergehend noch laufendes Retirement darf nicht als endgültiger Speicherfehler einer angeforderten Neuverbindung gespeichert werden. Der Start wartet auf das konkrete Abfluss-/Worker-Exitereignis und kann abgebrochen werden. Er läuft nicht auf dem GUI-Thread. Sind sämtliche Retirementereignisse abgeschlossen und reicht das reale Angebot weiterhin nicht, bleibt die bestehende Ressourcenablehnung richtig. Es gibt keinen Timeout, der eine Zusage ungeprüft freigibt.

## Verbindliche Budget- und Kompatibilitätsgrenzen

- `VERSIONED_WORKING_IMAGES=24`, `MAX_SESSION_WORKING_BYTES=864 MiB`, die Legacy-/RAW-Grenzen und die Paketformel werden durch diese Rendereränderung nicht erhöht oder gelockert. Capture-Admission ist ein getrennter Auftrag.
- Jeder neue Rendererplatz verwendet die vorhandenen Bildrollen: Quelle, laufende/nächste Skalierquelle, fertige Ausgabe, alter/neuer Rendereroutput. Ein `PreparedImage` aliasiert die vorhandene fertige Ausgabe, statt sie vor jedem Upload erneut vollständig zu klonen. Ein Pending-Slot ersetzt seinen Inhalt; er wächst nicht mit der Bildanzahl.
- Jede RGBA-Allokation bleibt gegen die zur Quelle gehörenden `FrameLimits` und die tatsächliche Renderer-Texturgrenze geprüft. Eine kleinere Zielgröße darf nicht als Recht zur Übernahme einer größeren ungeprüften Quelle dienen.
- YUV-I420/NV12, BT.601/BT.709, Nearest bei 1:1, Area beim Verkleinern, Catmull-Rom beim Vergrößern, Pan, Clipping, Dimmen und bestehende Full-/Previewgrößen und Bildraten bleiben erhalten.
- Native 3840×2160 wird weiterhin zugelassen, wenn der bereits angenommene Vertrag und die tatsächlichen Ressourcen es erlauben. Die Rendererreparatur darf keine neue feste 512-MiB- oder 2048-/4096-Seitenbeschränkung einführen.
- Es gibt keine unbegrenzte Prozessliste aus Leases, Texture-IDs, GL-Fences oder WGPU-Submissionclosures. Retired-Zustände sind vor der Ressourcenanlage einem vorhandenen Renderplatz zugeordnet; bei belegtem Vorgängerplatz wird ein Kandidat zusammengefasst oder wartet. Endgültig abgeflossene Slots werden entfernt.
- Das Budget ist weiterhin keine Gesamt-RSS-, VRAM-, Codec-, Treiber- oder Betriebssystemgarantie. Der neue Uploadticketmechanismus begrenzt die von der Anwendung ausgelösten noch nicht abgeflossenen Transfers. Diese Abgrenzung darf keine nichtgezählten CPU-Bildkopien verdecken.

## Meilensteine und gemeinsame Abnahmesignale

| Meilenstein | Dateien/Module | Abhängigkeit | Konkretes erwartetes Ergebnis für die eine Remote-Task-Suite |
|---|---|---|---|
| M1 Identität und Laufanker | `screen_presented.rs`, `screen_session.rs`, `screen_decode_thread.rs`, `screen_viewer_fake.rs`, betroffene Stempelaufrufer | Keine Rendereränderung erforderlich | Zwei Sitzungen mit identischen Epoch-/GOP-/Framezahlen bleiben verschieden. Ende löst den Shared-Statusanker; ein angehaltener laufender Decode hält die Zusage weiter. |
| M2 Begrenzte Besitzerzustände | `screen_render_owner.rs`, `screen_render_upload.rs`, `screen_texture_cpu.rs` | M1 | Wiederholte Kandidaten, Hidden und Rebind erzeugen höchstens einen laufenden Upload und keinen zweiten noch lebenden Skalierer derselben Rendergeneration. Drop eines UI-Besitzers allein gibt gehaltene Uploaddaten nicht frei. |
| M3 WGPU-Ressourcen | `screen_render_wgpu.rs`, `screen_render_caps.rs`, `screen_texture_cpu.rs` | M2 | Registrierter User-ID, native Textur und Uploadticket werden nach explizitem Retirement entfernt; auch ohne Surface und ohne Folgebild wird der Pool nach bestätigtem Queue-Abfluss wieder frei. |
| M4 Glow-Ressourcen | `screen_render_glow.rs`, `screen_texture_cpu.rs`, die drei Bildzeichner | M2 | Ein vor dem Painter verworfener Callback lässt kein verlorenes Free zurück. Cleanup entfernt die noch residente eigene Textur idempotent. Schmale Clips und Display-Skalierung bewahren die sichtbaren Bildkoordinaten. |
| M5 YUV-Besitz | `screen_wgpu_yuv.rs`, `screen_view_widget.rs`, bei Bedarf eigenes kleines YUV-Slotmodul | M2/M3 | Ende, Hidden und CPU-Wechsel entfernen den bildabhängigen globalen Besitz; ein alter Callback lädt nach Widerruf nichts neu hoch. I420/NV12 behalten ihre Matrix-/Skalierungswege. |
| M6 Alle UI-Übergänge | `screen_tile_grid.rs`, `screen_tile_face.rs`, `screen_viewer_view.rs`, `screen_viewer_app.rs`, `family_viewer.rs`, `login_viewer.rs`, `login_viewer_image.rs` | M1–M5 | Offline, Ende, Retry, Hidden, Quellenwechsel und Preview/Full-Handover haben jeweils das oben definierte Retirement. Ein fertiges altes Workerresultat kann keine neue Sitzung quittieren. |
| M7 App-/Fehlerlebenszyklus | `screen_render_app.rs`, `app/mod.rs`, `lib.rs`, `screen_launch.rs`, `family_viewer.rs` | M3–M6; gebundener eframe-Rückkehrpfad ist nachgelesen | Frühe Main-App-Returns, reguläres Exit und gescheiterter WGPU-Versuch lassen keine inaktive Rendererinstanz mit voller Zusage zurück. LoginImage besitzt auf beiden Backends einen installierten Renderer. |
| M8 Integrierte Prüfquellen | neue begrenzte Renderer-Fixturedatei; `batches_preview_task_tests.rs`; vorhandene MemoryPool-Fixtures | Alle Produktänderungen abgeschlossen | Gemeinsame Suite ordnet jede oben genannte Übergangserwartung einem tatsächlich ausgeführten Fall zu; keine lokale Ausführung, keine zusätzliche Suite. |

Die deterministischen Fixtures benötigen steuerbare Ereignisse für Upload übernommen, Output verworfen, GPU-Abfluss bestätigt, Worker tatsächlich beendet und Renderer tatsächlich beendet. Das kontrollierte Nichtbestätigen eines Ereignisses muss die Zusage halten; dessen echtes Bestätigen muss sie freigeben. Eine Fixture, die nur `drop(CpuTexture)` ausführt und einen nächsten Tick simuliert, reicht nicht.

`batches_preview_task_tests.rs:23–40` benötigt eine konkrete Anpassung: `egui::Context::default()` enthält keinen Renderer. `CpuTexture::prepare` darf in diesem Zustand weder Produktionserfolg noch einen residenten Präsentationsstempel vortäuschen. Die Fixture installiert ausdrücklich einen nur unter `cfg(test)` verfügbaren steuerbaren Treiber an der schmalen Upload-/Drawgrenze. Sie führt weiter die produktiven Methoden für Bildannahme, Vorbereitung, residenten Stempel, Preview-Receipt und `ViewerSession::presented_callback` aus. Erst das simulierte erfolgreiche Uploadereignis macht den exakten Stempel resident; erst das anschließend simulierte wirkliche Zeichnen ruft den produktiven Receipt auf. Ohne Treiber, nach bloßer Vorbereitung, nach verworfenem Callback und mit einem Stempel der vorherigen Sitzung bleibt `input_ready` falsch. Dieser kontrollierte Treiber prüft die echte Besitz-/Receiptverkabelung, behauptet aber keinen GL-/WGPU-Laufzeitnachweis. Die realen Backendfälle bleiben davon getrennte Fälle derselben einen Remote-Suite.

Die Remote-Suite soll außerdem die tatsächlichen Bibliotheksgrenzen abdecken: WGPU ohne benutzbare Surface nach einem Upload, Glow-Callbackverwerfung vor Paint bei schon residenter Textur, anschließende explizite Bereinigung sowie die drei produktiven CPU-Aufrufer Vollansicht, Kachel und Login. Wenn der konfigurierte Runner keinen realen GL-/WGPU-Kontext bereitstellt, wird genau dieser fehlende Backendnachweis berichtet; er wird nicht als bestanden behandelt und begründet gemäß Projektregel keinen Aufschub des abschließenden Releases.

## Verworfen und begründet

- `uploaded_memory` hinter `handle.set` verschieben: ändert den fremden Arc-Besitz nicht.
- Nur `Weak<ColorImage>` mit Lease sammeln: schützt einen CPU-Puffer, beweist aber weder GPU-Free noch Queue-Submit; ohne Slotlimit entsteht ein zusätzlicher Rückstau.
- Nächster Tick oder fester Repaint-/Zeitabstand: liefert keinen Backendbeleg.
- Bei Retirement nur 1×1 einstellen und Handle droppen: entfernt nicht die bereits abgetrennten großen Deltas; Surfacefehler kann Free verlieren. Nach verlorenem Glow-Free fehlt weiterhin die mutierende API auf den äußeren Painter.
- Paint-Callback plus Drop-Witness am äußeren Glow-Painter: beweist bei tatsächlich ausgeführtem, normal beendetem Paint dessen Free-Loop, hat aber keinen wiederholbaren Recoverypfad für den vor Paint verworfenen Output. Daher keine vollständige Lösung dieses Auftrags.
- GL-Textur direkt löschen, während ihr ID im äußeren Painter bleibt: ermöglicht späteres doppeltes Delete und Namenswiederverwendung.
- Native Glow-Textur beim äußeren Painter registrieren und selbst löschen: widerspricht dessen ausdrücklich übernommener Ownership (`eframe::Frame::register_native_glow_texture`).
- Sessionmemory beim UI-Ende pauschal leeren: kann einen laufenden Decoder oder Skalierer unfinanziert lassen.
- Globale YUV-Planes bis zur nächsten Sitzung behalten: bleibt ein Zulassungszirkel.
- Neue Farbshader, eigene eframe-Forks oder lokale Cargo-Registrypatches: für den beschriebenen Anschluss nicht erforderlich.

## Abschlussbericht dieses Planauftrags

**Gelesene Repositorydateien, jeweils vollständig oder gezielt an den genannten Symbolen:** `docs/ARCHITEKTUR.md`, `docs/plaene/2026-10-24-batches/umsetzung.md`, `docs/plaene/2026-10-24-batches/review-b6.md`, `native/Cargo.toml`, `native/Cargo.lock`, `native/src/lib.rs`, `native/src/app/mod.rs`, `native/src/app/core/frame_update.rs`, `init.rs`, `screen_launch.rs`, `login_viewer.rs`, `login_viewer_image.rs`, `screen_texture_cpu.rs`, `screen_wgpu_yuv.rs`, `screen_view_widget.rs`, `screen_tile_grid.rs`, `screen_tile_face.rs`, `screen_viewer_app.rs`, `screen_viewer_view.rs`, `screen_scaling.rs`, `screen_presented.rs`, `screen_session.rs`, `screen_decode_thread.rs`, `screen_render_caps.rs`, `family_viewer.rs`, `batches_preview_task_tests.rs`, `native/src/screen/core/frame_budget.rs`, `frame_size_caps.rs`, `frame_memory.rs`, `frame_memory_tests.rs`, `native/src/screen/os/shared/session_memory.rs`. Die verkürzten Dateinamen nach einem Verzeichnispfad gehören jeweils zu demselben zuvor genannten Verzeichnis. Der zuerst freigegebene, nicht vorhandene Pfad `native/src/app/core/mod.rs` wurde an Main gemeldet; Main ersetzte ihn durch `native/src/app/mod.rs`.

**Gelesene lokale Primärquellen:** eframe 0.29.1 `src/epi.rs`, `src/native/glow_integration.rs`, `src/native/wgpu_integration.rs`; egui 0.29.1 `src/context.rs`; egui-wgpu 0.29.1 `src/renderer.rs`, `src/winit.rs`; egui_glow 0.29.1 `src/painter.rs`; epaint 0.29.1 `src/image.rs`, `src/texture_handle.rs`, `src/textures.rs`. Zusätzlich gelesen: `/tmp/smartexplorer-renderer-plan-scope.json` und die ausdrücklich verlinkten externen Primärquellen. Die vom Nutzer übermittelten AGENTS-Anweisungen wurden angewandt; die lokale AGENTS-Datei wurde in diesem Unterauftrag nicht erneut geöffnet.

**Angelegt/geändert:** ausschließlich `/tmp/smartexplorer-renderer-retirement-plan.md`.

**Entscheidungen:** ausdrücklich besessene Rendererressourcen über vorhandene APIs; ein begrenztes Uploadticket je Platz; eigenständiger Glow-Painter als begründete Lösung der unveränderlichen äußeren Painter-API; eindeutige Sitzungsinstanz; Retirement an allen zwölf aufgeführten Übergangsarten; separate Decoder-/Transport-Laufzusage; keine Änderung der 24-Bild- oder 4K-Verträge.

**Offene Punkte:** Produktimplementierung, statische Prüfung des konkreten neuen Anschlusses und die eine Remote-Suite stehen aus. Die zunächst fehlende eframe-Rückkehrquelle wurde als gebundene upstream-Primärquelle nachgelesen; es verbleibt keine behauptete Freigabe allein aus einem GUI-Tick, einem Callback-Drop oder einem Glow-Context-Weak. Ein GL-Retirement über `glFinish` kann auf einem Kontext ohne Sync-Objekte die GUI bis zum tatsächlichen GPU-Abfluss blockieren. GPU-Hängern wird keine künstliche Fertigstellung zugeschrieben.

## Integrationsentscheidung des Hauptagenten

Am 2026-10-26 ist dieser Detailplan nach Quellenabgleich für die Umsetzung
freigegeben. Die WGPU-Native-Registrierung verlangt laut gebundener
`egui-wgpu`-Quelle tatsächlich `Rgba8UnormSrgb`. Der äußere Glow-Painter
stellt nach einem eingebetteten Callback seinen GL-Zustand wieder her.

Für Retirement gilt ausdrücklich: Der letzte Uploadbeleg allein reicht nicht,
wenn die Textur danach noch gezeichnet wurde. Die Freigabe benötigt einen
neuen Queue-Abflussbeleg hinter dem letzten eingereichten Renderpass; der
Owner entfernt seine Registrierung an der beschriebenen GUI-Lifecyclegrenze
und erstellt anschließend den zugehörigen Retirement-Submit. Die letzte
MemoryHold bleibt bis zu dessen tatsächlicher Bestätigung erhalten.

Rendererregistrierung ohne Kontext darf keinen Präsentationserfolg liefern.
Deterministische Treiberfixtures belegen nur die produktive Zustands- und
Receiptverkabelung. Tatsächlich fehlende GL-/WGPU-Ausführung wird in der
gemeinsamen Suite als fehlender Backendnachweis ausgewiesen; kein simuliertes
Ereignis wird als realer Grafiktreiberlauf ausgegeben.


## F3-Nachplan: GL-Fehler bis zum Präsentationsbeleg, 2026-10-26

Der normale Login-Batchreview belegt einen fehlenden Fehleranschluss zwischen
Glowtexturupload und Loginpräsentation. Im Ausgangsstand bestätigte der Renderer
ein Residentbild auch bei GL_OUT_OF_MEMORY. Der vollständige Befund steht in
`review-login-bildspeicher.md`, F3.

Erste Bestandsplanung: Upload und Draw benötigen einen nach außen sichtbaren
Erfolg; fehlgeschlagene Uploads behalten ihre tatsächlichen Besitzclaims bis
zum bestätigten nativen Ende und erhalten keinen gültigen Präsentationsbeleg.
Bestehende Bilder, Größenwechsel, Clip, Sampling, sRGB-/Ditheringverhalten und
Bildschirm-/Loginbedienung bleiben an denselben Renderer angeschlossen.

Quellenabgleich: Die gepinnte egui_glow-Version 0.29.1 konsumiert GL-Fehler im
Debugmacro und gibt sie nicht an den Aufrufer weiter. Ein get_error hinter dem
Painter ist daher nicht hinreichend. Ein eigener vollständiger GL-Painter
würde Shader-, Format-, Clip- und Zustandslogik duplizieren. Gewählt ist eine
kleine additive Fehlerweitergabe im gepinnten Vendorcrate: eine ausdrücklich
aktivierte Prüfscope muss auch intern konsumierte Fehler speichern und
Upload-/Drawresultate an den Aufrufer geben. Der bisherige Standardaufruf des
eframe-Painters bleibt verfügbar. Versionssprung und ein zweiter Renderer sind
für diesen Fehler nicht erforderlich.

Detailmeilenstein F3a: additive fallible Operationen am gepinnten Painter;
CreateTexture-Fehler dürfen im aufgerufenen Fehlerpfad nicht panicen. Abnahme:
ein intern gelesener GL-Fehler bleibt für die aufrufende Operation nachweisbar,
in Debug und Release; erfolgreiche Operationen bleiben erfolgreich.

Detailmeilenstein F3b: gemeinsamer Glow-/CPU-Resident- und Receiptanschluss.
Abnahme: fehlgeschlagener Upload oder Draw quittiert nie den neuen FrameToken;
Residency/Retirement halten tatsächliche Bytes bis zum Ende; nach einem
abgewiesenen Versuch wird kein alter Texturinhalt als neues Frame behandelt.

Detailmeilenstein F3c: gezielte Quellenfixtures im bestehenden Rendererabschnitt
der einzigen Remote-Suite sowie statischer Abgleich aller betroffenen
Bildschirm-/Loginaufrufer. Keine lokale Suite oder separate Laufzeitkampagne.

Zweite Lückenrecherche: Khronos glGetError setzt ein gelesenes Flag zurück,
mehrere Flags verlangen wiederholtes Lesen. Bei GL_OUT_OF_MEMORY ist der
GL-Zustand nicht als unverändert belegt. Ein Fence bestätigt lediglich Ende.
Die konkrete Fehlerbehandlung muss diese Fälle einschließlich Endbesitz
abbilden; ihre Detailentscheidung wird vor dem Code im Autorbefund festgehalten.
Primärquellen (Abruf 2026-10-26):
[egui_glow lib.rs 0.29.1](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui_glow/src/lib.rs),
[Painter 0.29.1](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui_glow/src/painter.rs),
[Khronos glGetError](https://raw.githubusercontent.com/KhronosGroup/OpenGL-Refpages/main/gl4/glGetError.xml).

### F3-Autorbefund und Fehlerentscheidung vor dem Produktedit

Stand 2026-10-26. Main lieferte das read-only Dokumentationsgate: HEAD
`7ae228ff`; in dieser Schreibfläche ist der vorstehende Main-Nachplan geändert,
das gehashte Vendorcrate ist neu, die sechs bestehenden Rendererquellen sind
gegenüber HEAD sauber. Eigene Gitaktionen bleiben ausgeschlossen.

Der gelesene Aufrufweg ist CpuPixels → GlowBackend → Painter::set_texture →
upload_texture_srgb; anschließend installiert screen_render_cpu den Resident
und quittiert nach paint_primitives. Das vorhandene Debugmacro liest genau ein
GL-Flag und verliert diesen Fehler für den Aufrufer. Release liest dort gar
kein Flag. Die gepinnte glow-0.14.2-native Quelle verschärft den Createfall:
create_texture benutzt NonZeroU32::new(name).expect, obwohl die öffentliche
Signatur Result liefert. Ein GenTextures-Ergebnis null erreicht deshalb die
äußere Resultbehandlung nicht. catch_unwind würde den Panic-Hook trotzdem
auslösen und wird nicht als Lösung eingesetzt.

Gewählt ist eine additive, ausdrücklich aktivierte Fehler-Scope im bestehenden
Painter. Interne Macrochecks und der Operationsabschluss speichern alle
gelesenen Flags auch im Releasepfad. Mehrere Flags werden bis NO_ERROR gelesen;
Contextverlust oder wiederholte nicht abfließende Fehler werden als unsicher
behandelt. Die bisherigen unprüfenden Painteraufrufe bleiben verfügbar. Der
geprüfte Upload erhält einen kleinen falliblen Texturecreator: derselbe native
GL-Kontext liefert glGenTextures über CreationContext::get_proc_address, und
null wird vor dem NativeTexture-Aufbau als Fehler zurückgegeben. Shader,
Texelformat, Sampling, Clip, VBO-/VAOaufbau und Drawbefehle bleiben beim Painter.

Bei normalem Uploadfehler wird kein Resident des neuen Pakets installiert.
Ein bereits versuchter Überschreibvorgang macht auch das bisherige Paket
dieser Textur als Darstellungsbeleg unbrauchbar: erfolgreiche Teilbefehle
dürfen nicht als Rückkehr zum alten Inhalt interpretiert werden. Der echte
Retirementbesitzer hält neues Paket, Uploadclaim, Texturclaim und den
überschriebenen Vorgänger bis zum gesonderten Abfluss nach Löschung. Erst danach
darf der gültige Kandidat erneut mit einer frischen Textur versucht werden.
Drawfehler entfernen entsprechend den betroffenen Resident, halten seine
Claims im Retirement und erzeugen keinen Receipt; das gilt auch für Altbild-
Fallbackdraws. Ein erfolgreicher Upload allein ersetzt keinen erfolgreichen
Draw und kein Fence wird zum Inhaltsnachweis umgedeutet.

GL_OUT_OF_MEMORY, Kontextverlust und unzuverlässige Fehlerzustände sperren
weitere Glowuploads und -draws dieser Instanz. Die Khronosquelle erklärt nach
OUT_OF_MEMORY den GL-Zustand ausdrücklich für undefiniert. Deshalb beweisen
nachfolgende Finish-/Fenceereignisse dann weder unveränderte Altbilder noch
zuverlässig gelöschte Ressourcen. Diese Instanz behält die tatsächlichen
Ressourcenclaims bis zur konkreten run_native-Rückkehr. Fehler im Abbau- oder
Fencepfad werden ebenfalls nicht in erfolgreiche Freigaben umgedeutet.

Arbeitsfolge und Abnahme: zuerst additive Scope-/Painterfehler, danach
transaktionale Resident-/Retirement-/Receiptverkettung, dann gezielte
Quellenfixtures für intern konsumierte Fehler, null-Create, fehlgeschlagenen
Besitzerwechsel, Drawfehler und OOM-Endbesitz. Abschließend werden die
betroffenen Bildschirm-/Loginaufrufer und die unveränderten Renderbefehle
statisch abgeglichen. Der vorhandene WGPU- und W/24-Vertrag bleibt erhalten.
Lokale Compiler, Formatter und Tests sind nicht autorisiert; echte GL-
Fehlerinjektion, Remoteausführung und Release bleiben fehlende Laufzeitbelege.

Erneut gelesene Primärbelege am 2026-10-26: gepinntes Vendor-painter.rs und
lib.rs, glow-0.14.2/native.rs::create_texture/non_zero_gl_name/get_error,
eframe-0.29.1/epi.rs::CreationContext::get_proc_address sowie
[Khronos glGetError](https://raw.githubusercontent.com/KhronosGroup/OpenGL-Refpages/main/gl4/glGetError.xml)
und [Khronos glFenceSync](https://raw.githubusercontent.com/KhronosGroup/OpenGL-Refpages/main/gl4/glFenceSync.xml).

### F3-Quellkorrektur und anschließender statischer Abgleich

Die additive Fehler-Scope ist im gepinnten Vendor angeschlossen. Der geprüfte
Pfad verwendet try_set_texture_with und try_paint_primitives; normale
Painteraufrufe bleiben erhalten. GenTextures=null erreicht den NativeTexture-
Konstruktor nicht. GlErrorScope sammelt intern konsumierte Fehler unabhängig
vom Debugprofil und hält dieselbe Operation am selben Kontext/Thread gebunden.

GlowUpload trägt Ergebnis, konkreten Bildbesitzer und eigenen Uploadabfluss.
finish_glow_upload installiert ausschließlich erfolgreiche Uploads. Bei einem
Fehler hält Retirement auch das überschreibversuchte alte Paket; dessen Inhalt
wird nicht als wiederhergestellt ausgegeben. finish_glow_draw sperrt den
Receipt bei Drawfehlern und verschiebt den Resident einschließlich Upload-
und Texturclaim in dasselbe echte Retirement. Das gilt auch für Fallbackdraws.
Ein unzuverlässiger Kontext gibt weder durch service noch shutdown vorzeitig
Claims frei; der bestehende konkrete run_native-Rückkehrweg bleibt sein Ende.

Der statische Abgleich mit dem vor dem Edit gelesenen Stand ergab identische
Quellen für prepare_painting, paint_primitives, paint_mesh,
upload_texture_srgb, set_clip_rect, destroy_gl, destroy, clear sowie Filter-
und Wrapparameter. Der Color-/Font-/Bind-/Uploadkörper im gemeinsamen
Texturpfad, das bisherige GL-Errorlogging und der WGPU-CPU-Drawzweig sind
ebenfalls identisch. screen_render_upload.rs blieb unverändert. Die tatsächlich
berührten Login-/Preview-/Full-Aufrufer verwenden weiterhin CpuTexture und den
gemeinsamen Receipt; die von Main ergänzte Fensterfokusbindung bleibt erhalten.

Die ergänzten Quellenfixtures bilden gespeicherte interne GL-Fehler, mehrere
Flags, verschachtelte Scope-/Kontextfehler, null-Create, erfolgloses
Überschreiben, Upload-Erfolg mit Drawfehler und OOM-Endbesitz ab. Sie liefern
ausdrücklich nur kontrollierte IO-/Besitzzustände. Main hat die eng gefilterten
Vendor-Libraryfälle und Appfälle demselben Remote-Suiteeinstieg zugeordnet.
Tree-sitter meldete für die vorhandenen Rust-Schreibdateien keine Syntaxfehler;
die statische Prüfung meldete keine nachgestellten Leerzeichen. Das ist kein
Compiler-, Typ-, Formatter-, Test- oder GL-Treiberbeleg. Echte Fehlerinjektion,
Remoteausführung und Release bleiben außerhalb dieses begrenzten Auftrags.
