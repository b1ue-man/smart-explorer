# Entwurf: Verbinden ist die Einrichtung (r3, r4, r5, r7)

Stand: 2026-10-28, Quellstand `a67d7296`. Gelesen, nichts ausgeführt, nichts gebaut.
Grundlage: [Inventur](inventur-einrichtung.md) (Zeilen A1–F7), [Plan](plan.md), `docs/TODO.md`
(SELF-SETUP, SCREEN-RDP, DESKTOP-ALLE-PROGRAMME), `docs/oberflaechen/fernzugriff.md`,
`docs/GOTCHAS.md` (Abschnitte „Screen streaming“, „Share / local network“, „Updater“).
`N/` steht für `native/src/`, `A/` für `android/app/src/main/java/app/smartexplorer/android/`.
Dieser Entwurf enthält keine Geräte-, Konto-, Domänen- oder Pfadangaben einzelner Rechner.

Zweck: Umsetzungsagenten setzen nach diesem Dokument um, ohne weitere Gestaltungsentscheidungen.
Jede Aussage über bestehenden Code nennt Datei:Zeile. **Schluss** markiert eine Folgerung aus dem Code,
**Annahme An** eine nicht belegte Voraussetzung samt Prüfung in der Suite.

## 0. Auftrag, Annahmen, Begriffe

Nutzeraussagen 2026-10-28 (wörtlich im Auftrag): r3 „die eine freigabe muss reichen, der rest muss
laufen“, r4 „er verbindet zwei geräte … tcp, alles“, r5 „wenn n fenster offen ist sehen es andere se
desktops, egal ob lokal der se desktop an ist … wenn auf localhost was offen ist wird das frei gegeben
für andere geräte. per default“, r7 „keinen random ass default timeouts“.

Markierte Auslegungen (aus dem Auftrag übernommen, bleiben Annahmen):
- **Annahme A1** „beim migrieren“ = bestehende Freigaben gehen in eine neue se-Version oder auf ein
  migriertes Gerät über; die Selbsteinrichtung läuft dann erneut.
- **Annahme A2** „localhost was offen“ = lokal lauschende TCP-Dienste (127.0.0.0/8, ::1).
- **Annahme A3** „egal ob lokal der se desktop an ist“ = auf dem Gerät mit dem offenen Fenster (Host)
  muss weder die se-Oberfläche noch die se-Desktopfläche laufen; ebenso braucht der Betrachter keine
  geöffnete Geräteseite.

Begriffe:
- **Die eine Freigabe** = „Bildschirm steuern: Erlauben“ (`Capability::ScreenControl` =
  `Allow`, `N/share/core/access_grant.rs:25-40`, Variante Zeile 33, geprüft durch `steers`,
  `N/share/core/access_steering.rs:30-32`).
- **Host** = Gerät, dessen Dienst/Fenster genutzt wird; **Betrachter** = Gerät, das ihn nutzt.
- **Gerätefeed** = neuer, dauerhaft offener Kanal Host → Betrachter (Abschnitt 5).
- **Geräteadresse** = stabile Loopbackadresse je Gegengerät auf dem Betrachter (Abschnitt 4.3).

## 1. Entscheidungen auf einen Blick

| Nr. | Entscheidung | Abschnitt |
|---|---|---|
| E1 | „Remote Desktop“ ist eine Handlung ohne Formular: frische Probe, Relation, Dienst, Client, ganzer Desktop werden abgeleitet | 2 |
| E2 | Relation: anwesende Direktbeziehung vor anwesender Raumbeziehung | 2.2 |
| E3 | Dienstrang: geteilte Konsole vor Zusatzbildschirm vor bildschirmloser Sitzung vor eigener Xorg/Xvnc-Sitzung vor Remote Login vor Konsole mit Sperre | 2.3 |
| E4 | TCP-Lease ohne feste Lebensdauer: RDP-Tunnel an Clientprozess und QUIC-Verbindung gebunden; einzige Zeitgrenze ist die Toterkennung der QUIC-Verbindung (20 s Leerlauf) | 2.6, 4.6 |
| E5 | Kästchen „jeder lokale Prozess“ entfällt: RDP-Listener nimmt nur Verbindungen des gestarteten Clientprozesses an (PID + Startzeit), automatische Weiterleitungen nur Prozesse desselben Betriebssystemkontos | 2.7, 4.4 |
| E6 | mstsc-Zertifikatdialog entfällt: se hinterlegt den authentifizierten SHA-1-Thumbprint als `CertHash` für die Geräteadresse; Profil ohne `/public` und `/prompt` | 2.7 |
| E7 | Kästchen „Sitzungswirkung“ entfällt: Zustimmung liegt mit „Bildschirm steuern: Erlauben“ beim Host; der Hinweis wird angezeigt, nicht bestätigt | 2.7 |
| E8 | Zugangsdaten: Betriebssystemkonten tippt die Person im OS-Client (einmal, dank stabiler Geräteadresse speicherbar); von se erzeugte Dienstzugangsdaten (GNOME-Desktopfreigabe) liefert se automatisch | 2.8 |
| E9 | Windows-Host schaltet RDP selbst ein (UAC einmal je Version und Auslöser), öffnet keine Firewallregel; Home: ausdrückliche Meldung | 3.2 |
| E10 | Linux-Host: GNOME-Desktopfreigabe bzw. KRdp bzw. xrdp nur auf Loopback; se installiert fehlende Pakete über Root-Worker oder pkexec | 3.3 |
| E11 | localhost-Dienste sind unter „Bildschirm steuern: Erlauben“ freigegeben (loopbackgebunden jeder Besitzer; wildcardgebunden nur Dienste normaler Benutzerkonten) | 4.2 |
| E12 | Betrachter bildet jeden freigegebenen Dienst automatisch auf `Geräteadresse:gleicher Port` ab | 4.3 |
| E13 | Fensterlisten und Dienstlisten kommen über einen Gerätefeed des Hintergrunddienstes; weder Host- noch Betrachteroberfläche muss laufen | 5 |
| E14 | Programme: Katalog lädt automatisch, Belege werden automatisch nachgefragt, bestätigte Fenster öffnen sich selbst | 6 |
| E15 | Zeitgrenzen: alle Bedienwartezeiten und Lebensdauern entfallen; behalten werden nur Toterkennung, Protokollschrittgrenzen und Werkzeugschutz | 7 |

## 2. RDP mit einer Handlung (Betrachter)

### 2.1 Ablauf „Remote Desktop“ (alle Oberflächen mit RDP-Client)

Heute verlangt die Desktop-Geräteseite Probe-Klick, Relation, Dienst, Client, Darstellung,
Lebensdauer und drei Kästchen (Inventur A1–A12; Formular `N/app/core/share_rdp_form.rs:7-217`,
Seite `N/app/core/share_rdp_ui.rs:68-148`). Neu läuft eine Funktion `rdp::open_device` in dieser
festen Reihenfolge:

1. **Relation wählen** (2.2). Keine anwesende Relation → Fehler `rdp_device_offline` mit Text
   „Gerät ist nicht erreichbar“.
2. **Frische Probe** `AppsCommand::RdpProbeV2` über diese Relation (Vertrag 9.2). Keine Wartefrist:
   unter „Erlauben“ antwortet der Host sofort; unter „Nachfragen“ wartet die Anfrage, bis der Host
   antwortet, die Person abbricht oder die QUIC-Verbindung als tot erkannt wird
   (`IROH_CONNECTION_IDLE_TIMEOUT` 20 s, `N/share/core/keepalive.rs:8`).
3. **Hosteinrichtung anstoßen**, wenn die Probe keinen startbaren Dienst, aber `setup` ≠ `impossible`
   meldet: `AppsCommand::RdpPrepare` (Abschnitt 3); danach neue Probe aus der Antwort.
4. **Dienst** nach E3 unter `HostService::startable()` (`N/rdp/core/types.rs:117-131`).
5. **Client** nach 2.4.
6. **Darstellung**: ganzer Desktop. RemoteApp nur, wenn die Person einen RemoteApp-Eintrag gewählt hat (2.5).
7. **Start**: TCP-Tunnel Version 2 mit prozessgebundenem Listener auf der Geräteadresse (2.6, 2.7),
   Vertrauen hinterlegen (2.7), Client starten, PID an den Listener binden, Sitzung in die bestehende
   Besitzerregistry (`N/rdp/os/shared/session_owners.rs:138-265`) übergeben.

Die bisherige 5-Minuten-Grenze der Probe (`N/rdp/core/admission.rs:35-37`) entfällt. Ersatz: die
Probe wird je Start einmal erhoben und mit `start` verbraucht (`AuthenticatedProbe` wird by value
übergeben, kein Wiederverwenden). Begründung: Zeitalter war Ersatz für eine Bindung; die Bindung
ist jetzt „eine Probe, ein Start“, zusätzlich prüft der Host beim Verbinden den Besitz des
Dienstports (2.7, Punkt 4).

### 2.2 Relation

Regel: Unter allen gespeicherten Beziehungen zur bestätigten Geräteidentität (Gerät-ID und
Schlüssel, wie heute `N/app/core/share_rdp_ui.rs:70-104`) gilt die erste anwesende in dieser
Reihenfolge: Direktkontakt mit Anwesenheit, dann Raummitglied mit Anwesenheit (nicht gesperrt);
bei mehreren gleicher Art die mit der jüngsten Anwesenheitsmeldung, danach die kleinere
Beziehungs-ID. Grund: Kernprinzip „direkt zuerst“ (`docs/ARCHITEKTUR.md:13-17`); Direktbeziehungen
tragen die gerätespezifischen Rechte, Raumbeziehungen sind der Rückfall. Dieselbe Funktion
`share::present_relation(device_id, public_key) -> Option<PeerOpenTarget>` (Vertrag 9.1) nutzen
RDP, Gerätefeed, Programme und TCP.

### 2.3 Dienstrang

Unter den startbaren Diensten der Probe gewinnt der mit dem kleinsten Rang:

| Rang | `SessionMode` (`N/rdp/core/types.rs:17-28`) | Grund |
|---|---|---|
| 1 | `SharedConsole` | „Bildschirmfreigabe heißt Bildschirmfreigabe“ (r3): derselbe sichtbare Bildschirm, keine zweite Anmeldung, die Person am Host bleibt ungestört |
| 2 | `AdditionalConsoleDisplay` | bestehende Sitzung, nichts wird gesperrt |
| 3 | `HeadlessUserSession` | eigene bildschirmlose Sitzung desselben Kontos, sperrt nichts |
| 4 | `SeparateXorg`, dann `SeparateXvnc` | eigene Sitzung; Xorg vor Xvnc wegen Eingabe-/Grafikumfang |
| 5 | `WindowsServerSession`, `RemoteLogin` | neue Anmeldung |
| 6 | `WindowsConsoleLocked`, `ExclusiveConsole` | sperrt den lokalen Bildschirm |

Gleichstand: kleinere Dienst-ID (`HostService::id`) zuerst. `Unknown` ist nie startbar
(`types.rs:124-126`). Schluss: Windows liefert genau einen Dienst (`N/rdp/os/windows/probe.rs:68-75`),
der Rang wirkt nur unter Linux (bis zu fünf Dienste, Inventur A3).

### 2.4 Client

- Windows: `mstsc` aus dem Systemverzeichnis (`N/rdp/os/windows/clients.rs:10-14`), einziger Client.
- Linux, Reihenfolge: `sdl-freerdp3` (eigene grafische Anmelde- und Zertifikatdialoge im SDL-Client,
  FreeRDP 3.10.3 `client/SDL/SDL3/dialogs/sdl_dialogs.cpp`: `sdl_authenticate_ex`), danach
  `xfreerdp3` in einer X11-Sitzung bzw. `wlfreerdp3` in einer Wayland-Sitzung. Neuer Clienteintrag
  `sdl-freerdp3` mit Suchnamen `sdl-freerdp3`, `sdl-freerdp` in `N/rdp/os/linux_os/clients.rs:12-18`,
  gleiche Vertrauensprüfung (`N/rdp/os/linux_os/client_trust.rs:9-80`) und Versionprüfung
  (`clients.rs:19-34`).
- Die Terminalbedingung (`clients.rs:45,49,52`, Inventur B5) gilt nur noch für `xfreerdp3`/
  `wlfreerdp3` ohne von se gelieferte Zugangsdaten (2.8). `sdl-freerdp3` braucht kein Terminal.
- Android: kein RDP-Client (`N/rdp/os/mod.rs:8-32`, Inventur B6). Die Android-Oberfläche zeigt
  „Remote Desktop“ nicht; der Bildschirmstrom bleibt der Weg. Keine URI, kein Ersatzstart
  (`docs/oberflaechen/fernzugriff.md`, Abschnitt Plattformgrenzen).
- **Annahme A4**: Debian/Ubuntu liefern den SDL-Client als `/usr/bin/sdl-freerdp3` (Paket
  `freerdp3-sdl`). Suite: `ubuntu-24.04` installiert `freerdp3-sdl` und prüft, dass die Clientliste
  `sdl-freerdp3` als `available` meldet.

### 2.5 Darstellung und RemoteApps

Standard ist der ganze Desktop (`remote_app: None`). Jeder in der Probe veröffentlichte
RemoteApp-Alias (nur Windows Server mit RDS, `N/rdp/core/types.rs:223-225`) erscheint als eigener
Eintrag „<Anzeigename> (RemoteApp)“ neben „Remote Desktop“ auf Geräteseite, se-Desktop-Kachelmenü,
CLI (`se rdp open <gerät> --remote-app <alias>`) und MCP (`remote_app`). Ein Klick startet genau
diesen Alias. Kein Auswahlfeld mehr (`share_rdp_form.rs:118-142` entfällt).

### 2.6 Lebensdauer ohne feste Frist (r7)

Heute: `StartRequest.lifetime_secs` Vorgabe 900 (`N/rdp/core/admission.rs:85-89`), TCP-Vorgabe 900
(`N/share/core/tcp_forward.rs:136-141`), Grenze 1–86400 (`tcp_forward.rs:148-150`); Host beendet nach
`lifetime_secs` (`N/share/core/node_tcp.rs:217,227`), Betrachter ebenso
(`N/share/core/node_tcp_link.rs:227,234`); der Besitzer beendet den Client, wenn die Lease endet
(`N/rdp/os/shared/session_owners.rs:217-232`).

Neu:
- **Host**: Ein Tunnel gehört genau einer QUIC-Verbindung (`node_tcp.rs:222-229` endet schon bei
  `connection.closed()`, Schließbyte, Entzug). Mit Protokoll Version 2 (4.6) bedeutet
  `lifetime_secs = 0` „ohne Frist“: der `ttl`-Zweig (`node_tcp.rs:217-218,227`) wird nur bei
  `lifetime_secs > 0` angelegt. Der Host erneuert nichts; er beendet den Tunnel bei Verbindungsende,
  Rechteentzug (`node_tcp.rs:225`) oder Freigabeende. Toterkennung: QUIC-Leerlauf 20 s bei 5 s
  Keepalive (`N/share/core/keepalive.rs:6-8`).
- **Betrachter**: Der Listener lebt, solange (a) die QUIC-Verbindung lebt und (b) der gebundene
  Clientprozess lebt. Der Hintergrunddienst hält dafür ab der Bindung ein Prozesshandle:
  Windows `OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION)` und Warten auf das Handle,
  Linux `pidfd_open` und Lesbarkeit des pidfd. Endet der Prozess, schließt der Dienst den Tunnel,
  auch wenn die besitzende Oberfläche abgestürzt ist.
- **Bindungsvertrag** (9.3): `TcpIpcRequest::Start` mit `admission: ListenerAdmission::AwaitProcess`
  öffnet den Tunnel; der Dienst antwortet `Opened`, hält den IPC-Strom offen und erwartet auf
  demselben Strom `BindProcess { pid, started }`. Endet der Strom vor `BindProcess`, schließt der
  Dienst den Tunnel. Verbindungen, die vor `BindProcess` eintreffen, werden angenommen, aber erst
  nach der Bindung geprüft und weitergeleitet.
- `start_guarded` verliert die 1–300-s-Prüfung (`N/rdp/os/shared/session.rs:67-69`); `wait` wird
  `Option<Duration>` mit `None` als Vorgabe.
- Der Besitzer-Monitor (`session_owners.rs:186-237`) bleibt; er meldet weiter Entzug/Ende.

### 2.7 Technische Garantien statt Kästchen

1. **Loopbackzugriff (A7)**: Der RDP-Listener nimmt eine Verbindung nur an, wenn sie vom gebundenen
   Clientprozess stammt (gleiche PID und gleiche Prozessstartzeit). Mehrere Verbindungen nacheinander
   und gleichzeitig sind erlaubt (Wiederverbindung nach Netzwechsel, erneute NLA-Anmeldung nach
   falschem Kennwort öffnen neue TCP-Verbindungen); jede wird geprüft. Abweichung vom Auftragstext
   „genau eine Verbindung“: begründet durch diese Wiederverbindungen, die sonst die Sitzung beenden.
   - Windows: `GetExtendedTcpTable(AF_INET, TCP_TABLE_OWNER_PID_CONNECTIONS)` liefert die Zeile mit
     lokaler Adresse = Gegenadresse der angenommenen Verbindung und entfernter Adresse = Listener;
     deren `dwOwningPid` muss die gebundene PID sein; `GetProcessTimes(CreationTime)` muss der
     gebundenen Startzeit entsprechen.
   - Linux: `/proc/net/tcp` liefert die Zeile mit lokal = Gegenadresse, entfernt = Listener; deren
     Inode muss als `socket:[inode]` unter `/proc/<pid>/fd` der gebundenen PID stehen; Startzeit aus
     Feld 22 von `/proc/<pid>/stat`.
   - Fremde Verbindung: sofort schließen, `last_error` „Verbindung eines fremden Prozesses abgewiesen
     (PID n)“ am Tunnel (`N/share/core/tcp_registry.rs:96-102`).
   - **Annahme A5**: `mstsc.exe` und FreeRDP öffnen die RDP-TCP-Verbindung im eigenen Prozess.
     Suite: Ende-zu-Ende-Start (10.4) gelingt mit aktiver Prüfung; eine `nc`/`Test-NetConnection`-
     Verbindung eines anderen Prozesses wird abgewiesen.
2. **Zertifikat mstsc (A9)**: Primärquelle zur Speicherung: Microsoft dokumentiert keinen
   Zertifikatpin für `.rdp`-Dateien; die Microsoft-Antworten empfehlen ein vertrauenswürdiges
   Listenerzertifikat am Server
   ([Q&A 275215](https://learn.microsoft.com/en-us/answers/questions/275215/powershell-how-to-add-rdp-thumbprint-of-remote-com)).
   Ein Listenerzertifikat mit vertrauter Kette am Host würde dessen RDP-Einstellung für alle übrigen
   Clients ändern und am Betrachter einen Stammzertifikatseintrag verlangen; beides verworfen.
   Entscheidung: se schreibt beim Start unter
   `HKCU\Software\Microsoft\Terminal Server Client\Servers\<Geräteadresse>` und zusätzlich unter
   `…\Servers\<Geräteadresse>:<Port>` den Wert `CertHash` (REG_BINARY, 20 Byte SHA-1-Thumbprint aus der
   authentifizierten Probe v2). Das ist der Speicher, den mstsc selbst nach „Nicht erneut fragen“
   anlegt. **Annahme A6**: mstsc überspringt mit passendem `CertHash` die Zertifikatwarnung, auch
   bei Namensabweichung, und die Schlüsselbenennung ist eine der beiden Formen. Suite: 10.4
   Windows-Ende-zu-Ende ohne Dialogfenster der Klasse `#32770` des mstsc-Prozesses bis zur Sitzung.
   Profiländerungen (`N/rdp/os/windows/profile.rs:68-78`): `full address:s:<Geräteadresse>:<Port>`,
   `authentication level:i:2` und `enablecredsspsupport:i:1` bleiben, neu `prompt for credentials:i:0`;
   Startargumente ohne `/prompt` und ohne `/public` (`N/rdp/os/windows/clients.rs:28-29`), damit
   `CertHash` und gespeicherte Anmeldedaten gelten. Stimmt der Thumbprint nicht (Host hat sein
   Zertifikat zwischen Probe und Verbindung gewechselt), erscheint der native Warndialog; das ist
   dann ein echter Befund und kein Bedienschritt des Normalfalls.
   Ein alter Host ohne Probe v2 liefert keinen SHA-1-Wert: Meldung „Gegenstelle älter als 0.5.189:
   Remote Desktop braucht dort das Update“, kein Start (keine stille Rückkehr zum Dialogvergleich).
3. **Zertifikat FreeRDP**: unverändert `/cert:deny,fingerprint:sha256:<pin>`
   (`N/rdp/os/linux_os/clients.rs:72-75`); `ExactPeerPin` entfällt als Vorgabe (B2): CLI, MCP und
   Bridge wählen die Politik des gewählten Clients (`ClientCapability::certificate_policy`,
   `N/rdp/core/types.rs:257-266`). Feld `certificate_policy` bleibt im `StartRequest` als
   abgeleiteter Wert, nicht mehr als Eingabe.
4. **Hostseitiger Dienstbesitz**: Bevor der Host eine Verbindung an einen RDP-Port weitergibt
   (`N/share/os/shared/tcp_sockets.rs:79-142`), prüft er, dass der Listener des Zielports dem
   RDP-Dienstprozess gehört (Windows: Besitzer-PID = TermService-PID; Linux: Inode im fd-Verzeichnis
   des Dienstprozesses, ersatzweise Socket-UID = Dienstkonto, 3.3). Das schließt einen fremden
   Prozess aus, der den Port zwischen Probe und Verbindung belegt.
5. **Sitzungswirkung (A8)**: entfällt als Bestätigung. Die Zustimmung liegt beim Host mit
   „Bildschirm steuern: Erlauben“ (`N/share/core/access_steering.rs:1-8`). Der Betrachter zeigt
   `SessionMode::notice()` (`types.rs:30-43`) als Hinweiszeile neben der laufenden Sitzung.
   `acknowledge_session_effect` (`admission.rs:81,97-99`) wird entfernt.

### 2.8 Zugangsdaten (A10)

- **Betriebssystemkonten** (Windows TermService, xrdp über PAM, GNOME Remote Login, KRdp mit
  Systemkonto): Die Person tippt Benutzer und Kennwort im Dialog des OS-Clients. Grund: se kennt keine
  Kontokennwörter und gibt keine weiter (Vertrag in `N/rdp/README.md`, „Keine Passwörter kommen in
  argv, Dateien, URIs oder SE-Logs“). Verbesserung: Weil die Geräteadresse je Gegengerät stabil ist
  (4.3), wirkt „Anmeldedaten speichern“ von mstsc (Windows-Anmeldeinformationsverwaltung,
  Ziel `TERMSRV/<Geräteadresse>`) dauerhaft; die Eingabe ist einmal je Gerätepaar nötig.
- **Von se verwaltete Dienstzugangsdaten** (GNOME-Desktopfreigabe hat eigene RDP-Zugangsdaten, nicht
  das Kontokennwort; GNOME 48.2 `src/grd-ctl.c`, Befehl `set-credentials`): Der Host erzeugt sie
  (3.3) und liefert sie über `AppsCommand::RdpCredential` (9.2). Der Betrachter übergibt sie an
  mstsc über `CredWriteW` (Typ `CRED_TYPE_GENERIC`, Ziel `TERMSRV/<Geräteadresse>`,
  `CRED_PERSIST_SESSION`, gelöscht beim Sitzungsende) und an FreeRDP über `/args-from:stdin`
  (FreeRDP 3 `client/common/cmdline.h:176-180`: ein Argument je Zeile, nicht kombinierbar mit
  weiteren argv-Argumenten; alle Argumente gehen dann über die Pipe, keine in argv).
- **Linux-GUI ohne `sdl-freerdp3`** mit Betriebssystemkonto: `xfreerdp3`/`wlfreerdp3` haben ohne
  Terminal keine Eingabe. Die Desktop-GUI zeigt dann einen eigenen maskierten Dialog (Benutzer,
  Domäne, Kennwort) und übergibt die Werte ausschließlich über `/args-from:stdin`; der Puffer wird
  nach dem Schreiben überschrieben. MCP und Bridge zeigen keinen Dialog und melden
  „Für Remote Desktop ohne Terminal das Paket freerdp3-sdl installieren“.

### 2.9 Vorgaben für CLI, MCP, Bridge

- `se rdp open <gerät> [--remote-app <alias>] [--json]` reicht. Die Schalter `--service`,
  `--client`, `--certificate-policy`, `--acknowledge-*`, `--lifetime-secs`, `--wait`
  (`N/cli/rdp.rs:32-53`) bleiben verborgen annehmbar (`hide = true`) und werden ignoriert, außer
  `--service`/`--client` (übersteuern die Auswahl, falls angegeben) — so brechen keine Skripte.
- MCP `rdp` Aktion `open`: Pflicht nur `device`; `remote_app` optional; `acknowledge_*`,
  `certificate_policy`, `lifetime_secs` werden angenommen und ignoriert; `wait` optional ohne Vorgabe
  (`N/cli/mcp/rdp_tool.rs:10-24`). Beschreibungstext (`rdp_tool.rs:10`) wird an die neuen Garantien
  angepasst.
- Bridge `rdp.open` (`N/mobile/os/shared/domains/rdp.rs:110-160`): gleiche Ableitung; auf Android
  bleibt die Fehlermeldung des fehlenden Clients (`rdp.rs:114-119`).

## 3. Hosteinrichtung von RDP

### 3.1 Auslöser und Marker

Die Einrichtung läuft im Hintergrunddienst des Hosts (Modul `N/rdp/os/shared/self_setup.rs`, neu),
bei genau diesen Auslösern:
- **T1 Freigabe**: eine Beziehung erhält „Bildschirm steuern: Erlauben“ (Vergleich alter/neuer
  Rechte in `RuntimeConfiguration::apply`, `N/share/core/configuration_runtime.rs:44-60`).
- **T2 Migration** (Annahme A1): erster Start einer Version, wenn mindestens eine Beziehung
  „Bildschirm steuern: Erlauben“ hat. Marker je Version wie beim Firewallweg
  (`N/app/core/share_drain.rs:386-400`), Datei `<sync_data_dir>/rdp-setup-<version>.json` mit
  Ergebnis (`done`, `declined`, `impossible:<grund>`, `failed:<grund>`).
- **T3 Online**: ein Gerät mit dieser Freigabe wird anwesend (ShareEvent-Varianten mit Anwesenheit,
  `N/share/core/types.rs:366-405`) und der Marker dieser Version lautet nicht `declined`/`impossible`.
- **T4 Anfrage**: ein Betrachter sendet `RdpPrepare` (Klick „Remote Desktop“). Das ist eine
  Nutzerhandlung und fragt auch nach `declined` erneut.

Nach jeder Einrichtung: `rdp::probe_local_host` und `note_own_rdp_ports`
(`N/share/core/access_steering.rs:18-23`). Zusätzlich beim Dienststart einmal im Hintergrund, damit
die Portliste nach einem Neustart nicht leer ist (heute nur bei einer Probe gefüllt,
`N/share/core/node_apps_host.rs:14-19`).

### 3.2 Windows

Bestand: Probe liest nur (`N/rdp/os/windows/probe.ps1:1-2`), meldet „wird nicht automatisch
aktiviert“ (`N/rdp/os/windows/probe.rs:61-62`); Home-SKUs `probe.ps1:10`, unterstützte SKUs `:9`.

Neu `N/rdp/os/windows/host_setup.rs` mit festem Skript `host_setup.ps1` (keine Peerdaten im Skript;
einziger Parameter ist die SID des angemeldeten Benutzers, im Dienst ermittelt). Ausführung über den
vorhandenen Administratorweg `net::run_elevated_system_powershell`
(`N/net/os/windows/uplink_install.rs:150-171`, UAC über `Start-Process -Verb RunAs`), wie die
Firewallregel des Peer-Listeners (`N/share/os/windows/system.rs:91-105`). Das Skript:
1. bricht ab, wenn eine Gruppenrichtlinie RDP verbietet
   (`HKLM\SOFTWARE\Policies\Microsoft\Windows NT\Terminal Services\fDenyTSConnections = 1`);
2. setzt `fDenyTSConnections = 0` (`HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server`);
3. setzt `UserAuthentication = 1` unter `WinStations\RDP-Tcp` **nur**, wenn RDP vorher aus war
   (bestehende Administratorwahl bleibt unangetastet);
4. startet `TermService`, falls nicht laufend;
5. nimmt die Benutzer-SID in `Remote Desktop Users` (S-1-5-32-555) auf, wenn sie weder dort noch in
   `Administrators` (S-1-5-32-544) Mitglied ist;
6. **öffnet keine Firewallregel**: Der Tunnel erreicht den Dienst über Loopback, den die
   Windows-Firewall nicht filtert. Ein vorher eingeschalteter Regelsatz „Remotedesktop“ bleibt, wie
   er ist.

Editionsgrenzen: Home/Core (`probe.ps1:10`) → kein UAC, Marker `impossible:windows_home`, Text
„Windows Home kann keinen Remote-Desktop-Zugang anbieten; Bildschirm ansehen/steuern bleibt
verfügbar“. Server ohne RDS-Rolle: Einrichtung wie Pro (zwei Verwaltungssitzungen).

Zusätzliche Probe-Diagnose (in `probe.ps1`): Wert
`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\PasswordLess\Device\DevicePasswordLessBuildVersion = 2`
ergibt den Hinweis „Kennwortlose Windows-Anmeldung aktiv: RDP braucht das Kontokennwort“.

### 3.3 Linux

Entscheidungsbaum je Host (erste zutreffende Zeile gilt):

| Fall | Handlung | Ausführender |
|---|---|---|
| startbarer Dienst vorhanden | nichts | – |
| GNOME-Sitzung und `grdctl` vorhanden | GNOME-Desktopfreigabe einrichten (unten) | Benutzerdienst der Sitzung |
| Plasma-Sitzung und `krdpserver` vorhanden | Unit `app-org.kde.krdpserver.service` aktivieren/starten (Unitname aus `N/rdp/os/linux_os/krdp.rs:13`), wenn KRdp-Benutzer konfiguriert sind | Benutzerdienst |
| `xrdp` installiert | xrdp nur auf Loopback einrichten (unten) | Root-Dienst direkt, sonst `pkexec` |
| nichts installiert | Paket installieren: GNOME-Sitzung → `gnome-remote-desktop`; Plasma → `krdp`; sonst und ohne grafische Sitzung → `xrdp` und `xorgxrdp`; danach obige Zeile | Root-Dienst direkt, sonst `pkexec` |

- **Paketverwaltung**: nur `apt-get install -y --no-install-recommends` (Debian/Ubuntu, erkannt über
  `/etc/os-release` `ID`/`ID_LIKE` ∋ `debian`) und `dnf install -y` (`ID_LIKE` ∋ `fedora`/`rhel`).
  Andere Verteilungen: `impossible:package_manager_unsupported` mit Paketnamen im Text.
- **Privilegweg**: Läuft der Dienst als root (Root-Dienst auf Servern), führt er die Schritte selbst
  aus. Sonst startet er `pkexec <se> rdp host-setup --system <schritt>` (polkit-Dialog = Freigabe des
  Betriebssystems, Gegenstück zu UAC). Vorbedingung: `<se>` und alle Elternordner gehören root und sind
  weder gruppen- noch weltbeschreibbar (gleiche Prüfung wie `trusted_system_path`,
  `N/rdp/os/linux_os/client_trust.rs:82-97`); sonst `impossible:se_binary_not_root_owned`. Ohne
  polkit-Agent: `failed:no_polkit_agent`. `<schritt>` ist ein fester Bezeichner (`install-xrdp`,
  `configure-xrdp`, `install-gnome-rd`, `install-krdp`), keine freien Argumente.
- **xrdp nur Loopback** (Entscheidung: se installiert und richtet xrdp ein, aber nie netzweit):
  Schreibt im `[Globals]`-Abschnitt der von systemd verwendeten `xrdp.ini` (Pfadermittlung wie
  `N/rdp/os/linux_os/xrdp.rs:19-22`) `port=tcp://.:<p>`; laut xrdp 0.10.3 `xrdp/xrdp.ini.in:16`
  bedeutet `tcp://.:3389` „127.0.0.1:3389“ (identisch in 0.9.24). `<p>` = vorheriger Port, sonst 3389.
  `security_layer=tls` (nur wenn nicht schon `tls`/`negotiate`), `autorun` unverändert. Danach
  `systemctl enable --now xrdp` bzw. `restart xrdp`. Die Probe liest `port=tcp://.:N` als Port `N`
  (heute nur Zahl, `xrdp.rs:31`). Bestehende xrdp-Installation mit netzweitem Port: se stellt sie auf
  Loopback um, **nur wenn se sie selbst installiert hat** (Marker in `/var/lib/smart-explorer/rdp-xrdp-installed`);
  eine fremde netzweite Einrichtung bleibt (bestehendes Verhalten erhalten).
- **Portbeleg ohne fd-Zugriff**: Ein Benutzerdienst kann `/proc/<pid>/fd` des root-/xrdp-Prozesses
  nicht lesen (`N/rdp/os/linux_os/inventory.rs:58-63` endet dann mit Fehler). Neu: Wird der fd-Zugriff
  verweigert, gilt der konfigurierte Port als belegt, wenn `/proc/net/tcp{,6}` eine LISTEN-Zeile mit
  diesem Port und UID 0 oder der UID von `runtime_user` zeigt.
- **GNOME-Desktopfreigabe** (Benutzerdienst, Sitzungsbenutzer; GNOME 48.2 `src/grd-ctl.c:380-391`):
  1. Fehlen TLS-Zertifikat/Schlüssel: se erzeugt mit `rcgen` ein selbstsigniertes Zertifikat mit den
     `rcgen`-Gültigkeitsvorgaben (keine eigene Frist), Dateien `<app_data>/rdp/gnome-tls.{crt,key}`
     (Schlüssel 0600), dann `grdctl rdp set-tls-cert` und `set-tls-key`.
  2. `grdctl rdp enable`, `grdctl rdp disable-view-only`.
  3. `systemctl --user enable --now gnome-remote-desktop.service`.
  4. Zugangsdaten: Benutzername fest `se-<8 Hex aus Geräte-ID>`, Kennwort 32 Zufallsbytes
     base64url, gesetzt über `grdctl rdp set-credentials` mit **stdin** (grdctl liest bei fehlenden
     Argumenten zeilenweise von stdin, `grd-ctl.c:150-190`). Neu gesetzt bei jedem
     `RdpCredential`-Abruf und bei jedem Entzug von „Bildschirm steuern“; se speichert das Kennwort
     nicht.
  GNOME bindet RDP an alle Schnittstellen (`grd-ctl.c` kennt nur `set-port`); Absicherung siehe 8.
- **KRdp**: se legt keine KRdp-Benutzer an (KRdp verwaltet deren Kennwörter selbst; Schluss aus der
  Konfigurationsgrenze `N/rdp/os/linux_os/krdp.rs:16-32`). Ohne Benutzer: `impossible:krdp_no_users`
  mit Text „KRdp: in den Systemeinstellungen unter Remote Desktop einen Benutzer anlegen“.
- **Wayland-GNOME-Remote-Login** (`gnome-login`, Systemdienst über `pkexec`, `gnome.rs:1`): se richtet
  ihn nicht ein (eigene Zugangsdaten und GDM-Übergabe); er bleibt nutzbar, wenn vorhanden.

### 3.4 Fehlerpunkte

Jeder Fehler wird im Marker gespeichert, auf der Host-Seite „Dieses Gerät“ und als `setup`-Feld der
Probe v2 an den Betrachter gemeldet. Kein stiller Rückfall.

| Punkt | Behandlung |
|---|---|
| UAC/polkit abgelehnt | `declined`; erneut bei T1, T4 oder neuer Version |
| keine interaktive Sitzung für UAC/polkit | `failed:no_interactive_session`; erneut bei T3/T4 |
| Gruppenrichtlinie verbietet RDP | `impossible:policy_denies_rdp` |
| Windows Home | `impossible:windows_home` |
| TermService/xrdp startet nicht | `failed:<Dienstfehlertext>`; erneut bei T4 |
| Listener nach Einrichtung nicht belegt | `failed:no_listener` mit Probe-Diagnose |
| Paketinstallation scheitert | `failed:package:<Exitcode>` mit letzter Fehlerzeile; erneut bei T4 |
| Paketverwaltung unbekannt | `impossible:package_manager_unsupported` |
| se-Binärdatei nicht root-eigen | `impossible:se_binary_not_root_owned` |
| GNOME-Schlüsselbund gesperrt (Zugangsdaten nicht speicherbar) | `failed:keyring_locked`; erneut bei T3/T4 |
| KRdp ohne Benutzer | `impossible:krdp_no_users` |
| Android-Host | `impossible:android` (`N/rdp/os/mod.rs:8-32`) |

## 4. TCP ganzheitlich und localhost (r4, r5)

### 4.1 Lauschende Dienste ermitteln

| Plattform | Quelle | Prozessname | Bindungsumfang |
|---|---|---|---|
| Windows | `GetExtendedTcpTable` für `AF_INET`/`AF_INET6`, `TCP_TABLE_OWNER_PID_LISTENER` | `QueryFullProcessImageNameW` nach `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`; Konto über `OpenProcessToken`/`GetTokenInformation(TokenUser)` | lokale Adresse |
| Linux | `/proc/net/tcp`, `/proc/net/tcp6`, Zustand `0A`, Spalte UID, Inode | `/proc/<pid>/comm` über Inodesuche in `/proc/*/fd` (für fremde Konten nur als root lesbar; sonst „Konto UID n“) | lokale Adresse |
| Android | nicht ermittelbar: Apps lesen `/proc/net` seit Android 10 nicht (**Annahme A7**, Suite: Android-Bridge meldet `unavailable`) | – | – |

Prüfung des Musters: das bestehende Linux-Inventar liest `/proc/<pid>/net/tcp{,6}` mit Zustand `0A`
(`N/rdp/os/linux_os/inventory.rs:78-101`). Neues gemeinsames Modul je OS (9.1):
`N/share/os/linux_os/proc_net_tcp.rs` und `N/share/os/windows/tcp_table.rs`; RDP-Inventar und
Listenerprüfung (2.7) verwenden dieselben Funktionen.

### 4.2 Rechteregel (E11)

Freigegeben sind PeerToHost-Verbindungen zu Loopback auf einen Port, der in der aktuellen
Freigabeliste steht, wenn die Beziehung „Bildschirm steuern: Erlauben“ hat. Die Liste enthält:
- jeden Listener, der **nur an Loopback** gebunden ist (127.0.0.0/8, ::1, ::ffff:127.0.0.0/104),
  gleich welchem Konto er gehört;
- jeden **Wildcard**-Listener (0.0.0.0, ::), dessen Prozess einem normalen Benutzerkonto gehört
  (Linux: UID ≥ `UID_MIN` aus `/etc/login.defs`, Vorgabe 1000; Windows: Token-Benutzer ist weder
  SYSTEM, LOCAL SERVICE, NETWORK SERVICE noch ein Dienstkonto `NT SERVICE\*`; nicht ermittelbar zählt
  als Systemkonto);
- **ausgenommen**: Ports eigener se-Prozesse (Bild-Pfad gleich einem installierten se-Programm im
  Ordner von `current_exe`), der IPC-Port des Dienstes (`ipc_storage::read_ipc_addr`), die
  RDP-Dienstports (`OWN_RDP_PORTS`; sie laufen über „Remote Desktop“).

Umsetzung: `access_steering::includes_request` (`N/share/core/access_steering.rs:40-50`) nimmt
zusätzlich Ports aus `exposed_loopback_port(port)` an (statische Menge wie `OWN_RDP_PORTS`,
`access_steering.rs:16`), gefüllt vom Host bei jeder Ermittlung. Richtung, Loopbackprüfung und
aufgelöste IP bleiben (`access_steering.rs:44-46`).

Begründung gegenüber der Sicherheit: Wer den Bildschirm steuern darf, bedient die angemeldete
Sitzung und erreicht jeden lokalen Port mit jedem Programm dieser Sitzung; die Weiterleitung gibt
keine Reichweite hinzu, die die eine Freigabe nicht schon hat (Begründungslinie wie
`access_steering.rs:1-8`). Verworfen: „Befehle: Erlauben“ als Träger, weil eingeschränkte Konten und
Programmlisten (`AccountProfile`, `N/share/core/access_grant.rs:151-200`) keine gleichwertige
Reichweite haben. Wildcard-Systemdienste bleiben ausgenommen (Abschnitt 8, Bedrohung T-L2).

### 4.3 Betrachter: automatische Abbildung (E12)

- **Geräteadresse**: je Gegengerät eine stabile IPv4-Loopbackadresse `127.b1.b2.b3` aus den ersten
  drei Bytes von SHA-256(`"se-loopback-v1|" + device_id + "|" + public_key`), mit
  `b1 = 1 + (byte0 mod 254)` (also nie 127.0.x.x, wo 127.0.0.1, 127.0.0.53 und 127.0.1.1 üblich sind),
  `b3 = 1 + (byte2 mod 254)`. Kollision mit einem anderen bekannten Gerät: SHA-256 über
  `…|<zähler>` mit Zähler 1, 2, … Die Zuordnung wird in `<app_data>/loopback-addresses.json`
  gespeichert und nie neu vergeben, solange das Gerät bekannt ist. **Annahme A8**: Windows bindet
  Adressen aus 127.0.0.0/8 außer 127.0.0.1. Suite: `windows-2025` bindet und verbindet
  `127.<b1>.<b2>.<b3>`.
- **Port**: gleicher Port wie am Host. Scheitert das Binden (Port belegt oder Linux-Wildcardbelegung
  desselben Ports), wählt der Dienst einen freien Port auf derselben Adresse; die Liste zeigt dann
  „<Hostport> → <Adresse>:<Ersatzport> (Port belegt)“. Kein Abbruch, keine Rückfrage.
- **Zeitpunkt**: Desktop (Windows, Linux): Listener entstehen, sobald der Gerätefeed eine Dienstliste
  liefert, und verschwinden mit dem Dienst, beim Offline-Gehen des Geräts oder beim Freigabeende.
  Der QUIC-Tunnel öffnet sich erst bei der ersten angenommenen Verbindung und schließt, wenn die
  letzte lokale Verbindung endet (ereignisgesteuert, kein Leerlauftimer). Android: Listener entstehen
  erst auf Tippen „Öffnen“ am Dienst und bleiben bis „Schließen“, Offline oder Freigabeende (Grund: 4.4).
- **Anzeige**: Geräteseite (Desktop und Android) und se-Desktop-Kachel zeigen je Gerät
  „Dienste: <Prozessname> – http://<Adresse>:<Port>“; für Port 80/443/8000–8999/3000–3999/5000–5999
  wird `http://` als Link angeboten, sonst `<Adresse>:<Port>` zum Kopieren.
- **Grenze**: `MAX_TCP_TUNNELS` = 64 gleichzeitig offene Tunnel (`N/share/core/tcp_forward.rs:12`)
  bleibt; Listener ohne offenen Tunnel zählen nicht. Bei Erschöpfung wird die neue lokale Verbindung
  geschlossen und `last_error` „TCP-Grenze 64 erreicht“ gesetzt.

### 4.4 Annahmeregel statt Bestätigung (E2-Ersatz)

Heute verlangt jeder Start `acknowledge_local_access` (`N/share/core/tcp_forward.rs:132-135,145-147`),
und der Listener nimmt jede Verbindung an (`N/share/core/node_tcp_link.rs:238-243`). Neu prüft der
Listener jede Verbindung nach `ListenerAdmission` (9.3):
- `Process { pid, started }` — RDP (2.7).
- `SameAccount` — automatische Abbildungen und `se tcp start`: Konto des verbindenden Prozesses =
  Konto des Dienstes. Linux: UID-Spalte der passenden `/proc/net/tcp{,6}`-Zeile; ein Root-Dienst nimmt
  UID 0 und die UID der aktiven Sitzung auf `seat0` (logind) an. Windows: `dwOwningPid` →
  Token-Benutzer-SID = SID des Dienstes.
- `AnyLocal` — nur Android (keine Ermittlung möglich, 4.1); deshalb dort nur Listener auf Tippen.
Das Wirefeld `acknowledge_local_access` bleibt für ältere Hosts erhalten und wird vom Betrachter
immer `true` gesendet (der Host prüft es in `node_tcp.rs:122`); Bedeutung ab Version 2 in 9.3.

### 4.5 Manuelle Weiterleitung bleibt

Ziele außerhalb von Loopback (Host-LAN, DNS-Namen) und nicht freigegebene Ports bleiben beim
bestehenden Recht `TcpForward` mit Regeln (`N/share/core/access_constraints.rs:180-211`) und dem
Formular `N/app/core/share_tcp_ui.rs` (Inventur E1, E3). Geändert: Lebensdauer leer = ohne Frist,
kein Bestätigungskästchen, Annahmeregel `SameAccount`, keine 30-s-Wartefrist
(`share_tcp_ui.rs:191`). Grund für den Verbleib: die Weitergabe des Host-LANs ist eine andere
Vertrauensstufe als die Sitzung des Hosts (Pivot ins fremde Netz).

### 4.6 Protokoll Version 2 und alte Geräte

- Neue ALPN `smart-explorer/share-tcp/2`; der Betrachter verbindet mit `/2` und bietet `/1` zusätzlich
  an (`ConnectOptions::with_additional_alpns`, `vendor/iroh-1.3.0/src/endpoint.rs:1852-1853`); der
  Host registriert beide (`N/share/core/node.rs:160-167`) und verzweigt in `node_accept.rs:54,184`.
- `/2`: `TcpServerHello.version = 2`; `check_server` (`node_tcp_link.rs:182`) akzeptiert die zur ALPN
  passende Version; `TcpStart::validate` erlaubt `lifetime_secs = 0` nur unter `/2`.
- `/1` (Host bis 0.5.188): Betrachter sendet `lifetime_secs = 86400` (Höchstwert alter Hosts) und zeigt
  „Gegenstelle älter als 0.5.189: Weiterleitung endet nach 24 h“.

## 5. Fenster anderer Geräte (r5)

### 5.1 Befund heute (aus dem Code)

- **Host**: Die Fensterliste beantwortet der Hintergrunddienst über den Bildschirm-Metadatenkanal
  (`ViewerFrame::ListFamilies` → `Request::Families`, `N/screen/core/icon_host.rs:86-87`;
  `host_session_control.rs:10-21`). Die se-Oberfläche oder se-Desktopfläche des Hosts ist dafür nicht
  nötig. Schluss: hostseitig ist r5 bereits erfüllt; Bedingung ist das Recht „Bildschirm ansehen“,
  das „Steuern“ einschließt (`access_steering.rs:34-37`).
- **Betrachter**: Die se-Desktopfläche zeigt keine fremden Fenster. Ihr Modell enthält Kacheln,
  Dateien, Systemeinträge (`N/desktop/core/protocol.rs:35-43`); eine Gerätekachel trägt keine
  Fensterliste (`N/desktop/core/tiles.rs:96-116`); das Kachelmenü bietet „Bildschirm ansehen/
  steuern“ und „Programme auf diesem Gerät …“ (`N/app/core/desktop_surface_menu_ui.rs:135-145`).
  Offene Fenster erscheinen nur nach Klick „Fensterliste abrufen / aktualisieren“ auf der
  Geräteseite (`N/app/core/family_picker.rs:50`) bzw. mit `se screen families`; das Kästchen
  „Fenster mit Steuerrecht öffnen“ (`family_picker.rs:56`) ist eine weitere Wahl. Abruf je Klick über
  einen neuen Kanal (`N/daemon/os/shared/screen_ipc_families.rs:22-37`).
- Der Host liest Fensterzustände abfragend, nicht ereignisgesteuert
  (`N/remote_window/INTEGRATION.md`: „Ein Pollingadapter wurde … gewählt“).

### 5.2 Entwurf: Gerätefeed

- **Kanal**: Bildschirm-ALPN, Metadatensitzung wie heute (`Relayed::open_for_node(…, true, …)`,
  `screen_ipc_families.rs:28`). Neues Hello-Feld `family_feed_version: Option<u16>` (=1) in
  `ScreenServerHello`/`ScreenClientHello`/`ScreenHelloOk` (`N/screen/core/wire_hello.rs:85-99,138-150,256-262`;
  keine `deny_unknown_fields`, also abwärtsverträglich). Neue Rahmen `SUBSCRIBE_FEED = 45`
  (Betrachter → Host) und `FEED_UPDATE = 46` (Host → Betrachter) in `N/screen/core/wire.rs`
  (Codes 45–59 sind frei, `wire.rs:25-60`).
- **Host** (`N/screen/core/feed_host.rs`, neu): Nach `SUBSCRIBE_FEED` tastet der Host einmal je
  Sekunde ab — Fensterfamilien über denselben Weg wie `Request::Families`, Dienste über 4.1, RDP-
  Kurzstand aus dem letzten Probe-/Einrichtungsergebnis — und sendet `FEED_UPDATE` nur bei Änderung
  (vollständiger Stand, Grenze `MAX_WINDOWS_BYTES` 1 MiB, `wire.rs:66-69`). Die Abtastung ist eine
  Rate, keine Frist: Windows und Linux bieten keine Benachrichtigung für neue lauschende Sockets
  (Schluss: keine solche API in `iphlpapi` bzw. kein sock_diag-Multicast für LISTEN), und der
  Fensteradapter ist ohnehin abfragend (5.1). Rechte je Sendung: Fenster nur bei „Bildschirm
  ansehen“ = Erlauben, Dienste und RDP-Stand nur bei `steers`. Entzug beendet die Sitzung.
  Der 15-s-Empfangstimeout der Metadatensitzung (`icon_host.rs:76`) gilt nach `SUBSCRIBE_FEED` nicht
  mehr; Toterkennung übernimmt QUIC (20 s).
- **Betrachter** (`N/share/os/shared/peer_feed.rs`, neu, im Hintergrunddienst): je bekanntem Gerät mit
  anwesender Relation (2.2) eine Feed-Sitzung. Erneuter Versuch nach Ablehnung oder Abbruch nur bei
  einem Anwesenheitsereignis dieses Geräts oder einer neuen Profilrevision (kein Timer). Ergebnis im
  `ShareWorkerSnapshot` als `peer_feeds: Vec<PeerFeedView>` mit `#[serde(default)]`
  (`N/daemon/os/shared/ipc_protocol.rs:41-71`). Alter Host ohne `family_feed_version`:
  `PeerFeedView.unavailable = "Gerät hat eine ältere Version"`.
- **se-Desktop**: Die Fläche liest den Snapshot bereits alle 900 ms (`N/desktop/os/shared/model_feed.rs:28,62`).
  `DeviceTile` erhält `windows: Vec<TileWindow>` (Titel, App, Familienquelle) und
  `services: Vec<TileService>`; die Kachel zeigt die Fenster als Unterzeilen; ein Doppelklick öffnet
  den Familienbetrachter mit Steuerrecht, wenn das Recht besteht, sonst ansehend (das Kästchen
  `family_picker.rs:56` entfällt). Kachelmenü: „Remote Desktop“ und je RemoteApp ein Eintrag, wenn der
  RDP-Kurzstand `startable` oder `setup_possible` meldet; Start über verborgenes
  `se rdp open <gerät>` als Besitzerprozess (die Fläche startet ohnehin abgelöste Prozesse,
  `N/app/core/desktop_surface_actions.rs:1-9`).
- **Geräteseite Desktop/Android**: zeigen dieselbe Liste ohne Abrufknopf.
- **CLI/MCP**: `se screen families <gerät>` und MCP `screen_window` liefern den letzten Feedstand,
  wenn vorhanden, sonst den bisherigen Einzelabruf.
- Plattformgrenzen bleiben: Wayland-Host nur Portalauswahl, Android-Host eine App
  (`docs/TODO.md` DESKTOP-ALLE-PROGRAMME); der Feed meldet sie als `families_unavailable`.

## 6. Programme (F1, F4, F5, F6)

- **F1 Katalog**: Der Katalog lädt beim ersten Anzeigen der Programme eines Geräts und bei jeder
  Änderung der Katalogrevision automatisch (`jobs::catalog`, heute nur auf Klick,
  `N/app/core/share_apps_ui.rs:156-161,328-335`). Der Knopf heißt „Neu laden“ und bleibt nur als
  Handlung. Die 30-s-Wartefrist (`N/app/core/share_apps_jobs.rs:64`) entfällt (`None`).
- **F4 Beleg und neuer Auftrag**: Nach `OutcomeUnknown` fragt der Betrachter denselben Beleg
  (`AppsCommand::Receipt`, `share_apps_jobs.rs:158-180`) automatisch erneut: sofort, dann bei jedem
  Anwesenheitsereignis des Hosts und bei jeder Feedänderung der Fensterliste — ereignisgesteuert,
  ohne Takt. Ist der Beleg endgültig (`Started`, `Activated`, `Denied`, `Unsupported`), endet das
  Nachfragen. Das Kästchen „neuen Auftrag erstellen“ (`share_apps_ui.rs:276-279`) entfällt: ein
  weiterer Klick „Starten“ erstellt nach endgültigem Beleg einen neuen Auftrag; solange der Beleg
  unklar ist, ist „Starten“ gesperrt mit Text „Startergebnis wird geklärt“. Grund: die
  Doppelstartsperre bleibt, die Klärung macht se statt der Person.
- **F5 Zustimmung**: Desktop-Hosts liefern `ConsentRequired` nur bei gesperrter Sitzung
  (`N/apps/os/windows/launch.rs:66-70`, `N/apps/os/linux_os/launch.rs:51-55`). Neuer Text:
  „Hostsitzung gesperrt – zum Entsperren den Bildschirm öffnen“ mit einer Handlung „Bildschirm
  steuern“. Meldet der Feed danach eine entsperrte Sitzung (Katalogfeld `session.unlocked`), startet
  der Betrachter denselben Programmeintrag mit neuer Request-ID automatisch, weil `ConsentRequired`
  belegt nicht gestartet ist. Android-Host (`N/apps/os/host.rs:70-72`, F7) behält die lokale
  Zustimmung (Android-Hintergrundstartregeln); Text unverändert.
- **F6 Fenster öffnen**: Enthält der Beleg ein bestätigtes Fenster (`confirmed`,
  `share_apps_ui.rs:378-400`) und ist ein Öffner registriert, öffnet der Betrachter es sofort; der Knopf
  „Bestätigtes Appfenster öffnen“ (`share_apps_ui.rs:184-198`) entfällt.
- Android `AppsDialog.kt` und Bridge `apps.*` (`N/mobile/os/shared/domains/apps.rs:80`) folgen
  denselben Regeln.

## 7. Zeitgrenzen-Inventur (r7)

| Stelle | Wert | Entscheidung | Grund |
|---|---|---|---|
| `N/rdp/core/admission.rs:35-37` | Probe 300 s alt | ersetzen | eine Probe je Start (2.1) |
| `N/rdp/core/admission.rs:85-89` | RDP-Lease 900 s | ersetzen | prozess-/verbindungsgebunden (2.6) |
| `N/share/core/tcp_forward.rs:136-141,148-150` | TCP 900 s, 1–86400 | ersetzen | 0 = ohne Frist unter `/2` (4.6) |
| `N/share/core/node_tcp.rs:217,227` | Host-TTL | nur bei `lifetime_secs > 0` | ausdrücklich gewählte Frist bleibt möglich (CLI `--lifetime-secs`) |
| `N/share/core/node_tcp_link.rs:227,234` | Betrachter-TTL | wie oben | – |
| `N/rdp/os/shared/session.rs:67-69` | Einrichtungswarten 1–300 s | ersetzen | `Option`, Vorgabe ohne Frist |
| `N/app/core/share_rdp_ui.rs:222,282` | 30 s Probe/Start | ersetzen | `None` |
| `N/app/core/share_tcp_ui.rs:191` | 30 s | ersetzen | `None` |
| `N/app/core/share_apps_jobs.rs:64,130,147,174` | 30 s | ersetzen | `None` |
| `N/app/core/share_apps_jobs.rs:286` | 15 s Symbolabruf | ersetzen | `None`; Abbruch beim Seitenwechsel bleibt |
| `N/cli/rdp.rs:51-52`, `N/cli/mcp/rdp_tool.rs:23`, `N/cli/mcp/tcp_tool.rs:67`, `N/mobile/os/shared/domains/rdp.rs:155`, `tcp.rs:162`, `apps.rs:80` | 30 s Vorgaben | ersetzen | Parameter bleibt optional, ohne Vorgabe |
| `N/share/core/keepalive.rs:6-8` | Keepalive 5 s, Leerlauf 20 s | behalten | einzige Toterkennung der Verbindung |
| `N/share/core/node_tcp_link.rs:39-44` | Verbindungsaufbau 30 s | behalten | Toterkennung beim Wählen; ohne Grenze hinge ein unerreichbares Gerät endlos |
| `N/share/core/node_tcp.rs:78`, `node_tcp_link.rs:102,121,256-261`, `node_tcp.rs:284,310-313` | 20–30 s je Protokollschritt | behalten | gelten auf aktiver Route je Schritt (`route_protocol::run_until_active`), Toterkennung des Handshakes |
| `N/share/core/node_tcp.rs:196-202` | 2 s Ablehnung senden | behalten | Höflichkeitsversand auf dem Fehlerpfad |
| `N/share/os/shared/tcp_sockets.rs:14` | Zielverbindung/DNS 10 s | behalten | ein SYN ins Leere hinge sonst die OS-Vorgabe lang; Einrichtungsschritt, keine Sitzungsfrist |
| `N/rdp/os/shared/process.rs:43-51` | Inventarwerkzeug 10 s | ersetzen (Suite-Lauf 40774661103) | die PowerShell-Inventur brauchte auf einem kalten Windows-Rechner länger als 10 s und ließ H4 scheitern; das Werkzeug endet selbst (D-Bus- und CIM-Aufrufe haben eigene Protokollfristen), begrenzt bleibt nur die Ausgabe (128 KiB, darüber wird es beendet) |
| `N/daemon/os/shared/apps_ipc_client.rs:173` | lokales IPC 30 s | behalten | Schutz gegen einen hängenden lokalen Dienst; kein Peerwarten |
| `N/screen/core/icon_host.rs:76` | Metadaten 15 s | behalten für Einzelabruf, entfällt nach `SUBSCRIBE_FEED` | Feed nutzt QUIC-Toterkennung |
| `N/rdp/os/shared/session_owners.rs:236`, `session.rs:149-171` | 1 s/100 ms Takt | behalten | Beobachtungstakt, keine Frist |
| `N/desktop/os/shared/model_feed.rs:28` | 900 ms Takt | behalten | Anzeigetakt |
| Gerätefeed 1 s (neu, 5.2) | Abtastrate | neu | keine Ereignisquelle für Sockets |

## 8. Sicherheitsreview

Bedrohungen und Gegenmaßnahmen; „per default“ bleibt ohne Menü.

| Nr. | Bedrohung | Gegenmaßnahme |
|---|---|---|
| T-L1 | Freigegebene localhost-Dienste ohne eigene Anmeldung (Debug-Ports, Datenbanken im Vertrauensmodus) werden für berechtigte Geräte erreichbar, auch bei gesperrter Hostsitzung | Träger ist allein „Bildschirm steuern: Erlauben“ (gleiche Reichweite wie die bediente Sitzung, 4.2); Entzug beendet alle Tunnel sofort (`node_tcp.rs:211,225,252`); Host zeigt laufende Zieltunnel (Rolle `Target`) auf „Dieses Gerät“ |
| T-L2 | Wildcard-Systemdienste (RPC, SMB, Anmelde-/Druckdienste) als Ziel von Relay-/Zwangsauthentisierungsangriffen über den Tunnel, an der Host-Firewall vorbei | ausgenommen: Wildcard nur bei normalem Benutzerkonto; nicht ermittelbarer Besitzer = System |
| T-L3 | se-eigene Loopbackdienste (Dienst-IPC, Anhängedienst, Updater-Handshake) würden erreichbar | ausgenommen über IPC-Port und Bild-Pfad (4.2) |
| T-L4 | Andere lokale Konten oder Prozesse am Betrachter erreichen die Abbildungen | `SameAccount` bzw. `Process`-Annahme (4.4); Android nur auf Tippen |
| T-L5 | Raummitglieder (fremde Personen) | gleiche Regel: nur, wenn der Host diesem Mitglied „Bildschirm steuern: Erlauben“ gegeben hat; Raumrechte bleiben getrennt |
| T-R1 | Automatisches Einschalten von RDP öffnet einen Netzdienst | Windows: keine Firewallregel, Erreichbarkeit nur über Loopback/Tunnel; NLA wird bei Neuaktivierung erzwungen; xrdp nur `tcp://.:<p>` |
| T-R2 | GNOME-Desktopfreigabe lauscht an allen Schnittstellen | Zugangsdaten zufällig (256 Bit), je Abruf und bei Entzug neu gesetzt, nie gespeichert; ein entzogenes Gerät kennt danach kein gültiges Kennwort |
| T-R3 | Fremder Prozess am Betrachter fängt RDP ab (Anmeldedaten) | PID+Startzeit-Bindung (2.7.1); `CertHash` aus authentifizierter Probe; FreeRDP-Pin |
| T-R4 | Fremder Prozess am Host belegt den RDP-Port | Dienstbesitzprüfung je Verbindung (2.7.4) |
| T-R5 | Erhöhte Ausführung mit fremden Daten | Skripte fest im Binärprogramm, nur SID bzw. feste Schrittbezeichner als Parameter; `pkexec` nur auf root-eigenes se |
| T-R6 | Spontane UAC-/polkit-Abfragen als Täuschungsfläche | nur bei T1–T4; je Version höchstens einmal ohne Nutzerhandlung; Statuszeile „Remote Desktop wird eingerichtet (Administratorfreigabe)“ |
| T-R7 | Gespeicherte RDP-Kennwörter am Betrachter | nur durch die Person im OS-Dialog („Anmeldedaten speichern“, DPAPI-geschützt); se-verwaltete GNOME-Daten nur sitzungsweise (`CRED_PERSIST_SESSION`), beim Ende gelöscht |
| T-W1 | Fensterliste verrät Titel (Dokumentnamen) dauerhaft | gleiche Bedingung wie heute (Bildschirm ansehen); Feed endet mit dem Recht |
| T-D1 | Dauerhafte Feed-Verbindungen erhöhen Angriffs- und Energieaufwand | eine Sitzung je Gerät und nur bei Anwesenheit; Android nur, solange der App-Prozess läuft |

Restrisiken (ausdrücklich): Annahmen A4–A8; ein Dienst, der nur Loopback-Clients vertraut, wird für
berechtigte Geräte nutzbar (gewollt, r5).

## 9. Verträge

### 9.1 Kern- und OS-Funktionen (Rust)

```rust
// N/share/core/peer_select.rs (neu)
pub(crate) fn present_relation(profiles: &ShareProfiles, device_id: &str, public_key: &str)
    -> Option<PeerOpenTarget>;

// N/share/core/loopback_services.rs (neu, plattformneutral)
pub(crate) enum BindScope { LoopbackOnly, Wildcard }
pub(crate) enum OwnerClass { NormalUser { uid_or_sid: String }, System, Unknown }
pub(crate) struct LoopbackService { pub port: u16, pub scope: BindScope, pub owner: OwnerClass,
    pub process: Option<String>, pub pid: Option<u32> }
pub(crate) fn exposable(s: &LoopbackService, excluded_ports: &BTreeSet<u16>) -> bool;
// N/share/core/access_steering.rs
pub(crate) fn note_exposed_ports(ports: impl IntoIterator<Item = u16>);

// N/share/os/{windows/tcp_table.rs, linux_os/proc_net_tcp.rs} (neu), Fassade crate::share::os_sockets
pub(crate) struct ListenerRow { pub local: SocketAddr, pub pid: Option<u32>, pub owner: OwnerClass }
pub(crate) fn listeners() -> io::Result<Vec<ListenerRow>>;
pub(crate) fn connection_owner(local: SocketAddr, remote: SocketAddr) -> io::Result<Option<u32>>;
pub(crate) fn connection_account(local: SocketAddr, remote: SocketAddr) -> io::Result<Option<String>>;
pub(crate) struct ProcessIdentity { pub pid: u32, pub started: u64, pub image: Option<PathBuf> }
pub(crate) fn process_identity(pid: u32) -> io::Result<ProcessIdentity>;
pub(crate) fn process_name(pid: u32) -> Option<String>;

// N/share/core/device_address.rs (neu)
pub(crate) fn device_loopback(device_id: &str, public_key: &str) -> io::Result<Ipv4Addr>;

// N/rdp/os/shared/self_setup.rs (neu)
pub(crate) enum SetupTrigger { Grant, VersionStart, PeerOnline, PeerRequest }
pub(crate) enum SetupState { Done, Declined, Impossible(String), Failed(String), NotNeeded }
pub(crate) fn ensure_rdp_host(trigger: SetupTrigger) -> SetupState;

// N/rdp/mod.rs
pub(crate) fn open_device(device_id: &str, public_key: &str, remote_app: Option<&str>)
    -> Result<SessionView, String>; // CLI wartet als Besitzer; GUI/MCP übergeben an Registry
```

### 9.2 Apps-Draht (`N/share/core/apps_wire.rs:18-41`)

Neue Varianten (alte Hosts antworten mit Fehler „unknown variant“ → Betrachter meldet
„Gegenstelle älter als 0.5.189“):
- `AppsCommand::RdpProbeV2` → `AppsResult::RdpProbeV2 { host: HostProbe, setup: SetupState }`;
  `HostProbe.version = 2`; `HostService` erhält `certificate_sha1: Option<CertificateSha1>`
  (`#[serde(default, skip_serializing_if = "Option::is_none")]`) und `managed_credentials: bool`
  (`#[serde(default)]`). Auf `RdpProbe` (v1) antwortet der Host weiterhin mit Version 1 ohne die neuen
  Felder. `HostProbe::validate` (`N/rdp/core/types.rs:153-164`) nimmt 1 und 2 an.
- `AppsCommand::RdpPrepare` → `AppsResult::RdpPrepare { setup: SetupState, host: HostProbe }`.
- `AppsCommand::RdpCredential { service_id: String }` → `AppsResult::RdpCredential { username: String,
  password: String }`; nur bei `steers` und `managed_credentials`.
- `AppsCommand::LocalServices` → `AppsResult::LocalServices { services: Vec<LoopbackService>,
  unavailable: Option<String> }`; nur bei `steers` (für CLI/MCP ohne Feed).
Zulassung: `RdpProbeV2` wie `RdpProbe` über `require_inventory` (`N/share/core/node_apps_host.rs:148-162`);
die übrigen über `steers`.

### 9.3 TCP-Draht und Dienst-IPC

- ALPN `/2` (4.6). `TcpStart` unverändert in der Form; Semantik unter `/2`: `lifetime_secs = 0` =
  ohne Frist; `acknowledge_local_access` = „Listener prüft Herkunft nach `ListenerAdmission`“.
- `TcpIpcRequest::Start` (`N/daemon/os/shared/ipc_protocol_tcp.rs:9-13`) erhält
  `#[serde(default)] admission: ListenerAdmission` und `#[serde(default)] bind: Option<Ipv4Addr>`
  (Geräteadresse). `ListenerAdmission = SameAccount (Vorgabe) | AwaitProcess | AnyLocal`.
- Neue Anfrage auf demselben Strom: `TcpIpcRequest::BindProcess { id: TunnelId, pid: u32, started: u64 }`
  → `TcpIpcReply::Bound`.
- `TunnelView` (`N/share/core/tcp_forward.rs:165-185`): `expires_at` wird `Option<i64>`
  (`None` = ohne Frist); neues Feld `admission`.

### 9.4 Bildschirmdraht

`family_feed_version` (5.2); `SUBSCRIBE_FEED` ohne Nutzlast; `FEED_UPDATE` mit JSON
`{ "families": FamilyListing | null, "families_unavailable": string | null,
   "services": [LoopbackService] | null, "services_unavailable": string | null,
   "rdp": { "startable": bool, "setup": SetupState, "remote_apps": [PublishedApp] } | null }`.

## 10. Arbeitspakete (höchstens drei, ein Schreiber je Datei)

Reihenfolge der Abhängigkeiten: Verträge in Abschnitt 9 sind fest; WP1, WP2, WP3 arbeiten parallel
gegen diese Signaturen. Die Suite-Dateien (`.github/workflows/…`, `native/test-…`) schreibt der
Hauptagent; jedes Paket liefert seine Rusttests mit Präfix `einrichtung_task_` in eigenen Testdateien.

### 10.1 WP1 Host (Einrichtung, Ermittlung, Rechte, Hostseite der Protokolle)

Schreibt: `N/share/core/access_steering.rs`, `N/share/core/loopback_services.rs` (neu),
`N/share/core/peer_select.rs` (neu), `N/share/core/apps_wire.rs`, `N/share/core/node_apps_host.rs`,
`N/share/core/node.rs`, `N/share/core/node_accept.rs`, `N/share/core/node_tcp.rs`,
`N/share/core/tcp_forward.rs`, `N/share/core/configuration_runtime.rs`,
`N/share/os/shared/tcp_sockets.rs`, `N/share/os/windows/tcp_table.rs` (neu),
`N/share/os/linux_os/proc_net_tcp.rs` (neu), Fassade-Eintrag in `N/share/os/mod.rs` bzw. dem
bestehenden OS-Wähler, `N/rdp/core/types.rs`, `N/rdp/os/windows/probe.ps1`, `N/rdp/os/windows/probe.rs`,
`N/rdp/os/windows/host_setup.rs` und `host_setup.ps1` (neu), `N/rdp/os/linux_os/{xrdp,gnome,krdp,inventory}.rs`,
`N/rdp/os/linux_os/host_setup.rs` (neu), `N/rdp/os/shared/self_setup.rs` (neu), `N/screen/core/wire.rs`,
`N/screen/core/wire_hello.rs`, `N/screen/core/wire_control.rs`, `N/screen/core/wire_control_codec.rs`,
`N/screen/core/icon_host.rs`, `N/screen/core/feed_host.rs` (neu), `native/Cargo.toml` (`rcgen` direkt;
`windows-sys`-Features `Win32_NetworkManagement_IpHelper`, `Win32_Security_Credentials`).
Liest zusätzlich: `N/net/os/windows/uplink_install.rs`, `N/screen/core/host_metadata.rs`.
Testdateien: `N/share/core/einrichtung_host_task_tests.rs`, `N/rdp/os/shared/einrichtung_setup_task_tests.rs`.

Erwartete Ergebnisse in der Suite:
- H1 (beide OS): Host mit Python-HTTP-Dienst auf 127.0.0.1:<p> und einem auf 0.0.0.0:<q> als
  Testkonto meldet `LocalServices` mit `<p>` (Prozessname `python`) und `<q>`; der IPC-Port des
  Dienstes erscheint nicht; unter Windows erscheinen 135 und 445 nicht.
- H2: `access_steering`-Einheitstest: steuernde Relation erhält `Allow` für freigegebene Ports,
  `Off` für andere Ports, für HostToPeer und für eine Relation mit `ScreenControl = Ask`.
- H3 (ubuntu-24.04, root): `se rdp host-setup --system install-xrdp` und `configure-xrdp` → `ss -ltn`
  zeigt `127.0.0.1:3389` und kein `0.0.0.0:3389`/`[::]:3389`; `se rdp local-probe --json` meldet `xrdp`
  startbar.
- H4 (windows-2025, erhöht): Host-Setupskript → `fDenyTSConnections = 0`, `TermService` läuft,
  Firewallregelgruppe „Remote Desktop“ unverändert gegenüber vorher; Probe v2 meldet startbar mit
  `certificate_sha1` gleich `Win32_TSGeneralSetting.SSLCertificateSHA1Hash`.
- H5: TCP `/2` mit `lifetime_secs = 0`: Tunnel lebt nach 960 s Leerlauf mit Keepalive weiter
  (`expires_at = null`); Abbruch des Host-Dienstes schließt den Betrachtertunnel nach ≤ 25 s.
- H6: Kandidat als Betrachter gegen veröffentlichten 0.5.188-Host: Verbindung über `/1`,
  `lifetime_secs = 86400`, Meldung „Gegenstelle älter als 0.5.189“.
- H7: Feed: Xvfb-Host mit `xmessage`-Fenster; Betrachter-Snapshot enthält den Fenstertitel, ohne dass
  Betrachter-GUI oder Desktopfläche laufen; nach Schließen verschwindet er aus dem nächsten Update.

### 10.2 WP2 Betrachterkern (Listener, Clients, Besitz, Feed-Verbraucher, Abbildung)

Schreibt: `N/share/core/node_tcp_link.rs`, `N/share/core/tcp_registry.rs`,
`N/share/core/device_address.rs` (neu), `N/share/os/shared/peer_feed.rs` (neu),
`N/share/os/shared/auto_forward.rs` (neu), `N/daemon/os/shared/ipc_protocol_tcp.rs`,
`N/daemon/os/shared/tcp_ipc.rs`, `N/daemon/os/shared/tcp_ipc_client.rs`,
`N/daemon/os/shared/ipc_protocol.rs` (nur `ShareWorkerSnapshot.peer_feeds`),
`N/daemon/os/shared/apps_ipc_client.rs`, `N/rdp/mod.rs`, `N/rdp/core/admission.rs`,
`N/rdp/os/shared/{session,session_owners,clients}.rs`, `N/rdp/os/shared/open_device.rs` (neu),
`N/rdp/os/windows/{clients,profile}.rs`, `N/rdp/os/windows/trust.rs` (neu: `CertHash`, `CredWriteW`),
`N/rdp/os/linux_os/clients.rs`.
Liest zusätzlich: WP1-Fassaden aus 9.1 (nur Signaturen).
Testdateien: `N/share/core/einrichtung_link_task_tests.rs`, `N/rdp/os/shared/einrichtung_open_task_tests.rs`.

Erwartete Ergebnisse:
- V1 (ubuntu-24.04): RDP-Listener mit `AwaitProcess`, gebunden an einen gestarteten Testprozess:
  dessen Verbindung wird weitergeleitet; eine `nc`-Verbindung wird geschlossen und `last_error` nennt
  die fremde PID.
- V2 (beide OS): automatische Abbildung eines Hostdienstes auf `Geräteadresse:<p>`; `curl` desselben
  Kontos erhält den Inhalt; unter Linux erhält `sudo -u nobody curl` einen Verbindungsabbruch.
- V3: belegter Port am Betrachter → Ersatzport auf derselben Adresse, im Snapshot vermerkt.
- V4 (ubuntu-24.04, Xvfb, xrdp aus H3, Testkonto): `se rdp open <host> --json` ohne weitere Schalter
  startet `sdl-freerdp3` bzw. `xfreerdp3`; xrdp-Protokoll meldet erfolgreiche Anmeldung; Beenden des
  Clients schließt den Tunnel (`tcp list` leer).
- V5 (windows-2025, Paar auf einem Rechner, Testkonto mit Kennwort, `CredWriteW` aus dem Test):
  `se rdp open <host>` erreicht eine Sitzung (`qwinsta` zeigt `rdp-tcp#…` aktiv) und der mstsc-Prozess
  hat bis dahin kein Fenster der Klasse `#32770` (prüft A5, A6, A8).
- V6: `AuthenticatedProbe` wird von `start` verbraucht; zweiter Start ohne neue Probe ist ein
  Kompilierfehler bzw. Einheitstest-Nachweis über die API.

### 10.3 WP3 Oberflächen und Dokumentation

Schreibt: `N/app/core/share_rdp_ui.rs`, `N/app/core/share_rdp_form.rs` (entfernen),
`N/app/core/share_rdp_sessions.rs`, `N/app/core/share_tcp_ui.rs`, `N/app/core/share_apps_ui.rs`,
`N/app/core/share_apps_jobs.rs`, `N/app/core/family_picker.rs`, `N/app/core/share_device_page_ui.rs`,
`N/desktop/core/tiles.rs`, `N/desktop/os/shared/model_feed.rs`, `N/app/core/desktop_surface_menu_ui.rs`,
`N/app/core/desktop_surface_tile_ui.rs`, `N/desktop/os/shared/open_target.rs`, `N/cli/rdp.rs`,
`N/cli/tcp.rs`, `N/cli/screen/families.rs`, `N/cli/mcp/rdp_tool.rs`, `N/cli/mcp/tcp_tool.rs`,
`N/cli/mcp/catalog.rs`, `N/mobile/os/shared/domains/{rdp,tcp,apps,app_launch,screen_view}.rs`,
`A/api/{RdpApi,TcpApi,AppsApi,FamilyApi}.kt`, `A/ui/share/{RdpDialog,TcpDialog,AppsDialog}.kt`,
`A/ui/screen/FamilyPicker.kt`, `docs/oberflaechen/fernzugriff.md`, `N/rdp/README.md`, `docs/TODO.md`
(Zeile SELF-SETUP), `docs/GOTCHAS.md` (neue Einträge zu A4–A8 nach Suite-Ergebnis).
Testdateien: `N/app/core/einrichtung_ui_task_tests.rs`, `N/cli/einrichtung_cli_task_tests.rs`.

Änderungen je Oberfläche:
- Desktop-GUI: Geräteseite „Remote Desktop“ = ein Knopf plus RemoteApp-Knöpfe, Sitzungszeile mit
  Hinweis aus 2.7.5, Einrichtungsstand aus `setup`; Formular entfällt. TCP: Liste der Dienste (4.3),
  darunter das bisherige Formular nur für Nicht-Loopback-Ziele, ohne Lebensdauer- und
  Bestätigungsfeld. Programme nach Abschnitt 6. Fenster: Liste aus dem Feed ohne Abrufknopf und ohne
  Steuerrecht-Kästchen; „Dieses Gerät“ zeigt Einrichtungsstand und laufende Zieltunnel.
- se-Desktop: Kachel mit Fenster- und Dienstzeilen, Menü „Remote Desktop“/RemoteApps (5.2).
- Android: keine RDP-Handlung (Grund aus `rdp.local_clients`), Dienste mit „Öffnen/Schließen“
  (4.3), Fensterliste aus dem Feed, Programme nach Abschnitt 6.
- CLI: `se rdp open <gerät>`, `se tcp start` ohne Pflicht zu `--acknowledge-local-access` und mit
  Vorgabe ohne Frist, `se tcp services <gerät>` (neu, `LocalServices`), `se screen families` aus dem Feed.
- MCP: `rdp` (2.9), `tcp_forward` mit `action: services` und ohne Pflichtbestätigung, Katalogtexte.

Erwartete Ergebnisse:
- U1: Einheitstest der Geräteseite: Zustand „Probe startbar“ rendert genau einen Knopf
  „Remote Desktop“ und je veröffentlichtem Alias einen Knopf; kein Kästchen, kein Textfeld.
- U2: `se rdp open --help` nennt nur `<DEVICE>`, `--remote-app`, `--json`; verborgene Altschalter
  werden angenommen (Parser-Test).
- U3: MCP-Schema `rdp` hat `required: ["action"]` und `open` gelingt mit nur `device`
  (Einheitstest gegen `rdp_tool::rdp` mit injizierter Probe).
- U4: Programme: Zustand `OutcomeUnknown` löst ohne Klick eine `Receipt`-Anfrage aus; endgültiger
  Beleg mit Fenster ruft den Öffner ohne Klick.
- U5: Android-Bridge: `tcp.start` ohne `acknowledgeLocalAccess` und ohne `lifetimeSecs` gelingt
  (Einheitstest der Argumentauswertung), `rdp.open` meldet den fehlenden Client.

### 10.4 Gemeinsame Ende-zu-Ende-Abnahme (Hauptagent, eine Suite)

Die Ergebnisse H1–H7, V1–V6, U1–U5 laufen in der einen Batch-Suite auf `ubuntu-24.04` und
`windows-2025`, Zeitlimit des Jobs ≥ 30 min. GNOME-Desktopfreigabe und KRdp sind auf den Runnern
nicht prüfbar (keine GNOME/Plasma-Sitzung); ihre Befehlsfolgen werden nur als Einheitstests der
erzeugten Befehle und stdin-Inhalte geprüft und im Abschlussbericht als ungeprüft markiert.

## 11. Erhaltenes Verhalten (AGENTS.md „preserve existing behavior“)

- Manuelle TCP-Regeln mit `Ask`/`Allow`, DNS-/IP-Ziele, `HostToPeer`-Trennung
  (`access_constraints.rs:180-211`) bleiben unverändert wirksam.
- Ausdrücklich gewählte TCP-Lebensdauer (`--lifetime-secs`, `lifetimeSecs`) bleibt möglich.
- RDP-Besitz, Exitgate und Drain (`N/app/os/shared/rdp_exit_gate.rs`, `session_owners.rs:266-323`)
  bleiben; Schließen der App beendet eigene Clients wie bisher.
- Bestehende RDP-Konfigurationen am Host (NLA, Firewall, netzweites xrdp ohne se-Marker) bleiben.
- Befehle und Terminal bleiben eigene Rechte (`access_steering.rs:7`).
- Raumbeziehungen behalten ihre Rechte; Direkt vor Raum ändert nur die Auswahl.
- Alte Geräte (≤ 0.5.188) werden über ALPN `/1`, Probe v1 und fehlendes Feedfeld erkannt und
  ausdrücklich gemeldet.

## 12. Offene Fragen an den Nutzer

Keine. Alle Wahlpunkte sind aus r3–r7 und den Rechtegrenzen abgeleitet. Ausdrücklich als Entscheidung
markiert (bei Widerspruch durch den Nutzer änderbar): Wildcard-gebundene Systemdienste bleiben
ausgenommen (T-L2), und das Host-LAN wird nicht automatisch weitergegeben (4.5).

## 13. Nachträge nach der Gesamtreview (2026-10-28)

- 2.7.1: Eine abgewiesene Verbindung nennt die gebundene PID („nicht PID n“), nicht die fremde.
- 3.1: Lehnt der Mensch am Host die Administratorfreigabe für ein Gerät ab, fragt dessen nächste
  Anfrage nicht erneut; ein anderes Gerät, eine neue Freigabe oder eine neue Version fragt wieder.
  Unter Windows erhöht die Abfrage `se.exe rdp host-setup --system …`, nicht PowerShell.
- 3.3: GNOME lässt sich nicht auf Loopback begrenzen (`grdctl` kennt nur `set-port`). se dreht die
  verwalteten Zugangsdaten bei jedem Ende der Abdeckung (Widerruf, Änderung, Fristablauf zum
  Zeitpunkt) und schaltet die von se eingerichtete GNOME-Freigabe ab, sobald kein Gerät mehr
  steuern darf. Während eine Sitzung läuft, gibt der Host dieselben Zugangsdaten erneut aus (nur im
  Speicher). Offen: Solange mindestens ein Gerät steuern darf, bleibt der Netzlistener an
  (zufälliges 256-Bit-Passwort); eine laufende GNOME-Sitzung eines widerrufenen Geräts endet erst
  mit dem Ende aller Abdeckung.
- 4.2: Nur `127.0.0.1`, `::1` und `::ffff:127.0.0.1` gelten als nur-Loopback; eine fehlgeschlagene
  Auflistung gibt nichts frei; vor jedem neuen Datenstrom prüft der Host Besitzer und Bereich neu.
- 9.3: `expires_at` ist eine Zahl (`i64::MAX` = ohne feste Frist); die Relay-Startphase reist als
  `relay_fallback` mit `relay_start: true`.

