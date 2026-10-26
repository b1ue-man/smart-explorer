# B8 – statischer Code- und Primärquellenreview

Stand: 2026-10-26. B8-R1 bis B8-R6 und die Erstgrant-Zusammenführung sind statisch abgeschlossen. Der Capture-Nachtrag bestätigt die Korrektur des X11-Rootcrop-Defekts für Familien bei erhaltenem Raw-Window-Fallback und Windows-WGC-Pfad. Keine Compiler- oder Laufzeitfreigabe.

## Auftrag und Prüfplan

**A – Auftrag:** P4/P5 einschließlich ihrer freigegebenen Integrationsgrenzen im Worktree über Planbasis `bea312ea` und HEAD `126f9325` kritisch prüfen. Nutzerauftrag laut delegiertem Originalbeleg vom 24./25. Oktober: „aufgrund des umfangs der aufgaben, ohne dass das weiteres vorgehen blockt, will ich dass du danach n astra xhigh die sachen kritisch reviewn lässt. per Batch“ und „also rein codeseitig und gegen quellen reviewen lässt“. Produktkorrekturen, Builds, Compiler, Tests, Server, Installationen, Graphneubauten, Commits, Pushes, Releases und Unteragenten sind ausgeschlossen. Schreibumfang ist ausschließlich dieses Dokument. Die tatsächliche Modellkennung ist im Reviewkontext nicht belegt; aus dem Auftragsnamen wird keine Modellkennung abgeleitet.

**B – Bestand:** Das freigegebene Manifest enthält 154 Dateien. AppHost besitzt einen dauerhaften Auftrag/Peerbeleg; die Plattformadapter behalten auf Windows das ursprüngliche Prozesshandle, auf Linux ein vor `exec` vom Kind geliefertes pidfd. P4 besitzt opaque Familienselektoren, getrennte Capturelayoutrevisionen und eine korrelierte Fokusbestätigung. Desktop, Android, IPC, CLI und MCP sind angeschlossen. Der erste Integrationsabgleich zeigt nicht übereinstimmende HostFrame-Variantennamen zwischen Wirevertrag und Betrachtercode.

**C – Entscheidungen:** Der Review verfolgt die gekoppelte Grenze Appfreigabe → OS-Start → ursprünglicher Prozesszeuge → Fensterpin → authentifizierte Liste → IPC → tatsächliche Capturefläche → dargestelltes Bild → Fokus/Eingabe. Anzeigeetiketten, PID-Werte und Quellkommentare gelten allein nicht als Identitätsbeweis. Ein Befund braucht einen konkreten Auslöser und einen belegten Kontrollfluss. Externe Verträge werden in ihrer verwendeten Version mit Primärquellen verglichen. Nicht freigegebene direkte Abhängigkeiten werden beim Hauptagenten angefragt; keine eigenständige Scopeerweiterung.

**D – Ergebnisplan:** (1) Start-/Rechtegrenze und Plattformprovenienz, (2) Pinübertragung und Capture/Input, (3) Desktop-/Android-/IPC-Anschluss, (4) Quellabgleich und priorisiertes Befundregister mit vollständigem Lese-/Schreibinventar. Jeder Punkt endet mit belegten Defekten, konkret erhaltenen Fähigkeiten oder ausdrücklich ungeprüften Grenzen. Laufzeitannahmen werden nicht als bestätigtes Verhalten ausgegeben.

## Befunde

### B8-R1 – P1: HostFrame-Varianten zwischen Wirevertrag und Betrachtern widersprechen sich

**Auslöser:** Kompilierung der integrierten Desktop- oder Mobile-Betrachterfläche.

**Beleg:** `native/src/screen/core/wire_control.rs::HostFrame` definiert `FamilyOperation` und `FamilyInput` (Zeilen 282–283). `native/src/app/core/family_viewer_events.rs::receive` verwendet dagegen `FamilyOperationResult` und `FamilyInputResult` (Zeilen 12, 36, 37). Dieselben nicht definierten Varianten stehen in `native/src/mobile/os/shared/domains/screen_family.rs::event` (Zeilen 64–65) und `native/src/mobile/os/shared/domains/screen_view_run.rs` (Zeilen 354–355). Wiredecoder und Hostsender verwenden die tatsächlich definierten Namen.

**Wirkung:** Die Betrachter können so nicht gegen den aktuellen HostFrame-Typ aufgelöst werden; dadurch ist das integrierte Paket bereits statisch inkonsistent. Diese Feststellung stammt aus Quelllektüre; ein Compiler wurde nicht ausgeführt.

**Korrekturbedarf:** Einen Variantennamensatz durchgehend in Wirevertrag, Desktop und Mobile verwenden. Alle freigegebenen Consumer abgleichen, ohne die Drahtkennungen zu ändern.

**Zusätzliche Fundstelle:** `native/src/app/core/screen_decode_thread.rs:306` enthielt dieselben beiden falschen Varianten.

**Quellenstand:** Primärbeleg ist der tatsächliche Enumvertrag im geprüften Worktree über `126f9325`, nicht ein angenommener Compilerlauf.

**Status:** Alle vier gemeldeten Consumer sind inzwischen statisch nachgelesen und verwenden `HostFrame::FamilyOperation`/`HostFrame::FamilyInput`. Der konkrete Variantenfehler ist behoben. Keine Compilerfreigabe.

### B8-R2 – P1: Bestätigte Programmfenster werden wegen verschiedener Quellen-ID-Formate nie geöffnet

**Auslöser:** Ein echter Programmstart liefert erfolgreich einen korrelierten P4-Fensterbeleg; der Nutzer wählt „Bestätigtes Appfenster öffnen“.

**Beleg:** `native/src/apps/os/host.rs::correlate_window` extrahiert den inneren Wert von `SourceId::Window` und speichert ihn als `WindowBinding.window_id` (ursprünglich Zeilen 328–330). `native/src/screen/core/window_list.rs::Display` ergänzt dagegen `window:`; auch die Serde-Darstellung verwendet diesen Displaywert. Der ursprüngliche Filter in `native/src/app/core/family_picker.rs::resolve_confirmed:87` verglich `entry.source.to_string()` mit dem inneren `window_id`. `android/app/src/main/java/app/smartexplorer/android/ui/share/FamilyTargetsDialog.kt:67` verglich dieselben verschiedenen Formate.

**Wirkung:** Für jede gültige Familiennonce ist `window:@family.<nonce>` ungleich `@family.<nonce>`. Der Filter findet keinen Eintrag und verweigert den Übergang zum Viewer trotz gültigem Request-/App-/Benutzer-/Sitzungs-/Generationbeleg. Manuelles Öffnen aus der Familienliste ist davon getrennt.

**Korrekturbedarf:** Den bestehenden inneren `window_id`-Vertrag erhalten und in beiden Betrachtern typgerecht abgleichen; Request-ID, Nodepin und Generation bleiben erforderlich. Kein PID-/Titelfallback.

**Quellenstand:** Primärbelege sind `WindowBinding`, `SourceId::{Display,From}` und beide Consumer im Worktree über `126f9325`. Der Fehler benötigt keine externe Plattformannahme.

**Status:** Die Korrektur ist statisch nachgelesen: Desktop vergleicht den inneren `SourceId::Window`-Wert; Android vergleicht mit `"window:${confirmed.window.windowId}"`. Request-ID und Generation bleiben in beiden Filtern erhalten. Der konkrete Formatdefekt ist damit behoben; der tatsächliche Viewerstart wurde nicht ausgeführt.

### B8-R3 – P2: Windows-Familienaufnahme und Zeigereingabe verwenden verschiedene Rechtecke

**Auslöser:** Ein Windows-Familienfenster besitzt unsichtbare Resize-Ränder; der Nutzer klickt in der dargestellten Fensterfläche, besonders nahe deren Rand.

**Beleg:** `native/src/remote_window/os/windows/facts.rs::WindowFacts::read:117` ersetzt `GetWindowRect` durch `DWMWA_EXTENDED_FRAME_BOUNDS`. Diese sichtbare Geometrie bestimmt das Familienbild und `FamilySnapshot::pointer_target`. `native/src/screen/os/shared/family_backend.rs::WorkerFamily::input` reicht die membernormalisierten Koordinaten unverändert an `backend.inject` weiter. `native/src/screen/os/windows/input_send.rs::target_area` holt für denselben HWND erneut ausschließlich `GetWindowRect`; `absolute` skaliert über dieses abweichende Rechteck.

**Wirkung:** Der tatsächlich injizierte Punkt stimmt nicht mit dem angezeigten Punkt überein. Die Abweichung ist positionsabhängig; an den Rändern kann der Zeiger außerhalb der dargestellten sichtbaren Fläche liegen. Eine konkrete Pixelabweichung hängt von Fensterstil und DPI-Kontext ab und wurde nicht gemessen.

**Korrekturbedarf:** Für die Familieninjektion dieselbe validierte physische Capturegeometrie verwenden. Eine pauschale Änderung der bisherigen Monitor-/Rohfensterpfade wäre eine zusätzliche Kompatibilitätsänderung und muss vermieden oder ausdrücklich begründet werden.

**Primärquelle:** Microsoft dokumentiert für Windows Vista und neuer unsichtbare Resize-Ränder in `GetWindowRect`, DPI-Virtualisierung dieses Aufrufs und die davon verschiedene `DWMWA_EXTENDED_FRAME_BOUNDS`-Geometrie. Dokumentstand 2025-04-11, abgerufen 2026-10-25: [GetWindowRect](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect). Die verwendeten Bindings sind `windows 0.58.0` und `windows-sys 0.59.0` laut `native/Cargo.lock`.

**Status:** Die Korrektur ist statisch nachgelesen: `WorkerFamily::input` ruft `backend.inject_family` mit dem validierten CaptureMember auf; der Windowsadapter verwendet dessen `CaptureGeometry` direkt und prüft Desktopgrenzen. Der bisherige gewöhnliche Inputpfad bleibt separat. Der konkrete Rechteckwechsel ist behoben; physische DPI-/Mehrmonitorkoordinaten wurden nicht gemessen.

### B8-R4 – P1: Fokusverlust gibt bereits gedrückte Hosttasten nicht frei

**Auslöser:** In einer freigegebenen Familiensteuersitzung wird eine Taste gedrückt. Am Host wechselt der Fokus aus der Familie, ohne deren Geometrie, Sichtbarkeit oder Z-Reihenfolge zu ändern. Danach kommt vom Betrachter die zugehörige Tastenfreigabe.

**Beleg:** `native/src/remote_window/core/host.rs::HostSession::refresh:186` setzt bei fehlendem Familienfokus nur `input_armed=false`; reine Fokusänderungen erhöhen die Snapshotrevision, nicht die Layoutrevision. `native/src/screen/core/host_family.rs::publish_family:47` ruft `release_pressed` ausschließlich bei geändertem Layout. `WorkerFamily::snapshot` besitzt keinen Freigabeschritt. `native/src/screen/core/family_viewer.rs::receive` disarmt bei Fokusverlust; `FamilyViewer::input` verwirft das spätere Key-up bei `snapshot.focused_target()==None` vollständig. Es entsteht daher auch kein abgewiesener Hostinput, dessen Fehlerpfad freigeben könnte.

**Wirkung:** Die zuvor injizierte Taste bleibt ohne Trennung, expliziten Kontrollentzug oder nachfolgenden Hostfehler gedrückt. Das kann folgende Eingaben in der lokalen Hostsitzung verändern. Der Fehler widerspricht dem konkreten V6-Vertrag in `docs/plaene/2026-10-24-batches/vertraege.md:337`: Fokusabweichung muss gehaltene Tasten lösen und neue Aktivierung verlangen.

**Korrekturbedarf:** Fokusverlust beziehungsweise Wechsel des aktiven Familienmitglieds muss Hostfreigaben unabhängig von der Capturelayoutrevision auslösen. Freigaben dürfen weder eine neue Fensteraktivierung auslösen noch durch fehlenden Fokus unterdrückt werden. Die tatsächlich fehlgeschlagene Freigabe muss bis zum erfolgreichen Release erhalten bleiben.

**Primärquelle:** Win32 `SendInput` setzt den vorhandenen Tastaturzustand ausdrücklich nicht zurück; die API liefert eine Zahl tatsächlich eingefügter Ereignisse und unterliegt UIPI. Dokumentstand 2021-10-13, abgerufen 2026-10-25: [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput). Der plattformunabhängige Verlustpfad ist im gemeinsamen Familienviewer belegt; eine X11-Laufzeitfolge wurde nicht ausgeführt.

**Status:** Fokusänderungen besitzen inzwischen eine separate `input_epoch`; Worker-Snapshotabfragen lösen gehaltene Eingaben auch bei unverändertem Capturelayout. Die Sitzungsseite vergleicht ebenfalls die Fokusidentitäten. Windows besitzt einen reinen Freigabepfad ohne Lookup oder Aktivierung des alten Fensters. Diese Korrektur ist statisch nachgelesen.

**Zusätzlich gefundener und behobener Korrekturrand R4-X11:** Der erste neue `WorkerFamily::release` rief `ScreenHostBackend::release_input`; Linux besaß zunächst keinen Override und fiel auf `inject` zurück. `x11_input.rs::inject` löste dabei vor den Key-/Button-ups noch die Geometrie des ursprünglichen, möglicherweise zerstörten Fensters auf. Nach unmittelbarer Rückmeldung wurde ein reiner `x11_input::release`-Pfad ergänzt und in `linux_os/host_backend.rs::release_input` angeschlossen. Er akzeptiert ausschließlich Key-/Button-ups beziehungsweise Touch-End/Cancel, benötigt keine Fenstergeometrie und sendet keine Zeigerbewegung. Der korrigierte Pfad ist statisch nachgelesen. Primärvertrag: [X11R7.7 Core Protocol, GetGeometry](https://xorg.freedesktop.org/archive/X11R7.7/doc/xproto/x11protocol.html), Abruf 2026-10-25; ein nicht existierendes Drawable kann nicht als aktuelle Fenstergeometrie aufgelöst werden.

### B8-R5 – P2: Android meldet einen möglichen erfolgreichen OS-Start nach Journalfehler als verweigert

**Auslöser:** Das erste `outcome_unknown`-Journal ist geschrieben. `LauncherApps.startMainActivity` nimmt den Start an. Das anschließende Schreiben des Ergebnisjournals scheitert, beispielsweise weil für die neue AtomicFile-Datei kein Speicher verfügbar ist.

**Beleg:** `android/app/src/main/java/app/smartexplorer/android/screen/AppLaunch.kt::dispatch` ruft nach `startMainActivity` erneut `write(context, result)` auf (ursprünglich Zeilen 251–265). Diese Operation liegt außerhalb des OS-Aufruf-`try` und darf eine Schreibexception weiterwerfen. `AppHostRuntime.kt::attach:48` behandelt die Exception pauschal mit `reject(event, "android_local_app_host_failed")`; `reject` sendet für `appsHostLaunch` stets `state="denied"`. Die native Bridge akzeptiert diesen Status in `apps/os/android/bridge.rs::finish_launch`. `AppsDialog.kt::unresolved` berücksichtigt ausschließlich fehlende Belege, `outcome_unknown` und `consent_required`.

**Wirkung:** Die Remoteoberfläche zeigt einen verweigerten Start, obwohl das Programm bereits laufen kann. Sie bietet einen neuen Start ohne die für ungewisse Ergebnisse erforderliche ausdrückliche Duplikatbestätigung an. Die dauerhafte Request-ID schützt weiterhin gegen Wiederholung derselben ID; sie korrigiert den falschen Ergebnisstatus nicht.

**Korrekturbedarf:** Nach Beginn der Dispatchbearbeitung muss ein nicht klassifizierbarer Fehler `outcome_unknown` erhalten. Die ausdrückliche Verweigerung einer noch gar nicht bearbeiteten Warteschlangenanforderung kann `denied` bleiben. Ein Fehler beim Ergebnisspeichern darf keinen neuen Start auslösen.

**Primärquellen:** `AtomicFile.startWrite` ist seit Android API 17 vorhanden und dokumentiert `IOException`; `failWrite` verwirft die neue Datei. [AtomicFile](https://developer.android.com/reference/android/util/AtomicFile#startWrite()), Dokumentstand 2026-08-03, abgerufen 2026-10-25. Der tatsächliche OS-Aufruf ist [LauncherApps.startMainActivity](https://developer.android.com/reference/android/content/pm/LauncherApps#startMainActivity(android.content.ComponentName,%20android.os.UserHandle,%20android.graphics.Rect,%20android.os.Bundle)), seit API 21. Eine erfolgreiche Dateipersistenz ist kein Bestandteil dieses OS-Aufrufs.

**Status:** Statisch nachgelesen: Der Catch nach begonnener Eventbearbeitung setzt jetzt `launchMayHaveDispatched=true`; ausschließlich dieser Pfad meldet `outcome_unknown`. Vor Bearbeitung abgewiesene Queue-/Identitätsanforderungen bleiben `denied`. Der konkrete falsche Ergebnisstatus ist behoben; ein realer Schreibfehler wurde nicht erzeugt.

### B8-R6 – P1: X11-Familien können überdeckende Fremdfenster als Mitgliedsbild übertragen

**Nachtragsauftrag und Plan:** Am 26.10.2026 meldet der Hauptagent den Rootcrop-Pfad als zusätzliche konkrete Capturegrenze und gibt dafür die unten vollständig inventarisierten 17 Dateien frei. Ergebnisauftrag ist der statische Defektbeleg und die Nachprüfung der von `window_family` geschriebenen Korrektur. Der gewöhnliche ausdrücklich gewählte Raw-Windowpfad muss seinen bestehenden Overlap-Fallback behalten; Familien müssen ohne verwertbaren Pixmapbeleg scheitern. Windows-WGC-Familien bleiben unterstützt. Die größere zusammenhängende Lesefläche reicht vom Backendaufruf über das tatsächliche Pixmaplesen bis zur Familienkomposition und enthält den Windowsvergleich, damit die neue Grenze keine Plattformfähigkeit still entfernt.

**Belegter Ausgangsstand:** Worktree über `8d970a63`. `native/src/screen/os/shared/family_backend.rs::WorkerFamily::open` übergibt dem Familienkompositor eine Closure auf `raw_backend.open_source(Window(...), None)` (vor Korrektur Zeilen 181–184). `native/src/screen/os/linux_os/host_backend.rs::open_source` öffnet dafür denselben `X11Source` wie für ein gewöhnliches Raw-Window (Zeilen 236–247). `x11_capture.rs::pass_window` versucht bei `self.compositor` zunächst `read_pixmap`; jeder Fehler dieses Versuchs wird durch `if let Ok` verworfen. Anschließend liest die Funktion `self.root` am Fensterrechteck (Zeilen 352–362). Ohne Compositor erreicht sie diesen Rootpfad unmittelbar. `FamilyScreenSource::capture` speichert das zurückgegebene Bild als Mitgliedsfläche; `compose` kopiert dessen Pixel in das Familienbild (`remote_window/os/shared/family_source.rs:174–188,78–104`).

**Auslöser und Wirkung:** Ein sichtbares Familienmitglied wird ohne Compositor aufgenommen, oder sein zuvor funktionierender XComposite-Pixmapzugriff scheitert während einer laufenden Sitzung. Ein nicht zur Familie gehörendes Fenster überdeckt dessen Bildschirmrechteck. Der Rootcrop enthält diese sichtbaren Fremdpixel und überträgt sie innerhalb des Familienbilds. Damit verletzt der tatsächliche OS-Quellpfad den P4-Vertrag unabhängiger Mitgliedsflächen und kann nicht freigegebene Fensterinhalte offenlegen. Ein opaker Hintergrund im gemeinsamen Kompositor entfernt diese bereits im Mitgliedsbild enthaltenen Pixel nicht. Ein echter X-Server-Versuch wurde nicht ausgeführt; der Defekt ergibt sich aus dem gelesenen Kontrollfluss.

**Entscheidung und Korrekturmaßstab:** Ein eigener, standardmäßig verweigernder Backend-Einstieg muss die Familienanforderung bis zur OS-Quelle erhalten. X11-Familien dürfen ausschließlich einen tatsächlich lesbaren XComposite-Pixmap verwenden. Sowohl fehlender Anfangsbeleg als auch späterer Namens-/Lesefehler müssen einen konkreten Fehler liefern; keine Umdeutung zu Monitoraufnahme oder Rootcrop. Der normale Raw-Windowpfad behält seine ausdrücklich dokumentierte Überlappungsgrenze. Windows muss den Familieneinstieg explizit an seinen vorhandenen fenstergebundenen WGC-Pfad anschließen. Diese vier Kontrollflüsse wurden nach der gemeldeten Autorenfreigabe statisch nachgelesen; keine Produktänderung und keine ausführende Prüfung durch den Reviewer.

**Primärquellen:** [X.Org Composite Extension 0.4, Stand 2007-07-03, Abschnitte 3.1 und 10](https://xorg.freedesktop.org/archive/X11R7.7/doc/compositeproto/compositeproto.txt) beschreibt `NameWindowPixmap` als Zugriff auf den Offscreen-Speicher des Fensters. Bei Map/Resize entsteht ein neuer Pixmap; ein nicht umgeleitetes oder nicht sichtbares Fenster ergibt `Match`. Das stützt die Trennung vom Rootbild und die wiederholte Prüfung des aktuellen Pixmaps. [Microsoft `IGraphicsCaptureItemInterop::CreateForWindow`](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow), Dokumentstand 2024-04-11, unterstützt ab Windows 10 1903/Build 18362 ein CaptureItem für genau den übergebenen HWND. Beide Primärquellen abgerufen am 26.10.2026. Der bestehende Windowsquellpfad verwendet diesen Aufruf in `capture_wgc.rs:93–105`; sein Fensterzweig besitzt im gelesenen Ausgangsstand keinen DXGI-/Monitorfallback (`capture_source.rs:168–180`).

**Korrektur der früheren Reviewaussage:** Die frühere Zeile „Fensterfamilie … verwendet keinen Desktopausschnitt“ beschrieb den gemeinsamen Kompositor, war aber ohne den damals ausgeschlossenen X11-Rohcapturepfad als Gesamtaussage zu weitgehend. B8-R6 dokumentiert diese Beleglücke ausdrücklich. Die nachfolgende Bewahrungstabelle stützt sich nun auf die zusätzlich gelesenen konkreten OS-Quellpfade.

**Nachgelesene Korrektur vom 26.10.2026:** Der Autor hat fünf Produktdateien geändert: `screen/core/host_backend.rs`, `screen/os/shared/family_backend.rs`, `screen/os/linux_os/host_backend.rs`, `screen/os/linux_os/x11_capture.rs` und `screen/os/windows/host_backend.rs`, jeweils unter `native/src`. Die folgende Bewertung bezieht sich auf deren freigegebenen Worktreestand und die unveränderten direkten Captureverbraucher.

| Kontrollfluss | Statischer Beleg und Entscheidung |
|---|---|
| Familienanforderung erreicht die OS-Quelle | `ScreenHostBackend::open_family_member` verweigert standardmäßig mit `window_family_member_capture_unsupported` (`screen/core/host_backend.rs:213–217`). `WorkerFamily::open` ruft für Mitgliedsflächen ausschließlich diesen Einstieg auf (`screen/os/shared/family_backend.rs:181–185`). Ein Backend ohne ausdrückliche Unterstützung fällt damit nicht auf gewöhnliches Raw-Capture zurück. |
| Fehlender erster Pixmapbeleg | Linux führt nur X11 zu `X11Source::open_family_window`; Wayland bleibt beim Portalhinweis, eine Sitzung ohne Display verweigert (`screen/os/linux_os/host_backend.rs:265–272`). `open_family_window` prüft den beim Öffnen abgefragten Compositor und setzt `OwnPixmap` (`x11_capture.rs:221–225,250`). Der Konstruktor liest noch keinen Pixmap. Die tatsächlich verwertbare Mitgliedsfläche wird beim ersten sichtbaren Capturepass durch `read_pixmap` belegt. Fehlender Compositor ergibt `x11_family_capture_requires_compositor`; fehlendes Naming oder Lesen ergibt `x11_family_member_pixmap_failed` mit dem ursprünglichen Fehler (`:368–376`). Vor diesem Erfolg wird kein Familienbild ausgegeben. |
| Späterer Belegverlust und Bildcache | Jeder sichtbare `OwnPixmap`-Pass prüft den Compositor erneut, benennt den aktuellen Fensterpixmap neu und liest ausschließlich diesen Drawable (`x11_capture.rs:368–397`). Die gemeinsame Lesefunktion behält den übergebenen Drawable sowohl bei SHM als auch bei `GetImage` bei (`:291–325`). Jeder Fehler beendet den strikten Zweig vor dem Rootread. `capture` ruft `pass()?` vor Bildvergleich und Cacheverwendung auf (`:423–446`). `FamilyScreenSource::capture` gibt Öffnungs-/Capturefehler weiter; nur bei inzwischen veralteter Boundary invalidiert es den Bezug und meldet `Starting`, ebenfalls ohne Bild (`remote_window/os/shared/family_source.rs:161–185`). Ein minimiertes oder verborgenes Mitglied pausiert weiterhin vor dem Pixmapversuch (`x11_capture.rs:358–363`); beim nächsten sichtbaren Pass gilt erneut die strikte Prüfung. |
| Ausdrücklich ausgewähltes Raw-Window bleibt kompatibel | Gewöhnliches Linux-`open_source` ruft unverändert `X11Source::open` auf (`screen/os/linux_os/host_backend.rs:236–247`); dessen Modus beginnt mit `RawFallback` (`x11_capture.rs:262`). Dieser Zweig versucht bei vorhandenem Compositor den Pixmap und liest bei dessen Fehler oder ohne Compositor weiterhin den Rootcrop (`:378–384`). Die bestehende Überlappungsgrenze wird damit nur für diesen ausdrücklich gewählten Quelltyp erhalten. |
| Windows-Familien bleiben an WGC gebunden | Der Windows-Override verwendet den vorhandenen `SourceId::Window`-Zweig (`screen/os/windows/host_backend.rs:121–125`). `WindowsSource::open_window` verlangt WGC und erstellt `Target::Window`, ohne DXGI-/Monitorersatz (`capture_source.rs:168–180`); `capture_wgc.rs:93–105` verwendet `CreateForWindow`. Wiederaufnahme verwendet das vorhandene CaptureItem und verweigert nach dessen Schließung (`:266–275`). Der DXGI-Fallback bleibt auf den getrennten Monitorzweig beschränkt (`capture_source.rs:150–165`). |

**Status und Grenze:** B8-R6 ist für den belegten Rootcrop-Auslöser statisch korrigiert. Die erste Pixmapprüfung erfolgt bei der ersten sichtbaren Aufnahme, nicht bereits im Quellkonstruktor; durch die verzögerte Mitgliedsöffnung und Fehlerweitergabe entsteht daraus kein ungeprüftes Familienbild. Der abgegrenzte Korrekturabgleich ergab keinen zusätzlichen belegten Defekt. Es wurden weder Compositorverlust an einem realen X-Server noch Windows-WGC-Aufnahme ausgeführt. X-Server-/Treiberverhalten, vollständige D3D-Readbackimplementierung, vollständige X11-Pixelkonvertierung und Compilerauflösung sind nicht durch diesen Nachtrag freigegeben.

**Vollständiges Leseinventar dieses Capture-Nachtrags:** Die folgenden 17 Dateien wurden an den genannten Vertrags-/Capturegrenzen gelesen; die Liste behauptet keine vollständige Prüfung sämtlicher Zeilen. Außerhalb dieser Liste wurde für den Nachtrag kein Repositoryquelltext geöffnet.

```text
AGENTS.md
docs/ARCHITEKTUR.md
docs/oberflaechen/bildschirm.md
docs/oberflaechen/fernzugriff.md
docs/plaene/2026-10-24-batches/batches.md
docs/plaene/2026-10-24-batches/vertraege.md
docs/plaene/2026-10-24-batches/review-b8.md
docs/plaene/2026-10-24-batches/umsetzung.md
native/src/screen/core/host_backend.rs
native/src/screen/os/shared/family_backend.rs
native/src/screen/os/linux_os/host_backend.rs
native/src/screen/os/linux_os/x11_capture.rs
native/src/screen/os/linux_os/x11_capture_target.rs
native/src/screen/os/windows/host_backend.rs
native/src/screen/os/windows/capture_source.rs
native/src/screen/os/windows/capture_wgc.rs
native/src/remote_window/os/shared/family_source.rs
```

Zusätzlich wurden ausschließlich die oben verlinkten X.Org-/Microsoft-Primärquellen inhaltlich ausgewertet. Der Reviewer hat nur `review-b8.md` geändert. Die Capture-/Backendkorrektur schreibt `window_family`; dieser Bericht schreibt dem Reviewer keine Produktänderung zu.

## Ergänzungsauftrag: Erstgrant-Zusammenführung

**A – Auftrag und Grenze:** Der Hauptagent hat die verbliebene Grenze von `ShareHost::mutate_access_grant` über `ExecGrantMutation` bis zur gespeicherten und aktiven Policy ausdrücklich zur statischen Prüfung freigegeben. Zu klären sind bestehende Fähigkeiten, CAS-Revision, Identitätspins und die Trennung von Login-Maschinenrechten. Produktänderungen und ausführende Prüfungen bleiben ausgeschlossen. Ausgangsreview über `126f9325`; beim Ergänzungsreview zeigt `git log -1 --oneline` `ed83a910`. Die Befunde beziehen sich auf den zusätzlich gelesenen, noch veränderten Worktree.

**B – belegter Bestand:** `prepare_local_app_grant` erstellt einen AppLaunch-Patch mit kopierten Constraints. `prepare_access` klont die aktuelle Policy und prüft die erwartete Revision vor dem Patch. `AccessGrant::apply` verändert nur benannte Fähigkeiten beziehungsweise ausdrücklich gelieferte Felder. `ExecGrantWire` hält Account, Programmliste und Revision neben dem V2-Objekt fest. Das Persistenzjournal bindet Ziel und vollständigen Principal. Die zunächst offene Vergleichsstelle `member.blocked` gegenüber `member.is_admitted()` wurde anhand des Typs und seiner Zustandsübergänge geklärt; siehe Ergebnis unten.

**C – Entscheidung:** Die Prüfung verwendet die vorhandene Mutationskette einschließlich ihrer tatsächlichen Serialisierung. Ein korrekter vorbereiteter Patch allein reicht nicht als Bewahrungsnachweis. Downgrade-Deaktivierung wird anhand des ausdrücklichen Vertrags V2 bewertet und nicht als unbeabsichtigter Rechteverlust bezeichnet. Der P1-Maschinenbroker bleibt in B7; hier werden sein getrenntes Recht und der fehlende Schreibpfad aus dem Appeditor geprüft.

**D – konkreter Abschluss:** Mutation, Serialisierung, Policy-CAS, Dateirevisions-CAS, Runtimepins und Logintrennung sind statisch ausgewertet. Der Hauptagent hat die zusätzlichen direkten Abhängigkeiten einschließlich `RoomMember::is_admitted`, `profile_store.rs` und `profile_transaction.rs` ausdrücklich freigegeben. Die OS-Promote-/Privatdateiadapter hat er ausdrücklich außerhalb dieses B8-Auftrags belassen. Ergebnis: kein zusätzlicher belegter Defekt in der benannten Erstgrantgrenze; keine Aussage über ausgeführte Speicherfehler-, Konkurrenz-, Compiler- oder Plattformprüfungen.

| Prüffrage | Statischer Abschluss und konkrete Fundstelle |
|---|---|
| Bleiben bestehende Fähigkeiten erhalten? | `node_apps.rs:54–86` bindet den Patch an die gewählte exakte Quelle und deren Revision; er setzt ausschließlich `AppLaunch`. Beim Einschalten kopiert er `source.grant.constraints` und ersetzt nur `app_launch`. Beim Ausschalten lässt er die bestehenden Constraints vollständig stehen. `access_grant.rs:351–385` klont Account, sieben Entscheidungen, Constraints und globale Gültigkeit; die Programmliste wird ebenfalls geklont. `exec_policy.rs:184–195,255–276` übernimmt diese Werte samt unbekannten bewahrten Feldern. Damit bleiben bei gültigen bestehenden Policies `ExecCommands`, `ExecTerminal`, `ScreenView`, `ScreenControl`, `LoginAccess`, `TcpForward`, Account, Programmliste, globale Gültigkeit sowie Login-/TCP-Constraints erhalten. Die bestehende Invariante „Programmliste erlaubt kein Terminal“ wird weiterhin geprüft; ein bereits widersprüchlicher Datensatz wird nicht als gültige Ausgangspolicy behandelt. |
| Bleiben Erstgrant-Defaults und Altgrants konsistent? | `access_grant.rs:239–247` beginnt mit sieben ausgeschalteten Fähigkeiten; der erste Appgrant aktiviert nur die ausdrücklich ausgewählte Entscheidung. Ein bestehender Legacy-Execgrant wird in `access_grant_wire.rs:164–175` mit seinem Konto und seiner bisherigen Befehls-/Terminalbedeutung übernommen. `exec_policy_wire.rs:52–81,85–119` serialisiert über das tatsächliche Accessobjekt und rekonstruiert daraus die aktuelle Policy. V2 setzt die Legacyfelder `enabled` und `restricted_enabled` auf false; das ist die in `vertraege.md:101–112` ausdrücklich verlangte vollständige Deaktivierung bei einem Downgrade, kein unbemerkter Verlust auf dem aktuellen Leser. |
| Ist ein veralteter Dialog gegen Überschreiben geschützt? | `apps_ipc.rs:14–17` reicht die UI-Revision zwingend als `Some(expected_policy_revision)` weiter. `exec_grant_persistence.rs:22–32` vergleicht sie vor dem Patch. `exec_policy.rs:244–252` erhöht die Revision mit `checked_add(1)` genau einmal und verwirft den überholten Base-Decision-Bezug. `exec_grant_persistence.rs:75–98,134–145` verlangt beim Schreiben erneut die erwartete Revision und genau deren Nachfolger. Ein bereits identisch gespeichertes Resultat ist idempotent; eine abweichende Policy derselben Nachfolgerrevision wird nicht als Erfolg akzeptiert. |
| Wie wirken Dateikonflikte auf den Policy-CAS? | `profile_store.rs:181–193` ruft `profile_transaction::run` auf. Diese Schleife lädt bei einem Dateikonflikt neu und wiederholt dieselbe feldbezogene Mutation; Load-, Mutations- und sonstige Commitfehler werden sofort zurückgegeben (`profile_transaction.rs:40–64`). `profile_store.rs:380–398,446–459` hält Prozessmutex und exklusives Dateilock über den erneuten Vergleich der vollständigen Profildateirevision. Unabhängige Profiländerungen können dadurch übernommen werden; eine inzwischen geänderte Zielpolicy scheitert weiterhin an `apply_persisted_cas`, bevor sie geschrieben wird. Der Stagingpfad führt Write/Flush/Sync vor der Übergabe an den bestehenden Promoteadapter aus (`profile_store.rs:412–443`). |
| Bleiben Identität und Beziehung bis zur aktiven Policy gebunden? | `exec_grant_persistence.rs:189–274` löst Gerät, öffentlichen Schlüssel, Fingerprint und Node exakt auf und prüft Schlüssel/Nodegleichheit sowie Fingerprint. Direct erhält die lokale Direct-Beziehungs-ID; Room zusätzlich die explizite Room-ID. Persistenz und Runtime vergleichen anschließend den vollständigen Principal (`exec_grant_persistence.rs:84–85`; `exec_grant_runtime.rs:110–125`). Die aktive Requestprüfung verlangt weiterhin die nachgewiesene Beziehung, Direct-online/accepted beziehungsweise auto_join/admitted; sie vereinigt keine Rechte verschiedener Quellen (`access_authorize.rs:23–70,104–130`; `access_relation_view.rs:78–149`). |
| Ist die unterschiedliche Room-Prüfung ein Defekt? | Kein erreichbarer Fehler aus den gelesenen Zustandsübergängen belegt. `exec_grant_persistence.rs:254` verwendet `blocked`, Runtime und Requestprüfung verwenden `is_admitted`. `types.rs:121–130` und `room_relation_members.rs:47–70` definieren `Pending ⇒ blocked`. Neue Mitglieder und Runtimeübernahmen setzen beide Werte zusammen (`room_relation_members.rs:207–215,292–315`); Zulassen/Entsperren entfernt Pending zusammen mit blocked (`:122–146`). Eine manuell inkonsistent veränderte Profildatei ist damit nicht als gültiger Projektzustand nachgewiesen; die aktive Autorisierung bleibt selbst dort strenger. |
| Bleiben Login-Maschinenrechte getrennt? | `access_grant_journal.rs:45–51` verweigert jede ausdrückliche Aktivierung von `LoginAccess`, auch `Ask`, durch den Benutzerdaemon. Der Appeditor erzeugt keine solche Änderung und kopiert lediglich bestehende Loginbedingungen. `access_authorize.rs:122` begrenzt den gemeinsamen Loginrequest zusätzlich auf Direct. Dieser gemeinsame Requestentscheid ersetzt keine Maschinenpolicy; der P1-Broker mit seiner administrativ geschützten Autorisierung gehört weiterhin zu B7. In der gelesenen Appgrant-Mutationskette besteht kein Aufruf zum Erteilen oder Ändern dieses Maschinenrechts. |
| Werden erfolgreiche Mutation und laufende Fähigkeiten korrekt getrennt? | `exec_grant_journal.rs:274–316,426–464` journalisiert den vorbereiteten Kandidaten und bestätigt ihn erst nach Persistenz, Runtimeapply und Journalabschluss. Ein vorhandener ausstehender Auftrag wird nicht durch einen neuen Patch überschrieben. `exec_grant_runtime.rs:118–143` installiert die Policy unter dem Authmutex und prüft die Accessleases erneut. `access_grant_runtime.rs:75–81` behandelt einen reinen Appgrant nicht als Verengung der Execfähigkeiten; bestehende Befehls-/Terminaljobs behalten den vorhandenen Erweiterungspfad. Die Implementierung des Exec-Registrys wurde in diesem Ergänzungsauftrag nicht neu geprüft. |

**Primärquellen des Ergänzungsreviews:** Der lokale Vertragsstand ist `vertraege.md`, V2, im Worktree über `ed83a910`. `native/Cargo.lock:6513–6520` bindet Serde an 1.0.228. Der [offizielle Serde-1.0.228-Serializer, `serialize_into`](https://raw.githubusercontent.com/serde-rs/serde/v1.0.228/serde_derive/src/ser.rs) und [Deserializer, `deserialize_from`](https://raw.githubusercontent.com/serde-rs/serde/v1.0.228/serde_derive/src/de.rs) bestätigen, dass die in `ExecGrant` angegebenen Konversionen tatsächlich den gespeicherten Typ bestimmen; ergänzend [Serde-Containerattribute](https://serde.rs/container-attrs.html). Die [Rust-Dokumentation zu `File::lock`](https://doc.rust-lang.org/std/fs/struct.File.html#method.lock), abgerufener Dokumentstand 1.99.1, API seit 1.89.0, beschreibt Exklusivität gegenüber Lockteilnehmern und Freigabe beim Schließen aller zugehörigen Handles. Sie verspricht keinen Schutz vor Schreibzugriffen, die das Lock ignorieren. Alle Quellen abgerufen am 25.10.2026; die genaue Rustversion des Remote-Runners wurde hier nicht ermittelt.

**Verbleibende Grenze:** Die benannte Erstgrant-Zusammenführung ist statisch abgeschlossen. Die Implementierung der privaten Dateiadapter, die OS-Ersetzung durch `promote_staged_replace`, tatsächliche Crashdauerhaftigkeit und Verhalten konkurrierender realer Prozesse sind nicht geprüft. Der allgemeine Profil-Migrations-/Reconcilepfad und der Exec-Registry-Innenaufbau wurden nicht als eigenständige Subsysteme neu bewertet. Daraus wird keine vollständige Speicher-, B7-Broker- oder Laufzeitfreigabe abgeleitet.

## Konkret bewahrte Fähigkeiten und Vertragsentscheidungen

Die folgenden Aussagen beschreiben den gelesenen Kontrollfluss. Sie sind keine Laufzeitfreigabe. Die Befundabschnitte dokumentieren die ursprünglichen Defekte und ihren jeweils nachgelesenen Korrekturstand.

| Grenze | Statisch belegte Eigenschaft | Beleg |
|---|---|---|
| Lokale Appfreigabe | Der Patch setzt nur `AppLaunch`, kopiert bestehende V2-Constraints und ergänzt explizite App-/Benutzer-/Katalogscopewerte. Die zusätzlich gelesene Zusammenführung, V2-Serialisierung und CAS-Persistenz erhalten die gültigen bestehenden Fähigkeitsregeln, Account, Programmliste und globale Gültigkeit. | `share/core/node_apps.rs::prepare_local_app_grant`, `apps/core/grant.rs`, Desktop-/Android-Appgranteditor und Abschnitt „Ergänzungsauftrag: Erstgrant-Zusammenführung“ |
| Start und Wiederaufnahme | Vollständiger Request und Besitzer werden dauerhaft vor Dispatch gebunden. Die Besitzeridentität enthält Beziehungstyp, Beziehungs-ID, Gerät, öffentlichen Schlüssel, Fingerprint und Node-ID. Ungewisse Ergebnisse führen beim erneuten Request nicht zu einem automatischen OS-Neustart. | `apps/os/host.rs`, `apps/os/shared/journal.rs`, `share/core/node_apps.rs::owner_key`, `share_apps_jobs.rs`, `AppsApi.kt` |
| Windows-Prozessbeleg | Der Zeuge dupliziert das ursprüngliche Shellprozesshandle und prüft Lebensdauer, CreationTime, tatsächliche Sitzung sowie SID/Auth-ID. Fehlendes Originalhandle führt nicht zur Rekonstruktion über Titel oder PID. | `apps/os/windows/{launch,session,window_correlation}.rs` |
| Linux-Prozessbeleg | Das Kind übergibt vor `exec` sein eigenes pidfd über SCM_RIGHTS. UID, Prozessstart, Sitzung und Executableidentität werden zusätzlich geprüft. Ein beendeter Zeuge wird nicht durch eine neue numerische PID ersetzt. | `apps/os/linux_os/window_correlation.rs`, `apps/os/shared/window_correlation.rs` |
| Fensterfamilie | Eigene Generationskennungen, originale Pins und nachträgliche Capturegrenzprüfung bleiben erhalten. Der gemeinsame Kompositor setzt die zugelassenen Mitgliedsquellen auf opakem Hintergrund zusammen. Nach B8-R6 verwenden X11-Mitgliedsquellen ausschließlich aktuelle Pixmaps und scheitern bei Belegverlust vor dem Rootread; Windows-Mitgliedsquellen bleiben an WGC gebunden. Der gewöhnliche Raw-Window-Fallback bleibt getrennt erhalten. | `remote_window/core/host.rs`, Windows-/X11-Adapter, `remote_window/os/shared/family_source.rs`, B8-R6 |
| Sichtbare Eingabegrundlage | Familienlayout kommt aus der tatsächlichen CaptureBoundary über Source/Worker/GOP/Relay. Decoder prüfen den Layoutwert. Gezeichnete Android-Bitmaps beziehungsweise passende MediaCodec-Surfacecallbacks erzeugen PRESENTED. Der gemeinsame Viewer verlangt eine korrelierte FocusConfirmed-Antwort. | `host_frame_send.rs`, `worker_frame_wire.rs`, `wire_binary.rs`, `ipc_protocol_screen.rs`, Desktop-/Mobiledecoder, `ScreenDecoder.kt`, `ScreenViewScreen.kt` |
| Quellen-/Nodepin | Der Appfensteröffner bindet Request, App, Benutzer, Sitzung, Quell-ID und Generation. `PinnedMetadata` und `PinnedStream` sind additive IPC-Varianten; ein alter Daemon kann die neue Variante ablehnen, statt einen unbekannten optionalen Pin still zu ignorieren. | `family_picker.rs`, `FamilyTargetsDialog.kt`, `screen_ipc_client.rs`, `screen_ipc.rs`, `node_screen_link.rs` |
| Bedienwege | Normale Bildschirmansicht bleibt erreichbar. Im Familienmodus bleibt die ausgewählte Familiensource fest. Reguläres Programmschließen und Trennen sind getrennte Aktionen; Close trennt den Stream nicht sofort und lässt einen Speicherdialog sichtbar werden. | `family_viewer.rs`, `screen_launch.rs`, `FamilyScreen.kt`, `FamilySession.kt`, `ScreenViewScreen.kt` |
| Android-Start | Kotlin benutzt einen LauncherApps-Katalog der eigenen Android-Benutzerkennung, einen tatsächlichen sichtbaren/entsperrten Activityzustand und eine einmalige native Admission mit Ablauf. Background-/Launcher-/Accessibility-Rollen werden nicht pauschal als Startberechtigung angenommen. Hostevents erreichen eine begrenzte direkte Queue vor dem verlustbehafteten UI-Flow. | `AppCatalog.kt`, `AppLaunch.kt`, `AppHostRuntime.kt`, `Core.kt`, `MainActivity.kt`, `apps/os/android/bridge.rs` |
| Portal-/Android-Quellen | ScreenCast-v6-Serial wird aus dem wirklichen Startresultat übernommen; nach terminalem Quellenverlust kann die Streamidentität nicht wiederbelebt werden. Android behauptet für die freie MediaProjection-Systemauswahl keine bestätigte Paketidentität und lehnt entsprechende Fensterbehauptungen ab. | `portal_source_binding.rs`, `pipewire_identity.rs`, `portal_input.rs`, `ScreenProjectionIdentity.kt`, `ScreenConsentActivity.kt`, `bridge_backend.rs` |

## Primärquellenabgleich und ungeprüfte Grenzen

- Linux-pidfd: [Linux-man-pages, pidfd_open(2)](https://man7.org/linux/man-pages/man2/pidfd_open.2.html), Kernel-API seit Linux 5.3, Abruf 2026-10-25. Die dokumentierte Wiederverwendungsgefahr nach externem Reaping begründet die Übergabe des ursprünglichen Deskriptors; der tatsächlich verwendete Kernel und dessen Ressourcenverfügbarkeit wurden nicht geprüft.
- Linux-Desktopstart: [Desktop Entry Specification 1.5, Exec](https://specifications.freedesktop.org/desktop-entry/1.5/exec-variables.html) und [D-Bus Activation](https://specifications.freedesktop.org/desktop-entry/1.5/dbus.html), Abruf 2026-10-25. Der untersuchte Parser führt keine Shell aus. `org.freedesktop.Application.Activate` besitzt keinen Prozessrückgabewert. Der D-Bus-Startpfad erzeugt im aktuellen Code keinen Originalprozesszeugen und kann deshalb keinen automatischen P4-Fensterbeleg liefern. Dasselbe gilt für Shellstarts ohne ursprüngliches Prozesshandle und für Android ohne bestätigte Paket-/Fensteridentität. Das sind konkrete verbliebene Plattformgrenzen, keine bestandenen Start-/Fenstertests.
- X11: [EWMH 1.5, `_NET_ACTIVE_WINDOW`](https://specifications.freedesktop.org/wm/1.5/ar01s03.html), Abruf 2026-10-25. Aktivierung ist eine Anforderung; eine WM darf sie ablehnen. Der Code bestätigt Fokus separat. Reale WM-Umsetzung, verzögerte Fokusantworten, Override-Redirect-Menüs, Modaldialoge und XComposite-Aufnahme wurden nicht ausgeführt.
- Windows: [SetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow), [GetWindowRect](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect) und [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput), Abruf 2026-10-25. Fokusprüfung ist keine atomare Appisolation der sitzungsweiten Eingabe. Der in V6 ausdrücklich benannte Restrisikoübergang zwischen letzter Fokusabfrage und OS-Injektion bleibt bestehen; B8-R4 ist zusätzlich ein konkret behebbarer Zustandsverlust.
- Rust-Windowsbindung: [windows-rs 0.58.0, Shell-Quelltext](https://raw.githubusercontent.com/microsoft/windows-rs/0.58.0/crates/libs/windows/src/Windows/Win32/UI/Shell/mod.rs), Abruf 2026-10-25. `IEnumShellItems::Next` liefert in genau dieser Bindingsversion `Result<()>`. Der geprüfte `map_err`-Aufruf ist deshalb kein belegter Typfehler.
- Portal: [ScreenCast-Interface v6](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html), Abruf 2026-10-25. Die dokumentierte `pipewire-serial`-/`target.object`-Bindung und die compositorlogischen Größen werden getrennt von Capturepixeln behandelt. Konkrete GNOME-/KDE-/wlroots-Portalversionen, Wiredecoding auf diesen Hosts und PipeWire-Lebensdauer wurden nicht praktisch geprüft.
- Android: [Activity Security/BAL](https://developer.android.com/guide/components/activities/secure-bal), Regeln seit Android 10/API 29, Abruf 2026-10-25. Die engere sichtbare-Activity-Regel im Code behauptet keine nicht belegte BAL-Ausnahme. Android-OS-Version, installierter Launcher, Benachrichtigungsrechte, tatsächliche MediaCodec-Callbacks und sichtbare Activityübergänge wurden nicht praktisch geprüft.
- Zusätzlich nach Freigabe gelesen: `native/src/mobile/os/shared/domains/app_launch.rs`, `native/src/daemon/os/shared/ipc_host.rs` und `native/src/daemon/os/shared/exec_grant_journal.rs`. Der lokale JNI-Router bindet Hostantworten an die Bridge-Challenge. Die zunächst offene Erstgrant-Zusammenführung ist durch den dokumentierten Ergänzungsreview geschlossen: tatsächlicher Patch, Policykonversion, V2-Serialisierung, Journal, Policy-/Dateirevisions-CAS und aktive Identitätsprüfung sind jetzt nachgelesen. Die OS-Promote-/Privatdateigrenze bleibt ausdrücklich außerhalb dieses Reviews.
- Im Ausgangsreview waren die WGC-/XComposite-Rohcaptureimplementierungen ausgeschlossen. Der Capture-Nachtrag B8-R6 erweitert diesen Umfang gezielt auf die inventarisierten Quellöffnungs-/Pixmap-/WGC-Zweige; die vollständige B6-Pipeline, die D3D-Readbackimplementierung und der X11-Bildkonvertierungspfad sind damit nicht vollständig bewertet. TCP/RDP und der P1-Broker haben einen eigenen B7-Review. Es wurden keine lokalen oder entfernten Tests und keine Compiler ausgeführt.

## Reviewentscheidungen und offener Korrekturstand

- Die angefragten Windows-/Linux-Inputadapter, `SourceId`, Android-Decoder und Mobile-PRESENTED-Consumer wurden als zusätzliche Lesefläche freigegeben und untersucht.
- Die zusätzliche Lesefläche `AppHostRuntime.kt`, `AppLaunch.kt`, `AppCatalog.kt`, `AppIcons.kt`, `Core.kt` und `MainActivity.kt` wurde freigegeben und untersucht; daraus stammt B8-R5.
- B8-R1 bis B8-R5 einschließlich des zusätzlichen X11-Releasepfads sind im zuletzt gelesenen Worktree statisch korrigiert. Diese Aussage betrifft ausschließlich die beschriebenen Auslöser und Kontrollflüsse. Typauflösung, Plattformlaufzeit und die oben konkret abgegrenzten nicht gelesenen Implementierungen bleiben ungeprüft.
- B8-R6 ist nach Autorenfreigabe statisch korrigiert: eigener verweigernder Familieneinstieg, X11-Pixmapprüfung vor jedem sichtbaren Mitgliedsbild, kein Rootcrop bei Belegverlust, erhaltener gewöhnlicher Raw-Window-Fallback und erhaltener Windows-WGC-Familienpfad. Der genaue Zeitpunkt der ersten Pixmapprüfung und die verbleibenden Grenzen stehen im Nachtrag.
- Der Ergänzungsreview der Erstgrant-Zusammenführung ergibt keinen zusätzlichen belegten Defekt. Die unterschiedliche Room-Prüfung wurde anhand der tatsächlich gelesenen Zustandsübergänge eingeordnet; daraus wird kein unbewiesener Sicherheitsbefund erzeugt. Login-Maschinenrechte werden nicht aus einer Appfreigabe abgeleitet. Die genaue Abgrenzung steht im Ergänzungsabschnitt.
- Der Reviewer ändert ausschließlich diesen Bericht. Der Hauptagent koordiniert Reparaturen und die spätere Remoteprüfung. Der Bericht setzt keinen neuen Releasehalt wegen fehlender Realweltzugänge.


## Vollständiges Lese- und Schreibinventar

Die 154 Manifestpfade plus die ausdrücklich freigegebenen Ergänzungen ergeben 190 verschiedene Repositoryquelldateien zusätzlich zu diesem gelesenen und geschriebenen Bericht. Der Erstgrant-Ergänzungsauftrag hat 13 neue Pfade hinzugefügt; `access_grant.rs` und `exec_grant_journal.rs` waren bereits erfasst. Der Capture-Nachtrag B8-R6 ergänzt sechs neue Pfade: beide Oberflächendokumente sowie die beiden X11- und beiden Windows-Capturedateien. `Q` bezeichnet inhaltlich oder an konkreten Symbolen/Registrierungen untersuchten Quelltext, einschließlich gezielter statischer Suchen; es behauptet keine vollständige Zeilenprüfung der gesamten Datei. `I` bezeichnet ausschließlich die technische Umfangserfassung beziehungsweise nur einen beiläufigen Treffer und ist ausdrücklich keine semantische Reviewfreigabe dieser Datei. Alle hier aufgeführten Dateien wurden ausschließlich gelesen.

Die zusätzliche Freigabe für `types.rs` beschränkt sich auf `RoomMember` mit seinen unmittelbar zugehörigen Zustandsfeldern; gelesen wurden Zeilen 104–132 sowie die Suche nach diesen Symbolen. `room_relation_members.rs` wurde für `is_admitted` und die zugehörigen Zustandsübergänge gelesen. `profile_store.rs` wurde für `mutate_persisted`, Dateirevisions-CAS, Staging und Lockhilfen gelesen. `profile_transaction.rs` wurde einschließlich des vorhandenen Testquelltexts gelesen; kein Test wurde ausgeführt. Der Report selbst wurde vor seinen Ergänzungen erneut gelesen und bleibt die einzige Schreibdatei. `git status --short` und `git log -1 --oneline` dienten dem vorgeschriebenen Dokumentationskontext; daraus wurden keine nicht freigegebenen Produktdateien geöffnet.

Zusätzlich gelesen: `/tmp/smartexplorer-review-b8-scope.json`, `/root/.agents/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/arbeitsweise/SKILL.md` sowie die im Quellenabschnitt genannten Primärseiten. Die aktuelle Arbeitsweise wurde über das MCP geladen. Keine Quelle außerhalb der freigegebenen Repositorypfade wurde im Repository erkundet.

```text
Q AGENTS.md
Q android/app/src/main/java/app/smartexplorer/android/MainActivity.kt
Q android/app/src/main/java/app/smartexplorer/android/api/AppsApi.kt
Q android/app/src/main/java/app/smartexplorer/android/api/FamilyApi.kt
I android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt
I android/app/src/main/java/app/smartexplorer/android/api/TcpApi.kt
Q android/app/src/main/java/app/smartexplorer/android/core/Core.kt
Q android/app/src/main/java/app/smartexplorer/android/core/CoreEvents.kt
Q android/app/src/main/java/app/smartexplorer/android/screen/AppCatalog.kt
Q android/app/src/main/java/app/smartexplorer/android/screen/AppHostRuntime.kt
Q android/app/src/main/java/app/smartexplorer/android/screen/AppIcons.kt
Q android/app/src/main/java/app/smartexplorer/android/screen/AppLaunch.kt
Q android/app/src/main/java/app/smartexplorer/android/screen/ScreenConsentActivity.kt
Q android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt
Q android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostNotifier.kt
Q android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionIdentity.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/screen/FamilyPicker.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/screen/FamilyScreen.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/screen/FamilySession.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenDecoder.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenSession.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenTarget.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewScreen.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/share/AppGrantDialog.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/share/AppsDialog.kt
I android/app/src/main/java/app/smartexplorer/android/ui/share/DeviceSettingsPage.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/share/FamilyTargetsDialog.kt
Q android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDialogHost.kt
I android/app/src/main/java/app/smartexplorer/android/ui/share/ShareScreen.kt
Q docs/ARCHITEKTUR.md
Q docs/oberflaechen/bildschirm.md
Q docs/oberflaechen/fernzugriff.md
Q docs/plaene/2026-10-24-batches/batches.md
Q docs/plaene/2026-10-24-batches/recherche.md
Q docs/plaene/2026-10-24-batches/umsetzung.md
Q docs/plaene/2026-10-24-batches/vertraege.md
Q native/Cargo.lock
Q native/Cargo.toml
I native/src/app/core/family_device_ui.rs
Q native/src/app/core/family_picker.rs
Q native/src/app/core/family_viewer.rs
Q native/src/app/core/family_viewer_events.rs
Q native/src/app/core/family_viewer_session.rs
I native/src/app/core/frame_update.rs
Q native/src/app/core/screen_decode_thread.rs
Q native/src/app/core/screen_launch.rs
Q native/src/app/core/screen_session.rs
Q native/src/app/core/screen_target.rs
Q native/src/app/core/screen_view_widget.rs
Q native/src/app/core/share_app_grant_ui.rs
Q native/src/app/core/share_apps_jobs.rs
I native/src/app/core/share_apps_store.rs
Q native/src/app/core/share_apps_ui.rs
I native/src/app/core/share_device_page_ui.rs
Q native/src/app/mod.rs
Q native/src/app/screen_modules.rs
Q native/src/apps/core/grant.rs
Q native/src/apps/core/mod.rs
Q native/src/apps/core/model.rs
Q native/src/apps/mod.rs
Q native/src/apps/os/android/bridge.rs
Q native/src/apps/os/android/launch.rs
Q native/src/apps/os/android/mod.rs
Q native/src/apps/os/host.rs
Q native/src/apps/os/linux_os/catalog.rs
Q native/src/apps/os/linux_os/desktop_entry.rs
Q native/src/apps/os/linux_os/launch.rs
Q native/src/apps/os/linux_os/mod.rs
Q native/src/apps/os/linux_os/session.rs
Q native/src/apps/os/linux_os/window_correlation.rs
Q native/src/apps/os/mod.rs
Q native/src/apps/os/shared/journal.rs
Q native/src/apps/os/shared/mod.rs
Q native/src/apps/os/shared/platform.rs
Q native/src/apps/os/shared/unsupported.rs
Q native/src/apps/os/shared/window_correlation.rs
Q native/src/apps/os/windows/catalog.rs
Q native/src/apps/os/windows/launch.rs
Q native/src/apps/os/windows/mod.rs
Q native/src/apps/os/windows/session.rs
Q native/src/apps/os/windows/storage.rs
Q native/src/apps/os/windows/window_correlation.rs
Q native/src/cli/apps.rs
Q native/src/cli/mcp/apps_tool.rs
I native/src/cli/mcp/screen_tool.rs
Q native/src/cli/mcp/screen_window_tool.rs
Q native/src/cli/mcp/server.rs
Q native/src/cli/mod.rs
I native/src/cli/screen.rs
Q native/src/cli/screen/families.rs
I native/src/cli/screen/input.rs
I native/src/cli/screen/view.rs
Q native/src/daemon/mod.rs
Q native/src/daemon/os/shared/access_grant_journal.rs
Q native/src/daemon/os/shared/apps_host.rs
Q native/src/daemon/os/shared/apps_ipc.rs
Q native/src/daemon/os/shared/apps_ipc_client.rs
Q native/src/daemon/os/shared/exec_grant_journal.rs
Q native/src/daemon/os/shared/ipc.rs
Q native/src/daemon/os/shared/ipc_host.rs
Q native/src/daemon/os/shared/ipc_protocol.rs
Q native/src/daemon/os/shared/ipc_protocol_apps.rs
Q native/src/daemon/os/shared/ipc_protocol_screen.rs
Q native/src/daemon/os/shared/screen_ipc.rs
Q native/src/daemon/os/shared/screen_ipc_client.rs
Q native/src/daemon/os/shared/screen_ipc_families.rs
Q native/src/daemon/os/shared/screen_ipc_still.rs
Q native/src/lib.rs
I native/src/mobile/core/error.rs
Q native/src/mobile/mod.rs
Q native/src/mobile/os/shared/domains/app_launch.rs
Q native/src/mobile/os/shared/domains/apps.rs
Q native/src/mobile/os/shared/domains/mod.rs
Q native/src/mobile/os/shared/domains/screen_family.rs
Q native/src/mobile/os/shared/domains/screen_host.rs
Q native/src/mobile/os/shared/domains/screen_view.rs
Q native/src/mobile/os/shared/domains/screen_view_presented.rs
Q native/src/mobile/os/shared/domains/screen_view_run.rs
Q native/src/remote_window/INTEGRATION.md
Q native/src/remote_window/core/host.rs
Q native/src/remote_window/core/mod.rs
Q native/src/remote_window/core/model.rs
Q native/src/remote_window/core/selection.rs
Q native/src/remote_window/core/wire.rs
Q native/src/remote_window/mod.rs
Q native/src/remote_window/os/mod.rs
Q native/src/remote_window/os/shared/family_source.rs
Q native/src/remote_window/os/windows.rs
Q native/src/remote_window/os/windows/facts.rs
Q native/src/remote_window/os/x11.rs
Q native/src/remote_window/os/x11/properties.rs
Q native/src/screen/core/family_listing.rs
Q native/src/screen/core/family_protocol.rs
Q native/src/screen/core/family_viewer.rs
Q native/src/screen/core/host_backend.rs
Q native/src/screen/core/host_family.rs
Q native/src/screen/core/host_frame_send.rs
Q native/src/screen/core/host_server.rs
Q native/src/screen/core/window_list.rs
Q native/src/screen/core/wire_binary.rs
Q native/src/screen/core/wire_control.rs
Q native/src/screen/core/wire_hello.rs
Q native/src/screen/core/worker_family.rs
Q native/src/screen/core/worker_frame_wire.rs
Q native/src/screen/core/worker_wire.rs
Q native/src/screen/os/android/bridge_backend.rs
Q native/src/screen/os/android/bridge_channels.rs
Q native/src/screen/os/linux_os/host_backend.rs
Q native/src/screen/os/linux_os/pipewire_abi.rs
Q native/src/screen/os/linux_os/pipewire_identity.rs
Q native/src/screen/os/linux_os/pipewire_pause.rs
Q native/src/screen/os/linux_os/pipewire_stream.rs
Q native/src/screen/os/linux_os/portal_input.rs
Q native/src/screen/os/linux_os/portal_session.rs
Q native/src/screen/os/linux_os/portal_source_binding.rs
Q native/src/screen/os/linux_os/x11_capture.rs
Q native/src/screen/os/linux_os/x11_capture_target.rs
Q native/src/screen/os/linux_os/x11_input.rs
Q native/src/screen/os/shared/family_backend.rs
Q native/src/screen/os/shared/host_worker_backend.rs
Q native/src/screen/os/shared/host_worker_client.rs
Q native/src/screen/os/shared/host_worker_main.rs
Q native/src/screen/os/windows/capture_source.rs
Q native/src/screen/os/windows/capture_wgc.rs
Q native/src/screen/os/windows/host_backend.rs
Q native/src/screen/os/windows/input_integrity.rs
Q native/src/screen/os/windows/input_send.rs
Q native/src/share/core/access_authorize.rs
Q native/src/share/core/access_constraints.rs
Q native/src/share/core/access_grant.rs
Q native/src/share/core/access_grant_runtime.rs
Q native/src/share/core/access_grant_wire.rs
Q native/src/share/core/access_relation_view.rs
Q native/src/share/core/exec_access_view.rs
Q native/src/share/core/exec_grant_persistence.rs
Q native/src/share/core/exec_grant_runtime.rs
Q native/src/share/core/exec_policy.rs
Q native/src/share/core/exec_policy_wire.rs
Q native/src/share/core/node_apps.rs
Q native/src/share/core/node_apps_link.rs
Q native/src/share/core/node_apps_window.rs
Q native/src/share/core/node_screen.rs
Q native/src/share/core/node_screen_link.rs
I native/src/share/core/node_screen_tests.rs
I native/src/share/core/relation_proof.rs
Q native/src/share/core/room_relation_members.rs
Q native/src/share/core/types.rs
Q native/src/share/mod.rs
Q native/src/share/os/shared/profile_store.rs
Q native/src/share/os/shared/profile_transaction.rs
```

Neu erstellt und anschließend ergänzt: ausschließlich `docs/plaene/2026-10-24-batches/review-b8.md`. Keine Produktdatei erstellt oder geändert. Keine Builds, Compiler, Testausführungen, Server, Installationen, Graphneubauten, Commits, Pushes oder Releases durchgeführt. Eine zunächst verwendete `python`-Programmsuche scheiterte mit „command not found“; die reine Text-/Inventarverarbeitung wurde mit vorhandenem `python3` ausgeführt. Das ist keine Produktprüfung.
