# P4a/c/d: Hostvertrag und Integrationsnachweis

Stand: 2026-10-25. Dies ist ein Quelltextkandidat. Es wurden keine Builds,
Compiler, Formatter, Tests, Server, Commits, Pushes oder Graphneubauten ausgeführt.
Eine erfolgreiche Laufzeit oder vollständige Produktintegration ist nicht belegt.

## Ausgangsphase: Ziel, Bestand und Entscheidungen

Der erste delegierte Auftrag begrenzte Änderungen auf dieses neue Modul. Die
anschließenden Freigaben für Host-/Workerintegration und Viewer stehen in den
Ergänzungen dieses Berichts. P4b, Portal-/Androidhostanschlüsse und P4e gehören
weiterhin zum Integrationsauftrag des Hauptagenten. Der freigegebene Plan ist `docs/plaene/2026-10-24-batches/batches.md`
P4 und `vertraege.md` V6. Normale Desktopansicht und sitzungsweite Steuerrechte
bleiben als bestehende Wege bestehen.

Der zu Beginn gelesene Bestand verwendet rohe HWND-/XID-Quellen. Die bestehenden
`input_send`-/`x11_input`-Adapter bestätigen vor ihrer Injektion keinen passenden
Fensterfokus. Der neue Vertrag liefert deshalb eigene Fernidentitäten und
ergänzt die bestehenden Capture-/Inputwege, statt deren Rechte umzudeuten.

Die umgesetzten, statisch nachvollziehbaren Meilensteine sind:

1. Ein vollständiger versionierter Snapshot mit Besitzergenerationen, App-ID,
   Iconreferenz, Titel, Geometrie, Z-Reihenfolge, Sichtbarkeit, Fokus,
   Minimierung und verfügbaren Operationen. Verlorene Ereignisse können durch
   einen vollständigen Snapshot ersetzt werden. Der JSON-Adapter weist
   ungültige Besitzerbäume und Familien über dem bestehenden 1-MiB-Budget ab.
2. HostSession mit frischer 128-Bit-Familiennonce, monotonen Mitgliedsgenerationen,
   aktuellen Rechte-/Listenscopeprüfungen und OS-Lebensdauermarkern.
   Wiederverwendete Handles erhalten keine alte Fernidentität.
3. Windows-GW_OWNER-/Prozess-/Sitzungszuordnung, tatsächliche Fokusbestätigung,
   asynchrone Größenanforderung und reguläres WM_CLOSE. Systemmenüs ohne
   GW_OWNER werden ausschließlich mit belegtem GetGUIThreadInfo-Menübesitzer
   zugeordnet. Nicht zuordenbare Oberflächen werden mit ihrer nativen ID gemeldet.
4. X11-WM_TRANSIENT_FOR mit XRes-1.2-Client-PID und lokalen UID-/Prozessstart-/
   Sitzungstatsachen. Größenänderung verwendet angekündigtes EWMH. Schließen
   sendet ausschließlich angekündigtes WM_DELETE_WINDOW: _NET_CLOSE_WINDOW
   wurde verworfen, weil dessen WM-Verarbeitung einen Kill-Fallback erlauben kann.
5. Separate WGC-/XComposite-Mitgliedsquellen auf einer opaken schwarzen
   BGRA-Leinwand. Kein Ausschnitt aus einer Desktopaufnahme. Grenzen werden nach
   Aufnahme erneut geprüft. Geänderte Mitglieder und Geometrien verändern die
   Referenzgeneration auch bei unveränderter Leinwandgröße.
6. Typisierte Grenzen für die tatsächlich gewählte Portalquelle und
   Android-Einzelappprojektion. Daraus werden keine Größen-/Close-/Fokusrechte
   und keine beliebige Appwahl abgeleitet.

Ein Pollingadapter wurde gegenüber einem erfundenen universellen nativen
Fensterereignisprotokoll gewählt: jeder Abruf aktualisiert die OS-Tatsachen,
Snapshots bleiben vollständig. HWND-/XID-Werte, Titel und PID allein sind keine
Lebensdaueridentität. WGC IncludeSecondaryWindows ersetzt keine unabhängigen
Dialogflächen, weil diese Option an der Hauptfenstergrenze beschneidet.

## Ursprünglicher Modulanschluss und inzwischen erfolgte Hostintegration

Die folgende Liste dokumentiert den ursprünglichen Modulvertrag. Die
Host-/Worker-/Protokollpunkte sind durch den Produktionsanschluss weiter unten
implementiert. Die dort ausdrücklich genannten Client-/UI-/Bridgeanschlüsse
bleiben getrennt zu integrieren.

- Rootregistrierung: `pub(crate) mod remote_window;` am tatsächlichen
  Bibliothekseinstieg. Es wurde kein Bestandsmodul geändert.
- `HostSession::open_bound(session, binding, adapter, authority)` übernimmt den
  bereits beim Listing gepinnten Hostbeleg. Es validiert und bindet niemals neu.
  `open(native)` ist ausschließlich eine neue hostseitige Quellwahl; eine alte
  rohe `SourceId::Window` ist kein Beleg einer vorherigen Listengeneration.
  Ein Listing-/Auswahlweg muss den Adapter samt Binding im Host/Worker halten.
- `WindowAuthority` an die bestehende aktuelle ScreenView-/ScreenControl-
  Zulassung und den tatsächlichen Listenscope anschließen. ScreenControl bleibt
  ein Recht für die interaktive Sitzung. Die Familie erzeugt keine Appisolation.
- `snapshot`/`refresh` über den authentifizierten Sitzungsweg zugänglich machen.
  Pro Peer die zuletzt gesendete Familienrevision halten: Captureaufrufe können
  den internen Snapshot bereits aktualisiert haben. `previous_revision` ist
  ein beobachteter Vorzustand und keine Empfangsbestätigung des Peers.
- Für jede Operation aktuelle `WindowIdentity` verwenden. Resize/Close liefern
  nur `Requested`; die Anwendung beziehungsweise der WM entscheidet über das
  Ergebnis. Der nächste Snapshot ist maßgeblich. Close beendet keine Verbindung,
  damit ein Speichern-Dialog sichtbar und bedienbar bleiben kann.
- Vor der ersten Eingabe `operate(identity, Focus, authority)` erfolgreich
  bestätigen. `with_input(identity, snapshot_revision, authority, closure)`
  führt bei Fokusabweichung keine automatische erneute Aktivierung aus. Nach
  beobachtetem Fokusverlust bleibt Input bis zur bestätigten Focusoperation
  gesperrt. Bei jeder Gate-/Injectablehnung gehaltene Eingaben über den
  bestehenden PressedInputs-Weg freigeben und den konkreten Zustand anzeigen.
- Maus/Touch mit `CaptureBoundary::map_pointer` dem sichtbaren Eingabemitglied
  zuordnen; Zwischenraum erzeugt kein Ziel. Tooltips werden dargestellt, aber
  für die Eingabezuordnung übersprungen. Keyboard an das aktuell bestätigte
  Fokusmitglied richten. Der bestehende Injector muss unmittelbar im Gate
  bleiben und seine Secure-Desktop-/UIPI-/SAS-/XTestprüfungen behalten.
- `FamilyScreenSource::new` nimmt `SharedHost`, `SharedAuthority`, die
  Fernidentität, einen Raw-Source-Callback und ausgehandelte `FrameLimits`.
  Der Callback öffnet die tatsächliche Plattformquelle im gleichen berechtigten
  Hostworker mit bestehendem Indikator. Er darf nicht wieder in den
  Familienwrapper auflösen. Quelle/Encoder dürfen gemeinsam deaktiviert werden,
  ohne die autorisierte Quelle auszutauschen.
- ScreenSource::reference_generation ist inzwischen von Main ergänzt worden;
  FamilyScreenSource überschreibt den Hook. Main muss dessen Wechsel nach
  Capture in Pipeline und Worker zur Referenzinvalidierung durchreichen.
  Eine erkannte Grenzänderung ist vor Frame-, Unchanged- oder Paused-Rückgabe
  sichtbar; wiederholtes Paused für dieselbe beobachtete Grenze hält den Wert.
- Das von Main ergänzte x11rb-Feature `res` ist erforderlich. Windows und X11
  verwenden `screen::os::app_icons::window_identity(native)` von Main. Die
  Linux-Desktopfile-ID wird einmal pro gepinnter Quelllebensdauer aufgelöst;
  P3 aktualisiert Iconbytes und Hash unabhängig davon. `set_icon` akzeptiert nur
  die passende aktuelle App-ID und den vorhandenen begrenzten Iconvertrag.
- Der von Main ergänzte Hook `screen::os::window_control_block` wird vor
  Windows-Fokus, Resize, Close und Fokusbestätigung geprüft. Geschützte oder
  ungeklärte Desktops und inaktive oder ungeklärte Sitzungen geben keine
  Fernfensteroperation frei; der Snapshot deaktiviert die Operationen.
- PortalSelection nur aus dem erfolgreichen echten Start, den gelieferten
  Geräterechten, der logischen Streamgröße, Sitzung und verfügbarem
  pipewire-serial erzeugen. Ohne Serial verlangt `validate_stream` denselben
  noch offenen Stream; Wiederöffnung nach Verlust braucht eine neue Sitzung.
  Lebensdauer/Closed-Signal und Berechtigungsentzug muss der echte Portaladapter
  durchsetzen. Notify und EIS werden über einen festgelegten Inputpfad getrennt.
  AndroidSelection verlangt die aktuelle Einzelappprojektionsgeneration; eine
  angefragte Package-ID wird nur bei tatsächlicher Systembestätigung akzeptiert.

## Verbleibende Laufzeitgrenzen

Die Fokusprüfung von SendInput/XTest ist keine Sicherheitsisolation und keine
atomare Transaktion mit der späteren OS-Injektion. Win32 bietet hier keinen
gepinnten Fremdfensterhandle, der Windowzerstörung und asynchrone Nachrichten
atomar mit dem Markercheck verbindet. Eine verbleibende Änderung zwischen dem
letzten Check und OS-Aufruf kann deshalb nicht ausgeschlossen werden. Der Code
verspricht keine stärkere Garantie. X11-Resize/Close prüfen Marker und Anfrage
in einer kurzen Servertransaktion; der bestehende separate XTestkanal bleibt
sitzungsweit. Fremde X11-Clients können innerhalb derselben X-Sitzungsrechte
Eigenschaften verändern; die Familie ist kein X11-Sicherheitscontainer.

Nicht eindeutig zugeordnete Tooltips/Popups und prozessfremde Dialoge werden
nicht mitgefilmt. Die Liste meldet die Grenze; Main muss lokale Dialogbedienung
oder ausdrücklich gewählten vollständigen Desktop anbieten. Fehlendes XRes,
nichtlokale X11-Clients und ein anderer lokaler Benutzer ergeben konkrete
Ablehnungen des neuen Fernfensterwegs. Die vorhandene normale Bildschirmansicht
bleibt davon getrennt. Native X11-Fernfenster werden unter Wayland nicht über
XWayland angeboten.

WGC-/DWM-Geometrie, tatsächliche Capturefähigkeit jeder Menü-/Tooltipfläche,
modaler Speichern-Dialog, WM-Resizekonventionen, DPI-/Monitorwechsel,
Rechteentzug, Sitzungswechsel und reale Portal-/Androidzustände sind nicht
praktisch geprüft. Eine abweichende Surfacegröße pausiert die neue Familie
anstatt falsch platzierte Pixel zu veröffentlichen. Die Hauptintegration,
Oberflächen, IPC-/Netzrahmen, Graphaktualisierung und Veröffentlichung bleiben
Aufgaben des Hauptagenten.

## Primärquellen und APIstände

Am 2026-10-25 abgerufen und für die Entscheidungen verwendet:

- Microsoft Win32: [SetPropW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setpropw),
  [SetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow),
  [SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos),
  [WM_CLOSE](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-close).
  Propertyzugriff unterliegt UIPI; Foreground kann verweigert werden; Close
  erlaubt der Anwendung ihren eigenen Bestätigungsdialog.
- [WGC IncludeSecondaryWindows](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.includesecondarywindows?view=winrt-26100),
  Windows 11 24H2 / UniversalApiContract 19: kein Ersatz für selbständige Flächen.
- [X11-Protokoll X11R7.7](https://www.x.org/releases/X11R7.7/doc/xproto/x11protocol.html),
  [ICCCM 2.0](https://www.x.org/releases/X11R7.7/doc/xorg-docs/icccm/icccm.html),
  [EWMH Root Messages](https://specifications.freedesktop.org/wm/latest/ar01s04.html)
  und [Root Properties](https://specifications.freedesktop.org/wm/latest/ar01s03.html).
- [XRes 1.2](https://www.x.org/releases/current/doc/resourceproto/resproto.txt),
  einschließlich der dortigen Kennzeichnung als Entwurf: Ressourcen-XID
  identifiziert den tatsächlichen Client; LocalClientPid kann fehlen.
- [ScreenCast v6](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html);
  die weitergehenden Portal-/Androidbelege sind im gelesenen freigegebenen
  `recherche.md` R6 vorhanden und ersetzen keinen eigenen Laufzeitbeleg.
- Versionsgebundene veröffentlichte Quellen von windows-sys 0.59.0 und
  x11rb/x11rb-protocol 0.13.2 aus static.crates.io wurden ausschließlich im
  Speicher gelesen. Sie bestätigen die verwendeten Signaturen einschließlich
  ProcessIdToSessionId unter System::RemoteDesktop und XRes ClientIdSpec.
  Es wurden keine Pakete installiert oder kompiliert.

## Dateinachweis des ursprünglichen Modulauftrags

Neu erstellt beziehungsweise im Verlauf dieses Teilauftrags geändert:
`mod.rs`, `core/mod.rs`, `core/model.rs`, `core/host.rs`, `core/selection.rs`,
`core/wire.rs`, `os/mod.rs`, `os/windows.rs`, `os/windows/facts.rs`, `os/x11.rs`,
`os/x11/properties.rs`, `os/shared/family_source.rs`, diese `INTEGRATION.md`.
Sämtliche Pfade beziehen sich ausschließlich auf `native/src/remote_window/`.

Zusätzlich tatsächlich gelesen oder gezielt durchsucht:

- Nutzerkontext mit vollständigen Repositoryregeln; `AGENTS.md` nochmals im
  kombinierten Dateiaufruf. `docs/ARCHITEKTUR.md`; P4-/V6-/R6-Ausschnitte und
  Suchtreffer in `docs/plaene/2026-10-24-batches/{batches,vertraege,recherche}.md`;
  der kombinierte Abruf von `agentenvertrag.md` war gekürzt und gilt nicht als
  vollständiger Befund dieses für P4 nicht maßgeblichen Vertrags.
- `docs/oberflaechen/bildschirm.md` gezielte Suchtreffer; `native/Cargo.toml`
  Plattformdependencies; `native/Cargo.lock` x11rb-/getrandom-Versionen.
- Unter `native/src/screen/core/`: `window_list.rs`, `input_model.rs`,
  `host_backend.rs`, `pixel_convert.rs`, `app_icon.rs`, `host_pipeline.rs`,
  `host_stage.rs`, `session_state.rs`, jeweils der für den neuen Vertrag
  benötigte Inhalt beziehungsweise die gezielten Signaturstellen.
- `native/src/screen/os/mod.rs`; Windows `window_list.rs`, `input_send.rs`,
  `capture_source.rs`, `host_backend.rs`; Linux `x11_windows.rs`, `x11_input.rs`,
  `host_backend.rs`, `portal_session.rs`, `session_detect.rs`; Android
  `bridge_backend.rs`. Kombinierte Ausgaben waren teilweise gekürzt; daraus
  werden keine vollständigen Lesebehauptungen abgeleitet.
- `native/src/share/core/{access_constraints,access_grant}.rs` gezielte
  Rechte-/Identitätsdefinitionen.
- Nach ausdrücklicher Flächenerweiterung von Main:
  `native/src/screen/os/windows/app_icons.rs` und
  `native/src/screen/os/linux_os/app_icons.rs` jeweils Identityhook und dessen
  unmittelbar benötigte Signaturstellen. Diese Dateien wurden nicht geändert.
- `native/src/screen/os/windows/secure_desktop.rs` gezielte Export-/Signatur-
  und Importstellen; der Bestandsadapter bleibt ausschließlich bei Main.
- Lokaler Arbeitsweise-Skill und gelieferte MCP-Anweisungen. Die im Brief
  genannte `native/src/screen/core/source.rs` existiert nicht; der tatsächliche
  ScreenSource-Vertrag steht in `host_backend.rs`.

## Produktionsanschluss des Host-/Workerpakets

Der Folgeauftrag vom 2026-10-25 verbindet nun die tatsächliche authentifizierte
Fensterliste, HELLO-/SET_MODE-Auswahl, Workeraufnahme und den Kontrollkanal.
Die bestehende rohe Monitor-/Window-/Portal-/Androidsystemauswahl bleibt als
eigener Weg erhalten. `family_version: Some(1)` wird in ServerHello,
ClientHello und HelloOk ausgehandelt.

`family_listing::list(context,principal)` läuft unter der bestehenden aktuellen
ScreenView-Zulassung. `FamilyListing` enthält `FamilyListEntry` mit `source`,
`identity`, optionalem `launch_request` und vorhandener WindowEntry-Metadatenform.
Der Selector ist `window:@family.<32hex>` und wird durch
`family_protocol::selector` gelesen. Er ist an Relation, Device, Node und PublicKey
des bewiesenen Principals gebunden. Pro Principal ersetzt ein neues normales
Listing seine noch nicht gewählten normalen Leases. Aktive Quellen und echte
Appstartregistrierungen werden dadurch nicht ausgetauscht.

Das Listing pinnt die native Inkarnation und prüft anschließend die tatsächlichen
Fenstermetadaten. Ein OS-Adapter pro Listing hält dessen Pins; X11 erzeugt nicht
einen zusätzlichen Serverclient pro Fenster. Die vollständige Familienenumeration
erfolgt erst für die gewählte Quelle. Metadaten-/Iconzuordnung ist niemals der
Inkarnationsbeweis.

`ScreenHostBackend::open_session_source(owner,session,source,wanted,control)`
verbraucht den passenden Selector genau einmal. Über privaten Worker-stdin
überträgt `OpenFamily` den `WindowPin {platform,namespace,binding}`. Der Worker
adoptiert denselben Marker, validiert den ursprünglichen Binding und übernimmt
dieselbe öffentliche Familiennonce. Er bindet keine rohe HWND/XID neu. Erst nach
bestätigtem OPEN_FAMILY relinquiert der Katalog die Markerbereinigung. HELLO und
SET_MODE prüfen nach blockierender Sourceöffnung Stop und aktuelle Rechte erneut.

Der neue `host_family`-Dispatch verwendet den `ScreenInputSink` der tatsächlichen
Pipelinequelle. Er verwendet für Familien keinen globalen „neuesten Controller“.
Wartende Focus-/Resize-/Close-/Inputjobs erhalten einen Abbruchbeleg, der nach
Erwerb des Workerlinks unmittelbar vor der Übermittlung geprüft wird. Ein
Sessionende oder Rechteentzug widerruft diesen Beleg. Bereits an das OS
übermittelte Fensteranforderungen lassen sich nicht nachträglich zurücknehmen.

Die Netzwerkrahmen sind:

| Richtung | Tag | Vertrag |
|---|---:|---|
| Viewer → Host | 28 | `ListFamilies` |
| Viewer → Host | 29 | `FamilySnapshot` erneut abrufen |
| Viewer → Host | 30 | `FamilyOperation {request_id,identity,operation}` |
| Viewer → Host | 31 | `FamilyInput {target,events}` |
| Host → Viewer | 41 | `Families(FamilyListing)` |
| Host → Viewer | 42 | vollständiger `FamilySnapshot` und aktuelle Ereignisse |
| Host → Viewer | 43 | `FamilyOperationResult` mit aktuellem Snapshot |
| Host → Viewer | 44 | `FamilyInputResult` mit konkretem Blockgrund |

Familiensnapshots haben eine allgemeine `revision` und eine getrennte
`layout_revision`. Titel, Icon und Fokusmetadaten erzwingen keinen neuen GOP.
Mitglieds-, Geometrie-, Z-Reihenfolge-, Sichtbarkeits- und Minimierungsänderungen
setzen dagegen Capture-/Pipelinegrenze zurück. Der Host sendet beim Einstieg,
nach Captureänderungen, auf Abruf und im Statuspoll vollständige Snapshots.
Vor Eingabe müssen die gesendete Snapshotrevision, die aktuelle ausgewählte
Familie, das Steuerrecht und die tatsächlich dargestellte aktuelle Referenz
übereinstimmen. Raw INPUT ist für Familienquellen gesperrt. Maus/Touch werden
mit `FamilySnapshot::pointer_target` von der Familienleinwand auf das Mitglied
abgebildet; ein Batch enthält Koordinaten genau dieses Mitglieds. Keyboard
verwendet `focused_target`.

Freigaben gehen über einen getrennten Release-Workerrahmen. Sie enthalten keine
Zeigerposition, keinen Text und keine Systemaktion, aktivieren kein Fenster und
bleiben an den ursprünglichen Worker gebunden. Ein verschwundenes Appfenster
verhindert ihre Zustellung nicht allein wegen seiner Geometrie. Der Familienworker
behält möglicherweise gedrückte Eingaben, bis ihre Freigabe tatsächlich gelingt;
eine mögliche teilweise native Injektion löst eine kompensierende Freigabe aus.
Sichere Desktops und Integritätsregeln bleiben wirksam.

`register_family_pin(owner,request_id,pin) -> io::Result<FamilyListEntry>` ist der
explizite Anschluss für echte Appstartkorrelation. Der erfolgreiche Store liefert
atomar den wirklichen Entry. Erst danach darf der Originalowner `transfer_pin`
aufrufen. Ein Fehler vor dem Store lässt die ursprüngliche Markerbereinigung beim
Originalowner; nach dem Store folgt kein fallibler Schritt. Derselbe Request mit
demselben Pin erhält denselben historischen Entry, auch wenn dessen Selector
bereits verbraucht wurde. Ein Retry erzeugt keine neue Fensterinkarnation. Der
Metadatapfad veröffentlicht laufende Registrierungen über `launch_request`.
PID-/Titelheuristiken werden nicht als Korrelation angenommen.

Der separate Reviewbefund R06 ist ebenfalls umgesetzt: `heard` in
`host_session::run` verwendet `route_protocol::active_now` und
`saturating_duration_since`. Routepausen verbrauchen keinen Viewer-Silence-Timer;
Rechtefristen behalten ihre Wandzeit.

Zusätzlich erstellt: `screen/core/{family_protocol,family_listing,host_family,
worker_family}.rs` und `screen/os/shared/family_backend.rs`. Zusätzlich geändert:
`screen/core/{host_backend,host_server,host_session,host_visibility,icon_host,wire,
wire_control,wire_hello,worker_wire,worker_frame_wire}.rs` sowie
`screen/os/shared/{host_worker_backend,host_worker_client,host_worker_main}.rs`.
Die RemoteWindow-Kern-/OS-Dateien wurden für inspect und echten Pintransfer
erweitert; `layout_revision` ergänzt den Snapshot. Die danach vom Appstartautor
übernommenen Änderungen an seinen vier freigegebenen RemoteWindow-Dateien sind
nicht Änderungen dieses Integrationsschritts.

Zusätzlich gelesen: die vorstehenden geänderten Bestandsmodule sowie
`screen/core/{mod,host_pipeline,host_capture,host_mode,host_stage,host_reference,
host_pipeline_actor,host_work_control,host_input_release,control_sender,registry,
authorize}.rs`, jeweils für den verbundenen Vertrag. Gelesene Ausschnitte werden
nicht als vollständige semantische Prüfung jeder dieser Dateien ausgegeben.

Tree-sitter/Rust hat die 30 zunächst geprüften Host-/Worker-/Moduldateien ohne
Syntaxfehler gelesen; die Abschlussprüfung wird nach dem letzten APIanschluss
erneut statisch durchgeführt. Typprüfung, Builds, Tests und echte Fensterläufe
wurden nicht ausgeführt. Main besitzt Rootregistrierungen, allgemeine
Client-/IPC-/CLI-/MCPanschlüsse, P30-Größenhooks und spätere Veröffentlichung.
Der anschließende native und Android-Familienviewer ist ein eigener aktueller
Teilauftrag, der dieses Hostpaket verwendet.
# Vieweranschluss P4, 2026-10-25

Plan A: Beauftragt sind ein eigenes natives Fernfenster und getrennt wählbare
Android-Familienansichten. Auswahl und Appstart übergeben den vollständigen
FamilyListEntry. Fenster-X fordert reguläres Close an; explizites Trennen beendet
nur den Strom. Die bestehenden Vollbild-/Gruppenansichten bleiben erhalten.
Builds, Tests, Formatierer, Installationen, Unteragenten, Commits, Pushes und
Graphläufe sind diesem delegierten Auftrag untersagt; Main integriert und liefert aus.

Plan B: screen_session und screen_decode_thread besitzen bereits die tatsächliche
PRESENTED-Bestätigung durch CPU-/GPU-Renderer. ViewWidget kann unverändert verwendet
werden, wenn ViewerSession seine Canvas-Eingaben vor Versand in FamilyInput umsetzt.
Android ScreenSession und screen_view_run besitzen dieselbe Render-/Streamgrenze;
ScreenViewScreen wird durch einen von Main anzuschließenden Familienmodus genutzt.
Die bisherige Kontrolle kennt keine bestätigte Familienaktivierung. Das neue reine
screen/core/family_viewer-Modul schließt diese Lücke für beide Clients.

Plan C: Wiederverwendung der existierenden Renderer erhält Ressourcenlimits,
Sichtbarkeitsprotokoll und wirkliche Bildbestätigung. Ein eigener Bitmap-/Video-
Renderer würde diese Verträge duplizieren und wird ausgeschlossen. Der gemeinsame
Viewerzustand korreliert FocusConfirmed mit der Anforderung; beobachteter Fokus
allein erteilt keine Eingabebewilligung. Ereignisse mit veraltetem Layout werden
verworfen. Ein kleines lokales Menü ersetzt keine Bildfläche durch eine dauerhafte
Werkzeugleiste. Wiederkehrende Eingabe bleibt eine direkte Geste; Fokus, reguläres
Close, Trennen und Diagnose sind lokale Menüauswahlen. Ein sichtbarer Menütaster
und dessen angezeigter Shortcut vermeiden einen ausschließlich erlernbaren Einstieg.

Plan D / erwartete Ergebnisse:
1. family_viewer.rs: vollständigen Snapshot an die ausgewählte Nonce binden,
   monotone Revisionen prüfen, Canvas auf Member abbilden, Fokusantwort korrelieren.
   Ergebnis: kein rohes INPUT und keine Eingabe vor FocusConfirmed.
2. Desktop family_viewer*- und family_picker*-Module plus schmale Sessionhooks:
   exakte Auswahl/Öffner, P3-Metadaten, Titel Programm–Gerät, lokales Menü,
   bestätigte Operationen, reguläres Close ohne sofortiges Trennen.
3. Mobile screen_family*-Module und ScreenSession-/ScreenSession.kt-Hooks:
   identischer Protokollzustand und vorhandene tatsächliche Bildbestätigung.
4. Android FamilyApi/FamilySession/FamilyPicker/FamilyScreen: konkrete Auswahl,
   Geräte-/Programmbezug, Familienmenü und regulärer Zurück-/Trennenweg.
5. Statischer Quellen-/ASTreview und vollständige Root-/Aufruferhaken an Main.
   Laufzeitverhalten bleibt bis zur von Main verwalteten Remoteprüfung unbestätigt.

Primärquellen für diese Ergänzung: egui/eframe 0.29.1, gebunden durch Cargo.lock;
https://github.com/emilk/egui/blob/0.29.1/crates/egui/src/viewport.rs
(close_requested/CancelClose, Title, InnerSize, Icon),
https://github.com/emilk/egui/blob/0.29.1/crates/eframe/src/icon_data.rs
(from_png_bytes → Result<IconData,ImageError>), und
https://developer.android.com/develop/ui/compose/components/menu
(DropdownMenu expanded/onDismissRequest, DropdownMenuItem text/onClick).
Abrufdatum jeweils 2026-10-25. Die menu-Quelle belegt den temporären lokalen
Auswahlweg; sie belegt keine Laufzeitprüfung der Projektoberfläche.

Viewerstand zur Integration / B8review, 2026-10-25:

- P4-Viewerzustand, Desktopfenster, Familienpicker, mobile Bridge und Android-
  Familienmenü sind geschrieben. Die zugehörigen Source-, Worker-, Codec-,
  Framebudget-, Root-, IPC- und Navigationsanschlüsse besitzt Main.
- Die tatsächliche Bildbindung verwendet jetzt
  `GopHeader.family_layout_revision == snapshot.layout_revision`. Eine neuere
  GOP-Nummer allein ist ausdrücklich kein Layoutbeleg. Desktop und mobile Bridge
  verwerfen unpassende Bildgruppen. Android prüft zusätzlich dieselbe Revision
  vor Bitmap-/Surface-Darstellung und PRESENTED. Main liefert den Wert aus der
  tatsächlichen CaptureBoundary durch Source, Worker und GOP-Header.
- Die native Ansicht verwendet den bestehenden ViewWidget mit dessen CPU-/GPU-
  Präsentationscallbacks. Sie zeigt Programm–Gerät, fordert das P3-Icon separat
  an und prüft dessen Hash/Dimensionen vor dem nativen Fenstericon. Ein kleiner
  Menütaster bleibt sichtbar; eine dauerhafte Werkzeugleiste wird nicht angelegt.
- Im aktiven Steuermodus fordert das native Fenster-X reguläres App-Close an und
  hält den Strom für Speichern-/Modaldialoge offen. Die Menüaktion Trennen,
  View-only-Fenster-X, Android-Zurück und Session-Drop schließen ausschließlich
  den Strom. Resize/Close-Antwort Requested wird als angenommene asynchrone
  Anfrage bezeichnet; die tatsächliche Geometrie und Dialoge stammen aus dem
  folgenden Snapshot/Bild. FocusConfirmed autorisiert ausschließlich die
  korrelierte konkrete Fokusanforderung.
- Menüeingaben bleiben auf ihrer Memberidentität. Bei WindowKind::Menu erfolgt
  die Fokusbestätigung auf der ausdrücklich zugeordneten Ownerkette; der Host
  prüft anschließend nochmals den tatsächlichen Menü-/Ownerfokus. Windows-observe
  besaß den passenden Menu.input-Pfad bereits. An facts.rs/windows.rs bleibt aus
  diesem Viewerauftrag deshalb keine Änderung.
- Der Appstartöffner vergleicht Request-ID, App-ID, Benutzer, OS-Sitzung, exakte
  Source und Generation. List-Families und der anschließend gestartete Stream
  verwenden die bestätigte Node-ID. Die Familiennonce folgt zwingend aus genau
  diesem Sourcebeleg. Es gibt keine PID-/Titelkorrelation. Entscheidung für den
  Appstartweg: control=true fordert die echte ScreenControl-Autorisierung an;
  die normale Familienliste lässt Ansehen/Steuern explizit auswählen.

Neu erstellt:
- native/src/screen/core/family_viewer.rs
- native/src/app/core/family_viewer.rs
- native/src/app/core/family_viewer_session.rs
- native/src/app/core/family_viewer_events.rs
- native/src/app/core/family_picker.rs
- native/src/mobile/os/shared/domains/screen_family.rs
- android/app/src/main/java/app/smartexplorer/android/api/FamilyApi.kt
- android/app/src/main/java/app/smartexplorer/android/ui/screen/FamilySession.kt
- android/app/src/main/java/app/smartexplorer/android/ui/screen/FamilyPicker.kt
- android/app/src/main/java/app/smartexplorer/android/ui/screen/FamilyScreen.kt

Bestehende Dateien im Viewerauftrag geändert:
- native/src/app/core/screen_session.rs
- native/src/app/core/screen_decode_thread.rs
- native/src/app/core/screen_launch.rs
- native/src/app/core/screen_target.rs
- native/src/mobile/os/shared/domains/screen_view.rs
- native/src/mobile/os/shared/domains/screen_view_run.rs
- android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenSession.kt
- dieser INTEGRATION.md-Bericht

Zusätzlich zum vorher dokumentierten Hostbestand für diesen Vieweranschluss gelesen:
- native/src/app/{mod.rs,screen_modules.rs}
- native/src/app/core/{screen_launch.rs,screen_target.rs,screen_viewer_app.rs,
  screen_viewer_view.rs,screen_view_widget.rs,screen_session.rs,
  screen_decode_thread.rs,screen_pool.rs,screen_devices.rs,screen_input_keys.rs,
  screen_presented.rs,screen_prefs.rs,screen_stats_overlay.rs,screen_icons.rs,
  screen_icon_requests.rs,share_apps_ui.rs}
- native/src/app/os/shared/screen_visibility.rs
- native/src/app/os/windows/screen_visibility.rs
- native/src/daemon/os/shared/{screen_ipc_client.rs,screen_ipc_families.rs,
  ipc_protocol_screen.rs}
- native/src/mobile/core/error.rs
- native/src/mobile/os/shared/domains/{screen_view.rs,screen_view_run.rs,
  screen_view_json.rs,screen_view_presented.rs}
- native/src/apps/core/model.rs
- android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt
- android/app/src/main/java/app/smartexplorer/android/core/{Core.kt,CoreEvents.kt}
- android/app/src/main/java/app/smartexplorer/android/ui/AppNav.kt
- android/app/src/main/java/app/smartexplorer/android/ui/screen/{ScreenSession.kt,
  ScreenViewScreen.kt,ScreenViewBars.kt,ScreenPreviewTile.kt,ScreenTouch.kt,
  ScreenTarget.kt}

Konkrete öffentliche Anschlüsse:
- Desktop: family_viewer::run(ScreenTarget); family_picker::panel(&mut Ui,
  &ScreenTarget), open_entry(&ScreenTarget,&FamilyListEntry),
  open_confirmed(&Context,ConfirmedAppWindow), notifications(&Context).
  Main registriert die vier app-Module sowie den WindowOpener und ruft Picker/
  Notifications auf. screen_launch erkennt die Familiensource bereits.
- ScreenTarget.expected_node bleibt für normale Quellen None. ViewerSession
  kopiert es in StreamRequest.expected_node; Main ergänzt fremde Structliterale.
- Mobile Bridge: screen.families, screen.viewFamilyState,
  screen.viewFamilySnapshot, screen.viewFamilyOperation, screen.viewFamilyInput.
  Familien-DTOs behalten snake_case; Ereignis kind=family enthält snapshot,
  notice,last_operation. kind=stream enthält familyLayoutRevision.
- Android: FamilyPicker(target,deviceName,onSelect,onBack);
  FamilyScreen(target,entry,deviceName,onBack,render). Der render-Adapter benutzt
  den bestehenden ScreenViewScreen mit family=FamilySession. FamilySession.open
  benutzt ScreenSession und die tatsächliche awaitRendererLimit-Grenze.
- FamilyWindowMenu(family,onBack,onLocalUiOpen,onKeyboard) gehört innerhalb
  desselben Dialogs über das Bild. onLocalUiOpen meldet Menü-/Diagnose-/Resize-
  Dialogzustand und false beim Dispose; Main setzt währenddessen sending=false
  und gibt gehaltene Eingaben frei. Der bisherige obere 32dp-Aufklapphandler muss
  für Familien ausgesetzt sein. Die normale Ansicht behält ihren Handler.

Statische Autorprüfung: Rust- und Kotlin-Syntaxbäume der eigenen Viewerdateien
wurden ohne verbleibende ERROR-/missing-Knoten gelesen; Whitespace-Diffkontrolle
der geänderten Bestandsdateien ohne Befund. Der Kotlin-Catch-Parserbefund wurde
mit der offiziellen Kotlin-Referenz
https://kotlinlang.org/docs/exceptions.html (try/catch/finally, Abruf 2026-10-25)
abgeglichen und durch die dortige zusammenhängende Schreibweise beseitigt.
Diese Kontrolle belegt keine Typprüfung, Kompilierung oder Laufzeitfunktion.
Es liefen keine lokalen Builds, Compiler, Tests, Formatierer, Server,
Installationen, Unteragenten, Commits, Pushes, Graphläufe oder Releases.

Verbleibende Nachweise liegen bei Main: integrierte Typprüfung auf dem Remote-
Runner, reale Fokus-/Resize-/Speicherdialog-/Reconnectabläufe, tatsächliches
Fenstericon auf Windows/X11/Wayland und Android-Renderer-/Navigationseingaben.
Die anfänglichen Win32-/X11-Rennbedingungen und die fehlende Appisolation von
sessionweitem Input bleiben wie oben dokumentiert. Portal-/Androidhostrechte
werden durch diesen Viewerauftrag nicht erweitert.

