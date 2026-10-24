# Fernzugriff, se Desktop und Launcher: Machbarkeit (2026-10-22)

## Auftrag

Rückmeldung vom 2026-10-22 zu 0.5.184. Die Punkte P1 bis P9 stehen unten. Der Auftrag dazu:
- die Punkte aufschreiben und noch nichts umsetzen;
- für jeden Punkt einschätzen, ob und wie er umsetzbar ist;
- keine konkrete Implementierungsplanung und keine Fehlersuche.

Die Punkte stehen als offene Zeilen in `docs/TODO.md`:
- `SCREEN-ANMELDUNG`
- `SCREEN-RDP`
- `LAUNCHER-WIDGET-MOVE`
- `DESKTOP-APP-ICONS`
- `DESKTOP-ALLE-PROGRAMME`
- `DESKTOP-PROGRAMME-STARTEN`
- `SCREEN-NUR-SICHTBAR`
- `SCREEN-HANDY-BILD`
- `SCREEN-VOLLBILD-TASTE`

Kennzeichnung:
- **Befund** = im Code oder in der Doku dieses Repositorys nachgelesen, Stand 0.5.184 (Commit `4bf05182`), mit `Datei:Zeile`.
- **Quelle** = externe Primärquelle, geprüft am 2026-10-22. Sekundärquellen sind als solche genannt.
- **Annahme** = ungeprüft, ausdrücklich so benannt.
- **Lücke** = genau benannte fehlende Angabe.

Pfadkürzel: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

## Die Punkte

| Nr. | Rückmeldung |
|---|---|
| P1 | Auf einem Linux-Host mit X11-Anmeldebildschirm (LightDM) lässt sich vom Android-Betrachter kein Passwort eintippen. Bei Windows-Hosts ist der Anmeldebildschirm im Fernbild nicht zu sehen. Wunsch: wo unterstützt, den Standardweg für Remote Desktop gehen, statt nur den Bildschirm zu streamen |
| P2 | Android-Launcher: Ein Widget wird unsichtbar, sobald das Verschiebe-Ereignis auslöst |
| P3 | se Desktop: die jeweiligen App-Symbole anzeigen |
| P4 | se Desktop „ein Desktop, alle Programme“: Was auf einem entfernten Gerät offen ist, wird ohne Streaming-Betrachter und ohne Untereinstellungen geöffnet und bedient, als wäre es lokal offen |
| P5 | Auf demselben Weg neue Programme auf allen Geräten öffnen |
| P6 | Gestreamt wird nur, was sichtbar ist: alle aktuell sichtbaren Fenster, nicht nur das fokussierte; wenn möglich auch nur der sichtbare Teil |
| P7 | Das Streaming vom Handy zum Desktop schmiert zu stark |
| P8 | Der Eindruck vom se Desktop kann noch von 0.5.183 stammen, als keine Verbindung zustande kam |
| P9 | Eine Tastenkombination, um beim Streaming das Vollbild zu verlassen, wie die übliche Host-Taste von Fernzugriffsprogrammen |
| P27 | Nachtrag 2026-10-24: „das musst du echt nich gründlicher überarbeiten. ich meine wie kann es sein dass ich via etwas wie shadowpc zum gleichen mit 20ms latenz und 7-8mbits pro sekunde verbunden bin und via se es bei 2000kbits maxed.“ |
| P28 | Nachtrag 2026-10-24: „warum läuft das streaming via relay, zumindest laut statistik. ich hätte gerne alles immer per default direkt wenn möglich und relay nur als fallback klar makiert. erst wenn möglich via selben lan, wlan netz, etc.“ Präzisiert: „also relay auch nicht direkt nach lan/wlan … das ist das kernprinzip dieses se programms, so wenig wie möglich zwischen A und B, server als router nur und relay als fallback“ |
| P29 | Nachtrag 2026-10-24: „via se sollten auch pins verarbeitet werden können“ |
| P30 | Nachtrag 2026-10-24: „selbes in überarbeitung mit anzeigequalität, das skalieren bei einpassen ist schrecklich unscharf als, obwohl das gestreamte bild mehr hergibt. ich frage mich grad auch ob das streaming auf eine bestimmte bildgröße/auflösung begrenzt ist, währe mir nicht recht“ |

## Übersicht

| Punkt | Urteil | Hauptweg | Größte Hürde |
|---|---|---|---|
| P1 Linux | umsetzbar | Ursache im eigenen Eingabeweg finden und beheben. XTest-Ereignisse erreichen einen X11-Greeter wie echte Eingaben | Ursache unbekannt (Fehlersuche nicht beauftragt). Für den Anmeldebildschirm fehlt ein vorgesehener Betrieb, denn nötig ist ein Worker mit root-Rechten am Display-Manager |
| P1 Windows | umsetzbar mit Systemdienst | Ein Dienst als LocalSystem startet einen Helfer mit SYSTEM-Token in die Konsolensitzung. Der Helfer nutzt DXGI Desktop Duplication und SendInput auf dem Eingabe-Desktop, dazu SendSAS für Strg+Alt+Entf | Ein SYSTEM-Dienst mit Netzbezug ist ein Ziel für Rechteausweitung. Die Einrichtung braucht Administratorrechte, außerdem ein eigenes Recht |
| P1 Standardweg RDP | teilweise umsetzbar | RDP durch den gerätegebundenen Share-Tunnel. Hosts: Windows Pro/Enterprise, GNOME Remote Desktop, KRdp, xrdp. Client: Systemclient oder IronRDP | Windows Home hat keinen RDP-Host. Auf Client-Windows sperrt RDP den lokalen Bildschirm. Android hat keinen RDP-Host |
| P2 | umsetzbar, klein | Die Ziehvorschau zeigt ein Abbild des Widgets | keine bekannt |
| P3 | umsetzbar | Der Host liefert je App ein Symbol über einen eigenen Abruf mit Zwischenspeicher | Unter Linux ist die Zuordnung Fenster → App heuristisch |
| P4 | umsetzbar für Windows- und X11-Hosts; unter Wayland eingeschränkt; Android-Host nur eine App zugleich | Fenster einzeln erfassen; je Fernfenster ein natives lokales Fenster (Vorbild RDP RemoteApp) | Großer Umfang (eigener Hauptstrang). Unter GNOME braucht jedes Fenster eine Zustimmung am Host. Eingaben müssen das richtige Fenster treffen |
| P5 | umsetzbar unter Windows und Linux in der Nutzersitzung; unter Android mit Bedingung | Der Host liefert seine Programmliste und startet in der Nutzersitzung; das neue Fenster öffnet sich über P4 | Android sperrt Starts aus dem Hintergrund. Ein root-Worker hat keine Nutzersitzung. Das Rechtemodell muss den Start abdecken |
| P6 | umsetzbar | Der Betrachter meldet je Strom, ob er sichtbar ist; der Host pausiert. Mit dem Kachelcodec lassen sich auch Teilbereiche senden | Verdeckung muss unter Windows und X11 selbst berechnet werden |
| P7 | umsetzbar; Ursache vorher messen | Encoder-Einstellungen, Schlüsselbild bei Verlust, Bitrate passend zur Auflösung | Ursache unbekannt (Fehlersuche nicht beauftragt) |
| P8 | Hinweis, kein Arbeitspunkt | Der Bestand von 0.5.184 ist unten unter P4 beschrieben | — |
| P9 | umsetzbar, klein | Eine feste Ausstiegskombination mit mehreren Modifikatoren, Hinweis beim Eintritt; Vollbild auch im Android-Betrachter | Die Kombination darf nicht mit Fernprogrammen kollidieren |
| P27 | umsetzbar, mittel | Relay-Deckel von 2 Mbit/s und Wachstum der Ratensteuerung überarbeiten; Startwerte und Qualitätsleiter nach Messung | Der Relay-Server verwirft über 1 MiB Warteschlange je Ziel; eine höhere Rate über das Relay braucht dort mehr Spielraum |
| P28 | umsetzbar, mittel bis groß | Für alle Datenwege: LAN/WLAN zuerst, dann die übrigen Direktwege, Relay erst als letzter, sichtbar gekennzeichneter Rückfall; der Server nur als Vermittler | se überlässt die Wegwahl heute Iroh; ob ein Direktweg zustande kommt, hängt an NAT und Firewall beider Seiten (Annahme) |
| P29 | umsetzbar, klein | `se`-Befehle zum Auflisten angebotener Geräte und zum Koppeln mit PIN; der Hintergrunddienst kann das schon | keine bekannt |
| P30 | umsetzbar, mittel | hochwertige Skalierung im Betrachter; Auflösungsgrenzen anheben und die Qualitätsleiter für Schrift auslegen | Grenzen der Encoder (H.264-Level, OpenH264 bis 3840×2160) |

## P1 Anmeldung über die Fernverbindung

### Bestand

- **Befund, Android-Betrachter:**
  - Der Schalter „Tastatur“ erscheint nur bei eingeschaltetem „Steuern“ und wenn der Host Text oder Tasten meldet (`A/ui/screen/ScreenViewBars.kt:94-95`).
  - Er öffnet die Systemtastatur über ein unsichtbares Textfeld (`A/ui/screen/ScreenViewScreen.kt:333-355`).
  - Eingaben gehen als `text`, Löschen geht als `backspace`, Senden als `enter`.
  - Hardwaretasten gehen als `key` mit HID-Code (`A/ui/screen/ScreenInputMapper.kt:173-182`).
- **Befund, X11-Host:**
  - XTest im Host-Prozess; Text über Keysyms, Tasten als evdev-Code plus 8.
  - Strg+Alt+Entf wird verworfen (`N/screen/os/linux_os/x11_input.rs:1-20`).
- **Befund, Windows-Host:**
  - `N/screen/os/windows/secure_desktop.rs:20-56` erkennt den sicheren Desktop: `OpenInputDesktop` scheitert, oder der Desktopname ist nicht `Default`.
  - Die Aufnahme pausiert dann mit `secure_desktop`, Eingaben auf dem sicheren Desktop gibt es nicht, und Strg+Alt+Entf wird nicht angeboten (`docs/oberflaechen/bildschirm.md:141`, `:180-181`).
  - Die Brettzeile `SYSTEM-Hilfsdienst` (later) führt den Systemdienst schon als Möglichkeit.

### Linux: X11-Anmeldebildschirm

- **Quelle:** XTest-Ereignisse bewirken „event propagation, passive grab activation, and so on, just as if the corresponding input device action had occurred“. Das Protokoll sieht keine Sperre für Greeter vor (https://xorg.freedesktop.org/archive/X11R7.7/doc/xextproto/xtest.html).
- **Sekundärquelle:** Die Fernbedienung des LightDM-Greeters über x11vnc mit dessen Xauthority ist verbreitet (https://bbs.archlinux.org/viewtopic.php?id=227716).
- **Folgerung:**
  - Ein Passwort lässt sich am X11-Greeter grundsätzlich eintippen; die Ursache liegt in diesem Programm.
  - Annahme zu den Kandidaten: Der Betrachter bietet die Tastatur nicht an, die Tastenzuordnung keysym → keycode passt nicht zur Belegung des Greeters, oder der Fokus fehlt.
- **Grenze:**
  - Ein Worker kann den Anmeldebildschirm nur zeigen, wenn er die Xauthority des Display-Managers lesen kann, also mit root-Rechten. Das ist kein vorgesehener Betrieb: Ein root-Worker ohne Anzeige meldet `no_display` (`docs/oberflaechen/bildschirm.md`, `N/screen/os/linux_os/session_detect.rs:1-4`).
  - Ein vorgesehener Weg bräuchte eine systemweite Komponente am X-Server des Display-Managers, entsprechend dem Windows-Systemdienst.
  - Unter Wayland-Greetern (GDM, SDDM) gibt es keine Fremdeingabe ohne Portal. Der einzige dokumentierte Weg zum GNOME-Anmeldebildschirm ist GNOME Remote Login per RDP (siehe unten).
- **Sicherheitsfolge:**
  - Wer den Anmeldebildschirm bedient, tippt Anmeldedaten und sieht sie im Bild. Dafür braucht es ein eigenes Recht, getrennt von „Bildschirm steuern“.
  - Ein root-Worker vergrößert die Angriffsfläche.

### Windows: Anmeldebildschirm, Sperrbildschirm, UAC

- **Quelle:** `IDXGIOutput1::DuplicateOutput` liefert `E_ACCESSDENIED`; „only an application that runs at LOCAL_SYSTEM can access the secure desktop“. Bei Desktopwechsel muss neu dupliziert werden (https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput).
- **Quelle:** `SendSAS` wirkt nur aus einem Dienst oder mit `uiAccess`. Zusätzlich muss die Richtlinie `SoftwareSASGeneration` Dienste zulassen (https://learn.microsoft.com/en-us/windows/win32/api/sas/nf-sas-sendsas).
- **Quelle, Vorbild RustDesk** (`src/platform/windows.rs`):
  - Ein Dienst als LocalSystem startet `--server` mit SYSTEM-Token in die aktive Konsolensitzung.
  - Dieser Prozess folgt dem Eingabe-Desktop.
  - Für Strg+Alt+Entf ruft der Dienst `SendSAS(FALSE)` und setzt `SoftwareSASGeneration` nur vorübergehend auf 1.
- **Quelle:** AnyDesk bedient UAC-Abfragen nur mit dem installierten Dienst (https://support.anydesk.com/docs/administrative-privileges-and-elevation-uac).
- **Befund:** Das Projekt richtet unter Windows schon einen Dienst mit Administratorrechten ein, den Kontodienst für Befehle unter `%ProgramFiles%\Smart Explorer\exec-accounts` (`README.md:800`). Annahme: Einrichtung, Update und Rechteprüfung dieses Dienstes sind ein Vorbild für einen Bildschirmdienst.
- **Weg:**
  1. Ein Dienst als LocalSystem startet einen Helfer mit SYSTEM-Token in die Konsolensitzung.
  2. Der Helfer folgt mit `OpenInputDesktop`/`SetThreadDesktop` dem Eingabe-Desktop.
  3. Er erfasst dort mit DXGI Desktop Duplication; WGC auf dem Winlogon-Desktop ist nicht belegt.
  4. Er schickt Eingaben mit `SendInput` und Strg+Alt+Entf über `SendSAS` aus dem Dienst.
- **Sicherheitsfolgen:**
  - Ein SYSTEM-Dienst mit Netzbezug ist ein Ziel für Rechteausweitung: Jeder Fehler in IPC, Kommandozeile, DLL-Suchpfad oder Update-Ersatz wird zu SYSTEM-Codeausführung.
  - Eingaben am Anmeldebildschirm und an UAC entsprechen faktisch Administratorrecht. Das braucht ein eigenes Recht und einen Hinweis am Host.
  - `SoftwareSASGeneration` setzt man nach dem RustDesk-Muster besser nur vorübergehend.
- **Lücken:**
  - Verhalten mit Windows Hello Enhanced Sign-in Security: nicht recherchiert.
  - WGC auf dem Winlogon-Desktop: nur durch Versuch klärbar.

### Standardweg Remote Desktop (RDP)

- **Quelle:** RDP-Hosts sind Windows 10/11 Pro und Enterprise sowie Windows Server. „Windows SKUs that aren't listed …, such as Windows 10 Home, aren't compatible“ (https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/remotepc/remote-desktop-supported-config).
- **Sekundärquelle** (Microsoft Q&A): Client-Windows erlaubt eine Sitzung, und der lokale Bildschirm wird beim RDP-Verbinden gesperrt. Annahme: Verbindet derselbe Nutzer, übernimmt RDP die bestehende Sitzung mit allen offenen Programmen.
- **Quelle:** Bei NLA übergibt CredSSP Passwort oder Smartcard-PIN an den Host (https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/e36b36f6-edf4-4df1-9905-9e53b7d7c7b7). Remote Credential Guard vermeidet das nur mit Kerberos zu AD-gebundenen Hosts, im Heimnetz also nicht (https://learn.microsoft.com/en-us/windows/security/identity-protection/remote-credential-guard).
- **Annahme** (WinApps-README): Eine Windows-Hello-PIN wird über RDP nicht angenommen; es bleibt das Kontopasswort.
- **Quelle, GNOME Remote Desktop** (https://gitlab.gnome.org/GNOME/gnome-remote-desktop/-/raw/main/docs/configuration.md und https://release.gnome.org/46/):
  - Der Modus „Remote assistance“ teilt die laufende Sitzung; das Sperren beendet die Verbindung.
  - „Remote Login“ gibt es ab GNOME 46, nur für eine Maschine, an der niemand angemeldet ist.
- **Quelle:** KRdp teilt die laufende Plasma-Sitzung; SDDM hat kein RDP (https://invent.kde.org/plasma/krdp/-/raw/master/README.md).
- **Quelle:** xrdp braucht xorgxrdp oder Xvnc (https://raw.githubusercontent.com/neutrinolabs/xrdp/devel/README.md). **Sekundärquelle** (xrdp-Wiki): Es startet eine eigene Sitzung, nicht :0, und derselbe Nutzer ist nicht gleichzeitig lokal und per xrdp grafisch angemeldet.
- **Quelle, Bausteine für den Client:**
  - IronRDP: Rust, MIT/Apache-2.0, NLA, `ironrdp-egfx` mit AVC420/AVC444, `ironrdp-rail` nur als Codec des RAIL-Kanals. Android wird nicht genannt (https://raw.githubusercontent.com/Devolutions/IronRDP/master/README.md).
  - FreeRDP: Apache-2.0, eigener Android-Client, RemoteApp über `/app`, Zertifikatsprüfung über `/cert:fingerprint` (https://raw.githubusercontent.com/FreeRDP/FreeRDP/master/README.md).
  - Systemclients: mstsc mit `.rdp`-Datei oder `/v:host:port`. Das `rdp://`-Schema gilt laut Microsoft nur für die Clients unter Android, macOS und iOS (https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/clients/remote-desktop-uri). Ob die „Windows App“ es 2026 noch annimmt, ist nicht belegt.
- **Weg:**
  1. Smart Explorer leitet den RDP-Port des Hosts durch die bestehende, gerätegebundene Share-Verbindung, gebunden an ein eigenes Recht; Port 3389 bleibt nach außen zu, NLA bleibt an.
  2. Smart Explorer überträgt den Fingerabdruck des RDP-Zertifikats über den authentifizierten Peer-Kanal und nagelt ihn fest.
  3. Auf dem Gerät des Nutzers startet zuerst der Systemclient gegen den lokalen Tunnel-Port. Später kann ein eingebetteter Client folgen: IronRDP passt zu Rust und zur Lizenz, die Android-Tauglichkeit ist offen.
- **Grenzen:**
  - Windows Home hat keinen RDP-Host; dort bleibt die eigene Bildschirmverbindung mit dem Systemdienst.
  - Auf Client-Windows sperrt RDP den lokalen Bildschirm. Für „zusehen, während jemand am Gerät arbeitet“ taugt RDP daher nicht; beide Wege bleiben nebeneinander.
  - Unter GNOME gibt es den Anmeldebildschirm nur ohne angemeldeten Nutzer; KDE hat keinen.
  - Android ist kein RDP-Host.
- **Sicherheitsfolgen:**
  - Das Kontopasswort landet auf dem Host, der also vertrauenswürdig sein muss.
  - Zugangsdaten gehören weder in `.rdp`-Dateien, URIs noch Kommandozeilen.
  - Ein eingebetteter RDP-Parser verarbeitet Netzdaten im Programmprozess; Fuzzing-Stand und Prozessgrenze sind zu prüfen.
  - `fAllowUnlistedRemotePrograms=1` (RemoteApp auf Client-Windows) erlaubt jedem RDP-berechtigten Nutzer beliebige Programme.

### Android als Host

- **Quelle:** Jede Sitzung braucht eine Zustimmung am Gerät. Ab Android 14 lässt sich eine einzelne App freigeben. Ab Android 15 QPR1 „screen projection automatically stops when the device screen is locked“ (https://developer.android.com/media/grow/media-projection).
- **Quelle:** Fenster mit `FLAG_SECURE` bleiben leer; Fernfreigabe-Apps werden ausdrücklich als Missbrauchsweg genannt (https://developer.android.com/security/fraud-prevention/activities).
- **Folgerung:** Den Android-Sperrbildschirm aus der Ferne zu sehen und zu entsperren ist für eine normale App nicht machbar. Der Host meldet den Sperrzustand besser als Sitzungsende.
- **Lücke:** Verhalten unter Android 14 und unter 15 vor QPR1 beim Sperren.

### Urteil P1

- **Linux X11:** umsetzbar als Fehlerbehebung. Den Anmeldebildschirm vor einer Anmeldung zu bedienen braucht eine neue systemweite Komponente.
- **Windows:** umsetzbar mit Systemdienst; sicherheitskritisch.
- **RDP:** als zweiter Weg für Pro/Enterprise-Windows und GNOME/KDE/xrdp-Hosts umsetzbar.
- **Android:** Sperrbildschirm nicht umsetzbar.
- **Oberflächen:**
  - Android-Betrachter, Desktop-Betrachter, `se screen input`;
  - Rechte (`se share grants access`, Desktop-GUI, Android-GUI, MCP über `se`);
  - Windows-Installer und Dienst;
  - `docs/oberflaechen/bildschirm.md`;
  - für RDP zusätzlich Tunnel im Worker, Start des Clients aus CLI, Desktop-GUI und Android-GUI.

## P2 Android-Launcher: Widget verschwindet beim Verschieben

- **Befund:**
  - Ein Langdruck startet das Ziehen (`A/launcher/widgets/WidgetView.kt:98-99`, `:176-191`).
  - Das Original wird während des Ziehens mit `alpha(0f)` ausgeblendet (`A/launcher/ui/CellGrid.kt:336-354`).
  - Erst nach Überschreiten des Ziehwegs erscheint als Vorschau eine Platte mit `alpha(0.6f)`, die ein Widget nur als Fläche zeigt (`A/launcher/ui/DragLayer.kt:305-323`).
- **Annahme** (nicht am Gerät geprüft, keine Fehlersuche): Das Ausblenden zusammen mit der inhaltslosen Vorschau ist das, was als „unsichtbar“ wahrgenommen wird.
- **Weg:** Beim Ziehbeginn ein Abbild der `AppWidgetHostView` als Vorschau zeigen, oder das Original bis zum Ablegen halbtransparent stehen lassen. Annahme: Gängige Launcher zeigen ein solches Abbild; nicht recherchiert.
- **Abgrenzung:** `LAUNCHER-WIDGET-DRAG` (later) betrifft das Ziehen aus der Widget-Auswahl, nicht das Verschieben.
- **Urteil:** umsetzbar, klein.
- **Oberflächen:** Android-Launcher.

## P3 App-Symbole

- **Befund:**
  - Fenster- und App-Listen der Hosts haben kein Symbolfeld: `WindowEntry` und `AppEntry` (`N/screen/core/window_list.rs:95`, `:110`), Android `ScreenAppEntry(packageName, label)` (`A/api/ScreenApi.kt:41`).
  - Die Kacheln der Fläche zeigen Emoji (`N/app/core/desktop_surface_tile_ui.rs:86-135`).
  - Der Android-Launcher zeigt lokale App-Symbole (`A/launcher/ui/CellGrid.kt:215-256`).
- **Quellen:**
  - Windows: `IShellItemImageFactory::GetImage` liefert das Symbol eines Shell-Elements (https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ishellitemimagefactory).
  - Linux: Die Icon Theme Specification sucht in Theme, Elternthemes, `hicolor` und Pixmaps (https://specifications.freedesktop.org/icon-theme/latest/).
  - Wayland: `app_id` soll dem Namen der `.desktop`-Datei entsprechen; das ist nur Konvention (https://wayland.app/protocols/xdg-shell).
- **Weg:**
  - Der Host liefert je App ein Symbol als PNG in festen Größen.
  - Das geschieht über einen eigenen Abruf, damit die Fensterliste unter dem 1-MiB-Rahmen bleibt (`SCREEN-FENSTERLISTE`).
  - Betrachter und Fläche speichern die Symbole nach Inhalts-Hash zwischen.
- **Plattformen:**
  - Windows: über Programmdatei oder AppUserModelID. Annahme: Store-Apps über `shell:AppsFolder`.
  - Linux X11: `.desktop` über `WM_CLASS`/`StartupWMClass`, sonst `_NET_WM_ICON` (Annahme).
  - Wayland: `app_id` → `.desktop` → Icon-Theme.
  - Android: PackageManager (Annahme).
- **Urteil:** umsetzbar; die Zuordnung Fenster → Symbol bleibt unter Linux heuristisch.
- **Oberflächen:**
  - se Desktop-Fläche;
  - Quellwahl im Desktop- und im Android-Betrachter;
  - `se screen windows --json` (Verweis auf das Symbol);
  - MCP über `se`;
  - Host-Backends unter Windows, Linux und Android.

## P4 „Ein Desktop, alle Programme“

### Ursprung und Bestand (zugleich P8)

- **Befund, Ursprung:** Der ursprüngliche Auftrag E5 im Plan `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md:19` lautet „operate on any device … see what they are running directly, be it with a window video stream or in other ways“. Umgesetzt wurde er als Wechsel zum Bildschirm des Geräts (`strang-desktop.md`, Kriterium K2). Die Rückmeldung präzisiert das Ziel: die Programme selbst.
- **Befund, se Desktop in 0.5.184:**
  - Die Fläche zeigt Systemelemente (nur Windows), Desktop-Dateien, Geräte, Räume, Tresore und „Alle Bildschirme“, aber keine Fenster oder Programme (`N/desktop/core/items.rs:112-151`).
  - Ein Klick auf ein Gerät startet den Betrachter als eigenen Prozess (`N/desktop/core/screen_target.rs:1-3`).
  - Der Mangel besteht also unabhängig davon, ob der Eindruck von 0.5.183 stammt.
- **Befund, vorhandene Bausteine:**
  - Fensterlisten:
    - Windows: `EnumWindows` mit der Alt+Tab-Regel (`N/screen/os/windows/window_list.rs:20-46`).
    - X11: EWMH.
    - Wayland: ext/wlr-Toplevel; KDE über das Desktop-Programm; GNOME nur Apps.
    - Android: über die Bedienungshilfe oder die Nutzungsstatistik (`docs/oberflaechen/bildschirm.md:120`, `A/screen/RunningApps.kt:8-28`).
  - Fensterquellen `window:<id>`:
    - Windows: über WGC (`N/screen/os/windows/capture_wgc.rs:67-74`).
    - X11: über die XID.
    - Wayland: nur über den Systemdialog.
    - Android: keine.
- **Doku-Abweichung (Befund):** `docs/oberflaechen/desktop-launcher.md:35` sagt, die Ansicht der Fläche „folgt mit DSK-M4 (U30)“, der Code zeichnet aber Kacheln (`N/app/core/desktop_surface_tile_ui.rs:86-135`). Das ist abzugleichen.

### Vorbilder

- **Quelle, RDP RemoteApp** ([MS-RDPERP], https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp/485e6f6d-2401-4a9c-9330-46454f0c5aba):
  - Je Server-Fenster ein lokales Fenster.
  - Fensterbeschreibungen kommen als Window Orders.
  - Eingaben, Programmstart, Verschieben und Größenänderung laufen über einen Kanal.
- **Quelle, xpra seamless:** Jede gewählte Anwendung erscheint als eigenes lokales Fenster; Seamless-Server gibt es nur für X11 (https://github.com/Xpra-org/xpra/blob/master/docs/Usage/Seamless.md).
- **Phone Link „Apps“ (laut Suchergebnis der Microsoft-Supportseite, Seite selbst nicht abrufbar; Samsung: https://www.samsung.com/us/support/answer/ANS10001340/):** nur auf ausgewählten Geräten von Samsung, HONOR, OPPO, ASUS, vivo und Xiaomi mit vorinstalliertem „Link to Windows“, also eine Herstellerintegration ohne offene Schnittstelle.
- **Annahme** (nicht recherchiert): VirtualBox Seamless, Parallels Coherence und VMware Unity arbeiten mit einem Gast-Agenten, der Fensterbereiche meldet.

### Weg

Auf Machbarkeitsebene:
- **Host:**
  - Fensterliste mit Geometrie, Z-Ordnung, Zustand und Änderungsereignissen;
  - Erfassung je Fenster;
  - Eingaben gehen an das Zielfenster, das vorher aktiviert wird.
- **Verbindung:** je sichtbarem Fernfenster ein Bildstrom (P6).
- **Lokale Seite:**
  - ein Prozess mit je einem nativen Fenster pro Fernfenster, mit Taskleisteneintrag, Symbol aus P3 und dem Titel „Programm – Gerät“;
  - geöffnet direkt aus der se Desktop-Fläche oder der Programmliste, ohne Werkzeugleiste.
- **Bildübertragung bleibt nötig.** Annahme zur Rückmeldung: „ohne Streaming“ meint „ohne Betrachter“, denn der Zusatz verlangt, nur Sichtbares zu streamen. Für schon offene Fenster gibt es keinen Weg ohne Bildübertragung: X11-Forwarding, Waypipe und xpra wirken nur für Programme, die darunter gestartet wurden (Annahme für Waypipe und X11-Forwarding).
- **RemoteApp ist nur Vorbild, nicht Mechanismus:** Auf Client-Windows läuft es nur über inoffizielle Registry-Schalter und startet neue Programminstanzen statt der offenen (Annahme, siehe P1).
- **Gleichzeitige Nutzung (Annahme):** Wer am Host sitzt, sieht die Fernbedienung; Fenster kommen nach vorn. Bei gleichzeitiger Nutzung konkurrieren Fokus und Maus.

### Plattformen

- **Windows-Host:**
  - WGC erfasst einzelne Fenster ab Windows 10 1803 (https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureitem).
  - Annahme aus inoffiziellen Quellen: Verdeckte Fenster liefern weiter Bilder, minimierte nicht; das ist zu testen.
  - Der gelbe Rahmen am Host lässt sich erst ab Build 20348 abschalten, und nur mit der Paket-Fähigkeit `graphicsCaptureWithoutBorder`. Für ein unverpacktes Programm ist das offen (https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired).
  - `SetForegroundWindow` ist eingeschränkt, erlaubt ist es unter anderem dem Prozess, der das letzte Eingabeereignis erhielt (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow).
  - Urteil: machbar.
- **Linux X11-Host:**
  - Annahme: Composite liefert den Inhalt verdeckter Fenster; Aktivierung über `_NET_ACTIVE_WINDOW`, Eingabe über XTest.
  - Urteil: machbar.
- **Linux Wayland-Host:**
  - Fenster lassen sich nur über das ScreenCast-Portal mit Zustimmung am Host erfassen; `restore_token` erlaubt die Wiederholung (https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).
  - `ext-image-capture-source-v1` und `ext-image-copy-capture-v1` mit Fensterquelle bieten Sway, COSMIC, Labwc, river, Jay, Mir, phoc und Treeland; KWin, Mutter, Hyprland, niri, Wayfire und Weston bieten sie nicht (https://wayland.app/protocols/ext-image-capture-source-v1).
  - KWin `zkde_screencast_unstable_v1` `stream_window` ist nur für berechtigte Programme (Sunshine-Doku).
  - Wer Mutter `RecordWindow` aufrufen darf, ist nicht belegt.
  - Eingaben über das RemoteDesktop-Portal gelten im logischen Koordinatenraum des Streams (https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html). Bei einem Fenster-Stream also vermutlich fensterbezogen (Annahme, zu testen).
  - Unter wlroots gibt es heute keine Eingabe (`docs/oberflaechen/bildschirm.md:135`).
  - Urteil: GNOME nur mit Zustimmung je Fenster; KDE mit Sonderrecht; wlroots und COSMIC ohne Eingabe.
- **Android-Host:**
  - Ab Android 14 lässt sich eine einzelne App freigeben, mit Wahl am Gerät je Sitzung.
  - Fremde Apps auf ein eigenes VirtualDisplay zu starten verlangt Systemrechte (laut Suchergebnis zu https://source.android.com/docs/core/display/multi_display/activity-launch, Seite nicht selbst gelesen).
  - Also immer nur die gewählte App als ein Fenster. Mehrere Apps gleichzeitig gingen nur über Shell-Rechte (Shizuku nach dem scrcpy-Muster, Brettzeile `SYSTEM-Hilfsdienst`; Annahme, nicht recherchiert).
- **Lokale Seite:**
  - Windows und X11: native Fenster mit Position.
  - Wayland: Der Compositor platziert, die Position eines Fernfensters ist nicht spiegelbar; winit `set_outer_position` meldet dort „Unsupported“ (https://docs.rs/winit/latest/winit/window/struct.Window.html).
  - Android-Betrachter, Annahme: je Fernprogramm eine eigene Aufgabe in der Übersicht oder ein Vollbild je Programm.

### Urteil P4

Umsetzbar für Windows- und X11-Hosts. Unter Wayland eingeschränkt, der Android-Host zeigt eine App zugleich. Großer Umfang: ein eigener Hauptstrang mit eigener Planung.

Mit wachsendem Bedarf verbunden sind „wie lokal“, Zwischenablage (`SCREEN-ZWISCHENABLAGE`) und Ton (`SCREEN-TON`).

**Oberflächen:**
- Host-Backends unter Windows, Linux und Android;
- Protokoll;
- neuer Fensterbetrachter am Desktop, Android-Betrachter;
- se Desktop-Fläche und Launcher;
- `se screen windows` und Öffnen aus der CLI;
- MCP: Listen ja, Live-Bild nein (wie heute);
- Doku.

## P5 Programme auf allen Geräten starten

- **Befund:**
  - Weder se Desktop noch die Bildschirmverbindung starten Programme auf fremden Geräten.
  - `se exec` startet Befehle mit Befehlsrecht. Unter Windows laufen Befehle unter einem anderen Konto in Sitzung 0 ohne Fenster (`README.md:806`).
  - Lücke: Ob ein Programm des eigenen Kontos sichtbar in der Sitzung erscheint, ist hier nicht geprüft.
- **Quelle, Android:**
  - Seit Android 10 sind Aktivitätsstarts aus dem Hintergrund gesperrt (https://developer.android.com/guide/components/activities/background-starts).
  - Ausnahmen sind unter anderem ein sichtbares Fenster, `SYSTEM_ALERT_WINDOW` vom Nutzer erteilt und „the launch is initiated by the device's launcher app“.
  - Eine Ausnahme für Bedienungshilfen nennt die Seite nicht.
  - Programmlisten gehen mit `<queries>` für `MAIN`/`LAUNCHER` (https://developer.android.com/training/package-visibility).
- **Quelle, Linux:** Apps laufen als Einheiten `app-<launcher>-<ID>` in `app.slice` (https://systemd.io/DESKTOP_ENVIRONMENTS/).
- **Lücke, Windows:** Der Start über die AppUserModelID ist nicht belegt; Annahme: `IApplicationActivationManager`.
- **Weg:**
  1. Der Host liefert seine Programmliste mit Symbolen (P3): Windows-Startmenü einschließlich Store-Apps, Linux-`.desktop`, Android-Launcher-Apps.
  2. Der Start erfolgt in der interaktiven Sitzung.
  3. Das neue Fenster öffnet sich lokal über P4.
- **Plattformen:**
  - Windows: Der Worker läuft in der Nutzersitzung, also machbar.
  - Linux mit Nutzer-Worker: machbar. Ein root-Worker hat keine Nutzersitzung (Annahme), also nicht vorgesehen.
  - Android: nur mit „Über anderen Apps einblenden“ oder wenn der se Launcher Standard-Launcher ist. Annahme: Die Launcher-Ausnahme greift dann auch für einen Start auf Fernauftrag.
- **Recht:** Ein Programmstart ist Ausführung auf dem Host. Er braucht eine Bindung an das Recht „Befehle“ oder ein eigenes Recht (Entscheidung E4).
- **Urteil:** umsetzbar unter Windows und Linux in der Nutzersitzung; Android mit Bedingung. Baut auf P3 und P4 auf.
- **Oberflächen:**
  - se Desktop-Fläche und Launcher;
  - CLI zum Auflisten und Starten mit `--json`;
  - MCP über `se`;
  - Rechte auf allen Rechte-Oberflächen;
  - Host-Backends.

## P6 Nur Sichtbares streamen

- **Befund, Host:**
  - Der Kachelcodec sendet nur geänderte Kacheln (`N/screen/core/tiles_codec.rs:4-5`); unveränderte Bilder erzeugen keine Daten (`N/screen/core/host_capture.rs:37`).
  - Der Host pausiert bei minimiertem Fenster, sicherem Desktop, gesperrtem Host und inaktiver Sitzung (`N/screen/core/session_state.rs:62-65`).
- **Befund, Betrachter:**
  - Er pausiert nicht, wenn sein Fenster verdeckt oder minimiert ist. Bei Fokusverlust löst er nur Eingaben (`N/app/core/screen_view_widget.rs:293-297`).
  - Die Gruppenansicht aktualisiert alle Kacheln weiter (`N/app/core/screen_tile_grid.rs:36`).
- **Quellen:**
  - Windows hat keine Verdeckungs-API für Win32; Chromium berechnet Verdeckung selbst (laut Suchergebnis zu https://chromium.googlesource.com/chromium/src/+/master/docs/windows_native_window_occlusion_tracking.md). `DWMWA_CLOAKED` meldet versteckte Fenster, zum Beispiel auf anderen virtuellen Desktops (https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute).
  - xdg-shell `suspended` (Version 6) gilt nur für eigene Fenster (https://wayland.app/protocols/xdg-shell).
- **Weg:**
  - Der Betrachter meldet je Strom „sichtbar“, „verdeckt“ oder „minimiert“, und auf Wunsch den sichtbaren Bereich.
  - Der Host stoppt Erfassung und Kodierung unsichtbarer Ströme.
  - Der Kachelcodec kodiert nur sichtbare Kacheln. Bei H.264 ginge ein Ausschnitt oder ein Neuaufsetzen der Größe (Annahme).
- **Plattformen, lokale Seite:**
  - Windows: Minimiert und versteckt direkt; Verdeckung nach dem Chromium-Muster berechnen.
  - X11: Verdeckung aus der Stapelreihenfolge berechnen (Annahme).
  - Wayland: eigene Fenster über `suspended` bzw. gedrosselte Frame-Callbacks (Annahme).
  - Android: Lebenszyklus der Activity (Annahme).
- **Urteil:** umsetzbar; es hilft auch dem heutigen Betrachter und der Gruppenansicht.
- **Oberflächen:** Desktop-Betrachter, Gruppenansicht, Android-Betrachter, Protokoll und Host.

## P7 Bild vom Handy schmiert

- **Befund, Android-Host** (`A/screen/ScreenEncoder.kt:55-70`, `:153`):
  - H.264 (AVC) mit Profil high und I-Bild-Abstand 10 s;
  - `KEY_BITRATE_MODE` nicht gesetzt, es gilt der Codec-Standard;
  - nach 500 ms Stillstand wird das letzte Bild wiederholt.
- **Befund, Ratensteuerung** (`N/screen/core/rate_control.rs:1-25`):
  - Start direkt mit 6 Mbit/s, über Relay mit 2 Mbit/s;
  - Regler nach Verzögerung und Verlust;
  - Qualitätsleiter: erst Bildrate bis 10/s, dann Auflösung mal 0,75.
- **Befund, Relay:**
  - Es verwirft Pakete, wenn die Warteschlange eines Ziels voll ist (512 Pakete oder 1 MiB, `README.md:252ff.`).
  - Schlüsselbilder über das Relay werden auf 3/4 MiB begrenzt (`N/screen/core/host_mode.rs:109-123`).
- **Quellen:**
  - MediaFormat kennt `KEY_BITRATE_MODE` (CQ/VBR/CBR), `KEY_PRIORITY` (0 = Echtzeit), `KEY_MAX_B_FRAMES` (Standard 0), `KEY_INTRA_REFRESH_PERIOD` (nur mit `FEATURE_IntraRefresh`) und `KEY_PREPEND_HEADER_TO_SYNC_FRAMES` (https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/master/media/java/android/media/MediaFormat.java).
  - scrcpy nutzt standardmäßig 8 Mbit/s (https://github.com/Genymobile/scrcpy/blob/master/doc/video.md).
  - Sunshine nutzt 20 % Vorwärtsfehlerkorrektur (https://github.com/LizardByte/Sunshine/blob/master/docs/configuration.md).
- **Annahmen** (nicht geprüft, keine Fehlersuche):
  - (a) Die Bitrate ist zu niedrig für die Auflösung eines Handybildschirms in Bewegung.
  - (b) Verworfene Bilder ohne sofortiges Schlüsselbild wirken als Bildfehler bis zum nächsten I-Bild nach, also bis zu 10 s.
  - (c) Der Standard-Bitratenmodus des Codecs passt nicht.
- **Weg:**
  1. Zuerst messen: `se screen bench` mit Android-Host, direkt und über Relay.
  2. Danach je nach Ergebnis: Bitratenmodus und Priorität bewusst setzen; Schlüsselbild auf Anforderung bei erkanntem Verlust; kürzerer I-Bild-Abstand oder Intra-Refresh, wo verfügbar; Startbitrate und Auflösung passend zum Betrachter.
- **Urteil:** umsetzbar; die Ursache vor der Umsetzung messen.
- **Oberflächen:** Android-Host, Ratensteuerung, `se screen bench`.

## P9 Vollbild verlassen

- **Befund, Desktop-Betrachter:**
  - F11 schaltet das Vollbild um und wird nie an den Host gesendet (`N/app/core/screen_input_keys.rs:10-25`, `N/app/core/screen_viewer_app.rs:293-296`).
  - Escape verlässt das Vollbild nur, wenn „Steuern“ aus ist; sonst geht Escape an den Host (`N/app/core/screen_viewer_app.rs:258-259`).
  - Dazu gibt es den Knopf „Vollbild“ und die eigenen Tasten Strg+Umschalt+S (Statistik) und Strg+Umschalt+K (Steuern).
- **Befund, Android-Betrachter:** kein Vollbildmodus; die Leisten schalten durch Antippen um (`A/ui/screen/ScreenViewScreen.kt:267`).
- **Quellen:**
  - mstsc: Strg+Alt+Pause wechselt zwischen Vollbild und Fenster (Microsoft-Seite zu Terminal-Services-Tastenkürzeln, laut Suchergebnis).
  - Parsec: Strg+Umschalt+W für den Fenstermodus (laut Suchergebnis zu https://support.parsec.app/hc/en-us/articles/32381778420372-Configure-Hotkeys).
  - Moonlight: Strg+Alt+Umschalt+K für den Tastatur-Grab (laut Suchergebnis zu https://flathub.org/pl/apps/com.moonlight_stream.Moonlight).
  - Wayland `keyboard-shortcuts-inhibit`: Der Compositor darf eine Notfallkombination behalten (https://wayland.app/protocols/keyboard-shortcuts-inhibit-unstable-v1).
  - Annahme: VirtualBox nutzt die Host-Taste (rechte Strg) plus F.
- **Weg:**
  - Eine feste Kombination mit mehreren Modifikatoren, die nie an den Host geht. Vorschlag: Strg+Alt+Pause wie mstsc, dazu eine Ausweichkombination für Tastaturen ohne Pause-Taste. F11 bleibt.
  - Ein Hinweis beim Eintritt ins Vollbild.
  - Android bekommt einen Vollbildmodus mit Ausstieg per Kombination an einer Hardwaretastatur und per Geste.
- **Urteil:** umsetzbar, klein.
- **Oberflächen:** Desktop-Betrachter, Android-Betrachter, Tastenübersicht in `docs/oberflaechen/bildschirm.md`, `se screen view --fullscreen` (Doku).

## Nachtrag 2026-10-24: Streaming-Rückmeldung (P27 bis P30)

Stand: Nur dokumentiert. Der Nutzer schrieb dazu: „nur dass es klar ist, noch immer nicht starten mit der musetzung“. Die Befunde sind am Code von `be7a23d0` nachgelesen. Die Ursache im Einzelfall ist nicht gemessen, weil die Protokolle der beteiligten Geräte nicht vorliegen.

### P27 Bitrate über das Relay

- **Befund, Startwerte:** direkt 6 Mbit/s, über das Relay 2 Mbit/s (`N/screen/core/rate_control.rs:24-25`).
- **Befund, Deckel:** Wechselt der Pfad von direkt auf Relay, setzt `set_path` beide Ziele auf höchstens 2 Mbit/s (`N/screen/core/rate_control.rs:238`). Danach wächst das Ziel nur über den Regler: um 8 % je Sekunde ohne Verzögerungsanstieg, um 5 % bei geringem Verlust (`:37`, `:40`).
- **Befund, Warteschlange:** Auf dem Relay-Pfad dürfen höchstens 768 KiB unbestätigt unterwegs sein (`N/screen/core/pacing.rs:17`, `:32`). Grund ist, dass der Relay-Server oberhalb von 1 MiB je Ziel verwirft (`:14`).
- **Folgerung (eigene):** 2000 kbit/s ist genau der Relay-Startwert. Die Statistik zeigt also, dass der Regler über das Relay nicht über den Start hinauskam. Ob Verzögerung, Verlust oder die Warteschlangengrenze das verhinderte, ist ohne Messung offen.
- **Vergleich:** Die Angaben zu ShadowPC (20 ms, 7–8 Mbit/s) stammen vom Nutzer; Herstellerangaben sind nicht recherchiert.
- **Weg:** zuerst P28, denn über einen Direktweg gilt der Deckel nicht. Danach Relay-Grenzen und Regler überarbeiten, also Startwert, Wachstum und Warteschlange im Relay-Server. Messen mit `se screen bench` direkt und über das Relay.

### P28 Alles direkt zwischen den Geräten, Relay nur als Rückfall

**Präzisierung des Nutzers (2026-10-24):** „also relay auch nicht direkt nach lan/wlan, wir tauschen ja die direkten adressen aus gutem grund aus, genau so wie file übertragung ursprünglich war und hoffentlich noch ist, auch alles andere nach möglichkeit direkt zwischen den geräten. das ist das kernprinzip dieses se programms, so wenig wie möglich zwischen A und B, server als router nur und relay als fallback“. Das Kernprinzip steht seitdem auch in `docs/ARCHITEKTUR.md`, Abschnitt „Zweck“.

- **Befund, eine Verbindung für alles:** Dateien, Befehle, LAN-Verbindung und Bildschirm laufen über denselben Iroh-Endpunkt eines Geräts, mit getrennten ALPNs (`N/share/core/node.rs:156-161`). Was für den Bildschirm gilt, gilt also auch für Dateiübertragung und Laufwerke.
- **Befund, Wählen der Gegenstelle:** `endpoint_addr` übergibt Iroh in einem Aufruf alle Direktkandidaten aus der Anwesenheit der Gegenstelle zusammen mit den Relay-Adressen (`N/share/core/session.rs:307-340`). Das sind LAN-Adressen und weitere Adressen.
  - se legt dabei keine eigene Reihenfolge fest.
  - se wartet nicht auf die Direktwege, bevor es das Relay nutzt.
  - se hat keine Regel, die das Relay als letzten Weg erzwingt.
  - Welcher Weg ausgewählt wird, entscheidet Iroh.
- **Annahme, nicht am Hersteller belegt:** Iroh baut die Verbindung zuerst über das Relay auf und wechselt auf einen Direktweg, sobald das Durchstoßen von NAT gelingt. Lücke: das tatsächliche Pfadverhalten der verwendeten Iroh-Version aus deren Dokumentation.
- **Befund, Anzeige:** `transport_label` und der Bildstrom lesen nur den gerade ausgewählten Pfad (`N/share/core/session.rs:343-350`, `N/screen/core/gop_stream.rs:43-56`). Ohne ausgewählten Pfad zeigt die Bezeichnung „unknown“, der Bildstrom rechnet dann mit dem Relay-Budget.
- **Befund, LAN:** Für die LAN-Verbindung nutzt se einen Pfad nur, wenn genau einer ausgewählt ist, dieser über IP läuft und kein Relay ist (`N/share/os/shared/lan_link_transport.rs:50-58`). Die Anwesenheit im LAN über mDNS gibt es schon (`se share lan`).
- **Befund am Gerät des Nutzers:** Der neu gekoppelte Cloud-PC meldete als Direktkandidaten nur eine private Adresse. Gerätedaten stehen nicht hier.
  - Annahme: Ohne öffentliche Adresse braucht ein Direktweg das Durchstoßen von NAT, und das kam nicht zustande.
  - Lücke: ob und wann Iroh den Direktweg dort versucht hat.
- **Weg (Reihenfolge nach der Präzisierung):**
  1. Gleiches LAN oder WLAN, mit den ausgetauschten LAN-Adressen und der mDNS-Anwesenheit.
  2. Die übrigen Direktwege: öffentliche Adressen, Durchstoßen von NAT, IPv6.
  3. Erst wenn kein Direktweg zustande kommt, das Relay als Rückfall.
  - Der Server bleibt dabei Vermittler: Er tauscht Anwesenheit und Adressen aus und trägt keine Nutzdaten, solange ein Direktweg besteht.
  - Ein Wechsel aufs Relay wird gemeldet; danach versucht die Verbindung weiter, auf einen Direktweg zurückzukommen.
  - Die Anzeige „über Relay (Rückfall)“ steht deutlich im Betrachter, in der Gruppenansicht, auf der Geräteseite, im Übertragungsfenster und in `se share status`, nicht nur in der Statistik.
- **Gilt für alle Datenwege:** Dateiübertragung, Laufwerke, Sync über Direktfreigaben, Befehle und Terminal, Bildschirm.
- **Oberflächen:** Desktop-App (Betrachter, Gruppenansicht, Geräteseite, Übertragungen), Android-App (Betrachter, Geräte), `se share status`, `se screen`, MCP über `se`.
- **Lücke für die Planung:** Messung, wie oft und wie lange heutige Verbindungen über das Relay laufen, je Gerätepaar und Netz.

### P29 PIN-Kopplung im Terminal

- **Befund:** `se share discoverable` macht nur das eigene Gerät auffindbar (`N/cli/share/discoverable.rs`). Ein Befehl zum Auflisten fremder Angebote und zum Koppeln mit PIN fehlt.
- **Befund:** Der Hintergrunddienst nimmt beides schon an: `ListDiscoveries` und `StartDiscoveryExchange` mit PIN (`N/share/core/discovery_signal_types.rs:157-171`). Die Desktop-App nutzt das (`N/share/os/shared/discovery_events.rs:212`).
- **Beleg im Betrieb:** Am 2026-10-24 ließ sich ein angebotenes Gerät auf diesem Weg mit PIN koppeln. Die Anfrage ging direkt an den Hintergrunddienst, weil `se` keinen Befehl dafür hat.
- **Weg:**
  - `se share discover list` und `se share discover pair <Eintrag>` mit `--pin-stdin` oder `--pin-prompt`, wie bei `discoverable`.
  - Dazu `--json` und MCP über `se`.
- **Urteil:** umsetzbar, klein.

### P30 Anzeigequalität und Auflösungsgrenze

- **Befund, Skalierung im Betrachter:** Beide Wege tasten das Bild mit einem einzelnen linearen Filter ab, ohne Mipmaps.
  - CPU-Weg: `TextureOptions::LINEAR` (`N/app/core/screen_texture_cpu.rs:33`).
  - GPU-Weg: ein Sampler mit demselben Filter für Vergrößern und Verkleinern, `mip_level_count: 1` (`N/app/core/screen_wgpu_yuv.rs:136`, `:233`).
  - Folgerung (eigene): Beim Verkleinern auf die Fenstergröße fallen Bildpunkte aus, sodass Schrift ausfranst. Beim Vergrößern verschwimmt sie.
- **Annahme:** H.264 mit 4:2:0-Farbunterabtastung macht farbige Schrift zusätzlich unscharf. Nicht geprüft.
- **Befund, feste Grenzen:**
  - Das Protokoll erlaubt höchstens 4096 Bildpunkte je Seite und 4096 × 2304 Bildpunkte insgesamt (`N/screen/core/pixel_convert.rs:14-26`). Größere Quellen werden verkleinert (`:117`).
  - Der Software-Encoder OpenH264 kodiert höchstens 3840 × 2160 (`N/screen/os/shared/codec_inventory.rs:24`).
- **Befund, Grenze nach Bitrate:** Die Qualitätsleiter rechnet mit 0,05 Bit je Bildpunkt (`N/screen/core/rate_control.rs:330`, `:355`). Sie senkt zuerst die Bildrate bis 10/s und dann die Größe in Schritten von × 0,75.
  - Eigene Rechnung für 2 Mbit/s: 1920 × 1080 bleibt bei etwa 19 Bildern/s, 2560 × 1440 bei etwa 10 Bildern/s.
  - 3840 × 2160 fällt auf 2160 × 1215 bei 10 Bildern/s.
  - Bei niedriger Rate ist der Strom also kleiner als der Bildschirm.
- **Weg:**
  - Hochwertige Skalierung: Verkleinern mit Flächenfilter oder Mipmaps, Vergrößern bikubisch oder mit Lanczos, dazu eine Einstellung „pixelgenau“.
  - Die Grenze 4096 dort anheben, wo der Encoder mehr kann. Lücke: die Level-Grenzen je Encoder.
  - Für Desktop-Inhalt mit Schrift lieber die Bildrate als die Auflösung senken.
  - 4:4:4, wo Encoder und Decoder es können. Lücke.
- **Oberflächen:** Desktop-Betrachter (beide Wege), Android-Betrachter, Gruppenansicht, `se screen view`.

## Plattformprüfung

| Mechanismus | Windows | Linux X11 | Linux Wayland | Android |
|---|---|---|---|---|
| Anmeldebildschirm sehen und bedienen | Systemdienst (LocalSystem) | systemweite Komponente am Display-Manager; XTest wirkt | nur GNOME Remote Login per RDP, ohne angemeldeten Nutzer | nicht machbar (Projektion endet beim Sperren ab 15 QPR1) |
| RDP-Host | Pro/Enterprise; nicht Home | xrdp (eigene Sitzung) | GNOME Remote Desktop, KRdp | keiner |
| Fenster einzeln erfassen | WGC | Composite (Annahme) | Portal mit Zustimmung; wlroots/COSMIC über ext-Protokolle | eine App ab 14, mit Wahl je Sitzung |
| Eingabe ins Fernfenster | Aktivieren, dann SendInput | `_NET_ACTIVE_WINDOW` plus XTest (Annahme) | RemoteDesktop-Portal (GNOME, KDE); wlroots keine | Bedienungshilfe; keine Tastenereignisse |
| Programmliste und Start | Startmenü und Store; Start in der Nutzersitzung | `.desktop`; Start als `app.slice`-Einheit | wie X11 | `<queries>`; Start nur mit Ausnahme |
| App-Symbole | Shell-Symbol | Icon-Theme, `_NET_WM_ICON` | Icon-Theme über `app_id` | PackageManager |
| Sichtbarkeit lokaler Fenster | minimiert/versteckt direkt; Verdeckung berechnen | berechnen | eigene Fenster `suspended` | Lebenszyklus |
| Vollbild-Ausstieg | eigene Kombination | eigene Kombination | Kombination plus Notfalltaste des Compositors | Hardwaretastatur und Geste |

Verbindungsweg:
- Direktverbindung und Relay unterscheiden sich in der Bandbreite: Start mit 6 bzw. 2 Mbit/s, das Relay verwirft bei voller Warteschlange.
- Viele gleichzeitige Fensterströme (P4) treffen über das Relay auf dieselbe Warteschlange je Ziel. P6 senkt die Last.
- Annahme: Mehrere Fensterströme passen über das Relay nur mit niedriger Bitrate je Strom.

## Entscheidungen vor einer Umsetzung

**Zuständigkeit (Nutzer, 2026-10-22):** Die Entscheidungen E1 bis E6 liegen beim Agenten. Sie fallen in Planung und Recherche des nächsten Laufs, nach dem Maßstab der bestmöglichen Umsetzung. Die Entscheidungen E1, E2 und E5 betreffen die Sicherheit und brauchen vor der Festlegung eine eigene Sicherheitsprüfung in der Planung.

**Stand 2026-10-24:** Für E6 steht ein vorläufiger Vorschlag im Batchplan `docs/plaene/2026-10-24-batches/batches.md`; festgelegt wird nach der Planung. Die übrigen Entscheidungen fallen nach dem Vorschlag in der Planung dieser Batches:
- E1, E2 und E5 in Batch 4;
- E3 und E4 in Batch 5.

- **E1:** Ein Windows-Systemdienst (LocalSystem) für Anmeldebildschirm, Sperrbildschirm, UAC und Strg+Alt+Entf. Dafür nötig sind eine Einrichtung mit Administratorrechten und ein eigenes Recht.
- **E2:** RDP als zweiter Weg. Er sperrt auf Client-Windows den lokalen Bildschirm, gibt das Kontopasswort an den Host und fehlt unter Windows Home und Android. Offen: Systemclient zuerst oder gleich ein eingebetteter Client?
- **E3:** Mehrere Android-Apps gleichzeitig als Fenster, nur mit Shell-Rechten (Shizuku). Die Einrichtung läuft über Wireless-Debugging (Annahme).
- **E4:** Recht für den Programmstart: das bestehende Recht „Befehle“ oder ein eigenes Recht.
- **E5:** Recht für den Anmeldebildschirm: ein eigenes Recht, getrennt von „Bildschirm steuern“ (Vorschlag).
- **E6:** Reihenfolge. Vorschlag:
  1. die kleinen Punkte P9, P2 und P3, sowie P7 nach der Messung;
  2. danach P6 und P1;
  3. P4 und P5 als eigener Hauptstrang.

## Abhängigkeiten

- P5 braucht P4, und P4 braucht P3 und P6.
- P1 unter Windows braucht den Systemdienst (`SYSTEM-Hilfsdienst`).
- RDP braucht eine Port-Weiterleitung durch die Share-Verbindung. Dieser Baustein ist neu.
- P4 auf Android-Hosts mit mehreren Apps braucht E3.

## Lücken

- WGC: Verhalten bei verdeckten, minimierten und versteckten Fenstern sowie auf dem Winlogon-Desktop; nur durch Versuch klärbar.
- Mechanismen von VirtualBox Seamless, Parallels Coherence und VMware Unity: nicht recherchiert.
- Wer Mutter `RecordWindow` aufrufen darf: nicht belegt.
- Koordinaten des RemoteDesktop-Portals bei Fenster-Strömen: Annahme fensterbezogen.
- Verhalten mit Windows Hello Enhanced Sign-in Security: nicht recherchiert.
- Ob sich TermService an Loopback binden lässt: nicht recherchiert.
- Android 14 und 15 vor QPR1 beim Sperren: nicht belegt.
- Ursachen von P1 (Linux), P2 und P7: nicht gesucht, wie beauftragt.
- Ob ein mit `se exec` unter dem eigenen Konto gestartetes Programm unter Windows sichtbar in der Sitzung erscheint: nicht geprüft.
- P27/P28: Protokolle der beiden beteiligten Geräte zum Zeitpunkt der Messung; ob iroh dort einen Direktweg versucht hat; Herstellerangaben zu ShadowPC.
- P30: Level- und Größengrenzen je Encoder (Media Foundation, VA-API, MediaCodec); 4:4:4-Unterstützung der Decoder.
