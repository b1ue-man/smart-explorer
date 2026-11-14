# Bildkette: Weg eines Bildes, Referenzen, Potential, Entscheidungen, Arbeitsplan

Stand: 2026-11-14. Teil des Auftrags `docs/plaene/2026-11-14-verbindungen-je-zweck/` (neben `plan.md`, `recherche.md`, `bestand.md`). Dieses Dokument plant; es ändert keinen Code. Nichts wurde gebaut, getestet oder committet.

## 0 Auftrag, Belegarten, Lesestand

### 0.1 Nutzerwortlaut (2026-11-14, alle für dieses Dokument geltenden Vorgaben)

1. „ich müsst auch noch erwähnen, dass der stream von der latenz, der refreshrate und qualität WEIT hinter anderen anbietern, vorallem welchen wie shadopc, hinterher hängt. um welten. da möchte ich noch große verbesserungen sehen. sehr große.“
2. „die bildübertragung selbst bedarf auch weiterer optimierung, also wenn du siehst dass da potential ist, mach es“
3. „das ziel ist das jede einzelne komponente optimal funktioniert, nicht das alles das gleiche nutzt und dann alles nicht ideal ist.“
4. „bei shadow kann man zum beispiel zwischen udt und tcp wählen, unterschiedliche bild decoder, und nderen sachen. die datenübertragunsrate allein zeigt dass anbieter wie shadow wesentlich mehr übertragen. ich sage immer das limit darf nicht die software sein, es muss die hardware sein. bisher ist es eindeutig die software, nicht das netzwerk.“
5. „ich weiß auch nicht wie das programm es grade macht, vielleicht ist der fehler ja schon bei der bildaufnahme. ich sehe immer 2 mäuße, meine und die des zielpcs, macht se screenshots, wie capturen andere den pc in echtzeit. die gesamte strecke ist zu prüfen ob sie ideal ist.“

Weiter geltend: 2026-11-08 „kein zusätzlicher se command oder s9nsyiger sche8ss“ (der Nutzer führt keine Handgriffe aus; was das Programm braucht, richtet es selbst ein; keine Adminrechte als Voraussetzung); die Bildschirmverbindung bleibt verschlüsselt (Rücknahme der Klartextoption am 2026-11-14, `plan.md` A und C4); Geräte des Nutzers sind Windows, Linux und Android, jeweils als Gastgeber und als Betrachter.

Eigene Folgerungen (nicht Nutzerwortlaut): „udt“ in Vorgabe 4 lese ich als „UDP“, weil Shadow die Wahl „Prefer speed (UDP)“ und „Prefer reliability (TCP)“ anbietet (R07). Genannte Anbieter sind Beispiele; Maßstab ist jeder etablierte Streamingweg (Vorgabe des Auftraggebers). „Limit muss die Hardware sein“ heißt für diesen Plan: Jeder feste Wert, der Datenrate, Bildrate, Auflösung oder Qualität unter die Grenze von Kodierer, Dekodierer, Netz oder Anzeige drückt, wird durch eine gemessene oder von der Hardware gemeldete Grenze ersetzt.

### 0.2 Belegarten

- **[Q]** Quelltext des Arbeitsbaums am 2026-11-14 gelesen (Datei:Zeile).
- **[R…]** Referenz aus Abschnitt 2.0 (Quelle mit Version oder Commit und Abrufdatum).
- **[P]** Planfestlegung dieses Dokuments (kein Quellenwert).
- **[S]** Schätzung; dabei steht die Messung, die sie belegt oder widerlegt.
- **[A]** Annahme, ausdrücklich ungeprüft, mit Folge.
- **[U]** ungeklärt; in Abschnitt 6 gelistet.

Pfadkürzel: `E/` = `native/crates/engine/src/screen/`, `D/` = `native/crates/engine/src/daemon/os/shared/`, `G/` = `native/crates/gui/src/app/core/`, `C/` = `native/crates/cli/src/cli/`, `M/` = `native/crates/mobile/src/mobile/os/shared/domains/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

### 0.3 Lesestand

Arbeitsbaum auf Commit `6e5c6b11` mit den uncommitteten Änderungen des Vorauftrags und dieses Auftrags (P1 bis P6 und PX umgesetzt, P7 läuft parallel und ändert `E/core/bench.rs`, `E/core/bench_stages.rs`, `E/core/metrics.rs`). Zeilenangaben gelten für diesen Arbeitsbaum. Zeilenzahlen von Dateien in Abschnitt 5 wurden am 2026-11-14 mit `wc -l` gemessen.

---

## 1 Heutiger Weg eines Bildes

### 1.1 Prozesse, Threads und der gemeinsame Kern

**Gastgeber (Windows, Linux X11, Linux Wayland):** Zwei Prozesse je Sitzung.

- Daemon: Sitzungsschleife `Session::run` mit einem Tokio-Intervall als Aufnahmetakt, `tick = 1_000_000 / max_fps` µs, `MissedTickBehavior::Skip` (`E/core/host_session.rs:150-152` [Q]). Ein verpasster Takt wird nach dem Ende der laufenden Bildoperation nachgeholt (`:157-160`, `:199-200` [Q]).
- Hostprozess `--screen-host-worker` je Sitzung, gestartet vom Daemon, Anfragen über Standardeingabe, Antworten über Standardausgabe (`E/os/shared/host_worker_main.rs:1-13`, `E/os/shared/host_worker_client.rs:1-14` [Q]). Er bearbeitet Anfragen strikt nacheinander in einer Schleife (`E/os/shared/host_worker_run.rs:219-235`, `Worker::handle` in `host_worker_main.rs:126-240` [Q]).
- Je Bild: Takt → `Pacer::decide` → `spawn_blocking` → eine Anfrage `Produce` über die Pipe → im Hostprozess Aufnahme, Skalierung, Farbumwandlung, Kodierung → Antwort mit kodierten Bytes → Daemon schreibt die Sätze in den QUIC-Bildstrom und wartet auf das Schreiben (`E/core/host_pipeline_produce.rs:76-160` [Q], Schreiben `E/core/gop_stream.rs:109-110`, `:161`, `:205` [Q]). Erst danach beginnt das nächste Bild. Es ist immer genau ein Bild in der Kette Aufnahme bis Versand.
- Frist je Bildanfrage an den Hostprozess: `FRAME_TIMEOUT` 10 s (`host_worker_client.rs:83` [Q], ohne Beleg). Öffnen: `OPEN_TIMEOUT` 120 s (`:81`).

**Gastgeber Android:** ein Prozess (App mit eingebettetem Kern). MediaProjection → VirtualDisplay → Eingabefläche des MediaCodec-Kodierers (`A/screen/ScreenHostControl.kt:453-456`, `A/screen/ScreenEncoder.kt:81` [Q]); kodierte Zugriffseinheiten gehen per `screenPush` aus einem direkten `ByteBuffer` in den Kern (`native/android-bridge/src/screen_jni.rs:1-7` [Q]), werden dort einmal in zugelassenen Speicher kopiert und in eine Warteschlange von höchstens `MAX_QUEUED` 8 Bildern gelegt (`E/os/android/bridge_channels.rs:40`, `bridge_channel_state.rs:144-170` [Q]); der Takt der Pipeline holt je Takt ein Bild (`E/os/android/bridge_source_capture.rs:141-161` [Q]).

**Betrachter Desktop (Windows, Linux):** Zwei Prozesse.

- Daemon des Betrachters: QUIC-Empfang, Annahme von höchstens 4 Bildströmen (`E/core/gop_stream.rs:24-30` [Q]), `FRAME_ACK` sofort nach dem letzten Satz eines Bildes, also vor Dekodierung und Anzeige (`gop_stream.rs:1-8`, `D/screen_ipc.rs:383` [Q]); Weitergabe über eine Loopback-TCP-Verbindung an das Betrachterfenster (`D/screen_ipc.rs:10`, `:322-336`, `:397-434` [Q]).
- Betrachterprozess `smart_explorer --screen-view`: Dekodierthread je Sitzung (liest den Relaisstrom, dekodiert, legt nur das jüngste Bild in einen Platz), GUI-Thread (zeichnet), Sendethread für `SET_MODE`, `INPUT` (`G/screen_session.rs:1-7`, `G/screen_decode_thread.rs:1-13` [Q]). Der Dekodierthread weckt den GUI-Thread je Bild (`G/screen_session_backend.rs:99` [Q]).

**Betrachter Android:** ein Prozess; der Kern nimmt die Ströme an, legt Zugriffseinheiten in eine Warteschlange (`E/os/android/viewer_sink.rs:1-20` [Q]), die App holt sie mit `screenPull` in den direkten Eingabepuffer des MediaCodec-Dekodierers (`native/android-bridge/src/screen_jni.rs:1-7`, `docs/oberflaechen/bildschirm.md` Abschnitt 8 [Q]).

### 1.2 Gemeinsame Stationen (alle Gastgeber)

| Station | Verfahren heute | Feste Werte (Beleg) |
|---|---|---|
| Bildtakt | Tokio-Intervall im Daemon, nicht an das Bild des Kompositors gebunden (`host_session.rs:150-152` [Q]); `FrameClock::take` nimmt ein Bild bis ¼ Intervall zu früh (`E/core/pacing.rs:255-272` [Q]) | `MAX_FPS` 240 (`pacing.rs:27`, Beleg „höchste übliche Bildwiederholrate“, keine Messung) |
| Bildrate des Betrachters | Desktop fordert fest `FULL_FPS` 60 (`G/screen_pool.rs:31`, `:85`, `:102` [Q]); Android fordert die Bildwiederholrate des Telefons (`A/ui/screen/ScreenViewScreen.kt:132` [Q]) | Desktop 60: **ohne Beleg**; gilt auch an Bildschirmen mit 120, 144 oder 240 Hz |
| Bildrate aus Bitrate | `quality_for`: `fps = Ziel-Bitrate / (Pixel × 0,05)`, begrenzt auf `max_fps` (`E/core/rate_control.rs:363`, `:387-394` [Q]) | `BITS_PER_PIXEL` 0,05 („kalibriert mit `se screen bench`“, Messwerte nicht abgelegt: **ohne prüfbaren Beleg**) |
| Bildratenwechsel | Jeder Wechsel der Bildrate startet den Kodierer neu: Schlüsselbild, neue Bildgruppe (`rate_control.rs:396-402`, `E/core/host_mode.rs:116-120`, `host_pipeline_produce.rs:56-61` [Q]); Schritte nur ab 25 % (`STEP_UP_HEADROOM` 1,25, `rate_control.rs:371`) | 1,25: **ohne Beleg** (Kommentar: „does not flap“) |
| Startbitrate | 6 000 000 bit/s direkt, 2 000 000 bit/s über Relay (`rate_control.rs:22-24` [Q]) | Beleg „A1“ der Entscheidung 2026-10-17, keine Messung |
| Bitratenregelung | Zwei Regler nach GCC, Minimum gilt: verzögerungsbasiert aus `FRAME_ACK` (Steigung über 20 Bilder, Überlast → 0,85 × Empfangsrate, sonst Probe × 1,25 je max(RTT, Bildabstand)); verlustbasiert aus `Connection::stats()` je Sekunde (`rate_control.rs:1-11`, `:25-47`, `:340-358` [Q]). Wachstum nur, wenn der Kodierer mindestens 50 % des Ziels nutzt (`APP_LIMITED_SHARE`, `:45-47`) | Untergrenze 150 000 bit/s, Obergrenze 240 000 000 bit/s = MaxBR von H.264 Level 5.1 Baseline/Main (`:18-21`); `OVERUSE_SLOPE` 0,05, `OVERUSE_PERSISTENCE` 3, `DECREASE_FACTOR` 0,85, `INTERVAL_GROWTH` 1,25, `LOSS_HIGH` 0,10, `LOSS_LOW` 0,02: Werte aus GCC-Entwurf (refs C2), 0,05 und 3 ohne Messung |
| Makroblockrate | Bildrate gedeckelt durch `max_macroblocks_per_second` der gewählten Codec-Paarung (`pacing.rs:229-253`, `host_pipeline_produce.rs:47-55` [Q]); unter Windows aus dem H.264-Level, den das MFT in seinem ersten passenden Ausgabetyp nennt (`E/os/windows/mf_common.rs:425-480` [Q]) | Level-Tabelle H.264 Annex A (`E/core/frame_size_avc.rs:14-39` [Q]); ob der aufgezählte Ausgabetyp den höchsten unterstützten Level nennt: [U] |
| Auflösung | Quelle eingepasst in die Anzeigegröße des Betrachters und die Grenze des Kodierers (`E/core/host_pipeline.rs:241-278` [Q]); Verkleinerung auf der CPU mit Flächenmittelung (`E/core/pixel_convert.rs:1-6`, `:425-440` [Q]) | v1-Grenze 4096 × 2304 (`E/core/frame_size_caps.rs:7-19`); OpenH264 höchstens 3840 × 2160 (`E/os/shared/codec_inventory.rs:24`) |
| Taktschutz | Bild wird vor der Kodierung verworfen, wenn unbestätigte Bytes + erwartete Bildgröße > Ziel × (RTT + 100 ms); über Relay höchstens 768 KiB (`pacing.rs:13-39`, `:105-131` [Q]) | 100 ms (`QUEUE_ALLOWANCE_US`), 768 KiB / 1 MiB Relay-Warteschlange (Beleg NR N19 laut Kommentar) |
| Rücksetzen | Ältestes unbestätigtes Bild älter als 1 s oder ≥ 1024 Bilder in Flug → Strom zurücksetzen, nächstes Bild Schlüsselbild auf neuem Strom (`pacing.rs:20-24`, `:112-117` [Q]) | 1 s: Planfestlegung C3 (2026-10-17), keine Messung |
| Paketierung | Ein QUIC-Unistrom je Bildgruppe, Kopf 20 Byte, Satzkopf 21 Byte (`E/core/gop_stream.rs:1-8`, `E/core/wire_binary.rs:1-9` [Q]) | Kopfanteil < 0,1 % bei 1080p [S, `plan.md` C5] |
| Transport | QUIC (iroh/noq), TLS 1.3, Cubic, eigene Verbindung je Betrachter (`plan.md` B2 [Q]) | `plan.md` B2 |
| Hybridbild | Abgeschaltet: der Betrachter setzt `hybrid_regions = false`, weil es am 2026-10-27 „about 1 picture/s“ und flackernde, meist schwarze Bilder lieferte (`E/core/viewer_client.rs:229-235` [Q]) | — |
| Steuerstrom | Gastgeber öffnet ihn, Priorität 1 über den Bildgruppen (P4); `PING` je Sekunde (`viewer_client.rs:38` [Q]) | — |

**Rechenbeispiel aus dem Code (keine Schätzung, deterministisch aus `quality_for`):** Bei der Startbitrate 6 Mbit/s ergibt sich für 1920 × 1080 `floor(6 000 000 / (2 073 600 × 0,05))` = 57 Bilder/s, für 2560 × 1440 32 Bilder/s, für 3840 × 2160 14 Bilder/s. Das Ziel steigt nur, solange der Kodierer ≥ 50 % davon nutzt; jede Stufe der Bildrate kostet einen Kodiererneustart mit Schlüsselbild. Ein Desktop-Betrachter bekommt nie mehr als 60 Bilder/s.

### 1.3 Gastgeber Windows

| Station | Verfahren, Kopien, Speicher | Stelle |
|---|---|---|
| Aufnahme | Monitor und Fenster über Windows Graphics Capture (WGC); DXGI Desktop Duplication nur für einen Monitor ohne WGC (`docs/oberflaechen/bildschirm.md` Abschnitt 6). WGC: Bildpool `CreateFreeThreaded`, 2 Puffer, BGRA (`E/os/windows/capture_retirement.rs:55-60` [Q]); der Kompositor füllt den Pool fortlaufend, se holt je Takt mit `TryGetNextFrame` das jüngste Bild und schließt ältere (`E/os/windows/capture_wgc.rs:188-207` [Q]); `MinUpdateInterval` wird nicht gesetzt (Suche ohne Treffer [Q]). DXGI: `AcquireNextFrame` mit Wartezeit 0 nach dem ersten Bild (`E/os/windows/capture_dxgi.rs:353-365` [Q]); Dirty- und Move-Rechtecke werden gelesen (`:278-291`) | Bild liegt als D3D11-Textur im GPU-Speicher |
| Kopie auf die CPU | Jede genommene Textur → `CopySubresourceRegion` in eine Staging-Textur (GPU-Kopie) → `Map` → Kopie in zugelassenen CPU-Speicher: 8,3 MB je 1080p-Bild, 33 MB je 2160p-Bild (`E/os/windows/capture_readback.rs:201`, `:221` [Q]; `E/os/windows/d3d_device.rs:1-6`: „There is no GPU path to the encoder“) | 2 Kopien je Bild |
| Skalierung | CPU-Flächenmittelung, wenn Ziel kleiner als Quelle (`E/os/shared/host_worker_produce.rs:84` [Q]) | CPU |
| Farbumwandlung | BGRA → I420 auf der CPU, skalar, ein Thread (`E/core/pixel_convert.rs:311-340` [Q]: keine SIMD-, keine Thread-Aufrufe), danach I420 → NV12 für Media Foundation (`E/os/windows/mf_capture_media.rs:48`, `mf_encoder.rs:259` [Q]) | 2 CPU-Durchläufe |
| Kodierung | Rangfolge: Hardware-MFT (asynchron, `MFT_ENUM_FLAG_HARDWARE`) → Software-MFT `Mfh264enc` (synchron) → OpenH264 (nur nach Zustimmung) → Kacheln (`mf_common.rs:123-140`, `:246-278`; `E/core/negotiate.rs:1-7` [Q]). Eingabe NV12 im Systemspeicher, auch für das Hardware-MFT („the GPU path follows with SCR-M4“, `mf_common.rs:11-14` [Q]). Einstellungen: `CODECAPI_AVLowLatencyMode`, B-Bilder 0, CBR, mittlere Bitrate (`E/os/windows/mf_media.rs:188-210` [Q]). Nicht gesetzt: Puffergröße (VBV), GOP-Länge, LTR, Intra-Refresh, Slices [Q: keine Treffer]. Bitrate zur Laufzeit; Bildratenwechsel = Neustart (1.2) | GOP-Länge: Vorgabe des MFT [U] |
| Ausgabe | Zugriffseinheit `Lock` → `to_vec` → zugelassene Kopie (`mf_media.rs:150-172`, `E/os/windows/mf_capture_output.rs:1` [Q]) → Pipe zum Daemon | 2 bis 3 Kopien der kodierten Bytes |
| Zeiger | WGC: mit Überlagerung (P6) aus der Aufnahme genommen, Form und Position über `GetCursorInfo` alle 8 ms (`E/os/windows/cursor_overlay.rs:1-10`, `capture_retirement.rs:77-80` [Q]); ohne Überlagerung im Bild. DXGI: der Zeiger fehlt im Bild (Grenze `no_cursor_capture`, `docs/oberflaechen/bildschirm.md` Abschnitt 6 und 10) | — |
| Takt | Tokio-Intervall im Daemon (1.1). Windows gibt Prozessen ohne `timeBeginPeriod` keine höhere Auflösung als die Standardauflösung des Systems (R13); se ruft weder `timeBeginPeriod` noch einen hochauflösenden Wartetimer auf (Suche [Q]) | Standardauflösung des Systems: [A] 15,6 ms; Wirkung auf das Tokio-Intervall [U], Messung MB1 |

### 1.4 Gastgeber Linux X11

| Station | Verfahren, Kopien, Speicher | Stelle |
|---|---|---|
| Aufnahme | **Bildschirmfoto je Takt**: MIT-SHM `GetImage` des Monitors oder der Composite-Pixmap eines Fensters in ein mit dem X-Server geteiltes memfd; ohne MIT-SHM `GetImage` über den Socket (`E/os/linux_os/x11_capture.rs:1-10` [Q]). DAMAGE ungenutzt. Gleichheit mit dem Vorbild per Hash und exaktem Vergleich → `Unchanged` (ebenda) | CPU; 1 Kopie durch den X-Server, 1 Vergleichsdurchlauf |
| Zeiger | Mit Überlagerung (P6) nicht gemischt, Form über XFixes; ohne Überlagerung in das CPU-Bild gemischt (`E/os/linux_os/x11_capture.rs:230`, `x11_cursor.rs` [Q]) | — |
| Skalierung, Farbumwandlung | CPU wie 1.3 (`E/os/native_ffi/openh264_capture.rs:56` [Q]) | CPU |
| Kodierung | **Ohne Zustimmung zu OpenH264 nur die verlustfreien Kacheln** (64 × 64, Einfarbig/Roh/LZ4; `E/core/tiles_codec.rs:1-11` [Q]); mit Zustimmung OpenH264 Constrained Baseline, `ScreenContentRealTime`, `Complexity::Low`, Bitratenmodus, keine Bildauslassung (`E/os/native_ffi/openh264_encode.rs:118-131` [Q]). Linux hat keinen Hardwarekodierer (keine VA-API-, NVENC- oder Vulkan-Video-Treffer [Q]; `E/os/codec_facade.rs:8-17`: Media Foundation „none elsewhere“). OpenH264 lädt Cisco-Binärdatei erst nach Lizenzannahme durch den Nutzer (`docs/oberflaechen/bildschirm.md` Abschnitt 5) | Software, CPU |

### 1.5 Gastgeber Linux Wayland

| Station | Verfahren, Kopien, Speicher | Stelle |
|---|---|---|
| Aufnahme | **Fortlaufender Bildstrom des Kompositors** über Portal ScreenCast und PipeWire; Format nur als Shared Memory angeboten (kein `VIDEO_modifier`, daher kein DMA-BUF), Bildrate 0 bis 1000 variabel (`E/os/linux_os/spa_pod.rs:104-146` [Q]). Der Kompositor schreibt jedes Bild in SHM (GPU → CPU beim Kompositor); der PipeWire-Rückruf kopiert jedes Bild in `LatestFrame` (`E/os/linux_os/pipewire_stream.rs:1-7`, `:195-230` [Q]); der Takt nimmt das jüngste | CPU; mindestens 2 Kopien (Kompositor → SHM, SHM → `LatestFrame`); eine weitere beim Übergeben an den Kodierer [U] |
| Zeiger | `CursorMode::Embedded`, also immer im Bild (`E/os/linux_os/portal_source_binding.rs:58-63` [Q]); keine Überlagerung (P6-Bericht „Ungelöste Punkte“ 6) | — |
| Skalierung, Farbumwandlung, Kodierung | wie X11 (1.4) | Software |
| Eingabe | Portal RemoteDesktop, je Ereignis ein D-Bus-Aufruf `Notify*` (`E/os/linux_os/portal_input.rs:1-15` [Q]) | — |

### 1.6 Gastgeber Android

| Station | Verfahren | Stelle |
|---|---|---|
| Aufnahme bis Kodierer | **Fortlaufender Strom**: VirtualDisplay rendert direkt in die Eingabefläche des Kodierers (`COLOR_FormatSurface`), keine CPU-Kopie (`A/screen/ScreenEncoder.kt:81`, `:367` [Q]) | GPU, Nullkopie |
| Kodierung | MediaCodec, H.264 `cb` oder `high`; `KEY_LATENCY` 1, B-Bilder 0, `KEY_PRIORITY` 0, `KEY_OPERATING_RATE` = fps, CBR (sonst VBR), `KEY_I_FRAME_INTERVAL` 10 s, `KEY_REPEAT_PREVIOUS_FRAME_AFTER` 500 ms (`A/screen/ScreenEncoder.kt:306-310`, `:360-378`; `A/screen/ScreenCodecLatency.kt:20-35` [Q]) | 10 s und 500 ms ohne Beleg |
| Takt | Kodierer läuft frei mit dem Kompositor; die Pipeline holt je Takt ein Bild aus einer Warteschlange von höchstens 8 (1.1); Warte­zeit je Bild `max(2000/fps, 50)` ms (`E/os/android/bridge_source.rs:100-102` [Q]); Bitratenwechsel erst ab 10 % (`:47`) | — |
| Zeiger | keiner im Bild [A: MediaProjection zeichnet keinen Mauszeiger bei Toucheingabe; mit angeschlossener Maus nicht geprüft] | — |

### 1.7 Betrachter Desktop (wgpu und glow)

| Station | Verfahren | Stelle |
|---|---|---|
| Empfang | Daemon nimmt Bildströme an, bestätigt bei Ankunft, leitet über Loopback-TCP weiter (1.1) | `D/screen_ipc.rs:322-336` [Q] |
| Dekodierer | Gleiche Registry wie der Gastgeber: Windows Media-Foundation-H.264-Dekodierer als **synchrones MFT ohne D3D-Manager** (Aufzählung nur `MFT_ENUM_FLAG_SYNCMFT`, `mf_common.rs:138-140`; keine D3D-/DXVA-Aufrufe in `E/os/windows/mf_decoder.rs` [Q]) → Ausgabe NV12 im Systemspeicher; danach OpenH264 (nur nach Zustimmung); Kacheln. Linux: OpenH264 (nur nach Zustimmung), sonst nur Kacheln | Software [A: der MF-Dekodierer nutzt DXVA nur mit gesetztem D3D-Manager; Microsoft-Seite in dieser Sitzung nicht abgerufen] |
| Folge für Linux-Betrachter | Ohne OpenH264 kann ein Linux-Betrachter kein H.264 dekodieren; die Aushandlung wählt dann auch von einem Windows-Gastgeber mit Hardwarekodierer die verlustfreien Kacheln (`negotiate.rs:1-7` [Q]) | — |
| Darstellung | wgpu: NV12/I420-Ebenen per `queue.write_texture` hochgeladen (3,11 MB je 1080p-Bild), Shader rechnet RGB (`G/screen_wgpu_yuv.rs:1-13` [Q]). glow, Kacheln, Gruppenansicht: BGRA als egui-Textur (CPU-Umrechnung, `docs/oberflaechen/bildschirm.md` Abschnitt 8) | GPU-Upload je Bild |
| Bildtakt | `AutoVsync`, `desired_maximum_frame_latency: Some(1)`; „Niedrigste Latenz“ = `AutoNoVsync`, wirkt erst beim nächsten Start; `Mailbox` nie (`G/screen_viewer_app.rs:52-61` [Q]); jedes dekodierte Bild weckt den GUI-Thread, gezeigt wird das jüngste | ≤ 1 Bild Warteschlange |
| Latenzanzeige | Statistik misst Anzeige-Übergabe minus Aufnahmezeit, korrigiert um den Uhrversatz, ohne Anzeigeweg (`G/screen_decode_thread.rs:280-292` [Q]) | — |

### 1.8 Betrachter Android

| Station | Verfahren | Stelle |
|---|---|---|
| Dekodierer | MediaCodec, Hardware zuerst (`sortedByDescending { it.isHardwareAccelerated }`), `KEY_LOW_LATENCY` wenn `FEATURE_LowLatency` (`A/ui/screen/ScreenDecoder.kt:336-353` [Q]) | Hardware |
| Darstellung | Ausgabe direkt auf eine `SurfaceView` mit `releaseOutputBuffer(index, true)` ohne Zeitstempel, also bei der nächsten Bildwiederholung (`ScreenDecoder.kt:370-390`, `ScreenViewScreen.kt:7` [Q]) | Nullkopie |
| Stau | Ältestes Bild der Warteschlange älter als `STALL_US` 1 s → leeren, Schlüsselbild anfordern (`E/os/android/viewer_sink.rs:54` [Q]) | 1 s ohne Messung |

### 1.9 Eingabe zurück

| Abschnitt | Verfahren | Stelle |
|---|---|---|
| Desktop-Betrachter | egui-Ereignisse eines Bildes werden zu Stapeln gebündelt, Sendethread → Loopback-TCP → Daemon → verschlüsselter Steuerstrom (`G/screen_input_map.rs` Kopf, `G/screen_decode_thread.rs:1-8`, `D/screen_ipc.rs:372-383` [Q]) | Wartezeit bis zum nächsten egui-Durchlauf [U], Messung MB6 |
| Android-Betrachter | Berührungen → JNI → Steuerstrom (`docs/oberflaechen/bildschirm.md` Abschnitt 8) | — |
| Gastgeber-Daemon → Hostprozess | `INPUT` über dieselbe Pipe wie `Produce`; der Hostprozess bearbeitet Anfragen nacheinander, eine Eingabe wartet also hinter einer laufenden Aufnahme und Kodierung (`host_worker_run.rs:219-235`, `host_worker_main.rs:213-240` [Q]); Frist `INPUT_TIMEOUT` 2 s (`host_worker_client.rs:86`) | ≤ eine Bildoperation Zusatzwartezeit |
| Auslösen | Windows `SendInput` (`E/os/windows/input_send.rs:1-15`), X11 XTest (`x11_input.rs:1-7`), Wayland Portal `Notify*` (`portal_input.rs:1-15`), Android Bedienungshilfe (`docs/oberflaechen/bildschirm.md` Abschnitt 7) [Q] | — |

### 1.10 Antwort auf „macht se screenshots, wie capturen andere den pc in echtzeit“

| Gastgeber | Bildschirmfoto oder fortlaufender Strom | Wie oft | Kopien bis zum Kodierer | Speicher |
|---|---|---|---|---|
| Windows WGC | Fortlaufender Strom des Kompositors in einen Pool von 2 Puffern; se nimmt je Takt das jüngste Bild | Takt des Daemons (≤ angeforderte Bildrate, Desktop 60) | GPU-Kopie in Staging, CPU-Kopie, Farbumwandlung CPU (2 Durchläufe), Kodierer-Upload durch den Treiber | GPU → CPU → GPU |
| Windows DXGI | Duplizierte Ausgabe; `AcquireNextFrame` je Takt ohne Warten | Takt | wie WGC | GPU → CPU → GPU |
| Linux X11 | **Bildschirmfoto** (`XShmGetImage`) je Takt, danach Vergleich mit dem Vorbild | Takt | X-Server-Kopie, Hash und Vergleich, Farbumwandlung CPU | CPU |
| Linux Wayland | Fortlaufender PipeWire-Strom, nur SHM | so oft der Kompositor liefert; se nimmt das jüngste je Takt | Kompositor GPU → SHM, SHM → `LatestFrame`, Farbumwandlung CPU | GPU → CPU |
| Android | Fortlaufender Strom direkt in den Kodierer | Kompositortakt | keine | GPU |
| Referenzen | Sunshine: Desktop Duplication oder WGC mit `FrameArrived`, Farbumwandlung im Pixel-Shader, Kodierer liest die Textur (R01); Parsec: Desktop Duplication, „The raw frame never leaves the GPU“ (R05); Sunshine Linux: KMS, wlroots-Screencopy, PipeWire mit DMA-BUF (R01) | Kompositortakt mit eigenem hochauflösendem Takt (R01) | 0 CPU-Kopien | GPU |

### 1.11 Herkunft der „2 Mäuse“

Beobachtung des Nutzers (Vorgabe 5). Erklärung am Quelltext je Kombination; „Stand v0.5.196“ ist der installierte Stand, „nach P6“ der Arbeitsbaum.

| Gastgeber, Aufnahme | Betrachter, Modus | Stand v0.5.196 | Nach P6 | Ursache |
|---|---|---|---|---|
| Windows WGC | Desktop, Steuern | **2 Zeiger**: lokaler Systemzeiger über dem Bild plus Gastgeberzeiger im Bild, dieser um die volle Bildlatenz verspätet | 1 Zeiger: Überlagerung an der lokalen Position, Systemzeiger über dem Bild ausgeblendet | `SetIsCursorCaptureEnabled(true)` bis P6 (`capture_retirement.rs:77-80` [Q]); P6-Bericht „Betrachter“ |
| Linux X11 | Desktop, Steuern | 2 Zeiger (XFixes ins Bild gemischt) | 1 Zeiger | `x11_cursor.rs` [Q]; P6 |
| Linux Wayland | Desktop, Steuern | 2 Zeiger | **weiter 2 Zeiger** | `CursorMode::Embedded` (`portal_source_binding.rs:58-63` [Q]); P6 lässt Wayland aus |
| Windows DXGI | Desktop, Steuern | 1 Zeiger (nur lokal; das Bild hat keinen) | 1 Zeiger | Desktop Duplication liefert den Zeiger getrennt (refs A2); se liest ihn nicht |
| Windows DXGI | Desktop oder Android, Ansehen | **kein** Gastgeberzeiger sichtbar | kein Gastgeberzeiger | wie oben |
| Android | Desktop, Steuern | 1 Zeiger (lokal) | 1 Zeiger | kein Zeiger im Bild [A] |
| jeder Gastgeber | Desktop, Ansehen | lokaler Zeiger plus Gastgeberzeiger (gewollt: der Betrachter steuert nicht) | gleich, Gastgeberzeiger als Überlagerung (WGC, X11) | — |
| jeder Gastgeber mit Zeiger | Android, Steuern mit Touch | 1 Zeiger (im Bild) | 1 Zeiger (Überlagerung, Gastgeberposition) | P6-Bericht Abweichung 8 |
| jeder Gastgeber mit Zeiger | Android mit angeschlossener Maus | [A] 2 Zeiger: Android zeichnet einen eigenen Mauszeiger | unverändert | nicht geprüft |

Schluss der übrigen Fälle: Abschnitt 4.9 und Pakete BK2b (DXGI) und BK8a (Wayland).

### 1.12 Tatsächlich benutzte Kodierer und Dekodierer

| Gastgeber → Betrachter | Ergebnis der Aushandlung heute |
|---|---|
| Windows → Windows | H.264 High über Hardware-MFT (falls vorhanden, sonst Software-MFT) → Software-Dekodierung mit MF auf der CPU |
| Windows → Linux ohne OpenH264 | **Kacheln** (verlustfrei, BGRA/LZ4) |
| Windows → Linux mit OpenH264 | H.264 → OpenH264-Software-Dekodierung |
| Windows → Android | H.264 → MediaCodec Hardware |
| Linux (X11/Wayland) ohne OpenH264 → jeder | **Kacheln** |
| Linux mit OpenH264 → Windows, Linux mit OpenH264, Android | H.264 Constrained Baseline (Software, CPU) → MF-Software, OpenH264 oder MediaCodec |
| Android → Windows | H.264 MediaCodec (Hardware) → MF-Software |
| Android → Linux ohne OpenH264 | Kacheln: der Android-Gastgeber kodiert dann Kacheln aus Standbildern [A: Pfad `FLAG_STILL` in `bridge_channels.rs:10-13`; Bildrate dieses Wegs nicht gelesen] |
| Android → Android | MediaCodec → MediaCodec (Hardware beidseitig) |

Codecs überhaupt: nur H.264 und Kacheln (`negotiate.rs:18-31` [Q]; Code 3 ist für AV1 reserviert).

### 1.13 Software-Grenzen unter der Hardware-Grenze (Vorgabe 4, Teil a)

| Nr. | Wert | Stelle | Wirkung | Beleg |
|---|---|---|---|---|
| G1 | Desktop-Betrachter fordert 60 Bilder/s | `G/screen_pool.rs:31` | Bildrate nie über 60, auch an 120-, 144-, 240-Hz-Bildschirmen | ohne Beleg |
| G2 | Bildrate = Bitrate / (Pixel × 0,05) | `rate_control.rs:363`, `:387-394` | Bildrate fällt mit der Auflösung (2160p: 14 Bilder/s bei Start) | ohne prüfbaren Beleg |
| G3 | Neustart des Kodierers bei jeder Bildratenstufe | `host_mode.rs:116-120`, `host_pipeline_produce.rs:56-61` | Schlüsselbild je Stufe; Hochlauf erzeugt Schlüsselbild-Folgen | Kommentar `rate_control.rs:396-402` |
| G4 | Startbitrate 6 / 2 Mbit/s | `rate_control.rs:22-24` | Erste Sekunden unscharf und langsam | A1, keine Messung |
| G5 | Wachstum nur bei ≥ 50 % Nutzung | `rate_control.rs:45-47` | Ruhiger Bildschirm hält die Rate niedrig; beim Bewegungsbeginn zuerst Unschärfe | ohne Messung |
| G6 | Obergrenze 240 Mbit/s | `rate_control.rs:20-21` | H.264-Level-5.1-Grenze; für HEVC, AV1 oder höhere Level zu niedrig, für LAN-Codecs (R06) weit zu niedrig | Level-Tabelle, nicht Hardware |
| G7 | Makroblockrate aus dem ersten aufgezählten MF-Ausgabetyp | `mf_common.rs:425-480` | Bildrate kann unter der Fähigkeit des Kodierers liegen | [U] |
| G8 | Kein H.264 unter Linux ohne Nutzerhandlung (OpenH264-Zustimmung) | `codec_inventory.rs:1-10` | Linux-Gastgeber und -Betrachter fallen auf Kacheln | Lizenzbedingung Cisco (refs B2) |
| G9 | Takt aus Tokio-Intervall, Windows ohne hohe Timerauflösung | `host_session.rs:150-152` | Taktfehler bis zur Systemtimerauflösung [A] | — |
| G10 | Ein Bild zur Zeit, Anfrage/Antwort über Pipe | `host_pipeline_produce.rs:109-116` | Bildrate ≤ 1 / (Aufnahme + Umwandlung + Kodierung + Pipe + Schreiben) | — |
| G11 | CPU-Rücklesen und CPU-Farbumwandlung unter Windows | `capture_readback.rs:201-221`, `pixel_convert.rs:311-340` | Hardwarekodierer wartet auf CPU-Arbeit | `d3d_device.rs:1-6` |
| G12 | Software-Dekodierung am Desktop | `mf_common.rs:138-140`, `mf_decoder.rs` | Dekodierzeit auf der CPU | — |
| G13 | `SHARPNESS_FPS` 15 bei „Schärfe vor Flüssigkeit“ | `E/core/host_quality.rs:8` | Nutzerwahl; Wert ohne Beleg | ohne Beleg |
| G14 | Rücksetzen nach 1 s | `pacing.rs:21` | Bis 1 s Standbild vor Erholung | C3 ohne Messung |
| G15 | Android: Schlüsselbild alle 10 s, Wiederholung nach 500 ms | `ScreenEncoder.kt:306-310` | periodische Schlüsselbild-Spitzen | ohne Beleg |
| G16 | WGC ohne `MinUpdateInterval` | `capture_wgc.rs` (kein Treffer) | Laut Sunshine „may be capped to 60fps“ (R01 `display_wgc.cpp:159-165`) | R01 |
| G17 | Präsentationsmodus nur beim Start änderbar | `screen_viewer_app.rs:1-8` | „Niedrigste Latenz“ braucht Neustart | eframe-Grenze laut Kommentar |

---

## 2 Referenzuntersuchung

### 2.0 Quellen

| Kennung | Quelle | Version / Commit | Abruf |
|---|---|---|---|
| R01 | Sunshine (Quelltext, GPL-3.0), https://github.com/SunshineStream/Sunshine (früher LizardByte) | Tag `v2026.914.233613`, Commit `63d35f702ee9e362e43263742981836ec0710384` | 2026-11-14, `git clone` |
| R02 | moonlight-common-c (GPL-3.0), https://github.com/moonlight-stream/moonlight-common-c | Commit `94c65fd364d17b223f00a8763c9a5d94a66cb8dd` | 2026-11-14 |
| R03 | Moonlight Qt (GPL-3.0), https://github.com/moonlight-stream/moonlight-qt | Tag `v6.2.0`, Commit `de2467e433821664cdd2224aad8c89a625be1ad9` | 2026-11-14 |
| R04 | RustDesk (AGPL-3.0), https://github.com/rustdesk/rustdesk | Tag `1.5.3`, Commit `95cca3f661f635b3cc85c04935fe8d96d8f432ef` | 2026-11-14 |
| R05 | Parsec: https://parsec.app/blog/description-of-parsec-technology-b2738dcc3842 (ohne Datum); Hilfeseiten support.parsec.app (Abruf verweigert, HTTP 403; Inhalte nur aus Suchauszügen); `recherche.md` Q45 | — | 2026-11-14 |
| R06 | Steam Remote Play: https://www.gamingonlinux.com/2026/09/steam-beta-adds-experimental-new-pyrowave-video-codec-for-remote-play/ (2026-09-22, Sekundärquelle; Valve-Hilfeseite lieferte keinen Artikeltext); PyroWave https://github.com/Themaister/pyrowave (MIT) | PyroWave Commit `a32b05e03d70826b19d4d11ff74fe166ed74371b` | 2026-11-14 |
| R07 | Shadow: support.shadow.tech (Abruf verweigert, HTTP 403; Inhalte nur aus Suchauszügen der Artikel „Streaming Preferences Explained TCP UDP“, „Configuring Bitrate and Bandwidth in Shadow PC“, „Using Color Enhancement on Shadow PC“, „Using High Efficiency Video Coding H 265“, „Shadow PC FPS Limitations“) | — | 2026-11-14 |
| R08 | WebRTC: W3C „MediaStreamTrack Content Hints“, https://www.w3.org/TR/mst-content-hint/ (Working Draft 2025-09-19); GCC-Entwurf und RFC 8834/8854 über `docs/refs/bildschirm-streaming-2026-10-17.md` C2 und `recherche.md` 6.1 | — | 2026-11-14 |
| R09 | Microsoft RDP: https://learn.microsoft.com/en-us/azure/virtual-desktop/graphics-encoding (git `34b4f117`, aktualisiert 2025-10-23); MS-RDPEGFX „YUV420p Stream Combination for YUV444 mode“ https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/8131c1bc-1af8-4907-a05a-f72f4581160f (git `1a26d38e`); RDP Shortpath `recherche.md` Q49, Q50 | — | 2026-11-14 |
| R10 | NVIDIA Video Codec SDK 13.0, NVENC Programming Guide, Abschnitte 3.3 und 8.13, https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html | 13.0 | 2026-11-14 |
| R11 | AMD AMF `amf/doc/AMF_Video_Encode_API.md`, https://github.com/GPUOpen-LibrariesAndSDKs/AMF | Commit `57ee0cfb0e1a08d9041f77435653794c1b012be0` (2026-10-16) | 2026-11-14 |
| R12 | Vulkan Video: Finalisierung H.264/H.265-Encode Dezember 2023, AV1-Encode November 2024; RADV-Encode hinter `RADV_PERFTEST=video_encode` (Suchauszüge Khronos/Phoronix/Rastergrid, Sekundärquellen) | — | 2026-11-14 |
| R13 | Microsoft `timeBeginPeriod`, https://learn.microsoft.com/en-us/windows/win32/api/timeapi/nf-timeapi-timebeginperiod (git `92672624`, aktualisiert 2024-02-22) | — | 2026-11-14 |
| R14 | Microsoft `GraphicsCaptureSession.MinUpdateInterval`, https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.minupdateinterval (Moniker ab `winrt-26100`) | — | 2026-11-14 |
| R15 | `docs/refs/bildschirm-streaming-2026-10-17.md` (A1–A5 Aufnahme, B1–B4 Kodierung, C1–C4 Transport; Primärquellen dort mit Abrufdatum 2026-10-17) | — | 2026-10-17 |
| R16 | `docs/plaene/2026-11-14-verbindungen-je-zweck/recherche.md` (Q01–Q57) | — | 2026-11-14 |

### 2.1 Sunshine (Gastgeber) und Moonlight (Betrachter)

**Aufnahme.** Windows: Desktop Duplication (`ddx`, mit `DuplicateOutput1` für weitere Formate, `src/platform/windows/display_base.cpp:82`, `AcquireNextFrame` mit Wartezeit `:150`) oder WGC (`display_wgc.cpp:142` `CreateFreeThreaded` mit 2 Puffern, `:144` Ereignis `FrameArrived`, `:159-165` `MinUpdateInterval(4ms)` „250Hz“, sonst Warnung „Screen capture may be capped to 60fps“). Linux: `nvfbc`, `wlr` (wlr-screencopy), `kms` (DRM-Framebuffer, braucht `CAP_SYS_ADMIN`, `src/platform/linux/kmsgrab.cpp:47-51`), `kwin`, `x11`, Portal/PipeWire mit DMA-BUF und Modifiern (`src/platform/linux/pipewire.cpp:469`, `:538`, `:629`, `:723-725`); Liste in `docs/configuration.md:2178-2238`. Zeiger im Portal-Fall eingebettet (`portalgrab.cpp:476`). [R01]

**Takt und Pacing (Gastgeber).** Eigener Aufnahmefaden; Pacing in Bildgruppen gegen die Uhr (`display_base.cpp:238-300`); hochauflösender Wartetimer `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` (`misc.cpp:1799-1803`); Timerauflösung 0,5 ms über `NtSetTimerResolution`, sonst `timeBeginPeriod(1)`, `DwmEnableMMCSS(true)`, `HIGH_PRIORITY_CLASS` (`misc.cpp:1171-1183`); GPU-Planungspriorität `D3DKMTSetProcessSchedulingPriorityClass` und `SetGPUThreadPriority(7)` (`display_base.cpp:676-705`); Fadenpriorität `critical` für Aufnahme (`video.cpp:1699`). Unveränderter Bildschirm: Duplikate bis zur Mindestbildrate `minimum_fps_target`, Vorgabe Bildrate/2 (`video.cpp:2434-2436`; `docs/configuration.md:1471`). [R01]

**Farbumwandlung.** Im Pixel-Shader auf der GPU, Ziele 4:2:0 (NV12/P010) und 4:4:4 (`display_vram.cpp:126-135`; Shader `src_assets/windows/assets/shaders/directx/convert_yuv420_*.hlsl`). [R01]

**Kodierung.** NVENC direkt über das SDK (`src/nvenc/`): unendliche GOP, `frameIntervalP` 1, CBR, `zeroReorderDelay`, kein Lookahead, `lowDelayKeyFrameScale` 1, VBV = Bitrate / Bildrate, also ein Bild (`nvenc_base.cpp:260-283`), Tuning `NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY` (`:638`), Intra-Refresh auf Wunsch des Betrachters (`:331-340`), Referenzbild-Invalidierung `nvEncInvalidateRefFrames` für den gemeldeten Bereich, Schlüsselbild nur wenn der Bereich die DPB übersteigt (`:795-830`); `nvenc_split_encode` verteilt ein Bild auf mehrere NVENC-Einheiten, `nvenc_latency_over_power` erzwingt den Hochleistungszustand (`docs/configuration.md:2472`, `:2512`). Intel Quick Sync über FFmpeg mit `async_depth` 1, `low_delay_brc` 1, `low_power` 1, `forced_idr` 1 (`video.cpp:863-912`). AMD AMF über FFmpeg mit `latency` niedrigste und Nutzung „low latency“ (`video.cpp:977-1081`). VA-API mit Einstiegspunkt `VAEntrypointEncSliceLP` (`low_power`, `vaapi.cpp:215`, `:304-306`). Vulkan-Video-Kodierer unter Linux (`src/platform/linux/vulkan_encode.cpp`). Media-Foundation-Kodierer `h264_mf` als weiterer Weg (`video.cpp:1092-1146`). Codecs H.264, HEVC (`hevc_mode`), AV1 (`av1_mode`). [R01]

**Transport und Verlust.** Video in RTP-artigen UDP-Paketen; Reed-Solomon-FEC je Bild (`RtpVideoQueue.c:21`, `:94-104`), `fec_percentage` Vorgabe 20 (`docs/configuration.md:2015`); nicht wiederherstellbares Bild → Meldung an den Gastgeber zur Referenzinvalidierung (`ControlStream.c:148`, `:413`, `:462`, `:1728`), ohne RFI striktes Warten auf IDR (`VideoDepacketizer.c:108`), nach 120 verworfenen Bildern IDR (`:56`, `:151-160`). Verschlüsselung je Strom wählbar (`Limelight.h:32-36`; Sunshine: im LAN aus, `recherche.md` 6.2). [R02]

**Betrachter.** Dekodierer wählbar „Automatisch / Hardware erzwingen / Software erzwingen“, Codec „Automatisch / H.264 / HEVC / AV1“ (`app/settings/streamingpreferences.h:29-43`); Renderer D3D11VA, DXVA2, VA-API, DRM, CUDA, Vulkan über libplacebo (`plvk`), EGL, SDL (`app/streaming/video/ffmpeg-renderers/`); D3D11 mit `DXGI_SWAP_EFFECT_FLIP_DISCARD` und `ALLOW_TEARING` bei ausgeschaltetem V-Sync (`d3d11va.cpp:557`, `:592-607`); Frame-Pacing-Schalter, Pacer mit höchstens 3 wartenden Bildern (`pacer/pacer.cpp:21`); V-Sync-Schalter, HDR, YUV 4:4:4 (`streamingpreferences.cpp:128-162`). [R03]

**Wahl der Datenrate (Moonlight).** Vorgabe aus einer Auflösungstabelle (1080p Faktor 10, 1440p 20, 2160p 40 Mbit/s je 30 Bilder/s, über 60 Bilder/s mit Wurzel skaliert): 1080p60 = 20 Mbit/s, 2160p60 = 80 Mbit/s (`streamingpreferences.cpp:367-415`); Schieberegler bis 150 Mbit/s, freigeschaltet bis 500 Mbit/s (`app/gui/SettingsView.qml:694-696`); Bildratenliste 30, 60 und die Bildwiederholrate des Bildschirms (`SettingsView.qml:575-590`, `:644-649`). Keine automatische Bitratenregelung im Betrachter (Folgerung aus den gelesenen Einstellungen; der Nutzer legt die Rate fest). [R03]

**Randbedingungen.** GPL-3.0 (Code nicht übernehmbar, Verfahren ja); NVENC braucht NVIDIA-Treiber ab Kepler (R15 B1); KMS braucht `CAP_SYS_ADMIN`; Sunshine erhöht Prozess- und GPU-Priorität (Windows).

### 2.2 Parsec

- Aufnahme Windows über Desktop Duplication, „places the frame in video memory for processing“; „The raw frame never leaves the GPU“; Farbumwandlung „on the GPU via pixel shaders“; nur Hardwarekodierung und -dekodierung mit den Schnittstellen von NVIDIA, AMD, Intel; V-Sync standardmäßig an, „but drops accumulated frames“; Flip-Sequential-Präsentation (R05 Blogartikel).
- Veröffentlichte Werte: Kodierung plus Dekodierung „lower than 10 ms“ auf neuer Hardware (R05); Laborwert 4 bis 8 ms Gesamtlatenz bei 240 Bilder/s im selben Netz (Suchauszug „parsec game streaming total latency at 240 frames per second“, nicht geöffnet).
- Transport BUD: UDP, DTLS 1.2, eigene Staukontrolle, Stau erkennen bevor er entsteht (`recherche.md` 6.3, Q45).
- Wahlmöglichkeiten (nur Suchauszüge, Seiten verweigern den Abruf): Bandbreitengrenze `encoder_bitrate` (Vorgabe 10 Mbit/s, über 50 „will noticeably hurt latency“), H.265 am Betrachter (`client_decoder_h265`), 4:4:4 als kostenpflichtige Funktion, Software-Dekodierung über FFmpeg für 4:4:4 ohne Hardware.

### 2.3 Steam Remote Play

- Wahlmöglichkeiten (R06, Sekundärquelle): HDR automatisch, YUV 4:4:4 (Vorgabe aus, „for desktop streaming or reading text on a 4K display“), Qualitätsregler mit konstanter Bildgröße, automatische oder feste Bitrate, Leistungsgraph mit Kodierung + Netz + Dekodierung + Anzeige; Schalter für Hardwarekodierung und -dekodierung und HEVC (Gemeinschaftsquellen, nicht von Valve gelesen).
- PyroWave (Beta, Vorgabe aus): Intra-only-Wavelet-Codec (CDF 9/7, 64 × 64-Blöcke), „fully implemented in Vulkan compute shaders“, „< ~0.1 ms encode and decode at 1080p, < ~0.2 ms at 4K“, 4:2:0 und 4:4:4, Zielraten „~200+ mbit/s“ im kabelgebundenen LAN (README des Repositorys, Commit `a32b05e0`, MIT); Valve: 100 bis 500 Mbit/s manuell, „5-10 times the amount of bandwidth of other streaming codecs“ (R06).
- Randbedingungen: Vulkan auf beiden Seiten; Gigabit-LAN; MIT-Lizenz.

### 2.4 Shadow (öffentlich belegt)

Alle Angaben aus Suchauszügen der Shadow-Hilfeseiten (R07); die Seiten verweigern den automatischen Abruf.

- Transportwahl „Prefer speed (UDP)“ (Vorgabe) und „Prefer reliability (TCP)“, TCP empfohlen bei Paketverlust und Stottern, WLAN, ADSL, Hotspot.
- Bitrate „Auto-detect“ oder frei; Obergrenze 70 Mbit/s; Empfehlung etwa 10 Mbit/s unter der gemessenen Leitung.
- H.265 als Schalter („Low bandwidth mode“ auf Mobilgeräten), Vorgabe aus.
- 4:4:4 „Color Enhancement“ (nicht auf Android, Android TV, VR).
- Bildratengrenze „unlimited, 60 or 50“; höchstens 144 Bilder/s bei 1440p, über 2K teils 120; Bildrate begrenzt durch die Dekodierleistung des Endgeräts.
- Eine eigene Dekodiererwahl ist in den gefundenen Auszügen nicht belegt [U]; Aufnahme, Kodierer und Latenzwerte sind nicht öffentlich gefunden [U].

### 2.5 RustDesk

- Bildrate: Vorgabe 30, höchstens 120, Start 15; Qualitätsverhältnis alle 3 s nach Netzverzögerung (Schwelle 150 ms) (`src/server/video_qos.rs:51-67`). [R04]
- Codecs VP8, VP9, AV1, H.264, H.265, Hardware über `hwcodec` (FFmpeg) und `vram` (D3D11-Texturen ohne CPU-Kopie unter Windows) (`libs/scrap/src/common/{hwcodec.rs,vram.rs,mediacodec.rs}`); Wahl „codec-preference“ und 4:4:4 „i444“ (`libs/scrap/src/common/codec.rs:826`, `:841`); Schalter `enable-hwcodec`, `allow-d3d-render`, `enable-udp-punch` (`libs/base/src/config/keys.rs:31`, `:72`, `:131`).
- Transport: TCP, KCP über UDP (`kcp-sys`, `Cargo.toml:90`; `src/kcp_stream.rs`) und WebRTC mit eigenen Anpassungen (`Cargo.toml:238-256`).
- Randbedingung: AGPL-3.0.

### 2.6 WebRTC-basierte Dienste

- Inhaltshinweise: „detail“ und „text“ → `maintain-resolution` (Bildrate sinkt, Auflösung bleibt), „motion“ → `maintain-framerate`; für AV1 schaltet „text“ die Bildschirminhalt-Werkzeuge ein (R08, Abschnitte 3.2 und 4.3). se folgt heute mit G2 dem Muster „maintain-resolution“, aber ohne Nutzer- oder Inhaltshinweis und auch bei Bewegung.
- GCC verzögerungs- plus verlustbasiert, NACK/RTX, FEC (RFC 8854), PLI (R15 C2, `recherche.md` 6.1); Chrome Remote Desktop über WebRTC mit VP8, VP9, AV1 und GPU-Kodierer (R15 C4).

### 2.7 Microsoft RDP (AVC444, Shortpath)

- Gemischter Modus: Text (rund 80 % der Daten) mit eigenem Textcodec, Bilder mit AVC/H.264 oder RemoteFX, Video mit AVC; „Mixed-mode is only available with software encoding“; Vollbildkodierung mit AVC oder HEVC liefert höhere Bildrate; HEVC „25-50% data compression compared to AVC/H.264“; Dekodierung auf der Hardware des Endgeräts; 4:2:0 Vorgabe, 4:4:4 mit AVC-Vollbild und Gruppenrichtlinie (R09).
- AVC444: ein 4:4:4-Bild wird in zwei 4:2:0-Ströme gepackt (Hauptsicht Luma, Hilfssicht Chroma), die ein gewöhnlicher 4:2:0-Dekodierer dekodiert und der Betrachter wieder zusammensetzt (R09, MS-RDPEGFX).
- Shortpath: UDP mit URCP-Ratenregelung, „Avoid using TCP-based VPN solutions“ (`recherche.md` 6.5).

### 2.8 Herstellerschnittstellen und Plattformwege

| Thema | Befund | Quelle |
|---|---|---|
| NVENC | Tuning „Low latency, with CBR“ / „Ultra-low latency, with CBR“ für Cloud-Gaming; Referenzinvalidierung über `inputTimeStamp`; Intra-Refresh mit kleiner Periode bei fehleranfälligem Netz; Eingaben als registrierte D3D/CUDA-Ressourcen (`NvEncRegisterResource`, `NvEncMapInputResource`) | R10 Abschnitte 3.3, 4.1.2, 8.13 |
| AMF | Nutzung `ULTRA_LOW_LATENCY`, `LOW_LATENCY`; `LOWLATENCY_MODE`; LTR bis 16 Referenzen; Eingaben DX11, Vulkan | R11 Zeilen 186-214, 351-354, 614-643, 734 |
| Quick Sync | Niedrige Latenz über `async_depth` 1, `low_delay_brc`, `low_power` (Verfahren aus R01 `video.cpp:863-912`); oneVPL-Herstellerdokument nicht abgerufen [U] | R01 |
| VA-API | Einstiegspunkt `VAEntrypointEncSliceLP` (Low-Power-Kodierung); libva MIT | R01 `vaapi.cpp:215`; R15 B1 |
| Vulkan Video | Encode H.264/H.265 seit Dezember 2023, AV1 seit November 2024; Treiberunterstützung uneinheitlich (RADV hinter Umgebungsvariable) | R12 |
| Media Foundation | Hardware-MFTs asynchron; LTR-Eigenschaften optional; `CODECAPI_AVLowLatencyMode`; Bitrate zur Laufzeit; HEVC-MFT nur Main | R15 B1 |
| MediaCodec | `KEY_LATENCY`, `KEY_PRIORITY`, `KEY_INTRA_REFRESH_PERIOD`, `PARAMETER_KEY_REQUEST_SYNC_FRAME`, `KEY_LOW_LATENCY` (Dekodierer); AV1-Kodierer ab Android 14 Pflicht | R15 B1 |
| DXGI Desktop Duplication | Zeiger getrennt (`PointerPosition`, `GetFramePointerShape`), Dirty-/Move-Rechtecke, Formate über `DuplicateOutput1` | R15 A2 |
| WGC | `MinUpdateInterval` ab Moniker `winrt-26100`; Sunshine setzt 4 ms und warnt sonst vor 60-Bilder/s-Grenze | R14; R01 `display_wgc.cpp:159-165` |
| KMS/DRM | Framebuffer-Aufnahme ohne Kompositor, `CAP_SYS_ADMIN` nötig | R01 `kmsgrab.cpp:47-51` |
| PipeWire DMA-BUF | Zwei `EnumFormat` (DMA-BUF mit Modifier, SHM als Rückfall) | R15 A3; R01 `pipewire.cpp:538`, `:723-725` |
| Timer Windows | Ab Windows 10 2004 wirkt `timeBeginPeriod` nur für den aufrufenden Prozess; ohne Aufruf keine höhere Auflösung als die Systemvorgabe; unter Windows 11 nicht für verdeckte oder minimierte Fensterprozesse | R13 |
| Anzeige | D3D11 Flip-Modell mit Tearing bei V-Sync aus (R03 `d3d11va.cpp:557`, `:592-607`); Parsec V-Sync an mit Verwerfen angesammelter Bilder (R05); Moonlight Pacer optional (R03) | R03, R05 |

VideoToolbox (Apple) entfällt: keines der Geräte des Nutzers ist ein Apple-Gerät (Auftrag).

**HDR.** Sunshine wandelt HDR auf der GPU (PQ-Shader `convert_yuv420_*_perceptual_quantizer`, R01 `display_vram.cpp:126-135`); Moonlight und Steam bieten HDR (R03, R06); AMF beschreibt HDR-Farbparameter (R11 Zeilen 915-993); WGC empfiehlt `R16G16B16A16_FLOAT` für HDR (R15 A1). se nimmt heute 8-Bit-BGRA auf (1.3–1.5).

### 2.9 Wahlmöglichkeiten und Datenraten der Referenzen (Vorgabe 4, Teil c)

| Wahl | Sunshine/Moonlight | Parsec | Steam | Shadow | RustDesk | RDP |
|---|---|---|---|---|---|---|
| Transport UDP/TCP | nur UDP | nur UDP (BUD) | — [U] | UDP oder TCP | TCP, KCP/UDP, WebRTC (Schalter UDP-Lochstanzen) | UDP (Shortpath) oder TCP |
| Dekodierer | Auto / Hardware / Software | Software für 4:4:4 [Suchauszug] | Hardware-Dekodierung an/aus [Gemeinschaftsquelle] | nicht belegt | Hardware-Codec an/aus, D3D-Darstellung | Hardware des Endgeräts |
| Codec | Auto / H.264 / HEVC / AV1 | H.265 an/aus | HEVC, PyroWave | H.265 an/aus | Auto / VP8 / VP9 / AV1 / H.264 / H.265 | AVC / HEVC (Richtlinie) |
| Bitrate | fest vom Nutzer, 0,5–150 (500) Mbit/s | Grenze, Vorgabe 10 | automatisch oder fest; PyroWave 100–500 | Auto oder fest, bis 70 | Qualitätsstufe | adaptiv |
| Bildrate | 30 / 60 / Bildwiederholrate / frei | — [U] | — [U] | unbegrenzt / 60 / 50 | frei bis 120 | — |
| 4:4:4 | ja | ja (kostenpflichtig) | ja | ja | ja | ja (Richtlinie) |
| V-Sync / Pacing | V-Sync, Frame Pacing | — | — | — | — | — |

Datenraten im Vergleich: se startet mit 6 Mbit/s und begrenzt bei 240 Mbit/s; Moonlight nimmt 20 Mbit/s für 1080p60 und 80 Mbit/s für 2160p60 als Vorgabe; Shadow bis 70 Mbit/s; Steam PyroWave 100 bis 500 Mbit/s im LAN.

### 2.10 Vergleich je Station

| Station | se heute | Referenztechnik | Abstand |
|---|---|---|---|
| Aufnahmetakt | Tokio-Intervall im Daemon, Windows ohne Timerauflösung | eigener Faden, hochauflösender Timer, Kompositorereignis (`FrameArrived`, `AcquireNextFrame` mit Wartezeit) (R01) | Taktfehler und halbe Taktperiode Wartezeit [S], MB1 |
| Aufnahme → Kodierer | GPU → CPU → GPU (Windows), CPU (Linux) | GPU bleibt GPU (R01, R05), DMA-BUF (R01) | 2 Kopien + CPU-Umwandlung je Bild [S], MB3 |
| Farbumwandlung | CPU skalar, 1 Faden | Pixel-Shader (R01, R05) | MB3 |
| Kodierer | Windows MF (Hardware möglich, Systemspeicher), Linux nur Software nach Zustimmung | Herstellerschnittstellen direkt, VA-API, Vulkan (R01), Hardware only (R05) | Linux ohne Hardware; Windows ohne Herstelleroptionen |
| Kodierereinstellung | CBR ohne VBV-Größe, GOP-Vorgabe, Neustart je Bildratenstufe | VBV = 1 Bild, unendliche GOP, RFI, Intra-Refresh (R01, R10) | Schlüsselbild-Spitzen |
| Bildrate | aus Bitrate abgeleitet, Desktop 60 | vom Betrachter gewählt bis Bildwiederholrate (R03), unbegrenzt (R07) | G1, G2 |
| Bitrate | Start 6, langsamer Hochlauf, Grenze 240 | fest hoch (R03), Auto-Messung (R07), LAN-Codecs 100–500 (R06) | G4, G5, G6 |
| Verlust | QUIC wiederholt; Rücksetzen nach 1 s → IDR | FEC + RFI (R02), Intra-Refresh (R10) | `plan.md` E11/B1, B4 |
| Dekodierung | MF Software / OpenH264 / MediaCodec | D3D11VA, VA-API, CUDA, Vulkan (R03) | Desktop ohne Hardware |
| Anzeige | wgpu Fifo mit 1 Bild, Low-Latency nur nach Neustart | Flip mit Tearing, Pacer optional, V-Sync mit Verwerfen (R03, R05) | G17 |
| Eingabe | hinter `Produce` im Hostprozess | eigener Faden, Priorität `critical` (R01 `stream.cpp:1306`) | ≤ 1 Bildoperation [S], MB6 |
| Zeiger | P6: WGC, X11; Wayland eingebettet; DXGI ohne | Sunshine eingebettet (Portal); Desktop Duplication getrennt (R15 A2) | 1.11 |

### 2.11 Veröffentlichte Latenz- und Qualitätswerte

| Referenz | Wert | Quelle |
|---|---|---|
| Parsec | Kodierung + Dekodierung „lower than 10 ms“ auf neuer Hardware; Netz unter 20 ms und über 10 Mbit/s „near-native“ | R05 |
| Parsec | 4 bis 8 ms Gesamtlatenz bei 240 Bilder/s im selben Netz (Labor) | Suchauszug, Artikel nicht geöffnet [U] |
| PyroWave (Steam) | Kodierung und Dekodierung < ~0,1 ms bei 1080p, < ~0,2 ms bei 2160p auf der GPU; 200+ Mbit/s | R06 README |
| Shadow | höchstens 144 Bilder/s bei 1440p, Bitrate bis 70 Mbit/s; keine Latenzwerte gefunden | R07 |
| Sunshine/Moonlight | keine Latenzwerte in den gelesenen Quellen; Vorgabe-Datenraten 20 Mbit/s (1080p60), 80 Mbit/s (2160p60) | R03 |
| RDP | HEVC 25–50 % weniger Daten als AVC bei gleicher Qualität; keine Latenzwerte | R09 |
| WebRTC, RustDesk | keine Latenzwerte in den gelesenen Quellen | R04, R08 |
| se | keine Messwerte (`plan.md` B2, Abschnitt 6 Punkt 17) | — |

---

## 3 Potential je Station

Ordnung nach erwartetem Beitrag zu Latenz, Bildrate und Qualität. Jede Schätzung trägt die Messung, die sie belegt (Messplan in 5.3).

1. **Bildrate an Bitrate gekoppelt, Desktop 60, Startrate niedrig, Neustart je Stufe (G1–G5)** — alle Gastgeber. Rechnung aus dem Code (1.2): 1440p startet bei 32, 2160p bei 14 Bilder/s; kein Desktop-Betrachter bekommt mehr als 60. Jede Stufe kostet ein Schlüsselbild. Erwarteter Beitrag: Bildrate auf Bildwiederholrate des Betrachters (bis Faktor 4 bei 2160p, Faktor 2,4 bei 144 Hz) [S, MB2]. Das knappste Mittel (Kodierer, Netz) ist heute nicht ausgelastet, weil die Software die Bildrate vorher senkt.
2. **Linux ohne Hardwarekodierer und ohne H.264 ohne Zustimmung (G8)** — Linux-Gastgeber und jeder Weg zu einem Linux-Betrachter. Kacheln brauchen ein Vielfaches der H.264-Datenrate [S, MB3/MB8]; OpenH264 Constrained Baseline mit `Complexity::Low` auf der CPU begrenzt Bildrate und Qualität je Bit [S, MB3]. Hardware (VA-API/NVENC/Vulkan) bleibt ungenutzt.
3. **Windows-Gastgeber: GPU → CPU → GPU (G11)** — je 1080p-Bild 8,3 MB Rücklesen, zwei skalare CPU-Durchläufe, Treiber-Upload; bei 2160p das Vierfache [S: mehrere ms je 1080p-Bild, MB3]. Der Hardwarekodierer wartet auf CPU-Arbeit, die weder das Ergebnis noch eine Garantie braucht.
4. **Serielle Kette und Takt (G9, G10)** — Bildrate ≤ 1/(Summe aller Stationen); Taktfehler unter Windows bis zur Systemtimerauflösung [A, MB1]; Aufnahme wartet im Mittel eine halbe Taktperiode auf den nächsten Takt statt auf das Kompositorbild [S, MB1].
5. **Software-Dekodierung am Desktop (G12)** — CPU-Dekodierung und CPU-Plane-Upload; bei 2160p kann die Dekodierzeit das Bildintervall übersteigen [S, MB4].
6. **Codec-Effizienz** — nur H.264 (Baseline bei OpenH264); HEVC spart laut R09 25–50 % bei gleicher Qualität, AV1 hat Bildschirmwerkzeuge (R08, R15 B3) → Qualität bei gegebener Rate [S, MB3 mit PSNR].
7. **Eingabe hinter `Produce` (1.9)** — Eingabewartezeit ≤ Dauer einer Bildoperation [S, MB6].
8. **Erholung nach Stau (G14, G15)** — bis 1 s Standbild, dann Schlüsselbild-Spitze; Android periodisch alle 10 s [S, `plan.md` M7].
9. **Anzeige (G17)** — Fifo mit einem Bild ist bereits knapp; Gewinn nur durch Tearing/VRR im Vollbild und Umschalten ohne Neustart [S: ≤ 1 Bildwiederholung, nur am Gerät messbar].
10. **Zeiger auf Wayland und DXGI** — Wahrnehmung (zwei Zeiger, kein Zeiger), kein Datenratengewinn.

Stationen, die nach Befund bereits an der Hardwaregrenze arbeiten:
- Android-Gastgeber Aufnahme → Kodierer: Nullkopie über die Kodiererfläche (1.6 [Q]).
- Android-Betrachter Dekodierung → Anzeige: Hardware-Dekodierer rendert direkt auf `SurfaceView` (1.8 [Q]).
- Paketierung: Kopfanteil < 0,1 % (`plan.md` C5 [S]).
- Verschlüsselung: nicht die Grenze eines Bildstroms (`plan.md` C0 Punkt 3, Q01, Q26 [R16]).
- Transport-Staukontrolle zwischen Bild und Massenübertragung: in `plan.md` E2, E3, E4 bereits umgesetzt.

---

## 4 Entscheidungsplan

Jede gewählte Technik steht als Ergebnisanforderung **B-xx** (Eigenschaft des fertigen Codes). Zahlenwerte von Referenzen werden nicht übernommen; Grenzen kommen aus Hardwaremeldung oder Messung zur Laufzeit.

### 4.1 Bildtakt, Bildrate, Bitrate (alle Gastgeber und Betrachter)

**Gewählt:**
- **B-01** Die vom Betrachter gewünschte Bildrate ist die Bildwiederholrate seines Bildschirms (Desktop: Monitor des Betrachterfensters; Android wie heute `display.refreshRate`), nicht ein fester Wert. Beleg: G1; R03 (Bildwiederholrate in der Liste), R07 („unlimited“).
- **B-02** Die Bildrate des Gastgebers = Minimum aus gewünschter Bildrate, Bildrate der Quelle und gemessener Kodiererleistung (Kodierzeit p95 je Bild über die letzten Sekunden). Die Bitrate bestimmt nur die Qualität (Quantisierung), nicht die Bildrate. Beleg: G2; R01/R03 (feste Bildrate, Bitrate frei); `maintain-framerate` als Vorgabe für Bewegung (R08).
- **B-03** Ein Bildratenwechsel startet den Kodierer nicht neu und erzeugt kein Schlüsselbild; der Kodierer wird mit der höchsten erlaubten Bildrate geöffnet und bekommt weniger Bilder. Beleg: G3; R01 (Duplikate bis Mindestbildrate statt Neustart, `video.cpp:2434-2436`).
- **B-04** Bei unverändertem Bildschirm sendet der Gastgeber weiter keine Bilder (Bestand `Unchanged`); der Betrachter zeigt das letzte Bild. Keine Mindestbildrate, weil der eigene Betrachter keinen gleichmäßigen Strom braucht [P].
- **B-05** Startbitrate je Gegenstelle und Weg aus der zuletzt gemessenen tragfähigen Rate dieser Gegenstelle (prozesslokal gemerkt, nicht dauerhaft gespeichert [P]); ohne Messwert ein Hochlauf, der das Ziel je RTT verdoppelt, bis die verzögerungsbasierte Erkennung Überlast meldet, danach GCC wie heute. Beleg: G4, G5; GCC-Probing (R15 C2); Verfahren „Slow Start“, Zahlen nicht übernommen.
- **B-06** Die Obergrenze der Bitrate ist das Minimum aus der Grenze des gewählten Kodierers (Level × Profil oder Herstellerangabe), der Grenze des Dekodierers und der gemessenen Netzrate; kein fester 240-Mbit/s-Deckel. Beleg: G6; Vorgabe 4.
- **B-07** Die Makroblock-/Pixelratengrenze kommt aus der höchsten vom Kodierer und Dekodierer gemeldeten Fähigkeit (MF: alle aufgezählten Ausgabetypen und `MFT_ENUM`-Attribute; MediaCodec `PerformancePoint`/`VideoCapabilities`), sonst aus der gemessenen Kodierzeit. Beleg: G7.
- **B-08** Unter Windows läuft der Aufnahmetakt mit hochauflösendem Wartetimer, und der Prozess fordert für die Dauer einer Bildschirmsitzung die höchste Timerauflösung an (`timeBeginPeriod` je Prozess, R13). Beleg: G9; R01 `misc.cpp:1171-1183`, `:1799-1803`.
- **B-09** „Höchste Datenrate“ als Einstellung (Vorgabe: automatisch) auf Desktop, Android, CLI (`se screen view --max-rate`), IPC; MCP über `se`. Beleg 2.9 (alle Referenzen bieten eine Grenze); Grund: getaktete oder geteilte Leitungen.

**Verworfen:**
- Feste Bitrate durch den Nutzer wie Moonlight (R03): Grund: braucht Nutzerhandlung und kennt die Leitung nicht; Vorgabe 2026-11-08 „kein … s9nsyiger sche8ss“.
- Mindestbildrate mit Duplikaten (R01): Grund: se-Betrachter braucht keinen konstanten Strom; Duplikate kosten Kodierung und Netz ohne Ergebnis.
- Bildrate fest auf 60 lassen: Grund G1, Vorgabe 1 („refreshrate“).

### 4.2 Aufnahme

**Windows — gewählt:**
- **B-10** Aufnahme läuft im Hostprozess in einem eigenen Faden, ausgelöst vom Kompositor (WGC `FrameArrived`, DXGI `AcquireNextFrame` mit Wartezeit bis zum nächsten fälligen Bild), nicht vom Daemon-Takt. Beleg R01 `display_wgc.cpp:144`, `display_base.cpp:150`.
- **B-11** WGC setzt `MinUpdateInterval` auf das Bildintervall der gewünschten Bildrate, wo die Eigenschaft existiert (R14); wo nicht, meldet die Sitzung die Grenze `capture_rate_capped` in `se screen sessions --json` und im Statistikfeld.
- **B-12** Für ganze Monitore ist Desktop Duplication der erste Weg (getrennter Zeiger, Dirty-/Move-Rechtecke, kein gelber Rahmen), WGC der Rückfall (Adapterwechsel, `DXGI_ERROR_UNSUPPORTED`, Fenster). Beleg R01 (Vorgabe `ddx`, `docs/configuration.md:2178-2238`), R05, R15 A1/A2.
- **B-13** Unter Desktop Duplication liefert der Gastgeber Zeigerform und -position aus `DXGI_OUTDUPL_FRAME_INFO.PointerPosition` und `GetFramePointerShape` an die Überlagerung (P6-Rahmen); ohne Überlagerung beim Betrachter mischt er den Zeiger ins Bild, damit „Ansehen“ den Zeiger zeigt. Beleg 1.11, R15 A2.

**Linux X11 — gewählt:** XShm bleibt (keine Kompositor-Bildquelle unter X11 ohne Root; KMS siehe verworfen). **B-14** Der Vergleich „unverändert“ läuft auf der Kopie, die ohnehin für den Kodierer entsteht (eine CPU-Kopie weniger, wenn der Hash heute separat läuft [U]), Messung MB3 entscheidet.

**Linux Wayland — gewählt:**
- **B-15** PipeWire bietet DMA-BUF mit den Modifiern des gewählten GPU-Kodierers an und SHM als Rückfall (R15 A3, R01 `pipewire.cpp:538`); DMA-BUF nur, wenn ein GPU-Kodierer den Puffer importiert (sonst würde ein Rücklesen folgen).
- **B-16** Zeiger als Metadaten (`CursorMode::Metadata`, `SPA_META_Cursor`), wenn das Portal ihn ankündigt, sonst eingebettet wie heute. Beleg 1.11, R15 A3.

**Android — gewählt:** unverändert (Hardwaregrenze, Abschnitt 3).

**Verworfen:**
- KMS/DRM-Aufnahme (R01): verlangt `CAP_SYS_ADMIN`; widerspricht „keine Adminrechte als Voraussetzung“; X11 und Wayland liefern das Bild ohne Rechte.
- NvFBC (R01): NVIDIA-spezifisch, nur X11, Freischaltung je Treiber [A]; X11 hat mit XShm einen funktionierenden Weg.
- WGC-Rahmen abschalten (`IsBorderRequired`): verlangt Nutzerzustimmung (R15 A1); B-12 entfernt den Rahmen für Monitore ohne Zustimmung.

### 4.3 Farbumwandlung und Skalierung

- **B-17** Windows mit Hardwarekodierer: Farbumwandlung BGRA → NV12 und Skalierung auf der GPU (D3D11-Videoprozessor oder Pixel-Shader), die Textur geht ohne CPU-Kopie an den Kodierer. Beleg R01 `display_vram.cpp:126-135`, R05.
- **B-18** Linux mit VA-API: Umwandlung im VA-Videoprozessor aus dem DMA-BUF (Wayland) oder aus einem hochgeladenen XShm-Bild (X11).
- **B-19** Software-Kodierer (OpenH264, Software-MFT): die CPU-Umwandlung nutzt alle verfügbaren Kerne zeilenweise und SIMD des Compilers (keine `unsafe`-Intrinsics [P]); Ergebnis bitgleich zur heutigen Referenz (`pixel_convert.rs` bleibt Referenz der Tests).
- Verworfen: CPU-Umwandlung für Hardwarekodierer beibehalten (G11).

### 4.4 Kodierung

**Windows — gewählt:**
- **B-20** Media-Foundation-Hardware-MFT bekommt den D3D11-Gerätemanager der Aufnahme (`MFT_MESSAGE_SET_D3D_MANAGER`) und Texturen als Eingabe. Beleg R15 B1 (Hardware-MFT asynchron, D3D-fähig), R10 (registrierte Ressourcen).
- **B-21** Einstellungen jedes Hardwarekodierers: CBR mit Puffergröße ein Bild (Bitrate/Bildrate), keine B-Bilder, niedrige Latenz, unendliche GOP mit Schlüsselbild nur auf Anforderung, LTR-Markierung für spätere Referenzinvalidierung, wo `CODECAPI_AVEncVideoLTRBufferControl` angenommen wird. Beleg R01 `nvenc_base.cpp:260-283`, R10, R15 B1. Werte, die ein MFT ablehnt, bleiben auf seiner Vorgabe und werden in `se screen codec --json` gemeldet.
- **B-22** Bedingt (Paket BK12): NVENC und AMF direkt über die Treiberbibliotheken (`nvEncodeAPI64.dll`, `amfrt64.dll`, zur Laufzeit geladen), wenn MB3 zeigt, dass das MF-Hardware-MFT bei 1440p p95 > 50 % des Bildintervalls der Bildwiederholrate braucht oder LTR ablehnt [P]. Beleg R01, R10, R11.
- **B-23** HEVC und AV1 über MF, wo ein Hardware-MFT sie anbietet (Abschnitt 4.6).

**Linux — gewählt:**
- **B-24** VA-API-Kodierer (Intel, AMD) über zur Laufzeit geladene `libva.so.2`/`libva-drm.so.2` (keine Linkabhängigkeit, wie `mf_common.rs` mit `LoadLibraryExW`), Einstiegspunkt Low-Power wo vorhanden, sonst normal; NVENC über `libnvidia-encode.so.1` für NVIDIA. Beide brauchen keine Nutzerzustimmung und keine Adminrechte [A: Zugriff auf `/dev/dri/renderD*` hat der Sitzungsnutzer; Root-Worker startet den Hostprozess als Sitzungskonto, `docs/oberflaechen/bildschirm.md` „Root-Worker“]. Beleg R01 `vaapi.cpp:215`, `:304-306`; R15 B1, B4.
- **B-25** Ohne Hardwarekodierer und ohne OpenH264-Zustimmung kodiert Linux AV1 in Software (SVT-AV1 im Echtzeitmodus, BSD-3-Clause-Clear + AOM-Patentlizenz, R15 B2), wenn MB3 auf dem CI-Linux-Runner p95 der Kodierzeit für 1080p unter dem Bildintervall von 60 Bildern/s zeigt [P]; sonst bleiben die Kacheln der Rückfall. Grund: AV1 braucht keine Cisco-Zustimmung (G8), Dekodierung mit dav1d (BSD-2, R15 B2).
- Verworfen: Vulkan-Video-Kodierung jetzt (R12: Treiberunterstützung uneinheitlich, RADV hinter Umgebungsvariable); Neubewertung, wenn Mesa sie standardmäßig aktiviert [U]. FFmpeg (R15 B2: LGPL mit Lieferpflichten, Patentfrage) — VA-API und NVENC direkt sind kleiner.

**Android — gewählt:**
- **B-26** `KEY_FRAME_RATE` und `KEY_OPERATING_RATE` = gewünschte Bildrate des Betrachters (bis zur Bildwiederholrate); `KEY_INTRA_REFRESH_PERIOD`, wenn der Kodierer es kann, statt periodischer Schlüsselbilder; `KEY_I_FRAME_INTERVAL` so groß wie die Plattform erlaubt [U: Höchstwert]; HEVC und AV1 nach 4.6. Beleg R15 B1, G15.

### 4.5 Kette, Prozessgrenze, Eingabe am Gastgeber

- **B-27** Der Hostprozess treibt Aufnahme und Kodierung selbst im Kompositortakt (B-10) und sendet kodierte Bilder unaufgefordert; der Daemon gibt Kredit (Anzahl erlaubter Bilder in Flug, aus `Pacer` und `SenderBudget`), statt je Bild anzufragen. Aufnahme von Bild n+1 überlappt mit Versand von Bild n. Beleg G10; R01 (eigener Aufnahmefaden).
- **B-28** Eingaben laufen im Hostprozess auf einem eigenen Faden und warten nie hinter einer Bildoperation. Beleg 1.9; R01 `stream.cpp:1306` (eigener Faden, Priorität `critical`).
- **B-29** Unter Windows laufen Aufnahme- und Kodierfaden mit erhöhter Fadenpriorität (`THREAD_PRIORITY_HIGHEST`, ohne Administratorrecht möglich [A]), Kodierer mit Hochleistungszustand wo der Hersteller ihn anbietet (R01 `nvenc_latency_over_power`). Verworfen: `HIGH_PRIORITY_CLASS` für den ganzen Prozess und GPU-Planungspriorität `D3DKMT…` (R01): Grund: wirkt auf das ganze System des Nutzers; ohne Messung kein Beleg des Nutzens für se [P].
- Die Prozessgrenze bleibt (Absturzisolation, `host_worker_client.rs:1-6`); über sie gehen nur kodierte Bytes.

### 4.6 Codecs und Aushandlung

- **B-30** `CodecKind` erhält `Hevc` und `Av1`; Rangfolge der Paarung: Hardware/Hardware vor Hardware/Software; innerhalb einer Stufe AV1 vor HEVC vor H.264, wenn beide Seiten Hardware haben, sonst H.264 vor Software-AV1 [P]; Kacheln zuletzt. Beleg R09 (HEVC 25–50 %), R01 (H.264/HEVC/AV1).
- **B-31** Rückwärtsverträglichkeit: Der Gastgeber kündigt im `ScreenServerHello` ein neues optionales Feld `codecs_v2: Some(1)` an; nur dann führt der Betrachter HEVC/AV1-Dekodierer in einem neuen optionalen Feld `decoders_v2` seines Hallos, und nur dann nennt der Gastgeber HEVC/AV1 in einem neuen Feld `encoders_v2`. Die bestehenden Felder `decoders` und `encoders` enthalten weiter nur H.264 und Kacheln, weil `CodecKind` beim Deserialisieren eines unbekannten Wertes scheitert (kein `#[serde(other)]` in `negotiate.rs` [Q]). Gegenstellen ab v0.5.196 sehen damit unverändert H.264 und Kacheln.
- **B-32** 4:4:4: angeboten, wenn Kodierer und Dekodierer es melden (NVENC 4:4:4, HEVC RExt, AV1 High); verwendet bei „Schärfe vor Flüssigkeit“ und automatisch bei der Inhaltsart „Text“ [P]. Verworfen: AVC444-Doppelstrom (R09): Grund: zweiter Kodiervorgang je Bild und eigene Zusammensetzung beim Betrachter; Neubewertung, falls MB3 zeigt, dass kein Gerät des Nutzers 4:4:4-Hardware hat [P].
- HDR: in diesem Zuschnitt nicht gebaut. Grund: Aufnahme, Umwandlung und Anzeige sind durchgehend 8 Bit (1.3–1.7); ob eframe 0.29/wgpu eine HDR-Fläche anbietet, ist ungeklärt (Abschnitt 6 Punkt 18); ohne HDR-Anzeige beim Betrachter bringt eine HDR-Kette kein sichtbares Ergebnis. Neubewertung nach Klärung von Punkt 18 [P].
- **B-33** Hybridbild bleibt aus (`viewer_client.rs:229-235`, Befund 2026-10-27). Grund: gemessener Fehlschlag; Schärfe kommt über 4:4:4 und Bitrate.
- **B-34** Bedingt (Paket BK11): LAN-Hochratencodec nach dem Verfahren von PyroWave (Intra-only, GPU-Compute, MIT-Lizenz erlaubt Abhängigkeit), nur auf dem direkten LAN-Weg mit gemessener Netzrate über der Mindestrate des Codecs, wenn MB3/MB4 zeigen, dass Hardware-H.264/HEVC Kodierung + Dekodierung p95 über 25 % des Bildintervalls liegen [P]. Beleg R06.

### 4.7 Transport und Verlust

- Bestand bleibt: QUIC-Strom je Bildgruppe, `plan.md` E2/E3/E4 (Rückzug, Präsenz, Steuerpriorität), verschlüsselt (C4).
- **B-35** Die Rücksetzschwelle ist kein fester 1-s-Wert, sondern max(4 × geglättete RTT, 3 Bildintervalle) [P: Verfahren „Vielfaches der RTT“ aus RFC 9002 PTO; Faktoren durch `plan.md` M7 bestätigt].
- **B-36** Nach einem Rücksetzen fordert der Gastgeber vom Kodierer eine Referenz auf das letzte bestätigte Bild (LTR/RFI), wo der Kodierer es kann (R01 `nvenc_base.cpp:795-830`, R10, R11); sonst Schlüsselbild wie heute. Setzt voraus, dass der Betrachter seinen Dekodierer über Bildgruppen hinweg behält, wenn die neue Gruppe als „Fortsetzung“ markiert ist (Fähigkeit `gop_continue_v1` im Hallo). Bedingung aus `plan.md` B4 (M7) bleibt.
- FEC über Datagramme: bleibt `plan.md` E11/B1.
- Verworfen: TCP als Nutzerwahl wie Shadow (R07): Grund: QUIC-Ströme liefern Zuverlässigkeit bereits ohne Kopfblockade über Ströme hinweg; ein TCP-Weg besteht als Relay-Rückfall automatisch (`plan.md` B2).

### 4.8 Dekodierung und Anzeige

- **B-37** Windows-Betrachter: MF-Dekodierer mit D3D11-Gerätemanager (DXVA); Ausgabe NV12; zuerst Rücklesen in den bestehenden Plane-Pfad (`G/screen_wgpu_yuv.rs`), danach bedingt geteilte Textur in wgpu (Paket BK9, wenn MB4 zeigt, dass das Rücklesen > 1 ms p95 bei 1440p kostet [P]).
- **B-38** Linux-Betrachter: VA-API-Dekodierer (H.264, HEVC, AV1), dav1d für Software-AV1, OpenH264 nach Zustimmung; Ausgabe NV12 in den bestehenden Plane-Pfad.
- **B-39** Android-Betrachter: unverändert plus `Surface.setFrameRate` mit der Bildrate des Stroms (API 30+) [A: API-Level aus Android-Doku, in dieser Sitzung nicht abgerufen], HEVC/AV1-Dekodierer.
- **B-40** Desktop-Anzeige: „Niedrigste Latenz“ wirkt sofort (Fläche neu konfigurieren), wenn eframe 0.29 das erlaubt [U]; im Vollbild mit „Niedrigste Latenz“ Tearing erlaubt (VRR-Bildschirme synchronisieren dann selbst). Standard bleibt V-Sync mit einem Bild (bestehende Abwägung `screen_viewer_app.rs:1-8`). Beleg R03, R05.
- Verworfen: Moonlight-Pacer (R03): Grund: hält bis 3 Bilder zurück; se zeigt das jüngste Bild (Latenz vor Gleichmäßigkeit, Vorgabe 1).

### 4.9 Zeiger (Vorgabe 5)

- Nach P6 verschwinden die zwei Zeiger für WGC- und X11-Gastgeber am Desktop-Betrachter im Modus „Steuern“ (1.11).
- **B-13** schließt DXGI (Zeiger auch beim Ansehen), **B-16** schließt Wayland (Metadaten-Zeiger, wenn das Portal ihn anbietet; sonst bleibt der eingebettete Zeiger und die Sitzung meldet `cursor_embedded` [P]).
- Android-Gastgeber: kein Zeiger im Bild [A]; keine Änderung.
- Android-Betrachter mit Maus: [U] Prüfung im Paket BK7a, ob Android einen eigenen Zeiger zeichnet; falls ja, blendet die Ansicht ihn über dem Bild aus (`View.setPointerIcon(TYPE_NULL)` [A]).

### 4.10 Wahlmöglichkeiten für den Nutzer (Vorgabe 4, Teil c)

| Wahl (Referenz) | Entscheidung für se | Grund |
|---|---|---|
| Transport UDP/TCP (Shadow, RustDesk, RDP) | entfällt als Einstellung; automatisch | QUIC direkt, Relay über TCP als automatischer Rückfall; Vorgabe 2026-11-08 |
| Dekodierer (Moonlight, Steam, RustDesk) | Einstellung „Dekodierer: Automatisch / Hardware / Software“ auf Desktop (Betrachterleiste), Android (Betrachtereinstellungen), CLI `se screen view --decoder`, IPC `StreamRequest`; MCP über `se`; Vorgabe automatisch mit Rückfall bei Fehler | Treiberfehler einzelner Hardwaredekodierer (R06 Gemeinschaftsberichte); Diagnose ohne Neuinstallation |
| Codec (Moonlight, RustDesk, Shadow) | Einstellung „Codec: Automatisch / H.264 / HEVC / AV1 / Verlustfrei“ (ersetzt den Schalter „Verlustfrei“ ohne dessen Bedeutung zu ändern) auf Desktop, Android, CLI `--codec`, IPC; MCP über `se` | Vergleich und Umgehung von Codecfehlern |
| Bitrate (alle) | Einstellung „Höchste Datenrate“ (B-09), Vorgabe automatisch | getaktete Leitungen |
| Bildrate (Moonlight, Shadow, RustDesk) | entfällt als Einstellung; automatisch = Bildwiederholrate (B-01); „Schärfe vor Flüssigkeit“ bleibt | Datenmenge regelt „Höchste Datenrate“ |
| 4:4:4 (alle) | automatisch bei „Schärfe vor Flüssigkeit“ und Text (B-32) | keine Handgriffe |
| V-Sync (Moonlight) | besteht als „Niedrigste Latenz“; wirkt sofort (B-40) | — |
| Frame Pacing (Moonlight) | entfällt | 4.8 |
| FEC-Anteil (Sunshine) | entfällt als Einstellung; automatisch, falls `plan.md` B1 gebaut wird | — |

### 4.11 Garantien und Verträge

- Verschlüsselung, Rechteprüfung, Routenregeln unverändert (`plan.md` C8).
- Ältere Gegenstellen ab v0.5.196: jede neue Fähigkeit nur nach Ankündigung im Hallo (`codecs_v2`, `gop_continue_v1`); ohne sie verhalten sich beide Seiten wie heute (Kodierer-Neustart bei Bildratenwechsel bleibt im alten Pfad nur, wenn der Gastgeber alt ist).
- Hostprozess und Daemon stammen aus derselben Installation (`host_worker_client.rs:5-6`); das Worker-Protokoll braucht keine Altversion.

---

## 5 Arbeitsplan

### 5.1 Einordnung und Reihenfolge

**Stand 2026-11-14 (Entscheidung des Managers, nach diesem Entwurf): Die Pakete dieses Abschnitts bilden den nächsten Batch mit eigenem Release (Release 2) und beginnen erst nach dem Release von „Verbindungen je Zweck“ (`plan.md` D5); BK0 bis BK12 sind nicht begonnen (`docs/TODO.md` Zeile `VERB-BILDKETTE`). Der folgende Absatz und die Wellentabelle beschreiben die ursprüngliche Einordnung und gelten für die Reihenfolge innerhalb des Batches weiter.**

Die Pakete BK0 bis BK10 gehören zum laufenden Auftrag (Nutzervorgaben 1 bis 5 vom 2026-11-14) und laufen nach `plan.md` P7 (dessen Messinstrument sie nutzen) und vor den Reviews R; die Suite P8 deckt sie mit ab [P: Einordnung legt der Manager fest]. BK11 und BK12 sind bedingt (Messbedingung in 4.4, 4.6).

| Welle | Pakete (parallel) | Abhängigkeit |
|---|---|---|
| A | BK0 | `plan.md` P7 fertig |
| B | BK1, BK2b, BK5w, BK7a, BK8a | BK0 |
| C | BK2a, BK3, BK4, BK6 | BK1 (BK3), BK2b (BK2a) |
| D | BK7b, BK8b, BK9, BK10 | BK6 (BK7b), BK4 (BK8b), BK5w (BK9) |
| E (bedingt) | BK11, BK12 | Messungen der Suite |

Prüfung „parallele Pakete teilen keine änderbaren Dateien“: Tabelle 5.4.

### 5.2 Pakete

#### BK0 Messinstrument mit echter Quelle und allen Stationen

- Ergebnis: `se screen bench GERÄT --source QUELLE` misst mit der echten Aufnahme des Gastgebers (heute nur Testbild); `stages_ms` enthält `capture`, `convert`, `encode`, `worker_handoff`, `send`, `network`, `decode` je p50/p95 und zusätzlich `tick_jitter` (Abweichung des Aufnahmetakts vom Soll) und `input` (Eingabe am Betrachter bis Auslösung am Gastgeber, mit Uhrversatz); die Textausgabe nennt sie. Der Gastgeber liefert die Stationszeiten nur, wenn der Betrachter sie anfordert (Fähigkeit `frame_timing_v1` im Server-Hallo, neuer Steuerrahmen).
- Lesbar: `E/core/{bench.rs,bench_stages.rs,metrics.rs,host_frame_send.rs,host_session_control.rs,wire_control.rs,wire_control_codec.rs,wire_hello.rs,worker_frame_wire.rs}`, `D/{ipc_protocol_screen.rs,screen_ipc.rs}`, `C/screen/{bench.rs,output.rs}`, `E/os/shared/{host_worker_produce.rs,host_worker_run.rs}`.
- Änderbar: `E/core/bench_stages.rs` (164), neu `E/core/bench_source.rs`, `E/core/host_frame_send.rs` (116), neu `E/core/frame_timing.rs` (Rahmen und Ankündigung), `E/core/wire_control_codec.rs` (229), `E/core/worker_frame_wire.rs` (200), `E/os/shared/host_worker_produce.rs` (186), `D/ipc_protocol_screen.rs` (498; zuerst `BenchRequest` in neu `D/ipc_protocol_screen_bench.rs` auslagern), `C/screen/bench.rs` (112), `C/screen/output.rs` (229), `E/core/mod.rs` (146; zwei `mod`-Zeilen).
- Nicht anfassen: `E/core/bench.rs` (488, P7), `wire_hello.rs` (BK6), alle Kodierer- und Aufnahmedateien.
- Verträge: bestehende JSON-Felder von `se screen bench` unverändert (P5a-Vertrag); `--source` optional; ohne `frame_timing_v1` beim Gastgeber fehlen die Gastgeberstationen in `stages_ms` und die Ausgabe sagt es.
- Abnahmesignal: Tests `bildkette_task_bench_*`: Stationen vollständig mit synthetischer Quelle; `--source` lehnt eine nicht vorhandene Quelle mit Code ab; ältere Gegenstelle ohne Fähigkeit liefert die alten Felder.
- Oberflächen: CLI (Text, JSON), IPC; MCP über `se`; Doku `docs/oberflaechen/bildschirm.md` Abschnitt 3.
- Messung: ist das Instrument; „vorher“-Werte aller MB-Messungen entstehen mit ihm vor BK1.

#### BK1 Bildtakt, Bildrate und Bitrate (B-01 bis B-09)

- Ergebnis: B-01 bis B-09.
- Lesbar: `E/core/{rate_control.rs,host_mode.rs,host_pipeline.rs,host_pipeline_produce.rs,host_quality.rs,pacing.rs,host_session.rs,frame_size_avc.rs,negotiate.rs}`, `G/{screen_pool.rs,screen_viewer_app.rs,screen_session.rs}`, `M/screen_view.rs`, `E/os/mod.rs`.
- Änderbar: `E/core/rate_control.rs` (432; zuerst Leiter und `quality_for` nach neu `E/core/rate_cadence.rs`), neu `E/core/rate_probe.rs` (Hochlauf, Merkwert je Gegenstelle), `E/core/host_mode.rs` (163), `E/core/host_pipeline_produce.rs` (216), `E/core/host_quality.rs` (136), `E/core/pacing.rs` (273; B-35), `E/core/host_session.rs` (499; nur Austausch des Takts gegen neu `E/core/host_session_clock.rs`, Zeilenzahl sinkt), neu `E/os/windows/timer_resolution.rs`, `E/os/mod.rs` (413; eine `mod`-Zeile), `G/screen_pool.rs` (199), neu `G/screen_refresh.rs` (Bildwiederholrate des Monitors über die OS-Schnittstelle: Windows `EnumDisplaySettingsW`, Linux XRandR bzw. `wl_output`-Modus [U: Weg über eframe 0.29]), neue Einstellung „Höchste Datenrate“ in `G/screen_prefs.rs` (85) und `G/screen_toolbar_ui.rs` (464), `M/screen_view.rs` (489; nur Feld `maxRate`), `C/screen/view.rs` (162; `--max-rate`), `D/ipc_protocol_screen.rs` (nach BK0; Feld `max_rate_bps` in `StreamRequest`). Die Gastgeberseite der Grenze ist `RateControl::set_ceiling`; den Steuerrahmen `RATE_CEILING` mit Fähigkeit `rate_ceiling_v1` liefert BK6 (Besitzer von `wire_hello.rs`), die Android-Einstellung BK7a.
- Nicht anfassen: `wire_hello.rs`, `A/ui/screen/ScreenViewScreen.kt` (BK7a), Kodierer-, Aufnahme- und Worker-Dateien.
- Verträge: `max_fps` im Client-Hallo bleibt `u16`; ein alter Gastgeber bekommt die höhere Bildrate und regelt nach seinem alten Verfahren; ein alter Betrachter sendet 60 und bekommt 60. Statistikfelder unverändert, neue nur ergänzend.
- Abnahmesignal: `bildkette_task_rate_*`: Bildrate unabhängig von der Bitrate; kein Kodiererneustart bei Bildratenwechsel; Hochlauf verdoppelt je RTT bis zur ersten Überlast; Obergrenze aus Kodierer-/Dekodiererfähigkeit; Windows-Takt mit hoher Auflösung (Text-/Plattformtest).
- Oberflächen: Desktop (Leiste, Einstellung), Android (Einstellung), CLI, IPC; MCP über `se`; Doku `bildschirm.md` Abschnitte 4 und 8.
- Messung: MB1, MB2 vorher/nachher.

#### BK2b Windows-Aufnahme (B-10 bis B-13)

- Ergebnis: Kompositor-ausgelöste Aufnahme in eigenem Faden, `MinUpdateInterval`, Desktop Duplication zuerst für Monitore, DXGI-Zeiger für Überlagerung und Mischung.
- Lesbar: `E/os/windows/{capture_wgc.rs,capture_dxgi.rs,capture_retirement.rs,capture_source.rs,capture_readback.rs,cursor_overlay.rs,host_backend.rs,monitors.rs}`, `E/core/cursor_overlay.rs`, R14, R15 A1/A2.
- Änderbar: `E/os/windows/capture_wgc.rs` (391), `E/os/windows/capture_dxgi.rs` (499; zuerst Zeiger und Wartelogik nach neu `E/os/windows/dxgi_pointer.rs` und `E/os/windows/dxgi_wait.rs`, eingebunden per `#[path]` in `capture_dxgi.rs`), `E/os/windows/capture_retirement.rs` (138), `E/os/windows/capture_source.rs` (215), `E/os/windows/cursor_overlay.rs` (348), `E/os/windows/host_backend.rs` (245).
- Nicht anfassen: `capture_readback.rs`, `d3d_device.rs`, `mf_*.rs` (BK2a, BK5w).
- Verträge: Quellen-IDs `monitor:`/`window:` unverändert; Grenzen in `limits` ändern sich nur ergänzend (`capture_border` entfällt für Monitore unter DXGI, neu `capture_rate_capped`).
- Abnahmesignal: `bildkette_task_win_capture_*` (Windows-Runner: Texttests und Fallbacklogik, weil der Runner keine Aufnahme hat [A]).
- Oberflächen: CLI/JSON (`limits`), Doku `bildschirm.md` Abschnitte 6, 10, 11.
- Messung: MB1 (`tick_jitter`, Aufnahme-p95) nur am Gerät.

#### BK2a Windows-GPU-Kodierpfad (B-17, B-20, B-21, B-23 Teil MF)

- Ergebnis: Textur → GPU-Umwandlung/Skalierung → Hardware-MFT mit D3D-Manager; Einstellungen B-21; CPU-Pfad nur für Software-MFT, OpenH264, Kacheln.
- Lesbar: `E/os/windows/{d3d_device.rs,capture_readback.rs,mf_common.rs,mf_media.rs,mf_encoder.rs,mf_encoder_sync.rs,mf_capture_media.rs,mf_capture_output.rs,mf_buffer_lease.rs}`, `E/core/{host_capture.rs,capture_grant.rs,frame_memory.rs}`, R15 B1.
- Änderbar: `E/os/windows/d3d_device.rs` (144), `E/os/windows/capture_readback.rs` (251), `E/os/windows/mf_media.rs` (266), `E/os/windows/mf_encoder.rs` (296), `E/os/windows/mf_capture_media.rs` (85), neu `E/os/windows/mf_d3d_input.rs`, neu `E/os/windows/d3d_video_convert.rs` (beide per `#[path]` in `mf_encoder.rs`).
- Nicht anfassen: `mf_common.rs` (BK6), `mf_decoder.rs` (BK5w), Aufnahmedateien (BK2b).
- Verträge: Speicherzulassung (`capture_grant`) gilt auch für GPU-Texturen [U: Zählweise GPU-Speicher]; Kodiererausgabe unverändert Annex B.
- Abnahmesignal: `bildkette_task_win_gpu_*` (Texttests; Funktion nur am Gerät).
- Oberflächen: `se screen codec --json` (Einstellungen, die ein MFT abgelehnt hat), Statistik „Kodierer: Hardware“; Doku `bildschirm.md` Abschnitt 5.
- Messung: MB3 vorher/nachher am Windows-Gerät.

#### BK3 Hostprozess-Takt und Eingabefaden (B-27 bis B-29)

- Ergebnis: Hostprozess sendet Bilder unaufgefordert nach Kredit; Eingabe auf eigenem Faden; Fadenpriorität.
- Lesbar: `E/os/shared/{host_worker_main.rs,host_worker_run.rs,host_worker_client.rs,host_worker_reader.rs,host_worker_source.rs,capture_dispatch.rs}`, `E/core/{worker_wire.rs,worker_request_wire.rs,worker_frame_wire.rs,host_stage.rs,host_pipeline_produce.rs,sender_budget.rs}`.
- Änderbar: `E/os/shared/host_worker_run.rs` (259), `E/os/shared/host_worker_main.rs` (340), `E/os/shared/host_worker_client.rs` (437), neu `E/os/shared/host_worker_input.rs`, neu `E/os/shared/host_worker_clock.rs`, `E/core/worker_wire.rs` (291), `E/core/worker_request_wire.rs` (157), `E/core/host_stage.rs` (78), `E/core/host_pipeline_produce.rs` (nach BK1).
- Nicht anfassen: Plattformaufnahme (BK2b, BK8a), `worker_frame_wire.rs` (BK0 fertig; nur lesen).
- Verträge: Absturz des Hostprozesses beendet nur die Sitzung; Update-Stopp (Exit 75) unverändert; Android-Pfad unverändert.
- Abnahmesignal: `bildkette_task_worker_*`: Eingabe während laufender Kodierung wird sofort ausgelöst (Test mit verzögerndem Testkodierer); Bilder überlappen; Kredit begrenzt Bilder in Flug.
- Oberflächen: keine Bedienung; Doku `ARCHITEKTUR.md`.
- Messung: MB6, MB1.

#### BK4 Linux-Hardwarekodierung und -dekodierung (B-18, B-24, B-25, B-38)

- Ergebnis: VA-API-Kodierer und -Dekodierer, NVENC-Kodierer, Software-AV1 bedingt (B-25), dav1d-Dekodierer; Registrierung in der Codec-Registry; kein H.264-Bedarf an OpenH264 mehr auf Geräten mit Hardware.
- Lesbar: `E/os/{codec_facade.rs,mod.rs}`, `E/os/shared/codec_inventory.rs`, `E/os/native_ffi/openh264_*.rs` (Muster), `E/core/{codec_registry.rs,codec_registry_open.rs,host_backend_codecs.rs}`, `native/Cargo.toml`, R01 `vaapi.cpp`, R15 B1/B2/B4.
- Änderbar: neu `E/os/linux_os/vaapi_ffi.rs`, `vaapi_encode.rs`, `vaapi_decode.rs`, `vaapi_surface.rs`, `nvenc_ffi.rs`, `nvenc_encode.rs`, neu `E/os/native_ffi/av1_soft.rs` (SVT-AV1, dav1d), `E/os/codec_facade.rs` (121), `E/os/shared/codec_inventory.rs` (139), `E/os/mod.rs` (Modulzeilen, nach BK1), `native/Cargo.toml` (nur bei B-25: `dav1d`/SVT-AV1-Bindung nach Lizenzprüfung).
- Nicht anfassen: `negotiate.rs`, `wire_hello.rs` (BK6), PipeWire-Dateien (BK8).
- Verträge: Bibliotheken zur Laufzeit geladen; fehlt eine, bleibt der heutige Weg; keine Zustimmung, kein Adminrecht.
- Abnahmesignal: `bildkette_task_linux_hw_*`: Laden ohne Bibliothek gibt leere Liste (CI); Software-AV1 kodiert/dekodiert Testbild mit PSNR-Schwelle (CI).
- Oberflächen: `se screen codec --json`, Desktop „Kodierung“, Doku `bildschirm.md` Abschnitt 5 (korrigiert dort die offene Aussage zu Linux).
- Messung: MB3, MB4 (CI für Software-AV1, Gerät für VA-API/NVENC).

#### BK5w Windows-Betrachter Hardware-Dekodierung (B-37 Teil 1)

- Änderbar: `E/os/windows/mf_decoder.rs` (350), neu `E/os/windows/mf_decoder_d3d.rs` (per `#[path]`).
- Lesbar: `G/{screen_decode_thread.rs,screen_wgpu_yuv.rs}`, `E/core/decoded_frame.rs`.
- Verträge: Ausgabe weiter NV12-Ebenen; Rückfall auf Software bei Fehler.
- Abnahmesignal: `bildkette_task_win_decode_*` (Text-/Rückfalltest).
- Oberflächen: Statistik „Dekodierer: Hardware“; Einstellung „Dekodierer“ kommt mit BK6.
- Messung: MB4.

#### BK6 Codecs HEVC/AV1, Aushandlung v2, Codec- und Dekodiererwahl (B-23, B-30 bis B-32, 4.10)

- Änderbar: `E/core/negotiate.rs` (387), `E/core/wire_hello.rs` (469; Felder `codecs_v2`, `encoders_v2`, `decoders_v2`; Prüfung bleibt < 500 Zeilen, sonst Auslagerung nach neu `E/core/wire_hello_codecs.rs`), `E/core/codec_registry.rs` (224), `E/core/codec_registry_open.rs` (311), neu `E/core/frame_size_hevc_av1.rs`, `E/core/wire_binary.rs` (Codecnummern), `E/os/windows/mf_common.rs` (485; zuerst Aufzählung nach neu `E/os/windows/mf_enum.rs`), `G/screen_toolbar_ui.rs` (nach BK1), `G/screen_prefs.rs` (nach BK1), `C/screen/view.rs` (nach BK1), `D/ipc_protocol_screen.rs` (nach BK0).
- Zusätzlich: Steuerrahmen `RATE_CEILING` (Betrachter → Gastgeber) mit Fähigkeit `rate_ceiling_v1` im Server-Hallo, Anschluss an `RateControl::set_ceiling` aus BK1; dafür `E/core/wire_control.rs` (366) und `E/core/wire_control_codec.rs` (nach BK0).
- Verträge: B-31; `RATE_CEILING` nur an Gastgeber mit Ankündigung (ein unbekannter Betrachterrahmen beendet bei alten Gastgebern die Sitzung, P6-Bericht „Entscheidungen“).
- Abnahmesignal: `bildkette_task_codec_*`: alter Betrachter (Hallo ohne `decoders_v2`) gegen neuen Gastgeber → H.264; neuer gegen alten → H.264; Rangfolge; unbekannter Codec im alten Feld wird nie gesendet.
- Oberflächen: Desktop, Android (über BK7b), CLI `--codec`, `--decoder`, IPC; MCP über `se`; Doku.
- Messung: MB3 (PSNR je Codec bei gleicher Rate).

#### BK7a Android ohne neue Codecs (B-26 ohne HEVC/AV1, B-39 ohne neue Codecs, Zeiger bei Maus)

- Änderbar: `A/screen/ScreenEncoder.kt` (421), `A/screen/ScreenCodecLatency.kt`, `A/ui/screen/ScreenDecoder.kt` (519; Kotlin, Regel 500 gilt für Rust), `A/ui/screen/ScreenViewScreen.kt` (575; `setFrameRate`, Zeigerausblendung, Einstellung „Höchste Datenrate“ über das Bridge-Feld `maxRate` aus BK1), `A/api/ScreenApi.kt` (Feld `maxRate`).
- Abnahmesignal: JVM-Tests `bildkette*` im Teil `android` der Suite.
- Messung: MB3, MB4 am Android-Gerät.

#### BK7b Android HEVC/AV1 (nach BK6)

- Änderbar: `A/screen/ScreenEncoder.kt`, `A/ui/screen/ScreenDecoder.kt`, `E/os/android/bridge_backend.rs` (324), `M/screen_view.rs`.

#### BK8a Wayland-Zeiger (B-16)

- Änderbar: `E/os/linux_os/portal_source_binding.rs` (224), `E/os/linux_os/pipewire_stream.rs` (397), neu `E/os/linux_os/pipewire_cursor.rs` (per `#[path]`), `E/os/linux_os/spa_pod.rs` (391; Metadaten-Parameter), `E/os/linux_os/portal_session.rs` (477; nur `set_supported`, Zeilenzahl ≤ 499).
- Abnahmesignal: `bildkette_task_wayland_cursor_*`: SPA-Metadaten-Parser mit Testdaten; Modus nur bei Ankündigung.
- Messung: Wahrnehmung nur am Gerät.

#### BK8b PipeWire DMA-BUF (B-15, nach BK4)

- Änderbar: `E/os/linux_os/spa_pod.rs`, `pipewire_stream.rs`, `pipewire_buffers.rs` (407), `pipewire_import.rs` (333), `E/os/linux_os/vaapi_surface.rs` (aus BK4).
- Abnahmesignal: Pod mit Modifiern, SHM-Rückfall (CI).

#### BK9 Anzeige (B-37 Teil 2 bedingt, B-40)

- Änderbar: `G/screen_viewer_app.rs` (488; zuerst `wgpu_configuration` nach neu `G/screen_present.rs`), `G/screen_render_wgpu.rs` (208), `G/screen_wgpu_yuv.rs` (360).
- Abnahmesignal: `bildkette_task_present_*` (Konfiguration je Modus); [U] eframe-Fähigkeit.

#### BK10 Wayland-Eingabe über EIS (bedingt)

- Ergebnis: Eingabe über `ConnectToEIS` statt einzelner D-Bus-Aufrufe, wenn MB6 unter Wayland p95 über dem Wert unter X11 liegt [P].
- Änderbar: `E/os/linux_os/portal_input.rs` (467; Auslagerung zuerst), neu `E/os/linux_os/eis_input.rs`.

#### BK11 LAN-Hochratencodec (bedingt, B-34) und BK12 NVENC/AMF direkt unter Windows (bedingt, B-22)

Eigener Planabschnitt vor Umsetzung, wenn die Bedingung erfüllt ist.

### 5.3 Messungen

| Messung | Frage | CI (GitHub-Runner ohne GPU) | Gerät des Nutzers |
|---|---|---|---|
| MB1 Takt | `tick_jitter` p50/p95, erreichte Bildrate bei 60/120/240 angefordert | Linux und Windows mit Testbild; Windows-Runner zeigt die Timerauflösung | Windows, Linux X11, Wayland, Android mit `--source` |
| MB2 Hochlauf | Zeit bis Zielbildrate und Zielrate, Schlüsselbilder im Hochlauf | Linux, Netem `lan`, `wan` (`native/rdt_task/speed_net.py`) | Weg LAN und Internet |
| MB3 Gastgeberstationen | `capture`, `convert`, `encode` p95 je 1080p/1440p/2160p; PSNR je Codec | OpenH264, Kacheln, Software-AV1, CPU-Umwandlung; X11 über Xvfb | MF-Hardware, VA-API, NVENC, MediaCodec |
| MB4 Dekodierung | `decode` p95 je Dekodierer und Auflösung | OpenH264, dav1d | MF/DXVA, VA-API, MediaCodec |
| MB5 Anzeige | Abstand Dekodierende bis Bildschirm | nicht messbar (kein Bildschirm) | nur mit Kamera; widerspricht „keine Handgriffe“ → nicht gemessen, Grenze im Bericht |
| MB6 Eingabe | `input` p95 bei laufender Kodierung | Testbild-Gastgeber im Prozess | jeder Gastgeber |
| MB7 Verlust | Erholzeit nach Rücksetzen | wie `plan.md` M7 | — |
| MB8 Datenrate | höchste dauerhafte Rate ohne Verzögerungsanstieg | Loopback (Softwaregrenze der Kette) | LAN |

Messweg am Gerät ohne Handgriff des Nutzers: Der Manager startet `se screen bench GERÄT --source QUELLE --seconds 30 --json` (BK0) von einem gekoppelten Gerät aus gegen das Zielgerät über die bestehende Kopplung; die Rechte „Bildschirm ansehen“ muss das Zielgerät bereits erlauben [A: für die Geräte des Nutzers gesetzt]. Android als Betrachter misst die App-Statistik (`ScreenSessionState`); als Gastgeber über `se screen bench` vom Desktop.

### 5.4 Prüfung der Dateien paralleler Pakete

- Welle B: BK1 (`rate_*`, `host_mode`, `host_pipeline_produce`, `host_quality`, `pacing`, `host_session*`, `os/mod.rs`, `os/windows/timer_resolution.rs`, `G/screen_pool`, `G/screen_refresh`, `G/screen_prefs`, `G/screen_toolbar_ui`, `M/screen_view`, `C/screen/view.rs`, `D/ipc_protocol_screen.rs`), BK2b (`os/windows/capture_*`, `dxgi_*`, `cursor_overlay`, `host_backend`), BK5w (`mf_decoder*`), BK7a (`ScreenEncoder.kt`, `ScreenCodecLatency.kt`, `ScreenDecoder.kt`, `ScreenViewScreen.kt`, `ScreenApi.kt`), BK8a (`portal_*`, `pipewire_stream`, `pipewire_cursor`, `spa_pod`). Keine Überschneidung; die Android-Einstellung „Höchste Datenrate“ liegt bei BK7a, BK1 liefert nur das Bridge-Feld in `M/screen_view.rs`.
- Welle C: BK2a (`d3d_device`, `capture_readback`, `mf_media`, `mf_encoder`, `mf_capture_media`, `mf_d3d_input`, `d3d_video_convert`), BK3 (`host_worker_*`, `worker_wire`, `worker_request_wire`, `host_stage`, `host_pipeline_produce`), BK4 (`vaapi_*`, `nvenc_*`, `av1_soft`, `codec_facade`, `codec_inventory`, `os/mod.rs`, `Cargo.toml`), BK6 (`negotiate`, `wire_hello*`, `codec_registry*`, `frame_size_hevc_av1`, `wire_binary`, `mf_common`, `mf_enum`, `wire_control`, `wire_control_codec`, `G/screen_toolbar_ui`, `G/screen_prefs`, `C/screen/view.rs`, `D/ipc_protocol_screen.rs`). Keine Überschneidung.
- Welle D: BK7b (`ScreenEncoder.kt`, `ScreenDecoder.kt`, `bridge_backend.rs`, `M/screen_view.rs`), BK8b (`spa_pod`, `pipewire_*`, `vaapi_surface`), BK9 (`G/screen_viewer_app`, `G/screen_present`, `G/screen_render_wgpu`, `G/screen_wgpu_yuv`), BK10 (`portal_input`, `eis_input`). Keine Überschneidung.

### 5.5 Dokumentation

`docs/oberflaechen/bildschirm.md` (Abschnitte 3, 4, 5, 6, 8, 10, 11), `docs/ARCHITEKTUR.md` (Hostprozess-Takt, GPU-Pfad, Codecs), `docs/GOTCHAS.md` (Laufzeitladen von VA-API/NVENC, Timerauflösung), `docs/TODO.md` (bedingte Pakete) — im Paket P9 von `plan.md` oder je Paket für die eigenen Abschnitte [P: Zuordnung legt der Manager fest].

---

## 6 Ungeklärte Angaben

1. Standard-Timerauflösung der Windows-Geräte des Nutzers und ihre Wirkung auf das Tokio-Intervall (G9) — MB1.
2. Ob der erste aufgezählte MF-Ausgabetyp den höchsten Level des Kodierers nennt (G7) — Lesen weiterer Typen am Gerät.
3. GOP-Vorgabe der verwendeten MF-Kodierer und von OpenH264 (`openh264` 0.9.9 Vorgabe nicht gelesen).
4. Ob der Wayland-Pfad eine dritte Kopie zwischen `LatestFrame` und Kodierer macht (1.5).
5. Ob der X11-Hash eine eigene Kopie oder nur einen Lesedurchlauf kostet (B-14).
6. Weg zur Bildwiederholrate des Monitors in eframe 0.29 (BK1) und ob eframe 0.29 den Präsentationsmodus ohne Neustart umschalten kann (B-40).
7. Zählweise von GPU-Texturen in der Speicherzulassung (`capture_grant`) für BK2a.
8. Zugriff des Sitzungskontos auf `/dev/dri/renderD*` auf den Linux-Geräten des Nutzers (B-24).
9. Rechtslage H.264/HEVC-Patente bei Hardwarekodierern auf Linux ohne Cisco-Binärdatei (B-24): nicht juristisch geprüft.
10. Parsec- und Shadow-Hilfeseiten: Inhalte nur aus Suchauszügen; Dekodiererwahl bei Shadow nicht belegt; Parsec-Optionsliste nicht gelesen.
11. Steam Remote Play: Valve-Hilfeseite ohne Artikeltext; Optionen nur aus Sekundärquellen.
12. Vulkan-Video-Kodierung: standardmäßige Aktivierung in Mesa RADV/ANV/NVK.
13. Android: Höchstwert von `KEY_I_FRAME_INTERVAL` ohne periodische Schlüsselbilder; API-Level von `Surface.setFrameRate`; eigener Mauszeiger bei angeschlossener Maus.
14. Android-Gastgeber → Linux-Betrachter ohne OpenH264: Bildrate des Standbildwegs.
15. MF-H.264-Dekodierer: DXVA-Nutzung nur mit D3D-Manager (Microsoft-Seite nicht abgerufen).
16. Ob ein Wechsel der Monitoraufnahme von WGC zu Desktop Duplication auf Hybridgrafik-Laptops des Nutzers scheitert (B-12, Rückfall vorhanden).
17. Messwerte aller Stationen: keine; die Potentiale in Abschnitt 3 sind bis auf die Rechnung in 1.2 Schätzungen.
18. HDR-Fläche in eframe 0.29/wgpu und HDR-Fähigkeit der Bildschirme des Nutzers.
19. Ob der Daemon unter Windows 11 als fensterloser Prozess die erhöhte Timerauflösung behält (R13 nennt die Einschränkung für fensterbesitzende Prozesse).
20. Ob Moonlight/Sunshine, Shadow oder Steam veröffentlichte Glas-zu-Glas-Messungen haben: in den gelesenen Quellen keine.
