# Oberflächen der Bildschirmverbindung

Stand: 2026-10-26, laufender Quellkandidat. Die gemeinsame Remote-Suite und Veröffentlichung stehen aus. Bestehende S4-Bedienwege bleiben erhalten; die Ergänzungen zu Bildreferenzen, Icons, Größen und Skalierung stehen in Abschnitt 11. Maschinenlogin, TCP, RDP, Programme und Fensterfamilien stehen in [fernzugriff.md](fernzugriff.md). Abkürzungen: `N/` = `native/crates/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`. Historische Zeilennummern vor Abschnitt 11 stammen aus dem S4-Bestandsbericht; für neue Verträge sind die genannten Symbole maßgeblich.

Erreichbarkeit: CLI, IPC, Betrachter, Anzeigefenster und Tab 9 sind verdrahtet. Tab 9 (`TAB_SCREEN`) steht in `N/gui/src/app/core/share_window_ui.rs:66`. Die Registrierungen der Module stehen in den Commits `9456b8ad` (U10), `f4599a0e` (U19), `f83d1d94` (U16), `ecca6583` (U22), `1dc11011` (U25) und `33744c73` (U27).

## 1. Oberflächen

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se screen devices`, `probe`, `windows`, `snapshot`, `input`, `bench`, `codec` (mit `openh264 enable|disable|remove` und `licenses`), `portal forget`, `sessions`, `stop` | `N/cli/src/cli/screen.rs:77-120` (Befehle), `N/cli/src/cli/screen/windows.rs`, `N/cli/src/cli/screen/snapshot.rs`, `N/cli/src/cli/screen/input.rs`, `N/cli/src/cli/screen/bench.rs`, `N/cli/src/cli/screen/codec.rs`, `N/cli/src/cli/screen/portal.rs`, `N/cli/src/cli/screen/sessions.rs`, `N/cli/src/cli/screen/output.rs`; Vervollständigung `N/cli/src/cli/completions_screen.rs` (Geräte für `probe`, `windows`, `snapshot`, `bench`) |
| CLI (Betrachter) | `se screen view GERÄT [--source QUELLE] [--control] [--fullscreen] [--json]`, `se screen grid [--room RAUM] [--json]`: starten das Betrachterfenster der Desktop-App losgelöst und geben `launched\t<pid>` bzw. `{"launched":true,"pid":<n>}` aus; ohne Desktop-App neben `se` endet der Befehl mit „Desktop-App nicht installiert“ (Exit 1) | `N/cli/src/cli/screen/view.rs` |
| Betrachter | Fenster `smart_explorer --screen-view <ziel-json>` (ein Gerät) und `--screen-grid [--room <id>]` (Gruppenansicht); beide Ansichten wechseln im selben Fenster | `N/gui/src/app/core/screen_launch.rs` (Start, `arm_window`, `close_requested`), `screen_viewer_app.rs`, `screen_viewer_view.rs`, `screen_tile_grid.rs`; Abschnitt 8 |
| Desktop | Fenster „Geräte & Freigaben“, Tab 9 „Bildschirm“ (Knopf „Alle Geräte“), Geräteseite (Knöpfe „Bildschirm öffnen“, „Bildschirm steuern“), Raumseite (Knopf „Bildschirme des Raums“) | `N/gui/src/app/core/share_pages_ui.rs` (`TAB_SCREEN`), `N/gui/src/app/core/share_screen_tab_ui.rs:71-148` |
| Anzeigefenster | `smart_explorer --screen-indicator`, immer im Vordergrund, solange eine Sitzung Bildschirminhalt zeigt | `N/gui/src/app/core/screen_indicator_ui.rs:26-35`; Start durch den Host `N/engine/src/screen/os/shared/indicator.rs:82` |
| IPC | `IpcRequest::Screen` mit `ScreenIpcRequest::{Probe, Stream, Bench, HostSessions, HostStop, HostWatch}` | `N/engine/src/daemon/os/shared/ipc_protocol_screen.rs:64-87`, Antworten ab `:108`; Host `N/engine/src/daemon/os/shared/screen_ipc.rs:78-118`; Client `N/engine/src/daemon/os/shared/screen_ipc_client.rs` |
| MCP | Werkzeug `screen` (Aktionen `devices`, `probe`, `windows`, `snapshot`; nur lesend) und Werkzeug `screen_input` (eine Eingabe je Aufruf, Abschnitt 7), daneben `se` für jeden CLI-Befehl | `N/cli/src/cli/mcp/screen_tool.rs:151` (`screen_input`), `N/cli/src/cli/mcp/catalog.rs:198` |
| Netz | ALPN `smart-explorer/share-screen/1` neben FS, Exec und LAN-Link | `N/engine/src/screen/core/wire.rs:17`; Annahme `N/engine/src/share/core/node_accept.rs:154`; Host `N/engine/src/share/core/node_screen.rs:234-341`; Betrachter `N/engine/src/share/core/node_screen_link.rs:25-58` |
| Android | Host (Abschnitt 9) und Betrachter mit Gruppenansicht (Abschnitt 8); Bridge-Methoden `screen.hostPrereqs`, `screen.hostSessions`, `screen.hostStop`, `screen.hostReady`, `screen.hostEnded` (`N/mobile/src/mobile/os/shared/domains/screen_host.rs:56-60`); Bildabruf über JNI `screenPull` (`native/android-bridge/src/screen_jni.rs`) | Abschnitte 8 und 9 |

## 2. Rechte

Die Rechte „Bildschirm ansehen“ (`screen_view`) und „Bildschirm steuern“ (`screen_control`) setzt S1 je Gerät mit `off`, `ask` oder `allow`. Optionen, Dialog und Bridge beschreibt `docs/oberflaechen/share-exec.md`. Die beiden Katalogzeilen entstehen nur mit grafischer Sitzung (`N/engine/src/screen/core/catalog_entries.rs:12-36`, Aufruf in `N/engine/src/share/core/access_catalog.rs`).

Prüfung jeder Sitzung (`N/engine/src/screen/core/authorize.rs:70-93`):

| Modus | `screen_view` | `screen_control` | Ergebnis |
|---|---|---|---|
| jeder | `off` | beliebig | `permission_denied` |
| `probe` | `ask` oder `allow` | beliebig | nur Rechte, Plattform, Codecs, Monitore, Grenzen; kein Bild |
| `preview` | `ask` | beliebig | nur Rechte; eine Kachelvorschau fragt nie (C6) |
| `preview` | `allow` | beliebig | Vorschau |
| `view` | `ask` | beliebig | Rückfrage „Bildschirm ansehen“ |
| `view` | `allow` | beliebig | Sitzung |
| `control` | `ask` oder `allow` | `off` | `right_not_granted` |
| `control` | `ask` oder `allow` | `ask` | Rückfrage „Bildschirm steuern“ |
| `control` | `ask` | `allow` | Rückfrage „Bildschirm ansehen“ |
| `control` | `allow` | `allow` | Sitzung |

Rückfrage: Der Host meldet `APPROVAL_PENDING` und wartet ohne Frist. Antworten sind „Diesmal erlauben“, „Immer erlauben“ und „Ablehnen“ im Zustimmungsfenster von S1 oder in Tab 9 (`N/gui/src/app/core/share_screen_tab_ui.rs:111-124`). Schließt der Betrachter die Verbindung, wird die Rückfrage zurückgezogen. Springt das Recht während der Wartezeit auf `allow`, gilt die Sitzung als erlaubt; das Recht wird jede Sekunde gelesen (`N/engine/src/screen/core/authorize.rs:25`, `:194-286`). Kann niemand gefragt werden, lautet die Antwort `approval_unavailable`.

Ende durch Rechte: Eine Verengung (`N/engine/src/share/core/node_restrictions.rs:120`) beendet die betroffenen Sitzungen mit `capability_narrowed`, das Ende eines befristeten Rechts (Zeitgeber `N/engine/src/share/core/node_screen.rs:345-395`) mit `grant_expired` (`N/engine/src/screen/core/registry.rs:204-224`). Die Zulassung einer Sitzung geschieht unter der Sperre des Rechtezustands, damit keine Änderung zwischen Prüfung und Eintrag fällt.

Zugriffsprotokoll (Art `screen`, mit Modus): `approval_requested`, `approval_granted`, `approval_denied`, `approval_cancelled`, `approval_unavailable` (`N/engine/src/screen/core/authorize.rs:130-155`), `session_refused` mit Code (`N/engine/src/screen/core/host_server.rs:85`, `:265`, `:282`), `session_started` mit Art der Zustimmung (`:302`), `control_taken` (`:307`, `N/engine/src/screen/core/host_session.rs:301`), `control_released` (`N/engine/src/screen/core/host_session.rs:338`), `session_ended` mit Grund, Dauer in Sekunden und Bytes (`N/engine/src/screen/core/host_session.rs:363`).

## 3. Geräte, Probe, Messung

| Befehl | Ausgabe | Beleg |
|---|---|---|
| `se screen devices [--room RAUM] [--probe] [--json]` | Text `devices N`, je Gerät `device <Endpunkt> <Name> <Beziehung> <Raum oder -> <online/offline>`, mit `--probe` zusätzlich `view=… control=…`, `screen=unsupported` oder `probe_error=…`; JSON `{"devices":[{"target","device_id","name","relation","room","online","probe"}]}` | `N/cli/src/cli/screen.rs:233-263`, `N/cli/src/cli/screen/output.rs:143-190` |
| `se screen probe GERÄT [--json]` | JSON `{"device","supported","platform","view","control","monitors","input","codecs","limits"}`; Text zeilenweise `screen`, `platform`, `view`, `control`, `codecs`, `monitors`, `limits` | `N/engine/src/daemon/os/shared/ipc_protocol_screen.rs:91`, `N/cli/src/cli/screen/output.rs:43-66` |
| `se screen bench (GERÄT \| --local) [--codec auto\|h264\|tiles] [--size BxH] [--fps N] [--seconds N] [--json]` | JSON `{"codec","encoder","decoder","path","frames_sent","frames_shown","fps_mean","fps_min","latency_ms":{"p50","p95","max"},"bitrate_kbps":{"mean","max"},"keyframes","recoveries","corrupt_frames","static_psnr_db","stages_ms"}`; `static_psnr_db` ist `null` bei verlustfreiem Bild (unendlich), der Text zeigt `inf`. `stages_ms` ist ein Objekt `{"<Station>":{"p50","p95"}}` in Millisekunden und fehlt im JSON, solange es leer ist (ein Bericht ohne das Feld liest sich mit leerem Feld); der Text zeigt je Station eine Zeile `stage_ms<TAB><Station><TAB>p50=<ms><TAB>p95=<ms>` mit einer Nachkommastelle, zwischen `bitrate_kbps` und `recoveries_ms`, und ohne `stages_ms` ist der Text zeichengleich zu vorher (Plan `docs/plaene/2026-11-14-verbindungen-je-zweck`, P5a und P7) | `N/cli/src/cli/screen/bench.rs:17-41`, `:89-108`, `N/cli/src/cli/screen/output.rs:71-106` |
| `se screen windows GERÄT [--json]` | JSON `{"windows":[{"id","title","app","pid","monitor","minimized","focused"}],"apps":[{"app_id","name","pid"}],"list_source":"enum_windows\|…","limits":[…]}`; Text: `list_source\t<Wert>`, je Fenster `window\t<id>\t<Monitor oder ->\t<minimized oder ->\t<focused oder ->\t<App>\t<Titel>`, je App `app\t<app_id>\t<Name>\t<pid oder ->`, je Grenze `limit\t<Text>`; Tabulatoren und Zeilenumbrüche in Namen werden zu Leerzeichen | `N/cli/src/cli/screen/windows.rs` |
| `se screen snapshot GERÄT [--source QUELLE] --out DATEI [--max-size BxH] [--force] [--json]` | JSON `{"path","width","height","bytes","source"}`; Text `snapshot\t<Pfad>\t<Breite>x<Höhe>\t<Bytes>\t<Quelle>`; `--out` ist Pflicht; eine bestehende Datei bricht mit „existiert schon; --force überschreibt“ ab, bevor das Gerät gefragt wird | `N/cli/src/cli/screen/snapshot.rs` |

Geräteauswahl: `share://`-Endpunkt, Name, Geräte-ID oder Präfix des Fingerabdrucks; bei mehreren Treffern eine Liste (`N/cli/src/cli/screen.rs:185-225`). `--seconds` liegt zwischen 1 und 3600 (`N/engine/src/screen/core/bench.rs:30`, `:54-55`). Die Messung nutzt das Testbild mit Strichcode je Bild: Bildrate, Latenz über den Uhrenabgleich, Bitrate, Schlüsselbilder, Erholzeiten nach Störungen und beschädigte Bilder. **Stationen (`stages_ms`):** `--local` liefert `capture` (Zeichnen des Testbildes, steht für die Aufnahme), `encode` (Aufruf des Kodierers für ein Bild) und `decode` (Aufruf des Dekodierers, der ein Bild fertigstellt). `se screen bench GERÄT` liefert nur `decode` und `capture_to_arrival`: die Zeit von der Aufnahme am Gastgeber bis zur Ankunft des letzten Satzes eines Bildes beim Betrachter, mit dem Uhrenversatz aus `PING`/`PONG` korrigiert; sie enthält Aufnahme, Kodierung, Senden und Netz zusammen, weil der Betrachter nur den Aufnahmezeitstempel jedes Bildes und die eigene Uhr sieht. Die Einzelwerte `capture`, `encode`, `send`, `network` und `decode` eines Bildes über das Netz liefert die Messung M6 (Fall `paar`, Test `verbindungen_messung_task_m6_*` im Suite-Teil `verbindungen`, Gastgeber und Betrachter im selben Prozess). Eine Station ohne Probe fehlt im Bericht (kein Platzhalter). Einzelwerte der Gastgeberstationen gegen ein fernes Gerät brauchen ein neues Feld auf der Leitung (TODO `VERB-BENCH-STATIONEN`). Beleg: `N/engine/src/screen/core/bench.rs`, `bench_stages.rs`, `N/cli/src/cli/screen/output.rs` (`stage_lines`). Die Messung läuft auf der CLI (Text und JSON); MCP erreicht sie über das Werkzeug `se` (`["screen","bench","<Gerät>","--json"]`) mit demselben JSON; Desktop und Android haben keinen Bedienweg für diese Messung. Vervollständigung: Geräte aus den gespeicherten Profilen, Sitzungen nur aus einem laufenden Worker (`N/cli/src/cli/completions_screen.rs:17-61`).

## 4. Sitzungen und Anzeige

| Fähigkeit | CLI | Desktop | Beleg |
|---|---|---|---|
| Sitzungen dieses Hosts | `se screen sessions [--json]`: `{"sessions":[{"id","device_id","device_name","relation","room_id","mode","source","codec","encoder","path","started_at","bitrate_kbps","fps","controlling","state"}]}`; `state` umfasst `waiting_for_approval` | Tab 9 „Laufende Sitzungen“: Gerät, Modus, Quelle, Zustand, Codec, Pfad, Bilder/s, kbit/s | `N/cli/src/cli/screen/output.rs:108-134`, `N/gui/src/app/core/share_screen_tab_ui.rs:51-67`, `:125-148` |
| Sitzung beenden | `se screen stop SITZUNG [--json]` → `{"stopped":N}` | „Beenden“ je Zeile | `N/cli/src/cli/screen/sessions.rs:13-24`, `N/gui/src/app/core/share_screen_tab_ui.rs:140-142` |
| Alle beenden | `se screen stop --all` | „Alle beenden“; Schließen des Anzeigefensters | `N/gui/src/app/core/share_screen_tab_ui.rs:145-147`, `N/gui/src/app/core/screen_indicator_ui.rs:169` |
| Offene Rückfragen | über S1 (`docs/oberflaechen/share-exec.md`) | Tab 9 „Offene Rückfragen“ | `N/gui/src/app/core/share_screen_tab_ui.rs:107-124` |

Regeln des Hosts:
- Bildschirminhalt nur mit grafischer Sitzung (`no_display`, `N/engine/src/screen/core/host_server.rs:209-213`) und mit Anzeigefenster (`indicator_unavailable`, `:215-218`, `:282-288`). Das Testbild `synthetic` braucht keines.
- Steuern ist exklusiv: ein zweites Steuern erhält `control_busy` (`N/engine/src/screen/core/host_server.rs:220-221`, `:272-273`, `N/engine/src/screen/core/registry.rs:294-310`).
- Keine Anzahlgrenze für Sitzungen je Host oder je Gerät (R15c). Den Code `session_limit` sendet der Host nicht mehr; der Betrachter kennt ihn noch für ältere Geräte (`N/engine/src/screen/core/wire_hello.rs:273`, `N/engine/src/screen/core/viewer_client.rs:113`). Die Gruppenansicht lässt die Kachel bei diesem Code geschlossen, bis sie angeklickt wird (`N/gui/src/app/core/screen_tile_grid.rs:9`, `:302`).
- Das Anzeigefenster schließt sich 3 Sekunden nach der letzten Sitzung (`N/gui/src/app/core/screen_indicator_ui.rs:21`, `:159`). Endet sein Prozess mit dem Code des Update-Stopps, enden die Sitzungen mit Bildschirminhalt als `update_stop`; endet er nach weniger als 2 Sekunden oder scheitert der Neustart, als `host_stopped` (`N/engine/src/screen/os/shared/indicator.rs:122-133`).

Quellen auf Linux (SCR-M5, `N/engine/src/screen/os/linux_os/`): unter X11 `monitor:<Ausgabename>` und `window:<XID dezimal>`; unter Wayland gibt es genau einen Eintrag `monitor:portal` mit Breite und Höhe 0 („unbekannt bis zur Auswahl“), die Auswahl trifft der Nutzer im Systemdialog; jede andere Quelle antwortet mit `portal_selects_source`. Während der Dialog offen ist, steht die Sitzung im Zustand `waiting_for_portal`; ohne PipeWire erscheint er nicht (`pipewire_missing`).

Endgründe (`N/engine/src/screen/core/session_state.rs:160-171`): `viewer_closed`, `host_stopped`, `capability_narrowed`, `grant_expired`, `host_shutdown`, `update_stop`, `connection_lost`, `failed`.

Ablehnungscodes und ihre Texte beim Betrachter (`N/engine/src/screen/core/viewer_client.rs:90-118`):

| Code | Text |
|---|---|
| `permission_denied` | Dieses Gerät darf den Bildschirm nicht ansehen (Recht „Bildschirm ansehen“ ist aus) |
| `right_not_granted` | Nur Ansehen ist erlaubt; „Bildschirm steuern“ ist aus |
| `approval_denied` | Am Gerät abgelehnt |
| `approval_cancelled` | Die Rückfrage wurde zurückgezogen |
| `approval_unavailable` | Am Gerät kann niemand gefragt werden |
| `no_display` | Das Gerät hat keine grafische Sitzung zum Zeigen |
| `indicator_unavailable` | Bildschirmfreigabe braucht die Desktop-App auf dem Gerät |
| `control_busy` | Ein anderes Gerät steuert gerade |
| `control_unavailable` | Steuern ist auf diesem Gerät nicht möglich |
| `host_shutdown` | Smart Explorer auf dem Gerät wurde beendet |
| `screen_unsupported` | Gerät kennt die Bildschirmfreigabe nicht (ältere Version) |
| `capture_failed` | Die Aufnahme auf dem Gerät ist fehlgeschlagen |
| `encoder_failed` | Der Kodierer auf dem Gerät ist fehlgeschlagen |
| `session_limit` | Ältere Gerät-Version: Das Gerät hat seine Sitzungsgrenze erreicht. Neuere Versionen senden diesen Code nicht |

Ein älteres Gerät ohne das Bildschirm-ALPN lehnt mit TLS-Alarm 120 ab (QUIC-Transportcode `0x178`); der Betrachter meldet `screen_unsupported` (`N/engine/src/screen/core/viewer_client.rs:231-252`, `N/engine/src/share/core/node_screen_link.rs:44`).

## 5. Kodierung und OpenH264

Codecs: `tiles` ist immer verfügbar und verlustfrei (`N/engine/src/screen/core/codec_registry.rs:133`). H.264 bieten die Plattform-Kodierer an: Windows über Media Foundation mit den Profilen High und ConstrainedBaseline (`N/engine/src/screen/os/windows/mf_common.rs:292-293`, `:340-341`), Android über MediaCodec mit den Profilen `cb` und `high` (`N/engine/src/screen/os/android/bridge_backend.rs:56-59`, `:92`). Welcher H.264-Weg auf Linux angeboten wird, belegt dieser Abschnitt nicht. Hardware-Kodierer über VA-API und NVENC sind nicht umgesetzt; ihre Offenen Punkte stehen im Plan `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-bildschirm.md` (SCR-M11).

OpenH264 (Cisco, verlustbehaftet, nicht im Programm enthalten): `se screen codec openh264 enable` zeigt die Cisco-Lizenz v1.0, fragt nach der Annahme (Vorgabe Nein; `--yes` überspringt die Frage; ohne Terminal und ohne `--yes` bricht der Befehl ab, `N/cli/src/cli/screen/codec.rs:92-99`) und lädt OpenH264 2.6.0 von `ciscobinary.openh264.org` (`N/cli/src/cli/screen/codec.rs:101`, `N/engine/src/screen/os/native_ffi/openh264_pins.rs:26-47`). Der Download ist auf Größe und SHA-256 festgelegt; Cisco veröffentlicht nur MD5-Summen (`openh264_pins.rs:1-7`). `disable` schaltet aus und lässt die Datei; `remove` löscht die Bibliothek dieses Nutzers (`N/cli/src/cli/screen/codec.rs:32-50`).

Lizenztexte: `se screen codec licenses [NAME]` zeigt sie vollständig. Die Quelle ist `third-party/openh264/BINARY_LICENSE.txt`; `N/engine/src/screen/core/licenses.rs` bindet sie mit `include_str!` ein, die Anzeige steht unter Bildschirm → Kodierung → Lizenzen. Der Installer legt die Datei unter `licenses\OpenH264\BINARY_LICENSE.txt` ab (`native/installer.nsi:143-144`).

Bildgruppen laufen als eigene Uni-Streams. Ein Betrachter nimmt höchstens 4 gleichzeitig an (je eine laufende und eine neue Gruppe für `video` und `preview`, `N/engine/src/screen/core/gop_stream.rs:24-27`); einen zurückgesetzten Strom stoppt er sofort (`:162`).

## 6. Fenster und Standbild

Quellen: `monitor:<id>`, `window:<id>` und `synthetic`. Auf Windows (SCR-M4) ist die Monitor-ID der GDI-Name ohne `\\.\\`, zum Beispiel `monitor:DISPLAY1`, die Fenster-ID der Fenstergriff als Dezimalzahl (`window:123456`); `MonitorInfo.id` und `WindowEntry.id` tragen nur den Teil nach dem Doppelpunkt. Monitore nennen Lage und Größe in physischen Pixeln des virtuellen Bildschirms (`N/engine/src/screen/os/windows/monitors.rs`), sortiert nach `y`, dann `x`.

`LIST_WINDOWS` liefert unter Windows die Fenster, die Alt+Tab zeigt: sichtbar, mit Titel, nicht verborgen (`DWMWA_CLOAKED`), nicht das Shell-Fenster, ohne Besitzer oder mit `WS_EX_APPWINDOW`, kein Werkzeugfenster ohne `WS_EX_APPWINDOW`; `list_source` ist `enum_windows`, `apps` bleibt leer (`N/engine/src/screen/os/windows/window_list.rs`). Die Grenzen (`limits`) sind `secure_desktop_paused` und `one_monitor_per_session` immer, `capture_border` mit Graphics Capture, sonst `window_capture_unsupported` und `no_cursor_capture` (`N/engine/src/screen/os/windows/host_backend.rs`). Ein Gerät ohne Windows-Aufnahme (Linux bis SCR-M5, Testbild) liefert eine leere Liste mit `list_source: unknown` (`N/engine/src/screen/os/shared/synthetic_backend.rs`).

Fensterliste (alle Plattformen): Die Liste hat keine Eintragszahl. Ihre Grenze ist der 1-MiB-Rahmen von Protokoll v1 (`MAX_WINDOWS_BYTES`, `N/engine/src/screen/core/wire.rs:54`). `WindowList::bounded` nimmt die Teile in der Reihenfolge Grenzen, Fenster, Apps auf und lässt jeden Eintrag weg, der nicht mehr hineinpasst; ein Eintrag wird nie abgeschnitten (`N/engine/src/screen/core/window_list.rs:171-190`). Titel und Namen bleiben unverkürzt. Fenster ohne gültige Kennung fallen weg; eine ungültige Monitor-Kennung wird leer (`window_list.rs:172-176`).

Aufnahme (Windows): Graphics Capture ab Windows 10 1903 für Monitore und Fenster, für einen Monitor ohne Graphics Capture die DXGI-Duplikation (`N/engine/src/screen/os/windows/capture_wgc.rs`, `capture_dxgi.rs`); jedes Bild wird auf die CPU zurückgelesen (`d3d_device.rs`, 8,3 MB je Bild bei 1920 × 1080), Alpha ist 255. Aufnahmezustände vor jedem Bild in dieser Reihenfolge: `session_inactive`, `secure_desktop`, bei einem Fenster „Fenster existiert nicht mehr“ (die Sitzung endet mit `capture_failed`) und `window_minimized` (`N/engine/src/screen/os/windows/capture_source.rs`). Eine Quelle, die nicht öffnet, nennt `monitor_not_found`, `window_not_found`, `session_inactive`, `window_capture_unsupported` oder `capture_unavailable` als Grund von `no_display`. Ohne ausgehandelten v2-Bildvertrag bleibt die bisherige v1-Pixelgrenze erhalten. Mit v2 bestimmen tatsächliche Capture-, Codec-, Renderer- und Ressourcengrenzen die Bildgröße; eine reine FPS-Grenze verkleinert die Pixel nicht.

Bei einer vorübergehend verweigerten Speicherzulassung bleibt die angeforderte Windows-Aufnahme aktiv und versucht denselben WGC-/DXGI-Neuaufbau beim nächsten Captureaufruf erneut. Eine ausdrückliche Pause startet keinen Producer. X11 veröffentlicht bei einem Größenwechsel die neue Bildreferenz vor der fehlbaren Zulassung, damit der Worker den alten Encoder tatsächlich freigeben kann; wiederholte Versuche für dieselbe wartende Geometrie behalten ihre Generation. Die Windows-Loginaufnahme bewahrt ein noch nicht ausgeliefertes Standbild über unveränderte Geometrieproben hinweg. Bindungswechsel, Capturefehler und ungültige Bildgrößen verwerfen dieses Bild weiterhin (`N/engine/src/screen/os/windows/capture_recovery.rs`, `login_source.rs`, `N/engine/src/screen/os/linux_os/x11_capture_memory.rs`).

`SNAPSHOT` liefert ein verlustfreies Standbild der Quelle der Sitzung auf einem eigenen Bildstrom (`N/engine/src/screen/core/host_capture.rs`); die Quelle des Rahmens (`SnapshotRequest.source`) wird vom Host nicht ausgewertet, die Quelle benennt die Sitzung (`StreamRequest.source`). Die Oberflächen: `se screen windows` und `se screen snapshot` (Abschnitt 3) und das MCP-Werkzeug `screen`. Der Client dafür ist `N/engine/src/daemon/os/shared/screen_ipc_still.rs` (über `ScreenIpcRequest::Stream`, ohne neue IPC-Varianten): bis zur Annahme durch das Gerät gibt es keine Frist (Rückfrage beim Nutzer), danach höchstens 30 Sekunden; das PNG schreibt `N/engine/src/screen/core/snapshot.rs` (8 Bit RGB, verlustfrei).

Fensterlisten auf Linux (`list_source`): X11 `ewmh` mit Rückfall `x11_query_tree`; Wayland `ext_foreign_toplevel`, `wlr_foreign_toplevel`, unter KDE `kde_plasma_window_management` (nur aus dem Desktop-Programm, sonst die Grenze `kde_window_list_needs_desktop_app`); Apps aus systemd-Einheiten (`systemd_units`). GNOME hat keine Fensterliste, nur Apps (`no_window_list`). Das Fenster unter Wayland wählt allein der Nutzer am Gerät (`wayland_window_pick_on_host`). Weitere Grenzen: `consent_each_session` (Wayland ohne gespeicherte Freigabe), `x11_no_shm`, `x11_window_overlaps_without_compositor`, `pipewire_missing`, `portal_persist_needs_xdpw_env` (wlroots speichert die Freigabe nur mit `XDPW_PERSIST_MODE`). `se screen portal forget [--json]` (`N/cli/src/cli/screen/portal.rs`) löscht die gespeicherte Wayland-Freigabe, im Tab Bildschirm „Freigabe vergessen“.

MCP `screen`: `devices` (wie `se screen devices --json`, mit `probe`), `probe` (`device`), `windows` (`device`), `snapshot` (`device`, `source`, `max_edge`, `max_bytes`, `wait`). Das Standbild kommt als ein Bildinhalt (`image/png`) und ein Textinhalt mit `{"width","height","bytes","source","device"}`, derselbe Text steht als strukturierter Inhalt; die lange Kante ist ohne `max_edge` höchstens 1568 Pixel, `max_bytes` verkleinert das PNG in Schritten von 3/4 je Kante aus dem Original. Fragt das Gerät seinen Nutzer, wartet der Aufruf bis zu `wait` Sekunden (Vorgabe 30), beendet dann die Sitzung (das Gerät sieht „Rückfrage zurückgezogen“) und endet mit `approval_pending`.

## 7. Eingabe

`se screen input GERÄT [--source QUELLE] (--text TEXT | --key TASTE | --click X,Y | --move X,Y | --scroll DX,DY | --action NAME) [--json]` (`N/cli/src/cli/screen/input.rs:1-2`, Optionen `:38-65`). Das MCP-Werkzeug `screen_input` nimmt dieselben Angaben, genau eine je Aufruf (`N/cli/src/cli/mcp/screen_tool.rs:151-175`); Tastenkombinationen heißen zum Beispiel `ctrl_left+c` (`:133`). Die Sitzung läuft im Modus `control` und braucht „Bildschirm steuern“ (Abschnitt 2). Die Eingabe wirkt nur in der Sitzung, die steuert (`N/engine/src/screen/core/host_session.rs:266-286`); beim Loslassen werden gedrückte Tasten und Knöpfe gelöst (`:327-343`).

Was ein Host anbietet (`input_for` und die Hostadapter):

| Host | Zeiger | Text | Taste | Berührung | Systemaktion |
|---|---|---|---|---|---|
| Windows | ja | ja | ja | ja (`N/engine/src/screen/os/windows/host_backend.rs:79-86`) | nein |
| Linux, X11 | ja | ja | ja | ja (`N/engine/src/screen/os/linux_os/host_backend.rs:283-294`, Eingabe `x11_input.rs`) | nein |
| Linux, Wayland mit RemoteDesktop-Portal | ja | ja | ja | ja (`portal_input.rs`) | nein |
| Linux, Wayland ohne RemoteDesktop-Portal (zum Beispiel wlroots) | nein | nein | nein | nein (`N/engine/src/screen/os/linux_os/host_backend.rs:283-294`) | nein |
| Android, Bedienungshilfe an | ja | ja | nein (`keyboard: false`, `N/engine/src/screen/os/android/bridge_backend.rs:103-106`) | ja | ja (`bridge_backend.rs:107-115`) |
| Android, Bedienungshilfe aus | nein (`bridge_backend.rs:98-100`) | nein | nein | nein | nein |

Systemaktionen (Android): `back`, `home`, `recents`, `notifications`, `quick_settings`, `lock_screen`, `power_dialog` (`N/engine/src/screen/core/input_model.rs:52-60`, `N/cli/src/cli/screen/input.rs:62-65`).

Tasten, Zeiger und Text am Desktop-Betrachter: Abschnitt 8. Strg+Alt+Entf wird nicht angeboten. Den Zeiger des Gastgebers zeichnet der Betrachter selbst, wenn der Gastgeber das kann (Abschnitt 11, Zeigerüberlagerung).

## 8. Betrachter und Gruppenansicht

Stand SCR-M7 (U25), Desktop (Windows und Linux). Der Betrachter ist ein eigener Prozess der Desktop-App: `smart_explorer --screen-view '{"relation","relation_id","device_id","source","control"}'` (Vertrag V5; optional `"fullscreen":true`, das `se screen view --fullscreen` setzt) und `smart_explorer --screen-grid [--room <lokale Raumprofil-ID>]`. Die Gerätekennung eines Direktgeräts muss die aktuelle `remote_device_id` des Kontakts sein; hat sie sich geändert, öffnet der Betrachter nichts und zeigt „Das Gerät hat sich geändert. Bitte die Verbindung zum Gerät prüfen.“

**Ansicht.** Werkzeugleiste: „Gerät:“ (alle Direkt- und Raumgeräte), „Alle Geräte“ (zur Gruppenansicht), „Quelle:“ („Ganzer Bildschirm“, jeder Monitor, jedes Fenster aus der Fensterliste des Geräts; unter Wayland steht dort „Das Fenster wählt der Nutzer am Gerät im Systemdialog.“), „Steuern“ (nur wenn das Gerät Steuern nicht ausgeschaltet hat und Eingaben annimmt; ist ein anderes Gerät der Steuernde, steht dort „Gesteuert von …“), „Einpassen“/„1:1“, „Vollbild“ (F11), „Schärfe vor Flüssigkeit“, „Verlustfrei“, „Niedrigste Latenz (Bildrisse möglich)“, „Statistik“ (Strg+Umschalt+S), das Menü „Tasten“ (nur Tasten, die das Gerät anbietet: Windows-Taste, Menütaste, Druck, Pause, Rollen, Feststelltaste, NumLock, Wiedergabe/Pause, Lauter, Leiser, Stumm; bei einem Android-Gerät auch Zurück, Start, Übersicht, Benachrichtigungen, Schnelleinstellungen, Sperren, Ausschalten-Dialog). Strg+Umschalt+K schaltet „Steuern“. Eine Statuszeile nennt den Zustand der Sitzung auf Deutsch (`N/gui/src/app/core/screen_texts.rs`) und bietet nach einem Ende „Erneut verbinden“ und „Schließen“. Die Einstellungen stehen in `<App-Daten>/screen/viewer.json`; „Niedrigste Latenz“ wirkt erst beim nächsten Start des Betrachters (wgpu liest `present_mode` beim Öffnen der Fläche; Standard ist `AutoVsync` mit einem Bild Latenz, `Mailbox` wird nie gesetzt).

**Darstellung.** Der Dekodierthread jeder Sitzung liest den Strom des Workers, dekodiert und legt nur das jüngste Bild in einen Platz; der GUI-Thread blockiert weder auf dem Dekodieren noch auf dem Socket (Eingaben gehen über eine Warteschlange und einen Sendethread). Mit wgpu geht die YUV-Ebene des Dekoders (NV12 oder I420) unverändert zur Grafikkarte und ein Shader rechnet in RGB um (±2 gegen die CPU-Umrechnung); ohne wgpu, bei einem Bild im Kachelcodec und in der Gruppenansicht wird BGRA als egui-Textur gezeichnet.

**Eingabe.** Nur bei „Steuern“ an und Fenster im Fokus. Zeiger nur auf dem Bild (0–65535 über die Quelle), Rad in 1/120-Rasten, Tasten als HID-Verwendungen. Buchstaben, Ziffern und Satzzeichen ohne Strg/Alt gehen als `Text` (das Gerät bestimmt das Zeichen), mit Strg/Alt als Taste; eine IME-Eingabe geht als `Text` aus `Commit`. Beim Ausschalten von „Steuern“, bei Fokusverlust und beim Ende der Sitzung werden alle gedrückten Tasten, Knöpfe und Berührungen gelöst. Grenzen: nur die **linken** Modifikatoren (egui meldet keine rechten), AltGr-Zeichen unter Windows kommen von egui nicht als Text (Strg+Alt gilt als Tastenkombination), Nummernblocktasten kommen als Haupttasten an, F25–F35 haben keine Verwendung, keine Zwischenablage und kein Ton, Strg+Alt+Entf wird nicht angeboten.

**Gruppenansicht.** Eine Kachel je Gerät (Direktgeräte, dann Räume mit Überschrift). Je Gerät eine Anfrage (`probe`): „Erlauben“ zeigt die Vorschau (lange Kante 320 Pixel, größere Kacheln fordern in Schritten von 160 Pixeln mehr), „Fragen“ zeigt „Ansehen anfragen“ ohne Bild, „Aus“ „Bildschirm nicht freigegeben“, ein Gerät ohne Bildschirmfreigabe „Keine Bildschirmfreigabe (ältere Version)“, ein nicht erreichbares „Offline“ (keine Verbindung). „Aktualisierung“ (0,5/1/2/5 s): das Gerät kennt nur ganze Bilder je Sekunde, 2 s und 5 s zeigt der Betrachter selbst, indem er nur jedes n-te Bild zeigt (alle werden dekodiert). Ein Klick auf eine Kachel mit „Erlauben“ stellt dieselbe Verbindung per `SET_MODE` auf das Vollbild; bei „Fragen“ oder für „Bildschirm steuern“ öffnet er eine neue Sitzung, bei der das Gerät fragt. Der Rückweg macht aus der Sitzung wieder die Vorschau ihrer Kachel (bei „Erlauben“) oder beendet sie. Es gibt keine feste Obergrenze der Kacheln; sagt ein Gerät `session_limit`, bleibt seine Kachel zu, bis sie angeklickt wird.

**Geräte- und Raumseite.** Die Geräteseite eines Direktgeräts hat „Bildschirm öffnen“ und „Bildschirm steuern“ (aktiv, wenn die Gerätekennung bekannt ist und das Gerät den Zugriff angenommen hat; ob es seinen Bildschirm zeigt, sagt der Betrachter), die Raumseite „Bildschirme des Raums“, der Tab „Bildschirm“ „Alle Geräte“. Alle starten den Betrachter als eigenen Prozess (`N/gui/src/app/core/screen_launch.rs`).

### Android-Betrachter (SCR-M9, U28)

- Sitzungen: `N/mobile/src/mobile/os/shared/domains/screen_view_run.rs` führt die laufenden Betrachtersitzungen (Tabelle, Sitzungsfaden, Ereignisse und Warteschlange der Bilder). Eine Sitzung endet einmal: beim Ereignis `ended`, bei geschlossener Warteschlange (`screenPull` liefert −2) oder beim Entfernen aus der Tabelle.
- Bildabruf: `screenPull(session, dst, timeoutMs)` schreibt das nächste Bild in den direkten Puffer `dst` (`native/android-bridge/src/screen_jni.rs:131-134`). Rückgaben: Anzahl geschriebener Bytes; −1 Zeitüberschreitung; −2 Sitzung beendet; −3 `dst` zu klein, Bytes 12 bis 15 nennen die Länge der Nutzlast, es wird nichts kopiert; −4 Puffer nicht direkt (`N/engine/src/screen/os/android/viewer_sink.rs:36-39`, `:152-157`).
- Kopf eines Bildes, Little-Endian (`viewer_sink.rs`): Bytes 0 bis 7 `pts_us`, 8 bis 11 `flags`, 12 bis 15 `len`. Die Identitätsflags `0x100 | 0x80` verlangen anschließend die vollständige GOP-ID und Frame-ID als je `u64LE`; Codecbytes beginnen bei Offset 32. `len` umfasst beide Identitäten und die Codecbytes. Die oberen 16 Flagbits sind kein ausreichender GOP-Beleg. Auch ein tatsächlich ausgehandelter v1-Wirestrom erhält diese Identität am internen JNI-Pullanschluss.
- H.264: Die Zugriffseinheiten gehen im Annex-B-Format unverändert an die App; SPS und PPS stehen vor jedem IDR (`viewer_sink.rs:1-8`). Der Dekoder (MediaCodec) liest über `pullLarger`, wenn ein Bild nicht in den Eingangspuffer passt (`A/ui/screen/ScreenDecoder.kt:104-114`).
- Kacheln: Der Kern setzt die Kachelbilder zu einem fertigen Bild in der Bytefolge R, G, B, A zusammen (`viewer_sink.rs:3-4`, `:310`, `bgra_to_rgba` in `:341`). Die App zeichnet sie als Bitmap (`ScreenDecoder.kt:33-34`).
- Stau: Ist das älteste Bild der Warteschlange älter als `STALL_US`, leert der Kern die Warteschlange und fordert ein Schlüsselbild an (`viewer_sink.rs:6-9`).
- Eingabe (`A/ui/screen/ScreenInputMapper.kt:14-22`): Am Desktop-Host gilt: Tippen ist ein Linksklick, Halten ohne Bewegung ein Rechtsklick, Ziehen mit einem Finger zieht mit der linken Taste, zwei Finger bewegt als Rad (nach oben scrollt nach unten), Kneifen zoomt ohne Eingabe. Am Android-Host ist jeder Finger (höchstens 10, `ScreenInputMapper.kt:271`) ein Berührungsereignis. Tastenereignisse an Android-Hosts gibt es nicht (Abschnitt 7).
- Gruppenansicht: `A/ui/screen/ScreenTilesScreen.kt` mit den Vorschaukacheln `ScreenPreviewTile.kt`. Jede Vorschau reserviert ihren geometriebezogenen Bild-/Paketbedarf innerhalb des gemeinsamen Prozessbudgets. Das Antippen öffnet eine eigene Vollansichtssitzung mit deren tatsächlichem Budget; die kleine unveränderliche Vorschauvereinbarung begrenzt dadurch nicht die Vollansicht. Schließen führt zur bestehenden Gruppenkachel zurück.

## 9. Android als Host

Stand SCR-M8 (U27). Der Host läuft im Dienst `A/screen/ScreenHostService.kt`. Die Aufnahme beginnt nach dem Systemdialog „Bildschirm teilen“ und kodiert mit einem MediaCodec-Kodierer (`A/screen/ScreenEncoder.kt`). Die Rückfrage an den Telefonbesitzer beantwortet die Aktivität `A/screen/ScreenConsentActivity.kt` (Aktionen „Diesmal erlauben“, „Immer erlauben“ und „Ablehnen“, Abschnitt 2). Vorbedingungen meldet der Kern mit `screen.hostPrereqs` (`N/mobile/src/mobile/os/shared/domains/screen_host.rs:56`). Eingabe und Standbilder setzen den Bedienungshilfe-Dienst voraus (`N/engine/src/screen/os/android/bridge_backend.rs:98-100`); ohne ihn bietet der Host keine Eingabe (Tabelle in Abschnitt 7).

## 10. Ausnahmen mit Grund

| Oberfläche | Fehlt | Grund |
|---|---|---|
| MCP | Antwort auf Rückfragen | gesperrt durch S1 (C7): eine Zustimmung gibt nur der Mensch am Gerät |
| MCP | Live-Video (Betrachter, Gruppenansicht) | MCP überträgt Werkzeugantworten, keine Videoströme; `screen` liefert Standbilder (`snapshot`) |
| MCP | Rückfrage-Entscheidung über `screen` | derselbe Grund wie die Antwort auf Rückfragen |
| MCP | `se screen portal forget` | löscht lokal eine Freigabe des Nutzers; kein Werkzeug für Agenten |
| Windows | Aufnahme auf dem sicheren Desktop (UAC-Abfrage, Sperrbildschirm) | Windows sperrt sie für Benutzerprozesse; die Aufnahme pausiert mit `secure_desktop`; mit gekoppeltem Maschinenziel wechselt der Betrachter selbst in die Anmeldeansicht (E22, Verdrahtung offen) |
| Windows | Strg+Alt+Entf | wird nicht angeboten; der Grund ist im Quelltext nicht belegt |
| Windows | Fenster vor Windows 10 1903 | Graphics Capture fehlt; nur Monitore über die DXGI-Duplikation, ohne Zeiger (`no_cursor_capture`) |
| Windows | gelber Rahmen um das Aufgenommene | Graphics Capture zeichnet ihn (`capture_border`); das Programm schaltet ihn nicht ab (`IsBorderRequired` bleibt unverändert) |
| Windows | mehrere Monitore in einer Sitzung | je Sitzung ein Monitor oder ein Fenster (`one_monitor_per_session`): zwei 4K-Monitore ergäben mehr Makroblöcke, als H.264 Level 5.1 erlaubt |
| Linux | Fensterliste unter GNOME | GNOME bietet Programmen keine Fensterliste an; nur Apps (`no_window_list`) |
| Linux | Steuern unter wlroots | ohne RemoteDesktop-Portal gibt es keine Eingabe (Abschnitt 7) |
| Linux | Fensterwahl unter Wayland aus der Ferne | das Portal lässt nur den Nutzer am Gerät wählen |
| Android | Eingabe mit Tasten am Android-Host | der Host meldet `keyboard: false` (`N/engine/src/screen/os/android/bridge_backend.rs:103-106`); Systemaktionen bleiben |
| Android | Eingabe ohne Bedienungshilfe | ohne den Bedienungshilfe-Dienst meldet der Host keine Eingabe (`bridge_backend.rs:98-100`) |
| Desktop | rechte Modifikatoren, Zwischenablage, Ton | egui meldet nur `Modifiers` ohne Seite; Zwischenablage und Ton gehören nicht zu diesem Strang |
| Desktop | AltGr-Zeichen unter Windows | egui-winit liefert unter Strg keinen Text; AltGr gilt als Strg+Alt |
| Desktop | Umlaufzeit im Statistikfeld | der Worker meldet nur den Uhrversatz; das Feld zeigt „–“ |
| CLI | Rückfragen beantworten unter `se screen` | Rückfragen beantworten die Oberflächen von S1 für alle Rechte gemeinsam |

## 11. Ergänzungen des laufenden Gesamtkandidaten

| Fähigkeit | Oberflächen und konkrete Quellgrenze |
|---|---|
| Appicons | Desktopquellmenü und Kacheln (`N/gui/src/app/core/screen_icons.rs`, `screen_icon_requests.rs`); Androidquellmenü (`A/ui/screen/ScreenIcons.kt`), `screen.viewIcon` und Screen-IPC. Cacheeinträge sind an Gegenstelle, Appidentität und Inhaltsdigest gebunden; Viewer erhalten begrenzte Rasterdaten. |
| Sichtbarkeit und Eingabefreigabe | Desktop `screen_visibility.rs` mit OS-Adaptern; Android `ScreenVisibility.kt`. Host `host_capture_pause.rs` und `host_reference.rs` binden Wiederaufnahme an eine neue Referenz. Fokusverlust allein beweist keine Verdeckung. Unsichere Verdeckung bleibt sichtbar beziehungsweise unknown. |
| Vollbild und Skalierung | Desktop `screen_viewport.rs`, `screen_scaling.rs`, `screen_wgpu_yuv.wgsl`; Android `ScreenViewBars.kt`, `ScreenScaling.kt`, `ScreenScaleWorker.kt` und `ScreenResample.kt`. Lokaler Ausstieg bleibt erreichbar. Pixelmaßstab, Verkleinerung und Vergrößerung verwenden getrennte Darstellungsregeln. |
| Qualitätswechsel | `N/engine/src/screen/core/host_quality.rs` hält Vollbild- und Vorschaugrößen/-raten getrennt. Fehlende optionale Größen-/FPS-Felder behalten die Wahl des adressierten Modus. `lossless` wählt Tiles; `sharpness` begrenzt die Rate. Ein tatsächlicher Codecfehler bleibt als sichtbarer verlustfreier Rückfall bestehen. |
| Hybridregionen und Ausgabeidentität | `frame_protocol.rs`, `frame_reference.rs`, `viewer_reconstruct.rs`: Begin/Video/Moves/Dirty/Commit beziehen sich auf denselben Captureframe. MFT-/OpenH264-Ausgaben tragen ihre tatsächliche Eingabeidentität. Androids Encoder und Hybriddecoder bestätigen reale Einzelinputausgabe; ein ungeeigneter Codec führt zum expliziten verlustfreien Rückfall. |
| Darstellungsbeleg | Desktop `screen_presented.rs` und Android `ScreenPresentedFrame` bestätigen die tatsächlich konsumierte GOP-/Frame-/Epochenidentität. Empfang allein schaltet Eingabe nicht frei. Familienansichten binden zusätzlich die Layoutrevision. |
| Zeigerüberlagerung (Plan `docs/plaene/2026-11-14-verbindungen-je-zweck`, Paket P6) | Fähigkeit `cursor_overlay` (Feld des Gastgeber-Hallos, Version 1; ältere Gastgeber senden es nicht). **Gastgeber:** X11 (XFixes ohne Mischen, `N/engine/src/screen/os/linux_os/x11_cursor.rs`) und Windows mit Graphics Capture (Zeigeraufnahme der Aufnahmesitzung aus, Form und Position aus `GetCursorInfo`, `N/engine/src/screen/os/windows/cursor_overlay.rs`; Zeigerpunkte, die den Untergrund invertieren, etwa der Textzeiger, kommen schwarz mit weißem Rand, `N/engine/src/screen/core/cursor_mask.rs`). Wayland und Graphics Capture vor Windows 10 Version 2004 behalten den Zeiger im Bild (Grund: kein PipeWire-Metadatenleser; vor Version 2004 kein Schalter für die Zeigeraufnahme einer Aufnahmesitzung). Die DXGI-Aufnahme (Windows ohne Graphics Capture) hat wie bisher keinen Zeiger im Bild (`no_cursor_capture`), der Android-Gastgeber hat keinen Zeiger. Keiner dieser Gastgeber sendet ein Zeigerbild. **Ablauf:** Der Betrachter sendet nach der Annahme `CURSOR_REQUEST` (Tag 47, `[1]`), und nur an einen Gastgeber mit der Ankündigung; der Gastgeber schaltet im Hostprozess der Sitzung um und sendet `CURSOR_SHAPE` (Tag 48, bei Formwechsel: RGBA mit vormultiplizierten Farben, bis 16 000 Bildpunkte, ein größerer Zeiger wird verkleinert und mit Faktor `shrink` gezeichnet) und `CURSOR_POSITION` (Tag 49, höchstens eine je aufgenommenem Bild; die Sitzung gibt sie weiter, sobald der Hostprozess sie meldet, nicht erst beim nächsten Bildtakt; Hotspot in Bildpunkten des aufgenommenen Bereichs samt Bereichsgröße, damit jeder Maßstab passt) auf dem verschlüsselten Steuerstrom. Nach jedem `CURSOR_REQUEST` und nach jedem Wechsel der Quelle (`SET_MODE` mit Quelle) sendet der Gastgeber zuerst eine Position mit `visible` = falsch (nichts zu zeichnen) und gibt den Wunsch an den Hostprozess der neuen Quelle; ein Wunsch, der vor dem Start der Aufnahme eintrifft, wirkt mit deren Start. Ein Zeiger außerhalb des Bereichs, verborgen oder ohne darstellbare Form blendet die Überlagerung aus. **Desktop-Betrachter:** `N/gui/src/app/core/screen_cursor.rs` zeichnet den Zeiger als egui-Bild über dem Bild (wgpu und glow gleich). „Ansehen“ zeigt die Gastgeberposition; „Steuern“ (Schalter an und Eingabe bereit, also geht die Eingabe der Ansicht an den Gastgeber) zeichnet ihn an der lokalen Zeigerposition, solange der lokale Zeiger über dem Bild und über nichts liegt, was das Bild verdeckt, und blendet den Systemzeiger dort aus: über dem Bild ist genau ein Zeiger zu sehen. Sonst gilt die Gastgeberposition. Wird eine Vollansicht zur Kachel der Gruppenansicht, schaltet der Betrachter die Überlagerung ab (`CURSOR_REQUEST` `[0]`, der Zeiger steht wieder im Bild der Kachel), beim Wechsel zurück wieder an (`N/gui/src/app/core/screen_pool.rs`). **Android-Betrachter:** die Ereignisse `cursorShape` und `cursorPosition` (`N/mobile/src/mobile/os/shared/domains/screen_view_cursor.rs`, `A/core/ScreenCursorEvent.kt`) zeichnet `A/ui/screen/ScreenCursorOverlay.kt` über das Bild; ein Telefon hat zwischen Berührungen keine eigene Zeigerposition, daher gilt dort in beiden Modi die vom Gastgeber gemeldete Position (eine Berührung bewegt den Zeiger des Gastgebers, der sie zurückmeldet). **Ohne Fähigkeit** (alter Gastgeber, alter Betrachter, CLI, MCP, Vorschau, Metadatenkanal, Fensterfamilie) bleibt alles wie in v0.5.196: der Zeiger steckt im Bild. Nach einem Sitzungswechsel des Gastgebers (`session_changed`) baut das Relais die Sitzung neu auf, spielt die gefaltete Wahl des Betrachters wieder ein (Quelle, Anhebung von der Gruppenansicht zur Vollansicht, Bildgröße; `N/engine/src/daemon/os/shared/screen_ipc_mode_memory.rs`, im Arbeitsbaum umgesetzt, nicht durch die Suite bestätigt) und fragt danach den Zeigerwunsch erneut (`screen_ipc_reconnect.rs`). Kurz zwischen Anfrage und Umschalten kann ein Bild noch den Zeiger enthalten, dann zeigt der Betrachter ihn doppelt, bis das nächste Bild kommt; unter Windows startet die Aufnahmesitzung neu, wenn nach dem Umschalten 150 ms lang kein Bild kam (stehender Bildschirm), damit ein Bild im neuen Zustand folgt (`N/engine/src/screen/os/windows/capture_wgc.rs`). Eine Ansicht, die aus einer Vorschaukachel angehoben wurde (Sitzung als `preview` ausgehandelt), behält den Zeiger im Bild. Grenze: Bei einem Gastgeber, der den Zeiger im Bild behält (Wayland, Graphics Capture vor Windows 10 Version 2004, jeder Gastgeber bis v0.5.196), sieht ein steuernder Desktop-Betrachter weiterhin zwei Zeiger, den eigenen Systemzeiger und den des Gastgebers im Bild. Die Wirkung auf die Wahrnehmung ist auf CI nicht messbar; die Messung M6 des Plans erfasst die umgangene Bildlatenz. |
| Präsenz und Steuerpriorität (Plan `docs/plaene/2026-11-14-verbindungen-je-zweck`, E3, E4; Quellstand 2026-11-14) | Jede Bildschirmsitzung in den Modi `view` und `control` zählt auf dem Gastgeber (`N/engine/src/screen/core/host_pipeline_actor.rs`) und auf dem Betrachter (`viewer_client_tasks.rs`) im Präsenzregister als interaktiv (`N/engine/src/share/core/interactive_presence.rs`); `probe`, Metadatenkanal und eine Vorschau (Modus `preview`) zählen nicht. Die Zählung folgt `SET_MODE` (`gop_stream.rs` `PicturePresence`): eine zur Vollansicht angehobene Vorschau zählt von da an, eine zur Kachel abgesenkte Vollansicht nicht mehr. Solange eine Sitzung zählt, ziehen sich alle Dateiübertragungen dieses Geräts zurück (`docs/TRANSFER_ENGINE.md`, Abschnitt vom 2026-11-14), damit sie das Bild nicht verzögern. Auf der Bildschirmverbindung hat der Steuerstrom des Gastgebers Priorität 1, die Bildgruppen haben 0 (`gop_stream.rs` `CONTROL_STREAM_PRIORITY`, gesetzt in `N/engine/src/share/core/node_screen.rs`), damit Steuerrahmen, `STATE` und Zeigerrahmen nicht hinter einer großen Bildgruppe warten. Die Verbindung hat das Profil `Screen` (Cubic, Sendebündel 10/20). Es gibt keine Einstellung und keinen Bedienweg: das Verhalten ist auf CLI, Desktop, Android und MCP gleich, und ein Gegenüber der Fassung v0.5.196 braucht dafür nichts (keine Aushandlung: Präsenz und Priorität wirken nur auf dem eigenen Gerät beziehungsweise beim Sender). |
| Hostprozess und Daemon aus einer Installation | Jede Sitzung nimmt in einem eigenen Hostprozess auf, dem Desktop-Programm neben `se` (`N/engine/src/screen/os/shared/host_worker_client.rs`). Seit Paket P6 kennt das Hostprozess-Protokoll `CaptureRequest::CursorOverlay` und den Rahmen `CURSOR` (Tag 27). Ein älterer Hostprozess kennt beides nicht; der Quelltext liest daraus, dass die Sitzung mit `capture_failed` endet (kein Lauf). Daemon und Desktop-Programm müssen daher aus derselben Installation stammen; geprüft wird heute nur GUI oder CLI gegen Daemon (`ipc_client.rs`, `worker_version_is_current`). Offen: TODO `VERB-HOSTPROZESS-VERSION`. |
| Größen- und Speicherangebot | `frame_budget.rs`, `session_admission.rs` und `ScreenFrameLimits.kt` binden tatsächliche Sessionbudgets vor Paket-, Queue-, Pull- und Decoderallokationen. Android-Vorschauen erhalten einen geometriebezogenen Teilbetrag der gemeinsamen Prozessgrenze. Explizite kleinere Legacybudgets behalten v1 auf dem Netzwerk und begrenzen lokale Bild-, Record-, JNI- und Skalierungsbesitzer. Full und Preview teilen innerhalb einer Sitzung ihren Skalierungsbesitzer; Zwischenpuffer und gehaltene Ausgaben werden vor der Allokation gegen das angenommene Budget zugelassen. Die statische Bilanz gilt für lebende Produktpuffer; Prozess-RSS, Codec-/Treiberpuffer und GC sind kein Bestandteil dieser Zusage. Speicher- und Geräteverhalten wurden noch nicht ausgeführt. |

Die v2-Klasse erlaubt im aktuellen Quellstand bis zu 864 MiB bei unveränderter
Reserve von 24 Bildäquivalenten. Windows und Linux begrenzen neue Zusagen durch
physische, Commit- und Adressraumfakten sowie die bereits gezählten Sitzungen.
Das ermöglicht ein natives 3840×2160-Angebot bei ausreichenden Ressourcen;
fehlende Fakten, alte Gegenstellen und kleinere Android-Angebote erlauben keine
pauschale Erweiterung. Die Legacy-/RAW-Allokationsklasse bleibt auf 512 MiB
begrenzt; eine Legacyverbindung verlangt diesen Betrag nicht pauschal. Kleinere
Leases begrenzen die tatsächlich darstellbare Geometrie und ihre Paketbesitzer.
Eine Zusage ist
interne Ressourcenrechnung, keine physische Reservierung gegen fremde Prozesse.

Beim Android-Captureneustart endet die alte Allokationsbefugnis vor der neuen
Zulassung. Tatsächlich noch lebende Slices bleiben dabei gebucht. Ein
Konfigurationswechsel beendet die alten Java-Producer; ein ausstehender
synthetischer Abschluss setzt die aktuelle Konfiguration erst nach dem echten
Retirementcallback fort. Der eigene statische Fixzyklus und die vorbereiteten
Remote-Fälle stehen in
[`fix-regression-screenbudget.md`](../plaene/2026-10-24-batches/fix-regression-screenbudget.md).
Diese Quelländerung ist noch kein ausgeführter Pause-/Resume- oder Gerätenachweis.

Die aktuelle Renderergrenze verwendet konkrete `RendererOwner`-/`RenderSlot`-
Besitzer für CPU-/YUV-Uploads, Renderflächen und ausstehendes Retirement. Der
Glow-F3-Quellkandidat gibt fehlgeschlagene Uploads und Draws bis zur
Präsentationsentscheidung weiter. Der anschließende Quellfix `43f46c8f` entzieht
auch bereits bestätigte Login-Input-/SAS-Belege am tatsächlichen Fehlerpfad;
veraltete Callbacks können keine neue gültige Darstellung entwerten. Die eigene
statische Anti-Regressionsreview steht in
[`fix-renderer-eingabefreigabe.md`](../plaene/2026-10-24-batches/fix-renderer-eingabefreigabe.md).
Ein echter GL-/SAS-Lauf ist weiterhin nicht ausgeführt. Der Capturefix
`f945ef59` korrigiert Cursorzulassung, SHM-Import, Worker-Ausgabesperre und
Formatwechsel; seine eigene statische Anti-Regressionsreview steht in
[`fix-capture-integration.md`](../plaene/2026-10-24-batches/fix-capture-integration.md).
Dieser Dokumentstand bestätigt weder die vollständige Speicherbilanz noch
einen ausgeführten 4K-Nachweis.

CLI und MCP erhalten Metadaten, Quellen, Standbilder und Steuerbefehle über
bestehende Screen-Verträge. Livebilder werden von Desktop-/Androidrenderern
gezeichnet; MCP ist weiterhin kein Videoplayer. Die gemeinsame Suite muss die
geänderten Wire-, IPC-, JNI- und Darstellungsgrenzen zusammen prüfen.

## 12. Prüfstatus und historische Inventurkorrekturen

Der aktuelle Code registriert Tab 9 in `share_window_ui.rs`. Desktop und CLI
verwenden beim Capabilitykatalog `crate::screen::graphical_session()`;
Android meldet die tatsächlich grafische App weiterhin ausdrücklich als solche.
Die widersprüchlichen früheren offenen Listen sind damit ersetzt.

Dieser Dokumentstand bestätigt keine Hardwarecodecs, GUIgesten oder
Remote-CI-Ausführung. Statische Befunde und verbliebene Beweisgrenzen stehen in
[Review B6](../plaene/2026-10-24-batches/review-b6.md). Offene Produktarbeit steht
nur in [TODO](../TODO.md); frühere SCR-M11-Pläne sind historische Evidenz.

## Root-Worker und aktive Sitzung (Linux, E23, E46 bis E48, E62)

Ein Worker ohne eigene grafische Sitzungsvariablen (zum Beispiel ein Root-Dienst) ermittelt die Sitzung selbst über logind (`Seat0.ActiveSession`, Klasse, Typ, Anzeige, Benutzer). Eine Sitzung gilt nur, wenn sie aktiv, lokal und nicht schließend ist, die Klasse `user`, `greeter` oder `lock-screen` hat, den Typ `x11` oder `wayland` hat und nicht die UID 0 trägt. Sonst gibt es keine Sitzung, und der Host zeigt keinen Bildinhalt (`no_display`).

- Die Umgebung (`DISPLAY`, `XAUTHORITY`, `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, D-Bus-Adresse) stammt nur aus Prozessen mit der UID der Sitzung. Mehrdeutige Werte führen zur Ablehnung.
- Der Host startet als Sitzungskonto. Ein Sitzungswechsel beendet den Host in der alten Sitzung; der nächste Start läuft in der neuen Sitzung. Es gibt keinen Übergabebefehl und keine Übergabe nach einem Update.
- Ein Worker mit eigenen Sitzungsvariablen behält den bisherigen Weg.
- Anzeige- und Rückfragefenster eines Root-Workers erhalten je Fenster ein eigenes Token (256 Bit Zufall). Es wird als erste Zeile über die Standardeingabe des Fensters übergeben, nicht über Umgebung oder Datei (E46). Der Worker nimmt es nur für Ping, Liste und Beobachtung der Bildschirmsitzungen, für deren Ende und für Liste und Entscheidung genau der beim Start genannten Rückfragen an. Das Token erlischt mit dem Ende des Fensterprozesses und beim Sitzungswechsel.
- Fensterliste, Programmeinheiten und Symbole eines gefolgten Root-Workers werden in einem Prozess des Sitzungskontos mit dessen Umgebung abgefragt, nie als root gegen Bus oder Compositor des Kontos (E47).
- Endet der Host, weil die aktive Sitzung wechselt, meldet er den Grund `session_changed`. Der Betrachter baut dieselbe Sitzung selbst neu auf, solange das Recht besteht: höchstens ein Versuch je Sekunde und höchstens 30 Sekunden lang (E48).
- Der Greeter (Sitzungsklasse `greeter`) ist mit „Bildschirm steuern: Erlauben“ über den normalen Bildschirmweg erreichbar, aber nur für einen Worker mit effektiver UID 0, dessen Rechte der Administrator dieses Rechners vergibt (E62). Die Login-Eigenschaften (Nonce je Operation, 30-Sekunden-Challenge, Konsolensitzung, Kontobindung, Darstellungsbeleg) gelten für den Weg über den Login-Dienst.
- Stand 2026-11-14: im Arbeitsbaum umgesetzt, nicht durch die Remote-Suite bestätigt. Offen: Der Fensterkanal bindet einen zufälligen Loopback-Port (`native/crates/engine/src/daemon/os/shared/ipc_window_channel.rs`, Funktion `open_with`, `127.0.0.1:0`); dieser Port ist nicht als se-eigener Port angemeldet, obwohl se-eigene Ports nie als Dienst angeboten werden. Restrisiko: Bis zum ersten Lesen liegt das Token in der Pipe; ein anderer Prozess desselben Sitzungskontos kann es über `/proc/<pid>/fd/0` des Fensters lesen.
