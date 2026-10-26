# G1/G2: X11-Familieneingabe

Stand: 2026-10-26. Autorauftrag: `/tmp/smartexplorer-fix-x11-family-scope.json`;
wörtliche Nutzervorgaben: `/tmp/smartexplorer-final-review-user-contract.txt`.
Reviewkandidat: `ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b`.
Releasebasis laut Hauptagent: `a814f263096fe69d3746e840b5f3fad3fd7e88a0`
(`v0.5.184`). Zu Beginn dieser Bearbeitung stand die isolierte Arbeitskopie
auf `a3f10e85`.

## Erstplan aus dem Auftrag

Beauftragtes Ergebnis: Zugehörige X11-Combo-Popups und verschachtelte Menüs
lassen sich im Fensterfamilienviewer mit Maus und Touch bedienen. Fokusbestätigung,
rekursive Eigentümerbindung, geprüfte Prozess- und Fensterlebensdauer, Rechte,
Modalität und Entwaffnung bleiben wirksam. Die Freigabe gilt ausschließlich für
die tatsächlich autorisierte Fensterfamilie.

Der vollständige eigene Fixzyklus umfasst Funktionsauflistung, direkte und
indirekte Verknüpfungskarte, Fixplan mit Regressionserhalt, Umsetzung und
anschließende eigene Anti-Regressionsreview. Ausschließlich die sechs im Scope
genannten Produkt-/Testdateien und dieser Bericht dürfen verändert werden.
Keine lokalen Builds, Compiler, Formatter, Tests, Server, Installationen,
Commits, Pushes oder Graphupdates. Hauptagent besitzt gemeinsame Remote-Suite
und terminalen Release.

Promptwissen: G1 betrifft Combo-Popups; G2 betrifft rekursive Menü-Eigentümer.
Die tatsächlichen Fokuseigenschaften und Lebensdauerverträge sind durch Code
zu klären. Keine Nutzerangabe verhindert die Bestandsaufnahme.

Vorläufige Schritte: (1) Architektur, G1/G2-Belege und betroffene Verträge lesen;
(2) Familien-/Fokus-/Eingabekette vollständig bis zu den zugewiesenen
Verbrauchergrenzen abbilden; (3) EWMH/GTK/Qt-Belege und verbleibende API-Fragen
klären; (4) detaillierten Plan mit konkreten erwarteten Ergebnissen sichern;
(5) innerhalb der Schreibfläche umsetzen; (6) vorbereitete Remote-Fälle
registrierbar machen und eigene statische Anti-Regressionsreview dokumentieren.

Dokumentationskontext vor der Anlage: `git status --short` zeigt parallele
Änderungen außerhalb der eigenen Schreibfläche; `git log -1 --oneline` ergibt
`a3f10e85 test(screen): add isolated relay reference acceptance for P27 [task candidate]`.
Gezielte Suche im Gesamtbericht bestätigt G1/G2. Keine fremde Änderung wird
übernommen oder zurückgesetzt.

## B: Bestand und Funktionsauflistung

G1: `properties::kind` unterscheidet `_NET_WM_WINDOW_TYPE_COMBO` nicht vom
Resttyp. `X11Adapter::observe` bietet dafür trotz Override-redirect eigene
WM-Aktivierung an. `FamilyViewer::focus_identity` fordert diesen eigenen Fokus;
`HostSession::operate` und `X11Adapter::focused_inner` bestätigen ihn nicht.

G2: `X11Adapter::observe` und `FamilyViewer::focus_identity` verfolgen
Eigentümer rekursiv, `X11Adapter::focused_inner` verfolgt nur einen Menübesitzer.
`HostSession::with_input` weist das sichtbare Untermenü danach zurück.

Zusätzlich direkt betroffen sind die Bestätigung eines regulären Eigentümers,
während dessen zugehöriges Popup den X-Eingabefokus hält, und der Übergang
Popup → modaler Dialog → Hauptfenster. Der Dialog muss eine eigene
Aktivierungsgrenze bleiben. Planungstiefe 2: gekoppelte bestehende Komponenten,
kein neues Protokoll und keine neue Betriebssystemfähigkeit.

## Vollständige direkte und indirekte Verknüpfungskarte des Fixwegs

Diese Karte beschreibt die betroffene Funktionskette bis zu ihren bestehenden
Verbrauchergrenzen; unveränderte Dateisystemübertragung, Sync, Backup und
Konfliktentscheidungen werden von diesem Weg nicht aufgerufen.

| Station | Direkte Verbindung und indirekte Wirkung | Erhaltungsbedingung |
|---|---|---|
| `app/core/family_device_ui.rs::device` → `family_picker.rs::{panel,open_entry,resolve_confirmed}` | Konkrete Direct-/Room-Beziehung, gepinnter Listeneintrag oder bestätigter Appstart → Source und Familiennonce → Familienfenster | Geräte-/App-/Generationbezug, vorhandene Auswahl und Trennen bleiben erhalten. |
| `app/core/family_viewer.rs::{run,update,menu}` → `family_viewer_session.rs::{input,operate,disarm}` | Bestehender ViewWidget liefert Eingaben; Menü/Diagnose/Fokusverlust lösen Freigaben und Entwaffnung aus. | Keine Änderung am Renderer, Fenster-X/Close, Resize, lokalem Menü oder normalen Vollbildviewer. |
| `app/core/family_viewer_events.rs::receive` | Familien-Snapshot/Antwort → gemeinsames Gate; zurückgehaltene Eingaben verlassen es nur bei offener tatsächlicher Bildbestätigung. | Request-ID, Layoutrevision, PRESENTED-Grenze und Fehlerentwaffnung bleiben erforderlich. |
| Android `FamilyScreen.kt` → `FamilySession.kt` → `ScreenSession.kt` | Bestehende Bildschirmdarstellung und serialisierte Eingabewarteschlange; tatsächliche Rendererbestätigung plus Familienlayout vor Eingabe | Kein neuer Androidbedienweg; Steuern, lokales Menü, Zurück und Disconnect bleiben unverändert. |
| `mobile/.../screen_view.rs::input` → `screen_family.rs::{route_input,operation,event}` und `screen_view_run.rs::{send,on_event,release_pressed}` | Derselbe `FamilyViewer` für Touch und Desktopmaus; Bridge lässt kein rohes INPUT einer Familiensitzung zu. | `control_allowed`, `family_presented`, Familiennonce und Layout-/GOP-Bindung bleiben erhalten. |
| `screen/core/family_viewer.rs::{receive,input,focus_identity,operation,result,disarm}` | Canvas-Hitprüfung → Zielmitglied → Fokusidentität → korrelierte Hostantwort → ursprüngliche Member-Eingabe | Zielidentität und Memberkoordinaten bleiben vom Fokusbesitzer getrennt; kein Input vor FocusConfirmed. |
| `screen/core/family_protocol.rs::{pointer_target,focused_target}` und `remote_window/core/{model,wire}.rs` | Z-Reihenfolge, Geometrie, Sichtbarkeit und Tooltipauslassung; vollständige validierte Eigentümerbäume und Generationen | Kein neues Wirefeld/Enum, kein unzugeordnetes Fenster, kein Desktopziel im Zwischenraum. |
| `screen/core/host_session.rs` → `host_family.rs::{family_operation,family_input,publish_family}` | Authentifizierter Steuerhalter, aktueller Listenscope, Snapshotrevision und gerendertes Bild → originaler Worker | Widerruf, Verbindungsende, Fokus-/Layoutwechsel und gehaltene Eingaben bleiben über bestehende Releases behandelt. |
| `screen/os/shared/family_backend.rs::WorkerFamily::{operation,input,snapshot,release}` | Ursprünglicher Pin → `HostSession`; erneute Prüfung vor Injektor; fehlgeschlagene Teilinjektion hält kompensierende Ups wiederholbar | Kein Rebind einer alten Fernidentität, kein Kill, keine verlorene Eingabefreigabe. |
| `remote_window/core/host.rs::{open_bound_as,refresh,target,operate,with_input,boundary,boundary_current,disconnect}` | Rechte + Rootlebensdauer + rekursive Prozess-/Owner-Zuordnung → gebundene Generationen → `WindowAdapter` | Eigene Aktivierungsbestätigung, aktuelle Revision, Modalität und Entwaffnung bleiben zwingend. |
| `remote_window/os/x11.rs::{adopt,bind,observe,validate_inner,focused_inner,activate_if,request,disconnect}` | Private X11-Marker, XRes-Prozessidentität, Familienmitgliedschaft, EWMH und echter Inputfokus → erlaubte OS-Anforderung | Rekursion liest ausschließlich bereits gebundene Fenster; sie bindet kein fremdes Fenster nachträglich. |
| Neues `x11/focus.rs::{can_activate,owner_focus,anchor,confirmed}` ← `x11.rs::{focused_inner,focus_node,validated_process,observe}` | Typisierte Fokusfakten aus vorhandenen Markern → nächste reguläre Aktivierungsgrenze → konkreter tatsächlicher X-Fokus; jeder Ownerhop prüft gleiche Prozesssignatur und Lifetime. | Kein gleichprozessiger Ersatz ohne Ownerbeleg; reguläre Dialoge bleiben eigene Anker; Livefokus auf gehörigem Popup/Control ersetzt keine WM-Aktivierungsbestätigung. |
| `x11/properties.rs::{process,kind,words,bytes,geometry}` | Tatsächlicher XRes-Client, `/proc`-Start/UID/Sitzung, vollständige Propertywerte → Identitäts- und Fenstertatsachen | Fremde UID, falsche PID, tote Prozesse, ersetzte Marker und übergroße Properties werden weiterhin abgewiesen. |
| `remote_window/os/shared/{family_source,family_capture_plan,family_capture_members}.rs` | Unveränderte CaptureBoundary erfasst jedes sichtbare Mitglied separat und prüft die Grenze vor Veröffentlichung erneut. | Popup bleibt Member, nicht Ausschnitt vom Desktop; Ressourcen- und Referenzbesitz bleibt unverändert. |
| `screen/os/shared/host_worker_{run,prepare,main}.rs` → `screen/os/mod.rs::{platform_backend,note_input_target}` → `screen/os/linux_os/host_backend.rs::{inject_family,inject_x11_family}` | Der Worker besitzt den direkten LinuxBackend; `WorkerFamily::input` setzt das konkrete Memberziel. Der ergänzte X11-Arm übergibt Member-XID und geprüfte Area an `inject_bound`. Der äußere WorkerBackend ist nicht der innere Familieninjektor. | Kein Wechsel zum jüngsten fremden Kontrollworker; Eingaben und Geometrie bleiben beim originalen Prozess und Memberziel. |
| `screen/os/linux_os/x11_input.rs::{inject_bound,area_of,send,release}` | Maus und Touch-ID 0 gehen über den bestehenden XTest-Injektor. `inject_bound` vergleicht aktuelle Geometrie mit der unveränderten geprüften Area; Key/SAS- und Releasegrenzen bleiben bestehen. Allgemeines `inject` bleibt der unveränderte Weg für Quellen ohne Familiengate. | Keine globale Eingabeumleitung; sessionweites XTest bleibt ausdrücklich ohne Appisolation. |
| Windows `remote_window/os/windows.rs` und `windows/facts.rs`; `core/selection.rs` | Unveränderte Windowsmenü-/Dialogprüfung; Wayland-/Androidhost behalten ausschließlich ihre tatsächliche Auswahl. | Fokusfähige Windows-Popups verwenden weiter eigene Identität; keine erweiterten Portal- oder Projektionsrechte. |

Die Graphabfrage `X11Adapter focused_inner FamilyViewer focus_identity input
HostSession operate` lieferte 259 Knoten und die vorstehenden Domänengrenzen.
Die zweite Abfrage zu `inject_family inject_bound` ergänzte die Injektorgrenze.
Graphify meldet Skill 0.9.11/Paket 0.9.17. Kein Graph wurde verändert; der Graph
ersetzt keine Quellprüfung. Die letzte Injektorübergabe ist nach den vom
Hauptagenten einzeln freigegebenen Lesescoperweiterungen vollständig belegt.

## C: Quellen, Alternativen und Entscheidung

Herstellerquellen erneut geprüft am 2026-10-26:

- [EWMH 1.5, §§2.11 und 5.6](https://specifications.freedesktop.org/wm/latest-single/):
  Override-redirect hat keine reguläre WM-Verwaltung; Combo bezeichnet die
  aufgeklappte Auswahlliste. Folgerung: Ownerfokus für diese bereits zugeordnete
  Fläche, keine vorgetäuschte eigene WM-Aktivierung.
- [GTK 3.24.43, `gtk_combo_box_set_popup_widget` und `gtk_combo_box_popup_for_device`](https://raw.githubusercontent.com/GNOME/gtk/3.24.43/gtk/gtkcombobox.c):
  tatsächliche Combo-Popupfläche mit COMBO-Hinweis und transientem Toplevelbezug.
- [Qt 6.8.0, `QMenuPrivate::transientParentWindow`](https://raw.githubusercontent.com/qt/qtbase/v6.8.0/src/widgets/widgets/qmenu.cpp)
  und [`QXcbWindow::updateWmTransientFor`](https://raw.githubusercontent.com/qt/qtbase/v6.8.0/src/plugins/platforms/xcb/qxcbwindow.cpp):
  rekursive Menüzuordnung ist ein wirklicher Erzeugungsweg.
- [X.Org, Controlling Input Focus](https://www.x.org/releases/X11R7.6/doc/libX11/specs/libX11/libX11.html):
  GetInputFocus unterscheidet Fenster, None und PointerRoot. Nur ein konkretes
  geprüftes Fenster kann hier die Eingabefreigabe bestätigen.
- [x11rb 0.13.2, generiertes xproto-API](https://raw.githubusercontent.com/psychon/x11rb/v0.13.2/x11rb/src/protocol/xproto.rs):
  vorhandene GetProperty-/GetWindowAttributes-/QueryTree-/GetInputFocus-
  Rückgaben und kurze GrabServer/UngrabServer-Transaktion. Version belegt durch
  `Cargo.lock:9241`; docs.rs war beim Abruf nicht erreichbar, der versionierte
  Herstellerquelltext war erreichbar.

Gewählt: Combo bleibt Wiretyp Popup; Override-redirect-Popups bieten keine
eigene Aktivierung an. Das gemeinsame Gate folgt bei nichtaktivierbaren Popups
und Menüs der expliziten Ownerkette bis zum ersten sichtbaren, interaktiven,
aktivierbaren Eigentümer. Die X11-Prüfung verwendet denselben nächsten
Aktivierungsanker, prüft jede Lifetime und Prozesssignatur frisch und bestätigt
zusätzlich den tatsächlichen Inputfokus innerhalb dieser Popupkette. Ein
regulärer Dialog wird niemals in Richtung Hauptfenster übersprungen.

Verworfen: eigener Fokus auf Override-redirect (kein WM-Vertrag); Combo als
Menu umlabeln (verliert bestehende Typbedeutung); beliebiges gleichprozessiges
Fenster akzeptieren (fehlender Ownerbeleg); nur den direkten Owner prüfen
(G2 bleibt); alle Popups unabhängig von `operations.focus` umleiten (regressiert
aktivierbare Windows-/X11-Popups); neues Wireenum (unnötige Peeränderung).
Keine neue Abhängigkeit oder Installation nötig.

## D: endgültiger Fixplan mit Regressionserhalt

Vor Produktänderungen sind die fünf Herstellergrenzen oben geprüft. Zweite
Lückenrecherche: Der tatsächliche X-Fokus kann in einem Popup/Control liegen,
obwohl der WM den regulären Eigentümer aktiv meldet. Daher muss auch dessen
explizite Fokusbestätigung dieselbe frisch verifizierte Popupkette zulassen.
None/PointerRoot, fremde Eigentümer, neue Fenstergenerationen und nicht sichtbare
Mitglieder bleiben gesperrt. Eine kurze Servertransaktion hält den gelesenen
Eigentümer-/Fokusstand konsistent; sie behauptet keine Isolation während XTest.

1. **X11-Fokussierung** (`x11.rs`, neues `x11/focus.rs`, `properties.rs`):
   Combo ausdrücklich erkennen, Override-redirect-Fokusfähigkeit korrekt melden,
   rekursiven nächsten Aktivierungsanker und konkrete Fokusabstammung prüfen.
   Jeder Kettenschritt verlangt bestehenden Marker, lebende gleiche Prozess-/
   Sitzungs-/UID-Identität, Sichtbarkeit und fehlende Minimierung. Erwartung:
   H → M → S und H → Combo bestätigen H; ein dazwischenliegender Dialog
   bestätigt ausschließlich sich selbst; ungültige Ketten erzeugen keinen Input.
2. **Gemeinsamer Viewergate** (`screen/core/family_viewer.rs`):
   nichtaktivierbares Popup verwendet Owner-Fokus, behält seine eigene Ziel-
   und Koordinatenidentität. Ein nichtaktivierbarer regulärer Dialog wird nicht
   übersprungen. Erwartung: Maus/Touch warten auf korrelierte FocusConfirmed-
   Antwort; vorhandene fokusfähige Popups und reguläre Dialoge behalten Eigenfokus.
3. **Vorbereitete Remote-Fälle** (`x11/batches_popup_focus_task_tests.rs` plus
   Registrierung in `x11.rs`): kontrollierte X11-Fakten über den produktiven
   Fokusentscheider, echten HostSession und gemeinsamen FamilyViewer verbinden.
   Positivfälle: Combo, direktes Menü, verschachtelte Menüs und Popupfokuscontrol.
   Negativfälle: falscher Prozess, ungebundener/recycelter Owner, Zyklen,
   versteckt/minimiert, modaler Dialog, fremder Fokus, Rechte-/Layoutwechsel,
   falsche Antwort-ID und Entwaffnung. Keine Ausführung durch diesen Autor.
4. **Eigene anschließende Anti-Regressionsreview**: alle Tabellenstationen gegen
   finalen Diff erneut verfolgen; unveränderte Rechte, Marker, Snapshot-/Bildbezug,
   Geometrie, gehaltene Eingaben, Close/Resize und Disconnect belegen. Ergebnis
   und konkrete Laufzeitgrenzen getrennt dokumentieren. Main registriert die
   Fallnamen in seiner einen späteren Remote-Suite und aktualisiert den Rootgraph.

## Umsetzung

**Ergänzter enger Plan vor der Adapteränderung (2026-10-26):** Main hat
`screen/os/linux_os/host_backend.rs` nach dem belegten Anschlussbefund zusätzlich
schreibend freigegeben. Die vorhandene API
`x11_input::inject_bound(&SourceId, Area, &[InputEvent])` vergleicht die aktuelle
X11-Geometrie mit der übergebenen Area und injiziert nur bei Gleichheit.
Der neue `LinuxBackend::inject_family`-Arm übergibt deshalb genau das gebundene
Member-XID und dessen bereits geprüfte `CaptureGeometry`. Null-/überlaufende
Geometrie und fehlende Inputfähigkeit werden vor der Injektion abgewiesen.
Der allgemeine `inject`-Pfad und Wayland-/None-Verhalten bleiben erhalten.
Ein schmaler interner Aufrufadapter erlaubt die vorbereitete Prüfung von
Source, Area, unveränderten Events und Abweisung vor dem Injektor ohne X-Server.
Erwartung: Eine am abschließenden Abgleich festgestellte Bewegung/Resize wird von `inject_bound` gesperrt;
kein Live-Neumappen der schon bestätigten Memberkoordinaten. Keine Änderung
an `x11_input.rs` ist nötig. Diese Ergänzung hat denselben Autorzyklus wie G1/G2.

`properties.rs` erkennt COMBO ausdrücklich, auch mit NORMAL als Fallbacktyp;
der bestehende Wiretyp Popup bleibt erhalten. `focus::can_activate` verhindert
WM-Aktivierungsangebote für Override-redirect. Der neue enge `focus.rs`-
Entscheider trennt Inputziel und Aktivierungsanker, erkennt Ownerzyklen, prüft
gleiche Prozesssignaturen über jede Eigentümerebene und endet am ersten
regulären Fenster. `focus_node` liest ausschließlich bestehende Marker und
validiert deren tatsächlichen Prozess und Lifetime erneut; kein Fokuslauf
bindet ein neues XID.

`focused_inner` liest aktive WM-Identität, tatsächlichen Inputfokus, Owner,
Sichtbarkeit und Marker in einer kurzen GrabServer/UngrabServer-Transaktion.
Der Server wird auch bei Fehlerpfaden wieder freigegeben. Ein tatsächliches
Fokuscontrol in einem gehörigen Popup ist zulässig, wenn seine vollständig
geprüfte Popupkette am selben regulären Anker endet. Ein fremder Dialog,
unzugeordnetes gleichprozessiges Popup, fehlender Owner, falscher Prozess,
veralteter Marker, verstecktes/minimiertes Mitglied, None und PointerRoot
bestätigen keinen Fokus. Eine bereits tatsächlich bestätigte reguläre
Eigentümeraktivierung sendet keinen erneuten unnötigen WM-Aktivierungsauftrag.

`FamilyViewer::focus_identity` verwendet für Menü oder nichtaktivierbares Popup
die explizite Eigentümerkette. Fokusfähige Popups und Dialoge behalten ihre
eigene Fokusidentität. Sichtbarkeit, Minimierung und Inputfähigkeit gelten für
jeden Schritt; ein nichtaktivierbarer regulärer Dialog stoppt die Kette.
`PendingFocus.target`, normalisierte Memberkoordinaten und eigentliche
FamilyInput-Zielidentität werden nicht zum Eigentümer umgeschrieben.

## Eigene anschließende Anti-Regressionsreview

Gegenstand war der fertige eigene Produktdiff einschließlich der unveränderten
Tabellenstationen. Prüffragen: Erreicht erlaubte Maus-/Touchbedienung das
richtige Mitglied; bleibt Fokusnachweis konkret; werden Owner, Prozess,
Lifetime und Modalität geprüft; bleiben Rechte, Revision, tatsächliche
Bildbestätigung und Entwaffnung wirksam; bleiben vorhandene Popup-/Dialog-,
Close-/Resize-/Disconnect- und Capturepfade erhalten. Befundform ist jeweils
Quellbeleg, Auswirkung und offene Grenze, kein pauschales Laufzeiturteil.

| Prüffrage | Eigener statischer Befund am fertigen Diff |
|---|---|
| G1, Maus und Touch | Der Combo-Typ bleibt Popup, `operations.focus` ist bei Override-redirect false. Pointer und Touch wählen über denselben `pointer_target` das Popup, bestätigen den nächsten regulären Owner und senden dessen ursprüngliches Memberziel. Kein rohes INPUT wurde ergänzt. |
| G2, rekursives Menü | S → M → H wird in Aufnahme, Viewergate und X11-Fokusentscheidung durchgehend verfolgt. M wird nicht als eigene erforderliche WM-Aktivierung behandelt. Jeder Hop wird erneut gebunden validiert und auf dieselbe Prozesssignatur geprüft. |
| Tatsächlicher Fokus | WM-Aktivierung allein genügt weiterhin nicht. Ein konkreter X-Fokus oder dessen physisches Elterncontrol muss zum bestätigten Anker beziehungsweise zu dessen geprüfter Popupkette gehören. Ein gleichprozessiges, unzugeordnetes Popup liefert keinen Beleg. |
| Modalität | S → Dialog D → H endet bei D. Aktives H reicht für D/S nicht; ein tatsächlich auf D gesetzter WM-Fokus mit Input auf S ist zulässig. Ein regulärer Dialog mit fehlender Fokus-/Inputfähigkeit wird vom gemeinsamen Gate nicht übersprungen. Windows `IsWindowEnabled` und dessen native Fokusprüfung bleiben unverändert. |
| Lebensdauer und Mitgliedschaft | `bind`, `adopt`, `transfer_pin`, XRes-PID-/UID-/Start-/Sitzungsprüfung, 16-Byte-Marker und Hostgenerationen bleiben erhalten. Neue Fokustraversierung verwendet ausschließlich bestehende Marker und bindet kein fremdes Fenster. Marker-/Prozesswechsel verweigern die Bestätigung. |
| Rechte und Scope | `HostSession::{admitted,refresh,operate,with_input}`, Daemon-/Workerrechte und mobile `control_allowed` bleiben unverändert. Rekursive Focusowner müssen Inputfähigkeit besitzen; ein wiederhergestelltes Recht rearmt den Host nicht automatisch. |
| Korrelation und Entwaffnung | `PendingFocus.request_id`, FocusConfirmed, Layoutrevision und Snapshotrevision bleiben unverändert erforderlich. Layoutwechsel, Controlverlust und Inputablehnung verwerfen/freigeben weiter über die bestehenden Viewer-/Worker-/Hostpfade. Nur der explizite korrelierte Fokusauftrag armiert. |
| Darstellung und Besitz | Separate Popupflächen, CaptureBoundary, Referenzgeneration, Grantbesitz und Captureprüfung vor Ausgabe sind unverändert. Desktop- und mobile Renderer behalten ihre tatsächliche PRESENTED-/Layoutgrenze. Der Fix besitzt keinen neuen Bild-/Codec-/Serverprozess. |
| Abschließende Eingabegeometrie | `LinuxBackend::inject_family` erreicht für X11 ausschließlich `inject_x11_family → x11_input::inject_bound`. Der Adapter reicht exaktes Member-XID, dessen geprüfte Area und unveränderte Events durch. Nichtinputfähige Member, Nullgrößen, nicht darstellbare i32-Spannen und ungültige XIDs erreichen den Injektor nicht. Fehler werden an `WorkerFamily::input` weitergereicht und lösen dort die bestehende wiederholbare Eingabefreigabe aus. Es gibt keinen Rückfall auf ungeprüfte Familieninjektion. |
| Vorhandene Fensteraktionen | Fokusfähige Dialoge/Popups bleiben selbständig aktivierbar; direktes Menü behält Ownerfokus; Tooltip bleibt sichtbar, aber kein Eingabeziel. `request` sendet weiterhin nur graceful Close und erlaubtes Resize; Disconnect löscht ausschließlich eigene Marker. Kein Appkill und kein Ersatz für reguläre Speicherdialoge. |
| API, Rustgrenzen und Abschluss | Neue Logik enthält keine OS-Imports in `core/`, keine neue Abhängigkeit, keinen Produktionspanic und keine neue Persistenz. UngrabServer wird vor Weitergabe jedes Result-Fehlers ausgeführt. Alle sechs Rustdateien liegen unter 500 Zeilen und 50 KiB. `git diff --check` auf den geänderten Bestandsdateien war ohne Ausgabe. Das ist ausschließlich statisches Feedback. |

Eigene Korrektur während dieser Review: Der Lifetime-Fixture nach Rechte-/
Fokuswechsel verwendet jetzt die aktuelle Hostrevision, damit dessen Abweisung
tatsächlich den ungültigen Popupmarker und nicht zufällig eine alte Revision
prüft. Die Maus-/Touchfixtures vergleichen außerdem sämtliche nach der
Ownerbestätigung gesendeten Phasen, Knöpfe und normalisierten Memberkoordinaten.

**Eigener Anschlussbefund im selben Autorzyklus behoben:**
Vor der Ergänzung verwendete LinuxBackend den Traitdefault
`inject_family → inject`; dadurch konnte `area_of` eine zwischen Hostgate und
Injektion geänderte Geometrie still übernehmen. Main gab die schmale
`host_backend.rs`-Korrektur ausdrücklich frei. Der ergänzte Adapter verwendet
jetzt das vorhandene `inject_bound` mit der bereits geprüften Membergeometrie.
Die anschließende eigene Review hat den Weg vom Worker bis `area_of`/`send`
erneut gelesen: Gleichheit ist Bedingung vor Injektion; bei Abweichung folgt
der bestehende Block-/Releasepfad. `inject`, Wayland-Portal, None-Verhalten,
SAS-Filter, Text, Touch-ID 0 und positionslose kompensierende Ups sind
unverändert. `x11_input.rs` wurde nicht geändert. XTest besitzt weiterhin keine
atomare Fensterisolation zwischen letzter OS-Prüfung und Ereigniszustellung;
der Fix behauptet diese auch für die gebundene Geometrie nicht.

**Nachweisgrenzen:** Keine Kompilierung, kein Formatter, kein lokaler Test,
kein X-Server, keine tatsächliche XRes-/XTest-/GTK-/Qt-/WM-Ausführung und kein
Androidgerät wurden gestartet. System-Python besitzt kein `tree_sitter` und
kein `tree_sitter_rust`; der Autor hat deshalb keinen eigenen AST-Parserlauf
ausgeführt und kein Paket installiert. Main meldete anschließend für alle
sechs eigenen Rustdateien einen statischen Parse mit dem vorhandenen
Graphify-Python ohne ERROR-/missing-Knoten und eine unauffällige Diff-/
Größenprüfung. Das ist Mains Integrationsbeleg, keine eigene Typprüfung oder
Laufzeitprüfung. Die kontrollierten Fixtures liefern keine reale
Fenstermanagerabnahme und keine Appisolation. Typprüfung, Ausführung der
registrierten Fälle und terminaler Release liegen beim Hauptagenten.

## Vorbereitete Fälle und Registrierung durch Main

Die Modulregistrierungen sind `remote_window/os/x11.rs::batches_popup_focus_task_tests`
und das Inline-Modul `screen/os/linux_os/host_backend.rs::batches_family_geometry_task_tests`.
Die folgenden Fälle sind in diesen beiden Modulen vorbereitet und wurden nicht ausgeführt:

| Genauer Fallname | Konkretes Orakel |
|---|---|
| `batches_family_x11_combo_popup_keeps_identity_after_owner_focus_ack` | Maus und Touch-ID 0: keine Vorabeingabe; Fokusauftrag H; nach echter HostSession-Bestätigung unveränderte Popupidentität, Memberkoordinaten und Ereignisphasen. X-Fokus auf H, Popup oder Popupcontrol bleibt zugeordnet. |
| `batches_family_x11_nested_menus_use_recursive_owner_focus` | S → M → H; Inputfokus auf H/M/S/Control bestätigt ausschließlich H und injiziert auf S. |
| `batches_family_x11_modal_dialog_is_the_nearest_activation_boundary` | Dialog D statt H wird angefordert; H kann D nicht bestätigen; D mit zugehörigem Popupfokus funktioniert; deaktivierter Dialog wird nicht übersprungen. |
| `batches_family_x11_focus_rejects_foreign_unbound_recycled_owners_and_cycles` | Fremde Prozesssignatur, ungebundener/ungültiger/recycelter Marker, Ownerzyklus, fehlender Owner, versteckt/minimiert, gleichprozessiges unzugeordnetes Popup, fremder Dialog, None/PointerRoot/fremder Fokus und falsches aktives Fenster bestätigen nicht. |
| `batches_family_x11_host_preserves_disarming_permissions_and_lifetimes` | Tatsächlicher Fokusverlust entwaffnet; Fokus-/Rechterückkehr rearmt nicht; EWMH-Fokuszyklus verlangt Neufreigabe; aktuelle Revision umgeht keinen ungültigen Popup-/Rootmarker; Viewwiderruf sperrt. |
| `batches_family_x11_viewer_discards_uncorrelated_stale_layout_and_disarmed_input` | Falsche Antwort-ID, neuer Layoutstand und explizites Disarm senden keine zurückgehaltenen Eingaben. |
| `batches_family_x11_regular_windows_menus_tooltips_close_resize_and_disconnect_survive` | Regulärer Dialog und fokusfähiges Popup verwenden Eigenfokus; direktes Menü verwendet Owner; Tooltip wird nicht Eingabeziel; Close erhält Speicherdialog, Resize bleibt Requested, Disconnect fordert kein zusätzliches Close an. |
| `batches_family_x11_combo_atom_and_activation_capabilities_are_explicit` | COMBO allein/mit NORMAL bleibt Popup; Tooltip-/Dialog-/Main-Klassifikation bleibt; Override-redirect hat keine eigene WM-Fokusoperation. |
| `batches_family_x11_input_retains_admitted_member_geometry_until_inject_bound` | Der produktive Aufrufadapter reicht Source-ID, negative globale Position, konkrete Größe und unveränderte Maus-/Touchevents an den gebundenen Injektor; dessen Ablehnung wird zurückgereicht, ohne allgemeinen Inputfallback. |
| `batches_family_x11_invalid_member_geometry_never_reaches_the_injector` | Nichtinputfähiger Member, Nullbreite, gültige Geometrie mit nicht darstellbarer i32-Spanne und unzulässige XID rufen keinen Injektor auf. |

Main übernimmt laut Integrationsmeldung die vollständigen Namen als erforderliche Linuxfälle im
gemeinsamen `native/review-task-manifest.json` zur G1/G2-Zuordnung. Das vorhandene
Präfix `batches_family_` erfasst das Modul bereits; die explizite Pflichtliste
verhindert ein still fehlendes Modul. Keine zweite Suite und kein eigener
Startbefehl wurden angelegt. Der gemeinsame plattformneutrale Viewergate wird
in diesen Linuxfällen geprüft; sie ersetzen keine native Windows-/Android-
Fokus- oder Touchlaufzeit. Main besitzt ferner Rootgraphrefresh, kohärenten
Commit/Push und den einen terminalen Remote-Release.

## Gelesene und geänderte Dateien

Gelesen wurden die nachstehenden Dateien vollständig oder für den ausdrücklich
genannten Vertragsabschnitt; eine Suchtrefferliste gilt nicht als Volllektüre:

- Vorgaben/Belege: `/root/.codex/AGENTS.md`, Root-`AGENTS.md`, Scope-JSON,
  Nutzervertrag, Graphify-SKILL.md; `docs/ARCHITEKTUR.md` (Einstiege und
  Domänengrenzen), `docs/oberflaechen/INDEX.md`, `bildschirm.md` (Eingabe,
  Darstellung und Gesamtkandidat), `batches.md` (P4), `vertraege.md` (V6),
  `regressionsreview.md` (G1/G2-Zuordnung), Original-`review-gesamt.md` (G1/G2),
  Original-`review-regression-sol.md` (Auftrags-/Vergleichsgrenzen).
- Native Familienkette: `remote_window/{mod.rs,core/mod.rs,core/model.rs,
  core/wire.rs,core/selection.rs,core/host.rs,core/batches_family_task_tests.rs,
  os/mod.rs,os/x11.rs,os/x11/properties.rs,os/windows.rs,os/windows/facts.rs}`;
  `os/shared/family_capture_plan.rs` sowie `family_source.rs` (Capture und
  Boundaryprüfung), `family_capture_members.rs` (Aktivierung/Komposition);
  `INTEGRATION.md` (nur Familien-/Viewerverträge, historischer Kontext).
- Gemeinsame Screenkette: `screen/core/{family_viewer.rs,family_protocol.rs,
  host_family.rs,host_backend.rs,host_session.rs}` (Familydispatch und
  PRESENTED-Grenze), `screen/os/{mod.rs,shared/family_backend.rs,
  shared/host_worker_backend.rs,shared/host_worker_main.rs,
  shared/host_worker_prepare.rs,shared/host_worker_run.rs,
  linux_os/host_backend.rs,linux_os/x11_input.rs,linux_os/x11_input_tests.rs,
  linux_os/batches_capture_x11_task_tests.rs}`.
- Desktop: `app/core/{family_device_ui.rs,family_picker.rs,family_viewer.rs,
  family_viewer_session.rs,family_viewer_events.rs}`.
- Mobile: `mobile/os/shared/domains/{screen_family.rs,screen_view.rs,
  screen_view_run.rs}` (Familienrouting, Gate und tatsächliche Bildbindung);
  Android `ui/screen/{FamilySession.kt,FamilyScreen.kt,ScreenSession.kt}`
  (Serialisierung, Steuer-/Darstellungsgrenze und Disconnect).
- `native/Cargo.toml` und `Cargo.lock` (x11rb-Konfiguration/0.13.2),
  `native/review-task-manifest.json` (Familienpräfix/-registrierung),
  `graphify-out/graph.json` (gezielte Abfragen), final alle eigenen Rustdateien.

Geändert: `native/src/remote_window/os/x11.rs`,
`native/src/remote_window/os/x11/properties.rs`,
`native/src/screen/core/family_viewer.rs`,
`native/src/screen/os/linux_os/host_backend.rs`.
Neu: `native/src/remote_window/os/x11/focus.rs`,
`native/src/remote_window/os/x11/batches_popup_focus_task_tests.rs`,
dieser Bericht. Keine fremde Datei geändert; keine Version oder Abhängigkeit
angepasst. Die Versionierung gehört zur einen terminalen Transaktion von Main.

Entscheidungen: bestehende Popup-/Wirebedeutung erhalten; nächste reguläre
Aktivierungsgrenze statt pauschalem Rootfokus; ausschließlich frisch validierte
Marker und gleiche Prozesssignaturen; keine behauptete XTestisolation.
G1/G2 sind quellseitig umgesetzt und eigenständig statisch geprüft. Gemeinsamer
Remotenachweis, kanonische Oberflächendokumentation, Graphrefresh und Release
sind noch nicht durch diesen Autor erfolgt und verbleiben bei Main. Main hat
die Fallregistrierung und den Dokumentationsanschluss in `fernzugriff.md`
übernommen. Innerhalb der freigegebenen Codefläche bleibt keine offene
Umsetzungsabhängigkeit; es wird kein laufender oder erfolgreicher Remotelauf
behauptet. Der anschließende Remote-Größenbefund wird unten im selben
Autorzyklus weitergeführt.

## Fortsetzung: Modulgrößen nach tatsächlicher Remoteformatierung

Nachrichtenquelle: erneuter eigener Autorauftrag vom 2026-10-26;
unveränderter Nutzervertrag und aktualisierte Scope-JSON. Remote-Vorbereitung
`40332971455` beanstandete vor jeder Kompilierung die tatsächlichen formatierten
Modulgrößen. Die vom Main übernommenen gebundenen Patches stehen auf
`10988167`. Laut Scope sind sämtliche Remotejobs beendet; diese Fortsetzung
startet keinen Build, Test, Formatter oder Hostprozess.

### A: Ergänzter Erstplan

Ergebnis: Die fünf zugewiesenen Verantwortlichkeiten und jede neue
Extraktionsdatei bleiben in normaler Rustformatierung unter 500 Zeilen und
50 KiB. Öffentliche Einstiege, Host-/Viewer-Familienlogik, Fokus- und
Geometriebindung, Memberidentität, Input-/Pausenpfade und alle bestehenden
Fallnamen bleiben erhalten. Keine künstliche Verdichtung und keine
Gateausnahme. Gitmutation, Manifest, Rootgraph und derselbe Remote-Suite-
Fixloop verbleiben bei Main.

Promptwissen: Bereits formatierte Kandidatenbasis, fünf explizite Zielmodule,
genehmigte Extraktionspfade. Durch Bestandslektüre zu klären: genaue
zusammenhängende Methodenverantwortungen, Sichtbarkeit über Teilmodule,
Aufruferpfade und nötige Modulregistrierung. Kein nur vom Nutzer lieferbarer
Wert fehlt. Vorläufige Tiefe 2: fachliche Extraktion bestehender
Verantwortlichkeiten ohne neuen Funktions-/Wire-/Persistenzvertrag.

Vorgesehener Ablauf: Remotegrößen und aktuelle Quellen lesen; Verantwortung
und vorhandene Verbraucher abgleichen; Sichtbarkeits-/Dateipfade gegen die
Rustreferenz klären; konkrete fachliche Teilung und Erhaltungsorakel sichern;
unveränderte Funktionskörper verschieben; anschließend eigene statische
Anti-Regressionsreview der vollständigen bestehenden Kette und neuen
Modulgrenzen dokumentieren.

### B: Erster Bestandsbefund

Dokumentationskontext vor dieser Ergänzung: `git status --short` für die
zugewiesenen Dateien ohne Ausgabe; `git log -1 --oneline` ergibt
`10988167 chore: apply candidate-bound remote dependency and format corrections [task candidate]`.
Gezielte Symbollektüre und `wc -l -c` bestätigen:

| Datei | Formatierte Zeilen | Bytes |
|---|---:|---:|
| `app/core/family_viewer.rs` | 563 | 22021 |
| `remote_window/core/host.rs` | 641 | 23517 |
| `remote_window/os/x11.rs` | 750 | 27758 |
| `remote_window/os/x11/batches_popup_focus_task_tests.rs` | 734 | 23789 |
| `screen/os/linux_os/host_backend.rs` | 553 | 20839 |

Die frühere Zeilenbilanz dieses Berichts beschrieb die damalige kompakte
Quellfassung; sie ist kein Größenbeleg nach der tatsächlichen
Remoteformatierung. Sie wird für diesen Fortsetzungsstand durch die Tabelle
oben und die spätere Abschlussbilanz ersetzt. Der Remotejob lieferte keinen
Compiler- oder Laufzeitbefund.

### C: Strukturentscheidung und zweite Lückenklärung

Alle fünf Zieldateien wurden vollständig im formatierten Stand gelesen.
Die Architektur und der vorhandene Graph bestätigen dieselben externen
Einstiege; die erneute Graphabfrage nach `FamilyViewerApp HostSession X11Adapter
LinuxBackend` liefert 325 Knoten. Der Graph wurde nicht verändert. Die
Quelllektüre entscheidet über die tatsächlichen neuen Teilgrenzen.

Gewählt werden direkte interne Teilmodule mit unveränderten Funktionskörpern.
Der Zustand bleibt jeweils beim bisherigen `FamilyViewerApp`, `HostSession`,
`X11Adapter` beziehungsweise `LinuxBackend`. Die bestehenden Traitimpls und
Re-Exports bleiben die Verbrauchergrenze. Bei X11 delegieren vier
Traitmethoden in enge Inherent-Methoden; der Trait wird nicht mehrfach für
denselben Typ implementiert.

Verworfen sind Verdichten von Zeilen, Ausnehmen des Größenwächters, Einbetten
von Source über `include!`, neue globale Besitzer, neue DTOs und Umbenennen
von Fällen. Diese Wege würden den beanstandeten Verantwortungsschnitt
verbergen oder vorhandene Verträge unnötig ändern.

Primärquellen erneut geprüft am 2026-10-26:
[Rustreferenz: Sichtbarkeit](https://doc.rust-lang.org/reference/visibility-and-privacy.html)
belegt Zugriff von Kindmodulen auf private Elterninhalte sowie die begrenzte
Freigabe mit `pub(super)`.
[Rustreferenz: Moduldateien](https://doc.rust-lang.org/reference/items/modules.html)
belegt `#[path = "..."] mod name;` relativ zum Verzeichnis der deklarierenden
Source. Folgerung: bestehende Datentypen und Felder benötigen keine neue
crateweite Veröffentlichung; nur vom Elternmodul verwendete neue
Hilfsmethoden erhalten `pub(super)`.

Zweite Lückenklärung vor der Änderung: Nach Verschieben der UI-Methoden
bedeutet `super` das Familienviewermodul. Referenzen auf bisherige
App-Geschwister müssen deshalb explizit über `super::super` beziehungsweise
einen gleichbedeutenden `app`-Alias laufen. X11-Fokus-/Familienhelfer dürfen
private Adapterfelder über ihr Elternmodul verwenden. Der Popupfixture-
Hilfsadapter erhält ausschließlich Sichtbarkeit zum privaten Testelternmodul;
die bestehenden Testfunktionen bleiben in ihrem bisherigen Modul. Das
Linux-Geometriefixture behält beim Auslagern seinen bisherigen logischen
Modulnamen, damit seine vollqualifizierten Fallnamen gleich bleiben.

### D: Endgültiger fachlicher Extraktionsplan und Karte

| Bestehende Verantwortung | Neues Teilmodul und konkrete Inhalte | Direkte und indirekte Verbindung / Erhaltungsorakel |
|---|---|---|
| Desktopfamilienfenster | `app/core/family_viewer_ui.rs`: `can_control`, `metadata`, `menu`, `resize` als weiterer impl des vorhandenen `FamilyViewerApp` | `run → new/open → eframe::App::update → UI-Methoden → family_viewer_session` bleibt erhalten. Renderer, Eingabefreigabe, lokale Menüsperre, Titel/Icon, reguläres Close, Resizeverzögerung und Disconnect behalten dieselben Ausdrücke und dieselbe Reihenfolge. Initialisierung, Update, Statefelder und Dropbesitz bleiben im Stamm. |
| Familienbestand und Capturegrenze | `remote_window/core/host_family.rs`: `HostSession::{refresh,boundary,boundary_current,set_icon}` | Bestehende `open/snapshot/operate/with_input` rufen denselben `refresh`; Viewersnapshots, Capture und Iconupdate bleiben an dieselbe Member-/Owner-/Prozess-/Rechte-/Layout-/Generationprüfung gebunden. |
| Bestätigte Hosteingabe | `remote_window/core/host_input.rs`: `HostSession::{operate,with_input}` | Der bestehende Worker ruft dieselben öffentlichen Methodennamen auf. Rechteprüfung, Fokusaktivierung/-bestätigung, Inputarming, aktuelle Revision, echte Memberidentität und Fehlerrückgabe bleiben im gleichen Hostobjekt. Stammmodul behält Traits, Box-Delegation, Konstruktion, `admitted`, `target`, `snapshot`, Epoche und Disconnect. |
| X11-Familienbeobachtung | `remote_window/os/x11/family.rs`: `app_identity`, `candidates`, `candidate_windows_inner`, `observe_inner` | Unveränderte `WindowAdapter::{candidate_windows,observe}` delegieren an diese interne Verantwortung. Aufnahme, sichtbare Flächen, rekursive Ownerfamilie, Prozess-/Markerbindung, Iconcache und Z-Reihenfolge bleiben in der oben dokumentierten Capture-/Viewer-/Inputkette. |
| X11-Fokus und Fensteranforderungen | `remote_window/os/x11/input.rs`: `root_message`, `focused_inner`, `focus_node`, `activate_if_inner`, `request_inner` | Bestehende Traitmethoden `activate/activate_if/focused/request` bleiben erhalten; Ownerfokus, Grab-/Ungrab-Abschluss, geordnete OS-Abfragen, admission, graceful Close und Resizegrenzen werden lediglich weitergeleitet. Verbindung, Markerbesitz, Bind/Adopt/Validierung, Inspect, Pintransfer und Disconnect bleiben im Stamm. |
| Kontrollierte Popupfixturefakten | `remote_window/os/x11/batches_popup_focus_fixtures.rs`: Rights/Facts/Adapter, H/M/S/ROOT, Window-/Host-/Viewerfabrik, Request-/Ack-/Inputhelfer | Die vorhandenen acht Popup-Fallfunktionen bleiben in `batches_popup_focus_task_tests`; sie verwenden unveränderte Eingaben und Orakel über privat importierte Hilfen. Kein Fallname und kein produktiver Fokusentscheider wird ersetzt. |
| Linux-Geometriefixture | `screen/os/linux_os/host_backend_family_task_tests.rs`: bisheriges Inline-Fixture unverändert | Stamm behält `LinuxBackend` und `inject_x11_family`; `[path] mod batches_family_geometry_task_tests` erhält den bisherigen Modulnamen. Source-ID, geprüfte Area, Eventphasen, Blockrückgabe und die zwei vorhandenen Fallnamen bleiben unverändert. X11-/Waylandinput und Capture-/Pauseadapter werden nicht bewegt. |

Jede Teilung ist ein eigener zusammenhängender Edit, kein eigener
Build-/Test-/Releasezyklus. Vor jeder Änderung wird der gelesene Symbolblock
erneut mit diesem Schnitt abgeglichen. Danach werden die verschobenen
Funktionskörper als statischer Text gegen `10988167` verglichen;
zulässige Unterschiede sind ausschließlich begrenzte Sichtbarkeit,
Modulpfadauflösung und die dokumentierten vier X11-Delegationen. Die
abschließende eigene Review verfolgt dieselbe vollständige Funktionskarte
einschließlich Memberidentität, Fokus, Geometrie, Eingabeablehnung,
Pausen/Ressourcenbesitz und unveränderten Testregistrierungen erneut.
Zeilen-/Bytebilanz und Diffkontrolle sind lokales statisches Feedback;
Rustformatierung, Typprüfung und Laufzeitnachweis erfolgen ausschließlich
über Mains denselben Remote-Suiteeinstieg. Kein neuer Test wird für diese
Extraktion erfunden, kein bisheriger Fall umbenannt.

### E: Umsetzung der fachlichen Teilung

Die sieben geplanten Extraktionen sind umgesetzt. Alle Funktionskörper wurden
aus dem gebundenen formatierten Stand übernommen. Beim Desktop-UI-impl ersetzt
ein Alias `use super::super as app` ausschließlich die früheren
`super::family_viewer_session`-Pfadbezüge; Titel, Icon, lokale Menüsperre,
Steuerrechte, Resizeberechnung und Zeitbedingungen sind unverändert. Der
Stamm lädt `family_viewer_ui.rs` als privates Teilmodul. Die vier vom Stamm
aufgerufenen Methoden sind nur bis zu diesem Elternmodul sichtbar.

`HostSession` behält seinen vollständigen Zustand und seine Typ-/Traitgrenze
in `host.rs`; die beiden neuen impl-Dateien werden von dort mit explizitem
`#[path]` geladen. Damit sind `HostSession::refresh`, `operate`, `with_input`,
`boundary`, `boundary_current` und `set_icon` weiter unter ihren bisherigen
Typ- und Re-Exportpfaden erreichbar. Die Source enthält keine neue Kopie des
Hosts und keine neue Mutex-/Lease-/Arc-Schicht.

Der X11-Stamm behält Verbindung, Namespace, Marker, Appcache, Validierung und
Disconnect. Seine vier Traitdelegationen sind genau:

| Bisherige Traitmethode | Neue interne Weiterleitung |
|---|---|
| `candidate_windows()` | `candidate_windows_inner()` in `family.rs` |
| `observe(root)` | `observe_inner(root)` in `family.rs` |
| `activate_if(binding, admitted)` | `activate_if_inner(binding, admitted)` in `input.rs` |
| `request(binding, operation)` | `request_inner(binding, operation)` in `input.rs` |

`activate` ruft weiterhin `activate_if` auf; `focused` verwendet unverändert
`focused_inner`. Die benötigten X11-Erweiterungstraits werden in den neuen
Dateien ausdrücklich importiert. Ergänzende Primärquellen vom 2026-10-26:
[Rustreferenz: Imports](https://doc.rust-lang.org/reference/items/use-declarations.html)
und [mehrere Inherent-Implementierungen](https://doc.rust-lang.org/reference/items/implementations.html).
Sie belegen die Namensauflösung und die fortgesetzte Erreichbarkeit der
Methoden über den bestehenden Typ, obwohl ihre impls in Teilmodulen stehen.

Die Popupfälle bleiben im bisherigen Modul
`remote_window::os::x11::batches_popup_focus_task_tests`. Sein neues privates
`fixtures`-Teilmodul liefert denselben Facts-/Rights-/Adapterbestand und
dieselben Request-/Ack-/Inputhelfer. Deren begrenzte Sichtbarkeit ist keine
Produkt-API. Die Linuxfälle behalten trotz externer Source den logischen
Modulnamen `batches_family_geometry_task_tests`. Das vorhandene allgemeine
`tests`-Modul des Linux-Backends bleibt zusätzlich unverändert registriert.

### F: Eigene anschließende Anti-Regressionsreview der Teilung

Prüfgegenstand war der fertige Extraktionsdiff gegen `10988167` und die
vollständige oben fortgeschriebene Funktionskarte. Kriterien waren
Verhaltensgleichheit, stabile externe Einstiege, derselbe Zustands-/
Ressourcenbesitzer, unveränderte Rechte-/Fokus-/Member-/Geometriegrenzen,
identische Pausen-/Abbruchpfade und tatsächlich erhaltene Fallregistrierung.
Die Review ist eine eigene statische Quellenprüfung; sie wird nicht als
Compiler-, Plattform- oder Laufzeitpass bezeichnet.

| Grenze | Eigener konkreter Abschlussbefund |
|---|---|
| Unveränderte Funktionskörper | Der statische Vergleich aller Funktionsdefinitionen der fünf Ausgangsdateien gegen ihre jeweiligen Stamm-/Extraktionsdateien stimmt überein. Ausnahmen sind ausschließlich die vier geprüften X11-Trampolinen, eingeschränkte Methodensichtbarkeit und der gleichbedeutende UI-Pfadalias. Keine Abfrage, Bedingung, Ereignisphase, Rückgabe oder Fehlerbehandlung wurde entfernt oder umgeordnet. |
| Zustandsbesitz | Die vollständigen Deklarationen `FamilyViewerApp`, `HostSession`, `X11Adapter` und `LinuxBackend` sind gegenüber `10988167` textgleich. Member-/Generationstabellen, Fokusarming/-epoche, Verbindung, Marker-/Iconcache und Quellenzustand werden nicht neu angelegt, kopiert oder zwischen Besitzern verschoben. |
| Viewer und Eingabe | `eframe::App::update` bleibt im Stamm; es ruft dieselben UI-Methoden auf. Der gemeinsame Desktop-/Android-`FamilyViewer` wird in dieser Fortsetzung nicht verändert. Korrelation, tatsächliche Bild-/Layoutbestätigung und Zielmitglied bleiben identisch. |
| Host und Capture | Die bestehenden Worker-/FamilySource-/PreparedFamily-Aufrufer verwenden weiterhin dieselben `HostSession`-Methoden. `refresh` prüft Rechte, Scope, Prozess, Owner, Generation, Sichtbarkeit und Layout in derselben Reihenfolge. `boundary_current` und `with_input` erreichen dieselben Fakten desselben Hosts. |
| X11-Fokus und Modalität | Neue `input.rs`-Methoden enthalten exakt die bisherigen Aktivierungs-/Owner-/Lifetime-/Sichtbarkeitsprüfungen. `focus::confirmed` und seine Prozess-/Ankerregeln werden nicht verändert. Die GrabServer/UngrabServer-Reihenfolge und das Fehlerweiterreichen sind textgleich; ein regulärer Dialog bleibt ein eigener Fokusanker. |
| Fensteroperationen und Abschluss | Graceful Close, Resizegrenzen, admission, Pintransfer und Disconnect bleiben unverändert. Der ursprüngliche Drop besitzt weiterhin die Verbindung und entfernt ausschließlich eigene Marker. Keine neue Appbeendigung und kein Killpfad. |
| Linux-Geometrie und Pausen | Ausschließlich das Geometriefixture wurde aus dem Backend verschoben. `prepare_session_source`, `prepare_family_member`, `inject`, `inject_family`, `inject_x11_family` und `release_input` sind textgleich. Vorhandene X11-/Wayland-/None-, Capture-/Readmission-/Pausen- und Releasepfade erhalten keine neue Verzweigung oder Initialisierung. |
| Sichtbarkeit und Dateipfade | Implteile sind private Kindmodule; Elternfelder bleiben privat. `pub(super)` öffnet nur die notwendigen Helperzugriffe. Core-Dateien enthalten keine neuen OS-Imports. Explizite `[path]`-Ziele liegen an den genehmigten Dateipfaden; X11-Teilmodule bleiben unter dem bestehenden Linux-`cfg`. Kein äußerer Re-Export oder App-Rootanschluss wurde geändert. |
| Fallnamen und Orakel | Die vorhandenen `#[test]`-Attribute und die vollständigen bestehenden Fallnamen sind im statischen Vergleich gleich. Die Fallkörper sowie Facts-/Adapter-/Ackhelfer sind unverändert. Es ist keine zusätzliche Manifestregistrierung oder Umbenennung erforderlich; dieselben bereits registrierten Linuxfälle bleiben maßgeblich. |

Die zusätzlichen Import-/Sichtbarkeitsstellen und die X11-Trampolinen wurden
nach dem Quellenvergleich einzeln gelesen. Der Diff-/Whitespacecheck ist ohne
Befund. Es gibt keinen offenen Kettenanschluss innerhalb der freigegebenen
Fläche und keine nicht genehmigte Sourceänderung. Die zuvor dokumentierte
fehlende Appisolation von sessionweitem XTest bleibt unverändert als Grenze
benannt.

### G: Größenbilanz, Dateien und verbleibender Remotenachweis

Gemessen am bearbeiteten Stand nach Übernahme der tatsächlich formatierten
Funktionskörper; kein lokaler Formatter wurde gestartet:

| Datei | Zeilen | Bytes |
|---|---:|---:|
| `app/core/family_viewer.rs` | 318 | 12145 |
| `app/core/family_viewer_ui.rs` | 257 | 10101 |
| `remote_window/core/host.rs` | 281 | 9988 |
| `remote_window/core/host_family.rs` | 260 | 9913 |
| `remote_window/core/host_input.rs` | 113 | 3953 |
| `remote_window/os/x11.rs` | 322 | 11315 |
| `remote_window/os/x11/family.rs` | 244 | 9465 |
| `remote_window/os/x11/input.rs` | 218 | 8017 |
| `remote_window/os/x11/batches_popup_focus_task_tests.rs` | 433 | 14807 |
| `remote_window/os/x11/batches_popup_focus_fixtures.rs` | 308 | 9447 |
| `screen/os/linux_os/host_backend.rs` | 476 | 18299 |
| `screen/os/linux_os/host_backend_family_task_tests.rs` | 78 | 2365 |

Jede Datei bleibt deutlich unter 50 KiB; die größte Verantwortung liegt in
normal übernommener Formatierung bei 476 Zeilen. Die nächste tatsächliche
Remoteformatierung und der Modulgrößenwächter müssen diesen Extraktionsstand
im selben Suiteeinstieg bestätigen. Der abgebrochene Vorbereitungsjob
`40332971455` ist hierfür kein Pass; er hat keine Kompilierung oder
Plattform-/Funktionsprüfung ausgeführt.

Zusätzlich für diese Fortsetzung gelesen: der aktualisierte eigene Scope und
Bericht, `format.json` und Größeninventar ausschließlich für die fünf
Zielpfade; alle fünf vollständigen formatierten Ausgangsdateien; die neuen
sieben Teilmodule; `app/mod.rs` (Familienregistrierung),
`remote_window/{mod.rs,core/mod.rs,os/mod.rs}` (Typ-/Adaptergrenzen);
`screen/os/shared/family_backend.rs`,
`remote_window/os/shared/{family_source.rs,family_capture_plan.rs}` und die
Desktop-/Mobile-Familienadapter für die bestehenden konkreten Aufrufer.
Der Graph wurde gezielt gelesen, nicht neu gebaut. Die vier genannten
Rustreferenzseiten wurden für Modulpfad, Sichtbarkeit, Imports und impl-
Erreichbarkeit gelesen; keine externe Produkt-API wurde geändert.

Geändert in dieser Fortsetzung: die fünf in der Ausgangstabelle genannten
Rustdateien und dieser eigene Bericht. Neu sind die sieben in der
Extraktionstabelle genannten Dateien; der genehmigte Pfad
`batches_popup_input_task_tests.rs` wurde nicht benötigt und nicht angelegt.
Manifest, Graph, Versionszahlen, Wire-/Persistenzformate und Abhängigkeiten
blieben durch diesen Autor unverändert. Der Autor führte keine Gitoperation
mit Schreibwirkung aus. Es liefen keine lokalen Builds,
Compiler, Formatter, Tests, Server, Installationen, Unteragenten oder
Hostprozesse.

Main besitzt den kohärenten Commit/Push, den Rootgraph und den unveränderten
Remote-Suite-Fixloop einschließlich tatsächlicher Format-/Größenbestätigung,
Typprüfung und Laufzeitnachweis sowie den terminalen Release. Es bleibt kein
zusätzlicher Scopebedarf. Diese eigene Fortsetzung wird mit Strukturplan,
Kartenfortschreibung, Umsetzung und anschließender eigener statischer
Anti-Regressionsreview übergeben; ein erfolgreicher Remotelauf wird nicht
behauptet.

## Fortsetzung: C12/C16/C17 aus Remote-Lauf 40354605251

### A: Ergebnis und Grenzen

Grundlage ist `d95dc404e834edfe7be0e1c96be162b36865efa9` im
isolierten Worktree; vollständige Evidenz:
`/tmp/smartexplorer-rv1-40354605251/x11-family-errors.json`. Dieser Remote-Lauf
ist beendet. Die aktuelle Evidenz enthält C12/C16/C17 aus Linux-
Compilerartefakten. Die Fehler belegen keinen
Funktionspass. Der bisherige G1/G2-Fixzyklus wird vom selben Autor fortgesetzt.

Beauftragtes Ergebnis: Familienfenster verwenden weiterhin dieselben echten
Viewer-, Session- und Renderertypen; X11 bleibt bei
`RustConnection<AdmissionStream>` mit vollständigem bestehenden Header-/
Raw-/Parser-Speichergate und der öffentlichen API von x11rb 0.13.2.
Keine Zweittypen, keine Rendererattrappe, keine Umgehung der Speicherzulassung,
kein Wechsel zum Defaultstream und keine gestrichene Testaussage.

Die endgültige Scope-JSON ersetzt die vorherige Strukturfläche. Schreibbar
sind ausschließlich die drei genannten X11-Admission-/Capture-/Cursorquellen
und dieser Bericht. C02–C06 samt Modulregistrierung, Planung und Eigenreview
gehören laut Mains anschließender Präzisierung vollständig zum Captureautor.
Er registriert Login- und Familienmodule gemeinsam unter `app::screen_modules`
und erhält die Aufrufernamen durch private Familienaliases an der Appwurzel.
Die nachfolgende Familienanalyse ist übergebener Bestandsinput, keine fertige
Review seiner Umsetzung. Eigene Familienimporte, Getter oder
Sichtbarkeitsöffnungen sind ausgeschlossen. Keine lokalen Builds, Compiler, Tests,
Formatter, Server, Installationen, Gitmutationen, Graphupdates oder Unteragenten.

Erstplan: vollständige Compilertexte lesen, ihre tatsächlichen Typdefinitionen
und Aufrufer verfolgen, die öffentliche gepinnte Stream-API prüfen,
Erhaltungsplan je Grenze festhalten, schmalen Fix umsetzen und danach eigene
Anti-Regressionsreview liefern. Die Aufrufkanten der drei im Elternmodul
deklarierten Capture-Unterdateien werden nach Mains konkreter Lesefreigabe
ergänzt. Kein Nutzerentscheid ist nötig.

Dokumentationskontext vor dieser Ergänzung: gezieltes `git status --short`
für die eigene Fläche ohne Ausgabe; letzter Commit `d95dc404 fix(transport):
initialize path policy within its privacy boundary [task candidate]`.
Architektur-/Oberflächeneinstiege und `rv1-modulplan.md` wurden für die
bestehende Familien-/Capture-/Renderergrenze geprüft. Der aktuelle Scope
erteilt keinen erneuten Graphzugriff; Main hat den Graph laut Auftrag geprüft.

### B: Fehlerhafte Funktionen und berührte Kette

C02–C06 wurden vor Mains Neuzuordnung bereits selbst untersucht. Die fünf
Tabellenzeilen und die ersten fünf Kartenpunkte halten genau diesen an den
Captureautor übergebenen Befund fest. Die eigene aktuelle Korrektur und
nachfolgende Review betreffen C12/C16/C17.

| Befund | Tatsächliche Grenze und Endnutzerfolge |
|---|---|
| C02 | `FamilyViewerApp` importiert `ViewerPrefs` über den nicht vorhandenen App-Geschwisterpfad `super::screen_prefs`; die echte Definition liegt in `app::screen_modules::screen_prefs`. Das Familienfenster kann so nicht kompiliert werden. |
| C03 | `FamilyViewerApp::{new,open,stop,update}` importiert `Backend` und `ViewerSession` über denselben falschen Elternpfad; der echte Session-/Decoder-/Rendererbesitz muss erhalten bleiben. |
| C04 | `FamilyViewerApp::{new,update}` erreicht `ViewWidget` und `ViewParams` nicht über `super::screen_view_widget`; ein Ersatzwidget würde PRESENTED-/Layout-/Inputgrenzen umgehen und ist ausgeschlossen. |
| C05 | `family_viewer_events::receive` erreicht den echten `Shared`-Typ nicht. Außerdem muss dessen tatsächliches `resume.ready()`-Gate app-intern erreichbar bleiben, ohne ein zweites Shared oder einen ungeprüften Boolzustand anzulegen. |
| C06 | `family_viewer_session::{input,operate,disarm}` erreicht den echten `ViewerSession`-Typ nicht. Die vorhandene Inputfreigabe, Sessionwarteschlange und korrelierte Familienoperation dürfen nicht ersetzt werden. |
| C12 | `x11_admission_stream.rs` importiert `Stream`/`PollMode` über das private Untermodul `rust_connection::stream`, obwohl die gepinnte Bibliothek dieselben Typen an `rust_connection` öffentlich reexportiert. |
| C16/C17 | `X11Source.conn` und `x11_cursor::fetch` verwenden korrekt den Alias `CaptureConnection = RustConnection<AdmissionStream>`, können dessen Traitbindung wegen C12 aber nicht auflösen. Der Alias und die AdmissionStream-Implementierung bleiben bestehen. |

Die vollständige direkte und indirekte Karte dieser berührten Grenze ist:

1. Im untersuchten Fehlerstand lädt die Appwurzel `screen_modules` einmal
   und reexportiert dessen freigegebene Inhalte. Dort sind die bestehenden
   Prefs-, Session-, Widget-, Geräte-, Render-, Sichtbarkeits-, Statistik- und
   Tastentypen definiert. Die damals direkt unter `app` liegenden Familienmodule
   müssen dieselben Definitionen benutzen. Mains bestätigter Modulansatz
   ordnet sie beim Captureautor demselben `screen_modules`-Elternmodul zu.
2. `family_viewer::run → FamilyViewerApp::{new,open,update}` verwendet
   `Backend`, `ViewerSession::open`, `ViewWidget::new/show/release_input`,
   Geräteprofile, `full_request`, GPU-YUV-Eignung, Sichtbarkeitsfakten,
   Statistik und lokale Tasten. WGPU-/Glow-Auswahl, Titel/Icon, Menü,
   reguläres Close, Resize und Disconnect bleiben im vorhandenen Appobjekt.
3. Der echte `ViewerSession` besitzt `Shared`, Outbox und Decoderthread;
   `send(Input)` routet bei Familiensitzungen zurück an
   `family_viewer_session::input`. Dieser ruft denselben gemeinsamen
   `FamilyViewer::input` auf und sendet ausschließlich daraus entstandene
   Familienframes. `input_ready` prüft weiterhin Presentationgültigkeit und
   Resume-Gate. Es entsteht kein roher Inputfallback.
4. `family_viewer_events::receive` verarbeitet Snapshot, korrelierte
   Operationsantwort und Inputablehnung auf demselben `Shared`. Layoutwechsel
   invalidiert Presentation; zurückgehaltene Eingaben verlassen das Gate nur,
   solange dessen wirkliche Bereitschaft offen ist. Iconinhalte bleiben an
   erwartete Iconreferenz/Hash und echte Sessionmetadaten gebunden.
5. Der echte `ViewWidget` nimmt Sessionbilder, verwendet die bestehende
   Presentationidentität, tatsächlichen CPU-/YUV-/GPU-Renderer und meldet
   Sichtbarkeit. Seine Maus-/Touch-/Tasteneingaben behalten die bisherige
   Geometrie- und Steuersperre. Die Familien-UI hält dieselbe WindowVisibility-
   TypeId im egui-Temporärspeicher; ein gleich aussehender Zweittyp wäre falsch.
6. `X11Source::prepare → AdmissionStream::connect` führt die unveränderte
   DISPLAY-Auflösung/Authentifizierung durch, übernimmt Setup und eröffnet
   die äußere `RustConnection<AdmissionStream>` auf demselben Transport.
   Der Transport sendet nach Setup keine eigenen Requests; die äußere
   Verbindung besitzt die bekannte Sequenzfolge.
7. `AdmissionStream::poll → Gate::authorize` lässt den 32-Byte-Replyheader erst
   nach passender Raw-/Parserzulassung an x11rb weiter. `read` wartet nicht auf
   Parent-IPC; es hält Header und Body bis zur Gatefreigabe zurück. Fehler,
   FD-Reihenfolge, Teilreads, Raw-/Parser-Slices und `retired`-Besitz bleiben in
   denselben vorhandenen Objekten. `write` und `write_vectored` delegieren
   unverändert an denselben authentifizierten Transport.
8. `X11Source` besitzt weiter `CaptureConnection`, `Gate`, Admission, SourceKey,
   Generation und Grant. Capture, Größenwechsel, SHM-/GetImage-Pfad und
   Wiederzulassung bleiben davon abhängig. `x11_cursor::fetch` verwendet
   dieselbe Verbindung, erwartet die konkrete Replysequenz, armiert
   `Gate::cursor`, liest den Reply und hält den zugelassenen Parser-Slice bis
   zum Ende des zugehörigen Cursorbildes.

Die vorhandene Root-/Host-/Fokus-/Geometrie-/Remote-Framekette aus den
vorherigen Abschnitten bleibt ihr anschließender Verbraucher. Es wird weder
ein anderer Worker noch eine andere Familienidentität erzeugt.

### C/D: Konkreter Erhaltungsplan und Quellen

Gewählt für den eigenen Fix: Änderung allein des privaten x11rb-Importpfads
zum öffentlichen Reexport. Der zunächst betrachtete Zugriff über schmale
Screen-Exporte oder Resume-Getter ist durch Mains gemeinsame Modulregistrierung
beim Captureautor ersetzt und wird hier nicht umgesetzt. Verworfen:
duplizierte `Backend`-/`Shared`-/Widgettypen, neue Sessionobjekte,
Defaultstream statt AdmissionStream, höhere Speicherbudgets, entfernte
Assertions oder verbreiterte Plattformrechte.

Herstellerquelle geprüft am 2026-10-26:
[x11rb 0.13.2, rust_connection/mod.rs](https://raw.githubusercontent.com/psychon/x11rb/v0.13.2/x11rb/src/rust_connection/mod.rs)
reexportiert `PollMode` und `Stream` öffentlich. Dieselbe Quelle belegt
`RustConnection<S: Stream>`, `for_connected_stream(stream, setup)` und
`stream() -> &S`. Der [versionierte Streamvertrag](https://raw.githubusercontent.com/psychon/x11rb/v0.13.2/x11rb/src/rust_connection/stream.rs)
belegt `poll`, nichtblockierendes `read`/`write`, `write_vectored`, FD-Reihenfolge
und `WouldBlock`-/Interruptverhalten. Die vorhandene Implementierung verwendet
genau diese Signaturen. `Cargo.lock:9602` belegt Version 0.13.2. Es ist keine
Abhängigkeitsänderung nötig.

1. **Kanonische Typanschlüsse, übergeben:** Main hat die vollständig
   benannten Screenzugänge und Rückkanten erhalten. C02–C06 verbleiben
   einschließlich Umsetzung und Eigenreview beim Captureautor. Der eigene
   Bericht behauptet dafür keinen Abschluss und verändert keine Familienquelle.
2. **Öffentliche X11-API:** Import auf
   `x11rb::rust_connection::{PollMode, RustConnection, Stream}` ändern.
   Erwartung: dieselbe bestehende `impl Stream for AdmissionStream` erfüllt
   die generische CaptureConnection-Bindung; sämtliche Gate-/Leser-/Writer-
   und Fixturekörper bleiben textgleich. `x11_capture.rs` und `x11_cursor.rs`
   erhalten nur dann einen Edit, wenn ihr konkreter Verbraucheranschluss
   eine notwendige weitere API-Korrektur belegt.
3. **Eigene Anti-Regressionsreview:** vollständigen Diff gegen `d95dc404`
   entlang der Karte prüfen; Gatezustand, Speicherbesitz, FD-/Sequenzbezug,
   sämtliche vorhandenen Fallaussagen erhalten. Für die Familienquelle wird
   ausschließlich das Ausbleiben eines eigenen Edits bestätigt.
   Nur statische Text-/Syntaxrückmeldung; funktionaler Nachweis ausschließlich
   in Mains derselben Remote-Suite. Kein zusätzlicher lokaler Testlauf.

### E: Vollständige X11-Funktionskarte und Umsetzung

Die nachgeforderte Lesefläche wurde von Main um `x11_capture_read.rs`,
`x11_capture_memory.rs`, `x11_capture_source.rs`, `x11_capture_target.rs`
und `x11_capture_shm.rs` ergänzt. Alle fünf Dateien wurden vollständig
gelesen. Der abgegrenzte Reviewgegenstand ist die öffentliche Streambindung
und ihr vollständiger bestehender Capture-/Cursor-/SHM-Verbraucherweg:

| Funktion beziehungsweise Typgrenze | Direkter Anschluss und indirekter Erhaltungsvertrag |
|---|---|
| Öffentlicher x11rb-Import; `CaptureConnection` | `Stream` und `PollMode` kommen aus `rust_connection`; `CaptureConnection` bleibt genau `RustConnection<AdmissionStream>`. C12 betrifft den Import, C16 das Feld `X11Source.conn`, C17 den Parameter `x11_cursor::fetch`. |
| `State::default`, `invalid`, `blocked` | Unveränderte Initialisierung von Header, Teilread, Freigabe, erwarteter Sequenz, Raw-/Parserbesitz, zurückgehaltenen Slices und Fehler; vorhandene `InvalidData`-/`WouldBlock`-Semantik. |
| `AdmissionStream::{connect,gate}` | Die ursprüngliche Verbindung erledigt DISPLAY, Authentifizierung und Setup; `for_connected_stream` umschließt denselben Transport. SHM-Abschluss erreicht über `conn.stream().gate()` dasselbe Gate. |
| `AdmissionStream::{poll,read,write,write_vectored}` | `poll` autorisiert den zurückgehaltenen Header vor der Parserfreigabe; `read` liest niemals Parent-IPC und liefert Header/Body nur in begrenzten Teilen; beide Schreibmethoden behalten Transport und FD-Vektoren. Der bestehende endliche Poll und Interruptretry bleiben erhalten. |
| `Gate::{image,cursor,authorize}` | GetImage bindet die erwartete Sequenz an einen bereits beanspruchten Raw-Slice. XFixes bindet die erwartete Sequenz an den Bildgrant; Headergeometrie bestimmt die ergänzende Raw-/Parserzulassung vor Parserwachstum. Falsche Bildlänge, ungültige Cursorgröße und unarmierte Replies über 64 KiB werden abgewiesen; kleine Metadatenreplies behalten ihren bestehenden Weg. |
| `Gate::{finish,retain}` | `finish` löst Rawbesitz erst nach abgeschlossener Replyverarbeitung und übergibt den Parserslice. Nicht bestätigter nativer Abschluss bleibt in `retired` gebucht, bis die Verbindung zerstört ist. |
| `X11Source::{open,prepare,prepared_info,activate}` | Der Testeinstieg verwendet denselben Prepare-/Plan-/Activate-Weg. Prepare hält tatsächliche Quelle, Geometrie und Familien-Pixmapanforderung; Activate prüft aktuelle Geometrie, Compositor und Grant gegen den Plan, bevor der Grant gespeichert wird. |
| `X11Source::{aim,current_area,note_geometry,pass}` | Monitor-ID beziehungsweise X11-Fenster-ID bleibt dieselbe Quelle; leere oder mehrdeutige Geometrie scheitert. Orts-/Größenwechsel invalidiert Bildcache und Eingabereferenz; jeder Monitorpass liest aktuelle RandR-Geometrie. |
| `X11Source::{is_normal,pass_monitor,pass_window}` | Minimierte/verborgene Fenster bleiben pausiert. Familienfenster verlangen weiterhin ihre eigene Compositorpixmap; ein verlorener Compositor führt weiter zum Fehler. Der bisher ausdrücklich freigegebene Rohfensterpfad behält seinen Root-Crop-Fallback. |
| `capture_memory::{plan,verify}`; `X11Source::{plan,pixmap_geometry,admit_geometry}` | SourcePixels, Raw-Readback, SHM-/Pixmap-NativeSurface und Encoderanteile bleiben getrennte Eigentümer. Identität und Generation werden geprüft. Geometriewechsel gibt alte Segmente und Bilder frei, invalidiert die Referenz, behandelt Mitgliedsgrants gesondert und fordert den neuen Plan über dieselbe Authority an. |
| `X11Source::{ensure_segment,read,read_pixmap}` | SHM beansprucht Segment und Zielbild getrennt. GetImage beansprucht geparste Pixel vor der Allokation und armiert den Rawstream vor dem sequenzgeprüften Request. Pixmapbesitz wird vor Servergrab beansprucht, die Geometrie im Grab erneut geprüft und kein Parent-Aufruf im Grab ausgeführt; unsicheres Free behält den Slice im Gate. |
| `Segment::{create,bytes,release,drop}` | Derselbe `CaptureConnection` übergibt den duplizierten SHM-FD. Attach-/Detachfehler behalten die Zulassung über `gate.retain`; Mapping und lokaler FD behalten ihre vorhandenen Abschlussbesitzer. |
| `x11_cursor::{enable,fetch,blend}`; `AdmittedCursor` | XFixes bleibt optional. Fetch prüft Fence-/Cookie-Sequenz, armiert dasselbe Gate, übernimmt den zugelassenen Parser-Slice und prüft die tatsächliche Vec-Kapazität. Der Cursor besitzt zuerst seine ARGB-Pixel und danach den Slice; Blend bleibt auf den erfassten Bildausschnitt begrenzt. |
| `ScreenSource for X11Source::{capture_grant,legacy_input_invalid,reference_generation,inject_input,frame_limits,size,capture}`; `X11Source::drop` | Capture führt Zielpass, zugelassenen Cursor, Freeze, Geometrienachprüfung, Hash-/Pixelvergleich und Keyframecache zusammen. Input verlangt dieselbe Generation, aktuelle tatsächliche Geometrie und ein vorhandenes Bild vor `inject_bound`. Drop löst Segment und Bildcache vor Grantabschluss. |
| `picture_hash`, `failed`, `lost`, `reply_error`, `refuse`, `shm_usable`, `compositor_running`, `probe` | Hash-/Fehlerklassifikation und Metadatenproben bleiben unverändert. Die reine Serverprobe verwendet weiterhin ihre eigenständige Metadatenverbindung; der bildproduzierende Pfad verwendet weiterhin ausschließlich den Admissionstream. |

Die gerichtete Vorbereitungskette lautet `prepare → AdmissionStream::connect
→ CaptureConnection → aim → current_area/note_geometry`; der vom Host
übergebene Grant wird über `activate → current_area/Compositorprüfung
→ plan/verify` geprüft. Der Bildweg lautet `ScreenSource::capture → pass`:
Beim Monitor folgen `current_area → admit_geometry → pass_monitor → read`,
beim Fenster `pass_window → Zustandsprüfung → admit_geometry → read_pixmap`
beziehungsweise der erlaubte Rohfensterfallback `read`. `read_pixmap` führt
über `pixmap_geometry → NativeSurface-Slice → Grab/Name/UnGrab → read
→ Free/gegebenenfalls Gate::retain`.
Der GetImage-Zweig von `read` lautet `SourcePixels-Zulassung → Fence-Reply
→ Gate::image → sequenzgeprüftes GetImage → Stream::{write,write_vectored,
poll,read}/Gate::authorize → Reply → Gate::finish → CaptureImageBuilder`.
Der SHM-Zweig von `read` lautet `ensure_segment → Segment::create → derselbe
Stream → shm_get_image → zugelassene SourcePixels → CaptureImageBuilder`;
Wiederzulassung und Drop führen über `Segment::release → gate.retain` bei
unsicherem Detach. Der zusätzliche Cursorzweig lautet `capture
→ x11_cursor::fetch → Fence-Reply → Gate::cursor → derselbe
Stream/Gate::authorize → Reply → Gate::finish → AdmittedCursor → blend
→ freeze`.
Der nachfolgende Eingabezweig bleibt `inject_input → Referenz-/Geometrieprüfung
→ inject_bound`. Die im ersten Fixteil untersuchte Host-/Familien-/Fokuskette
bleibt jenseits dieser unveränderten Adaptergrenze bestehen.

Umgesetzt ist ausschließlich:
`use x11rb::rust_connection::{PollMode, RustConnection, Stream};`.
Die vorhandene `impl Stream for AdmissionStream` bindet damit den öffentlich
reexportierten Bibliothekstrait. Weder eine neue Streamimplementierung noch
ein Dependency-, Budget-, Quell-, Renderer-, Rechte- oder Eingabevertrag ist
hinzugekommen. `x11_capture.rs` und `x11_cursor.rs` benötigten keinen Edit.

### F: Eigene anschließende Anti-Regressionsreview

Der Autor hat nach der Importänderung den vollständigen eigenen Produktdiff
gegen `d95dc404` und die acht Quellen der obigen Karte geprüft. Ein statischer
Bytevergleich bestätigt: `x11_admission_stream.rs` entspricht exakt dem
Basisinhalt mit genau dem ersetzten Importblock; die sieben Verbraucherdateien
sind vollständig bytegleich zur Basis. Damit sind nicht nur einzelne
Funktionsnamen, sondern sämtliche Zustands-, Funktions-, Fehler- und
Fixturekörper dieses abgegrenzten Wegs erhalten. Vorhandene Fallnamen und
Testattribute bleiben gleich; kein Test wurde ausgeführt.

Die APIprüfung an der versionierten Herstellerquelle bestätigt die vier
vorhandenen Traitsignaturen und den öffentlichen Reexport. Aus diesem Befund
folgt die Korrektur der gemeldeten privaten Importgrenze; ein tatsächlicher
erfolgreicher Compilerlauf wird daraus nicht abgeleitet. C16/C17 behalten den
echten AdmissionStream als generischen Besitzer und werden durch dieselbe
Traitbindung adressiert.

Die eigene Review hat insbesondere diese Erhaltungsfragen geprüft:

- Header-/Bodyfreigabe und Raw-/Parserzulassung bleiben vor dem Parserwachstum;
  ausschließlich `poll` kann dabei Parent-Zulassung anfordern.
- Replysequenz, genaue Bildlänge, Cursorgröße und tatsächliche Parserkapazität
  behalten ihre vorhandenen Prüfungen. Ein Fehler öffnet keine Ersatzverbindung.
- Grantidentität, Generation, Referenzwechsel, Mitgliedsgrants und gehaltene
  Raw-/Parser-/SHM-/Pixmap-Slices behalten dieselben Besitzer und Lebensdauern.
- Familienpixmaps, Compositorverlust, minimierte Fenster, Root-Crop für den
  bisherigen Rohfensterpfad, SHM/GetImage und Cursorclipping bleiben vorhanden.
- Der erfasste Quellbereich und die tatsächliche Eingabegeometrie bleiben vor
  `inject_bound` gebunden. Der Importfix berührt weder FocusConfirmed noch
  Eigentümerketten, Modalität, Rechte oder Entwaffnung aus dem G1/G2-Fix.
- Familien-, Session- und Rendererquellen erhielten durch diesen Autor keinen
  Edit. Der gemeinsame Modulfix ist Gegenstand des Captureautors; dessen
  Eigenreview wird hier nicht stellvertretend behauptet.

Statischer Diff-/Whitespacecheck ohne Befund. Im geprüften Sourcebestand hat
die geänderte Datei 363 Zeilen / 13.233 Bytes. Die unveränderten unmittelbaren
Verbraucher haben: Capture 386 / 13.786, Cursor 120 / 4.479, Read 184 / 6.628,
Memory 163 / 5.618, Source 112 / 3.878, Target 87 / 3.507 und SHM 94 / 3.155
(jeweils Zeilen / Bytes). Alle acht liegen unter 500 Zeilen und 50 KiB;
kein Formatter wurde lokal ausgeführt. Die neue Importzeile ist 63 Zeichen
lang und benötigt keine künstliche Verdichtung.

### G: Fälle, Dateien und Übergabegrenze

Keine neue Fallregistrierung und keine Änderung am gemeinsamen Manifest.
Im gelesenen `native/review-task-manifest.json` sind diese bestehenden
gezielten Admission-/Cursor-/SHM-Fälle registriert:

- `batches_task_capture_linux_x11_cursor_header_precedes_parent_approval`
- `batches_task_capture_linux_x11_oversized_cursor_is_rejected_before_credit_request`
- `batches_task_capture_linux_x11_exact_reply_and_immutable_last_owner`
- `batches_task_capture_linux_x11_malformed_reply_never_opens_parser_gate`
- `batches_task_capture_linux_x11_shm_and_getimage_count_distinct_native_owners`
- `batches_task_capture_linux_x11_supplement_grants_raw_and_argb_without_retiring_image`

Der letzte Fall bleibt mit allen Assertions unverändert in der geänderten
Streamdatei. Die zehn zuvor in diesem Bericht genannten G1/G2-Fallnamen sind
im aktuellen Manifest ebenfalls vorhanden. Deren Ausführung und Auswertung
bleiben Main im selben Remote-Suiteeinstieg vorbehalten.

In dieser Fortsetzung gelesen: `AGENTS.md`, `docs/ARCHITEKTUR.md`,
`docs/oberflaechen/bildschirm.md`, dieser Bericht,
`docs/plaene/2026-10-24-batches/rv1-modulplan.md`, `native/Cargo.toml`,
`native/Cargo.lock`, `native/review-task-manifest.json`,
`native/src/app/mod.rs`, `native/src/app/screen_modules.rs`,
`native/src/app/core/{family_viewer,family_viewer_events,family_viewer_session,
screen_prefs,screen_session,screen_target,screen_view_widget}.rs` sowie
`native/src/screen/os/linux_os/{x11_admission_stream,x11_capture,x11_cursor,
x11_capture_read,x11_capture_memory,x11_capture_source,x11_capture_target,
x11_capture_shm}.rs`. Architektur, Oberflächen, Prefs, Widget, Target und
Manifest wurden auftragsbezogen in den relevanten Abschnitten gelesen;
die acht X11-Quellen und die drei Familienkonsumenten wurden vollständig gelesen.
Zusätzliche Auftragsbelege: `/tmp/smartexplorer-final-review-user-contract.txt`,
`/tmp/smartexplorer-fix-x11-family-scope.json`,
`/tmp/smartexplorer-rv1-40354605251/x11-family-errors.json` und
`/root/.codex/AGENTS.md`. Herstellerquellen: die beiden versionierten x11rb-
Dateien aus C/D, geprüft am 2026-10-26.

Geändert: ausschließlich dieser Bericht und
`native/src/screen/os/linux_os/x11_admission_stream.rs`. Keine neue Datei.
Entscheidung: öffentliche gepinnte API statt Admissionumgehung;
C02–C06 beim gemeinsamen Modulautor belassen. In der aktuellen eigenen
Schreibfläche bleibt keine belegte ungeklärte Quellabhängigkeit. Noch nicht
belegt sind die erfolgreiche Remote-Kompilierung, die funktionale Ausführung
der registrierten Fälle und der terminale Release; diese Schritte besitzt Main.
Keine lokalen Builds, Compiler, Tests, Formatter, Server, Installationen,
Gitmutationen, Graphupdates oder Unteragenten. Der eigene statische Fixzyklus
ist mit dieser Übergabe beendet.

## Fortsetzung: D13/D18/D19/D20 aus Remote-Lauf 40368746815

### A: Erstplan, Ergebnis und Grenzen

Quelle ist Mains Fortsetzungsauftrag zum vollständig beendeten Lauf
`40368746815`, Kandidat `b233e5336a0ee177e19382a7adf592d297dcf202`.
Beauftragtes Ergebnis: native Fensteridentität, tatsächliche Enabled- und
Fokusprüfung, X11-Serverfreigabe und konservative Sichtbarkeit bleiben nach
der Korrektur von D13/D18/D19/D20 wirksam. Derselbe Autor liefert selbst
Fehlerfunktionsliste, vollständige verbundene Karte, vorherigen Erhaltungsplan,
Umsetzung und anschließende eigene Anti-Regressionsreview. Der unveränderte
Nutzervertrag in `/tmp/smartexplorer-final-review-user-contract.txt` gilt.

Die aktuelle Scope-JSON ersetzt frühere Schreibflächen. Ausschließlich dieser
Bericht, `native/src/app/os/linux_os/screen_visibility.rs`,
`native/src/remote_window/os/windows.rs`, dessen `windows/facts.rs` und
`native/src/remote_window/os/x11.rs` sind schreibbar. Cargo.toml gehört dem
Geräteautor. Keine lokalen Builds, Compiler, Formatter, Tests, Server,
Installationen, Gitmutationen, Graphupdates, Releases oder Unteragenten.
Main hat den Dokumentationskontextgate und die auftragsbezogene Graphabfrage
laut Auftrag bereits erledigt; diese Delegation führt keinen neuen Lauf aus.

Promptwissen: Main hat `IsWindowEnabled` in windows-sys 0.59 im Namespace
KeyboardAndMouse nachgewiesen; das passende Feature ist bereits aktiviert.
Die eigene Primärquellenprüfung steht als nächster Schritt an. Die zwei
X11-Grenzen müssen ihre bisherigen Fehler- und Antwortlebensdauern behalten.
Die genauen fehlerhaften Ausdrücke, ihre Aufrufer und vorhandenen Fallnamen
sind noch durch die ausdrücklich freigegebenen Quellen und Fehlertexte zu
klären. Eine fehlende Nutzerentscheidung ist bisher nicht erkennbar.

Vorläufige Planungstiefe: A bis D für die gekoppelte Windows-/X11-
Fensteridentitäts- und Sichtbarkeitsgrenze. Zuerst vollständige vier Befunde
und betroffene Quellen lesen; anschließend Definitions-/Aufruf-/Fixturekette
und versionierte Primärverträge prüfen. Daraus den konkreten Erhaltungsplan
festhalten, schmale Korrekturen umsetzen und danach dieselbe Kette selbst
statisch prüfen. Bestätigung nur durch Main im selben Remote-Einstieg.

### B: Belegter Bestand, Fehlerfunktionen und Verknüpfungskarte

Die vollständigen vier Compilertexte in
`/tmp/smartexplorer-rv1-40368746815/x11-family-errors.json` wurden gelesen.
D13 und D20 stammen aus beiden Linux-Compilerartefakten, D18 und D19 aus
beiden Windows-Compilerartefakten; der Export enthält keine Androidfehler.
Die vier Fehler sind kein Funktionspass. Der Code verwendet Rust-Edition
2021, windows-sys 0.59 und x11rb 0.13; die vorher belegte Lockversion von
x11rb ist 0.13.2.

| Befund | Tatsächliche fehlerhafte Funktion und Folge |
|---|---|
| D13 / E0597 | `app/os/linux_os/screen_visibility::property`: Der Tail-Ausdruck `Some(reply.value32()?.collect())` enthält einen von `reply` geliehenen Iterator und dessen `?`-Temporärwert. Die Rust-2021-Abschlussreihenfolge verhindert die Kompilierung. Erfolgreiche Ergebnisse müssen weiterhin ein eigenes `Vec<u32>` sein; unvollständige oder falsch formatierte Properties bleiben unbekannt. |
| D18 / E0425 | `WindowsAdapter::focused`: `IsWindowEnabled` fehlt im Importbereich, obwohl die korrekte API aktiviert ist. Der unveränderte Ausdruck verlangt gleichzeitig Vordergrundfenster, Enabled und nicht minimiert. Die ähnlich benannte Compilerhilfe `IsWindowArranged` erfüllt diesen Vertrag nicht. |
| D19 / E0425 | `windows::facts::WindowFacts::read`: Derselbe fehlende Elternimport erreicht das Kindmodul über dessen `use super::*` nicht. Die tatsächliche Enabled-Eigenschaft muss weiterhin Größenänderung, Aktivierung, Schließen und reguläre Eingabe begrenzen. Die vorhandenen separaten Menüregeln bleiben bestehen. |
| D20 / E0282 | `X11Adapter::bind`: Die unter Servergrab ausgeführte Closure liefert `Ok(WindowBinding)`, während `?`-Konvertierungen den Fehlertyp nicht eindeutig bestimmen. Genau `io::Result<WindowBinding>` muss den bestehenden Bindetransaktionswert typisieren; keine Änderung von Grab, Marker, Prozessprüfung, UnGrab oder Fehlerpriorität. |

Die vollständige direkt und indirekt berührte Karte an diesen Grenzen:

1. `remote_window::os::{adapter,adopt}` wählt denselben Windows- oder
   X11-Adapter. `WindowAdapter` und dessen Box-Weiterleitungen erhalten ihre
   bestehenden Signaturen. `WindowBinding` bleibt native Kennung plus
   Lebensdauermarker; `WindowPin` bindet Plattform, Namespace und genau diese
   Bindung. Wire-Mitglieder verwenden weiterhin `WindowIdentity`, keine
   ungeprüfte rohe HWND/XID als entfernte Identität.
2. `FamilyPins::list → adapter.bind → inspect → validate` hält einen
   gemeinsamen Adapter für die Liste und erzeugt den bisherigen Selektor mit
   Lebensdauerkennung. `take` bleibt an Besitzer und Selektor gebunden;
   `register/receipt → os::adopt → inspect` behält den Startbeleg und prüft
   wiederholte Pins. `PinnedFamily::{prepare,open} → export_pin →
   PreparedWorker::new/WorkerSource::open_family → transfer_pin` übergibt die
   Aufräumverantwortung erst nach bestätigtem Öffnen. Diese vorhandene typisierte
   Workerübergabe und die früher im Bericht kartierte Familienkette bleiben
   die anschließenden Integrationsgrenzen; kein Worker wird hier geändert.
3. Windows `new/adopt → process → facts::process` hält Session, PID, Thread
   und Prozesserstellungszeit. `bind → SetPropW → validate` verbindet den
   zufälligen Marker mit der Prozesssignatur; `validate` prüft HWND, passende
   Property und Signatur. `process_user_matches → facts::user_matches`
   vergleicht tatsächliche Anmeldung und SID vor erneuter Validierung.
4. Windows `candidate_windows/inspect/observe → WindowFacts::read` verwendet
   physische DWM-Geometrie, tatsächliche Owner, Fensterklasse, Stile,
   Sichtbarkeit, Minimizierung und die echte Enabled-Abfrage. `observe` nimmt
   nur denselben Prozessstart und rekursiv zugeordnete Owner auf; Systemmenüs
   benötigen den tatsächlichen `GetGUIThreadInfo`-Menüowner. Die bisherigen
   Menu-/Tooltip-Regeln sowie Session-/Desktopblockierung bleiben unverändert.
5. Windows `activate/activate_if → control_ready → validate →
   ShowWindowAsync/SetForegroundWindow → validate` bewahrt frische Zulassung
   vor OS-Eingriffen. `focused` verlangt für reguläre Fenster Vordergrund,
   `IsWindowEnabled` und nicht minimiert; beim Menü verlangt es den separat
   validierten gebundenen Menüowner als Vordergrundfenster. `request` behält
   Close/Resize/Focus; `disconnect/drop` entfernt ausschließlich passende
   eigene Marker und beendet kein Programm.
6. X11 `connect/adopt → XRes-/Namespaceprüfung → validated_process` hält
   tatsächlichen XRes-Clientprozess, UID, Session und Prozessstart. Wayland
   bleibt am Portalweg. `bind` verwendet die vorhandene Bindung nur nach
   frischer Prüfung; sonst folgen geprüfter Grab, Prozessprüfung, zufälliger
   Lebensdauermarker, Propertyinstallation und erneute Validierung, danach
   geprüfter UnGrab und erst bei Gesamterfolg Speicherung in `markers`.
7. X11 `candidate_windows_inner/observe_inner → candidates/properties → bind`
   nimmt Root und rekursiv bewiesene Transient-Nachfahren desselben Prozesses
   auf. `properties::{process,bytes,words,geometry,title,app,kind}` behalten
   XRes-/UID-/Startbeweis, begrenzte vollständige Properties, Geometrie und
   atomgebundene Typen. Combo-Popups bleiben Popup; zugehörige Menüs dürfen
   Eingabe behalten, Tooltips keine Eingabe, fremde Oberflächen keine Bindung.
8. X11 `activate_if_inner/focused_inner/request_inner` verwendet dieselben
   Marker und kurzlebigen Servertransaktionen. `focus_node →
   focus::{owner_focus,can_activate,confirmed,anchor}` verfolgt ausschließlich
   bereits gebundene und frisch validierte Owner-/Prozessketten. Reguläre
   Dialoge bleiben eigene Aktivierungsgrenzen; Zyklen, fremde Prozesse,
   recycelte XIDs, verborgene Owner und ungebundene Oberflächen bestätigen
   keinen Fokus. Close bleibt das beworbene graceful Protokoll, Resize bleibt
   durch WM-Fähigkeit und erlaubte Aktion begrenzt.
9. `HostSession::{open,open_bound,open_bound_as} → bind/validate → refresh`
   bindet Sessionfreigabe, Root, Mitglieder und Generationen. `refresh` prüft
   Sicht-/Steuerrechte und Listenscope, übernimmt nur rekursive Owner desselben
   Prozesses, invalidiert Eingabefreigabe bei Fokus-/Rechtewechsel und hält
   Layoutrevision getrennt von Metadaten. `boundary/boundary_current/set_icon`
   behalten validierte Capturemitglieder, Geometrie, Revision und Iconbezug.
10. `HostSession::operate → validate/activate/refresh/focused/validate` liefert
    weiterhin nur bei bestätigtem Fokus `FocusConfirmed`. `with_input` prüft
    Rechte, aktuelle Snapshotrevision, vorherige Fokusbestätigung, Mitgliedsfähigkeit,
    Bindung und Fokus erneut; der Injektor erhält dieselbe konkrete
    `CaptureMember`-Geometrie. Fehlender Fokus und Injektorfehler entwaffnen.
11. `app::screen_modules` registriert den portablen Sichtbarkeitstyp und
    `os/shared/screen_visibility`; dessen Linux-Adapter ist der Fehlerort D13.
    `FamilyViewerApp::update → sample → sample_x11 → property` liest tatsächliche
    Xlib-/Xcb-Kennung, Stack, opaque Region und Opazität. Formate, Restbytes,
    Shape, Bounds, Wiederholungsabfrage und Kartenstatus bleiben unverändert.
    Nur bewiesene opaque Shape-Schnittmengen werden über `subtract` entfernt;
    Unsicherheit und Rennen geben den bisherigen konservativen Wert zurück.
12. `sample → WindowVisibility → egui("screen_window_visibility") →
    ViewWidget::show → VisibilityTracker::report → session.set_visibility`
    verwendet dieselbe TypeId und projiziert belegte Rechtecke auf die
    tatsächliche Bildgeometrie. `shown=None` bleibt Unknown, nicht Hidden;
    bestätigte Minimizierung und nicht VIEWABLE bleiben Hidden. Widget und
    `ResumeGate::{report,acknowledge,can_present,complete,ready}` behalten
    Bildboden, Geometriegeneration und tatsächliche Präsentation als
    Voraussetzung der Eingabefreigabe. `reserve_pause/pause` und die
    Strg+Alt+Pause-Ausleitung aus Vollbild bleiben unverändert.

### C: Primärverträge und Entscheidung

Geprüft am 2026-10-26: die bereitgestellte
`primary/windows-sys-0.59-keyboard.rs:22` und die
[versionierte Microsoftquelle](https://raw.githubusercontent.com/microsoft/windows-rs/0.59.0/crates/libs/sys/src/Windows/Win32/UI/Input/KeyboardAndMouse/mod.rs)
deklarieren `IsWindowEnabled(HWND) -> BOOL` im Namespace
`UI::Input::KeyboardAndMouse`. Microsoft beschreibt den
[Enabled-Vertrag](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindowenabled)
als Abfrage der Maus-/Tastatureingabefähigkeit mit nicht null bei Enabled.
`Cargo.toml:251` aktiviert bereits das passende windows-sys-Feature.
Gewählt ist ein einziger privater Elternimport; `facts` übernimmt ihn durch
seinen bestehenden Globimport. Keine ähnlich benannte API und keine
Feature-/Manifeständerung.

Der [x11rb-0.13.2-Vertrag von value32](https://docs.rs/x11rb-protocol/0.13.2/x11rb_protocol/protocol/xproto/struct.GetPropertyReply.html#method.value32)
liefert einen an `&self` geliehenen Iterator oder `None` bei falschem Format.
Der [Rust Edition Guide zur Tail-Lebensdauer](https://doc.rust-lang.org/edition-guide/rust-2024/temporary-tail-expr-scope.html)
belegt die für Edition 2021 relevante temporäre Abschlussreihenfolge.
Gewählt ist daher das Sammeln in einem eigenen lokalen `Vec<u32>` vor dem
abschließenden `Some(values)`. Kein Reply-Leak, kein Clone, kein Standardwert
bei fehlender Property und kein Editionwechsel.

Die [Rustreferenz zu E0282](https://doc.rust-lang.org/error_codes/E0282.html)
belegt die fehlende eindeutige Typinferenz und explizite Typisierung als
Korrektur. Gewählt ist `let result: io::Result<WindowBinding> = (|| { ... })();`
am bestehenden Ort. Ein Herausziehen von `?` aus der Closure oder ein Umbau
der Servertransaktion würde deren Abschluss-/Fehlerreihenfolge berühren und
ist für die belegte Typkorrektur unnötig.

### D: Konkreter vorheriger Fix- und Erhaltungsplan

1. In Linux `property` nur den eigenen `Vec<u32>` als lokale Variable
   sammeln und danach `Some(values)` zurückgeben. Erwartetes Ergebnis:
   Iteratorabschluss vor Replyabschluss, identischer Propertywert und
   identische `None`-Pfade bei Restbytes, falschem Format oder Abfragefehler.
2. In Windows `windows.rs` den privaten Import von
   `UI::Input::KeyboardAndMouse::IsWindowEnabled` ergänzen. Erwartetes Ergebnis:
   `focused` und `WindowFacts::read` rufen genau dieselbe ursprüngliche API
   auf; beide Funktionskörper sowie `facts.rs` bleiben textgleich.
3. In X11 `bind` ausschließlich den lokalen Transaktionswert als
   `io::Result<WindowBinding>` annotieren. Erwartetes Ergebnis: gleiche
   Marker-/Prozessbindung, nach Erfolg oder Fehler derselbe geprüfte UnGrab-
   Versuch vor Fehlerweitergabe; Bindefehler behalten Vorrang, Marker werden
   erst nach erfolgreicher Bindung und Freigabe gespeichert.
4. Anschließend eigene Anti-Regressionsreview gegen `b233e533` anhand der
   obigen vollständigen Karte: unveränderte Plattform-/Host-/Fokus-/Fixture-
   Funktionskörper und Zustandsbesitzer, einzige beabsichtigte Property-
   Lebensdauerkorrektur, alle bestehenden Fallnamen/Assertions erhalten,
   Quellgrößen unter 500 Zeilen/50 KiB und keine Scopeverletzung. Nur statische
   Text-/Parsingrückmeldung; Bestätigung ausschließlich durch Main im selben
   Remote-Einstieg. Genaue Sichtbarkeitsfälle werden nach der angefragten
   konkreten Lesefreigabe ergänzt; daraus entsteht kein eigener Testlauf.

### E: Umsetzung

Der Produktdiff enthält exakt drei schmale Korrekturen:

- Linux `property`: `let values: Vec<u32> = reply.value32()?.collect();`
  gefolgt von `Some(values)`. Abfrage, Restbyteprüfung, Formatprüfung und
  Rückgabetyp bleiben erhalten. Es wird weiterhin genau ein Ergebnisvektor
  gesammelt; Iterator und `?`-Temporärwert enden jetzt am Let-Statement.
- Windows `windows.rs`: privater Import
  `windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled`.
  Der bestehende Elternimport des Kindmoduls versorgt damit beide
  Fehlerstellen. `focused`, `WindowFacts::read` und die vollständige Datei
  `windows/facts.rs` bleiben textgleich. Keine Manifeständerung.
- X11 `bind`: genau `let result: io::Result<WindowBinding> = (|| { ... })();`.
  Die Closure, der komplette Grab-/UnGrab-Code und die Markerübernahme sind
  unverändert. Es entsteht kein zusätzlicher Plattformaufruf und kein neuer
  Fehlerpfad.

### F: Eigene anschließende Anti-Regressionsreview

Nach der Umsetzung hat derselbe Autor den vollständigen Produktdiff und die
gesamte oben ermittelte Plattform-/Host-/Pin-/Sichtbarkeits-/Fixturekette
gegen `b233e5336a0ee177e19382a7adf592d297dcf202` geprüft. Der statische
Bytevergleich bestätigt für die drei bearbeiteten Quellen exakt den jeweiligen
Basisinhalt mit ausschließlich der in E genannten Ersetzung. `windows/facts.rs`,
die vier X11-Unteradapter, beide Popupfocus-Fixturedateien, die drei Hostdateien,
das Domainmodell, beide Remote-Window-Modulregistrierungen, der portable
Sichtbarkeitstyp, der gemeinsame Sichtbarkeitsadapter, Familienviewer,
ViewWidget und FamilyPins sind vollständig bytegleich zur Basis.

Eigene Erhaltungsbefunde:

1. D13 liefert weiterhin `None` bei Request-/Replyfehler, verbleibenden
   Propertybytes und unpassendem 32-Bit-Format. Ein erfolgreiches Ergebnis
   besitzt weiterhin dieselben in derselben Reihenfolge gesammelten Werte.
   Der geliehene Iterator lebt innerhalb des Replybesitzes; der zurückgegebene
   Vektor ist davon unabhängig. Kein `unwrap_or_default`, kein Leak und keine
   neue verdeckte Fehlerbehandlung.
2. D18/D19 verwenden weiterhin `IsWindowEnabled(HWND) -> BOOL` aus der
   gepinnten API. Reguläre Fokusbestätigung verlangt gleichzeitig Vordergrund,
   Enabled und nicht minimiert. Die Enabled-Fähigkeiten aus `WindowFacts`,
   die vorhandenen Menüregeln, die reale DWM-Geometrie, Prozess-/Anmeldeprüfung,
   Marker und Modalitätsgrenzen bleiben unverändert. Es wurde weder
   `IsWindowArranged` noch ein konstantes Erfolgsflag eingesetzt.
3. D20 typisiert ausschließlich den bereits vollständigen Transaktionswert.
   Nach Abschluss der Closure wird derselbe `ungrab_server`-Request samt
   `check()` ausgeführt, bevor `result?` einen Bindefehler weitergibt.
   Scheitern Bindung und Freigabe beide, behält der Bindefehler seinen bisherigen
   Vorrang. Bei erfolgreicher Bindung scheitert ein fehlerhafter UnGrab weiterhin
   vor `markers.insert`; ein Servergrab wird nicht durch einen neuen frühen
   Rücksprung verlassen.
4. Eigentümerketten, echte Fenster-/Prozesslebensdauer, Modalität,
   FocusConfirmed-Korrelation, Session-/Listenscope-/Steuerrechte und
   Entwaffnung bleiben in denselben unveränderten Objekten. Close/Resize,
   reguläre Fenster, Combo-Popups, verschachtelte Menüs, Tooltips, Maus und
   Touch behalten die bestehenden Wege und Fallaussagen aus G1/G2.
5. Unbekannte Sichtbarkeit bleibt Unknown beziehungsweise konservativ sichtbar.
   Es wird weder aus Fokusverlust Hidden abgeleitet noch ein unvollständiges
   Property als vollständige Verdeckung gewertet. Opaque-/Shape-Schnittmengen,
   Begrenzung von `subtract`, erneute Raceprüfung, konkrete X11-Kennung,
   Hidden-/Minimized-Behandlung und Waylandunsicherheit sind textgleich.
6. Die bestehende `WindowVisibility`-TypeId, Bild-/Geometriegeneration,
   tatsächliche Präsentation, Resume-Bildboden und Eingabelösung bei Hidden
   bleiben erhalten. Der Import- und Typschlussfix erzeugt weder eine zweite
   Session noch einen Ersatzrenderer oder neue Sichtbarkeitswahrheit.
7. Die beiden gelesenen Popupfocus-Fixturedateien bleiben vollständig bytegleich,
   also einschließlich aller Fallnamen, Testattribute, Assertions und
   kontrollierten Adapterfakten. Kein Fall wurde gestrichen, abgeschwächt,
   umbenannt oder lokal ausgeführt.

Statischer Diff-/Whitespacecheck ohne Befund. Die geänderten normalen
Rustdateien haben 308 Zeilen / 10.114 Bytes (Linuxsichtbarkeit),
444 / 16.993 (Windowsadapter) und 321 / 11.341 (X11-Adapter).
`windows/facts.rs` bleibt mit 292 / 10.539 unverändert. Keine Extraktion,
künstliche Verdichtung oder Größenausnahme ist nötig. Alle diese Dateien
liegen unter 500 Zeilen und 50 KiB. Keine lokalen Compiler, Tests oder
Formatter; Kompilierbarkeit und Funktionsverhalten sind dadurch noch nicht
ausgeführt bestätigt.

### G: Vorhandene Fälle, Dateien und Ergebnisgrenze

Die angefragte Datei `native/src/app/core/batches_visibility_task_tests.rs`
ist von Main ausschließlich lesend ergänzt, vollständig gelesen und statisch
als bytegleich zu `b233e533` bestätigt. Sie erhält die genaue physische
Rechteckprojektion, Unknown, Hidden, passenden Resume-ACK und frischen
Bildboden. Zusammen mit den vollständig gelesenen Popupfocus-Fixtures sind
für Main diese vorhandenen Fälle konkret belegt:

- `batches_task_b6_visibility_projects_physical_rects_and_preserves_unknown`
- `batches_task_b6_resume_gate_requires_matching_generation_and_fresh_frame`
- `batches_family_x11_combo_popup_keeps_identity_after_owner_focus_ack`
- `batches_family_x11_nested_menus_use_recursive_owner_focus`
- `batches_family_x11_modal_dialog_is_the_nearest_activation_boundary`
- `batches_family_x11_focus_rejects_foreign_unbound_recycled_owners_and_cycles`
- `batches_family_x11_host_preserves_disarming_permissions_and_lifetimes`
- `batches_family_x11_viewer_discards_uncorrelated_stale_layout_and_disarmed_input`
- `batches_family_x11_regular_windows_menus_tooltips_close_resize_and_disconnect_survive`
- `batches_family_x11_combo_atom_and_activation_capabilities_are_explicit`

Das gelesene gemeinsame Manifest führt für P6 die bestehenden Marker
`visibility_` und `resume_gate_`; die acht genannten Familienfälle stehen
zusätzlich wörtlich im Manifest. Keine neue Fixture und keine neue
Fallregistrierung durch diesen Autor. Main übernimmt die Ausführung und
Auswertung im selben Remote-Einstieg. Die kontrollierten Focusfixtures
belegen statisch ihre Erwartungen, keinen bereits ausgeführten nativen
Windows- oder X11-Servernachweis.

Gelesene Projektquellen und konkreter Umfang in dieser Fortsetzung:

- `AGENTS.md` (einschlägiger Auszug und die bereits mitgegebenen Projektregeln),
  `/root/.codex/AGENTS.md` (vollständig).
- `docs/ARCHITEKTUR.md` (Programme/Fensterfamilien, Bildreferenz),
  `docs/oberflaechen/bildschirm.md` (Betrachtereingabe und Sichtbarkeit),
  `docs/plaene/2026-10-24-batches/rv1-compilerplan.md` (zugewiesene
  Compilergrenze und zugehörige Primärquelle), dieser Bericht.
- `native/Cargo.toml` (Edition, Windows-Feature, Plattformabhängigkeiten),
  `native/Cargo.lock` (windows-sys 0.59.0 und x11rb 0.13.2),
  `native/review-task-manifest.json` (P6 und vorhandene Familienfälle).
- `native/src/app/core/family_viewer.rs` (`update`, Sichtbarkeit, Ende),
  `native/src/app/core/screen_view_widget.rs` (`show`, Sichtbarkeitsreport,
  Eingabefreigabe und Freigabe gehaltener Eingaben),
  `native/src/app/screen_modules.rs` (Sichtbarkeitsregistrierung).
- `native/src/app/core/screen_visibility.rs`,
  `native/src/app/os/linux_os/screen_visibility.rs`,
  `native/src/app/os/shared/screen_visibility.rs` und
  `native/src/app/core/batches_visibility_task_tests.rs` (vollständig).
- `native/src/remote_window/core/host.rs`,
  `native/src/remote_window/core/host_family.rs`,
  `native/src/remote_window/core/host_input.rs`,
  `native/src/remote_window/core/model.rs`,
  `native/src/remote_window/mod.rs`,
  `native/src/remote_window/os/mod.rs` (vollständig).
- `native/src/remote_window/os/windows.rs`,
  `native/src/remote_window/os/windows/facts.rs`,
  `native/src/remote_window/os/x11.rs`,
  `native/src/remote_window/os/x11/family.rs`,
  `native/src/remote_window/os/x11/focus.rs`,
  `native/src/remote_window/os/x11/input.rs`,
  `native/src/remote_window/os/x11/properties.rs`,
  `native/src/remote_window/os/x11/batches_popup_focus_task_tests.rs`,
  `native/src/remote_window/os/x11/batches_popup_focus_fixtures.rs` und
  `native/src/screen/os/shared/family_pins.rs` (vollständig).
- `/tmp/smartexplorer-final-review-user-contract.txt`,
  `/tmp/smartexplorer-fix-x11-family-scope.json`,
  `/tmp/smartexplorer-rv1-40368746815/x11-family-errors.json` (vollständig),
  `/tmp/smartexplorer-rv1-40368746815/primary/windows-sys-0.59-keyboard.rs`
  (`IsWindowEnabled`-Deklaration); die externen Primärabschnitte aus C.

Geändert in dieser Fortsetzung: ausschließlich
`native/src/app/os/linux_os/screen_visibility.rs`,
`native/src/remote_window/os/windows.rs`,
`native/src/remote_window/os/x11.rs` und dieser Bericht. Keine neue Datei;
`windows/facts.rs` benötigt durch den bestehenden Elternimport keinen Edit.

Entscheidungen: tatsächliche Enabled-API privat importieren, Besitz des
Property-Ergebnisvektors vor Replyabschluss klären und genau den lokalen
X11-Transaktionswert typisieren. Keine gelockerte Fensterbindung, keine
gelockerte konservative Sichtbarkeit, keine geänderte Serverfreigabe und keine
Manifest-/Rechte-/Rendereränderung. Die angefragten Leselücken sind geschlossen;
keine konkrete ungelöste Quellabhängigkeit bleibt in dieser eigenen Fläche.
Noch unbestätigt sind der nächste erfolgreiche Remote-Compilerlauf und das
funktionale Ergebnis der vorhandenen Fälle. Suite, Graph, Commit/Push und
terminaler Release bleiben bei Main. Kein lokaler Build, Compiler, Formatter,
Test, Server, Installations-, Gitmutations-, Graph-, Release- oder
Unteragentenaufruf. Dieser eigene statische Fixzyklus endet mit der Übergabe.
