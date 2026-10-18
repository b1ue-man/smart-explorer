# Oberflächen der Bildschirmverbindung

Stand: 2026-10-18, Paket U10 (SCR-M2, Strang S4 des Plans `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`). Belege als `Datei:Zeile` am Stand dieses Pakets. Abkürzung: `N/` = `native/src/`. Pflege: jede Änderung an einer Bildschirmfähigkeit aktualisiert diese Datei im selben Commit (Regel in `docs/oberflaechen/INDEX.md`).

Erreichbarkeit: CLI, IPC, Anzeigefenster und Tab 9 sind erst erreichbar, wenn das Registrierungspaket R4 die Module einträgt (`N/share/mod.rs`, `N/daemon/mod.rs`, `N/daemon/os/shared/ipc_protocol.rs`, `N/daemon/os/shared/ipc.rs`, `N/daemon/os/shared/ipc_host_ext.rs`, `N/cli/mod.rs`, `N/app/mod.rs`, `N/lib.rs`). Tab 9 zeigt bis zur Verteilerzeile in `N/app/core/share_window_ui.rs` die Diagnose.

## 1. Oberflächen

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se screen devices`, `se screen probe`, `se screen bench`, `se screen sessions`, `se screen stop` | `N/cli/screen.rs:47-97`, `N/cli/screen/bench.rs`, `N/cli/screen/sessions.rs`, `N/cli/screen/output.rs`; Vervollständigung `N/cli/completions_screen.rs:17-61` |
| Desktop | Fenster „Geräte & Freigaben“, Tab 9 „Bildschirm“ | `N/app/core/share_pages_ui.rs` (`TAB_SCREEN`), `N/app/core/share_screen_tab_ui.rs:71-148` |
| Anzeigefenster | `smart_explorer --screen-indicator`, immer im Vordergrund, solange eine Sitzung Bildschirminhalt zeigt | `N/app/core/screen_indicator_ui.rs:26-35`; Start durch den Host `N/screen/os/shared/indicator.rs:82` |
| IPC | `IpcRequest::Screen` mit `ScreenIpcRequest::{Probe, Stream, Bench, HostSessions, HostStop, HostWatch}` | `N/daemon/os/shared/ipc_protocol_screen.rs:64-87`, Antworten ab `:108`; Host `N/daemon/os/shared/screen_ipc.rs:78-118`; Client `N/daemon/os/shared/screen_ipc_client.rs` |
| MCP | Werkzeug `se` (führt jeden CLI-Befehl aus) | `N/cli/mcp/catalog.rs` (Beispiel `se screen bench` trägt R4 ein) |
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

Folgt mit SCR-M4. Stand SCR-M2: `LIST_WINDOWS` erhält eine leere Liste mit `list_source: unknown` (`N/screen/os/shared/synthetic_backend.rs:73-80`); Monitore und Fenster als Quelle ergeben `capture_unavailable` (`:92`). `SNAPSHOT` liefert ein verlustfreies Standbild der aktuellen Quelle auf einem eigenen Bildstrom (`N/screen/core/host_capture.rs`). Eine Oberfläche dafür gibt es noch nicht.

## 7. Eingabe

Folgt mit SCR-M6. Stand SCR-M2: `INPUT` wirkt nur in der Sitzung, die steuert (`N/screen/core/host_session.rs:266-286`); beim Loslassen des Steuerns werden gedrückte Tasten und Knöpfe gelöst (`:327-343`). Das Testbild-Backend lehnt jede Eingabe mit `unsupported_on_host` ab (`N/screen/os/shared/synthetic_backend.rs:111-113`).

## 8. Betrachter und Gruppenansicht

Folgt mit SCR-M7 und SCR-M9. Stand SCR-M2: Der Worker kann einen Bildstrom für ein anderes Programm durchreichen (`ScreenIpcRequest::Stream`, `N/daemon/os/shared/screen_ipc.rs:241`); der Client dafür (`screen_stream` in `N/daemon/os/shared/screen_ipc_client.rs`) ist vorbereitet und noch ohne Aufrufer. Ein Betrachterfenster gibt es noch nicht. Der Modus `preview` für Kachelvorschauen besteht im Protokoll (C6).

## 9. Android als Host

Folgt mit SCR-M8. Unter Android meldet `graphical_session()` immer „nein“ (`N/screen/os/shared/synthetic_backend.rs:30-32`), daher entstehen dort keine Katalogzeilen, sobald die Aufrufer den Wert übergeben (Abschnitt 11). Bridge-Methoden für den Bildschirm gibt es nicht.

## 10. Ausnahmen mit Grund

| Oberfläche | Fehlt | Grund |
|---|---|---|
| MCP | eigenes Werkzeug `screen` | kommt mit SCR-M4 (R6); bis dahin deckt `se` jeden CLI-Befehl ab |
| MCP | Antwort auf Rückfragen | gesperrt durch S1 (C7): eine Zustimmung gibt nur der Mensch am Gerät |
| Android | Bildschirmzeilen, Sitzungen, Tab | Android ist erst ab SCR-M8 Host |
| Desktop | Betrachterfenster, Gruppenansicht | kommen mit SCR-M7 und SCR-M9 |
| CLI | Rückfragen beantworten unter `se screen` | Rückfragen beantworten die Oberflächen von S1 für alle Rechte gemeinsam |

## 11. Offene Punkte

- Drei Aufrufer von `capability_catalog` übergeben fest `graphical_session: true`: `N/app/core/share_exec_programs_ui.rs:251`, `N/cli/share/grants_access.rs:102`, `N/mobile/os/shared/domains/share_access.rs:75`. Sie sollen `crate::screen::graphical_session()` übergeben; bis dahin zeigen sie die Bildschirmzeilen auch ohne grafische Sitzung und unter Android.
- Die Verteilerzeile für Tab 9 in `N/app/core/share_window_ui.rs` fehlt (Datei von U24).
- `endpoint_for_screen` (`N/share/core/node_screen_link.rs:151`) kopiert das private `ShareService::endpoint_for_target`; wird jene Funktion für das Modul sichtbar, entfällt die Kopie.
- Die Annahmen zur iroh-Schnittstelle (`Path::rtt`, `Connection::stats`, `TransportErrorCode::crypto`) sind ohne Build nicht geprüft; die Aufgabensuite prüft sie.
