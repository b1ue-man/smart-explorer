# Bildschirm-Streaming und Fernsteuerung — Primärquellen (S4, E5)

Stand und Abrufdatum 2026-10-17. Alle Quellen am 2026-10-17 abgerufen, sofern nicht anders vermerkt.

Zweck: Faktenbasis für Strang S4 (`docs/plaene/2026-10-17-rechte-desktop-tresore/strang-bildschirm.md`, Ergebnis E5 in `plan.md` A). Diese Datei trifft keine Entscheidung. Mit „Folgerung:“ markierte Sätze sind eigene Schlüsse, nicht Quelleninhalt. Mit „Annahme:“ markierte Sätze sind ungeprüft.

Projektstand (aus `native/Cargo.toml` und `native/Cargo.lock`, Codestand `da2646d2`): `iroh = 1.3.0` (zieht `iroh-relay 1.3.0`, `noq 1.3.0`, `noq-proto 1.3.0`, `noq-udp 1.3.0`; der Crate `quinn` ist **nicht** im Lockfile), `eframe = 0.29.1` / `egui = 0.29.1` (Features `glow`, `wgpu`, `accesskit`, `default_fonts`), `tokio = 1.52.3`.

---

## A. Bildschirm- und Fensteraufnahme je Betriebssystem

### A1. Windows.Graphics.Capture (WGC)

| Frage | Befund | Quelle |
|---|---|---|
| Was wird aufgenommen | „acquire frames from a display or application window“; Rahmen kommen als `Direct3D11CaptureFrame` (D3D11-Textur) aus einem `Direct3D11CaptureFramePool`, Format `DXGI_FORMAT_B8G8R8A8_UNORM`; bei HDR empfohlen `DXGI_FORMAT_R16G16B16A16_FLOAT`. Zeitstempel `SystemRelativeTime` (QPC). | https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture (ms.date 2026-08-23, git_commit 89095f60) |
| Auswahl durch Nutzer | `GraphicsCapturePicker` zeigt „secure system UI“; Ergebnis `GraphicsCaptureItem`. | dieselbe Seite |
| Programmatische Auswahl ohne Picker | `IGraphicsCaptureItemInterop::CreateForWindow(HWND)` (Mindestversion „Window 10 Version 1903 (Build 18362)“); Gegenstück `CreateForMonitor` (HMONITOR) in derselben Schnittstelle. | https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow |
| Zugriffsabfrage | `GraphicsCaptureAccess.RequestAccessAsync(GraphicsCaptureAccessKind)` mit den Werten `Borderless` (0) und `Programmatic` (1); eingeführt in Windows 10, version 2104 (10.0.20348.0), UniversalApiContract v12. | https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccesskind |
| Gelber Rahmen | System zeichnet „a yellow notification border … around the actively captured item“. `GraphicsCaptureSession.IsBorderRequired` (ab 10.0.20348.0, UniversalApiContract v12): Abschalten nur nach Zustimmung über `RequestAccessAsync(GraphicsCaptureAccessKind.Borderless)`; dafür Manifest-Capability `graphicsCaptureWithoutBorder`. „If the user denies access, setting this property to false will succeed, but the value will be ignored“. Setzt eine andere App für dasselbe Element `true`, bleibt der Rahmen. | https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired |
| Cursor | `GraphicsCaptureSession.IsCursorCaptureEnabled` ab Windows 10, version 2004 (10.0.19041.0), UniversalApiContract v10. | https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.iscursorcaptureenabled |
| Mindest-Windows-Version | Namespace und `CreateForWindow`: Windows 10 1903 (18362); Cursor-Schalter: 2004 (19041); Rahmen abschalten: 10.0.20348.0. | drei Quellen oben |
| Minimiertes Fenster | Microsoft-Doku nennt das Verhalten nicht. WebRTC-Quelltext (`modules/desktop_capture/win/wgc_capture_session.h`, Commit c6283a2c): „Will be reset if a source becomes momentarilly non-capturable - e.g. a window that gets minimized.“ und „e.g. when a captured window is brought back after being minimized.“ Folgerung: Ein minimiertes Fenster liefert keine neuen Rahmen; der Empfänger muss bis zum Wiederherstellen warten. | https://webrtc.googlesource.com/src/+/c6283a2cfc7e46a952d9215efcc40b80d0927ab0/modules/desktop_capture/win/wgc_capture_session.h |
| Sicherer Desktop / UAC | Die WGC-Doku nennt den sicheren Desktop nicht (Lücke). Für Desktop Duplication siehe A2: nur LOCAL_SYSTEM darf den sicheren Desktop lesen. Folgerung: WGC aus einem Nutzerprozess sieht den UAC-Dialog nicht; Beleg fehlt. | — |
| Größenwechsel | `Direct3D11CaptureFramePool.Recreate` bei Größen- oder Gerätewechsel; alle ausstehenden Rahmen werden verworfen. | Screen-capture-Seite oben |

### A2. DXGI Desktop Duplication

| Frage | Befund | Quelle |
|---|---|---|
| Zweck, Mindest-OS | Ersatz für XDDM-Spiegeltreiber ab Windows 8; `IDXGIOutput1::DuplicateOutput` Mindestclient „Windows 8 and Platform Update for Windows 7 [desktop apps only]“ (auf Windows 7 mit Platform Update aber `E_NOTIMPL`). | https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api ; https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput |
| Format | „always DXGI_FORMAT_B8G8R8A8_UNORM no matter what the current display mode is“ (bei `DuplicateOutput`); für weitere Formate und bessere Leistung verweist die Doku auf `IDXGIOutput5::DuplicateOutput1`. | dieselben |
| Änderungsinformation | `GetFrameDirtyRects` (geänderte Rechtecke) und `GetFrameMoveRects` (verschobene Bereiche: Quellpunkt + Zielrechteck); erst Move-, dann Dirty-Rechtecke anwenden. | desktop-dup-api |
| Zeiger | Zeiger kann im Bild enthalten oder separat sein; `PointerPosition` in `DXGI_OUTDUPL_FRAME_INFO`, Form über `GetFramePointerShape`. | desktop-dup-api |
| Drehung | Fläche immer unrotiert; Anwendung muss selbst drehen. | desktop-dup-api |
| Mehrfachmonitor | „it must create a desktop duplication interface on each active output“; keine Synchronisation zwischen Ausgängen, Zeitstempel je Ausgang. Gerät muss vom Adapter des Ausgangs stammen. | DuplicateOutput |
| Grenzen | Standard höchstens vier Prozesse je Sitzung gleichzeitig (`DXGI_ERROR_NOT_CURRENTLY_AVAILABLE`); ein Prozess je Ausgang nur eine Duplizierung. `DXGI_ERROR_UNSUPPORTED` bei 8bpp und Nicht-DWM-Modi (erneut nach `EVENT_SYSTEM_DESKTOPSWITCH`/`WM_DISPLAYCHANGE`). `DXGI_ERROR_SESSION_DISCONNECTED` bei getrennter Sitzung. | DuplicateOutput |
| Sicherer Desktop | „E_ACCESSDENIED … For example, only an application that runs at LOCAL_SYSTEM can access the secure desktop.“ | DuplicateOutput |
| HDR | Seite verweist für andere Formate auf `DuplicateOutput1`; HDR-Einzelheiten wurden nicht gelesen (Lücke). | — |
| Einzelfenster | Desktop Duplication liefert nur ganze Ausgänge; Fensteraufnahme nur durch Zuschneiden (Folgerung aus der API-Form „one interface per output“). | DuplicateOutput |

### A3. Linux Wayland: xdg-desktop-portal ScreenCast + PipeWire

| Frage | Befund | Quelle |
|---|---|---|
| Ablauf | `CreateSession` → `SelectSources` (nur einmal je Sitzung, vor `Start`) → `Start` → `OpenPipeWireRemote` (liefert FD für `pw_context_connect_fd`; nur die Screencast-Knoten sind darüber sichtbar). Doku beschreibt Interface-Version 6. | https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html |
| Quellentypen | Bitmaske `types`: 1 = MONITOR, 2 = WINDOW, 4 = VIRTUAL („Extend with new virtual monitor“); `multiple` für Mehrfachauswahl. Fensterauswahl ist damit Teil des Portals. | dieselbe |
| Rückfrage | `Start` „will typically result the portal presenting a dialog“. | dieselbe |
| Cursor | `cursor_mode` (ab Version 2): 1 Hidden, 2 Embedded, 4 Metadata (Cursor als PipeWire-Metadaten); nur in `AvailableCursorModes` angekündigte Werte, sonst wird die Sitzung geschlossen. | dieselbe |
| Dauerhafte Freigabe | `persist_mode` (ab Version 4): 0 nicht, 1 solange die Anwendung läuft, 2 bis zum ausdrücklichen Widerruf. `restore_token` aus `Start`; „The restore token is invalidated after using it once“ — jede Sitzung liefert den nächsten Token. Kann nicht wiederhergestellt werden, fragt das Portal normal. | dieselbe |
| Strombeschreibung | `streams` `a(ua{sv})` mit `id`, `position`, `size`, `source_type`, `mapping_id`, `pipewire-serial`; ab Version 6 Knoten-ID für das Zielen veraltet, `pipewire-serial` bevorzugt. | dieselbe |
| dmabuf | PipeWire 1.4.11: Erzeuger bietet zwei `SPA_PARAM_EnumFormat` an, „one for DMA-BUFs, one for shared memory buffers as a fallback“; Modifier über `SPA_FORMAT_VIDEO_modifier` mit `SPA_POD_PROP_FLAG_MANDATORY | SPA_POD_PROP_FLAG_DONT_FIXATE`; die Festlegung des Modifiers übernimmt der allozierende Knoten. | https://docs.pipewire.org/1.4/page_dma_buf.html |
| Kombination mit Eingabe | Mit einer RemoteDesktop-Sitzung werden `ScreenCast.SelectSources` und `OpenPipeWireRemote` auf der RemoteDesktop-Sitzung aufgerufen; Persistenz dann ausschließlich über RemoteDesktop (siehe D). | https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html |

### A4. Linux X11

| Frage | Befund | Quelle |
|---|---|---|
| MIT-SHM (XShm) | „essentially a version of the ximage interface where the actual image data is stored in a shared memory segment“; `XShmGetImage` liest ein Drawable. Funktioniert nicht „over the net“ (SysV-Shared-Memory). | https://xorg.freedesktop.org/archive/current/doc/xextproto/shm.html |
| DAMAGE | meldet geänderte Bereiche; Stufen `DamageReportRawRectangles`, `DamageReportDeltaRectangles`, `DamageReportBoundingBox`, `DamageReportNonEmpty`. | https://xorg.freedesktop.org/archive/current/doc/damageproto/damageproto.txt |
| Composite | Leitet Fensterhierarchie in Off-Screen-Speicher um, Inhalte „available without respect to sibling and antecedent clipping“ (verdeckte Fenster lesbar). `NameWindowPixmap` erzeugt „Match“-Fehler, wenn das Fenster nicht umgeleitet oder nicht sichtbar ist; nach jedem Map/Resize neue Pixmap. Minimieren wird nicht behandelt. | https://xorg.freedesktop.org/archive/current/doc/compositeproto/compositeproto.txt |
| Zustimmung | X11 kennt keine Aufnahme-Rückfrage (Folgerung: jeder Client derselben X-Sitzung kann lesen; kein Spezifikationszitat gelesen). | — |

### A5. Android MediaProjection

| Frage | Befund | Quelle |
|---|---|---|
| Zustimmung je Sitzung | „Your app must request user consent before each media projection session. A session is a single call to createVirtualDisplay().“ Ab Android 14 wirft `createVirtualDisplay()` `SecurityException`, wenn das `Intent` aus `createScreenCaptureIntent()` mehr als einmal an `getMediaProjection()` übergeben oder `createVirtualDisplay()` zweimal auf derselben `MediaProjection` aufgerufen wird. | https://developer.android.com/media/grow/media-projection |
| Vordergrunddienst | Ziel Android 14+: Berechtigungen `FOREGROUND_SERVICE` und `FOREGROUND_SERVICE_MEDIA_PROJECTION`, Dienst mit `android:foregroundServiceType="mediaProjection"`; fehlt der Typ: `MissingForegroundServiceTypeException`. | dieselbe |
| Einzelne App | „On Android 14 QPR2, users can share or record an app window rather than the entire device screen.“ Opt-out über `createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())`; Hersteller können das Opt-out übersteuern. | https://developer.android.com/about/versions/14/features/app-screen-sharing ; media-projection-Seite |
| Ziel-Surface | `createVirtualDisplay(name, w, h, dpi, VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, surface, …)`; Doku nennt `MediaRecorder`, `SurfaceTexture`, `ImageReader` als Surface-Lieferanten. Die MediaCodec-Input-Surface nennt diese Seite nicht (siehe B: `MediaCodec.createInputSurface`). Ab Android 12L wird Inhalt seitenverhältnistreu skaliert und zentriert. | media-projection |
| Rückrufe | `onCapturedContentResize`, `onCapturedContentVisibilityChanged`, `onStop` (Nutzer beendet, Bildschirm gesperrt, andere Sitzung startet, Prozess beendet). Ohne registrierten Callback wirft `createVirtualDisplay()` `IllegalStateException`. | media-projection |
| Anzeige | Ab Android 15 QPR1 großer Statusleisten-Chip zum Beenden; Projektion endet automatisch beim Sperren des Bildschirms. | media-projection |
| `FLAG_SECURE` | Die MediaProjection-Seite behandelt `FLAG_SECURE` nicht; die Referenzseite `WindowManager.LayoutParams#FLAG_SECURE` konnte im Abruf nicht bis zur Stelle gelesen werden (Lücke). Annahme: sichere Fenster erscheinen schwarz. | — |

### Vergleichstabelle A

| Plattform/API | Ganzer Bildschirm | Einzelfenster | Rückfrage | Dauerfreigabe | Cursor | GPU-Pfad | Sicherer Desktop / gesperrt |
|---|---|---|---|---|---|---|---|
| Windows WGC | ja (`CreateForMonitor`) | ja (`CreateForWindow`, 1903+) | Picker oder `RequestAccessAsync(Programmatic)` | Lücke: Dauer der Programmatic-Zustimmung nicht gelesen | `IsCursorCaptureEnabled` (2004+) | D3D11-Textur | Lücke (siehe A1) |
| Windows Desktop Duplication | ja, je Ausgang | nein (zuschneiden) | keine | — | separat (Form + Position) | D3D11-Textur, Dirty/Move-Rects | nur LOCAL_SYSTEM |
| Wayland Portal ScreenCast | ja (MONITOR) | ja (WINDOW) | Dialog bei `Start` | `persist_mode` 1/2 + einmaliger `restore_token` | Hidden/Embedded/Metadata | PipeWire dmabuf mit Shared-Memory-Rückfall | Lücke |
| X11 XShm/Damage/Composite | ja | ja über Composite (nur sichtbare/umgeleitete Fenster) | keine | — | Lücke (XFixes nicht gelesen) | CPU-Shared-Memory | — |
| Android MediaProjection | ja | ja ab 14 QPR2 (Nutzerwahl) | Dialog je Sitzung | nein (ab 14 Token einmalig) | im Bild (Annahme) | Surface → MediaCodec | endet bei Sperre |

---

## B. Kodierung und Dekodierung

### B1. Hardware über Betriebssystem-Schnittstellen

| Frage | Befund | Quelle |
|---|---|---|
| Windows Media Foundation H.264-Encoder (Microsoft) | MFT `Mfh264enc.dll`, Profile Baseline, Main, High (High ab Windows 8); Eingaben `MFVideoFormat_I420`, `IYUV`, `NV12`, `YUY2`, `YV12`; Ausgabe `MFVideoFormat_H264`. Ausgewählte Eigenschaften (die Seite listet für Windows 7 zwei und für Windows 8 fünfzehn weitere): `CODECAPI_AVEncCommonRateControlMode` (CBR und PeakConstrainedVBR ab Windows 8), `CODECAPI_AVEncCommonMeanBitRate` (zur Laufzeit änderbar ab Windows 8), `CODECAPI_AVEncMPVGOPSize`, `CODECAPI_AVEncMPVDefaultBPictureCount` (Standard 0), `CODECAPI_AVEncVideoForceKeyFrame`, `CODECAPI_AVLowLatencyMode`. „If a certified hardware encoder is present, it will generally be used instead of the inbox system encoder“. Pflichtattribute für zertifizierte Hardware-Encoder: RateControlMode, Quality, QualityVsSpeed, MeanBitRate, MaxBitRate, BufferSize, GOPSize, EncodeQP, ForceKeyFrame; optional (Windows 8.1, in HCK geprüft): `CODECAPI_AVEncVideoMinQP`, `CODECAPI_AVEncVideoLTRBufferControl`, `CODECAPI_AVEncVideoMarkLTRFrame`, `CODECAPI_AVEncVideoUseLTRFrame`, `CODECAPI_AVEncVideoEncodeFrameTypeQP`, `CODECAPI_AVEncSliceControlMode`, `CODECAPI_AVEncSliceControlSize`, `CODECAPI_AVEncVideoMaxNumRefFrame`, `CODECAPI_AVEncVideoMeanAbsoluteDifference`, `CODECAPI_AVEncVideoMaxQP`, `CODECAPI_AVEncVideoROIEnabled`, `CODECAPI_AVEncVideoTemporalLayerCount` (dynamisch), `CODECAPI_AVEncH264CABACEnable`; optional (Windows 8/8.1): `CODECAPI_AVEncVideoTemporalLayerCount` (statisch), `CODECAPI_AVLowLatencyMode`; optional ohne HCK-Test: `CODECAPI_AVEncMPVDefaultBPictureCount`. | https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder (git_commit 0d36b58c) |
| `CODECAPI_AVLowLatencyMode` | „This property applies to both encoders and decoders.“ „The encoder is expected to not add any sample delay due to frame reordering in encoding process, and one input sample shall produce one output sample.“ Mindestclient Windows 8. Warnung: MF-H.264-Decoder erwartet `VT_UI4`, alle übrigen Implementierungen `VT_BOOL`. Im Microsoft-H.264-Encoder schaltet `VARIANT_FALSE` auf Mehrbild-Kodierung (mehr Latenz), Standard ist Slice-Kodierung. | https://learn.microsoft.com/en-us/windows/win32/medfound/codecapi-avlowlatencymode (git_commit 4aee0701) |
| Windows HEVC-Encoder | MFT `Mfh265enc.dll`, nur Profil Main (`eAVEncH265VProfile_Main_420_8`), Mindestclient Windows 10; Eigenschaft `CODECAPI_VideoEncoderDisplayContentType` (Bildschirminhalt vs. Vollbildvideo), `CODECAPI_AVLowLatencyMode`, `CODECAPI_AVEncVideoForceKeyFrame`. Ob HEVC auf jedem Windows ohne Store-Erweiterung „HEVC Video Extensions“ verfügbar ist, nennt die Seite nicht (Lücke). | https://learn.microsoft.com/en-us/windows/win32/medfound/h-265---hevc-video-encoder |
| Windows AV1 über MF | Keine Microsoft-Learn-Seite für einen AV1-Encoder-MFT gefunden (Lücke). FFmpeg 8.1 enthält einen AV1-Encoder „via MediaFoundation“ (`mfenc.c`), d. h. Hersteller-MFTs für AV1 existieren (Folgerung). | https://fossies.org/dox/ffmpeg-8.1/mfenc_8c.html (nur Suchtreffer, nicht gelesen) |
| Hardware-MFT-Modell | Hardware-MFTs „must use the new asynchronous processing model“ und „must support dynamic format changes“; Attribut `MF_TRANSFORM_ASYNC = TRUE`. | https://learn.microsoft.com/en-us/windows/win32/medfound/hardware-mfts (git_commit 393520fb) |
| NVENC (NVIDIA Video Codec SDK 13.0) | Tuning „Ultra-low latency, with CBR“ für Cloud-Gaming/Streaming; Presets P1–P7; Intra-Refresh (`enableIntraRefresh`, `intraRefreshPeriod`, `intraRefreshCnt`) als Erholung bei unendlicher GOP; `NvEncInvalidateRefFrames` nimmt einen als verloren gemeldeten Rahmen (per `inputTimeStamp`) aus den Referenzen, ohne Referenz wird intra kodiert. Laufzeit `nvEncodeAPI.dll` / `libnvidia-encode.so` aus dem NVIDIA-Treiber; GPU ab Kepler. | https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html |
| VA-API (Linux) | `libva` steht unter MIT-Lizenz (MIT/X11-Variante). Welche Codecs kodiert werden können, hängt vom Treiber ab (Intel iHD, Mesa radeonsi/nouveau); die Treiberliste wurde nicht gelesen (Lücke). Rust-Bindung `cros-libva` 0.0.13 (BSD-3-Clause, letzte Veröffentlichung 2024-12-06, Repository github.com/chromeos/cros-libva). | https://raw.githubusercontent.com/intel/libva/master/COPYING ; https://crates.io/api/v1/crates/cros-libva |
| Android MediaCodec | `MediaFormat.KEY_LATENCY`: „desired encoder latency in frames“; `KEY_PRIORITY`: „Higher value means lower priority“ (0 = Echtzeit, Folgerung aus Wortlaut: niedriger Wert = höhere Priorität); `KEY_INTRA_REFRESH_PERIOD`: „period of intra refresh in frames“, wird ignoriert, wenn der Encoder es nicht kann; `KEY_LOW_LATENCY`: „applies only to decoders“; `KEY_PREPEND_HEADER_TO_SYNC_FRAMES`: SPS/PPS vor IDR; `KEY_REPEAT_PREVIOUS_FRAME_AFTER`: nur im „surface-input“-Modus, wiederholt den letzten Rahmen nach der angegebenen Zeit in µs (einmal). `MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME`: „Request that the encoder produce a sync frame "soon"“; `PARAMETER_KEY_VIDEO_BITRATE`: Zielbitrate „on the fly“; `createInputSurface`: „Requests a Surface to use as the input to an encoder, in place of input buffers.“ API-Level der einzelnen Schlüssel wurden nicht aus der Primärquelle bestätigt (developer.android.com-Referenz lieferte nur Navigation; Lücke). | AOSP-Quelltext `media/java/android/media/MediaFormat.java` und `MediaCodec.java`, Zweig `main` im Spiegel https://github.com/aosp-mirror/platform_frameworks_base (Commit nicht ermittelt) |
| Android-Codecverfügbarkeit | Android 16 CDD (Handheld): Encoder MUST H.264 [5.2/H-0-1], VP8 [5.2/H-0-2], AV1 [5.2/H-0-3]; Decoder MUST H.264, H.265, MPEG-4 SP, VP8, VP9, AV1 [5.3/H-0-1…6]. Hardware-Encoder-Pflicht (4K60) nur bei Media Performance Class V. developer.android.com: H.264 Baseline-Encoder ab Android 3.0, Main ab 6.0; AV1 „Encoder and decoder are mandatory beginning with Android 14.“ HEVC-Encoder ist nicht Pflicht. | https://source.android.com/docs/compatibility/16/android-16-cdd ; https://developer.android.com/media/platform/supported-formats |

### B2. Software-Codecs und Lizenzen

| Codec/Bibliothek | Lizenz | Fakten | Quelle |
|---|---|---|---|
| OpenH264 (Cisco) | Quelltext BSD-2-Clause; Binär-Lizenz von Cisco | Encoder „Constrained Baseline Profile up to Level 5.2 (Max frame size is 36864 macro-blocks)“, Decoder dieselbe Grenze; Plattformen Windows, macOS, Linux, Android, iOS. Patentabdeckung (MPEG LA AVC) nur für Ciscos Binärdatei, wenn sie „separately downloaded to an end user's device“ wird, nicht vorab gebündelt ist, der Nutzer sie ein-/ausschalten kann und „OpenH264 Video Codec provided by Cisco Systems, Inc.“ angezeigt wird; AVC-Lizenz begrenzt auf persönliche, nicht vergütete Nutzung. Folgerung: Selbst kompiliertes OpenH264 (über den `openh264`-Crate aus Quelltext gebaut; Feature-Name nicht geprüft) ist BSD-lizenziert, aber nicht von Ciscos Patentlizenz gedeckt. | https://raw.githubusercontent.com/cisco/openh264/master/README.md ; https://www.openh264.org/BINARY_LICENSE.txt |
| x264 | GNU GPL Version 2 (COPYING) | GPL ist mit Verteilung unter MIT nur vereinbar, wenn das Gesamtwerk unter GPL verteilt wird (Folgerung). | https://raw.githubusercontent.com/mirror/x264/master/COPYING (GitHub-Spiegel; code.videolan.org blockiert automatischen Abruf durch Anubis) |
| FFmpeg | LGPL 2.1 or later; mit `--enable-gpl`/GPL-Bibliotheken (libx264) GPL für das Ganze; `--enable-nonfree` nicht verteilbar | LGPL-Prüfliste: dynamisch linken, Quelltext samt configure-Optionen mitliefern, FFmpeg nennen. FFmpeg-Team „does not know“, ob Patente berührt sind. | https://ffmpeg.org/legal.html |
| rav1e (AV1-Encoder, Rust) | BSD-2-Clause | Crate 0.8.1, letzte Veröffentlichung 2025-06-16. | https://crates.io/api/v1/crates/rav1e |
| SVT-AV1 | BSD 3-Clause Clear License + „Alliance for Open Media Patent License 1.0“ (PATENTS.md); die Clear-BSD-Lizenz selbst gewährt keine Patente | Parameter: `--scm` (0 None, 1 Block Copy + Palette, 2 content adaptive; Standard 2), `--rtc` („fast settings for real-time communication“, erzwingt Low-Delay), `--pred-struct 1` (low delay); `--color-format` nennt yuv444, Beschreibung sagt aber „only yuv420 is supported at this time“. | https://gitlab.com/AOMediaCodec/SVT-AV1/-/raw/master/LICENSE.md ; …/PATENTS.md ; …/Docs/Parameters.md (Zweig master) |
| dav1d (AV1-Decoder) | BSD-2-Clause | Rust-Bindung `dav1d` 0.11.1 (MIT), 2025-11-25. | https://raw.githubusercontent.com/videolan/dav1d/master/COPYING ; https://crates.io/api/v1/crates/dav1d |
| libvpx (VP8/VP9) | BSD-3-Clause (WebM Project authors) | — | https://chromium.googlesource.com/webm/libvpx/+/refs/heads/main/LICENSE |

### B3. Bildschirminhalt-Werkzeuge

| Werkzeug | Befund | Quelle |
|---|---|---|
| AV1 Screen Content (Palette, IntraBC) | SVT-AV1 `--scm 1` = „Block Copy + Palette“, `2` = inhaltsabhängig (Standard). | SVT-AV1 Parameters.md |
| HEVC Bildschirminhalt-Hinweis auf Windows | `CODECAPI_VideoEncoderDisplayContentType` im MS-HEVC-Encoder. HEVC SCC-Erweiterung (Profile „Screen-Extended“) in Hardware-Encodern: nicht gelesen (Lücke). | HEVC-Encoder-Seite |
| 4:4:4 | MS-H.264-Encoder: nur 4:2:0/4:2:2-Eingaben (`NV12`, `I420`, `YUY2` …), kein 4:4:4 genannt. SVT-AV1: nur 4:2:0. OpenH264: Constrained Baseline (4:2:0). NVENC 4:4:4: nicht gelesen (Lücke). | Seiten oben |
| Dirty-Rectangles | Desktop Duplication liefert Dirty/Move-Rects (A2); X11 DAMAGE (A4). Folgerung: unveränderte Bilder müssen nicht kodiert werden. | A2, A4 |

### B4. Rust-Crates (Stand crates.io-API 2026-10-17)

| Crate | Version | Lizenz | Letzte Veröffentlichung | Repository | Zweck/Plattformen |
|---|---|---|---|---|---|
| `openh264` | 0.9.9 | BSD-2-Clause | 2026-10-15 | github.com/ralfbiedert/openh264-rs | Bindung an OpenH264 |
| `ffmpeg-next` | 9.0.0 | WTFPL (Bindung; FFmpeg selbst LGPL/GPL) | 2026-08-05 | github.com/zmwangx/rust-ffmpeg | FFmpeg-Wrapper |
| `gstreamer` | 0.25.4 | MIT OR Apache-2.0 (Bindung; GStreamer selbst LGPL, nicht gelesen) | 2026-09-21 | gitlab.freedesktop.org/gstreamer/gstreamer-rs | GStreamer-Bindung |
| `windows-capture` | 2.0.1 | MIT | 2026-08-08 | github.com/NiiightmareXD/windows-capture | WGC (Windows) |
| `xcap` | 0.9.8 | Apache-2.0 | 2026-08-01 | github.com/nashaofu/xcap | Bildschirmfotos Linux (X11, Wayland), macOS, Windows, HarmonyOS; Videoaufnahme laut Beschreibung noch in Arbeit |
| `scap` | 0.0.8 (stabil), 0.1.0-beta.1 | MIT | 2025-08-04 | github.com/helmerapp/scap | plattformübergreifende Aufnahme |
| `ashpd` | 0.13.13 | MIT | 2026-07-17 | github.com/bilelmoussaoui/ashpd | xdg-desktop-portal über zbus |
| `pipewire` | 0.10.1 | MIT | 2026-08-19 | gitlab.freedesktop.org/pipewire/pipewire-rs | PipeWire-Bindung |
| `cros-libva` | 0.0.13 | BSD-3-Clause | 2024-12-06 | github.com/chromeos/cros-libva | VA-API |
| `rav1e` | 0.8.1 | BSD-2-Clause | 2025-06-16 | github.com/xiph/rav1e | AV1-Encoder |
| `dav1d` | 0.11.1 | MIT | 2025-11-25 | github.com/rust-av/dav1d-rs | dav1d-Bindung |
| `hwcodec` (RustDesk) | 0.8.0 (Cargo.toml, nicht auf crates.io: 404) | kein `license`-Feld, keine Lizenz auf der Repository-Seite angezeigt | Lücke | github.com/rustdesk-org/hwcodec | „A real-time hardware codec library for RustDesk based on FFmpeg“: Windows QSV, NVENC, AMF, D3D11-Dekodierung; Linux VAAPI, NVENC/NVDEC, AMF; macOS/Android FFmpeg-Software |

### Vergleichstabelle B

| Weg | Windows | Linux | Android | Latenz-Stellschrauben | Lizenz/Patente |
|---|---|---|---|---|---|
| MF-Hardware-MFT (H.264/HEVC/AV1) | ja (je nach GPU-MFT) | — | — | `CODECAPI_AVLowLatencyMode`, ForceKeyFrame, LTR optional | OS-Komponente; Patentlizenz beim Gerätehersteller (Annahme) |
| VA-API | — | ja (treiberabhängig) | — | Lücke | libva MIT |
| NVENC direkt | ja | ja | — | Ultra-Low-Latency-Tuning, Intra-Refresh, `NvEncInvalidateRefFrames` | SDK-Lizenz nicht gelesen (Lücke) |
| MediaCodec | — | — | ja (H.264 und AV1 Pflicht-Encoder, HEVC optional) | `KEY_LATENCY`, `KEY_PRIORITY`, `KEY_INTRA_REFRESH_PERIOD`, `PARAMETER_KEY_REQUEST_SYNC_FRAME`, `KEY_LOW_LATENCY` (Decoder) | OS-Komponente |
| OpenH264 | ja | ja | ja | Constrained Baseline | BSD; Patente nur mit Cisco-Binärdownload |
| x264 | ja | ja | ja | — | GPL-2 |
| rav1e / SVT-AV1 | ja | ja | Lücke (Android-Build nicht geprüft) | SVT-AV1 `--rtc`, `--scm` | BSD-2 / BSD-3-Clear + AOM-Patentlizenz |
| dav1d (Dekoder) | ja | ja | ja (Annahme) | — | BSD-2 |

---

## C. Transport und Zuverlässigkeit

### C1. QUIC-Datagramme in iroh 1.3.0 / noq 1.3.0

Hinweis: iroh 1.3.0 nutzt nicht `quinn`, sondern n0s QUIC-Implementierung `noq` 1.3.0 („General purpose QUIC transport protocol implementation“, MIT OR Apache-2.0, https://github.com/n0-computer/noq; https://crates.io/api/v1/crates/noq). Die Frage „quinn in der Projektversion“ entfällt damit: `quinn` ist nicht im Lockfile.

| Frage | Befund | Quelle |
|---|---|---|
| Datagramm-API | `Connection::send_datagram(Bytes) -> Result<(), SendDatagramError>`: „Transmits data as an unreliable, unordered application datagram“, muss in ein QUIC-Paket passen. `send_datagram_wait` wartet bei Stau auf Pufferplatz. `read_datagram()` empfängt. | https://docs.rs/iroh/1.3.0/iroh/endpoint/struct.Connection.html |
| Größenlimit | `max_datagram_size() -> Option<usize>`: `None`, wenn der Peer Datagramme nicht unterstützt oder lokal deaktiviert; Wert ändert sich mit der Pfad-MTU-Schätzung. `initial_mtu` Standard und Minimum 1200, `min_mtu` ≥ 1200, MTU-Discovery standardmäßig an. Fehler `SendDatagramError::{UnsupportedByPeer, Disabled, TooLarge, ConnectionLost}`. | Connection-Seite; https://docs.rs/iroh/1.3.0/iroh/endpoint/struct.QuicTransportConfigBuilder.html ; https://docs.rs/iroh/1.3.0/iroh/endpoint/enum.SendDatagramError.html |
| Puffer | `datagram_receive_buffer_size(Option<usize>)` (`None` schaltet Empfang ab), `datagram_send_buffer_size(usize)`. Bei vollem Puffer werden die **ältesten** Datagramme verworfen (Senden und Empfangen). `datagram_send_buffer_space()`: Ein Datagramm bis zu dieser Größe „is guaranteed not to cause older datagrams to be dropped“. Standardwerte im `noq-proto`-Quelltext (Zweig `main`, nicht Tag 1.3.0): Empfang 1 250 000 Byte, Senden 1 048 576 Byte. | QuicTransportConfigBuilder; https://docs.rs/noq-proto/1.3.0/noq_proto/struct.TransportConfig.html ; https://raw.githubusercontent.com/n0-computer/noq/main/noq-proto/src/config/transport.rs |
| Staukontrolle | Module `noq::congestion`: `Cubic` (RFC 8312), `NewReno`, `Bbr3` („Experimental … use at your own risk“). Standard in `noq-proto` (Zweig `main`): `CubicConfig::default()`. Ob iroh 1.3.0 diesen Standard überschreibt, ist nicht geprüft (Lücke). Setzbar über `QuicTransportConfigBuilder::congestion_controller_factory`. Datagramme laufen nach RFC 9221 §5.4 unter derselben Staukontrolle. | https://docs.rs/noq/1.3.0/noq/congestion/index.html ; transport.rs |
| Messwerte | `rtt(path_id)`, `stats()`, `congestion_state(path_id)` („for debugging purposes“). | Connection-Seite |
| Relay | „A connection typically has one path via the relay server and, once holepunching succeeds, a direct path.“ Relay-Verbindung beginnt als HTTP/1.1 mit TLS und wird auf ein eigenes Relay-Protokoll über TCP umgeschaltet; Relays „can not decode any traffic … and only forward it“. Folgerung: QUIC-Pakete (also auch DATAGRAM-Frames) werden über das Relay als Ganzes weitergeleitet; über den Relay-Pfad läuft der Verkehr innerhalb einer TCP-Verbindung, d. h. Verlust auf der Strecke Client↔Relay wird von TCP wiederholt und erscheint als Verzögerung statt als Verlust. Die maximale Datagrammgröße über den Relay-Pfad ist nicht dokumentiert (Lücke). | https://docs.rs/iroh/1.3.0/iroh/index.html ; Connection-Seite |
| RFC 9221 | DATAGRAM-Frames werden nicht wiederholt, sind aber ack-auslösend (§5.2); nutzen die Staukontrolle der Verbindung (§5.4); keine Fragmentierung (§5); Transportparameter `max_datagram_frame_size` (0x20), Standard 0 = keine Unterstützung (§3); Bestätigung heißt nur, dass der Transport den Frame verarbeitet hat (§5.2). | https://www.rfc-editor.org/rfc/rfc9221.html |

### C2. Echtzeit-Staukontrolle

| Verfahren | Status/Datum | Kern | Quelle |
|---|---|---|---|
| Google Congestion Control (GCC) | draft-ietf-rmcat-gcc-02, abgelaufen, 2016-07-08 | Verzögerungsbasierter Regler (Gradient der Laufzeitdifferenz, Kalman-Filter, Überlastdetektor mit adaptiver Schwelle) + verlustbasierter Regler (Verlust > 10 %: Rate × (1 − 0,5 p); < 2 %: +5 %; dazwischen halten); Senderate = Minimum beider. Rückmeldung als transportweites Feedback (Senderseite) oder REMB (Empfängerseite). | https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02 |
| RFC 8888 | Standards Track, Januar 2021 | RTCP-Rückmeldung mit Ankunftszeit und ECN je Paket für Staukontrolle interaktiver RTP-Medien. | https://www.rfc-editor.org/rfc/rfc8888.html |
| SCReAM (RFC 8298) | Experimental, Dezember 2017 | „Self-Clocked Rate Adaptation for Multimedia“ für schwankende (LTE-)Bandbreite. | https://www.rfc-editor.org/rfc/rfc8298.html |
| SCReAMv2 | draft-ietf-ccwg-rfc8298bis-screamv2-00 vom 18. Mai 2026, Ziel Experimental | „adds support for L4S“; „similar principles apply to for instance QUIC“; nicht auf RTP beschränkt. libwebrtc enthält eine Implementierung unter `modules/congestion_controller/scream` auf Basis von -01, „currently in development“. | https://datatracker.ietf.org/doc/html/draft-ietf-ccwg-rfc8298bis-screamv2-00 ; https://webrtc.googlesource.com/src/+show/HEAD/modules/congestion_controller/scream/g3doc/implementation_diff.md |
| L4S (RFC 9330) | Informational, Januar 2023 | Architektur für niedrige Warteschlangenverzögerung, geringen Verlust und hohen Durchsatz zugleich. | https://www.rfc-editor.org/rfc/rfc9330.html |
| BBR in noq | `Bbr3` experimentell | siehe C1 | docs.rs noq congestion |

### C3. Fehlerbehandlung (Schlüsselbild, Referenzinvalidierung, FEC, Bitrate)

| Mechanismus | Befund | Quelle |
|---|---|---|
| Schlüsselbild anfordern | MF: `CODECAPI_AVEncVideoForceKeyFrame`; MediaCodec: `PARAMETER_KEY_REQUEST_SYNC_FRAME`; NVENC: Picture-Flag (nicht gelesen) | B1 |
| Referenzinvalidierung statt IDR | NVENC `NvEncInvalidateRefFrames`; MF optional LTR (`CODECAPI_AVEncVideoMarkLTRFrame`/`UseLTRFrame`); Moonlight nutzt RFI (siehe C4) | B1 |
| Intra-Refresh statt IDR | NVENC `enableIntraRefresh`; MediaCodec `KEY_INTRA_REFRESH_PERIOD` | B1 |
| Bitrate zur Laufzeit | MF `CODECAPI_AVEncCommonMeanBitRate` (ab Windows 8 während der Kodierung); MediaCodec `PARAMETER_KEY_VIDEO_BITRATE` | B1 |
| FEC | Moonlight: Reed-Solomon je Bild (Daten- + Paritätspakete); Sunshine `fec_percentage` Standard 20 | C4 |

### C4. Wie Referenzprojekte Latenz und Verlust lösen

| Projekt | Transport | Codec/Encoder | Verlust/Latenz | Quelle |
|---|---|---|---|---|
| Sunshine (Host) + Moonlight (Client) | GameStream-Protokoll; Steuerkanal über gebündeltes, modifiziertes ENet (UDP); Video als RTP-Pakete (Folgerung aus „RTP headers“ im Quelltext) | Sunshine: `nvenc`, `quicksync`, `amdvce`, `vaapi`, `vulkan` (nur Linux), `software`; H.264, HEVC (`hevc_mode`), AV1 (`av1_mode`); Erfassung Windows `ddx` (Desktop Duplication) und `wgc` (Beta, „not compatible with the Sunshine service“), Linux `nvfbc`, `wlr`, `kms`, `kwin`, `x11` | FEC Reed-Solomon je Bild, `fec_percentage` Standard 20; nicht wiederherstellbares Bild → `notifyFrameLost` an Host (RFI); sonst Warten auf IDR (`LiRequestIdrFrame`), nach 120 aufeinanderfolgenden verworfenen Bildern erzwungen; `nvenc_preset` Standard P1 | https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2configuration.html ; https://raw.githubusercontent.com/moonlight-stream/moonlight-common-c/master/src/RtpVideoQueue.c ; …/src/VideoDepacketizer.c ; https://github.com/moonlight-stream/moonlight-common-c |
| RustDesk | Direkt per Hole-Punching (TCP 21116 Registrierung/Hole-Punching, UDP 21116 Registrierung), sonst Relay `hbbr` über TCP 21117 bzw. WebSocket 21119; „In the majority of cases, hole punching is successful“ | Erfassung `libs/scrap`; Hardware über `hwcodec` (FFmpeg-basiert); unterstützte Codec-Liste im Protokoll nicht gelesen (`message.proto` nicht abrufbar, Lücke) | Verlustbehandlung nicht gelesen (Lücke) | https://rustdesk.com/docs/en/self-host/ ; https://github.com/rustdesk/rustdesk |
| Parsec | Proprietäres „BUD“: UDP, DTLS 1.2 (OpenSSL), „basic reliability semantics like TCP“, „highly tuned and custom congestion control algorithm“; Web-Client nutzt DTLS/SCTP | nicht öffentlich | nicht öffentlich | https://parsec.app/blog/a-networking-protocol-built-for-the-lowest-latency-interactive-game-streaming-1fd5a03a6007 (Datum nicht angezeigt) |
| Chrome Remote Desktop | WebRTC (`WebrtcVideoStream` mit `webrtc::PeerConnectionInterface`) | Encoder-Dateien `webrtc_video_encoder_vpx`, `webrtc_video_encoder_av1`, `webrtc_video_encoder_gpu` | WebRTC-Staukontrolle (welche, nicht gelesen) | https://chromium.googlesource.com/chromium/src/+/HEAD/remoting/protocol/webrtc_video_stream.h ; https://chromium.googlesource.com/chromium/src/+/HEAD/remoting/codec/ |
| scrcpy | adb-Tunnel (reverse `localabstract:scrcpy_<SCID>` → `tcp:27183`, sonst forward), bis zu drei Sockets Video/Audio/Steuerung, also TCP | Server per `app_process` als `shell`; MediaCodec auf Surface der Anzeige; 12-Byte-Rahmenkopf (Config-, Key-Frame-Flag, 61-Bit-PTS, u32-Länge); `--video-codec=h265` | Client zeigt Bilder „as soon as possible, without buffering“ (außer `--video-buffer`); Projektangabe 35–70 ms | https://raw.githubusercontent.com/Genymobile/scrcpy/master/doc/develop.md ; https://github.com/Genymobile/scrcpy (v5.0.1) |

### Vergleichstabelle C

| Ansatz | Verlustverhalten | Head-of-line-Blockade | Staukontrolle | Passt zu iroh 1.3.0 |
|---|---|---|---|---|
| QUIC-Datagramme + FEC + RFI/IDR | Verlust sichtbar, durch FEC/RFI behandelt | nein (direkter Pfad); ja auf Relay-Strecke durch TCP (Folgerung) | QUIC-CC (Cubic Standard in noq main) + eigene Ratenregelung | ja, API vorhanden |
| QUIC-Stream je Bild/GOP, veraltete Streams abbrechen | Wiederholung bis Abbruch | nur innerhalb eines Streams | QUIC-CC | ja (Streams vorhanden) |
| WebRTC (RTP/SRTP, GCC) | NACK/FEC/PLI | nein | GCC | nein, eigener ICE/DTLS-Stack neben iroh (Folgerung) |
| TCP (scrcpy, RustDesk-Relay) | Wiederholung, Latenzspitzen | ja | TCP | — |
| GameStream (Sunshine/Moonlight) | Reed-Solomon-FEC + RFI + IDR | nein | eigene Bitratenwahl (nicht gelesen) | Konzepte übertragbar, Code GPL-3.0 |

---

## D. Eingabe und laufende Programme

### D1. Windows

| Frage | Befund | Quelle |
|---|---|---|
| `SendInput` | „Synthesizes keystrokes, mouse motions, and button clicks.“ „This function is subject to UIPI. Applications are permitted to inject input only into applications that are at an equal or lesser integrity level.“ Blockade durch UIPI wird weder durch Rückgabewert noch `GetLastError` angezeigt. Ereignisse werden nicht mit anderen Eingaben vermischt; Tastaturzustand wird nicht zurückgesetzt. | https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput (git_commit fa536415) |
| Integritätsstufen und UIAccess | Ohne UIAccess: medium IL, kein Zugriff auf erhöhte Fenster. Mit UIAccess, gestartet von Nicht-Admin: „medium+“, kein Zugriff auf „high“-Fenster. Mit UIAccess, gestartet von Admin: „high“, Zugriff auf erhöhte Fenster. „None of the previously listed scenarios provide access to UI running under the system IL. This is only possible if the process is launched in the user account control (UAC) desktop under SYSTEM“. Anforderungen: Authenticode-Signatur, Installation an sicherem Ort (Liste in der nächsten Zeile), Manifest `uiAccess="true"`. „UIAccess should not be used: By applications that are not assistive technologies.“ | https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview (ms.date 2026-02-18, git_commit 0c26f44f) |
| Sichere Orte / UAC-Richtlinien | Sichere Orte für UIAccess: `%ProgramFiles%`, `%SystemRoot%\system32\`, `%ProgramFiles(x86)%`; Signaturprüfung immer. „Allow UIAccess applications to prompt for elevation without using the secure desktop“ (Standard Disabled, Registry `EnableUIADesktopToggle`): wenn aktiviert, erscheinen Erhöhungsdialoge für Standardnutzer auf dem interaktiven Desktop, „This setting allows the remote administrator to provide the appropriate credentials“. „Switch to the secure desktop when prompting for elevation“ Standard Enabled (`PromptOnSecureDesktop`). | https://learn.microsoft.com/en-us/windows/security/application-security/application-control/user-account-control/settings-and-configuration (ms.date 2026-05-14, git_commit 52ef23b6) |
| Sicherer Desktop | Aufnahme nur als LOCAL_SYSTEM (A2); Eingabe in System-IL-UI nur aus Prozess auf dem UAC-Desktop unter SYSTEM (oben). Folgerung: Bedienung von UAC-Dialogen braucht einen SYSTEM-Dienst, der einen Helfer in der Nutzersitzung auf dem Winlogon-Desktop startet; Microsoft-Beleg für dieses Muster nicht gelesen (Lücke). | A2; uiauto-securityoverview |
| Fensterliste | `EnumWindows` zählt Fenster der obersten Ebene auf, keine Kindfenster; „For Windows 8 and later, EnumWindows enumerates only top-level windows of desktop apps.“ | https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumwindows |

### D2. Linux

| Frage | Befund | Quelle |
|---|---|---|
| X11 XTest | XTEST ist „required to completely test the X11 server with no user intervention“; `XTestFakeKeyEvent`, `XTestFakeButtonEvent`, `XTestFakeMotionEvent`. Keine Zustimmung (Folgerung wie A4). | https://xorg.freedesktop.org/archive/X11R7.7/doc/libXtst/xtestlib.html |
| RemoteDesktop-Portal | Interface-Version 2; Gerätetypen KEYBOARD=1, POINTER=2, TOUCHSCREEN=4; `Start` „presents a dialog“; `persist_mode` 0/1/2 und einmaliger `restore_token` ab Version 2; `Notify*`-Methoden (PointerMotion, PointerMotionAbsolute im Koordinatenraum eines PipeWire-Stroms, PointerButton (evdev-Codes), PointerAxis, PointerAxisDiscrete, KeyboardKeycode, KeyboardKeysym, TouchDown, TouchMotion, TouchUp); `ConnectToEIS()` (ab v2) liefert FD für `ei_setup_backend_fd()`, einmal je Sitzung, danach liefern `Notify*` Fehler. Zwischenablage über `Clipboard.RequestClipboard()` vor `Start`; Ergebnis `clipboard_enabled`. | https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html |
| libei | „library for emulated input, primarily aimed at the Wayland stack“; Server-Seite libeis im Compositor; Protokoll über UNIX-Socket; `liboeffis` ist „an helper library for DBus communication with the XDG RemoteDesktop portal“. Unterstützende Compositoren und Lizenz stehen nicht auf der Seite (Lücke). | https://libinput.pages.freedesktop.org/libei/ |
| uinput | „kernel module that makes it possible to emulate input devices from userspace“ über `/dev/uinput`; Ablauf `UI_SET_EVBIT`/`UI_SET_KEYBIT` → `UI_DEV_SETUP` → `UI_DEV_CREATE` → Ereignisse → `UI_DEV_DESTROY`. Nötige Rechte nennt die Kernel-Doku nicht (Lücke; Annahme: Schreibrecht auf `/dev/uinput`, üblich root oder udev-Regel). uinput wirkt unterhalb des Compositors auf alle Sitzungen und braucht keine Portal-Zustimmung (Folgerung). | https://www.kernel.org/doc/html/latest/input/uinput.html |
| Fensterliste Wayland: wlroots | `wlr-foreign-toplevel-management-unstable-v1`: Taskleisten/Docks, auflisten und steuern (activate, close, minimize, maximize, fullscreen). Laut Kompositortabelle von wayland.app nicht in Mutter und KWin („x“, Bedeutung der Markierung auf der Seite nicht definiert); in Sway, Hyprland, niri, river vorhanden. | https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1 |
| Fensterliste Wayland: ext | `ext-foreign-toplevel-list-v1` (staging): Titel, `app_id`, stabile Kennung; keine Steuerung. Tabelle: Cosmic, Hyprland, Jay, Labwc, Louvre, Mir, niri, phoc, river, Sway, Treeland „1“; Cage, GameScope, KWin, Muffin, Mutter, Wayfire, Weston „x“. | https://wayland.app/protocols/ext-foreign-toplevel-list-v1 |
| Fensterliste KDE | `kde-plasma-window-management`: Fensterliste, Zustände, Schließen, Metadaten (Titel, App-ID, PID, Icon, Geometrie); „Regular clients must not use this protocol.“ Kompositortabelle auf wayland.app leer; KWin-Freigabe für Drittclients nicht belegt (Lücke). | https://wayland.app/protocols/kde-plasma-window-management |
| Fensterliste GNOME | `org.gnome.Shell.Introspect.GetWindows`: Maintainer (2024-05-28): „The Shell checks for access against an allow-list of D-Bus senders“; sonst nur im „unsafe mode“ über Looking Glass. | https://discourse.gnome.org/t/unable-to-call-the-remote-getwindows-gnome-method-via-dbus/21201 |

### D3. Android

| Frage | Befund | Quelle |
|---|---|---|
| Gesten | `AccessibilityService.dispatchGesture`: „Dispatch a gesture to the touch screen.“ „Any gestures currently in progress, whether from the user, this service, or another service, will be cancelled.“ Erfordert die Capability `android:canPerformGestures`. | AOSP `core/java/android/accessibilityservice/AccessibilityService.java`, Zweig `main` (https://github.com/aosp-mirror/platform_frameworks_base) |
| Globale Aktionen | `performGlobalAction` mit `GLOBAL_ACTION_BACK`, `HOME`, `RECENTS`, `NOTIFICATIONS`, `QUICK_SETTINGS`, `POWER_DIALOG`, `TOGGLE_SPLIT_SCREEN`, `LOCK_SCREEN`, `TAKE_SCREENSHOT`, `KEYCODE_HEADSETHOOK`, `ACCESSIBILITY_BUTTON`, `ACCESSIBILITY_BUTTON_CHOOSER`, `ACCESSIBILITY_SHORTCUT`, `ACCESSIBILITY_ALL_APPS`, `DISMISS_NOTIFICATION_SHADE`, `DPAD_UP`, `DPAD_DOWN`, `DPAD_LEFT`, `DPAD_RIGHT`, `DPAD_CENTER`, `MENU`, `MEDIA_PLAY_PAUSE`. API-Level je Konstante nicht aus Primärquelle bestätigt (Lücke). | dieselbe |
| Texteingabe | `AccessibilityNodeInfo.ACTION_SET_TEXT` („sets the text of the node“, Argument `ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE`), `ACTION_PASTE`. Folgerung: Texteingabe ersetzt Feldinhalt je Knoten, kein Tastendruck-Strom; Tastatur-Ereignisse beliebiger Apps sind über AccessibilityService nicht injizierbar (kein Beleg gelesen, Lücke). | AOSP `core/java/android/view/accessibility/AccessibilityNodeInfo.java` |
| Sichere Fenster | `ERROR_TAKE_SCREENSHOT_SECURE_WINDOW`: „the window contains secure content“. | AccessibilityService.java |
| Zustimmung / Einschränkung | Bedienungshilfe muss vom Nutzer in den Einstellungen aktiviert werden (Annahme, Seite nicht gelesen). „Restricted settings“: manche Einstellungen werden bei der Installation einer App gesperrt; Freigabe über App-Info → Menü → „Allow restricted settings“; Hinweis auf Android 13+. | https://support.google.com/android/answer/12623953 |
| Shizuku / `input` | Shizuku lässt Apps Systemschnittstellen „with adb or root privileges“ nutzen (Java-Prozess über `app_process`). Start mit root, per Wireless Debugging (ab Android 11, ohne Computer) oder per adb (Android ≤ 10, Computer nötig). „Due to system limitations, the startup steps need to be performed again after each reboot“ (ohne root). scrcpy injiziert über verstecktes `InputManager.injectInputEvent()` im `shell`-Prozess. | https://shizuku.rikka.app/introduction/ ; https://shizuku.rikka.app/guide/setup/ ; scrcpy develop.md |

### Vergleichstabelle D

| Plattform/Weg | Maus/Touch | Tastatur | Text | Erhöhte/sichere UI | Zustimmung | Fensterliste |
|---|---|---|---|---|---|---|
| Windows `SendInput` (medium IL) | ja | ja | ja (Unicode über `KEYEVENTF_UNICODE`, nicht gelesen) | nein (UIPI) | keine | `EnumWindows` |
| Windows mit UIAccess | ja | ja | ja | erhöhte Fenster, wenn Admin startet; kein System-IL | Signatur + sicherer Ort | `EnumWindows` |
| Windows SYSTEM-Helfer auf Winlogon-Desktop | ja (Folgerung) | ja (Folgerung) | — | ja (Folgerung) | Dienstinstallation | — |
| X11 XTest | ja | ja | ja | — | keine | EWMH (nicht gelesen) |
| Wayland RemoteDesktop-Portal + libei | ja (absolut im Stromraum) | ja (Keycode/Keysym) | über Keysyms | Lücke | Dialog, `persist_mode` 2 möglich | GNOME: nein; KDE: Lücke; wlroots: `wlr-foreign-toplevel` |
| Linux uinput | ja | ja | — | unterhalb des Compositors | Geräterechte | — |
| Android AccessibilityService | Gesten | nein (Lücke) | `ACTION_SET_TEXT` je Feld | `FLAG_SECURE`-Fenster: Screenshot-Fehler | Nutzer aktiviert Dienst; Restricted Settings | `getWindows` (nicht gelesen) |
| Android Shizuku/`shell` | ja (`injectInputEvent`) | ja | ja | Lücke | adb/Wireless Debugging je Neustart oder root | Lücke |

---

## E. Wiederverwendung vs. Neubau

### E1. Referenzprojekte: Lizenz und Architektur

| Projekt | Lizenz | Architektur (belegt) | Quelle |
|---|---|---|---|
| RustDesk | AGPL-3.0 (Datei `LICENCE`) | Rust + Flutter; `libs/scrap` (Aufnahme), `libs/enigo` (Eingabe), `libs/clipboard`, `libs/hbb_common` (Codecs, Konfiguration, TCP/UDP), `src/server` (Audio-, Zwischenablage-, Eingabe-, Videodienste), `src/rendezvous_mediator.rs` (direkt oder Relay); Server `hbbs`/`hbbr`; Hardwarecodec `hwcodec` (FFmpeg-basiert, Lizenz nicht angegeben) | https://github.com/rustdesk/rustdesk ; https://raw.githubusercontent.com/rustdesk/rustdesk/master/LICENCE ; https://rustdesk.com/docs/en/self-host/ ; https://github.com/rustdesk-org/hwcodec |
| Sunshine | GPL-3.0 | Host für GameStream/Moonlight; Erfassung `ddx`/`wgc` (Windows), `nvfbc`/`wlr`/`kms`/`kwin`/`x11` (Linux); Encoder `nvenc`/`quicksync`/`amdvce`/`vaapi`/`vulkan`/`software`; Host-Systeme laut README: FreeBSD 15.1+, Debian 13+, Fedora 43+, Ubuntu 22.04+, macOS 14.2+, Windows 11+ | https://github.com/LizardByte/Sunshine ; Sunshine-Konfigurationsdoku |
| Moonlight (moonlight-qt, moonlight-common-c) | GPL-3.0 (beide) | Client; `moonlight-common-c` „Core implementation of Nvidia's GameStream protocol“, gebündeltes modifiziertes ENet, Reed-Solomon-FEC, RFI/IDR | https://github.com/moonlight-stream/moonlight-qt ; https://github.com/moonlight-stream/moonlight-common-c |
| scrcpy | Apache-2.0 | Android-Server per `app_process` als `shell`, MediaCodec, adb-Tunnel, `InputManager.injectInputEvent()`; ab Android 5.0 (API 21); „does not require root“ | https://github.com/Genymobile/scrcpy ; https://raw.githubusercontent.com/Genymobile/scrcpy/master/LICENSE ; doc/develop.md |
| IronRDP | MIT OR Apache-2.0 | Rust-RDP; Client dekodiert Raw, Interleaved RLE, RDP 6.0, RemoteFX; Server (`ironrdp-server`) kodiert RDP 6.0, RemoteFX, optional NSCodec, QOI/QOI+zstd; Bibliotheken für ClearCodec, RemoteFX Progressive, ZGFX, EGFX-PDUs; H.264 im README nicht genannt | https://github.com/Devolutions/IronRDP |
| FreeRDP | Apache-2.0 | RDP-Bibliothek und Clients | https://github.com/FreeRDP/FreeRDP |
| gnome-remote-desktop | GPL-2.0-or-later | RDP (über FreeRDP) und VNC (LibVNCServer); Modi: Single-User-Headless, Headless-Remote-Login, Remote-Assistance | https://github.com/GNOME/gnome-remote-desktop |
| KRdp | Lizenz auf Projektseite nicht angezeigt (Lücke) | „Library and examples for creating an RDP server“; Streaming über KDE-Remote-Desktop-Portal; H.264 über Graphics Pipeline, Hardware nur VAAPI | https://invent.kde.org/plasma/krdp ; README.md |
| Weylus | AGPL-3.0-or-later (Beiträge BSD-3-Clause) | Aufnahme X11 bzw. PipeWire/GStreamer, H.264 per statisch gelinktem ffmpeg, fragmentiertes MP4 im Browser (MSE), Eingabe über WebSockets, Linux uinput | https://github.com/H-M-H/Weylus ; LICENSE |
| Deskreen | AGPL-3.0 | WebRTC-Stream des Bildschirms in einen Browser; Fernsteuerung nicht erwähnt | https://github.com/pavlobu/deskreen |

### E2. Eingebaute Fernzugriffs-Server

| System | Befund | Quelle |
|---|---|---|
| Windows | „The PC you want to connect to (the remote PC) must be running Windows Pro edition of Windows.“ Enterprise/Education nennt die Seite nicht (Lücke). Client auf jeder Edition. | https://support.microsoft.com/en-us/windows/how-to-use-remote-desktop-5fe128d5-8fb1-7a23-3b8a-41e636865e8c |
| GNOME | gnome-remote-desktop (RDP, VNC) | E1 |
| KDE Plasma | KRdp | E1 |
| Android | Kein eingebauter Fernzugriffs-Server in den gelesenen Quellen (Lücke: nicht gezielt gesucht). | — |

### Vergleichstabelle E (Verträglichkeit mit einer MIT-Verteilung)

Folgerungen aus den Lizenznamen, keine Rechtsberatung:

| Baustein | Lizenz | Code in MIT-Verteilung einbindbar | Als getrennter Prozess/Systemdienst nutzbar | Bemerkung |
|---|---|---|---|---|
| IronRDP | MIT OR Apache-2.0 | ja | ja | RDP-Client und -Server in Rust |
| FreeRDP | Apache-2.0 | ja (Apache-Hinweispflichten) | ja | C-Bibliothek |
| scrcpy | Apache-2.0 | ja | ja | Muster `shell`-Server für Android |
| windows-capture, ashpd, pipewire-rs | MIT | ja | — | Bindungen |
| xcap | Apache-2.0 | ja | — | — |
| openh264-Crate / OpenH264 | BSD-2-Clause | ja | — | Patentdeckung nur über Cisco-Binärdownload |
| rav1e, dav1d, libvpx | BSD-2 / BSD-2 / BSD-3 | ja | — | — |
| SVT-AV1 | BSD-3-Clause-Clear + AOM-Patentlizenz | ja | — | Clear-BSD gewährt keine Patente; AOM-Patentlizenz separat |
| FFmpeg | LGPL-2.1+ (GPL mit `--enable-gpl`) | nur dynamisch gelinkt mit LGPL-Pflichten | ja | x264 macht das Ganze GPL |
| x264 | GPL-2 | nein (Gesamtwerk würde GPL) | Lücke (Abgrenzung „separates Programm“ nicht geprüft) | — |
| Sunshine, Moonlight | GPL-3.0 | nein | Protokollkompatibilität ohne Codeübernahme: Lücke (Protokolldoku nicht gefunden) | — |
| RustDesk, Weylus, Deskreen | AGPL-3.0 | nein | — | — |
| gnome-remote-desktop | GPL-2.0+ | nein | ja, als Systemdienst über RDP ansprechbar (Folgerung) | — |
| Windows-RDP-Host | proprietär, Teil von Windows Pro | — | ja, über einen RDP-Client (IronRDP oder FreeRDP) | nicht auf Home |

---

## Widersprüchliche Angaben

1. App-Bildschirmfreigabe Android: https://developer.android.com/media/grow/media-projection sagt „Android 14 (API level 34) introduces app screen sharing“; https://developer.android.com/about/versions/14/features/app-screen-sharing sagt „On Android 14 QPR2, users can share or record an app window“. Folgerung: API-Level 34, ausgeliefert mit QPR2.
2. SVT-AV1 `--color-format`: Wertebereich nennt `3: yuv444`, Beschreibung sagt „only yuv420 is supported at this time“.
3. GCC-Draft: Datatracker nennt Zielstatus Experimental, Dokumentkopf Informational.
4. Sunshine `wgc`: als Erfassungsmethode angeboten, aber „not compatible with the Sunshine service“ (Dienstbetrieb in Session 0 vs. Nutzersitzung; Ursache nicht gelesen).
5. MF-H.264-Decoder: `CODECAPI_AVLowLatencyMode` als `VT_UI4`, alle übrigen MF-Komponenten `VT_BOOL`.
6. noq-Standardwerte (Cubic, Datagrammpuffer) stammen aus Zweig `main`, nicht aus Tag 1.3.0; iroh 1.3.0 kann eigene Standardwerte setzen.

## Offene Lücken

1. WGC: Verhalten bei sicherem Desktop/UAC und Dauer einer `GraphicsCaptureAccessKind.Programmatic`-Zustimmung — nicht in Microsoft-Doku gefunden.
2. DXGI `DuplicateOutput1`/HDR-Formate — nicht gelesen.
3. X11-Cursor (XFixes) und EWMH-Fensterliste — nicht gelesen.
4. Android `FLAG_SECURE`-Wortlaut und API-Level der Schlüssel `KEY_LATENCY`, `KEY_PRIORITY`, `KEY_INTRA_REFRESH_PERIOD`, `KEY_LOW_LATENCY` sowie der `GLOBAL_ACTION_*`-Konstanten — developer.android.com-Referenz lieferte im Abruf nur Navigation; Javadoc aus AOSP-Quelltext enthält keine API-Level.
5. Windows: Verfügbarkeit des HEVC-MFT ohne Store-Erweiterung; Microsoft-Doku zu AV1-Encoder-MFT.
6. VA-API: Codec-/Treibermatrix (Intel iHD, Mesa) für Kodierung.
7. NVENC: 4:4:4-Unterstützung und SDK-Lizenzbedingungen; HEVC-SCC in Hardware.
8. iroh 1.3.0: tatsächlicher Standard-Staukontroller, Standard-Datagrammpuffer, maximale Datagrammgröße über den Relay-Pfad.
9. RustDesk: `message.proto` (Codec-Aushandlung, Schlüsselbild-Anforderung) nicht abrufbar (`hbb_common`-Pfad 404); Verlustbehandlung nicht belegt.
10. GameStream-Protokollspezifikation (öffentliche Doku) nicht gefunden; Moonlight-Belege stammen aus Quelltext.
11. Chrome Remote Desktop: verwendete WebRTC-Staukontrolle nicht gelesen.
12. Parsec: Veröffentlichungsdatum des BUD-Artikels nicht angezeigt.
13. libei: Liste der Compositoren mit EIS-Unterstützung und Lizenz.
14. KDE `plasma-window-management` für Drittclients; KRdp-Lizenz.
15. uinput: erforderliche Rechte laut Primärquelle.
16. Android: Tastatur-Ereignisinjektion ohne Shizuku/root; AccessibilityService-Aktivierungsablauf; eingebauter Android-Fernzugriff.
17. Windows-Editionen mit RDP-Host außer Pro (Enterprise, Education) laut Primärquelle.
18. x264/GPL bei Nutzung als separater Prozess (rechtliche Abgrenzung).
