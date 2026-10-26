# B7 — statischer Review von Maschinenanmeldung, TCP und RDP

Stand des abgeschlossenen Nachreviews: 2026-10-26, 00:19 UTC. Gegenstand ist der aktuelle Arbeitsbaum, kein unveränderlicher Releasekandidat. Zuletzt gelesener HEAD: `8d970a63` (`feat(apps): authorize catalog launches and preserve confirmed outcomes [task candidate]`); der Arbeitsbaum enthält umfangreiche parallele Änderungen. Zeilennummern im ursprünglichen Befund bezeichnen die zuerst gelesene Fassung; die Schließungsbelege nennen die Fassung des Nachreviews.

Auftrag: unabhängiger kritischer Quellreview von P1a–P1f/B7 gegen `batches.md` D7, `vertraege.md` V1/V2 und offizielle Primärquellen. Prüffragen waren Maschinenautorität, Besitz und Abbruch der Kanäle/Prozesse, tatsächliche Darstellung vor Eingabe, Desktop-/Sitzungsbindung, lebende Hostindikatoren, TCP-Zielbindung, RDP-Zertifikatsverhalten und die tatsächliche Distribution der administrativen Einrichtung. Der Erstplan trennte diese Verträge; die vertiefte Prüfung verfolgte ihre Aufrufer bis zu den zugewiesenen Installer-, UI-, IPC- und Releasegrenzen.

**Ergebnis des Nachreviews:** B7-01, B7-02, B7-03, B7-04 und B7-05 sind statisch geschlossen. Der zusätzlich erhobene DXGI-Befund B7-06 wurde korrigiert und statisch geschlossen. Der Android-Besitzerfehler B7-R1 bleibt statisch geschlossen. In der freigegebenen Nachprüfungsfläche verbleibt kein bestätigter offener Defekt. Compiler-, Test- und Laufzeitverhalten bleiben unbestätigt. Es wurden keine Builds, Compiler, Tests, Server, Installationen, Grapharbeiten, Commits, Pushes oder Releasekommandos ausgeführt.

## Nachreview der integrierten Korrekturen

Die ursprünglichen Fehlerabläufe unten bleiben als nachvollziehbare Reviewbelege erhalten. Ihr aktueller Status ergibt sich aus dieser Tabelle. Für die Schließung wurde jeweils der ursprüngliche Auslöser durch den neuen Code verfolgt; bei TCP wurden zusätzlich Eigentum, Entzug, Halbschließen und vorab gepufferte Dienstbegrüßungen geprüft. Eine bloße angekündigte Umsetzung genügte nicht.

| Befund | Statischer Status | Beleg der Korrektur |
|---|---|---|
| B7-01 | Geschlossen | Capture und tatsächliche Eingabe teilen die Quellgeneration; Workeraufträge behalten die beim Admit gespeicherte Generation. Die Loginadapter halten unveränderliche Geometriekontexte. Ein Wechsel beendet den Helper und den alten Darstellungsnachweis, statt im selben Kontext neue Koordinaten zuzulassen. Die fertiggestellte Integration wurde von Main zur abschließenden Prüfung freigegeben und die betroffenen Grenzen erneut gelesen. |
| B7-02 | Geschlossen | Windows prüft den SHA-256 in einem isolierten geschützten Stagingverzeichnis (`install-login-service.ps1:245`–`256`) und ruft den Offline-Produktvalidator vor jeder Änderung am aktiven Konfigurationsbestand auf (`:319`–`320`). Erst nach vollständiger Journalsicherung und geordnetem Dienststopp werden dessen ACLs geändert (`:347`–`358`). Der ursprüngliche Hashfehler erreicht diese Mutationen nicht. |
| B7-03 | Geschlossen | Beide Linux-Units verwenden `Type=notify` und `NotifyAccess=main` (`install-login-service.sh:311`, `:338`). Der Broker meldet Bereitschaft nach Listener-/Statusinitialisierung (`service.rs:54`–`55`); der Gateway nach Schlüsselprüfung, Endpointbindung und Adresspublikation (`gateway.rs:25`–`51`). Das Skript prüft konkrete PID, InvocationID, StatusText, UID, Imagepfad und Hash vor `rollback_active=0` (`:363`–`382`). |
| B7-04 | Geschlossen | `node_tcp_target.rs:29`–`71` erzeugt je Zieladresse eine eigene konkrete Anfrage und Lease, verbindet diese Adresse tatsächlich und publiziert die Registryaufnahme anschließend unter der bestehenden Autorität. Ein Verbindungsfehler beendet deren Lease; der nächste Kandidat benötigt seine eigene Autorisierung. Der erste erfolgreiche Socket bleibt erhalten. |
| B7-05 | Geschlossen | Beide Skripte stehen im 26-Asset-Mapping (`release-publication.ps1:156`–`164`), in CI-Staging, Bytevergleich und Veröffentlichung (`build.yml:767`, `:861`, `:962`, `:1134`). NSIS legt das Windows-Skript neben `se.exe` ab (`installer.nsi:127`); die Kandidatenprüfung vergleicht die eingebetteten Bytes mit dem veröffentlichten Quellasset (`release-publication.ps1:516`–`519`). README und RELEASING erklären Bezug, Erstinstallation, administratives Update und Uninstall. |
| B7-06 | Geschlossen | `capture_dxgi.rs:273` gibt die alte Duplication vor dem erneuten `DuplicateOutput` frei. Cache und Readback werden vorher geleert. Ein fehlgeschlagener Neuaufbau lässt die Duplication ausdrücklich fehlen; `source_available` und die zugreifenden Methoden melden diesen Zustand. |

**B7-01 — Nachweis am fertiggestellten Integrationsstand:** `x11_capture_target.rs:36`–`52` erfasst einen geänderten Ursprung ebenso wie geänderte Abmessungen, erhöht die Referenzgeneration, verwirft alte Bilddaten und liest die Monitorgeometrie vor jedem Capture erneut. `x11_capture.rs:381`–`394` prüft bei der tatsächlichen Eingabe sowohl die zugelassene Generation als auch die aktuelle Geometrie. `inject_bound` verwendet das bestätigte Rechteck und verweigert eine Abweichung, statt die Eingabe neu auf einen verschobenen Monitor abzubilden (`x11_input.rs:242`–`264`).

Die Generation wird bereits vor dem Warten auf den Worker gespeichert (`host_session.rs:353`–`369`) und über `BoundInput` bis zu `source.inject_input` transportiert (`worker_wire.rs:139`–`143`, `host_worker_backend.rs:271`–`287`, `host_worker_main.rs:228`–`235`). Eine in der Zwischenzeit aufgenommene neue Generation kann den alten Auftrag daher nicht nachträglich autorisieren. Der aktuelle Präsentationsnachweis gehört zur Quellgeneration (`host_reference.rs:29`–`51`). Ein v1-Viewer ohne Präsentationsprotokoll erhält nach erkannter X11-Geometrieänderung keine Steuerung mehr und wird ausdrücklich auf einen neuen Verbindungskontext verwiesen; die Aufnahme bleibt verfügbar (`host_session.rs:357`–`386`).

Der Linux-Loginpfad hält zusätzlich eine unveränderliche Lease aus Monitorrechteck, Rootgröße und Primärstatus (`login_geometry.rs:8`–`43`); Capture und Eingabe prüfen dieselbe Lease (`linux_os/login_source.rs:18`–`29`, `:55`–`65`). Der Windows-Loginpfad bindet das bereits geöffnete DXGI-Outputobjekt, Monitorhandle, Rechteck, Rotation, Bildgröße und Duplicationgeneration sowie das virtuelle Desktoprechteck (`windows/login_source.rs:21`–`44`; `capture_dxgi.rs:110`–`133`). Seine Eingabe prüft diesen Capturekontext erneut. Nach einer Abweichung bleibt er ungültig. Reine Freigaben umgehen die neue Monitorauflösung; Touch-Ende/-Abbruch wird vor dem Recordbau in Button-up übersetzt (`windows/login_source.rs:86`–`109`, `input_send.rs:184`–`218`). Die X11-Freigabe erzeugt ebenfalls nur geprüfte Key-/Button-ups (`x11_input.rs:269`–`311`). Primärbelege für die Trennung von Bewegung und Freigabe: [MOUSEINPUT](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-mouseinput) und [KEYBDINPUT](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-keybdinput), abgerufen 2026-10-26.

`helper.rs:15`, `:27`, `:38` und `:47` prüfen die Bindung bei Initialisierung, im laufenden Kontext sowie vor Operationen und vor Pixelpublikation. Der Fehlerpfad gibt gehaltene Eingaben frei und beendet den Helper. `broker.rs:59`–`64` beendet anschließend die bestehende Verbindung ohne Folgechallenge. Auch SAS benötigt zuerst eine erfolgreiche Antwort des laufenden Helpers (`:44`–`53`). Der gemeinsame Viewer schließt bei terminalem Ende den Guard und verwirft Bild- und Eingabebelege (`viewer.rs:30`–`43`, `:199`–`206`; `viewer_guard.rs:59`). Der ursprüngliche dauerhafte Versatz ist in diesen gelesenen Pfaden verhindert. Die statische Schließung umfasst die freigegebene B7-Integration; X11- und Windows-Ausführung bleiben unbestätigt.

**B7-02/03 — Fehlerpfade und Bereitschaft:** Der neue interne Einstieg `login_entry.rs:6` ruft ausschließlich `installation::validate_candidate` auf. Dieser prüft tatsächliche Binaryversion, Manifest, Maschinenkey zur Policy, Netzkonfiguration und Imagehash, ohne Dienst oder Endpoint zu starten (`installation.rs:44`–`64`). Unter Windows sichern Journal und Fehlerpfad Records, ACLs und vorhandene Diensteinstellungen; der Abschluss prüft neue SCM-Prozessinstanzen samt Imagepfad und erneuter Hashbindung (`install-login-service.ps1:96`–`115`, `:397`–`418`). Unter Linux laufen Startfehler vor der Deaktivierung des Rollbacks; Records, Verzeichnismetadaten sowie vorheriger Aktivierungszustand werden wiederhergestellt. `readiness.rs:3`–`13` sendet das vom langlebigen Hauptprozess stammende `READY=1` zusammen mit Version, Generation und Hash. Damit ist die ursprüngliche Verwechslung von Fork-Erfolg und Anwendungsbereitschaft beseitigt. Diese Prüfung bestätigt den Initialisierungszeitpunkt, keine künftige störungsfreie Verfügbarkeit. Primärquellen: [systemd.service, Type=notify](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html#Type=), [sd_notify: Protokoll und Absenderzuordnung](https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html), [Microsoft: SetServiceStatus](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-setservicestatus), erneut abgerufen 2026-10-25.

**B7-04 — Besitz und erhaltene TCP-Semantik:** `connect_bound_reusing` überwacht Rechteentzug während DNS/Connect, prüft nach dem Connect erneut und bindet einen übernommenen Socket an exakt dieselbe `Arc<AccessHandle>` und Adresse (`tcp_sockets.rs:48`–`88`). `admit_existing_access` hält Lifecycle- und Autorisierungsgrenze während der erneuten Leaseprüfung und Registryaufnahme (`node_access.rs:59`–`65`, `access_revocation.rs:179`–`195`). `node_tcp.rs:159` übergibt den ersten Socket einmalig; spätere Streams verbinden nur die bereits gebundene IP. Es erfolgt keine Lese-/Peek-Probe, die gepufferte Begrüßungsbytes verbraucht. Ein reines FIN wird nicht als fehlende Verbindung verworfen; die zwei Pump-Richtungen werden weiterhin getrennt beendet (`tcp_pump.rs:21`–`27`, `:44`–`48`). `SO_ERROR` und `peer_addr` werden lediglich auf bereits bekannte Fehler beziehungsweise Adressidentität geprüft, nicht als Beweis einer zukünftigen Dienstantwort. Die Wirkung eines späteren Reset bleibt ein normaler Transportfehler. Primärbeleg: [Tokio 1.52.3: TcpStream, take_error, peer_addr und shutdown](https://docs.rs/tokio/1.52.3/tokio/net/struct.TcpStream.html), erneut abgerufen 2026-10-25.

**B7-05 — Dokumentationsnachtrag nachgelesen:** `docs/RELEASING.md:832`–`843` nennt jetzt ausdrücklich das erneute administrative Installieren mit neuer Version/Hash und erhaltener Maschinenidentität sowie beide Uninstallaufrufe. Linux verwendet `bash`. Policy, private Identität, Netzkonfiguration und Generationen bleiben beim Uninstall erhalten. Die normale Benutzerinstallation aktiviert weiterhin keinen Login-Dienst. Statisch ist die zuvor fehlende Auslieferungsstrecke geschlossen; ein gebauter Installer oder ein veröffentlichter Download wurde nicht geprüft.

**B7-05 — Plattformübergreifender Bytevertrag:** Der Windows-Wrapper bildet den erwarteten Skriptdigest aus den lokalen Quelldateibytes, während das Linux-Publishing die Bytes seines Checkouts verwendet. Die ergänzte genaue Regel `/native/install-login-service.ps1 text eol=lf` in `.gitattributes:3` beseitigt die zuvor mögliche CRLF/LF-Abweichung bei `core.autocrlf=true`; das Linuxskript ist bereits durch `*.sh text eol=lf` gebunden. Damit verwendet ein regulärer Checkout auf beiden Plattformen denselben Zeilenendestil für beide neuen Assets und das eingebettete Windows-Skript. Die Regel wurde nach ihrer Ergänzung gelesen. Primärbeleg: [Git: text- und eol-Attribute beim Ein- und Auschecken](https://git-scm.com/docs/gitattributes), abgerufen 2026-10-25.

### B7-06 · P2 · DXGI-Wiederaufbau hält die alte Duplication während der Neuanlage

**Fundstellen:** `native/src/screen/os/windows/capture_dxgi.rs:233`–`245`, insbesondere `self.live = duplicate(&self.device)?` in Zeile 243; `duplicate` ruft `DuplicateOutput` in Zeile 80 auf.

**Konkreter Ablauf und Auswirkung:** Ein Modus- oder Desktopwechsel liefert `DXGI_ERROR_ACCESS_LOST`. `reopen` versucht die neue Duplication anzulegen, während `self.live` die bisherige Schnittstelle noch besitzt. Rust wertet die rechte Zuweisungsseite vor der Freigabe des alten Feldwerts aus. Damit verletzt der Wiederaufbau die vorgeschriebene Reihenfolge und kann mit `E_INVALIDARG` scheitern, weil derselbe Prozess den Output noch dupliziert. Eine wiederherstellbare Captureunterbrechung wird so zum Capturefehler. Der Login-Kontext muss beim Geometriewechsel ohnehin enden; dieser zusätzliche Befund betrifft den gemeinsam genutzten DXGI-Adapter und behauptet keine Eingabe unter fremder Autorität.

**Primärbelege:** Microsoft verlangt nach `ACCESS_LOST` ausdrücklich das Freigeben der alten Duplication vor ihrer Neuanlage; `DuplicateOutput` nennt eine bereits bestehende Duplication desselben Outputs als `E_INVALIDARG`-Grund. [AcquireNextFrame](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe), [DuplicateOutput](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput), [Rust Reference: Assignment expressions](https://doc.rust-lang.org/reference/expressions/operator-expr.html#assignment-expressions), abgerufen 2026-10-26.

**Statische Schließung:** Die korrigierte Fassung hält die Duplication als `Option`. `capture_dxgi.rs:273` führt `drop(self.live.duplication.take())` vor der Neuanlage in Zeile 276 aus; Readback und Bildcache werden in Zeile 274 geleert. Der fehlerhafte Öffnungsversuch lässt `None` zurück. `live_duplication` und `output_binding` liefern dann einen Fehler, `Grabber::source_available` liefert `false`. `capture_source.rs:201`–`205` verweigert die Wiederaktivierung dieses fehlenden Capturebestands. Der erfolgreiche Neuaufbau erhöht weiterhin die Bildgeneration; der Login-Kontext akzeptiert diese neue Generation nicht als Fortsetzung des alten Darstellungsnachweises. Der konkrete API-Verstoß ist damit statisch geschlossen; die Windows-Laufzeitwirkung bleibt unbestätigt.

## Ursprüngliche Befunde und ihre Auslöser

### B7-01 · P1 · X11-Monitorverschiebung trennt dargestellten Ausschnitt und Eingabeziel

**Fundstellen:** `native/src/screen/os/linux_os/x11_capture.rs:430`, `native/src/screen/os/linux_os/x11_input.rs:69`, `native/src/screen/os/linux_os/x11_input.rs:225`, `native/src/screen/os/linux_os/x11_monitors.rs:64`, `native/src/login/os/linux_os/session.rs:28`, `native/src/screen/os/linux_os/login_source.rs:12`.

**Konkreter Ablauf:** Ein X11-Greeter verwendet einen 3840×1080 großen Root-Framebuffer. Der ausgewählte Monitor hat zunächst den Ausschnitt `(0,0,1920,1080)`. Eine RandR-Änderung verschiebt denselben benannten Monitor nach `(1920,0,1920,1080)`; die Rootgröße bleibt gleich. Die Monitor-ID bleibt erhalten, weil `x11_monitors` sie ausschließlich aus dem Namen erzeugt. `X11Source::pass` liest weiter die gespeicherten alten Koordinaten. Da der alte Ausschnitt weiterhin innerhalb des Rootfensters liegt, schlägt `pass_monitor` nicht fehl; `refresh_monitor` wird deshalb nicht aufgerufen. `x11_input::inject` ermittelt dagegen bei jeder Eingabe über dieselbe ID die aktuelle Monitorgeometrie und verwendet sofort den neuen Ursprung.

**Auswirkung:** Ein dargestellter Punkt aus der linken Bildschirmhälfte führt zu einer Eingabe in der rechten Hälfte. Der Fehler kann dauerhaft bestehen; er ist kein bloßes Einzelbild während einer Umschaltung. Die Loginbindung enthält Sitzung, Display und Xauthority-Hash, jedoch keine Monitorgeometrie. Deshalb beenden die vorhandenen Sitzungsprüfungen diesen Zustand nicht. Ein gültiger Darstellungsnachweis bestätigt hier Pixel, deren Koordinaten nicht mehr der tatsächlichen Eingabeabbildung entsprechen.

**Primärbeleg:** RandR trennt den Output-/CRTC-Viewport von der Rootgröße und erlaubt dessen Lage innerhalb des gemeinsamen Screens; Konfigurationsänderungen sind asynchron. Das Protokoll rechtfertigt nicht, eine Änderung erst aus einem fehlgeschlagenen Bildabruf abzuleiten. [X.Org RandR-Protokoll, Abschnitte 1.2 und 2](https://xorg.freedesktop.org/archive/current/doc/randrproto/randrproto.txt), abgerufen 2026-10-25.

**Erforderliche Korrektur:** Capture und Input benötigen dieselbe gebundene Geometriegeneration. Ein Wechsel muss die alte Eingabefreigabe sperren, bevor Koordinaten der neuen Geometrie verwendet werden; danach muss ein Bild dieser neuen Geometrie dargestellt werden. Nur häufigeres Aktualisieren der Capturegröße beseitigt die fehlende gemeinsame Bindung nicht vollständig. Erwartetes Ergebnis: Der beschriebene Positionswechsel erzeugt keine Eingabe mit neuem Ursprung unter einem alten Bildnachweis.

### B7-02 · P1 · Windows-Installervorprüfung entzieht der aktiven Installation ihre Leserechte

**Fundstellen:** `native/install-login-service.ps1:64`, `native/install-login-service.ps1:126`, `native/install-login-service.ps1:159`, `native/install-login-service.ps1:191`, `native/install-login-service.ps1:262`, `native/install-login-service.ps1:299`. Aufrufer: `native/src/login/os/windows/protected.rs:25`, `native/src/login/os/shared/installation.rs:15`, `native/src/login/os/shared/gateway.rs:54`.

**Konkreter Ablauf:** Auf einem bereits eingerichteten Host wird `Install` mit vorhandenem Image, syntaktisch gültiger Version und einem falschen, aber formal gültigen SHA-256-Wert gestartet. Noch vor Kopie und Hashprüfung ersetzt Zeile 126 die DACL des aktiven Konfigurationsverzeichnisses durch eine ACL mit ausschließlich SYSTEM- und Administratorzugriff. Das Leserecht der eingeschränkten Gateway-Dienst-SID wird erst in Zeile 191 wieder eingesetzt. Die Hashprüfung in Zeile 159 wirft vorher einen Fehler. `$changed` ist weiterhin `false`; der zugehörige Fehlerpfad entfernt nur neu angelegte Dienste und restauriert die ursprüngliche DACL nicht.

**Auswirkung:** Der unverändert weiterlaufende Gateway muss die Policy spätestens beim nächsten 200-ms-Durchlauf erneut lesen. `open_protected` öffnet dabei ausdrücklich auch jedes Elternverzeichnis mit `READ_CONTROL | FILE_READ_ATTRIBUTES`. Diese Öffnung wird nun verweigert; der Fehler beendet `serve` und dessen Verbindungen. Ein abgelehnter Updateversuch beschädigt somit die zuvor nutzbare Installation, obwohl noch keine neue Generation aktiviert wurde. Selbst eine erfolgreiche, längere Vorprüfung kann den Gateway vor dem vorgesehenen Stoppen beenden.

**Primärbeleg:** Windows prüft die angeforderten Rechte beim Öffnen jedes konkreten Datei-/Verzeichnisobjekts gegen dessen Sicherheitsdeskriptor. Hier beruht der Fehler auf dem expliziten Öffnen des Elternverzeichnisses, nicht auf der falschen Annahme, jede Eltern-DACL kontrolliere automatisch den Zugriff auf eine Kinddatei. [Microsoft: File security and access rights](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights), abgerufen 2026-10-25.

**Erforderliche Korrektur:** Die Vorprüfung darf die Zugriffsfähigkeit der aktiven Generation nicht verändern. Aktive ACLs und Dienstzustand müssen bis zur kontrollierten Umschaltung erhalten bleiben; sämtliche früheren Mutationen brauchen andernfalls eine vollständige Wiederherstellung. Erwartetes Ergebnis: Ein Hashfehler lässt die alte Gateway-Instanz und deren benötigte ACLs unverändert nutzbar.

### B7-03 · P2 · Linux-Installer bestätigt Erfolg vor der Anwendungsbereitschaft

**Fundstellen:** `native/install-login-service.sh:64`, `native/install-login-service.sh:234`, `native/install-login-service.sh:259`, `native/install-login-service.sh:279`, `native/src/login/os/shared/installation.rs:29`, `native/src/login/os/linux_os/service.rs:33`.

**Konkreter Ablauf:** Ein administrativer Installationsaufruf übergibt ein korrekt gehashtes Releaseimage, aber eine davon abweichende syntaktisch gültige Versionsnummer. Der Installer akzeptiert diese Nummer und schreibt sie in das Manifest. Beide Units verwenden `Type=simple`. Nach `systemctl start` setzt das Skript sofort `rollback_active=0` und meldet die Installation als erfolgreich. Der Broker prüft erst in seiner eigenen Initialisierung die Manifestversion gegen `CARGO_PKG_VERSION` und beendet sich bei der Abweichung.

**Auswirkung:** Das Skript kann einen unbrauchbaren Kandidaten erfolgreich aktivieren und seine Wiederherstellung deaktivieren. Die vorherige Generation ist bereits gestoppt und durch die neuen Records/Units ersetzt. Auch fehlerhafte private Schlüssel oder fehlgeschlagene Gateway-Netzinitialisierung werden durch den vorhandenen Abschluss nicht als zuverlässig zugehöriger Installationsfehler erfasst.

**Primärbeleg:** systemd betrachtet `Type=simple` bereits nach dem Fork als gestartet, vor `execve` und vor jeglicher Anwendungsinitialisierung. `systemctl start` kann daher sogar bei einem nicht ausführbaren Dienstimage Erfolg melden. `Type=exec` allein würde die nachgelagerte Policy-/Schlüssel-/Netzinitialisierung ebenfalls nicht bestätigen. [systemd.service, Type=](https://freedesktop.org/software/systemd/man/latest/systemd.service.html#Type=), abgerufen 2026-10-25.

**Erforderliche Korrektur:** Der Transaktionsabschluss muss einen aktuellen Bereitschaftsnachweis der konkreten Broker- und Gateway-Generation abwarten und Initialisierungsfehler dem bestehenden Rollback zuführen. Erwartetes Ergebnis: Der genannte Versionsfehler kann keinen erfolgreichen Installationsabschluss erzeugen.

### B7-04 · P2 · Automatisches Loopbackziel erreicht einen ausschließlich auf IPv6 laufenden Dienst nicht

**Fundstellen:** `native/src/share/os/shared/tcp_sockets.rs:15`, `native/src/share/core/node_tcp.rs:196`, `native/src/share/os/shared/tcp_sockets.rs:39`, `native/src/rdp/core/admission.rs:83`, `native/src/rdp/os/windows/probe.ps1:29`.

**Konkreter Ablauf:** Der Zielhost hat einen Listener ausschließlich auf `[::1]:PORT`; seine TCP-Regel erlaubt Loopback. `resolve` erzeugt IPv4- und IPv6-Loopbackadressen und sortiert sie. `tcp_target` wählt den ersten erlaubten Kandidaten, ohne eine Verbindung herzustellen. Bei erlaubtem IPv4 wird deshalb `127.0.0.1:PORT` gebunden. `connect_bound` verbindet später ausschließlich diese eine numerische Adresse. Nach dem Fehler wird nicht der ebenfalls erlaubte IPv6-Kandidat geprüft; erneutes Öffnen wählt wieder IPv4.

**Auswirkung:** Die zugesagte IPv6-Loopbackvariante funktioniert über das automatische Loopbackziel nicht. Der RDP-Pfad verschärft dies: Er erzeugt zwingend `destination=loopback`, während die Windows-Probe ausdrücklich einen ausschließlich auf `::1` vorhandenen Listener als geeignet anerkennt. Die Oberfläche kann daher einen vorhandenen RDP-Dienst anbieten, den ihr eigener Tunnelpfad nicht erreicht. DNS-Ziele mit mehreren erlaubten Adressen besitzen dieselbe Auswahlgrenze.

**Primärbeleg:** Tokio versucht mehrere Ziele nur, wenn der an `connect` übergebene Wert mehrere Adressen liefert. Der vorliegende Aufruf erhält genau einen `SocketAddr`; es gibt keinen impliziten IPv6-Fallback. Gepinnte Projektversion: Tokio 1.52.3 (`native/Cargo.lock:7466`). [Tokio 1.52.3: TcpStream::connect](https://docs.rs/tokio/1.52.3/tokio/net/struct.TcpStream.html#method.connect), abgerufen 2026-10-25.

**Erforderliche Korrektur im Erstbefund:** Die Auswahl muss einen tatsächlich erreichbaren, für genau diese Adresse autorisierten Kandidaten berücksichtigen. Eine bereits erteilte Rückfragezustimmung darf dabei nicht auf eine neue IP übertragen werden. Erwartetes Ergebnis: Ein ausschließlich auf `::1` erreichbarer, ausdrücklich erlaubter Listener funktioniert; jeder neue Kandidat bleibt an seine eigene Zustimmung/Lease gebunden. Der Nachreview oben bestätigt die entsprechende statische Schließung.

### B7-05 · P2 · Administrativer Installationsweg fehlt in der installierbaren Distribution

**Fundstellen:** `native/src/cli/login.rs:38`, `native/installer.nsi:103`, `.github/workflows/build.yml:1107`. Gezielte Integrationssuche in `native/publish-release-local.ps1`, `native/publish-update.ps1`, `native/publish-linux-feed-wsl.sh`, `native/build.rs`, `native/Cargo.toml`, `README.md` und `docs/RELEASING.md`.

**Konkreter Ablauf:** Ein Nutzer installiert den durch die vorhandenen Releasepfade erzeugten Desktopbestand und führt `se login provision` aus. Die Ausgabe verweist für die eigentliche Einrichtung auf `install-login-service.ps1` beziehungsweise `install-login-service.sh`. Diese neuen Skripte liegen im Repository, werden aber weder vom Windows-Installer noch von der expliziten Releaseasset-Liste oder den geprüften Stagingpfaden mitgeliefert. README und RELEASING beschreiben keinen zugehörigen administrativen Bezugs-/Installations-/Updateweg.

**Auswirkung:** Der bereitgestellte Produktpfad endet bei einem Kommando auf eine nicht mitgelieferte Datei. Die optionale Maschinenanmeldung lässt sich aus der beschriebenen installierbaren Distribution nicht vollständig einrichten; dafür ist ein zusätzlicher, undokumentierter Bezug aus dem Quellrepository nötig. Dies ist keine Forderung nach automatischer Elevation oder Aktivierung: Die getrennte ausdrückliche administrative Einrichtung ist richtig und muss erhalten bleiben.

**Vertragsbeleg:** `vertraege.md:44`–`53` verlangt den geschützten administrativen Updatebestand einschließlich Dokumentation in README/RELEASING; `batches.md:330`–`331` verlangt installierbare Windows-/Linux-Dienstkomponenten. Das tatsächliche Auslieferungsverhalten ergibt sich aus den genannten Quelllisten; eine bereits veröffentlichte GitHub-Releaseinstallation wurde nicht untersucht.

**Erforderliche Korrektur im Erstbefund:** Beide opt-in Installationsskripte müssen über den stabilen Releasepfad versioniert bereitgestellt und mit dem konkreten Bezugs-, Hashprüfungs-, Provisionierungs-, Update- und Uninstallweg dokumentiert werden. Erwartetes Ergebnis: Ein Nutzer mit ausschließlich den vorgesehenen Releaseartefakten kann den ausgegebenen administrativen Installationsweg ausführen. Der Nachreview oben bestätigt die entsprechende statische Schließung.

## Während des Reviews statisch behoben

**B7-R1 · Android-Neustart verwendete den Besitzer der beendeten Sitzung.** In der zuerst gelesenen Fassung ersetzte „Neuen Kontext starten“ durch `attempt++` die `LoginSession`, während `AndroidView` am selben Composition-Ort bestehen blieb. Dessen `factory` hatte `LoginCanvas(context, alteSession)` erzeugt; `LoginCanvas.owner` ist unveränderlich (`LoginInput.kt:21`). `update` ersetzte nur Bild und Steuerungsflag. Die neue Sitzung lieferte somit Bilder, aber `focus`, `presented` und `input` erreichten weiter die beendete Sitzung. Der belegte Effekt war ausbleibende Eingabefreigabe des neuen Kontexts; eine erfolgreiche Eingabe unter fremder Autorität wurde daraus nicht behauptet.

Die aktuelle Fassung von `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginViewScreen.kt:83` umschließt `AndroidView` mit `key(session)`. Der neue Sitzungsbesitzer erzeugt damit einen eigenen Canvas; der konkrete Fehler ist statisch geschlossen. Die alte Sitzung wird weiterhin durch `DisposableEffect(session)` freigegeben und geschlossen. Primärquellen: [AndroidView-Quellvertrag: einmalige factory, wiederholtes update](https://android.googlesource.com/platform/frameworks/support/+/049d5e65e6e205e4366b52ce6690ef6bb537b733/compose/ui/ui/src/androidMain/kotlin/androidx/compose/ui/viewinterop/AndroidView.android.kt) und [Compose-Identität mit key](https://developer.android.com/develop/ui/compose/lifecycle), abgerufen 2026-10-25. Eine Androidausführung wurde nicht vorgenommen.

## Geprüfte Grenzen und Entscheidungen

- Die geprüfte Maschinenautorität verwendet separate administrative Policy, Maschinenidentität, genaue Dienst-SID/UID und peer-signierte Operationen. In den gelesenen normalen Geräte-/Raumeinstiegen wurde kein Pfad gefunden, der daraus Maschinenrechte erzeugt. Diese Aussage ist ein statischer Befund der genannten Fläche, keine allgemeine Sicherheitszertifizierung.
- Der Desktop-/Android-Viewer prüft Bildtoken, Fokus, Steuerungszustand und Präsentationsepoche sowohl vor dem Einreihen als auch vor der Ausführung von Eingaben. Die Darstellung wird am Desktop durch den Paint-Nachweis und unter Android nach `onDraw` bestätigt. B7-01 betrifft zusätzlich die hostseitige Abbildung dieses dargestellten Bildes auf Koordinaten.
- Die gelesenen Besitzerpfade schließen Transport/IPC bei Viewerende, Rechteentzug und Sitzungswechsel. Helper sind durch Besitz, Bindungsprüfung und begrenzte Lease abgesichert. Der Hostindikator verlangt geschützte Statusdateien, höchstens fünf Sekunden Gültigkeit und passende lebende Broker-/Helper-Prozessinstanzen. Ein bloß vorhandener Installationsbestand oder ausgehender Viewer gilt dort nicht als Aktivität.
- Wayland-Greeter und Android-Hostlogin wurden als ausdrücklich nicht verfügbar bewertet. Ihre Nichtverfügbarkeit ist kein offener Implementierungsfehler dieses Reviews.
- RDP nutzt installierte Clients und den authentifizierten TCP-Weg. Die geprüften Adapter behaupten keine exklusive Zertifikatspin-Garantie für einen Client, der nur Pin-oder-native-Vertrauensprüfung kann; unsichere Prüfabschaltung wurde nicht als Lösung akzeptiert.
- Es werden keine Produktdateien durch diesen Review geändert. Die bestätigten Fehler wurden Main zur getrennten Umsetzung gemeldet. Die Installer-, TCP- und Distributionskorrekturen wurden anschließend auf den betroffenen Fehlerpfaden nachgelesen; der Nachreview umfasst keine neue Vollprüfung des übrigen Releases.

## Verbleibende Prüflücken

- Rust- und Kotlin-Kompilierbarkeit, tatsächliche Windows-SCM-/Token-/Secure-Desktopwirkung, systemd-Start/Rollback, X11-Greeterbedienung, Androidlebenszyklus und RDP-Clientwirkung sind ohne Ausführung unbestätigt.
- Der zusätzliche X11-Scope erlaubte Capture, Input und Monitoridentität. `native/src/screen/os/linux_os/x11_keymap.rs` und die Android-Implementierung von `ScreenInputMapper` lagen nicht im freigegebenen Read-Scope. Die vollständige hostseitige Keysym-/Layout-/Deadkeyumsetzung und ihre Verbindung zur Android-IME wurden daher nicht abschließend verifiziert. Der Review behauptet insbesondere keine Behebung des ursprünglich gemeldeten fehlenden Passwortzeichens.
- Die genaue Compose-Abhängigkeitsversion konnte aus der zugewiesenen Dateiliste nicht abgelesen werden; Gradle-Versionsdateien waren nicht freigegeben. Der Android-Besitzerbefund stützt sich auf den offiziellen AndroidView-Vertrag und die aktuelle offizielle Compose-Lebenszyklusbeschreibung.
- Installerartefakte, SHA-256-Feeddateien, signierte Releasebytes und sichtbare GitHub Releases lagen außerhalb dieser statischen Quellenprüfung. B7-05 beschreibt die gelesene Auslieferungslogik, keinen beobachteten Downloadfehler einer veröffentlichten Version.
- Parallel geänderte Dateien bilden keinen eingefrorenen Kandidaten. B7-01 bis B7-06 und B7-R1 wurden nach Korrektur erneut geprüft. Die anschließend angekündigte B8-Änderung am Familien-Rootausschnitt in `host_backend.rs` und `x11_capture.rs` gehört zum separaten B8-Review; diese Schließung nimmt dessen Ergebnis nicht vorweg.

## Gelesen, erstellt und geändert

Gelesen wurden das Scope-Manifest `/tmp/smartexplorer-review-b7-scope.json`, die nachfolgend genannten Funktionsdateien beziehungsweise ihre betroffenen Abschnitte und die oben direkt verlinkten Primärquellen. Die Einträge unter „Integrationssuche“ wurden gezielt auf Einstieg, Weiterleitung und Distribution untersucht; dies ist keine Behauptung einer vollständigen Zeile-für-Zeile-Prüfung jeder großen Rootdatei.

Inhaltliche Code-/Vertragslektüre:

```text
AGENTS.md
docs/ARCHITEKTUR.md
docs/plaene/2026-10-24-batches/batches.md
docs/plaene/2026-10-24-batches/vertraege.md
docs/plaene/2026-10-24-batches/recherche.md
docs/plaene/2026-10-24-batches/umsetzung.md
native/Cargo.toml
native/Cargo.lock
native/install-login-service.ps1
native/install-login-service.sh
native/src/login_entry.rs
native/src/login/mod.rs
native/src/login/core/model.rs
native/src/login/core/authority.rs
native/src/login/core/machine.rs
native/src/login/core/manifest.rs
native/src/login/core/gateway_types.rs
native/src/login/core/gateway_wire.rs
native/src/login/core/viewer_guard.rs
native/src/login/core/wire.rs
native/src/login/os/shared/broker.rs
native/src/login/os/shared/child.rs
native/src/login/os/shared/client.rs
native/src/login/os/shared/gateway.rs
native/src/login/os/shared/gateway_client.rs
native/src/login/os/shared/helper.rs
native/src/login/os/shared/host_status.rs
native/src/login/os/shared/installation.rs
native/src/login/os/shared/provision.rs
native/src/login/os/shared/status.rs
native/src/login/os/shared/target.rs
native/src/login/os/shared/viewer.rs
native/src/login/os/linux_os/host_status.rs
native/src/login/os/linux_os/mod.rs
native/src/login/os/linux_os/protected.rs
native/src/login/os/linux_os/service.rs
native/src/login/os/linux_os/session.rs
native/src/login/os/linux_os/spawn.rs
native/src/login/os/windows/host_status.rs
native/src/login/os/windows/mod.rs
native/src/login/os/windows/pipe.rs
native/src/login/os/windows/protected.rs
native/src/login/os/windows/sas.rs
native/src/login/os/windows/security.rs
native/src/login/os/windows/service.rs
native/src/login/os/windows/session.rs
native/src/login/os/windows/spawn.rs
native/src/login/os/unavailable.rs
native/src/app/core/login_viewer.rs
native/src/app/core/login_viewer_image.rs
native/src/app/core/share_login_ui.rs
native/src/app/core/screen_presented.rs
native/src/app/core/screen_texture_cpu.rs
native/src/screen/core/input_model.rs
native/src/screen/core/pixel_convert.rs
native/src/screen/os/windows/login_source.rs
native/src/screen/os/linux_os/login_source.rs
native/src/screen/os/linux_os/x11_capture.rs
native/src/screen/os/linux_os/x11_input.rs
native/src/screen/os/linux_os/x11_monitors.rs
native/src/daemon/os/shared/ipc_protocol_login.rs
native/src/daemon/os/shared/login_ipc.rs
native/src/daemon/os/shared/login_ipc_client.rs
native/src/daemon/os/shared/login_ipc_registry.rs
native/src/daemon/os/shared/login_ipc_session.rs
native/src/daemon/os/shared/login_shutdown.rs
native/src/cli/login.rs
native/src/cli/mcp/login_tool.rs
native/src/cli/mcp/login_shutdown.rs
native/src/mobile/os/shared/domains/login_view.rs
native/android-bridge/src/login_jni.rs
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginViewScreen.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginInput.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginSession.kt
native/src/share/core/access_constraints.rs
native/src/share/core/access_grant.rs
native/src/share/core/access_grant_runtime.rs
native/src/share/core/login_transport.rs
native/src/share/core/relation_proof.rs
native/src/share/core/node_tcp.rs
native/src/share/core/node_tcp_link.rs
native/src/share/core/tcp_forward.rs
native/src/share/core/tcp_registry.rs
native/src/share/core/tcp_wire.rs
native/src/share/os/shared/tcp_pump.rs
native/src/share/os/shared/tcp_sockets.rs
native/src/daemon/os/shared/ipc_protocol_tcp.rs
native/src/daemon/os/shared/tcp_ipc.rs
native/src/daemon/os/shared/tcp_ipc_client.rs
native/src/rdp/README.md
native/src/rdp/core/admission.rs
native/src/rdp/core/types.rs
native/src/rdp/os/shared/clients.rs
native/src/rdp/os/shared/process.rs
native/src/rdp/os/shared/session.rs
native/src/rdp/os/linux_os/client_trust.rs
native/src/rdp/os/linux_os/clients.rs
native/src/rdp/os/linux_os/gnome.rs
native/src/rdp/os/linux_os/inventory.rs
native/src/rdp/os/linux_os/krdp.rs
native/src/rdp/os/linux_os/xrdp.rs
native/src/rdp/os/windows/clients.rs
native/src/rdp/os/windows/probe.ps1
native/src/rdp/os/windows/probe.rs
native/src/rdp/os/windows/profile.rs
```

Gezielte Integrationssuche und Lektüre der relevanten Fundstellen:

```text
.github/workflows/build.yml
README.md
docs/RELEASING.md
native/build.rs
native/installer.nsi
native/publish-linux-feed-wsl.sh
native/publish-release-local.ps1
native/publish-update.ps1
native/src/lib.rs
native/src/app/mod.rs
native/src/app/screen_modules.rs
native/src/app/core/desktop_surface_app.rs
native/src/app/core/frame_update.rs
native/src/app/core/share_device_page_ui.rs
native/src/app/core/screen_input_map.rs
native/src/cli/mod.rs
native/src/cli/mcp/server.rs
native/src/daemon/mod.rs
native/src/daemon/os/shared/ipc.rs
native/src/mobile/mod.rs
native/src/mobile/os/shared/domains/mod.rs
native/android-bridge/src/lib.rs
native/src/screen/os/mod.rs
native/src/share/mod.rs
android/app/src/main/java/app/smartexplorer/android/core/Core.kt
android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/DeviceSettingsPage.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDialogHost.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareScreen.kt
```

Zusätzlich wurden die tatsächlich verwendeten Windows-Konstanten/Signaturen in diesen gepinnten Registryquellen gelesen:

```text
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/System/Threading/mod.rs
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/System/StationsAndDesktops/mod.rs
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/System/RemoteDesktop/mod.rs
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-sys-0.59.0/src/Windows/Win32/Storage/FileSystem/mod.rs
```

Im Nachreview zusätzlich oder erneut gelesen:

```text
.gitattributes
native/src/screen/core/host_backend.rs
native/src/screen/core/host_session.rs
native/src/screen/core/host_reference.rs
native/src/screen/core/host_pipeline_actor.rs
native/src/screen/core/worker_wire.rs
native/src/screen/os/mod.rs
native/src/screen/os/linux_os/login_source.rs
native/src/screen/os/linux_os/login_geometry.rs
native/src/screen/os/linux_os/x11_capture.rs
native/src/screen/os/linux_os/x11_capture_target.rs
native/src/screen/os/linux_os/x11_input.rs
native/src/screen/os/shared/host_worker_backend.rs
native/src/screen/os/shared/host_worker_client.rs
native/src/screen/os/shared/host_worker_main.rs
native/src/screen/os/windows/login_source.rs
native/src/screen/os/windows/monitors.rs
native/src/screen/os/windows/capture_dxgi.rs
native/src/screen/os/windows/capture_source.rs
native/src/screen/os/windows/input_send.rs
native/src/login/os/shared/helper.rs
native/src/login/os/shared/broker.rs
native/src/login/os/shared/child.rs
native/src/login/os/shared/viewer.rs
native/src/login/core/viewer_guard.rs
native/install-login-service.ps1
native/install-login-service.sh
native/src/login_entry.rs
native/src/login/os/shared/installation.rs
native/src/login/os/shared/gateway.rs
native/src/login/os/linux_os/mod.rs
native/src/login/os/linux_os/service.rs
native/src/login/os/linux_os/readiness.rs
native/src/login/os/windows/service.rs
native/src/share/core/node_tcp.rs
native/src/share/core/node_tcp_target.rs
native/src/share/core/node_tcp_link.rs
native/src/share/core/node_access.rs
native/src/share/core/access_revocation.rs
native/src/share/core/tcp_registry.rs
native/src/share/os/shared/tcp_sockets.rs
native/src/share/os/shared/tcp_pump.rs
native/release-publication.ps1
native/publish-release-local.ps1
native/publish-update.ps1
native/publish-linux-feed-wsl.sh
native/installer.nsi
.github/workflows/build.yml
README.md
docs/RELEASING.md
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginViewScreen.kt
docs/plaene/2026-10-24-batches/review-b7.md
```

Erstellt und im Nachreview aktualisiert: ausschließlich `docs/plaene/2026-10-24-batches/review-b7.md`. Geänderte Produktdateien durch diesen Reviewer: keine. Der Dokumentationskontext wurde mit Status-/HEAD-Abfrage, gezielter Integrationssuche und aktueller Quellenlektüre geprüft. Diese Datei ist ein Reviewbeleg; offene Umsetzungsarbeit wird weiterhin ausschließlich auf dem bestehenden Projektboard geführt.
