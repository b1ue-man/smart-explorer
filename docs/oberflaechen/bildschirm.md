# Oberflächen der Bildschirmverbindung

Stand: 2026-10-20, Pakete U10 (SCR-M2), U16 (SCR-M4, Windows als Host) und U25 (SCR-M7, Desktop-Betrachter) des Strangs S4 im Plan `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`. Belege als `Datei:Zeile` am Stand dieses Pakets. Abkürzung: `N/` = `native/src/`. Pflege: jede Änderung an einer Bildschirmfähigkeit aktualisiert diese Datei im selben Commit (Regel in `docs/oberflaechen/INDEX.md`).

Erreichbarkeit: CLI, IPC, Anzeigefenster und Tab 9 sind erst erreichbar, wenn das Registrierungspaket R4 die Module einträgt (`N/share/mod.rs`, `N/daemon/mod.rs`, `N/daemon/os/shared/ipc_protocol.rs`, `N/daemon/os/shared/ipc.rs`, `N/daemon/os/shared/ipc_host_ext.rs`, `N/cli/mod.rs`, `N/app/mod.rs`, `N/lib.rs`). Tab 9 zeigt bis zur Verteilerzeile in `N/app/core/share_window_ui.rs` die Diagnose.

## 1. Oberflächen

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se screen devices`, `se screen probe`, `se screen windows`, `se screen snapshot`, `se screen bench`, `se screen sessions`, `se screen stop` | `N/cli/screen.rs`, `N/cli/screen/windows.rs`, `N/cli/screen/snapshot.rs`, `N/cli/screen/bench.rs`, `N/cli/screen/sessions.rs`, `N/cli/screen/output.rs`; Vervollständigung `N/cli/completions_screen.rs` (Geräte für `probe`, `windows`, `snapshot`, `bench`) |
| CLI (Betrachter) | `se screen view GERÄT [--source QUELLE] [--control] [--fullscreen] [--json]`, `se screen grid [--room RAUM] [--json]`: starten das Betrachterfenster der Desktop-App losgelöst und geben `launched\t<pid>` bzw. `{"launched":true,"pid":<n>}` aus; ohne Desktop-App neben `se` endet der Befehl mit „Desktop-App nicht installiert“ (Exit 1) | `N/cli/screen/view.rs` |
| Betrachter | Fenster `smart_explorer --screen-view <ziel-json>` (ein Gerät) und `--screen-grid [--room <id>]` (Gruppenansicht); beide Ansichten wechseln im selben Fenster | `N/app/core/screen_launch.rs` (Start, `arm_window`, `close_requested`), `screen_viewer_app.rs`, `screen_viewer_view.rs`, `screen_tile_grid.rs`; Abschnitt 8 |
| Desktop | Fenster „Geräte & Freigaben“, Tab 9 „Bildschirm“ (Knopf „Alle Geräte“), Geräteseite (Knöpfe „Bildschirm öffnen“, „Bildschirm steuern“), Raumseite (Knopf „Bildschirme des Raums“) | `N/app/core/share_pages_ui.rs` (`TAB_SCREEN`), `N/app/core/share_screen_tab_ui.rs:71-148` |
| Anzeigefenster | `smart_explorer --screen-indicator`, immer im Vordergrund, solange eine Sitzung Bildschirminhalt zeigt | `N/app/core/screen_indicator_ui.rs:26-35`; Start durch den Host `N/screen/os/shared/indicator.rs:82` |
| IPC | `IpcRequest::Screen` mit `ScreenIpcRequest::{Probe, Stream, Bench, HostSessions, HostStop, HostWatch}` | `N/daemon/os/shared/ipc_protocol_screen.rs:64-87`, Antworten ab `:108`; Host `N/daemon/os/shared/screen_ipc.rs:78-118`; Client `N/daemon/os/shared/screen_ipc_client.rs` |
| MCP | Werkzeug `screen` (Aktionen `devices`, `probe`, `windows`, `snapshot`; nur lesend), daneben `se` für jeden CLI-Befehl | `N/cli/mcp/screen_tool.rs`, `N/cli/mcp/server.rs`; Eintrag im Katalog trägt R6 ein (`N/cli/mcp/catalog.rs`) |
| Netz | ALPN `smart-explorer/share-screen/1` neben FS, Exec und LAN-Link | `N/screen/core/wire.rs:17`; Annahme `N/share/core/node_accept.rs:154`; Host `N/share/core/node_screen.rs:234-341`; Betrachter `N/share/core/node_screen_link.rs:25-58` |
| Android | keine Bildschirmzeilen und keine Sitzungen | Abschnitt 9 |

## 2. Rechte

Die Rechte „Bildschirm ansehen“ (`screen_view`) und „Bildschirm steuern“ (`screen_control`) setzt S1 je Gerät mit `off`, `ask` oder `allow`. Optionen, Dialog und Bridge beschreibt `docs/oberflaechen/share-exec.md`. Die beiden Katalogzeilen entstehen nur mit grafischer Sitzung (`N/screen/core/catalog_entries.rs:12-36`, Aufruf in `N/share/core/access_catalog.rs`).

Prüfung jeder Sitzung (`N/screen/core/authorize.rs:70-93`):

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

Rückfrage: Der Host meldet `APPROVAL_PENDING` und wartet ohne Frist. Antworten sind „Diesmal erlauben“, „Immer erlauben“ und „Ablehnen“ im Zustimmungsfenster von S1 oder in Tab 9 (`N/app/core/share_screen_tab_ui.rs:111-124`). Schließt der Betrachter die Verbindung, wird die Rückfrage zurückgezogen. Springt das Recht während der Wartezeit auf `allow`, gilt die Sitzung als erlaubt; das Recht wird jede Sekunde gelesen (`N/screen/core/authorize.rs:25`, `:194-286`). Kann niemand gefragt werden, lautet die Antwort `approval_unavailable`.

Ende durch Rechte: Eine Verengung (`N/share/core/node_restrictions.rs:120`) beendet die betroffenen Sitzungen mit `capability_narrowed`, das Ende eines befristeten Rechts (Zeitgeber `N/share/core/node_screen.rs:345-395`) mit `grant_expired` (`N/screen/core/registry.rs:204-224`). Die Zulassung einer Sitzung geschieht unter der Sperre des Rechtezustands, damit keine Änderung zwischen Prüfung und Eintrag fällt.

Zugriffsprotokoll (Art `screen`, mit Modus): `approval_requested`, `approval_granted`, `approval_denied`, `approval_cancelled`, `approval_unavailable` (`N/screen/core/authorize.rs:130-155`), `session_refused` mit Code (`N/screen/core/host_server.rs:85`, `:265`, `:282`), `session_started` mit Art der Zustimmung (`:302`), `control_taken` (`:307`, `N/screen/core/host_session.rs:301`), `control_released` (`N/screen/core/host_session.rs:338`), `session_ended` mit Grund, Dauer in Sekunden und Bytes (`N/screen/core/host_session.rs:363`).

## 3. Geräte, Probe, Messung

| Befehl | Ausgabe | Beleg |
|---|---|---|
| `se screen devices [--room RAUM] [--probe] [--json]` | Text `devices N`, je Gerät `device <Endpunkt> <Name> <Beziehung> <Raum oder -> <online/offline>`, mit `--probe` zusätzlich `view=… control=…`, `screen=unsupported` oder `probe_error=…`; JSON `{"devices":[{"target","device_id","name","relation","room","online","probe"}]}` | `N/cli/screen.rs:233-263`, `N/cli/screen/output.rs:143-190` |
| `se screen probe GERÄT [--json]` | JSON `{"device","supported","platform","view","control","monitors","input","codecs","limits"}`; Text zeilenweise `screen`, `platform`, `view`, `control`, `codecs`, `monitors`, `limits` | `N/daemon/os/shared/ipc_protocol_screen.rs:91`, `N/cli/screen/output.rs:43-66` |
| `se screen bench (GERÄT \| --local) [--codec auto\|h264\|tiles] [--size BxH] [--fps N] [--seconds N] [--json]` | JSON `{"codec","encoder","decoder","path","frames_sent","frames_shown","fps_mean","fps_min","latency_ms":{"p50","p95","max"},"bitrate_kbps":{"mean","max"},"keyframes","recoveries","corrupt_frames","static_psnr_db"}`; `static_psnr_db` ist `null` bei verlustfreiem Bild (unendlich), der Text zeigt `inf` | `N/cli/screen/bench.rs:17-41`, `:89-108`, `N/cli/screen/output.rs:71-106` |
| `se screen windows GERÄT [--json]` | JSON `{"windows":[{"id","title","app","pid","monitor","minimized","focused"}],"apps":[{"app_id","name","pid"}],"list_source":"enum_windows\|…","limits":[…]}`; Text: `list_source\t<Wert>`, je Fenster `window\t<id>\t<Monitor oder ->\t<minimized oder ->\t<focused oder ->\t<App>\t<Titel>`, je App `app\t<app_id>\t<Name>\t<pid oder ->`, je Grenze `limit\t<Text>`; Tabulatoren und Zeilenumbrüche in Namen werden zu Leerzeichen | `N/cli/screen/windows.rs` |
| `se screen snapshot GERÄT [--source QUELLE] --out DATEI [--max-size BxH] [--force] [--json]` | JSON `{"path","width","height","bytes","source"}`; Text `snapshot\t<Pfad>\t<Breite>x<Höhe>\t<Bytes>\t<Quelle>`; `--out` ist Pflicht; eine bestehende Datei bricht mit „existiert schon; --force überschreibt“ ab, bevor das Gerät gefragt wird | `N/cli/screen/snapshot.rs` |

Geräteauswahl: `share://`-Endpunkt, Name, Geräte-ID oder Präfix des Fingerabdrucks; bei mehreren Treffern eine Liste (`N/cli/screen.rs:185-225`). `--seconds` liegt zwischen 1 und 3600 (`N/screen/core/bench.rs:30`, `:54-55`). Die Messung nutzt das Testbild mit Strichcode je Bild: Bildrate, Latenz über den Uhrenabgleich, Bitrate, Schlüsselbilder, Erholzeiten nach Störungen und beschädigte Bilder. Vervollständigung: Geräte aus den gespeicherten Profilen, Sitzungen nur aus einem laufenden Worker (`N/cli/completions_screen.rs:17-61`).

## 4. Sitzungen und Anzeige

| Fähigkeit | CLI | Desktop | Beleg |
|---|---|---|---|
| Sitzungen dieses Hosts | `se screen sessions [--json]`: `{"sessions":[{"id","device_id","device_name","relation","room_id","mode","source","codec","encoder","path","started_at","bitrate_kbps","fps","controlling","state"}]}`; `state` umfasst `waiting_for_approval` | Tab 9 „Laufende Sitzungen“: Gerät, Modus, Quelle, Zustand, Codec, Pfad, Bilder/s, kbit/s | `N/cli/screen/output.rs:108-134`, `N/app/core/share_screen_tab_ui.rs:51-67`, `:125-148` |
| Sitzung beenden | `se screen stop SITZUNG [--json]` → `{"stopped":N}` | „Beenden“ je Zeile | `N/cli/screen/sessions.rs:13-24`, `N/app/core/share_screen_tab_ui.rs:140-142` |
| Alle beenden | `se screen stop --all` | „Alle beenden“; Schließen des Anzeigefensters | `N/app/core/share_screen_tab_ui.rs:145-147`, `N/app/core/screen_indicator_ui.rs:169` |
| Offene Rückfragen | über S1 (`docs/oberflaechen/share-exec.md`) | Tab 9 „Offene Rückfragen“ | `N/app/core/share_screen_tab_ui.rs:107-124` |

Regeln des Hosts:
- Bildschirminhalt nur mit grafischer Sitzung (`no_display`, `N/screen/core/host_server.rs:209-213`) und mit Anzeigefenster (`indicator_unavailable`, `:215-218`, `:282-288`). Das Testbild `synthetic` braucht keines.
- Steuern ist exklusiv: ein zweites Steuern erhält `control_busy` (`N/screen/core/host_server.rs:220-221`, `:272-273`, `N/screen/core/registry.rs:294-310`).
- Höchstens 8 Sitzungen je Host und 4 je Gerät, sonst `session_limit` (`N/screen/core/registry.rs:24-27`, `:161`).
- Das Anzeigefenster schließt sich 3 Sekunden nach der letzten Sitzung (`N/app/core/screen_indicator_ui.rs:21`, `:159`). Endet sein Prozess mit dem Code des Update-Stopps, enden die Sitzungen mit Bildschirminhalt als `update_stop`; endet er nach weniger als 2 Sekunden oder scheitert der Neustart, als `host_stopped` (`N/screen/os/shared/indicator.rs:122-133`).

Quellen auf Linux (SCR-M5, `N/screen/os/linux_os/`): unter X11 `monitor:<Ausgabename>` und `window:<XID dezimal>`; unter Wayland gibt es genau einen Eintrag `monitor:portal` mit Breite und Höhe 0 („unbekannt bis zur Auswahl“), die Auswahl trifft der Nutzer im Systemdialog; jede andere Quelle antwortet mit `portal_selects_source`. Während der Dialog offen ist, steht die Sitzung im Zustand `waiting_for_portal`; ohne PipeWire erscheint er nicht (`pipewire_missing`).

Endgründe (`N/screen/core/session_state.rs:160-171`): `viewer_closed`, `host_stopped`, `capability_narrowed`, `grant_expired`, `host_shutdown`, `update_stop`, `connection_lost`, `failed`.

Ablehnungscodes und ihre Texte beim Betrachter (`N/screen/core/viewer_client.rs:90-118`):

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
| `session_limit` | Das Gerät zeigt schon die höchste Zahl an Sitzungen |

Ein älteres Gerät ohne das Bildschirm-ALPN lehnt mit TLS-Alarm 120 ab (QUIC-Transportcode `0x178`); der Betrachter meldet `screen_unsupported` (`N/screen/core/viewer_client.rs:231-252`, `N/share/core/node_screen_link.rs:44`).

## 5. Kodierung und OpenH264

Folgt mit SCR-M3 und SCR-M10. Stand SCR-M2: nur der verlustfreie Kachelcodec `tiles` (`N/screen/core/host_backend.rs`, `TilesScreenEncoder`, `TilesScreenDecoder`). `se screen bench --codec h264` endet mit „this version has no H.264 decoder yet; use --codec tiles“ (`N/screen/core/bench.rs:66`). Bildgruppen laufen als eigene Uni-Streams; ein Betrachter nimmt höchstens 4 gleichzeitig an und stoppt einen zurückgesetzten Strom sofort (`N/screen/core/gop_stream.rs:24-27`, `:162`).

## 6. Fenster und Standbild

Quellen: `monitor:<id>`, `window:<id>` und `synthetic`. Auf Windows (SCR-M4) ist die Monitor-ID der GDI-Name ohne `\\.\\`, zum Beispiel `monitor:DISPLAY1`, die Fenster-ID der Fenstergriff als Dezimalzahl (`window:123456`); `MonitorInfo.id` und `WindowEntry.id` tragen nur den Teil nach dem Doppelpunkt. Monitore nennen Lage und Größe in physischen Pixeln des virtuellen Bildschirms (`N/screen/os/windows/monitors.rs`), sortiert nach `y`, dann `x`.

`LIST_WINDOWS` liefert unter Windows die Fenster, die Alt+Tab zeigt: sichtbar, mit Titel, nicht verborgen (`DWMWA_CLOAKED`), nicht das Shell-Fenster, ohne Besitzer oder mit `WS_EX_APPWINDOW`, kein Werkzeugfenster ohne `WS_EX_APPWINDOW`; `list_source` ist `enum_windows`, `apps` bleibt leer (`N/screen/os/windows/window_list.rs`). Die Grenzen (`limits`) sind `secure_desktop_paused` und `one_monitor_per_session` immer, `capture_border` mit Graphics Capture, sonst `window_capture_unsupported` und `no_cursor_capture` (`N/screen/os/windows/host_backend.rs`). Ein Gerät ohne Windows-Aufnahme (Linux bis SCR-M5, Testbild) liefert eine leere Liste mit `list_source: unknown` (`N/screen/os/shared/synthetic_backend.rs`).

Aufnahme (Windows): Graphics Capture ab Windows 10 1903 für Monitore und Fenster, für einen Monitor ohne Graphics Capture die DXGI-Duplikation (`N/screen/os/windows/capture_wgc.rs`, `capture_dxgi.rs`); jedes Bild wird auf die CPU zurückgelesen (`d3d_device.rs`, 8,3 MB je Bild bei 1920 × 1080), Alpha ist 255. Aufnahmezustände vor jedem Bild in dieser Reihenfolge: `session_inactive`, `secure_desktop`, bei einem Fenster „Fenster existiert nicht mehr“ (die Sitzung endet mit `capture_failed`) und `window_minimized` (`N/screen/os/windows/capture_source.rs`). Eine Quelle, die nicht öffnet, nennt `monitor_not_found`, `window_not_found`, `session_inactive`, `window_capture_unsupported` oder `capture_unavailable` als Grund von `no_display`. Ein Monitor über 4096 × 2304 Pixel wird auf diese Größe verkleinert.

`SNAPSHOT` liefert ein verlustfreies Standbild der Quelle der Sitzung auf einem eigenen Bildstrom (`N/screen/core/host_capture.rs`); die Quelle des Rahmens (`SnapshotRequest.source`) wird vom Host nicht ausgewertet, die Quelle benennt die Sitzung (`StreamRequest.source`). Die Oberflächen: `se screen windows` und `se screen snapshot` (Abschnitt 3) und das MCP-Werkzeug `screen`. Der Client dafür ist `N/daemon/os/shared/screen_ipc_still.rs` (über `ScreenIpcRequest::Stream`, ohne neue IPC-Varianten): bis zur Annahme durch das Gerät gibt es keine Frist (Rückfrage beim Nutzer), danach höchstens 30 Sekunden; das PNG schreibt `N/screen/core/snapshot.rs` (8 Bit RGB, verlustfrei).

Fensterlisten auf Linux (`list_source`): X11 `ewmh` mit Rückfall `x11_query_tree`; Wayland `ext_foreign_toplevel`, `wlr_foreign_toplevel`, unter KDE `kde_plasma_window_management` (nur aus dem Desktop-Programm, sonst die Grenze `kde_window_list_needs_desktop_app`); Apps aus systemd-Einheiten (`systemd_units`). GNOME hat keine Fensterliste, nur Apps (`no_window_list`). Das Fenster unter Wayland wählt allein der Nutzer am Gerät (`wayland_window_pick_on_host`). Weitere Grenzen: `consent_each_session` (Wayland ohne gespeicherte Freigabe), `x11_no_shm`, `x11_window_overlaps_without_compositor`, `pipewire_missing`, `portal_persist_needs_xdpw_env` (wlroots speichert die Freigabe nur mit `XDPW_PERSIST_MODE`). `se screen portal forget [--json]` (`N/cli/screen/portal.rs`) löscht die gespeicherte Wayland-Freigabe, im Tab Bildschirm „Freigabe vergessen“.

MCP `screen`: `devices` (wie `se screen devices --json`, mit `probe`), `probe` (`device`), `windows` (`device`), `snapshot` (`device`, `source`, `max_edge`, `max_bytes`, `wait`). Das Standbild kommt als ein Bildinhalt (`image/png`) und ein Textinhalt mit `{"width","height","bytes","source","device"}`, derselbe Text steht als strukturierter Inhalt; die lange Kante ist ohne `max_edge` höchstens 1568 Pixel, `max_bytes` verkleinert das PNG in Schritten von 3/4 je Kante aus dem Original. Fragt das Gerät seinen Nutzer, wartet der Aufruf bis zu `wait` Sekunden (Vorgabe 30), beendet dann die Sitzung (das Gerät sieht „Rückfrage zurückgezogen“) und endet mit `approval_pending`.

## 7. Eingabe

Folgt mit SCR-M6. Stand SCR-M2: `INPUT` wirkt nur in der Sitzung, die steuert (`N/screen/core/host_session.rs:266-286`); beim Loslassen des Steuerns werden gedrückte Tasten und Knöpfe gelöst (`:327-343`). Das Testbild-Backend lehnt jede Eingabe mit `unsupported_on_host` ab (`N/screen/os/shared/synthetic_backend.rs:111-113`).

## 8. Betrachter und Gruppenansicht

Stand SCR-M7 (U25), Desktop (Windows und Linux). Der Betrachter ist ein eigener Prozess der Desktop-App: `smart_explorer --screen-view '{"relation","relation_id","device_id","source","control"}'` (Vertrag V5; optional `"fullscreen":true`, das `se screen view --fullscreen` setzt) und `smart_explorer --screen-grid [--room <lokale Raumprofil-ID>]`. Die Gerätekennung eines Direktgeräts muss die aktuelle `remote_device_id` des Kontakts sein; hat sie sich geändert, öffnet der Betrachter nichts und zeigt „Das Gerät hat sich geändert. Bitte die Verbindung zum Gerät prüfen.“

**Ansicht.** Werkzeugleiste: „Gerät:“ (alle Direkt- und Raumgeräte), „Alle Geräte“ (zur Gruppenansicht), „Quelle:“ („Ganzer Bildschirm“, jeder Monitor, jedes Fenster aus der Fensterliste des Geräts; unter Wayland steht dort „Das Fenster wählt der Nutzer am Gerät im Systemdialog.“), „Steuern“ (nur wenn das Gerät Steuern nicht ausgeschaltet hat und Eingaben annimmt; ist ein anderes Gerät der Steuernde, steht dort „Gesteuert von …“), „Einpassen“/„1:1“, „Vollbild“ (F11), „Schärfe vor Flüssigkeit“, „Verlustfrei“, „Niedrigste Latenz (Bildrisse möglich)“, „Statistik“ (Strg+Umschalt+S), das Menü „Tasten“ (nur Tasten, die das Gerät anbietet: Windows-Taste, Menütaste, Druck, Pause, Rollen, Feststelltaste, NumLock, Wiedergabe/Pause, Lauter, Leiser, Stumm; bei einem Android-Gerät auch Zurück, Start, Übersicht, Benachrichtigungen, Schnelleinstellungen, Sperren, Ausschalten-Dialog). Strg+Umschalt+K schaltet „Steuern“. Eine Statuszeile nennt den Zustand der Sitzung auf Deutsch (`N/app/core/screen_texts.rs`) und bietet nach einem Ende „Erneut verbinden“ und „Schließen“. Die Einstellungen stehen in `<App-Daten>/screen/viewer.json`; „Niedrigste Latenz“ wirkt erst beim nächsten Start des Betrachters (wgpu liest `present_mode` beim Öffnen der Fläche; Standard ist `AutoVsync` mit einem Bild Latenz, `Mailbox` wird nie gesetzt).

**Darstellung.** Der Dekodierthread jeder Sitzung liest den Strom des Workers, dekodiert und legt nur das jüngste Bild in einen Platz; der GUI-Thread blockiert weder auf dem Dekodieren noch auf dem Socket (Eingaben gehen über eine Warteschlange und einen Sendethread). Mit wgpu geht die YUV-Ebene des Dekoders (NV12 oder I420) unverändert zur Grafikkarte und ein Shader rechnet in RGB um (±2 gegen die CPU-Umrechnung); ohne wgpu, bei einem Bild im Kachelcodec und in der Gruppenansicht wird BGRA als egui-Textur gezeichnet.

**Eingabe.** Nur bei „Steuern“ an und Fenster im Fokus. Zeiger nur auf dem Bild (0–65535 über die Quelle), Rad in 1/120-Rasten, Tasten als HID-Verwendungen. Buchstaben, Ziffern und Satzzeichen ohne Strg/Alt gehen als `Text` (das Gerät bestimmt das Zeichen), mit Strg/Alt als Taste; eine IME-Eingabe geht als `Text` aus `Commit`. Beim Ausschalten von „Steuern“, bei Fokusverlust und beim Ende der Sitzung werden alle gedrückten Tasten, Knöpfe und Berührungen gelöst. Grenzen: nur die **linken** Modifikatoren (egui meldet keine rechten), AltGr-Zeichen unter Windows kommen von egui nicht als Text (Strg+Alt gilt als Tastenkombination), Nummernblocktasten kommen als Haupttasten an, F25–F35 haben keine Verwendung, keine Zwischenablage und kein Ton, Strg+Alt+Entf wird nicht angeboten.

**Gruppenansicht.** Eine Kachel je Gerät (Direktgeräte, dann Räume mit Überschrift). Je Gerät eine Anfrage (`probe`): „Erlauben“ zeigt die Vorschau (lange Kante 320 Pixel, größere Kacheln fordern in Schritten von 160 Pixeln mehr), „Fragen“ zeigt „Ansehen anfragen“ ohne Bild, „Aus“ „Bildschirm nicht freigegeben“, ein Gerät ohne Bildschirmfreigabe „Keine Bildschirmfreigabe (ältere Version)“, ein nicht erreichbares „Offline“ (keine Verbindung). „Aktualisierung“ (0,5/1/2/5 s): das Gerät kennt nur ganze Bilder je Sekunde, 2 s und 5 s zeigt der Betrachter selbst, indem er nur jedes n-te Bild zeigt (alle werden dekodiert). Ein Klick auf eine Kachel mit „Erlauben“ stellt dieselbe Verbindung per `SET_MODE` auf das Vollbild; bei „Fragen“ oder für „Bildschirm steuern“ öffnet er eine neue Sitzung, bei der das Gerät fragt. Der Rückweg macht aus der Sitzung wieder die Vorschau ihrer Kachel (bei „Erlauben“) oder beendet sie. Es gibt keine feste Obergrenze der Kacheln; sagt ein Gerät `session_limit`, bleibt seine Kachel zu, bis sie angeklickt wird.

**Geräte- und Raumseite.** Die Geräteseite eines Direktgeräts hat „Bildschirm öffnen“ und „Bildschirm steuern“ (aktiv, wenn die Gerätekennung bekannt ist und das Gerät den Zugriff angenommen hat; ob es seinen Bildschirm zeigt, sagt der Betrachter), die Raumseite „Bildschirme des Raums“, der Tab „Bildschirm“ „Alle Geräte“. Alle starten den Betrachter als eigenen Prozess (`N/app/core/screen_launch.rs`).

## 9. Android als Host

Folgt mit SCR-M8. Unter Android meldet `graphical_session()` immer „nein“ (`N/screen/os/shared/synthetic_backend.rs:30-32`), daher entstehen dort keine Katalogzeilen, sobald die Aufrufer den Wert übergeben (Abschnitt 11). Bridge-Methoden für den Bildschirm gibt es nicht.

## 10. Ausnahmen mit Grund

| Oberfläche | Fehlt | Grund |
|---|---|---|
| MCP | Antwort auf Rückfragen | gesperrt durch S1 (C7): eine Zustimmung gibt nur der Mensch am Gerät |
| MCP | Live-Video (Betrachter, Gruppenansicht) | MCP überträgt Werkzeugantworten, keine Videoströme; `screen` liefert Standbilder (`snapshot`) |
| MCP | Eingabe über `screen` | Eingabe hat ein eigenes Werkzeug `screen_input` (U22) |
| Windows | Aufnahme auf dem sicheren Desktop (UAC-Abfrage, Sperrbildschirm) | Windows sperrt sie für Benutzerprozesse; die Aufnahme pausiert mit `secure_desktop` |
| Windows | Fenster vor Windows 10 1903 | Graphics Capture fehlt; nur Monitore über die DXGI-Duplikation, ohne Zeiger (`no_cursor_capture`) |
| Windows | gelber Rahmen um das Aufgenommene | Graphics Capture zeichnet ihn (`capture_border`); dieses Paket schaltet ihn nicht ab (`IsBorderRequired` bleibt unverändert) |
| Windows | mehrere Monitore in einer Sitzung | je Sitzung ein Monitor oder ein Fenster (`one_monitor_per_session`): zwei 4K-Monitore ergäben mehr Makroblöcke, als H.264 Level 5.1 erlaubt |
| Windows | Eingabe und Strg+Alt+Entf | folgen mit SCR-M6 (U22); `inject` lehnt bis dahin mit `unsupported_on_host` ab |
| MCP | `se screen portal forget` | löscht lokal eine Freigabe des Nutzers; kein Werkzeug für Agenten |
| Linux | Fensterliste unter GNOME | GNOME bietet Programmen keine Fensterliste an; nur Apps |
| Linux | Steuern unter wlroots | folgt mit SCR-M6 (U22) |
| Linux | Fensterwahl unter Wayland aus der Ferne | das Portal lässt nur den Nutzer am Gerät wählen |
| Android | Bildschirmzeilen, Sitzungen, Tab | Android ist erst ab SCR-M8 Host |
| Desktop | rechte Modifikatoren, Zwischenablage, Ton | egui meldet nur `Modifiers` ohne Seite; Zwischenablage und Ton gehören nicht zu diesem Strang |
| Desktop | AltGr-Zeichen unter Windows | egui-winit liefert unter Strg keinen Text; AltGr gilt als Strg+Alt |
| Desktop | Umlaufzeit im Statistikfeld | der Worker meldet nur den Uhrversatz; das Feld zeigt „–“ |
| Android | Betrachter, Gruppenansicht | folgen mit SCR-M9 (U28) |
| CLI | Rückfragen beantworten unter `se screen` | Rückfragen beantworten die Oberflächen von S1 für alle Rechte gemeinsam |

## 11. Offene Punkte

- Drei Aufrufer von `capability_catalog` übergeben fest `graphical_session: true`: `N/app/core/share_exec_programs_ui.rs:251`, `N/cli/share/grants_access.rs:102`, `N/mobile/os/shared/domains/share_access.rs:75`. Sie sollen `crate::screen::graphical_session()` übergeben; bis dahin zeigen sie die Bildschirmzeilen auch ohne grafische Sitzung und unter Android.
- Die Verteilerzeile für Tab 9 in `N/app/core/share_window_ui.rs` fehlt (Datei von U24).
- `endpoint_for_screen` (`N/share/core/node_screen_link.rs:151`) kopiert das private `ShareService::endpoint_for_target`; wird jene Funktion für das Modul sichtbar, entfällt die Kopie.
- Die Annahmen zur iroh-Schnittstelle (`Path::rtt`, `Connection::stats`, `TransportErrorCode::crypto`) sind ohne Build nicht geprüft; die Aufgabensuite prüft sie.
