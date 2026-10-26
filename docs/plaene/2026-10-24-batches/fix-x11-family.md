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
